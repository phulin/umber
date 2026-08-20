use super::*;

#[test]
fn tex82_vert_break_cost_depth_and_tie_matrix() {
    let mut universe = Universe::new();
    let nodes = vec![
        hbox(&mut universe, 8, 6),
        Node::Penalty(0),
        hbox(&mut universe, 4, 0),
        Node::Penalty(EJECT_PENALTY),
    ];
    let split = vert_break(&universe, &nodes, sp(10), sp(2)).expect("vertical break");
    assert_eq!(split.break_index, Some(1));
    assert_eq!(split.best_height_plus_depth, sp(14));

    let tied = vec![
        hbox(&mut universe, 10, 0),
        Node::Penalty(0),
        Node::Penalty(0),
    ];
    let split = vert_break(&universe, &tied, sp(10), sp(0)).expect("tie break");
    assert_eq!(
        split.break_index, None,
        "the artificial forced end wins for a whole fitting list"
    );
}

#[test]
fn vertical_break_ignores_perpendicular_box_overflow() {
    let mut universe = Universe::new();
    let children = universe.publish_page_nodes(&[]);
    let nodes = vec![Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(i32::MAX),
        height: sp(0),
        depth: sp(0),
        shift: sp(1),
        box_lr: tex_state::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }))];

    let split = vert_break(&universe, &nodes, sp(0), sp(0)).expect("vertical break");

    assert_eq!(split.best_height_plus_depth, sp(0));
}

#[test]
fn vertical_break_preserves_height_then_depth_addition_order() {
    let mut universe = Universe::new();
    let nodes = vec![
        hbox(&mut universe, -1, i32::MAX),
        Node::Kern {
            amount: sp(1),
            kind: KernKind::Explicit,
        },
    ];

    let split = vert_break(
        &universe,
        &nodes,
        Scaled::from_raw(i32::MAX),
        Scaled::from_raw(i32::MAX),
    )
    .expect("vertical break");

    assert_eq!(split.best_height_plus_depth, Scaled::from_raw(i32::MAX));
}
