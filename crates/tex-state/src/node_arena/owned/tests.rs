use super::NodeListRef;
use crate::node::{AdjustNode, Node};
use crate::stores::Stores;

fn freeze(stores: &mut Stores, nodes: impl IntoIterator<Item = Node>) -> NodeListRef {
    let mut builder = stores.node_list_builder();
    for node in nodes {
        builder.push(node);
    }
    stores.freeze_node_list_ref(builder)
}

#[test]
fn builder_freeze_preserves_and_resolves_child_values() {
    let mut stores = Stores::new();
    let child = freeze(&mut stores, [Node::Penalty(17)]);
    let parent = freeze(
        &mut stores,
        [Node::Adjust(AdjustNode::ordinary(child.clone()))],
    );

    let Node::Adjust(adjust) = parent.get(0).expect("parent node") else {
        panic!("expected adjustment")
    };
    assert_eq!(adjust.content.nodes().to_vec(), [Node::Penalty(17)]);
    assert_eq!(
        parent
            .child_nodes(adjust.content.id())
            .expect("child remains readable")
            .to_vec(),
        [Node::Penalty(17)]
    );
}

#[test]
fn nested_values_remain_readable_after_local_builders_are_gone() {
    let parent = {
        let mut stores = Stores::new();
        let grandchild = freeze(&mut stores, [Node::Penalty(41)]);
        let child = freeze(
            &mut stores,
            [Node::Adjust(AdjustNode::ordinary(grandchild))],
        );
        freeze(&mut stores, [Node::Adjust(AdjustNode::ordinary(child))])
    };

    let Node::Adjust(parent_adjust) = parent.get(0).expect("parent node") else {
        panic!("expected parent adjustment")
    };
    let child = parent_adjust.content;
    let Node::Adjust(child_adjust) = child.get(0).expect("child node") else {
        panic!("expected child adjustment")
    };
    assert_eq!(child_adjust.content.nodes().to_vec(), [Node::Penalty(41)]);
}

#[test]
fn semantic_identity_ignores_allocation_order() {
    fn nested(stores: &mut Stores, filler: bool) -> NodeListRef {
        if filler {
            drop(freeze(stores, [Node::Penalty(999)]));
        }
        let child = freeze(stores, [Node::Penalty(41)]);
        freeze(stores, [Node::Adjust(AdjustNode::ordinary(child))])
    }

    let first = nested(&mut Stores::new(), false);
    let shifted = nested(&mut Stores::new(), true);

    assert_eq!(first.semantic_id(), shifted.semantic_id());
    assert!(first.exact_semantic_eq(&shifted));
}
