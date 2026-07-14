use super::PageBuilderState;
use super::sequence::PageNodeTree;
use super::state_hash::PageHashCache;
use crate::node::{KernKind, Node};
use crate::scaled::Scaled;
use crate::state_hash::StateHasher;
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

fn hash_page(page: &PageBuilderState, cache: &mut PageHashCache) -> u64 {
    let mut hasher = StateHasher::new(0x7061_6765_5f74_6573);
    page.hash_semantic(
        &mut hasher,
        cache,
        |nodes, projection| {
            projection.usize(nodes.len());
            nodes.len()
        },
        |nodes, projection| {
            projection.usize(nodes.len());
            for node in nodes {
                let Node::Kern { amount, .. } = node else {
                    panic!("cache stress fixture contains only kerns");
                };
                projection.i32(amount.raw());
            }
            nodes.len()
        },
        |_, projection| projection.tag(0),
        |_, projection| projection.tag(0),
    );
    hasher.finish()
}

#[test]
fn current_page_tree_projection_is_lazy_and_shared() {
    let mut page = PageBuilderState::default();
    for value in 0..128 {
        page.push_current_page(kern(value));
    }
    let fork = page.clone();
    assert_eq!(page.current_page.testing_cached_projection_count(), 0);

    let _ = hash_page(&page, &mut PageHashCache::default());
    assert_eq!(page.current_page.testing_cached_projection_count(), 3);
    assert_eq!(fork.current_page.testing_cached_projection_count(), 3);

    for value in 128..192 {
        page.push_current_page(kern(value));
    }
    assert_eq!(page.current_page.testing_cached_projection_count(), 3);
    let _ = hash_page(&page, &mut PageHashCache::default());
    assert_eq!(page.current_page.testing_cached_projection_count(), 4);
}

#[test]
fn current_page_tail_projection_is_lazy_and_follows_copy_on_write() {
    let mut page = PageBuilderState::default();
    for value in 0..3 {
        page.push_current_page(kern(value));
    }
    let fork = page.clone();
    assert_eq!(page.current_page.testing_cached_tail_projection_count(), 0);

    let _ = hash_page(&page, &mut PageHashCache::default());
    assert_eq!(page.current_page.testing_cached_tail_projection_count(), 3);
    assert_eq!(fork.current_page.testing_cached_tail_projection_count(), 3);

    page.push_current_page(kern(3));
    assert_eq!(page.current_page.testing_cached_tail_projection_count(), 3);
    assert_eq!(fork.current_page.testing_cached_tail_projection_count(), 3);
    let _ = hash_page(&page, &mut PageHashCache::default());
    assert_eq!(page.current_page.testing_cached_tail_projection_count(), 4);
    assert_eq!(fork.current_page.testing_cached_tail_projection_count(), 3);
}

#[test]
fn checkpoint_identity_keys_do_not_pin_mutable_page_buffers() {
    let mut page = PageBuilderState::default();
    for value in 0..3 {
        page.push_current_page(kern(value));
    }
    let tail_data = page.current_page.tail.as_ptr();
    assert_eq!(Arc::strong_count(&page.current_page.tail), 1);
    assert_eq!(Arc::strong_count(&page.current_page.forest), 1);

    let _cursor = page.state_hash_cursor();
    let mut cache = PageHashCache::default();
    let _ = hash_page(&page, &mut cache);
    assert_eq!(Arc::strong_count(&page.current_page.tail), 1);
    assert_eq!(Arc::strong_count(&page.current_page.forest), 1);

    page.push_current_page(kern(3));
    assert_eq!(page.current_page.tail.as_ptr(), tail_data);
    assert_eq!(Arc::strong_count(&page.current_page.tail), 1);
}

fn assert_projection_count_follows_live_page(page: &PageBuilderState) {
    let full_leaves = page.current_page.len() / 64;
    assert!(page.current_page.testing_cached_projection_count() <= full_leaves * 2);
}

#[test]
fn page_projection_memoization_follows_live_trees_across_forks_and_rollback() {
    let mut page = PageBuilderState::default();
    let mut cache = PageHashCache::default();

    for value in 0..2_048 {
        page.push_current_page(kern(value));
        if value % 64 == 63 {
            let _ = hash_page(&page, &mut cache);
            assert_projection_count_follows_live_page(&page);
        }
    }

    let rollback = page.clone();
    let rollback_hash = hash_page(&rollback, &mut PageHashCache::default());
    let mut fork = page.clone();
    let mut fork_cache = cache.clone();
    for value in 2_048..2_560 {
        fork.push_current_page(kern(value));
    }
    let _ = hash_page(&fork, &mut fork_cache);
    assert_projection_count_follows_live_page(&fork);

    page.push_current_page(kern(9_999));
    let _ = hash_page(&page, &mut cache);
    page = rollback;
    assert_eq!(hash_page(&page, &mut cache), rollback_hash);
    assert_projection_count_follows_live_page(&page);

    for page_number in 0..32 {
        let _ = page.take_current_page_prefix(usize::MAX);
        assert_eq!(
            hash_page(&page, &mut cache),
            hash_page(&page, &mut PageHashCache::default())
        );
        assert_eq!(page.current_page.testing_cached_projection_count(), 0);
        for value in 0..256 {
            page.push_current_page(kern(page_number * 256 + value));
        }
        let _ = hash_page(&page, &mut cache);
        assert_projection_count_follows_live_page(&page);
    }
}
