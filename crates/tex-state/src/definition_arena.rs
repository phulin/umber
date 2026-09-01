//! Immutable, generation-branded macro-definition storage.

use core::marker::PhantomData;
use core::num::NonZeroU32;
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

/// Identity work admitted by one destination generation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DefinitionIdentityPolicy {
    Disabled,
    Enabled,
}

impl DefinitionIdentityPolicy {
    const fn enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }
}

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
    identity: Option<StateHasher>,
    sealed_identity: u64,
    policy: DefinitionIdentityPolicy,
    phase: DefinitionBuildPhase,
    #[cfg(test)]
    fail_next_reserve: bool,
}

impl DefinitionData {
    fn new(policy: DefinitionIdentityPolicy) -> Self {
        let mut data = Self {
            words: Vec::new(),
            parameter_len: 0,
            replacement_len: 0,
            pattern: MacroParameterPatternBuilder::new(),
            identity: None,
            sealed_identity: 0,
            policy,
            phase: DefinitionBuildPhase::OpenParameters,
            #[cfg(test)]
            fail_next_reserve: false,
        };
        data.reset(policy);
        data
    }

    fn reset(&mut self, policy: DefinitionIdentityPolicy) {
        self.words.clear();
        self.parameter_len = 0;
        self.replacement_len = 0;
        self.pattern = MacroParameterPatternBuilder::new();
        self.identity = policy.enabled().then(|| {
            let mut hasher = StateHasher::new(DEFINITION_IDENTITY_V2_DOMAIN);
            hasher.tag(PARAMETER_START);
            hasher
        });
        self.sealed_identity = 0;
        self.policy = policy;
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

impl DefinitionBuilder {
    #[must_use]
    pub fn new(policy: DefinitionIdentityPolicy) -> Self {
        Self {
            data: DefinitionData::new(policy),
        }
    }

    /// Clears one recycled cold-path staging row.
    pub fn reset(&mut self, policy: DefinitionIdentityPolicy) {
        self.data.reset(policy);
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
        if let Some(identity) = &mut data.identity {
            identity.u32(word.raw());
        }
        data.words.push(word);
        Ok(())
    }

    pub fn finish_parameters(&mut self) -> Result<(), DefinitionBuildError> {
        let data = self.data_mut();
        if data.phase != DefinitionBuildPhase::OpenParameters {
            return Err(DefinitionBuildError::InvalidPhase);
        }
        if let Some(identity) = &mut data.identity {
            identity.tag(PARAMETER_END);
            identity.u32(data.parameter_len);
            identity.tag(REPLACEMENT_START);
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
        if let Some(identity) = &mut data.identity {
            identity.u32(word.raw());
        }
        data.words.push(word);
        data.replacement_len = replacement_len;
        Ok(())
    }

    pub fn seal(&mut self) -> Result<(), DefinitionBuildError> {
        let data = self.data_mut();
        if data.phase != DefinitionBuildPhase::OpenReplacement {
            return Err(DefinitionBuildError::InvalidPhase);
        }
        if let Some(mut identity) = data.identity.take() {
            identity.tag(REPLACEMENT_END);
            identity.u32(data.replacement_len);
            data.sealed_identity = identity.finish().max(1);
        }
        data.phase = DefinitionBuildPhase::Sealed;
        Ok(())
    }

    #[must_use]
    pub fn phase(&self) -> DefinitionBuildPhase {
        self.data().phase
    }

    #[must_use]
    pub fn policy(&self) -> DefinitionIdentityPolicy {
        self.data().policy
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

/// Compact coordinate of one immutable macro definition.
pub struct DefinitionId<G> {
    region: DefinitionRegionCoordinate,
    row: NonZeroU32,
    identity: u64,
    _brand: PhantomData<fn(&G) -> &G>,
}

/// Non-owning four-byte region coordinate reserved in full for region
/// addressing. No bit is borrowed for a carrier tag, so a later compact
/// definition key can pair this coordinate with its four-byte row unchanged.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
#[repr(transparent)]
struct DefinitionRegionCoordinate(NonZeroU32);

impl DefinitionRegionCoordinate {
    fn new(raw: u32) -> Self {
        Self(NonZeroU32::new(raw).expect("definition region is nonzero"))
    }

    const fn get(self) -> u32 {
        self.0.get()
    }
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

impl<G> Clone for DefinitionId<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for DefinitionId<G> {}

impl<G> PartialEq for DefinitionId<G> {
    fn eq(&self, other: &Self) -> bool {
        self.region == other.region && self.row == other.row
    }
}

impl<G> Eq for DefinitionId<G> {}

impl<G> core::hash::Hash for DefinitionId<G> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.region.hash(state);
        self.row.hash(state);
    }
}

impl<G> core::fmt::Debug for DefinitionId<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("DefinitionId(..)")
    }
}

impl<G> DefinitionId<G> {
    pub(crate) const fn format_index(self) -> u32 {
        self.row.get() - 1
    }

    pub(crate) const fn semantic_identity(self) -> Option<u64> {
        if self.identity == 0 {
            None
        } else {
            Some(self.identity)
        }
    }

    #[must_use]
    pub(crate) const fn region(self) -> u32 {
        self.region.get()
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
    semantic_identity: u64,
}

const FORMAT_REGION: u32 = 1;
const GLOBAL_REGION: u32 = 2;
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
    words: Vec<TokenWord>,
    parent: u32,
    promotions: Vec<DefinitionPromotion>,
    changed_epoch: u64,
    changed_mutation: u32,
}

impl DefinitionRegion {
    fn new(parent: u32) -> Self {
        Self {
            headers: Vec::new(),
            words: Vec::new(),
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
        let word_end = if cursor == 0 {
            0
        } else {
            self.headers[cursor as usize - 1].end as usize
        };
        for header in &self.headers[cursor as usize..] {
            accounting.release_shared_dynamic(definition_memory_words(
                (header.end - header.start) as usize,
            ));
        }
        self.headers.truncate(cursor as usize);
        self.words.truncate(word_end);
        self.promotions
            .retain(|promotion| promotion.source_row < cursor);
    }
}

#[derive(Clone, Copy)]
struct DefinitionPromotion {
    source_row: u32,
    destination_row: NonZeroU32,
    destination_identity: u64,
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

pub struct DefinitionRegionLease<G> {
    region: Option<LocalDefinitionRegionLease>,
    _brand: PhantomData<fn(&G) -> &G>,
}

struct LocalDefinitionRegionLease {
    slots: Rc<LocalDefinitionSlots>,
    key: u32,
}

pub(crate) struct DefinitionCheckpointLease<G> {
    region: DefinitionRegionLease<G>,
}

impl<G> Clone for DefinitionCheckpointLease<G> {
    fn clone(&self) -> Self {
        Self {
            region: self.region.clone(),
        }
    }
}

impl<G> Clone for DefinitionRegionLease<G> {
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

impl<G> Drop for DefinitionRegionLease<G> {
    fn drop(&mut self) {
        if let Some(region) = self.region.take() {
            region.slots.release(region.key);
        }
    }
}

impl<G> core::fmt::Debug for DefinitionRegionLease<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("DefinitionRegionLease(..)")
    }
}

impl<G> PartialEq for DefinitionRegionLease<G> {
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

impl<G> Eq for DefinitionRegionLease<G> {}

impl<G> core::hash::Hash for DefinitionRegionLease<G> {
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
    identity: Option<StateHasher>,
    origin: crate::token::OriginId,
    phase: DefinitionBuildPhase,
}

struct DefinitionRegionSuffix {
    headers: Vec<DefinitionHeader>,
    words: Vec<TokenWord>,
    promotions: Vec<DefinitionPromotion>,
}

#[derive(Clone, Copy)]
struct DefinitionRegionMark {
    headers: u32,
    words: u32,
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
    _head_lease: DefinitionRegionLease<G>,
    next_build_serial: u32,
    _brand: PhantomData<fn(&G) -> &G>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DefinitionAllocationError {
    CapacityOverflow,
    AllocationFailed,
    InvalidDefinition,
    IdentityPolicyMismatch,
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
    semantic_identity_enabled: bool,
    #[cfg(any(test, feature = "profiling", feature = "testing"))]
    retirement_counters: Rc<DefinitionRetirementCounterCells>,
    _brand: PhantomData<fn(&G) -> &G>,
}

impl<G> DefinitionArena<G> {
    fn split_region_suffix(
        region: &mut DefinitionRegion,
        mark: DefinitionRegionMark,
    ) -> DefinitionRegionSuffix {
        assert!(mark.headers as usize <= region.headers.len());
        assert!(mark.words as usize <= region.words.len());
        assert!(mark.promotions as usize <= region.promotions.len());
        DefinitionRegionSuffix {
            headers: region.headers.split_off(mark.headers as usize),
            words: region.words.split_off(mark.words as usize),
            promotions: region.promotions.split_off(mark.promotions as usize),
        }
    }

    fn append_region_suffix(region: &mut DefinitionRegion, mut suffix: DefinitionRegionSuffix) {
        region.headers.append(&mut suffix.headers);
        region.words.append(&mut suffix.words);
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
                        words: self.format.words.len() as u32,
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
                        words: self.global.words.len() as u32,
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
                        words: region.data.words.len() as u32,
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

    fn lease_active_region(&self, key: u32) -> DefinitionRegionLease<G> {
        if key == 0 {
            return DefinitionRegionLease {
                region: None,
                _brand: PhantomData,
            };
        }
        assert!(self.local_slots.acquire(key));
        DefinitionRegionLease {
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
        let head_lease = self.lease_active_region(head.active_local);
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
        let current_lease = self.lease_active_region(self.active_local);
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
            semantic_identity_enabled: false,
            #[cfg(any(test, feature = "profiling", feature = "testing"))]
            retirement_counters,
            _brand: PhantomData,
        }
    }

    pub(crate) fn allocate(
        &mut self,
        parameter_text: &[TokenWord],
        replacement_text: &[TokenWord],
    ) -> Result<DefinitionId<G>, DefinitionAllocationError> {
        self.allocate_from_iter(
            parameter_text.iter().copied(),
            replacement_text.iter().copied(),
        )
    }

    pub(crate) fn allocate_from_iter<Parameters, Replacement>(
        &mut self,
        parameter_text: Parameters,
        replacement_text: Replacement,
    ) -> Result<DefinitionId<G>, DefinitionAllocationError>
    where
        Parameters: ExactSizeIterator<Item = TokenWord>,
        Replacement: ExactSizeIterator<Item = TokenWord>,
    {
        let mut builder = DefinitionBuilder::new(self.identity_policy());
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
    ) -> Result<DefinitionId<G>, DefinitionAllocationError> {
        self.validate_builder(builder)?;
        self.reserve_batch(1, builder.words().len())?;
        Ok(self.publish_prevalidated(builder))
    }

    pub(crate) fn publish_prevalidated(
        &mut self,
        builder: &mut DefinitionBuilder,
    ) -> DefinitionId<G> {
        self.publish_prevalidated_to(builder, GLOBAL_REGION)
    }

    pub(crate) fn publish_format_prevalidated(
        &mut self,
        builder: &mut DefinitionBuilder,
    ) -> DefinitionId<G> {
        self.publish_prevalidated_to(builder, FORMAT_REGION)
    }

    fn publish_prevalidated_to(
        &mut self,
        builder: &mut DefinitionBuilder,
        region_id: u32,
    ) -> DefinitionId<G> {
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
        let start = u32::try_from(region.words.len()).expect("reserved word extent");
        region.words.extend_from_slice(builder.words());
        let end = u32::try_from(region.words.len()).expect("reserved word extent");
        region.headers.push(DefinitionHeader {
            start,
            parameter_len: builder.data.parameter_len,
            end,
            origin: crate::token::OriginId::UNKNOWN,
            semantic_identity: builder.data.sealed_identity,
        });
        drop(region);
        accounting.allocate_shared_dynamic(definition_memory_words(builder.words().len()));
        builder.data.phase = DefinitionBuildPhase::Published;
        DefinitionId {
            region: DefinitionRegionCoordinate::new(region_id),
            row,
            identity: builder.data.sealed_identity,
            _brand: PhantomData,
        }
    }

    pub(crate) fn validate_builder(
        &self,
        builder: &DefinitionBuilder,
    ) -> Result<(), DefinitionAllocationError> {
        builder.validate_completed()?;
        if builder.policy() != self.identity_policy() {
            return Err(DefinitionAllocationError::IdentityPolicyMismatch);
        }
        u32::try_from(builder.words().len())
            .map_err(|_| DefinitionAllocationError::CapacityOverflow)?;
        Ok(())
    }

    #[must_use]
    pub(crate) const fn identity_policy(&self) -> DefinitionIdentityPolicy {
        if self.semantic_identity_enabled {
            DefinitionIdentityPolicy::Enabled
        } else {
            DefinitionIdentityPolicy::Disabled
        }
    }

    pub(crate) fn enable_semantic_identity(&mut self) -> bool {
        if self.semantic_identity_enabled {
            return true;
        }
        if !self.format.headers.is_empty()
            || !self.global.headers.is_empty()
            || !self.local_slots.is_empty()
        {
            return false;
        }
        self.semantic_identity_enabled = true;
        true
    }

    pub(crate) fn reserve_batch(
        &mut self,
        rows: usize,
        words: usize,
    ) -> Result<(), DefinitionAllocationError> {
        self.global
            .headers
            .len()
            .checked_add(rows)
            .and_then(|len| u32::try_from(len).ok())
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        self.global
            .headers
            .try_reserve(rows)
            .map_err(|_| DefinitionAllocationError::AllocationFailed)?;
        self.global
            .words
            .try_reserve(words)
            .map_err(|_| DefinitionAllocationError::AllocationFailed)?;
        Ok(())
    }

    pub(crate) fn reserve_format_batch(
        &mut self,
        rows: usize,
        words: usize,
    ) -> Result<(), DefinitionAllocationError> {
        self.format
            .headers
            .len()
            .checked_add(rows)
            .and_then(|len| u32::try_from(len).ok())
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        self.format
            .headers
            .try_reserve(rows)
            .map_err(|_| DefinitionAllocationError::AllocationFailed)?;
        self.format
            .words
            .try_reserve(words)
            .map_err(|_| DefinitionAllocationError::AllocationFailed)?;
        Ok(())
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

    fn region_word_len(&self, id: u32) -> Option<usize> {
        self.region(id).map(|region| region.words.len())
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

    pub(crate) fn lease(&self, id: DefinitionId<G>) -> DefinitionRegionLease<G> {
        let region = (id.region() >= 3).then(|| {
            assert!(
                self.local_slots.acquire(id.region()),
                "definition lease names a live region"
            );
            LocalDefinitionRegionLease {
                slots: Rc::clone(&self.local_slots),
                key: id.region(),
            }
        });
        DefinitionRegionLease {
            region,
            _brand: PhantomData,
        }
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
            region: self.lease_active_region(self.active_local),
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
        let word_start = u32::try_from(
            self.region_word_len(region)
                .expect("selected definition region exists"),
        )
        .map_err(|_| DefinitionAllocationError::CapacityOverflow)?;
        let serial = NonZeroU32::new(self.next_build_serial)
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        self.next_build_serial = self
            .next_build_serial
            .checked_add(1)
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        let identity = self.semantic_identity_enabled.then(|| {
            let mut hasher = StateHasher::new(DEFINITION_IDENTITY_V2_DOMAIN);
            hasher.tag(PARAMETER_START);
            hasher
        });
        self.active_build = Some(ActiveDefinitionBuild {
            serial,
            region,
            word_start,
            parameter_len: 0,
            replacement_len: 0,
            pattern: MacroParameterPatternBuilder::new(),
            identity,
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
            .words
            .try_reserve(1)
            .map_err(|_| DefinitionBuildError::AllocationFailed)?;
        let build = self.active_build_mut(key)?;
        build.parameter_len = parameter_len;
        build.pattern = pattern;
        if let Some(identity) = &mut build.identity {
            identity.u32(word.raw());
        }
        self.region_mut(region_id)
            .expect("active build region exists")
            .words
            .push(word);
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
        if let Some(identity) = &mut build.identity {
            identity.tag(PARAMETER_END);
            identity.u32(build.parameter_len);
            identity.tag(REPLACEMENT_START);
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
            .words
            .try_reserve(1)
            .map_err(|_| DefinitionBuildError::AllocationFailed)?;
        let build = self.active_build_mut(key)?;
        build.replacement_len = replacement_len;
        if let Some(identity) = &mut build.identity {
            identity.u32(word.raw());
        }
        self.region_mut(region_id)
            .expect("active build region exists")
            .words
            .push(word);
        Ok(())
    }

    pub(crate) fn seal_build(
        &mut self,
        key: DefinitionBuildKey<G>,
    ) -> Result<DefinitionId<G>, DefinitionBuildError> {
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
        let end = u32::try_from(
            self.region(region_id)
                .expect("active build region exists")
                .words
                .len(),
        )
        .map_err(|_| DefinitionBuildError::CapacityOverflow)?;
        self.region_mut(region_id)
            .expect("active build region exists")
            .headers
            .try_reserve(1)
            .map_err(|_| DefinitionBuildError::AllocationFailed)?;
        let mut build = self.active_build.take().expect("validated active build");
        let semantic_identity = if let Some(mut identity) = build.identity.take() {
            identity.tag(REPLACEMENT_END);
            identity.u32(build.replacement_len);
            identity.finish().max(1)
        } else {
            0
        };
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
        region.headers.push(DefinitionHeader {
            start,
            parameter_len,
            end,
            origin,
            semantic_identity,
        });
        drop(region);
        if region_id != FORMAT_REGION && region_id != GLOBAL_REGION {
            self.local_slots.add_rows(1);
        }
        accounting.allocate_shared_dynamic(definition_memory_words((end - start) as usize));
        Ok(DefinitionId {
            region: DefinitionRegionCoordinate::new(region_id),
            row,
            identity: semantic_identity,
            _brand: PhantomData,
        })
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
        if let Some(build) = self.active_build.take() {
            self.region_mut(build.region)
                .expect("active build region exists")
                .words
                .truncate(build.word_start as usize);
        }
    }

    pub(crate) fn promote_global(
        &mut self,
        id: DefinitionId<G>,
    ) -> Result<DefinitionId<G>, DefinitionAllocationError> {
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
            .find(|promotion| promotion.source_row == id.format_index())
        {
            return Ok(DefinitionId {
                region: DefinitionRegionCoordinate::new(GLOBAL_REGION),
                row: promotion.destination_row,
                identity: promotion.destination_identity,
                _brand: PhantomData,
            });
        }
        let source_header = *source_region
            .headers
            .get(id.format_index() as usize)
            .ok_or(DefinitionAllocationError::InvalidDefinition)?;
        let source_word_len = (source_header.end - source_header.start) as usize;
        self.global
            .headers
            .try_reserve(1)
            .map_err(|_| DefinitionAllocationError::AllocationFailed)?;
        self.global
            .words
            .try_reserve(source_word_len)
            .map_err(|_| DefinitionAllocationError::AllocationFailed)?;
        source_region
            .promotions
            .try_reserve(1)
            .map_err(|_| DefinitionAllocationError::AllocationFailed)?;
        let source_words =
            &source_region.words[source_header.start as usize..source_header.end as usize];
        let start = u32::try_from(self.global.words.len())
            .map_err(|_| DefinitionAllocationError::CapacityOverflow)?;
        self.global.words.extend_from_slice(source_words);
        let end = u32::try_from(self.global.words.len())
            .map_err(|_| DefinitionAllocationError::CapacityOverflow)?;
        let row = NonZeroU32::new(
            u32::try_from(self.global.headers.len() + 1)
                .map_err(|_| DefinitionAllocationError::CapacityOverflow)?,
        )
        .expect("definition row is nonzero");
        self.global.headers.push(DefinitionHeader {
            start,
            parameter_len: source_header.parameter_len,
            end,
            origin: source_header.origin,
            semantic_identity: source_header.semantic_identity,
        });
        self.accounting
            .allocate_shared_dynamic(definition_memory_words(source_words.len()));
        let promoted = DefinitionId {
            region: DefinitionRegionCoordinate::new(GLOBAL_REGION),
            row,
            identity: id.identity,
            _brand: PhantomData,
        };
        source_region.promotions.push(DefinitionPromotion {
            source_row: id.format_index(),
            destination_row: row,
            destination_identity: id.identity,
        });
        Ok(promoted)
    }

    pub(crate) fn set_origin(
        &mut self,
        id: DefinitionId<G>,
        origin: crate::token::OriginId,
    ) -> Result<(), DefinitionAllocationError> {
        let region_id = id.region();
        let row = id.format_index();
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
    pub(crate) fn get(&self, id: DefinitionId<G>) -> DefinitionView<'_, G> {
        let region = self.region(id.region()).expect("definition region is live");
        DefinitionView {
            region,
            row: id.format_index() as usize,
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

impl<'a, G> DefinitionView<'a, G> {
    fn header(&self) -> &DefinitionHeader {
        &self.region.headers[self.row]
    }

    fn words(&self) -> &[TokenWord] {
        let header = self.header();
        &self.region.words[header.start as usize..header.end as usize]
    }

    #[must_use]
    pub fn parameter_pattern(&self) -> MacroParameterPattern {
        MacroParameterPattern::from_words(self.parameter_text())
            .expect("published definitions have a validated parameter program")
    }

    #[must_use]
    pub fn parameter_text(&self) -> &[TokenWord] {
        &self.words()[..self.header().parameter_len as usize]
    }

    #[must_use]
    pub fn replacement_text(&self) -> &[TokenWord] {
        &self.words()[self.header().parameter_len as usize..]
    }

    #[must_use]
    pub fn replacement_word(&self, index: usize) -> Option<TokenWord> {
        self.replacement_text().get(index).copied()
    }

    #[cfg(test)]
    pub(crate) fn semantic_identity(&self) -> Option<u64> {
        (self.header().semantic_identity != 0).then_some(self.header().semantic_identity)
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
