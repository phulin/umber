//! Immutable, generation-branded macro-definition storage.

use core::marker::PhantomData;
use core::num::NonZeroU32;
use core::num::NonZeroU64;
use std::cell::{Cell, Ref, RefCell, RefMut};
use std::rc::Rc;

use crate::generation::ArenaToken;
use crate::macro_definition::{
    MacroParameterPattern, MacroParameterPatternBuilder, MacroParameterProgramError,
};
use crate::memory_accounting::MemoryAccounting;
use crate::state_hash::StateHasher;
use crate::token::TokenWord;

#[cfg(test)]
#[path = "definition_arena/tests.rs"]
mod tests;

pub(super) enum DefinitionNamespace {}

const DEFINITION_IDENTITY_V2_DOMAIN: u64 = 0x6465_6669_6e69_7432;
const PARAMETER_START: u8 = 0;
const PARAMETER_END: u8 = 1;
const REPLACEMENT_START: u8 = 2;
const REPLACEMENT_END: u8 = 3;

/// Monotonic mutable phase of an unpublished definition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DefinitionBuildPhase {
    OpenParameters,
    OpenReplacement,
    Sealed,
    Published,
}

/// Failure while constructing mutable definition attempt data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DefinitionBuildError {
    AllocationFailed,
    CapacityOverflow,
    InvalidPhase,
    InvalidProgram(MacroParameterProgramError),
}

impl From<MacroParameterProgramError> for DefinitionBuildError {
    fn from(error: MacroParameterProgramError) -> Self {
        match error {
            MacroParameterProgramError::CapacityOverflow => Self::CapacityOverflow,
            error => Self::InvalidProgram(error),
        }
    }
}

#[derive(Debug)]
struct DefinitionData {
    words: Vec<TokenWord>,
    parameter_len: u32,
    replacement_len: u32,
    pattern: MacroParameterPatternBuilder,
    phase: DefinitionBuildPhase,
    #[cfg(test)]
    fail_next_reserve: bool,
}

impl DefinitionData {
    fn new() -> Self {
        let mut data = Self {
            words: Vec::new(),
            parameter_len: 0,
            replacement_len: 0,
            pattern: MacroParameterPatternBuilder::new(),
            phase: DefinitionBuildPhase::OpenParameters,
            #[cfg(test)]
            fail_next_reserve: false,
        };
        data.reset();
        data
    }

    fn reset(&mut self) {
        self.words.clear();
        self.parameter_len = 0;
        self.replacement_len = 0;
        self.pattern = MacroParameterPatternBuilder::new();
        self.phase = DefinitionBuildPhase::OpenParameters;
        #[cfg(test)]
        {
            self.fail_next_reserve = false;
        }
    }
}

/// Detached cold-path staging for imported and replayed definitions.
///
/// Ordinary `def`-family scanning writes into [`DefinitionArena`] directly.
/// This builder is retained for batches whose source is not the live scanner,
/// such as immutable format decode and memo replay; publication copies those
/// already-detached words into the selected immutable region.
#[derive(Debug)]
pub struct DefinitionBuilder {
    data: DefinitionData,
}

impl Default for DefinitionBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl DefinitionBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: DefinitionData::new(),
        }
    }

    /// Clears one recycled cold-path staging row.
    pub fn reset(&mut self) {
        self.data.reset();
    }

    pub fn push_parameter(&mut self, word: TokenWord) -> Result<(), DefinitionBuildError> {
        let data = self.data_mut();
        if data.phase != DefinitionBuildPhase::OpenParameters {
            return Err(DefinitionBuildError::InvalidPhase);
        }
        let mut pattern = data.pattern;
        pattern.push_parameter(word)?;
        let parameter_len = data
            .parameter_len
            .checked_add(1)
            .ok_or(DefinitionBuildError::CapacityOverflow)?;
        Self::reserve_word(data)?;
        data.parameter_len = parameter_len;
        data.pattern = pattern;
        data.words.push(word);
        Ok(())
    }

    pub fn finish_parameters(&mut self) -> Result<(), DefinitionBuildError> {
        let data = self.data_mut();
        if data.phase != DefinitionBuildPhase::OpenParameters {
            return Err(DefinitionBuildError::InvalidPhase);
        }
        data.phase = DefinitionBuildPhase::OpenReplacement;
        Ok(())
    }

    pub fn push_replacement(&mut self, word: TokenWord) -> Result<(), DefinitionBuildError> {
        let data = self.data_mut();
        if data.phase != DefinitionBuildPhase::OpenReplacement {
            return Err(DefinitionBuildError::InvalidPhase);
        }
        data.pattern.validate_replacement(word)?;
        let replacement_len = data
            .replacement_len
            .checked_add(1)
            .ok_or(DefinitionBuildError::CapacityOverflow)?;
        Self::reserve_word(data)?;
        data.words.push(word);
        data.replacement_len = replacement_len;
        Ok(())
    }

    pub fn seal(&mut self) -> Result<(), DefinitionBuildError> {
        let data = self.data_mut();
        if data.phase != DefinitionBuildPhase::OpenReplacement {
            return Err(DefinitionBuildError::InvalidPhase);
        }
        data.phase = DefinitionBuildPhase::Sealed;
        Ok(())
    }

    #[must_use]
    pub fn phase(&self) -> DefinitionBuildPhase {
        self.data().phase
    }

    #[must_use]
    pub fn parameter_text(&self) -> &[TokenWord] {
        let data = self.data();
        &data.words[..data.parameter_len as usize]
    }

    #[must_use]
    pub fn replacement_text(&self) -> &[TokenWord] {
        let data = self.data();
        &data.words[data.parameter_len as usize..]
    }

    #[must_use]
    pub fn words(&self) -> &[TokenWord] {
        &self.data().words
    }

    #[must_use]
    pub fn capacity(&self) -> usize {
        self.data().words.capacity()
    }

    fn data(&self) -> &DefinitionData {
        &self.data
    }

    fn data_mut(&mut self) -> &mut DefinitionData {
        &mut self.data
    }

    fn reserve_word(data: &mut DefinitionData) -> Result<(), DefinitionBuildError> {
        #[cfg(test)]
        if core::mem::take(&mut data.fail_next_reserve) {
            return Err(DefinitionBuildError::AllocationFailed);
        }
        data.words
            .try_reserve(1)
            .map_err(|_| DefinitionBuildError::AllocationFailed)
    }

    #[cfg(test)]
    fn force_next_reserve_failure(&mut self) {
        self.data_mut().fail_next_reserve = true;
    }

    fn validate_completed(&self) -> Result<(), DefinitionAllocationError> {
        let data = &self.data;
        if data.phase != DefinitionBuildPhase::Sealed
            || data.words.len() != data.parameter_len as usize + data.replacement_len as usize
        {
            return Err(DefinitionAllocationError::InvalidDefinition);
        }
        Ok(())
    }
}

/// Compact opaque reference to one immutable macro definition.
///
/// The packed region and row are private to this store. Callers may copy and
/// compare the reference, but cannot decode or manufacture storage
/// coordinates.
pub struct DefinitionRef<G> {
    packed: NonZeroU64,
    _brand: PhantomData<fn(&G) -> &G>,
}

#[cfg(any(test, feature = "profiling", feature = "testing"))]
thread_local! {
    static DEFINITION_RETAIN_COUNT: Cell<u64> = const { Cell::new(0) };
}

#[cfg(any(test, feature = "profiling", feature = "testing"))]
#[must_use]
pub fn definition_retain_count() -> u64 {
    DEFINITION_RETAIN_COUNT.with(Cell::get)
}

impl<G> Clone for DefinitionRef<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for DefinitionRef<G> {}

impl<G> PartialEq for DefinitionRef<G> {
    fn eq(&self, other: &Self) -> bool {
        self.packed == other.packed
    }
}

impl<G> Eq for DefinitionRef<G> {}

impl<G> core::hash::Hash for DefinitionRef<G> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.packed.hash(state);
    }
}

impl<G> core::fmt::Debug for DefinitionRef<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("DefinitionRef(..)")
    }
}

impl<G> DefinitionRef<G> {
    pub(crate) const fn runtime_word(self) -> NonZeroU64 {
        self.packed
    }

    pub(crate) const fn from_runtime_word(packed: NonZeroU64) -> Self {
        Self {
            packed,
            _brand: PhantomData,
        }
    }

    fn new(region: u32, row: NonZeroU32) -> Self {
        let packed = (u64::from(region) << 32) | u64::from(row.get());
        Self {
            packed: NonZeroU64::new(packed).expect("definition reference is nonzero"),
            _brand: PhantomData,
        }
    }

    const fn row(self) -> NonZeroU32 {
        NonZeroU32::new(self.packed.get() as u32).expect("definition row is nonzero")
    }

    const fn row_index(self) -> u32 {
        self.row().get() - 1
    }

    const fn region(self) -> u32 {
        (self.packed.get() >> 32) as u32
    }

    #[cfg(any(test, feature = "testing"))]
    #[doc(hidden)]
    pub const fn semantic_owner_count(&self) -> usize {
        0
    }
}

#[derive(Clone, Copy, Debug)]
struct DefinitionHeader {
    start: u32,
    parameter_len: u32,
    end: u32,
    origin: crate::token::OriginId,
    pattern: MacroParameterPattern,
}

const FORMAT_REGION: u32 = 1;
const GLOBAL_REGION: u32 = 2;
const INLINE_DEFINITION_WORD_CAPACITY: usize = 8;
const DEFINITION_WORD_CHUNK_CAPACITY: usize = 4096;
const LOCAL_SLOT_CHUNK_LEN: usize = 64;
const LOCAL_SLOT_ADDRESS_CAPACITY: usize = u16::MAX as usize - 2;

#[cfg(any(test, feature = "testing"))]
const fn definition_word_storage_segment(index: u32) -> u32 {
    let inline = INLINE_DEFINITION_WORD_CAPACITY as u32;
    if index < inline {
        0
    } else {
        1 + (index - inline) / DEFINITION_WORD_CHUNK_CAPACITY as u32
    }
}

fn local_region_key(address: usize, incarnation: u16) -> u32 {
    debug_assert!(address < LOCAL_SLOT_ADDRESS_CAPACITY);
    debug_assert_ne!(incarnation, 0);
    (u32::from(incarnation) << 16) | (address as u32 + 3)
}

fn local_region_address(key: u32) -> Option<(usize, u16)> {
    let encoded_address = (key & u32::from(u16::MAX)) as u16;
    let incarnation = (key >> 16) as u16;
    if encoded_address < 3 || incarnation == 0 {
        return None;
    }
    Some(((encoded_address - 3) as usize, incarnation))
}

#[inline(always)]
fn definition_word_chunk_coordinate(index: u32) -> (u32, usize) {
    let inline = INLINE_DEFINITION_WORD_CAPACITY as u32;
    if index < inline {
        (0, index as usize)
    } else {
        let overflow = index - inline;
        (
            1 + overflow / DEFINITION_WORD_CHUNK_CAPACITY as u32,
            overflow as usize % DEFINITION_WORD_CHUNK_CAPACITY,
        )
    }
}

#[inline(always)]
fn definition_word_tail_offset(head: u32) -> u32 {
    if head == 0 {
        return 0;
    }
    let (_, offset) = definition_word_chunk_coordinate(head - 1);
    u32::try_from(offset + 1).expect("definition chunk tail offset fits in u32")
}

struct DefinitionRegion {
    /// The one canonical dense header directory for this semantic region.
    ///
    /// Word payloads live in the shared region owner below, but headers are
    /// region state: published row numbers never move and all lookup paths
    /// read this directory.  Keeping a second copy in the owner made every
    /// reserve and publication pay for duplicate header storage.
    headers: Vec<DefinitionHeader>,
    owner: Option<Rc<DefinitionRegionOwner>>,
    word_head: u32,
    parent: u32,
    promotions: Vec<DefinitionPromotion>,
    changed_epoch: u64,
    changed_mutation: u32,
}

impl DefinitionRegion {
    fn new(parent: u32) -> Self {
        Self {
            headers: Vec::new(),
            owner: None,
            word_head: 0,
            parent,
            promotions: Vec::new(),
            changed_epoch: 0,
            changed_mutation: 0,
        }
    }

    fn truncate_to(&mut self, cursor: u32, accounting: &MemoryAccounting) {
        assert!(
            cursor as usize <= self.headers.len(),
            "definition cursor is beyond the region"
        );
        for header in &self.headers[cursor as usize..] {
            accounting.release_shared_dynamic(definition_memory_words(
                (header.end - header.start) as usize,
            ));
        }
        self.headers.truncate(cursor as usize);
        self.promotions
            .retain(|promotion| promotion.source_row < cursor);
    }

    fn header_len(&self) -> u32 {
        u32::try_from(self.headers.len()).expect("definition header directory fits in a row")
    }

    fn build_mark(&self, region: u32, serial: NonZeroU32) -> DefinitionBuildMark {
        let owner = self.owner.as_ref();
        DefinitionBuildMark {
            region,
            serial,
            headers: self.header_len(),
            promotions: u32::try_from(self.promotions.len())
                .expect("definition promotion directory fits in a row"),
            word_head: self.word_head,
            overflow_chunks: owner.map_or(0, |owner| {
                u32::try_from(owner.overflow_len())
                    .expect("definition overflow directory fits in a row")
            }),
            tail_offset: definition_word_tail_offset(self.word_head),
            owner_present: owner.is_some(),
        }
    }

    /// Restores an unpublished build to its exact physical frontier.
    ///
    /// The logical word head is authoritative.  A retained tail chunk is
    /// never cleared: its prefix may belong to a published definition and
    /// its unpublished suffix is safe to overwrite only after this build has
    /// been proven not to have published a reference.  Whole overflow chunks
    /// created after the mark are dropped in one vector truncation.
    fn restore_build_mark(&mut self, mark: DefinitionBuildMark, accounting: &MemoryAccounting) {
        assert!(
            mark.headers as usize <= self.headers.len(),
            "definition build header mark is beyond the region"
        );
        for header in &self.headers[mark.headers as usize..] {
            accounting.release_shared_dynamic(definition_memory_words(
                (header.end - header.start) as usize,
            ));
        }
        self.headers.truncate(mark.headers as usize);
        assert!(
            mark.promotions as usize <= self.promotions.len(),
            "definition build promotion mark is beyond the region"
        );
        self.promotions.truncate(mark.promotions as usize);

        let owner = self.owner.as_ref().map(Rc::clone);
        if mark.owner_present {
            let owner = owner.expect("definition build mark retained its region owner");
            assert!(
                mark.overflow_chunks as usize <= owner.overflow_len(),
                "definition build chunk mark is beyond the region owner"
            );
            owner.truncate_overflow_to(mark.overflow_chunks as usize);
        } else {
            assert_eq!(mark.word_head, 0, "ownerless definition mark has no words");
            assert_eq!(
                mark.overflow_chunks, 0,
                "ownerless definition mark has no chunks"
            );
            self.owner = None;
        }
        self.word_head = mark.word_head;
        debug_assert_eq!(
            definition_word_tail_offset(self.word_head),
            mark.tail_offset
        );
        debug_assert_eq!(
            self.owner.as_ref().map_or(0, |owner| owner.overflow_len()),
            mark.overflow_chunks as usize
        );
    }

    fn begin_word_span(&self) -> u32 {
        self.word_head
    }

    fn push_word(&mut self, word: TokenWord) -> Result<(), DefinitionBuildError> {
        let next = self
            .word_head
            .checked_add(1)
            .ok_or(DefinitionBuildError::CapacityOverflow)?;
        self.owner
            .get_or_insert_with(|| Rc::new(DefinitionRegionOwner::new()))
            .push_word(self.word_head, word)?;
        self.word_head = next;
        Ok(())
    }

    fn reserve(&mut self, rows: usize, words: usize) -> Result<(), DefinitionBuildError> {
        self.headers
            .len()
            .checked_add(rows)
            .and_then(|len| u32::try_from(len).ok())
            .ok_or(DefinitionBuildError::CapacityOverflow)?;
        let words = u32::try_from(words).map_err(|_| DefinitionBuildError::CapacityOverflow)?;
        let word_end = self
            .word_head
            .checked_add(words)
            .ok_or(DefinitionBuildError::CapacityOverflow)?;
        self.headers
            .try_reserve(rows)
            .map_err(|_| DefinitionBuildError::AllocationFailed)?;
        if rows != 0 || words != 0 {
            let owner = self
                .owner
                .get_or_insert_with(|| Rc::new(DefinitionRegionOwner::new()));
            owner.reserve_word_span(self.word_head, word_end)?;
        }
        Ok(())
    }

    fn extend_words(&mut self, source: &[TokenWord]) -> Result<(u32, u32), DefinitionBuildError> {
        let start = self.word_head;
        for word in source {
            self.push_word(*word)?;
        }
        Ok((start, self.word_head))
    }

    fn word(&self, index: u32) -> Option<TokenWord> {
        self.owner.as_ref()?.word(index)
    }

    fn push_header(&mut self, header: DefinitionHeader) {
        self.headers.push(header);
    }
}

/// One fixed physical word block.
///
/// The cells are private to the definition arena. Builders are the only code
/// which can write them, while admitted readers retain an `Rc` to this block
/// and load a slot without borrowing the region directory. The two variants
/// keep the small common prefix cheap while giving overflow blocks a uniform
/// per-physical-chunk handle. Overflow payloads live inline in the reference-
/// counted object so each physical chunk has exactly one heap allocation.
#[allow(
    clippy::large_enum_variant,
    reason = "the fixed overflow payload must remain inline in its Rc allocation"
)]
enum DefinitionWordChunk {
    Inline([Cell<TokenWord>; INLINE_DEFINITION_WORD_CAPACITY]),
    Overflow([Cell<TokenWord>; DEFINITION_WORD_CHUNK_CAPACITY]),
}

impl DefinitionWordChunk {
    fn new_inline() -> Self {
        Self::Inline(std::array::from_fn(|_| Cell::new(TokenWord::from_raw(0))))
    }

    /// Initializes one cold overflow payload before moving it into its single
    /// `Rc` allocation. The bounded stack temporary is released when this
    /// non-recursive constructor returns; it is never retained per chunk.
    #[cold]
    #[inline(never)]
    fn new_overflow() -> Self {
        Self::Overflow(std::array::from_fn(|_| Cell::new(TokenWord::from_raw(0))))
    }

    #[inline(always)]
    fn get(&self, slot: usize) -> Option<TokenWord> {
        match self {
            Self::Inline(words) => words.get(slot).map(Cell::get),
            Self::Overflow(words) => words.get(slot).map(Cell::get),
        }
    }

    #[inline(always)]
    fn set(&self, slot: usize, word: TokenWord) -> Option<()> {
        let cell = match self {
            Self::Inline(words) => words.get(slot),
            Self::Overflow(words) => words.get(slot),
        }?;
        cell.set(word);
        Some(())
    }

    #[inline(always)]
    fn cells(&self) -> &[Cell<TokenWord>] {
        match self {
            Self::Inline(words) => words.as_slice(),
            Self::Overflow(words) => words.as_slice(),
        }
    }

    #[inline(always)]
    const fn capacity(&self) -> usize {
        match self {
            Self::Inline(_) => INLINE_DEFINITION_WORD_CAPACITY,
            Self::Overflow(_) => DEFINITION_WORD_CHUNK_CAPACITY,
        }
    }
}

struct DefinitionRegionOwner {
    /// Inline common prefix plus a flat directory of stable overflow blocks.
    ///
    /// A resident cursor cannot safely retain a borrow into the same `Rc`
    /// that owns this storage without becoming self-referential. Keeping the
    /// overflow directory flat makes the required short reborrow one checked,
    /// constant-time slot access instead of a linked-page walk. Tiny regions
    /// never acquire an overflow block. Each overflow payload is part of its
    /// `Rc` allocation and has no owner or lifetime independent of this region.
    inline_words: Rc<DefinitionWordChunk>,
    overflow_words: RefCell<Vec<Rc<DefinitionWordChunk>>>,
}

impl DefinitionRegionOwner {
    fn new() -> Self {
        Self {
            inline_words: Rc::new(DefinitionWordChunk::new_inline()),
            overflow_words: RefCell::new(Vec::new()),
        }
    }

    fn push_word(&self, index: u32, word: TokenWord) -> Result<(), DefinitionBuildError> {
        let index = index as usize;
        if index < INLINE_DEFINITION_WORD_CAPACITY {
            self.inline_words
                .set(index, word)
                .expect("inline definition word slot exists");
            return Ok(());
        }
        let overflow = index - INLINE_DEFINITION_WORD_CAPACITY;
        let chunk = overflow / DEFINITION_WORD_CHUNK_CAPACITY;
        self.ensure_chunk(chunk)?;
        self.overflow_words.borrow()[chunk]
            .set(overflow % DEFINITION_WORD_CHUNK_CAPACITY, word)
            .expect("overflow definition word slot exists");
        Ok(())
    }

    fn ensure_chunk(&self, chunk: usize) -> Result<(), DefinitionBuildError> {
        let mut words = self.overflow_words.borrow_mut();
        if chunk >= words.len() {
            let additional = chunk + 1 - words.len();
            words
                .try_reserve_exact(additional)
                .map_err(|_| DefinitionBuildError::AllocationFailed)?;
            while words.len() <= chunk {
                words.push(Rc::new(DefinitionWordChunk::new_overflow()));
            }
        }
        Ok(())
    }

    fn reserve_word_span(&self, start: u32, end: u32) -> Result<(), DefinitionBuildError> {
        let inline_end = INLINE_DEFINITION_WORD_CAPACITY as u32;
        if start == end || end <= inline_end {
            return Ok(());
        }
        let first = start.saturating_sub(inline_end) as usize / DEFINITION_WORD_CHUNK_CAPACITY;
        let last = (end - 1 - inline_end) as usize / DEFINITION_WORD_CHUNK_CAPACITY;
        for chunk in first..=last {
            self.ensure_chunk(chunk)?;
        }
        Ok(())
    }

    #[inline(always)]
    fn overflow_len(&self) -> usize {
        self.overflow_words.borrow().len()
    }

    /// Drops only whole overflow chunks beyond an unpublished build mark.
    ///
    /// A chunk containing the published prefix is retained even when the
    /// mark lies in its middle.  Its unpublished suffix is deliberately left
    /// in place and is overwritten by the next build, while any resident
    /// cursor retaining that published chunk remains valid.
    fn truncate_overflow_to(&self, chunks: usize) {
        self.overflow_words.borrow_mut().truncate(chunks);
    }

    #[inline(always)]
    fn word(&self, index: u32) -> Option<TokenWord> {
        let index = index as usize;
        if index < INLINE_DEFINITION_WORD_CAPACITY {
            return self.inline_words.get(index);
        }
        let overflow = index.checked_sub(INLINE_DEFINITION_WORD_CAPACITY)?;
        let chunk = overflow / DEFINITION_WORD_CHUNK_CAPACITY;
        self.word_in_chunk(chunk as u32, overflow % DEFINITION_WORD_CHUNK_CAPACITY)
    }

    #[inline(always)]
    fn word_in_chunk(&self, chunk: u32, offset: usize) -> Option<TokenWord> {
        self.overflow_words
            .borrow()
            .get(chunk as usize)?
            .get(offset)
    }

    /// Lends the one physical word span containing `start`.
    ///
    /// The callback keeps the overflow-directory borrow shorter than the
    /// owning `Rc`; no reference or parallel cursor escapes admission.
    fn with_word_span<R>(
        &self,
        start: u32,
        end: u32,
        consume: impl FnOnce(&[Cell<TokenWord>]) -> R,
    ) -> Option<R> {
        if start >= end {
            return None;
        }
        let start = start as usize;
        let end = end as usize;
        if start < INLINE_DEFINITION_WORD_CAPACITY {
            let end = end.min(INLINE_DEFINITION_WORD_CAPACITY);
            return Some(consume(&self.inline_words.cells()[start..end]));
        }
        let overflow = start - INLINE_DEFINITION_WORD_CAPACITY;
        let chunk = overflow / DEFINITION_WORD_CHUNK_CAPACITY;
        let offset = overflow % DEFINITION_WORD_CHUNK_CAPACITY;
        let words = self.overflow_words.borrow();
        let chunk = words.get(chunk)?;
        let len = (end - start).min(DEFINITION_WORD_CHUNK_CAPACITY - offset);
        Some(consume(&chunk.cells()[offset..offset + len]))
    }

    /// Clones one immutable physical chunk handle for an admitted cursor.
    ///
    /// The directory borrow ends before the handle reaches the resident row;
    /// subsequent word reads therefore touch only the retained chunk.
    fn chunk_handle(&self, chunk: u32) -> Option<Rc<DefinitionWordChunk>> {
        if chunk == 0 {
            return Some(Rc::clone(&self.inline_words));
        }
        self.overflow_words
            .borrow()
            .get(chunk.checked_sub(1)? as usize)
            .map(Rc::clone)
    }
}

#[derive(Clone, Copy)]
struct DefinitionPromotion {
    source_row: u32,
    destination_row: NonZeroU32,
}

#[cfg(any(test, feature = "profiling", feature = "testing"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) struct DefinitionRetirementCounters {
    pub(crate) group_entry_slot_inspections: u64,
    pub(crate) local_slot_chunk_allocations: u64,
    pub(crate) group_region_inspections: u64,
    pub(crate) lease_release_region_inspections: u64,
    pub(crate) checkpoint_region_inspections: u64,
    pub(crate) regions_reclaimed: u64,
    pub(crate) rows_reclaimed: u64,
    pub(crate) promotions_reclaimed: u64,
}

#[cfg(any(test, feature = "profiling", feature = "testing"))]
#[derive(Default)]
struct DefinitionRetirementCounterCells {
    group_entry_slot_inspections: Cell<u64>,
    local_slot_chunk_allocations: Cell<u64>,
    group_region_inspections: Cell<u64>,
    lease_release_region_inspections: Cell<u64>,
    checkpoint_region_inspections: Cell<u64>,
    regions_reclaimed: Cell<u64>,
    rows_reclaimed: Cell<u64>,
    promotions_reclaimed: Cell<u64>,
}

struct LocalDefinitionRegion {
    data: DefinitionRegion,
    retired: bool,
    leases: u32,
}

impl LocalDefinitionRegion {
    fn new(parent: u32) -> Self {
        Self {
            data: DefinitionRegion::new(parent),
            retired: false,
            leases: 0,
        }
    }
}

#[derive(Default)]
struct LocalDefinitionSlot {
    incarnation: u16,
    region: Option<LocalDefinitionRegion>,
    next_free: Option<u16>,
}

#[derive(Default)]
struct LocalDefinitionSlotStore {
    chunks: Vec<Box<[LocalDefinitionSlot]>>,
    free_head: Option<u16>,
}

impl LocalDefinitionSlotStore {
    fn slot(&self, address: usize) -> Option<&LocalDefinitionSlot> {
        let chunk = address / LOCAL_SLOT_CHUNK_LEN;
        let offset = address % LOCAL_SLOT_CHUNK_LEN;
        self.chunks.get(chunk).and_then(|chunk| chunk.get(offset))
    }

    fn slot_mut(&mut self, address: usize) -> Option<&mut LocalDefinitionSlot> {
        let chunk = address / LOCAL_SLOT_CHUNK_LEN;
        let offset = address % LOCAL_SLOT_CHUNK_LEN;
        self.chunks
            .get_mut(chunk)
            .and_then(|chunk| chunk.get_mut(offset))
    }

    fn region(&self, key: u32) -> Option<&LocalDefinitionRegion> {
        let (address, incarnation) = local_region_address(key)?;
        let slot = self.slot(address)?;
        (slot.incarnation == incarnation)
            .then_some(slot.region.as_ref())
            .flatten()
    }

    fn region_mut(&mut self, key: u32) -> Option<&mut LocalDefinitionRegion> {
        let (address, incarnation) = local_region_address(key)?;
        let slot = self.slot_mut(address)?;
        (slot.incarnation == incarnation)
            .then_some(slot.region.as_mut())
            .flatten()
    }

    fn allocate_chunk(&mut self) -> Result<usize, DefinitionAllocationError> {
        let base = self
            .chunks
            .len()
            .checked_mul(LOCAL_SLOT_CHUNK_LEN)
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        if base >= LOCAL_SLOT_ADDRESS_CAPACITY {
            return Err(DefinitionAllocationError::CapacityOverflow);
        }
        let len = LOCAL_SLOT_CHUNK_LEN.min(LOCAL_SLOT_ADDRESS_CAPACITY - base);
        self.chunks
            .try_reserve(1)
            .map_err(|_| DefinitionAllocationError::AllocationFailed)?;
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(len)
            .map_err(|_| DefinitionAllocationError::AllocationFailed)?;
        slots.resize_with(len, LocalDefinitionSlot::default);
        self.chunks.push(slots.into_boxed_slice());
        for address in (base..base + len).rev() {
            let free_head = self.free_head;
            let slot = self
                .slot_mut(address)
                .expect("new definition free slot exists");
            slot.next_free = free_head;
            self.free_head = Some(address as u16);
        }
        Ok(base)
    }

    fn allocate(&mut self, parent: u32) -> Result<u32, DefinitionAllocationError> {
        if self.free_head.is_none() {
            self.allocate_chunk()?;
        }
        let address = self
            .free_head
            .expect("allocated chunk supplies a free slot") as usize;
        let next_free = self
            .slot(address)
            .expect("new or reusable definition slot exists")
            .next_free;
        self.free_head = next_free;
        let slot = self
            .slot_mut(address)
            .expect("new or reusable definition slot exists");
        slot.next_free = None;
        let incarnation = slot
            .incarnation
            .checked_add(1)
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        slot.incarnation = incarnation;
        slot.region = Some(LocalDefinitionRegion::new(parent));
        Ok(local_region_key(address, incarnation))
    }

    fn recycle(&mut self, address: usize) {
        let free_head = self.free_head;
        let slot = self
            .slot_mut(address)
            .expect("recycled definition slot exists");
        slot.next_free = free_head;
        self.free_head = Some(address as u16);
    }
}

struct LocalDefinitionSlots {
    store: RefCell<LocalDefinitionSlotStore>,
    rows: Cell<usize>,
    accounting: MemoryAccounting,
    #[cfg(any(test, feature = "profiling", feature = "testing"))]
    counters: Rc<DefinitionRetirementCounterCells>,
}

impl LocalDefinitionSlots {
    fn new(
        accounting: MemoryAccounting,
        #[cfg(any(test, feature = "profiling", feature = "testing"))] counters: Rc<
            DefinitionRetirementCounterCells,
        >,
    ) -> Self {
        Self {
            store: RefCell::new(LocalDefinitionSlotStore::default()),
            rows: Cell::new(0),
            accounting,
            #[cfg(any(test, feature = "profiling", feature = "testing"))]
            counters,
        }
    }

    fn allocate(&self, parent: u32) -> Result<u32, DefinitionAllocationError> {
        #[cfg(any(test, feature = "profiling", feature = "testing"))]
        let before = self.store.borrow().chunks.len();
        let key = self.store.borrow_mut().allocate(parent)?;
        #[cfg(any(test, feature = "profiling", feature = "testing"))]
        if self.store.borrow().chunks.len() != before {
            self.counters.local_slot_chunk_allocations.set(
                self.counters
                    .local_slot_chunk_allocations
                    .get()
                    .saturating_add(1),
            );
        }
        if parent != 0 {
            assert!(
                self.acquire(parent),
                "nested definition region pins its exact parent"
            );
        }
        Ok(key)
    }

    fn region(&self, key: u32) -> Option<Ref<'_, DefinitionRegion>> {
        Ref::filter_map(self.store.borrow(), |store| {
            store.region(key).map(|region| &region.data)
        })
        .ok()
    }

    fn region_mut(&self, key: u32) -> Option<RefMut<'_, DefinitionRegion>> {
        RefMut::filter_map(self.store.borrow_mut(), |store| {
            store.region_mut(key).map(|region| &mut region.data)
        })
        .ok()
    }

    fn parent(&self, key: u32) -> Option<u32> {
        self.store
            .borrow()
            .region(key)
            .map(|region| region.data.parent)
    }

    fn lease_count(&self, key: u32) -> Option<u32> {
        self.store.borrow().region(key).map(|region| region.leases)
    }

    fn acquire(&self, key: u32) -> bool {
        let mut store = self.store.borrow_mut();
        let Some(region) = store.region_mut(key) else {
            return false;
        };
        region.leases = region
            .leases
            .checked_add(1)
            .expect("definition region lease count overflow");
        true
    }

    fn release(&self, key: u32) {
        self.release_inner(key, true);
    }

    fn release_inner(&self, key: u32, _count_inspection: bool) {
        #[cfg(any(test, feature = "profiling", feature = "testing"))]
        if _count_inspection {
            self.counters.lease_release_region_inspections.set(
                self.counters
                    .lease_release_region_inspections
                    .get()
                    .saturating_add(1),
            );
        }
        let mut current = key;
        loop {
            let reclaim = {
                let mut store = self.store.borrow_mut();
                let region = store
                    .region_mut(current)
                    .expect("definition lease names its live slot incarnation");
                region.leases = region
                    .leases
                    .checked_sub(1)
                    .expect("definition region lease count underflow");
                region.leases == 0 && region.retired
            };
            if !reclaim {
                break;
            }
            current = self.reclaim(current);
            if current == 0 {
                break;
            }
        }
    }

    fn retire(&self, key: u32) {
        let reclaim = {
            let mut store = self.store.borrow_mut();
            let region = store
                .region_mut(key)
                .expect("retired definition region exists");
            region.retired = true;
            region.leases == 0
        };
        if reclaim {
            let parent = self.reclaim(key);
            if parent != 0 {
                self.release_inner(parent, false);
            }
        }
    }

    fn add_rows(&self, rows: usize) {
        self.rows.set(
            self.rows
                .get()
                .checked_add(rows)
                .expect("definition row count overflow"),
        );
    }

    fn remove_rows(&self, rows: usize) {
        self.rows.set(
            self.rows
                .get()
                .checked_sub(rows)
                .expect("definition row count underflow"),
        );
    }

    fn reclaim(&self, key: u32) -> u32 {
        let (address, incarnation) =
            local_region_address(key).expect("local definition key is encoded");
        let mut store = self.store.borrow_mut();
        let slot = store
            .slot_mut(address)
            .expect("reclaimed definition slot exists");
        assert_eq!(slot.incarnation, incarnation, "definition slot is current");
        let Some(mut region) = slot.region.take() else {
            return 0;
        };
        let parent = region.data.parent;
        #[cfg(any(test, feature = "profiling", feature = "testing"))]
        let rows = region.data.headers.len() as u64;
        #[cfg(any(test, feature = "profiling", feature = "testing"))]
        let promotions = region.data.promotions.len() as u64;
        self.remove_rows(region.data.headers.len());
        region.data.truncate_to(0, &self.accounting);
        if incarnation != u16::MAX {
            store.recycle(address);
        }
        drop(store);
        #[cfg(any(test, feature = "profiling", feature = "testing"))]
        {
            self.counters
                .regions_reclaimed
                .set(self.counters.regions_reclaimed.get().saturating_add(1));
            self.counters
                .rows_reclaimed
                .set(self.counters.rows_reclaimed.get().saturating_add(rows));
            self.counters.promotions_reclaimed.set(
                self.counters
                    .promotions_reclaimed
                    .get()
                    .saturating_add(promotions),
            );
        }
        parent
    }

    fn row_count(&self) -> usize {
        self.rows.get()
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.row_count() == 0
    }
}

impl Drop for LocalDefinitionSlots {
    fn drop(&mut self) {
        for chunk in &mut self.store.get_mut().chunks {
            for slot in chunk.iter_mut() {
                if let Some(region) = &mut slot.region {
                    region.data.truncate_to(0, &self.accounting);
                }
            }
        }
    }
}

enum DefinitionRegionRef<'a> {
    Fixed(&'a DefinitionRegion),
    Local(Ref<'a, DefinitionRegion>),
}

impl core::ops::Deref for DefinitionRegionRef<'_> {
    type Target = DefinitionRegion;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Fixed(region) => region,
            Self::Local(region) => region,
        }
    }
}

enum DefinitionRegionMut<'a> {
    Fixed(&'a mut DefinitionRegion),
    Local(RefMut<'a, DefinitionRegion>),
}

impl core::ops::Deref for DefinitionRegionMut<'_> {
    type Target = DefinitionRegion;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Fixed(region) => region,
            Self::Local(region) => region,
        }
    }
}

impl core::ops::DerefMut for DefinitionRegionMut<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Fixed(region) => region,
            Self::Local(region) => region,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DefinitionArenaCursor {
    format_rows: u32,
    global_rows: u32,
    active_local: u32,
    active_local_rows: u32,
    mutation_mark: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DefinitionDestination {
    Format,
    Global,
    Local,
}

pub struct DefinitionBuildKey<G> {
    serial: NonZeroU32,
    _brand: PhantomData<fn(&G) -> &G>,
}

struct LocalRegionPin<G> {
    region: Option<LocalDefinitionRegionLease>,
    _brand: PhantomData<fn(&G) -> &G>,
}

/// Exact structural work performed by one resident replacement cursor.
#[cfg(any(test, feature = "testing"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ResidentMacroBodyReadCounters {
    pub admission_chunk_lookups: u64,
    pub region_owner_acquisitions: u64,
    pub direct_chunk_slot_reads: u64,
    pub chunk_boundary_transitions: u64,
    pub whole_body_copies: u64,
}

#[cfg(any(test, feature = "testing"))]
thread_local! {
    static RESIDENT_MACRO_BODY_READ_COUNTERS: Cell<ResidentMacroBodyReadCounters> =
        const { Cell::new(ResidentMacroBodyReadCounters {
            admission_chunk_lookups: 0,
            region_owner_acquisitions: 0,
            direct_chunk_slot_reads: 0,
            chunk_boundary_transitions: 0,
            whole_body_copies: 0,
        }) };
}

#[cfg(any(test, feature = "testing"))]
#[doc(hidden)]
pub fn reset_resident_macro_body_read_counters() {
    RESIDENT_MACRO_BODY_READ_COUNTERS.set(ResidentMacroBodyReadCounters::default());
}

#[cfg(any(test, feature = "testing"))]
#[doc(hidden)]
#[must_use]
pub fn resident_macro_body_read_counters() -> ResidentMacroBodyReadCounters {
    RESIDENT_MACRO_BODY_READ_COUNTERS.get()
}

/// Store-minted resident coordinate for one executing macro replacement body.
///
/// Admission validates the opaque reference, replacement span, and first
/// physical chunk once. The row then owns the region lifetime, one immutable
/// chunk handle, and scalar logical/physical cursor state. Ordinary delivery
/// reads the retained chunk directly; only a physical crossing reborrows the
/// region's flat directory in the cold helper below.
pub struct ResidentMacroBody<G> {
    definition: DefinitionRef<G>,
    owner: Rc<DefinitionRegionOwner>,
    parameter_start: u32,
    start: u32,
    end: u32,
    position: u32,
    chunk_index: u32,
    chunk_slot: usize,
    remaining_in_chunk: u32,
    current_chunk: Option<Rc<DefinitionWordChunk>>,
}

/// One immutable macro definition admitted for activation.
///
/// A parameterless empty replacement needs no resident region owner until an
/// exceptional observed or recovery path explicitly requests the traditional
/// empty input row. Nonempty simple and matching definitions carry the exact
/// resident body owner acquired by this admission.
pub enum AdmittedMacroDefinition<G> {
    SimpleMacro {
        pattern: MacroParameterPattern,
        body: Option<ResidentMacroBody<G>>,
    },
    MatchingMacro {
        pattern: MacroParameterPattern,
        parameter_len: usize,
        body: ResidentMacroBody<G>,
    },
}

impl<G> ResidentMacroBody<G> {
    #[must_use]
    pub const fn definition_ref(&self) -> DefinitionRef<G> {
        self.definition
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        (self.end - self.start) as usize
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Number of parameter-text words retained with this active macro.
    #[must_use]
    pub const fn parameter_len(&self) -> usize {
        (self.start - self.parameter_start) as usize
    }

    /// Reads one parameter-text word from the resident definition owner.
    #[must_use]
    #[cold]
    #[inline(never)]
    pub fn parameter_word(&self, position: usize) -> Option<TokenWord> {
        (position < self.parameter_len()).then_some(())?;
        self.owner.word(self.parameter_start + position as u32)
    }

    #[must_use]
    #[cold]
    #[inline(never)]
    pub fn word(&self, position: usize) -> Option<TokenWord> {
        (position < self.len()).then_some(())?;
        let absolute = self.start + position as u32;
        self.record_word_read(absolute);
        self.owner.word(absolute)
    }

    /// Reads and advances one already-admitted replacement word.
    ///
    /// The returned flag is true only when this word exhausted a physical
    /// chunk while the logical replacement continues. The caller must route
    /// that flag to [`Self::advance_chunk_cold`] before the next read.
    #[inline(always)]
    pub fn read_current_word(&mut self, position: u32) -> Option<(TokenWord, bool)> {
        let length = self.end - self.start;
        if position != self.position || position >= length {
            return None;
        }
        let word = self.current_chunk.as_ref()?.get(self.chunk_slot)?;
        debug_assert!(self.remaining_in_chunk != 0);
        self.position += 1;
        self.chunk_slot += 1;
        self.remaining_in_chunk -= 1;
        self.record_direct_word_read();
        let boundary = self.remaining_in_chunk == 0 && self.position < length;
        Some((word, boundary))
    }

    /// Advances a direct contiguous read from the current physical chunk.
    ///
    /// The span consumer uses this after its append succeeds. It keeps the
    /// logical frame position authoritative while updating the admitted
    /// physical cursor once for the entire consumed prefix.
    #[inline(always)]
    pub fn advance_current_run(&mut self, count: u32) -> bool {
        if count == 0 || count > self.remaining_in_chunk {
            return false;
        }
        self.position += count;
        self.chunk_slot += count as usize;
        self.remaining_in_chunk -= count;
        self.record_direct_word_reads(count);
        self.remaining_in_chunk == 0 && self.position < self.end - self.start
    }

    /// Rebuilds the physical cursor from the logical rollback coordinate.
    ///
    /// Checkpoints retain only the packed input position. Re-admission here
    /// is cold and clones no body words; it merely reacquires the one chunk
    /// handle containing that position.
    #[cold]
    pub fn restore_position(&mut self, position: u32) {
        let length = self.end - self.start;
        assert!(
            position <= length,
            "macro body rollback position is in range"
        );
        self.position = position;
        if length == 0 {
            self.current_chunk = None;
            self.chunk_index = 0;
            self.chunk_slot = 0;
            self.remaining_in_chunk = 0;
            return;
        }
        let logical = position.min(length - 1);
        let absolute = self
            .start
            .checked_add(logical)
            .expect("admitted macro body absolute position fits");
        let (chunk_index, offset) = definition_word_chunk_coordinate(absolute);
        let chunk = self
            .owner
            .chunk_handle(chunk_index)
            .expect("admitted macro body rollback chunk remains live");
        let chunk_capacity = chunk.capacity();
        let (slot, remaining) = if position < length {
            let current_absolute = self
                .start
                .checked_add(position)
                .expect("admitted macro body position fits");
            let (_, slot) = definition_word_chunk_coordinate(current_absolute);
            let available = length - position;
            (slot, available.min((chunk_capacity - slot) as u32))
        } else {
            (offset + 1, 0)
        };
        self.current_chunk = Some(chunk);
        self.chunk_index = chunk_index;
        self.chunk_slot = slot;
        self.remaining_in_chunk = remaining;
    }

    /// Acquires the next physical chunk after a direct reader reports a
    /// boundary. This is deliberately cold: ordinary words never borrow the
    /// region owner or directory.
    #[cold]
    #[inline(never)]
    pub fn advance_chunk_cold(&mut self) {
        let length = self.end - self.start;
        assert!(
            self.position < length,
            "macro chunk boundary is not exhausted"
        );
        assert_eq!(self.remaining_in_chunk, 0);
        let absolute = self
            .start
            .checked_add(self.position)
            .expect("admitted macro body boundary fits");
        let (chunk_index, slot) = definition_word_chunk_coordinate(absolute);
        assert_eq!(slot, 0, "macro body boundary starts a fresh chunk");
        assert_eq!(
            chunk_index,
            self.chunk_index + 1,
            "macro body crosses the next physical chunk"
        );
        let chunk = self
            .owner
            .chunk_handle(chunk_index)
            .expect("admitted macro body next chunk remains live");
        let remaining = (length - self.position).min(chunk.capacity() as u32);
        self.current_chunk = Some(chunk);
        self.chunk_index = chunk_index;
        self.chunk_slot = 0;
        self.remaining_in_chunk = remaining;
        self.record_chunk_boundary_transition();
    }

    /// Lends the physical replacement span beginning at `position`.
    ///
    /// A span ends only at the replacement end or a storage-block boundary.
    /// Semantic consumers may stop earlier and commit only their consumed
    /// prefix. Words remain in their sole canonical representation.
    #[must_use]
    #[inline]
    pub fn with_contiguous_span<R>(
        &self,
        position: usize,
        consume: impl FnOnce(&[Cell<TokenWord>]) -> R,
    ) -> Option<R> {
        (position < self.len()).then_some(())?;
        let absolute = self.start.checked_add(position as u32)?;
        if self.position == position as u32 {
            let chunk = self.current_chunk.as_ref()?;
            let end = self
                .chunk_slot
                .saturating_add(self.remaining_in_chunk as usize)
                .min(chunk.capacity());
            if self.chunk_slot < end {
                return Some(consume(&chunk.cells()[self.chunk_slot..end]));
            }
        }
        self.owner.with_word_span(absolute, self.end, consume)
    }

    #[inline(always)]
    fn record_word_read(&self, _absolute: u32) {
        #[cfg(any(test, feature = "testing"))]
        RESIDENT_MACRO_BODY_READ_COUNTERS.set({
            let mut counters = RESIDENT_MACRO_BODY_READ_COUNTERS.get();
            counters.direct_chunk_slot_reads = counters.direct_chunk_slot_reads.saturating_add(1);
            if _absolute != self.start
                && definition_word_storage_segment(_absolute)
                    != definition_word_storage_segment(_absolute - 1)
            {
                counters.chunk_boundary_transitions =
                    counters.chunk_boundary_transitions.saturating_add(1);
            }
            counters
        });
    }

    #[inline(always)]
    fn record_direct_word_read(&self) {
        self.record_direct_word_reads(1);
    }

    #[inline(always)]
    fn record_direct_word_reads(&self, _count: u32) {
        #[cfg(any(test, feature = "testing"))]
        RESIDENT_MACRO_BODY_READ_COUNTERS.set({
            let mut counters = RESIDENT_MACRO_BODY_READ_COUNTERS.get();
            counters.direct_chunk_slot_reads = counters
                .direct_chunk_slot_reads
                .saturating_add(u64::from(_count));
            counters
        });
    }

    #[inline(always)]
    fn record_chunk_boundary_transition(&self) {
        record_chunk_boundary_transition();
    }

    #[cfg(any(test, feature = "profiling", feature = "testing"))]
    #[doc(hidden)]
    #[must_use]
    pub fn profile_region_owner_count(&self) -> usize {
        Rc::strong_count(&self.owner)
    }
}

#[inline(always)]
fn record_chunk_boundary_transition() {
    #[cfg(any(test, feature = "testing"))]
    RESIDENT_MACRO_BODY_READ_COUNTERS.set({
        let mut counters = RESIDENT_MACRO_BODY_READ_COUNTERS.get();
        counters.chunk_boundary_transitions = counters.chunk_boundary_transitions.saturating_add(1);
        counters
    });
}

impl<G> core::fmt::Debug for ResidentMacroBody<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ResidentMacroBody(..)")
    }
}

impl<G> PartialEq for ResidentMacroBody<G> {
    fn eq(&self, other: &Self) -> bool {
        self.definition == other.definition
            && self.start == other.start
            && self.end == other.end
            && Rc::ptr_eq(&self.owner, &other.owner)
    }
}

impl<G> Eq for ResidentMacroBody<G> {}

impl<G> core::hash::Hash for ResidentMacroBody<G> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.definition.hash(state);
        self.start.hash(state);
        self.end.hash(state);
        (Rc::as_ptr(&self.owner) as usize).hash(state);
    }
}

struct LocalDefinitionRegionLease {
    slots: Rc<LocalDefinitionSlots>,
    key: u32,
}

pub(crate) struct DefinitionCheckpointLease<G> {
    region: LocalRegionPin<G>,
}

impl<G> Clone for DefinitionCheckpointLease<G> {
    fn clone(&self) -> Self {
        Self {
            region: self.region.clone(),
        }
    }
}

impl<G> Clone for LocalRegionPin<G> {
    fn clone(&self) -> Self {
        if let Some(region) = &self.region {
            assert!(region.slots.acquire(region.key));
        }
        Self {
            region: self
                .region
                .as_ref()
                .map(|region| LocalDefinitionRegionLease {
                    slots: Rc::clone(&region.slots),
                    key: region.key,
                }),
            _brand: PhantomData,
        }
    }
}

impl<G> Drop for LocalRegionPin<G> {
    fn drop(&mut self) {
        if let Some(region) = self.region.take() {
            region.slots.release(region.key);
        }
    }
}

impl<G> core::fmt::Debug for LocalRegionPin<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("LocalRegionPin(..)")
    }
}

impl<G> PartialEq for LocalRegionPin<G> {
    fn eq(&self, other: &Self) -> bool {
        match (&self.region, &other.region) {
            (Some(left), Some(right)) => {
                left.key == right.key && Rc::ptr_eq(&left.slots, &right.slots)
            }
            (None, None) => true,
            (Some(_), None) | (None, Some(_)) => false,
        }
    }
}

impl<G> Eq for LocalRegionPin<G> {}

impl<G> core::hash::Hash for LocalRegionPin<G> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.region
            .as_ref()
            .map_or((0, 0), |region| {
                (Rc::as_ptr(&region.slots) as usize, region.key)
            })
            .hash(state);
    }
}

/// Move-only frontier for one unpublished definition build.
///
/// The header count protects stable row numbering, while the word/chunk
/// fields identify the exact physical suffix which may be reclaimed.  The
/// mark is intentionally not `Copy`: a successful seal or an abort consumes
/// the only restoration capability before a definition reference can escape.
struct DefinitionBuildMark {
    region: u32,
    serial: NonZeroU32,
    headers: u32,
    promotions: u32,
    word_head: u32,
    overflow_chunks: u32,
    tail_offset: u32,
    owner_present: bool,
}

impl<G> Clone for DefinitionBuildKey<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for DefinitionBuildKey<G> {}

impl<G> core::fmt::Debug for DefinitionBuildKey<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("DefinitionBuildKey(..)")
    }
}

impl<G> PartialEq for DefinitionBuildKey<G> {
    fn eq(&self, other: &Self) -> bool {
        self.serial == other.serial
    }
}

impl<G> Eq for DefinitionBuildKey<G> {}

struct ActiveDefinitionBuild {
    serial: NonZeroU32,
    region: u32,
    mark: DefinitionBuildMark,
    parameter_len: u32,
    replacement_len: u32,
    pattern: MacroParameterPatternBuilder,
    origin: crate::token::OriginId,
    phase: DefinitionBuildPhase,
}

struct DefinitionRegionSuffix {
    headers: Vec<DefinitionHeader>,
    owner: Option<Rc<DefinitionRegionOwner>>,
    promotions: Vec<DefinitionPromotion>,
}

#[derive(Clone, Copy)]
struct DefinitionRegionMark {
    headers: u32,
    promotions: u32,
    retired: bool,
}

enum DefinitionRegionMutation {
    Existing {
        region: u32,
        mark: DefinitionRegionMark,
    },
    Created {
        region: u32,
    },
}

struct AcceptedRegionMutation {
    mutation: DefinitionRegionMutation,
    suffix: Option<DefinitionRegionSuffix>,
    head_retired: bool,
}

pub(crate) struct AcceptedDefinitionTail<G> {
    head: DefinitionArenaCursor,
    accepted: Vec<AcceptedRegionMutation>,
    _head_lease: LocalRegionPin<G>,
    next_build_serial: u32,
    _brand: PhantomData<fn(&G) -> &G>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DefinitionAllocationError {
    CapacityOverflow,
    AllocationFailed,
    InvalidDefinition,
}

/// Revision-owned packed definition headers and words.
pub(crate) struct DefinitionArena<G> {
    format: DefinitionRegion,
    global: DefinitionRegion,
    local_slots: Rc<LocalDefinitionSlots>,
    active_local: u32,
    mutation_epoch: Cell<u64>,
    mutations: Vec<DefinitionRegionMutation>,
    next_build_serial: u32,
    active_build: Option<ActiveDefinitionBuild>,
    accounting: MemoryAccounting,
    #[cfg(any(test, feature = "profiling", feature = "testing"))]
    retirement_counters: Rc<DefinitionRetirementCounterCells>,
    _brand: PhantomData<fn(&G) -> &G>,
}

fn admit_resident_macro_body<G>(
    id: DefinitionRef<G>,
    region: DefinitionRegionRef<'_>,
    header: DefinitionHeader,
) -> Option<ResidentMacroBody<G>> {
    let replacement_start = header.start.checked_add(header.parameter_len)?;
    let replacement_len = header.end.checked_sub(replacement_start)?;
    if header.start > replacement_start || header.end > region.word_head {
        return None;
    }
    let owner = region.owner.as_ref()?;
    let (current_chunk, chunk_index, chunk_slot, remaining_in_chunk) = if replacement_len == 0 {
        (None, 0, 0, 0)
    } else {
        let (chunk_index, chunk_slot) = definition_word_chunk_coordinate(replacement_start);
        let current_chunk = owner.chunk_handle(chunk_index)?;
        let remaining_in_chunk = replacement_len
            .min(u32::try_from(current_chunk.capacity().checked_sub(chunk_slot)?).ok()?);
        (
            Some(current_chunk),
            chunk_index,
            chunk_slot,
            remaining_in_chunk,
        )
    };
    #[cfg(any(test, feature = "testing"))]
    RESIDENT_MACRO_BODY_READ_COUNTERS.set({
        let mut counters = RESIDENT_MACRO_BODY_READ_COUNTERS.get();
        counters.admission_chunk_lookups = counters
            .admission_chunk_lookups
            .saturating_add(u64::from(replacement_len != 0));
        counters.region_owner_acquisitions = counters.region_owner_acquisitions.saturating_add(1);
        counters
    });
    let owner = Rc::clone(owner);
    drop(region);
    Some(ResidentMacroBody {
        definition: id,
        owner,
        parameter_start: header.start,
        start: replacement_start,
        end: header.end,
        position: 0,
        chunk_index,
        chunk_slot,
        remaining_in_chunk,
        current_chunk,
    })
}

impl<G> DefinitionArena<G> {
    pub(crate) fn admit_macro_definition(
        &self,
        id: DefinitionRef<G>,
    ) -> Option<AdmittedMacroDefinition<G>> {
        let region_id = id.region();
        let region = self.region(region_id)?;
        let header = *region.headers.get(id.row_index() as usize)?;
        if header.parameter_len == 0 && header.start == header.end {
            return Some(AdmittedMacroDefinition::SimpleMacro {
                pattern: header.pattern,
                body: None,
            });
        }
        let body = admit_resident_macro_body(id, region, header)?;
        let pattern = header.pattern;
        let parameter_len = header.parameter_len as usize;
        if parameter_len == 0 {
            Some(AdmittedMacroDefinition::SimpleMacro {
                pattern,
                body: Some(body),
            })
        } else {
            Some(AdmittedMacroDefinition::MatchingMacro {
                pattern,
                parameter_len,
                body,
            })
        }
    }

    pub(crate) fn admit_macro_body(
        &self,
        id: DefinitionRef<G>,
    ) -> Option<(MacroParameterPattern, usize, ResidentMacroBody<G>)> {
        let region_id = id.region();
        let region = self.region(region_id)?;
        let header = *region.headers.get(id.row_index() as usize)?;
        let parameter_len = header.parameter_len as usize;
        let pattern = header.pattern;
        let body = admit_resident_macro_body(id, region, header)?;
        Some((pattern, parameter_len, body))
    }

    fn try_get(&self, id: DefinitionRef<G>) -> Option<DefinitionView<'_, G>> {
        let region = self.region(id.region())?;
        let row = id.row_index() as usize;
        region.headers.get(row)?;
        Some(DefinitionView {
            region,
            row,
            _brand: PhantomData,
        })
    }

    fn raw_contents_equal(
        left_pattern: MacroParameterPattern,
        left_parameters: DefinitionWords<'_>,
        left_replacement: DefinitionWords<'_>,
        right_pattern: MacroParameterPattern,
        right_parameters: DefinitionWords<'_>,
        right_replacement: DefinitionWords<'_>,
    ) -> bool {
        left_pattern == right_pattern
            && left_parameters.len() == right_parameters.len()
            && left_parameters.iter().eq(right_parameters.iter())
            && left_replacement.len() == right_replacement.len()
            && left_replacement.iter().eq(right_replacement.iter())
    }

    pub(crate) fn contents_equal(&self, left: DefinitionRef<G>, right: DefinitionRef<G>) -> bool {
        if left == right {
            return true;
        }
        let Some(left) = self.try_get(left) else {
            return false;
        };
        let Some(right) = self.try_get(right) else {
            return false;
        };
        Self::raw_contents_equal(
            left.parameter_pattern(),
            left.parameter_text(),
            left.replacement_text(),
            right.parameter_pattern(),
            right.parameter_text(),
            right.replacement_text(),
        )
    }

    pub(crate) fn current_and_accepted_contents_equal(
        &self,
        accepted: &AcceptedDefinitionTail<G>,
        current: DefinitionRef<G>,
        prior: DefinitionRef<G>,
    ) -> bool {
        if current == prior {
            return true;
        }
        let Some(current) = self.try_get(current) else {
            return false;
        };
        if let Some(prior) = self.try_get(prior) {
            return Self::raw_contents_equal(
                current.parameter_pattern(),
                current.parameter_text(),
                current.replacement_text(),
                prior.parameter_pattern(),
                prior.parameter_text(),
                prior.replacement_text(),
            );
        }
        for mutation in &accepted.accepted {
            let DefinitionRegionMutation::Existing { region, mark, .. } = &mutation.mutation else {
                continue;
            };
            if *region != prior.region() {
                continue;
            }
            let Some(suffix) = mutation.suffix.as_ref() else {
                return false;
            };
            let Some(suffix_row) = prior.row_index().checked_sub(mark.headers) else {
                continue;
            };
            let Some(header) = suffix.headers.get(suffix_row as usize) else {
                continue;
            };
            return current.parameter_pattern() == header.pattern
                && current
                    .parameter_text()
                    .iter()
                    .eq((header.start..header.start + header.parameter_len)
                        .filter_map(|index| suffix.owner.as_ref()?.word(index)))
                && current
                    .replacement_text()
                    .iter()
                    .eq((header.start + header.parameter_len..header.end)
                        .filter_map(|index| suffix.owner.as_ref()?.word(index)));
        }
        false
    }

    fn split_region_suffix(
        region: &mut DefinitionRegion,
        mark: DefinitionRegionMark,
    ) -> DefinitionRegionSuffix {
        assert!(mark.headers as usize <= region.headers.len());
        assert!(mark.promotions as usize <= region.promotions.len());
        let headers = region.headers.split_off(mark.headers as usize);
        DefinitionRegionSuffix {
            headers,
            owner: region.owner.as_ref().map(Rc::clone),
            promotions: region.promotions.split_off(mark.promotions as usize),
        }
    }

    fn append_region_suffix(region: &mut DefinitionRegion, mut suffix: DefinitionRegionSuffix) {
        region.headers.append(&mut suffix.headers);
        debug_assert!(match (&region.owner, &suffix.owner) {
            (Some(region), Some(suffix)) => Rc::ptr_eq(region, suffix),
            (None, None) => true,
            _ => false,
        });
        region.promotions.append(&mut suffix.promotions);
    }

    fn release_region_suffix(&self, suffix: &DefinitionRegionSuffix) {
        for header in &suffix.headers {
            self.accounting
                .release_shared_dynamic(definition_memory_words(
                    (header.end - header.start) as usize,
                ));
        }
    }

    fn advance_mutation_epoch(&self) {
        self.mutation_epoch.set(
            self.mutation_epoch
                .get()
                .checked_add(1)
                .expect("definition mutation epoch exhausted"),
        );
    }

    fn record_region_change(&mut self, region_id: u32) {
        let epoch = self.mutation_epoch.get();
        let mutation_index = u32::try_from(self.mutations.len())
            .expect("definition mutation journal capacity exhausted");
        let mutation = match region_id {
            FORMAT_REGION => {
                if self.format.changed_epoch == epoch {
                    return;
                }
                self.format.changed_epoch = epoch;
                self.format.changed_mutation = mutation_index;
                DefinitionRegionMutation::Existing {
                    region: region_id,
                    mark: DefinitionRegionMark {
                        headers: self.format.headers.len() as u32,
                        promotions: self.format.promotions.len() as u32,
                        retired: false,
                    },
                }
            }
            GLOBAL_REGION => {
                if self.global.changed_epoch == epoch {
                    return;
                }
                self.global.changed_epoch = epoch;
                self.global.changed_mutation = mutation_index;
                DefinitionRegionMutation::Existing {
                    region: region_id,
                    mark: DefinitionRegionMark {
                        headers: self.global.headers.len() as u32,
                        promotions: self.global.promotions.len() as u32,
                        retired: false,
                    },
                }
            }
            _ => {
                let mut region = self.local_slots.store.borrow_mut();
                let region = region
                    .region_mut(region_id)
                    .expect("changed local definition region exists");
                if region.data.changed_epoch == epoch {
                    return;
                }
                region.data.changed_epoch = epoch;
                region.data.changed_mutation = mutation_index;
                DefinitionRegionMutation::Existing {
                    region: region_id,
                    mark: DefinitionRegionMark {
                        headers: region.data.headers.len() as u32,
                        promotions: region.data.promotions.len() as u32,
                        retired: region.retired,
                    },
                }
            }
        };
        self.mutations.push(mutation);
    }

    fn record_created_region(&mut self, region: u32) {
        let mutation_index = u32::try_from(self.mutations.len())
            .expect("definition mutation journal capacity exhausted");
        let mut local = self
            .local_slots
            .region_mut(region)
            .expect("created local definition region exists");
        local.changed_epoch = self.mutation_epoch.get();
        local.changed_mutation = mutation_index;
        drop(local);
        self.mutations
            .push(DefinitionRegionMutation::Created { region });
    }

    fn local_retired(&self, key: u32) -> Option<bool> {
        self.local_slots
            .store
            .borrow()
            .region(key)
            .map(|region| region.retired)
    }

    fn set_local_retired(&self, key: u32, retired: bool) {
        if let Some(region) = self.local_slots.store.borrow_mut().region_mut(key) {
            region.retired = retired;
        }
    }

    fn pin_active_region(&self, key: u32) -> LocalRegionPin<G> {
        if key == 0 {
            return LocalRegionPin {
                region: None,
                _brand: PhantomData,
            };
        }
        assert!(self.local_slots.acquire(key));
        LocalRegionPin {
            region: Some(LocalDefinitionRegionLease {
                slots: Rc::clone(&self.local_slots),
                key,
            }),
            _brand: PhantomData,
        }
    }

    fn undo_mutation(&mut self, mut mutation: DefinitionRegionMutation) -> AcceptedRegionMutation {
        match &mut mutation {
            DefinitionRegionMutation::Existing { region, mark } => {
                let region_id = *region;
                let mark = *mark;
                let head_retired = self.local_retired(region_id).unwrap_or(false);
                let suffix = {
                    let mut data = self
                        .region_mut(region_id)
                        .expect("changed definition region remains addressable");
                    Self::split_region_suffix(&mut data, mark)
                };
                if region_id >= 3 {
                    self.local_slots.remove_rows(suffix.headers.len());
                    self.set_local_retired(region_id, mark.retired);
                }
                AcceptedRegionMutation {
                    mutation,
                    suffix: Some(suffix),
                    head_retired,
                }
            }
            DefinitionRegionMutation::Created { region } => {
                let region_id = *region;
                let head_retired = self.local_retired(region_id).unwrap_or(true);
                if self.local_retired(region_id).is_some() {
                    self.local_slots.retire(region_id);
                }
                AcceptedRegionMutation {
                    mutation,
                    suffix: None,
                    head_retired,
                }
            }
        }
    }

    fn replay_mutation(&mut self, accepted: &mut AcceptedRegionMutation) {
        match &mut accepted.mutation {
            DefinitionRegionMutation::Existing { region, .. } => {
                let region_id = *region;
                let suffix = accepted
                    .suffix
                    .take()
                    .expect("existing region mutation owns its detached suffix");
                if region_id >= 3 {
                    self.local_slots.add_rows(suffix.headers.len());
                }
                let mut data = self
                    .region_mut(region_id)
                    .expect("accepted definition region remains addressable");
                Self::append_region_suffix(&mut data, suffix);
                drop(data);
                if region_id >= 3 {
                    self.set_local_retired(region_id, accepted.head_retired);
                }
            }
            DefinitionRegionMutation::Created { region } => {
                self.set_local_retired(*region, accepted.head_retired);
            }
        }
    }

    pub(crate) fn begin_checkpoint_candidate(
        &mut self,
        cursor: DefinitionArenaCursor,
    ) -> AcceptedDefinitionTail<G> {
        assert!(self.validates_cursor(cursor));
        assert!(self.active_build.is_none());
        let head = self.cursor();
        let head_lease = self.pin_active_region(head.active_local);
        let mutations = self.mutations.split_off(cursor.mutation_mark as usize);
        let mut accepted = Vec::with_capacity(mutations.len());
        for mutation in mutations.into_iter().rev() {
            accepted.push(self.undo_mutation(mutation));
            #[cfg(any(test, feature = "profiling", feature = "testing"))]
            self.retirement_counters.checkpoint_region_inspections.set(
                self.retirement_counters
                    .checkpoint_region_inspections
                    .get()
                    .saturating_add(1),
            );
        }
        self.active_local = cursor.active_local;
        AcceptedDefinitionTail {
            head,
            accepted,
            _head_lease: head_lease,
            next_build_serial: self.next_build_serial,
            _brand: PhantomData,
        }
    }

    pub(crate) fn reject_checkpoint_candidate(
        &mut self,
        cursor: DefinitionArenaCursor,
        tail: AcceptedDefinitionTail<G>,
    ) {
        let mut tail = tail;
        self.restore_cursor(cursor);
        for mut accepted in tail.accepted.drain(..).rev() {
            self.replay_mutation(&mut accepted);
            self.mutations.push(accepted.mutation);
        }
        self.active_local = tail.head.active_local;
        self.next_build_serial = tail.next_build_serial;
        self.advance_mutation_epoch();
        debug_assert_eq!(self.cursor(), tail.head);
    }

    pub(crate) fn accept_checkpoint_candidate(&self, tail: AcceptedDefinitionTail<G>) {
        for accepted in &tail.accepted {
            if let Some(suffix) = &accepted.suffix {
                self.release_region_suffix(suffix);
            }
        }
    }

    pub(crate) fn cursor(&self) -> DefinitionArenaCursor {
        self.advance_mutation_epoch();
        let active_local = self.active_local;
        let active_local_rows = self
            .local_slots
            .region(active_local)
            .map_or(0, |region| region.headers.len() as u32);
        DefinitionArenaCursor {
            format_rows: self.format.headers.len() as u32,
            global_rows: self.global.headers.len() as u32,
            active_local,
            active_local_rows,
            mutation_mark: u32::try_from(self.mutations.len())
                .expect("definition mutation journal capacity exhausted"),
        }
    }

    pub(crate) fn validates_cursor(&self, cursor: DefinitionArenaCursor) -> bool {
        cursor.format_rows as usize <= self.format.headers.len()
            && cursor.global_rows as usize <= self.global.headers.len()
            && cursor.mutation_mark as usize <= self.mutations.len()
            && (cursor.active_local == 0
                || self
                    .local_slots
                    .region(cursor.active_local)
                    .is_some_and(|region| {
                        cursor.active_local_rows as usize <= region.headers.len()
                    }))
    }

    pub(crate) fn restore_cursor(&mut self, cursor: DefinitionArenaCursor) {
        assert!(self.validates_cursor(cursor));
        self.abort_active_build();
        let current_lease = self.pin_active_region(self.active_local);
        let mutations = self.mutations.split_off(cursor.mutation_mark as usize);
        for mutation in mutations.into_iter().rev() {
            let accepted = self.undo_mutation(mutation);
            if let Some(suffix) = &accepted.suffix {
                self.release_region_suffix(suffix);
            }
            #[cfg(any(test, feature = "profiling", feature = "testing"))]
            self.retirement_counters.checkpoint_region_inspections.set(
                self.retirement_counters
                    .checkpoint_region_inspections
                    .get()
                    .saturating_add(1),
            );
        }
        self.active_local = cursor.active_local;
        drop(current_lease);
        self.advance_mutation_epoch();
    }

    pub(super) fn new(
        _token: ArenaToken<G, DefinitionNamespace>,
        accounting: MemoryAccounting,
    ) -> Self {
        #[cfg(any(test, feature = "profiling", feature = "testing"))]
        let retirement_counters = Rc::new(DefinitionRetirementCounterCells::default());
        let local_slots = Rc::new(LocalDefinitionSlots::new(
            accounting.clone(),
            #[cfg(any(test, feature = "profiling", feature = "testing"))]
            Rc::clone(&retirement_counters),
        ));
        Self {
            format: DefinitionRegion::new(0),
            global: DefinitionRegion::new(0),
            local_slots,
            active_local: 0,
            mutation_epoch: Cell::new(1),
            mutations: Vec::new(),
            next_build_serial: 1,
            active_build: None,
            accounting,
            #[cfg(any(test, feature = "profiling", feature = "testing"))]
            retirement_counters,
            _brand: PhantomData,
        }
    }

    pub(crate) fn allocate(
        &mut self,
        parameter_text: &[TokenWord],
        replacement_text: &[TokenWord],
    ) -> Result<DefinitionRef<G>, DefinitionAllocationError> {
        if self.active_build.is_some() {
            return Err(DefinitionAllocationError::InvalidDefinition);
        }
        self.allocate_from_iter(
            parameter_text.iter().copied(),
            replacement_text.iter().copied(),
        )
    }

    pub(crate) fn allocate_from_iter<Parameters, Replacement>(
        &mut self,
        parameter_text: Parameters,
        replacement_text: Replacement,
    ) -> Result<DefinitionRef<G>, DefinitionAllocationError>
    where
        Parameters: ExactSizeIterator<Item = TokenWord>,
        Replacement: ExactSizeIterator<Item = TokenWord>,
    {
        if self.active_build.is_some() {
            return Err(DefinitionAllocationError::InvalidDefinition);
        }
        let mut builder = DefinitionBuilder::new();
        for word in parameter_text {
            builder.push_parameter(word).map_err(map_build_error)?;
        }
        builder.finish_parameters().map_err(map_build_error)?;
        for word in replacement_text {
            builder.push_replacement(word).map_err(map_build_error)?;
        }
        builder.seal().map_err(map_build_error)?;
        self.publish(&mut builder)
    }

    pub(crate) fn publish(
        &mut self,
        builder: &mut DefinitionBuilder,
    ) -> Result<DefinitionRef<G>, DefinitionAllocationError> {
        if self.active_build.is_some() {
            return Err(DefinitionAllocationError::InvalidDefinition);
        }
        self.validate_builder(builder)?;
        self.reserve_batch(1, builder.words().len())?;
        Ok(self.publish_prevalidated(builder))
    }

    pub(crate) fn publish_prevalidated(
        &mut self,
        builder: &mut DefinitionBuilder,
    ) -> DefinitionRef<G> {
        self.publish_prevalidated_to(builder, GLOBAL_REGION)
    }

    /// Writes one validated wire definition into the immutable format region
    /// without first rebuilding it in a detached `DefinitionBuilder`.
    pub(crate) fn publish_format_row(
        &mut self,
        row: &crate::format::schema::FormatDefinition,
    ) -> Result<DefinitionRef<G>, DefinitionAllocationError> {
        if self.active_build.is_some() {
            return Err(DefinitionAllocationError::InvalidDefinition);
        }
        let parameter_len = u32::try_from(row.parameter_text.len())
            .map_err(|_| DefinitionAllocationError::CapacityOverflow)?;
        let word_len = row
            .parameter_text
            .len()
            .checked_add(row.replacement_text.len())
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        let mut pattern = MacroParameterPatternBuilder::new();
        for &word in &row.parameter_text {
            pattern
                .push_parameter(TokenWord::from_raw(word))
                .map_err(DefinitionBuildError::from)
                .map_err(map_build_error)?;
        }
        for &word in &row.replacement_text {
            pattern
                .validate_replacement(TokenWord::from_raw(word))
                .map_err(DefinitionBuildError::from)
                .map_err(map_build_error)?;
        }

        self.record_region_change(FORMAT_REGION);
        let accounting = self.accounting.clone();
        let mut region = self
            .region_mut(FORMAT_REGION)
            .expect("fixed definition region exists");
        let definition_row = NonZeroU32::new(
            u32::try_from(region.headers.len() + 1)
                .expect("format batch preflight reserved every definition row"),
        )
        .expect("definition row zero is not publishable");
        let start = region.begin_word_span();
        for &word in row.parameter_text.iter().chain(&row.replacement_text) {
            region
                .push_word(TokenWord::from_raw(word))
                .expect("format batch preflight reserved the complete word extent");
        }
        let end = region.begin_word_span();
        region.push_header(DefinitionHeader {
            start,
            parameter_len,
            end,
            origin: crate::token::OriginId::UNKNOWN,
            pattern: pattern.finish(),
        });
        drop(region);
        accounting.allocate_shared_dynamic(definition_memory_words(word_len));
        Ok(DefinitionRef::new(FORMAT_REGION, definition_row))
    }

    fn publish_prevalidated_to(
        &mut self,
        builder: &mut DefinitionBuilder,
        region_id: u32,
    ) -> DefinitionRef<G> {
        self.record_region_change(region_id);
        let accounting = self.accounting.clone();
        let mut region = self
            .region_mut(region_id)
            .expect("fixed definition region exists");
        let row = NonZeroU32::new(
            u32::try_from(region.headers.len() + 1)
                .expect("batch preflight reserved a definition row"),
        )
        .expect("definition row zero is not publishable");
        let (start, end) = region
            .extend_words(builder.words())
            .expect("batch preflight reserved a definition word extent");
        region.push_header(DefinitionHeader {
            start,
            parameter_len: builder.data.parameter_len,
            end,
            origin: crate::token::OriginId::UNKNOWN,
            pattern: builder.data.pattern.finish(),
        });
        drop(region);
        accounting.allocate_shared_dynamic(definition_memory_words(builder.words().len()));
        builder.data.phase = DefinitionBuildPhase::Published;
        DefinitionRef::new(region_id, row)
    }

    pub(crate) fn validate_builder(
        &self,
        builder: &DefinitionBuilder,
    ) -> Result<(), DefinitionAllocationError> {
        builder.validate_completed()?;
        u32::try_from(builder.words().len())
            .map_err(|_| DefinitionAllocationError::CapacityOverflow)?;
        Ok(())
    }

    pub(crate) fn reserve_batch(
        &mut self,
        rows: usize,
        words: usize,
    ) -> Result<(), DefinitionAllocationError> {
        self.global.reserve(rows, words).map_err(map_build_error)
    }

    pub(crate) fn reserve_format_batch(
        &mut self,
        rows: usize,
        words: usize,
    ) -> Result<(), DefinitionAllocationError> {
        self.format.reserve(rows, words).map_err(map_build_error)
    }

    fn fixed_region(&self, id: u32) -> Option<&DefinitionRegion> {
        match id {
            FORMAT_REGION => Some(&self.format),
            GLOBAL_REGION => Some(&self.global),
            _ => None,
        }
    }

    fn region(&self, id: u32) -> Option<DefinitionRegionRef<'_>> {
        if let Some(region) = self.fixed_region(id) {
            return Some(DefinitionRegionRef::Fixed(region));
        }
        self.local_slots.region(id).map(DefinitionRegionRef::Local)
    }

    fn region_mut(&mut self, id: u32) -> Option<DefinitionRegionMut<'_>> {
        match id {
            FORMAT_REGION => Some(DefinitionRegionMut::Fixed(&mut self.format)),
            GLOBAL_REGION => Some(DefinitionRegionMut::Fixed(&mut self.global)),
            id if local_region_address(id).is_some() => self
                .local_slots
                .region_mut(id)
                .map(DefinitionRegionMut::Local),
            _ => None,
        }
    }

    pub(crate) fn begin_group(&mut self) -> Result<(), DefinitionAllocationError> {
        if self.active_build.is_some() {
            return Err(DefinitionAllocationError::InvalidDefinition);
        }
        let parent = self.active_local;
        #[cfg(any(test, feature = "profiling", feature = "testing"))]
        self.retirement_counters.group_entry_slot_inspections.set(
            self.retirement_counters
                .group_entry_slot_inspections
                .get()
                .saturating_add(1),
        );
        let region = self.local_slots.allocate(parent)?;
        self.record_created_region(region);
        self.active_local = region;
        Ok(())
    }

    pub(crate) fn end_group(&mut self) {
        assert!(
            self.active_build.is_none(),
            "definition scan crosses group exit"
        );
        let child = self.active_local;
        assert_ne!(child, 0, "definition group stack matches TeX groups");
        let parent = self
            .local_slots
            .parent(child)
            .expect("active definition region stores its exact parent");
        self.record_region_change(child);
        self.active_local = parent;
        #[cfg(any(test, feature = "profiling", feature = "testing"))]
        self.retirement_counters.group_region_inspections.set(
            self.retirement_counters
                .group_region_inspections
                .get()
                .saturating_add(1),
        );
        if self.local_slots.lease_count(child) == Some(0)
            && matches!(
                self.mutations.last(),
                Some(DefinitionRegionMutation::Created { region }) if *region == child
            )
        {
            self.mutations.pop();
        }
        self.local_slots.retire(child);
    }

    pub(crate) fn checkpoint_lease(&self) -> DefinitionCheckpointLease<G> {
        if self.active_local != 0 {
            #[cfg(any(test, feature = "profiling", feature = "testing"))]
            self.retirement_counters.checkpoint_region_inspections.set(
                self.retirement_counters
                    .checkpoint_region_inspections
                    .get()
                    .saturating_add(1),
            );
        }
        DefinitionCheckpointLease {
            region: self.pin_active_region(self.active_local),
        }
    }

    #[cfg(test)]
    fn lease(&self, id: DefinitionRef<G>) -> LocalRegionPin<G> {
        let region = (id.region() >= 3).then(|| {
            assert!(self.local_slots.acquire(id.region()));
            LocalDefinitionRegionLease {
                slots: Rc::clone(&self.local_slots),
                key: id.region(),
            }
        });
        LocalRegionPin {
            region,
            _brand: PhantomData,
        }
    }

    pub(crate) fn begin_build(
        &mut self,
        destination: DefinitionDestination,
        origin: crate::token::OriginId,
    ) -> Result<DefinitionBuildKey<G>, DefinitionAllocationError> {
        if self.active_build.is_some() {
            return Err(DefinitionAllocationError::InvalidDefinition);
        }
        let region = match destination {
            DefinitionDestination::Format => FORMAT_REGION,
            DefinitionDestination::Global => GLOBAL_REGION,
            DefinitionDestination::Local => {
                if self.active_local == 0 {
                    return Err(DefinitionAllocationError::InvalidDefinition);
                }
                self.active_local
            }
        };
        let serial = NonZeroU32::new(self.next_build_serial)
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        self.next_build_serial = self
            .next_build_serial
            .checked_add(1)
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        let mark = self
            .region(region)
            .expect("selected definition region exists")
            .build_mark(region, serial);
        self.record_region_change(region);
        self.active_build = Some(ActiveDefinitionBuild {
            serial,
            region,
            mark,
            parameter_len: 0,
            replacement_len: 0,
            pattern: MacroParameterPatternBuilder::new(),
            origin,
            phase: DefinitionBuildPhase::OpenParameters,
        });
        Ok(DefinitionBuildKey {
            serial,
            _brand: PhantomData,
        })
    }

    fn active_build_mut(
        &mut self,
        key: DefinitionBuildKey<G>,
    ) -> Result<&mut ActiveDefinitionBuild, DefinitionBuildError> {
        self.active_build
            .as_mut()
            .filter(|build| build.serial == key.serial)
            .ok_or(DefinitionBuildError::InvalidPhase)
    }

    pub(crate) fn push_parameter(
        &mut self,
        key: DefinitionBuildKey<G>,
        word: TokenWord,
    ) -> Result<(), DefinitionBuildError> {
        let build = self.active_build_mut(key)?;
        if build.phase != DefinitionBuildPhase::OpenParameters {
            return Err(DefinitionBuildError::InvalidPhase);
        }
        let mut pattern = build.pattern;
        pattern.push_parameter(word)?;
        let parameter_len = build
            .parameter_len
            .checked_add(1)
            .ok_or(DefinitionBuildError::CapacityOverflow)?;
        let region_id = build.region;
        self.region_mut(region_id)
            .expect("active build region exists")
            .push_word(word)?;
        let build = self.active_build_mut(key)?;
        build.parameter_len = parameter_len;
        build.pattern = pattern;
        Ok(())
    }

    pub(crate) fn finish_parameters(
        &mut self,
        key: DefinitionBuildKey<G>,
    ) -> Result<(), DefinitionBuildError> {
        let build = self.active_build_mut(key)?;
        if build.phase != DefinitionBuildPhase::OpenParameters {
            return Err(DefinitionBuildError::InvalidPhase);
        }
        build.phase = DefinitionBuildPhase::OpenReplacement;
        Ok(())
    }

    pub(crate) fn set_build_origin(
        &mut self,
        key: DefinitionBuildKey<G>,
        origin: crate::token::OriginId,
    ) -> Result<(), DefinitionBuildError> {
        self.active_build_mut(key)?.origin = origin;
        Ok(())
    }

    pub(crate) fn push_replacement(
        &mut self,
        key: DefinitionBuildKey<G>,
        word: TokenWord,
    ) -> Result<(), DefinitionBuildError> {
        let build = self.active_build_mut(key)?;
        if build.phase != DefinitionBuildPhase::OpenReplacement {
            return Err(DefinitionBuildError::InvalidPhase);
        }
        build.pattern.validate_replacement(word)?;
        let replacement_len = build
            .replacement_len
            .checked_add(1)
            .ok_or(DefinitionBuildError::CapacityOverflow)?;
        let region_id = build.region;
        self.region_mut(region_id)
            .expect("active build region exists")
            .push_word(word)?;
        let build = self.active_build_mut(key)?;
        build.replacement_len = replacement_len;
        Ok(())
    }

    pub(crate) fn seal_build(
        &mut self,
        key: DefinitionBuildKey<G>,
    ) -> Result<DefinitionRef<G>, DefinitionBuildError> {
        let Some(build) = self
            .active_build
            .as_ref()
            .filter(|build| build.serial == key.serial)
        else {
            return Err(DefinitionBuildError::InvalidPhase);
        };
        if build.phase != DefinitionBuildPhase::OpenReplacement {
            return Err(DefinitionBuildError::InvalidPhase);
        }
        let region_id = build.region;
        let end = self
            .region(region_id)
            .expect("active build region exists")
            .word_head;
        self.region_mut(region_id)
            .expect("active build region exists")
            .reserve(1, 0)?;
        let build = self.active_build.take().expect("validated active build");
        let parameter_len = build.parameter_len;
        let start = build.mark.word_head;
        let origin = build.origin;
        let accounting = self.accounting.clone();
        let mut region = self
            .region_mut(region_id)
            .expect("active build region exists");
        let row = NonZeroU32::new(
            u32::try_from(region.headers.len() + 1)
                .map_err(|_| DefinitionBuildError::CapacityOverflow)?,
        )
        .expect("definition row is nonzero");
        region.push_header(DefinitionHeader {
            start,
            parameter_len,
            end,
            origin,
            pattern: build.pattern.finish(),
        });
        drop(region);
        if region_id != FORMAT_REGION && region_id != GLOBAL_REGION {
            self.local_slots.add_rows(1);
        }
        accounting.allocate_shared_dynamic(definition_memory_words((end - start) as usize));
        Ok(DefinitionRef::new(region_id, row))
    }

    pub(crate) fn abort_build(&mut self, key: DefinitionBuildKey<G>) {
        if self
            .active_build
            .as_ref()
            .is_some_and(|build| build.serial == key.serial)
        {
            self.abort_active_build();
        }
    }

    fn abort_active_build(&mut self) {
        let Some(build) = self.active_build.take() else {
            return;
        };
        let region_id = build.region;
        let mark = build.mark;
        assert_eq!(
            mark.region, region_id,
            "definition build mark belongs to its active region"
        );
        assert_eq!(
            mark.serial, build.serial,
            "definition build mark belongs to its active key"
        );
        let accounting = self.accounting.clone();
        self.region_mut(region_id)
            .expect("active build region exists")
            .restore_build_mark(mark, &accounting);
    }

    pub(crate) fn promote_global(
        &mut self,
        id: DefinitionRef<G>,
    ) -> Result<DefinitionRef<G>, DefinitionAllocationError> {
        if self.active_build.is_some() {
            return Err(DefinitionAllocationError::InvalidDefinition);
        }
        if id.region() == GLOBAL_REGION || id.region() == FORMAT_REGION {
            return Ok(id);
        }
        self.record_region_change(id.region());
        self.record_region_change(GLOBAL_REGION);
        let source_slots = Rc::clone(&self.local_slots);
        let mut source_region = source_slots
            .region_mut(id.region())
            .ok_or(DefinitionAllocationError::InvalidDefinition)?;
        if let Some(promotion) = source_region
            .promotions
            .iter()
            .find(|promotion| promotion.source_row == id.row_index())
        {
            return Ok(DefinitionRef::new(GLOBAL_REGION, promotion.destination_row));
        }
        let source_header = *source_region
            .headers
            .get(id.row_index() as usize)
            .ok_or(DefinitionAllocationError::InvalidDefinition)?;
        self.global
            .reserve(1, (source_header.end - source_header.start) as usize)
            .map_err(map_build_error)?;
        source_region
            .promotions
            .try_reserve(1)
            .map_err(|_| DefinitionAllocationError::AllocationFailed)?;
        let source_words = (source_header.start..source_header.end)
            .map(|index| {
                source_region
                    .word(index)
                    .ok_or(DefinitionAllocationError::InvalidDefinition)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (start, end) = self
            .global
            .extend_words(&source_words)
            .map_err(map_build_error)?;
        let row = NonZeroU32::new(
            u32::try_from(self.global.headers.len() + 1)
                .map_err(|_| DefinitionAllocationError::CapacityOverflow)?,
        )
        .expect("definition row is nonzero");
        self.global.push_header(DefinitionHeader {
            start,
            parameter_len: source_header.parameter_len,
            end,
            origin: source_header.origin,
            pattern: source_header.pattern,
        });
        self.accounting
            .allocate_shared_dynamic(definition_memory_words(source_words.len()));
        let promoted = DefinitionRef::new(GLOBAL_REGION, row);
        source_region.promotions.push(DefinitionPromotion {
            source_row: id.row_index(),
            destination_row: row,
        });
        Ok(promoted)
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn get(&self, id: DefinitionRef<G>) -> DefinitionView<'_, G> {
        let region = self.region(id.region()).expect("definition region is live");
        DefinitionView {
            region,
            row: id.row_index() as usize,
            _brand: PhantomData,
        }
    }

    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.format.headers.len() + self.global.headers.len() + self.local_slots.row_count()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.format.headers.is_empty()
            && self.global.headers.is_empty()
            && self.local_slots.is_empty()
    }

    #[cfg(any(test, feature = "profiling", feature = "testing"))]
    #[allow(dead_code)]
    pub(crate) fn retirement_counters(&self) -> DefinitionRetirementCounters {
        DefinitionRetirementCounters {
            group_entry_slot_inspections: self
                .retirement_counters
                .group_entry_slot_inspections
                .get(),
            local_slot_chunk_allocations: self
                .retirement_counters
                .local_slot_chunk_allocations
                .get(),
            group_region_inspections: self.retirement_counters.group_region_inspections.get(),
            lease_release_region_inspections: self
                .retirement_counters
                .lease_release_region_inspections
                .get(),
            checkpoint_region_inspections: self
                .retirement_counters
                .checkpoint_region_inspections
                .get(),
            regions_reclaimed: self.retirement_counters.regions_reclaimed.get(),
            rows_reclaimed: self.retirement_counters.rows_reclaimed.get(),
            promotions_reclaimed: self.retirement_counters.promotions_reclaimed.get(),
        }
    }
}

fn map_build_error(error: DefinitionBuildError) -> DefinitionAllocationError {
    match error {
        DefinitionBuildError::AllocationFailed => DefinitionAllocationError::AllocationFailed,
        DefinitionBuildError::CapacityOverflow => DefinitionAllocationError::CapacityOverflow,
        DefinitionBuildError::InvalidPhase | DefinitionBuildError::InvalidProgram(_) => {
            DefinitionAllocationError::InvalidDefinition
        }
    }
}

fn definition_memory_words(word_len: usize) -> usize {
    let header_words =
        std::mem::size_of::<DefinitionHeader>().div_ceil(std::mem::size_of::<usize>());
    word_len
        .checked_mul(std::mem::size_of::<TokenWord>())
        .and_then(|bytes| bytes.checked_add(std::mem::size_of::<usize>() - 1))
        .map(|bytes| bytes / std::mem::size_of::<usize>())
        .and_then(|words| words.checked_add(header_words))
        .expect("validated definition length has a canonical word count")
}

pub struct DefinitionView<'a, G> {
    region: DefinitionRegionRef<'a>,
    row: usize,
    _brand: PhantomData<fn(&G) -> &G>,
}

#[derive(Clone, Copy)]
pub struct DefinitionWords<'a> {
    owner: &'a DefinitionRegionOwner,
    start: u32,
    len: u32,
}

impl<'a> DefinitionWords<'a> {
    #[must_use]
    pub const fn len(self) -> usize {
        self.len as usize
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    #[must_use]
    pub fn get(self, index: usize) -> Option<TokenWord> {
        (index < self.len as usize).then(|| self.owner.word(self.start + index as u32))?
    }

    #[must_use]
    pub const fn iter(self) -> DefinitionWordIter<'a> {
        DefinitionWordIter {
            words: self,
            position: 0,
        }
    }

    #[must_use]
    pub fn to_vec(self) -> Vec<TokenWord> {
        self.iter().collect()
    }
}

impl core::fmt::Debug for DefinitionWords<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_list().entries(self.iter()).finish()
    }
}

impl<T> PartialEq<T> for DefinitionWords<'_>
where
    T: AsRef<[TokenWord]> + ?Sized,
{
    fn eq(&self, other: &T) -> bool {
        self.len() == other.as_ref().len() && self.iter().eq(other.as_ref().iter().copied())
    }
}

impl PartialEq for DefinitionWords<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl Eq for DefinitionWords<'_> {}

impl<'a> IntoIterator for DefinitionWords<'a> {
    type Item = TokenWord;
    type IntoIter = DefinitionWordIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

pub struct DefinitionWordIter<'a> {
    words: DefinitionWords<'a>,
    position: u32,
}

impl Iterator for DefinitionWordIter<'_> {
    type Item = TokenWord;

    fn next(&mut self) -> Option<Self::Item> {
        let word = self.words.get(self.position as usize)?;
        self.position += 1;
        Some(word)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.words.len() - self.position as usize;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for DefinitionWordIter<'_> {}

impl<'a, G> DefinitionView<'a, G> {
    fn header(&self) -> &DefinitionHeader {
        &self.region.headers[self.row]
    }

    #[must_use]
    pub fn parameter_pattern(&self) -> MacroParameterPattern {
        self.header().pattern
    }

    #[must_use]
    pub fn parameter_text(&self) -> DefinitionWords<'_> {
        let header = self.header();
        DefinitionWords {
            owner: self
                .region
                .owner
                .as_deref()
                .expect("definition region owns parameter words"),
            start: header.start,
            len: header.parameter_len,
        }
    }

    #[must_use]
    pub fn replacement_text(&self) -> DefinitionWords<'_> {
        let header = self.header();
        DefinitionWords {
            owner: self
                .region
                .owner
                .as_deref()
                .expect("definition region owns replacement words"),
            start: header.start + header.parameter_len,
            len: header.end - header.start - header.parameter_len,
        }
    }

    #[must_use]
    pub fn replacement_word(&self, index: usize) -> Option<TokenWord> {
        self.replacement_text().get(index)
    }

    /// Computes the allocation-independent content identity only at a cold
    /// semantic-state boundary. Definition construction and delivery never
    /// maintain or carry this value.
    pub(crate) fn semantic_identity(&self) -> Option<u64> {
        let mut identity = StateHasher::new(DEFINITION_IDENTITY_V2_DOMAIN);
        identity.tag(PARAMETER_START);
        for word in self.parameter_text().iter() {
            identity.u32(word.raw());
        }
        identity.tag(PARAMETER_END);
        identity.u32(self.parameter_text().len() as u32);
        identity.tag(REPLACEMENT_START);
        for word in self.replacement_text().iter() {
            identity.u32(word.raw());
        }
        identity.tag(REPLACEMENT_END);
        identity.u32(self.replacement_text().len() as u32);
        Some(identity.finish().max(1))
    }

    #[must_use]
    pub fn definition_origin(&self) -> crate::token::OriginId {
        self.header().origin
    }

    pub(crate) fn capture_format(&self) -> crate::format::schema::FormatDefinition {
        crate::format::schema::FormatDefinition {
            parameter_text: self
                .parameter_text()
                .iter()
                .map(|word| word.raw())
                .collect(),
            replacement_text: self
                .replacement_text()
                .iter()
                .map(|word| word.raw())
                .collect(),
        }
    }
}
