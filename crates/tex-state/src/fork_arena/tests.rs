use super::{ActiveListBuilder, ChunkPool, ForkArena, ForkArenaError};
use crate::node::Node;
use crate::node_arena::NodeCursor;

enum ActiveLane {}
enum PageLane {}

impl super::RegionValue<ActiveLane> for u32 {
    fn visit_region_lists(&self, _visit: &mut dyn FnMut(super::ArenaListId<ActiveLane>)) {}

    fn rebrand_region_lists(&mut self, _destination_arena: u32) {}
}

impl super::RegionValue<ActiveLane> for u64 {
    fn visit_region_lists(&self, _visit: &mut dyn FnMut(super::ArenaListId<ActiveLane>)) {}

    fn rebrand_region_lists(&mut self, _destination_arena: u32) {}
}

#[test]
fn coarse_pool_pages_hold_many_stable_chunks_and_reject_stale_keys() {
    let mut pool = ChunkPool::<u64>::with_chunk_bytes(32);
    assert_eq!(
        pool.chunk_capacity(),
        (32 / std::mem::size_of::<Option<u64>>()).max(1)
    );
    let mut keys = Vec::new();
    for value in 0..17_u64 {
        let key = pool.payload.allocate(7).expect("chunk allocation");
        pool.payload
            .append(key, 7, value, None)
            .expect("chunk append");
        keys.push(key);
    }
    assert_eq!(pool.page_count(), 2);
    assert_eq!(pool.payload.get(keys[0], 7, 0), Some(&0));
    let stale = keys[0];
    assert_eq!(pool.payload.release(stale, 7), Ok(1));
    let replacement = pool.payload.allocate(7).expect("reused chunk");
    assert_eq!(replacement.slot, stale.slot);
    assert_ne!(replacement.generation, stale.generation);
    assert_eq!(pool.payload.get(stale, 7, 0), None);
}

#[test]
fn block_byte_budgets_report_payload_and_metadata_overhead() {
    for bytes in [128, 512, 4_096] {
        let pool = ChunkPool::<u64>::with_chunk_bytes(bytes);
        let capacity = (bytes / std::mem::size_of::<Option<u64>>()).max(1);
        assert_eq!(pool.chunk_capacity(), capacity);
        assert_eq!(
            pool.physical_page_payload_bytes(),
            capacity * super::CHUNKS_PER_PAGE * std::mem::size_of::<Option<u64>>()
        );
        assert_eq!(
            pool.physical_page_metadata_bytes(),
            super::CHUNKS_PER_PAGE * pool.logical_block_metadata_bytes()
        );
        assert!(
            pool.physical_page_metadata_bytes() < pool.physical_page_payload_bytes(),
            "the supported block classes keep chain metadata below payload capacity"
        );
    }
}

#[test]
fn builder_drop_and_partial_operation_mark_truncate_without_payload_copy() {
    let mut pool = ChunkPool::<u32>::with_chunk_bytes(16);
    let mut arena = ForkArena::<u32, ActiveLane>::new();
    {
        let mut abandoned = arena.begin_builder(&mut pool).expect("builder");
        abandoned.push(1).expect("append");
        abandoned.push(2).expect("append");
    }
    assert_eq!(arena.counters().candidate_chunks_truncated, 1);

    let first = {
        let mut builder = arena.begin_builder(&mut pool).expect("builder");
        builder.push(3).expect("append");
        builder.push(4).expect("append");
        builder.seal().expect("list seal")
    };
    let operation = arena.operation_mark(&pool);
    let second = {
        let mut builder = arena.begin_builder(&mut pool).expect("builder");
        builder.push(5).expect("append");
        builder.seal().expect("list seal")
    };
    assert_eq!(
        arena.list(&pool, second).expect("second view").get(0),
        Some(&5)
    );
    arena
        .restore_operation(&mut pool, operation)
        .expect("partial rollback");
    assert_eq!(
        arena
            .list(&pool, second)
            .expect_err("rolled-back list must be stale"),
        ForkArenaError::InvalidRange
    );
    assert_eq!(
        arena
            .list(&pool, first)
            .expect("first survives")
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![3, 4]
    );
    assert_eq!(arena.counters().source_nodes_copied, 0);
}

#[test]
fn unique_whole_list_capability_splices_once_without_copying_payload() {
    let mut pool = ChunkPool::<u32>::with_chunk_bytes(16);
    let mut arena = ForkArena::<u32, ActiveLane>::new();
    let right = {
        let mut builder = arena.begin_builder(&mut pool).expect("right builder");
        builder.push(2).expect("right node");
        builder.push(3).expect("right node");
        builder.seal_unique().expect("unique right chain")
    };
    let right_root = right.root;
    let right_address = arena
        .list(&pool, right_root)
        .expect("right view")
        .get(0)
        .expect("right node") as *const u32;

    let mut destination = ActiveListBuilder::vacant();
    arena
        .open_active_list(&pool, &mut destination)
        .expect("destination builder");
    arena
        .push_active_list(&mut pool, &mut destination, 1)
        .expect("left node");
    arena
        .append_unique_active_list(&mut pool, &mut destination, right)
        .expect("consume unique chain");
    arena
        .finalize_active_list(&mut pool, &mut destination)
        .expect("finalize destination");
    let combined = destination.take_sealed().expect("combined root");
    let view = arena.list(&pool, combined).expect("combined view");

    assert_eq!(view.iter().copied().collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!(
        view.get(1).expect("moved right node") as *const u32,
        right_address
    );
    assert_eq!(arena.counters().source_nodes_copied, 0);
}

#[test]
fn copying_a_shared_right_root_never_rewrites_an_earlier_composite() {
    let mut pool = ChunkPool::<u32>::with_chunk_bytes(16);
    let mut arena = ForkArena::<u32, ActiveLane>::new();
    let first = list(&mut arena, &mut pool, [1]);
    let shared = list(&mut arena, &mut pool, [2, 3]);
    let other = list(&mut arena, &mut pool, [9]);
    let mut scratch = Vec::new();
    let earlier = arena
        .compose_lists(&mut pool, &[first, shared], &mut scratch)
        .expect("first composite");
    let later = arena
        .compose_lists(&mut pool, &[other, shared], &mut scratch)
        .expect("second composite");

    assert_eq!(
        arena
            .list(&pool, earlier)
            .expect("earlier root remains valid")
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(
        arena
            .list(&pool, later)
            .expect("later root")
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [9, 2, 3]
    );
    // Each explicitly shared right input is copied; the first root can remain
    // the immutable prefix because no predecessor is changed.
    assert_eq!(arena.counters().source_nodes_copied, 4);
}

#[test]
fn explicit_shared_copy_scaling_counts_each_node_once_at_required_sizes() {
    for size in [1_usize, 64, 4_096] {
        let mut pool = ChunkPool::<u32>::with_chunk_bytes(512);
        let mut arena = ForkArena::<u32, ActiveLane>::new();
        let source = {
            let mut builder = arena.begin_builder(&mut pool).expect("source builder");
            for value in 0..size {
                builder.push(value as u32).expect("source node");
            }
            builder.seal().expect("source list")
        };
        let before = arena.counters();
        let mut destination = ActiveListBuilder::vacant();
        arena
            .open_active_list(&pool, &mut destination)
            .expect("destination builder");
        arena
            .append_active_list(&mut pool, &mut destination, source)
            .expect("explicit shared copy");
        arena
            .finalize_active_list(&mut pool, &mut destination)
            .expect("finalize copy");
        let copied = destination.take_sealed().expect("copied root");
        let after = arena.counters();

        assert_eq!(copied.len(), size);
        assert_eq!(
            after.source_nodes_copied - before.source_nodes_copied,
            size as u64
        );
        assert_eq!(after.new_semantic_nodes, before.new_semantic_nodes);
        assert_eq!(
            after.partial_edge_nodes_copied,
            before.partial_edge_nodes_copied
        );
        eprintln!(
            "DIRECT_SHARED_COPY_SCALE nodes={size} copied_nodes={}",
            after.source_nodes_copied - before.source_nodes_copied
        );
    }
}

#[test]
fn one_block_list_stores_its_direct_head_and_tail_cursors() {
    let mut pool = ChunkPool::<u32>::with_chunk_bytes(16);
    let mut arena = ForkArena::<u32, ActiveLane>::new();
    let direct = list(&mut arena, &mut pool, [1, 2]);
    let mark = arena.operation_mark(&pool);

    assert_eq!(direct.len(), 2);
    assert_eq!(direct.head.raw, direct.tail.raw);
    assert_eq!(direct.head.offset, 0);
    assert_eq!(direct.tail.offset, 2);
    assert_eq!(mark.descriptor_chunks, 0);
    assert_eq!(
        arena
            .list(&pool, direct)
            .expect("direct descriptor")
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn arena_rejects_a_different_physical_pool_after_binding() {
    let mut first_pool = ChunkPool::<u32>::with_chunk_bytes(16);
    let mut other_pool = ChunkPool::<u32>::with_chunk_bytes(16);
    let mut arena = ForkArena::<u32, ActiveLane>::new();
    let published = list(&mut arena, &mut first_pool, [1, 2]);

    assert_eq!(
        arena
            .list(&other_pool, published)
            .expect_err("foreign pool"),
        ForkArenaError::InvalidChunk
    );
    assert!(matches!(
        arena.begin_builder(&mut other_pool),
        Err(ForkArenaError::InvalidChunk)
    ));
}

#[test]
fn checkpoint_marks_are_whole_chunk_boundaries_and_rejection_reattaches_prior() {
    let mut pool = ChunkPool::<u32>::with_chunk_bytes(32);
    let mut arena = ForkArena::<u32, ActiveLane>::new();
    let first = list(&mut arena, &mut pool, [10, 11]);
    let early = {
        let boundary = arena
            .seal_boundary(&mut pool)
            .expect("sealed early boundary");
        arena.checkpoint_mark(boundary).expect("checkpoint")
    };
    let later = list(&mut arena, &mut pool, [20, 21, 22]);
    let late = {
        let boundary = arena
            .seal_boundary(&mut pool)
            .expect("sealed late boundary");
        arena.checkpoint_mark(boundary).expect("checkpoint")
    };

    arena
        .begin_checkpoint_candidate(early)
        .expect("select early sibling");
    assert_eq!(arena.list(&pool, first).expect("prefix").get(1), Some(&11));
    assert_eq!(
        arena
            .list(&pool, later)
            .expect_err("detached accepted suffix must be unavailable"),
        ForkArenaError::InvalidRange
    );
    let candidate = list(&mut arena, &mut pool, [30, 31]);
    let settlement = arena
        .seal_boundary(&mut pool)
        .expect("candidate settlement");
    arena
        .reject_checkpoint_candidate(&mut pool, settlement)
        .expect("reject candidate");
    assert_eq!(
        arena
            .list(&pool, later)
            .expect("prior suffix reattached")
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![20, 21, 22]
    );
    assert_eq!(
        arena
            .list(&pool, candidate)
            .expect_err("rejected candidate must be stale"),
        ForkArenaError::InvalidRange
    );
    arena
        .begin_checkpoint_candidate(late)
        .expect("later sibling remains selectable");
    let settlement = arena
        .seal_boundary(&mut pool)
        .expect("empty candidate settlement");
    arena
        .reject_checkpoint_candidate(&mut pool, settlement)
        .expect("reject empty sibling");
    assert!(arena.counters().accepted_chunks_reattached > 0);
}

#[test]
fn released_checkpoint_prefix_reuses_chunks_and_keeps_rebased_reject_exact() {
    let mut pool = ChunkPool::<u32>::with_chunk_bytes(16);
    let mut arena = ForkArena::<u32, ActiveLane>::new();
    let empty = {
        let boundary = arena.seal_boundary(&mut pool).expect("empty boundary");
        arena.checkpoint_mark(boundary).expect("empty checkpoint")
    };
    let released = list(&mut arena, &mut pool, [1, 2]);
    let floor = {
        let boundary = arena.seal_boundary(&mut pool).expect("floor boundary");
        arena.checkpoint_mark(boundary).expect("floor checkpoint")
    };
    let accepted = list(&mut arena, &mut pool, [3, 4]);
    let accepted_address = std::ptr::from_ref(
        arena
            .list(&pool, accepted)
            .expect("accepted suffix")
            .get(0)
            .expect("accepted value"),
    );
    let pages = pool.page_count();

    assert_eq!(
        arena
            .release_accepted_prefix(&mut pool, floor)
            .expect("accepted prefix releases"),
        1,
        "one direct payload block is returned"
    );
    assert!(!arena.validates_checkpoint(empty));
    assert!(arena.validates_checkpoint(floor));
    assert!(matches!(
        arena.list(&pool, released),
        Err(ForkArenaError::InvalidRange)
    ));

    arena
        .begin_checkpoint_candidate(floor)
        .expect("rebased floor forks");
    let candidate = list(&mut arena, &mut pool, [9]);
    assert_eq!(pool.page_count(), pages, "released chunks are reused");
    let settlement = arena.seal_boundary(&mut pool).expect("candidate boundary");
    arena
        .reject_checkpoint_candidate(&mut pool, settlement)
        .expect("rebased candidate rejects");

    assert_eq!(
        arena
            .list(&pool, accepted)
            .expect("accepted suffix reattaches")
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [3, 4]
    );
    assert_eq!(
        std::ptr::from_ref(
            arena
                .list(&pool, accepted)
                .expect("accepted suffix")
                .get(0)
                .expect("accepted value"),
        ),
        accepted_address,
        "rejection reindexes the same physical accepted chunk"
    );
    assert!(matches!(
        arena.list(&pool, candidate),
        Err(ForkArenaError::InvalidRange)
    ));
}

#[test]
fn forked_journal_visitors_preserve_reverse_undo_and_forward_redo_order() {
    let mut pool = ChunkPool::<u32>::with_chunk_bytes(32);
    let mut arena = ForkArena::<u32, ActiveLane>::new();
    let _prefix = list(&mut arena, &mut pool, [1]);
    let selected = {
        let boundary = arena.seal_boundary(&mut pool).expect("selected boundary");
        arena.checkpoint_mark(boundary).expect("checkpoint")
    };
    let accepted_suffix = list(&mut arena, &mut pool, [2, 3]);
    arena.seal_boundary(&mut pool).expect("accepted head");

    let mut accepted_reverse = Vec::new();
    arena
        .visit_accepted_checkpoint_suffix_mut_reverse(&mut pool, selected, |value| {
            accepted_reverse.push(*value);
            *value += 10;
        })
        .expect("accepted journal rewind");
    assert_eq!(accepted_reverse, [3, 2]);
    arena
        .begin_checkpoint_candidate(selected)
        .expect("candidate fork");
    let candidate = list(&mut arena, &mut pool, [4, 5]);

    let mut candidate_reverse = Vec::new();
    arena
        .visit_current_checkpoint_suffix_mut_reverse(&mut pool, selected, |value| {
            candidate_reverse.push(*value);
        })
        .expect("candidate journal rewind");
    assert_eq!(candidate_reverse, [5, 4]);
    let mut accepted_forward = Vec::new();
    arena
        .visit_detached_checkpoint_suffix_mut(&mut pool, |value| {
            accepted_forward.push(*value);
            *value -= 10;
        })
        .expect("accepted journal redo");
    assert_eq!(accepted_forward, [12, 13]);

    let settlement = arena.seal_boundary(&mut pool).expect("settlement");
    arena
        .reject_checkpoint_candidate(&mut pool, settlement)
        .expect("reject candidate");
    assert_eq!(
        arena
            .list(&pool, accepted_suffix)
            .expect("accepted suffix reattached")
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [2, 3]
    );
    assert_eq!(
        arena
            .list(&pool, candidate)
            .expect_err("candidate suffix dropped"),
        ForkArenaError::InvalidRange
    );
}

#[test]
fn acceptance_prunes_detached_prior_and_keeps_candidate_chunks() {
    let mut pool = ChunkPool::<u32>::with_chunk_bytes(32);
    let mut arena = ForkArena::<u32, ActiveLane>::new();
    let accepted = list(&mut arena, &mut pool, [1, 2]);
    let selected = {
        let boundary = arena.seal_boundary(&mut pool).expect("selected boundary");
        arena.checkpoint_mark(boundary).expect("checkpoint")
    };
    let superseded = list(&mut arena, &mut pool, [3, 4]);
    arena
        .seal_boundary(&mut pool)
        .expect("accepted head boundary");
    arena
        .begin_checkpoint_candidate(selected)
        .expect("candidate fork");
    let candidate = list(&mut arena, &mut pool, [8, 9, 10]);
    let settlement = arena
        .seal_boundary(&mut pool)
        .expect("candidate settlement");
    arena
        .accept_checkpoint_candidate(&mut pool, settlement)
        .expect("candidate acceptance");
    assert_eq!(
        arena.list(&pool, accepted).expect("prefix").get(0),
        Some(&1)
    );
    assert_eq!(
        arena.list(&pool, candidate).expect("candidate").get(2),
        Some(&10)
    );
    assert_eq!(
        arena
            .list(&pool, superseded)
            .expect_err("superseded accepted list must be stale"),
        ForkArenaError::InvalidRange
    );
    assert!(arena.counters().obsolete_chunks_pruned > 0);
    assert_eq!(arena.counters().source_nodes_copied, 0);
}

#[test]
fn accepted_restore_prunes_superseded_chunks_without_exposing_a_partial_mark() {
    let mut pool = ChunkPool::<u32>::with_chunk_bytes(32);
    let mut arena = ForkArena::<u32, ActiveLane>::new();
    let retained = list(&mut arena, &mut pool, [1, 2]);
    let selected = {
        let boundary = arena.seal_boundary(&mut pool).expect("selected boundary");
        arena.checkpoint_mark(boundary).expect("checkpoint")
    };
    let superseded = list(&mut arena, &mut pool, [3, 4]);
    arena
        .seal_boundary(&mut pool)
        .expect("accepted head boundary");

    assert!(arena.can_begin_checkpoint_candidate(selected));
    arena
        .restore_accepted_checkpoint(&mut pool, selected)
        .expect("prevalidated accepted restore");

    assert_eq!(
        arena.list(&pool, retained).expect("retained prefix").get(1),
        Some(&2)
    );
    assert_eq!(
        arena
            .list(&pool, superseded)
            .expect_err("accepted restore prunes its superseded suffix"),
        ForkArenaError::InvalidRange
    );
    assert_eq!(arena.counters().source_nodes_copied, 0);
    assert!(arena.counters().obsolete_chunks_pruned > 0);
}

#[test]
fn direct_chunk_sequence_has_indexed_and_sequential_parity() {
    let mut pool = ChunkPool::<u32>::with_chunk_bytes(24);
    let mut arena = ForkArena::<u32, ActiveLane>::new();
    let left = list(&mut arena, &mut pool, [1, 2, 3]);
    let right = list(&mut arena, &mut pool, [7, 8]);
    let mut scratch = Vec::new();
    let composite = arena
        .compose_lists(&mut pool, &[left, right], &mut scratch)
        .expect("range sequence");
    assert_ne!(composite.head.raw, composite.tail.raw);
    {
        let view = arena.list(&pool, composite).expect("sequence view");
        assert_eq!(view.len(), 5);
        assert_eq!(view.get(3), Some(&7));
        assert_eq!(
            view.iter().copied().collect::<Vec<_>>(),
            vec![1, 2, 3, 7, 8]
        );
    }

    let sliced = arena
        .slice_list(&mut pool, composite, 1..4, &mut scratch)
        .expect("slice across canonical ranges");
    assert_eq!(
        arena
            .list(&pool, sliced)
            .expect("sliced view")
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![2, 3, 7]
    );
    assert_eq!(arena.counters().source_nodes_copied, 2);
}

#[test]
fn stable_payload_reference_outlives_the_temporary_view() {
    let mut pool = ChunkPool::<u32>::with_chunk_bytes(24);
    let mut arena = ForkArena::<u32, ActiveLane>::new();
    let values = list(&mut arena, &mut pool, [41, 42]);

    fn first<'a>(
        arena: &'a ForkArena<u32, ActiveLane>,
        pool: &'a ChunkPool<u32>,
        values: super::ArenaListId<ActiveLane>,
    ) -> &'a u32 {
        arena
            .list(pool, values)
            .expect("stable direct view")
            .get(0)
            .expect("first value")
    }

    let borrowed = first(&arena, &pool, values);
    assert_eq!(*borrowed, 41);
}

#[test]
fn borrowed_node_cursor_traverses_page_material_without_materialization() {
    let mut pool = ChunkPool::<Node>::with_chunk_bytes(128);
    let mut arena = ForkArena::<Node, super::PageMaterialLane>::new();
    let list = {
        let mut builder = arena.begin_builder(&mut pool).expect("builder");
        builder.push(Node::Penalty(17)).expect("first node");
        builder.push(Node::Penalty(23)).expect("second node");
        builder.seal().expect("sealed page list")
    };
    let cursor = NodeCursor::fork_arena(arena.list(&pool, list).expect("page view"));

    assert!(matches!(cursor.owned_node(0), Some(Node::Penalty(17))));
    assert_eq!(
        cursor
            .iter()
            .map(|node| match node {
                Node::Penalty(value) => *value,
                _ => unreachable!("test list contains only penalties"),
            })
            .collect::<Vec<_>>(),
        vec![17, 23]
    );
    assert_eq!(arena.counters().source_nodes_copied, 0);
}

#[test]
fn detached_active_builder_rejects_foreign_lane_owner() {
    let mut pool = ChunkPool::<u32>::with_chunk_bytes(24);
    let mut first = ForkArena::<u32, ActiveLane>::new();
    let mut second = ForkArena::<u32, ActiveLane>::new();
    let mut builder = ActiveListBuilder::vacant();
    first
        .open_active_list(&pool, &mut builder)
        .expect("open active list");
    assert_eq!(
        second.push_active_list(&mut pool, &mut builder, 7),
        Err(ForkArenaError::InvalidActiveListBuilder)
    );
    first
        .rollback_active_list(&mut pool, &mut builder)
        .expect("owner rolls back its builder");
    assert!(builder.is_vacant());
}

#[test]
fn open_active_builder_blocks_checkpoint_sealing_and_rolls_back_partial_tail() {
    let mut pool = ChunkPool::<u32>::with_chunk_bytes(24);
    let mut arena = ForkArena::<u32, ActiveLane>::new();
    let prior = list(&mut arena, &mut pool, [1, 2]);
    let before = arena.operation_mark(&pool);
    let mut builder = ActiveListBuilder::vacant();
    arena
        .open_active_list(&pool, &mut builder)
        .expect("open active list");
    arena
        .push_active_list(&mut pool, &mut builder, 3)
        .expect("append partial tail");
    assert!(matches!(
        arena.seal_boundary(&mut pool),
        Err(ForkArenaError::ActiveBuilder)
    ));
    arena
        .rollback_active_list(&mut pool, &mut builder)
        .expect("operation rollback");
    assert_eq!(
        arena.operation_mark(&pool).payload_chunks,
        before.payload_chunks
    );
    assert_eq!(
        arena
            .list(&pool, prior)
            .expect("prior list survives")
            .get(1),
        Some(&2)
    );
}

#[test]
fn shared_active_append_copies_explicitly_and_keeps_the_source_stable() {
    let mut pool = ChunkPool::<Node>::with_chunk_bytes(128);
    let mut arena = ForkArena::<Node, super::PageMaterialLane>::new();
    let source = {
        let mut source = arena.begin_builder(&mut pool).expect("source builder");
        source.push(Node::Penalty(11)).expect("source node");
        source.push(Node::Penalty(12)).expect("source node");
        source.seal().expect("source list")
    };
    let source_address = arena
        .list(&pool, source)
        .expect("source view")
        .get(0)
        .expect("source node") as *const Node;

    let mut active = ActiveListBuilder::vacant();
    arena
        .open_active_list(&pool, &mut active)
        .expect("open unbox destination");
    arena
        .append_active_list(&mut pool, &mut active, source)
        .expect("append source range");
    arena
        .push_active_list(&mut pool, &mut active, Node::Penalty(13))
        .expect("append new semantic node");
    arena
        .finalize_active_list(&mut pool, &mut active)
        .expect("seal active destination");
    let output = active.take_sealed().expect("sealed coordinate");
    let output_view = arena.list(&pool, output).expect("output view");
    assert_eq!(output_view.len(), 3);
    assert_ne!(
        output_view.get(0).expect("retained source") as *const Node,
        source_address
    );
    assert!(matches!(output_view.get(2), Some(Node::Penalty(13))));
    assert_eq!(arena.counters().source_nodes_copied, 2);
    assert_eq!(arena.counters().new_semantic_nodes, 3);
}

#[test]
fn active_shared_subrange_crosses_chunks_with_one_counted_copy() {
    let mut pool = ChunkPool::<u32>::with_chunk_bytes(24);
    let mut arena = ForkArena::<u32, ActiveLane>::new();
    let left = list(&mut arena, &mut pool, [1, 2]);
    let right = list(&mut arena, &mut pool, [3, 4, 5]);
    let mut scratch = Vec::new();
    let source = arena
        .compose_lists(&mut pool, &[left, right], &mut scratch)
        .expect("composed source");
    let source_address = arena
        .list(&pool, source)
        .expect("source view")
        .get(1)
        .expect("source value") as *const u32;

    let mut active = ActiveListBuilder::vacant();
    arena
        .open_active_list(&pool, &mut active)
        .expect("open destination");
    arena
        .append_active_list_range(&mut pool, &mut active, source, 1..4)
        .expect("append cross-descriptor range");
    arena
        .finalize_active_list(&mut pool, &mut active)
        .expect("finalize destination");
    let output = active.take_sealed().expect("sealed destination");
    let output_view = arena.list(&pool, output).expect("output view");
    assert_eq!(
        output_view.iter().copied().collect::<Vec<_>>(),
        vec![2, 3, 4]
    );
    assert_ne!(
        output_view.get(0).expect("retained source") as *const u32,
        source_address
    );
    assert_eq!(arena.counters().source_nodes_copied, 6);
}

#[test]
fn active_shared_subrange_copies_an_offset_past_the_first_payload_chunk() {
    let mut pool = ChunkPool::<u32>::with_chunk_bytes(24);
    let mut arena = ForkArena::<u32, ActiveLane>::new();
    let source = list(&mut arena, &mut pool, [0, 1, 2, 3, 4, 5, 6, 7]);
    let source_address = arena
        .list(&pool, source)
        .expect("source view")
        .get(4)
        .expect("source value") as *const u32;

    let mut active = ActiveListBuilder::vacant();
    arena
        .open_active_list(&pool, &mut active)
        .expect("open destination");
    arena
        .append_active_list_range(&mut pool, &mut active, source, 4..7)
        .expect("append range beginning after the first payload chunk");
    arena
        .finalize_active_list(&mut pool, &mut active)
        .expect("finalize destination");
    let output = active.take_sealed().expect("sealed destination");
    let output_view = arena.list(&pool, output).expect("output view");
    assert_eq!(output_view.iter().copied().collect::<Vec<_>>(), [4, 5, 6]);
    assert_ne!(
        output_view.get(0).expect("retained source") as *const u32,
        source_address
    );
    assert_eq!(arena.counters().source_nodes_copied, 3);
}

#[test]
fn rejected_candidate_truncates_detached_active_builder_output() {
    let mut pool = ChunkPool::<u32>::with_chunk_bytes(24);
    let mut arena = ForkArena::<u32, ActiveLane>::new();
    let accepted = list(&mut arena, &mut pool, [1]);
    let selected = {
        let boundary = arena.seal_boundary(&mut pool).expect("accepted boundary");
        arena.checkpoint_mark(boundary).expect("checkpoint")
    };
    arena
        .begin_checkpoint_candidate(selected)
        .expect("candidate fork");
    let mut active = ActiveListBuilder::vacant();
    arena
        .open_active_list(&pool, &mut active)
        .expect("candidate active list");
    arena
        .push_active_list(&mut pool, &mut active, 9)
        .expect("candidate node");
    arena
        .finalize_active_list(&mut pool, &mut active)
        .expect("candidate list seal");
    let candidate = active.take_sealed().expect("candidate coordinate");
    let settlement = arena.seal_boundary(&mut pool).expect("settlement boundary");
    arena
        .reject_checkpoint_candidate(&mut pool, settlement)
        .expect("reject candidate");
    assert_eq!(
        arena.list(&pool, accepted).expect("accepted prefix").get(0),
        Some(&1)
    );
    assert_eq!(
        arena
            .list(&pool, candidate)
            .expect_err("candidate coordinate is stale"),
        ForkArenaError::InvalidRange
    );
    assert_eq!(arena.counters().source_nodes_copied, 0);
}

#[test]
fn sealed_batch_promotes_whole_chunks_between_typed_lanes() {
    let mut pool = ChunkPool::<u32>::with_chunk_bytes(24);
    let mut active = ForkArena::<u32, ActiveLane>::new();
    let mut page = active.empty_lane::<PageLane>();
    let region = active
        .begin_batch(&mut pool)
        .expect("exclusive batch region");
    let left = list(&mut active, &mut pool, [1, 2]);
    let right = list(&mut active, &mut pool, [3, 4, 5]);
    let batch = active
        .seal_batch(&mut pool, region, vec![left, right])
        .expect("sealed batch");
    assert!(matches!(
        active.begin_builder(&mut pool),
        Err(ForkArenaError::ActiveBatch)
    ));
    let promoted = active
        .promote_batch_into(&mut pool, &mut page, batch)
        .expect("whole-chunk promotion");
    assert_eq!(promoted.len(), 2);
    assert_eq!(
        page.list(&pool, promoted[0])
            .expect("promoted list")
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(
        active
            .list(&pool, left)
            .expect_err("promoted source list must be unavailable"),
        ForkArenaError::InvalidRange
    );
    assert!(page.counters().chunks_promoted > 0);
    assert_eq!(page.counters().source_nodes_copied, 0);
}

#[test]
fn sequence_summaries_move_atomically_with_promoted_direct_chunks() {
    const CHUNK_VALUES: usize = 8;
    let mut pool =
        ChunkPool::<u64>::with_chunk_bytes(std::mem::size_of::<Option<u64>>() * CHUNK_VALUES);
    let mut active = ForkArena::<u64, ActiveLane>::new();
    let mut page = active.empty_lane::<PageLane>();
    let region = active.begin_batch(&mut pool).expect("batch region");
    let source = {
        let mut builder = active.begin_builder(&mut pool).expect("builder");
        for value in 0..64_u64 {
            builder
                .push_summarized(value, value.wrapping_add(100))
                .expect("summarized append");
        }
        builder.seal().expect("source list")
    };
    let batch = active
        .seal_batch(&mut pool, region, vec![source])
        .expect("sealed batch");
    let promoted = active
        .promote_batch_into(&mut pool, &mut page, batch)
        .expect("promote summary storage")[0];
    let mut scratch = Vec::new();
    let (middle, summary, work) = page
        .slice_list_summarized(&mut pool, promoted, 3..61, &mut scratch, |value| {
            value.wrapping_add(100)
        })
        .expect("summarized promoted slice");
    let mut expected = crate::node_sequence::SemanticSequenceIdentity::empty();
    for value in 3..61_u64 {
        expected.push_back(value.wrapping_add(100));
    }

    assert_eq!(summary, expected);
    assert_eq!(page.list(&pool, middle).expect("middle").len(), 58);
    assert!(work.hashed_values <= (2 * CHUNK_VALUES) as u64);
    assert!(work.combined_summaries > 0);
    assert_eq!(page.counters().source_nodes_copied, 0);
}

fn list<const N: usize>(
    arena: &mut ForkArena<u32, ActiveLane>,
    pool: &mut ChunkPool<u32>,
    values: [u32; N],
) -> super::ArenaListId<ActiveLane> {
    let mut builder = arena.begin_builder(pool).expect("builder");
    for value in values {
        builder.push(value).expect("append");
    }
    builder.seal().expect("seal")
}
