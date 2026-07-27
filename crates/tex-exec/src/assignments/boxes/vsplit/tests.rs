use super::*;
use tex_state::node::{BoxNode, BoxNodeFields, Sign};
use tex_state::node_arena::NodeRef;
use tex_state::scaled::GlueSetRatio;

fn sp(raw: i32) -> Scaled {
    Scaled::from_raw(raw)
}

#[test]
fn tex82_vsplit_marks_remainder_and_trivial_case_matrix() {
    let mut stores = Universe::new();
    assert!(
        split_vbox_register(&mut stores, 0, sp(10))
            .expect("void split")
            .is_none()
    );

    let children = stores.freeze_node_list(&[]);
    let hbox = stores.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
        width: sp(1),
        height: sp(4),
        depth: sp(0),
        shift: sp(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }))]);
    stores.set_box_reg(0, hbox);
    assert!(
        split_vbox_register(&mut stores, 0, sp(10))
            .expect("hbox recovery")
            .is_none()
    );
    let retained = stores
        .box_reg(0)
        .expect("wrong-kind source remains nonvoid");
    assert!(matches!(
        stores.nodes(retained).first(),
        Some(NodeRef::HList(_))
    ));
}
