use super::{ArenaListId, ChunkPool, ForkArena, ForkArenaError};
use crate::node::Node;
use crate::node_arena::NodeCursor;

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
        let key = pool.payload.allocate(7).expect("chunk allocation");
        pool.payload.append(key, 7, value).expect("chunk append");
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
fn canonical_range_sequence_has_indexed_and_sequential_parity() {
    let mut pool = ChunkPool::<u32>::with_chunk_bytes(24);
    let mut arena = ForkArena::<u32, ActiveLane>::new();
    let left = list(&mut arena, &mut pool, [1, 2, 3]);
    let right = list(&mut arena, &mut pool, [7, 8]);
    let mut scratch = Vec::new();
    let composite = arena
        .compose_lists(&mut pool, &[left, right], &mut scratch)
        .expect("range sequence");
    assert!(matches!(composite, ArenaListId::Sequence { .. }));
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
    assert_eq!(arena.counters().source_nodes_copied, 0);
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
