use super::support::*;
use super::*;
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
        .map(|ch| tex_state::node::Node::Char { font, ch })
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
fn paragraph_hyphenation_requires_a_valid_font_hyphen_character() {
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
        .map(|ch| tex_state::node::Node::Char { font, ch })
        .collect();

    stores.set_font_hyphen_char(font, -1, false);
    let disabled = crate::assignments::test_hyphenated_hlist(&mut stores, &word);
    stores.set_font_hyphen_char(font, i32::from(b'-'), false);
    let enabled = crate::assignments::test_hyphenated_hlist(&mut stores, &word);

    assert!(
        !disabled
            .iter()
            .any(|node| matches!(node, tex_state::node::Node::Disc { .. }))
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
        tex_state::node::Node::Char { font, ch: 'f' },
        tex_state::node::Node::Char { font, ch: 'f' },
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
    stores.set_font_hyphen_char(first, i32::from(b'-'), false);
    stores.set_font_hyphen_char(second, i32::from(b'-'), false);
    let glue = stores.glue_param(GlueParam::PAR_SKIP);
    let mut nodes = vec![Node::Glue {
        spec: glue,
        kind: GlueKind::Normal,
        leader: None,
    }];
    nodes.extend("abcd".chars().map(|ch| Node::Char { font: first, ch }));
    nodes.extend("efgh".chars().map(|ch| Node::Char { font: second, ch }));
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
    stores.set_font_hyphen_char(font, i32::from(b'-'), false);
    let par_fill = stores.glue_param(GlueParam::PAR_FILL_SKIP);
    let mut nodes = vec![
        Node::Char { font, ch: 'x' },
        Node::Glue {
            spec: stores.glue_param(GlueParam::SPACE_SKIP),
            kind: GlueKind::Normal,
            leader: None,
        },
    ];
    nodes.extend("abcdefgh".chars().map(|ch| Node::Char { font, ch }));
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
        left_skip: stores.glue(stores.glue_param(GlueParam::LEFT_SKIP)),
        right_skip: stores.glue(stores.glue_param(GlueParam::RIGHT_SKIP)),
        par_fill_skip: stores.glue(par_fill),
        shape: tex_typeset::linebreak::LineShape::natural(Scaled::from_raw(400 * Scaled::UNITY)),
    };
    let nodes_before = stores.testing_epoch_node_count();

    let _ = crate::assignments::test_break_hlist(&mut stores, &nodes, params);

    assert_eq!(stores.testing_epoch_node_count(), nodes_before);
}
