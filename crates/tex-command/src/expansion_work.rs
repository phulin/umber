//! Parked ownership substrate for one structurally nested expansion.
//!
//! Ordinary synchronous expanded delivery deliberately does not enter these
//! lanes. A later cutover may move its one caller-owned command here only when
//! resource suspension or structural nesting requires a stable parked owner.

#![allow(dead_code)] // Foundation is intentionally unused until the reviewed cutover.

use core::marker::PhantomData;
use core::num::{NonZeroU32, NonZeroU64};
use core::sync::atomic::{AtomicU64, Ordering};

use crate::CurrentCommand;
use crate::execution_scratch::ScratchError;

mod control;
use control::*;

const COMMANDS_PER_CHUNK: usize = 32;
const CONTROLS_PER_CHUNK: usize = 32;
const NAME_BYTES_PER_CHUNK: usize = 1_024;

static NEXT_WORK_OWNER: AtomicU64 = AtomicU64::new(1);

fn next_work_owner() -> NonZeroU64 {
    let raw = NEXT_WORK_OWNER
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .expect("expansion-work owner serial capacity exhausted");
    NonZeroU64::new(raw).expect("expansion-work owner serial starts nonzero")
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ExpansionMark {
    controls: u32,
    commands: u32,
    name_bytes: u32,
}

#[derive(Debug)]
struct LaneId<G> {
    packed: NonZeroU64,
    _generation: PhantomData<fn(&G) -> &G>,
}

impl<G> LaneId<G> {
    fn new(index: u32, serial: NonZeroU32) -> Result<Self, ScratchError> {
        let stored_index = index.checked_add(1).ok_or(ScratchError::CapacityOverflow)?;
        let packed = (u64::from(serial.get()) << 32) | u64::from(stored_index);
        Ok(Self {
            packed: NonZeroU64::new(packed).ok_or(ScratchError::InvalidCoordinate)?,
            _generation: PhantomData,
        })
    }

    const fn index(self) -> u32 {
        (self.packed.get() as u32) - 1
    }

    const fn serial(self) -> u32 {
        (self.packed.get() >> 32) as u32
    }
}

impl<G> Copy for LaneId<G> {}
impl<G> Clone for LaneId<G> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<G> PartialEq for LaneId<G> {
    fn eq(&self, other: &Self) -> bool {
        self.packed == other.packed
    }
}
impl<G> Eq for LaneId<G> {}

#[derive(Debug)]
struct LaneSlot<T> {
    serial: u32,
    value: Option<T>,
}

impl<T> Default for LaneSlot<T> {
    fn default() -> Self {
        Self {
            serial: 0,
            value: None,
        }
    }
}

#[derive(Debug)]
struct FixedChunkLane<T, G, const N: usize> {
    chunks: Vec<Box<[LaneSlot<T>]>>,
    len: u32,
    next_serial: u32,
    _generation: PhantomData<fn(&G) -> &G>,
}

impl<T, G, const N: usize> Default for FixedChunkLane<T, G, N> {
    fn default() -> Self {
        Self {
            chunks: Vec::new(),
            len: 0,
            next_serial: 1,
            _generation: PhantomData,
        }
    }
}

impl<T, G, const N: usize> FixedChunkLane<T, G, N> {
    fn len(&self) -> u32 {
        self.len
    }

    fn push(&mut self, value: T) -> Result<LaneId<G>, ScratchError> {
        if N == 0 || self.len == u32::MAX || self.next_serial == u32::MAX {
            return Err(ScratchError::CapacityOverflow);
        }
        let index = self.len;
        let serial = NonZeroU32::new(self.next_serial).ok_or(ScratchError::CapacityOverflow)?;
        self.ensure_chunk(index as usize / N)?;
        let slot = self.slot_by_index_mut(index)?;
        if slot.value.is_some() {
            return Err(ScratchError::InvalidCoordinate);
        }
        slot.serial = serial.get();
        slot.value = Some(value);
        self.len += 1;
        self.next_serial += 1;
        LaneId::new(index, serial)
    }

    fn get(&self, id: LaneId<G>) -> Result<&T, ScratchError> {
        let slot = self.slot_by_index(id.index())?;
        if slot.serial != id.serial() {
            return Err(ScratchError::InvalidCoordinate);
        }
        slot.value.as_ref().ok_or(ScratchError::InvalidCoordinate)
    }

    fn get_mut(&mut self, id: LaneId<G>) -> Result<&mut T, ScratchError> {
        let slot = self.slot_by_index_mut(id.index())?;
        if slot.serial != id.serial() {
            return Err(ScratchError::InvalidCoordinate);
        }
        slot.value.as_mut().ok_or(ScratchError::InvalidCoordinate)
    }

    fn take_top(&mut self, id: LaneId<G>) -> Result<T, ScratchError> {
        if id.index().checked_add(1) != Some(self.len) {
            return Err(ScratchError::InvalidCoordinate);
        }
        let slot = self.slot_by_index_mut(id.index())?;
        if slot.serial != id.serial() {
            return Err(ScratchError::InvalidCoordinate);
        }
        let value = slot.value.take().ok_or(ScratchError::InvalidCoordinate)?;
        self.len -= 1;
        Ok(value)
    }

    fn truncate(&mut self, mark: u32) -> Result<(), ScratchError> {
        if mark > self.len {
            return Err(ScratchError::InvalidCoordinate);
        }
        while self.len > mark {
            let index = self.len - 1;
            let slot = self.slot_by_index_mut(index)?;
            slot.value.take().ok_or(ScratchError::InvalidCoordinate)?;
            self.len = index;
        }
        Ok(())
    }

    fn ensure_chunk(&mut self, index: usize) -> Result<(), ScratchError> {
        while self.chunks.len() <= index {
            self.chunks
                .try_reserve(1)
                .map_err(|_| ScratchError::AllocationFailed)?;
            let mut slots = Vec::new();
            slots
                .try_reserve_exact(N)
                .map_err(|_| ScratchError::AllocationFailed)?;
            slots.resize_with(N, LaneSlot::default);
            self.chunks.push(slots.into_boxed_slice());
        }
        Ok(())
    }

    fn slot_by_index(&self, index: u32) -> Result<&LaneSlot<T>, ScratchError> {
        let index = index as usize;
        self.chunks
            .get(index / N)
            .and_then(|chunk| chunk.get(index % N))
            .ok_or(ScratchError::InvalidCoordinate)
    }

    fn slot_by_index_mut(&mut self, index: u32) -> Result<&mut LaneSlot<T>, ScratchError> {
        let index = index as usize;
        self.chunks
            .get_mut(index / N)
            .and_then(|chunk| chunk.get_mut(index % N))
            .ok_or(ScratchError::InvalidCoordinate)
    }
}

#[derive(Debug)]
struct NameChunk {
    bytes: Box<[u8]>,
}

impl NameChunk {
    fn new() -> Result<Self, ScratchError> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(NAME_BYTES_PER_CHUNK)
            .map_err(|_| ScratchError::AllocationFailed)?;
        bytes.resize(NAME_BYTES_PER_CHUNK, 0);
        Ok(Self {
            bytes: bytes.into_boxed_slice(),
        })
    }
}

#[derive(Debug, Default)]
struct ExpansionNameLane {
    chunks: Vec<NameChunk>,
    len: u32,
}

impl ExpansionNameLane {
    fn mark(&self) -> ExpansionNameMark {
        ExpansionNameMark(self.len)
    }

    fn push(&mut self, byte: u8) -> Result<(), ScratchError> {
        if self.len == u32::MAX {
            return Err(ScratchError::CapacityOverflow);
        }
        let index = self.len as usize;
        let chunk = index / NAME_BYTES_PER_CHUNK;
        if self.chunks.len() <= chunk {
            self.chunks
                .try_reserve(1)
                .map_err(|_| ScratchError::AllocationFailed)?;
            self.chunks.push(NameChunk::new()?);
        }
        self.chunks[chunk].bytes[index % NAME_BYTES_PER_CHUNK] = byte;
        self.len += 1;
        Ok(())
    }

    fn truncate(&mut self, mark: u32) -> Result<(), ScratchError> {
        if mark > self.len {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.len = mark;
        Ok(())
    }

    fn bytes_from(&self, mark: ExpansionNameMark) -> Result<NameBytes<'_>, ScratchError> {
        if mark.0 > self.len {
            return Err(ScratchError::InvalidCoordinate);
        }
        Ok(NameBytes {
            lane: self,
            position: mark.0,
        })
    }

    fn get(&self, index: u32) -> Option<u8> {
        if index >= self.len {
            return None;
        }
        let index = index as usize;
        self.chunks
            .get(index / NAME_BYTES_PER_CHUNK)?
            .bytes
            .get(index % NAME_BYTES_PER_CHUNK)
            .copied()
    }
}

pub(crate) struct NameBytes<'a> {
    lane: &'a ExpansionNameLane,
    position: u32,
}

impl Iterator for NameBytes<'_> {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        let byte = self.lane.get(self.position)?;
        self.position += 1;
        Some(byte)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ExpansionNameMark(u32);

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ExpansionCommandSlot<G>(LaneId<G>);

impl<G> Copy for ExpansionCommandSlot<G> {}
impl<G> Clone for ExpansionCommandSlot<G> {
    fn clone(&self) -> Self {
        *self
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ExpansionControlSlot<G>(LaneId<G>);

impl<G> Copy for ExpansionControlSlot<G> {}
impl<G> Clone for ExpansionControlSlot<G> {
    fn clone(&self) -> Self {
        *self
    }
}

/// Move-only capability for one nested control with an exact result route.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ExpansionChild<G, D> {
    control: ExpansionControlSlot<G>,
    destination: D,
}

impl<G, D> ExpansionChild<G, D> {
    pub(crate) fn new(control: ExpansionControlSlot<G>, destination: D) -> Self {
        Self {
            control,
            destination,
        }
    }

    pub(crate) fn restore(self) -> (ExpansionControlSlot<G>, D) {
        (self.control, self.destination)
    }
}

/// Move-only external ownership wrapper used by a particular caller phase.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct OwnedExpansionWork<G, D> {
    key: ExpansionWorkKey<G>,
    destination: D,
}

impl<G, D> OwnedExpansionWork<G, D> {
    pub(crate) fn new(key: ExpansionWorkKey<G>, destination: D) -> Self {
        Self { key, destination }
    }

    pub(crate) fn restore(self) -> (ExpansionWorkKey<G>, D) {
        (self.key, self.destination)
    }
}

/// Exact move-only root for one live invocation.
#[must_use = "a live expansion must be completed, aborted, or moved to its exact owner"]
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ExpansionWorkKey<G> {
    owner: NonZeroU64,
    root: LaneId<G>,
    mark: ExpansionMark,
}

// These are shipping layout contracts, not test-only observations. Keeping
// controls in their stable lane prevents a parked expansion from inflating the
// heterogeneous continuation value.
const _: () = {
    assert!(core::mem::size_of::<ExpansionWorkKey<()>>() <= 32);
    assert!(core::mem::size_of::<ExpansionControl<()>>() <= 128);
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ExpansionWorkCounters {
    pub(crate) command_moves_in: u64,
    pub(crate) command_moves_out: u64,
    pub(crate) command_clones: u64,
    pub(crate) whole_control_copies: u64,
    pub(crate) control_pushes: u64,
    pub(crate) max_control_depth: u32,
    pub(crate) max_command_depth: u32,
    pub(crate) max_name_bytes: u32,
    pub(crate) completed_roots: u64,
    pub(crate) aborted_roots: u64,
    pub(crate) stale_key_rejections: u64,
}

#[derive(Debug)]
pub(crate) struct ExpansionWork<G> {
    owner: NonZeroU64,
    commands: FixedChunkLane<CurrentCommand<G>, G, COMMANDS_PER_CHUNK>,
    controls: FixedChunkLane<ExpansionControl<G>, G, CONTROLS_PER_CHUNK>,
    names: ExpansionNameLane,
    active_root: Option<LaneId<G>>,
    counters: ExpansionWorkCounters,
}

impl<G> PartialEq for ExpansionWork<G> {
    fn eq(&self, other: &Self) -> bool {
        if self.is_quiescent() && other.is_quiescent() {
            return true;
        }
        self.owner == other.owner
            && self.active_root == other.active_root
            && self.commands.len() == other.commands.len()
            && self.controls.len() == other.controls.len()
            && self.names.len == other.names.len
    }
}

impl<G> Eq for ExpansionWork<G> {}

impl<G> Default for ExpansionWork<G> {
    fn default() -> Self {
        Self {
            owner: next_work_owner(),
            commands: FixedChunkLane::default(),
            controls: FixedChunkLane::default(),
            names: ExpansionNameLane::default(),
            active_root: None,
            counters: ExpansionWorkCounters::default(),
        }
    }
}

impl<G> ExpansionWork<G> {
    pub(crate) fn begin_dispatch(
        &mut self,
        command: CurrentCommand<G>,
    ) -> Result<ExpansionWorkKey<G>, ScratchError> {
        if !self.is_quiescent() {
            return Err(ScratchError::InvalidCoordinate);
        }
        let mark = self.mark();
        let command = self.park_command(command)?;
        let root = match self.push_control(ExpansionControl::Dispatch {
            command,
            trace: TraceState::Unseen,
        }) {
            Ok(root) => root,
            Err(error) => {
                self.commands
                    .truncate(mark.commands)
                    .expect("begin rollback uses its own command mark");
                return Err(error);
            }
        };
        self.active_root = Some(root.0);
        Ok(ExpansionWorkKey {
            owner: self.owner,
            root: root.0,
            mark,
        })
    }

    pub(crate) fn park_command(
        &mut self,
        command: CurrentCommand<G>,
    ) -> Result<ExpansionCommandSlot<G>, ScratchError> {
        let id = self.commands.push(command)?;
        crate::command::record_expansion_command_move_in();
        self.counters.command_moves_in = self.counters.command_moves_in.saturating_add(1);
        self.counters.max_command_depth = self.counters.max_command_depth.max(self.commands.len());
        Ok(ExpansionCommandSlot(id))
    }

    pub(crate) fn take_command(
        &mut self,
        slot: ExpansionCommandSlot<G>,
    ) -> Result<CurrentCommand<G>, ScratchError> {
        let command = self.commands.take_top(slot.0)?;
        crate::command::record_expansion_command_move_out();
        self.counters.command_moves_out = self.counters.command_moves_out.saturating_add(1);
        Ok(command)
    }

    pub(crate) fn command(
        &self,
        slot: ExpansionCommandSlot<G>,
    ) -> Result<&CurrentCommand<G>, ScratchError> {
        self.commands.get(slot.0)
    }

    pub(crate) fn push_control(
        &mut self,
        control: ExpansionControl<G>,
    ) -> Result<ExpansionControlSlot<G>, ScratchError> {
        let id = self.controls.push(control)?;
        self.counters.control_pushes = self.counters.control_pushes.saturating_add(1);
        self.counters.max_control_depth = self.counters.max_control_depth.max(self.controls.len());
        Ok(ExpansionControlSlot(id))
    }

    pub(crate) fn control_mut(
        &mut self,
        slot: ExpansionControlSlot<G>,
    ) -> Result<&mut ExpansionControl<G>, ScratchError> {
        self.controls.get_mut(slot.0)
    }

    pub(crate) fn pop_control(
        &mut self,
        slot: ExpansionControlSlot<G>,
    ) -> Result<ExpansionControl<G>, ScratchError> {
        self.controls.take_top(slot.0)
    }

    pub(crate) fn name_mark(&self) -> ExpansionNameMark {
        self.names.mark()
    }

    pub(crate) fn push_name_byte(&mut self, byte: u8) -> Result<(), ScratchError> {
        self.names.push(byte)?;
        self.counters.max_name_bytes = self.counters.max_name_bytes.max(self.names.len);
        Ok(())
    }

    pub(crate) fn name_bytes(
        &self,
        mark: ExpansionNameMark,
    ) -> Result<NameBytes<'_>, ScratchError> {
        self.names.bytes_from(mark)
    }

    pub(crate) fn finish(&mut self, key: ExpansionWorkKey<G>) -> Result<(), ScratchError> {
        self.validate_key(&key)?;
        self.truncate_to(key.mark)?;
        self.active_root = None;
        self.counters.completed_roots = self.counters.completed_roots.saturating_add(1);
        Ok(())
    }

    pub(crate) fn abort(&mut self, key: ExpansionWorkKey<G>) -> Result<(), ScratchError> {
        self.validate_key(&key)?;
        // Controls are retired newest-first before their command and text
        // destinations, preserving the child-before-parent ownership order.
        self.truncate_to(key.mark)?;
        self.active_root = None;
        self.counters.aborted_roots = self.counters.aborted_roots.saturating_add(1);
        Ok(())
    }

    pub(crate) fn is_quiescent(&self) -> bool {
        self.active_root.is_none()
            && self.controls.len() == 0
            && self.commands.len() == 0
            && self.names.len == 0
    }

    pub(crate) const fn counters(&self) -> ExpansionWorkCounters {
        self.counters
    }

    fn mark(&self) -> ExpansionMark {
        ExpansionMark {
            controls: self.controls.len(),
            commands: self.commands.len(),
            name_bytes: self.names.len,
        }
    }

    fn validate_key(&mut self, key: &ExpansionWorkKey<G>) -> Result<(), ScratchError> {
        let valid = key.owner == self.owner
            && self.active_root == Some(key.root)
            && self.controls.get(key.root).is_ok()
            && key.mark.controls < self.controls.len()
            && key.mark.commands <= self.commands.len()
            && key.mark.name_bytes <= self.names.len;
        if !valid {
            self.counters.stale_key_rejections =
                self.counters.stale_key_rejections.saturating_add(1);
            return Err(ScratchError::InvalidCoordinate);
        }
        Ok(())
    }

    fn truncate_to(&mut self, mark: ExpansionMark) -> Result<(), ScratchError> {
        self.controls.truncate(mark.controls)?;
        self.commands.truncate(mark.commands)?;
        self.names.truncate(mark.name_bytes)
    }
}

#[cfg(test)]
#[path = "expansion_work/tests.rs"]
mod tests;
