//! Exact-size typed allocations with one checked dense initialized prefix.
//!
//! This crate deliberately contains no arena or TeX lifetime semantics. Its
//! entire job is to make one raw 64 KiB allocation look like a safe, fixed-
//! capacity append/truncate owner.

#![deny(unsafe_op_in_unsafe_fn)]

use core::fmt;
use core::marker::PhantomData;
use core::mem::{align_of, size_of};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU64, Ordering};
use std::alloc::{Layout, alloc, dealloc};

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
#[cfg(all(test, target_arch = "wasm32"))]
mod wasm_tests;

/// The exact requested allocation size of every [`Superblock`].
pub const SUPERBLOCK_BYTES: usize = 65_536;

static ALLOCATION_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static REQUESTED_BYTES: AtomicU64 = AtomicU64::new(0);
static SUPERBLOCKS_ALLOCATED: AtomicU64 = AtomicU64::new(0);
static SUPERBLOCKS_TRUNCATED: AtomicU64 = AtomicU64::new(0);
static SUPERBLOCKS_DROPPED: AtomicU64 = AtomicU64::new(0);
static SUPERBLOCKS_DEALLOCATED: AtomicU64 = AtomicU64::new(0);
static VALUES_CONSTRUCTED: AtomicU64 = AtomicU64::new(0);
static VALUES_DROPPED: AtomicU64 = AtomicU64::new(0);

/// Process-wide low-level event counters intended for isolated measurements.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SubstrateMetrics {
    pub allocation_attempts: u64,
    pub requested_bytes: u64,
    pub superblocks_allocated: u64,
    pub superblocks_truncated: u64,
    pub superblocks_dropped: u64,
    pub superblocks_deallocated: u64,
    pub values_constructed: u64,
    pub values_dropped: u64,
}

impl SubstrateMetrics {
    #[must_use]
    pub fn snapshot() -> Self {
        Self {
            allocation_attempts: ALLOCATION_ATTEMPTS.load(Ordering::Relaxed),
            requested_bytes: REQUESTED_BYTES.load(Ordering::Relaxed),
            superblocks_allocated: SUPERBLOCKS_ALLOCATED.load(Ordering::Relaxed),
            superblocks_truncated: SUPERBLOCKS_TRUNCATED.load(Ordering::Relaxed),
            superblocks_dropped: SUPERBLOCKS_DROPPED.load(Ordering::Relaxed),
            superblocks_deallocated: SUPERBLOCKS_DEALLOCATED.load(Ordering::Relaxed),
            values_constructed: VALUES_CONSTRUCTED.load(Ordering::Relaxed),
            values_dropped: VALUES_DROPPED.load(Ordering::Relaxed),
        }
    }
}

impl core::ops::Sub for SubstrateMetrics {
    type Output = Self;

    fn sub(self, earlier: Self) -> Self {
        Self {
            allocation_attempts: self.allocation_attempts - earlier.allocation_attempts,
            requested_bytes: self.requested_bytes - earlier.requested_bytes,
            superblocks_allocated: self.superblocks_allocated - earlier.superblocks_allocated,
            superblocks_truncated: self.superblocks_truncated - earlier.superblocks_truncated,
            superblocks_dropped: self.superblocks_dropped - earlier.superblocks_dropped,
            superblocks_deallocated: self.superblocks_deallocated - earlier.superblocks_deallocated,
            values_constructed: self.values_constructed - earlier.values_constructed,
            values_dropped: self.values_dropped - earlier.values_dropped,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutError {
    ZeroSizedType,
    TypeTooLarge { size: usize },
    InvalidLayout,
    AllocationFailed,
}

impl fmt::Display for LayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroSizedType => formatter.write_str("zero-sized superblock payload"),
            Self::TypeTooLarge { size } => {
                write!(formatter, "payload size {size} exceeds 64 KiB")
            }
            Self::InvalidLayout => formatter.write_str("invalid 64 KiB payload layout"),
            Self::AllocationFailed => formatter.write_str("64 KiB allocation failed"),
        }
    }
}

impl std::error::Error for LayoutError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapacityError;

impl fmt::Display for CapacityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("superblock is full")
    }
}

impl std::error::Error for CapacityError {}

/// One stable exactly-64-KiB allocation containing `T` in slots `0..len`.
pub struct Superblock<T> {
    allocation: NonNull<u8>,
    len: usize,
    _payload: PhantomData<T>,
}

impl<T> Superblock<T> {
    /// Returns the monomorphized number of payload slots in one block.
    #[must_use]
    pub const fn capacity() -> usize {
        let size = size_of::<T>();
        assert!(size != 0, "zero-sized superblock payload");
        assert!(size <= SUPERBLOCK_BYTES, "superblock payload is too large");
        SUPERBLOCK_BYTES / size
    }

    #[must_use]
    pub const fn payload_bytes() -> usize {
        Self::capacity() * size_of::<T>()
    }

    #[must_use]
    pub const fn tail_slack_bytes() -> usize {
        SUPERBLOCK_BYTES - Self::payload_bytes()
    }

    fn checked_layout() -> Result<Layout, LayoutError> {
        let size = size_of::<T>();
        if size == 0 {
            return Err(LayoutError::ZeroSizedType);
        }
        if size > SUPERBLOCK_BYTES {
            return Err(LayoutError::TypeTooLarge { size });
        }
        Layout::from_size_align(SUPERBLOCK_BYTES, align_of::<T>())
            .map_err(|_| LayoutError::InvalidLayout)
    }

    pub fn try_new() -> Result<Self, LayoutError> {
        let layout = Self::checked_layout()?;
        ALLOCATION_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
        REQUESTED_BYTES.fetch_add(SUPERBLOCK_BYTES as u64, Ordering::Relaxed);
        // SAFETY: `layout` is a non-zero valid layout. Null is handled below;
        // the resulting allocation is owned exclusively by the returned block.
        let pointer = unsafe { alloc(layout) };
        Self::finish_allocation(pointer)
    }

    fn finish_allocation(pointer: *mut u8) -> Result<Self, LayoutError> {
        let allocation = NonNull::new(pointer).ok_or(LayoutError::AllocationFailed)?;
        SUPERBLOCKS_ALLOCATED.fetch_add(1, Ordering::Relaxed);
        Ok(Self {
            allocation,
            len: 0,
            _payload: PhantomData,
        })
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn slot_pointer(&self, offset: usize) -> NonNull<T> {
        assert!(offset < Self::capacity());
        let byte_offset = offset
            .checked_mul(size_of::<T>())
            .expect("validated slot byte offset");
        assert!(byte_offset < Self::payload_bytes());
        // SAFETY: the checked byte offset lies within this allocation and is a
        // multiple of `size_of::<T>()`; the allocation start has `T` alignment.
        unsafe { NonNull::new_unchecked(self.allocation.as_ptr().add(byte_offset).cast()) }
    }

    #[must_use]
    pub fn get(&self, offset: usize) -> Option<&T> {
        if offset >= self.len {
            return None;
        }
        let pointer = self.slot_pointer(offset);
        // SAFETY: only slots below `len` are exposed, and every prefix increase
        // follows a completed `T` write. The shared borrow ties the reference to
        // this block and excludes mutation through this owner.
        Some(unsafe { pointer.as_ref() })
    }

    #[must_use]
    pub fn get_mut(&mut self, offset: usize) -> Option<&mut T> {
        if offset >= self.len {
            return None;
        }
        let mut pointer = self.slot_pointer(offset);
        // SAFETY: only an initialized slot is exposed, and `&mut self` excludes
        // every other reference obtainable through this owner.
        Some(unsafe { pointer.as_mut() })
    }

    /// Borrows the complete dense initialized prefix.
    #[must_use]
    pub fn initialized(&self) -> &[T] {
        // SAFETY: the allocation start has `T` alignment and exactly `len`
        // consecutive slots are initialized. The slice borrow is tied to the
        // shared block borrow.
        unsafe { core::slice::from_raw_parts(self.allocation.as_ptr().cast(), self.len) }
    }

    /// Exclusively borrows the complete dense initialized prefix.
    #[must_use]
    pub fn initialized_mut(&mut self) -> &mut [T] {
        // SAFETY: the allocation start has `T` alignment and exactly `len`
        // consecutive slots are initialized. The exclusive block borrow
        // prevents any other reference to those values for the slice lifetime.
        unsafe { core::slice::from_raw_parts_mut(self.allocation.as_ptr().cast(), self.len) }
    }

    /// Appends a `Copy` slice as one initialized range.
    pub fn extend_copy_from_slice(&mut self, values: &[T]) -> Result<(), CapacityError>
    where
        T: Copy,
    {
        let new_len = self.len.checked_add(values.len()).ok_or(CapacityError)?;
        if new_len > Self::capacity() {
            return Err(CapacityError);
        }
        if values.is_empty() {
            return Ok(());
        }
        let destination = self.slot_pointer(self.len);
        // SAFETY: `values.len()` slots fit after the initialized prefix. The
        // exclusive block borrow prevents `values` from aliasing this owner,
        // and `T: Copy` permits duplicating every source value bit-for-bit.
        unsafe {
            destination
                .as_ptr()
                .copy_from_nonoverlapping(values.as_ptr(), values.len());
        }
        self.len = new_len;
        VALUES_CONSTRUCTED.fetch_add(values.len() as u64, Ordering::Relaxed);
        Ok(())
    }

    pub fn push_with<F>(&mut self, build: F) -> Result<&mut T, CapacityError>
    where
        F: for<'slot> FnOnce(VacantSlot<'slot, T>) -> InitializedSlot<'slot, T>,
    {
        if self.len == Self::capacity() {
            return Err(CapacityError);
        }
        let mut pointer = self.slot_pointer(self.len);
        let slot = VacantSlot {
            pointer,
            _borrow: PhantomData,
        };
        let mut initialized = build(slot);
        assert_eq!(initialized.pointer, pointer, "foreign initialized slot");
        initialized.armed = false;
        self.len += 1;
        VALUES_CONSTRUCTED.fetch_add(1, Ordering::Relaxed);
        // SAFETY: the builder returned the guard created by writing this exact
        // vacant slot, and the initialized prefix was published immediately
        // above while the exclusive block borrow is still held.
        Ok(unsafe { pointer.as_mut() })
    }

    /// Drops the suffix after `new_len`, preserving a dense initialized prefix.
    ///
    /// A value whose destructor panics is never retried. The guard continues
    /// draining the remaining removed values while unwinding.
    pub fn truncate(&mut self, new_len: usize) {
        if new_len >= self.len {
            return;
        }
        let old_len = self.len;
        self.len = new_len;
        SUPERBLOCKS_TRUNCATED.fetch_add(1, Ordering::Relaxed);
        let mut drain = DrainGuard::<T> {
            allocation: self.allocation,
            next: old_len,
            stop: new_len,
            _payload: PhantomData,
        };
        drain.drain();
    }
}

/// Exclusive capability to initialize exactly the next unpublished slot.
pub struct VacantSlot<'a, T> {
    pointer: NonNull<T>,
    _borrow: PhantomData<&'a mut T>,
}

impl<'a, T> VacantSlot<'a, T> {
    #[must_use]
    pub fn insert(self, value: T) -> InitializedSlot<'a, T> {
        // SAFETY: `VacantSlot` is created only for the next slot outside the
        // initialized prefix and is consumed here, so this write initializes a
        // unique correctly aligned location exactly once.
        unsafe { self.pointer.as_ptr().write(value) };
        InitializedSlot {
            pointer: self.pointer,
            armed: true,
            _borrow: PhantomData,
        }
    }
}

/// Unpublished initialized value returned to [`Superblock::push_with`].
pub struct InitializedSlot<'a, T> {
    pointer: NonNull<T>,
    armed: bool,
    _borrow: PhantomData<&'a mut T>,
}

impl<T> Drop for InitializedSlot<'_, T> {
    fn drop(&mut self) {
        if self.armed {
            VALUES_DROPPED.fetch_add(1, Ordering::Relaxed);
            // SAFETY: an armed guard uniquely owns the valid `T` written by
            // `VacantSlot::insert`; it has not been published into the prefix.
            unsafe { self.pointer.as_ptr().drop_in_place() };
        }
    }
}

struct DrainGuard<T> {
    allocation: NonNull<u8>,
    next: usize,
    stop: usize,
    _payload: PhantomData<T>,
}

impl<T> DrainGuard<T> {
    fn drain(&mut self) {
        while self.next > self.stop {
            self.next -= 1;
            let byte_offset = self.next * size_of::<T>();
            VALUES_DROPPED.fetch_add(1, Ordering::Relaxed);
            // SAFETY: `next` walks each formerly initialized suffix slot once,
            // back to `stop`. The owning block shortened its public prefix
            // before this guard was created, so no safe reference can name it.
            unsafe {
                self.allocation
                    .as_ptr()
                    .add(byte_offset)
                    .cast::<T>()
                    .drop_in_place();
            }
        }
    }
}

impl<T> Drop for DrainGuard<T> {
    fn drop(&mut self) {
        self.drain();
    }
}

struct DeallocationGuard {
    allocation: NonNull<u8>,
    layout: Layout,
}

impl Drop for DeallocationGuard {
    fn drop(&mut self) {
        // SAFETY: the guard is created exactly once by the allocation owner,
        // after layout revalidation, and runs exactly once even during unwind.
        unsafe { dealloc(self.allocation.as_ptr(), self.layout) };
        SUPERBLOCKS_DEALLOCATED.fetch_add(1, Ordering::Relaxed);
    }
}

impl<T> Drop for Superblock<T> {
    fn drop(&mut self) {
        SUPERBLOCKS_DROPPED.fetch_add(1, Ordering::Relaxed);
        let layout = Self::checked_layout().expect("validated superblock layout remains valid");
        let deallocation = DeallocationGuard {
            allocation: self.allocation,
            layout,
        };
        self.truncate(0);
        drop(deallocation);
    }
}
