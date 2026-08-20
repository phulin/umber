use super::*;
use tex_state::glue::Order;
use tex_state::node::{BoxNode, BoxNodeFields, KernKind, Sign};
use tex_state::scaled::GlueSetRatio;

fn sp(raw: i32) -> Scaled {
    Scaled::from_raw(raw)
}

#[test]
fn tex82_prune_page_top_prefix_and_split_skip_matrix() {
    let mut stores = Universe::new();
    let top = GlueSpec {
        width: sp(10),
        ..GlueSpec::ZERO
    };
    let discarded_glue = GlueSpec {
        width: sp(2),
        ..GlueSpec::ZERO
    };
    let children = tex_state::node_arena::PageListId::empty();
    let box_node = Node::HList(BoxNode::new(BoxNodeFields {
        width: sp(1),
        height: sp(4),
        depth: sp(1),
        shift: sp(0),
        box_lr: tex_state::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }));
    let (pruned, discarded) = prune_page_top_with_discards(
        &mut stores,
        vec![
            Node::Penalty(5),
            Node::Kern {
                amount: sp(1),
                kind: KernKind::Explicit,
            },
            Node::Glue {
                spec: discarded_glue,
                kind: GlueKind::Normal,
                leader: None,
            },
            box_node,
        ],
        top,
    );
    assert_eq!(discarded.len(), 3);
    let Node::Glue {
        ref spec,
        kind: GlueKind::SplitTopSkip,
        ..
    } = pruned[0]
    else {
        panic!("split top skip")
    };
    assert_eq!(spec.width, sp(6));
    assert!(matches!(pruned[1], Node::HList(_)));
}

#[test]
fn pdftex_prune_page_top_discards_snapy_but_preserves_other_whatsits() {
    // pdftex.web §§1378-1379 adds `pdf_snapy_node` to `prune_page_top`'s
    // discardable prefix without making other whatsit subtypes discardable.
    let mut stores = Universe::new();
    let top = GlueSpec::ZERO;
    let snap_glue = GlueSpec {
        width: sp(7),
        ..GlueSpec::ZERO
    };
    let box_node = Node::HList(BoxNode::new(BoxNodeFields {
        width: sp(1),
        height: sp(2),
        depth: sp(0),
        shift: sp(0),
        box_lr: tex_state::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: tex_state::node_arena::PageListId::empty(),
    }));

    let (pruned, discarded) = prune_page_top_with_discards(
        &mut stores,
        vec![
            Node::Whatsit(tex_state::node::Whatsit::PdfSnapY { glue: snap_glue }),
            Node::Whatsit(tex_state::node::Whatsit::PdfSnapRefPoint),
            box_node.clone(),
        ],
        top,
    );

    assert_eq!(
        discarded,
        [Node::Whatsit(tex_state::node::Whatsit::PdfSnapY {
            glue: snap_glue
        })]
    );
    assert_eq!(
        pruned,
        [
            Node::Whatsit(tex_state::node::Whatsit::PdfSnapRefPoint),
            Node::Glue {
                spec: top,
                kind: GlueKind::SplitTopSkip,
                leader: None,
            },
            box_node,
        ]
    );
}
