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
const DEFINITION_WORD_CHUNK_CAPACITY: usize = 4096;
const LOCAL_SLOT_CHUNK_LEN: usize = 64;
const LOCAL_SLOT_ADDRESS_CAPACITY: usize = u16::MAX as usize - 2;

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

struct DefinitionRegion {
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
            owner
                .headers
                .borrow_mut()
                .try_reserve(rows)
                .map_err(|_| DefinitionBuildError::AllocationFailed)?;
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
        self.owner
            .get_or_insert_with(|| Rc::new(DefinitionRegionOwner::new()))
            .headers
            .borrow_mut()
            .push(header);
        self.headers.push(header);
    }
}

struct DefinitionWordChunk {
    words: Box<[Cell<TokenWord>; DEFINITION_WORD_CHUNK_CAPACITY]>,
}

impl DefinitionWordChunk {
    fn new() -> Result<Self, DefinitionBuildError> {
        let mut words = Vec::new();
        words
            .try_reserve_exact(DEFINITION_WORD_CHUNK_CAPACITY)
            .map_err(|_| DefinitionBuildError::AllocationFailed)?;
        words.resize_with(DEFINITION_WORD_CHUNK_CAPACITY, || {
            Cell::new(TokenWord::from_raw(0))
        });
        let words = words
            .into_boxed_slice()
            .try_into()
            .map_err(|_| DefinitionBuildError::AllocationFailed)?;
        Ok(Self { words })
    }
}

struct DefinitionRegionOwner {
    /// Flat append-only directory of stable coarse word allocations.
    ///
    /// A resident cursor cannot safely retain a borrow into the same `Rc`
    /// that owns this directory without becoming self-referential. Keeping
    /// the directory flat makes the required short reborrow one checked,
    /// constant-time slot access instead of a linked-page walk. The boxed word
    /// arrays never move and have no owner or lifetime independent of this
    /// region.
    words: RefCell<Vec<DefinitionWordChunk>>,
    headers: RefCell<Vec<DefinitionHeader>>,
}

impl DefinitionRegionOwner {
    fn new() -> Self {
        Self {
            words: RefCell::new(Vec::new()),
            headers: RefCell::new(Vec::new()),
        }
    }

    fn push_word(&self, index: u32, word: TokenWord) -> Result<(), DefinitionBuildError> {
        let chunk = index as usize / DEFINITION_WORD_CHUNK_CAPACITY;
        self.ensure_chunk(chunk)?;
        self.words.borrow()[chunk].words[index as usize % DEFINITION_WORD_CHUNK_CAPACITY].set(word);
        Ok(())
    }

    fn ensure_chunk(&self, chunk: usize) -> Result<(), DefinitionBuildError> {
        let mut words = self.words.borrow_mut();
        if chunk >= words.len() {
            let additional = chunk + 1 - words.len();
            words
                .try_reserve_exact(additional)
                .map_err(|_| DefinitionBuildError::AllocationFailed)?;
            while words.len() <= chunk {
                words.push(DefinitionWordChunk::new()?);
            }
        }
        Ok(())
    }

    fn reserve_word_span(&self, start: u32, end: u32) -> Result<(), DefinitionBuildError> {
        if start == end {
            return Ok(());
        }
        let first = start as usize / DEFINITION_WORD_CHUNK_CAPACITY;
        let last = (end - 1) as usize / DEFINITION_WORD_CHUNK_CAPACITY;
        for chunk in first..=last {
            self.ensure_chunk(chunk)?;
        }
        Ok(())
    }

    #[inline(always)]
    fn word(&self, index: u32) -> Option<TokenWord> {
        let chunk = index as usize / DEFINITION_WORD_CHUNK_CAPACITY;
        self.word_in_chunk(
            chunk as u32,
            index as usize % DEFINITION_WORD_CHUNK_CAPACITY,
        )
    }

    #[inline(always)]
    fn word_in_chunk(&self, chunk: u32, offset: usize) -> Option<TokenWord> {
        self.words
            .borrow()
            .get(chunk as usize)?
            .words
            .get(offset)
            .map(Cell::get)
    }

    fn has_chunk(&self, chunk: u32) -> bool {
        (chunk as usize) < self.words.borrow().len()
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

/// Store-minted resident cursor for one executing macro replacement body.
///
/// The definition store resolves the opaque reference and immutable header
/// once at admission, including validation of the initial coarse chunk. The
/// cursor then retains the exact replacement extent and the exact region owner;
/// downstream command code can neither inspect nor manufacture its storage
/// coordinates. Safe Rust requires a short constant-time borrow of the
/// region's flat chunk directory for each word: caching a direct chunk borrow
/// beside its owning `Rc` would be self-referential. This store owner therefore
/// keeps the absolute cursor and exposes only its relative semantic position;
/// rollback swaps one opaque coordinate, and only a 4,096-word crossing changes
/// the derived chunk coordinate.
pub struct ResidentMacroBody<G> {
    definition: DefinitionRef<G>,
    owner: Rc<DefinitionRegionOwner>,
    start: u32,
    position: u32,
    end: u32,
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

    /// Current replacement-relative delivery position.
    #[must_use]
    pub const fn position(&self) -> usize {
        (self.position - self.start) as usize
    }

    /// Advances the store-owned absolute cursor and returns its relative
    /// semantic position with the resident word.
    #[must_use]
    #[inline(always)]
    pub fn advance_word(&mut self) -> Option<(u32, TokenWord)> {
        let absolute = self.position;
        (absolute < self.end).then_some(())?;
        let word = self.owner.word(absolute)?;
        self.record_word_read(absolute);
        self.position = absolute + 1;
        Some((absolute - self.start, word))
    }

    #[must_use]
    pub const fn cursor(&self) -> ResidentMacroBodyCursor {
        ResidentMacroBodyCursor(self.position)
    }

    pub fn swap_cursor(&mut self, cursor: &mut ResidentMacroBodyCursor) {
        core::mem::swap(&mut self.position, &mut cursor.0);
    }

    #[must_use]
    #[inline(always)]
    pub fn word(&self, position: usize) -> Option<TokenWord> {
        (position < self.len()).then_some(())?;
        let absolute = self.start + position as u32;
        self.record_word_read(absolute);
        self.owner.word(absolute)
    }

    #[inline(always)]
    fn record_word_read(&self, _absolute: u32) {
        #[cfg(any(test, feature = "testing"))]
        RESIDENT_MACRO_BODY_READ_COUNTERS.set({
            let mut counters = RESIDENT_MACRO_BODY_READ_COUNTERS.get();
            counters.direct_chunk_slot_reads = counters.direct_chunk_slot_reads.saturating_add(1);
            if _absolute != self.start
                && _absolute / DEFINITION_WORD_CHUNK_CAPACITY as u32
                    != (_absolute - 1) / DEFINITION_WORD_CHUNK_CAPACITY as u32
            {
                counters.chunk_boundary_transitions =
                    counters.chunk_boundary_transitions.saturating_add(1);
            }
            counters
        });
    }

    #[cfg(any(test, feature = "profiling", feature = "testing"))]
    #[doc(hidden)]
    #[must_use]
    pub fn profile_region_owner_count(&self) -> usize {
        Rc::strong_count(&self.owner)
    }
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
            && self.position == other.position
            && self.end == other.end
            && Rc::ptr_eq(&self.owner, &other.owner)
    }
}

impl<G> Eq for ResidentMacroBody<G> {}

impl<G> core::hash::Hash for ResidentMacroBody<G> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.definition.hash(state);
        self.start.hash(state);
        self.position.hash(state);
        self.end.hash(state);
        (Rc::as_ptr(&self.owner) as usize).hash(state);
    }
}

/// Opaque rollback coordinate for one store-owned resident replacement cursor.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ResidentMacroBodyCursor(u32);

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
    word_start: u32,
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

#[derive(Clone, Copy)]
struct DefinitionOriginEdit {
    row: u32,
    origin: crate::token::OriginId,
}

enum DefinitionRegionMutation {
    Existing {
        region: u32,
        mark: DefinitionRegionMark,
        origin_edits: smallvec::SmallVec<[DefinitionOriginEdit; 1]>,
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

impl<G> DefinitionArena<G> {
    pub(crate) fn admit_macro_body(
        &self,
        id: DefinitionRef<G>,
    ) -> Option<(MacroParameterPattern, usize, ResidentMacroBody<G>)> {
        let region_id = id.region();
        let region = self.region(region_id)?;
        let header = *region.headers.get(id.row_index() as usize)?;
        let replacement_start = header.start.checked_add(header.parameter_len)?;
        let replacement_len = header.end.checked_sub(replacement_start)?;
        let owner = region.owner.as_ref()?;
        let initial_chunk = replacement_start / DEFINITION_WORD_CHUNK_CAPACITY as u32;
        if replacement_len != 0 && !owner.has_chunk(initial_chunk) {
            return None;
        }
        #[cfg(any(test, feature = "testing"))]
        RESIDENT_MACRO_BODY_READ_COUNTERS.set({
            let mut counters = RESIDENT_MACRO_BODY_READ_COUNTERS.get();
            counters.admission_chunk_lookups = counters
                .admission_chunk_lookups
                .saturating_add(u64::from(replacement_len != 0));
            counters.region_owner_acquisitions =
                counters.region_owner_acquisitions.saturating_add(1);
            counters
        });
        let owner = Rc::clone(owner);
        let parameter_len = header.parameter_len as usize;
        let pattern = header.pattern;
        drop(region);
        Some((
            pattern,
            parameter_len,
            ResidentMacroBody {
                definition: id,
                owner,
                start: replacement_start,
                position: replacement_start,
                end: header.end,
            },
        ))
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

    fn record_region_change(&mut self, region_id: u32) -> usize {
        let epoch = self.mutation_epoch.get();
        let mutation_index = u32::try_from(self.mutations.len())
            .expect("definition mutation journal capacity exhausted");
        let mutation = match region_id {
            FORMAT_REGION => {
                if self.format.changed_epoch == epoch {
                    return self.format.changed_mutation as usize;
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
                    origin_edits: smallvec::SmallVec::new(),
                }
            }
            GLOBAL_REGION => {
                if self.global.changed_epoch == epoch {
                    return self.global.changed_mutation as usize;
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
                    origin_edits: smallvec::SmallVec::new(),
                }
            }
            _ => {
                let mut region = self.local_slots.store.borrow_mut();
                let region = region
                    .region_mut(region_id)
                    .expect("changed local definition region exists");
                if region.data.changed_epoch == epoch {
                    return region.data.changed_mutation as usize;
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
                    origin_edits: smallvec::SmallVec::new(),
                }
            }
        };
        self.mutations.push(mutation);
        mutation_index as usize
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

    fn swap_origin_edits(region: &mut DefinitionRegion, edits: &mut [DefinitionOriginEdit]) {
        for edit in edits {
            let header = region
                .headers
                .get_mut(edit.row as usize)
                .expect("journaled definition origin row remains addressable");
            core::mem::swap(&mut header.origin, &mut edit.origin);
        }
    }

    fn undo_mutation(&mut self, mut mutation: DefinitionRegionMutation) -> AcceptedRegionMutation {
        match &mut mutation {
            DefinitionRegionMutation::Existing {
                region,
                mark,
                origin_edits,
            } => {
                let region_id = *region;
                let mark = *mark;
                let head_retired = self.local_retired(region_id).unwrap_or(false);
                let suffix = {
                    let mut data = self
                        .region_mut(region_id)
                        .expect("changed definition region remains addressable");
                    let suffix = Self::split_region_suffix(&mut data, mark);
                    Self::swap_origin_edits(&mut data, origin_edits);
                    suffix
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
            DefinitionRegionMutation::Existing {
                region,
                origin_edits,
                ..
            } => {
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
                Self::swap_origin_edits(&mut data, origin_edits);
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

    pub(crate) fn publish_format_prevalidated(
        &mut self,
        builder: &mut DefinitionBuilder,
    ) -> DefinitionRef<G> {
        self.publish_prevalidated_to(builder, FORMAT_REGION)
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
        self.record_region_change(region);
        let word_start = self
            .region_mut(region)
            .expect("selected definition region exists")
            .begin_word_span();
        let serial = NonZeroU32::new(self.next_build_serial)
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        self.next_build_serial = self
            .next_build_serial
            .checked_add(1)
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        self.active_build = Some(ActiveDefinitionBuild {
            serial,
            region,
            word_start,
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
        let start = build.word_start;
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
        let _ = self.active_build.take();
    }

    pub(crate) fn promote_global(
        &mut self,
        id: DefinitionRef<G>,
    ) -> Result<DefinitionRef<G>, DefinitionAllocationError> {
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

    pub(crate) fn set_origin(
        &mut self,
        id: DefinitionRef<G>,
        origin: crate::token::OriginId,
    ) -> Result<(), DefinitionAllocationError> {
        let region_id = id.region();
        let row = id.row_index();
        let old_origin = self
            .region(region_id)
            .and_then(|region| region.headers.get(row as usize).map(|header| header.origin))
            .ok_or(DefinitionAllocationError::InvalidDefinition)?;
        let mutation_index = self.record_region_change(region_id);
        if let DefinitionRegionMutation::Existing {
            mark, origin_edits, ..
        } = self
            .mutations
            .get_mut(mutation_index)
            .expect("changed definition region mutation remains addressable")
            && row < mark.headers
        {
            match origin_edits.binary_search_by_key(&row, |edit| edit.row) {
                Ok(_) => {}
                Err(index) => {
                    origin_edits
                        .try_reserve(1)
                        .map_err(|_| DefinitionAllocationError::AllocationFailed)?;
                    origin_edits.insert(
                        index,
                        DefinitionOriginEdit {
                            row,
                            origin: old_origin,
                        },
                    );
                }
            }
        }
        let mut region = self
            .region_mut(region_id)
            .ok_or(DefinitionAllocationError::InvalidDefinition)?;
        let header = region
            .headers
            .get_mut(row as usize)
            .ok_or(DefinitionAllocationError::InvalidDefinition)?;
        header.origin = origin;
        Ok(())
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
