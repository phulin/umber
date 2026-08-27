use std::hint::black_box;
use std::time::{Duration, Instant};

use tex_state::hyphenation::{ExceptionSpec, PatternSpec};
use tex_state::measurement::{
    HotCoreAllocationMeasurement, HotCoreAllocationOwner, HotCoreAllocator,
    hot_core_allocation_scope, hot_core_thread_allocation_measurement,
};
use tex_state::{EngineCapacityProfile, with_universe};
use tex_state_benchmarks::engine_budget;

#[global_allocator]
static ALLOCATOR: HotCoreAllocator = HotCoreAllocator;

const ITERATIONS: usize = 1_000;

#[derive(Clone, Copy)]
struct OperationMeasurement {
    elapsed: Duration,
    allocations: HotCoreAllocationMeasurement,
}

struct TrieMeasurement {
    pattern_nodes: usize,
    capture: OperationMeasurement,
    checkpoint_clone: OperationMeasurement,
    restore: OperationMeasurement,
    fork: OperationMeasurement,
}

fn measured<T>(operation: impl FnOnce() -> T) -> (T, OperationMeasurement) {
    let owner = HotCoreAllocationOwner::GenerationBoundary;
    let before = hot_core_thread_allocation_measurement(owner);
    let start = Instant::now();
    let value = {
        let _scope = hot_core_allocation_scope(owner);
        operation()
    };
    let elapsed = start.elapsed();
    let after = hot_core_thread_allocation_measurement(owner);
    (
        value,
        OperationMeasurement {
            elapsed,
            allocations: HotCoreAllocationMeasurement {
                calls: after.calls - before.calls,
                requested_bytes: after.requested_bytes - before.requested_bytes,
            },
        },
    )
}

fn profile(pattern_nodes: usize) -> TrieMeasurement {
    with_universe(engine_budget(), |universe| {
        universe.set_engine_capacity_profile(EngineCapacityProfile::Texlive2026);
        {
            let mut context = universe.command_context().expect("hyphenation context");
            for index in 0..pattern_nodes {
                let ch = char::from_u32(0x1000 + index as u32).expect("profile character");
                context
                    .add_hyphenation_pattern_for_language(
                        0,
                        PatternSpec {
                            letters: vec![ch],
                            values: vec![0, 1],
                        },
                    )
                    .expect("profile trie fits");
            }
            for index in 0..16 {
                context.add_hyphenation_exception_for_language(
                    0,
                    ExceptionSpec {
                        word: format!("profile{index}"),
                        positions: vec![3],
                    },
                );
            }
            context.save_hyphenation_codes(
                0,
                (0_u8..=u8::MAX).map(|code| {
                    let ch = char::from(code);
                    (ch, ch)
                }),
            );
            context.close_hyphenation_patterns();
        }

        let checkpoint = universe.runtime_checkpoint().expect("seed checkpoint");
        let (_, capture) = measured(|| {
            for _ in 0..ITERATIONS {
                black_box(universe.runtime_checkpoint().expect("capture checkpoint"));
            }
        });
        let (_, checkpoint_clone) = measured(|| {
            for _ in 0..ITERATIONS {
                black_box(checkpoint.clone());
            }
        });
        let (_, restore) = measured(|| {
            for _ in 0..ITERATIONS {
                universe
                    .restore_runtime_checkpoint_with_roots(&checkpoint, || {})
                    .expect("restore checkpoint");
            }
        });
        let (fork, fork_measurement) = measured(|| {
            universe
                .fork_runtime_checkpoint(&checkpoint)
                .expect("fork checkpoint")
        });
        black_box(fork);

        TrieMeasurement {
            pattern_nodes,
            capture,
            checkpoint_clone,
            restore,
            fork: fork_measurement,
        }
    })
    .expect("profile universe")
}

fn assert_flat(small: &TrieMeasurement, large: &TrieMeasurement) {
    for (name, small, large) in [
        ("capture", small.capture, large.capture),
        (
            "checkpoint_clone",
            small.checkpoint_clone,
            large.checkpoint_clone,
        ),
        ("restore", small.restore, large.restore),
        ("fork", small.fork, large.fork),
    ] {
        assert_eq!(
            small.allocations, large.allocations,
            "{name} allocation must be independent of initialized trie size"
        );
    }
}

fn print_measurement(measurement: &TrieMeasurement) {
    for (name, operation, iterations) in [
        ("capture", measurement.capture, ITERATIONS),
        (
            "checkpoint_clone",
            measurement.checkpoint_clone,
            ITERATIONS,
        ),
        ("restore", measurement.restore, ITERATIONS),
        ("fork", measurement.fork, 1),
    ] {
        println!(
            "HYPHEN_CHECKPOINT_GATE pattern_nodes={} operation={} iterations={} ns_per_op={} allocations={} requested_bytes={}",
            measurement.pattern_nodes,
            name,
            iterations,
            operation.elapsed.as_nanos() / iterations as u128,
            operation.allocations.calls,
            operation.allocations.requested_bytes,
        );
    }
}

fn main() {
    let small = profile(64);
    let large = profile(7_000);
    assert_flat(&small, &large);
    print_measurement(&small);
    print_measurement(&large);
}
