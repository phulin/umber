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

#[test]
fn sparse_mark_classes_use_dense_positions_and_canonical_order() {
    let mut page = PageBuilderState::default();
    let low = tokens(&[Token::param(1)]);
    let middle = tokens(&[Token::param(2)]);
    let high = tokens(&[Token::param(3)]);
    page.set_mark_class(PageMark::Bot, 32_767, high.clone());
    page.set_mark_class(PageMark::First, 7, low.clone());
    page.set_mark_class(PageMark::SplitBot, 255, middle.clone());

    assert_eq!(page.mark_class_ids().collect::<Vec<_>>(), [7, 255, 32_767]);
    assert_eq!(page.mark_class(PageMark::Bot, 32_767), high);
    assert_eq!(page.mark_class(PageMark::First, 7), low);

    let mut same_semantics = PageBuilderState::default();
    same_semantics.set_mark_class(PageMark::First, 7, low.clone());
    same_semantics.set_mark_class(PageMark::SplitBot, 255, middle.clone());
    same_semantics.set_mark_class(PageMark::Bot, 32_767, high.clone());
    assert_eq!(
        hash_page(&same_semantics),
        hash_page(&page),
        "class activation order does not affect canonical hashing"
    );

    page.clear_mark_class(PageMark::SplitBot, 255);
    page.set_mark_class(PageMark::Top, 128, middle.clone());
    assert_eq!(page.mark_class_ids().collect::<Vec<_>>(), [7, 128, 32_767]);
    assert_eq!(page.mark_class(PageMark::Top, 128), middle);
    assert!(page.mark_class(PageMark::SplitBot, 255).is_empty());
}

#[test]
fn runtime_checkpoint_restores_sparse_mark_class_positions() {
    let budget = crate::interner::InternerBudget::new(64, 64, 4096).expect("test budget");
    crate::with_universe(budget, |universe| {
        let expected = tokens(&[Token::param(4)]);
        {
            let mut context = universe.command_context().expect("command context");
            context.set_page_mark_class(PageMark::Bot, 32_767, expected.clone());
        }
        let checkpoint = universe.runtime_checkpoint().expect("runtime checkpoint");
        {
            let mut context = universe.command_context().expect("command context");
            context.clear_page_mark_class(PageMark::Bot, 32_767);
            context.set_page_mark_class(PageMark::First, 9, tokens(&[Token::param(9)]));
        }
        universe
            .restore_runtime_checkpoint_with_roots(&checkpoint, || {})
            .expect("runtime checkpoint restores");
        let context = universe.command_context().expect("command context");
        assert_eq!(
            context.page_mark_class_value(PageMark::Bot, 32_767),
            Some(&expected)
        );
        assert_eq!(context.page_mark_classes().collect::<Vec<_>>(), [32_767]);
    })
    .expect("test universe");
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
fn bounded_checkpoint_mark_restores_lists_insertions_marks_and_scalars() {
    let mut page = PageBuilderState::default();
    page.push_contribution(kern(1));
    page.push_current_page(kern(2));
    page.push_page_discard(kern(3));
    page.set_split_discards(vec![kern(4)]);
    page.upsert_page_insertion(PageInsertion::new(7, Scaled::from_raw(5)));
    page.set_mark_class(
        super::PageMark::Bot,
        7,
        crate::node::NodeTokenList::default(),
    );
    page.set_dimension(super::PageDimension::Goal, Scaled::from_raw(6));
    let expected = hash_page(&page);
    let mark = page.checkpoint_mark();

    page.push_contribution(kern(10));
    page.prepend_contribution(kern(11));
    if let Some(carrier) = page.pop_contribution_front() {
        page.discard_carrier(carrier);
    }
    page.push_current_page(kern(12));
    page.push_page_discard(kern(13));
    page.set_split_discards(vec![kern(14), kern(15)]);
    page.upsert_page_insertion(PageInsertion::new(7, Scaled::from_raw(16)));
    page.upsert_page_insertion(PageInsertion::new(8, Scaled::from_raw(17)));
    page.clear_mark_class(super::PageMark::Bot, 7);
    page.set_dimension(super::PageDimension::Goal, Scaled::from_raw(18));
    assert_ne!(hash_page(&page), expected);

    assert!(page.validates_checkpoint_mark(mark));
    page.restore_checkpoint_mark(mark);
    assert_eq!(hash_page(&page), expected);
    assert_eq!(
        page.page_insertion(7)
            .expect("restored insertion class")
            .height(),
        Scaled::from_raw(5)
    );
    assert_eq!(page.page_insertion(8), None);
}

#[test]
fn rooted_fork_uses_coordinate_roots_across_large_later_lanes() {
    let mut page = PageBuilderState::default();
    let rooted_mark = tokens(&[Token::param(7)]);
    page.push_contribution(kern(-1));
    page.push_current_page(kern(-2));
    page.push_page_discard(kern(-3));
    page.set_split_discards(vec![kern(-4)]);
    page.upsert_page_insertion(PageInsertion::new(7, Scaled::from_raw(-5)));
    page.set_mark_class(PageMark::Bot, 7, rooted_mark.clone());
    let checkpoint = page.checkpoint_mark();

    for index in 0..4_096 {
        page.push_contribution(kern(index));
        page.push_current_page(kern(index));
        page.push_page_discard(kern(index));
        page.upsert_page_insertion(PageInsertion::new(
            u16::try_from(index % 256).expect("class fits u16"),
            Scaled::from_raw(index),
        ));
        page.set_mark_class(
            PageMark::Bot,
            u16::try_from(index % 256).expect("class fits u16"),
            tokens(&[Token::param(
                u8::try_from(index % 9 + 1).expect("parameter fits u8"),
            )]),
        );
    }
    let accepted_hash = hash_page(&page);

    let tail = page.begin_checkpoint_candidate(checkpoint);
    assert_eq!(page.contribution().to_vec(), [kern(-1)]);
    assert_eq!(page.current_page().cloned().collect::<Vec<_>>(), [kern(-2)]);
    assert_eq!(
        page.page_insertion(7).expect("rooted insertion").height(),
        Scaled::from_raw(-5)
    );
    assert_eq!(page.mark_class_value(PageMark::Bot, 7), Some(&rooted_mark));

    let carrier = page.pop_contribution_front().expect("root contribution");
    assert_eq!(carrier.node(), &kern(-1));
    page.discard_carrier(carrier);
    assert_eq!(page.pop_current_page(), Some(kern(-2)));
    page.clear_page_discards();
    page.clear_split_discards();
    page.upsert_page_insertion(PageInsertion::new(7, Scaled::from_raw(99)));
    page.set_mark_class(PageMark::Bot, 7, tokens(&[Token::param(8)]));
    page.reject_checkpoint_candidate(tail);

    assert_eq!(hash_page(&page), accepted_hash);
    assert_eq!(page.contribution().len(), 4_097);
}

#[test]
fn rooted_candidate_shipout_rollback_restores_accepted_coordinates() {
    let mut page = PageBuilderState::default();
    page.push_contribution(kern(-1));
    page.push_current_page(kern(-2));
    page.push_page_discard(kern(-3));
    page.set_split_discards(vec![kern(-4)]);
    let checkpoint = page.checkpoint_mark();
    for index in 0..4_096 {
        page.push_contribution(kern(index));
        page.push_current_page(kern(index));
        page.push_page_discard(kern(index));
    }

    let tail = page.begin_checkpoint_candidate(checkpoint);
    let shipout = page.checkpoint_mark();
    let carrier = page.pop_contribution_front().expect("root contribution");
    assert_eq!(carrier.node(), &kern(-1));
    page.discard_carrier(carrier);
    assert_eq!(page.pop_current_page(), Some(kern(-2)));
    page.clear_page_discards();
    page.clear_split_discards();
    page.rollback_transaction(shipout);

    assert_eq!(page.contribution().to_vec(), [kern(-1)]);
    assert_eq!(page.current_page().cloned().collect::<Vec<_>>(), [kern(-2)]);
    assert_eq!(page.take_page_discards(), [kern(-3)]);
    assert_eq!(page.take_split_discards(), [kern(-4)]);
    page.reject_checkpoint_candidate(tail);
    assert_eq!(page.contribution().len(), 4_097);
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
            .map(|insertion| insertion.class())
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

#[test]
fn maintained_page_identity_covers_mutation_matrix_and_restore() {
    let mut page = PageBuilderState::default();
    page.enable_reachable_state_identity();
    let initial = page
        .checkpoint_mark()
        .reachable_state_identity_root()
        .expect("page root is available");
    page.set_dimension(super::PageDimension::Goal, Scaled::from_raw(1));
    page.push_contribution(kern(2));
    page.push_current_page(kern(3));
    page.push_page_discard(kern(4));
    page.set_split_discards(vec![kern(5)]);
    page.upsert_page_insertion(PageInsertion::new(7, Scaled::from_raw(6)));
    page.set_mark_class(PageMark::Bot, 7, tokens(&[Token::param(7)]));
    let rooted = page.checkpoint_mark();
    let expected = rooted
        .reachable_state_identity_root()
        .expect("page root is available");
    assert_ne!(expected, initial);

    page.prepend_contribution(kern(8));
    page.push_current_page(kern(9));
    page.clear_page_discards();
    page.clear_split_discards();
    page.upsert_page_insertion(PageInsertion::new(7, Scaled::from_raw(10)));
    page.clear_mark_class(PageMark::Bot, 7);
    assert_ne!(
        page.checkpoint_mark().reachable_state_identity_root(),
        Some(expected)
    );
    page.restore_checkpoint_mark(rooted);
    assert_eq!(
        page.checkpoint_mark().reachable_state_identity_root(),
        Some(expected)
    );
}

#[test]
fn page_identity_is_order_invariant_for_sparse_maps_and_constant_read_after_suffix() {
    let mut left = PageBuilderState::default();
    let mut right = PageBuilderState::default();
    left.enable_reachable_state_identity();
    right.enable_reachable_state_identity();
    let mark7 = tokens(&[Token::param(7)]);
    let mark8 = tokens(&[Token::param(8)]);
    left.upsert_page_insertion(PageInsertion::new(7, Scaled::from_raw(70)));
    left.upsert_page_insertion(PageInsertion::new(8, Scaled::from_raw(80)));
    left.set_mark_class(PageMark::First, 7, mark7.clone());
    left.set_mark_class(PageMark::Bot, 8, mark8.clone());
    right.set_mark_class(PageMark::Bot, 8, mark8);
    right.set_mark_class(PageMark::First, 7, mark7);
    right.upsert_page_insertion(PageInsertion::new(8, Scaled::from_raw(80)));
    right.upsert_page_insertion(PageInsertion::new(7, Scaled::from_raw(70)));
    assert_eq!(
        left.checkpoint_mark().reachable_state_identity_root(),
        right.checkpoint_mark().reachable_state_identity_root()
    );

    let early = left.checkpoint_mark();
    let expected = early.reachable_state_identity_root();
    for index in 0..4_096 {
        left.push_contribution(kern(index));
        left.push_current_page(kern(index));
    }
    assert_eq!(early.reachable_state_identity_root(), expected);
}

#[test]
fn page_candidate_identity_follows_reject_and_accept_ownership_transfer() {
    let mut page = PageBuilderState::default();
    page.enable_reachable_state_identity();
    page.push_contribution(kern(1));
    let early = page.checkpoint_mark();
    page.push_contribution(kern(2));
    let accepted_future = page
        .checkpoint_mark()
        .reachable_state_identity_root()
        .expect("accepted future root");

    let rejected_tail = page.begin_checkpoint_candidate(early);
    let after_rewind = page.accepted_replay_work();
    assert_eq!(after_rewind[2..], [1, 0]);
    page.push_contribution(kern(3));
    let rejected_candidate = page
        .checkpoint_mark()
        .reachable_state_identity_root()
        .expect("rejected candidate root");
    assert_ne!(rejected_candidate, accepted_future);
    page.reject_checkpoint_candidate(rejected_tail);
    let after_reject = page.accepted_replay_work();
    assert_eq!(after_reject[0], after_reject[1]);
    assert_eq!(after_reject[2..], [1, 1]);
    assert_eq!(
        page.checkpoint_mark().reachable_state_identity_root(),
        Some(accepted_future),
    );

    let accepted_tail = page.begin_checkpoint_candidate(early);
    let before_accept = page.accepted_replay_work();
    page.push_contribution(kern(4));
    let committed_candidate = page
        .checkpoint_mark()
        .reachable_state_identity_root()
        .expect("committed candidate root");
    page.accept_checkpoint_candidate(accepted_tail);
    let after_accept = page.accepted_replay_work();
    assert_eq!(after_accept[2], before_accept[2]);
    assert_eq!(after_accept[3], before_accept[3]);
    assert_eq!(after_accept[1], before_accept[1]);
    assert_eq!(
        page.checkpoint_mark().reachable_state_identity_root(),
        Some(committed_candidate),
    );
}
