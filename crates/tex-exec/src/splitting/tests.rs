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
    let top = stores.intern_glue(GlueSpec {
        width: sp(10),
        ..GlueSpec::ZERO
    });
    let discarded_glue = stores.intern_glue(GlueSpec {
        width: sp(2),
        ..GlueSpec::ZERO
    });
    let children = stores.freeze_node_list(&[]);
    let box_node = Node::HList(BoxNode::new(BoxNodeFields {
        width: sp(1),
        height: sp(4),
        depth: sp(1),
        shift: sp(0),
        display: false,
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
        spec,
        kind: GlueKind::SplitTopSkip,
        ..
    } = pruned[0]
    else {
        panic!("split top skip")
    };
    assert_eq!(stores.glue(spec).width, sp(6));
    assert!(matches!(pruned[1], Node::HList(_)));
}
