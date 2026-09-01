use std::hint::black_box;
use std::time::Instant;

use tex_state::measurement::{
    HotCoreAllocationOwner, HotCoreAllocator, hot_core_allocation_scope,
    hot_core_thread_allocation_measurement,
};
use tex_state::with_universe;
use tex_state_benchmarks::engine_budget;

#[global_allocator]
static ALLOCATOR: HotCoreAllocator = HotCoreAllocator;

const RETAINED_STRINGS: usize = 32_768;
const ITERATIONS: usize = 4_096;

fn main() {
    let result = with_universe(engine_budget(), |universe| {
        {
            let mut context = universe.command_context().expect("string-pool context");
            for index in 0..RETAINED_STRINGS {
                context.slow_make_string_pool_string(&format!(
                    "retained-checkpoint-spelling-{index:05}"
                ));
            }
        }

        drop(universe.runtime_checkpoint().expect("warm checkpoint"));
        let owner = HotCoreAllocationOwner::GenerationBoundary;
        let before = hot_core_thread_allocation_measurement(owner);
        let started = Instant::now();
        let checksum = {
            let _scope = hot_core_allocation_scope(owner);
            let mut checksum = 0_usize;
            for _ in 0..ITERATIONS {
                let checkpoint = black_box(
                    universe
                        .runtime_checkpoint()
                        .expect("measured checkpoint"),
                );
                checksum ^= checkpoint.retention().core_bytes();
                drop(checkpoint);
            }
            checksum
        };
        let elapsed = started.elapsed();
        let after = hot_core_thread_allocation_measurement(owner);
        (
            elapsed,
            after.calls - before.calls,
            after.requested_bytes - before.requested_bytes,
            checksum,
        )
    })
    .expect("profile universe");

    println!(
        "STRING_POOL_CHECKPOINT_GATE retained_strings={RETAINED_STRINGS} iterations={ITERATIONS} elapsed_ns={} ns_per_checkpoint={} allocations={} requested_bytes={} checksum={}",
        result.0.as_nanos(),
        result.0.as_nanos() / ITERATIONS as u128,
        result.1,
        result.2,
        result.3,
    );
}
