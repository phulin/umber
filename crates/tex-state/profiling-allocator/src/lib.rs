//! Profiling-only allocation attribution for Umber's named hot-core scopes.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

const OWNER_LIMIT: usize = 16;

static CALLS: [AtomicU64; OWNER_LIMIT] = [const { AtomicU64::new(0) }; OWNER_LIMIT];
static REQUESTED_BYTES: [AtomicU64; OWNER_LIMIT] = [const { AtomicU64::new(0) }; OWNER_LIMIT];

thread_local! {
    static OWNER: Cell<Option<usize>> = const { Cell::new(None) };
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AllocationMeasurement {
    pub calls: u64,
    pub requested_bytes: u64,
}

/// The profiling binary's allocator. Unscoped process allocations are ignored.
pub struct HotCoreAllocator;

// SAFETY: every method delegates the same pointer and layout contract directly
// to `System`; the only additional work mutates independent atomic counters.
unsafe impl GlobalAlloc for HotCoreAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        // SAFETY: the caller supplies the `GlobalAlloc::alloc` layout contract unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        // SAFETY: the caller supplies the `GlobalAlloc::alloc_zeroed` contract unchanged.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the pointer and layout are forwarded unchanged to their owning allocator.
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record(new_size);
        // SAFETY: the pointer, old layout, and requested size are forwarded unchanged.
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

#[must_use]
pub struct AllocationScope {
    previous: Option<usize>,
}

impl Drop for AllocationScope {
    fn drop(&mut self) {
        OWNER.set(self.previous);
    }
}

pub fn scope(owner: usize) -> AllocationScope {
    assert!(
        owner < OWNER_LIMIT,
        "profiling allocation owner exceeds table"
    );
    AllocationScope {
        previous: OWNER.replace(Some(owner)),
    }
}

pub fn record(requested_bytes: usize) {
    let Ok(owner) = OWNER.try_with(Cell::get) else {
        return;
    };
    if let Some(owner) = owner {
        CALLS[owner].fetch_add(1, Ordering::Relaxed);
        REQUESTED_BYTES[owner].fetch_add(requested_bytes as u64, Ordering::Relaxed);
    }
}

#[must_use]
pub fn measurement(owner: usize) -> AllocationMeasurement {
    assert!(
        owner < OWNER_LIMIT,
        "profiling allocation owner exceeds table"
    );
    AllocationMeasurement {
        calls: CALLS[owner].load(Ordering::Relaxed),
        requested_bytes: REQUESTED_BYTES[owner].load(Ordering::Relaxed),
    }
}
