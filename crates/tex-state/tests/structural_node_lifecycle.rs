use tex_state::env::AssignmentScope;
use tex_state::glue::Order;
use tex_state::interner::InternerBudget;
use tex_state::node::{BoxLr, BoxNode, BoxNodeFields, Node, Sign};
use tex_state::node_arena::PageListId;
use tex_state::scaled::{GlueSetRatio, Scaled};
use tex_state::{Universe, with_universe};

fn boxed_penalty<G>(universe: &mut Universe<G>, penalty: i32) -> PageListId {
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

fn boxed_penalty_value<G>(universe: &Universe<G>, root: PageListId) -> i32 {
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

fn box_register_penalty<G>(universe: &mut Universe<G>) -> i32 {
    let root = universe
        .copy_box_to_page(0)
        .expect("box register 0 remains populated");
    boxed_penalty_value(universe, root)
}

#[test]
fn checkpoint_restore_preserves_promoted_box_values_and_discards_suffixes() {
    let budget = InternerBudget::new(16, 16, 256).expect("budget");
    with_universe(budget, |universe| {
        let baseline = boxed_penalty(universe, 10);
        universe
            .assign_page_box(0, Some(baseline), AssignmentScope::Global)
            .expect("promote baseline box");
        let checkpoint = universe.state_checkpoint().expect("checkpoint");

        let replacement = boxed_penalty(universe, 20);
        universe
            .assign_page_box(0, Some(replacement), AssignmentScope::Global)
            .expect("promote replacement box");
        assert_eq!(box_register_penalty(universe), 20);

        universe
            .restore_state_checkpoint(&checkpoint)
            .expect("restore checkpoint");
        assert_eq!(box_register_penalty(universe), 10);
        assert!(universe.page_node_list(replacement).is_err());
    })
    .expect("fresh universe");
}
