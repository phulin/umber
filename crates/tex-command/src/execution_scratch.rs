//! Generation-owned reusable command execution scratch.
//!
//! Macro matchers write directly into one reusable stable segment arena.
//! Sealing changes only the owning frame role; retirement returns that frame's
//! intrusive segment chain to the arena free head. No macro invocation owns a
//! heap buffer or an attempt-arena scope.

#[cfg(any(test, feature = "profiling"))]
use core::cell::Cell;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;

use tex_state::token::TracedTokenWord;

const MACRO_SEGMENT_WORDS: usize = 4_096;
const NO_MACRO_SLOT: u32 = u32::MAX;
const NO_MACRO_SEGMENT: u32 = u32::MAX;

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
    Expansion(crate::state::PendingExpansion<G>),
    ExpandAfter(crate::processor::expand::PendingExpandAfter<G>),
    PdfStringCompare(crate::processor::expand::PendingPdfStringCompare<G>),
    AlignmentPreamble(crate::scanners::PendingAlignmentPreamble<G>),
    StructuredScanner(crate::scanners::PendingStructuredScanner<G>),
}

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
        self.allocate(payload)
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

    fn allocate(&mut self, payload: T) -> Result<ResumeFrameId<G>, ScratchError> {
        let index = if let Some(index) = self.free_slots.pop() {
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
            self.slots.push(ResumeFrameSlot::default());
            index
        };
        let serial = self.next_serial;
        self.next_serial = self.next_serial.wrapping_add(1).max(1);
        let slot = &mut self.slots[index as usize];
        if slot.payload.is_some() {
            return Err(ScratchError::InvalidCoordinate);
        }
        *slot = ResumeFrameSlot {
            serial,
            payload: Some(payload),
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
pub(crate) struct MacroFrameId<G> {
    slot: u32,
    serial: u64,
    _generation: PhantomData<fn(&G) -> &G>,
}

impl<G> Copy for MacroFrameId<G> {}
impl<G> Clone for MacroFrameId<G> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<G> PartialEq for MacroFrameId<G> {
    fn eq(&self, other: &Self) -> bool {
        self.slot == other.slot && self.serial == other.serial
    }
}
impl<G> Eq for MacroFrameId<G> {}
impl<G> Hash for MacroFrameId<G> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.slot.hash(state);
        self.serial.hash(state);
    }
}

#[derive(Debug)]
pub(crate) struct MacroArgumentRange<G> {
    frame: MacroFrameId<G>,
    slot: u8,
}

/// Exact TeX82 §394 facts established while one argument is collected.
///
/// `rejects_non_long_paragraph` records only a `par_token` which passed
/// through §394's ordinary-token check. A token first held as a delimiter
/// prefix is deliberately not equivalent, even if that prefix is later
/// committed to the argument. `removable_outer_group` describes the collected
/// pre-stripping span whose outer pair is omitted from the sealed range.
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

/// Facts about one token at the point §394 admits it as argument material.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct MacroArgumentTokenFacts {
    pub(crate) rejects_non_long_paragraph: bool,
    pub(crate) begin_group: bool,
    pub(crate) end_group: bool,
}

impl<G> Copy for MacroArgumentRange<G> {}
impl<G> Clone for MacroArgumentRange<G> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<G> PartialEq for MacroArgumentRange<G> {
    fn eq(&self, other: &Self) -> bool {
        self.frame == other.frame && self.slot == other.slot
    }
}
impl<G> Eq for MacroArgumentRange<G> {}
impl<G> Hash for MacroArgumentRange<G> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.frame.hash(state);
        self.slot.hash(state);
    }
}

#[derive(Debug)]
pub(crate) struct MacroMatch<G> {
    _generation: PhantomData<fn(&G) -> &G>,
}

#[derive(Debug)]
pub(crate) struct MacroMatchBuffer<G> {
    slot: u8,
    _generation: PhantomData<fn(&G) -> &G>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PackedRange {
    first_segment: u32,
    first_offset: u16,
    len: u32,
}

impl Default for PackedRange {
    fn default() -> Self {
        Self {
            first_segment: NO_MACRO_SEGMENT,
            first_offset: 0,
            len: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PackedArgument {
    range: PackedRange,
    facts: PendingArgumentFacts,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum OuterGroupProgress {
    #[default]
    Empty,
    Open(u32),
    Closed,
    Invalid,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PendingArgumentFacts {
    rejects_non_long_paragraph: bool,
    outer_group: OuterGroupProgress,
}

impl PendingArgumentFacts {
    fn push(&mut self, facts: MacroArgumentTokenFacts) {
        self.rejects_non_long_paragraph |= facts.rejects_non_long_paragraph;
        self.outer_group = match self.outer_group {
            OuterGroupProgress::Empty if facts.begin_group => OuterGroupProgress::Open(1),
            OuterGroupProgress::Empty => OuterGroupProgress::Invalid,
            OuterGroupProgress::Open(depth) if facts.begin_group => {
                OuterGroupProgress::Open(depth.saturating_add(1))
            }
            OuterGroupProgress::Open(depth) if facts.end_group && depth > 1 => {
                OuterGroupProgress::Open(depth - 1)
            }
            OuterGroupProgress::Open(1) if facts.end_group => OuterGroupProgress::Closed,
            OuterGroupProgress::Open(depth) => OuterGroupProgress::Open(depth),
            OuterGroupProgress::Closed | OuterGroupProgress::Invalid => OuterGroupProgress::Invalid,
        };
    }

    const fn seal(self) -> MacroArgumentFacts {
        MacroArgumentFacts {
            rejects_non_long_paragraph: self.rejects_non_long_paragraph,
            removable_outer_group: matches!(self.outer_group, OuterGroupProgress::Closed),
        }
    }
}

#[derive(Debug, Default, Eq, PartialEq)]
struct MacroSlot {
    serial: u64,
    first_segment: u32,
    last_segment: u32,
    word_len: u32,
    arguments: [PackedArgument; 9],
    argument_count: u8,
    current_argument: Option<u8>,
    parent_slot: u32,
    sealed: bool,
    live: bool,
}

impl MacroSlot {
    fn clear(&mut self) {
        self.serial = 0;
        self.first_segment = NO_MACRO_SEGMENT;
        self.last_segment = NO_MACRO_SEGMENT;
        self.word_len = 0;
        self.argument_count = 0;
        self.current_argument = None;
        self.parent_slot = NO_MACRO_SLOT;
        self.sealed = false;
        self.live = false;
    }
}

#[derive(Debug, Eq, PartialEq)]
struct MacroWordSegment {
    words: Vec<TracedTokenWord>,
    next: u32,
    live: bool,
}

impl MacroWordSegment {
    fn new() -> Result<Self, ScratchError> {
        let mut words = Vec::new();
        words
            .try_reserve_exact(MACRO_SEGMENT_WORDS)
            .map_err(|_| ScratchError::AllocationFailed)?;
        Ok(Self {
            words,
            next: NO_MACRO_SEGMENT,
            live: true,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScratchError {
    InvalidCoordinate,
    CapacityOverflow,
    AllocationFailed,
}

pub(crate) struct MacroWords<'a, G> {
    scratch: &'a ExecutionScratch<G>,
    segment: u32,
    offset: u16,
    remaining: u32,
}

impl<G> Clone for MacroWords<'_, G> {
    fn clone(&self) -> Self {
        Self {
            scratch: self.scratch,
            segment: self.segment,
            offset: self.offset,
            remaining: self.remaining,
        }
    }
}

impl<G> Iterator for MacroWords<'_, G> {
    type Item = TracedTokenWord;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let word = self.scratch.segment_word(self.segment, self.offset).ok()?;
        self.offset += 1;
        if usize::from(self.offset) == MACRO_SEGMENT_WORDS && self.remaining > 1 {
            self.segment = self.scratch.next_segment(self.segment).ok()?;
            self.offset = 0;
        }
        self.remaining -= 1;
        Some(word)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.remaining as usize;
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
    macro_segments: Vec<MacroWordSegment>,
    free_macro_segment: u32,
    next_macro_serial: u64,
    delimiter_head: u32,
    delimiter_len: u32,
    delimiter_first_segment: u32,
    delimiter_last_segment: u32,
    scanner_resumes: ResumeFrameLane<ContinuationFrame<G>, G>,
    expression_frames: Vec<crate::scanners::ExpressionFrame<G>>,
    _generation: PhantomData<fn(&G) -> &G>,
    #[cfg(test)]
    copied_macro_words: u64,
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
            macro_segments: Vec::new(),
            free_macro_segment: NO_MACRO_SEGMENT,
            next_macro_serial: 1,
            delimiter_head: 0,
            delimiter_len: 0,
            delimiter_first_segment: NO_MACRO_SEGMENT,
            delimiter_last_segment: NO_MACRO_SEGMENT,
            scanner_resumes: ResumeFrameLane::default(),
            expression_frames: Vec::new(),
            _generation: PhantomData,
            #[cfg(test)]
            copied_macro_words: 0,
            #[cfg(any(test, feature = "profiling"))]
            match_word_reads: Cell::new(0),
        }
    }
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
        let frame = self.scanner_resumes.take(key.id)?;
        let matches_kind = matches!(
            (&frame, key.kind),
            (ContinuationFrame::Scanner(_), ContinuationKind::Scanner)
                | (ContinuationFrame::Scalar(_), ContinuationKind::Scalar)
                | (ContinuationFrame::Expansion(_), ContinuationKind::Expansion)
                | (
                    ContinuationFrame::ExpandAfter(_),
                    ContinuationKind::ExpandAfter
                )
                | (
                    ContinuationFrame::PdfStringCompare(_),
                    ContinuationKind::PdfStringCompare
                )
                | (
                    ContinuationFrame::AlignmentPreamble(_),
                    ContinuationKind::AlignmentPreamble
                )
                | (
                    ContinuationFrame::StructuredScanner(_),
                    ContinuationKind::StructuredScanner
                )
        );
        if !matches_kind {
            return Err(ScratchError::InvalidCoordinate);
        }
        Ok(frame)
    }

    pub(crate) fn store_scalar_frame(
        &mut self,
        pending: crate::scanners::PendingScalarFrame<G>,
    ) -> Result<ScannerFrameKey<G>, ScratchError> {
        self.scanner_resumes
            .insert(ContinuationFrame::Scalar(pending))
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
        match self.scanner_resumes.take(key.id)? {
            ContinuationFrame::Scalar(pending) => Ok(pending),
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
        match self.scanner_resumes.slot_mut(key.id)?.payload.as_mut() {
            Some(ContinuationFrame::Scalar(pending)) => Ok(pending),
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
        pending: crate::scan_toks::PendingScanToks<G>,
    ) -> Result<ScannerFrameKey<G>, ScratchError> {
        self.scanner_resumes
            .insert(ContinuationFrame::Scanner(pending))
            .map(|id| ScannerFrameKey {
                id,
                kind: ContinuationKind::Scanner,
            })
    }

    pub(crate) fn take_scanner_frame(
        &mut self,
        key: ScannerFrameKey<G>,
    ) -> Result<crate::scan_toks::PendingScanToks<G>, ScratchError> {
        if !key.is_scanner() {
            return Err(ScratchError::InvalidCoordinate);
        }
        match self.scanner_resumes.take(key.id)? {
            ContinuationFrame::Scanner(pending) => Ok(pending),
            _ => Err(ScratchError::InvalidCoordinate),
        }
    }

    pub(crate) fn scanner_frame(
        &self,
        key: &ScannerFrameKey<G>,
    ) -> Result<&crate::scan_toks::PendingScanToks<G>, ScratchError> {
        if !key.is_scanner() {
            return Err(ScratchError::InvalidCoordinate);
        }
        match self.scanner_resumes.get(key.id)? {
            ContinuationFrame::Scanner(pending) => Ok(pending),
            _ => Err(ScratchError::InvalidCoordinate),
        }
    }

    pub(crate) fn store_expansion_frame(
        &mut self,
        pending: crate::state::PendingExpansion<G>,
    ) -> Result<ScannerFrameKey<G>, ScratchError> {
        self.scanner_resumes
            .insert(ContinuationFrame::Expansion(pending))
            .map(|id| ScannerFrameKey {
                id,
                kind: ContinuationKind::Expansion,
            })
    }

    pub(crate) fn take_expansion_frame(
        &mut self,
        key: ScannerFrameKey<G>,
    ) -> Result<crate::state::PendingExpansion<G>, ScratchError> {
        if !key.is_expansion() {
            return Err(ScratchError::InvalidCoordinate);
        }
        match self.scanner_resumes.take(key.id)? {
            ContinuationFrame::Expansion(pending) => Ok(pending),
            _ => Err(ScratchError::InvalidCoordinate),
        }
    }

    pub(crate) fn expansion_frame(
        &self,
        key: &ScannerFrameKey<G>,
    ) -> Result<&crate::state::PendingExpansion<G>, ScratchError> {
        if !key.is_expansion() {
            return Err(ScratchError::InvalidCoordinate);
        }
        match self.scanner_resumes.get(key.id)? {
            ContinuationFrame::Expansion(pending) => Ok(pending),
            _ => Err(ScratchError::InvalidCoordinate),
        }
    }

    pub(crate) fn expansion_frame_mut(
        &mut self,
        key: &ScannerFrameKey<G>,
    ) -> Result<&mut crate::state::PendingExpansion<G>, ScratchError> {
        if !key.is_expansion() {
            return Err(ScratchError::InvalidCoordinate);
        }
        match self.scanner_resumes.get_mut(key.id)? {
            ContinuationFrame::Expansion(pending) => Ok(pending),
            _ => Err(ScratchError::InvalidCoordinate),
        }
    }

    pub(crate) fn discard_expansion_frame(
        &mut self,
        key: ScannerFrameKey<G>,
    ) -> Result<(), ScratchError> {
        self.take_expansion_frame(key).map(drop)
    }

    pub(crate) fn store_expandafter_frame(
        &mut self,
        pending: crate::processor::expand::PendingExpandAfter<G>,
    ) -> Result<ScannerFrameKey<G>, ScratchError> {
        self.scanner_resumes
            .insert(ContinuationFrame::ExpandAfter(pending))
            .map(|id| ScannerFrameKey {
                id,
                kind: ContinuationKind::ExpandAfter,
            })
    }

    pub(crate) fn take_expandafter_frame(
        &mut self,
        key: ScannerFrameKey<G>,
    ) -> Result<crate::processor::expand::PendingExpandAfter<G>, ScratchError> {
        if !key.is_expandafter() {
            return Err(ScratchError::InvalidCoordinate);
        }
        match self.scanner_resumes.take(key.id)? {
            ContinuationFrame::ExpandAfter(pending) => Ok(pending),
            _ => Err(ScratchError::InvalidCoordinate),
        }
    }

    pub(crate) fn store_pdf_string_compare_frame(
        &mut self,
        pending: crate::processor::expand::PendingPdfStringCompare<G>,
    ) -> Result<ScannerFrameKey<G>, ScratchError> {
        self.scanner_resumes
            .insert(ContinuationFrame::PdfStringCompare(pending))
            .map(|id| ScannerFrameKey {
                id,
                kind: ContinuationKind::PdfStringCompare,
            })
    }

    pub(crate) fn take_pdf_string_compare_frame(
        &mut self,
        key: ScannerFrameKey<G>,
    ) -> Result<crate::processor::expand::PendingPdfStringCompare<G>, ScratchError> {
        if !key.is_pdf_string_compare() {
            return Err(ScratchError::InvalidCoordinate);
        }
        match self.scanner_resumes.take(key.id)? {
            ContinuationFrame::PdfStringCompare(pending) => Ok(pending),
            _ => Err(ScratchError::InvalidCoordinate),
        }
    }

    pub(crate) fn store_alignment_preamble_frame(
        &mut self,
        pending: crate::scanners::PendingAlignmentPreamble<G>,
    ) -> Result<ScannerFrameKey<G>, ScratchError> {
        self.scanner_resumes
            .insert(ContinuationFrame::AlignmentPreamble(pending))
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
        match self.scanner_resumes.take(key.id)? {
            ContinuationFrame::AlignmentPreamble(pending) => Ok(pending),
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
        match self.scanner_resumes.get_mut(key.id)? {
            ContinuationFrame::AlignmentPreamble(pending) => Ok(pending),
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
        self.scanner_resumes
            .insert(ContinuationFrame::StructuredScanner(pending))
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
        match self.scanner_resumes.take(key.id)? {
            ContinuationFrame::StructuredScanner(pending) => Ok(pending),
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
        match self.scanner_resumes.get_mut(key.id)? {
            ContinuationFrame::StructuredScanner(pending) => Ok(pending),
            _ => Err(ScratchError::InvalidCoordinate),
        }
    }

    pub(crate) fn discard_structured_scanner_frame(
        &mut self,
        key: ScannerFrameKey<G>,
    ) -> Result<(), ScratchError> {
        self.take_structured_scanner_frame(key).map(drop)
    }

    pub(crate) fn begin_macro_match(&mut self) -> Result<MacroMatch<G>, ScratchError> {
        if self.delimiter_len != 0 || self.pending_slot().is_ok() {
            return Err(ScratchError::InvalidCoordinate);
        }
        let slot_index = if self.free_macro_slot == NO_MACRO_SLOT {
            self.macro_slots.len()
        } else {
            let index = self.free_macro_slot as usize;
            self.free_macro_slot = self.macro_slots[index].parent_slot;
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
        self.next_macro_serial = self.next_macro_serial.wrapping_add(1).max(1);
        let slot = &mut self.macro_slots[slot_index];
        slot.serial = serial;
        slot.first_segment = NO_MACRO_SEGMENT;
        slot.last_segment = NO_MACRO_SEGMENT;
        slot.word_len = 0;
        slot.argument_count = 0;
        slot.current_argument = None;
        slot.parent_slot = NO_MACRO_SLOT;
        slot.sealed = false;
        slot.live = true;
        self.pending_macro_slot =
            u32::try_from(slot_index).map_err(|_| ScratchError::CapacityOverflow)?;
        Ok(MacroMatch {
            _generation: PhantomData,
        })
    }

    pub(crate) fn begin_match_buffer(
        &mut self,
        _matching: &MacroMatch<G>,
    ) -> Result<MacroMatchBuffer<G>, ScratchError> {
        let slot = self.pending_slot_mut()?;
        if slot.current_argument.is_some() || slot.argument_count >= 9 {
            return Err(ScratchError::InvalidCoordinate);
        }
        let argument_slot = slot.argument_count;
        slot.arguments[usize::from(argument_slot)] = PackedArgument {
            range: PackedRange {
                ..PackedRange::default()
            },
            facts: PendingArgumentFacts::default(),
        };
        slot.current_argument = Some(argument_slot);
        Ok(MacroMatchBuffer {
            slot: argument_slot,
            _generation: PhantomData,
        })
    }

    pub(crate) fn push_match_word(
        &mut self,
        buffer: &mut MacroMatchBuffer<G>,
        word: TracedTokenWord,
        facts: MacroArgumentTokenFacts,
    ) -> Result<(), ScratchError> {
        let slot_index = self.pending_macro_slot as usize;
        debug_assert_eq!(
            self.macro_slots[slot_index].current_argument,
            Some(buffer.slot)
        );
        let word_len = self.macro_slots[slot_index].word_len;
        if (word_len as usize).is_multiple_of(MACRO_SEGMENT_WORDS) {
            let segment = self.take_macro_segment()?;
            let previous = self.macro_slots[slot_index].last_segment;
            if previous == NO_MACRO_SEGMENT {
                self.macro_slots[slot_index].first_segment = segment;
            } else {
                self.macro_segments[previous as usize].next = segment;
            }
            self.macro_slots[slot_index].last_segment = segment;
        }
        let segment = self.macro_slots[slot_index].last_segment;
        let offset = self.macro_segments[segment as usize].words.len();
        self.macro_segments[segment as usize].words.push(word);
        let slot = &mut self.macro_slots[slot_index];
        slot.word_len = slot
            .word_len
            .checked_add(1)
            .ok_or(ScratchError::CapacityOverflow)?;
        let argument = &mut slot.arguments[usize::from(buffer.slot)];
        if argument.range.len == 0 {
            argument.range.first_segment = segment;
            argument.range.first_offset =
                u16::try_from(offset).map_err(|_| ScratchError::CapacityOverflow)?;
        }
        argument.range.len = argument
            .range
            .len
            .checked_add(1)
            .ok_or(ScratchError::CapacityOverflow)?;
        argument.facts.push(facts);
        Ok(())
    }

    pub(crate) fn match_words(
        &self,
        buffer: &MacroMatchBuffer<G>,
    ) -> Result<MacroWords<'_, G>, ScratchError> {
        let argument = self.pending_argument(buffer)?;
        Ok(MacroWords {
            scratch: self,
            segment: argument.range.first_segment,
            offset: argument.range.first_offset,
            remaining: argument.range.len,
        })
    }

    pub(crate) fn strip_match_outer_group(
        &mut self,
        buffer: &MacroMatchBuffer<G>,
    ) -> Result<(), ScratchError> {
        let range = self.pending_argument(buffer)?.range;
        if range.len < 2 {
            return Err(ScratchError::InvalidCoordinate);
        }
        let (first_segment, first_offset) =
            self.advance_segment(range.first_segment, range.first_offset)?;
        let argument = self.pending_argument_mut(buffer)?;
        argument.range.first_segment = first_segment;
        argument.range.first_offset = first_offset;
        argument.range.len -= 2;
        Ok(())
    }

    pub(crate) fn match_argument_facts(
        &self,
        buffer: &MacroMatchBuffer<G>,
    ) -> Result<MacroArgumentFacts, ScratchError> {
        Ok(self.pending_argument(buffer)?.facts.seal())
    }

    pub(crate) fn finish_match_buffer(
        &mut self,
        buffer: MacroMatchBuffer<G>,
    ) -> Result<(), ScratchError> {
        let slot = self.pending_slot_mut()?;
        if slot.current_argument != Some(buffer.slot) || slot.argument_count != buffer.slot {
            return Err(ScratchError::InvalidCoordinate);
        }
        slot.current_argument = None;
        slot.argument_count += 1;
        Ok(())
    }

    pub(crate) fn clear_delimiter_prefix(&mut self) {
        self.delimiter_head = 0;
        self.delimiter_len = 0;
        self.release_delimiter_segments();
    }

    pub(crate) const fn delimiter_prefix_len(&self) -> usize {
        self.delimiter_len as usize
    }

    pub(crate) const fn delimiter_prefix_is_empty(&self) -> bool {
        self.delimiter_len == 0
    }

    pub(crate) fn delimiter_prefix_word(
        &self,
        index: usize,
    ) -> Result<TracedTokenWord, ScratchError> {
        if index >= self.delimiter_len as usize {
            return Err(ScratchError::InvalidCoordinate);
        }
        let index = self
            .delimiter_head
            .checked_add(u32::try_from(index).map_err(|_| ScratchError::CapacityOverflow)?)
            .ok_or(ScratchError::CapacityOverflow)?;
        self.chain_word(self.delimiter_first_segment, index)
    }

    pub(crate) fn delimiter_prefix_words(&self) -> impl Iterator<Item = TracedTokenWord> + '_ {
        (0..self.delimiter_len as usize).map(|index| {
            self.delimiter_prefix_word(index)
                .expect("live delimiter prefix")
        })
    }

    pub(crate) fn push_delimiter_prefix(
        &mut self,
        word: TracedTokenWord,
    ) -> Result<(), ScratchError> {
        let tail = self
            .delimiter_head
            .checked_add(self.delimiter_len)
            .ok_or(ScratchError::CapacityOverflow)?;
        if (tail as usize).is_multiple_of(MACRO_SEGMENT_WORDS) {
            let segment = self.take_macro_segment()?;
            if self.delimiter_last_segment == NO_MACRO_SEGMENT {
                self.delimiter_first_segment = segment;
            } else {
                self.macro_segments[self.delimiter_last_segment as usize].next = segment;
            }
            self.delimiter_last_segment = segment;
        }
        let segment = self.delimiter_last_segment;
        self.segment(segment)?;
        self.macro_segments[segment as usize].words.push(word);
        self.delimiter_len = self
            .delimiter_len
            .checked_add(1)
            .ok_or(ScratchError::CapacityOverflow)?;
        Ok(())
    }

    pub(crate) fn pop_delimiter_prefix_word(&mut self) -> Result<TracedTokenWord, ScratchError> {
        if self.delimiter_len == 0 {
            return Err(ScratchError::InvalidCoordinate);
        }
        let word = self.chain_word(self.delimiter_first_segment, self.delimiter_head)?;
        self.delimiter_head = self
            .delimiter_head
            .checked_add(1)
            .ok_or(ScratchError::CapacityOverflow)?;
        self.delimiter_len -= 1;
        if self.delimiter_len == 0 {
            self.clear_delimiter_prefix();
        }
        Ok(word)
    }

    pub(crate) fn commit_macro_match(
        &mut self,
        _matching: MacroMatch<G>,
    ) -> Result<MacroFrameId<G>, ScratchError> {
        if self.delimiter_len != 0 {
            return Err(ScratchError::InvalidCoordinate);
        }
        let slot_index = self.pending_slot_index()? as u32;
        let slot = self.pending_slot()?;
        if slot.current_argument.is_some() {
            return Err(ScratchError::InvalidCoordinate);
        }
        let serial = slot.serial;
        let slot = &mut self.macro_slots[slot_index as usize];
        slot.parent_slot = self.active_macro_slot;
        slot.sealed = true;
        self.active_macro_slot = slot_index;
        self.pending_macro_slot = NO_MACRO_SLOT;
        self.macro_depth = self
            .macro_depth
            .checked_add(1)
            .ok_or(ScratchError::CapacityOverflow)?;
        let frame = MacroFrameId {
            slot: slot_index,
            serial,
            _generation: PhantomData,
        };
        Ok(frame)
    }

    pub(crate) fn discard_macro_match(
        &mut self,
        _matching: MacroMatch<G>,
    ) -> Result<(), ScratchError> {
        let slot_index = self.pending_slot_index()?;
        self.pending_macro_slot = NO_MACRO_SLOT;
        self.release_slot_segments(slot_index as u32)?;
        self.release_macro_slot(slot_index as u32);
        self.clear_delimiter_prefix();
        Ok(())
    }

    pub(crate) fn pop_macro_frame(&mut self, frame: MacroFrameId<G>) -> Result<(), ScratchError> {
        self.release_slot(frame)
    }

    pub(crate) fn argument_count(&self, frame: MacroFrameId<G>) -> Result<usize, ScratchError> {
        Ok(self.sealed_slot(frame)?.argument_count as usize)
    }

    pub(crate) fn argument_range(
        &self,
        frame: MacroFrameId<G>,
        slot: u8,
    ) -> Result<Option<MacroArgumentRange<G>>, ScratchError> {
        if !(1..=9).contains(&slot) {
            return Err(ScratchError::InvalidCoordinate);
        }
        Ok(
            (slot <= self.sealed_slot(frame)?.argument_count).then_some(MacroArgumentRange {
                frame,
                slot: slot - 1,
            }),
        )
    }

    #[cfg(test)]
    pub(crate) fn argument_facts(
        &self,
        range: MacroArgumentRange<G>,
    ) -> Result<MacroArgumentFacts, ScratchError> {
        Ok(self.sealed_argument(range)?.facts.seal())
    }

    pub(crate) fn argument_len(&self, range: MacroArgumentRange<G>) -> Result<usize, ScratchError> {
        Ok(self.sealed_argument(range)?.range.len as usize)
    }

    pub(crate) fn argument_word(
        &self,
        range: MacroArgumentRange<G>,
        index: usize,
    ) -> Result<TracedTokenWord, ScratchError> {
        let slot = self.sealed_slot(range.frame)?;
        if range.slot >= slot.argument_count {
            return Err(ScratchError::InvalidCoordinate);
        }
        let argument = &slot.arguments[usize::from(range.slot)];
        if index >= argument.range.len as usize {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.range_word(argument.range, index)
    }

    pub(crate) fn argument_word_len(&self) -> usize {
        self.macro_slots
            .iter()
            .filter(|slot| slot.live && slot.sealed)
            .map(|slot| slot.word_len as usize)
            .sum()
    }

    pub(crate) fn frame_len(&self) -> usize {
        self.macro_depth as usize
    }

    #[cfg(test)]
    pub(crate) fn retained_slot_len(&self) -> usize {
        self.macro_slots.len()
    }

    #[cfg(test)]
    pub(crate) fn retained_segment_len(&self) -> usize {
        self.macro_segments.len()
    }

    pub(crate) fn is_quiescent(&self) -> bool {
        self.frame_len() == 0
            && self.pending_slot().is_err()
            && !self.macro_segments.iter().any(|segment| segment.live)
            && self.delimiter_len == 0
            && self.scanner_resumes.live_len() == 0
    }

    #[cfg(test)]
    pub(crate) const fn copied_macro_words(&self) -> u64 {
        self.copied_macro_words
    }

    #[cfg(any(test, feature = "profiling"))]
    pub(crate) fn match_word_reads(&self) -> u64 {
        self.match_word_reads.get()
    }

    fn pending_argument(
        &self,
        buffer: &MacroMatchBuffer<G>,
    ) -> Result<&PackedArgument, ScratchError> {
        let slot = self.pending_slot()?;
        if slot.current_argument != Some(buffer.slot) {
            return Err(ScratchError::InvalidCoordinate);
        }
        Ok(&slot.arguments[usize::from(buffer.slot)])
    }

    fn pending_argument_mut(
        &mut self,
        buffer: &MacroMatchBuffer<G>,
    ) -> Result<&mut PackedArgument, ScratchError> {
        let slot = self.pending_slot_mut()?;
        if slot.current_argument != Some(buffer.slot) {
            return Err(ScratchError::InvalidCoordinate);
        }
        Ok(&mut slot.arguments[usize::from(buffer.slot)])
    }

    fn sealed_argument(
        &self,
        range: MacroArgumentRange<G>,
    ) -> Result<&PackedArgument, ScratchError> {
        let slot = self.sealed_slot(range.frame)?;
        (range.slot < slot.argument_count)
            .then(|| &slot.arguments[usize::from(range.slot)])
            .ok_or(ScratchError::InvalidCoordinate)
    }

    fn pending_slot(&self) -> Result<&MacroSlot, ScratchError> {
        let index = self.pending_slot_index()?;
        self.macro_slots
            .get(index)
            .ok_or(ScratchError::InvalidCoordinate)
    }

    fn pending_slot_mut(&mut self) -> Result<&mut MacroSlot, ScratchError> {
        let index = self.pending_slot_index()?;
        self.macro_slots
            .get_mut(index)
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

    fn sealed_slot(&self, frame: MacroFrameId<G>) -> Result<&MacroSlot, ScratchError> {
        self.slot(frame)
    }

    fn slot(&self, frame: MacroFrameId<G>) -> Result<&MacroSlot, ScratchError> {
        self.macro_slots
            .get(frame.slot as usize)
            .filter(|slot| slot.live && slot.sealed && slot.serial == frame.serial)
            .ok_or(ScratchError::InvalidCoordinate)
    }

    fn range_word(
        &self,
        range: PackedRange,
        index: usize,
    ) -> Result<TracedTokenWord, ScratchError> {
        let absolute = usize::from(range.first_offset)
            .checked_add(index)
            .ok_or(ScratchError::CapacityOverflow)?;
        let mut segment = range.first_segment;
        for _ in 0..(absolute / MACRO_SEGMENT_WORDS) {
            segment = self.next_segment(segment)?;
        }
        let offset = u16::try_from(absolute % MACRO_SEGMENT_WORDS)
            .map_err(|_| ScratchError::CapacityOverflow)?;
        self.segment_word(segment, offset)
    }

    fn chain_word(&self, first_segment: u32, index: u32) -> Result<TracedTokenWord, ScratchError> {
        let mut segment = first_segment;
        for _ in 0..(index as usize / MACRO_SEGMENT_WORDS) {
            segment = self.next_segment(segment)?;
        }
        let offset = u16::try_from(index as usize % MACRO_SEGMENT_WORDS)
            .map_err(|_| ScratchError::CapacityOverflow)?;
        self.segment_word(segment, offset)
    }

    fn segment(&self, segment: u32) -> Result<&MacroWordSegment, ScratchError> {
        self.macro_segments
            .get(segment as usize)
            .filter(|segment| segment.live)
            .ok_or(ScratchError::InvalidCoordinate)
    }

    fn segment_word(&self, segment: u32, offset: u16) -> Result<TracedTokenWord, ScratchError> {
        self.segment(segment)?
            .words
            .get(usize::from(offset))
            .copied()
            .ok_or(ScratchError::InvalidCoordinate)
    }

    fn next_segment(&self, segment: u32) -> Result<u32, ScratchError> {
        let next = self.segment(segment)?.next;
        (next != NO_MACRO_SEGMENT)
            .then_some(next)
            .ok_or(ScratchError::InvalidCoordinate)
    }

    fn advance_segment(&self, segment: u32, offset: u16) -> Result<(u32, u16), ScratchError> {
        if usize::from(offset) + 1 < MACRO_SEGMENT_WORDS {
            Ok((segment, offset + 1))
        } else {
            Ok((self.next_segment(segment)?, 0))
        }
    }

    fn take_macro_segment(&mut self) -> Result<u32, ScratchError> {
        if self.free_macro_segment != NO_MACRO_SEGMENT {
            let index = self.free_macro_segment;
            let segment = &mut self.macro_segments[index as usize];
            if segment.live {
                return Err(ScratchError::InvalidCoordinate);
            }
            self.free_macro_segment = segment.next;
            segment.next = NO_MACRO_SEGMENT;
            segment.live = true;
            return Ok(index);
        }
        let index =
            u32::try_from(self.macro_segments.len()).map_err(|_| ScratchError::CapacityOverflow)?;
        self.macro_segments
            .try_reserve(1)
            .map_err(|_| ScratchError::AllocationFailed)?;
        self.macro_segments.push(MacroWordSegment::new()?);
        Ok(index)
    }

    fn release_delimiter_segments(&mut self) {
        let first = self.delimiter_first_segment;
        self.delimiter_first_segment = NO_MACRO_SEGMENT;
        self.delimiter_last_segment = NO_MACRO_SEGMENT;
        self.release_segment_chain(first)
    }

    fn release_slot(&mut self, frame: MacroFrameId<G>) -> Result<(), ScratchError> {
        if self.macro_depth == 0 || frame.slot != self.active_macro_slot {
            return Err(ScratchError::InvalidCoordinate);
        }
        let slot = self.slot(frame)?;
        let parent_slot = slot.parent_slot;
        self.release_slot_segments(frame.slot)?;
        self.release_macro_slot(frame.slot);
        self.macro_depth -= 1;
        self.active_macro_slot = parent_slot;
        Ok(())
    }

    fn release_macro_slot(&mut self, slot_index: u32) {
        let slot = &mut self.macro_slots[slot_index as usize];
        slot.clear();
        slot.parent_slot = self.free_macro_slot;
        self.free_macro_slot = slot_index;
    }

    fn release_slot_segments(&mut self, slot_index: u32) -> Result<(), ScratchError> {
        let first = self
            .macro_slots
            .get(slot_index as usize)
            .filter(|slot| slot.live)
            .map(|slot| slot.first_segment)
            .ok_or(ScratchError::InvalidCoordinate)?;
        self.release_segment_chain(first);
        Ok(())
    }

    fn release_segment_chain(&mut self, mut segment: u32) {
        while segment != NO_MACRO_SEGMENT {
            let slot = &mut self.macro_segments[segment as usize];
            let next = slot.next;
            slot.words.clear();
            slot.live = false;
            slot.next = self.free_macro_segment;
            self.free_macro_segment = segment;
            segment = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tex_state::token::{Catcode, OriginId, Token};

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
    ) -> MacroFrameId<G> {
        let matching = scratch.begin_macro_match().expect("macro match");
        let mut buffer = scratch
            .begin_match_buffer(&matching)
            .expect("argument buffer");
        for word in words {
            let token = word.semantic_token();
            scratch
                .push_match_word(
                    &mut buffer,
                    word,
                    MacroArgumentTokenFacts {
                        rejects_non_long_paragraph: false,
                        begin_group: matches!(
                            token,
                            Token::Char {
                                cat: Catcode::BeginGroup,
                                ..
                            }
                        ),
                        end_group: matches!(
                            token,
                            Token::Char {
                                cat: Catcode::EndGroup,
                                ..
                            }
                        ),
                    },
                )
                .expect("argument word");
        }
        scratch.finish_match_buffer(buffer).expect("argument range");
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
        let expected = (0..(MACRO_SEGMENT_WORDS * 2 + 3))
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
                .argument_word(range, MACRO_SEGMENT_WORDS * 2 + 3)
                .is_err()
        );
        assert_eq!(scratch.copied_macro_words(), 0);
        scratch.pop_macro_frame(frame).expect("frame retirement");
        assert!(scratch.is_quiescent());
        assert_eq!(scratch.retained_slot_len(), 1);
        assert_eq!(scratch.retained_segment_len(), 3);
    }

    #[test]
    fn first_scan_facts_seal_beside_the_range_without_word_rereads() {
        let mut scratch = ExecutionScratch::<()>::default();
        let matching = scratch.begin_macro_match().expect("macro match");
        let mut buffer = scratch
            .begin_match_buffer(&matching)
            .expect("argument buffer");
        let words = [
            (
                brace('{', Catcode::BeginGroup),
                MacroArgumentTokenFacts {
                    begin_group: true,
                    ..MacroArgumentTokenFacts::default()
                },
            ),
            (
                brace('[', Catcode::BeginGroup),
                MacroArgumentTokenFacts {
                    begin_group: true,
                    ..MacroArgumentTokenFacts::default()
                },
            ),
            (
                word('p'),
                MacroArgumentTokenFacts {
                    rejects_non_long_paragraph: true,
                    ..MacroArgumentTokenFacts::default()
                },
            ),
            (
                brace(']', Catcode::EndGroup),
                MacroArgumentTokenFacts {
                    end_group: true,
                    ..MacroArgumentTokenFacts::default()
                },
            ),
            (
                brace('}', Catcode::EndGroup),
                MacroArgumentTokenFacts {
                    end_group: true,
                    ..MacroArgumentTokenFacts::default()
                },
            ),
        ];
        for (word, facts) in words {
            scratch
                .push_match_word(&mut buffer, word, facts)
                .expect("argument word");
        }
        let facts = scratch
            .match_argument_facts(&buffer)
            .expect("pending facts");
        assert!(facts.rejects_non_long_paragraph());
        assert!(facts.removable_outer_group());
        assert_eq!(scratch.match_word_reads(), 0);

        scratch
            .strip_match_outer_group(&buffer)
            .expect("outer group strips by metadata");
        scratch.finish_match_buffer(buffer).expect("argument range");
        let frame = scratch.commit_macro_match(matching).expect("sealed frame");
        let range = scratch
            .argument_range(frame, 1)
            .expect("live frame")
            .expect("first argument");
        assert_eq!(scratch.argument_len(range), Ok(3));
        assert_eq!(
            scratch.argument_facts(range),
            Ok(MacroArgumentFacts {
                rejects_non_long_paragraph: true,
                removable_outer_group: true,
            })
        );
        assert_eq!(scratch.match_word_reads(), 0);

        scratch.pop_macro_frame(frame).expect("frame retirement");
        assert_eq!(
            scratch.argument_facts(range),
            Err(ScratchError::InvalidCoordinate)
        );
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
    fn repeated_same_depth_replacement_reuses_one_slot_without_copying() {
        let mut scratch = ExecutionScratch::<()>::default();
        let mut frame = seal_argument(&mut scratch, [word('a')]);
        for index in 0..8_192 {
            let matching = scratch.begin_macro_match().expect("replacement match");
            let mut buffer = scratch
                .begin_match_buffer(&matching)
                .expect("replacement argument");
            scratch
                .push_match_word(
                    &mut buffer,
                    word(if index % 2 == 0 { 'b' } else { 'c' }),
                    MacroArgumentTokenFacts::default(),
                )
                .expect("replacement word");
            scratch
                .finish_match_buffer(buffer)
                .expect("replacement range");
            scratch.pop_macro_frame(frame).expect("tail retirement");
            frame = scratch
                .commit_macro_match(matching)
                .expect("replacement activation");
        }
        assert_eq!(scratch.frame_len(), 1);
        assert_eq!(scratch.retained_slot_len(), 2);
        assert_eq!(scratch.retained_segment_len(), 2);
        assert_eq!(scratch.copied_macro_words(), 0);
        scratch.pop_macro_frame(frame).expect("final retirement");
        assert!(scratch.is_quiescent());
    }

    #[test]
    fn discarded_match_releases_its_slot_without_touching_parent_output() {
        let mut scratch = ExecutionScratch::<()>::default();
        let parent = seal_argument(&mut scratch, [word('p')]);
        let rejected = scratch.begin_macro_match().expect("rejected match");
        let mut buffer = scratch
            .begin_match_buffer(&rejected)
            .expect("rejected argument");
        scratch
            .push_match_word(&mut buffer, word('x'), MacroArgumentTokenFacts::default())
            .expect("rejected word");
        scratch
            .discard_macro_match(rejected)
            .expect("rollback releases match");
        let range = scratch
            .argument_range(parent, 1)
            .expect("parent frame")
            .expect("parent argument");
        assert_eq!(scratch.argument_word(range, 0), Ok(word('p')));
        scratch.pop_macro_frame(parent).expect("parent retirement");
        assert!(scratch.is_quiescent());
    }

    #[test]
    fn discarded_tail_match_releases_after_its_parent_retires() {
        let mut scratch = ExecutionScratch::<()>::default();
        let parent = seal_argument(&mut scratch, [word('p')]);
        let rejected = scratch.begin_macro_match().expect("rejected tail match");
        let mut buffer = scratch
            .begin_match_buffer(&rejected)
            .expect("rejected tail argument");
        scratch
            .push_match_word(&mut buffer, word('x'), MacroArgumentTokenFacts::default())
            .expect("rejected tail word");
        scratch
            .pop_macro_frame(parent)
            .expect("parent retires beneath pending frame");
        scratch
            .discard_macro_match(rejected)
            .expect("rollback releases canonical pending frame");
        assert!(scratch.is_quiescent());
        assert_eq!(scratch.retained_slot_len(), 2);
        assert_eq!(scratch.retained_segment_len(), 2);
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
            (0..(MACRO_SEGMENT_WORDS + 1)).map(|_| word('c')),
        );
        assert_eq!(
            scratch.pop_macro_frame(parent),
            Err(ScratchError::InvalidCoordinate)
        );
        assert_eq!(scratch.argument_word(parent_range, 0), Ok(word('p')));
        scratch.pop_macro_frame(child).expect("child retirement");
        scratch.pop_macro_frame(parent).expect("parent retirement");
        assert_eq!(
            scratch.argument_word(parent_range, 0),
            Err(ScratchError::InvalidCoordinate)
        );

        let replacement = seal_argument(&mut scratch, [word('r')]);
        assert_eq!(replacement.slot, parent.slot);
        assert_ne!(replacement.serial, parent.serial);
        assert_eq!(
            scratch.argument_word(parent_range, 0),
            Err(ScratchError::InvalidCoordinate)
        );
        scratch
            .pop_macro_frame(replacement)
            .expect("replacement retirement");
    }

    #[cfg(feature = "profiling")]
    #[test]
    fn warmed_8192_same_depth_replacements_allocate_zero_heap() {
        let mut scratch = ExecutionScratch::<()>::default();
        let mut frame = seal_argument(&mut scratch, [word('a')]);
        let replace = |scratch: &mut ExecutionScratch<()>, frame: MacroFrameId<()>| {
            let matching = scratch.begin_macro_match().expect("replacement match");
            let mut buffer = scratch
                .begin_match_buffer(&matching)
                .expect("replacement buffer");
            scratch
                .push_match_word(&mut buffer, word('b'), MacroArgumentTokenFacts::default())
                .expect("replacement word");
            scratch
                .finish_match_buffer(buffer)
                .expect("replacement range");
            scratch.pop_macro_frame(frame).expect("prior retirement");
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
        assert_eq!(scratch.retained_segment_len(), 2);
        assert_eq!(scratch.copied_macro_words(), 0);
        scratch.pop_macro_frame(frame).expect("final retirement");
    }
}
