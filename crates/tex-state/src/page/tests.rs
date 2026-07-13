use super::{PageBuilderState, PageNodeTree};
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
    assert!(Arc::ptr_eq(
        &page.current_page.forest,
        &snapshot.current_page.forest
    ));
    assert!(Arc::ptr_eq(&page.insertions, &snapshot.insertions));
    assert!(Arc::ptr_eq(&page.mark_classes, &snapshot.mark_classes));

    page.push_contribution(kern(3));
    assert!(!Arc::ptr_eq(&page.contribution, &snapshot.contribution));
    assert_eq!(snapshot.contribution.len(), 1);
    assert!(Arc::ptr_eq(
        &page.current_page.forest,
        &snapshot.current_page.forest
    ));

    page.push_current_page(kern(4));
    assert!(!Arc::ptr_eq(
        &page.current_page.tail,
        &snapshot.current_page.tail
    ));
    assert_eq!(snapshot.current_page.len(), 1);
    assert!(Arc::ptr_eq(&page.insertions, &snapshot.insertions));
    assert!(Arc::ptr_eq(&page.mark_classes, &snapshot.mark_classes));
}

#[test]
fn current_page_forest_has_content_position_shape_and_shares_full_prefixes() {
    let mut page = PageBuilderState::default();
    for value in 0..64 {
        page.push_current_page(kern(value));
    }
    let boundary = page.clone();
    page.push_current_page(kern(64));
    page.push_current_page(kern(65));

    assert_eq!(boundary.current_page.forest.len(), 1);
    assert_eq!(page.current_page.forest.len(), 1);
    assert!(Arc::ptr_eq(
        &boundary.current_page.forest[0],
        &page.current_page.forest[0]
    ));
    let (prefix, suffix) = page.take_current_page_prefix(65);
    assert_eq!(prefix.len(), 65);
    assert_eq!(suffix, vec![kern(65)]);
    assert_eq!(page.current_page.len(), 0);
}

#[test]
fn current_page_binary_carry_rebuilds_only_the_affected_path() {
    let mut page = PageBuilderState::default();
    for value in 0..192 {
        page.push_current_page(kern(value));
    }
    let boundary = page.clone();
    assert_eq!(boundary.current_page.forest.len(), 2);

    for value in 192..256 {
        page.push_current_page(kern(value));
    }
    assert_eq!(page.current_page.forest.len(), 1);
    let PageNodeTree::Branch { left, .. } = page.current_page.forest[0].as_ref() else {
        panic!("four full leaves must form a height-two root");
    };
    assert!(Arc::ptr_eq(left, &boundary.current_page.forest[0]));
    assert_eq!(
        page.current_page.iter().cloned().collect::<Vec<_>>().len(),
        256
    );
}
