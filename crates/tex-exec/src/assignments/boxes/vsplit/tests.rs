use super::*;
use tex_state::env::banks::{DimenParam, GlueParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::node::{BoxNode, BoxNodeFields, GlueKind, Sign};
use tex_state::node_arena::NodeRef;
use tex_state::scaled::GlueSetRatio;

fn sp(raw: i32) -> Scaled {
    Scaled::from_raw(raw)
}

#[test]
fn etex_every_vsplit_replaces_stale_saved_discards() {
    let mut stores = crate::test_harness::universe();

    for index in [0, 1] {
        stores.set_split_discards(vec![Node::Penalty(100 + i32::from(index))]);
        if index == 1 {
            let children = stores.freeze_node_list(&[]);
            let hbox = stores.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
                width: sp(0),
                height: sp(0),
                depth: sp(0),
                shift: sp(0),
                box_lr: tex_state::node::BoxLr::Normal,
                glue_set: GlueSetRatio::ZERO,
                glue_sign: Sign::Normal,
                glue_order: Order::Normal,
                children,
            }))]);
            stores.set_box_reg(index, hbox);
        }

        assert!(
            split_vbox_register(&mut stores, index, sp(0))
                .expect("recoverable split")
                .is_none()
        );
        assert!(
            stores.split_discards().is_empty(),
            "split of register {index} retained stale discards"
        );
    }
}

#[test]
fn tex82_vsplit_marks_remainder_and_trivial_case_matrix() {
    let mut stores = crate::test_harness::universe();
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
        box_lr: tex_state::node::BoxLr::Normal,
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

    // TeX.web §§977--979: split at the middle glue, package the prefix to the
    // requested height/depth, then discard the remainder prefix and insert an
    // adjusted split_top_skip before repacking the source register naturally.
    let split_top_skip = stores.intern_glue(GlueSpec {
        width: sp(7),
        stretch: sp(0),
        stretch_order: Order::Normal,
        shrink: sp(0),
        shrink_order: Order::Normal,
    });
    stores.set_glue_param(GlueParam::SPLIT_TOP_SKIP, split_top_skip);
    stores.set_dimen_param(DimenParam::SPLIT_MAX_DEPTH, sp(1));
    let break_glue = stores.intern_glue(GlueSpec {
        width: sp(6),
        stretch: sp(0),
        stretch_order: Order::Normal,
        shrink: sp(0),
        shrink_order: Order::Normal,
    });
    let source_children = stores.freeze_node_list(&[
        Node::Rule {
            width: Some(sp(2)),
            height: Some(sp(8)),
            depth: Some(sp(2)),
        },
        Node::Glue {
            spec: break_glue,
            kind: GlueKind::Normal,
            leader: None,
        },
        Node::Penalty(50),
        Node::Rule {
            width: Some(sp(3)),
            height: Some(sp(3)),
            depth: Some(sp(2)),
        },
    ]);
    let source = stores.freeze_node_list(&[Node::VList(BoxNode::new(BoxNodeFields {
        width: sp(3),
        height: sp(19),
        depth: sp(2),
        shift: sp(0),
        box_lr: tex_state::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: source_children,
    }))]);
    stores.set_box_reg(7, source);

    let Node::VList(extracted) = split_vbox_register(&mut stores, 7, sp(10))
        .expect("vsplit succeeds")
        .expect("vsplit returns a box")
    else {
        panic!("vsplit must return a vbox");
    };
    assert_eq!((extracted.height, extracted.depth), (sp(10), sp(1)));
    let extracted_nodes = stores.nodes(extracted.children);
    assert_eq!(extracted_nodes.len(), 1);
    assert!(matches!(
        extracted_nodes.first(),
        Some(NodeRef::Rule { .. })
    ));

    let remainder = stores.box_reg(7).expect("remainder stays in the register");
    let remainder_box = stores.nodes(remainder);
    assert_eq!(remainder_box.len(), 1);
    let Some(NodeRef::VList(remainder)) = remainder_box.first() else {
        panic!("remainder register must contain one vbox");
    };
    assert_eq!((remainder.height, remainder.depth), (sp(7), sp(2)));
    let remainder_nodes = stores.nodes(remainder.children);
    assert_eq!(remainder_nodes.len(), 2);
    let mut remainder_nodes = remainder_nodes.iter();
    let Some(NodeRef::Glue { spec, kind, .. }) = remainder_nodes.next() else {
        panic!("adjusted split-top skip must prefix the remainder");
    };
    assert_eq!(kind, GlueKind::SplitTopSkip);
    assert_eq!(stores.glue(spec).width, sp(4));
    assert!(matches!(remainder_nodes.next(), Some(NodeRef::Rule { .. })));
}
