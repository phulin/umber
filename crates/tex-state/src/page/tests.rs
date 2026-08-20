use super::PageBuilderState;
use super::state_hash::PageHashCache;
use crate::ids::TokenListId;
use crate::node::{KernKind, Node};
use crate::page::PageMark;
use crate::scaled::Scaled;
use crate::state_hash::StateHasher;
use crate::stores::Stores;
use crate::token::Token;

fn kern(value: i32) -> Node {
    Node::Kern {
        amount: Scaled::from_raw(value),
        kind: KernKind::Explicit,
    }
}

#[test]
fn page_snapshot_clone_preserves_values_and_isolates_later_writes() {
    let mut page = PageBuilderState::default();
    page.push_contribution(kern(1));
    page.push_current_page(kern(2));
    let snapshot = page.clone();

    assert_eq!(page.contribution.as_ref(), snapshot.contribution.as_ref());
    assert_eq!(
        page.current_page.iter().collect::<Vec<_>>(),
        snapshot.current_page.iter().collect::<Vec<_>>()
    );

    page.push_contribution(kern(3));
    assert_eq!(page.contribution.as_ref(), &[kern(1), kern(3)]);
    assert_eq!(snapshot.contribution.as_ref(), &[kern(1)]);

    page.push_current_page(kern(4));
    assert_eq!(
        page.current_page.iter().cloned().collect::<Vec<_>>(),
        [kern(2), kern(4)]
    );
    assert_eq!(
        snapshot.current_page.iter().cloned().collect::<Vec<_>>(),
        [kern(2)]
    );
}

#[test]
fn scalar_and_class_mark_values_survive_page_clone_and_clear() {
    let mut stores = Stores::new();
    let root = stores.intern_token_list_ref_in_domain(&[Token::param(3)], None);
    let mut page = PageBuilderState::default();
    page.set_mark(PageMark::Bot, root);
    page.set_mark_class(PageMark::SplitFirst, 19, root);
    let mut snapshot = page.clone();

    page.clear_mark(PageMark::Bot);
    page.clear_mark_class(PageMark::SplitFirst, 19);
    assert_eq!(
        stores.tokens(snapshot.mark(PageMark::Bot)).tokens(),
        &[Token::param(3)]
    );
    assert_eq!(
        stores
            .tokens(snapshot.mark_class(PageMark::SplitFirst, 19))
            .tokens(),
        &[Token::param(3)]
    );

    snapshot.clear_mark(PageMark::Bot);
    snapshot.clear_mark_class(PageMark::SplitFirst, 19);
    assert_eq!(snapshot.mark(PageMark::Bot), TokenListId::EMPTY);
    assert_eq!(
        snapshot.mark_class(PageMark::SplitFirst, 19),
        TokenListId::EMPTY
    );
}

fn hash_page(page: &PageBuilderState, cache: &mut PageHashCache) -> u64 {
    let mut hasher = StateHasher::new_exact(0x7061_6765_5f74_6573);
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
fn page_semantic_hash_follows_values_across_clones_and_restoration() {
    let mut page = PageBuilderState::default();
    let mut cache = PageHashCache::default();

    for value in 0..2_048 {
        page.push_current_page(kern(value));
        let _ = hash_page(&page, &mut cache);
    }

    let rollback = page.clone();
    let rollback_hash = hash_page(&rollback, &mut PageHashCache::default());
    let mut fork = page.clone();
    for value in 2_048..2_560 {
        fork.push_current_page(kern(value));
    }
    assert_ne!(
        hash_page(&fork, &mut PageHashCache::default()),
        rollback_hash
    );

    page.push_current_page(kern(9_999));
    let _ = hash_page(&page, &mut cache);
    page = rollback;
    assert_eq!(hash_page(&page, &mut cache), rollback_hash);
    assert_eq!(
        hash_page(&page, &mut cache),
        hash_page(&page, &mut PageHashCache::default())
    );
}
