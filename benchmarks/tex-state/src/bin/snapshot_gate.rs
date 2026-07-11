use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tex_state_benchmarks::{
    LATENCY_NOISE_ALLOWANCE_NS, LATENCY_SCALE_BUDGET, RETAINED_BYTES_PER_CAPTURE_BUDGET,
    RETAINED_CAPTURES, WORKLOADS, WorkloadKind, build_workload,
};

struct TrackingAllocator;

static LIVE_REQUESTED_BYTES: AtomicU64 = AtomicU64::new(0);
static PEAK_REQUESTED_BYTES: AtomicU64 = AtomicU64::new(0);

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

// SAFETY: every operation delegates to System with the original pointer/layout,
// and the counters do not affect allocation behavior.
unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: delegated with the caller-provided valid layout.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            add_live(layout.size() as u64);
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: delegated with the caller-provided valid layout.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            add_live(layout.size() as u64);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE_REQUESTED_BYTES.fetch_sub(layout.size() as u64, Ordering::Relaxed);
        // SAFETY: delegated with the allocation's original pointer/layout.
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: delegated with the allocation's original pointer/layout.
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            if new_size >= layout.size() {
                add_live((new_size - layout.size()) as u64);
            } else {
                LIVE_REQUESTED_BYTES
                    .fetch_sub((layout.size() - new_size) as u64, Ordering::Relaxed);
            }
        }
        new_ptr
    }
}

fn add_live(bytes: u64) {
    let live = LIVE_REQUESTED_BYTES.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK_REQUESTED_BYTES.fetch_max(live, Ordering::Relaxed);
}

#[derive(Clone, Copy)]
struct AllocationObservation {
    retained_bytes: u64,
    peak_bytes: u64,
}

#[derive(Clone, Copy)]
struct GateObservation {
    logical_live_bytes: u64,
    median_latency: Duration,
    one_capture: AllocationObservation,
    retained_run: AllocationObservation,
}

fn main() {
    let enforce = std::env::args().any(|arg| arg == "--enforce");
    let mut failures = Vec::new();
    println!(
        "workload scale logical_live_bytes median_ns one_retained_bytes one_peak_bytes retained_{}_bytes retained_{}_peak_bytes",
        RETAINED_CAPTURES, RETAINED_CAPTURES
    );
    for kind in WORKLOADS {
        let small = observe(kind, kind.small_units());
        let large = observe(kind, kind.large_units());
        print_observation(kind, "small", small);
        print_observation(kind, "large", large);
        check(kind, small, large, &mut failures);
    }
    if failures.is_empty() {
        println!("snapshot-gate: all budgets met");
    } else if enforce {
        for failure in &failures {
            eprintln!("snapshot-gate: {failure}");
        }
        std::process::exit(1);
    } else {
        println!(
            "snapshot-gate: {} budget violation(s); rerun with --enforce to fail",
            failures.len()
        );
    }
}

// This standalone benchmark is the clock-owning measurement boundary; engine
// code continues to obtain semantic time only through World.
#[allow(clippy::disallowed_methods)]
fn observe(kind: WorkloadKind, units: usize) -> GateObservation {
    let mut workload = build_workload(kind, units);
    workload.warm_capture();
    let logical_live_bytes = workload.logical_live_bytes();
    let mut timings = Vec::with_capacity(31);
    for _ in 0..31 {
        let start = Instant::now();
        let captured = black_box(workload.capture());
        timings.push(start.elapsed());
        drop(captured);
    }
    timings.sort_unstable();

    let (one_capture, captured) = allocation_delta(|| black_box(workload.capture()));
    black_box(&captured);
    drop(captured);
    let mut captures = Vec::with_capacity(RETAINED_CAPTURES);
    let (retained_run, ()) = allocation_delta(|| {
        for _ in 0..RETAINED_CAPTURES {
            captures.push(workload.capture());
        }
        black_box(&captures);
    });
    drop(captures);

    GateObservation {
        logical_live_bytes,
        median_latency: timings[timings.len() / 2],
        one_capture,
        retained_run,
    }
}

fn allocation_delta<T>(operation: impl FnOnce() -> T) -> (AllocationObservation, T) {
    let baseline = LIVE_REQUESTED_BYTES.load(Ordering::SeqCst);
    PEAK_REQUESTED_BYTES.store(baseline, Ordering::SeqCst);
    let result = operation();
    let retained = LIVE_REQUESTED_BYTES
        .load(Ordering::SeqCst)
        .saturating_sub(baseline);
    let peak = PEAK_REQUESTED_BYTES
        .load(Ordering::SeqCst)
        .saturating_sub(baseline);
    (
        AllocationObservation {
            retained_bytes: retained,
            peak_bytes: peak,
        },
        result,
    )
}

fn print_observation(kind: WorkloadKind, scale: &str, value: GateObservation) {
    println!(
        "{} {} {} {} {} {} {} {}",
        kind.name(),
        scale,
        value.logical_live_bytes,
        value.median_latency.as_nanos(),
        value.one_capture.retained_bytes,
        value.one_capture.peak_bytes,
        value.retained_run.retained_bytes,
        value.retained_run.peak_bytes,
    );
}

fn check(
    kind: WorkloadKind,
    small: GateObservation,
    large: GateObservation,
    failures: &mut Vec<String>,
) {
    let latency_limit = small
        .median_latency
        .as_nanos()
        .saturating_mul(LATENCY_SCALE_BUDGET)
        .saturating_add(LATENCY_NOISE_ALLOWANCE_NS);
    if large.median_latency.as_nanos() > latency_limit {
        failures.push(format!(
            "{} capture latency scales with payload: small={}ns large={}ns limit={}ns",
            kind.name(),
            small.median_latency.as_nanos(),
            large.median_latency.as_nanos(),
            latency_limit,
        ));
    }
    if large.one_capture.retained_bytes > RETAINED_BYTES_PER_CAPTURE_BUDGET {
        failures.push(format!(
            "{} one capture retained {} bytes (budget {})",
            kind.name(),
            large.one_capture.retained_bytes,
            RETAINED_BYTES_PER_CAPTURE_BUDGET,
        ));
    }
    let retained_budget = RETAINED_BYTES_PER_CAPTURE_BUDGET * RETAINED_CAPTURES as u64;
    if large.retained_run.retained_bytes > retained_budget {
        failures.push(format!(
            "{} retained {} bytes across {} captures (budget {})",
            kind.name(),
            large.retained_run.retained_bytes,
            RETAINED_CAPTURES,
            retained_budget,
        ));
    }
}
