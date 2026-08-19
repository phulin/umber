use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tex_state::hot_core_benchmark::TestingHotCore;

const CYCLES: u32 = 10_000;

struct CountingAllocator;

static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOCATION_CALLS: AtomicU64 = AtomicU64::new(0);
static REQUESTED_BYTES: AtomicU64 = AtomicU64::new(0);

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

// SAFETY: every operation delegates to System with the caller's original
// pointer and layout; relaxed counters do not affect allocation behavior.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            REQUESTED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        // SAFETY: delegated with the caller-provided valid layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            REQUESTED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        // SAFETY: delegated with the caller-provided valid layout.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: delegated with the allocation's original pointer/layout.
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            REQUESTED_BYTES.fetch_add(size as u64, Ordering::Relaxed);
        }
        // SAFETY: delegated with the allocation's original pointer/layout.
        unsafe { System.realloc(pointer, layout, size) }
    }
}

fn main() {
    let mut core = TestingHotCore::with_live_words(0);
    core.warm_bounded_cycle();
    let plateau = core.accounting();

    ALLOCATION_CALLS.store(0, Ordering::Relaxed);
    REQUESTED_BYTES.store(0, Ordering::Relaxed);
    COUNTING.store(true, Ordering::Release);
    let checksum = black_box(core.mixed_cycles(CYCLES));
    COUNTING.store(false, Ordering::Release);

    let calls = ALLOCATION_CALLS.load(Ordering::Relaxed);
    let bytes = REQUESTED_BYTES.load(Ordering::Relaxed);
    let final_accounting = core.accounting();
    assert_eq!(calls, 0, "warmed HotCore cycles allocated {calls} times");
    assert_eq!(bytes, 0, "warmed HotCore cycles requested {bytes} bytes");
    assert_eq!(final_accounting, plateau, "HotCore storage did not plateau");
    assert_eq!(TestingHotCore::snapshot_size(), 152);
    assert_eq!(TestingHotCore::snapshot_retained_bytes(), 0);
    println!(
        "hot-core-gate: cycles={CYCLES} allocations={calls} requested_bytes={bytes} snapshot_bytes={} retained_bytes={} checksum={checksum}",
        TestingHotCore::snapshot_size(),
        TestingHotCore::snapshot_retained_bytes(),
    );
}
