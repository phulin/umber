use tex_state::Universe;
use tex_state::glue::Order;
use tex_state::node::{BoxLr, BoxNode, BoxNodeFields, Node, Sign};
use tex_state::node_arena::NodeListRef;
use tex_state::scaled::{GlueSetRatio, Scaled};

fn boxed_penalty(universe: &mut Universe, penalty: i32) -> NodeListRef {
    let children = universe.freeze_node_list(&[Node::Penalty(penalty)]);
    universe.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
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

fn boxed_penalty_value(root: &NodeListRef) -> i32 {
    let Some(tex_state::node_arena::NodeRef::HList(box_node)) = root.nodes().first() else {
        panic!("root must contain one hbox")
    };
    let children = root
        .resolve(box_node.children)
        .expect("box child coordinate belongs to its structural owner");
    let Some(tex_state::node_arena::NodeRef::Penalty(value)) = children.nodes().first() else {
        panic!("hbox must contain one penalty")
    };
    value
}

fn box_register_penalty(universe: &Universe) -> i32 {
    let root = universe
        .box_reg_ref(0)
        .expect("box register 0 must retain its structural root");
    boxed_penalty_value(&root)
}

#[test]
fn aggregate_transitions_use_structural_node_ownership() {
    let mut universe = Universe::default();
    let baseline = boxed_penalty(&mut universe, 10);
    universe.set_box_reg_ref(0, baseline);

    let rollback = universe.snapshot();
    let replacement = boxed_penalty(&mut universe, 20);
    universe.set_box_reg_ref(0, replacement);
    assert_eq!(box_register_penalty(&universe), 20);
    universe.rollback(&rollback);
    assert_eq!(box_register_penalty(&universe), 10);

    let retry = universe.snapshot_for_local_retry();
    let retried = boxed_penalty(&mut universe, 30);
    universe.set_box_reg_ref(0, retried);
    universe.rollback_local_retry_snapshot(retry);
    assert_eq!(box_register_penalty(&universe), 10);

    let committed_failure = universe.snapshot_for_local_retry();
    let partial = boxed_penalty(&mut universe, 40);
    universe.set_box_reg_ref(0, partial);
    universe.discard_local_retry_allocations(committed_failure);
    assert_eq!(box_register_penalty(&universe), 40);

    {
        let mut rejected = universe.begin_box_build();
        let _scratch = boxed_penalty(&mut rejected, 50);
    }
    assert_eq!(box_register_penalty(&universe), 40);

    let mut accepted = universe.begin_box_build();
    let installed = boxed_penalty(&mut accepted, 60);
    accepted.finish(0, Some(installed), false);
    assert_eq!(box_register_penalty(&universe), 60);
}

#[test]
fn generation_fork_clones_checkpoint_node_roots() {
    let mut universe = Universe::default();
    let checkpoint_root = boxed_penalty(&mut universe, 70);
    universe.set_box_reg_ref(0, checkpoint_root);
    let checkpoint = universe.snapshot();

    let later = boxed_penalty(&mut universe, 80);
    universe.set_box_reg_ref(0, later);
    let substrate = universe.freeze_generation();
    let fork = substrate
        .fork_at(&checkpoint)
        .expect("checkpoint belongs to the frozen generation substrate");

    assert_eq!(box_register_penalty(&fork), 70);
}
