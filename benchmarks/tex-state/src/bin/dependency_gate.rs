use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::mem::size_of;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tex_state::cell::{BankTag, CellId};
use tex_state::{
    DependencyKey, DependencyRuntime, DependencyValue, ObservedDependency, TrackedRegionRecord,
    Universe,
};

const HOT_ITERATIONS: u32 = 2_000_000;
const UNIQUE_FACTS: u32 = 4_096;
const ENVIRONMENT_WRITES: u32 = 256;
const SAMPLES: usize = 9;

struct TrackingAllocator;

static LIVE_REQUESTED_BYTES: AtomicU64 = AtomicU64::new(0);
static PEAK_REQUESTED_BYTES: AtomicU64 = AtomicU64::new(0);

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

// SAFETY: every operation delegates to System with the allocation's original
// pointer and layout. The relaxed counters cannot affect allocator behavior.
unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: delegated with the caller-provided valid layout.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            add_live(layout.size() as u64);
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: delegated with the caller-provided valid layout.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            add_live(layout.size() as u64);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE_REQUESTED_BYTES.fetch_sub(layout.size() as u64, Ordering::Relaxed);
        // SAFETY: delegated with the allocation's original pointer and layout.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: delegated with the allocation's original pointer and layout.
        let new_pointer = unsafe { System.realloc(pointer, layout, new_size) };
        if !new_pointer.is_null() {
            if new_size >= layout.size() {
                add_live((new_size - layout.size()) as u64);
            } else {
                LIVE_REQUESTED_BYTES
                    .fetch_sub((layout.size() - new_size) as u64, Ordering::Relaxed);
            }
        }
        new_pointer
    }
}

fn add_live(bytes: u64) {
    let live = LIVE_REQUESTED_BYTES.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK_REQUESTED_BYTES.fetch_max(live, Ordering::Relaxed);
}

#[derive(Clone, Copy)]
struct AllocationObservation {
    retained_bytes: i128,
    peak_bytes: u64,
}

fn main() {
    warm_up();

    let control = median_sample(run_read_control);
    let disabled = median_sample(run_disabled_reads);
    let unique_reads = median_sample(run_unique_reads);
    let disabled_mutations = median_sample(|| run_mutations(false));
    let tracked_mutations = median_sample(|| run_mutations(true));
    let unchanged_validation = median_sample(run_unchanged_validation);
    let backdated_validation = median_sample(run_backdated_validation);
    let footprint_extraction = median_sample(run_footprint_extraction);

    let (observation_retention, observations) = allocation_delta(build_observation_record);
    let (tracker_retention, tracker) = allocation_delta(build_changed_tracker);
    let (footprint_retention, footprint) = allocation_delta(build_write_footprint);
    let rollback = observe_rollback();

    println!("tracked-region dependency gate");
    println!(
        "hot_iterations={HOT_ITERATIONS} unique_facts={UNIQUE_FACTS} environment_writes={ENVIRONMENT_WRITES} samples={SAMPLES}"
    );
    println!(
        "logical_sizes_bytes dependency_key={} dependency_value={} observed_dependency={} tracked_region_record={}",
        size_of::<DependencyKey>(),
        size_of::<DependencyValue>(),
        size_of::<ObservedDependency>(),
        size_of::<TrackedRegionRecord>(),
    );
    println!(
        "disabled_read control_ns={:.3} disabled_ns={:.3} incremental_ns={:.3}",
        ns_per(control, HOT_ITERATIONS),
        ns_per(disabled, HOT_ITERATIONS),
        signed_ns_per(disabled, control, HOT_ITERATIONS),
    );
    println!(
        "active_unique_read ns_per_fact={:.3} facts={UNIQUE_FACTS}",
        ns_per(unique_reads, UNIQUE_FACTS),
    );
    println!(
        "mutation_receipt disabled_ns={:.3} tracked_ns={:.3} incremental_ns={:.3}",
        ns_per(disabled_mutations, HOT_ITERATIONS),
        ns_per(tracked_mutations, HOT_ITERATIONS),
        signed_ns_per(tracked_mutations, disabled_mutations, HOT_ITERATIONS),
    );
    println!(
        "dependency_validation unchanged_ns_per_fact={:.3} backdated_ns_per_fact={:.3} facts={UNIQUE_FACTS}",
        ns_per(unchanged_validation, UNIQUE_FACTS),
        ns_per(backdated_validation, UNIQUE_FACTS),
    );
    println!(
        "write_footprint_extraction ns_per_write={:.3} writes={ENVIRONMENT_WRITES}",
        ns_per(footprint_extraction, ENVIRONMENT_WRITES),
    );
    println!(
        "rollback writes={} rollback_ns_per_write={:.3} validation_backdated={} retained_bytes={} peak_bytes={}",
        ENVIRONMENT_WRITES,
        ns_per(rollback.duration, ENVIRONMENT_WRITES),
        rollback.validation_backdated,
        rollback.allocation.retained_bytes,
        rollback.allocation.peak_bytes,
    );
    println!(
        "retained observations={} observations_bytes={} observations_peak_bytes={} tracker_keys={} tracker_bytes={} tracker_peak_bytes={} footprint_writes={} footprint_bytes={} footprint_peak_bytes={}",
        observations.len(),
        observation_retention.retained_bytes,
        observation_retention.peak_bytes,
        UNIQUE_FACTS,
        tracker_retention.retained_bytes,
        tracker_retention.peak_bytes,
        footprint.environment_writes().len(),
        footprint_retention.retained_bytes,
        footprint_retention.peak_bytes,
    );

    black_box((observations, tracker, footprint));
}

fn warm_up() {
    black_box(run_read_control());
    black_box(run_disabled_reads());
    black_box(run_unique_reads());
    black_box(run_mutations(false));
    black_box(run_mutations(true));
    black_box(run_unchanged_validation());
    black_box(run_backdated_validation());
    black_box(run_footprint_extraction());
}

fn median_sample(mut operation: impl FnMut() -> Duration) -> Duration {
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        samples.push(operation());
    }
    samples.sort_unstable();
    samples[SAMPLES / 2]
}

fn run_read_control() -> Duration {
    let started = Instant::now();
    for index in 0..HOT_ITERATIONS {
        black_box((count_key(index), DependencyValue::Integer(i64::from(index))));
    }
    started.elapsed()
}

fn run_disabled_reads() -> Duration {
    let mut runtime = DependencyRuntime::default();
    let started = Instant::now();
    for index in 0..HOT_ITERATIONS {
        black_box(&mut runtime).record(
            black_box(count_key(index)),
            black_box(DependencyValue::Integer(i64::from(index))),
        );
    }
    black_box(runtime);
    started.elapsed()
}

fn run_unique_reads() -> Duration {
    let mut runtime = DependencyRuntime::default();
    let token = runtime.begin_region().expect("begin unique-read region");
    let started = Instant::now();
    for index in 0..UNIQUE_FACTS {
        black_box(&mut runtime).record(
            black_box(count_key(index)),
            black_box(DependencyValue::Integer(i64::from(index))),
        );
    }
    black_box(
        runtime
            .finish_region(token)
            .expect("finish unique-read region"),
    );
    started.elapsed()
}

fn run_mutations(tracked: bool) -> Duration {
    let mut universe = Universe::new();
    if tracked {
        universe.track_dependency(count_key(0));
    }
    universe.set_count(0, 1);
    let started = Instant::now();
    for value in 2..HOT_ITERATIONS + 2 {
        black_box(&mut universe).set_count(0, black_box(value as i32));
    }
    black_box(universe);
    started.elapsed()
}

fn run_unchanged_validation() -> Duration {
    let (runtime, mut observations) = validation_case(false);
    let started = Instant::now();
    let valid = runtime.tracker().validate_region(&mut observations, |_| {
        unreachable!("unchanged stamps must not request semantic values")
    });
    assert!(valid);
    black_box(observations);
    started.elapsed()
}

fn run_backdated_validation() -> Duration {
    let (runtime, mut observations) = validation_case(true);
    let started = Instant::now();
    let valid = runtime
        .tracker()
        .validate_region(&mut observations, current_value);
    assert!(valid);
    black_box(observations);
    started.elapsed()
}

fn validation_case(stamps_changed: bool) -> (DependencyRuntime, Vec<ObservedDependency>) {
    let mut runtime = DependencyRuntime::default();
    let token = runtime.begin_region().expect("begin validation region");
    for index in 0..UNIQUE_FACTS {
        runtime.record(count_key(index), DependencyValue::Integer(i64::from(index)));
    }
    let observations = runtime
        .finish_region(token)
        .expect("finish validation region");
    if stamps_changed {
        for index in 0..UNIQUE_FACTS {
            runtime.mark_changed(count_key(index));
        }
    }
    (runtime, observations)
}

fn run_footprint_extraction() -> Duration {
    let (mut universe, mark) = footprint_case();
    let started = Instant::now();
    let record = universe
        .finish_tracked_region(mark)
        .expect("extract tracked write footprint");
    assert_eq!(
        record.environment_writes().len(),
        ENVIRONMENT_WRITES as usize
    );
    black_box(record);
    started.elapsed()
}

fn footprint_case() -> (Universe, tex_state::TrackedRegionMark) {
    let mut universe = Universe::new();
    let mark = universe
        .begin_tracked_region()
        .expect("begin write-footprint region");
    for index in 0..ENVIRONMENT_WRITES {
        universe.set_count(index as u16, index as i32 + 1);
    }
    (universe, mark)
}

fn build_observation_record() -> Vec<ObservedDependency> {
    let mut runtime = DependencyRuntime::default();
    let token = runtime.begin_region().expect("begin retained-read region");
    for index in 0..UNIQUE_FACTS {
        runtime.record(count_key(index), DependencyValue::Integer(i64::from(index)));
    }
    runtime
        .finish_region(token)
        .expect("finish retained-read region")
}

fn build_changed_tracker() -> DependencyRuntime {
    let mut runtime = DependencyRuntime::default();
    runtime.track(count_key(0));
    for index in 0..UNIQUE_FACTS {
        runtime.mark_changed(count_key(index));
    }
    runtime
}

fn build_write_footprint() -> TrackedRegionRecord {
    let (mut universe, mark) = footprint_case();
    universe
        .finish_tracked_region(mark)
        .expect("finish retained write footprint")
}

struct RollbackObservation {
    duration: Duration,
    allocation: AllocationObservation,
    validation_backdated: bool,
}

fn observe_rollback() -> RollbackObservation {
    let mut universe = Universe::new();
    let mark = universe
        .begin_tracked_region()
        .expect("begin rollback read");
    universe.record_dependency(count_key(0), DependencyValue::Integer(0));
    let record = universe
        .finish_tracked_region(mark)
        .expect("finish rollback read");
    let mut observations = record.observations().to_vec();
    let snapshot = universe.snapshot();
    for index in 0..ENVIRONMENT_WRITES {
        universe.set_count(index as u16, index as i32 + 1);
    }

    let (allocation, duration) = allocation_delta(|| {
        let started = Instant::now();
        universe.rollback(&snapshot);
        started.elapsed()
    });
    let before = observations[0].changed_at;
    let valid = universe
        .validate_dependencies(observations.as_mut_slice(), |_| DependencyValue::Integer(0));
    let validation_backdated = valid && observations[0].changed_at > before;
    black_box((universe, snapshot, record, observations));
    RollbackObservation {
        duration,
        allocation,
        validation_backdated,
    }
}

fn allocation_delta<T>(operation: impl FnOnce() -> T) -> (AllocationObservation, T) {
    let before = LIVE_REQUESTED_BYTES.load(Ordering::Relaxed);
    PEAK_REQUESTED_BYTES.store(before, Ordering::Relaxed);
    let value = operation();
    let after = LIVE_REQUESTED_BYTES.load(Ordering::Relaxed);
    let peak = PEAK_REQUESTED_BYTES.load(Ordering::Relaxed);
    (
        AllocationObservation {
            retained_bytes: i128::from(after) - i128::from(before),
            peak_bytes: peak.saturating_sub(before),
        },
        value,
    )
}

fn count_key(index: u32) -> DependencyKey {
    DependencyKey::Cell(CellId::new(BankTag::Count, index))
}

fn current_value(key: DependencyKey) -> DependencyValue {
    let DependencyKey::Cell(cell) = key else {
        unreachable!("validation fixture uses only cells")
    };
    DependencyValue::Integer(i64::from(cell.index()))
}

fn ns_per(duration: Duration, operations: u32) -> f64 {
    duration.as_nanos() as f64 / f64::from(operations)
}

fn signed_ns_per(left: Duration, right: Duration, operations: u32) -> f64 {
    (left.as_nanos() as f64 - right.as_nanos() as f64) / f64::from(operations)
}
