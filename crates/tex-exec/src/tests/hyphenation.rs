use super::support::*;
use super::*;

fn pretolerance_memo_config() -> tex_state::PureMemoConfig {
    tex_state::PureMemoConfig {
        recording: tex_state::PureMemoRecordingPolicy {
            pretolerance: true,
            paragraphs: false,
            pages: false,
            shipouts: false,
        },
        ..tex_state::PureMemoConfig::default()
    }
}
use tex_state::node::{GlueKind, Node};
use tex_state::scaled::Scaled;

#[test]
fn patterns_and_exceptions_feed_showhyphens() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\patterns{a1ba t2e1st}\\hyphenation{tes-ting}\\lefthyphenmin=1 \\righthyphenmin=1 \\showhyphens{aba testing test}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("hyphenation primitives execute");

    let output = terminal_effect_text(&stores);
    assert!(output.contains("a-ba"));
    assert!(output.contains("tes-ting"));
    assert!(output.contains("te-st"));
}

#[test]
fn etex_saved_hyphen_codes_are_language_specific_and_survive_lccode_changes() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    install_etex_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\savinghyphcodes=1 \\language=1 \\lccode`A=`a \\patterns{a1ba} \
         \\lccode`A=`z \\lefthyphenmin=1 \\righthyphenmin=1 \
         \\showhyphens{Aba} \
         \\language=2 \\lccode`A=`x \\patterns{x1ba} \\lccode`A=`z \
         \\hyphenation{Ab-a} \\showhyphens{Aba} \
         \\language=1 \\showhyphens{Aba} \\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("saved hyphenation codes execute");

    let output = terminal_effect_text(&stores);
    assert_eq!(output.matches("a-ba").count(), 2, "{output}");
    assert!(output.contains("xb-a"), "{output}");
}

#[test]
fn showhyphens_honors_hyphen_minima() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\patterns{a1ba}\\righthyphenmin=1 \\lefthyphenmin=3 \\showhyphens{aba}\\lefthyphenmin=1 \\showhyphens{aba}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("hyphen minima execute");

    let output = terminal_effect_text(&stores);
    assert!(output.contains("\naba\n! OK."));
    assert!(output.contains("\na-ba\n! OK."));
}

#[test]
fn paragraph_hyphenation_honors_uchyph_for_uppercase_start() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\tenrm=cmr10 \\relax \\tenrm \\patterns{a1ba}\\lefthyphenmin=1 \\righthyphenmin=1 \\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("hyphenation setup executes");
    let font = stores.current_font();
    let word: Vec<_> = "Aba"
        .chars()
        .map(|ch| tex_state::node::Node::Char {
            font,
            ch,
            origin: tex_state::token::OriginId::UNKNOWN,
        })
        .collect();

    stores.set_int_param(IntParam::UC_HYPH, 0);
    let lowercase_only = crate::assignments::test_hyphenated_hlist(&mut stores, &word);
    stores.set_int_param(IntParam::UC_HYPH, 1);
    let uppercase_enabled = crate::assignments::test_hyphenated_hlist(&mut stores, &word);

    assert!(
        !lowercase_only
            .iter()
            .any(|node| matches!(node, tex_state::node::Node::Disc { .. }))
    );
    assert!(
        uppercase_enabled
            .iter()
            .any(|node| matches!(node, tex_state::node::Node::Disc { .. }))
    );
}

#[test]
fn paragraph_hyphenation_requires_an_in_range_hyphen_and_omits_a_missing_glyph() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\tenrm=cmr10 \\relax \\tenrm \\patterns{a1ba}\\lefthyphenmin=1 \\righthyphenmin=1 \\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("hyphenation setup executes");
    let font = stores.current_font();
    let word: Vec<_> = "aba"
        .chars()
        .map(|ch| tex_state::node::Node::Char {
            font,
            ch,
            origin: tex_state::token::OriginId::UNKNOWN,
        })
        .collect();

    stores.set_font_hyphen_char(font, -1);
    let disabled = crate::assignments::test_hyphenated_hlist(&mut stores, &word);
    let missing_code = (0u8..=u8::MAX)
        .find(|&code| !stores.font_char_exists(font, code))
        .expect("test font has an in-range missing character");
    stores.set_font_hyphen_char(font, i32::from(missing_code));
    let missing_glyph = crate::assignments::test_hyphenated_hlist(&mut stores, &word);
    stores.set_font_hyphen_char(font, i32::from(b'-'));
    let enabled = crate::assignments::test_hyphenated_hlist(&mut stores, &word);

    assert!(
        !disabled
            .iter()
            .any(|node| matches!(node, tex_state::node::Node::Disc { .. }))
    );
    assert!(
        missing_glyph.iter().any(|node| {
            matches!(node, Node::Disc { pre, .. } if stores.nodes(*pre).is_empty())
        }),
        "TeX retains the discretionary but new_character returns null"
    );
    assert!(
        enabled
            .iter()
            .any(|node| matches!(node, tex_state::node::Node::Disc { .. }))
    );
}

#[test]
fn paragraph_hyphenation_preserves_existing_chars_when_no_break_is_found() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\tenrm=cmr10 \\relax \\tenrm \\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("font setup executes");
    let font = stores.current_font();
    let word = vec![
        tex_state::node::Node::Char {
            font,
            ch: 'f',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
        tex_state::node::Node::Char {
            font,
            ch: 'f',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
    ];

    let unchanged = crate::assignments::test_hyphenated_hlist(&mut stores, &word);
    assert_eq!(
        unchanged, word,
        "no-break hyphenation must not create an ff ligature"
    );
}

#[test]
fn paragraph_hyphenation_stops_at_a_font_change() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\a=cmr10 \\font\\b=cmmi10 \\relax \\hyphenation{ab-cdefgh} \\end",
    ));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect("mixed-font hyphenation setup executes");
    let first = font_meaning(&stores, "a");
    let second = font_meaning(&stores, "b");
    stores.set_font_hyphen_char(first, i32::from(b'-'));
    stores.set_font_hyphen_char(second, i32::from(b'-'));
    let glue = stores.glue_param(GlueParam::PAR_SKIP);
    let mut nodes = vec![Node::Glue {
        spec: glue,
        kind: GlueKind::Normal,
        leader: None,
    }];
    nodes.extend("abcd".chars().map(|ch| Node::Char {
        font: first,
        ch,
        origin: tex_state::token::OriginId::UNKNOWN,
    }));
    nodes.extend("efgh".chars().map(|ch| Node::Char {
        font: second,
        ch,
        origin: tex_state::token::OriginId::UNKNOWN,
    }));
    nodes.push(Node::Glue {
        spec: glue,
        kind: GlueKind::Normal,
        leader: None,
    });

    let hyphenated = crate::assignments::test_hyphenated_hlist(&mut stores, &nodes);

    assert!(
        !hyphenated
            .iter()
            .any(|node| matches!(node, Node::Disc { .. })),
        "TeX82 sections 897 and 899 stop the word at a font change"
    );
}

#[test]
fn successful_pretolerance_does_not_allocate_hyphenation_nodes() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\tenrm=cmr10 \\relax \\tenrm \\hyphenation{ab-cdefgh} \\end",
    ));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect("pretolerance allocation setup executes");
    let font = stores.current_font();
    stores.set_font_hyphen_char(font, i32::from(b'-'));
    let par_fill = stores.glue_param(GlueParam::PAR_FILL_SKIP);
    let mut nodes = vec![
        Node::Char {
            font,
            ch: 'x',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
        Node::Glue {
            spec: stores.glue_param(GlueParam::SPACE_SKIP),
            kind: GlueKind::Normal,
            leader: None,
        },
    ];
    nodes.extend("abcdefgh".chars().map(|ch| Node::Char {
        font,
        ch,
        origin: tex_state::token::OriginId::UNKNOWN,
    }));
    nodes.push(Node::Penalty(10_000));
    nodes.push(Node::Glue {
        spec: par_fill,
        kind: GlueKind::ParFillSkip,
        leader: None,
    });
    let params = tex_typeset::linebreak::LineBreakParams {
        pretolerance: 10_000,
        tolerance: 10_000,
        line_penalty: 0,
        hyphen_penalty: 50,
        ex_hyphen_penalty: 50,
        adj_demerits: 0,
        double_hyphen_demerits: 0,
        final_hyphen_demerits: 0,
        emergency_stretch: Scaled::from_raw(0),
        looseness: 0,
        last_line_fit: 0,
        pdf_adjust_spacing: 0,
        pdf_protrude_chars: 0,
        left_skip: stores.glue(stores.glue_param(GlueParam::LEFT_SKIP)),
        right_skip: stores.glue(stores.glue_param(GlueParam::RIGHT_SKIP)),
        par_fill_skip: stores.glue(par_fill),
        shape: tex_typeset::linebreak::LineShape::natural(Scaled::from_raw(400 * Scaled::UNITY)),
    };
    let nodes_before = stores.testing_epoch_node_count();

    let _ = crate::assignments::test_break_hlist(&mut stores, nodes, params);

    assert_eq!(stores.testing_epoch_node_count(), nodes_before);
}

#[test]
fn pretolerance_memo_hits_and_every_explicit_parameter_changes_its_strong_key() {
    use tex_state::glue::GlueSpec;
    use tex_typeset::linebreak::{LineShape, LineShapeEntry, ParagraphShape};

    let mut stores = Universe::new();
    stores.enable_pure_memo(pretolerance_memo_config());
    let nodes = vec![
        Node::Rule {
            width: Some(Scaled::from_raw(10)),
            height: Some(Scaled::from_raw(5)),
            depth: Some(Scaled::from_raw(0)),
        },
        Node::Penalty(-10_000),
    ];
    let base = tex_typeset::linebreak::LineBreakParams {
        pdf_adjust_spacing: 0,
        pdf_protrude_chars: 0,
        pretolerance: 10_000,
        tolerance: 9_999,
        line_penalty: 10,
        hyphen_penalty: 50,
        ex_hyphen_penalty: 51,
        adj_demerits: 52,
        double_hyphen_demerits: 53,
        final_hyphen_demerits: 54,
        emergency_stretch: Scaled::from_raw(55),
        looseness: 0,
        last_line_fit: 56,
        left_skip: GlueSpec::ZERO,
        right_skip: GlueSpec::ZERO,
        par_fill_skip: GlueSpec::ZERO,
        shape: LineShape::natural(Scaled::from_raw(1_000)),
    };

    let first = crate::assignments::test_break_hlist(&mut stores, nodes.clone(), base.clone());
    let second = crate::assignments::test_break_hlist(&mut stores, nodes.clone(), base.clone());
    assert_eq!(first.breaks, second.breaks);
    assert_eq!(stores.pure_memo_stats().hits, 1);

    let base_key = crate::assignments::test_pretolerance_memo_key(&stores, &nodes, &base);
    let mut variants = Vec::new();
    macro_rules! changed {
        ($field:ident, $value:expr) => {{
            let mut params = base.clone();
            params.$field = $value;
            variants.push(params);
        }};
    }
    changed!(pretolerance, 9_998);
    changed!(tolerance, 9_997);
    changed!(line_penalty, 11);
    changed!(hyphen_penalty, 60);
    changed!(ex_hyphen_penalty, 61);
    changed!(adj_demerits, 62);
    changed!(double_hyphen_demerits, 63);
    changed!(final_hyphen_demerits, 64);
    changed!(emergency_stretch, Scaled::from_raw(65));
    changed!(looseness, 1);
    changed!(last_line_fit, 66);
    changed!(
        left_skip,
        GlueSpec {
            width: Scaled::from_raw(1),
            ..GlueSpec::ZERO
        }
    );
    changed!(
        right_skip,
        GlueSpec {
            stretch: Scaled::from_raw(1),
            ..GlueSpec::ZERO
        }
    );
    changed!(
        par_fill_skip,
        GlueSpec {
            shrink: Scaled::from_raw(1),
            ..GlueSpec::ZERO
        }
    );
    let mut shape = base.shape.clone();
    shape.hang_indent = Scaled::from_raw(1);
    variants.push(tex_typeset::linebreak::LineBreakParams {
        shape,
        ..base.clone()
    });
    let mut shape = base.shape.clone();
    shape.hang_after = 2;
    variants.push(tex_typeset::linebreak::LineBreakParams {
        shape,
        ..base.clone()
    });
    let mut shape = base.shape.clone();
    shape.line_offset = 3;
    variants.push(tex_typeset::linebreak::LineBreakParams {
        shape,
        ..base.clone()
    });
    let mut shape = base.shape.clone();
    shape.parshape = Some(ParagraphShape {
        lines: vec![LineShapeEntry {
            indent: Scaled::from_raw(4),
            width: Scaled::from_raw(900),
        }],
    });
    variants.push(tex_typeset::linebreak::LineBreakParams {
        shape,
        ..base.clone()
    });

    for variant in variants {
        assert_ne!(
            crate::assignments::test_pretolerance_memo_key(&stores, &nodes, &variant),
            base_key
        );
    }
}

#[test]
fn malformed_pretolerance_entry_is_rejected_and_recomputed() {
    use tex_state::{DetachedMemoValue, DetachedPureKernelPlan, PureMemoStats};

    let mut stores = Universe::new();
    stores.enable_pure_memo(pretolerance_memo_config());
    let nodes = vec![Node::Penalty(-10_000)];
    let params = tex_typeset::linebreak::LineBreakParams {
        pdf_adjust_spacing: 0,
        pdf_protrude_chars: 0,
        pretolerance: 10_000,
        tolerance: 10_000,
        line_penalty: 0,
        hyphen_penalty: 0,
        ex_hyphen_penalty: 0,
        adj_demerits: 0,
        double_hyphen_demerits: 0,
        final_hyphen_demerits: 0,
        emergency_stretch: Scaled::from_raw(0),
        looseness: 0,
        last_line_fit: 0,
        left_skip: tex_state::glue::GlueSpec::ZERO,
        right_skip: tex_state::glue::GlueSpec::ZERO,
        par_fill_skip: tex_state::glue::GlueSpec::ZERO,
        shape: tex_typeset::linebreak::LineShape::natural(Scaled::from_raw(1_000)),
    };
    let key = crate::assignments::test_pretolerance_memo_key(&stores, &nodes, &params);
    let malformed = DetachedMemoValue::from_pure_kernel_plan(&DetachedPureKernelPlan {
        kernel: "line-break-pretolerance".to_owned(),
        plan_schema: 1,
        payload: vec![1, 2, 3],
    })
    .expect("malformed plan envelope");
    stores.insert_pure_memo(key, malformed);

    let result = crate::assignments::test_break_hlist(&mut stores, nodes, params);
    assert!(!result.breaks.is_empty());
    let PureMemoStats { malformed, .. } = stores.pure_memo_stats();
    assert_eq!(malformed, 1);
}

#[test]
fn enabled_pretolerance_memo_preserves_end_to_end_state_effects_and_dvi() {
    fn run(
        enabled: bool,
    ) -> (
        ExecutionStats,
        u64,
        Vec<EffectRecord>,
        tex_state::PureMemoStats,
    ) {
        let mut stores = stores_with_fonts();
        tex_expand::install_expandable_primitives(&mut stores);
        install_unexpandable_primitives(&mut stores);
        if enabled {
            stores.enable_pure_memo(pretolerance_memo_config());
        }
        let source = r"\hsize=20pt \pretolerance=10000
            identical paragraph text\par
            \prevgraf=0 \interlinepenalty=111 \clubpenalty=222 \widowpenalty=333
            \hbadness=0 \hfuzz=1pt \mag=1200
            identical paragraph text\par
            \prevgraf=0 \language=7 \lefthyphenmin=1 \righthyphenmin=1
            identical paragraph text\par
            \vfill\eject\end";
        let mut input = InputStack::new(MemoryInput::new(source));
        let stats = Executor::new()
            .run(&mut input, &mut stores)
            .expect("memo parity program");
        let hash = stores.snapshot().state_hash();
        let effects = stores.world().effect_records().to_vec();
        let memo = stores.pure_memo_stats();
        (stats, hash, effects, memo)
    }

    let (cold_stats, cold_hash, cold_effects, _) = run(false);
    let (memo_stats, memo_hash, memo_effects, memo) = run(true);
    assert_eq!(memo_stats, cold_stats);
    assert_eq!(memo_hash, cold_hash);
    assert_eq!(memo_effects, cold_effects);
    assert!(memo.hits >= 1, "expected the repeated paragraph to hit");
    assert!(
        memo.misses >= 2,
        "the initial and language-mutated paragraphs must miss"
    );
}

#[test]
fn direct_batch_paragraphs_do_not_build_incremental_history() {
    fn run(enabled: bool) -> (Vec<u8>, u64, tex_state::PureMemoStats) {
        let mut stores = Universe::with_world(tex_state::World::memory());
        tex_expand::install_expandable_primitives(&mut stores);
        install_unexpandable_primitives(&mut stores);
        if enabled {
            stores.enable_pure_memo(tex_state::PureMemoConfig::default());
            stores.enable_paragraph_memo();
        }
        let source = "\\font\\tenrm=cmr10 \\tenrm repeated literal paragraph text\\par\nrepeated literal paragraph text\\par\nrepeated literal paragraph text\\par\n\\vfill\\eject\\end";
        let mut input = InputStack::new(MemoryInput::new(source));
        let stats = Executor::new()
            .run(&mut input, &mut stores)
            .expect("literal paragraph program");
        let mut dvi = tex_out::dvi::DviStreamWriter::new(Vec::new());
        for plan in &stats.dvi_pages {
            dvi.write_page_plan(plan).expect("DVI page");
        }
        let bytes = dvi.finish().expect("DVI finish");
        let hash = stores.snapshot().state_hash();
        (bytes, hash, stores.pure_memo_stats())
    }

    let (cold_dvi, cold_hash, _) = run(false);
    let (memo_dvi, memo_hash, stats) = run(true);
    assert_eq!(memo_dvi, cold_dvi);
    assert_eq!(memo_hash, cold_hash);
    assert_eq!(stats.paragraph_hits, 0, "{stats:?}");
    assert_eq!(stats.paragraph_inserts, 0, "{stats:?}");
    assert_eq!(stats.paragraph_commands_skipped, 0);
    assert_eq!(stats.paragraph_eligible_regions, 0, "{stats:?}");
    assert_eq!(
        stats.paragraph_opportunities.published.regions, 0,
        "{stats:?}"
    );
}

#[test]
fn paragraph_front_end_replays_validated_count_mutations() {
    fn run(enabled: bool) -> (i32, i32, Vec<u8>, tex_state::PureMemoStats) {
        let mut stores = Universe::with_world(tex_state::World::memory());
        tex_expand::install_expandable_primitives(&mut stores);
        install_unexpandable_primitives(&mut stores);
        if enabled {
            stores.enable_pure_memo(tex_state::PureMemoConfig::default());
            stores.enable_paragraph_memo();
        }
        let paragraph =
            "\\count5=41 \\global\\count6=9 \\language=7 stateful paragraph text\\par\n";
        let source = format!("{paragraph}{paragraph}{paragraph}{paragraph}\\vfill\\eject\\end");
        let mut input = InputStack::new(MemoryInput::new(source));
        let stats = Executor::new()
            .run(&mut input, &mut stores)
            .expect("stateful paragraph program");
        let mut dvi = tex_out::dvi::DviStreamWriter::new(Vec::new());
        for plan in &stats.dvi_pages {
            dvi.write_page_plan(plan).expect("DVI page");
        }
        (
            stores.count(5),
            stores.count(6),
            dvi.finish().expect("DVI finish"),
            stores.pure_memo_stats(),
        )
    }

    let (cold_local, cold_global, cold_dvi, _) = run(false);
    let (memo_local, memo_global, memo_dvi, stats) = run(true);
    assert_eq!((memo_local, memo_global), (cold_local, cold_global));
    assert_eq!(memo_dvi, cold_dvi);
    assert_eq!(stats.paragraph_hits, 0, "{stats:?}");
    assert_eq!(stats.paragraph_mutations_replayed, 0, "{stats:?}");
    assert_eq!(stats.paragraph_eligible_regions, 0, "{stats:?}");
}

#[test]
fn grouped_paragraph_redo_preserves_local_and_global_assignment_scope() {
    let mut stores = Universe::with_world(tex_state::World::memory());
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    stores.enable_pure_memo(tex_state::PureMemoConfig::default());
    stores.enable_paragraph_memo();
    let local = "{\\count5=41 grouped local text\\par}\n";
    let global = "{\\global\\count6=9 grouped global text\\par}\n";
    let source = format!("{local}{local}{local}{global}{global}{global}\\vfill\\eject\\end");
    let mut input = InputStack::new(MemoryInput::new(source));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect("grouped stateful paragraphs");
    assert_eq!(
        stores.count(5),
        0,
        "local replay must unwind with its group"
    );
    assert_eq!(stores.count(6), 9, "global replay must survive group exit");
    let stats = stores.pure_memo_stats();
    assert_eq!(stats.paragraph_hits, 0, "{stats:?}");
    assert_eq!(stats.paragraph_mutations_replayed, 0, "{stats:?}");
    assert_eq!(stats.paragraph_eligible_regions, 0, "{stats:?}");
}

#[test]
fn effectful_paragraph_commands_remain_replay_barriers() {
    let mut stores = Universe::with_world(tex_state::World::memory());
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    stores.enable_pure_memo(tex_state::PureMemoConfig::default());
    stores.enable_paragraph_memo();
    let paragraph = "\\message{visible}\\advance\\count7 by1 effectful paragraph text\\par\n";
    let source = format!("{paragraph}{paragraph}{paragraph}\\vfill\\eject\\end");
    let mut input = InputStack::new(MemoryInput::new(source));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect("effectful paragraphs execute normally");
    assert_eq!(stores.count(7), 3);
    let stats = stores.pure_memo_stats();
    assert_eq!(stats.paragraph_hits, 0);
    assert_eq!(stats.paragraph_eligible_regions, 0, "{stats:?}");
}

#[test]
fn deterministic_message_effects_replay_in_original_order() {
    let mut stores = Universe::with_world(tex_state::World::memory());
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    stores.enable_pure_memo(tex_state::PureMemoConfig::default());
    stores.enable_paragraph_memo();
    let paragraph = "\\message{visible}message paragraph text\\par\n";
    let source = format!("{paragraph}{paragraph}{paragraph}");
    let mut input = InputStack::new(MemoryInput::new(source));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect("message paragraphs");
    assert_eq!(terminal_effect_text(&stores).matches("visible").count(), 3);
    let stats = stores.pure_memo_stats();
    assert_eq!(stats.paragraph_hits, 0, "{stats:?}");
    assert_eq!(stats.paragraph_eligible_regions, 0, "{stats:?}");
}

#[test]
fn direct_batch_executor_does_not_publish_paragraph_regions() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    stores.enable_pure_memo(tex_state::PureMemoConfig::default());
    stores.enable_paragraph_memo();
    let source = r"\font\tenrm=cmr10 \tenrm
        \def\body{office \accent18 a\discretionary{-}{}{x}}
        \everypar{\message{EP}}
        {\csname body\endcsname \mark{m}\insert0{\hbox{x}}\vadjust{\kern1pt}\par}
        \end";
    let mut input = InputStack::new(MemoryInput::new(source));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect("recordable macro paragraph");

    let regions = stores.recorded_paragraphs();
    assert!(regions.is_empty(), "{regions:#?}");
}

#[test]
fn direct_batch_executor_does_not_arm_incremental_barrier_tracking() {
    let run = |body: &str| {
        let mut stores = stores_with_fonts();
        tex_expand::install_expandable_primitives(&mut stores);
        tex_expand::install_etex_expandable_primitives(&mut stores);
        install_unexpandable_primitives(&mut stores);
        stores.enable_pure_memo(tex_state::PureMemoConfig::default());
        stores.enable_paragraph_memo();
        let mut input = InputStack::new(MemoryInput::new(format!(
            "\\font\\tenrm=cmr10 \\tenrm {body}\\end"
        )));
        Executor::new()
            .run(&mut input, &mut stores)
            .expect("barrier paragraph executes cold");
        stores.pure_memo_stats()
    };
    let display = run("display text$$x$$after\\par");
    assert_eq!(display.paragraph_display_math_barriers, 0, "{display:?}");
    let scantokens = run("scanned \\scantokens{more} text\\par");
    assert_eq!(
        scantokens.paragraph_scantokens_barriers, 0,
        "{scantokens:?}"
    );
}

#[test]
fn randomized_pretolerance_cache_differential_matches_disabled_kernel() {
    let mut disabled = Universe::new();
    let glue = disabled.intern_glue(tex_state::glue::GlueSpec {
        width: Scaled::from_raw(4),
        stretch: Scaled::from_raw(2),
        ..tex_state::glue::GlueSpec::ZERO
    });
    let mut enabled = disabled.clone();
    enabled.enable_pure_memo(pretolerance_memo_config());
    let mut seed = 0x9e37_79b9_u32;

    for case in 0..128 {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let mut nodes = Vec::new();
        for index in 0..(8 + seed as usize % 40) {
            if index % 2 == 0 {
                nodes.push(Node::Rule {
                    width: Some(Scaled::from_raw(1 + ((seed >> (index % 16)) as i32 & 31))),
                    height: Some(Scaled::from_raw(1)),
                    depth: Some(Scaled::from_raw(0)),
                });
            } else {
                nodes.push(Node::Glue {
                    spec: glue,
                    kind: GlueKind::Normal,
                    leader: None,
                });
            }
        }
        nodes.push(Node::Penalty(-10_000));
        let params = tex_typeset::linebreak::LineBreakParams {
            pdf_adjust_spacing: 0,
            pdf_protrude_chars: 0,
            pretolerance: 10_000,
            tolerance: 1_000 + (seed % 9_000) as i32,
            line_penalty: (seed % 100) as i32,
            hyphen_penalty: 50,
            ex_hyphen_penalty: 50,
            adj_demerits: (seed % 1_000) as i32,
            double_hyphen_demerits: 1_000,
            final_hyphen_demerits: 500,
            emergency_stretch: Scaled::from_raw((seed % 20) as i32),
            looseness: 0,
            last_line_fit: 0,
            left_skip: tex_state::glue::GlueSpec::ZERO,
            right_skip: tex_state::glue::GlueSpec::ZERO,
            par_fill_skip: tex_state::glue::GlueSpec::ZERO,
            shape: tex_typeset::linebreak::LineShape::natural(Scaled::from_raw(
                30 + (seed % 300) as i32,
            )),
        };
        let expected = crate::cached_pretolerance_plan(&mut disabled, &nodes, &params);
        let actual = crate::cached_pretolerance_plan(&mut enabled, &nodes, &params);
        assert_eq!(actual, expected, "random differential case {case}");
        assert_eq!(
            crate::cached_pretolerance_plan(&mut enabled, &nodes, &params),
            expected,
            "random cached differential case {case}"
        );
    }
    assert!(enabled.pure_memo_stats().hits >= 128);
}
