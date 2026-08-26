//! Generation-owned reusable command execution scratch.
//!
//! Macro matchers write into one reusable segmented staging lane. Sealing
//! moves whole segment owners onto an absolute-offset live bump stack without
//! copying token words; strict-LIFO retirement rewinds that stack and returns
//! the physical segments to the shared high-water pool. No macro invocation
//! owns a heap buffer or an attempt-arena scope.

use core::hash::{Hash, Hasher};
use core::marker::PhantomData;

use tex_state::token::TracedTokenWord;

const MACRO_SEGMENT_WORDS: usize = 4_096;

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
    start: u32,
    len: u32,
}

impl<G> Copy for MacroArgumentRange<G> {}
impl<G> Clone for MacroArgumentRange<G> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<G> PartialEq for MacroArgumentRange<G> {
    fn eq(&self, other: &Self) -> bool {
        self.frame == other.frame && self.start == other.start && self.len == other.len
    }
}
impl<G> Eq for MacroArgumentRange<G> {}
impl<G> Hash for MacroArgumentRange<G> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.frame.hash(state);
        self.start.hash(state);
        self.len.hash(state);
    }
}

#[derive(Debug)]
pub(crate) struct MacroMatch<G> {
    serial: u64,
    _generation: PhantomData<fn(&G) -> &G>,
}

#[derive(Debug)]
pub(crate) struct MacroMatchBuffer<G> {
    serial: u64,
    start_word: u32,
    end_word: Option<u32>,
    _generation: PhantomData<fn(&G) -> &G>,
}

impl<G> Copy for MacroMatchBuffer<G> {}
impl<G> Clone for MacroMatchBuffer<G> {
    fn clone(&self) -> Self {
        *self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PackedRange {
    start: u32,
    len: u32,
}

#[derive(Debug, Default, Eq, PartialEq)]
struct MacroSlot {
    serial: u64,
    watermark_segments: u32,
    word_len: u32,
    arguments: [Option<PackedRange>; 9],
    argument_count: u8,
    live: bool,
}

#[derive(Debug, Eq, PartialEq)]
struct MacroWordSegment {
    words: Vec<TracedTokenWord>,
}

impl MacroWordSegment {
    fn new() -> Result<Self, ScratchError> {
        let mut words = Vec::new();
        words
            .try_reserve_exact(MACRO_SEGMENT_WORDS)
            .map_err(|_| ScratchError::AllocationFailed)?;
        Ok(Self { words })
    }
}

#[derive(Debug, Eq, PartialEq)]
struct PendingMacroMatch {
    serial: u64,
    arguments: [Option<PackedRange>; 9],
    argument_count: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScratchError {
    InvalidCoordinate,
    CapacityOverflow,
    AllocationFailed,
}

pub(crate) struct MacroWords<'a, G> {
    scratch: &'a ExecutionScratch<G>,
    serial: u64,
    current: u32,
    remaining: u32,
}

impl<G> Clone for MacroWords<'_, G> {
    fn clone(&self) -> Self {
        Self {
            scratch: self.scratch,
            serial: self.serial,
            current: self.current,
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
        let word = self.scratch.match_word(self.serial, self.current).ok()?;
        self.current += 1;
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
    macro_segments: Vec<MacroWordSegment>,
    match_segments: Vec<MacroWordSegment>,
    match_word_len: u32,
    pending_macro_match: Option<PendingMacroMatch>,
    delimiter_segments: Vec<MacroWordSegment>,
    spare_macro_segments: Vec<MacroWordSegment>,
    next_macro_serial: u64,
    delimiter_head: u32,
    delimiter_len: u32,
    scanner_resumes: ResumeFrameLane<ContinuationFrame<G>, G>,
    expression_frames: Vec<crate::scanners::ExpressionFrame<G>>,
    _generation: PhantomData<fn(&G) -> &G>,
    #[cfg(test)]
    copied_macro_words: u64,
}

impl<G> Default for ExecutionScratch<G> {
    fn default() -> Self {
        Self {
            macro_slots: Vec::new(),
            macro_depth: 0,
            macro_segments: Vec::new(),
            match_segments: Vec::new(),
            match_word_len: 0,
            pending_macro_match: None,
            delimiter_segments: Vec::new(),
            spare_macro_segments: Vec::new(),
            next_macro_serial: 1,
            delimiter_head: 0,
            delimiter_len: 0,
            scanner_resumes: ResumeFrameLane::default(),
            expression_frames: Vec::new(),
            _generation: PhantomData,
            #[cfg(test)]
            copied_macro_words: 0,
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
        if self.delimiter_len != 0
            || self.pending_macro_match.is_some()
            || self.match_word_len != 0
            || !self.match_segments.is_empty()
        {
            return Err(ScratchError::InvalidCoordinate);
        }
        let serial = self.next_macro_serial;
        self.next_macro_serial = self.next_macro_serial.wrapping_add(1).max(1);
        self.pending_macro_match = Some(PendingMacroMatch {
            serial,
            arguments: [None; 9],
            argument_count: 0,
        });
        Ok(MacroMatch {
            serial,
            _generation: PhantomData,
        })
    }

    pub(crate) fn begin_match_buffer(
        &self,
        matching: &MacroMatch<G>,
    ) -> Result<MacroMatchBuffer<G>, ScratchError> {
        self.validate_matching(matching)?;
        Ok(MacroMatchBuffer {
            serial: matching.serial,
            start_word: self.match_word_len,
            end_word: None,
            _generation: PhantomData,
        })
    }

    pub(crate) fn push_match_word(
        &mut self,
        matching: &MacroMatch<G>,
        buffer: MacroMatchBuffer<G>,
        word: TracedTokenWord,
    ) -> Result<(), ScratchError> {
        self.validate_buffer(matching, buffer)?;
        if buffer.end_word.is_some() {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.push_match_segment_word(word)
    }

    pub(crate) fn match_words(
        &self,
        matching: &MacroMatch<G>,
        buffer: MacroMatchBuffer<G>,
    ) -> Result<MacroWords<'_, G>, ScratchError> {
        self.validate_buffer(matching, buffer)?;
        let end = buffer.end_word.unwrap_or(self.match_word_len);
        Ok(MacroWords {
            scratch: self,
            serial: matching.serial,
            current: buffer.start_word,
            remaining: end - buffer.start_word,
        })
    }

    pub(crate) fn strip_match_outer_group(
        &self,
        matching: &MacroMatch<G>,
        buffer: MacroMatchBuffer<G>,
    ) -> Result<MacroMatchBuffer<G>, ScratchError> {
        self.validate_buffer(matching, buffer)?;
        if buffer.end_word.is_some() {
            return Err(ScratchError::InvalidCoordinate);
        }
        let end = self.match_word_len;
        if end.saturating_sub(buffer.start_word) < 2 {
            return Err(ScratchError::InvalidCoordinate);
        }
        Ok(MacroMatchBuffer {
            serial: buffer.serial,
            start_word: buffer.start_word + 1,
            end_word: Some(end - 1),
            _generation: PhantomData,
        })
    }

    pub(crate) fn finish_match_buffer(
        &mut self,
        matching: &MacroMatch<G>,
        buffer: MacroMatchBuffer<G>,
    ) -> Result<(), ScratchError> {
        self.validate_buffer(matching, buffer)?;
        let end = buffer.end_word.unwrap_or(self.match_word_len);
        let range = PackedRange {
            start: buffer.start_word,
            len: end - buffer.start_word,
        };
        let pending = self
            .pending_macro_match
            .as_mut()
            .ok_or(ScratchError::InvalidCoordinate)?;
        if pending.serial != matching.serial || pending.argument_count >= 9 {
            return Err(ScratchError::InvalidCoordinate);
        }
        pending.arguments[pending.argument_count as usize] = Some(range);
        pending.argument_count += 1;
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
        Self::segmented_word(&self.delimiter_segments, index)
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
            self.delimiter_segments
                .try_reserve(1)
                .map_err(|_| ScratchError::AllocationFailed)?;
            let segment = self.take_macro_segment()?;
            self.delimiter_segments.push(segment);
        }
        self.delimiter_segments
            .last_mut()
            .ok_or(ScratchError::InvalidCoordinate)?
            .words
            .push(word);
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
        let word = Self::segmented_word(&self.delimiter_segments, self.delimiter_head)?;
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
        matching: MacroMatch<G>,
    ) -> Result<MacroFrameId<G>, ScratchError> {
        if self.delimiter_len != 0 {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.validate_matching(&matching)?;
        let pending = self
            .pending_macro_match
            .take()
            .ok_or(ScratchError::InvalidCoordinate)?;
        let slot_index = self.macro_depth;
        let slot_index_usize = slot_index as usize;
        if slot_index_usize == self.macro_slots.len() {
            self.macro_slots
                .try_reserve(1)
                .map_err(|_| ScratchError::AllocationFailed)?;
            self.macro_slots.push(MacroSlot::default());
        }
        let watermark_segments =
            u32::try_from(self.macro_segments.len()).map_err(|_| ScratchError::CapacityOverflow)?;
        let base = watermark_segments
            .checked_mul(MACRO_SEGMENT_WORDS as u32)
            .ok_or(ScratchError::CapacityOverflow)?;
        let mut arguments = pending.arguments;
        for range in arguments.iter_mut().flatten() {
            range.start = base
                .checked_add(range.start)
                .ok_or(ScratchError::CapacityOverflow)?;
        }
        let slot = &mut self.macro_slots[slot_index_usize];
        if slot.live {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.macro_segments
            .try_reserve(self.match_segments.len())
            .map_err(|_| ScratchError::AllocationFailed)?;
        self.macro_segments.append(&mut self.match_segments);
        *slot = MacroSlot {
            serial: pending.serial,
            watermark_segments,
            word_len: self.match_word_len,
            arguments,
            argument_count: pending.argument_count,
            live: true,
        };
        self.match_word_len = 0;
        self.macro_depth = self
            .macro_depth
            .checked_add(1)
            .ok_or(ScratchError::CapacityOverflow)?;
        let frame = MacroFrameId {
            slot: slot_index,
            serial: matching.serial,
            _generation: PhantomData,
        };
        Ok(frame)
    }

    pub(crate) fn discard_macro_match(
        &mut self,
        matching: MacroMatch<G>,
    ) -> Result<(), ScratchError> {
        self.validate_matching(&matching)?;
        self.pending_macro_match = None;
        self.match_word_len = 0;
        self.release_match_segments();
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
            self.sealed_slot(frame)?.arguments[usize::from(slot - 1)].map(|range| {
                MacroArgumentRange {
                    frame,
                    start: range.start,
                    len: range.len,
                }
            }),
        )
    }

    pub(crate) fn argument_len(&self, range: MacroArgumentRange<G>) -> Result<usize, ScratchError> {
        self.validate_argument_range(range)?;
        Ok(range.len as usize)
    }

    pub(crate) fn argument_word(
        &self,
        range: MacroArgumentRange<G>,
        index: usize,
    ) -> Result<TracedTokenWord, ScratchError> {
        self.validate_argument_range(range)?;
        if index >= range.len as usize {
            return Err(ScratchError::InvalidCoordinate);
        }
        let absolute = range
            .start
            .checked_add(u32::try_from(index).map_err(|_| ScratchError::CapacityOverflow)?)
            .ok_or(ScratchError::CapacityOverflow)?;
        Self::segmented_word(&self.macro_segments, absolute)
    }

    pub(crate) fn argument_word_len(&self) -> usize {
        self.macro_slots[..self.macro_depth as usize]
            .iter()
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
            + self.match_segments.len()
            + self.delimiter_segments.len()
            + self.spare_macro_segments.len()
    }

    pub(crate) fn is_quiescent(&self) -> bool {
        self.frame_len() == 0
            && self.pending_macro_match.is_none()
            && self.match_word_len == 0
            && self.delimiter_len == 0
            && self.scanner_resumes.live_len() == 0
    }

    #[cfg(test)]
    pub(crate) const fn copied_macro_words(&self) -> u64 {
        self.copied_macro_words
    }

    fn validate_matching(&self, matching: &MacroMatch<G>) -> Result<(), ScratchError> {
        matches!(
            self.pending_macro_match,
            Some(PendingMacroMatch { serial, .. }) if serial == matching.serial
        )
        .then_some(())
        .ok_or(ScratchError::InvalidCoordinate)
    }

    fn validate_buffer(
        &self,
        matching: &MacroMatch<G>,
        buffer: MacroMatchBuffer<G>,
    ) -> Result<(), ScratchError> {
        self.validate_matching(matching)?;
        let end = buffer.end_word.unwrap_or(self.match_word_len);
        if buffer.serial != matching.serial || buffer.start_word > end || end > self.match_word_len
        {
            return Err(ScratchError::InvalidCoordinate);
        }
        Ok(())
    }

    fn validate_argument_range(&self, range: MacroArgumentRange<G>) -> Result<(), ScratchError> {
        self.sealed_slot(range.frame)?
            .arguments
            .iter()
            .flatten()
            .any(|candidate| candidate.start == range.start && candidate.len == range.len)
            .then_some(())
            .ok_or(ScratchError::InvalidCoordinate)
    }

    fn sealed_slot(&self, frame: MacroFrameId<G>) -> Result<&MacroSlot, ScratchError> {
        self.slot(frame)
    }

    fn slot(&self, frame: MacroFrameId<G>) -> Result<&MacroSlot, ScratchError> {
        self.macro_slots
            .get(frame.slot as usize)
            .filter(|slot| {
                frame.slot < self.macro_depth && slot.live && slot.serial == frame.serial
            })
            .ok_or(ScratchError::InvalidCoordinate)
    }

    fn slot_mut(&mut self, frame: MacroFrameId<G>) -> Result<&mut MacroSlot, ScratchError> {
        self.macro_slots
            .get_mut(frame.slot as usize)
            .filter(|slot| slot.live && slot.serial == frame.serial)
            .ok_or(ScratchError::InvalidCoordinate)
    }

    fn push_match_segment_word(&mut self, word: TracedTokenWord) -> Result<(), ScratchError> {
        if (self.match_word_len as usize).is_multiple_of(MACRO_SEGMENT_WORDS) {
            self.match_segments
                .try_reserve(1)
                .map_err(|_| ScratchError::AllocationFailed)?;
            let segment = self.take_macro_segment()?;
            self.match_segments.push(segment);
        }
        self.match_segments
            .last_mut()
            .ok_or(ScratchError::InvalidCoordinate)?
            .words
            .push(word);
        self.match_word_len = self
            .match_word_len
            .checked_add(1)
            .ok_or(ScratchError::CapacityOverflow)?;
        Ok(())
    }

    fn match_word(&self, serial: u64, index: u32) -> Result<TracedTokenWord, ScratchError> {
        if self
            .pending_macro_match
            .as_ref()
            .is_none_or(|pending| pending.serial != serial)
            || index >= self.match_word_len
        {
            return Err(ScratchError::InvalidCoordinate);
        }
        Self::segmented_word(&self.match_segments, index)
    }

    fn segmented_word(
        segments: &[MacroWordSegment],
        absolute: u32,
    ) -> Result<TracedTokenWord, ScratchError> {
        let absolute = absolute as usize;
        segments
            .get(absolute / MACRO_SEGMENT_WORDS)
            .and_then(|segment| segment.words.get(absolute % MACRO_SEGMENT_WORDS))
            .copied()
            .ok_or(ScratchError::InvalidCoordinate)
    }

    fn take_macro_segment(&mut self) -> Result<MacroWordSegment, ScratchError> {
        if let Some(segment) = self.spare_macro_segments.pop() {
            return Ok(segment);
        }
        let physical_count = self
            .macro_segments
            .len()
            .saturating_add(self.match_segments.len())
            .saturating_add(self.delimiter_segments.len())
            .saturating_add(self.spare_macro_segments.len());
        let required_capacity = physical_count
            .checked_add(1)
            .ok_or(ScratchError::CapacityOverflow)?;
        if self.spare_macro_segments.capacity() < required_capacity {
            self.spare_macro_segments
                .try_reserve_exact(required_capacity - self.spare_macro_segments.len())
                .map_err(|_| ScratchError::AllocationFailed)?;
        }
        MacroWordSegment::new()
    }

    fn release_match_segments(&mut self) {
        while let Some(mut segment) = self.match_segments.pop() {
            segment.words.clear();
            self.spare_macro_segments.push(segment);
        }
    }

    fn release_delimiter_segments(&mut self) {
        while let Some(mut segment) = self.delimiter_segments.pop() {
            segment.words.clear();
            self.spare_macro_segments.push(segment);
        }
    }

    fn release_slot(&mut self, frame: MacroFrameId<G>) -> Result<(), ScratchError> {
        if self.macro_depth == 0 || frame.slot + 1 != self.macro_depth {
            return Err(ScratchError::InvalidCoordinate);
        }
        let slot = self.slot(frame)?;
        let watermark = slot.watermark_segments as usize;
        *self.slot_mut(frame)? = MacroSlot::default();
        while self.macro_segments.len() > watermark {
            let mut segment = self
                .macro_segments
                .pop()
                .ok_or(ScratchError::InvalidCoordinate)?;
            segment.words.clear();
            self.spare_macro_segments.push(segment);
        }
        self.macro_depth -= 1;
        Ok(())
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

    fn seal_argument<G>(
        scratch: &mut ExecutionScratch<G>,
        words: impl IntoIterator<Item = TracedTokenWord>,
    ) -> MacroFrameId<G> {
        let matching = scratch.begin_macro_match().expect("macro match");
        let buffer = scratch
            .begin_match_buffer(&matching)
            .expect("argument buffer");
        for word in words {
            scratch
                .push_match_word(&matching, buffer, word)
                .expect("argument word");
        }
        scratch
            .finish_match_buffer(&matching, buffer)
            .expect("argument range");
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
    fn repeated_same_depth_replacement_reuses_one_slot_without_copying() {
        let mut scratch = ExecutionScratch::<()>::default();
        let mut frame = seal_argument(&mut scratch, [word('a')]);
        for index in 0..8_192 {
            let matching = scratch.begin_macro_match().expect("replacement match");
            let buffer = scratch
                .begin_match_buffer(&matching)
                .expect("replacement argument");
            scratch
                .push_match_word(
                    &matching,
                    buffer,
                    word(if index % 2 == 0 { 'b' } else { 'c' }),
                )
                .expect("replacement word");
            scratch
                .finish_match_buffer(&matching, buffer)
                .expect("replacement range");
            scratch.pop_macro_frame(frame).expect("tail retirement");
            frame = scratch
                .commit_macro_match(matching)
                .expect("replacement activation");
        }
        assert_eq!(scratch.frame_len(), 1);
        assert_eq!(scratch.retained_slot_len(), 1);
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
        let buffer = scratch
            .begin_match_buffer(&rejected)
            .expect("rejected argument");
        scratch
            .push_match_word(&rejected, buffer, word('x'))
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
    fn nested_frames_rewind_strictly_and_reject_stale_absolute_ranges() {
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
            let buffer = scratch
                .begin_match_buffer(&matching)
                .expect("replacement buffer");
            scratch
                .push_match_word(&matching, buffer, word('b'))
                .expect("replacement word");
            scratch
                .finish_match_buffer(&matching, buffer)
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
        assert_eq!(scratch.retained_slot_len(), 1);
        assert_eq!(scratch.retained_segment_len(), 2);
        assert_eq!(scratch.copied_macro_words(), 0);
        scratch.pop_macro_frame(frame).expect("final retirement");
    }
}
