//! Fixed-width typed coordinates, marks, reservations, and accounting.

use core::fmt;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::num::{NonZeroU32, NonZeroU64};

/// A rejected arena operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RegionArenaError {
    NamespaceExhausted,
    SlotCapacityExhausted,
    GenerationExhausted,
    OffsetCapacityExhausted,
    AllocationFailed,
    ReservationActive,
    InvalidReservation,
    InvalidMark,
    ForeignNamespace,
    UnknownChunk,
    StaleGeneration,
    OffsetOutOfBounds,
}

impl fmt::Display for RegionArenaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NamespaceExhausted => "arena namespace space is exhausted",
            Self::SlotCapacityExhausted => "arena chunk-slot space is exhausted",
            Self::GenerationExhausted => "arena chunk generation is exhausted",
            Self::OffsetCapacityExhausted => "arena chunk offset space is exhausted",
            Self::AllocationFailed => "arena storage allocation failed",
            Self::ReservationActive => "another arena reservation is active",
            Self::InvalidReservation => "arena reservation is not active",
            Self::InvalidMark => "arena mark is not an ancestor of this candidate",
            Self::ForeignNamespace => "coordinate belongs to a foreign arena namespace",
            Self::UnknownChunk => "coordinate names no live arena chunk",
            Self::StaleGeneration => "coordinate names a stale arena chunk generation",
            Self::OffsetOutOfBounds => "coordinate is outside its arena chunk",
        })
    }
}

impl std::error::Error for RegionArenaError {}

/// Copy-only identity of the arena chunk that owns a coordinate or span.
///
/// The accepted layer or mutable candidate retains the allocation once. This
/// value is deliberately not an `Arc`, `Weak`, serialized handle, or liveness
/// root; consumers validate it at arena admission.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ChunkOwner {
    pub(in crate::hot_core) namespace: NonZeroU64,
    pub(in crate::hot_core) slot: u32,
    pub(in crate::hot_core) generation: NonZeroU32,
}

impl ChunkOwner {
    pub(in crate::hot_core) fn runtime_input(identity: u64) -> Self {
        Self {
            namespace: NonZeroU64::new(identity.checked_add(1).expect("input identity exhausted"))
                .expect("input identity offset is nonzero"),
            slot: 0,
            generation: NonZeroU32::MIN,
        }
    }

    pub(in crate::hot_core) const fn runtime_input_identity(self) -> u64 {
        self.namespace.get() - 1
    }

    pub(in crate::hot_core) const fn coordinate<T>(self, offset: u32) -> RegionCoordinate<T> {
        RegionCoordinate {
            key: self,
            offset,
            marker: PhantomData,
        }
    }

    pub(in crate::hot_core) const fn span<T>(self, start: u32, len: u32) -> RegionSpan<T> {
        RegionSpan {
            key: self,
            start,
            len,
            marker: PhantomData,
        }
    }
}

/// A compact typed coordinate naming one value.
pub(crate) struct RegionCoordinate<T> {
    pub(super) key: ChunkOwner,
    pub(super) offset: u32,
    pub(super) marker: PhantomData<fn() -> T>,
}

impl<T> RegionCoordinate<T> {
    pub(crate) const fn owner(self) -> ChunkOwner {
        self.key
    }

    pub(crate) const fn offset(self) -> u32 {
        self.offset
    }
}

impl<T> Copy for RegionCoordinate<T> {}

impl<T> Clone for RegionCoordinate<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> fmt::Debug for RegionCoordinate<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegionCoordinate")
            .field("namespace", &self.key.namespace)
            .field("slot", &self.key.slot)
            .field("generation", &self.key.generation)
            .field("offset", &self.offset)
            .finish()
    }
}

impl<T> PartialEq for RegionCoordinate<T> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.offset == other.offset
    }
}

impl<T> Eq for RegionCoordinate<T> {}

impl<T> Hash for RegionCoordinate<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key.hash(state);
        self.offset.hash(state);
    }
}

/// A compact typed half-open span in one chunk.
pub(crate) struct RegionSpan<T> {
    pub(super) key: ChunkOwner,
    pub(super) start: u32,
    pub(super) len: u32,
    pub(super) marker: PhantomData<fn() -> T>,
}

impl<T> RegionSpan<T> {
    pub(crate) const fn owner(self) -> ChunkOwner {
        self.key
    }

    pub(crate) const fn start(self) -> u32 {
        self.start
    }

    pub(crate) const fn len(self) -> u32 {
        self.len
    }

    pub(crate) const fn is_empty(self) -> bool {
        self.len == 0
    }
}

impl<T> Copy for RegionSpan<T> {}

impl<T> Clone for RegionSpan<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> fmt::Debug for RegionSpan<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegionSpan")
            .field("namespace", &self.key.namespace)
            .field("slot", &self.key.slot)
            .field("generation", &self.key.generation)
            .field("start", &self.start)
            .field("len", &self.len)
            .finish()
    }
}

impl<T> PartialEq for RegionSpan<T> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.start == other.start && self.len == other.len
    }
}

impl<T> Eq for RegionSpan<T> {}

/// An O(1) rollback watermark for one mutable candidate overlay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RegionArenaMark {
    pub(super) namespace: NonZeroU64,
    pub(super) live_chunks: u32,
    pub(super) tail_slot: u32,
    pub(super) tail_generation: u32,
    pub(super) tail_len: u32,
}

/// An exclusive append reservation. It owns no payload.
pub(crate) struct RegionReservation<T> {
    pub(super) key: ChunkOwner,
    pub(super) start: u32,
    pub(super) limit: u32,
    pub(super) marker: PhantomData<fn() -> T>,
}

impl<T> Copy for RegionReservation<T> {}

impl<T> Clone for RegionReservation<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> fmt::Debug for RegionReservation<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegionReservation")
            .field("key", &self.key)
            .field("start", &self.start)
            .field("limit", &self.limit)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RegionArenaAccounting {
    pub(crate) logical_values: usize,
    pub(crate) logical_value_bytes: usize,
    pub(crate) live_chunks: usize,
    pub(crate) reusable_chunks: usize,
    pub(crate) retained_payload_values: usize,
    pub(crate) retained_payload_bytes: usize,
    pub(crate) registry_slots: usize,
    pub(crate) retained_registry_bytes: usize,
}

impl RegionArenaAccounting {
    pub(crate) fn plus(self, other: Self) -> Self {
        Self {
            logical_values: self.logical_values.saturating_add(other.logical_values),
            logical_value_bytes: self
                .logical_value_bytes
                .saturating_add(other.logical_value_bytes),
            live_chunks: self.live_chunks.saturating_add(other.live_chunks),
            reusable_chunks: self.reusable_chunks.saturating_add(other.reusable_chunks),
            retained_payload_values: self
                .retained_payload_values
                .saturating_add(other.retained_payload_values),
            retained_payload_bytes: self
                .retained_payload_bytes
                .saturating_add(other.retained_payload_bytes),
            registry_slots: self.registry_slots.saturating_add(other.registry_slots),
            retained_registry_bytes: self
                .retained_registry_bytes
                .saturating_add(other.retained_registry_bytes),
        }
    }
}
