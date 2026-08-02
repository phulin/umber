use super::support::*;
use super::*;
use tex_command::CommandProfile;
use tex_state::InputOpenState;
use tex_state::meaning::UnexpandablePrimitive;
use tex_state::scaled::Scaled;

#[test]
fn pdf_font_output_actions_record_host_neutral_checkpointed_state() {
    let mut stores = stores_with_fonts();
    let mut control = canonical_pdf_font_control(&mut stores);
    register_canonical_font(&mut control, &mut stores, "cmr10.tfm", "cmr10.tfm");
    register_canonical_source(
        &mut control,
        concat!(
            "\\font\\base=cmr10 ",
            "\\pdfmapfile{+pdftex.map} ",
            "\\pdfmapline{+cmr10 CMR10 <cmr10.pfb} ",
            "\\pdffontattr\\base{/StemV 70} ",
            "\\pdfincludechars\\base{CABA} ",
            "\\pdfglyphtounicode{A}{0041} ",
            "\\pdfglyphtounicode{tfm:cmr10/ffi}{0066 0066 0069} ",
            "\\pdfglyphtounicode{Digamma}{D875 DFCB} ",
            "\\pdfnobuiltintounicode\\base \\end",
        )
        .as_bytes(),
    );
    run_canonical_to_end(&mut control, &mut stores);

    let font = font_meaning(&stores, "base");
    assert_eq!(stores.pdf_font_attribute(font), b"/StemV 70");
    assert_eq!(stores.included_pdf_font_chars(font), b"ABC");
    assert_eq!(
        stores.pdf_glyph_to_unicode(b"cmr10", b"A"),
        Some([0x41].as_slice())
    );
    assert_eq!(
        stores.pdf_glyph_to_unicode(b"cmr10", b"ffi.alt"),
        Some([0x66, 0x66, 0x69].as_slice())
    );
    assert_eq!(
        stores.pdf_glyph_to_unicode(b"cmr10", b"Digamma"),
        Some([0x2_D7CB].as_slice())
    );
    assert!(stores.pdf_builtin_to_unicode_disabled(font));
    let maps = stores.pdf_font_maps().collect::<Vec<_>>();
    assert!(matches!(
        maps[0],
        tex_state::PdfFontMapOperation::File(file)
            if file.logical_name == b"pdftex.map"
    ));
    assert!(matches!(
        maps[1],
        tex_state::PdfFontMapOperation::Line(line)
            if line.tex_name == b"cmr10" && line.font_file.as_deref() == Some(b"cmr10.pfb")
    ));
}

#[test]
fn empty_pdf_map_primitives_block_the_implicit_default_map() {
    for (name, primitive, source) in [
        (
            "pdfmapfile",
            UnexpandablePrimitive::PdfMapFile,
            "\\pdfmapfile{} \\end",
        ),
        (
            "pdfmapline",
            UnexpandablePrimitive::PdfMapLine,
            "\\pdfmapline{   } \\end",
        ),
    ] {
        let mut stores = stores_with_fonts();
        let mut control = canonical_pdf_font_control(&mut stores);
        assert_eq!(
            stores.meaning(stores.symbol(name).expect("PDF map primitive")),
            Meaning::UnexpandablePrimitive(primitive)
        );
        register_canonical_source(&mut control, source.as_bytes());
        run_canonical_to_end(&mut control, &mut stores);
        assert!(matches!(
            stores.pdf_font_maps().next(),
            Some(tex_state::PdfFontMapOperation::BlockDefault)
        ));
        assert!(stores.pdf_font_map_file_requests().is_empty());
    }
}

#[test]
fn pdf_glyph_to_unicode_warns_and_continues_for_out_of_range_value() {
    let mut stores = stores_with_fonts();
    let mut control = canonical_pdf_font_control(&mut stores);
    register_canonical_source(
        &mut control,
        concat!(
            "\\def\\legacyvalue{00740074}",
            "\\pdfglyphtounicode{t_t}{\\legacyvalue}",
            "\\pdfglyphtounicode{A}{0041}\\end"
        )
        .as_bytes(),
    );
    run_canonical_to_end(&mut control, &mut stores);
    assert!(
        terminal_effect_text(&stores)
            .contains("pdfTeX warning: pdftex: ToUnicode: value out of range [0,10FFFF]: 740074")
    );
    assert_eq!(stores.pdf_glyph_to_unicode(b"cmr10", b"t_t"), None);
    assert_eq!(
        stores.pdf_glyph_to_unicode(b"cmr10", b"A"),
        Some([0x41].as_slice())
    );
}

#[test]
fn duplicate_pdf_map_warning_uses_pdftex_positive_only_suppression() {
    const WARNING: &str =
        "pdfTeX warning: pdftex: fontmap entry for `cmr10' already exists, duplicates ignored";
    for (control, expects_warning) in [(-1, true), (0, true), (1, false)] {
        let mut stores = stores_with_fonts();
        let mut main_control = canonical_pdf_font_control(&mut stores);
        stores.set_int_param_global(
            tex_state::env::banks::IntParam::PDF_SUPPRESS_WARNING_DUP_MAP,
            control,
        );
        register_canonical_source(
            &mut main_control,
            concat!(
                "\\pdfmapline{cmr10 First <cmr10.pfb} ",
                "\\pdfmapline{+cmr10 Ignored <ignored.pfb} \\end",
            )
            .as_bytes(),
        );
        run_canonical_to_end(&mut main_control, &mut stores);
        assert_eq!(
            terminal_effect_text_unbroken(&stores).contains(WARNING),
            expects_warning,
            "\\pdfsuppresswarningdupmap={control}",
        );
    }
}

#[test]
fn pdf_font_expand_materializes_scaled_line_fonts() {
    let mut stores = stores_with_fonts();
    let mut control = canonical_pdf_font_control(&mut stores);
    register_canonical_font(&mut control, &mut stores, "cmr10.tfm", "cmr10.tfm");
    register_canonical_source(
        &mut control,
        br"\font\base=cmr10 \pdffontexpand\base 100 50 10 autoexpand \end",
    );
    run_canonical_to_end(&mut control, &mut stores);

    let base = font_meaning(&stores, "base");
    stores.set_pdf_font_code(tex_state::PdfFontCode::Ef, base, b'A', 1000);
    let source_width = stores
        .font_char_metrics(base, b'A')
        .expect("cmr10 contains A")
        .width;
    let mut nodes = vec![tex_state::node::Node::Char {
        font: base,
        ch: 'A',
        origin: tex_state::token::OriginId::UNKNOWN,
    }];
    let target = Scaled::from_raw(source_width.raw() + source_width.raw() / 10);
    crate::assignments::test_apply_line_expansion(&mut stores, &mut nodes, target)
        .expect("line expansion materializes a generated font");

    let tex_state::node::Node::Char { font: expanded, .. } = nodes[0] else {
        panic!("expanded line retains a character node")
    };
    assert_ne!(expanded, base);
    assert_eq!(
        stores
            .font_char_metrics(expanded, b'A')
            .expect("expanded A remains present")
            .width,
        target
    );
    assert!(matches!(
        stores.font(expanded).construction(),
        tex_fonts::FontConstruction::Expanded { ratio: 100, .. }
    ));

    stores.set_input_summary(tex_state::InputSummary::default());
    let format = stores.dump_format().expect("font expansion format dumps");
    let restored = Universe::from_format(tex_state::World::memory(), &format)
        .expect("font expansion format restores");
    let restored_base = font_meaning(&restored, "base");
    assert_eq!(
        restored.font_expansion(restored_base),
        Some(tex_state::font::FontExpansion {
            stretch: 100,
            shrink: 50,
            step: 10,
            auto_expand: true,
        })
    );
}

/// pdftex.web §§20580--21220 and §§24321--26029: modes 1 and 2 share
/// selected-line materialization; only mode 2 was allowed to affect the
/// breakpoint search proved in `tex-typeset`.
#[test]
fn pdftex_hz_modes_materialize_exact_selected_line_nodes() {
    let mut stores = stores_with_fonts();
    let mut control = canonical_pdf_font_control(&mut stores);
    register_canonical_font(&mut control, &mut stores, "cmr10.tfm", "cmr10.tfm");
    register_canonical_source(&mut control, br"\font\base=cmr10 \end");
    run_canonical_to_end(&mut control, &mut stores);
    let font = font_meaning(&stores, "base");
    stores
        .configure_font_expansion(
            font,
            tex_state::font::FontExpansion {
                stretch: 100,
                shrink: 50,
                step: 10,
                auto_expand: true,
            },
        )
        .expect("font expansion configuration is valid");
    for code in [b'A', b'.'] {
        stores.set_pdf_font_code(tex_state::PdfFontCode::Ef, font, code, 1000);
    }
    stores.set_pdf_font_code(tex_state::PdfFontCode::Lp, font, b'A', 500);
    stores.set_pdf_font_code(tex_state::PdfFontCode::Rp, font, b'.', 700);
    let source_width = stores
        .font_char_metrics(font, b'A')
        .expect("cmr10 contains A")
        .width;
    let period_width = stores
        .font_char_metrics(font, b'.')
        .expect("cmr10 contains period")
        .width;
    let natural = source_width
        .checked_add(period_width)
        .expect("two glyph widths fit");
    let target = Scaled::from_raw(natural.raw() + natural.raw() / 20);

    let mut signatures = Vec::new();
    for adjust in 0..=2 {
        for protrude in 0..=2 {
            let mut nodes = vec![
                tex_state::node::Node::Char {
                    font,
                    ch: 'A',
                    origin: tex_state::token::OriginId::UNKNOWN,
                },
                tex_state::node::Node::Char {
                    font,
                    ch: '.',
                    origin: tex_state::token::OriginId::UNKNOWN,
                },
            ];
            crate::assignments::test_materialize_pdf_line(
                &mut stores,
                &mut nodes,
                target,
                adjust > 0,
                protrude > 0,
            )
            .expect("selected-line microtype materializes");
            let fonts = nodes
                .iter()
                .filter_map(|node| match node {
                    tex_state::node::Node::Char { font, .. } => Some(*font),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let margins = nodes
                .iter()
                .filter_map(|node| match node {
                    tex_state::node::Node::MarginKern {
                        amount, side, ch, ..
                    } => Some((*amount, *side, *ch)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            signatures.push((fonts, margins));
        }
    }

    for protrude in 0..=2 {
        assert_eq!(signatures[3 + protrude], signatures[6 + protrude]);
    }
    assert_eq!(signatures[0].0, [font, font]);
    assert!(signatures[0].1.is_empty());
    assert_eq!(signatures[1].0, [font, font]);
    let quad = stores.font_parameter(font, 6);
    let amount = |code: i64| {
        Scaled::from_raw(
            i32::try_from(-((i64::from(quad.raw()) * code + 500) / 1000))
                .expect("protrusion amount fits scaled"),
        )
    };
    let left = amount(500);
    let right = amount(700);
    assert_eq!(
        signatures[1].1,
        [
            (left, tex_state::node::MarginKernSide::Left, b'A'),
            (right, tex_state::node::MarginKernSide::Right, b'.'),
        ]
    );
    assert_ne!(signatures[3].0, [font, font]);
    assert!(signatures[3].1.is_empty());
    assert_ne!(signatures[4].0, [font, font]);
    assert_eq!(
        signatures[4]
            .1
            .iter()
            .map(|(_, side, ch)| (*side, *ch))
            .collect::<Vec<_>>(),
        [
            (tex_state::node::MarginKernSide::Left, b'A'),
            (tex_state::node::MarginKernSide::Right, b'.'),
        ]
    );
}

#[test]
fn line_expansion_materializes_discrete_glyphs_kerns_and_reuses_fonts() {
    let mut stores = stores_with_fonts();
    let mut control = canonical_pdf_font_control(&mut stores);
    register_canonical_font(&mut control, &mut stores, "cmr10.tfm", "cmr10.tfm");
    register_canonical_source(
        &mut control,
        br"\font\base=cmr10 \pdffontexpand\base 100 50 10 autoexpand \end",
    );
    run_canonical_to_end(&mut control, &mut stores);
    let base = font_meaning(&stores, "base");
    for (code, efcode) in [(b'A', 1000), (b'V', 1000), (b'B', 500), (b'C', 0)] {
        stores.set_pdf_font_code(tex_state::PdfFontCode::Ef, base, code, efcode);
    }
    let kern = match stores.lig_kern_command(
        base,
        tex_fonts::LigKernChar::Char(b'A'),
        tex_fonts::LigKernChar::Char(b'V'),
    ) {
        Some(tex_fonts::LigKernCommand::Kern(kern)) => kern,
        command => panic!("cmr10 A/V font kern, got {command:?}"),
    };
    let origin = tex_state::token::OriginId::UNKNOWN;
    let source = vec![
        tex_state::node::Node::Char {
            font: base,
            ch: 'A',
            origin,
        },
        tex_state::node::Node::Kern {
            amount: kern,
            kind: tex_state::node::KernKind::Font,
        },
        tex_state::node::Node::Char {
            font: base,
            ch: 'V',
            origin,
        },
        tex_state::node::Node::Char {
            font: base,
            ch: 'B',
            origin,
        },
        tex_state::node::Node::Lig {
            font: base,
            ch: 'C',
            orig: vec!['C'],
            origins: vec![origin],
            left_hit: false,
            right_hit: false,
        },
    ];
    let mut first = source.clone();
    crate::assignments::test_apply_line_expansion(
        &mut stores,
        &mut first,
        Scaled::from_raw(100 * Scaled::UNITY),
    )
    .expect("maximum line expansion materializes");
    let glyph_font = |node: &tex_state::node::Node| match node {
        tex_state::node::Node::Char { font, .. } | tex_state::node::Node::Lig { font, .. } => *font,
        node => panic!("expected glyph, got {node:?}"),
    };
    let expanded_100 = glyph_font(&first[0]);
    assert_eq!(glyph_font(&first[2]), expanded_100);
    assert!(matches!(
        stores.font(expanded_100).construction(),
        tex_fonts::FontConstruction::Expanded { ratio: 100, .. }
    ));
    assert!(matches!(
        stores.font(glyph_font(&first[3])).construction(),
        tex_fonts::FontConstruction::Expanded { ratio: 50, .. }
    ));
    assert_eq!(
        glyph_font(&first[4]),
        base,
        "zero efcode retains the base ligature font"
    );
    let expected_kern = match stores.lig_kern_command(
        expanded_100,
        tex_fonts::LigKernChar::Char(b'A'),
        tex_fonts::LigKernChar::Char(b'V'),
    ) {
        Some(tex_fonts::LigKernCommand::Kern(kern)) => kern,
        command => panic!("expanded A/V font kern, got {command:?}"),
    };
    assert!(
        matches!(first[1], tex_state::node::Node::Kern { amount, kind: tex_state::node::KernKind::Font } if amount == expected_kern)
    );

    let mut second = source;
    crate::assignments::test_apply_line_expansion(
        &mut stores,
        &mut second,
        Scaled::from_raw(100 * Scaled::UNITY),
    )
    .expect("repeated line expansion reuses generated fonts");
    assert_eq!(glyph_font(&second[0]), expanded_100);
    assert_eq!(glyph_font(&second[3]), glyph_font(&first[3]));
}

#[test]
fn expanded_paragraph_shipout_retains_semantic_font_resource() {
    let mut stores = stores_with_fonts();
    let mut control = canonical_pdf_font_control(&mut stores);
    register_canonical_font(&mut control, &mut stores, "cmr10.tfm", "cmr10.tfm");
    register_canonical_source(&mut control, br"\font\base=cmr10 \end");
    run_canonical_to_end(&mut control, &mut stores);
    let base = font_meaning(&stores, "base");
    // This crate harness installs execution primitives, not the driver's
    // pdfTeX parameter/code aliases. Seed their live state directly so the
    // source below isolates paragraph materialization and shipout.
    stores
        .configure_font_expansion(
            base,
            tex_state::font::FontExpansion {
                stretch: 100,
                shrink: 50,
                step: 10,
                auto_expand: true,
            },
        )
        .expect("font expansion configuration is valid");
    stores.set_pdf_font_code(tex_state::PdfFontCode::Ef, base, b'A', 1000);
    stores.set_int_param_global(tex_state::env::banks::IntParam::PDF_ADJUST_SPACING, 1);
    stores.set_dimen_param_global(
        tex_state::env::banks::DimenParam::H_SIZE,
        Scaled::from_raw(23 * Scaled::UNITY),
    );
    let zero_glue = stores.intern_glue(tex_state::glue::GlueSpec::ZERO);
    stores.set_glue_param_global(tex_state::env::banks::GlueParam::PAR_FILL_SKIP, zero_glue);
    let mut control = canonical_pdf_font_control(&mut stores);
    register_canonical_source(
        &mut control,
        br"\shipout\vbox{\base \noindent AAA\par}\end",
    );
    run_canonical_to_end(&mut control, &mut stores);
    let artifact_id = stores.world().artifact_commits()[0];
    let bytes = stores
        .world()
        .read_artifact(artifact_id)
        .expect("read artifact")
        .expect("artifact stored");
    let artifact = tex_out::PageArtifact::from_bytes(&bytes).expect("artifact parses");
    let expanded = artifact
        .fonts
        .iter()
        .find_map(|font| match font.construction {
            tex_out::FontResourceConstruction::Expanded { ratio, .. } => {
                Some((font.font_id, ratio))
            }
            _ => None,
        })
        .expect("paragraph materialization registers an expanded font");
    assert_ne!(expanded.1, 0);
    fn contains_font(node: &tex_out::PageNode, font_id: u32) -> bool {
        match node {
            tex_out::PageNode::Char {
                font_id: actual, ..
            } => *actual == font_id,
            tex_out::PageNode::HList(list) | tex_out::PageNode::VList(list) => list
                .children
                .iter()
                .any(|child| contains_font(child, font_id)),
            _ => false,
        }
    }
    assert!(contains_font(&artifact.root, expanded.0));
    let reparsed = tex_out::PageArtifact::from_bytes(
        &artifact.to_bytes().expect("normalized artifact serializes"),
    )
    .expect("normalized artifact reparses");
    assert_eq!(reparsed, artifact);
}

#[test]
fn pdftex_generated_fonts_match_copy_and_letterspace_state() {
    let mut stores = stores_with_fonts();
    stores.set_int_param_global(tex_state::env::banks::IntParam::DEFAULT_HYPHEN_CHAR, 45);
    stores.set_int_param_global(tex_state::env::banks::IntParam::DEFAULT_SKEW_CHAR, -1);
    let mut control = canonical_pdf_font_control(&mut stores);
    register_canonical_font(&mut control, &mut stores, "cmr10.tfm", "cmr10.tfm");
    register_canonical_source(
        &mut control,
        "\\font\\base=cmr10 at 12pt \
         \\fontdimen2\\base=9pt \
         \\lpcode\\base`A=111 \
         \\hyphenchar\\base=99 \
         \\skewchar\\base=98 \
         \\pdfcopyfont\\copy=\\base \
         \\letterspacefont\\spaced=\\base 100 nolig \
         \\end"
            .as_bytes(),
    );
    if !run_canonical_generated_fonts_to_end(&mut control, &mut stores) {
        return;
    }

    let base = font_meaning(&stores, "base");
    let copy = font_meaning(&stores, "copy");
    let spaced = font_meaning(&stores, "spaced");
    assert_ne!(base, copy);
    assert_ne!(base, spaced);
    assert_eq!(stores.font_name(copy), "cmr10 at 12.0pt");
    assert_eq!(stores.font_name(spaced), "cmr10+100ls at 12.0pt");
    assert_eq!(stores.font_parameter(copy, 2).raw(), 9 * Scaled::UNITY);
    assert_eq!(stores.font_parameter(spaced, 2).raw(), 4 * Scaled::UNITY);
    assert_eq!(stores.font_hyphen_char(copy), 99);
    assert_eq!(stores.font_skew_char(copy), 98);
    assert_eq!(stores.font_hyphen_char(spaced), 45);
    assert_eq!(stores.font_skew_char(spaced), -1);
    assert_eq!(
        stores.pdf_font_code(tex_state::PdfFontCode::Lp, copy, b'A'),
        0
    );
    assert_eq!(
        stores.pdf_font_code(tex_state::PdfFontCode::Lp, spaced, b'A'),
        0
    );
    assert!(stores.pdf_font_ligatures_disabled(spaced));
    assert_eq!(
        stores
            .font_char_metrics(spaced, b'A')
            .expect("letterspaced A remains present")
            .width
            .raw()
            - stores
                .font_char_metrics(base, b'A')
                .expect("source A remains present")
                .width
                .raw(),
        78_643
    );
    let source = match stores.font(spaced).construction() {
        tex_fonts::FontConstruction::Letterspaced { source, .. } => *source,
        construction => panic!("unexpected construction {construction:?}"),
    };
    assert_eq!(stores.font_by_source_identity(source), Some(base));
}

#[test]
fn letterspaced_shipout_flattens_virtual_packets_onto_the_source_font() {
    let mut stores = stores_with_fonts();
    let mut control = canonical_pdf_font_control(&mut stores);
    register_canonical_font(&mut control, &mut stores, "cmr10.tfm", "cmr10.tfm");
    register_canonical_source(
        &mut control,
        "\\font\\base=cmr10 at 12pt \\
         \\letterspacefont\\spaced=\\base 100 nolig \\
         \\spaced \\shipout\\hbox{AA}\\end"
            .as_bytes(),
    );
    if !run_canonical_generated_fonts_to_end(&mut control, &mut stores) {
        return;
    }
    let artifact_id = stores.world().artifact_commits()[0];
    let bytes = stores
        .world()
        .read_artifact(artifact_id)
        .expect("read artifact")
        .expect("artifact stored");
    let artifact = tex_out::PageArtifact::from_bytes(&bytes).expect("artifact parses");
    let base = font_meaning(&stores, "base");
    let spaced = font_meaning(&stores, "spaced");
    let base_id = base.raw() - 1;
    let source_width = stores
        .font_char_metrics(base, b'A')
        .expect("source A remains present")
        .width;
    let spaced_width = stores
        .font_char_metrics(spaced, b'A')
        .expect("letterspaced A remains present")
        .width;
    let left = Scaled::from_raw(39_322);
    let right = spaced_width
        .checked_sub(source_width)
        .and_then(|difference| difference.checked_sub(left))
        .expect("letterspace movement");

    assert!(artifact.fonts.iter().any(|font| {
        matches!(
            font.construction,
            tex_out::FontResourceConstruction::Letterspaced {
                source_font_id,
                amount: 100,
                ..
            } if source_font_id == base_id
        )
    }));
    let tex_out::PageNode::HList(root) = &artifact.root else {
        panic!("shipout root should be an hlist")
    };
    assert!(matches!(
        root.children.as_slice(),
        [
            tex_out::PageNode::Kern { amount: first_left, kind: tex_out::KernKind::Explicit },
            tex_out::PageNode::Char { font_id: first_font, ch: 65, width: first_width },
            tex_out::PageNode::Kern { amount: first_right, kind: tex_out::KernKind::Explicit },
            tex_out::PageNode::Kern { amount: second_left, kind: tex_out::KernKind::Explicit },
            tex_out::PageNode::Char { font_id: second_font, ch: 65, width: second_width },
            tex_out::PageNode::Kern { amount: second_right, kind: tex_out::KernKind::Explicit },
        ] if *first_left == left
            && *second_left == left
            && *first_right == right
            && *second_right == right
            && *first_font == base_id
            && *second_font == base_id
            && *first_width == source_width
            && *second_width == source_width
    ));

    let dvi = tex_out::dvi::write_dvi(&[artifact]).expect("flattened DVI writes");
    assert!(dvi.windows(b"cmr10".len()).any(|bytes| bytes == b"cmr10"));
    assert!(!dvi.windows(b"+100ls".len()).any(|bytes| bytes == b"+100ls"));
}

fn canonical_font_control(stores: &mut Universe, profile: CommandProfile) -> CanonicalMainControl {
    match profile {
        CommandProfile::TEX82 => CanonicalMainControl::tex82_initex(stores),
        CommandProfile::ETEX26 => {
            let control = CanonicalMainControl::prepared_initex(profile);
            tex_command::install_tex82_expandable_primitives(stores);
            crate::install_unexpandable_primitives(stores);
            tex_command::install_etex_expandable_primitives(stores);
            crate::install_etex_unexpandable_primitives(stores);
            control
        }
        _ => panic!("font test helper supports TeX82 and e-TeX only"),
    }
}

fn canonical_pdf_font_control(stores: &mut Universe) -> CanonicalMainControl {
    tex_command::install_tex82_expandable_primitives(stores);
    for (name, primitive) in [
        ("pdffontattr", UnexpandablePrimitive::PdfFontAttr),
        ("pdffontexpand", UnexpandablePrimitive::PdfFontExpand),
        ("pdfincludechars", UnexpandablePrimitive::PdfIncludeChars),
        ("pdfmapfile", UnexpandablePrimitive::PdfMapFile),
        ("pdfmapline", UnexpandablePrimitive::PdfMapLine),
        (
            "pdfglyphtounicode",
            UnexpandablePrimitive::PdfGlyphToUnicode,
        ),
        (
            "pdfnobuiltintounicode",
            UnexpandablePrimitive::PdfNoBuiltinToUnicode,
        ),
        ("pdfcopyfont", UnexpandablePrimitive::PdfCopyFont),
        ("letterspacefont", UnexpandablePrimitive::LetterspaceFont),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    stores.set_int_param_global(tex_state::env::banks::IntParam::PDF_OUTPUT, 1);
    CanonicalMainControl::prepared_initex(CommandProfile::PDFTEX14027)
}

fn register_canonical_source(control: &mut CanonicalMainControl, bytes: &[u8]) {
    control
        .register_root_source(tex_command::SourceRegistration::new(
            tex_command::RegisteredSourceKind::Generated,
            bytes.to_vec(),
        ))
        .expect("register canonical font-test source");
}

fn register_canonical_font(
    control: &mut CanonicalMainControl,
    stores: &mut Universe,
    capability_name: &str,
    world_path: &str,
) {
    let metrics = tex_state::InputReadState::read_input_file(
        &mut stores.input_open_context(),
        std::path::Path::new(world_path),
    )
    .expect("font-test fixture reads through the world");
    control.capabilities_mut().register_font(
        capability_name,
        tex_command::FontResource::Tfm {
            metrics,
            opentype: None,
        },
    );
}

fn run_canonical_to_end(control: &mut CanonicalMainControl, stores: &mut Universe) {
    loop {
        match control
            .step(stores)
            .expect("canonical font program executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
}

/// Narrow xfail for umber2-alfh.4.11. Keep the state/output assertions live:
/// once canonical generated-font scanning lands, this falls through to them.
fn run_canonical_generated_fonts_to_end(
    control: &mut CanonicalMainControl,
    stores: &mut Universe,
) -> bool {
    loop {
        match control.step(stores) {
            Ok(MainControlStep::End | MainControlStep::EndOfInput) => return true,
            Ok(MainControlStep::Continue) => {}
            Err(ExecError::UnimplementedPrimitive {
                primitive:
                    UnexpandablePrimitive::PdfCopyFont
                    | UnexpandablePrimitive::LetterspaceFont,
                ..
            }) => return false,
            Err(error) => panic!("canonical generated-font program executes: {error:?}"),
        }
    }
}

#[test]
fn font_definition_loads_tfm_via_world_and_reuses_identity() {
    let mut stores = stores_with_fonts();
    let mut control = canonical_font_control(&mut stores, CommandProfile::TEX82);
    register_canonical_source(&mut control, br"\font\a=cmr10 \font\b=cmr10 \end");
    let state_before_request = stores.testing_state_hash();
    let request = match control.advance(&mut stores).expect("font request suspends") {
        CanonicalStepResult::Suspended(CanonicalResourceNeed::Font { request }) => request,
        other => panic!("expected font suspension, got {other:?}"),
    };
    assert_eq!(request.name, "cmr10");
    assert_eq!(stores.testing_state_hash(), state_before_request);
    register_canonical_font(&mut control, &mut stores, "cmr10.tfm", "cmr10.tfm");
    assert_eq!(
        control
            .advance(&mut stores)
            .expect("fulfilled font request retries atomically"),
        CanonicalStepResult::Progress(MainControlStep::Continue)
    );
    run_canonical_to_end(&mut control, &mut stores);

    let a = font_meaning(&stores, "a");
    let b = font_meaning(&stores, "b");
    assert_eq!(a, b);
    assert_eq!(stores.font_name(a), "cmr10");
    assert_eq!(stores.world().input_records().len(), 1);
}

#[test]
fn etex_font_character_enquiries_share_loaded_metrics() {
    let mut stores = stores_with_fonts();
    let mut control = canonical_font_control(&mut stores, CommandProfile::ETEX26);
    register_canonical_font(&mut control, &mut stores, "cmr10.tfm", "cmr10.tfm");
    register_canonical_source(&mut control,
        "\\font\\f=cmr10 \
         \\message{\\iffontchar\\f65Y\\else N\\fi/\\iffontchar\\f255Y\\else N\\fi}\
         \\message{\\the\\fontcharwd\\f65/\\the\\fontcharht\\f65/\\the\\fontchardp\\f65/\\the\\fontcharic\\f65/\\the\\fontcharwd\\f255}\
         \\end".as_bytes());
    run_canonical_to_end(&mut control, &mut stores);

    let output = terminal_effect_text(&stores);
    assert!(output.contains("Y/N"), "{output:?}");
    assert!(output.contains("/0.0pt"));
    assert!(!output.contains("0.0pt/0.0pt/0.0pt/0.0pt/0.0pt"));
}

#[test]
fn font_file_name_backs_up_the_first_non_character_token() {
    let mut stores = stores_with_fonts();
    let mut control = canonical_font_control(&mut stores, CommandProfile::TEX82);
    register_canonical_font(&mut control, &mut stores, "cmr10.tfm", "cmr10.tfm");
    register_canonical_source(&mut control, br"\font\a=cmr10\relax\message{loaded}\end");
    run_canonical_to_end(&mut control, &mut stores);

    assert_eq!(stores.font_name(font_meaning(&stores, "a")), "cmr10");
    assert!(terminal_effect_text(&stores).contains("loaded"));
}

#[test]
fn illegal_font_magnification_reports_and_uses_design_size() {
    let mut stores = stores_with_fonts();
    let mut control = canonical_font_control(&mut stores, CommandProfile::TEX82);
    register_canonical_font(&mut control, &mut stores, "cmr10.tfm", "cmr10.tfm");
    register_canonical_source(&mut control, br"\font\a=cmr10 scaled 32769 \end");
    run_canonical_to_end(&mut control, &mut stores);

    let font = font_meaning(&stores, "a");
    assert_eq!(stores.font(font).size(), stores.font(font).design_size());
    assert!(
        terminal_effect_text(&stores)
            .contains("Illegal magnification has been changed to 1000 (32769)")
    );
}

#[test]
fn font_definition_uses_driver_font_resolution_and_records_resolved_path() {
    const CMR10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    let mut stores = Universe::with_world(tex_state::World::memory()).with_plain_catcodes();
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("/fonts/cmr10.tfm", CMR10.to_vec())
        .expect("seed redirected font");
    let snapshot = stores.snapshot();
    let mut control = canonical_font_control(&mut stores, CommandProfile::TEX82);
    register_canonical_font(&mut control, &mut stores, "cmr10.tfm", "/fonts/cmr10.tfm");
    register_canonical_source(&mut control, br"\font\f=cmr10 \end");
    run_canonical_to_end(&mut control, &mut stores);

    let font = font_meaning(&stores, "f");
    assert_eq!(
        stores.font(font).path(),
        std::path::Path::new("/fonts/cmr10.tfm")
    );
    assert_eq!(stores.world().input_records().len(), 1);
    assert_eq!(
        stores.world().input_records()[0].path(),
        std::path::Path::new("/fonts/cmr10.tfm")
    );

    stores.rollback(&snapshot);
    assert!(stores.world().input_records().is_empty());
}

/// TeX.web §564 turns a malformed TFM into the ordinary recoverable
/// not-loadable path instead of aborting font-definition execution.
#[test]
fn malformed_tfm_uses_not_loadable_recovery() {
    let mut stores = Universe::with_world(tex_state::World::memory()).with_plain_catcodes();
    stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("/fonts/broken.tfm", vec![0, 1, 2])
        .expect("seed malformed font");
    let mut control = canonical_font_control(&mut stores, CommandProfile::TEX82);
    register_canonical_font(&mut control, &mut stores, "broken.tfm", "/fonts/broken.tfm");
    register_canonical_source(
        &mut control,
        br"\font\broken=broken \message{continued}\end",
    );
    run_canonical_to_end(&mut control, &mut stores);

    assert_eq!(font_meaning(&stores, "broken"), tex_state::font::NULL_FONT);
    let output = terminal_effect_text(&stores);
    assert!(output.contains("Font \\broken=broken not loadable: Bad metric (TFM) file"));
    assert!(output.contains("continued"));
}

/// TeX.web §567 reports capacity exhaustion after validating the TFM, leaves
/// the selector at nullfont, and commits no partial font row.
#[test]
fn font_capacity_reports_not_loaded_and_rolls_back() {
    let mut stores = stores_with_fonts();
    let mut serial = 0_u32;
    let mut survivor = tex_state::font::NULL_FONT;
    loop {
        let mut content_hash = [0_u8; 32];
        content_hash[..4].copy_from_slice(&serial.to_le_bytes());
        let font = tex_fonts::LoadedFont::new(
            format!("capacity-{serial}"),
            format!("capacity-{serial}.tfm"),
            content_hash,
            serial,
            Scaled::from_raw(10 * Scaled::UNITY),
            Scaled::from_raw(10 * Scaled::UNITY),
            Vec::new(),
            tex_fonts::FontMetrics::default(),
        );
        match stores.try_intern_font(font) {
            Ok(id) => {
                survivor = id;
                serial += 1;
            }
            Err(
                tex_state::FontParameterError::TooManyFonts { .. }
                | tex_state::FontParameterError::FontInfoCapacity { .. },
            ) => break,
            Err(error) => panic!("unexpected font fill error: {error:?}"),
        }
    }
    let survivor_name = stores.font(survivor).name().to_owned();
    let mut control = canonical_font_control(&mut stores, CommandProfile::TEX82);
    register_canonical_font(&mut control, &mut stores, "cmr10.tfm", "cmr10.tfm");
    register_canonical_source(
        &mut control,
        br"\font\overflow=cmr10 \message{continued}\end",
    );
    run_canonical_to_end(&mut control, &mut stores);

    assert_eq!(
        font_meaning(&stores, "overflow"),
        tex_state::font::NULL_FONT
    );
    assert_eq!(stores.font(survivor).name(), survivor_name);
    let output = terminal_effect_text(&stores);
    assert!(output.contains("Font \\overflow=cmr10 not loaded: Not enough room left"));
    assert!(output.contains("continued"));
}

#[test]
fn font_properties_are_inherently_global() {
    let mut stores = stores_with_fonts();
    let mut control = canonical_font_control(&mut stores, CommandProfile::TEX82);
    register_canonical_font(&mut control, &mut stores, "cmr10.tfm", "cmr10.tfm");
    register_canonical_source(
        &mut control,
        "\\font\\f=cmr10 \\relax \\fontdimen2\\f=10pt \
         {\\fontdimen2\\f=20pt \\hyphenchar\\f=128 \\skewchar\\f=129} \
         \\message{fd=\\the\\fontdimen2\\f,hc=\\the\\hyphenchar\\f,sc=\\the\\skewchar\\f}\\end"
            .as_bytes(),
    );
    run_canonical_to_end(&mut control, &mut stores);

    let output = terminal_effect_text(&stores);
    assert!(output.contains("fd=20.0pt,hc=128,sc=129"), "{output:?}");
}

/// TeX.web §580 grows the final loaded font's parameter bank until the
/// fixed `font_mem_size` pool is exhausted, then calls `overflow("font
/// memory", font_mem_size)`. The rejected word must not extend or otherwise
/// mutate the final loaded font.
#[test]
fn fontdimen_growth_reports_font_memory_capacity() {
    let mut stores = stores_with_fonts();
    let mut control = canonical_font_control(&mut stores, CommandProfile::TEX82);
    register_canonical_font(&mut control, &mut stores, "cmr10.tfm", "cmr10.tfm");
    register_canonical_source(
        &mut control,
        br"\font\f=cmr10 \relax \fontdimen1\f=1pt \fontdimen19993\f=2pt \fontdimen19994\f=9pt \end",
    );
    run_canonical_to_end(&mut control, &mut stores);

    assert_eq!(
        control.fatal_error(),
        Some(tex_command::FatalError::overflow("font memory", 20_000))
    );

    let font = font_meaning(&stores, "f");
    assert_eq!(stores.font_parameter_count(font), 19_993);
    assert_eq!(
        stores.font_parameter(font, 1),
        Scaled::from_raw(Scaled::UNITY)
    );
    assert_eq!(
        stores.font_parameter(font, 19_993),
        Scaled::from_raw(2 * Scaled::UNITY)
    );
}

#[test]
fn font_backed_integer_array_can_extend_and_read_entries() {
    let mut stores = stores_with_fonts();
    let mut control = canonical_font_control(&mut stores, CommandProfile::TEX82);
    register_canonical_font(&mut control, &mut stores, "cmr10.tfm", "cmr10.tfm");
    register_canonical_source(
        &mut control,
        br"\font\a=cmr10 at 1sp \fontdimen8\a=0sp \hyphenchar\a=128 \fontdimen85\a=85sp \end",
    );
    run_canonical_to_end(&mut control, &mut stores);
    let font = font_meaning(&stores, "a");
    assert_eq!(stores.font_hyphen_char(font), 128);
    assert_eq!(stores.font_parameter_count(font), 85);
    assert_eq!(stores.font_parameter(font, 85), Scaled::from_raw(85));
}

#[test]
fn grouped_font_backed_integer_array_setup_survives_group_exit() {
    let mut stores = stores_with_fonts();
    let mut control = canonical_font_control(&mut stores, CommandProfile::TEX82);
    register_canonical_font(&mut control, &mut stores, "cmr10.tfm", "cmr10.tfm");
    register_canonical_source(
        &mut control,
        br"{\global\font\a=cmr10 at 1001sp \fontdimen8\a=0sp \hyphenchar\a=128 \fontdimen85\a=85sp} \message{count=\the\hyphenchar\a,item=\the\fontdimen85\a}\end",
    );
    run_canonical_to_end(&mut control, &mut stores);

    let font = font_meaning(&stores, "a");
    assert_eq!(stores.font_hyphen_char(font), 128);
    assert_eq!(stores.font_parameter_count(font), 85);
    assert_eq!(stores.font_parameter(font, 85), Scaled::from_raw(85));
    assert!(
        terminal_effect_text(&stores).contains("count=128,item=0.0013pt"),
        "{:?}",
        terminal_effect_text(&stores)
    );
}

#[test]
fn the_fontdimen_reads_the_current_font_selector() {
    let mut stores = stores_with_fonts();
    let mut control = canonical_font_control(&mut stores, CommandProfile::TEX82);
    register_canonical_font(&mut control, &mut stores, "cmr10.tfm", "cmr10.tfm");
    register_canonical_source(
        &mut control,
        br"\font\f=cmr10 \fontdimen1\f=1.5pt \f\message{slant=\the\fontdimen1\font}\end",
    );
    run_canonical_to_end(&mut control, &mut stores);

    assert!(terminal_effect_text(&stores).contains("slant=1.5pt"));
}

#[test]
fn fontdimen_growth_is_limited_to_most_recently_loaded_font() {
    let mut stores = stores_with_fonts();
    let mut control = canonical_font_control(&mut stores, CommandProfile::TEX82);
    register_canonical_font(&mut control, &mut stores, "cmr10.tfm", "cmr10.tfm");
    register_canonical_font(&mut control, &mut stores, "cmtt10.tfm", "cmtt10.tfm");
    register_canonical_source(
        &mut control,
        br"\font\a=cmr10 \fontdimen8\a=1pt \font\b=cmtt10 \fontdimen9\a=2pt \end",
    );
    run_canonical_to_end(&mut control, &mut stores);

    let a = font_meaning(&stores, "a");
    assert_eq!(stores.font_parameter(a, 9).raw(), 0);
    assert!(terminal_effect_text(&stores).contains("has only"));
}

#[test]
fn short_tfm_keeps_fontdimen_seven_writable_after_a_later_font_load() {
    let mut stores = stores_with_fonts();
    let mut control = canonical_font_control(&mut stores, CommandProfile::TEX82);
    register_canonical_font(&mut control, &mut stores, "cmmi10.tfm", "cmmi10.tfm");
    register_canonical_font(&mut control, &mut stores, "cmr10.tfm", "cmr10.tfm");
    register_canonical_source(
        &mut control,
        br"\font\a=cmmi10 \font\b=cmr10 \fontdimen7\a=2pt \message{p7=\the\fontdimen7\a}\end",
    );
    run_canonical_to_end(&mut control, &mut stores);

    let a = font_meaning(&stores, "a");
    assert_eq!(stores.font_parameter_count(a), 7);
    assert_eq!(
        stores.font_parameter(a, 7),
        Scaled::from_raw(2 * Scaled::UNITY)
    );
    assert!(terminal_effect_text(&stores).contains("p7=2.0pt"));
}

#[test]
fn scanner_em_ex_units_use_current_font_parameters() {
    let mut stores = stores_with_fonts();
    let mut control = canonical_font_control(&mut stores, CommandProfile::TEX82);
    register_canonical_font(&mut control, &mut stores, "cmr10.tfm", "cmr10.tfm");
    register_canonical_source(
        &mut control,
        br"\font\f=cmr10 \relax \f\dimen0=1em \dimen1=1ex \end",
    );
    run_canonical_to_end(&mut control, &mut stores);

    let font = font_meaning(&stores, "f");
    assert_eq!(stores.dimen(0), stores.font_parameter(font, 6));
    assert_eq!(stores.dimen(1), stores.font_parameter(font, 5));
}

#[test]
fn scanner_em_ex_units_are_zero_for_nullfont() {
    let mut stores = stores_with_fonts();
    let mut control = canonical_font_control(&mut stores, CommandProfile::TEX82);
    register_canonical_source(&mut control, br"\dimen0=1em \dimen1=1ex \end");
    run_canonical_to_end(&mut control, &mut stores);

    assert_eq!(stores.dimen(0).raw(), 0);
    assert_eq!(stores.dimen(1).raw(), 0);
}

/// TeX.web §§552--556 initialize the complete nullfont record, including the
/// two mutable character codes that do not come from later font defaults.
#[test]
fn nullfont_has_all_canonical_defaults() {
    let mut stores = stores_with_fonts();
    let mut control = canonical_font_control(&mut stores, CommandProfile::TEX82);
    register_canonical_source(&mut control, br"\end");
    run_canonical_to_end(&mut control, &mut stores);
    let null = stores.font(tex_state::font::NULL_FONT);
    assert_eq!(null.name(), "nullfont");
    assert_eq!(null.path(), std::path::Path::new("nullfont"));
    assert_eq!(null.checksum(), 0);
    assert_eq!(null.design_size().raw(), 0);
    assert_eq!(null.size().raw(), 0);
    assert_eq!(stores.font_parameter_count(tex_state::font::NULL_FONT), 7);
    assert!((1..=7).all(|number| {
        stores
            .font_parameter(tex_state::font::NULL_FONT, number)
            .raw()
            == 0
    }));
    assert_eq!(stores.font_hyphen_char(tex_state::font::NULL_FONT), 45);
    assert_eq!(stores.font_skew_char(tex_state::font::NULL_FONT), -1);
    assert!((0_u8..=u8::MAX).all(|ch| !null.character_exists(char::from(ch))));
    assert!(matches!(
        null.construction(),
        tex_fonts::FontConstruction::Loaded
    ));
}

/// TeX.web §§548 and 560 dump and restore every loaded-font field needed to
/// reuse the same TeX82 font allocation and mutable parameter state.
#[test]
fn loaded_tfm_font_survives_format_round_trip() {
    let mut stores = stores_with_fonts();
    let mut control = canonical_font_control(&mut stores, CommandProfile::TEX82);
    register_canonical_font(&mut control, &mut stores, "cmr10.tfm", "cmr10.tfm");
    register_canonical_source(
        &mut control,
        br"\font\fixture=cmr10 at 12pt \fontdimen2\fixture=7pt \hyphenchar\fixture=99 \skewchar\fixture=98 \end",
    );
    run_canonical_to_end(&mut control, &mut stores);
    let original = font_meaning(&stores, "fixture");
    let expected = stores.font(original).clone();
    stores.set_input_summary(tex_state::InputSummary::default());
    let image = stores.dump_format().expect("loaded-font format dumps");

    let restored = Universe::from_format(tex_state::World::memory(), &image)
        .expect("loaded-font format restores");
    let font = font_meaning(&restored, "fixture");
    assert_eq!(font.raw(), original.raw());
    let loaded = restored.font(font);
    assert_eq!(loaded.name(), expected.name());
    assert_eq!(loaded.content_hash(), expected.content_hash());
    assert_eq!(loaded.checksum(), expected.checksum());
    assert_eq!(loaded.design_size(), expected.design_size());
    assert_eq!(loaded.size(), expected.size());
    assert_eq!(loaded.parameters(), expected.parameters());
    assert_eq!(loaded.metrics(), expected.metrics());
    assert_eq!(loaded.construction(), expected.construction());
    assert_eq!(
        restored.font_parameter(font, 2),
        Scaled::from_raw(7 * Scaled::UNITY)
    );
    assert_eq!(restored.font_hyphen_char(font), 99);
    assert_eq!(restored.font_skew_char(font), 98);
}

#[test]
fn scanner_em_unit_observes_runtime_fontdimen_write() {
    let mut stores = stores_with_fonts();
    let mut control = canonical_font_control(&mut stores, CommandProfile::TEX82);
    register_canonical_font(&mut control, &mut stores, "cmr10.tfm", "cmr10.tfm");
    register_canonical_source(
        &mut control,
        br"\font\f=cmr10 \relax \f\fontdimen6\f=12pt \dimen0=1em \end",
    );
    run_canonical_to_end(&mut control, &mut stores);

    assert_eq!(stores.dimen(0).raw(), 12 * tex_state::scaled::Scaled::UNITY);
}

#[test]
fn nullfont_the_font_and_fontname_render_from_font_state() {
    let mut stores = stores_with_fonts();
    let mut control = canonical_font_control(&mut stores, CommandProfile::TEX82);
    register_canonical_font(&mut control, &mut stores, "cmr10.tfm", "cmr10.tfm");
    register_canonical_source(
        &mut control,
        br"\message{A=\the\font|N=\fontname\nullfont}\font\foo=cmr10 \relax \foo\message{B=\the\font|F=\fontname\foo}\font\bar=cmr10 at 12pt \message{C=\fontname\bar}\end",
    );
    run_canonical_to_end(&mut control, &mut stores);

    let output = terminal_effect_text(&stores);
    assert!(output.contains("A=\\nullfont |N=nullfont"));
    assert!(output.contains("B=\\foo |F=cmr10"));
    assert!(output.contains("C=cmr10 at 12.0pt"));
}

#[test]
fn math_family_font_selectors_are_grouping_aware() {
    let mut stores = stores_with_fonts();
    let mut control = canonical_font_control(&mut stores, CommandProfile::TEX82);
    register_canonical_font(&mut control, &mut stores, "cmr10.tfm", "cmr10.tfm");
    register_canonical_font(&mut control, &mut stores, "cmtt10.tfm", "cmtt10.tfm");
    register_canonical_source(
        &mut control,
        br"\font\a=cmr10 \font\b=cmtt10 \textfont2=\a {\textfont2=\b \scriptfont2=\b}\end",
    );
    run_canonical_to_end(&mut control, &mut stores);

    let a = font_meaning(&stores, "a");
    assert_eq!(
        stores.math_family_font(tex_state::math::MathFontSize::Text, 2),
        a
    );
    assert_eq!(
        stores.math_family_font(tex_state::math::MathFontSize::Script, 2),
        tex_state::font::NULL_FONT
    );
}

/// TeX.web §§576--579 finalize a loaded font with seven addressable
/// parameters and the live default character codes, while `scan_font_ident`
/// recovers an invalid family and a missing selector to family zero/nullfont.
#[test]
fn canonical_font_defaults_and_family_selector_recovery() {
    let mut stores = stores_with_fonts();
    stores.set_int_param_global(tex_state::env::banks::IntParam::DEFAULT_HYPHEN_CHAR, 99);
    stores.set_int_param_global(tex_state::env::banks::IntParam::DEFAULT_SKEW_CHAR, 98);
    let mut control = canonical_font_control(&mut stores, CommandProfile::TEX82);
    register_canonical_font(&mut control, &mut stores, "cmmi10.tfm", "cmmi10.tfm");
    register_canonical_source(
        &mut control,
        br"\font\fixture=cmmi10 \textfont16=\relax \textfont1==\end",
    );
    run_canonical_to_end(&mut control, &mut stores);

    let fixture = font_meaning(&stores, "fixture");
    assert_eq!(stores.font_parameter_count(fixture), 7);
    assert_eq!(stores.font_hyphen_char(fixture), 99);
    assert_eq!(stores.font_skew_char(fixture), 98);
    assert_eq!(
        stores.math_family_font(tex_state::math::MathFontSize::Text, 0),
        tex_state::font::NULL_FONT
    );
    assert_eq!(
        stores.math_family_font(tex_state::math::MathFontSize::Text, 1),
        tex_state::font::NULL_FONT
    );
    let output = terminal_effect_text(&stores);
    assert!(output.contains("Bad number (16)"));
    assert!(output.contains("Missing font identifier"));
}
