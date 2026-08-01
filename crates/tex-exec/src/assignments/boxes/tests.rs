use super::*;

#[test]
fn unboxing_removes_margin_kerns_without_mutating_source_or_reordering_material() {
    let mut stores = Universe::new_with_plain_catcodes();
    let font = tex_state::font::NULL_FONT;
    let source_nodes = [
        Node::Penalty(1),
        Node::MarginKern {
            amount: Scaled::from_raw(-Scaled::UNITY),
            side: tex_state::node::MarginKernSide::Left,
            font,
            ch: b'A',
        },
        Node::Penalty(2),
        Node::MarginKern {
            amount: Scaled::from_raw(-2 * Scaled::UNITY),
            side: tex_state::node::MarginKernSide::Right,
            font,
            ch: b'.',
        },
        Node::Penalty(3),
    ];
    let children = stores.freeze_node_list(&source_nodes);
    let mut nest = ModeNest::new();
    nest.push(Mode::RestrictedHorizontal)
        .expect("test mode push");
    let mut fuel = tex_command::CommandFuelLedger::default();

    append_unboxed(
        &mut nest,
        &mut stores,
        Some(UnboxSource::PinnedSurvivor(children)),
        fuel.fuel_mut(),
    )
    .expect("unbox succeeds");

    assert_eq!(
        nest.current_list().nodes(),
        &[Node::Penalty(1), Node::Penalty(2), Node::Penalty(3)],
        "non-margin nodes retain their original order",
    );
    assert_eq!(
        stores.nodes(children),
        source_nodes.as_slice(),
        "copy-style unboxing does not mutate the frozen source box",
    );
}
