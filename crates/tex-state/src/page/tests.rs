use super::state_hash::PageHashCache;
use super::{PageBuilderState, PageInsertion, PageRegion, PageRegionHistory};
use crate::node::{KernKind, Node, NodeTokenList};
use crate::node_region::NodePool;
use crate::page::{PageInteger, PageMark};
use crate::page_node_arena::{PageListId, PageMaterialArena, PageMaterialRegion};
use crate::scaled::Scaled;
use crate::state_hash::StateHasher;
use crate::token::{Token, TokenWord};

macro_rules! page_arena {
    ($arena:ident, $pool:ident, $state:ident) => {
        let mut $pool = NodePool::new();
        let mut $state = PageMaterialRegion::new(&mut $pool);
        #[allow(unused_mut)]
        let mut $arena = PageMaterialArena::new(&mut $pool, &mut $state);
    };
}

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

fn publish_nodes(
    arena: &mut PageMaterialArena,
    nodes: impl IntoIterator<Item = Node>,
) -> PageListId {
    arena.publish_owned(nodes).expect("publish test page nodes")
}

fn set_split_discards(
    page: &mut PageBuilderState,
    arena: &mut PageMaterialArena,
    nodes: impl IntoIterator<Item = Node>,
) {
    let root = publish_nodes(arena, nodes);
    page.set_split_discards(arena, root);
}

fn list_nodes(arena: &PageMaterialArena, root: impl super::PageListRoot) -> Vec<Node> {
    arena
        .node_cursor(root.list_id())
        .expect("test page root remains live")
        .iter()
        .cloned()
        .collect()
}

fn carrier_node(arena: &PageMaterialArena, carrier: &super::PageNodeCarrier) -> Node {
    list_nodes(arena, carrier.list)
        .into_iter()
        .next()
        .expect("test carrier owns one node")
}

#[test]
fn page_buffers_mutate_directly_without_cow_roots() {
    page_arena!(arena, pool, state);
    let mut page = PageBuilderState::default();
    page.push_contribution(&mut arena, kern(1));
    page.push_current_page(&mut arena, kern(2));
    page.push_contribution(&mut arena, kern(3));
    page.push_current_page(&mut arena, kern(4));

    assert_eq!(page.contribution(&arena).to_vec(), [kern(1), kern(3)]);
    assert_eq!(
        page.current_page(&arena).cloned().collect::<Vec<_>>(),
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
    page_arena!(arena, pool, state);
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
        hash_page(&same_semantics, &arena),
        hash_page(&page, &arena),
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

fn hash_page(page: &PageBuilderState, arena: &PageMaterialArena) -> u64 {
    let mut hasher = StateHasher::new_exact(0x7061_6765_5f74_6573);
    page.hash_semantic(
        arena,
        &mut hasher,
        &mut PageHashCache,
        |nodes, projection| {
            projection.usize(nodes.len());
            for node in nodes.iter() {
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
    page_arena!(arena, pool, state);
    let mut page = PageBuilderState::default();
    page.push_current_page(&mut arena, kern(1));
    let before = hash_page(&page, &arena);
    page.push_current_page(&mut arena, kern(2));
    assert_ne!(hash_page(&page, &arena), before);
}

#[test]
fn bounded_checkpoint_mark_restores_lists_insertions_marks_and_scalars() {
    page_arena!(arena, pool, state);
    let mut page = PageBuilderState::default();
    page.push_contribution(&mut arena, kern(1));
    page.push_current_page(&mut arena, kern(2));
    page.push_page_discard(&mut arena, kern(3));
    set_split_discards(&mut page, &mut arena, [kern(4)]);
    page.upsert_page_insertion(PageInsertion::new(7, Scaled::from_raw(5)));
    page.set_mark_class(
        super::PageMark::Bot,
        7,
        crate::node::NodeTokenList::default(),
    );
    page.set_dimension(super::PageDimension::Goal, Scaled::from_raw(6));
    let expected = hash_page(&page, &arena);
    let mark = page.checkpoint_mark();

    page.push_contribution(&mut arena, kern(10));
    page.prepend_contribution(&mut arena, kern(11));
    if let Some(carrier) = page.pop_contribution_front(&mut arena) {
        page.discard_carrier(carrier);
    }
    page.push_current_page(&mut arena, kern(12));
    page.push_page_discard(&mut arena, kern(13));
    set_split_discards(&mut page, &mut arena, [kern(14), kern(15)]);
    page.upsert_page_insertion(PageInsertion::new(7, Scaled::from_raw(16)));
    page.upsert_page_insertion(PageInsertion::new(8, Scaled::from_raw(17)));
    page.clear_mark_class(super::PageMark::Bot, 7);
    page.set_dimension(super::PageDimension::Goal, Scaled::from_raw(18));
    assert_ne!(hash_page(&page, &arena), expected);

    assert!(page.validates_checkpoint_mark(mark));
    page.restore_checkpoint_mark(mark);
    assert_eq!(hash_page(&page, &arena), expected);
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
    page_arena!(arena, pool, state);
    let mut page = PageBuilderState::default();
    let rooted_mark = tokens(&[Token::param(7)]);
    page.push_contribution(&mut arena, kern(-1));
    page.push_current_page(&mut arena, kern(-2));
    page.push_page_discard(&mut arena, kern(-3));
    set_split_discards(&mut page, &mut arena, [kern(-4)]);
    page.upsert_page_insertion(PageInsertion::new(7, Scaled::from_raw(-5)));
    page.set_mark_class(PageMark::Bot, 7, rooted_mark.clone());
    let checkpoint = page.checkpoint_mark();

    for index in 0..4_096 {
        page.push_contribution(&mut arena, kern(index));
        page.push_current_page(&mut arena, kern(index));
        page.push_page_discard(&mut arena, kern(index));
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
    let accepted_hash = hash_page(&page, &arena);

    let tail = page.begin_checkpoint_candidate(checkpoint);
    assert_eq!(page.contribution(&arena).to_vec(), [kern(-1)]);
    assert_eq!(
        page.current_page(&arena).cloned().collect::<Vec<_>>(),
        [kern(-2)]
    );
    assert_eq!(
        page.page_insertion(7).expect("rooted insertion").height(),
        Scaled::from_raw(-5)
    );
    assert_eq!(page.mark_class_value(PageMark::Bot, 7), Some(&rooted_mark));

    let carrier = page
        .pop_contribution_front(&mut arena)
        .expect("root contribution");
    assert_eq!(carrier_node(&arena, &carrier), kern(-1));
    page.discard_carrier(carrier);
    assert_eq!(page.pop_current_page(&mut arena), Some(kern(-2)));
    page.clear_page_discards(&arena);
    page.clear_split_discards(&arena);
    page.upsert_page_insertion(PageInsertion::new(7, Scaled::from_raw(99)));
    page.set_mark_class(PageMark::Bot, 7, tokens(&[Token::param(8)]));
    page.prepare_checkpoint_candidate_rejection(&tail);
    page.finish_checkpoint_candidate_rejection(tail);

    assert_eq!(hash_page(&page, &arena), accepted_hash);
    assert_eq!(page.contribution(&arena).len(), 4_097);
}

#[test]
fn rooted_candidate_shipout_rollback_restores_accepted_coordinates() {
    page_arena!(arena, pool, state);
    let mut page = PageBuilderState::default();
    page.push_contribution(&mut arena, kern(-1));
    page.push_current_page(&mut arena, kern(-2));
    page.push_page_discard(&mut arena, kern(-3));
    set_split_discards(&mut page, &mut arena, [kern(-4)]);
    let checkpoint = page.checkpoint_mark();
    for index in 0..4_096 {
        page.push_contribution(&mut arena, kern(index));
        page.push_current_page(&mut arena, kern(index));
        page.push_page_discard(&mut arena, kern(index));
    }

    let tail = page.begin_checkpoint_candidate(checkpoint);
    let shipout = page.checkpoint_mark();
    let carrier = page
        .pop_contribution_front(&mut arena)
        .expect("root contribution");
    assert_eq!(carrier_node(&arena, &carrier), kern(-1));
    page.discard_carrier(carrier);
    assert_eq!(page.pop_current_page(&mut arena), Some(kern(-2)));
    page.clear_page_discards(&arena);
    page.clear_split_discards(&arena);
    page.rollback_transaction(shipout);

    assert_eq!(page.contribution(&arena).to_vec(), [kern(-1)]);
    assert_eq!(
        page.current_page(&arena).cloned().collect::<Vec<_>>(),
        [kern(-2)]
    );
    let page_discards = page.take_page_discards(&arena);
    let split_discards = page.take_split_discards(&arena);
    assert_eq!(list_nodes(&arena, page_discards), [kern(-3)]);
    assert_eq!(list_nodes(&arena, split_discards), [kern(-4)]);
    page.prepare_checkpoint_candidate_rejection(&tail);
    page.finish_checkpoint_candidate_rejection(tail);
    assert_eq!(page.contribution(&arena).len(), 4_097);
}

#[test]
fn repeated_accept_keeps_prefix_checkpoint_frames_physically_stable() {
    page_arena!(arena, pool, state);
    let mut page = PageBuilderState::default();
    page.push_contribution(&mut arena, kern(1));
    let first = page.checkpoint_mark();
    page.push_contribution(&mut arena, kern(2));
    let selected = page.checkpoint_mark();
    page.push_contribution(&mut arena, kern(3));
    let detached_later = page.checkpoint_mark();

    let tail = page.begin_checkpoint_candidate(selected);
    page.push_contribution(&mut arena, kern(20));
    let candidate_later = page.checkpoint_mark();
    page.prepare_checkpoint_candidate_acceptance(tail);

    assert!(page.validates_checkpoint_mark(first));
    assert!(page.validates_checkpoint_mark(selected));
    assert!(page.validates_checkpoint_mark(candidate_later));
    assert!(!page.validates_checkpoint_mark(detached_later));

    let tail = page.begin_checkpoint_candidate(first);
    page.push_contribution(&mut arena, kern(30));
    page.prepare_checkpoint_candidate_rejection(&tail);
    page.finish_checkpoint_candidate_rejection(tail);
    assert!(page.validates_checkpoint_mark(first));
    assert!(page.validates_checkpoint_mark(selected));
    assert!(page.validates_checkpoint_mark(candidate_later));

    let tail = page.begin_checkpoint_candidate(selected);
    page.push_contribution(&mut arena, kern(40));
    let replacement = page.checkpoint_mark();
    page.prepare_checkpoint_candidate_acceptance(tail);
    assert!(page.validates_checkpoint_mark(first));
    assert!(page.validates_checkpoint_mark(selected));
    assert!(page.validates_checkpoint_mark(replacement));
    assert!(!page.validates_checkpoint_mark(candidate_later));
}

#[test]
fn detached_page_memo_parts_roundtrip_all_owned_sequences() {
    page_arena!(arena, pool, state);
    let mut source = PageBuilderState::default();
    source.push_contribution(&mut arena, kern(1));
    source.push_current_page(&mut arena, kern(2));
    source.push_page_discard(&mut arena, kern(3));
    set_split_discards(&mut source, &mut arena, [kern(4)]);

    let (nodes, state) = source.memo_parts(&arena);
    let mut restored = PageBuilderState::default();
    restored
        .install_memo_parts(&mut arena, nodes.clone(), state.clone())
        .expect("detached page memo is internally aligned");
    let (roundtrip_nodes, roundtrip_state) = restored.memo_parts(&arena);

    assert_eq!(roundtrip_nodes, nodes);
    assert_eq!(roundtrip_state, state);
    assert!(restored.retained_bytes() >= std::mem::size_of::<PageBuilderState>());
}

#[test]
fn insertion_classes_use_dense_direct_positions_and_canonical_iteration_order() {
    page_arena!(arena, pool, state);
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

    page.start_page_after_output(&arena);
    assert!(page.page_insertions().is_empty());
    assert_eq!(page.page_insertion(255), None);
}

#[test]
fn maintained_page_identity_covers_mutation_matrix_and_restore() {
    page_arena!(arena, pool, state);
    arena.enable_semantic_identity();
    let mut page = PageBuilderState::default();
    page.enable_reachable_state_identity();
    let initial = page
        .checkpoint_mark()
        .reachable_state_identity_root()
        .expect("page root is available");
    page.set_dimension(super::PageDimension::Goal, Scaled::from_raw(1));
    page.push_contribution(&mut arena, kern(2));
    page.push_current_page(&mut arena, kern(3));
    page.push_page_discard(&mut arena, kern(4));
    set_split_discards(&mut page, &mut arena, [kern(5)]);
    page.upsert_page_insertion(PageInsertion::new(7, Scaled::from_raw(6)));
    page.set_mark_class(PageMark::Bot, 7, tokens(&[Token::param(7)]));
    let rooted = page.checkpoint_mark();
    let expected = rooted
        .reachable_state_identity_root()
        .expect("page root is available");
    assert_ne!(expected, initial);

    page.prepend_contribution(&mut arena, kern(8));
    page.push_current_page(&mut arena, kern(9));
    page.clear_page_discards(&arena);
    page.clear_split_discards(&arena);
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
    page_arena!(arena, pool, state);
    arena.enable_semantic_identity();
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
        left.push_contribution(&mut arena, kern(index));
        left.push_current_page(&mut arena, kern(index));
    }
    assert_eq!(early.reachable_state_identity_root(), expected);
}

#[test]
fn page_candidate_identity_follows_reject_and_accept_ownership_transfer() {
    page_arena!(arena, pool, state);
    arena.enable_semantic_identity();
    let mut page = PageBuilderState::default();
    page.enable_reachable_state_identity();
    page.push_contribution(&mut arena, kern(1));
    let early = page.checkpoint_mark();
    page.push_contribution(&mut arena, kern(2));
    let accepted_future = page
        .checkpoint_mark()
        .reachable_state_identity_root()
        .expect("accepted future root");

    let rejected_tail = page.begin_checkpoint_candidate(early);
    let after_rewind = page.accepted_replay_work();
    assert_eq!(after_rewind[2..], [1, 0]);
    page.push_contribution(&mut arena, kern(3));
    let rejected_candidate = page
        .checkpoint_mark()
        .reachable_state_identity_root()
        .expect("rejected candidate root");
    assert_ne!(rejected_candidate, accepted_future);
    page.prepare_checkpoint_candidate_rejection(&rejected_tail);
    page.finish_checkpoint_candidate_rejection(rejected_tail);
    let after_reject = page.accepted_replay_work();
    assert_eq!(after_reject[0], after_reject[1]);
    assert_eq!(after_reject[2..], [1, 1]);
    assert_eq!(
        page.checkpoint_mark().reachable_state_identity_root(),
        Some(accepted_future),
    );

    let accepted_tail = page.begin_checkpoint_candidate(early);
    let before_accept = page.accepted_replay_work();
    page.push_contribution(&mut arena, kern(4));
    let committed_candidate = page
        .checkpoint_mark()
        .reachable_state_identity_root()
        .expect("committed candidate root");
    page.prepare_checkpoint_candidate_acceptance(accepted_tail);
    let after_accept = page.accepted_replay_work();
    assert_eq!(after_accept[2], before_accept[2]);
    assert_eq!(after_accept[3], before_accept[3]);
    assert_eq!(after_accept[1], before_accept[1]);
    assert_eq!(
        page.checkpoint_mark().reachable_state_identity_root(),
        Some(committed_candidate),
    );
}

fn page_candidate_settlement_work(
    accepted_updates: usize,
    accept: bool,
) -> super::PageCandidateSettlementCounters {
    page_arena!(arena, pool, state);
    let mut page = PageBuilderState::default();
    page.push_contribution(&mut arena, kern(-1));
    page.push_current_page(&mut arena, kern(-2));
    page.push_page_discard(&mut arena, kern(-3));
    set_split_discards(&mut page, &mut arena, [kern(-4)]);
    page.upsert_page_insertion(PageInsertion::new(7, Scaled::from_raw(-5)));
    page.set_mark(PageMark::Top, tokens(&[Token::param(1)]));
    page.set_mark_class(PageMark::Bot, 7, tokens(&[Token::param(2)]));
    let selected = page.checkpoint_mark();

    for index in 0..accepted_updates {
        let value = i32::try_from(index).expect("test update fits i32");
        page.push_contribution(&mut arena, kern(value));
        page.push_current_page(&mut arena, kern(value));
        page.push_page_discard(&mut arena, kern(value));
        page.upsert_page_insertion(PageInsertion::new(7, Scaled::from_raw(value)));
        page.set_mark_class(
            PageMark::Bot,
            7,
            tokens(&[Token::param(
                u8::try_from(index % 9 + 1).expect("parameter fits u8"),
            )]),
        );
    }

    let tail = page.begin_checkpoint_candidate(selected);
    let carrier = page
        .pop_contribution_front(&mut arena)
        .expect("selected contribution");
    page.discard_carrier(carrier);
    assert_eq!(page.pop_current_page(&mut arena), Some(kern(-2)));
    page.clear_page_discards(&arena);
    page.clear_split_discards(&arena);
    page.upsert_page_insertion(PageInsertion::new(7, Scaled::from_raw(91)));
    page.set_mark(PageMark::Top, tokens(&[Token::param(8)]));
    page.set_mark_class(PageMark::Bot, 7, tokens(&[Token::param(9)]));

    if accept {
        page.prepare_checkpoint_candidate_acceptance(tail);
    } else {
        page.prepare_checkpoint_candidate_rejection(&tail);
        page.finish_checkpoint_candidate_rejection(tail);
    }
    page.candidate_settlement_counters()
}

#[test]
fn candidate_settlement_counters_exclude_accepted_page_payload_scans_and_copies() {
    let small_accept = page_candidate_settlement_work(1, true);
    let large_accept = page_candidate_settlement_work(4_096, true);
    for counters in [small_accept, large_accept] {
        assert_eq!(counters.checkpoint_capture_records_scanned, 0);
        assert_eq!(counters.acceptance_payload_records_scanned, 0);
        assert_eq!(counters.canonical_lane_records_scanned, 0);
        assert_eq!(counters.canonical_values_copied, 0);
        assert_eq!(counters.candidate_acceptances, 1);
        assert_eq!(counters.candidate_rejections, 0);
    }
    assert!(
        large_accept.selected_journal_records_rewound
            > small_accept.selected_journal_records_rewound,
        "checkpoint selection reports its explicitly chosen journal distance"
    );

    let small_reject = page_candidate_settlement_work(1, false);
    let large_reject = page_candidate_settlement_work(4_096, false);
    for counters in [small_reject, large_reject] {
        assert_eq!(counters.checkpoint_capture_records_scanned, 0);
        assert_eq!(counters.acceptance_payload_records_scanned, 0);
        assert_eq!(counters.canonical_lane_records_scanned, 0);
        assert_eq!(counters.canonical_values_copied, 0);
        assert_eq!(counters.candidate_acceptances, 0);
        assert_eq!(counters.candidate_rejections, 1);
        assert_eq!(
            counters.rejected_prior_records_redone, counters.selected_journal_records_rewound,
            "rejection redoes only the journal distance selected at edit start"
        );
    }
    assert_eq!(
        small_reject.rejected_candidate_records_rewound,
        large_reject.rejected_candidate_records_rewound,
        "candidate rejection work is independent of accepted page payload"
    );
}

#[test]
fn paragraph_checkpoints_share_one_page_region_without_node_copies() {
    let mut pool = NodePool::new();
    let mut region = PageRegion::new(&mut pool);
    let region_id = region.id();
    {
        let (mut nodes, page) = region.parts_mut(&mut pool);
        page.push_contribution(&mut nodes, kern(1));
    }
    let first_root = region.builder().contribution;
    let first_address = region
        .nodes(&pool)
        .span_list(first_root)
        .expect("first contribution")
        .get(0)
        .map(std::ptr::from_ref)
        .expect("first contribution address");
    let before = region.nodes(&pool).counters();

    let mut keys = Vec::new();
    for value in 2..=64 {
        keys.push(
            region
                .seal_checkpoint(&mut pool)
                .expect("sealed page checkpoint"),
        );
        let (mut nodes, page) = region.parts_mut(&mut pool);
        page.push_contribution(&mut nodes, kern(value));
    }

    assert!(keys.iter().all(|key| key.region == region_id));
    assert_eq!(region.checkpoints.len(), keys.len());
    assert_eq!(region.nodes(&pool).counters().source_nodes_copied, 0);
    assert_eq!(
        region.nodes(&pool).counters().new_semantic_nodes,
        before.new_semantic_nodes + 63
    );
    assert_eq!(
        region
            .nodes(&pool)
            .span_list(first_root)
            .expect("unchanged prefix remains live")
            .get(0)
            .map(std::ptr::from_ref),
        Some(first_address),
        "checkpoint publication never relocates the unchanged prefix"
    );
}

#[test]
fn page_region_fork_reject_and_accept_settle_roots_with_arena_suffix() {
    let mut pool = NodePool::new();
    let mut region = PageRegion::new(&mut pool);
    {
        let (mut nodes, page) = region.parts_mut(&mut pool);
        page.push_contribution(&mut nodes, kern(1));
        page.push_current_page(&mut nodes, kern(2));
        page.push_page_discard(&mut nodes, kern(3));
        set_split_discards(page, &mut nodes, [kern(4)]);
    }
    let selected = region
        .seal_checkpoint(&mut pool)
        .expect("selected checkpoint");
    let prefix = region.builder().contribution;
    let prefix_address = region
        .nodes(&pool)
        .span_list(prefix)
        .expect("selected prefix")
        .get(0)
        .map(std::ptr::from_ref)
        .expect("selected prefix address");
    {
        let (mut nodes, page) = region.parts_mut(&mut pool);
        page.push_contribution(&mut nodes, kern(10));
        page.push_current_page(&mut nodes, kern(20));
        page.push_page_discard(&mut nodes, kern(30));
        set_split_discards(page, &mut nodes, [kern(40)]);
    }
    let superseded = region
        .seal_checkpoint(&mut pool)
        .expect("accepted suffix checkpoint");
    let accepted_roots = [
        region.builder().contribution,
        region.builder().current_page,
        region.builder().page_discards,
        region.builder().split_discards,
    ];
    let accepted = {
        let nodes = region.nodes_mut(&mut pool);
        accepted_roots.map(|root| list_nodes(&nodes, root))
    };

    let rejected = region
        .begin_checkpoint_candidate(&mut pool, selected)
        .expect("fork selected page row");
    {
        let (mut nodes, page) = region.parts_mut(&mut pool);
        page.push_contribution(&mut nodes, kern(11));
        page.push_current_page(&mut nodes, kern(21));
        page.push_page_discard(&mut nodes, kern(31));
        set_split_discards(page, &mut nodes, [kern(41)]);
    }
    region
        .reject_checkpoint_candidate(&mut pool, rejected)
        .expect("atomic page rejection");
    let restored_roots = [
        region.builder().contribution,
        region.builder().current_page,
        region.builder().page_discards,
        region.builder().split_discards,
    ];
    let restored = {
        let nodes = region.nodes_mut(&mut pool);
        restored_roots.map(|root| list_nodes(&nodes, root))
    };
    assert_eq!(restored, accepted);
    assert!(region.validates_checkpoint(&pool, superseded));

    let accepted_tail = region
        .begin_checkpoint_candidate(&mut pool, selected)
        .expect("refork selected page row");
    {
        let (mut nodes, page) = region.parts_mut(&mut pool);
        page.push_contribution(&mut nodes, kern(12));
        page.push_current_page(&mut nodes, kern(22));
        page.push_page_discard(&mut nodes, kern(32));
        set_split_discards(page, &mut nodes, [kern(42)]);
    }
    let candidate = region
        .seal_checkpoint(&mut pool)
        .expect("candidate checkpoint");
    region
        .accept_checkpoint_candidate(&mut pool, accepted_tail)
        .expect("atomic page acceptance");

    assert!(!region.validates_checkpoint(&pool, superseded));
    let candidate_row = region
        .checkpoint(candidate)
        .expect("candidate row retained");
    assert!(
        region
            .nodes(&pool)
            .validates_checkpoint(candidate_row.nodes),
        "candidate arena mark remains accepted"
    );
    assert!(
        region
            .builder()
            .validates_checkpoint_mark(candidate_row.builder),
        "candidate builder mark remains accepted"
    );
    assert_eq!(region.nodes(&pool).counters().source_nodes_copied, 0);
    assert_eq!(
        region
            .nodes(&pool)
            .span_list(prefix)
            .expect("unchanged prefix survives acceptance")
            .get(0)
            .map(std::ptr::from_ref),
        Some(prefix_address)
    );
    let contribution = region.builder().contribution;
    assert_eq!(
        list_nodes(&region.nodes_mut(&mut pool), contribution),
        [kern(1), kern(12)]
    );
}

#[test]
fn held_over_material_is_self_contained_in_next_page_region() {
    let mut pool = NodePool::new();
    let mut old = PageRegion::new(&mut pool);
    let shipped = old
        .nodes_mut(&mut pool)
        .publish_owned((0..128).map(kern))
        .expect("shipped prefix");
    let child = old
        .nodes_mut(&mut pool)
        .publish_owned([kern(201), kern(202)])
        .expect("held-over child");
    let held_over = old
        .nodes_mut(&mut pool)
        .publish_owned([Node::Disc {
            kind: crate::node::DiscKind::Discretionary,
            pre: child,
            post: PageListId::empty(),
            replace: PageListId::empty(),
            physical_replace_count: 0,
        }])
        .expect("held-over root");
    let old_id = old.id();

    let succession = old
        .finish_shipout(&mut pool, held_over)
        .map_err(|(error, _)| error)
        .expect("page succession");
    assert_ne!(succession.current.id(), old_id);
    assert!(succession.retained_prior.is_none());
    assert_eq!(succession.current.counters().page_regions_started, 2);
    assert_eq!(succession.current.counters().page_regions_dropped, 1);
    assert_eq!(succession.current.counters().held_over_nodes_copied, 3);
    assert_eq!(
        succession
            .current
            .nodes(&pool)
            .counters()
            .source_nodes_copied,
        3,
        "only the recursively selected held-over closure is a semantic copy"
    );
    assert_eq!(
        succession
            .current
            .nodes(&pool)
            .counters()
            .new_semantic_nodes,
        3,
        "the 128-node shipped prefix is never copied"
    );
    assert!(!succession.current.nodes(&pool).contains(shipped));
    assert!(!succession.current.nodes(&pool).contains(held_over));
    let copied_root = succession
        .current
        .nodes(&pool)
        .list(succession.held_over)
        .expect("held-over root belongs to next region");
    let Node::Disc { pre, .. } = copied_root.get(0).expect("copied discretionary") else {
        panic!("held-over root shape changed");
    };
    assert_eq!(
        succession
            .current
            .nodes(&pool)
            .list(*pre)
            .expect("nested child is region-local")
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        [kern(201), kern(202)]
    );
}

#[test]
fn shared_pool_retires_uncheckpointed_page_region_on_succession() {
    let mut history = PageRegionHistory::default();
    let old = history.current().id();
    let held_over = publish_nodes(&mut history.nodes_mut(), [kern(5)]);

    history.finish_shipout(held_over).expect("page succession");

    assert!(!history.pool.validates_id(old));
    assert_ne!(history.current().id(), old);
}

#[test]
fn checkpoint_retains_old_page_region_until_last_row_is_pruned() {
    let mut pool = NodePool::new();
    let mut old = PageRegion::new(&mut pool);
    let held_over = old
        .nodes_mut(&mut pool)
        .publish_owned([kern(7)])
        .expect("held-over node");
    {
        let (mut nodes, page) = old.parts_mut(&mut pool);
        page.push_current_page(&mut nodes, kern(8));
    }
    let checkpoint = old.seal_checkpoint(&mut pool).expect("old-page checkpoint");
    let old_id = old.id();

    let mut succession = old
        .finish_shipout(&mut pool, held_over)
        .map_err(|(error, _)| error)
        .expect("page succession");
    let retained = succession
        .retained_prior
        .as_ref()
        .expect("checkpoint history owns the old page region");
    assert_eq!(retained.id(), old_id);
    assert!(retained.validates_checkpoint(&pool, checkpoint));
    assert_eq!(succession.current.counters().page_regions_retained, 1);

    assert!(succession.prune_retained_checkpoint(&mut pool, checkpoint));
    assert!(succession.retained_prior.is_none());
    assert_eq!(succession.current.counters().page_regions_dropped, 1);
}

#[test]
fn page_history_release_drops_the_last_noncurrent_region_and_stales_its_id() {
    let mut history = PageRegionHistory::default();
    let old_id = history.current().id();
    let held_over = publish_nodes(&mut history.nodes_mut(), [kern(7)]);
    let checkpoint = history.seal_checkpoint().expect("old-page checkpoint");

    history.finish_shipout(held_over).expect("page succession");
    assert!(history.pool.validates_id(old_id));
    assert!(history.validates_checkpoint(checkpoint));

    let receipt = history
        .release_checkpoint(checkpoint)
        .expect("outer history releases private page row");
    assert_eq!(receipt.rows_released, 1);
    assert_eq!(receipt.regions_retired, 1);
    assert_eq!(receipt.retained_regions, 1);
    assert_eq!(receipt.retained_rows, 0);
    assert!(!history.pool.validates_id(old_id));
    assert!(!history.validates_checkpoint(checkpoint));
    assert_eq!(history.current().counters().page_regions_dropped, 1);
    assert_eq!(
        history.release_checkpoint(checkpoint),
        Err(crate::fork_arena::ForkArenaError::InvalidCheckpoint)
    );
}

#[test]
fn foreign_held_over_root_is_rejected_without_consuming_page_owner() {
    let mut old_pool = NodePool::new();
    let old = PageRegion::new(&mut old_pool);
    let mut foreign_pool = NodePool::new();
    let mut foreign = PageRegion::new(&mut foreign_pool);
    let root = foreign
        .nodes_mut(&mut foreign_pool)
        .publish_owned([kern(9)])
        .expect("foreign root");

    let (error, old) = match old.finish_shipout(&mut old_pool, root) {
        Ok(_) => panic!("foreign held-over root must reject"),
        Err(failure) => failure,
    };
    assert_eq!(error, crate::fork_arena::ForkArenaError::InvalidRegion);
    assert_eq!(old.counters().cross_region_node_reference_rejections, 1);
    assert_eq!(old.counters().page_regions_dropped, 0);
}

#[test]
fn page_history_reject_restores_detached_later_regions_wholesale() {
    let mut history = PageRegionHistory::default();
    let first_root = publish_nodes(&mut history.nodes_mut(), [kern(1), kern(2)]);
    let (mut nodes, builder) = history.parts_mut();
    builder.push_current_page_list(&mut nodes, first_root);
    let first = history.seal_checkpoint().expect("first-page checkpoint");
    let held_over = publish_nodes(&mut history.nodes_mut(), [kern(3), kern(4)]);
    history.finish_shipout(held_over).expect("page succession");
    let later_region = history.current().id();
    let later = history.seal_checkpoint().expect("later-page checkpoint");

    let tail = history
        .begin_checkpoint_candidate(first)
        .expect("fork selected old page");
    assert_eq!(history.current().id(), first.region);
    let candidate = publish_nodes(&mut history.nodes_mut(), [kern(9)]);
    history
        .finish_shipout(candidate)
        .expect("candidate creates a later page");
    assert_ne!(history.current().id(), later_region);

    history
        .reject_checkpoint_candidate(tail)
        .expect("rejection restores accepted suffix");
    assert_eq!(history.current().id(), later_region);
    assert!(history.validates_checkpoint(first));
    assert!(history.validates_checkpoint(later));
}

#[test]
fn page_history_accept_drops_superseded_later_regions() {
    let mut history = PageRegionHistory::default();
    let first = history.seal_checkpoint().expect("first-page checkpoint");
    let held_over = publish_nodes(&mut history.nodes_mut(), [kern(5)]);
    history.finish_shipout(held_over).expect("page succession");
    let superseded_root = history.builder().current_page;
    let superseded = history.seal_checkpoint().expect("later-page checkpoint");

    let tail = history
        .begin_checkpoint_candidate(first)
        .expect("fork selected old page");
    history
        .accept_checkpoint_candidate(tail)
        .expect("accept selected-page replacement");

    assert!(history.validates_checkpoint(first));
    assert!(!history.validates_checkpoint(superseded));
    assert!(!history.nodes().contains(superseded_root.list()));
}

#[test]
fn prepared_successor_does_not_drop_current_owner_before_shipout_commit() {
    let mut history = PageRegionHistory::default();
    let old_region = history.current().id();
    let held_over = publish_nodes(&mut history.nodes_mut(), [kern(31), kern(32)]);

    history
        .prepare_shipout(held_over)
        .expect("prepare exact held-over evacuation");
    assert_eq!(history.current().id(), old_region);
    assert!(history.nodes().contains(held_over));

    let copied = history
        .commit_prepared_shipout()
        .expect("commit successor after output consumption");
    assert_ne!(history.current().id(), old_region);
    assert!(!history.nodes().contains(held_over));
    assert_eq!(
        history
            .nodes()
            .list(copied)
            .expect("copied holdover belongs to successor")
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        [kern(31), kern(32)]
    );
}

#[test]
fn production_succession_transfers_complete_page_builder_owner() {
    let mut history = PageRegionHistory::default();
    let old_region = history.current().id();
    let contribution = publish_nodes(&mut history.nodes_mut(), [kern(1), kern(2)]);
    let current_page = publish_nodes(&mut history.nodes_mut(), [kern(3)]);
    let split_discards = publish_nodes(&mut history.nodes_mut(), [kern(5), kern(6)]);
    {
        let (mut nodes, builder) = history.parts_mut();
        builder.prepend_contributions(&mut nodes, contribution);
        builder.push_current_page_list(&mut nodes, current_page);
        builder.push_page_discard(&mut nodes, kern(4));
        builder.set_split_discards(&nodes, split_discards);
        builder.set_integer(PageInteger::DeadCycles, 7);
        builder.set_mark_class(
            PageMark::Bot,
            0,
            tokens(&[Token::param(1), Token::param(2)]),
        );
    }

    history
        .prepare_production_shipout()
        .expect("complete page owner preflights");
    assert_eq!(history.current().id(), old_region);
    history
        .commit_prepared_shipout()
        .expect("production succession commits");

    assert!(!history.pool.validates_id(old_region));
    let roots = history.builder().payload_roots();
    assert_eq!(
        list_nodes(&history.nodes_mut(), roots.contribution),
        [kern(1), kern(2)]
    );
    assert_eq!(
        list_nodes(&history.nodes_mut(), roots.current_page),
        [kern(3)]
    );
    assert_eq!(
        list_nodes(&history.nodes_mut(), roots.page_discards),
        [kern(4)]
    );
    assert_eq!(
        list_nodes(&history.nodes_mut(), roots.split_discards),
        [kern(5), kern(6)]
    );
    let builder = history.builder();
    assert_eq!(builder.integer(PageInteger::DeadCycles), 7);
    assert_eq!(
        builder.mark_class_value(PageMark::Bot, 0),
        Some(&tokens(&[Token::param(1), Token::param(2)]))
    );
    assert_eq!(history.current().counters().page_regions_started, 2);
}

#[test]
fn production_uncheckpointed_pages_reuse_pool_at_a_fixed_high_water() {
    let mut history = PageRegionHistory::default();
    let mut warmed_pages = None;
    for page in 0..256 {
        let _shipped = publish_nodes(&mut history.nodes_mut(), (0..128).map(kern));
        history
            .prepare_production_shipout()
            .expect("rootless production page preflights");
        history
            .commit_prepared_shipout()
            .expect("rootless production page commits");
        if page == 31 {
            warmed_pages = Some(history.pool.chunks.page_count());
        }
    }
    assert_eq!(
        history.pool.chunks.page_count(),
        warmed_pages.expect("warm high water sampled"),
        "uncheckpointed page payload reuses fixed pool pages"
    );
    assert_eq!(history.current().counters().page_regions_started, 257);
    assert_eq!(history.current().counters().page_regions_dropped, 256);
}

#[test]
fn production_heldover_moves_a_self_contained_successor_envelope() {
    let mut history = PageRegionHistory::default();
    let _shipped_prefix = publish_nodes(&mut history.nodes_mut(), [kern(1), kern(2)]);
    history.arm_output_successor_build();
    let heldover = publish_nodes(&mut history.nodes_mut(), [kern(7), kern(8)]);
    let heldover_address = history
        .nodes_mut()
        .list(heldover)
        .expect("heldover list")
        .get(0)
        .map(std::ptr::from_ref)
        .expect("heldover address");
    {
        let (mut nodes, builder) = history.parts_mut();
        builder.prepend_contributions(&mut nodes, heldover);
    }

    history
        .prepare_production_shipout()
        .expect("self-contained heldover preflights");
    history
        .commit_prepared_shipout()
        .expect("self-contained heldover commits");

    let contribution = history.builder().payload_roots().contribution;
    assert_eq!(
        history
            .nodes_mut()
            .span_list(contribution)
            .expect("moved heldover")
            .get(0)
            .map(std::ptr::from_ref),
        Some(heldover_address)
    );
    let counters = history.current().counters();
    assert_eq!(counters.held_over_envelopes_moved, 1);
    assert_eq!(counters.held_over_nodes_copied, 0);
}

#[test]
fn production_heldover_copies_only_the_interleaved_prefix_closure() {
    let mut history = PageRegionHistory::default();
    let heldover = publish_nodes(&mut history.nodes_mut(), [kern(9), kern(10)]);
    let old_address = history
        .nodes_mut()
        .list(heldover)
        .expect("prefix heldover list")
        .get(0)
        .map(std::ptr::from_ref)
        .expect("prefix heldover address");
    history.arm_output_successor_build();
    {
        let (mut nodes, builder) = history.parts_mut();
        builder.prepend_contributions(&mut nodes, heldover);
    }

    history
        .prepare_production_shipout()
        .expect("interleaved heldover selects copy fallback");
    history
        .commit_prepared_shipout()
        .expect("interleaved heldover commits");

    let contribution = history.builder().payload_roots().contribution;
    assert_ne!(
        history
            .nodes_mut()
            .span_list(contribution)
            .expect("copied heldover")
            .get(0)
            .map(std::ptr::from_ref),
        Some(old_address)
    );
    let counters = history.current().counters();
    assert_eq!(counters.held_over_envelopes_moved, 0);
    assert_eq!(counters.held_over_nodes_copied, 2);
}
