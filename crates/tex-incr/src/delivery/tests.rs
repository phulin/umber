use super::*;

#[test]
fn semantic_content_shares_without_collapsing_trace_occurrences() {
    let content = ContentHash::from_bytes(b"same tokens");
    let root = Arc::new(DeliveryIdentity::SessionRoot(ContentHash::from_bytes(
        b"revision root",
    )));
    let left_parent = Arc::new(DeliveryIdentity::Synthetic {
        kind: SyntheticDeliveryKind::new(1),
        parent: Arc::clone(&root),
    });
    let right_parent = Arc::new(DeliveryIdentity::Synthetic {
        kind: SyntheticDeliveryKind::new(2),
        parent: root,
    });
    let left = DeliveryIdentity::TokenList {
        content,
        token_index: 2,
        parent: left_parent,
    };
    let right = DeliveryIdentity::TokenList {
        content,
        token_index: 2,
        parent: right_parent,
    };

    assert_ne!(left, right);
}

#[test]
fn argument_path_and_parent_are_part_of_macro_delivery_identity() {
    let definition = ContentHash::from_bytes(b"macro definition");
    let root = Arc::new(DeliveryIdentity::SessionRoot(ContentHash::from_bytes(
        b"root",
    )));
    let first = DeliveryIdentity::Macro {
        definition,
        invocation: Arc::clone(&root),
        argument_path: Arc::from([0_u8, 1]),
        token_index: 3,
    };
    let different_path = DeliveryIdentity::Macro {
        definition,
        invocation: root,
        argument_path: Arc::from([0_u8, 2]),
        token_index: 3,
    };

    assert_ne!(first, different_path);
}
