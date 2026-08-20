use tex_state::Universe;
use tex_state::glue::Order;
use tex_state::node::{BoxLr, BoxNode, BoxNodeFields, Node, Sign};
use tex_state::node_arena::PageListId;
use tex_state::scaled::{GlueSetRatio, Scaled};

fn boxed_penalty(universe: &mut Universe, penalty: i32) -> PageListId {
    let children = universe.publish_page_nodes(&[Node::Penalty(penalty)]);
    universe.publish_page_nodes(&[Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        box_lr: BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }))])
}

fn boxed_penalty_value(universe: &Universe, root: PageListId) -> i32 {
    let root = universe
        .page_node_list(root)
        .expect("page root remains readable");
    let Some(Node::HList(box_node)) = root.nodes().first() else {
        panic!("root must contain one hbox")
    };
    let children = universe
        .page_node_list(box_node.children)
        .expect("box child remains readable");
    let Some(Node::Penalty(value)) = children.nodes().first() else {
        panic!("hbox must contain one penalty")
    };
    *value
}

fn box_register_penalty(universe: &mut Universe) -> i32 {
    let root = universe
        .copy_box_to_page(0)
        .expect("box register 0 remains populated");
    boxed_penalty_value(universe, root)
}

#[test]
fn page_rollback_and_durable_promotion_preserve_box_values() {
    let mut universe = Universe::default();
    let baseline = boxed_penalty(&mut universe, 10);
    universe.assign_page_box_local(0, baseline);

    let rollback = universe.snapshot();
    let replacement = boxed_penalty(&mut universe, 20);
    universe.assign_page_box_local(0, replacement);
    assert_eq!(box_register_penalty(&mut universe), 20);
    universe.rollback(&rollback);
    assert_eq!(box_register_penalty(&mut universe), 10);

    let cursor = universe.page_node_cursor();
    let rejected = boxed_penalty(&mut universe, 40);
    assert_eq!(boxed_penalty_value(&universe, rejected), 40);
    universe
        .truncate_page_nodes(cursor)
        .expect("speculative page suffix rolls back");
    assert!(universe.page_node_list(rejected).is_err());
    assert_eq!(box_register_penalty(&mut universe), 10);
}

#[test]
fn durable_box_survives_format_round_trip() {
    let mut universe = Universe::default();
    let root = boxed_penalty(&mut universe, 70);
    universe.assign_page_box_global(0, root);

    let format = universe.dump_format().expect("box graph format dumps");
    let mut restored = Universe::from_format(tex_state::World::memory(), &format)
        .expect("box graph format restores");
    assert_eq!(box_register_penalty(&mut restored), 70);
}
