use super::support::*;
use super::*;

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
