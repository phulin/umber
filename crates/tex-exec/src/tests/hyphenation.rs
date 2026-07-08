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
        "\\font\\tenrm=cmr10 \\tenrm\\hsize=100pt \\patterns{a1ba}\\lefthyphenmin=1 \\righthyphenmin=1 \\pretolerance=-1 \\uchyph=0 Aba\\par \\uchyph=1 Aba\\par\\end",
    ));

    let mut executor = Executor::new();
    executor
        .run(&mut input, &mut stores)
        .expect("paragraph hyphenation executes");

    let hlists: Vec<_> = executor
        .nest()
        .current_list()
        .nodes()
        .iter()
        .filter_map(|node| match node {
            tex_state::node::Node::HList(box_node) => Some(box_node.children),
            _ => None,
        })
        .collect();
    let disc_counts: Vec<_> = hlists
        .iter()
        .map(|&list| {
            stores
                .nodes(list)
                .iter()
                .filter(|node| matches!(node, tex_state::node::Node::Disc { .. }))
                .count()
        })
        .collect();
    assert_eq!(disc_counts, vec![0, 1]);
}
