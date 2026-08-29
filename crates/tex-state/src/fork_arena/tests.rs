use super::{ActiveListBuilder, ChunkPool, ForkArena, ForkArenaError};
use crate::node::Node;
use crate::node_arena::NodeCursor;
use umber_hot_core_allocator::{AllocationMeasurement, scope, thread_measurement};

static DIRECT_CLONE_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[derive(Debug, Eq, PartialEq)]
struct CloneTracked(u32);

impl Clone for CloneTracked {
    fn clone(&self) -> Self {
        DIRECT_CLONE_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self(self.0)
    }
}

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
fn direct_root_admission_work_is_constant_at_one_sixty_four_and_four_thousand_ninety_six_chunks() {
    let mut observed = Vec::new();
    for chunks in [1_u32, 64, 4_096] {
        let mut pool = ChunkPool::<u32>::with_chunk_bytes(1);
        let mut arena = ForkArena::<u32, ActiveLane>::new();
        let root = {
            let mut builder = arena.begin_builder(&mut pool).expect("builder");
            for value in 0..chunks {
                builder.push(value).expect("one-node direct block");
            }
            builder.seal().expect("direct root")
        };
        assert_eq!(arena.counters().direct_blocks_allocated, u64::from(chunks));

        let validations_before = pool.payload.validation_reads();
        let links_before = pool.payload.previous_link_reads();
        arena
            .admit_owned_list(&pool, root)
            .expect("constant-time admission");
        let admission_validations = pool.payload.validation_reads() - validations_before;
        let admission_links = pool.payload.previous_link_reads() - links_before;

        let validations_before = pool.payload.validation_reads();
        let links_before = pool.payload.previous_link_reads();
        let view = arena
            .validated_list(&pool, root)
            .expect("constant-time admitted-root validation");
        let checked_validations = pool.payload.validation_reads() - validations_before;
        let checked_links = pool.payload.previous_link_reads() - links_before;

        let validations_before = pool.payload.validation_reads();
        let links_before = pool.payload.previous_link_reads();
        assert_eq!(
            arena.list(&pool, root).expect("ordinary view").len(),
            chunks as usize
        );
        let view_validations = pool.payload.validation_reads() - validations_before;
        let view_links = pool.payload.previous_link_reads() - links_before;

        assert_eq!(view.len(), chunks as usize);
        assert_eq!(admission_links, 0);
        assert_eq!(checked_links, 0);
        assert_eq!(view_links, 0);
        observed.push((admission_validations, checked_validations, view_validations));
        eprintln!(
            "DIRECT_ROOT_ADMISSION_SCALE chunks={chunks} admission_validations={admission_validations} checked_validations={checked_validations} view_validations={view_validations} predecessor_reads=0"
        );
    }

    assert!(observed.windows(2).all(|pair| pair[0] == pair[1]));
    assert_eq!(observed[0], (2, 2, 2));
}

#[test]
fn direct_chunk_visit_is_linear_and_allocation_free_at_one_sixty_four_and_four_thousand_ninety_six_chunks()
 {
    for chunks in [1_u32, 64, 4_096] {
        let mut pool = ChunkPool::<u32>::with_chunk_bytes(1);
        let mut arena = ForkArena::<u32, ActiveLane>::new();
        let root = {
            let mut builder = arena.begin_builder(&mut pool).expect("builder");
            for value in 0..chunks {
                builder.push(value).expect("one-node direct block");
            }
            builder.seal().expect("direct root")
        };
        let view = arena.list(&pool, root).expect("admitted view");
        let bytes_before = pool.allocated_heap_bytes();
        let validations_before = pool.payload.validation_reads();
        let links_before = pool.payload.previous_link_reads();
        let mut values = 0_u32;
        let mut visits = 0_u32;
        view.visit_chunks(|chunk| {
            visits += 1;
            for value in chunk.iter() {
                assert_eq!(*value, values);
                values += 1;
            }
        });
        let validations = pool.payload.validation_reads() - validations_before;
        let links = pool.payload.previous_link_reads() - links_before;

        assert_eq!(values, chunks);
        assert_eq!(visits, chunks);
        assert_eq!(validations, 0);
        assert_eq!(links, 0);
        assert_eq!(pool.allocated_heap_bytes(), bytes_before);
        eprintln!(
            "DIRECT_CHUNK_VISIT_SCALE chunks={chunks} visits={visits} validations={validations} predecessor_reads={links} allocation_bytes=0"
        );
    }
}

#[test]
fn admitted_index_lookup_is_allocation_free_and_repeats_no_owner_validation_at_required_sizes() {
    const ALLOCATION_OWNER: usize = 15;

    for chunks in [1_u32, 64, 4_096] {
        let mut pool = ChunkPool::<u32>::with_chunk_bytes(1);
        let mut arena = ForkArena::<u32, ActiveLane>::new();
        let root = {
            let mut builder = arena.begin_builder(&mut pool).expect("builder");
            for value in 0..chunks {
                builder.push(value).expect("one-node direct block");
            }
            builder.seal().expect("direct root")
        };
        let view = arena.list(&pool, root).expect("admitted view");
        let validations_before = pool.payload.validation_reads();
        let links_before = pool.payload.previous_link_reads();
        let allocation_before = thread_measurement(ALLOCATION_OWNER);
        let checksum = {
            let _scope = scope(ALLOCATION_OWNER);
            (0..chunks as usize)
                .map(|index| *view.get(index).expect("admitted indexed node"))
                .fold(0_u64, |sum, value| sum + u64::from(value))
        };
        let allocation_after = thread_measurement(ALLOCATION_OWNER);
        let allocation = AllocationMeasurement {
            calls: allocation_after
                .calls
                .saturating_sub(allocation_before.calls),
            requested_bytes: allocation_after
                .requested_bytes
                .saturating_sub(allocation_before.requested_bytes),
        };
        let validations = pool.payload.validation_reads() - validations_before;
        let links = pool.payload.previous_link_reads() - links_before;

        assert_eq!(checksum, u64::from(chunks) * u64::from(chunks - 1) / 2);
        assert_eq!(validations, 0);
        assert_eq!(links, 0);
        assert_eq!(allocation, AllocationMeasurement::default());
        eprintln!(
            "ADMITTED_INDEX_LOOKUP_SCALE chunks={chunks} lookups={chunks} owner_validations={validations} checked_predecessor_reads={links} allocation_calls={} allocation_bytes={}",
            allocation.calls, allocation.requested_bytes
        );
    }
}

#[test]
fn direct_mapped_clone_clones_each_source_once_and_reuses_warmed_storage() {
    const ALLOCATION_OWNER: usize = 14;
    const VALUES: usize = 4_096;

    let mut pool = ChunkPool::<CloneTracked>::with_chunk_bytes(512);
    let mut source = ForkArena::<CloneTracked, ActiveLane>::new();
    let source_root = {
        let mut builder = source.begin_builder(&mut pool).expect("source builder");
        for value in 0..VALUES as u32 {
            builder.push(CloneTracked(value)).expect("source value");
        }
        builder.seal().expect("source root")
    };
    let mut destination = ForkArena::<CloneTracked, ActiveLane>::new();
    let empty = destination.operation_mark(&pool);

    destination
        .clone_mapped_list_from(&mut pool, &source, source_root, |value| {
            value.0 += 1;
            Ok(None)
        })
        .expect("warm direct clone");
    destination
        .restore_operation(&mut pool, empty)
        .expect("return warmed destination chunks");

    DIRECT_CLONE_CALLS.store(0, std::sync::atomic::Ordering::Relaxed);
    let before = thread_measurement(ALLOCATION_OWNER);
    let copied = {
        let _scope = scope(ALLOCATION_OWNER);
        destination
            .clone_mapped_list_from(&mut pool, &source, source_root, |value| {
                value.0 += 1;
                Ok(None)
            })
            .expect("measured direct clone")
    };
    let after = thread_measurement(ALLOCATION_OWNER);

    assert_eq!(copied.len(), VALUES);
    assert_eq!(
        DIRECT_CLONE_CALLS.load(std::sync::atomic::Ordering::Relaxed),
        VALUES,
        "the destination is constructed from exactly one clone per source value"
    );
    assert_eq!(
        after.calls - before.calls,
        0,
        "warmed direct construction has no transient staging allocation"
    );
    assert_eq!(after.requested_bytes - before.requested_bytes, 0);
    assert_eq!(
        destination
            .list(&pool, copied)
            .expect("copied list")
            .iter()
            .map(|value| value.0)
            .collect::<Vec<_>>(),
        (1..=VALUES as u32).collect::<Vec<_>>()
    );
    assert_eq!(
        source
            .list(&pool, source_root)
            .expect("source remains live")
            .get(0),
        Some(&CloneTracked(0))
    );
}

#[test]
fn direct_mapped_clone_rewrite_failure_rolls_back_the_partial_destination() {
    let mut pool = ChunkPool::<CloneTracked>::with_chunk_bytes(64);
    let mut source = ForkArena::<CloneTracked, ActiveLane>::new();
    let source_root = {
        let mut builder = source.begin_builder(&mut pool).expect("source builder");
        for value in 0..32 {
            builder.push(CloneTracked(value)).expect("source value");
        }
        builder.seal().expect("source root")
    };
    let mut destination = ForkArena::<CloneTracked, ActiveLane>::new();

    assert_eq!(
        destination.clone_mapped_list_from(&mut pool, &source, source_root, |value| {
            if value.0 == 17 {
                return Err(ForkArenaError::InvalidRegion);
            }
            value.0 += 1;
            Ok(None)
        }),
        Err(ForkArenaError::InvalidRegion)
    );
    let mut replacement = destination
        .begin_builder(&mut pool)
        .expect("failed clone leaves no active builder");
    replacement
        .push(CloneTracked(41))
        .expect("rolled-back destination accepts new work");
    let replacement = replacement.seal().expect("replacement root");
    assert_eq!(
        destination
            .list(&pool, replacement)
            .expect("replacement list")
            .get(0),
        Some(&CloneTracked(41))
    );
    assert_eq!(
        source
            .list(&pool, source_root)
            .expect("source remains unchanged")
            .get(17),
        Some(&CloneTracked(17))
    );
}

#[test]
fn exhaustive_test_audit_rejects_unconstructible_length_and_chain_roots() {
    let mut pool = ChunkPool::<u32>::with_chunk_bytes(1);
    let mut arena = ForkArena::<u32, ActiveLane>::new();
    let root = list(&mut arena, &mut pool, [1, 2, 3]);

    let mut wrong_length = root;
    wrong_length.len += 1;
    assert_eq!(
        arena.audit_owned_list(&pool, wrong_length),
        Err(ForkArenaError::InvalidRange)
    );

    let previous = pool
        .payload
        .previous_in_list(root.tail.raw, arena.owner)
        .expect("tail metadata")
        .expect("middle block");
    pool.payload
        .validate_mut(previous.0, arena.owner)
        .expect("middle metadata")
        .previous_in_list = None;
    assert_eq!(
        arena.audit_owned_list(&pool, root),
        Err(ForkArenaError::InvalidRange)
    );

    let mut ingress_pool = ChunkPool::<u32>::with_chunk_bytes(1);
    let mut ingress_arena = ForkArena::<u32, ActiveLane>::new();
    let batch = ingress_arena
        .begin_batch(&mut ingress_pool)
        .expect("cold ingress boundary");
    let mut malformed = list(&mut ingress_arena, &mut ingress_pool, [4, 5, 6]);
    malformed.len += 1;
    assert!(matches!(
        ingress_arena.seal_batch(&mut ingress_pool, batch, vec![malformed]),
        Err(ForkArenaError::InvalidRange)
    ));
}

#[test]
fn reverse_tail_chunk_work_is_independent_of_list_size() {
    fn tail_work(size: u32) -> (Vec<u32>, usize, usize) {
        let mut pool = ChunkPool::<u32>::with_chunk_bytes(1);
        let mut arena = ForkArena::<u32, ActiveLane>::new();
        let root = {
            let mut builder = arena.begin_builder(&mut pool).expect("builder");
            for value in 0..size {
                builder.push(value).expect("node");
            }
            builder.seal().expect("direct root")
        };
        let view = arena.list(&pool, root).expect("direct view");
        let mut nodes = view.iter();
        let tail = (0..3)
            .map(|_| *nodes.next_back().expect("three-node tail"))
            .collect::<Vec<_>>();
        (
            tail,
            nodes.reverse_descriptor_visits(),
            nodes.reverse_chunk_crossings(),
        )
    }

    let short = tail_work(8);
    let long = tail_work(4_096);
    assert_eq!(short.0, vec![7, 6, 5]);
    assert_eq!(long.0, vec![4_095, 4_094, 4_093]);
    assert_eq!(short.1, 0, "direct roots visit no list descriptors");
    assert_eq!(long.1, 0, "direct roots visit no list descriptors");
    assert_eq!(short.2, long.2);
    assert_eq!(short.2, 2, "three one-node blocks cross two boundaries");
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
        .begin_checkpoint_candidate(&mut pool, early)
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
        .begin_checkpoint_candidate(&mut pool, late)
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
        .begin_checkpoint_candidate(&mut pool, floor)
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
        .begin_checkpoint_candidate(&mut pool, selected)
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
        .begin_checkpoint_candidate(&mut pool, selected)
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
        .begin_checkpoint_candidate(&mut pool, selected)
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

fn detached_promotion_allocation_cost(prefix_chunks: usize) -> AllocationMeasurement {
    const ALLOCATION_OWNER: usize = 15;

    let mut pool = ChunkPool::<u32>::with_chunk_bytes(std::mem::size_of::<Option<u32>>());
    let mut filler = ForkArena::<u32, ActiveLane>::new();
    if prefix_chunks != 0 {
        let mut builder = filler.begin_builder(&mut pool).expect("filler builder");
        for value in 0..prefix_chunks as u32 {
            builder.push(value).expect("filler append");
        }
        builder.seal().expect("filler root");
    }

    let mut source = ForkArena::<u32, ActiveLane>::new();
    let mark = source.begin_batch(&mut pool).expect("transfer boundary");
    let root = list(&mut source, &mut pool, [41]);
    let batch = source
        .seal_batch(&mut pool, mark, vec![root])
        .expect("sealed transfer batch");
    let detached = source
        .detach_batch(&mut pool, batch)
        .map_err(|failure| failure.error)
        .expect("detached transfer batch");
    let mut destination = source.empty_lane::<PageLane>();

    let before = thread_measurement(ALLOCATION_OWNER);
    let (promoted, scanned) = {
        let _scope = scope(ALLOCATION_OWNER);
        source
            .promote_detached_batch_into(&mut pool, &mut destination, detached)
            .map_err(|failure| failure.error)
            .expect("detached batch promotion")
    };
    let after = thread_measurement(ALLOCATION_OWNER);

    assert_eq!(scanned, 1, "promotion visits the one moved payload value");
    assert_eq!(destination.counters().source_nodes_copied, 0);
    assert_eq!(promoted.len(), 1);
    assert_eq!(
        destination
            .list(&pool, promoted[0])
            .expect("promoted root")
            .get(0),
        Some(&41)
    );
    AllocationMeasurement {
        calls: after.calls.saturating_sub(before.calls),
        requested_bytes: after.requested_bytes.saturating_sub(before.requested_bytes),
    }
}

#[test]
fn detached_promotion_allocation_is_independent_of_shared_pool_slot_depth() {
    let low = detached_promotion_allocation_cost(0);
    let high = detached_promotion_allocation_cost(4_096);

    eprintln!(
        "DETACHED_PROMOTION_ALLOCATION_SCALE resolver_entry_bytes={} chunk_meta_bytes={} low_calls={} low_bytes={} high_slot=4096 high_calls={} high_bytes={}",
        std::mem::size_of::<Option<(u32, usize)>>(),
        std::mem::size_of::<super::ChunkMeta>(),
        low.calls,
        low.requested_bytes,
        high.calls,
        high.requested_bytes
    );
    assert_eq!(high, low, "promotion storage scales with the moved suffix");
    assert!(
        high.calls <= 2 && high.requested_bytes <= 256,
        "one-root promotion owns only its result and destination chunk-key lanes"
    );
}

#[test]
fn failed_high_slot_promotion_reattaches_the_exact_source_suffix() {
    let mut pool = ChunkPool::<u32>::with_chunk_bytes(std::mem::size_of::<Option<u32>>());
    let mut filler = ForkArena::<u32, ActiveLane>::new();
    let mut filler_builder = filler.begin_builder(&mut pool).expect("filler builder");
    for value in 0..256 {
        filler_builder.push(value).expect("filler append");
    }
    filler_builder.seal().expect("filler root");

    let mut source = ForkArena::<u32, ActiveLane>::new();
    let mark = source.begin_batch(&mut pool).expect("transfer boundary");
    let root = list(&mut source, &mut pool, [71, 73]);
    let batch = source
        .seal_batch(&mut pool, mark, vec![root])
        .expect("sealed transfer batch");
    let detached = source
        .detach_batch(&mut pool, batch)
        .map_err(|failure| failure.error)
        .expect("detached transfer batch");

    let mut foreign_pool = ChunkPool::<u32>::with_chunk_bytes(32);
    let mut destination = source.empty_lane::<PageLane>();
    let mut foreign_builder = destination
        .begin_builder(&mut foreign_pool)
        .expect("foreign destination builder");
    foreign_builder.push(99).expect("foreign append");
    let foreign_root = foreign_builder.seal().expect("foreign root");
    let failure = source
        .promote_detached_batch_into(&mut pool, &mut destination, detached)
        .expect_err("foreign-bound destination rejects the transfer");
    assert_eq!(failure.error, ForkArenaError::InvalidChunk);
    assert!(
        source.reattach_batch(&mut pool, failure.batch).is_ok(),
        "failed transfer returns its exact suffix"
    );

    assert_eq!(
        source
            .list(&pool, root)
            .expect("reattached source root")
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [71, 73]
    );
    assert_eq!(
        destination
            .list(&foreign_pool, foreign_root)
            .expect("foreign destination remains unchanged")
            .get(0),
        Some(&99)
    );
    assert_eq!(source.counters().source_nodes_copied, 0);
    assert_eq!(destination.counters().chunks_promoted, 0);
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
