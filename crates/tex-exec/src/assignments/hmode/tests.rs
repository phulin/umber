use super::*;
use crate::mode::PendingHChar;
use crate::tests::support::TestRecorder;
use tex_lex::MemoryInput;
use tex_state::hyphenation::ExceptionSpec;
use tex_state::node::Node;
use tex_state::provenance::SyntheticOriginKind;
use tex_state::token::TracedTokenWord;

#[test]
fn non_character_accent_lookahead_replays_the_original_traced_token() {
    let mut stores = Universe::new();
    crate::install_unexpandable_primitives(&mut stores);
    let origin = stores.synthetic_origin(SyntheticOriginKind::Test);
    let closing_group = TracedTokenWord::pack(
        Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        },
        origin,
    );
    let mut input = InputStack::new(MemoryInput::new(""));
    push_traced_tokens(&mut input, &mut stores, [closing_group]);

    let base = scan_accent_base(
        &mut ModeNest::new(),
        &mut input,
        &mut stores,
        &mut crate::ExecutionContext::new("texput"),
        TracedTokenWord::pack(
            Token::Char {
                ch: '^',
                cat: Catcode::Other,
            },
            OriginId::UNKNOWN,
        ),
    )
    .expect("accent lookahead should recover");

    assert_eq!(base, None);
    let summary = input.summary();
    let mut resumed = InputStack::from_summary(&summary, |_, _, _| {
        Ok::<_, core::convert::Infallible>(MemoryInput::new(""))
    })
    .expect("pushed-back token should be checkpoint-resumable");
    let replayed = resumed
        .next_traced_token(&mut stores)
        .expect("read replayed token")
        .expect("closing group should be backed up");
    assert_eq!(replayed, closing_group);
}

#[test]
fn accent_lookahead_runs_assignments_and_accepts_char_num() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\count0=7 \\char65"));
    let mut recorder = TestRecorder::default();
    let mut execution = crate::ExecutionContext::new("texput").recording(&mut recorder);

    let base = scan_accent_base(
        &mut ModeNest::new(),
        &mut input,
        &mut stores,
        &mut execution,
        TracedTokenWord::pack(
            Token::Char {
                ch: '^',
                cat: Catcode::Other,
            },
            OriginId::UNKNOWN,
        ),
    )
    .expect("accent base should scan");

    assert_eq!(base, Some(b'A'));
    assert_eq!(stores.count(0), 7);
    assert!(
        recorder.meanings.len() >= 2,
        "lookahead meanings should be recorded"
    );
}

#[test]
fn sentence_space_factor_does_not_jump_after_an_uppercase_letter() {
    let mut stores = Universe::new();
    stores.set_sfcode('.', 3000);
    let mut nest = ModeNest::new();

    update_space_factor(&mut nest, &stores, 'A');
    assert_eq!(nest.current_list().space_factor(), 999);

    update_space_factor(&mut nest, &stores, '.');
    assert_eq!(nest.current_list().space_factor(), 1000);

    update_space_factor(&mut nest, &stores, 'a');
    update_space_factor(&mut nest, &stores, '.');
    assert_eq!(nest.current_list().space_factor(), 3000);
}

#[test]
fn opentype_cmap_accepts_a_non_byte_horizontal_character() {
    use tex_fonts::{
        AcceptedFontContainers, FontFeaturePolicy, FontLimits, FontMetrics, FontPurposes,
        FontRequest, FontRequestKey, OpenTypeFont, OpenTypeProgramSelection, ResolvedFont,
        VariationSelection, WritingDirection,
    };

    let key = FontRequestKey::new(
        "cmu-serif-roman",
        0,
        VariationSelection::default(),
        FontFeaturePolicy::default(),
    )
    .expect("font key");
    let request = FontRequest {
        key: key.clone(),
        accepted_containers: AcceptedFontContainers::WASM,
        purposes: FontPurposes::LAYOUT_AND_HTML,
    };
    let font = OpenTypeFont::parse(
        &request,
        ResolvedFont {
            request: key,
            container: tex_fonts::FontContainer::Woff2,
            declared_object_sha256: None,
            declared_program_identity: None,
            provenance: None,
            bytes: include_bytes!("../../../../umber-wasm/assets/cmu-serif-500-roman.woff2")
                .to_vec(),
        },
        FontLimits::default(),
    )
    .expect("fixture font");
    let ch = font
        .cmap
        .mappings()
        .keys()
        .copied()
        .find(|scalar| *scalar > u32::from(u8::MAX))
        .and_then(char::from_u32)
        .expect("fixture has a non-byte mapping");
    let size = Scaled::from_raw(10 * Scaled::UNITY);
    let loaded = tex_fonts::LoadedFont::new(
        "cmu-serif",
        "cmu-serif.tfm",
        [0; 32],
        0,
        size,
        size,
        vec![Scaled::from_raw(0); 7],
        FontMetrics::new(Vec::new(), Vec::new(), None, None, Vec::new()),
    )
    .with_opentype(OpenTypeProgramSelection {
        font,
        variation: VariationSelection::default(),
        features: FontFeaturePolicy::default(),
        direction: WritingDirection::LeftToRight,
    });
    let mut stores = Universe::new();
    let font = stores.intern_font(loaded);
    stores.set_current_font(font);
    let mut nest = ModeNest::new();

    append_hchar(&mut nest, &mut stores, ch, OriginId::UNKNOWN);
    flush_pending_hchars(&mut nest, &mut stores).expect("OpenType character flushes");

    assert!(matches!(
        nest.current_list().nodes(),
        [Node::Char { font: actual_font, ch: actual_ch, .. }]
            if *actual_font == font && *actual_ch == ch
    ));
}

fn opentype_test_font(stores: &mut Universe, points: i32) -> tex_state::ids::FontId {
    use tex_fonts::{
        AcceptedFontContainers, FontFeaturePolicy, FontLimits, FontPurposes, FontRequest,
        FontRequestKey, OpenTypeFont, OpenTypeProgramSelection, ResolvedFont, VariationSelection,
        WritingDirection,
    };

    let features = FontFeaturePolicy::default();
    let key = FontRequestKey::new(
        format!("cmu-serif-shaping-{points}"),
        0,
        VariationSelection::default(),
        features.clone(),
    )
    .expect("font key");
    let font = OpenTypeFont::parse(
        &FontRequest {
            key: key.clone(),
            accepted_containers: AcceptedFontContainers::WASM,
            purposes: FontPurposes::LAYOUT_AND_HTML,
        },
        ResolvedFont {
            request: key,
            container: tex_fonts::FontContainer::Woff2,
            declared_object_sha256: None,
            declared_program_identity: None,
            provenance: None,
            bytes: include_bytes!("../../../../umber-wasm/assets/cmu-serif-500-roman.woff2")
                .to_vec(),
        },
        FontLimits::default(),
    )
    .expect("fixture font");
    let size = Scaled::from_raw(points * Scaled::UNITY);
    stores.intern_font(tex_fonts::LoadedFont::new_opentype(
        "cmu-serif-shaping",
        "cmu-serif-shaping.woff2",
        size,
        size,
        OpenTypeProgramSelection {
            font,
            variation: VariationSelection::default(),
            features,
            direction: WritingDirection::LeftToRight,
        },
    ))
}

#[test]
fn opentype_run_is_batched_and_uses_shaped_cluster_advance() {
    let mut stores = Universe::new();
    let font = opentype_test_font(&mut stores, 10);
    stores.set_current_font(font);
    let mut nest = ModeNest::new();

    for ch in "ffi".chars() {
        append_hchar(&mut nest, &mut stores, ch, OriginId::UNKNOWN);
    }
    flush_pending_hchars(&mut nest, &mut stores).expect("run flushes");

    let nodes = nest.current_list().nodes();
    assert_eq!(
        nodes
            .iter()
            .filter(|node| matches!(node, Node::Char { .. }))
            .count(),
        3
    );
    assert!(nodes.iter().any(|node| matches!(
        node,
        Node::Kern {
            kind: KernKind::Font,
            ..
        }
    )));
    let shaped = tex_shape::shape_run(
        stores.font(font).shaping_font().expect("fixture shapes"),
        "ffi",
        stores
            .font(font)
            .shaping_features()
            .expect("feature policy"),
        tex_shape::Direction::LeftToRight,
    );
    let expected: i32 = shaped
        .glyphs
        .iter()
        .map(|glyph| glyph.x_advance.raw())
        .sum();
    let actual: i32 = nodes
        .iter()
        .map(|node| match node {
            Node::Char { ch, .. } => stores
                .font_character_metrics(font, *ch)
                .expect("mapped character")
                .width
                .raw(),
            Node::Kern { amount, .. } => amount.raw(),
            _ => 0,
        })
        .sum();
    assert_eq!(actual, expected);
}

#[test]
fn reshaping_respects_font_kern_glue_and_discretionary_boundaries() {
    let mut stores = Universe::new();
    let first = opentype_test_font(&mut stores, 10);
    let second = opentype_test_font(&mut stores, 12);
    let empty = stores.freeze_node_list(&[]);
    let glue = stores.glue_param(GlueParam::SPACE_SKIP);
    let boundary_nodes = [
        Node::Kern {
            amount: Scaled::from_raw(17),
            kind: KernKind::Explicit,
        },
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,
            leader: None,
        },
        Node::Disc {
            kind: DiscKind::Discretionary,
            pre: empty,
            post: empty,
            replace: empty,
        },
    ];

    for boundary in boundary_nodes {
        let mut nodes = vec![
            Node::Char {
                font: first,
                ch: 'f',
                origin: OriginId::UNKNOWN,
            },
            boundary.clone(),
            Node::Char {
                font: first,
                ch: 'i',
                origin: OriginId::UNKNOWN,
            },
            Node::Char {
                font: second,
                ch: 'f',
                origin: OriginId::UNKNOWN,
            },
        ];
        reshape_open_type_runs(&stores, &mut nodes);
        let boundary_index = nodes
            .iter()
            .position(|node| node == &boundary)
            .expect("boundary retained");
        assert!(matches!(
            nodes[boundary_index - 1],
            Node::Char { ch: 'f', .. }
        ));
        assert!(
            nodes[boundary_index + 1..]
                .iter()
                .any(|node| matches!(node, Node::Char { ch: 'i', .. }))
        );
    }
}

#[test]
fn flushing_a_character_run_appends_its_right_boundary_kern() {
    use tex_fonts::metrics::CharTag;
    use tex_fonts::{CharMetrics, FontMetrics, LigKernInstruction, LoadedFont};

    let mut characters = vec![None; 256];
    characters[usize::from(b'A')] = Some(CharMetrics {
        width: Scaled::from_raw(Scaled::UNITY),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        italic_correction: Scaled::from_raw(0),
        tag: CharTag::LigKern {
            program_index: 0,
            start_index: 0,
        },
    });
    let boundary_kern = Scaled::from_raw(12_345);
    let metrics = FontMetrics::new(
        characters,
        vec![LigKernInstruction {
            skip_byte: 128,
            next_char: 255,
            command: Some(LigKernCommand::Kern(boundary_kern)),
        }],
        Some(255),
        None,
        Vec::new(),
    );
    metrics
        .validate()
        .expect("right-boundary test metrics should be valid");
    let mut stores = Universe::new();
    let font = stores.intern_font(LoadedFont::new(
        "right-boundary-kern",
        "right-boundary-kern.tfm",
        [0; 32],
        0,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(10 * Scaled::UNITY),
        vec![Scaled::from_raw(0); 7],
        metrics,
    ));
    let mut nest = ModeNest::new();
    nest.current_list_mut()
        .begin_pending_hchars(font, 'A', OriginId::UNKNOWN);

    flush_pending_hchars(&mut nest, &mut stores).expect("character run flushes");

    assert!(matches!(
        nest.current_list().nodes(),
        [
            Node::Char { font: actual_font, ch: 'A', .. },
            Node::Kern { amount, kind: KernKind::Font },
        ] if *actual_font == font && *amount == boundary_kern
    ));
}

#[test]
fn accent_delta_rounds_half_scaled_points_like_tex82() {
    assert_eq!(
        tex_state::scaled::text_accent_delta(
            Scaled::from_raw(10),
            Scaled::from_raw(1),
            Scaled::from_raw(0),
            Scaled::from_raw(0),
            Scaled::from_raw(0),
            Scaled::from_raw(0),
        ),
        Scaled::from_raw(5)
    );
}

#[test]
fn paragraph_leading_accent_is_replayed_after_entering_horizontal_mode() {
    const CMR10: &[u8] = include_bytes!("../../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    let mut stores = Universe::with_world(tex_state::World::memory());
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("cmr10.tfm", CMR10.to_vec())
        .expect("seed cmr10");
    let mut input = InputStack::new(MemoryInput::new("\\font\\f=cmr10 \\relax \\f \\accent19 E"));
    let mut executor = crate::Executor::new();

    executor
        .run(&mut input, &mut stores)
        .expect("paragraph-leading accent should execute");

    assert_eq!(executor.nest().current_mode(), crate::Mode::Horizontal);
    let nodes = executor.nest().current_list().nodes();
    assert!(
        matches!(
            nodes,
            [
                Node::HList(_),
                Node::Kern {
                    kind: KernKind::Accent,
                    ..
                },
                Node::HList(_),
                Node::Kern {
                    kind: KernKind::Accent,
                    ..
                },
                Node::Char { ch: 'E', .. },
                ..
            ]
        ),
        "unexpected paragraph-leading accent nodes: {nodes:?}"
    );
    let Node::HList(accent_box) = &nodes[2] else {
        unreachable!("matched shifted accent box")
    };
    assert!(matches!(
        stores.nodes(accent_box.children).testing_decoded(),
        [Node::Char { ch, .. }] if *ch == char::from(19)
    ));
}

#[test]
fn unrestricted_reconstitution_inserts_null_disc_after_font_hyphen() {
    const CMR10: &[u8] = include_bytes!("../../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    let mut stores = Universe::with_world(tex_state::World::memory());
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("cmr10.tfm", CMR10.to_vec())
        .expect("seed cmr10");
    let mut input = InputStack::new(MemoryInput::new("\\font\\f=cmr10 \\relax \\f"));
    crate::Executor::new()
        .run(&mut input, &mut stores)
        .expect("font selection should execute");
    let font = stores.current_font();
    stores.set_font_hyphen_char(font, i32::from(b'-'));
    let pending: Vec<_> = "in-line"
        .chars()
        .map(|ch| PendingHChar {
            font,
            ch,
            origin: tex_state::token::OriginId::UNKNOWN,
        })
        .collect();

    let unrestricted = reconstitute(&mut stores, &pending, false, true);
    let restricted = reconstitute(&mut stores, &pending, false, false);

    assert!(matches!(
        unrestricted.as_slice(),
        [
            Node::Char { ch: 'i', .. },
            Node::Char { ch: 'n', .. },
            Node::Char { ch: '-', .. },
            Node::Disc {
                kind: DiscKind::ExplicitHyphen,
                ..
            },
            Node::Char { ch: 'l', .. },
            Node::Char { ch: 'i', .. },
            Node::Char { ch: 'n', .. },
            Node::Char { ch: 'e', .. },
        ]
    ));
    assert!(
        !restricted
            .iter()
            .any(|node| matches!(node, Node::Disc { .. }))
    );
}

#[test]
fn hyphenation_inside_ff_ligature_preserves_the_unbroken_ligature() {
    const CMR10: &[u8] = include_bytes!("../../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    let mut stores = Universe::with_world(tex_state::World::memory());
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("cmr10.tfm", CMR10.to_vec())
        .expect("seed cmr10");
    let mut input = InputStack::new(MemoryInput::new("\\font\\f=cmr10 \\relax \\f"));
    crate::Executor::new()
        .run(&mut input, &mut stores)
        .expect("font selection should execute");
    stores.add_hyphenation_exception(ExceptionSpec {
        word: "difference".to_owned(),
        positions: vec![3],
    });
    let font = stores.current_font();
    stores.set_font_hyphen_char(font, i32::from(b'-'));
    let nodes: Vec<_> = "difference"
        .chars()
        .map(|ch| Node::Char {
            font,
            ch,
            origin: tex_state::token::OriginId::UNKNOWN,
        })
        .collect();

    let hyphenated = super::super::hyphenation::test_hyphenated_word(&mut stores, &nodes);
    let disc = hyphenated
        .iter()
        .find_map(|node| match node {
            Node::Disc {
                pre, post, replace, ..
            } => Some((*pre, *post, *replace)),
            _ => None,
        })
        .expect("the exception should create a discretionary");

    assert!(matches!(
        stores.nodes(disc.2).testing_decoded(),
        [Node::Lig {
            ch: '\u{b}',
            orig,
            ..
        }] if orig == &['f', 'f']
    ));
    assert!(
        matches!(
            stores.nodes(disc.0).testing_decoded(),
            [Node::Char { ch: 'f', .. }, Node::Char { ch: '-', .. }]
        ),
        "unexpected pre-break nodes: {:?}",
        stores.nodes(disc.0).testing_decoded()
    );
    assert!(matches!(
        stores.nodes(disc.1).testing_decoded(),
        [Node::Char { ch: 'f', .. }]
    ));
}

#[test]
fn composite_rechar_keeps_ligature_provenance_when_emitted() {
    let current = PendingHRunChar {
        font: tex_state::ids::FontId::testing_new(7),
        ch: 'A',
        orig: vec!['B'],
        origins: vec![tex_state::token::OriginId::UNKNOWN],
        ligature_present: true,
    };

    assert!(matches!(
        rechar_node(current.clone()),
        Node::Lig {
            font,
            ch: 'A',
            orig,
            ..
        } if font == current.font && orig == ['B']
    ));
}

#[test]
fn arbitrary_chained_ligature_keeps_complete_source_provenance() {
    use tex_fonts::metrics::CharTag;
    use tex_fonts::{CharMetrics, FontMetrics, LigKernInstruction, LigatureCommand, LoadedFont};

    let mut characters = vec![None; 256];
    characters[usize::from(b'A')] = Some(CharMetrics {
        width: Scaled::from_raw(Scaled::UNITY),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        italic_correction: Scaled::from_raw(0),
        tag: CharTag::LigKern {
            program_index: 0,
            start_index: 0,
        },
    });
    let metrics = FontMetrics::new(
        characters,
        vec![LigKernInstruction {
            skip_byte: 128,
            next_char: b'A',
            command: Some(LigKernCommand::Ligature(LigatureCommand {
                replacement: b'A',
                delete_current: true,
                delete_next: true,
                pass_over: 0,
            })),
        }],
        None,
        None,
        Vec::new(),
    );
    metrics
        .validate()
        .expect("test font metrics should be valid");
    let mut stores = Universe::new();
    let font = stores.intern_font(LoadedFont::new(
        "same-glyph-ligature",
        "same-glyph-ligature.tfm",
        [0; 32],
        0,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(10 * Scaled::UNITY),
        vec![Scaled::from_raw(0); 7],
        metrics,
    ));
    let pending = [
        PendingHChar {
            font,
            ch: 'A',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
        PendingHChar {
            font,
            ch: 'A',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
        PendingHChar {
            font,
            ch: 'A',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
    ];

    assert!(matches!(
        reconstitute(&mut stores, &pending, true, false).as_slice(),
        [Node::Lig {
            ch: 'A',
            orig,
            ..
        }] if orig == &['A', 'A', 'A']
    ));
}

#[test]
fn char_primitive_continues_the_pending_ligature_run() {
    const CMR10: &[u8] = include_bytes!("../../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    let mut stores = Universe::with_world(tex_state::World::memory());
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("cmr10.tfm", CMR10.to_vec())
        .expect("seed cmr10");
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\f=cmr10 \\relax \\f \\setbox0=\\hbox{f\\char102}",
    ));

    crate::Executor::new()
        .run(&mut input, &mut stores)
        .expect("character run should execute");

    let root = stores.box_reg(0).expect("box0");
    let Some(tex_state::node_arena::NodeRef::HList(hbox)) = stores.nodes(root).first() else {
        panic!("box0 should contain an hbox");
    };
    assert!(matches!(
        stores.nodes(hbox.children).testing_decoded(),
        [Node::Lig {
            orig,
            ..
        }] if orig == &['f', 'f']
    ));
}

#[test]
fn chained_ligature_retains_every_source_character() {
    const CMR10: &[u8] = include_bytes!("../../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    let mut stores = Universe::with_world(tex_state::World::memory());
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("cmr10.tfm", CMR10.to_vec())
        .expect("seed cmr10");
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\f=cmr10 \\relax \\f \\setbox0=\\hbox{ffi}",
    ));
    crate::Executor::new()
        .run(&mut input, &mut stores)
        .expect("ligature run should execute");
    let root = stores.box_reg(0).expect("box0");
    let Some(tex_state::node_arena::NodeRef::HList(hbox)) = stores.nodes(root).first() else {
        panic!("box0 should contain an hbox");
    };
    assert!(matches!(
        stores.nodes(hbox.children).testing_decoded(),
        [Node::Lig { orig, .. }] if orig == &['f', 'f', 'i']
    ));
}

#[test]
fn hyphenation_does_not_partially_consume_a_boundary_ligature() {
    let mut stores = Universe::new();
    let font = stores.current_font();
    stores.set_lccode('C', 'c' as u32);
    stores.set_lccode('/', 0);
    let nodes = [
        Node::Char {
            font,
            ch: 'C',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
        Node::Lig {
            font,
            ch: 'B',
            orig: vec!['C', '/'],
            origins: vec![tex_state::token::OriginId::UNKNOWN; 2],
        },
    ];

    let hyphenated = super::super::hyphenation::test_hyphenated_word(&mut stores, &nodes);
    assert!(matches!(
        hyphenated.as_slice(),
        [Node::Char { ch: 'C', .. }, Node::Lig { ch: 'B', .. }]
    ));
}

#[test]
fn hyphenation_keeps_scanning_across_font_kerns() {
    const CMR10: &[u8] = include_bytes!("../../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    let mut stores = Universe::with_world(tex_state::World::memory());
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("cmr10.tfm", CMR10.to_vec())
        .expect("seed cmr10");
    let mut input = InputStack::new(MemoryInput::new("\\font\\f=cmr10 \\relax \\f"));
    crate::Executor::new()
        .run(&mut input, &mut stores)
        .expect("font selection should execute");
    stores.add_hyphenation_exception(ExceptionSpec {
        word: "availability".to_owned(),
        positions: vec![5, 9],
    });
    let font = stores.current_font();
    stores.set_font_hyphen_char(font, i32::from(b'-'));
    let pending: Vec<_> = "availability"
        .chars()
        .map(|ch| PendingHChar {
            font,
            ch,
            origin: tex_state::token::OriginId::UNKNOWN,
        })
        .collect();
    let nodes = reconstitute(&mut stores, &pending, false, false);
    assert!(
        nodes.iter().any(|node| matches!(
            node,
            Node::Kern {
                kind: KernKind::Font,
                ..
            }
        )),
        "the fixture must exercise an internal font kern: {nodes:?}"
    );

    let hyphenated = super::super::hyphenation::test_hyphenated_word(&mut stores, &nodes);
    assert_eq!(
        hyphenated
            .iter()
            .filter(|node| matches!(node, Node::Disc { .. }))
            .count(),
        2,
        "both exception points must survive font-kern reconstitution: {hyphenated:?}"
    );
}

#[test]
fn hyphenation_preserves_the_font_kern_after_a_reconstituted_word() {
    let mut stores = Universe::new();
    let font = stores.current_font();
    for ch in "abcd".chars() {
        stores.set_lccode(ch, ch as u32);
    }
    stores.add_hyphenation_exception(ExceptionSpec {
        word: "abcd".to_owned(),
        positions: vec![2],
    });
    stores.set_int_param(IntParam::LEFT_HYPHEN_MIN, 1);
    stores.set_int_param(IntParam::RIGHT_HYPHEN_MIN, 1);
    stores.set_font_hyphen_char(font, i32::from(b'-'));
    let trailing = Scaled::from_raw(-54_614);
    let nodes = [
        Node::Char {
            font,
            ch: 'a',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
        Node::Char {
            font,
            ch: 'b',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
        Node::Char {
            font,
            ch: 'c',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
        Node::Char {
            font,
            ch: 'd',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
        Node::Kern {
            amount: trailing,
            kind: KernKind::Font,
        },
    ];

    let hyphenated = super::super::hyphenation::test_hyphenated_word(&mut stores, &nodes);

    assert!(
        matches!(hyphenated.last(), Some(Node::Kern { amount, kind: KernKind::Font }) if *amount == trailing)
    );
}

#[test]
fn hyphenation_does_not_repeat_a_left_boundary_kern() {
    let mut stores = Universe::new();
    let font = stores.current_font();
    stores.set_lccode('A', 'a' as u32);
    let nodes = [
        Node::Kern {
            amount: Scaled::from_raw(-65537),
            kind: KernKind::Font,
        },
        Node::Char {
            font,
            ch: 'A',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
    ];

    let hyphenated = super::super::hyphenation::test_hyphenated_word(&mut stores, &nodes);

    assert!(matches!(
        hyphenated.as_slice(),
        [
            Node::Kern {
                kind: KernKind::Font,
                ..
            },
            Node::Char { ch: 'A', .. }
        ]
    ));
}

#[test]
fn discretionary_absorbs_font_kern_across_hyphenated_line_boundary() {
    const CMR10: &[u8] = include_bytes!("../../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    let mut stores = Universe::with_world(tex_state::World::memory());
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("cmr10.tfm", CMR10.to_vec())
        .expect("seed cmr10");
    let mut input = InputStack::new(MemoryInput::new("\\font\\f=cmr10 \\relax \\f"));
    crate::Executor::new()
        .run(&mut input, &mut stores)
        .expect("font selection should execute");
    stores.add_hyphenation_exception(ExceptionSpec {
        word: "sentence".to_owned(),
        positions: vec![3],
    });
    let font = stores.current_font();
    stores.set_font_hyphen_char(font, i32::from(b'-'));
    let pending: Vec<_> = "sentence"
        .chars()
        .map(|ch| PendingHChar {
            font,
            ch,
            origin: tex_state::token::OriginId::UNKNOWN,
        })
        .collect();
    let nodes = reconstitute(&mut stores, &pending, false, false);

    let hyphenated = super::super::hyphenation::test_hyphenated_word(&mut stores, &nodes);
    let disc_index = hyphenated
        .iter()
        .position(|node| matches!(node, Node::Disc { .. }))
        .expect("sentence exception should insert a discretionary");
    let Node::Disc { replace, .. } = &hyphenated[disc_index] else {
        unreachable!()
    };

    assert!(matches!(
        stores.nodes(*replace).testing_decoded(),
        [Node::Kern {
            kind: KernKind::Font,
            ..
        }]
    ));
    assert!(!matches!(
        hyphenated.get(disc_index + 1),
        Some(Node::Kern {
            kind: KernKind::Font,
            ..
        })
    ));
}
