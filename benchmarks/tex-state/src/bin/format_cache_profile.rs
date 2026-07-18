#![allow(
    clippy::disallowed_methods,
    reason = "this opt-in native profiler deliberately reads a fixture and measures wall time"
)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tex_state::{Universe, World};
use umber_fetch::{
    FormatCacheClock, FormatCacheIdentity, FormatCacheStore, FormatEngineMode, FormatFingerprint,
};

struct TrackingAllocator;

static LIVE_REQUESTED_BYTES: AtomicU64 = AtomicU64::new(0);
static PEAK_REQUESTED_BYTES: AtomicU64 = AtomicU64::new(0);

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

// SAFETY: every operation delegates to System with the original pointer/layout.
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
        // SAFETY: delegated with the allocation's original pointer/layout.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: delegated with the allocation's original pointer/layout.
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
struct Observation {
    median: Duration,
    retained_bytes: u64,
    peak_bytes: u64,
}

fn main() {
    let format_path = std::env::args_os().nth(1).map_or_else(
        || PathBuf::from("crates/umber-wasm/assets/plain.fmt"),
        PathBuf::from,
    );
    let format = std::fs::read(&format_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", format_path.display()));
    Universe::from_format(World::memory(), &format).expect("profile input must be schema-10");

    let root =
        std::env::temp_dir().join(format!("umber-format-cache-profile-{}", std::process::id()));
    assert!(!root.exists(), "refusing to replace {}", root.display());
    let cache = FormatCacheStore::new(&root);
    let identity = FormatCacheIdentity::current(
        FormatEngineMode::Tex82,
        FormatFingerprint::sha256(b"profile-distribution"),
        FormatFingerprint::sha256(b"profile-closure"),
        FormatFingerprint::sha256(b"profile-source-lock"),
        FormatCacheClock {
            time: 0,
            second: 0,
            day: 1,
            month: 1,
            year: 1970,
        },
        FormatFingerprint::sha256(b"profile-release-build"),
    );

    let miss = observe(21, || {
        assert!(cache.load(&identity).expect("cold cache probe").is_none());
    });
    let first_store = observe(1, || cache.store(&identity, &format).expect("first store"));
    let hit = observe(21, || {
        let loaded = cache.load(&identity).expect("warm load").expect("hit");
        black_box(loaded.as_bytes().len());
    });
    let repeated_store = observe(21, || {
        cache
            .store(&identity, &format)
            .expect("validated repeated store");
    });
    let direct_decode = observe(21, || {
        let universe = Universe::from_format(World::memory(), &format).expect("direct decode");
        black_box(universe);
    });

    println!(
        "schema={} format_bytes={}",
        Universe::FORMAT_SCHEMA_VERSION,
        format.len()
    );
    print("cold_miss", miss);
    print("first_store", first_store);
    print("warm_hit", hit);
    print("validated_repeated_store", repeated_store);
    print("direct_format_decode", direct_decode);
    std::fs::remove_dir_all(&root).expect("remove owned profile directory");
}

fn observe(samples: usize, mut operation: impl FnMut()) -> Observation {
    let mut durations = Vec::with_capacity(samples);
    let baseline = LIVE_REQUESTED_BYTES.load(Ordering::Relaxed);
    PEAK_REQUESTED_BYTES.store(baseline, Ordering::Relaxed);
    for _ in 0..samples {
        let started = Instant::now();
        operation();
        durations.push(started.elapsed());
    }
    durations.sort_unstable();
    Observation {
        median: durations[durations.len() / 2],
        retained_bytes: LIVE_REQUESTED_BYTES
            .load(Ordering::Relaxed)
            .saturating_sub(baseline),
        peak_bytes: PEAK_REQUESTED_BYTES
            .load(Ordering::Relaxed)
            .saturating_sub(baseline),
    }
}

fn print(name: &str, observation: Observation) {
    println!(
        "{name} median_us={} retained_requested_bytes={} peak_requested_bytes={}",
        observation.median.as_micros(),
        observation.retained_bytes,
        observation.peak_bytes
    );
}
