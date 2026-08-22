use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::mem::size_of;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tex_state::cell::{BankTag, CellId};
use tex_state::{DependencyKey, DependencyRuntime, DependencyValue, ObservedDependency};

const HOT_ITERATIONS: u32 = 2_000_000;
const UNIQUE_FACTS: u32 = 4_096;
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

fn main() {
    warm_up();
    let control = median_sample(run_read_control);
    let disabled = median_sample(run_disabled_reads);
    let unique_reads = median_sample(run_unique_reads);
    let unchanged_validation = median_sample(|| run_validation(false));
    let backdated_validation = median_sample(|| run_validation(true));
    let (retained_bytes, peak_bytes, observations) = retained_observations();

    println!("tracked-region dependency gate");
    println!("hot_iterations={HOT_ITERATIONS} unique_facts={UNIQUE_FACTS} samples={SAMPLES}");
    println!(
        "logical_sizes_bytes dependency_key={} dependency_value={} observed_dependency={}",
        size_of::<DependencyKey>(),
        size_of::<DependencyValue>(),
        size_of::<ObservedDependency>(),
    );
    println!(
        "disabled_read control_ns={:.3} disabled_ns={:.3} incremental_ns={:.3}",
        ns_per(control, HOT_ITERATIONS),
        ns_per(disabled, HOT_ITERATIONS),
        signed_ns_per(disabled, control, HOT_ITERATIONS),
    );
    println!(
        "active_unique_read ns_per_fact={:.3} validation_unchanged_ns_per_fact={:.3} validation_backdated_ns_per_fact={:.3}",
        ns_per(unique_reads, UNIQUE_FACTS),
        ns_per(unchanged_validation, UNIQUE_FACTS),
        ns_per(backdated_validation, UNIQUE_FACTS),
    );
    println!(
        "retained observations={} retained_bytes={} peak_bytes={}",
        observations.len(),
        retained_bytes,
        peak_bytes,
    );
    black_box(observations);
}

fn warm_up() {
    black_box(run_read_control());
    black_box(run_disabled_reads());
    black_box(run_unique_reads());
    black_box(run_validation(false));
    black_box(run_validation(true));
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
        runtime.record(
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
        runtime.record(
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

fn run_validation(stamps_changed: bool) -> Duration {
    let (runtime, mut observations) = validation_case(stamps_changed);
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

fn retained_observations() -> (i128, u64, Vec<ObservedDependency>) {
    let baseline = LIVE_REQUESTED_BYTES.load(Ordering::Relaxed);
    PEAK_REQUESTED_BYTES.store(baseline, Ordering::Relaxed);
    let (_, observations) = validation_case(false);
    let retained = i128::from(LIVE_REQUESTED_BYTES.load(Ordering::Relaxed)) - i128::from(baseline);
    let peak = PEAK_REQUESTED_BYTES
        .load(Ordering::Relaxed)
        .saturating_sub(baseline);
    (retained, peak, observations)
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
