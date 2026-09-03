//! Parked ownership substrate for one structurally nested expansion.
//!
//! Ordinary synchronous expanded delivery deliberately does not enter these
//! lanes. Production moves its one caller-owned command here only after an
//! immutable-resource suspension; later cutovers may use the already reviewed
//! structural controls without changing that direct path.

#![allow(dead_code)] // Structural controls remain staged after the suspension cutover.

use core::marker::PhantomData;
use core::num::{NonZeroU32, NonZeroU64};
use core::sync::atomic::{AtomicU64, Ordering};

use crate::CurrentCommand;
use crate::execution_scratch::ScratchError;

pub(crate) mod control;
use control::*;

const COMMANDS_PER_CHUNK: usize = 32;
const CONTROLS_PER_CHUNK: usize = 32;
const NAME_BYTES_PER_CHUNK: usize = 1_024;

/// Bounded state for the one synchronous expanded-delivery interpreter.
///
/// The interpreter's hot token/command pair remains in the processor loop;
/// this sidecar owns only its typed continuation depth.  A continuation is
/// therefore represented by a fixed-width control-lane record instead of a
/// Rust call frame.  The checked increment is the static recursion guard: a
/// malformed input can exhaust the generation-scoped coordinate space, but it
/// cannot recurse through the host stack.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ExpandedDeliveryDriver {
    continuation_depth: u32,
}

const _: () = assert!(core::mem::size_of::<ExpandedDeliveryDriver>() <= 16);

impl ExpandedDeliveryDriver {
    fn push_continuation(&mut self) -> Result<(), ScratchError> {
        self.continuation_depth = self
            .continuation_depth
            .checked_add(1)
            .ok_or(ScratchError::CapacityOverflow)?;
        Ok(())
    }

    fn pop_continuation(&mut self) -> Result<(), ScratchError> {
        self.continuation_depth = self
            .continuation_depth
            .checked_sub(1)
            .ok_or(ScratchError::InvalidCoordinate)?;
        Ok(())
    }

    #[cfg(test)]
    fn continuation_depth(&self) -> u32 {
        self.continuation_depth
    }
}

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
        let mut value = Some(value);
        self.push_from(&mut value)
    }

    /// Preflights every fallible coordinate and allocation before moving the
    /// value into the lane. A failed production park therefore leaves the
    /// caller's sole owner intact.
    fn push_from(&mut self, value: &mut Option<T>) -> Result<LaneId<G>, ScratchError> {
        if value.is_none() {
            return Err(ScratchError::InvalidCoordinate);
        }
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
        slot.value = value.take();
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

    fn top_id(&self) -> Result<LaneId<G>, ScratchError> {
        let index = self
            .len
            .checked_sub(1)
            .ok_or(ScratchError::InvalidCoordinate)?;
        let slot = self.slot_by_index(index)?;
        let serial = NonZeroU32::new(slot.serial).ok_or(ScratchError::InvalidCoordinate)?;
        LaneId::new(index, serial)
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

    fn bytes_from(&self, mark: u32) -> Result<NameBytes<'_>, ScratchError> {
        if mark > self.len {
            return Err(ScratchError::InvalidCoordinate);
        }
        Ok(NameBytes {
            lane: self,
            position: mark,
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
pub(crate) struct ExpansionNameMark {
    owner: NonZeroU64,
    root_serial: u32,
    offset: u32,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ExpansionCommandSlot<G> {
    owner: NonZeroU64,
    lane: LaneId<G>,
}

impl<G> Copy for ExpansionCommandSlot<G> {}
impl<G> Clone for ExpansionCommandSlot<G> {
    fn clone(&self) -> Self {
        *self
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ExpansionControlSlot<G> {
    owner: NonZeroU64,
    lane: LaneId<G>,
}

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
pub struct ExpansionWorkKey<G> {
    owner: NonZeroU64,
    root: LaneId<G>,
    mark: ExpansionMark,
}

// These are shipping layout contracts, not test-only observations. Keeping
// controls in their stable lane prevents a parked expansion from inflating the
// heterogeneous continuation value.
const _: () = {
    assert!(core::mem::size_of::<ExpansionWorkKey<()>>() <= 32);
    assert!(core::mem::size_of::<ExpansionCommandSlot<()>>() <= 16);
    assert!(core::mem::size_of::<ExpansionControlSlot<()>>() <= 16);
    assert!(core::mem::size_of::<ExpansionNameMark>() <= 16);
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
    /// Number of re-entries into expanded delivery observed while a caller
    /// already owned an expanded-delivery invocation.  The counter is an
    /// architectural guard: migrated controls must return to the existing
    /// loop, while legacy cold scanners may still be measured during the
    /// transition.
    pub(crate) recursive_delivery_entries: u64,
    pub(crate) recursive_delivery_entries_with_control: u64,
}

#[derive(Debug)]
pub(crate) struct ExpansionWork<G> {
    owner: NonZeroU64,
    commands: FixedChunkLane<CurrentCommand<G>, G, COMMANDS_PER_CHUNK>,
    controls: FixedChunkLane<ExpansionControl<G>, G, CONTROLS_PER_CHUNK>,
    names: ExpansionNameLane,
    active_roots: Vec<LaneId<G>>,
    driver: ExpandedDeliveryDriver,
    counters: ExpansionWorkCounters,
}

impl<G> PartialEq for ExpansionWork<G> {
    fn eq(&self, other: &Self) -> bool {
        if self.is_quiescent() && other.is_quiescent() {
            return true;
        }
        self.owner == other.owner
            && self.active_roots == other.active_roots
            && self.commands.len() == other.commands.len()
            && self.controls.len() == other.controls.len()
            && self.names.len == other.names.len
            && self.driver == other.driver
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
            active_roots: Vec::new(),
            driver: ExpandedDeliveryDriver::default(),
            counters: ExpansionWorkCounters::default(),
        }
    }
}

impl<G> ExpansionWork<G> {
    /// Records an expanded-delivery entry against the one driver owner.  A
    /// nonzero active depth means that a scanner re-entered delivery instead
    /// of yielding a typed request; keeping this check beside the owner makes
    /// the migration guard independent of any particular primitive.
    pub(crate) fn note_delivery_entry(&mut self, active_depth: u32) {
        if active_depth == 0 {
            return;
        }
        self.counters.recursive_delivery_entries =
            self.counters.recursive_delivery_entries.saturating_add(1);
        if self.driver.continuation_depth != 0 {
            self.counters.recursive_delivery_entries_with_control = self
                .counters
                .recursive_delivery_entries_with_control
                .saturating_add(1);
        }
    }

    /// Pushes a synchronous `\the` continuation into the same generation
    /// owned control lane used by cold expansion suspension.  The control is
    /// intentionally not a second mailbox: its position is the canonical
    /// LIFO continuation coordinate and its payload is copy-small.
    pub(crate) fn push_the_control(
        &mut self,
        opener: tex_state::token::OriginId,
    ) -> Result<(), ScratchError> {
        self.driver.push_continuation()?;
        if let Err(error) = self.push_control(ExpansionControl::The(TheControl {
            opener,
            phase: ThePhase::NeedTarget,
        })) {
            self.driver
                .pop_continuation()
                .expect("failed the-control push restores driver depth");
            return Err(error);
        }
        Ok(())
    }

    /// Starts a synchronous `\csname` continuation. Name bytes share the
    /// existing fixed-chunk lane, while the control itself remains copy-small
    /// and independent of the rich command representation.
    pub(crate) fn push_csname_control(
        &mut self,
        opener: tex_state::token::OriginId,
        previous_in_csname: bool,
    ) -> Result<(), ScratchError> {
        let name = self.synchronous_name_mark()?;
        self.driver.push_continuation()?;
        if let Err(error) = self.push_control(ExpansionControl::CsName(SynchronousCsNameControl {
            opener,
            name,
            previous_in_csname,
        })) {
            self.driver
                .pop_continuation()
                .expect("failed csname-control push restores driver depth");
            return Err(error);
        }
        Ok(())
    }

    /// Starts a synchronous `\ifcsname` continuation over the same name lane.
    pub(crate) fn push_ifcsname_control(
        &mut self,
        condition: crate::processor::status::ConditionId,
        inverted: bool,
        previous_in_csname: bool,
    ) -> Result<(), ScratchError> {
        let name = self.synchronous_name_mark()?;
        self.driver.push_continuation()?;
        if let Err(error) =
            self.push_control(ExpansionControl::IfCsName(SynchronousIfCsNameControl {
                condition,
                inverted,
                name,
                previous_in_csname,
            }))
        {
            self.driver
                .pop_continuation()
                .expect("failed ifcsname-control push restores driver depth");
            return Err(error);
        }
        Ok(())
    }

    /// Starts the hot `\expandafter` protocol. The control carries only the
    /// opener provenance and a compact first-command slot; the second command
    /// stays in the expanded-delivery loop until its expansion settles.
    pub(crate) fn push_expandafter_control(
        &mut self,
        opener: tex_state::token::OriginId,
    ) -> Result<(), ScratchError> {
        self.driver.push_continuation()?;
        if let Err(error) = self.push_control(ExpansionControl::ExpandAfterSync(
            SynchronousExpandAfterControl {
                opener,
                saved_first: None,
                phase: SynchronousExpandAfterPhase::NeedFirst,
            },
        )) {
            self.driver
                .pop_continuation()
                .expect("failed expandafter-control push restores driver depth");
            return Err(error);
        }
        Ok(())
    }

    /// Returns the active top `\the` opener, if the synchronous continuation
    /// at the top of the control lane is one.  Looking at the lane's top slot
    /// avoids a parallel stack of continuation pointers.
    pub(crate) fn top_the_control(&self) -> Result<Option<TheControl>, ScratchError> {
        let id = match self.controls.top_id() {
            Ok(id) => id,
            Err(ScratchError::InvalidCoordinate) => return Ok(None),
            Err(error) => return Err(error),
        };
        match self.controls.get(id)? {
            ExpansionControl::The(control) => Ok(Some(*control)),
            _ => Ok(None),
        }
    }

    pub(crate) fn set_the_phase(&mut self, phase: ThePhase) -> Result<(), ScratchError> {
        let id = self.controls.top_id()?;
        let control = self.controls.get_mut(id)?;
        let ExpansionControl::The(control) = control else {
            return Err(ScratchError::InvalidCoordinate);
        };
        control.phase = phase;
        Ok(())
    }

    /// Pops exactly one completed synchronous `\the` continuation.
    pub(crate) fn pop_the_control(&mut self) -> Result<tex_state::token::OriginId, ScratchError> {
        let id = self.controls.top_id()?;
        match self.controls.get(id)? {
            ExpansionControl::The(_) => {}
            _ => return Err(ScratchError::InvalidCoordinate),
        }
        let opener = match self.controls.take_top(id)? {
            ExpansionControl::The(control) => control.opener,
            _ => unreachable!("validated top control remains a the continuation"),
        };
        self.driver.pop_continuation()?;
        Ok(opener)
    }

    /// Starts a synchronous `\expanded` collector in the shared control
    /// lane.  The token buffer was admitted by the enclosing attempt before
    /// this call, so a failed control admission cannot leave a detached
    /// collector owner behind.
    pub(crate) fn push_expanded_control(
        &mut self,
        opener: tex_state::token::OriginId,
        attempt_opening: crate::attempt::AttemptMark,
        writer: crate::attempt::AttemptTokenBufferId,
    ) -> Result<(), ScratchError> {
        self.driver.push_continuation()?;
        if let Err(error) =
            self.push_control(ExpansionControl::Expanded(SynchronousExpandedControl {
                opener,
                attempt_opening,
                writer,
                cursor: crate::scanner_kernel::ScannerCursor::default(),
                phase: SynchronousExpandedPhase::NeedOpening,
                kind: SynchronousExpandedKind::Expanded,
            }))
        {
            self.driver
                .pop_continuation()
                .expect("failed expanded-control push restores driver depth");
            return Err(error);
        }
        Ok(())
    }

    /// Starts the raw balanced child used by `\unexpanded` while an expanded
    /// collector is active. The child writes directly into its parent's
    /// attempt buffer, so completion only retires the control and never
    /// creates a second inserted input level.
    pub(crate) fn push_unexpanded_control(
        &mut self,
        opener: tex_state::token::OriginId,
        attempt_opening: crate::attempt::AttemptMark,
        writer: crate::attempt::AttemptTokenBufferId,
    ) -> Result<(), ScratchError> {
        self.driver.push_continuation()?;
        if let Err(error) =
            self.push_control(ExpansionControl::Expanded(SynchronousExpandedControl {
                opener,
                attempt_opening,
                writer,
                cursor: crate::scanner_kernel::ScannerCursor::default(),
                phase: SynchronousExpandedPhase::NeedOpening,
                kind: SynchronousExpandedKind::Unexpanded,
            }))
        {
            self.driver
                .pop_continuation()
                .expect("failed unexpanded-control push restores driver depth");
            return Err(error);
        }
        Ok(())
    }

    /// Starts the raw balanced child used by `\detokenize` while an expanded
    /// collector is active. The child converts each settled spelling directly
    /// into the parent's token buffer and retires without an intermediate
    /// token-list source.
    pub(crate) fn push_detokenize_control(
        &mut self,
        opener: tex_state::token::OriginId,
        attempt_opening: crate::attempt::AttemptMark,
        writer: crate::attempt::AttemptTokenBufferId,
    ) -> Result<(), ScratchError> {
        self.driver.push_continuation()?;
        if let Err(error) =
            self.push_control(ExpansionControl::Expanded(SynchronousExpandedControl {
                opener,
                attempt_opening,
                writer,
                cursor: crate::scanner_kernel::ScannerCursor::default(),
                phase: SynchronousExpandedPhase::NeedOpening,
                kind: SynchronousExpandedKind::Detokenize,
            }))
        {
            self.driver
                .pop_continuation()
                .expect("failed detokenize-control push restores driver depth");
            return Err(error);
        }
        Ok(())
    }

    /// Returns the active top `\expanded` collector, if any.
    pub(crate) fn top_expanded_control(
        &self,
    ) -> Result<Option<SynchronousExpandedControl>, ScratchError> {
        let id = match self.controls.top_id() {
            Ok(id) => id,
            Err(ScratchError::InvalidCoordinate) => return Ok(None),
            Err(error) => return Err(error),
        };
        match self.controls.get(id)? {
            ExpansionControl::Expanded(control) => Ok(Some(*control)),
            _ => Ok(None),
        }
    }

    /// Opens the balanced body after the required expanded left brace has
    /// settled.  The cursor remains in the control lane and is advanced by
    /// the same hot loop that delivers each body token.
    pub(crate) fn begin_expanded_body(&mut self) -> Result<(), ScratchError> {
        let id = self.controls.top_id()?;
        let control = self.controls.get_mut(id)?;
        let ExpansionControl::Expanded(control) = control else {
            return Err(ScratchError::InvalidCoordinate);
        };
        if control.phase != SynchronousExpandedPhase::NeedOpening {
            return Err(ScratchError::InvalidCoordinate);
        }
        control.cursor.open_balanced_body();
        control.phase = SynchronousExpandedPhase::Collecting;
        Ok(())
    }

    /// Settles one body spelling and reports whether it was the closing
    /// delimiter.  Only the literal catcode matters for `scan_toks` balance;
    /// semantic resolution and expansion are handled by the delivery loop.
    pub(crate) fn settle_expanded_word(
        &mut self,
        word: tex_state::token::TokenWord,
    ) -> Result<bool, ScratchError> {
        let id = self.controls.top_id()?;
        let control = self.controls.get_mut(id)?;
        let ExpansionControl::Expanded(control) = control else {
            return Err(ScratchError::InvalidCoordinate);
        };
        if control.phase != SynchronousExpandedPhase::Collecting {
            return Err(ScratchError::InvalidCoordinate);
        }
        Ok(control.cursor.settle_balanced_word(word))
    }

    /// Retires one completed synchronous `\expanded` collector and returns
    /// its attempt-owned output coordinate and opener provenance.
    pub(crate) fn pop_expanded_control(
        &mut self,
    ) -> Result<SynchronousExpandedControl, ScratchError> {
        let id = self.controls.top_id()?;
        let control = match self.controls.get(id)? {
            ExpansionControl::Expanded(control) => *control,
            _ => return Err(ScratchError::InvalidCoordinate),
        };
        if control.phase != SynchronousExpandedPhase::Collecting {
            return Err(ScratchError::InvalidCoordinate);
        }
        let _ = self.controls.take_top(id)?;
        self.driver.pop_continuation()?;
        Ok(control)
    }

    /// Returns the top synchronous `\csname` record without borrowing it
    /// across the next delivery instruction.
    pub(crate) fn top_csname_control(
        &self,
    ) -> Result<Option<SynchronousCsNameControl>, ScratchError> {
        let id = match self.controls.top_id() {
            Ok(id) => id,
            Err(ScratchError::InvalidCoordinate) => return Ok(None),
            Err(error) => return Err(error),
        };
        match self.controls.get(id)? {
            ExpansionControl::CsName(control) => Ok(Some(*control)),
            _ => Ok(None),
        }
    }

    /// Pops one completed `\csname`, retiring only its name-lane suffix.
    pub(crate) fn pop_csname_control(&mut self) -> Result<SynchronousCsNameControl, ScratchError> {
        let id = self.controls.top_id()?;
        let control = match self.controls.get(id)? {
            ExpansionControl::CsName(control) => *control,
            _ => return Err(ScratchError::InvalidCoordinate),
        };
        let _ = self.controls.take_top(id)?;
        self.names.truncate(control.name.offset)?;
        self.driver.pop_continuation()?;
        Ok(control)
    }

    /// Returns the active top `\ifcsname` continuation.
    pub(crate) fn top_ifcsname_control(
        &self,
    ) -> Result<Option<SynchronousIfCsNameControl>, ScratchError> {
        let id = match self.controls.top_id() {
            Ok(id) => id,
            Err(ScratchError::InvalidCoordinate) => return Ok(None),
            Err(error) => return Err(error),
        };
        match self.controls.get(id)? {
            ExpansionControl::IfCsName(control) => Ok(Some(*control)),
            _ => Ok(None),
        }
    }

    /// Pops one completed `\ifcsname` and retires its name suffix.
    pub(crate) fn pop_ifcsname_control(
        &mut self,
    ) -> Result<SynchronousIfCsNameControl, ScratchError> {
        let id = self.controls.top_id()?;
        let control = match self.controls.get(id)? {
            ExpansionControl::IfCsName(control) => *control,
            _ => return Err(ScratchError::InvalidCoordinate),
        };
        let _ = self.controls.take_top(id)?;
        self.names.truncate(control.name.offset)?;
        self.driver.pop_continuation()?;
        Ok(control)
    }

    /// Returns the active top hot `\expandafter` control.
    pub(crate) fn top_expandafter_control(
        &self,
    ) -> Result<Option<SynchronousExpandAfterControl<G>>, ScratchError> {
        let id = match self.controls.top_id() {
            Ok(id) => id,
            Err(ScratchError::InvalidCoordinate) => return Ok(None),
            Err(error) => return Err(error),
        };
        match self.controls.get(id)? {
            ExpansionControl::ExpandAfterSync(control) => Ok(Some(*control)),
            _ => Ok(None),
        }
    }

    /// Stores the first `\expandafter` operand in place and advances the
    /// control to its second-operand phase. The top-row borrow ends before
    /// the next delivery instruction is selected.
    pub(crate) fn save_expandafter_first(
        &mut self,
        first: crate::command::HotCommand<G>,
    ) -> Result<(), ScratchError> {
        let id = self.controls.top_id()?;
        let control = self.controls.get_mut(id)?;
        let ExpansionControl::ExpandAfterSync(control) = control else {
            return Err(ScratchError::InvalidCoordinate);
        };
        if control.phase != SynchronousExpandAfterPhase::NeedFirst || control.saved_first.is_some()
        {
            return Err(ScratchError::InvalidCoordinate);
        }
        control.saved_first = Some(first);
        // The compact first operand now acquires a lane-owned logical
        // lifetime. Count that ownership edge alongside cold command-slot
        // moves; no rich command copy is involved.
        crate::command::record_expansion_command_move_in();
        control.phase = SynchronousExpandAfterPhase::NeedSecond;
        Ok(())
    }

    /// Marks a second-operand primitive while one of its scanners is using
    /// the expanded-token request lane. The parent control remains parked but
    /// is ignored by nested delivery until that scanner returns.
    pub(crate) fn await_expandafter_nested(&mut self) -> Result<(), ScratchError> {
        let id = self.controls.top_id()?;
        let control = self.controls.get_mut(id)?;
        let ExpansionControl::ExpandAfterSync(control) = control else {
            return Err(ScratchError::InvalidCoordinate);
        };
        if control.phase != SynchronousExpandAfterPhase::NeedSecond || control.saved_first.is_none()
        {
            return Err(ScratchError::InvalidCoordinate);
        }
        control.phase = SynchronousExpandAfterPhase::AwaitNested;
        Ok(())
    }

    /// Re-enables the parent `\expandafter` after its nested primitive scanner
    /// has returned. The scanner's result, if any, is then consumed by the
    /// normal top-control branch of the delivery loop.
    pub(crate) fn resume_expandafter_second(&mut self) -> Result<(), ScratchError> {
        let id = self.controls.top_id()?;
        let control = self.controls.get_mut(id)?;
        let ExpansionControl::ExpandAfterSync(control) = control else {
            return Err(ScratchError::InvalidCoordinate);
        };
        if control.phase != SynchronousExpandAfterPhase::AwaitNested
            || control.saved_first.is_none()
        {
            return Err(ScratchError::InvalidCoordinate);
        }
        control.phase = SynchronousExpandAfterPhase::NeedSecond;
        Ok(())
    }

    /// Retires one completed hot `\expandafter` control and returns its
    /// compact first operand. Retiring before backup ensures the replay path
    /// cannot accidentally interpret the parent control as a nested operand.
    pub(crate) fn pop_expandafter_control(
        &mut self,
    ) -> Result<SynchronousExpandAfterControl<G>, ScratchError> {
        let id = self.controls.top_id()?;
        let control = match self.controls.get(id)? {
            ExpansionControl::ExpandAfterSync(control) => *control,
            _ => return Err(ScratchError::InvalidCoordinate),
        };
        if control.phase != SynchronousExpandAfterPhase::NeedSecond || control.saved_first.is_none()
        {
            return Err(ScratchError::InvalidCoordinate);
        }
        let _ = self.controls.take_top(id)?;
        self.driver.pop_continuation()?;
        crate::command::record_expansion_command_move_out();
        Ok(control)
    }

    /// Starts the compact two-operand `\if`/`\ifcat` comparison control.
    pub(crate) fn push_if_compare_control(
        &mut self,
        condition: crate::processor::status::ConditionId,
        kind: crate::conditionals::ConditionalKind,
        inverted: bool,
    ) -> Result<(), ScratchError> {
        self.driver.push_continuation()?;
        if let Err(error) =
            self.push_control(ExpansionControl::IfCompare(SynchronousIfCompareControl {
                condition,
                kind,
                inverted,
                phase: SynchronousIfComparePhase::NeedFirst,
            }))
        {
            self.driver
                .pop_continuation()
                .expect("failed if-compare push restores driver depth");
            return Err(error);
        }
        Ok(())
    }

    /// Returns the active top compact `\if`/`\ifcat` comparison control.
    pub(crate) fn top_if_compare_control(
        &self,
    ) -> Result<Option<SynchronousIfCompareControl>, ScratchError> {
        let id = match self.controls.top_id() {
            Ok(id) => id,
            Err(ScratchError::InvalidCoordinate) => return Ok(None),
            Err(error) => return Err(error),
        };
        match self.controls.get(id)? {
            ExpansionControl::IfCompare(control) => Ok(Some(*control)),
            _ => Ok(None),
        }
    }

    /// Advances the compact comparison control after its first operand has
    /// settled. No command-sized value crosses this mutation boundary.
    pub(crate) fn save_if_compare_first(
        &mut self,
        character: u32,
        category: Option<tex_state::token::Catcode>,
    ) -> Result<(), ScratchError> {
        let id = self.controls.top_id()?;
        let control = self.controls.get_mut(id)?;
        let ExpansionControl::IfCompare(control) = control else {
            return Err(ScratchError::InvalidCoordinate);
        };
        if control.phase != SynchronousIfComparePhase::NeedFirst {
            return Err(ScratchError::InvalidCoordinate);
        }
        control.phase = SynchronousIfComparePhase::NeedSecond {
            character,
            category,
        };
        Ok(())
    }

    /// Hides the comparison parent while one operand's nested scanner invokes
    /// expanded-token requests.
    pub(crate) fn await_if_compare_operand(&mut self) -> Result<(), ScratchError> {
        let id = self.controls.top_id()?;
        let control = self.controls.get_mut(id)?;
        let ExpansionControl::IfCompare(control) = control else {
            return Err(ScratchError::InvalidCoordinate);
        };
        control.phase = match control.phase {
            SynchronousIfComparePhase::NeedFirst => SynchronousIfComparePhase::AwaitFirst,
            SynchronousIfComparePhase::NeedSecond {
                character,
                category,
            } => SynchronousIfComparePhase::AwaitSecond {
                character,
                category,
            },
            SynchronousIfComparePhase::AwaitFirst
            | SynchronousIfComparePhase::AwaitSecond { .. } => {
                return Err(ScratchError::InvalidCoordinate);
            }
        };
        Ok(())
    }

    /// Restores the comparison's operand phase once the nested primitive
    /// scanner has returned to the driver.
    pub(crate) fn resume_if_compare_operand(&mut self) -> Result<(), ScratchError> {
        let id = self.controls.top_id()?;
        let control = self.controls.get_mut(id)?;
        let ExpansionControl::IfCompare(control) = control else {
            return Err(ScratchError::InvalidCoordinate);
        };
        control.phase = match control.phase {
            SynchronousIfComparePhase::AwaitFirst => SynchronousIfComparePhase::NeedFirst,
            SynchronousIfComparePhase::AwaitSecond {
                character,
                category,
            } => SynchronousIfComparePhase::NeedSecond {
                character,
                category,
            },
            SynchronousIfComparePhase::NeedFirst | SynchronousIfComparePhase::NeedSecond { .. } => {
                return Err(ScratchError::InvalidCoordinate);
            }
        };
        Ok(())
    }

    /// Retires the completed compact comparison control.
    pub(crate) fn pop_if_compare_control(
        &mut self,
    ) -> Result<SynchronousIfCompareControl, ScratchError> {
        let id = self.controls.top_id()?;
        let control = match self.controls.get(id)? {
            ExpansionControl::IfCompare(control) => *control,
            _ => return Err(ScratchError::InvalidCoordinate),
        };
        if !matches!(control.phase, SynchronousIfComparePhase::NeedSecond { .. }) {
            return Err(ScratchError::InvalidCoordinate);
        }
        let _ = self.controls.take_top(id)?;
        self.driver.pop_continuation()?;
        Ok(control)
    }

    /// Starts the compact numeric/dimension comparison protocol.
    pub(crate) fn push_if_number_control(
        &mut self,
        condition: crate::processor::status::ConditionId,
        kind: crate::conditionals::ConditionalKind,
        inverted: bool,
    ) -> Result<(), ScratchError> {
        self.driver.push_continuation()?;
        if let Err(error) =
            self.push_control(ExpansionControl::IfNumber(SynchronousIfNumberControl {
                condition,
                kind,
                inverted,
                phase: SynchronousIfNumberPhase::NeedLeft,
            }))
        {
            self.driver
                .pop_continuation()
                .expect("failed if-number-control push restores driver depth");
            return Err(error);
        }
        Ok(())
    }

    /// Returns the active compact numeric/dimension comparison.
    pub(crate) fn top_if_number_control(
        &self,
    ) -> Result<Option<SynchronousIfNumberControl>, ScratchError> {
        let id = match self.controls.top_id() {
            Ok(id) => id,
            Err(ScratchError::InvalidCoordinate) => return Ok(None),
            Err(error) => return Err(error),
        };
        match self.controls.get(id)? {
            ExpansionControl::IfNumber(control) => Ok(Some(*control)),
            _ => Ok(None),
        }
    }

    /// Mutates the numeric/dimension comparison phase in place.  All payloads
    /// are scalar, so no command-sized owner is copied through the control
    /// lane while an operand is being expanded.
    pub(crate) fn set_if_number_phase(
        &mut self,
        phase: SynchronousIfNumberPhase,
    ) -> Result<(), ScratchError> {
        let id = self.controls.top_id()?;
        let control = self.controls.get_mut(id)?;
        let ExpansionControl::IfNumber(control) = control else {
            return Err(ScratchError::InvalidCoordinate);
        };
        control.phase = phase;
        Ok(())
    }

    /// Hides a numeric operand parent while a nested expandable command is
    /// being interpreted by the same delivery loop.
    pub(crate) fn await_if_number_operand(&mut self) -> Result<(), ScratchError> {
        let id = self.controls.top_id()?;
        let control = self.controls.get_mut(id)?;
        let ExpansionControl::IfNumber(control) = control else {
            return Err(ScratchError::InvalidCoordinate);
        };
        control.phase = match control.phase {
            SynchronousIfNumberPhase::NeedLeft => SynchronousIfNumberPhase::AwaitLeft {
                negative: false,
                value: 0,
                seen_digit: false,
            },
            SynchronousIfNumberPhase::Left {
                negative,
                value,
                seen_digit,
            } => SynchronousIfNumberPhase::AwaitLeft {
                negative,
                value,
                seen_digit,
            },
            SynchronousIfNumberPhase::NeedRelation { left } => {
                SynchronousIfNumberPhase::AwaitRelation { left }
            }
            SynchronousIfNumberPhase::Right {
                left,
                relation,
                negative,
                value,
                seen_digit,
            } => SynchronousIfNumberPhase::AwaitRight {
                left,
                relation,
                negative,
                value,
                seen_digit,
            },
            SynchronousIfNumberPhase::RegisterIndex {
                target,
                negative,
                value,
                seen_digit,
            } => SynchronousIfNumberPhase::RegisterIndexAwait {
                target,
                negative,
                value,
                seen_digit,
            },
            SynchronousIfNumberPhase::RegisterIndexAwait { .. } => {
                return Err(ScratchError::InvalidCoordinate);
            }
            SynchronousIfNumberPhase::AwaitLeft { .. }
            | SynchronousIfNumberPhase::AwaitRelation { .. }
            | SynchronousIfNumberPhase::AwaitRight { .. } => {
                return Err(ScratchError::InvalidCoordinate);
            }
        };
        Ok(())
    }

    /// Restores a numeric operand phase after its nested expansion settled.
    pub(crate) fn resume_if_number_operand(&mut self) -> Result<(), ScratchError> {
        let id = self.controls.top_id()?;
        let control = self.controls.get_mut(id)?;
        let ExpansionControl::IfNumber(control) = control else {
            return Err(ScratchError::InvalidCoordinate);
        };
        control.phase = match control.phase {
            SynchronousIfNumberPhase::AwaitLeft {
                negative,
                value,
                seen_digit,
            } => SynchronousIfNumberPhase::Left {
                negative,
                value,
                seen_digit,
            },
            SynchronousIfNumberPhase::AwaitRelation { left } => {
                SynchronousIfNumberPhase::NeedRelation { left }
            }
            SynchronousIfNumberPhase::AwaitRight {
                left,
                relation,
                negative,
                value,
                seen_digit,
            } => SynchronousIfNumberPhase::Right {
                left,
                relation,
                negative,
                value,
                seen_digit,
            },
            SynchronousIfNumberPhase::RegisterIndexAwait {
                target,
                negative,
                value,
                seen_digit,
            } => SynchronousIfNumberPhase::RegisterIndex {
                target,
                negative,
                value,
                seen_digit,
            },
            SynchronousIfNumberPhase::NeedLeft
            | SynchronousIfNumberPhase::Left { .. }
            | SynchronousIfNumberPhase::NeedRelation { .. }
            | SynchronousIfNumberPhase::Right { .. }
            | SynchronousIfNumberPhase::RegisterIndex { .. } => {
                return Err(ScratchError::InvalidCoordinate);
            }
        };
        Ok(())
    }

    /// Retires one completed numeric/dimension comparison control.
    pub(crate) fn pop_if_number_control(
        &mut self,
    ) -> Result<SynchronousIfNumberControl, ScratchError> {
        let id = self.controls.top_id()?;
        let control = match self.controls.get(id)? {
            ExpansionControl::IfNumber(control) => *control,
            _ => return Err(ScratchError::InvalidCoordinate),
        };
        if !matches!(
            control.phase,
            SynchronousIfNumberPhase::NeedRelation { .. }
                | SynchronousIfNumberPhase::Right { .. }
                // Unary `\ifodd`/`\ifcase` complete as soon as their
                // accumulator sees the first terminator, while the binary
                // protocol reaches `Right` first.
                | SynchronousIfNumberPhase::Left { .. }
                | SynchronousIfNumberPhase::RegisterIndex { .. }
        ) {
            return Err(ScratchError::InvalidCoordinate);
        }
        let _ = self.controls.take_top(id)?;
        self.driver.pop_continuation()?;
        Ok(control)
    }

    /// Starts the compact literal `\ifdim` protocol.
    pub(crate) fn push_if_dimension_control(
        &mut self,
        condition: crate::processor::status::ConditionId,
        kind: crate::conditionals::ConditionalKind,
        inverted: bool,
    ) -> Result<(), ScratchError> {
        self.driver.push_continuation()?;
        if let Err(error) = self.push_control(ExpansionControl::IfDimension(
            SynchronousIfDimensionControl {
                condition,
                kind,
                inverted,
                phase: SynchronousIfDimensionPhase::NeedLeft,
            },
        )) {
            self.driver
                .pop_continuation()
                .expect("failed if-dimension-control push restores driver depth");
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn top_if_dimension_control(
        &self,
    ) -> Result<Option<SynchronousIfDimensionControl>, ScratchError> {
        let id = match self.controls.top_id() {
            Ok(id) => id,
            Err(ScratchError::InvalidCoordinate) => return Ok(None),
            Err(error) => return Err(error),
        };
        match self.controls.get(id)? {
            ExpansionControl::IfDimension(control) => Ok(Some(*control)),
            _ => Ok(None),
        }
    }

    pub(crate) fn set_if_dimension_phase(
        &mut self,
        phase: SynchronousIfDimensionPhase,
    ) -> Result<(), ScratchError> {
        let id = self.controls.top_id()?;
        let control = self.controls.get_mut(id)?;
        let ExpansionControl::IfDimension(control) = control else {
            return Err(ScratchError::InvalidCoordinate);
        };
        control.phase = phase;
        Ok(())
    }

    pub(crate) fn await_if_dimension_operand(&mut self) -> Result<(), ScratchError> {
        let id = self.controls.top_id()?;
        let control = self.controls.get_mut(id)?;
        let ExpansionControl::IfDimension(control) = control else {
            return Err(ScratchError::InvalidCoordinate);
        };
        control.phase = match control.phase {
            SynchronousIfDimensionPhase::NeedLeft => SynchronousIfDimensionPhase::AwaitLeft {
                negative: false,
                value: 0,
                fraction: 0,
                fraction_digits: 0,
                decimal: false,
                unit: 0,
                seen_digit: false,
            },
            SynchronousIfDimensionPhase::Left {
                negative,
                value,
                fraction,
                fraction_digits,
                decimal,
                unit,
                seen_digit,
            } => SynchronousIfDimensionPhase::AwaitLeft {
                negative,
                value,
                fraction,
                fraction_digits,
                decimal,
                unit,
                seen_digit,
            },
            SynchronousIfDimensionPhase::NeedRelation { left } => {
                SynchronousIfDimensionPhase::AwaitRelation { left }
            }
            SynchronousIfDimensionPhase::Right {
                left,
                relation,
                negative,
                value,
                fraction,
                fraction_digits,
                decimal,
                unit,
                seen_digit,
            } => SynchronousIfDimensionPhase::AwaitRight {
                left,
                relation,
                negative,
                value,
                fraction,
                fraction_digits,
                decimal,
                unit,
                seen_digit,
            },
            SynchronousIfDimensionPhase::RegisterIndex {
                target,
                negative,
                value,
                seen_digit,
            } => SynchronousIfDimensionPhase::RegisterIndexAwait {
                target,
                negative,
                value,
                seen_digit,
            },
            SynchronousIfDimensionPhase::RegisterIndexAwait { .. } => {
                return Err(ScratchError::InvalidCoordinate);
            }
            SynchronousIfDimensionPhase::AwaitLeft { .. }
            | SynchronousIfDimensionPhase::AwaitRelation { .. }
            | SynchronousIfDimensionPhase::AwaitRight { .. } => {
                return Err(ScratchError::InvalidCoordinate);
            }
        };
        Ok(())
    }

    pub(crate) fn resume_if_dimension_operand(&mut self) -> Result<(), ScratchError> {
        let id = self.controls.top_id()?;
        let control = self.controls.get_mut(id)?;
        let ExpansionControl::IfDimension(control) = control else {
            return Err(ScratchError::InvalidCoordinate);
        };
        control.phase = match control.phase {
            SynchronousIfDimensionPhase::AwaitLeft {
                negative,
                value,
                fraction,
                fraction_digits,
                decimal,
                unit,
                seen_digit,
            } => SynchronousIfDimensionPhase::Left {
                negative,
                value,
                fraction,
                fraction_digits,
                decimal,
                unit,
                seen_digit,
            },
            SynchronousIfDimensionPhase::AwaitRelation { left } => {
                SynchronousIfDimensionPhase::NeedRelation { left }
            }
            SynchronousIfDimensionPhase::AwaitRight {
                left,
                relation,
                negative,
                value,
                fraction,
                fraction_digits,
                decimal,
                unit,
                seen_digit,
            } => SynchronousIfDimensionPhase::Right {
                left,
                relation,
                negative,
                value,
                fraction,
                fraction_digits,
                decimal,
                unit,
                seen_digit,
            },
            SynchronousIfDimensionPhase::RegisterIndexAwait {
                target,
                negative,
                value,
                seen_digit,
            } => SynchronousIfDimensionPhase::RegisterIndex {
                target,
                negative,
                value,
                seen_digit,
            },
            SynchronousIfDimensionPhase::NeedLeft
            | SynchronousIfDimensionPhase::Left { .. }
            | SynchronousIfDimensionPhase::NeedRelation { .. }
            | SynchronousIfDimensionPhase::Right { .. }
            | SynchronousIfDimensionPhase::RegisterIndex { .. } => {
                return Err(ScratchError::InvalidCoordinate);
            }
        };
        Ok(())
    }

    pub(crate) fn pop_if_dimension_control(
        &mut self,
    ) -> Result<SynchronousIfDimensionControl, ScratchError> {
        let id = self.controls.top_id()?;
        let control = match self.controls.get(id)? {
            ExpansionControl::IfDimension(control) => *control,
            _ => return Err(ScratchError::InvalidCoordinate),
        };
        if !matches!(
            control.phase,
            SynchronousIfDimensionPhase::NeedRelation { .. }
                | SynchronousIfDimensionPhase::Right { .. }
                | SynchronousIfDimensionPhase::RegisterIndex { .. }
        ) {
            return Err(ScratchError::InvalidCoordinate);
        }
        let _ = self.controls.take_top(id)?;
        self.driver.pop_continuation()?;
        Ok(control)
    }

    /// Starts a compact integer conversion (`\number` or `\romannumeral`).
    pub(crate) fn push_number_control(
        &mut self,
        opener: tex_state::token::OriginId,
        roman: bool,
    ) -> Result<(), ScratchError> {
        self.driver.push_continuation()?;
        if let Err(error) = self.push_control(ExpansionControl::Number(SynchronousNumberControl {
            opener,
            roman,
            phase: SynchronousNumberPhase::Need,
        })) {
            self.driver
                .pop_continuation()
                .expect("failed number-control push restores driver depth");
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn top_number_control(
        &self,
    ) -> Result<Option<SynchronousNumberControl>, ScratchError> {
        let id = match self.controls.top_id() {
            Ok(id) => id,
            Err(ScratchError::InvalidCoordinate) => return Ok(None),
            Err(error) => return Err(error),
        };
        match self.controls.get(id)? {
            ExpansionControl::Number(control) => Ok(Some(*control)),
            _ => Ok(None),
        }
    }

    pub(crate) fn set_number_phase(
        &mut self,
        phase: SynchronousNumberPhase,
    ) -> Result<(), ScratchError> {
        let id = self.controls.top_id()?;
        let control = self.controls.get_mut(id)?;
        let ExpansionControl::Number(control) = control else {
            return Err(ScratchError::InvalidCoordinate);
        };
        control.phase = phase;
        Ok(())
    }

    pub(crate) fn await_number_operand(&mut self) -> Result<(), ScratchError> {
        let id = self.controls.top_id()?;
        let control = self.controls.get_mut(id)?;
        let ExpansionControl::Number(control) = control else {
            return Err(ScratchError::InvalidCoordinate);
        };
        control.phase = match control.phase {
            SynchronousNumberPhase::Need => SynchronousNumberPhase::Await {
                negative: false,
                value: 0,
                seen_digit: false,
            },
            SynchronousNumberPhase::Accumulating {
                negative,
                value,
                seen_digit,
            } => SynchronousNumberPhase::Await {
                negative,
                value,
                seen_digit,
            },
            SynchronousNumberPhase::RegisterIndex {
                target,
                negative,
                value,
                seen_digit,
            } => SynchronousNumberPhase::RegisterIndexAwait {
                target,
                negative,
                value,
                seen_digit,
            },
            SynchronousNumberPhase::RegisterIndexAwait { .. } => {
                return Err(ScratchError::InvalidCoordinate);
            }
            SynchronousNumberPhase::Await { .. } => {
                return Err(ScratchError::InvalidCoordinate);
            }
        };
        Ok(())
    }

    pub(crate) fn resume_number_operand(&mut self) -> Result<(), ScratchError> {
        let id = self.controls.top_id()?;
        let control = self.controls.get_mut(id)?;
        let ExpansionControl::Number(control) = control else {
            return Err(ScratchError::InvalidCoordinate);
        };
        control.phase = match control.phase {
            SynchronousNumberPhase::Await {
                negative,
                value,
                seen_digit,
            } => SynchronousNumberPhase::Accumulating {
                negative,
                value,
                seen_digit,
            },
            SynchronousNumberPhase::RegisterIndexAwait {
                target,
                negative,
                value,
                seen_digit,
            } => SynchronousNumberPhase::RegisterIndex {
                target,
                negative,
                value,
                seen_digit,
            },
            SynchronousNumberPhase::Need
            | SynchronousNumberPhase::Accumulating { .. }
            | SynchronousNumberPhase::RegisterIndex { .. } => {
                return Err(ScratchError::InvalidCoordinate);
            }
        };
        Ok(())
    }

    pub(crate) fn pop_number_control(&mut self) -> Result<SynchronousNumberControl, ScratchError> {
        let id = self.controls.top_id()?;
        let control = match self.controls.get(id)? {
            ExpansionControl::Number(control) => *control,
            _ => return Err(ScratchError::InvalidCoordinate),
        };
        if !matches!(
            control.phase,
            SynchronousNumberPhase::Need
                | SynchronousNumberPhase::Accumulating { .. }
                | SynchronousNumberPhase::RegisterIndex { .. }
                | SynchronousNumberPhase::RegisterIndexAwait { .. }
        ) {
            return Err(ScratchError::InvalidCoordinate);
        }
        let _ = self.controls.take_top(id)?;
        self.driver.pop_continuation()?;
        Ok(control)
    }

    /// Starts a synchronous `\fontname` operand in the shared delivery lane.
    pub(crate) fn push_fontname_control(
        &mut self,
        opener: tex_state::token::OriginId,
    ) -> Result<(), ScratchError> {
        self.driver.push_continuation()?;
        if let Err(error) =
            self.push_control(ExpansionControl::FontName(SynchronousFontNameControl {
                opener,
            }))
        {
            self.driver
                .pop_continuation()
                .expect("failed fontname-control push restores driver depth");
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn top_fontname_control(
        &self,
    ) -> Result<Option<SynchronousFontNameControl>, ScratchError> {
        let id = match self.controls.top_id() {
            Ok(id) => id,
            Err(ScratchError::InvalidCoordinate) => return Ok(None),
            Err(error) => return Err(error),
        };
        match self.controls.get(id)? {
            ExpansionControl::FontName(control) => Ok(Some(*control)),
            _ => Ok(None),
        }
    }

    pub(crate) fn pop_fontname_control(
        &mut self,
    ) -> Result<SynchronousFontNameControl, ScratchError> {
        let id = self.controls.top_id()?;
        let control = match self.controls.get(id)? {
            ExpansionControl::FontName(control) => *control,
            _ => return Err(ScratchError::InvalidCoordinate),
        };
        let _ = self.controls.take_top(id)?;
        self.driver.pop_continuation()?;
        Ok(control)
    }

    /// Opens a name mark even when a synchronous driver has no parked cold
    /// root. A zero root serial is reserved for that rootless hot episode;
    /// parked resumptions continue to use their real active-root serial.
    fn synchronous_name_mark(&self) -> Result<ExpansionNameMark, ScratchError> {
        match self.active_roots.last().copied() {
            Some(root) => Ok(ExpansionNameMark {
                owner: self.owner,
                root_serial: root.serial(),
                offset: self.names.len,
            }),
            None => Ok(ExpansionNameMark {
                owner: self.owner,
                root_serial: 0,
                offset: self.names.len,
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn driver_continuation_depth(&self) -> u32 {
        self.driver.continuation_depth()
    }

    /// Parks the one command owner only after expansion has produced a real
    /// immutable-resource suspension. Every fallible step restores the exact
    /// pending value to the caller.
    // The cold error must return the sole command owner intact. Boxing it
    // would allocate precisely on the failed-park path this contract protects.
    #[allow(clippy::result_large_err)]
    pub(crate) fn park_suspension(
        &mut self,
        pending: crate::state::PendingExpansion<G>,
    ) -> Result<ExpansionWorkKey<G>, (ScratchError, crate::state::PendingExpansion<G>)> {
        if self.active_roots.try_reserve(1).is_err() {
            return Err((ScratchError::AllocationFailed, pending));
        }
        let mark = self.mark();
        let crate::state::PendingExpansion {
            command,
            resume,
            delivery_expanded,
            child,
        } = pending;
        let mut command = Some(command);
        let command_slot = match self.park_command_from(&mut command) {
            Ok(slot) => slot,
            Err(error) => {
                return Err((
                    error,
                    crate::state::PendingExpansion {
                        command: command.expect("failed command park preserves owner"),
                        resume,
                        delivery_expanded,
                        child,
                    },
                ));
            }
        };
        let mut control = Some(ExpansionControl::Suspended {
            command: command_slot,
            resume,
            delivery_expanded,
            child,
        });
        let root = match self.push_control_from(&mut control) {
            Ok(root) => root,
            Err(error) => {
                let command = self
                    .take_command(command_slot)
                    .expect("failed control park restores its top command");
                let ExpansionControl::Suspended {
                    resume,
                    delivery_expanded,
                    child,
                    ..
                } = control.expect("failed control park preserves owner")
                else {
                    unreachable!("production park builds a suspended root")
                };
                return Err((
                    error,
                    crate::state::PendingExpansion {
                        command,
                        resume,
                        delivery_expanded,
                        child,
                    },
                ));
            }
        };
        self.active_roots.push(root.lane);
        Ok(ExpansionWorkKey {
            owner: self.owner,
            root: root.lane,
            mark,
        })
    }

    /// Consumes a parked command and its exact continuation once for retry.
    pub(crate) fn resume_suspension(
        &mut self,
        key: ExpansionWorkKey<G>,
    ) -> Result<crate::state::PendingExpansion<G>, ScratchError> {
        self.take_suspension(key, true)
    }

    /// Consumes a parked command and continuation for cancellation/abort.
    pub(crate) fn cancel_suspension(
        &mut self,
        key: ExpansionWorkKey<G>,
    ) -> Result<crate::state::PendingExpansion<G>, ScratchError> {
        self.take_suspension(key, false)
    }

    pub(crate) fn begin_dispatch(
        &mut self,
        command: CurrentCommand<G>,
    ) -> Result<ExpansionWorkKey<G>, ScratchError> {
        if !self.is_quiescent() {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.active_roots
            .try_reserve(1)
            .map_err(|_| ScratchError::AllocationFailed)?;
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
        self.active_roots.push(root.lane);
        Ok(ExpansionWorkKey {
            owner: self.owner,
            root: root.lane,
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
        Ok(ExpansionCommandSlot {
            owner: self.owner,
            lane: id,
        })
    }

    fn park_command_from(
        &mut self,
        command: &mut Option<CurrentCommand<G>>,
    ) -> Result<ExpansionCommandSlot<G>, ScratchError> {
        let id = self.commands.push_from(command)?;
        crate::command::record_expansion_command_move_in();
        self.counters.command_moves_in = self.counters.command_moves_in.saturating_add(1);
        self.counters.max_command_depth = self.counters.max_command_depth.max(self.commands.len());
        Ok(ExpansionCommandSlot {
            owner: self.owner,
            lane: id,
        })
    }

    pub(crate) fn take_command(
        &mut self,
        slot: ExpansionCommandSlot<G>,
    ) -> Result<CurrentCommand<G>, ScratchError> {
        let command = self.commands.take_top(self.validate_command_slot(slot)?)?;
        crate::command::record_expansion_command_move_out();
        self.counters.command_moves_out = self.counters.command_moves_out.saturating_add(1);
        Ok(command)
    }

    pub(crate) fn command(
        &self,
        slot: ExpansionCommandSlot<G>,
    ) -> Result<&CurrentCommand<G>, ScratchError> {
        self.commands.get(self.validate_command_slot(slot)?)
    }

    pub(crate) fn push_control(
        &mut self,
        control: ExpansionControl<G>,
    ) -> Result<ExpansionControlSlot<G>, ScratchError> {
        let id = self.controls.push(control)?;
        self.counters.control_pushes = self.counters.control_pushes.saturating_add(1);
        self.counters.max_control_depth = self.counters.max_control_depth.max(self.controls.len());
        Ok(ExpansionControlSlot {
            owner: self.owner,
            lane: id,
        })
    }

    fn push_control_from(
        &mut self,
        control: &mut Option<ExpansionControl<G>>,
    ) -> Result<ExpansionControlSlot<G>, ScratchError> {
        let id = self.controls.push_from(control)?;
        self.counters.control_pushes = self.counters.control_pushes.saturating_add(1);
        self.counters.max_control_depth = self.counters.max_control_depth.max(self.controls.len());
        Ok(ExpansionControlSlot {
            owner: self.owner,
            lane: id,
        })
    }

    pub(crate) fn control_mut(
        &mut self,
        slot: ExpansionControlSlot<G>,
    ) -> Result<&mut ExpansionControl<G>, ScratchError> {
        let id = self.validate_control_slot(slot)?;
        self.controls.get_mut(id)
    }

    pub(crate) fn pop_control(
        &mut self,
        slot: ExpansionControlSlot<G>,
    ) -> Result<ExpansionControl<G>, ScratchError> {
        let id = self.validate_control_slot(slot)?;
        self.controls.take_top(id)
    }

    pub(crate) fn name_mark(&self) -> Result<ExpansionNameMark, ScratchError> {
        let root = self
            .active_roots
            .last()
            .copied()
            .ok_or(ScratchError::InvalidCoordinate)?;
        Ok(ExpansionNameMark {
            owner: self.owner,
            root_serial: root.serial(),
            offset: self.names.len,
        })
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
        let root = self.active_roots.last().copied();
        let root_matches = match (mark.root_serial, root) {
            (0, None) => true,
            (serial, Some(root)) => serial == root.serial(),
            _ => false,
        };
        if mark.owner != self.owner || !root_matches {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.names.bytes_from(mark.offset)
    }

    pub(crate) fn finish(&mut self, key: ExpansionWorkKey<G>) -> Result<(), ScratchError> {
        self.validate_key(&key)?;
        self.truncate_to(key.mark)?;
        let popped = self.active_roots.pop();
        debug_assert!(popped == Some(key.root));
        self.counters.completed_roots = self.counters.completed_roots.saturating_add(1);
        Ok(())
    }

    pub(crate) fn abort(&mut self, key: ExpansionWorkKey<G>) -> Result<(), ScratchError> {
        self.validate_key(&key)?;
        // Controls are retired newest-first before their command and text
        // destinations, preserving the child-before-parent ownership order.
        self.truncate_to(key.mark)?;
        let popped = self.active_roots.pop();
        debug_assert!(popped == Some(key.root));
        if self.active_roots.is_empty() {
            // A synchronous parent (currently `\the`) has no parked root of
            // its own.  Once its last cold child aborts, discard that parent
            // control as part of the same deepest-first abort rather than
            // leaking a continuation into the next processor episode.
            self.controls.truncate(0)?;
            self.driver = ExpandedDeliveryDriver::default();
        }
        self.counters.aborted_roots = self.counters.aborted_roots.saturating_add(1);
        Ok(())
    }

    pub(crate) fn is_quiescent(&self) -> bool {
        self.active_roots.is_empty()
            && self.controls.len() == 0
            && self.commands.len() == 0
            && self.names.len == 0
            && self.driver.continuation_depth == 0
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
            && self.active_roots.last().copied() == Some(key.root)
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

    fn take_suspension(
        &mut self,
        key: ExpansionWorkKey<G>,
        completed: bool,
    ) -> Result<crate::state::PendingExpansion<G>, ScratchError> {
        self.validate_key(&key)?;
        let root = ExpansionControlSlot {
            owner: self.owner,
            lane: key.root,
        };
        let command = match self.controls.get(key.root)? {
            ExpansionControl::Suspended { command, .. } => *command,
            _ => return Err(ScratchError::InvalidCoordinate),
        };
        if key.root.index().checked_add(1) != Some(self.controls.len())
            || command.lane.index().checked_add(1) != Some(self.commands.len())
        {
            return Err(ScratchError::InvalidCoordinate);
        }
        let ExpansionControl::Suspended {
            resume,
            delivery_expanded,
            child,
            ..
        } = self.pop_control(root)?
        else {
            unreachable!("validated suspended root remains suspended")
        };
        let command = self.take_command(command)?;
        self.truncate_to(key.mark)?;
        let popped = self.active_roots.pop();
        debug_assert!(popped == Some(key.root));
        if completed {
            self.counters.completed_roots = self.counters.completed_roots.saturating_add(1);
        } else {
            self.counters.aborted_roots = self.counters.aborted_roots.saturating_add(1);
        }
        Ok(crate::state::PendingExpansion {
            command,
            resume,
            delivery_expanded,
            child,
        })
    }

    fn validate_command_slot(
        &self,
        slot: ExpansionCommandSlot<G>,
    ) -> Result<LaneId<G>, ScratchError> {
        if slot.owner != self.owner {
            return Err(ScratchError::InvalidCoordinate);
        }
        Ok(slot.lane)
    }

    fn validate_control_slot(
        &self,
        slot: ExpansionControlSlot<G>,
    ) -> Result<LaneId<G>, ScratchError> {
        if slot.owner != self.owner {
            return Err(ScratchError::InvalidCoordinate);
        }
        Ok(slot.lane)
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
