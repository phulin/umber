//! Generation-owned reusable command execution scratch.
//!
//! Macro matchers write into linked fixed-size chunks drawn from one coarse
//! pool. Sealing transfers only a private slot descriptor to the activation;
//! retirement splices the whole chain onto an intrusive free list in O(1).
//! No macro invocation owns a heap buffer or an attempt-arena scope.

use core::hash::{Hash, Hasher};
use core::marker::PhantomData;

use tex_state::token::{OriginId, Token, TracedTokenWord};

const MACRO_CHUNK_WORDS: usize = 64;
const NO_CHUNK: u32 = u32::MAX;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ChunkCursor {
    chunk: u32,
    offset: u8,
}

impl ChunkCursor {
    const EMPTY: Self = Self {
        chunk: NO_CHUNK,
        offset: 0,
    };
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
    start: ChunkCursor,
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
        self.start.chunk.hash(state);
        self.start.offset.hash(state);
        self.len.hash(state);
    }
}

#[derive(Debug)]
pub(crate) struct MacroMatch<G> {
    frame: MacroFrameId<G>,
}

#[derive(Debug)]
pub(crate) struct MacroMatchBuffer<G> {
    frame: MacroFrameId<G>,
    start_word: u32,
    end_word: Option<u32>,
}

impl<G> Copy for MacroMatchBuffer<G> {}
impl<G> Clone for MacroMatchBuffer<G> {
    fn clone(&self) -> Self {
        *self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PackedRange {
    start: ChunkCursor,
    len: u32,
}

#[derive(Debug, Eq, PartialEq)]
struct MacroSlot {
    serial: u64,
    head: u32,
    tail: u32,
    word_len: u32,
    arguments: [Option<PackedRange>; 9],
    argument_count: u8,
    live: bool,
    sealed: bool,
}

impl Default for MacroSlot {
    fn default() -> Self {
        Self {
            serial: 0,
            head: NO_CHUNK,
            tail: NO_CHUNK,
            word_len: 0,
            arguments: [None; 9],
            argument_count: 0,
            live: false,
            sealed: false,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct MacroWordChunk {
    words: [TracedTokenWord; MACRO_CHUNK_WORDS],
    len: u8,
    next: u32,
}

impl Default for MacroWordChunk {
    fn default() -> Self {
        Self {
            words: [TracedTokenWord::pack(Token::Param(1), OriginId::UNKNOWN); MACRO_CHUNK_WORDS],
            len: 0,
            next: NO_CHUNK,
        }
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
    frame: MacroFrameId<G>,
    current: ChunkCursor,
    remaining: u32,
    require_sealed: bool,
}

impl<G> Clone for MacroWords<'_, G> {
    fn clone(&self) -> Self {
        Self {
            scratch: self.scratch,
            frame: self.frame,
            current: self.current,
            remaining: self.remaining,
            require_sealed: self.require_sealed,
        }
    }
}

impl<G> Iterator for MacroWords<'_, G> {
    type Item = TracedTokenWord;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let word = self
            .scratch
            .cursor_word(self.frame, self.current, self.require_sealed)
            .ok()?;
        self.current = self.scratch.next_cursor(self.current).ok()?;
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
    macro_free_slots: Vec<u32>,
    macro_chunks: Vec<MacroWordChunk>,
    free_chunk_head: u32,
    next_macro_serial: u64,
    delimiter_head: ChunkCursor,
    delimiter_tail: u32,
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
            macro_free_slots: Vec::new(),
            macro_chunks: Vec::new(),
            free_chunk_head: NO_CHUNK,
            next_macro_serial: 1,
            delimiter_head: ChunkCursor::EMPTY,
            delimiter_tail: NO_CHUNK,
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
        if self.delimiter_len != 0 {
            return Err(ScratchError::InvalidCoordinate);
        }
        let index = if let Some(index) = self.macro_free_slots.pop() {
            index
        } else {
            let index = u32::try_from(self.macro_slots.len())
                .map_err(|_| ScratchError::CapacityOverflow)?;
            self.macro_slots
                .try_reserve(1)
                .map_err(|_| ScratchError::AllocationFailed)?;
            self.macro_free_slots
                .try_reserve(1)
                .map_err(|_| ScratchError::AllocationFailed)?;
            self.macro_slots.push(MacroSlot::default());
            index
        };
        let serial = self.next_macro_serial;
        self.next_macro_serial = self.next_macro_serial.wrapping_add(1).max(1);
        let slot = &mut self.macro_slots[index as usize];
        if slot.live {
            return Err(ScratchError::InvalidCoordinate);
        }
        *slot = MacroSlot {
            serial,
            live: true,
            ..MacroSlot::default()
        };
        Ok(MacroMatch {
            frame: MacroFrameId {
                slot: index,
                serial,
                _generation: PhantomData,
            },
        })
    }

    pub(crate) fn begin_match_buffer(
        &self,
        matching: &MacroMatch<G>,
    ) -> Result<MacroMatchBuffer<G>, ScratchError> {
        Ok(MacroMatchBuffer {
            frame: matching.frame,
            start_word: self.matching_slot(matching)?.word_len,
            end_word: None,
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
        self.push_frame_word(matching.frame, word)
    }

    pub(crate) fn match_words(
        &self,
        matching: &MacroMatch<G>,
        buffer: MacroMatchBuffer<G>,
    ) -> Result<MacroWords<'_, G>, ScratchError> {
        self.validate_buffer(matching, buffer)?;
        let slot = self.matching_slot(matching)?;
        let end = buffer.end_word.unwrap_or(slot.word_len);
        Ok(MacroWords {
            scratch: self,
            frame: matching.frame,
            current: self.cursor_at(slot, buffer.start_word)?,
            remaining: end - buffer.start_word,
            require_sealed: false,
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
        let end = self.matching_slot(matching)?.word_len;
        if end.saturating_sub(buffer.start_word) < 2 {
            return Err(ScratchError::InvalidCoordinate);
        }
        Ok(MacroMatchBuffer {
            frame: buffer.frame,
            start_word: buffer.start_word + 1,
            end_word: Some(end - 1),
        })
    }

    pub(crate) fn finish_match_buffer(
        &mut self,
        matching: &MacroMatch<G>,
        buffer: MacroMatchBuffer<G>,
    ) -> Result<(), ScratchError> {
        self.validate_buffer(matching, buffer)?;
        let slot = self.matching_slot(matching)?;
        let end = buffer.end_word.unwrap_or(slot.word_len);
        let range = PackedRange {
            start: self.cursor_at(slot, buffer.start_word)?,
            len: end - buffer.start_word,
        };
        let slot = self.matching_slot_mut(matching)?;
        if slot.argument_count >= 9 {
            return Err(ScratchError::InvalidCoordinate);
        }
        slot.arguments[slot.argument_count as usize] = Some(range);
        slot.argument_count += 1;
        Ok(())
    }

    pub(crate) fn clear_delimiter_prefix(&mut self) {
        let head = self.delimiter_head.chunk;
        let tail = self.delimiter_tail;
        self.delimiter_head = ChunkCursor::EMPTY;
        self.delimiter_tail = NO_CHUNK;
        self.delimiter_len = 0;
        self.release_chunk_chain(head, tail);
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
        let mut cursor = self.delimiter_head;
        for _ in 0..index {
            cursor = self.next_cursor(cursor)?;
        }
        self.raw_cursor_word(cursor)
    }

    pub(crate) fn delimiter_prefix_words(&self) -> impl Iterator<Item = TracedTokenWord> + '_ {
        let mut cursor = self.delimiter_head;
        let mut remaining = self.delimiter_len;
        core::iter::from_fn(move || {
            if remaining == 0 {
                return None;
            }
            let word = self.raw_cursor_word(cursor).ok()?;
            cursor = self.next_cursor(cursor).ok()?;
            remaining -= 1;
            Some(word)
        })
    }

    pub(crate) fn push_delimiter_prefix(
        &mut self,
        word: TracedTokenWord,
    ) -> Result<(), ScratchError> {
        let (head, tail, len) = self.push_chain_word(
            self.delimiter_head.chunk,
            self.delimiter_tail,
            self.delimiter_len,
            word,
        )?;
        if self.delimiter_len == 0 {
            self.delimiter_head = ChunkCursor {
                chunk: head,
                offset: 0,
            };
        }
        self.delimiter_tail = tail;
        self.delimiter_len = len;
        Ok(())
    }

    pub(crate) fn pop_delimiter_prefix_word(&mut self) -> Result<TracedTokenWord, ScratchError> {
        if self.delimiter_len == 0 {
            return Err(ScratchError::InvalidCoordinate);
        }
        let cursor = self.delimiter_head;
        let word = self.raw_cursor_word(cursor)?;
        let next = self.next_cursor(cursor)?;
        self.delimiter_len -= 1;
        if next.chunk != cursor.chunk {
            self.push_free_chunk(cursor.chunk);
        }
        if self.delimiter_len == 0 {
            if next.chunk == cursor.chunk {
                self.push_free_chunk(cursor.chunk);
            }
            self.delimiter_head = ChunkCursor::EMPTY;
            self.delimiter_tail = NO_CHUNK;
        } else {
            self.delimiter_head = next;
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
        let frame = matching.frame;
        self.matching_slot_mut(&matching)?.sealed = true;
        Ok(frame)
    }

    pub(crate) fn discard_macro_match(
        &mut self,
        matching: MacroMatch<G>,
    ) -> Result<(), ScratchError> {
        self.clear_delimiter_prefix();
        self.release_slot(matching.frame, false)
    }

    pub(crate) fn pop_macro_frame(&mut self, frame: MacroFrameId<G>) -> Result<(), ScratchError> {
        self.release_slot(frame, true)
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
        let mut cursor = range.start;
        for _ in 0..index {
            cursor = self.next_cursor(cursor)?;
        }
        self.cursor_word(range.frame, cursor, true)
    }

    pub(crate) fn argument_word_len(&self) -> usize {
        self.macro_slots
            .iter()
            .filter(|slot| slot.live)
            .map(|slot| slot.word_len as usize)
            .sum()
    }

    pub(crate) fn frame_len(&self) -> usize {
        self.macro_slots.iter().filter(|slot| slot.live).count()
    }

    #[cfg(test)]
    pub(crate) fn retained_slot_len(&self) -> usize {
        self.macro_slots.len()
    }

    #[cfg(test)]
    pub(crate) fn retained_chunk_len(&self) -> usize {
        self.macro_chunks.len()
    }

    pub(crate) fn is_quiescent(&self) -> bool {
        self.frame_len() == 0 && self.delimiter_len == 0 && self.scanner_resumes.live_len() == 0
    }

    #[cfg(test)]
    pub(crate) const fn copied_macro_words(&self) -> u64 {
        self.copied_macro_words
    }

    fn matching_slot(&self, matching: &MacroMatch<G>) -> Result<&MacroSlot, ScratchError> {
        let slot = self.slot(matching.frame)?;
        (!slot.sealed)
            .then_some(slot)
            .ok_or(ScratchError::InvalidCoordinate)
    }

    fn matching_slot_mut(
        &mut self,
        matching: &MacroMatch<G>,
    ) -> Result<&mut MacroSlot, ScratchError> {
        let slot = self.slot_mut(matching.frame)?;
        (!slot.sealed)
            .then_some(slot)
            .ok_or(ScratchError::InvalidCoordinate)
    }

    fn validate_buffer(
        &self,
        matching: &MacroMatch<G>,
        buffer: MacroMatchBuffer<G>,
    ) -> Result<(), ScratchError> {
        let slot = self.matching_slot(matching)?;
        let end = buffer.end_word.unwrap_or(slot.word_len);
        if buffer.frame != matching.frame || buffer.start_word > end || end > slot.word_len {
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
        let slot = self.slot(frame)?;
        slot.sealed
            .then_some(slot)
            .ok_or(ScratchError::InvalidCoordinate)
    }

    fn slot(&self, frame: MacroFrameId<G>) -> Result<&MacroSlot, ScratchError> {
        self.macro_slots
            .get(frame.slot as usize)
            .filter(|slot| slot.live && slot.serial == frame.serial)
            .ok_or(ScratchError::InvalidCoordinate)
    }

    fn slot_mut(&mut self, frame: MacroFrameId<G>) -> Result<&mut MacroSlot, ScratchError> {
        self.macro_slots
            .get_mut(frame.slot as usize)
            .filter(|slot| slot.live && slot.serial == frame.serial)
            .ok_or(ScratchError::InvalidCoordinate)
    }

    fn cursor_at(&self, slot: &MacroSlot, index: u32) -> Result<ChunkCursor, ScratchError> {
        if index > slot.word_len {
            return Err(ScratchError::InvalidCoordinate);
        }
        if index == slot.word_len {
            return if slot.tail == NO_CHUNK {
                Ok(ChunkCursor::EMPTY)
            } else {
                Ok(ChunkCursor {
                    chunk: slot.tail,
                    offset: self.macro_chunks[slot.tail as usize].len,
                })
            };
        }
        let mut cursor = ChunkCursor {
            chunk: slot.head,
            offset: 0,
        };
        for _ in 0..index {
            cursor = self.next_cursor(cursor)?;
        }
        Ok(cursor)
    }

    fn push_frame_word(
        &mut self,
        frame: MacroFrameId<G>,
        word: TracedTokenWord,
    ) -> Result<(), ScratchError> {
        let slot = self.slot(frame)?;
        let (head, tail, len) = self.push_chain_word(slot.head, slot.tail, slot.word_len, word)?;
        let slot = self.slot_mut(frame)?;
        slot.head = head;
        slot.tail = tail;
        slot.word_len = len;
        Ok(())
    }

    fn push_chain_word(
        &mut self,
        mut head: u32,
        mut tail: u32,
        len: u32,
        word: TracedTokenWord,
    ) -> Result<(u32, u32, u32), ScratchError> {
        let tail_full =
            tail == NO_CHUNK || self.macro_chunks[tail as usize].len as usize == MACRO_CHUNK_WORDS;
        if tail_full {
            let next = self.allocate_chunk()?;
            if tail == NO_CHUNK {
                head = next;
            } else {
                self.macro_chunks[tail as usize].next = next;
            }
            tail = next;
        }
        let chunk = &mut self.macro_chunks[tail as usize];
        chunk.words[chunk.len as usize] = word;
        chunk.len += 1;
        Ok((
            head,
            tail,
            len.checked_add(1).ok_or(ScratchError::CapacityOverflow)?,
        ))
    }

    fn allocate_chunk(&mut self) -> Result<u32, ScratchError> {
        if self.free_chunk_head != NO_CHUNK {
            let index = self.free_chunk_head;
            let chunk = &mut self.macro_chunks[index as usize];
            self.free_chunk_head = chunk.next;
            chunk.len = 0;
            chunk.next = NO_CHUNK;
            return Ok(index);
        }
        let index =
            u32::try_from(self.macro_chunks.len()).map_err(|_| ScratchError::CapacityOverflow)?;
        self.macro_chunks
            .try_reserve(1)
            .map_err(|_| ScratchError::AllocationFailed)?;
        self.macro_chunks.push(MacroWordChunk::default());
        Ok(index)
    }

    fn cursor_word(
        &self,
        frame: MacroFrameId<G>,
        cursor: ChunkCursor,
        require_sealed: bool,
    ) -> Result<TracedTokenWord, ScratchError> {
        if self.slot(frame)?.sealed != require_sealed {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.raw_cursor_word(cursor)
    }

    fn raw_cursor_word(&self, cursor: ChunkCursor) -> Result<TracedTokenWord, ScratchError> {
        let chunk = self
            .macro_chunks
            .get(cursor.chunk as usize)
            .ok_or(ScratchError::InvalidCoordinate)?;
        (cursor.offset < chunk.len)
            .then(|| chunk.words[cursor.offset as usize])
            .ok_or(ScratchError::InvalidCoordinate)
    }

    fn next_cursor(&self, cursor: ChunkCursor) -> Result<ChunkCursor, ScratchError> {
        let chunk = self
            .macro_chunks
            .get(cursor.chunk as usize)
            .ok_or(ScratchError::InvalidCoordinate)?;
        let offset = cursor.offset + 1;
        if offset < chunk.len {
            Ok(ChunkCursor {
                chunk: cursor.chunk,
                offset,
            })
        } else if offset == chunk.len {
            Ok(ChunkCursor {
                chunk: chunk.next,
                offset: 0,
            })
        } else {
            Err(ScratchError::InvalidCoordinate)
        }
    }

    fn release_slot(
        &mut self,
        frame: MacroFrameId<G>,
        require_sealed: bool,
    ) -> Result<(), ScratchError> {
        let slot = self.slot_mut(frame)?;
        if slot.sealed != require_sealed {
            return Err(ScratchError::InvalidCoordinate);
        }
        let head = slot.head;
        let tail = slot.tail;
        *slot = MacroSlot::default();
        self.release_chunk_chain(head, tail);
        self.macro_free_slots.push(frame.slot);
        Ok(())
    }

    fn release_chunk_chain(&mut self, head: u32, tail: u32) {
        if head == NO_CHUNK {
            return;
        }
        self.macro_chunks[tail as usize].next = self.free_chunk_head;
        self.free_chunk_head = head;
    }

    fn push_free_chunk(&mut self, chunk: u32) {
        self.macro_chunks[chunk as usize].len = 0;
        self.macro_chunks[chunk as usize].next = self.free_chunk_head;
        self.free_chunk_head = chunk;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tex_state::token::Catcode;

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
        let expected = (0..(MACRO_CHUNK_WORDS * 2 + 3))
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
                .argument_word(range, MACRO_CHUNK_WORDS * 2 + 3)
                .is_err()
        );
        assert_eq!(scratch.copied_macro_words(), 0);
        scratch.pop_macro_frame(frame).expect("frame retirement");
        assert!(scratch.is_quiescent());
        assert_eq!(scratch.retained_slot_len(), 1);
        assert_eq!(scratch.retained_chunk_len(), 3);
    }

    #[test]
    fn repeated_same_depth_replacement_reuses_two_slots_without_copying() {
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
        assert_eq!(scratch.retained_slot_len(), 2);
        assert_eq!(scratch.retained_chunk_len(), 2);
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
        assert_eq!(scratch.retained_slot_len(), 2);
        assert_eq!(scratch.retained_chunk_len(), 2);
        assert_eq!(scratch.copied_macro_words(), 0);
        scratch.pop_macro_frame(frame).expect("final retirement");
    }
}
