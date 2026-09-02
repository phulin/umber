use std::hint::black_box;

use tex_state::fork_arena::{ArenaListId, ChunkPool, ForkArena};
use tex_state::measurement::{
    HotCoreAllocationOwner, HotCoreAllocator, hot_core_allocation_scope,
    hot_core_thread_allocation_measurement,
};

#[global_allocator]
static ALLOCATOR: HotCoreAllocator = HotCoreAllocator;

enum AnnexLane {}

const LISTS: usize = 4_096;
const WORDS_PER_LIST: usize = 11;
const LONG_WORDS: usize = 65_536;
const READ_PASSES: usize = 64;

fn main() {
    let mut pool = ChunkPool::<u32>::testing_with_packed_chunk_bytes(65_536);
    let mut arena = ForkArena::<u32, AnnexLane>::new();
    let empty = arena.operation_mark(&pool);
    let mut roots = Vec::with_capacity(LISTS);

    append_short_lists(&mut arena, &mut pool, &mut roots);
    arena
        .testing_append_unsealed_list(&mut pool, 0..LONG_WORDS as u32)
        .expect("append warm long list");
    arena
        .restore_operation(&mut pool, empty)
        .expect("restore warmed packed capacity");
    roots.clear();

    let counters_before = arena.counters();
    let owner = HotCoreAllocationOwner::SemanticApply;
    let allocations_before = hot_core_thread_allocation_measurement(owner);
    let checksum = {
        let _scope = hot_core_allocation_scope(owner);
        append_short_lists(&mut arena, &mut pool, &mut roots);
        let long = arena
            .testing_append_unsealed_list(&mut pool, 0..LONG_WORDS as u32)
            .expect("append measured long list");

        let mut checksum = 0_u64;
        for _ in 0..READ_PASSES {
            for &root in &roots {
                checksum = arena
                    .list(&pool, root)
                    .expect("admit short list")
                    .iter()
                    .fold(checksum, |sum, word| sum.wrapping_add(u64::from(*word)));
            }
            checksum = checksum.wrapping_add(
                arena
                    .testing_admitted_chunk_checksum(&pool, long)
                    .expect("read admitted long chunks"),
            );
        }
        black_box(checksum)
    };
    let allocations_after = hot_core_thread_allocation_measurement(owner);
    let counters_after = arena.counters();

    let allocations = allocations_after.calls - allocations_before.calls;
    let requested_bytes = allocations_after.requested_bytes - allocations_before.requested_bytes;
    let blocks = counters_after.direct_blocks_allocated - counters_before.direct_blocks_allocated;
    assert_ne!(checksum, 0);
    assert_eq!((allocations, requested_bytes), (0, 0));
    println!(
        "ARENA_BLOCK_OPS_GATE lists={LISTS} words_per_list={WORDS_PER_LIST} long_words={LONG_WORDS} read_passes={READ_PASSES} checksum={checksum} allocations={allocations} requested_bytes={requested_bytes} logical_blocks={blocks}"
    );
}

fn append_short_lists(
    arena: &mut ForkArena<u32, AnnexLane>,
    pool: &mut ChunkPool<u32>,
    roots: &mut Vec<ArenaListId<AnnexLane>>,
) {
    for list in 0..LISTS {
        roots.push(
            arena
                .testing_append_unsealed_list(
                    pool,
                    (0..WORDS_PER_LIST).map(|word| (list * WORDS_PER_LIST + word) as u32),
                )
                .expect("append short packed list"),
        );
    }
}
