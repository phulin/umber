//! Typed append-only regions over reusable, generation-checked chunks.
//!
//! A coordinate is a non-owning `(namespace, slot, generation, offset)` value.
//! Ownership is held once per accepted arena layer or mutable candidate, never
//! once per stored value. A candidate shares its accepted base and owns a
//! disjoint mutable namespace. Accepted chunks never move or change.

use core::marker::PhantomData;
use core::mem::size_of;
use core::num::{NonZeroU32, NonZeroU64};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

const FIRST_ARENA_NAMESPACE: u64 = 256;
const FIRST_GENERATION: NonZeroU32 = NonZeroU32::MIN;
static NEXT_ARENA_NAMESPACE: AtomicU64 = AtomicU64::new(FIRST_ARENA_NAMESPACE);

mod layout;
mod value_region;

pub(crate) use layout::*;
pub(crate) use value_region::*;

struct ChunkSlot<T> {
    generation: NonZeroU32,
    live: bool,
    appendable: bool,
    values: Vec<T>,
}

impl<T> ChunkSlot<T> {
    fn key(&self, namespace: NonZeroU64, slot: u32) -> ChunkOwner {
        ChunkOwner {
            namespace,
            slot,
            generation: self.generation,
        }
    }
}

struct AcceptedLayer<T> {
    namespace: NonZeroU64,
    slots: Box<[ChunkSlot<T>]>,
}

/// An explicit region-root set shared at region granularity, never per value.
///
/// Each entry owns exactly one immutable accepted layer. Layers do not retain
/// their predecessors, so constructing a narrower canonical root set can
/// release unrelated regions without walking or dismantling an ancestry chain.
pub(crate) struct AcceptedRegionArena<T> {
    regions: Vec<Arc<AcceptedLayer<T>>>,
    initial_chunk_capacity: NonZeroU32,
    accounting: RegionArenaAccounting,
}

impl<T> Clone for AcceptedRegionArena<T> {
    fn clone(&self) -> Self {
        Self {
            regions: self.regions.iter().map(Arc::clone).collect(),
            initial_chunk_capacity: self.initial_chunk_capacity,
            accounting: self.accounting,
        }
    }
}

impl<T> AcceptedRegionArena<T> {
    pub(crate) const fn new(initial_chunk_capacity: NonZeroU32) -> Self {
        Self {
            regions: Vec::new(),
            initial_chunk_capacity,
            accounting: RegionArenaAccounting {
                logical_values: 0,
                logical_value_bytes: 0,
                live_chunks: 0,
                reusable_chunks: 0,
                retained_payload_values: 0,
                retained_payload_bytes: 0,
                registry_slots: 0,
                retained_registry_bytes: 0,
            },
        }
    }

    pub(crate) fn candidate(&self) -> Result<RegionArena<T>, RegionArenaError> {
        RegionArena::new(self.clone())
    }

    pub(crate) fn resolve(&self, coordinate: RegionCoordinate<T>) -> Result<&T, RegionArenaError> {
        let values = self.resolve_chunk(coordinate.key)?;
        values
            .get(coordinate.offset as usize)
            .ok_or(RegionArenaError::OffsetOutOfBounds)
    }

    pub(crate) fn resolve_span(&self, span: RegionSpan<T>) -> Result<&[T], RegionArenaError> {
        resolve_span(self.resolve_chunk(span.key)?, span)
    }

    pub(crate) fn accounting(&self) -> RegionArenaAccounting {
        self.accounting
    }

    fn resolve_chunk(&self, key: ChunkOwner) -> Result<&[T], RegionArenaError> {
        for layer in self.regions.iter().rev() {
            if layer.namespace == key.namespace {
                let slot = layer
                    .slots
                    .get(key.slot as usize)
                    .ok_or(RegionArenaError::UnknownChunk)?;
                if !slot.live {
                    return Err(RegionArenaError::UnknownChunk);
                }
                if slot.generation != key.generation {
                    return Err(RegionArenaError::StaleGeneration);
                }
                return Ok(&slot.values);
            }
        }
        Err(RegionArenaError::ForeignNamespace)
    }

    #[cfg(test)]
    fn shares_newest_layer_with(&self, other: &Self) -> bool {
        match (self.regions.last(), other.regions.last()) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            (None, None) => true,
            _ => false,
        }
    }
}

struct ActiveReservation {
    key: ChunkOwner,
    start: u32,
    limit: u32,
}

/// Candidate-local mutable suffix over a shared accepted base.
pub(crate) struct RegionArena<T> {
    base: AcceptedRegionArena<T>,
    namespace: NonZeroU64,
    slots: Vec<ChunkSlot<T>>,
    live_slots: Vec<u32>,
    active: Option<ActiveReservation>,
    next_chunk_capacity: u32,
    storage_growth_events: usize,
}

impl<T> RegionArena<T> {
    fn new(base: AcceptedRegionArena<T>) -> Result<Self, RegionArenaError> {
        let namespace = fresh_namespace()?;
        Ok(Self {
            next_chunk_capacity: base.initial_chunk_capacity.get(),
            base,
            namespace,
            slots: Vec::new(),
            live_slots: Vec::new(),
            active: None,
            storage_growth_events: 0,
        })
    }

    pub(crate) fn reserve(
        &mut self,
        values: NonZeroU32,
    ) -> Result<RegionReservation<T>, RegionArenaError> {
        if self.active.is_some() {
            return Err(RegionArenaError::ReservationActive);
        }
        let values = values.get();
        let slot = if let Some(slot) = self.appendable_tail(values)? {
            slot
        } else {
            self.activate_chunk(values)?
        };
        let chunk = &self.slots[slot as usize];
        let start = u32::try_from(chunk.values.len())
            .map_err(|_| RegionArenaError::OffsetCapacityExhausted)?;
        let limit = start
            .checked_add(values)
            .ok_or(RegionArenaError::OffsetCapacityExhausted)?;
        let key = chunk.key(self.namespace, slot);
        self.active = Some(ActiveReservation { key, start, limit });
        Ok(RegionReservation {
            key,
            start,
            limit,
            marker: PhantomData,
        })
    }

    pub(crate) fn append(
        &mut self,
        reservation: RegionReservation<T>,
        value: T,
    ) -> Result<RegionCoordinate<T>, RegionArenaError> {
        self.validate_reservation(reservation)?;
        let slot = &mut self.slots[reservation.key.slot as usize];
        let offset = u32::try_from(slot.values.len())
            .map_err(|_| RegionArenaError::OffsetCapacityExhausted)?;
        if offset >= reservation.limit {
            return Err(RegionArenaError::InvalidReservation);
        }
        slot.values.push(value);
        Ok(RegionCoordinate {
            key: reservation.key,
            offset,
            marker: PhantomData,
        })
    }

    pub(crate) fn freeze(
        &mut self,
        reservation: RegionReservation<T>,
    ) -> Result<RegionSpan<T>, RegionArenaError> {
        self.validate_reservation(reservation)?;
        let end = u32::try_from(self.slots[reservation.key.slot as usize].values.len())
            .map_err(|_| RegionArenaError::OffsetCapacityExhausted)?;
        self.active = None;
        Ok(RegionSpan {
            key: reservation.key,
            start: reservation.start,
            len: end - reservation.start,
            marker: PhantomData,
        })
    }

    pub(crate) fn mark(&self) -> Result<RegionArenaMark, RegionArenaError> {
        if self.active.is_some() {
            return Err(RegionArenaError::ReservationActive);
        }
        let live_chunks = u32::try_from(self.live_slots.len())
            .map_err(|_| RegionArenaError::SlotCapacityExhausted)?;
        let Some(&tail_slot) = self.live_slots.last() else {
            return Ok(RegionArenaMark {
                namespace: self.namespace,
                live_chunks,
                tail_slot: 0,
                tail_generation: 0,
                tail_len: 0,
            });
        };
        let tail = &self.slots[tail_slot as usize];
        Ok(RegionArenaMark {
            namespace: self.namespace,
            live_chunks,
            tail_slot,
            tail_generation: tail.generation.get(),
            tail_len: u32::try_from(tail.values.len())
                .map_err(|_| RegionArenaError::OffsetCapacityExhausted)?,
        })
    }

    pub(crate) fn truncate(&mut self, mark: RegionArenaMark) -> Result<(), RegionArenaError> {
        self.validate_mark(mark)?;
        self.active = None;
        while self.live_slots.len() > mark.live_chunks as usize {
            let slot = self.live_slots.pop().expect("length was checked");
            let chunk = &mut self.slots[slot as usize];
            chunk.values.clear();
            chunk.live = false;
            chunk.appendable = false;
        }
        if mark.live_chunks != 0 {
            let tail = &mut self.slots[mark.tail_slot as usize];
            if tail.values.len() != mark.tail_len as usize {
                tail.values.truncate(mark.tail_len as usize);
                // Reusing this suffix would let an old offset become live
                // again without changing its chunk generation.
                tail.appendable = false;
            }
        }
        Ok(())
    }

    /// Validates a rollback watermark without changing arena state.
    ///
    /// Aggregate snapshots use this preflight before mutating any component.
    pub(crate) fn validate_mark(&self, mark: RegionArenaMark) -> Result<(), RegionArenaError> {
        self.validate_mark_inner(mark)
    }

    /// Validates that this overlay can be sealed into its accepted base.
    pub(crate) fn validate_accept(&self) -> Result<(), RegionArenaError> {
        if self.active.is_some() {
            return Err(RegionArenaError::ReservationActive);
        }
        Ok(())
    }

    pub(crate) fn resolve(&self, coordinate: RegionCoordinate<T>) -> Result<&T, RegionArenaError> {
        let values = self.resolve_chunk(coordinate.key)?;
        values
            .get(coordinate.offset as usize)
            .ok_or(RegionArenaError::OffsetOutOfBounds)
    }

    pub(crate) fn resolve_span(&self, span: RegionSpan<T>) -> Result<&[T], RegionArenaError> {
        resolve_span(self.resolve_chunk(span.key)?, span)
    }

    /// Admits one validated region for repeated direct indexing.
    pub(crate) fn admit_span(
        &self,
        span: RegionSpan<T>,
    ) -> Result<AdmittedRegion<'_, T>, RegionArenaError> {
        Ok(AdmittedRegion {
            values: self.resolve_span(span)?,
        })
    }

    pub(crate) fn accounting(&self) -> RegionArenaAccounting {
        let logical_values = self
            .live_slots
            .iter()
            .map(|&slot| self.slots[slot as usize].values.len())
            .sum::<usize>();
        let retained_payload_values = self
            .slots
            .iter()
            .map(|slot| slot.values.capacity())
            .sum::<usize>();
        let overlay = RegionArenaAccounting {
            logical_values,
            logical_value_bytes: logical_values.saturating_mul(size_of::<T>()),
            live_chunks: self.live_slots.len(),
            reusable_chunks: self.slots.len().saturating_sub(self.live_slots.len()),
            retained_payload_values,
            retained_payload_bytes: retained_payload_values.saturating_mul(size_of::<T>()),
            registry_slots: self.slots.len(),
            retained_registry_bytes: self
                .slots
                .capacity()
                .saturating_mul(size_of::<ChunkSlot<T>>())
                .saturating_add(self.live_slots.capacity().saturating_mul(size_of::<u32>())),
        };
        self.base.accounting().plus(overlay)
    }

    pub(crate) fn accept(mut self) -> Result<AcceptedRegionArena<T>, RegionArenaError> {
        if self.active.is_some() {
            return Err(RegionArenaError::ReservationActive);
        }
        if self.live_slots.is_empty() {
            return Ok(self.base);
        }
        while self.slots.last().is_some_and(|slot| !slot.live) {
            self.slots.pop();
        }
        for slot in &mut self.slots {
            if !slot.live {
                slot.values = Vec::new();
            }
            slot.appendable = false;
        }
        let logical_values = self
            .slots
            .iter()
            .filter(|slot| slot.live)
            .map(|slot| slot.values.len())
            .sum::<usize>();
        let retained_payload_values = self
            .slots
            .iter()
            .filter(|slot| slot.live)
            .map(|slot| slot.values.capacity())
            .sum::<usize>();
        let delta = RegionArenaAccounting {
            logical_values,
            logical_value_bytes: logical_values.saturating_mul(size_of::<T>()),
            live_chunks: self.slots.iter().filter(|slot| slot.live).count(),
            reusable_chunks: 0,
            retained_payload_values,
            retained_payload_bytes: retained_payload_values.saturating_mul(size_of::<T>()),
            registry_slots: self.slots.len(),
            retained_registry_bytes: self.slots.len().saturating_mul(size_of::<ChunkSlot<T>>()),
        };
        let accounting = self.base.accounting.plus(delta);
        let layer = AcceptedLayer {
            namespace: self.namespace,
            slots: self.slots.into_boxed_slice(),
        };
        self.base.regions.push(Arc::new(layer));
        self.base.accounting = accounting;
        Ok(self.base)
    }

    fn appendable_tail(&self, values: u32) -> Result<Option<u32>, RegionArenaError> {
        let Some(&slot) = self.live_slots.last() else {
            return Ok(None);
        };
        let chunk = &self.slots[slot as usize];
        if !chunk.appendable {
            return Ok(None);
        }
        let end = chunk
            .values
            .len()
            .checked_add(values as usize)
            .ok_or(RegionArenaError::OffsetCapacityExhausted)?;
        Ok((end <= chunk.values.capacity()).then_some(slot))
    }

    fn activate_chunk(&mut self, values: u32) -> Result<u32, RegionArenaError> {
        let reusable = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| !slot.live)
            .max_by_key(|(_, slot)| slot.values.capacity())
            .map(|(slot, _)| slot);
        let slot = if let Some(slot) = reusable {
            let old_live_capacity = self.live_slots.capacity();
            self.live_slots
                .try_reserve(1)
                .map_err(|_| RegionArenaError::AllocationFailed)?;
            if self.live_slots.capacity() != old_live_capacity {
                self.storage_growth_events += 1;
            }
            let chunk = &mut self.slots[slot];
            let generation = chunk
                .generation
                .get()
                .checked_add(1)
                .and_then(NonZeroU32::new)
                .ok_or(RegionArenaError::GenerationExhausted)?;
            if chunk.values.capacity() < values as usize {
                let old_capacity = chunk.values.capacity();
                chunk
                    .values
                    .try_reserve_exact(values as usize - old_capacity)
                    .map_err(|_| RegionArenaError::AllocationFailed)?;
                if chunk.values.capacity() != old_capacity {
                    self.storage_growth_events += 1;
                }
            }
            chunk.generation = generation;
            chunk.live = true;
            chunk.appendable = true;
            u32::try_from(slot).map_err(|_| RegionArenaError::SlotCapacityExhausted)?
        } else {
            let needed = values.max(self.next_chunk_capacity);
            let slot = u32::try_from(self.slots.len())
                .map_err(|_| RegionArenaError::SlotCapacityExhausted)?;
            let mut chunk_values = Vec::new();
            chunk_values
                .try_reserve_exact(needed as usize)
                .map_err(|_| RegionArenaError::AllocationFailed)?;
            let old_slot_capacity = self.slots.capacity();
            self.slots
                .try_reserve(1)
                .map_err(|_| RegionArenaError::AllocationFailed)?;
            if self.slots.capacity() != old_slot_capacity {
                self.storage_growth_events += 1;
            }
            let old_live_capacity = self.live_slots.capacity();
            self.live_slots
                .try_reserve(1)
                .map_err(|_| RegionArenaError::AllocationFailed)?;
            if self.live_slots.capacity() != old_live_capacity {
                self.storage_growth_events += 1;
            }
            self.slots.push(ChunkSlot {
                generation: FIRST_GENERATION,
                live: true,
                appendable: true,
                values: chunk_values,
            });
            self.storage_growth_events += 1;
            self.next_chunk_capacity = needed.saturating_mul(2).max(needed);
            slot
        };
        self.live_slots.push(slot);
        Ok(slot)
    }

    fn validate_reservation(
        &self,
        reservation: RegionReservation<T>,
    ) -> Result<(), RegionArenaError> {
        let Some(active) = &self.active else {
            return Err(RegionArenaError::InvalidReservation);
        };
        if active.key != reservation.key
            || active.start != reservation.start
            || active.limit != reservation.limit
        {
            return Err(RegionArenaError::InvalidReservation);
        }
        Ok(())
    }

    fn validate_mark_inner(&self, mark: RegionArenaMark) -> Result<(), RegionArenaError> {
        if mark.namespace != self.namespace || mark.live_chunks as usize > self.live_slots.len() {
            return Err(RegionArenaError::InvalidMark);
        }
        if mark.live_chunks == 0 {
            return (mark.tail_generation == 0 && mark.tail_len == 0)
                .then_some(())
                .ok_or(RegionArenaError::InvalidMark);
        }
        let tail_slot = self.live_slots[mark.live_chunks as usize - 1];
        let tail = &self.slots[tail_slot as usize];
        if tail_slot != mark.tail_slot
            || tail.generation.get() != mark.tail_generation
            || tail.values.len() < mark.tail_len as usize
        {
            return Err(RegionArenaError::InvalidMark);
        }
        Ok(())
    }

    fn resolve_chunk(&self, key: ChunkOwner) -> Result<&[T], RegionArenaError> {
        if key.namespace != self.namespace {
            return self.base.resolve_chunk(key);
        }
        let slot = self
            .slots
            .get(key.slot as usize)
            .ok_or(RegionArenaError::UnknownChunk)?;
        if !slot.live {
            return Err(RegionArenaError::UnknownChunk);
        }
        if slot.generation != key.generation {
            return Err(RegionArenaError::StaleGeneration);
        }
        Ok(&slot.values)
    }

    #[cfg(test)]
    pub(crate) fn testing_storage_growth_events(&self) -> usize {
        self.storage_growth_events
    }
}

/// A borrow admitted once through namespace, slot, generation, and bounds.
pub(crate) struct AdmittedRegion<'a, T> {
    values: &'a [T],
}

impl<'a, T> AdmittedRegion<'a, T> {
    pub(crate) const fn values(&self) -> &'a [T] {
        self.values
    }
}

fn resolve_span<T>(values: &[T], span: RegionSpan<T>) -> Result<&[T], RegionArenaError> {
    let start = span.start as usize;
    let end = start
        .checked_add(span.len as usize)
        .ok_or(RegionArenaError::OffsetOutOfBounds)?;
    values
        .get(start..end)
        .ok_or(RegionArenaError::OffsetOutOfBounds)
}

fn fresh_namespace() -> Result<NonZeroU64, RegionArenaError> {
    let raw = NEXT_ARENA_NAMESPACE
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .map_err(|_| RegionArenaError::NamespaceExhausted)?;
    NonZeroU64::new(raw).ok_or(RegionArenaError::NamespaceExhausted)
}

#[cfg(test)]
mod tests;
