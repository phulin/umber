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
    region: NonZeroU32,
    row: NonZeroU32,
    identity: u64,
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
}

impl DefinitionRegion {
    fn new(parent: u32) -> Self {
        Self {
            headers: Vec::new(),
            words: Vec::new(),
            parent,
            promotions: Vec::new(),
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
}

#[derive(Default)]
struct LocalDefinitionSlotStore {
    chunks: Vec<Box<[LocalDefinitionSlot]>>,
    free: Vec<u16>,
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
        self.free
            .try_reserve(len)
            .map_err(|_| DefinitionAllocationError::AllocationFailed)?;
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(len)
            .map_err(|_| DefinitionAllocationError::AllocationFailed)?;
        slots.resize_with(len, LocalDefinitionSlot::default);
        self.chunks.push(slots.into_boxed_slice());
        for address in (base + 1..base + len).rev() {
            self.free.push(address as u16);
        }
        Ok(base)
    }

    fn allocate(&mut self, parent: u32) -> Result<u32, DefinitionAllocationError> {
        let address = self
            .free
            .pop()
            .map_or_else(|| self.allocate_chunk(), |address| Ok(address as usize))?;
        let slot = self
            .slot_mut(address)
            .expect("new or reusable definition slot exists");
        let incarnation = slot
            .incarnation
            .checked_add(1)
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        slot.incarnation = incarnation;
        slot.region = Some(LocalDefinitionRegion::new(parent));
        Ok(local_region_key(address, incarnation))
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
        let reclaim = {
            let mut store = self.store.borrow_mut();
            let region = store
                .region_mut(key)
                .expect("definition lease names its live slot incarnation");
            region.leases = region
                .leases
                .checked_sub(1)
                .expect("definition region lease count underflow");
            region.leases == 0 && region.retired
        };
        #[cfg(any(test, feature = "profiling", feature = "testing"))]
        self.counters.lease_release_region_inspections.set(
            self.counters
                .lease_release_region_inspections
                .get()
                .saturating_add(1),
        );
        if reclaim {
            self.reclaim(key);
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
            self.reclaim(key);
        }
    }

    fn reactivate(&self, key: u32) {
        let mut store = self.store.borrow_mut();
        let region = store
            .region_mut(key)
            .expect("reactivated definition region remains leased");
        region.retired = false;
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

    fn reclaim(&self, key: u32) {
        let (address, incarnation) =
            local_region_address(key).expect("local definition key is encoded");
        let mut store = self.store.borrow_mut();
        let slot = store
            .slot_mut(address)
            .expect("reclaimed definition slot exists");
        assert_eq!(slot.incarnation, incarnation, "definition slot is current");
        let Some(mut region) = slot.region.take() else {
            return;
        };
        #[cfg(any(test, feature = "profiling", feature = "testing"))]
        let rows = region.data.headers.len() as u64;
        #[cfg(any(test, feature = "profiling", feature = "testing"))]
        let promotions = region.data.promotions.len() as u64;
        self.remove_rows(region.data.headers.len());
        region.data.truncate_to(0, &self.accounting);
        if incarnation != u16::MAX {
            store.free.push(address as u16);
        }
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
    regions: smallvec::SmallVec<[DefinitionRegionLease<G>; 8]>,
}

impl<G> Clone for DefinitionCheckpointLease<G> {
    fn clone(&self) -> Self {
        Self {
            regions: self.regions.clone(),
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

pub(crate) struct AcceptedDefinitionTail<G> {
    head: DefinitionArenaCursor,
    format: DefinitionRegionSuffix,
    global: DefinitionRegionSuffix,
    active_local: Option<DefinitionRegionSuffix>,
    head_active_locals: Vec<u32>,
    retire_on_accept: Vec<u32>,
    retire_on_reject: Vec<u32>,
    active_promotions: Vec<(u32, Vec<DefinitionPromotion>)>,
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
    active_locals: Vec<u32>,
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
        cursor: u32,
        global_cursor: u32,
    ) -> DefinitionRegionSuffix {
        let word = if cursor == 0 {
            0
        } else {
            region.headers[cursor as usize - 1].end as usize
        };
        let mut promotions = Vec::new();
        let mut retained = Vec::with_capacity(region.promotions.len());
        for promotion in region.promotions.drain(..) {
            if promotion.source_row >= cursor || promotion.destination_row.get() > global_cursor {
                promotions.push(promotion);
            } else {
                retained.push(promotion);
            }
        }
        region.promotions = retained;
        DefinitionRegionSuffix {
            headers: region.headers.split_off(cursor as usize),
            words: region.words.split_off(word),
            promotions,
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

    pub(crate) fn begin_checkpoint_candidate(
        &mut self,
        cursor: DefinitionArenaCursor,
    ) -> AcceptedDefinitionTail<G> {
        assert!(self.validates_cursor(cursor));
        assert!(self.active_build.is_none());
        let head = self.cursor();
        let cursor_active_locals = self
            .local_chain(cursor.active_local)
            .expect("validated checkpoint local chain");
        let head_active_locals = core::mem::take(&mut self.active_locals);
        let common_depth = head_active_locals
            .iter()
            .zip(&cursor_active_locals)
            .take_while(|(head, cursor)| head == cursor)
            .count();
        let retire_on_accept = head_active_locals[common_depth..].to_vec();
        let retire_on_reject = cursor_active_locals[common_depth..].to_vec();
        let format =
            Self::split_region_suffix(&mut self.format, cursor.format_rows, cursor.global_rows);
        let global =
            Self::split_region_suffix(&mut self.global, cursor.global_rows, cursor.global_rows);
        let active_local = if cursor.active_local == 0 {
            None
        } else {
            let mut local = self
                .local_slots
                .region_mut(cursor.active_local)
                .expect("validated active local region");
            let suffix =
                Self::split_region_suffix(&mut local, cursor.active_local_rows, cursor.global_rows);
            self.local_slots.remove_rows(suffix.headers.len());
            Some(suffix)
        };
        let mut active_promotions = Vec::new();
        for &id in head_active_locals
            .iter()
            .chain(&cursor_active_locals[common_depth..])
        {
            if id == cursor.active_local {
                continue;
            }
            let mut data = self
                .local_slots
                .region_mut(id)
                .expect("checkpoint active definition region exists");
            let mut detached = Vec::new();
            data.promotions.retain(|promotion| {
                if promotion.destination_row.get() > cursor.global_rows {
                    detached.push(*promotion);
                    false
                } else {
                    true
                }
            });
            if !detached.is_empty() {
                active_promotions.push((id, detached));
            }
            #[cfg(any(test, feature = "profiling", feature = "testing"))]
            self.retirement_counters.checkpoint_region_inspections.set(
                self.retirement_counters
                    .checkpoint_region_inspections
                    .get()
                    .saturating_add(1),
            );
        }
        for &id in &cursor_active_locals {
            self.local_slots.reactivate(id);
        }
        self.active_locals = cursor_active_locals;
        AcceptedDefinitionTail {
            head,
            format,
            global,
            active_local,
            head_active_locals,
            retire_on_accept,
            retire_on_reject,
            active_promotions,
            next_build_serial: self.next_build_serial,
            _brand: PhantomData,
        }
    }

    pub(crate) fn reject_checkpoint_candidate(
        &mut self,
        cursor: DefinitionArenaCursor,
        tail: AcceptedDefinitionTail<G>,
    ) {
        self.restore_cursor(cursor);
        Self::append_region_suffix(&mut self.format, tail.format);
        Self::append_region_suffix(&mut self.global, tail.global);
        if let Some(suffix) = tail.active_local {
            self.local_slots.add_rows(suffix.headers.len());
            let mut local = self
                .local_slots
                .region_mut(cursor.active_local)
                .expect("checkpoint active local region");
            Self::append_region_suffix(&mut local, suffix);
        }
        for (id, mut promotions) in tail.active_promotions {
            self.local_slots
                .region_mut(id)
                .expect("checkpoint promotion region exists")
                .promotions
                .append(&mut promotions);
        }
        for id in tail.retire_on_reject {
            self.local_slots.retire(id);
        }
        for &id in &tail.head_active_locals {
            self.local_slots.reactivate(id);
        }
        self.active_locals = tail.head_active_locals;
        self.next_build_serial = tail.next_build_serial;
        debug_assert_eq!(self.cursor(), tail.head);
    }

    pub(crate) fn accept_checkpoint_candidate(&self, tail: AcceptedDefinitionTail<G>) {
        self.release_region_suffix(&tail.format);
        self.release_region_suffix(&tail.global);
        if let Some(suffix) = &tail.active_local {
            self.release_region_suffix(suffix);
        }
        for region in tail.retire_on_accept {
            #[cfg(any(test, feature = "profiling", feature = "testing"))]
            self.retirement_counters.checkpoint_region_inspections.set(
                self.retirement_counters
                    .checkpoint_region_inspections
                    .get()
                    .saturating_add(1),
            );
            self.local_slots.retire(region);
        }
    }

    fn local_chain(&self, active_local: u32) -> Option<Vec<u32>> {
        let mut chain = Vec::new();
        let mut region = active_local;
        while region != 0 {
            chain.push(region);
            region = self.local_slots.parent(region)?;
        }
        chain.reverse();
        Some(chain)
    }

    pub(crate) fn cursor(&self) -> DefinitionArenaCursor {
        let active_local = self.active_locals.last().copied().unwrap_or(0);
        let active_local_rows = self
            .local_slots
            .region(active_local)
            .map_or(0, |region| region.headers.len() as u32);
        DefinitionArenaCursor {
            format_rows: self.format.headers.len() as u32,
            global_rows: self.global.headers.len() as u32,
            active_local,
            active_local_rows,
        }
    }

    pub(crate) fn validates_cursor(&self, cursor: DefinitionArenaCursor) -> bool {
        cursor.format_rows as usize <= self.format.headers.len()
            && cursor.global_rows as usize <= self.global.headers.len()
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
        self.format
            .truncate_to(cursor.format_rows, &self.accounting);
        self.global
            .truncate_to(cursor.global_rows, &self.accounting);
        if cursor.active_local != 0 {
            let mut local = self
                .local_slots
                .region_mut(cursor.active_local)
                .expect("active local region payload");
            let released = local.headers.len() - cursor.active_local_rows as usize;
            local.truncate_to(cursor.active_local_rows, &self.accounting);
            self.local_slots.remove_rows(released);
        }
        let cursor_active_locals = self
            .local_chain(cursor.active_local)
            .expect("validated cursor local chain");
        let current_active_locals = core::mem::take(&mut self.active_locals);
        let common_depth = current_active_locals
            .iter()
            .zip(&cursor_active_locals)
            .take_while(|(current, cursor)| current == cursor)
            .count();
        for &id in &cursor_active_locals {
            self.local_slots
                .region_mut(id)
                .expect("checkpoint active region payload")
                .promotions
                .retain(|promotion| promotion.destination_row.get() <= cursor.global_rows);
            #[cfg(any(test, feature = "profiling", feature = "testing"))]
            self.retirement_counters.checkpoint_region_inspections.set(
                self.retirement_counters
                    .checkpoint_region_inspections
                    .get()
                    .saturating_add(1),
            );
        }
        for &region in &current_active_locals[common_depth..] {
            #[cfg(any(test, feature = "profiling", feature = "testing"))]
            self.retirement_counters.checkpoint_region_inspections.set(
                self.retirement_counters
                    .checkpoint_region_inspections
                    .get()
                    .saturating_add(1),
            );
            self.local_slots.retire(region);
        }
        for &region in &cursor_active_locals[common_depth..] {
            self.local_slots.reactivate(region);
        }
        self.active_locals = cursor_active_locals;
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
            active_locals: Vec::new(),
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
            region: NonZeroU32::new(region_id).expect("fixed region is nonzero"),
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
        self.active_locals
            .try_reserve(1)
            .map_err(|_| DefinitionAllocationError::AllocationFailed)?;
        let parent = self.active_locals.last().copied().unwrap_or(0);
        #[cfg(any(test, feature = "profiling", feature = "testing"))]
        self.retirement_counters.group_entry_slot_inspections.set(
            self.retirement_counters
                .group_entry_slot_inspections
                .get()
                .saturating_add(1),
        );
        let region = self.local_slots.allocate(parent)?;
        self.active_locals.push(region);
        Ok(())
    }

    pub(crate) fn end_group(&mut self) {
        assert!(
            self.active_build.is_none(),
            "definition scan crosses group exit"
        );
        let child = self
            .active_locals
            .pop()
            .expect("definition group stack matches TeX groups");
        #[cfg(any(test, feature = "profiling", feature = "testing"))]
        self.retirement_counters.group_region_inspections.set(
            self.retirement_counters
                .group_region_inspections
                .get()
                .saturating_add(1),
        );
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
        let mut regions = smallvec::SmallVec::new();
        for &id in &self.active_locals {
            #[cfg(any(test, feature = "profiling", feature = "testing"))]
            self.retirement_counters.checkpoint_region_inspections.set(
                self.retirement_counters
                    .checkpoint_region_inspections
                    .get()
                    .saturating_add(1),
            );
            assert!(
                self.local_slots.acquire(id),
                "active checkpoint definition region exists"
            );
            regions.push(DefinitionRegionLease {
                region: Some(LocalDefinitionRegionLease {
                    slots: Rc::clone(&self.local_slots),
                    key: id,
                }),
                _brand: PhantomData,
            });
        }
        DefinitionCheckpointLease { regions }
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
            DefinitionDestination::Local => self
                .active_locals
                .last()
                .copied()
                .ok_or(DefinitionAllocationError::InvalidDefinition)?,
        };
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
            region: NonZeroU32::new(region_id).expect("definition region is nonzero"),
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
                region: NonZeroU32::new(GLOBAL_REGION).expect("global region is nonzero"),
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
            region: NonZeroU32::new(GLOBAL_REGION).expect("global region is nonzero"),
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
        let mut region = self
            .region_mut(id.region())
            .ok_or(DefinitionAllocationError::InvalidDefinition)?;
        let header = region
            .headers
            .get_mut(id.format_index() as usize)
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
