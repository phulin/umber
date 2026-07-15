use super::support::*;
use super::*;
use tex_state::meaning::UnexpandablePrimitive;
use tex_state::scaled::Scaled;

#[test]
fn pdf_font_output_actions_record_host_neutral_checkpointed_state() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    for (name, primitive) in [
        ("pdffontattr", UnexpandablePrimitive::PdfFontAttr),
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
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    let mut input = InputStack::new(MemoryInput::new(concat!(
        "\\font\\base=cmr10 ",
        "\\pdfmapfile{+pdftex.map} ",
        "\\pdfmapline{+cmr10 CMR10 <cmr10.pfb} ",
        "\\pdffontattr\\base{/StemV 70} ",
        "\\pdfincludechars\\base{CABA} ",
        "\\pdfglyphtounicode{A}{0041} ",
        "\\pdfglyphtounicode{tfm:cmr10/ffi}{0066 0066 0069} ",
        "\\pdfnobuiltintounicode\\base \\end",
    )));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect("PDF font actions execute");

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
fn pdf_glyph_to_unicode_rejects_non_scalar_diagnostics() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let primitive = stores.intern("pdfglyphtounicode");
    stores.set_meaning(
        primitive,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfGlyphToUnicode),
    );
    let mut input = InputStack::new(MemoryInput::new("\\pdfglyphtounicode{A}{D800}\\end"));
    let error = Executor::new()
        .run(&mut input, &mut stores)
        .expect_err("surrogate is not a Unicode scalar");
    assert_eq!(
        error.to_string(),
        "pdfTeX error (\\pdfglyphtounicode): Unicode value is not a scalar value"
    );
}

#[test]
fn duplicate_pdf_map_warning_uses_pdftex_positive_only_suppression() {
    const WARNING: &str =
        "pdfTeX warning: pdftex: fontmap entry for `cmr10' already exists, duplicates ignored";
    for (control, expects_warning) in [(-1, true), (0, true), (1, false)] {
        let mut stores = stores_with_fonts();
        tex_expand::install_expandable_primitives(&mut stores);
        let primitive = stores.intern("pdfmapline");
        stores.set_meaning(
            primitive,
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfMapLine),
        );
        stores.set_int_param_global(
            tex_state::env::banks::IntParam::PDF_SUPPRESS_WARNING_DUP_MAP,
            control,
        );
        let mut input = InputStack::new(MemoryInput::new(concat!(
            "\\pdfmapline{cmr10 First <cmr10.pfb} ",
            "\\pdfmapline{+cmr10 Ignored <ignored.pfb} \\end",
        )));
        Executor::new()
            .run(&mut input, &mut stores)
            .expect("duplicate map actions execute");
        assert_eq!(
            terminal_effect_text(&stores).contains(WARNING),
            expects_warning,
            "\\pdfsuppresswarningdupmap={control}",
        );
    }
}

#[test]
fn pdf_font_expand_materializes_scaled_line_fonts() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let primitive = stores.intern("pdffontexpand");
    stores.set_meaning(
        primitive,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfFontExpand),
    );
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\base=cmr10 \\pdffontexpand\\base 100 50 10 autoexpand \\end",
    ));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect("font expansion configuration executes");

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

#[test]
fn pdftex_generated_fonts_match_copy_and_letterspace_state() {
    let mut stores = stores_with_fonts();
    stores.set_int_param_global(tex_state::env::banks::IntParam::DEFAULT_HYPHEN_CHAR, 45);
    stores.set_int_param_global(tex_state::env::banks::IntParam::DEFAULT_SKEW_CHAR, -1);
    tex_expand::install_expandable_primitives(&mut stores);
    for (name, primitive) in [
        ("pdfcopyfont", UnexpandablePrimitive::PdfCopyFont),
        ("letterspacefont", UnexpandablePrimitive::LetterspaceFont),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\base=cmr10 at 12pt \
         \\fontdimen2\\base=9pt \
         \\lpcode\\base`A=111 \
         \\hyphenchar\\base=99 \
         \\skewchar\\base=98 \
         \\pdfcopyfont\\copy=\\base \
         \\letterspacefont\\spaced=\\base 100 nolig \
         \\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("generated font definitions execute");

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
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let letterspacefont = stores.intern("letterspacefont");
    stores.set_meaning(
        letterspacefont,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::LetterspaceFont),
    );
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\base=cmr10 at 12pt \\
         \\letterspacefont\\spaced=\\base 100 nolig \\
         \\spaced \\shipout\\hbox{AA}\\end",
    ));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("letterspaced shipout succeeds");
    let bytes = stores
        .world()
        .read_artifact(stats.shipped_artifacts[0])
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

#[test]
fn font_definition_loads_tfm_via_world_and_reuses_identity() {
    let mut stores = stores_with_fonts();
    let mut input = InputStack::new(MemoryInput::new("\\font\\a=cmr10 \\font\\b=cmr10 \\end"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("font definitions execute");

    let a = font_meaning(&stores, "a");
    let b = font_meaning(&stores, "b");
    assert_eq!(a, b);
    assert_eq!(stores.font_name(a), "cmr10");
    assert_eq!(stores.world().input_records().len(), 2);
}

#[test]
fn etex_font_character_enquiries_share_loaded_metrics() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\f=cmr10 \
         \\message{\\iffontchar\\f65Y\\else N\\fi/\\iffontchar\\f255Y\\else N\\fi}\
         \\message{\\the\\fontcharwd\\f65/\\the\\fontcharht\\f65/\\the\\fontchardp\\f65/\\the\\fontcharic\\f65/\\the\\fontcharwd\\f255}\
         \\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("font character enquiries");

    let output = terminal_effect_text(&stores);
    assert!(output.contains("Y/N"), "{output:?}");
    assert!(output.contains("/0.0pt"));
    assert!(!output.contains("0.0pt/0.0pt/0.0pt/0.0pt/0.0pt"));
}

#[test]
fn font_file_name_backs_up_the_first_non_character_token() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\a=cmr10\\relax\\message{loaded}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("font name terminator should be redispatched");

    assert_eq!(stores.font_name(font_meaning(&stores, "a")), "cmr10");
    assert!(terminal_effect_text(&stores).contains("loaded"));
}

#[test]
fn illegal_font_magnification_reports_and_uses_design_size() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\font\\a=cmr10 scaled 32769 \\end"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("illegal font scale is recoverable");

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
    let mut stores = Universe::with_world(tex_state::World::memory());
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("/fonts/cmr10.tfm", CMR10.to_vec())
        .expect("seed redirected font");
    let snapshot = stores.snapshot();
    let mut input = InputStack::new(MemoryInput::new("\\font\\f=cmr10 \\end"));
    let mut resolvers = MemoryResolvers::new().with_font_root("/fonts");
    let mut context = resolvers.context();

    Executor::new()
        .run_with_context(&mut input, &mut stores, &mut context)
        .expect("font definition resolves through driver hook");

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

#[test]
fn font_properties_are_inherently_global() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\f=cmr10 \\relax \\fontdimen2\\f=10pt \
         {\\fontdimen2\\f=20pt \\hyphenchar\\f=128 \\skewchar\\f=129} \
         \\message{fd=\\the\\fontdimen2\\f,hc=\\the\\hyphenchar\\f,sc=\\the\\skewchar\\f}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("fontdimen assignments execute");

    let output = terminal_effect_text(&stores);
    assert!(output.contains("fd=20.0pt,hc=128,sc=129"), "{output:?}");
}

#[test]
fn fontdimen_capacity_boundary_is_injective_and_recovers_without_mutation() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\f=cmr10 \\relax \\fontdimen1\\f=1pt \\fontdimen131072\\f=2pt \\fontdimen131073\\f=9pt \\message{first=\\the\\fontdimen1\\f,max=\\the\\fontdimen131072\\f,bad=\\the\\fontdimen131073\\f}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("out-of-range fontdimen assignment recovers");

    let font = font_meaning(&stores, "f");
    assert_eq!(stores.font_parameter_count(font), 131_072);
    assert_eq!(
        stores.font_parameter(font, 1),
        Scaled::from_raw(Scaled::UNITY)
    );
    assert_eq!(
        stores.font_parameter(font, 131_072),
        Scaled::from_raw(2 * Scaled::UNITY)
    );
    let output = terminal_effect_text(&stores);
    assert!(output.contains("I ignored this assignment"));
    assert!(output.contains("first=1.0pt,max=2.0pt,bad=0.0pt"));
}

#[test]
fn font_backed_integer_array_can_extend_and_read_entries() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\a=cmr10 at 1sp \\fontdimen8\\a=0sp \\hyphenchar\\a=128 \\fontdimen85\\a=85sp \\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("font-backed integer array setup executes");
    let font = font_meaning(&stores, "a");
    assert_eq!(stores.font_hyphen_char(font), 128);
    assert_eq!(stores.font_parameter_count(font), 85);
    assert_eq!(stores.font_parameter(font, 85), Scaled::from_raw(85));
}

#[test]
fn grouped_font_backed_integer_array_setup_survives_group_exit() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "{\\global\\font\\a=cmr10 at 1001sp \
         \\fontdimen8\\a=0sp \\hyphenchar\\a=128 \
         \\fontdimen85\\a=85sp} \
         \\message{count=\\the\\hyphenchar\\a,item=\\the\\fontdimen85\\a}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("grouped font-backed integer array setup executes");

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
    tex_expand::install_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\f=cmr10 \\fontdimen1\\f=1.5pt \\f\\message{slant=\\the\\fontdimen1\\font}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("current-font fontdimen expands");

    assert!(terminal_effect_text(&stores).contains("slant=1.5pt"));
}

#[test]
fn fontdimen_growth_is_limited_to_most_recently_loaded_font() {
    let mut stores = stores_with_fonts();
    let mut ok = InputStack::new(MemoryInput::new(
        "\\font\\a=cmr10 \\fontdimen8\\a=1pt \\end",
    ));
    Executor::new()
        .run(&mut ok, &mut stores)
        .expect("last loaded font may grow");

    let mut bad = InputStack::new(MemoryInput::new(
        "\\font\\b=cmtt10 \\fontdimen9\\a=2pt \\end",
    ));
    Executor::new()
        .run(&mut bad, &mut stores)
        .expect("older font growth failure is recoverable");

    let a = font_meaning(&stores, "a");
    assert_eq!(stores.font_parameter(a, 9).raw(), 0);
    assert!(terminal_effect_text(&stores).contains("has only"));
}

#[test]
fn short_tfm_keeps_fontdimen_seven_writable_after_a_later_font_load() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\a=cmmi10 \\font\\b=cmr10 \\fontdimen7\\a=2pt \\message{p7=\\the\\fontdimen7\\a}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("guaranteed fontdimens remain assignable");

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
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\f=cmr10 \\relax \\f\\dimen0=1em \\dimen1=1ex \\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("em/ex assignments execute");

    let font = font_meaning(&stores, "f");
    assert_eq!(stores.dimen(0), stores.font_parameter(font, 6));
    assert_eq!(stores.dimen(1), stores.font_parameter(font, 5));
}

#[test]
fn scanner_em_ex_units_are_zero_for_nullfont() {
    let mut stores = stores_with_fonts();
    let mut input = InputStack::new(MemoryInput::new("\\dimen0=1em \\dimen1=1ex \\end"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("nullfont em/ex assignments execute");

    assert_eq!(stores.dimen(0).raw(), 0);
    assert_eq!(stores.dimen(1).raw(), 0);
}

#[test]
fn scanner_em_unit_observes_runtime_fontdimen_write() {
    let mut stores = stores_with_fonts();
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\f=cmr10 \\relax \\f\\fontdimen6\\f=12pt \\dimen0=1em \\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("fontdimen write affects em");

    assert_eq!(stores.dimen(0).raw(), 12 * tex_state::scaled::Scaled::UNITY);
}

#[test]
fn nullfont_the_font_and_fontname_render_from_font_state() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\message{A=\\the\\font|N=\\fontname\\nullfont}\\font\\foo=cmr10 \\relax \\foo\\message{B=\\the\\font|F=\\fontname\\foo}\\font\\bar=cmr10 at 12pt \\message{C=\\fontname\\bar}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("font rendering execute");

    let output = terminal_effect_text(&stores);
    assert!(output.contains("A=\\nullfont |N=nullfont"));
    assert!(output.contains("B=\\foo |F=cmr10"));
    assert!(output.contains("C=cmr10 at 12.0pt"));
}

#[test]
fn math_family_font_selectors_are_grouping_aware() {
    let mut stores = stores_with_fonts();
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\a=cmr10 \\font\\b=cmtt10 \\textfont2=\\a {\\textfont2=\\b \\scriptfont2=\\b}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("math family font assignments execute");

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

#[test]
fn math_family_assignment_recovers_bad_family_and_missing_font() {
    let mut stores = stores_with_fonts();
    let mut input = InputStack::new(MemoryInput::new("\\textfont16=\\relax \\textfont1=="));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("bounded family and missing font recover like TeX");

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
