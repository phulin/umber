use super::PageBuilderState;
use crate::node::{KernKind, Node};
use crate::scaled::Scaled;
use std::sync::Arc;

fn kern(value: i32) -> Node {
    Node::Kern {
        amount: Scaled::from_raw(value),
        kind: KernKind::Explicit,
    }
}

#[test]
fn page_snapshot_clone_shares_roots_until_their_first_write() {
    let mut page = PageBuilderState::default();
    page.push_contribution(kern(1));
    page.push_current_page(kern(2));
    let snapshot = page.clone();

    assert!(Arc::ptr_eq(&page.contribution, &snapshot.contribution));
    assert!(Arc::ptr_eq(&page.current_page, &snapshot.current_page));
    assert!(Arc::ptr_eq(&page.insertions, &snapshot.insertions));
    assert!(Arc::ptr_eq(&page.mark_classes, &snapshot.mark_classes));

    page.push_contribution(kern(3));
    assert!(!Arc::ptr_eq(&page.contribution, &snapshot.contribution));
    assert_eq!(snapshot.contribution.len(), 1);
    assert!(Arc::ptr_eq(&page.current_page, &snapshot.current_page));

    page.push_current_page(kern(4));
    assert!(!Arc::ptr_eq(&page.current_page, &snapshot.current_page));
    assert_eq!(snapshot.current_page.len(), 1);
    assert!(Arc::ptr_eq(&page.insertions, &snapshot.insertions));
    assert!(Arc::ptr_eq(&page.mark_classes, &snapshot.mark_classes));
}
