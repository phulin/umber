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
