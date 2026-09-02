//! Generation-owned reusable command execution scratch.
//!
//! Macro matchers append directly to one reusable LIFO word lane. Sealing
//! changes only the owning frame role; retirement truncates to the frame's
//! inherited lane mark once no pending child owns the suffix. No macro
//! invocation owns a heap buffer, linked segment, or attempt-arena scope.

#[cfg(any(test, feature = "profiling"))]
use core::cell::Cell;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::num::NonZeroU64;

use tex_state::token::{Catcode, OriginId, TokenWord, TracedTokenWord};

use crate::token_collector::ClassifiedToken;

const MACRO_WORD_RESERVE: usize = 4_096;
const NO_MACRO_SLOT: u32 = u32::MAX;
const ARGUMENT_SET_SLOT_BITS: u32 = 24;
const ARGUMENT_SET_SLOT_MASK: u64 = (1_u64 << ARGUMENT_SET_SLOT_BITS) - 1;
const ARGUMENT_SET_SERIAL_LIMIT: u64 = 1_u64 << (64 - ARGUMENT_SET_SLOT_BITS);

#[derive(Debug)]
struct ResumeFrameId<G> {
    slot: u32,
    serial: u64,
    _generation: PhantomData<fn(&G) -> &G>,
}

impl<G> Copy for ResumeFrameId<G> {}
impl<G> Clone for ResumeFrameId<G> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<G> PartialEq for ResumeFrameId<G> {
    fn eq(&self, other: &Self) -> bool {
        self.slot == other.slot && self.serial == other.serial
    }
}
impl<G> Eq for ResumeFrameId<G> {}

/// Exact move-only root capability for one typed suspended continuation.
#[derive(Debug, Eq, PartialEq)]
pub struct ScannerFrameKey<G> {
    id: ResumeFrameId<G>,
    kind: ContinuationKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContinuationKind {
    Scanner,
    Scalar,
    Expansion,
    ExpandAfter,
    PdfStringCompare,
    AlignmentPreamble,
    StructuredScanner,
}

impl<G> ScannerFrameKey<G> {
    pub(crate) fn is_scanner(&self) -> bool {
        self.kind == ContinuationKind::Scanner
    }

    pub(crate) fn is_expansion(&self) -> bool {
        self.kind == ContinuationKind::Expansion
    }

    pub(crate) fn is_scalar(&self) -> bool {
        self.kind == ContinuationKind::Scalar
    }

    pub(crate) fn is_expandafter(&self) -> bool {
        self.kind == ContinuationKind::ExpandAfter
    }

    pub(crate) fn is_pdf_string_compare(&self) -> bool {
        self.kind == ContinuationKind::PdfStringCompare
    }

    pub(crate) fn is_alignment_preamble(&self) -> bool {
        self.kind == ContinuationKind::AlignmentPreamble
    }

    pub(crate) fn is_structured_scanner(&self) -> bool {
        self.kind == ContinuationKind::StructuredScanner
    }

    #[cfg(test)]
    pub(crate) const fn injected_scan_toks_publication_collision() -> Self {
        Self {
            id: ResumeFrameId {
                slot: u32::MAX,
                serial: u64::MAX,
                _generation: PhantomData,
            },
            kind: ContinuationKind::Scanner,
        }
    }

    #[cfg(test)]
    pub(crate) const fn is_injected_scan_toks_publication_collision(&self) -> bool {
        self.id.slot == u32::MAX
            && self.id.serial == u64::MAX
            && matches!(self.kind, ContinuationKind::Scanner)
    }
}

/// One structurally owned child edge and the caller phase that receives it.
///
/// The key is deliberately non-`Copy`: suspension moves the live child out
/// of the processor baton and resumption consumes this edge before the caller
/// phase can run again.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ChildContinuation<G, D> {
    key: ScannerFrameKey<G>,
    destination: D,
}

impl<G, D> ChildContinuation<G, D> {
    pub(crate) fn capture(baton: &mut Option<ScannerFrameKey<G>>, destination: D) -> Option<Self> {
        baton.take().map(|key| Self { key, destination })
    }

    pub(crate) fn restore(self) -> (ScannerFrameKey<G>, D) {
        (self.key, self.destination)
    }

    pub(crate) fn from_key(key: ScannerFrameKey<G>, destination: D) -> Self {
        Self { key, destination }
    }
}

impl<G, D: Copy> ChildContinuation<G, D> {
    pub(crate) fn destination(&self) -> D {
        self.destination
    }
}

// Keeping the heterogeneous payload inline is deliberate: boxing a live
// continuation would allocate on every first suspension after warmup.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ContinuationFrame<G> {
    Scanner(crate::scan_toks::PendingScanToks<G>),
    Scalar(crate::scanners::PendingScalarFrame<G>),
    Expansion(crate::ExpansionWorkKey<G>),
    ExpandAfter(crate::processor::expand_structural::PendingExpandAfter<G>),
    PdfStringCompare(crate::processor::expand_pdf_string::PendingPdfStringCompare<G>),
    AlignmentPreamble(crate::scanners::PendingAlignmentPreamble<G>),
    StructuredScanner(crate::scanners::PendingStructuredScanner<G>),
}

// `PendingScanToks` owns the definition builder and its attempt scope. Keep it
// in a dedicated typed lane so that growing that exact suspension owner cannot
// inflate every unrelated continuation row.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Eq, PartialEq)]
enum StoredContinuationFrame<G> {
    Scalar(crate::scanners::PendingScalarFrame<G>),
    Expansion(crate::ExpansionWorkKey<G>),
    ExpandAfter(crate::processor::expand_structural::PendingExpandAfter<G>),
    PdfStringCompare(crate::processor::expand_pdf_string::PendingPdfStringCompare<G>),
    AlignmentPreamble(crate::scanners::PendingAlignmentPreamble<G>),
    StructuredScanner(crate::scanners::PendingStructuredScanner<G>),
}

// Parked expansion controls belong in ExpansionWork's stable chunks. They
// must not enlarge this existing suspension lane into an 800-byte value.
const _: () = assert!(core::mem::size_of::<StoredContinuationFrame<()>>() < 800);

#[derive(Debug, Eq, PartialEq)]
struct ResumeFrameSlot<T, G> {
    serial: u64,
    payload: Option<T>,
    _generation: PhantomData<fn(&G) -> &G>,
}

impl<T, G> Default for ResumeFrameSlot<T, G> {
    fn default() -> Self {
        Self {
            serial: 0,
            payload: None,
            _generation: PhantomData,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct ResumeFrameLane<T, G> {
    slots: Vec<ResumeFrameSlot<T, G>>,
    free_slots: Vec<u32>,
    next_serial: u64,
}

impl<T, G> Default for ResumeFrameLane<T, G> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free_slots: Vec::new(),
            next_serial: 1,
        }
    }
}

impl<T, G> ResumeFrameLane<T, G> {
    fn insert(&mut self, payload: T) -> Result<ResumeFrameId<G>, ScratchError> {
        let mut payload = Some(payload);
        self.insert_from(&mut payload)
    }

    fn take(&mut self, id: ResumeFrameId<G>) -> Result<T, ScratchError> {
        let slot = self.slot_mut(id)?;
        let payload = slot.payload.take().ok_or(ScratchError::InvalidCoordinate)?;
        *slot = ResumeFrameSlot::default();
        self.free_slots.push(id.slot);
        Ok(payload)
    }

    fn get(&self, id: ResumeFrameId<G>) -> Result<&T, ScratchError> {
        self.slot(id)?
            .payload
            .as_ref()
            .ok_or(ScratchError::InvalidCoordinate)
    }

    fn get_mut(&mut self, id: ResumeFrameId<G>) -> Result<&mut T, ScratchError> {
        self.slot_mut(id)?
            .payload
            .as_mut()
            .ok_or(ScratchError::InvalidCoordinate)
    }

    /// Preflights every fallible coordinate and allocation before moving the
    /// payload out of `payload`. An error therefore leaves both the owner and
    /// the lane's reusable logical state unchanged.
    fn insert_from(&mut self, payload: &mut Option<T>) -> Result<ResumeFrameId<G>, ScratchError> {
        if payload.is_none() {
            return Err(ScratchError::InvalidCoordinate);
        }
        let serial = self.next_serial;
        let next_serial = serial
            .checked_add(1)
            .filter(|serial| *serial != 0)
            .ok_or(ScratchError::CapacityOverflow)?;
        let reused = self.free_slots.last().copied();
        let index = if let Some(index) = reused {
            let slot = self
                .slots
                .get(index as usize)
                .ok_or(ScratchError::InvalidCoordinate)?;
            if slot.payload.is_some() {
                return Err(ScratchError::InvalidCoordinate);
            }
            index
        } else {
            let index =
                u32::try_from(self.slots.len()).map_err(|_| ScratchError::CapacityOverflow)?;
            self.slots
                .try_reserve(1)
                .map_err(|_| ScratchError::AllocationFailed)?;
            self.free_slots
                .try_reserve(1)
                .map_err(|_| ScratchError::AllocationFailed)?;
            index
        };

        if reused.is_some() {
            let popped = self
                .free_slots
                .pop()
                .expect("preflighted reusable resume slot remains present");
            debug_assert_eq!(popped, index);
        } else {
            self.slots.push(ResumeFrameSlot::default());
        }
        self.next_serial = next_serial;
        let slot = &mut self.slots[index as usize];
        *slot = ResumeFrameSlot {
            serial,
            payload: payload.take(),
            _generation: PhantomData,
        };
        Ok(ResumeFrameId {
            slot: index,
            serial,
            _generation: PhantomData,
        })
    }

    fn slot(&self, id: ResumeFrameId<G>) -> Result<&ResumeFrameSlot<T, G>, ScratchError> {
        self.slots
            .get(id.slot as usize)
            .filter(|slot| slot.serial == id.serial && slot.payload.is_some())
            .ok_or(ScratchError::InvalidCoordinate)
    }

    fn slot_mut(
        &mut self,
        id: ResumeFrameId<G>,
    ) -> Result<&mut ResumeFrameSlot<T, G>, ScratchError> {
        self.slots
            .get_mut(id.slot as usize)
            .filter(|slot| slot.serial == id.serial && slot.payload.is_some())
            .ok_or(ScratchError::InvalidCoordinate)
    }

    fn live_len(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.payload.is_some())
            .count()
    }
}

#[derive(Debug)]
pub(crate) struct ArgumentSetId<G> {
    packed: NonZeroU64,
    _generation: PhantomData<fn(&G) -> &G>,
}

impl<G> ArgumentSetId<G> {
    fn new(slot: u32, serial: u64) -> Result<Self, ScratchError> {
        if u64::from(slot) > ARGUMENT_SET_SLOT_MASK
            || serial == 0
            || serial >= ARGUMENT_SET_SERIAL_LIMIT
        {
            return Err(ScratchError::CapacityOverflow);
        }
        let packed = (serial << ARGUMENT_SET_SLOT_BITS) | u64::from(slot);
        Ok(Self {
            packed: NonZeroU64::new(packed).ok_or(ScratchError::InvalidCoordinate)?,
            _generation: PhantomData,
        })
    }

    const fn slot(self) -> u32 {
        (self.packed.get() & ARGUMENT_SET_SLOT_MASK) as u32
    }

    const fn serial(self) -> u64 {
        self.packed.get() >> ARGUMENT_SET_SLOT_BITS
    }
}

impl<G> Copy for ArgumentSetId<G> {}
impl<G> Clone for ArgumentSetId<G> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<G> PartialEq for ArgumentSetId<G> {
    fn eq(&self, other: &Self) -> bool {
        self.packed == other.packed
    }
}
impl<G> Eq for ArgumentSetId<G> {}
impl<G> Hash for ArgumentSetId<G> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.packed.hash(state);
    }
}

#[derive(Debug)]
pub(crate) struct MacroArgumentRange<G> {
    frame: ArgumentSetId<G>,
    start: u32,
    end: u32,
}

impl<G> MacroArgumentRange<G> {
    pub(crate) const fn frame(self) -> ArgumentSetId<G> {
        self.frame
    }

    pub(crate) const fn len(self) -> u32 {
        self.end - self.start
    }

    pub(crate) const fn start(self) -> u32 {
        self.start
    }

    pub(crate) const fn end(self) -> u32 {
        self.end
    }
}

impl<G> Copy for MacroArgumentRange<G> {}
impl<G> Clone for MacroArgumentRange<G> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<G> PartialEq for MacroArgumentRange<G> {
    fn eq(&self, other: &Self) -> bool {
        self.frame == other.frame && self.start == other.start && self.end == other.end
    }
}
impl<G> Eq for MacroArgumentRange<G> {}
impl<G> Hash for MacroArgumentRange<G> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.frame.hash(state);
        self.start.hash(state);
        self.end.hash(state);
    }
}

/// Exact coordinate of the one synchronous, unpublished argument set.
///
/// Unlike the retired zero-sized `MacroMatch` marker, this capability rejects
/// a stale or foreign writer before it can touch the shared word lane. It is
/// consumed exactly once by commit or discard and never enters input state.
#[derive(Debug)]
pub(crate) struct PendingArgumentSet<G> {
    frame: ArgumentSetId<G>,
}

/// Exact TeX82 §394 facts established while one argument is collected.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct MacroArgumentFacts {
    rejects_non_long_paragraph: bool,
    removable_outer_group: bool,
}

impl MacroArgumentFacts {
    #[cfg(test)]
    pub(crate) const fn rejects_non_long_paragraph(self) -> bool {
        self.rejects_non_long_paragraph
    }

    pub(crate) const fn removable_outer_group(self) -> bool {
        self.removable_outer_group
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PendingArgumentFacts {
    rejects_non_long_paragraph: bool,
    word_count: u32,
    outer_group_candidate: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i8)]
enum ArgumentBraceDelta {
    Close = -1,
    Neither = 0,
    Open = 1,
}

const _: () = assert!(core::mem::size_of::<ArgumentBraceDelta>() == 1);

impl PendingArgumentFacts {
    fn settle(
        &mut self,
        token: ClassifiedToken,
        paragraph_checked: bool,
        brace_depth_before: u32,
    ) -> ArgumentBraceDelta {
        self.rejects_non_long_paragraph |= token.rejects_non_long_paragraph(paragraph_checked);
        let brace_delta = match token.spelling().literal_catcode() {
            Some(Catcode::BeginGroup) => ArgumentBraceDelta::Open,
            Some(Catcode::EndGroup) => ArgumentBraceDelta::Close,
            _ => ArgumentBraceDelta::Neither,
        };
        if self.word_count == 0 {
            self.outer_group_candidate = brace_delta == ArgumentBraceDelta::Open;
        } else if brace_depth_before == 0 {
            self.outer_group_candidate = false;
        }
        self.word_count = self.word_count.saturating_add(1);
        brace_delta
    }

    const fn seal(self, brace_depth: u32) -> MacroArgumentFacts {
        MacroArgumentFacts {
            rejects_non_long_paragraph: self.rejects_non_long_paragraph,
            removable_outer_group: self.outer_group_candidate && brace_depth == 0,
        }
    }
}

/// Purpose-built stack-local writer for one pending macro argument.
///
/// Range, brace, first-scan, trimming, and delimiter-prefix state live here
/// exactly once. The generic `TokenCollector` is reserved for scan_toks and
/// definition construction, whose phase and destination semantics differ.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct MacroArgumentWriter<G> {
    owner: ArgumentSetId<G>,
    slot: u8,
    start: u32,
    append: MacroAppendPosition,
    facts: PendingArgumentFacts,
    end_trim: u8,
    delimiter_start: usize,
    delimiter_head: usize,
    brace_depth: u32,
}

impl<G> MacroArgumentWriter<G> {
    pub(crate) const fn brace_depth(&self) -> u32 {
        self.brace_depth
    }

    pub(crate) const fn facts(&self) -> MacroArgumentFacts {
        self.facts.seal(self.brace_depth)
    }

    pub(crate) fn strip_outer_group(&mut self) -> Result<(), ScratchError> {
        let collected = self
            .append
            .absolute
            .checked_sub(self.start)
            .and_then(|len| len.checked_sub(u32::from(self.end_trim)))
            .ok_or(ScratchError::InvalidCoordinate)?;
        if collected < 2 {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.start = self
            .start
            .checked_add(1)
            .ok_or(ScratchError::CapacityOverflow)?;
        self.end_trim = self
            .end_trim
            .checked_add(1)
            .ok_or(ScratchError::CapacityOverflow)?;
        Ok(())
    }

    #[cfg(test)]
    fn set_end_trim(&mut self, trim: u8) {
        self.end_trim = trim;
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PackedRange {
    start: u32,
    len: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PackedArgument {
    range: PackedRange,
    facts: MacroArgumentFacts,
}

/// Admission-minted position in the stable fixed-block macro word lane.
///
/// The pending writer advances these scalars directly. Ordinary appends never
/// rediscover the tail block from the lane length or revalidate the macro
/// frame; only crossing a 4,096-word boundary acquires another fixed block.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct MacroAppendPosition {
    absolute: u32,
    chunk: usize,
    offset: usize,
    chunk_limit: usize,
    last_origin: Option<OriginId>,
}

#[derive(Debug, Default, Eq, PartialEq)]
struct MacroSlot {
    serial: u64,
    /// Absolute lane cursor at which this match began.
    lane_mark: u32,
    /// Earliest lane cursor this frame must restore when it retires. This can
    /// precede `lane_mark` when a parent retires beneath a pending child.
    reclaim_mark: u32,
    arguments: [PackedArgument; 9],
    argument_count: u8,
    current_argument: Option<u8>,
    parent_slot: u32,
    sealed: bool,
    live: bool,
    /// Nonzero while a synchronous command rollback point keeps this
    /// logically retired frame's storage available for exact restoration.
    transient_retired_at: u32,
    /// Depth which reused this previously free slot, plus its original link.
    /// These two scalars restore the free list without copying slot payloads.
    transient_reused_at: u32,
    transient_free_parent: u32,
}

impl MacroSlot {
    fn clear(&mut self) {
        self.serial = 0;
        self.lane_mark = 0;
        self.reclaim_mark = 0;
        self.argument_count = 0;
        self.current_argument = None;
        self.parent_slot = NO_MACRO_SLOT;
        self.sealed = false;
        self.live = false;
        self.transient_retired_at = 0;
        self.transient_reused_at = 0;
        self.transient_free_parent = NO_MACRO_SLOT;
    }
}

#[derive(Debug, Eq, PartialEq)]
struct MacroWordChunk {
    words: Box<[TokenWord; MACRO_WORD_RESERVE]>,
}

impl MacroWordChunk {
    fn new() -> Result<Self, ScratchError> {
        let empty = TokenWord::pack(tex_state::token::Token::Char {
            ch: '\0',
            cat: tex_state::token::Catcode::Other,
        });
        let mut words = Vec::new();
        words
            .try_reserve_exact(MACRO_WORD_RESERVE)
            .map_err(|_| ScratchError::AllocationFailed)?;
        words.resize(MACRO_WORD_RESERVE, empty);
        let words: Box<[TokenWord; MACRO_WORD_RESERVE]> = words
            .into_boxed_slice()
            .try_into()
            .map_err(|_| ScratchError::InvalidCoordinate)?;
        Ok(Self { words })
    }
}

/// One logically contiguous stable-address LIFO word lane.
///
/// Absolute indices map directly to a fixed chunk and offset. The outer
/// vectors may move their boxes, but an admitted word never moves. Truncation
/// returns only suffix chunks to the reusable spare stack.
#[derive(Debug, Default, Eq, PartialEq)]
struct MacroWordLane {
    active: Vec<MacroWordChunk>,
    spare: Vec<MacroWordChunk>,
    /// Exact provenance changes for the packed semantic lane. A run is
    /// appended only when the origin coordinate changes, so repeated-source
    /// tokens consume no side entry and no per-token provenance word.
    origins: Vec<MacroOriginRun>,
    len: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MacroOriginRun {
    start: u32,
    origin: OriginId,
}

impl MacroWordLane {
    fn len(&self) -> u32 {
        self.len
    }

    fn admit_append_position(&mut self) -> Result<MacroAppendPosition, ScratchError> {
        if self.len == u32::MAX {
            return Err(ScratchError::CapacityOverflow);
        }
        let absolute = self.len as usize;
        let chunk = absolute / MACRO_WORD_RESERVE;
        let offset = absolute % MACRO_WORD_RESERVE;
        if offset == 0 {
            // An empty argument can publish without consuming the destination
            // chunk admitted for its first word. The next argument owns the
            // same lane tail, so reuse that still-empty chunk instead of
            // attempting to admit a second physical owner for one coordinate.
            if chunk == self.active.len() {
                self.admit_chunk(chunk)?;
            } else if chunk.checked_add(1) != Some(self.active.len()) {
                return Err(ScratchError::InvalidCoordinate);
            }
        } else if chunk >= self.active.len() {
            return Err(ScratchError::InvalidCoordinate);
        }
        Ok(MacroAppendPosition {
            absolute: self.len,
            chunk,
            offset,
            chunk_limit: Self::chunk_limit(chunk),
            last_origin: self.origins.last().map(|run| run.origin),
        })
    }

    fn chunk_limit(chunk: usize) -> usize {
        let remaining = u32::MAX as usize - chunk * MACRO_WORD_RESERVE;
        remaining.min(MACRO_WORD_RESERVE)
    }

    fn admit_chunk(&mut self, chunk: usize) -> Result<(), ScratchError> {
        if chunk != self.active.len() {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.active
            .try_reserve(1)
            .map_err(|_| ScratchError::AllocationFailed)?;
        self.spare
            .try_reserve(1)
            .map_err(|_| ScratchError::AllocationFailed)?;
        let admitted = if let Some(chunk) = self.spare.pop() {
            chunk
        } else {
            MacroWordChunk::new()?
        };
        self.active.push(admitted);
        Ok(())
    }

    #[cold]
    fn advance_append_chunk(
        &mut self,
        position: &mut MacroAppendPosition,
    ) -> Result<(), ScratchError> {
        if position.absolute == u32::MAX {
            return Err(ScratchError::CapacityOverflow);
        }
        let chunk = position
            .chunk
            .checked_add(1)
            .ok_or(ScratchError::CapacityOverflow)?;
        self.admit_chunk(chunk)?;
        position.chunk = chunk;
        position.offset = 0;
        position.chunk_limit = Self::chunk_limit(chunk);
        Ok(())
    }

    #[cold]
    fn begin_origin_run(
        &mut self,
        position: &mut MacroAppendPosition,
        origin: OriginId,
    ) -> Result<(), ScratchError> {
        self.origins
            .try_reserve(1)
            .map_err(|_| ScratchError::AllocationFailed)?;
        self.origins.push(MacroOriginRun {
            start: position.absolute,
            origin,
        });
        position.last_origin = Some(origin);
        Ok(())
    }

    #[inline]
    fn append_at(
        &mut self,
        position: &mut MacroAppendPosition,
        word: TracedTokenWord,
    ) -> Result<(), ScratchError> {
        if position.offset == position.chunk_limit {
            self.advance_append_chunk(position)?;
        }
        let origin = word.origin();
        if position.last_origin != Some(origin) {
            self.begin_origin_run(position, origin)?;
        }
        let chunk = &mut self.active[position.chunk];
        chunk.words[position.offset] = word.token_word();
        position.offset += 1;
        position.absolute += 1;
        self.len = position.absolute;
        Ok(())
    }

    fn get(&self, index: u32) -> Option<TracedTokenWord> {
        if index >= self.len {
            return None;
        }
        let index = index as usize;
        let word = *self
            .active
            .get(index / MACRO_WORD_RESERVE)?
            .words
            .get(index % MACRO_WORD_RESERVE)?;
        let run = self
            .origins
            .partition_point(|run| run.start <= index as u32)
            .checked_sub(1)
            .and_then(|run| self.origins.get(run))?;
        Some(TracedTokenWord::from_parts(word, run.origin))
    }

    fn origin_run_at(&self, index: u32) -> Option<u32> {
        (index < self.len)
            .then(|| self.origins.partition_point(|run| run.start <= index))?
            .checked_sub(1)
            .and_then(|run| u32::try_from(run).ok())
    }

    /// Reads one admitted sequential word and its provenance without
    /// constructing the traced-token representation that resident delivery
    /// would immediately split again. Admission finds the opening run once;
    /// ordinary replay then performs one direct word lookup and at most one
    /// adjacent-run check. The admitted range proves `index < self.len`, and
    /// the cursor's stored run proves its start is not after `index`; repeating
    /// either comparison here would revalidate the same resident coordinate.
    #[inline(always)]
    fn get_sequential_parts(&self, index: u32, run: &mut u32) -> Option<(TokenWord, OriginId)> {
        let mut run_index = *run as usize;
        if self
            .origins
            .get(run_index + 1)
            .is_some_and(|next| next.start <= index)
        {
            run_index += 1;
            *run = u32::try_from(run_index).ok()?;
        }
        let origin = self.origins.get(run_index)?.origin;
        let index = index as usize;
        let word = *self
            .active
            .get(index / MACRO_WORD_RESERVE)?
            .words
            .get(index % MACRO_WORD_RESERVE)?;
        Some((word, origin))
    }

    /// Moves one unpublished pending-frame suffix into the dead prefix left by
    /// its last active ancestor. No admitted argument cursor can name the
    /// pending frame yet, so its compact ranges and provenance runs may move
    /// together before the frame is sealed.
    fn rebase_unpublished_suffix(
        &mut self,
        start: u32,
        destination: u32,
    ) -> Result<(u32, u32), ScratchError> {
        if destination > start || start > self.len {
            return Err(ScratchError::InvalidCoordinate);
        }
        let shift = start - destination;
        let suffix_len = self.len - start;
        if shift == 0 {
            return Ok((0, 0));
        }
        if suffix_len == 0 {
            self.truncate(destination)?;
            return Ok((shift, 0));
        }

        let first_run = self
            .origins
            .partition_point(|run| run.start <= start)
            .checked_sub(1)
            .ok_or(ScratchError::InvalidCoordinate)?;
        let first_origin = self.origins[first_run].origin;
        for offset in 0..suffix_len {
            let source = (start + offset) as usize;
            let word = *self
                .active
                .get(source / MACRO_WORD_RESERVE)
                .and_then(|chunk| chunk.words.get(source % MACRO_WORD_RESERVE))
                .ok_or(ScratchError::InvalidCoordinate)?;
            let target = (destination + offset) as usize;
            let target = self
                .active
                .get_mut(target / MACRO_WORD_RESERVE)
                .and_then(|chunk| chunk.words.get_mut(target % MACRO_WORD_RESERVE))
                .ok_or(ScratchError::InvalidCoordinate)?;
            *target = word;
        }

        let original_run_len = self.origins.len();
        let mut write = self.origins.partition_point(|run| run.start < destination);
        if write == 0 || self.origins[write - 1].origin != first_origin {
            let run = self
                .origins
                .get_mut(write)
                .ok_or(ScratchError::InvalidCoordinate)?;
            *run = MacroOriginRun {
                start: destination,
                origin: first_origin,
            };
            write += 1;
        }
        for read in first_run + 1..original_run_len {
            let mut run = self.origins[read];
            run.start = run
                .start
                .checked_sub(shift)
                .ok_or(ScratchError::InvalidCoordinate)?;
            self.origins[write] = run;
            write += 1;
        }
        self.origins.truncate(write);

        let end = destination
            .checked_add(suffix_len)
            .ok_or(ScratchError::CapacityOverflow)?;
        self.truncate(end)?;
        Ok((shift, suffix_len))
    }

    fn truncate(&mut self, mark: u32) -> Result<(), ScratchError> {
        if mark > self.len {
            return Err(ScratchError::InvalidCoordinate);
        }
        let needed = (mark as usize).div_ceil(MACRO_WORD_RESERVE);
        while self.active.len() > needed {
            let chunk = self.active.pop().expect("active suffix chunk");
            self.spare.push(chunk);
        }
        self.len = mark;
        while self.origins.last().is_some_and(|run| run.start >= mark) {
            self.origins.pop();
        }
        Ok(())
    }

    #[cfg(test)]
    fn retained_capacity(&self) -> usize {
        (self.active.len() + self.spare.len()) * MACRO_WORD_RESERVE
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScratchError {
    InvalidCoordinate,
    CapacityOverflow,
    AllocationFailed,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InjectedScannerFrameStoreFailure {
    Allocation,
    Capacity,
    Serial,
}

pub(crate) struct MacroWords<'a, G> {
    lane: &'a MacroWordLane,
    position: u32,
    end: u32,
    _generation: PhantomData<fn(&G) -> &G>,
}

impl<G> Clone for MacroWords<'_, G> {
    fn clone(&self) -> Self {
        Self {
            lane: self.lane,
            position: self.position,
            end: self.end,
            _generation: PhantomData,
        }
    }
}

impl<G> Iterator for MacroWords<'_, G> {
    type Item = TracedTokenWord;

    fn next(&mut self) -> Option<Self::Item> {
        if self.position >= self.end {
            return None;
        }
        let word = self.lane.get(self.position)?;
        self.position += 1;
        Some(word)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.end.saturating_sub(self.position) as usize;
        (len, Some(len))
    }
}
impl<G> ExactSizeIterator for MacroWords<'_, G> {}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ExecutionScratch<G> {
    macro_slots: Vec<MacroSlot>,
    macro_depth: u32,
    active_macro_slot: u32,
    pending_macro_slot: u32,
    free_macro_slot: u32,
    macro_words: MacroWordLane,
    next_macro_serial: u64,
    transient_depth: u32,
    delimiter_words: Vec<ClassifiedToken>,
    scanner_resumes: ResumeFrameLane<crate::scan_toks::PendingScanToks<G>, G>,
    continuation_resumes: ResumeFrameLane<StoredContinuationFrame<G>, G>,
    expression_frames: Vec<crate::scanners::ExpressionFrame<G>>,
    expansion_work: crate::expansion_work::ExpansionWork<G>,
    _generation: PhantomData<fn(&G) -> &G>,
    #[cfg(test)]
    physical_macro_word_copies: u64,
    #[cfg(test)]
    match_writer_admissions: u64,
    #[cfg(test)]
    match_writer_finalizations: u64,
    #[cfg(test)]
    match_writer_appends: u64,
    #[cfg(test)]
    match_writer_fact_updates: u64,
    #[cfg(test)]
    match_writer_slot_validations: u64,
    #[cfg(test)]
    fail_next_scanner_frame_store: Option<InjectedScannerFrameStoreFailure>,
    #[cfg(test)]
    inject_scan_toks_publication_collision: bool,
    /// Successful matching should append and classify, never read its stored
    /// words back for paragraph or outer-group decisions. Diagnostic tracing
    /// and observed token payloads are deliberate readers and remain visible
    /// in this assertion-bearing profiling counter.
    #[cfg(any(test, feature = "profiling"))]
    match_word_reads: Cell<u64>,
}

impl<G> Default for ExecutionScratch<G> {
    fn default() -> Self {
        Self {
            macro_slots: Vec::new(),
            macro_depth: 0,
            active_macro_slot: NO_MACRO_SLOT,
            pending_macro_slot: NO_MACRO_SLOT,
            free_macro_slot: NO_MACRO_SLOT,
            macro_words: MacroWordLane::default(),
            next_macro_serial: 1,
            transient_depth: 0,
            delimiter_words: Vec::new(),
            scanner_resumes: ResumeFrameLane::default(),
            continuation_resumes: ResumeFrameLane::default(),
            expression_frames: Vec::new(),
            expansion_work: crate::expansion_work::ExpansionWork::default(),
            _generation: PhantomData,
            #[cfg(test)]
            physical_macro_word_copies: 0,
            #[cfg(test)]
            match_writer_admissions: 0,
            #[cfg(test)]
            match_writer_finalizations: 0,
            #[cfg(test)]
            match_writer_appends: 0,
            #[cfg(test)]
            match_writer_fact_updates: 0,
            #[cfg(test)]
            match_writer_slot_validations: 0,
            #[cfg(test)]
            fail_next_scanner_frame_store: None,
            #[cfg(test)]
            inject_scan_toks_publication_collision: false,
            #[cfg(any(test, feature = "profiling"))]
            match_word_reads: Cell::new(0),
        }
    }
}

#[derive(Debug)]
pub(crate) struct ExecutionScratchTransientMark {
    depth: u32,
    macro_slots_len: usize,
    macro_words_len: u32,
    macro_depth: u32,
    active_macro_slot: u32,
    pending_macro_slot: u32,
    free_macro_slot: u32,
    next_macro_serial: u64,
    delimiter_words_len: usize,
}

impl<G> ExecutionScratch<G> {
    pub(crate) fn expression_stack_len(&self) -> usize {
        self.expression_frames.len()
    }

    pub(crate) fn push_expression_frame(
        &mut self,
        frame: crate::scanners::ExpressionFrame<G>,
    ) -> Result<(), ScratchError> {
        self.expression_frames
            .try_reserve(1)
            .map_err(|_| ScratchError::AllocationFailed)?;
        self.expression_frames.push(frame);
        Ok(())
    }

    pub(crate) fn pop_expression_frame(
        &mut self,
        mark: usize,
    ) -> Result<Option<crate::scanners::ExpressionFrame<G>>, ScratchError> {
        if self.expression_frames.len() < mark {
            return Err(ScratchError::InvalidCoordinate);
        }
        Ok((self.expression_frames.len() > mark)
            .then(|| self.expression_frames.pop())
            .flatten())
    }

    pub(crate) fn truncate_expression_stack(&mut self, mark: usize) -> Result<(), ScratchError> {
        if self.expression_frames.len() < mark {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.expression_frames.truncate(mark);
        Ok(())
    }

    pub(crate) fn take_continuation_frame(
        &mut self,
        key: ScannerFrameKey<G>,
    ) -> Result<ContinuationFrame<G>, ScratchError> {
        if key.is_scanner() {
            return self
                .scanner_resumes
                .take(key.id)
                .map(ContinuationFrame::Scanner);
        }
        let frame = self.continuation_resumes.take(key.id)?;
        let matches_kind = matches!(
            (&frame, key.kind),
            (StoredContinuationFrame::Scalar(_), ContinuationKind::Scalar)
                | (
                    StoredContinuationFrame::Expansion(_),
                    ContinuationKind::Expansion
                )
                | (
                    StoredContinuationFrame::ExpandAfter(_),
                    ContinuationKind::ExpandAfter
                )
                | (
                    StoredContinuationFrame::PdfStringCompare(_),
                    ContinuationKind::PdfStringCompare
                )
                | (
                    StoredContinuationFrame::AlignmentPreamble(_),
                    ContinuationKind::AlignmentPreamble
                )
                | (
                    StoredContinuationFrame::StructuredScanner(_),
                    ContinuationKind::StructuredScanner
                )
        );
        if !matches_kind {
            return Err(ScratchError::InvalidCoordinate);
        }
        Ok(match frame {
            StoredContinuationFrame::Scalar(pending) => ContinuationFrame::Scalar(pending),
            StoredContinuationFrame::Expansion(pending) => ContinuationFrame::Expansion(pending),
            StoredContinuationFrame::ExpandAfter(pending) => {
                ContinuationFrame::ExpandAfter(pending)
            }
            StoredContinuationFrame::PdfStringCompare(pending) => {
                ContinuationFrame::PdfStringCompare(pending)
            }
            StoredContinuationFrame::AlignmentPreamble(pending) => {
                ContinuationFrame::AlignmentPreamble(pending)
            }
            StoredContinuationFrame::StructuredScanner(pending) => {
                ContinuationFrame::StructuredScanner(pending)
            }
        })
    }

    pub(crate) fn store_scalar_frame(
        &mut self,
        pending: crate::scanners::PendingScalarFrame<G>,
    ) -> Result<ScannerFrameKey<G>, ScratchError> {
        self.continuation_resumes
            .insert(StoredContinuationFrame::Scalar(pending))
            .map(|id| ScannerFrameKey {
                id,
                kind: ContinuationKind::Scalar,
            })
    }

    pub(crate) fn take_scalar_frame(
        &mut self,
        key: ScannerFrameKey<G>,
    ) -> Result<crate::scanners::PendingScalarFrame<G>, ScratchError> {
        if !key.is_scalar() {
            return Err(ScratchError::InvalidCoordinate);
        }
        match self.continuation_resumes.take(key.id)? {
            StoredContinuationFrame::Scalar(pending) => Ok(pending),
            _ => Err(ScratchError::InvalidCoordinate),
        }
    }

    pub(crate) fn scalar_frame_mut(
        &mut self,
        key: &ScannerFrameKey<G>,
    ) -> Result<&mut crate::scanners::PendingScalarFrame<G>, ScratchError> {
        if !key.is_scalar() {
            return Err(ScratchError::InvalidCoordinate);
        }
        match self.continuation_resumes.slot_mut(key.id)?.payload.as_mut() {
            Some(StoredContinuationFrame::Scalar(pending)) => Ok(pending),
            _ => Err(ScratchError::InvalidCoordinate),
        }
    }

    pub(crate) fn discard_scalar_frame(
        &mut self,
        key: ScannerFrameKey<G>,
    ) -> Result<(), ScratchError> {
        self.take_scalar_frame(key).map(drop)
    }

    pub(crate) fn store_scanner_frame(
        &mut self,
        pending: &mut Option<crate::scan_toks::PendingScanToks<G>>,
    ) -> Result<ScannerFrameKey<G>, ScratchError> {
        #[cfg(test)]
        if let Some(failure) = self.fail_next_scanner_frame_store.take() {
            return Err(match failure {
                InjectedScannerFrameStoreFailure::Allocation => ScratchError::AllocationFailed,
                InjectedScannerFrameStoreFailure::Capacity
                | InjectedScannerFrameStoreFailure::Serial => ScratchError::CapacityOverflow,
            });
        }
        let id = self.scanner_resumes.insert_from(pending)?;
        Ok(ScannerFrameKey {
            id,
            kind: ContinuationKind::Scanner,
        })
    }

    #[cfg(test)]
    pub(crate) fn inject_scanner_frame_store_failure(
        &mut self,
        failure: InjectedScannerFrameStoreFailure,
    ) {
        self.fail_next_scanner_frame_store = Some(failure);
    }

    #[cfg(test)]
    pub(crate) fn inject_scan_toks_publication_collision(&mut self) {
        self.inject_scan_toks_publication_collision = true;
    }

    #[cfg(test)]
    pub(crate) fn take_scan_toks_publication_collision(&mut self) -> bool {
        core::mem::take(&mut self.inject_scan_toks_publication_collision)
    }

    #[cfg(test)]
    pub(crate) fn scanner_resume_storage_counts(&self) -> (usize, usize, usize) {
        (
            self.scanner_resumes.slots.len(),
            self.scanner_resumes.free_slots.len(),
            self.scanner_resumes.live_len(),
        )
    }

    pub(crate) fn take_scanner_frame(
        &mut self,
        key: ScannerFrameKey<G>,
    ) -> Result<crate::scan_toks::PendingScanToks<G>, ScratchError> {
        if !key.is_scanner() {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.scanner_resumes.take(key.id)
    }

    pub(crate) fn scanner_frame(
        &self,
        key: &ScannerFrameKey<G>,
    ) -> Result<&crate::scan_toks::PendingScanToks<G>, ScratchError> {
        if !key.is_scanner() {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.scanner_resumes.get(key.id)
    }

    // Failed wrapper admission returns the sole parked owner without boxing
    // or allocating on this already-cold recovery edge.
    #[allow(clippy::result_large_err)]
    pub(crate) fn store_expansion_frame(
        &mut self,
        pending: crate::state::PendingExpansion<G>,
    ) -> Result<ScannerFrameKey<G>, (ScratchError, crate::state::PendingExpansion<G>)> {
        let key = self.expansion_work.park_suspension(pending)?;
        let mut frame = Some(StoredContinuationFrame::Expansion(key));
        match self.continuation_resumes.insert_from(&mut frame) {
            Ok(id) => Ok(ScannerFrameKey {
                id,
                kind: ContinuationKind::Expansion,
            }),
            Err(error) => {
                let StoredContinuationFrame::Expansion(key) =
                    frame.expect("failed wrapper store preserves expansion key")
                else {
                    unreachable!("production wrapper contains one expansion key")
                };
                let pending = self
                    .expansion_work
                    .resume_suspension(key)
                    .expect("failed wrapper store restores just-parked expansion");
                Err((error, pending))
            }
        }
    }

    pub(crate) fn take_expansion_key(
        &mut self,
        key: ScannerFrameKey<G>,
    ) -> Result<crate::ExpansionWorkKey<G>, ScratchError> {
        if !key.is_expansion() {
            return Err(ScratchError::InvalidCoordinate);
        }
        match self.continuation_resumes.take(key.id)? {
            StoredContinuationFrame::Expansion(key) => Ok(key),
            _ => Err(ScratchError::InvalidCoordinate),
        }
    }

    pub(crate) fn resume_expansion(
        &mut self,
        key: crate::ExpansionWorkKey<G>,
    ) -> Result<crate::state::PendingExpansion<G>, ScratchError> {
        self.expansion_work.resume_suspension(key)
    }

    pub(crate) fn cancel_expansion(
        &mut self,
        key: crate::ExpansionWorkKey<G>,
    ) -> Result<crate::state::PendingExpansion<G>, ScratchError> {
        self.expansion_work.cancel_suspension(key)
    }

    pub(crate) fn store_expandafter_frame(
        &mut self,
        pending: crate::processor::expand_structural::PendingExpandAfter<G>,
    ) -> Result<ScannerFrameKey<G>, ScratchError> {
        self.continuation_resumes
            .insert(StoredContinuationFrame::ExpandAfter(pending))
            .map(|id| ScannerFrameKey {
                id,
                kind: ContinuationKind::ExpandAfter,
            })
    }

    pub(crate) fn take_expandafter_frame(
        &mut self,
        key: ScannerFrameKey<G>,
    ) -> Result<crate::processor::expand_structural::PendingExpandAfter<G>, ScratchError> {
        if !key.is_expandafter() {
            return Err(ScratchError::InvalidCoordinate);
        }
        match self.continuation_resumes.take(key.id)? {
            StoredContinuationFrame::ExpandAfter(pending) => Ok(pending),
            _ => Err(ScratchError::InvalidCoordinate),
        }
    }

    pub(crate) fn store_pdf_string_compare_frame(
        &mut self,
        pending: crate::processor::expand_pdf_string::PendingPdfStringCompare<G>,
    ) -> Result<ScannerFrameKey<G>, ScratchError> {
        self.continuation_resumes
            .insert(StoredContinuationFrame::PdfStringCompare(pending))
            .map(|id| ScannerFrameKey {
                id,
                kind: ContinuationKind::PdfStringCompare,
            })
    }

    pub(crate) fn take_pdf_string_compare_frame(
        &mut self,
        key: ScannerFrameKey<G>,
    ) -> Result<crate::processor::expand_pdf_string::PendingPdfStringCompare<G>, ScratchError> {
        if !key.is_pdf_string_compare() {
            return Err(ScratchError::InvalidCoordinate);
        }
        match self.continuation_resumes.take(key.id)? {
            StoredContinuationFrame::PdfStringCompare(pending) => Ok(pending),
            _ => Err(ScratchError::InvalidCoordinate),
        }
    }

    pub(crate) fn store_alignment_preamble_frame(
        &mut self,
        pending: crate::scanners::PendingAlignmentPreamble<G>,
    ) -> Result<ScannerFrameKey<G>, ScratchError> {
        self.continuation_resumes
            .insert(StoredContinuationFrame::AlignmentPreamble(pending))
            .map(|id| ScannerFrameKey {
                id,
                kind: ContinuationKind::AlignmentPreamble,
            })
    }

    pub(crate) fn take_alignment_preamble_frame(
        &mut self,
        key: ScannerFrameKey<G>,
    ) -> Result<crate::scanners::PendingAlignmentPreamble<G>, ScratchError> {
        if !key.is_alignment_preamble() {
            return Err(ScratchError::InvalidCoordinate);
        }
        match self.continuation_resumes.take(key.id)? {
            StoredContinuationFrame::AlignmentPreamble(pending) => Ok(pending),
            _ => Err(ScratchError::InvalidCoordinate),
        }
    }

    pub(crate) fn alignment_preamble_frame_mut(
        &mut self,
        key: &ScannerFrameKey<G>,
    ) -> Result<&mut crate::scanners::PendingAlignmentPreamble<G>, ScratchError> {
        if !key.is_alignment_preamble() {
            return Err(ScratchError::InvalidCoordinate);
        }
        match self.continuation_resumes.get_mut(key.id)? {
            StoredContinuationFrame::AlignmentPreamble(pending) => Ok(pending),
            _ => Err(ScratchError::InvalidCoordinate),
        }
    }

    pub(crate) fn discard_alignment_preamble_frame(
        &mut self,
        key: ScannerFrameKey<G>,
    ) -> Result<(), ScratchError> {
        self.take_alignment_preamble_frame(key).map(drop)
    }

    pub(crate) fn store_structured_scanner_frame(
        &mut self,
        pending: crate::scanners::PendingStructuredScanner<G>,
    ) -> Result<ScannerFrameKey<G>, ScratchError> {
        self.continuation_resumes
            .insert(StoredContinuationFrame::StructuredScanner(pending))
            .map(|id| ScannerFrameKey {
                id,
                kind: ContinuationKind::StructuredScanner,
            })
    }

    pub(crate) fn take_structured_scanner_frame(
        &mut self,
        key: ScannerFrameKey<G>,
    ) -> Result<crate::scanners::PendingStructuredScanner<G>, ScratchError> {
        if !key.is_structured_scanner() {
            return Err(ScratchError::InvalidCoordinate);
        }
        match self.continuation_resumes.take(key.id)? {
            StoredContinuationFrame::StructuredScanner(pending) => Ok(pending),
            _ => Err(ScratchError::InvalidCoordinate),
        }
    }

    pub(crate) fn structured_scanner_frame_mut(
        &mut self,
        key: &ScannerFrameKey<G>,
    ) -> Result<&mut crate::scanners::PendingStructuredScanner<G>, ScratchError> {
        if !key.is_structured_scanner() {
            return Err(ScratchError::InvalidCoordinate);
        }
        match self.continuation_resumes.get_mut(key.id)? {
            StoredContinuationFrame::StructuredScanner(pending) => Ok(pending),
            _ => Err(ScratchError::InvalidCoordinate),
        }
    }

    pub(crate) fn discard_structured_scanner_frame(
        &mut self,
        key: ScannerFrameKey<G>,
    ) -> Result<(), ScratchError> {
        self.take_structured_scanner_frame(key).map(drop)
    }

    pub(crate) fn begin_macro_match(&mut self) -> Result<PendingArgumentSet<G>, ScratchError> {
        if !self.delimiter_words.is_empty() || self.pending_slot().is_ok() {
            return Err(ScratchError::InvalidCoordinate);
        }
        let mut reused_free_parent = None;
        let slot_index = if self.free_macro_slot == NO_MACRO_SLOT {
            let index = self.macro_slots.len();
            let packed_index = u32::try_from(index).map_err(|_| ScratchError::CapacityOverflow)?;
            // Reject an unrepresentable slot before reserving storage or
            // changing any pending/free-list state. `commit_macro_match` can
            // consequently publish every admitted pending slot atomically.
            ArgumentSetId::<G>::new(packed_index, self.next_macro_serial)?;
            index
        } else {
            let index = self.free_macro_slot as usize;
            let slot = self
                .macro_slots
                .get(index)
                .filter(|slot| !slot.live)
                .ok_or(ScratchError::InvalidCoordinate)?;
            ArgumentSetId::<G>::new(self.free_macro_slot, self.next_macro_serial)?;
            reused_free_parent = Some(slot.parent_slot);
            self.free_macro_slot = slot.parent_slot;
            index
        };
        if slot_index == self.macro_slots.len() {
            self.macro_slots
                .try_reserve(1)
                .map_err(|_| ScratchError::AllocationFailed)?;
            self.macro_slots.push(MacroSlot::default());
        } else if self.macro_slots[slot_index].live {
            return Err(ScratchError::InvalidCoordinate);
        }
        let serial = self.next_macro_serial;
        self.next_macro_serial = if serial + 1 == ARGUMENT_SET_SERIAL_LIMIT {
            1
        } else {
            serial + 1
        };
        let lane_mark = self.macro_words.len();
        let slot = &mut self.macro_slots[slot_index];
        slot.serial = serial;
        slot.lane_mark = lane_mark;
        slot.reclaim_mark = lane_mark;
        slot.argument_count = 0;
        slot.current_argument = None;
        slot.parent_slot = NO_MACRO_SLOT;
        slot.sealed = false;
        slot.live = true;
        slot.transient_retired_at = 0;
        slot.transient_reused_at = reused_free_parent
            .map(|_| self.transient_depth)
            .unwrap_or(0);
        slot.transient_free_parent = reused_free_parent.unwrap_or(NO_MACRO_SLOT);
        self.pending_macro_slot =
            u32::try_from(slot_index).expect("macro slot representability was preflighted");
        Ok(PendingArgumentSet {
            frame: ArgumentSetId::new(
                u32::try_from(slot_index).expect("macro slot was preflighted"),
                serial,
            )?,
        })
    }

    pub(crate) fn begin_argument_writer(
        &mut self,
        matching: &PendingArgumentSet<G>,
    ) -> Result<MacroArgumentWriter<G>, ScratchError> {
        let start = self.macro_words.len();
        let delimiter_start = self.delimiter_words.len();
        let slot_index = matching.frame.slot() as usize;
        let slot = self.pending_slot_for(matching.frame)?;
        if slot.current_argument.is_some() || slot.argument_count >= 9 {
            return Err(ScratchError::InvalidCoordinate);
        }
        let argument_slot = slot.argument_count;
        let append = self.macro_words.admit_append_position()?;
        let slot = &mut self.macro_slots[slot_index];
        slot.current_argument = Some(argument_slot);
        #[cfg(test)]
        {
            self.match_writer_admissions = self.match_writer_admissions.saturating_add(1);
            self.match_writer_slot_validations =
                self.match_writer_slot_validations.saturating_add(1);
        }
        Ok(MacroArgumentWriter {
            owner: matching.frame,
            slot: argument_slot,
            start,
            append,
            facts: PendingArgumentFacts::default(),
            end_trim: 0,
            delimiter_start,
            delimiter_head: delimiter_start,
            brace_depth: 0,
        })
    }

    /// Settles one classified token through a writer checked once at admission.
    ///
    /// This is the sole accepted-token transition: the lane append returns its
    /// authoritative new cursor, then the resident writer updates paragraph,
    /// brace-depth, and removable-outer-group facts from the same classification.
    /// Frame publication validates once when the argument finishes.
    #[inline]
    pub(crate) fn append_argument_token(
        &mut self,
        writer: &mut MacroArgumentWriter<G>,
        token: ClassifiedToken,
        paragraph_checked: bool,
    ) -> Result<u32, ScratchError> {
        self.macro_words
            .append_at(&mut writer.append, token.word())?;
        match writer
            .facts
            .settle(token, paragraph_checked, writer.brace_depth)
        {
            ArgumentBraceDelta::Open => {
                writer.brace_depth = writer.brace_depth.saturating_add(1);
            }
            ArgumentBraceDelta::Close => {
                writer.brace_depth = writer.brace_depth.saturating_sub(1);
            }
            ArgumentBraceDelta::Neither => {}
        }
        #[cfg(test)]
        {
            self.match_writer_appends = self.match_writer_appends.saturating_add(1);
            self.match_writer_fact_updates = self.match_writer_fact_updates.saturating_add(1);
        }
        Ok(writer.brace_depth)
    }

    pub(crate) fn match_words(
        &self,
        writer: &MacroArgumentWriter<G>,
    ) -> Result<MacroWords<'_, G>, ScratchError> {
        let visible_end = writer
            .append
            .absolute
            .checked_sub(u32::from(writer.end_trim))
            .ok_or(ScratchError::InvalidCoordinate)?;
        if visible_end < writer.start || writer.append.absolute > self.macro_words.len() {
            return Err(ScratchError::InvalidCoordinate);
        }
        Ok(MacroWords {
            lane: &self.macro_words,
            position: writer.start,
            end: visible_end,
            _generation: PhantomData,
        })
    }

    pub(crate) fn publish_argument(
        &mut self,
        mut writer: MacroArgumentWriter<G>,
    ) -> Result<(), ScratchError> {
        let slot_index = writer.owner.slot() as usize;
        let slot = self.pending_slot_for(writer.owner)?;
        if slot.current_argument != Some(writer.slot) || slot.argument_count != writer.slot {
            return Err(ScratchError::InvalidCoordinate);
        }
        if writer.append.absolute != self.macro_words.len() {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.clear_delimiter_prefix(&mut writer)?;
        let range_len = writer
            .append
            .absolute
            .checked_sub(writer.start)
            .and_then(|len| len.checked_sub(u32::from(writer.end_trim)))
            .ok_or(ScratchError::InvalidCoordinate)?;
        let settled_facts = writer.facts.seal(writer.brace_depth);
        let slot = &mut self.macro_slots[slot_index];
        slot.arguments[usize::from(writer.slot)] = PackedArgument {
            range: PackedRange {
                start: writer.start,
                len: range_len,
            },
            facts: settled_facts,
        };
        slot.current_argument = None;
        slot.argument_count += 1;
        #[cfg(test)]
        {
            self.match_writer_finalizations = self.match_writer_finalizations.saturating_add(1);
            self.match_writer_slot_validations =
                self.match_writer_slot_validations.saturating_add(1);
        }
        Ok(())
    }

    pub(crate) fn clear_delimiter_prefix(
        &mut self,
        writer: &mut MacroArgumentWriter<G>,
    ) -> Result<(), ScratchError> {
        if writer.delimiter_start > self.delimiter_words.len() {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.delimiter_words.truncate(writer.delimiter_start);
        writer.delimiter_head = writer.delimiter_start;
        Ok(())
    }

    pub(crate) fn delimiter_prefix_len(
        &self,
        writer: &MacroArgumentWriter<G>,
    ) -> Result<usize, ScratchError> {
        Ok(self
            .delimiter_words
            .len()
            .saturating_sub(writer.delimiter_head))
    }

    pub(crate) fn delimiter_prefix_is_empty(
        &self,
        writer: &MacroArgumentWriter<G>,
    ) -> Result<bool, ScratchError> {
        Ok(self.delimiter_prefix_len(writer)? == 0)
    }

    pub(crate) fn delimiter_prefix_word(
        &self,
        writer: &MacroArgumentWriter<G>,
        index: usize,
    ) -> Result<ClassifiedToken, ScratchError> {
        self.delimiter_words
            .get(
                writer
                    .delimiter_head
                    .checked_add(index)
                    .ok_or(ScratchError::CapacityOverflow)?,
            )
            .copied()
            .ok_or(ScratchError::InvalidCoordinate)
    }

    pub(crate) fn delimiter_prefix_words<'a>(
        &'a self,
        writer: &'a MacroArgumentWriter<G>,
    ) -> Result<impl Iterator<Item = TracedTokenWord> + 'a, ScratchError> {
        let len = self.delimiter_prefix_len(writer)?;
        Ok((0..len).map(|index| {
            self.delimiter_prefix_word(writer, index)
                .expect("live delimiter prefix")
                .word()
        }))
    }

    pub(crate) fn push_delimiter_prefix(
        &mut self,
        writer: &MacroArgumentWriter<G>,
        token: ClassifiedToken,
    ) -> Result<(), ScratchError> {
        if writer.delimiter_start > self.delimiter_words.len() {
            return Err(ScratchError::InvalidCoordinate);
        }
        if self.delimiter_words.len() == self.delimiter_words.capacity() {
            self.delimiter_words
                .try_reserve_exact(MACRO_WORD_RESERVE)
                .map_err(|_| ScratchError::AllocationFailed)?;
        }
        self.delimiter_words.push(token);
        Ok(())
    }

    pub(crate) fn pop_delimiter_prefix_word(
        &mut self,
        writer: &mut MacroArgumentWriter<G>,
    ) -> Result<ClassifiedToken, ScratchError> {
        let token = self
            .delimiter_words
            .get(writer.delimiter_head)
            .copied()
            .ok_or(ScratchError::InvalidCoordinate)?;
        writer.delimiter_head = writer
            .delimiter_head
            .checked_add(1)
            .ok_or(ScratchError::CapacityOverflow)?;
        if writer.delimiter_head == self.delimiter_words.len() {
            self.delimiter_words.truncate(writer.delimiter_start);
            writer.delimiter_head = writer.delimiter_start;
        }
        Ok(token)
    }

    pub(crate) fn commit_macro_match(
        &mut self,
        matching: PendingArgumentSet<G>,
    ) -> Result<ArgumentSetId<G>, ScratchError> {
        if !self.delimiter_words.is_empty() {
            return Err(ScratchError::InvalidCoordinate);
        }
        let slot_index = self.pending_slot_index()? as u32;
        if matching.frame.slot() != slot_index {
            return Err(ScratchError::InvalidCoordinate);
        }
        let slot = self.pending_slot_for(matching.frame)?;
        if slot.current_argument.is_some() {
            return Err(ScratchError::InvalidCoordinate);
        }
        let next_depth = self
            .macro_depth
            .checked_add(1)
            .ok_or(ScratchError::CapacityOverflow)?;
        // Construct the externally visible coordinate before changing the
        // pending frame's role or the live-depth scalars. Even deliberately
        // corrupted/capacity-exhausted state therefore fails atomically.
        let frame = matching.frame;
        let slot = &mut self.macro_slots[slot_index as usize];
        slot.parent_slot = self.active_macro_slot;
        slot.sealed = true;
        self.active_macro_slot = slot_index;
        self.pending_macro_slot = NO_MACRO_SLOT;
        self.macro_depth = next_depth;
        Ok(frame)
    }

    pub(crate) fn discard_macro_match(
        &mut self,
        matching: PendingArgumentSet<G>,
    ) -> Result<(), ScratchError> {
        self.pending_slot_for(matching.frame)?;
        let slot_index = self.pending_slot_index()?;
        let reclaim_mark = self.macro_slots[slot_index].reclaim_mark;
        self.pending_macro_slot = NO_MACRO_SLOT;
        self.truncate_macro_words(reclaim_mark)?;
        self.release_macro_slot(slot_index as u32);
        self.delimiter_words.clear();
        Ok(())
    }

    pub(crate) fn release_argument_set(
        &mut self,
        frame: ArgumentSetId<G>,
    ) -> Result<(), ScratchError> {
        self.release_slot(frame)
    }

    pub(crate) fn argument_count(&self, frame: ArgumentSetId<G>) -> Result<usize, ScratchError> {
        Ok(self.sealed_slot(frame)?.argument_count as usize)
    }

    pub(crate) fn argument_range(
        &self,
        frame: ArgumentSetId<G>,
        slot: u8,
    ) -> Result<Option<MacroArgumentRange<G>>, ScratchError> {
        if !(1..=9).contains(&slot) {
            return Err(ScratchError::InvalidCoordinate);
        }
        let owner = self.sealed_slot(frame)?;
        if slot > owner.argument_count {
            return Ok(None);
        }
        let range = owner.arguments[usize::from(slot - 1)].range;
        let end = range
            .start
            .checked_add(range.len)
            .filter(|end| *end <= self.macro_words.len())
            .ok_or(ScratchError::InvalidCoordinate)?;
        if range.start < owner.lane_mark {
            return Err(ScratchError::InvalidCoordinate);
        }
        Ok(Some(MacroArgumentRange {
            frame,
            start: range.start,
            end,
        }))
    }

    #[cfg(test)]
    pub(crate) fn argument_facts(
        &self,
        range: MacroArgumentRange<G>,
    ) -> Result<MacroArgumentFacts, ScratchError> {
        Ok(self.sealed_argument(range)?.facts)
    }

    #[cfg(test)]
    pub(crate) fn argument_len(&self, range: MacroArgumentRange<G>) -> Result<usize, ScratchError> {
        self.validate_argument_range(range)?;
        Ok(range.end.saturating_sub(range.start) as usize)
    }

    #[cfg(test)]
    pub(crate) fn argument_word(
        &self,
        range: MacroArgumentRange<G>,
        index: usize,
    ) -> Result<TracedTokenWord, ScratchError> {
        self.argument_word_ref(range, index)
    }

    #[cfg(test)]
    pub(crate) fn argument_word_ref(
        &self,
        range: MacroArgumentRange<G>,
        index: usize,
    ) -> Result<TracedTokenWord, ScratchError> {
        self.validate_argument_range(range)?;
        let absolute = range
            .start
            .checked_add(u32::try_from(index).map_err(|_| ScratchError::CapacityOverflow)?)
            .ok_or(ScratchError::CapacityOverflow)?;
        if absolute >= range.end {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.macro_words
            .get(absolute)
            .ok_or(ScratchError::InvalidCoordinate)
    }

    /// Reads through a range which was fully validated when its input cursor
    /// was admitted.
    ///
    /// A cursor is always above the macro body which owns its frame, so that
    /// frame cannot retire or reuse its lane suffix before the cursor itself
    /// retires. Candidate rollback restores the input and scratch roots as one
    /// aggregate. Those ownership rules make the private admitted range the
    /// capability: replay checks its scalar half-open bounds and performs one
    /// direct chunk lookup, without repeating either the nine-entry range
    /// search or the activation-serial lookup for every word.
    pub(crate) fn admitted_argument_word(
        &self,
        range: MacroArgumentRange<G>,
        index: usize,
    ) -> Result<TracedTokenWord, ScratchError> {
        let absolute = range
            .start
            .checked_add(u32::try_from(index).map_err(|_| ScratchError::CapacityOverflow)?)
            .ok_or(ScratchError::CapacityOverflow)?;
        if absolute >= range.end {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.macro_words
            .get(absolute)
            .ok_or(ScratchError::InvalidCoordinate)
    }

    #[cfg(any(test, feature = "profiling"))]
    pub(crate) fn admitted_argument_word_at(
        &self,
        range: MacroArgumentRange<G>,
        absolute: u32,
    ) -> Result<TracedTokenWord, ScratchError> {
        if absolute < range.start || absolute >= range.end {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.macro_words
            .get(absolute)
            .ok_or(ScratchError::InvalidCoordinate)
    }

    pub(crate) fn admitted_argument_origin_run(
        &self,
        range: MacroArgumentRange<G>,
    ) -> Result<u32, ScratchError> {
        self.validate_admitted_argument_range(range)?;
        if range.start == range.end {
            return Ok(0);
        }
        self.macro_words
            .origin_run_at(range.start)
            .ok_or(ScratchError::InvalidCoordinate)
    }

    /// Reads through the cursor-owned admitted range.
    ///
    /// [`crate::input::MacroArgumentCursor`] owns and checks the half-open
    /// argument end before calling this method. Its absolute coordinate was
    /// validated at admission and can only advance, so repeating both range
    /// comparisons here would maintain a second per-word bounds pass.
    #[inline(always)]
    pub(crate) fn admitted_argument_parts_at_sequential(
        &self,
        absolute: u32,
        origin_run: &mut u32,
    ) -> Option<(TokenWord, OriginId)> {
        self.macro_words.get_sequential_parts(absolute, origin_run)
    }

    fn validate_admitted_argument_range(
        &self,
        range: MacroArgumentRange<G>,
    ) -> Result<(), ScratchError> {
        if range.start <= range.end && range.end <= self.macro_words.len() {
            Ok(())
        } else {
            Err(ScratchError::InvalidCoordinate)
        }
    }

    pub(crate) fn argument_word_len(&self) -> usize {
        self.macro_words.len() as usize
    }

    pub(crate) fn frame_len(&self) -> usize {
        self.macro_depth as usize
    }

    pub(crate) fn active_argument_set(&self) -> Option<ArgumentSetId<G>> {
        let slot = self.macro_slots.get(self.active_macro_slot as usize)?;
        (slot.live && slot.sealed)
            .then(|| ArgumentSetId::new(self.active_macro_slot, slot.serial).ok())
            .flatten()
    }

    pub(crate) fn begin_transient(&mut self) -> ExecutionScratchTransientMark {
        self.transient_depth = self
            .transient_depth
            .checked_add(1)
            .expect("nested scratch rollback depth is bounded");
        ExecutionScratchTransientMark {
            depth: self.transient_depth,
            macro_slots_len: self.macro_slots.len(),
            macro_words_len: self.macro_words.len(),
            macro_depth: self.macro_depth,
            active_macro_slot: self.active_macro_slot,
            pending_macro_slot: self.pending_macro_slot,
            free_macro_slot: self.free_macro_slot,
            next_macro_serial: self.next_macro_serial,
            delimiter_words_len: self.delimiter_words.len(),
        }
    }

    pub(crate) fn rollback_transient(
        &mut self,
        mark: ExecutionScratchTransientMark,
    ) -> Result<(), ScratchError> {
        if mark.depth != self.transient_depth
            || mark.macro_slots_len > self.macro_slots.len()
            || mark.macro_words_len > self.macro_words.len()
            || mark.delimiter_words_len > self.delimiter_words.len()
        {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.truncate_macro_words(mark.macro_words_len)?;
        for slot in &mut self.macro_slots[..mark.macro_slots_len] {
            if slot.transient_reused_at >= mark.depth {
                let parent = slot.transient_free_parent;
                slot.clear();
                slot.parent_slot = parent;
            }
            if slot.transient_retired_at >= mark.depth {
                slot.transient_retired_at = 0;
            }
        }
        self.macro_slots.truncate(mark.macro_slots_len);
        self.macro_depth = mark.macro_depth;
        self.active_macro_slot = mark.active_macro_slot;
        self.pending_macro_slot = mark.pending_macro_slot;
        self.free_macro_slot = mark.free_macro_slot;
        self.next_macro_serial = mark.next_macro_serial;
        self.delimiter_words.truncate(mark.delimiter_words_len);
        self.transient_depth -= 1;
        Ok(())
    }

    pub(crate) fn commit_transient(
        &mut self,
        mark: ExecutionScratchTransientMark,
    ) -> Result<(), ScratchError> {
        if mark.depth != self.transient_depth {
            return Err(ScratchError::InvalidCoordinate);
        }
        if mark.depth > 1 {
            for slot in &mut self.macro_slots {
                if slot.transient_retired_at == mark.depth {
                    slot.transient_retired_at -= 1;
                }
                if slot.transient_reused_at == mark.depth {
                    slot.transient_reused_at -= 1;
                }
            }
            self.transient_depth -= 1;
            return Ok(());
        }

        let reclaim_mark = self
            .macro_slots
            .iter()
            .filter(|slot| slot.transient_retired_at != 0)
            .map(|slot| slot.reclaim_mark)
            .min();
        self.transient_depth = 0;
        for index in 0..self.macro_slots.len() {
            if self.macro_slots[index].transient_retired_at != 0 {
                self.release_macro_slot(index as u32);
            } else {
                self.macro_slots[index].transient_reused_at = 0;
                self.macro_slots[index].transient_free_parent = NO_MACRO_SLOT;
            }
        }
        if let Some(reclaim_mark) = reclaim_mark
            && self
                .macro_slots
                .iter()
                .filter(|slot| slot.live)
                .all(|slot| slot.lane_mark < reclaim_mark)
        {
            self.truncate_macro_words(reclaim_mark)?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn retained_slot_len(&self) -> usize {
        self.macro_slots.len()
    }

    #[cfg(test)]
    pub(crate) fn retained_word_capacity(&self) -> usize {
        self.macro_words.retained_capacity()
    }

    pub(crate) fn is_quiescent(&self) -> bool {
        self.frame_len() == 0
            && self.pending_slot().is_err()
            && self.delimiter_words.is_empty()
            && self.scanner_resumes.live_len() == 0
            && self.continuation_resumes.live_len() == 0
            && self.expansion_work.is_quiescent()
    }

    #[cfg(test)]
    pub(crate) const fn physical_macro_word_copies(&self) -> u64 {
        self.physical_macro_word_copies
    }

    /// Admission, accepted-word, fused-fact, publication, and slot-validation
    /// counts for the direct macro argument writer.
    #[cfg(test)]
    pub(crate) const fn match_writer_operations(&self) -> (u64, u64, u64, u64, u64) {
        (
            self.match_writer_admissions,
            self.match_writer_appends,
            self.match_writer_fact_updates,
            self.match_writer_finalizations,
            self.match_writer_slot_validations,
        )
    }

    #[cfg(any(test, feature = "profiling"))]
    pub(crate) fn match_word_reads(&self) -> u64 {
        self.match_word_reads.get()
    }

    #[cfg(test)]
    fn sealed_argument(
        &self,
        range: MacroArgumentRange<G>,
    ) -> Result<&PackedArgument, ScratchError> {
        let slot = self.sealed_slot(range.frame)?;
        slot.arguments[..usize::from(slot.argument_count)]
            .iter()
            .find(|argument| {
                argument.range.start == range.start
                    && argument.range.start.checked_add(argument.range.len) == Some(range.end)
            })
            .ok_or(ScratchError::InvalidCoordinate)
    }

    fn pending_slot(&self) -> Result<&MacroSlot, ScratchError> {
        let index = self.pending_slot_index()?;
        self.macro_slots
            .get(index)
            .ok_or(ScratchError::InvalidCoordinate)
    }

    fn pending_slot_for(&self, frame: ArgumentSetId<G>) -> Result<&MacroSlot, ScratchError> {
        let index = self.pending_slot_index()?;
        let slot = &self.macro_slots[index];
        (frame.slot() as usize == index
            && slot.live
            && !slot.sealed
            && slot.serial == frame.serial())
        .then_some(slot)
        .ok_or(ScratchError::InvalidCoordinate)
    }

    fn pending_slot_index(&self) -> Result<usize, ScratchError> {
        if self.pending_macro_slot == NO_MACRO_SLOT {
            return Err(ScratchError::InvalidCoordinate);
        }
        let index = self.pending_macro_slot as usize;
        self.macro_slots
            .get(index)
            .filter(|slot| slot.live && !slot.sealed)
            .map(|_| index)
            .ok_or(ScratchError::InvalidCoordinate)
    }

    fn sealed_slot(&self, frame: ArgumentSetId<G>) -> Result<&MacroSlot, ScratchError> {
        self.slot(frame)
    }

    fn slot(&self, frame: ArgumentSetId<G>) -> Result<&MacroSlot, ScratchError> {
        self.macro_slots
            .get(frame.slot() as usize)
            .filter(|slot| slot.live && slot.sealed && slot.serial == frame.serial())
            .ok_or(ScratchError::InvalidCoordinate)
    }

    fn release_slot(&mut self, frame: ArgumentSetId<G>) -> Result<(), ScratchError> {
        if self.macro_depth == 0 || frame.slot() != self.active_macro_slot {
            return Err(ScratchError::InvalidCoordinate);
        }
        let slot = self.slot(frame)?;
        let parent_slot = slot.parent_slot;
        let reclaim_mark = slot.reclaim_mark;
        let pending_slot = self.pending_macro_slot;
        if pending_slot != NO_MACRO_SLOT {
            let pending = &mut self.macro_slots[pending_slot as usize];
            if pending.reclaim_mark > reclaim_mark {
                pending.reclaim_mark = reclaim_mark;
            }
        } else if self.transient_depth == 0 {
            self.truncate_macro_words(reclaim_mark)?;
        }
        if self.transient_depth != 0 {
            self.macro_slots[frame.slot() as usize].transient_retired_at = self.transient_depth;
            self.macro_depth -= 1;
            self.active_macro_slot = parent_slot;
            return Ok(());
        }
        self.release_macro_slot(frame.slot());
        self.macro_depth -= 1;
        self.active_macro_slot = parent_slot;
        if pending_slot != NO_MACRO_SLOT && self.active_macro_slot == NO_MACRO_SLOT {
            self.rebase_pending_macro_suffix(pending_slot)?;
        }
        Ok(())
    }

    fn release_macro_slot(&mut self, slot_index: u32) {
        if self.transient_depth != 0 {
            self.macro_slots[slot_index as usize].transient_retired_at = self.transient_depth;
            return;
        }
        let slot = &mut self.macro_slots[slot_index as usize];
        slot.clear();
        slot.parent_slot = self.free_macro_slot;
        self.free_macro_slot = slot_index;
    }

    fn rebase_pending_macro_suffix(&mut self, pending_slot: u32) -> Result<(), ScratchError> {
        let slot = self
            .macro_slots
            .get(pending_slot as usize)
            .filter(|slot| slot.live && !slot.sealed)
            .ok_or(ScratchError::InvalidCoordinate)?;
        // A live writer owns its append coordinates until publication. The
        // completed arguments of an unsealed frame have no external cursor and
        // are the only pending suffix that may be rebased.
        if slot.current_argument.is_some() {
            return Ok(());
        }
        let start = slot.lane_mark;
        let destination = slot.reclaim_mark;
        let (shift, physical_copies) = self
            .macro_words
            .rebase_unpublished_suffix(start, destination)?;
        #[cfg(test)]
        {
            self.physical_macro_word_copies = self
                .physical_macro_word_copies
                .checked_add(u64::from(physical_copies))
                .expect("test copy accounting exceeds u64");
        }
        #[cfg(not(test))]
        let _ = physical_copies;
        if shift == 0 {
            return Ok(());
        }
        let slot = &mut self.macro_slots[pending_slot as usize];
        slot.lane_mark -= shift;
        slot.reclaim_mark = slot.lane_mark;
        for argument in &mut slot.arguments[..usize::from(slot.argument_count)] {
            argument.range.start -= shift;
        }
        Ok(())
    }

    #[cfg(test)]
    fn validate_argument_range(&self, range: MacroArgumentRange<G>) -> Result<(), ScratchError> {
        self.sealed_argument(range).map(drop)
    }

    fn truncate_macro_words(&mut self, mark: u32) -> Result<(), ScratchError> {
        self.macro_words.truncate(mark)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tex_state::token::{Catcode, OriginId, Token, TokenWord};

    fn word(ch: char) -> TracedTokenWord {
        TracedTokenWord::pack(
            Token::Char {
                ch,
                cat: Catcode::Other,
            },
            OriginId::UNKNOWN,
        )
    }

    fn brace(ch: char, cat: Catcode) -> TracedTokenWord {
        TracedTokenWord::pack(Token::Char { ch, cat }, OriginId::UNKNOWN)
    }

    fn seal_argument<G>(
        scratch: &mut ExecutionScratch<G>,
        words: impl IntoIterator<Item = TracedTokenWord>,
    ) -> ArgumentSetId<G> {
        let matching = scratch.begin_macro_match().expect("macro match");
        let mut buffer = scratch
            .begin_argument_writer(&matching)
            .expect("argument buffer");
        for word in words {
            scratch
                .append_argument_token(&mut buffer, ClassifiedToken::from_word(word, None), true)
                .expect("argument word");
        }
        scratch.publish_argument(buffer).expect("argument range");
        scratch.commit_macro_match(matching).expect("sealed frame")
    }

    #[test]
    fn scanner_frame_coordinates_reject_stale_aba_and_double_consume() {
        let mut lane = ResumeFrameLane::<u32, ()>::default();
        let stale = lane.insert(1).expect("first frame");
        assert_eq!(lane.take(stale), Ok(1));
        assert_eq!(lane.take(stale), Err(ScratchError::InvalidCoordinate));
        let replacement = lane.insert(2).expect("reused slot");
        assert_eq!(replacement.slot, stale.slot);
        assert_ne!(replacement.serial, stale.serial);
        assert_eq!(lane.slot(stale), Err(ScratchError::InvalidCoordinate));
        assert_eq!(lane.take(replacement), Ok(2));
    }

    #[test]
    fn argument_set_capacity_failures_precede_every_lifecycle_mutation() {
        let first_outside_packed_slot =
            u32::try_from(ARGUMENT_SET_SLOT_MASK + 1).expect("24-bit slot limit fits u32");
        assert!(ArgumentSetId::<()>::new(ARGUMENT_SET_SLOT_MASK as u32, 1).is_ok());
        assert_eq!(
            ArgumentSetId::<()>::new(first_outside_packed_slot, 1),
            Err(ScratchError::CapacityOverflow)
        );

        let mut scratch = ExecutionScratch::<()> {
            next_macro_serial: 0,
            ..ExecutionScratch::default()
        };
        let pristine = (
            scratch.macro_slots.len(),
            scratch.pending_macro_slot,
            scratch.free_macro_slot,
            scratch.macro_depth,
            scratch.macro_words.len(),
        );
        assert!(matches!(
            scratch.begin_macro_match(),
            Err(ScratchError::CapacityOverflow)
        ));
        assert_eq!(
            (
                scratch.macro_slots.len(),
                scratch.pending_macro_slot,
                scratch.free_macro_slot,
                scratch.macro_depth,
                scratch.macro_words.len(),
            ),
            pristine
        );

        scratch.next_macro_serial = 1;
        let matching = scratch.begin_macro_match().expect("valid pending frame");
        let pending = scratch.pending_macro_slot as usize;
        scratch.macro_slots[pending].serial = ARGUMENT_SET_SERIAL_LIMIT;
        let before_commit = (
            scratch.pending_macro_slot,
            scratch.active_macro_slot,
            scratch.macro_depth,
            scratch.macro_slots[pending].sealed,
            scratch.macro_slots[pending].parent_slot,
        );
        assert!(matches!(
            scratch.commit_macro_match(matching),
            Err(ScratchError::InvalidCoordinate)
        ));
        assert_eq!(
            (
                scratch.pending_macro_slot,
                scratch.active_macro_slot,
                scratch.macro_depth,
                scratch.macro_slots[pending].sealed,
                scratch.macro_slots[pending].parent_slot,
            ),
            before_commit
        );

        let mut scratch = ExecutionScratch::<()>::default();
        let matching = scratch.begin_macro_match().expect("valid pending frame");
        scratch.macro_depth = u32::MAX;
        let pending = scratch.pending_macro_slot as usize;
        let before_commit = (
            scratch.pending_macro_slot,
            scratch.active_macro_slot,
            scratch.macro_depth,
            scratch.macro_slots[pending].sealed,
            scratch.macro_slots[pending].parent_slot,
        );
        assert!(matches!(
            scratch.commit_macro_match(matching),
            Err(ScratchError::CapacityOverflow)
        ));
        assert_eq!(
            (
                scratch.pending_macro_slot,
                scratch.active_macro_slot,
                scratch.macro_depth,
                scratch.macro_slots[pending].sealed,
                scratch.macro_slots[pending].parent_slot,
            ),
            before_commit
        );
    }

    #[test]
    fn scanner_frame_serial_failure_retains_payload_and_lane_state() {
        let mut lane = ResumeFrameLane::<u32, ()> {
            next_serial: u64::MAX,
            ..ResumeFrameLane::default()
        };
        let mut payload = Some(7);
        let before = (
            lane.slots.len(),
            lane.slots.capacity(),
            lane.free_slots.len(),
            lane.free_slots.capacity(),
            lane.next_serial,
        );

        assert_eq!(
            lane.insert_from(&mut payload),
            Err(ScratchError::CapacityOverflow)
        );
        assert_eq!(payload, Some(7));
        assert_eq!(
            (
                lane.slots.len(),
                lane.slots.capacity(),
                lane.free_slots.len(),
                lane.free_slots.capacity(),
                lane.next_serial,
            ),
            before
        );
    }

    #[test]
    fn warmed_8192_nested_scanner_frames_reuse_bounded_slots() {
        let mut lane = ResumeFrameLane::<Option<ResumeFrameId<()>>, ()>::default();
        let run = |lane: &mut ResumeFrameLane<Option<ResumeFrameId<()>>, ()>| {
            let mut child = None;
            for _ in 0..8_192 {
                child = Some(lane.insert(child).expect("nested frame"));
            }
            while let Some(frame) = child {
                child = lane.take(frame).expect("resume child");
            }
        };
        run(&mut lane);
        run(&mut lane);
        assert_eq!(lane.slots.len(), 8_192);
        assert_eq!(lane.live_len(), 0);
    }

    #[cfg(feature = "profiling")]
    #[test]
    fn warmed_8192_nested_scanner_frames_allocate_zero_heap() {
        let mut lane = ResumeFrameLane::<Option<ResumeFrameId<()>>, ()>::default();
        let run = |lane: &mut ResumeFrameLane<Option<ResumeFrameId<()>>, ()>| {
            let mut child = None;
            for _ in 0..8_192 {
                child = Some(lane.insert(child).expect("nested frame"));
            }
            while let Some(frame) = child {
                child = lane.take(frame).expect("resume child");
            }
        };
        run(&mut lane);

        let owner = tex_state::measurement::HotCoreAllocationOwner::AttemptScratch;
        let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        {
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            run(&mut lane);
        }
        let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        assert_eq!(after.calls - before.calls, 0);
        assert_eq!(after.requested_bytes - before.requested_bytes, 0);
        assert_eq!(lane.slots.len(), 8_192);
    }

    #[test]
    fn chunked_argument_seals_in_place_and_replays_sequentially() {
        let mut scratch = ExecutionScratch::<()>::default();
        let expected = (0..(MACRO_WORD_RESERVE * 2 + 3))
            .map(|index| word(char::from(b'a' + (index % 26) as u8)))
            .collect::<Vec<_>>();
        let frame = seal_argument(&mut scratch, expected.iter().copied());
        let range = scratch
            .argument_range(frame, 1)
            .expect("live frame")
            .expect("first argument");
        for (index, expected) in expected.into_iter().enumerate() {
            assert_eq!(scratch.argument_word(range, index), Ok(expected));
        }
        assert!(
            scratch
                .argument_word(range, MACRO_WORD_RESERVE * 2 + 3)
                .is_err()
        );
        assert_eq!(scratch.physical_macro_word_copies(), 0);
        scratch
            .release_argument_set(frame)
            .expect("frame retirement");
        assert!(scratch.is_quiescent());
        assert_eq!(scratch.retained_slot_len(), 1);
        assert_eq!(scratch.retained_word_capacity(), MACRO_WORD_RESERVE * 3);
    }

    #[test]
    fn sealed_argument_coordinates_survive_nested_chunk_growth_without_copy() {
        let mut scratch = ExecutionScratch::<()>::default();
        let parent = seal_argument(&mut scratch, [word('p'), word('q')]);
        let parent_range = scratch
            .argument_range(parent, 1)
            .expect("parent frame")
            .expect("parent argument");
        let before = scratch
            .argument_word_ref(parent_range, 1)
            .expect("parent word");

        let child = seal_argument(
            &mut scratch,
            (0..(MACRO_WORD_RESERVE * 3 + 1)).map(|_| word('c')),
        );
        let after = scratch
            .argument_word_ref(parent_range, 1)
            .expect("parent word after growth");
        assert_eq!(after, before);
        assert_eq!(scratch.physical_macro_word_copies(), 0);

        scratch
            .release_argument_set(child)
            .expect("child retirement");
        scratch
            .release_argument_set(parent)
            .expect("parent retirement");
    }

    #[test]
    fn admitted_brace_delta_drives_nested_depth_and_first_scan_facts_once() {
        let mut scratch = ExecutionScratch::<()>::default();
        let matching = scratch.begin_macro_match().expect("macro match");
        let mut buffer = scratch
            .begin_argument_writer(&matching)
            .expect("argument buffer");
        let words = [
            brace('{', Catcode::BeginGroup),
            brace('[', Catcode::BeginGroup),
            word('p'),
            brace(']', Catcode::EndGroup),
            brace('}', Catcode::EndGroup),
        ];
        let paragraph_token = word('p').semantic_token();
        for (word, expected_depth) in words.into_iter().zip([1, 2, 2, 1, 0]) {
            let depth = scratch
                .append_argument_token(
                    &mut buffer,
                    ClassifiedToken::from_word(word, Some(TokenWord::pack(paragraph_token))),
                    true,
                )
                .expect("argument word");
            assert_eq!(depth, expected_depth);
            assert_eq!(buffer.brace_depth(), expected_depth);
        }
        let facts = buffer.facts();
        assert!(facts.rejects_non_long_paragraph());
        assert!(facts.removable_outer_group());
        assert_eq!(scratch.match_word_reads(), 0);

        buffer
            .strip_outer_group()
            .expect("outer group strips by metadata");
        scratch.publish_argument(buffer).expect("argument range");
        let frame = scratch.commit_macro_match(matching).expect("sealed frame");
        let range = scratch
            .argument_range(frame, 1)
            .expect("live frame")
            .expect("first argument");
        assert_eq!(scratch.argument_len(range), Ok(3));
        let sealed = scratch.argument_facts(range).expect("sealed facts");
        assert!(sealed.rejects_non_long_paragraph());
        assert!(sealed.removable_outer_group());
        assert_eq!(scratch.match_word_reads(), 0);

        scratch
            .release_argument_set(frame)
            .expect("frame retirement");
        assert_eq!(
            scratch.argument_facts(range),
            Err(ScratchError::InvalidCoordinate)
        );
    }

    #[test]
    fn empty_one_64_and_multiblock_writers_publish_once_without_word_copy() {
        let mut scratch = ExecutionScratch::<()>::default();
        for token_count in [0, 1, 64, MACRO_WORD_RESERVE, MACRO_WORD_RESERVE + 1] {
            let before = scratch.match_writer_operations();
            let copies = scratch.physical_macro_word_copies();
            let matching = scratch.begin_macro_match().expect("macro match");
            let mut writer = scratch
                .begin_argument_writer(&matching)
                .expect("resident match writer");
            for index in 0..token_count {
                let token_word = match index % 4 {
                    0 => word('w'),
                    1 => brace('{', Catcode::BeginGroup),
                    2 => word('p'),
                    _ => brace('}', Catcode::EndGroup),
                };
                scratch
                    .append_argument_token(
                        &mut writer,
                        ClassifiedToken::from_word(token_word, Some(word('p').token_word())),
                        true,
                    )
                    .expect("fused classified-token settlement");
            }
            let written = scratch.match_writer_operations();
            assert_eq!(written.0 - before.0, 1);
            assert_eq!(written.1 - before.1, token_count as u64);
            assert_eq!(written.2 - before.2, token_count as u64);
            assert_eq!(written.3 - before.3, 0);
            assert_eq!(written.4 - before.4, 1);
            scratch
                .publish_argument(writer)
                .expect("single writer finalization");
            let published = scratch.match_writer_operations();
            assert_eq!(published.3 - before.3, 1);
            assert_eq!(published.4 - before.4, 2);
            assert_eq!(scratch.physical_macro_word_copies(), copies);
            let frame = scratch.commit_macro_match(matching).expect("sealed frame");
            let range = scratch
                .argument_range(frame, 1)
                .expect("live frame")
                .expect("argument range");
            assert_eq!(scratch.argument_len(range), Ok(token_count));
            scratch
                .release_argument_set(frame)
                .expect("frame retirement");
        }
        assert!(scratch.is_quiescent());
    }

    #[test]
    fn trimmed_match_iterator_stops_at_both_stored_range_boundaries() {
        let mut scratch = ExecutionScratch::<()>::default();
        let matching = scratch.begin_macro_match().expect("macro match");
        let mut buffer = scratch
            .begin_argument_writer(&matching)
            .expect("argument buffer");
        for token in [
            brace('{', Catcode::BeginGroup),
            word('x'),
            brace('}', Catcode::EndGroup),
            word('z'),
        ] {
            scratch
                .append_argument_token(&mut buffer, ClassifiedToken::from_word(token, None), true)
                .expect("argument word");
        }
        // The last word models unrelated live lane material beyond the
        // argument. Trimming excludes the outer pair, so neither boundary is
        // allowed to leak through the exact-size observation iterator.
        buffer.set_end_trim(1);
        buffer.strip_outer_group().expect("outer group trim");
        let mut words = scratch.match_words(&buffer).expect("trimmed words");
        assert_eq!(words.len(), 1);
        assert_eq!(words.next(), Some(word('x')));
        assert_eq!(words.len(), 0);
        assert_eq!(words.next(), None);
        assert_eq!(words.next(), None);
    }

    #[test]
    fn outer_group_fact_rejects_empty_one_token_and_trailing_material() {
        let cases = [
            (Vec::new(), false),
            (vec![word('x')], false),
            (
                vec![
                    brace('{', Catcode::BeginGroup),
                    word('x'),
                    brace('}', Catcode::EndGroup),
                ],
                true,
            ),
            (
                vec![
                    brace('{', Catcode::BeginGroup),
                    word('x'),
                    brace('}', Catcode::EndGroup),
                    word('y'),
                ],
                false,
            ),
        ];
        for (words, expected) in cases {
            let mut scratch = ExecutionScratch::<()>::default();
            let frame = seal_argument(&mut scratch, words);
            let range = scratch
                .argument_range(frame, 1)
                .expect("live frame")
                .expect("first argument");
            assert_eq!(
                scratch
                    .argument_facts(range)
                    .expect("sealed argument facts")
                    .removable_outer_group(),
                expected
            );
        }
    }

    #[test]
    fn repeated_same_depth_replacement_rebases_only_unpublished_child_arguments() {
        let mut scratch = ExecutionScratch::<()>::default();
        let mut frame = seal_argument(&mut scratch, [word('a')]);
        for index in 0..8_192 {
            let matching = scratch.begin_macro_match().expect("replacement match");
            let mut buffer = scratch
                .begin_argument_writer(&matching)
                .expect("replacement argument");
            scratch
                .append_argument_token(
                    &mut buffer,
                    ClassifiedToken::from_word(word(if index % 2 == 0 { 'b' } else { 'c' }), None),
                    true,
                )
                .expect("replacement word");
            scratch.publish_argument(buffer).expect("replacement range");
            scratch
                .release_argument_set(frame)
                .expect("tail retirement");
            frame = scratch
                .commit_macro_match(matching)
                .expect("replacement activation");
        }
        assert_eq!(scratch.frame_len(), 1);
        assert_eq!(scratch.retained_slot_len(), 2);
        assert_eq!(scratch.retained_word_capacity(), MACRO_WORD_RESERVE);
        assert_eq!(scratch.physical_macro_word_copies(), 8_192);
        scratch
            .release_argument_set(frame)
            .expect("final retirement");
        assert!(scratch.is_quiescent());
    }

    #[test]
    fn discarded_match_releases_its_slot_without_touching_parent_output() {
        let mut scratch = ExecutionScratch::<()>::default();
        let parent = seal_argument(&mut scratch, [word('p')]);
        let rejected = scratch.begin_macro_match().expect("rejected match");
        let mut buffer = scratch
            .begin_argument_writer(&rejected)
            .expect("rejected argument");
        scratch
            .append_argument_token(
                &mut buffer,
                ClassifiedToken::from_word(word('x'), None),
                true,
            )
            .expect("rejected word");
        scratch
            .discard_macro_match(rejected)
            .expect("rollback releases match");
        let range = scratch
            .argument_range(parent, 1)
            .expect("parent frame")
            .expect("parent argument");
        assert_eq!(scratch.argument_word(range, 0), Ok(word('p')));
        scratch
            .release_argument_set(parent)
            .expect("parent retirement");
        assert!(scratch.is_quiescent());
    }

    #[test]
    fn discarded_tail_match_releases_after_its_parent_retires() {
        let mut scratch = ExecutionScratch::<()>::default();
        let parent = seal_argument(&mut scratch, [word('p')]);
        let rejected = scratch.begin_macro_match().expect("rejected tail match");
        let mut buffer = scratch
            .begin_argument_writer(&rejected)
            .expect("rejected tail argument");
        scratch
            .append_argument_token(
                &mut buffer,
                ClassifiedToken::from_word(word('x'), None),
                true,
            )
            .expect("rejected tail word");
        scratch
            .release_argument_set(parent)
            .expect("parent retires beneath pending frame");
        scratch
            .discard_macro_match(rejected)
            .expect("rollback releases canonical pending frame");
        assert!(scratch.is_quiescent());
        assert_eq!(scratch.retained_slot_len(), 2);
        assert_eq!(scratch.retained_word_capacity(), MACRO_WORD_RESERVE);
    }

    #[test]
    fn nested_frames_rewind_strictly_and_reject_stale_argument_slots() {
        let mut scratch = ExecutionScratch::<()>::default();
        let parent = seal_argument(&mut scratch, [word('p')]);
        let parent_range = scratch
            .argument_range(parent, 1)
            .expect("parent frame")
            .expect("parent argument");
        let child = seal_argument(
            &mut scratch,
            (0..(MACRO_WORD_RESERVE + 1)).map(|_| word('c')),
        );
        assert_eq!(
            scratch.release_argument_set(parent),
            Err(ScratchError::InvalidCoordinate)
        );
        assert_eq!(scratch.argument_word(parent_range, 0), Ok(word('p')));
        scratch
            .release_argument_set(child)
            .expect("child retirement");
        scratch
            .release_argument_set(parent)
            .expect("parent retirement");
        assert_eq!(
            scratch.argument_word(parent_range, 0),
            Err(ScratchError::InvalidCoordinate)
        );

        let replacement = seal_argument(&mut scratch, [word('r')]);
        assert_eq!(replacement.slot(), parent.slot());
        assert_ne!(replacement.serial(), parent.serial());
        assert_eq!(
            scratch.argument_word(parent_range, 0),
            Err(ScratchError::InvalidCoordinate)
        );
        scratch
            .release_argument_set(replacement)
            .expect("replacement retirement");
    }

    #[cfg(feature = "profiling")]
    #[test]
    fn warmed_empty_one_64_and_multiblock_writers_allocate_zero_and_copy_zero() {
        let mut scratch = ExecutionScratch::<()>::default();
        let write = |scratch: &mut ExecutionScratch<()>, token_count: usize| {
            let matching = scratch.begin_macro_match().expect("macro match");
            let mut writer = scratch
                .begin_argument_writer(&matching)
                .expect("resident match writer");
            for index in 0..token_count {
                let token_word = match index % 4 {
                    0 => word('w'),
                    1 => brace('{', Catcode::BeginGroup),
                    2 => word('p'),
                    _ => brace('}', Catcode::EndGroup),
                };
                scratch
                    .append_argument_token(
                        &mut writer,
                        ClassifiedToken::from_word(token_word, Some(word('p').token_word())),
                        true,
                    )
                    .expect("fused classified-token settlement");
            }
            scratch
                .publish_argument(writer)
                .expect("single writer finalization");
            let frame = scratch.commit_macro_match(matching).expect("sealed frame");
            scratch
                .release_argument_set(frame)
                .expect("frame retirement");
        };
        write(&mut scratch, MACRO_WORD_RESERVE + 1);
        let owner = tex_state::measurement::HotCoreAllocationOwner::AttemptScratch;
        let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        let copies = scratch.physical_macro_word_copies();
        {
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            write(&mut scratch, 0);
            write(&mut scratch, 1);
            write(&mut scratch, 64);
            write(&mut scratch, MACRO_WORD_RESERVE);
            write(&mut scratch, MACRO_WORD_RESERVE + 1);
        }
        let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        assert_eq!(after.calls - before.calls, 0);
        assert_eq!(after.requested_bytes - before.requested_bytes, 0);
        assert_eq!(scratch.physical_macro_word_copies(), copies);
    }

    #[cfg(feature = "profiling")]
    #[test]
    fn warmed_8192_same_depth_replacements_allocate_zero_heap() {
        let mut scratch = ExecutionScratch::<()>::default();
        let mut frame = seal_argument(&mut scratch, [word('a')]);
        let replace = |scratch: &mut ExecutionScratch<()>, frame: ArgumentSetId<()>| {
            let matching = scratch.begin_macro_match().expect("replacement match");
            let mut buffer = scratch
                .begin_argument_writer(&matching)
                .expect("replacement buffer");
            scratch
                .append_argument_token(
                    &mut buffer,
                    ClassifiedToken::from_word(word('b'), None),
                    true,
                )
                .expect("replacement word");
            scratch.publish_argument(buffer).expect("replacement range");
            scratch
                .release_argument_set(frame)
                .expect("prior retirement");
            scratch
                .commit_macro_match(matching)
                .expect("replacement frame")
        };
        for _ in 0..64 {
            frame = replace(&mut scratch, frame);
        }
        let owner = tex_state::measurement::HotCoreAllocationOwner::AttemptScratch;
        let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        {
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            for _ in 0..8_192 {
                frame = replace(&mut scratch, frame);
            }
        }
        let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        assert_eq!(after.calls - before.calls, 0);
        assert_eq!(after.requested_bytes - before.requested_bytes, 0);
        assert_eq!(scratch.retained_slot_len(), 2);
        assert_eq!(scratch.retained_word_capacity(), MACRO_WORD_RESERVE);
        assert_eq!(scratch.physical_macro_word_copies(), 64 + 8_192);
        scratch
            .release_argument_set(frame)
            .expect("final retirement");
    }

    #[cfg(feature = "profiling")]
    #[test]
    fn ten_million_warmed_complete_calls_plateau_without_allocation() {
        let mut scratch = ExecutionScratch::<()>::default();
        for _ in 0..64 {
            let frame = seal_argument(&mut scratch, [word('a')]);
            scratch
                .release_argument_set(frame)
                .expect("warm retirement");
        }
        let capacity = scratch.retained_word_capacity();
        let owner = tex_state::measurement::HotCoreAllocationOwner::AttemptScratch;
        let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        {
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            for _ in 0..10_000_000 {
                let frame = seal_argument(&mut scratch, [word('b')]);
                let range = scratch
                    .argument_range(frame, 1)
                    .expect("live frame")
                    .expect("argument");
                let _ = scratch.argument_word_ref(range, 0).expect("direct word");
                scratch
                    .release_argument_set(frame)
                    .expect("call retirement");
            }
        }
        let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        assert_eq!(after.calls - before.calls, 0);
        assert_eq!(after.requested_bytes - before.requested_bytes, 0);
        assert_eq!(scratch.retained_word_capacity(), capacity);
        assert_eq!(capacity, MACRO_WORD_RESERVE);
        assert!(scratch.is_quiescent());
    }
}
