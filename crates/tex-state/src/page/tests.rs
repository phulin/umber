use super::PageBuilderState;
use super::state_hash::PageHashCache;
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
