use super::{
    PageListId, PageListSpan, PageMaterialActiveListBuilder, PageMaterialArena, PageMaterialRegion,
};
use crate::fork_arena::ForkArenaError;
use crate::glue::Order;
use crate::node::{BoxLr, BoxNode, BoxNodeFields, Node, Sign};
use crate::node_region::NodePool;
use crate::node_sequence::SemanticSequenceIdentity;
use crate::scaled::{GlueSetRatio, Scaled};
use umber_hot_core_allocator::{AllocationMeasurement, scope, thread_measurement};

macro_rules! page_arena {
    ($arena:ident, $pool:ident, $state:ident, $bytes:expr) => {
        let mut $pool = NodePool::with_chunk_bytes($bytes);
        let mut $state = PageMaterialRegion::new(&mut $pool);
        let mut $arena = PageMaterialArena::new(&mut $pool, &mut $state);
    };
}

type PageMaterialNode = Node<PageListId>;

fn penalties(values: &[i32]) -> Vec<PageMaterialNode> {
    values.iter().copied().map(Node::Penalty).collect()
}

fn identity(nodes: &[PageMaterialNode]) -> SemanticSequenceIdentity {
    SemanticSequenceIdentity::from_nodes(nodes)
}

fn resolved(arena: &PageMaterialArena, list: PageListId) -> Vec<PageMaterialNode> {
    arena
        .list(list)
        .expect("live page list")
        .iter()
        .cloned()
        .collect()
}

fn boxed(children: PageListId) -> PageMaterialNode {
    Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(10),
        height: Scaled::from_raw(20),
        depth: Scaled::from_raw(3),
        shift: Scaled::from_raw(0),
        box_lr: BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }))
}

#[test]
fn warmed_resident_slot_construction_is_allocation_free() {
    const ALLOCATION_OWNER: usize = 15;
    const NODES: usize = 4_096;

    page_arena!(arena, pool, state, 512);
    let empty = arena.operation_mark();
    let mut builder = PageMaterialActiveListBuilder::vacant();
    arena.open_active_list(&mut builder).expect("warm builder");
    for penalty in 0..NODES as i32 {
        arena
            .push_active_list(&mut builder, Node::Penalty(penalty))
            .expect("warm resident slot");
    }
    let _ = arena
        .finalize_active_list(&mut builder)
        .expect("finalize warm list");
    arena
        .restore_operation(empty)
        .expect("return warmed resident slots");

    let before = thread_measurement(ALLOCATION_OWNER);
    let list = {
        let _scope = scope(ALLOCATION_OWNER);
        arena
            .open_active_list(&mut builder)
            .expect("measured builder");
        for penalty in 0..NODES as i32 {
            arena
                .push_active_list(&mut builder, Node::Penalty(penalty))
                .expect("measured resident slot");
        }
        arena
            .finalize_active_list(&mut builder)
            .expect("finalize measured list")
    };
    let after = thread_measurement(ALLOCATION_OWNER);

    assert_eq!(
        AllocationMeasurement {
            calls: after.calls.saturating_sub(before.calls),
            requested_bytes: after.requested_bytes.saturating_sub(before.requested_bytes),
        },
        AllocationMeasurement::default()
    );
    assert_eq!(arena.list(list).expect("measured list view").len(), NODES);
    assert_eq!(arena.counters().source_nodes_copied, 0);
}

#[test]
fn page_root_readmission_rejects_rollback_and_reused_pool_slots() {
    page_arena!(arena, pool, state, 32);
    let operation = arena.operation_mark();
    let source = arena
        .publish_owned(penalties(&(0..128).collect::<Vec<_>>()))
        .expect("publish cursor source");
    let span = arena.admit_span(source).expect("admit cursor source");
    let _tail = arena
        .span_tail_chunk(span)
        .expect("resolve direct tail")
        .expect("source is nonempty");
    arena
        .restore_operation(operation)
        .expect("rollback cursor source");
    let replacement = arena
        .publish_owned(penalties(&(128..256).collect::<Vec<_>>()))
        .expect("reuse rolled-back pool slots");
    let warmed_reuse_bytes = arena.allocated_heap_bytes();

    assert_eq!(replacement.len(), source.len());
    assert_eq!(
        arena.admit_span(source).map(|_| ()),
        Err(ForkArenaError::InvalidRange),
        "a stale integer root cannot be readmitted over the replacement incarnation"
    );
    arena
        .restore_operation(operation)
        .expect("rollback first replacement");
    let second_replacement = arena
        .publish_owned(penalties(&(256..384).collect::<Vec<_>>()))
        .expect("reuse the same warmed pool slots again");
    assert_eq!(second_replacement.len(), source.len());
    assert_eq!(
        arena.allocated_heap_bytes(),
        warmed_reuse_bytes,
        "repeated rollback and incarnation-safe slot reuse reaches a fixed plateau"
    );
}

#[test]
fn disabled_demand_performs_no_semantic_hash_work() {
    page_arena!(arena, pool, state, 32);
    let list = arena
        .publish_owned(penalties(&[10, 20, 30]))
        .expect("publish");

    assert_eq!(arena.semantic_hash_work(), 0);
    assert_eq!(arena.semantic_summary_work(), 0);
    assert_eq!(list.semantic_identity(), None);
    assert_eq!(arena.counters().source_nodes_copied, 0);
    assert_eq!(arena.counters().new_semantic_nodes, 3);
}

#[test]
fn parent_nodes_reject_foreign_region_children_without_partial_publication() {
    page_arena!(local, local_pool, local_state, 32);
    local.enable_semantic_identity();
    let before_hash_work = local.semantic_hash_work();
    page_arena!(foreign, foreign_pool, foreign_state, 32);
    let foreign_child = foreign
        .publish_owned(penalties(&[91]))
        .expect("foreign child");

    let result = local.publish_owned([
        Node::Penalty(1),
        Node::Disc {
            kind: crate::node::DiscKind::Discretionary,
            pre: foreign_child,
            post: PageListId::empty(),
            replace: PageListId::empty(),
            physical_replace_count: 0,
        },
    ]);

    assert_eq!(
        result,
        Err(crate::fork_arena::ForkArenaError::InvalidRegion)
    );
    assert_eq!(local.len(), 0);
    assert_eq!(local.counters().source_nodes_copied, 0);
    assert_eq!(local.semantic_hash_work(), before_hash_work);
}

#[test]
fn parent_nodes_reject_stale_same_region_children_without_partial_publication() {
    page_arena!(arena, pool, state, 32);
    arena.enable_semantic_identity();
    let boundary = arena.operation_mark();
    let stale_child = arena
        .publish_owned(penalties(&[91]))
        .expect("temporary child");
    arena
        .restore_operation(boundary)
        .expect("retire temporary child");
    let before = arena.counters();

    let result = arena.publish_owned([
        Node::Penalty(1),
        Node::Disc {
            kind: crate::node::DiscKind::Discretionary,
            pre: stale_child,
            post: PageListId::empty(),
            replace: PageListId::empty(),
            physical_replace_count: 0,
        },
    ]);

    assert_eq!(
        result,
        Err(crate::fork_arena::ForkArenaError::InvalidRegion)
    );
    assert_eq!(arena.len(), 0);
    assert_eq!(
        arena.counters().source_nodes_copied,
        before.source_nodes_copied
    );
    assert_eq!(arena.semantic_hash_work(), before.identity_nodes_hashed);
}

#[test]
fn disabled_demand_keeps_range_and_composition_identity_work_at_zero() {
    page_arena!(
        arena,
        pool,
        region,
        std::mem::size_of::<Option<PageMaterialNode>>() * 8
    );
    let whole = arena
        .publish_owned(penalties(&(0..128).collect::<Vec<_>>()))
        .expect("publish");
    let middle = arena
        .slice_sequence(whole, 3..125, &mut Vec::new())
        .expect("middle slice");
    let composed = arena.compose_sequences(&[middle, middle]).expect("compose");
    let mut builder = PageMaterialActiveListBuilder::vacant();
    arena.open_active_list(&mut builder).expect("open builder");
    arena
        .append_range_to_active_list(&mut builder, composed, 5..239)
        .expect("append range");
    let _ = arena
        .finalize_active_list(&mut builder)
        .expect("finalize builder");

    assert_eq!(arena.semantic_hash_work(), 0);
    assert_eq!(arena.semantic_summary_work(), 0);
    assert_eq!(arena.counters().identity_nodes_hashed, 0);
    assert_eq!(arena.counters().identity_summaries_combined, 0);
    assert_eq!(arena.counters().source_nodes_copied, 356);
}

#[test]
fn active_list_preserves_disabled_demand_and_counts_shared_input_copies() {
    page_arena!(arena, pool, state, 32);
    let source = arena.publish_owned(penalties(&[10, 20])).expect("source");
    let source_address = arena
        .list(source)
        .expect("live source")
        .testing_node_address(0)
        .expect("source node");
    let mut builder = PageMaterialActiveListBuilder::vacant();

    arena.open_active_list(&mut builder).expect("open builder");
    arena
        .append_to_active_list(&mut builder, source)
        .expect("append source coordinates");
    arena
        .construct_active_list(&mut builder, |slot| slot.penalty(30))
        .expect("construct new semantic node");
    let composed = arena
        .finalize_active_list(&mut builder)
        .expect("finalize builder");

    assert_eq!(resolved(&arena, composed), penalties(&[10, 20, 30]));
    assert_eq!(composed.semantic_identity(), None);
    assert_eq!(arena.semantic_hash_work(), 0);
    assert_eq!(arena.counters().new_semantic_nodes, 3);
    assert_eq!(arena.counters().source_nodes_copied, 2);
    assert_ne!(
        arena
            .list(composed)
            .expect("live composed list")
            .testing_node_address(0)
            .expect("composed source node"),
        source_address
    );
}

#[test]
fn demand_enabled_shared_append_keeps_copied_tail_summary_open_for_generated_suffix() {
    page_arena!(arena, pool, state, 32);
    arena.enable_semantic_identity();
    let source = arena.publish_owned(penalties(&[10, 20])).expect("source");
    let mut builder = PageMaterialActiveListBuilder::vacant();

    arena.open_active_list(&mut builder).expect("open builder");
    arena
        .append_to_active_list(&mut builder, source)
        .expect("copy summarized source");
    arena
        .construct_active_list(&mut builder, |slot| slot.penalty(30))
        .expect("generated suffix extends a summarized copied block");
    let composed = arena
        .finalize_active_list(&mut builder)
        .expect("finalize builder");

    let expected = penalties(&[10, 20, 30]);
    assert_eq!(resolved(&arena, composed), expected);
    assert_eq!(
        composed.semantic_identity(),
        Some(identity(&expected).raw())
    );
    assert_eq!(arena.counters().source_nodes_copied, 2);
}

#[test]
fn demand_enabled_shared_append_preserves_nested_children_and_exact_identity() {
    page_arena!(arena, pool, state, 64);
    arena.enable_semantic_identity();
    let children = arena.publish_owned(penalties(&[11, 12])).expect("children");
    let source_nodes = vec![boxed(children), Node::Penalty(20)];
    let source = arena
        .publish_owned(source_nodes.clone())
        .expect("shared parent source");
    let source_identity = source.semantic_identity();
    let before = arena.counters();
    let mut builder = PageMaterialActiveListBuilder::vacant();

    arena.open_active_list(&mut builder).expect("open builder");
    arena
        .append_to_active_list(&mut builder, source)
        .expect("copy summarized parent source");
    let copied = arena
        .finalize_active_list(&mut builder)
        .expect("finalize copied parent source");
    let after = arena.counters();

    assert_eq!(resolved(&arena, copied), source_nodes);
    assert_eq!(copied.semantic_identity(), source_identity);
    assert_eq!(source.semantic_identity(), source_identity);
    assert_eq!(after.source_nodes_copied - before.source_nodes_copied, 2);
    assert_eq!(after.whole_payload_copies - before.whole_payload_copies, 2);
    let Node::HList(copied_box) = arena
        .list(copied.list())
        .expect("copied parent list")
        .get(0)
        .cloned()
        .expect("copied parent")
    else {
        panic!("copied parent lost box shape");
    };
    assert_eq!(copied_box.children, children);
    assert_eq!(resolved(&arena, copied_box.children), penalties(&[11, 12]));
}

#[test]
fn unique_suffix_scaling_is_exact_at_one_sixty_four_and_four_thousand_ninety_six() {
    for size in [1_usize, 64, 4_096] {
        page_arena!(arena, pool, state, 512);
        let mut list = PageListSpan::empty();
        for value in 0..size {
            let suffix = arena
                .publish_owned_unique([Node::Penalty(value as i32)])
                .expect("fresh unique suffix");
            list = arena
                .append_unique_to_span(list, suffix)
                .expect("constant-work unique suffix splice");
        }
        let counters = arena.counters();

        assert_eq!(list.len(), size);
        assert_eq!(arena.list(list.list()).expect("direct list").len(), size);
        assert_eq!(counters.new_semantic_nodes, size as u64);
        assert_eq!(counters.direct_blocks_allocated, size as u64);
        assert_eq!(counters.source_nodes_copied, 0);
        assert_eq!(counters.partial_edge_nodes_copied, 0);
        assert_eq!(counters.overlapping_nodes_copied, 0);
        eprintln!(
            "DIRECT_UNIQUE_SUFFIX_SCALE nodes={size} blocks={} copies=0",
            counters.direct_blocks_allocated
        );
    }
}

#[test]
fn shared_source_scaling_clones_each_node_directly_at_required_sizes() {
    const ALLOCATION_OWNER: usize = 15;

    for size in [1_usize, 64, 4_096] {
        page_arena!(arena, pool, state, 512);
        arena.enable_semantic_identity();
        let source_nodes = penalties(&(0..size as i32).collect::<Vec<_>>());
        let source = arena
            .publish_owned(source_nodes.clone())
            .expect("shared source");
        let source_identity = source.semantic_identity();
        let source_address = arena
            .list(source.list())
            .expect("source list")
            .testing_node_address(0)
            .expect("source node");
        let source_mark = arena.operation_mark();

        let copy = |arena: &mut PageMaterialArena<'_>| {
            let mut builder = PageMaterialActiveListBuilder::vacant();
            arena.open_active_list(&mut builder).expect("open builder");
            arena
                .construct_active_list(&mut builder, |slot| slot.penalty(-1))
                .expect("exact splice prefix");
            arena
                .append_to_active_list(&mut builder, source)
                .expect("copy shared source");
            arena
                .finalize_active_list(&mut builder)
                .expect("finalize copied source")
        };

        let _warm = copy(&mut arena);
        arena
            .restore_operation(source_mark)
            .expect("restore warmed copy");
        let before = arena.counters();
        let allocation_before = thread_measurement(ALLOCATION_OWNER);
        let copied = {
            let _scope = scope(ALLOCATION_OWNER);
            copy(&mut arena)
        };
        let allocation_after = thread_measurement(ALLOCATION_OWNER);
        let after = arena.counters();

        let mut expected = vec![Node::Penalty(-1)];
        expected.extend(source_nodes.iter().cloned());
        assert_eq!(resolved(&arena, copied.list()), expected);
        assert_eq!(copied.semantic_identity(), Some(identity(&expected).raw()));
        assert_eq!(source.semantic_identity(), source_identity);
        assert_eq!(
            arena
                .list(source.list())
                .expect("source remains live")
                .testing_node_address(0),
            Some(source_address)
        );
        assert_eq!(
            after.source_nodes_copied - before.source_nodes_copied,
            size as u64
        );
        assert_eq!(
            after.whole_payload_copies - before.whole_payload_copies,
            size as u64
        );
        assert_eq!(after.new_semantic_nodes - before.new_semantic_nodes, 1);
        let allocation_calls = allocation_after.calls - allocation_before.calls;
        assert_eq!(allocation_calls, 0);
        assert_eq!(
            allocation_after.requested_bytes - allocation_before.requested_bytes,
            0
        );
        eprintln!(
            "PAGE_SHARED_COPY_SCALE nodes={size} copied_nodes={} allocation_calls={allocation_calls} allocation_bytes={}",
            after.source_nodes_copied - before.source_nodes_copied,
            allocation_after.requested_bytes - allocation_before.requested_bytes,
        );
    }
}

#[test]
fn source_copy_counter_has_a_real_negative_control() {
    page_arena!(arena, pool, state, 32);
    let source = arena.publish_owned(penalties(&[4, 5])).expect("source");
    let copied = arena
        .publish_source_copy(source)
        .expect("explicit compatibility copy");

    assert_eq!(resolved(&arena, copied), penalties(&[4, 5]));
    assert_eq!(arena.counters().source_nodes_copied, 2);
    assert_eq!(arena.counters().new_semantic_nodes, 4);
}

#[test]
fn destination_construction_is_warmed_and_linear_across_chunks() {
    page_arena!(arena, pool, state, 512);
    assert_eq!(core::mem::size_of::<Node>(), 168);
    assert_eq!(core::mem::size_of::<PageListId>(), 40);
    assert_eq!(
        core::mem::size_of::<crate::fork_arena::ArenaListId<crate::fork_arena::PageMaterialLane>>(),
        32
    );
    assert_eq!(core::mem::size_of::<tex_dense_arena::LogicalPosition>(), 16);

    fn construct(arena: &mut PageMaterialArena<'_>, count: usize) {
        let mut builder = PageMaterialActiveListBuilder::vacant();
        arena.open_active_list(&mut builder).expect("open builder");
        for penalty in 0..count {
            arena
                .construct_active_list(&mut builder, |slot| slot.penalty(penalty as i32))
                .expect("construct resident node");
        }
        let list = arena
            .finalize_active_list(&mut builder)
            .expect("finalize constructed list");
        assert_eq!(list.len(), count);
    }

    let empty = arena.operation_mark();
    construct(&mut arena, 4_096);
    arena.restore_operation(empty).expect("restore warmup");
    let warmed_bytes = arena.allocated_heap_bytes();

    for count in [1, 4_096] {
        let mark = arena.operation_mark();
        let before = arena.counters();
        construct(&mut arena, count);
        let after = arena.counters();
        assert_eq!(arena.allocated_heap_bytes(), warmed_bytes);
        assert_eq!(after.whole_payload_moves - before.whole_payload_moves, 0);
        assert_eq!(after.whole_payload_copies - before.whole_payload_copies, 0);
        assert_eq!(
            after.destination_values_constructed - before.destination_values_constructed,
            count as u64
        );
        let blocks = after.direct_blocks_allocated - before.direct_blocks_allocated;
        assert_eq!(
            blocks,
            count.div_ceil(arena.payload_chunk_capacity()) as u64
        );
        arena.restore_operation(mark).expect("restore measured row");
        assert_eq!(arena.allocated_heap_bytes(), warmed_bytes);
    }
}

#[test]
fn generated_line_edges_preserve_the_selected_source_subrange_addresses() {
    page_arena!(arena, pool, state, 32);
    let source = arena
        .publish_owned(penalties(&[10, 20, 30]))
        .expect("source");
    let selected_addresses = [1, 2].map(|index| {
        arena
            .list(source)
            .expect("live source")
            .testing_node_address(index)
            .expect("selected source node")
    });
    let mut line = PageMaterialActiveListBuilder::vacant();
    arena.open_active_list(&mut line).expect("open line");
    arena
        .push_active_list(&mut line, Node::Penalty(1))
        .expect("generated left edge");
    arena
        .append_range_to_active_list(&mut line, source, 1..3)
        .expect("borrowed line interior");
    arena
        .push_active_list(&mut line, Node::Penalty(2))
        .expect("generated right edge");
    let line = arena
        .finalize_active_list(&mut line)
        .expect("finalize line");

    assert_eq!(resolved(&arena, line), penalties(&[1, 20, 30, 2]));
    let line_view = arena.list(line).expect("live line");
    assert_ne!(
        [1, 2].map(|index| {
            line_view
                .testing_node_address(index)
                .expect("borrowed line node")
        }),
        selected_addresses
    );
    assert_eq!(arena.counters().source_nodes_copied, 2);
    assert_eq!(arena.counters().new_semantic_nodes, 5);
}

#[test]
fn overlapping_checked_span_composition_counts_its_unavoidable_copy() {
    page_arena!(arena, pool, state, 4096);
    let source = arena
        .publish_owned(penalties(&(0..64).collect::<Vec<_>>()))
        .expect("source");
    let span = arena.admit_span(source).expect("admit source once");
    let selected_addresses = (7..57)
        .map(|index| {
            arena
                .span_node_cursor(span)
                .expect("checked source remains live")
                .testing_node_address(index)
                .expect("selected node")
        })
        .collect::<Vec<_>>();
    let bytes_before = arena.allocated_heap_bytes();
    let copies_before = arena.counters().source_nodes_copied;

    let mut retained = PageMaterialActiveListBuilder::vacant();
    arena
        .open_active_list(&mut retained)
        .expect("open retained list");
    arena
        .append_span_range_to_active_list(&mut retained, span, 7..57)
        .expect("retain checked span");
    let retained = arena
        .finalize_active_span(&mut retained)
        .expect("finalize retained list");
    let composed = arena
        .compose_spans(&[span, retained])
        .expect("checked roots compose without re-admission");
    let retained = arena
        .slice_span(composed, span.len()..composed.len())
        .expect("checked composite slices without re-admission");

    let retained_count = arena
        .span_node_cursor(retained)
        .expect("retained list")
        .len();
    assert_eq!(retained_count, selected_addresses.len());
    assert_eq!(arena.counters().source_nodes_copied, copies_before + 100);
    assert!(arena.allocated_heap_bytes() >= bytes_before);
}

#[test]
fn checked_span_rejects_foreign_owner_before_publishing_a_range() {
    page_arena!(source, source_pool, source_state, 64);
    let list = source.publish_owned(penalties(&[1, 2])).expect("source");
    let span = source.admit_span(list).expect("checked source");

    page_arena!(foreign, foreign_pool, foreign_state, 64);
    assert!(foreign.span_node_cursor(span).is_err());
    let before = foreign.counters();
    let mut builder = PageMaterialActiveListBuilder::vacant();
    foreign
        .open_active_list(&mut builder)
        .expect("open builder");
    assert!(
        foreign
            .append_span_range_to_active_list(&mut builder, span, 0..1)
            .is_err()
    );
    foreign
        .rollback_active_list(&mut builder)
        .expect("failed foreign append rolls back an empty suffix");
    assert_eq!(
        foreign.counters().new_semantic_nodes,
        before.new_semantic_nodes
    );
    assert_eq!(
        foreign.counters().source_nodes_copied,
        before.source_nodes_copied
    );
}

#[test]
fn checked_span_rejects_stale_descriptor_after_operation_rollback() {
    page_arena!(arena, pool, state, 64);
    let boundary = arena.operation_mark();
    let list = arena.publish_owned(penalties(&[1, 2])).expect("source");
    let span = arena.admit_span(list).expect("checked source");
    arena
        .restore_operation(boundary)
        .expect("discard checked source suffix");

    assert!(arena.span_node_cursor(span).is_err());
    let before = arena.counters();
    let mut builder = PageMaterialActiveListBuilder::vacant();
    arena.open_active_list(&mut builder).expect("open builder");
    assert!(
        arena
            .append_span_range_to_active_list(&mut builder, span, 0..1)
            .is_err()
    );
    arena
        .rollback_active_list(&mut builder)
        .expect("failed stale append rolls back an empty suffix");
    assert_eq!(
        arena.counters().new_semantic_nodes,
        before.new_semantic_nodes
    );
    assert_eq!(
        arena.counters().source_nodes_copied,
        before.source_nodes_copied
    );
}

#[test]
fn long_direct_span_survives_checkpoint_rejection_and_operation_rollback_stales_only_the_suffix() {
    page_arena!(arena, pool, state, 1);
    let retained = arena
        .publish_owned(penalties(&(0..4_096).collect::<Vec<_>>()))
        .expect("long retained root");
    let retained_span = arena.admit_span(retained).expect("admit long root");
    let checkpoint = arena
        .seal_boundary()
        .and_then(|boundary| arena.checkpoint_mark(boundary))
        .expect("retained checkpoint");
    let prior = arena
        .publish_owned(penalties(&[5_000, 5_001]))
        .expect("accepted suffix");

    arena
        .begin_checkpoint_candidate(checkpoint)
        .expect("begin candidate");
    let rejected = arena
        .publish_owned(penalties(&[9_000]))
        .expect("candidate suffix");
    let settlement = arena.seal_boundary().expect("candidate boundary");
    arena
        .reject_checkpoint_candidate(settlement)
        .expect("restore accepted suffix");

    assert_eq!(
        arena.span_list(retained_span).expect("retained span").len(),
        4_096
    );
    assert!(arena.contains(prior));
    assert!(!arena.contains(rejected));

    let operation = arena.operation_mark();
    let temporary = arena
        .publish_owned(penalties(&[7_000, 7_001]))
        .expect("operation suffix");
    let temporary_span = arena.admit_span(temporary).expect("admit operation suffix");
    arena
        .restore_operation(operation)
        .expect("rollback operation suffix");

    assert!(arena.span_list(temporary_span).is_err());
    assert_eq!(
        arena.span_list(retained_span).expect("retained span").len(),
        4_096
    );
    let bytes_before_visit = arena.allocated_heap_bytes();
    let mut expected = 0_i32;
    arena
        .span_node_cursor(retained_span)
        .expect("retained span cursor")
        .for_each(|node| {
            assert_eq!(node, crate::NodeView::Penalty(expected));
            expected += 1;
        });
    assert_eq!(expected, 4_096);
    assert_eq!(arena.allocated_heap_bytes(), bytes_before_visit);
    assert_eq!(arena.counters().source_nodes_copied, 0);
}

#[test]
fn active_list_concatenates_maintained_semantic_identity() {
    page_arena!(arena, pool, state, 32);
    arena.enable_semantic_identity();
    let source_nodes = penalties(&[1, 2]);
    let source = arena
        .publish_owned(source_nodes.clone())
        .expect("source list");
    let mut builder = PageMaterialActiveListBuilder::vacant();

    arena.open_active_list(&mut builder).expect("open builder");
    arena
        .append_to_active_list(&mut builder, source)
        .expect("append source identity");
    arena
        .push_active_list(&mut builder, Node::Penalty(3))
        .expect("append new node identity");
    let result = arena
        .finalize_active_list(&mut builder)
        .expect("finalize builder");
    let expected_nodes = penalties(&[1, 2, 3]);

    assert_eq!(
        result.semantic_identity(),
        Some(identity(&expected_nodes).raw())
    );
    assert_eq!(
        arena.semantic_hash_work(),
        5,
        "the two generated source copies are each hashed once before the new suffix"
    );
    assert_eq!(arena.counters().source_nodes_copied, 2);
}

#[test]
fn identity_is_preserved_across_build_split_and_compose() {
    page_arena!(arena, pool, state, 32);
    arena.enable_semantic_identity();
    let nodes = penalties(&[1, 2, 3, 4]);
    let whole = arena
        .publish_owned(nodes.clone())
        .expect("publish semantic list");
    let left_nodes = &nodes[..2];
    let right_nodes = &nodes[2..];
    let left = arena
        .slice_sequence(whole, 0..2, &mut Vec::new())
        .expect("split left");
    let right = arena
        .slice_sequence(whole, 2..4, &mut Vec::new())
        .expect("split right");
    let recomposed = arena.compose_sequences(&[left, right]).expect("compose");

    assert_eq!(whole.semantic_identity(), Some(identity(&nodes).raw()));
    assert_eq!(left.semantic_identity(), Some(identity(left_nodes).raw()));
    assert_eq!(right.semantic_identity(), Some(identity(right_nodes).raw()));
    assert_eq!(recomposed.semantic_identity(), whole.semantic_identity());
    assert_eq!(resolved(&arena, recomposed), nodes);
    assert_eq!(arena.semantic_hash_work(), 4);
    assert_eq!(arena.counters().source_nodes_copied, 2);
}

#[test]
fn long_middle_slice_uses_summaries_and_explicit_copy_hashes_each_node_once() {
    const CHUNK_VALUES: usize = 8;
    page_arena!(arena, pool, region, 32 * CHUNK_VALUES);
    arena.enable_semantic_identity();
    let nodes = penalties(&(0..1024).collect::<Vec<_>>());
    let whole = arena
        .publish_owned(nodes.clone())
        .expect("publish long list");
    let hash_before = arena.semantic_hash_work();
    let summaries_before = arena.semantic_summary_work();

    let middle = arena
        .slice_sequence(whole, 3..1021, &mut Vec::new())
        .expect("slice long middle");

    let hashed = arena.semantic_hash_work() - hash_before;
    let summaries = arena.semantic_summary_work() - summaries_before;
    assert_eq!(
        middle.semantic_identity(),
        Some(identity(&nodes[3..1021]).raw())
    );
    assert!(
        hashed <= (2 * CHUNK_VALUES) as u64,
        "only two partial boundary chunks may hash payload: {hashed}"
    );
    assert!(hashed < middle.len() as u64);
    assert!(summaries > 100, "long interior must use chunk summaries");

    let append_hash_before = arena.semantic_hash_work();
    let append_summaries_before = arena.semantic_summary_work();
    let mut builder = PageMaterialActiveListBuilder::vacant();
    arena.open_active_list(&mut builder).expect("open builder");
    arena
        .append_range_to_active_list(&mut builder, whole, 3..1021)
        .expect("append long middle");
    let appended = arena
        .finalize_active_list(&mut builder)
        .expect("finalize retained range");
    assert_eq!(appended.semantic_identity(), middle.semantic_identity());
    assert_eq!(
        arena.semantic_hash_work() - append_hash_before,
        appended.len() as u64,
        "explicit shared copying hashes each copied node once and never revisits a prefix"
    );
    assert_eq!(
        arena.semantic_summary_work(),
        append_summaries_before,
        "the explicit copy builds summarized destination blocks from its one-pass hashes"
    );

    let compose_hash_before = arena.semantic_hash_work();
    let doubled = arena
        .compose_sequences(&[middle, appended])
        .expect("compose summarized lists");
    let mut expected = nodes[3..1021].to_vec();
    expected.extend_from_slice(&nodes[3..1021]);
    assert_eq!(doubled.semantic_identity(), Some(identity(&expected).raw()));
    assert_eq!(arena.semantic_hash_work(), compose_hash_before);
    assert_eq!(arena.counters().source_nodes_copied, 2_036);
}

#[test]
fn multi_range_slice_identity_is_independent_of_descriptor_boundaries() {
    const CHUNK_VALUES: usize = 8;
    page_arena!(
        arena,
        pool,
        region,
        std::mem::size_of::<Option<PageMaterialNode>>() * CHUNK_VALUES
    );
    arena.enable_semantic_identity();
    let nodes = penalties(&(0..1024).collect::<Vec<_>>());
    let whole = arena.publish_owned(nodes.clone()).expect("publish");
    let left = arena
        .slice_sequence(whole, 0..300, &mut Vec::new())
        .expect("left");
    let right = arena
        .slice_sequence(whole, 700..1024, &mut Vec::new())
        .expect("right");
    let composite = arena.compose_sequences(&[left, right]).expect("composite");
    let hash_before = arena.semantic_hash_work();
    let selected = arena
        .slice_sequence(composite, 2..622, &mut Vec::new())
        .expect("cross-range selection");
    let mut expected = nodes[2..300].to_vec();
    expected.extend_from_slice(&nodes[700..1022]);

    assert_eq!(resolved(&arena, selected), expected);
    assert_eq!(
        selected.semantic_identity(),
        Some(identity(&expected).raw())
    );
    assert!(
        arena.semantic_hash_work() - hash_before <= selected.len() as u64,
        "shared copied chunks may require their summaries to be rebuilt: {}",
        arena.semantic_hash_work() - hash_before
    );
    assert_eq!(arena.counters().source_nodes_copied, 324);
}

#[test]
fn partial_operation_restore_restores_payload_chunk_summary() {
    page_arena!(
        arena,
        pool,
        region,
        std::mem::size_of::<Option<PageMaterialNode>>() * 8
    );
    arena.enable_semantic_identity();
    let retained_nodes = penalties(&[1, 2, 3]);
    let retained = arena
        .publish_owned(retained_nodes.clone())
        .expect("retained prefix");
    let operation = arena.operation_mark();
    let rejected = arena
        .publish_owned(penalties(&[8, 9]))
        .expect("operation suffix");
    assert!(arena.contains(rejected));

    arena
        .restore_operation(operation)
        .expect("restore partial payload tail");
    assert!(!arena.contains(rejected));
    let restored = arena
        .slice_sequence(retained, 1..3, &mut Vec::new())
        .expect("slice restored prefix");
    assert_eq!(
        restored.semantic_identity(),
        Some(identity(&retained_nodes[1..]).raw())
    );
    assert_eq!(arena.counters().source_nodes_copied, 0);
}

#[test]
fn accepted_identity_survives_reject_accept_and_prune() {
    page_arena!(arena, pool, state, 32);
    arena.enable_semantic_identity();
    let prefix = arena
        .publish_owned(penalties(&[7, 8]))
        .expect("accepted prefix");
    let checkpoint = arena
        .seal_boundary()
        .and_then(|boundary| arena.checkpoint_mark(boundary))
        .expect("checkpoint");
    let prior_nodes = penalties(&[30, 31]);
    let prior = arena
        .publish_owned(prior_nodes.clone())
        .expect("prior accepted suffix");
    let prior_identity = prior.semantic_identity();

    arena
        .begin_checkpoint_candidate(checkpoint)
        .expect("begin rejected candidate");
    let rejected = arena
        .publish_owned(penalties(&[90]))
        .expect("rejected list");
    assert_ne!(rejected.semantic_identity(), prior_identity);
    let boundary = arena.seal_boundary().expect("seal rejected candidate");
    arena
        .reject_checkpoint_candidate(boundary)
        .expect("reject candidate");
    assert_eq!(prior.semantic_identity(), prior_identity);
    assert_eq!(resolved(&arena, prior), prior_nodes);
    assert_eq!(resolved(&arena, prefix), penalties(&[7, 8]));

    arena
        .begin_checkpoint_candidate(checkpoint)
        .expect("begin accepted candidate");
    let replacement_nodes = penalties(&[11, 12, 13]);
    let replacement = arena
        .publish_owned(replacement_nodes.clone())
        .expect("replacement list");
    let replacement_identity = replacement.semantic_identity();
    let boundary = arena.seal_boundary().expect("seal accepted candidate");
    arena
        .accept_checkpoint_candidate(boundary)
        .expect("accept candidate");
    assert_eq!(replacement.semantic_identity(), replacement_identity);
    assert_eq!(resolved(&arena, replacement), replacement_nodes);
    assert_eq!(arena.counters().source_nodes_copied, 0);
    assert!(arena.counters().obsolete_chunks_pruned > 0);
}

#[test]
fn unique_durable_move_preserves_recursive_addresses_without_copying() {
    page_arena!(arena, pool, region, 64);
    let leaf = arena.publish_owned([Node::Penalty(41)]).expect("page leaf");
    let root = arena.publish_owned([boxed(leaf)]).expect("page box");
    let durable = arena
        .copy_page_root_to_durable(root)
        .expect("durable owner");
    let Node::HList(box_node) = arena
        .durable_list(&durable)
        .expect("durable root")
        .get(0)
        .cloned()
        .expect("box node")
    else {
        panic!("durable root lost box shape");
    };
    let durable_leaf_address = arena
        .durable_child_list(&durable, box_node.children)
        .expect("durable leaf")
        .testing_node_address(0)
        .expect("leaf address");
    let before = arena.counters().source_nodes_copied;

    let mut durable = Some(durable);
    let moved = arena
        .move_durable_to_page_in_place(&mut durable)
        .expect("unique move");
    assert!(durable.is_none());
    let Node::HList(box_node) = arena
        .list(moved)
        .expect("moved root")
        .get(0)
        .cloned()
        .expect("moved box")
    else {
        panic!("moved root lost box shape");
    };
    assert_eq!(
        arena
            .list(box_node.children)
            .expect("moved leaf")
            .testing_node_address(0),
        Some(durable_leaf_address)
    );
    assert_eq!(arena.counters().source_nodes_copied, before);
}

#[test]
fn built_durable_copy_preserves_page_root_created_in_construction_suffix() {
    page_arena!(arena, pool, region, 64);
    let build = arena.begin_closure_build().expect("open box build");
    let retained = arena
        .publish_owned([Node::Penalty(51)])
        .expect("page-owned side effect");
    let built = arena
        .publish_owned([Node::Penalty(52)])
        .expect("built box root");

    let durable = arena
        .finish_built_page_root_to_durable_preserving_roots(
            build,
            built,
            [
                PageListId::empty(),
                PageListId::empty(),
                PageListId::empty(),
                retained,
            ],
        )
        .expect("retained-root fallback");

    assert_eq!(resolved(&arena, retained), [Node::Penalty(51)]);
    assert_eq!(
        arena
            .durable_list(&durable)
            .expect("durable result")
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        [Node::Penalty(52)]
    );
    arena
        .retire_durable(durable)
        .expect("retire durable result");
}

#[test]
fn durable_copy_is_recursive_and_counts_only_the_selected_closure() {
    page_arena!(arena, pool, region, 64);
    let leaf = arena.publish_owned([Node::Penalty(43)]).expect("page leaf");
    let root = arena.publish_owned([boxed(leaf)]).expect("page box");
    let durable = arena
        .copy_page_root_to_durable(root)
        .expect("durable owner");
    let before = arena.durable_transition_counters();

    let copied = arena.copy_durable_to_page(&durable).expect("TeX copy");
    assert_eq!(resolved(&arena, copied).len(), 1);
    assert_eq!(
        arena.durable_list(&durable).expect("source retained").len(),
        1
    );
    let after = arena.durable_transition_counters();
    assert_eq!(
        after.tex_copy_nodes_copied - before.tex_copy_nodes_copied,
        2
    );
    assert_eq!(
        after.node_closure_scan_nodes - before.node_closure_scan_nodes,
        2
    );
    arena.retire_durable(durable).expect("retire source owner");
}

#[test]
fn durable_lifetime_copies_preserve_enabled_semantic_identity() {
    page_arena!(arena, pool, region, 64);
    arena.enable_semantic_identity();
    let leaf = arena.publish_owned([Node::Penalty(45)]).expect("page leaf");
    let root = arena.publish_owned([boxed(leaf)]).expect("page box");
    let root_identity = root.semantic_identity();
    let durable = arena
        .copy_page_root_to_durable(root)
        .expect("durable owner");
    assert_eq!(durable.root().list().semantic_identity(), root_identity);

    let copied = arena
        .copy_history_preserved_to_page(&durable)
        .expect("history-preserving copy");
    assert_eq!(copied.semantic_identity(), root_identity);
    let Node::HList(box_node) = arena
        .list(copied)
        .expect("copied root")
        .get(0)
        .cloned()
        .expect("copied box")
    else {
        panic!("copied root lost box shape");
    };
    assert_eq!(
        box_node.children.semantic_identity(),
        leaf.semantic_identity()
    );
    arena.retire_durable(durable).expect("retire source owner");
}

#[test]
fn historical_durable_owner_copy_is_move_only_and_independent() {
    page_arena!(arena, pool, region, 64);
    let root = arena.publish_owned([Node::Penalty(47)]).expect("page root");
    let original = arena
        .copy_page_root_to_durable(root)
        .expect("original owner");
    let original_id = original.region_id();
    let before = arena.durable_transition_counters();

    let history = arena.copy_durable_owner(&original).expect("history owner");
    assert_ne!(history.region_id(), original_id);
    assert_eq!(arena.durable_list(&original).expect("original").len(), 1);
    assert_eq!(arena.durable_list(&history).expect("history").len(), 1);
    let after = arena.durable_transition_counters();
    assert_eq!(
        after.history_preservation_nodes_copied - before.history_preservation_nodes_copied,
        1
    );
    arena.retire_durable(original).expect("retire original");
    assert_eq!(
        arena
            .durable_list(&history)
            .expect("history survives")
            .len(),
        1
    );
    arena.retire_durable(history).expect("retire history");
}
