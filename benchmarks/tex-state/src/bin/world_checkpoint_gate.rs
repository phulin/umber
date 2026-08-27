use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};

use tex_state::World;
use tex_state::world::{PrintSink, StreamSlot};

const SMALL_UNITS: usize = 1;
const LARGE_UNITS: usize = 64;

struct CountingAllocator;

static ALLOCATION_CALLS: AtomicU64 = AtomicU64::new(0);
static REQUESTED_BYTES: AtomicU64 = AtomicU64::new(0);

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

// SAFETY: every allocator operation delegates unchanged to `System`; the
// relaxed counters are observation-only and cannot affect allocation results.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: delegated with the caller-provided valid layout.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            REQUESTED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: delegated with the caller-provided valid layout.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            REQUESTED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: delegated with the allocation's original pointer and layout.
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: delegated with the allocation's original pointer and layout.
        let new_pointer = unsafe { System.realloc(pointer, layout, new_size) };
        if !new_pointer.is_null() && new_size > layout.size() {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            REQUESTED_BYTES.fetch_add((new_size - layout.size()) as u64, Ordering::Relaxed);
        }
        new_pointer
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AllocationStats {
    calls: u64,
    bytes: u64,
}

fn main() {
    let small = run_case(SMALL_UNITS);
    let large = run_case(LARGE_UNITS);

    assert_eq!(small.capture, AllocationStats { calls: 0, bytes: 0 });
    assert_eq!(small.clone, AllocationStats { calls: 0, bytes: 0 });
    assert_eq!(small.restore, AllocationStats { calls: 0, bytes: 0 });
    assert_eq!(large.capture, small.capture, "capture scales with effects");
    assert_eq!(large.clone, small.clone, "mark clone scales with effects");
    assert_eq!(large.restore, small.restore, "restore scales with effects");
    assert_eq!(large.fork, small.fork, "fork scales with accepted effects");

    println!("world checkpoint allocation gate");
    println!(
        "small_units={SMALL_UNITS} capture={:?} clone={:?} fork={:?} restore={:?}",
        small.capture, small.clone, small.fork, small.restore
    );
    println!(
        "large_units={LARGE_UNITS} capture={:?} clone={:?} fork={:?} restore={:?}",
        large.capture, large.clone, large.fork, large.restore
    );
}

#[derive(Clone, Copy)]
struct CaseStats {
    capture: AllocationStats,
    clone: AllocationStats,
    fork: AllocationStats,
    restore: AllocationStats,
}

fn run_case(units: usize) -> CaseStats {
    let mut world = World::memory();
    world.profile_begin_retained_session();
    world.open_out(StreamSlot::new(0), "world-checkpoint.out");
    for index in 0..units {
        let path = format!("input-{index:04}.tex");
        world
            .set_memory_file(&path, vec![index as u8; 96])
            .expect("profiling input is seeded");
        black_box(world.read_file(&path).expect("profiling input is read"));
        world.write_text(PrintSink::Stream(StreamSlot::new(0)), "bounded-line");
        world.record_special("checkpoint", vec![index as u8; 128]);
    }
    world.profile_publish_artifact(vec![0x5a; units * 256]);

    let (checkpoint, capture) = measure(|| world.profile_checkpoint_capture());
    let (cloned, clone) = measure(|| checkpoint.clone());
    let (candidate, fork) = measure(|| world.profile_checkpoint_fork(&cloned));
    let ((), restore) = measure(|| world.profile_checkpoint_restore(&checkpoint));

    black_box((world, checkpoint, cloned, candidate));
    CaseStats {
        capture,
        clone,
        fork,
        restore,
    }
}

fn measure<T>(operation: impl FnOnce() -> T) -> (T, AllocationStats) {
    let calls = ALLOCATION_CALLS.load(Ordering::Relaxed);
    let bytes = REQUESTED_BYTES.load(Ordering::Relaxed);
    let value = operation();
    black_box(&value);
    (
        value,
        AllocationStats {
            calls: ALLOCATION_CALLS.load(Ordering::Relaxed) - calls,
            bytes: REQUESTED_BYTES.load(Ordering::Relaxed) - bytes,
        },
    )
}
