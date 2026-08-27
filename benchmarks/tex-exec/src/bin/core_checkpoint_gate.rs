use std::alloc::System;
use std::hint::black_box;
use std::time::{Duration, Instant};

use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};
use tex_state::glue::GlueSpec;
use tex_state::interner::InternerBudget;
use tex_state::meaning::Meaning;
use tex_state::node::Node;
use tex_state::provenance::OriginRecord;
use tex_state::{AssignmentScope, GroupKind, Universe, with_universe};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const ACCUMULATED_UNITS: usize = 256;
const SAMPLES: usize = 17;

#[derive(Clone, Copy, Debug)]
enum Operation {
    Fork,
    Restore,
    Reject,
    Accept,
    FirstMutation,
}

#[derive(Clone, Copy)]
struct Measurement {
    elapsed: Duration,
    stats: Stats,
    checksum: u64,
}

fn main() {
    for operation in [
        Operation::Fork,
        Operation::Restore,
        Operation::Reject,
        Operation::Accept,
        Operation::FirstMutation,
    ] {
        let shallow = samples(operation, 1, 1);
        let accumulated = samples(operation, ACCUMULATED_UNITS, ACCUMULATED_UNITS);
        assert_eq!(
            shallow[0].stats.allocations, accumulated[0].stats.allocations,
            "{operation:?} allocations scale with accumulated core payload"
        );
        assert_eq!(
            shallow[0].stats.bytes_allocated, accumulated[0].stats.bytes_allocated,
            "{operation:?} requested bytes scale with accumulated core payload"
        );
        let shallow_median = median_elapsed(&shallow);
        let accumulated_median = median_elapsed(&accumulated);
        assert!(
            accumulated_median <= shallow_median.saturating_mul(4),
            "{operation:?} CPU work scales with accumulated core payload: shallow={shallow_median:?} accumulated={accumulated_median:?}"
        );
        let checksum = shallow
            .iter()
            .chain(&accumulated)
            .fold(0_u64, |value, sample| {
                value ^ sample.checksum.rotate_left(7)
            });
        println!(
            "CORE_CHECKPOINT_GATE operation={operation:?} shallow_allocations={} accumulated_allocations={} shallow_requested_bytes={} accumulated_requested_bytes={} shallow_median_ns={} accumulated_median_ns={} checksum={checksum}",
            shallow[0].stats.allocations,
            accumulated[0].stats.allocations,
            shallow[0].stats.bytes_allocated,
            accumulated[0].stats.bytes_allocated,
            shallow_median.as_nanos(),
            accumulated_median.as_nanos(),
        );
    }
}

fn samples(operation: Operation, before: usize, after: usize) -> Vec<Measurement> {
    (0..SAMPLES)
        .map(|_| sample(operation, before, after))
        .collect()
}

fn sample(operation: Operation, before: usize, after: usize) -> Measurement {
    with_universe(budget(), |universe| {
        populate_before_checkpoint(universe, before);
        let checkpoint = universe.runtime_checkpoint().expect("early checkpoint");
        populate_after_checkpoint(universe, after);

        let measurement = match operation {
            Operation::Fork => {
                let region = Region::new(GLOBAL);
                let start = Instant::now();
                let mut candidate = universe
                    .fork_runtime_checkpoint(&checkpoint)
                    .expect("fork checkpoint bank");
                let elapsed = start.elapsed();
                let stats = region.change();
                let checksum = candidate.primitive_registry_len() as u64;
                universe.return_rejected_pdf_from(&mut candidate);
                Measurement {
                    elapsed,
                    stats,
                    checksum,
                }
            }
            Operation::Restore => {
                let region = Region::new(GLOBAL);
                let start = Instant::now();
                universe
                    .restore_runtime_checkpoint_with_roots(&checkpoint, || {})
                    .expect("restore checkpoint bank");
                let elapsed = start.elapsed();
                let stats = region.change();
                Measurement {
                    elapsed,
                    stats,
                    checksum: semantic_checksum(universe, 11),
                }
            }
            Operation::Reject => {
                let mut candidate = universe
                    .fork_runtime_checkpoint(&checkpoint)
                    .expect("fork checkpoint bank");
                mutate_candidate(&mut candidate, 31);
                let region = Region::new(GLOBAL);
                let start = Instant::now();
                universe.return_rejected_pdf_from(&mut candidate);
                let elapsed = start.elapsed();
                let stats = region.change();
                Measurement {
                    elapsed,
                    stats,
                    checksum: universe.primitive_registry_len() as u64,
                }
            }
            Operation::Accept => {
                let mut candidate = universe
                    .fork_runtime_checkpoint(&checkpoint)
                    .expect("fork checkpoint bank");
                mutate_candidate(&mut candidate, 41);
                let region = Region::new(GLOBAL);
                let start = Instant::now();
                candidate.profile_commit_checkpoint_candidate();
                let elapsed = start.elapsed();
                let stats = region.change();
                Measurement {
                    elapsed,
                    stats,
                    checksum: semantic_checksum(&candidate, 41),
                }
            }
            Operation::FirstMutation => {
                let mut candidate = universe
                    .fork_runtime_checkpoint(&checkpoint)
                    .expect("fork checkpoint bank");
                let region = Region::new(GLOBAL);
                let start = Instant::now();
                mutate_candidate(&mut candidate, 51);
                let elapsed = start.elapsed();
                let stats = region.change();
                let checksum = semantic_checksum(&candidate, 51);
                universe.return_rejected_pdf_from(&mut candidate);
                Measurement {
                    elapsed,
                    stats,
                    checksum,
                }
            }
        };
        black_box(measurement)
    })
    .expect("core checkpoint gate universe")
}

fn populate_before_checkpoint<G>(universe: &mut Universe<G>, units: usize) {
    universe
        .assign_count(60_000, 11, AssignmentScope::Global)
        .expect("baseline sentinel");
    for index in 0..units {
        universe.register_primitive_meaning(&format!("coregateprimitive{index}"), Meaning::Relax);
        if index % 32 == 0 {
            universe
                .begin_group(GroupKind::Simple, index as u32)
                .expect("open save-history group");
        }
        universe
            .assign_count(index as u16, index as i32, AssignmentScope::Local)
            .expect("dense write");
        universe
            .allocate_glue(GlueSpec::ZERO)
            .expect("generation-local glue");
        universe
            .allocate_provenance(OriginRecord::UnknownBootstrap)
            .expect("generation-local provenance");
        let nodes = universe.publish_page_nodes(&[Node::Penalty(index as i32)]);
        universe.assign_page_box_global(index as u16, nodes);
    }
}

fn populate_after_checkpoint<G>(universe: &mut Universe<G>, units: usize) {
    universe
        .assign_count(60_000, 22, AssignmentScope::Global)
        .expect("accepted sentinel suffix");
    for index in 0..units {
        let register = 30_000_u16.saturating_add(index as u16);
        universe
            .assign_count(register, -(index as i32), AssignmentScope::Local)
            .expect("accepted dense suffix");
        universe
            .allocate_glue(GlueSpec::ZERO)
            .expect("accepted glue suffix");
        universe
            .allocate_provenance(OriginRecord::UnknownBootstrap)
            .expect("accepted provenance suffix");
        let nodes = universe.publish_page_nodes(&[Node::Penalty(-(index as i32))]);
        universe.assign_page_box_global(register, nodes);
    }
}

fn mutate_candidate<G>(universe: &mut Universe<G>, value: i32) {
    universe
        .assign_count(60_000, value, AssignmentScope::Global)
        .expect("candidate dense mutation");
    universe
        .allocate_glue(GlueSpec::ZERO)
        .expect("candidate glue suffix");
    universe
        .allocate_provenance(OriginRecord::UnknownBootstrap)
        .expect("candidate provenance suffix");
    let nodes = universe.publish_page_nodes(&[Node::Penalty(value)]);
    universe.assign_page_box_global(60_000, nodes);
}

fn semantic_checksum<G>(universe: &Universe<G>, expected: i32) -> u64 {
    let value = universe.count(60_000).expect("profile count");
    assert_eq!(value, expected);
    value as u64 ^ (universe.primitive_registry_len() as u64).rotate_left(17)
}

fn median_elapsed(samples: &[Measurement]) -> Duration {
    let mut elapsed = samples
        .iter()
        .map(|sample| sample.elapsed)
        .collect::<Vec<_>>();
    elapsed.sort_unstable();
    elapsed[elapsed.len() / 2]
}

fn budget() -> InternerBudget {
    InternerBudget::new(4096, 8192, 1 << 20).expect("core checkpoint gate interner budget")
}
