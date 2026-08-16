use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};

use criterion::{Criterion, criterion_group, criterion_main};
use tex_state::measurement::{FormatRestoreMeasurement, format_restore_measurement};
use tex_state::{Universe, World};

const PLAIN_FORMAT: &[u8] = include_bytes!("../../../crates/umber-wasm/assets/plain.fmt");

struct CountingAllocator;

static ALLOCATION_CALLS: AtomicU64 = AtomicU64::new(0);
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

// SAFETY: every operation delegates to `System` with the caller's unchanged
// pointer and layout. Only successful allocation requests update counters.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: delegated with the caller-provided valid layout.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            ALLOCATED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: delegated with the caller-provided valid layout.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            ALLOCATED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: delegated with the allocation's original pointer and layout.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: delegated with the allocation's original pointer and layout.
        let new_pointer = unsafe { System.realloc(pointer, layout, new_size) };
        if !new_pointer.is_null() {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            ALLOCATED_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        }
        new_pointer
    }
}

fn decode(c: &mut Criterion) {
    let restored = Universe::from_format(World::memory(), PLAIN_FORMAT)
        .expect("pinned Plain format must restore");
    assert_eq!(
        restored.dump_format().expect("restored format must redump"),
        PLAIN_FORMAT,
        "the focused workload must preserve pinned format identity"
    );
    drop(restored);

    let work_before = format_restore_measurement();
    let calls_before = ALLOCATION_CALLS.load(Ordering::Relaxed);
    let bytes_before = ALLOCATED_BYTES.load(Ordering::Relaxed);
    let restored = Universe::from_format(World::memory(), PLAIN_FORMAT)
        .expect("pinned Plain format must restore");
    let work = format_restore_measurement().saturating_sub(work_before);
    let allocation_calls = ALLOCATION_CALLS
        .load(Ordering::Relaxed)
        .saturating_sub(calls_before);
    let allocated_bytes = ALLOCATED_BYTES
        .load(Ordering::Relaxed)
        .saturating_sub(bytes_before);
    assert_eq!(work.calls, 1);
    assert_eq!(work.bytes_decoded, PLAIN_FORMAT.len() as u64);
    print_work(work, allocation_calls, allocated_bytes);
    drop(restored);

    c.bench_function("loaded_format_decode/plain_schema_11", |b| {
        b.iter(|| {
            let universe = Universe::from_format(World::memory(), black_box(PLAIN_FORMAT))
                .expect("pinned Plain format must restore");
            black_box(universe);
        });
    });
}

fn print_work(work: FormatRestoreMeasurement, allocation_calls: u64, allocated_bytes: u64) {
    eprintln!(
        "FORMAT_RESTORE_BENCH bytes_decoded={} token_entries={} macro_entries={} glue_entries={} node_entries={} validation_passes={} copies={} explicit_allocations={} allocator_calls={} allocated_bytes={}",
        work.bytes_decoded,
        work.token_entries_restored,
        work.macro_entries_restored,
        work.glue_entries_restored,
        work.node_entries_restored,
        work.validation_passes,
        work.copies,
        work.allocations,
        allocation_calls,
        allocated_bytes,
    );
}

criterion_group!(benches, decode);
criterion_main!(benches);
