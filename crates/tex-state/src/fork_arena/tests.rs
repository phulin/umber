use super::{ArenaListId, ChunkPool, ForkArena, ForkArenaError};

enum ActiveLane {}
enum PageLane {}

#[test]
fn coarse_pool_pages_hold_many_stable_chunks_and_reject_stale_keys() {
    let mut pool = ChunkPool::<u64>::with_chunk_bytes(32);
    assert_eq!(
        pool.chunk_capacity(),
        (32 / std::mem::size_of::<Option<u64>>()).max(1)
    );
    let mut keys = Vec::new();
    for value in 0..17_u64 {
        let key = pool.allocate(7).expect("chunk allocation");
        pool.append(key, 7, value).expect("chunk append");
        keys.push(key);
    }
    assert_eq!(pool.page_count(), 2);
    assert_eq!(pool.get(keys[0], 7, 0), Some(&0));
    let stale = keys[0];
    assert_eq!(pool.release(stale, 7), Ok(1));
    let replacement = pool.allocate(7).expect("reused chunk");
    assert_eq!(replacement.slot, stale.slot);
    assert_ne!(replacement.generation, stale.generation);
    assert_eq!(pool.get(stale, 7, 0), None);
}

#[test]
fn builder_drop_and_partial_operation_mark_truncate_without_payload_copy() {
    let mut arena = ForkArena::<u32, ActiveLane>::with_chunk_bytes(16);
    {
        let mut abandoned = arena.begin_builder().expect("builder");
        abandoned.push(1).expect("append");
        abandoned.push(2).expect("append");
    }
    assert_eq!(arena.counters().candidate_chunks_truncated, 1);

    let first = {
        let mut builder = arena.begin_builder().expect("builder");
        builder.push(3).expect("append");
        builder.push(4).expect("append");
        builder.seal().expect("list seal")
    };
    let operation = arena.operation_mark();
    let second = {
        let mut builder = arena.begin_builder().expect("builder");
        builder.push(5).expect("append");
        builder.seal().expect("list seal")
    };
    assert_eq!(arena.list(second).expect("second view").get(0), Some(&5));
    arena
        .restore_operation(operation)
        .expect("partial rollback");
    assert_eq!(
        arena
            .list(second)
            .expect_err("rolled-back list must be stale"),
        ForkArenaError::InvalidRange
    );
    assert_eq!(
        arena
            .list(first)
            .expect("first survives")
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![3, 4]
    );
    assert_eq!(arena.counters().payload_values_copied, 0);
}

#[test]
fn checkpoint_marks_are_whole_chunk_boundaries_and_rejection_reattaches_prior() {
    let mut arena = ForkArena::<u32, ActiveLane>::with_chunk_bytes(32);
    let first = list(&mut arena, [10, 11]);
    let early = {
        let boundary = arena.seal_boundary().expect("sealed early boundary");
        arena.checkpoint_mark(boundary).expect("checkpoint")
    };
    let later = list(&mut arena, [20, 21, 22]);
    let late = {
        let boundary = arena.seal_boundary().expect("sealed late boundary");
        arena.checkpoint_mark(boundary).expect("checkpoint")
    };

    arena
        .begin_checkpoint_candidate(early)
        .expect("select early sibling");
    assert_eq!(arena.list(first).expect("prefix").get(1), Some(&11));
    assert_eq!(
        arena
            .list(later)
            .expect_err("detached accepted suffix must be unavailable"),
        ForkArenaError::InvalidRange
    );
    let candidate = list(&mut arena, [30, 31]);
    let settlement = arena.seal_boundary().expect("candidate settlement");
    arena
        .reject_checkpoint_candidate(settlement)
        .expect("reject candidate");
    assert_eq!(
        arena
            .list(later)
            .expect("prior suffix reattached")
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![20, 21, 22]
    );
    assert_eq!(
        arena
            .list(candidate)
            .expect_err("rejected candidate must be stale"),
        ForkArenaError::InvalidRange
    );
    arena
        .begin_checkpoint_candidate(late)
        .expect("later sibling remains selectable");
    let settlement = arena.seal_boundary().expect("empty candidate settlement");
    arena
        .reject_checkpoint_candidate(settlement)
        .expect("reject empty sibling");
    assert!(arena.counters().accepted_chunks_reattached > 0);
}

#[test]
fn acceptance_prunes_detached_prior_and_keeps_candidate_chunks() {
    let mut arena = ForkArena::<u32, ActiveLane>::with_chunk_bytes(32);
    let accepted = list(&mut arena, [1, 2]);
    let selected = {
        let boundary = arena.seal_boundary().expect("selected boundary");
        arena.checkpoint_mark(boundary).expect("checkpoint")
    };
    let superseded = list(&mut arena, [3, 4]);
    arena.seal_boundary().expect("accepted head boundary");
    arena
        .begin_checkpoint_candidate(selected)
        .expect("candidate fork");
    let candidate = list(&mut arena, [8, 9, 10]);
    let settlement = arena.seal_boundary().expect("candidate settlement");
    arena
        .accept_checkpoint_candidate(settlement)
        .expect("candidate acceptance");
    assert_eq!(arena.list(accepted).expect("prefix").get(0), Some(&1));
    assert_eq!(arena.list(candidate).expect("candidate").get(2), Some(&10));
    assert_eq!(
        arena
            .list(superseded)
            .expect_err("superseded accepted list must be stale"),
        ForkArenaError::InvalidRange
    );
    assert!(arena.counters().obsolete_chunks_pruned > 0);
    assert_eq!(arena.counters().payload_values_copied, 0);
}

#[test]
fn canonical_range_sequence_has_indexed_and_sequential_parity() {
    let mut arena = ForkArena::<u32, ActiveLane>::with_chunk_bytes(24);
    let left = list(&mut arena, [1, 2, 3]);
    let right = list(&mut arena, [7, 8]);
    let mut scratch = Vec::new();
    let composite = arena
        .compose_lists(&[left, right], &mut scratch)
        .expect("range sequence");
    assert!(matches!(composite, ArenaListId::Sequence { .. }));
    let view = arena.list(composite).expect("sequence view");
    assert_eq!(view.len(), 5);
    assert_eq!(view.get(3), Some(&7));
    assert_eq!(
        view.iter().copied().collect::<Vec<_>>(),
        vec![1, 2, 3, 7, 8]
    );
}

#[test]
fn sealed_batch_promotes_whole_chunks_between_typed_lanes() {
    let mut active = ForkArena::<u32, ActiveLane>::with_chunk_bytes(24);
    let mut page = active.empty_lane::<PageLane>();
    let region = active.begin_batch().expect("exclusive batch region");
    let left = list(&mut active, [1, 2]);
    let right = list(&mut active, [3, 4, 5]);
    let batch = active
        .seal_batch(region, vec![left, right])
        .expect("sealed batch");
    assert!(matches!(
        active.begin_builder(),
        Err(ForkArenaError::ActiveBatch)
    ));
    let promoted = active
        .promote_batch_into(&mut page, batch)
        .expect("whole-chunk promotion");
    assert_eq!(promoted.len(), 2);
    assert_eq!(
        page.list(promoted[0])
            .expect("promoted list")
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(
        active
            .list(left)
            .expect_err("promoted source list must be unavailable"),
        ForkArenaError::InvalidRange
    );
    assert!(page.counters().chunks_promoted > 0);
    assert_eq!(page.counters().payload_values_copied, 0);
}

fn list<const N: usize>(
    arena: &mut ForkArena<u32, ActiveLane>,
    values: [u32; N],
) -> super::ArenaListId<ActiveLane> {
    let mut builder = arena.begin_builder().expect("builder");
    for value in values {
        builder.push(value).expect("append");
    }
    builder.seal().expect("seal")
}
