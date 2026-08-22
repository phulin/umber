use super::state_hash::PageHashCache;
use super::{PageBuilderState, PageInsertion};
use crate::node::{KernKind, Node, NodeTokenList};
use crate::page::PageMark;
use crate::scaled::Scaled;
use crate::state_hash::StateHasher;
use crate::token::{Token, TokenWord};

fn kern(value: i32) -> Node {
    Node::Kern {
        amount: Scaled::from_raw(value),
        kind: KernKind::Explicit,
    }
}

fn tokens(tokens: &[Token]) -> NodeTokenList {
    NodeTokenList::new(
        tokens
            .iter()
            .copied()
            .map(TokenWord::pack)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    )
}

#[test]
fn page_buffers_mutate_directly_without_cow_roots() {
    let mut page = PageBuilderState::default();
    page.push_contribution(kern(1));
    page.push_current_page(kern(2));
    page.push_contribution(kern(3));
    page.push_current_page(kern(4));

    assert_eq!(
        page.contribution.iter().cloned().collect::<Vec<_>>(),
        [kern(1), kern(3)]
    );
    assert_eq!(
        page.current_page.iter().cloned().collect::<Vec<_>>(),
        [kern(2), kern(4)]
    );
}

#[test]
fn scalar_and_class_marks_store_handle_free_page_values() {
    let mut page = PageBuilderState::default();
    let scalar = tokens(&[Token::param(3)]);
    let class = tokens(&[Token::param(7)]);
    page.set_mark(PageMark::Bot, scalar.clone());
    page.set_mark_class(PageMark::SplitFirst, 19, class.clone());

    assert_eq!(page.mark(PageMark::Bot), scalar);
    assert_eq!(page.mark_class(PageMark::SplitFirst, 19), class);

    page.clear_mark(PageMark::Bot);
    page.clear_mark_class(PageMark::SplitFirst, 19);
    assert!(page.mark(PageMark::Bot).is_empty());
    assert!(page.mark_class(PageMark::SplitFirst, 19).is_empty());
}

fn hash_page(page: &PageBuilderState) -> u64 {
    let mut hasher = StateHasher::new_exact(0x7061_6765_5f74_6573);
    page.hash_semantic(
        &mut hasher,
        &mut PageHashCache,
        |nodes, projection| {
            projection.usize(nodes.len());
            nodes.len()
        },
        |nodes, projection| {
            projection.usize(nodes.len());
            for node in nodes {
                let Node::Kern { amount, .. } = node else {
                    panic!("hash fixture contains only kerns");
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
fn page_semantic_hash_tracks_direct_suffix_changes() {
    let mut page = PageBuilderState::default();
    page.push_current_page(kern(1));
    let before = hash_page(&page);
    page.push_current_page(kern(2));
    assert_ne!(hash_page(&page), before);
}

#[test]
fn detached_page_memo_parts_roundtrip_all_owned_sequences() {
    let mut source = PageBuilderState::default();
    source.push_contribution(kern(1));
    source.push_current_page(kern(2));
    source.push_page_discard(kern(3));
    source.set_split_discards(vec![kern(4)]);

    let (nodes, state) = source.memo_parts();
    let mut restored = PageBuilderState::default();
    restored
        .install_memo_parts(nodes.clone(), state.clone())
        .expect("detached page memo is internally aligned");
    let (roundtrip_nodes, roundtrip_state) = restored.memo_parts();

    assert_eq!(roundtrip_nodes, nodes);
    assert_eq!(roundtrip_state, state);
    assert!(restored.retained_bytes() >= std::mem::size_of::<PageBuilderState>());
}

#[test]
fn insertion_classes_use_dense_direct_positions_and_canonical_iteration_order() {
    let mut page = PageBuilderState::default();
    page.upsert_page_insertion(PageInsertion::new(4095, Scaled::from_raw(3)));
    page.upsert_page_insertion(PageInsertion::new(7, Scaled::from_raw(1)));
    page.upsert_page_insertion(PageInsertion::new(255, Scaled::from_raw(2)));
    page.upsert_page_insertion(PageInsertion::new(255, Scaled::from_raw(9)));

    assert_eq!(
        page.page_insertions()
            .iter()
            .map(PageInsertion::class)
            .collect::<Vec<_>>(),
        [7, 255, 4095]
    );
    assert_eq!(
        page.page_insertion(255).map(|insertion| insertion.height()),
        Some(Scaled::from_raw(9))
    );
    assert_eq!(page.page_insertion(8), None);

    page.start_page_after_output();
    assert!(page.page_insertions().is_empty());
    assert_eq!(page.page_insertion(255), None);
}
