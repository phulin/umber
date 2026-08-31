//! Immutable, generation-branded macro-definition storage.

use core::marker::PhantomData;
use core::num::NonZeroU32;
#[cfg(any(test, feature = "profiling", feature = "testing"))]
use std::cell::Cell;
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

struct DefinitionRegion {
    headers: Vec<DefinitionHeader>,
    words: Vec<TokenWord>,
    parent: u32,
    group: u32,
    pin: Rc<()>,
    retired: bool,
}

impl DefinitionRegion {
    fn new(parent: u32, group: u32) -> Self {
        Self {
            headers: Vec::new(),
            words: Vec::new(),
            parent,
            group,
            pin: Rc::new(()),
            retired: false,
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
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DefinitionArenaCursor {
    format_rows: u32,
    global_rows: u32,
    local_regions: u32,
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
    pin: Option<Rc<()>>,
    _brand: PhantomData<fn(&G) -> &G>,
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
        Self {
            pin: self.pin.as_ref().map(Rc::clone),
            _brand: PhantomData,
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
        match (&self.pin, &other.pin) {
            (Some(left), Some(right)) => Rc::ptr_eq(left, right),
            (None, None) => true,
            (Some(_), None) | (None, Some(_)) => false,
        }
    }
}

impl<G> Eq for DefinitionRegionLease<G> {}

impl<G> core::hash::Hash for DefinitionRegionLease<G> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.pin
            .as_ref()
            .map_or(0, |pin| Rc::as_ptr(pin) as usize)
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
}

pub(crate) struct AcceptedDefinitionTail<G> {
    head: DefinitionArenaCursor,
    format: DefinitionRegionSuffix,
    global: DefinitionRegionSuffix,
    active_local: Option<DefinitionRegionSuffix>,
    locals: Vec<DefinitionRegion>,
    active_locals: Vec<u32>,
    promotions: Vec<(DefinitionId<G>, DefinitionId<G>)>,
    next_build_serial: u32,
    next_group_serial: u32,
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
    locals: Vec<DefinitionRegion>,
    active_locals: Vec<u32>,
    next_build_serial: u32,
    next_group_serial: u32,
    active_build: Option<ActiveDefinitionBuild>,
    promotions: Vec<(DefinitionId<G>, DefinitionId<G>)>,
    accounting: MemoryAccounting,
    semantic_identity_enabled: bool,
    _brand: PhantomData<fn(&G) -> &G>,
}

impl<G> DefinitionArena<G> {
    fn split_region_suffix(region: &mut DefinitionRegion, cursor: u32) -> DefinitionRegionSuffix {
        let word = if cursor == 0 {
            0
        } else {
            region.headers[cursor as usize - 1].end as usize
        };
        DefinitionRegionSuffix {
            headers: region.headers.split_off(cursor as usize),
            words: region.words.split_off(word),
        }
    }

    fn append_region_suffix(region: &mut DefinitionRegion, mut suffix: DefinitionRegionSuffix) {
        region.headers.append(&mut suffix.headers);
        region.words.append(&mut suffix.words);
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
        let format = Self::split_region_suffix(&mut self.format, cursor.format_rows);
        let global = Self::split_region_suffix(&mut self.global, cursor.global_rows);
        let active_local = if cursor.active_local == 0 {
            None
        } else {
            Some(Self::split_region_suffix(
                self.locals
                    .get_mut((cursor.active_local - 3) as usize)
                    .expect("validated active local region"),
                cursor.active_local_rows,
            ))
        };
        let locals = self.locals.split_off(cursor.local_regions as usize);
        let active_locals = core::mem::take(&mut self.active_locals);
        let promotions = self.promotions.clone();
        self.promotions.retain(|(source, destination)| {
            destination.format_index() < cursor.global_rows
                && Self::local_key_valid_at_cursor(*source, cursor)
        });
        self.rebuild_active_locals(cursor.active_local);
        AcceptedDefinitionTail {
            head,
            format,
            global,
            active_local,
            locals,
            active_locals,
            promotions,
            next_build_serial: self.next_build_serial,
            next_group_serial: self.next_group_serial,
        }
    }

    pub(crate) fn reject_checkpoint_candidate(
        &mut self,
        cursor: DefinitionArenaCursor,
        mut tail: AcceptedDefinitionTail<G>,
    ) {
        self.restore_cursor(cursor);
        Self::append_region_suffix(&mut self.format, tail.format);
        Self::append_region_suffix(&mut self.global, tail.global);
        if let Some(suffix) = tail.active_local {
            Self::append_region_suffix(
                self.locals
                    .get_mut((cursor.active_local - 3) as usize)
                    .expect("checkpoint active local region"),
                suffix,
            );
        }
        self.locals.append(&mut tail.locals);
        self.active_locals = tail.active_locals;
        self.promotions = tail.promotions;
        self.next_build_serial = tail.next_build_serial;
        self.next_group_serial = tail.next_group_serial;
        debug_assert_eq!(self.cursor(), tail.head);
    }

    pub(crate) fn accept_checkpoint_candidate(&self, tail: AcceptedDefinitionTail<G>) {
        self.release_region_suffix(&tail.format);
        self.release_region_suffix(&tail.global);
        if let Some(suffix) = &tail.active_local {
            self.release_region_suffix(suffix);
        }
        for region in &tail.locals {
            for header in &region.headers {
                self.accounting
                    .release_shared_dynamic(definition_memory_words(
                        (header.end - header.start) as usize,
                    ));
            }
        }
    }

    fn rebuild_active_locals(&mut self, active_local: u32) {
        self.active_locals.clear();
        let mut region = active_local;
        while region != 0 {
            self.active_locals.push(region);
            region = self
                .region(region)
                .expect("validated active definition region")
                .parent;
        }
        self.active_locals.reverse();
    }

    pub(crate) fn cursor(&self) -> DefinitionArenaCursor {
        let active_local = self.active_locals.last().copied().unwrap_or(0);
        let active_local_rows = self
            .region(active_local)
            .map_or(0, |region| region.headers.len() as u32);
        DefinitionArenaCursor {
            format_rows: self.format.headers.len() as u32,
            global_rows: self.global.headers.len() as u32,
            local_regions: self.locals.len() as u32,
            active_local,
            active_local_rows,
        }
    }

    pub(crate) fn validates_cursor(&self, cursor: DefinitionArenaCursor) -> bool {
        cursor.format_rows as usize <= self.format.headers.len()
            && cursor.global_rows as usize <= self.global.headers.len()
            && cursor.local_regions as usize <= self.locals.len()
            && (cursor.active_local == 0
                || (cursor.active_local >= 3
                    && cursor.active_local - 3 < cursor.local_regions
                    && self.region(cursor.active_local).is_some_and(|region| {
                        cursor.active_local_rows as usize <= region.headers.len()
                    })))
    }

    fn local_key_valid_at_cursor(id: DefinitionId<G>, cursor: DefinitionArenaCursor) -> bool {
        if id.region() < 3 || id.region() - 3 >= cursor.local_regions {
            return false;
        }
        id.region() != cursor.active_local || id.format_index() < cursor.active_local_rows
    }

    pub(crate) fn restore_cursor(&mut self, cursor: DefinitionArenaCursor) {
        assert!(self.validates_cursor(cursor));
        self.abort_active_build();
        self.format
            .truncate_to(cursor.format_rows, &self.accounting);
        self.global
            .truncate_to(cursor.global_rows, &self.accounting);
        if cursor.active_local != 0 {
            let accounting = self.accounting.clone();
            self.region_mut(cursor.active_local)
                .expect("validated active local region")
                .truncate_to(cursor.active_local_rows, &accounting);
        }
        for region in &mut self.locals[cursor.local_regions as usize..] {
            region.truncate_to(0, &self.accounting);
        }
        self.locals.truncate(cursor.local_regions as usize);
        self.rebuild_active_locals(cursor.active_local);
    }

    pub(super) fn new(
        _token: ArenaToken<G, DefinitionNamespace>,
        accounting: MemoryAccounting,
    ) -> Self {
        Self {
            format: DefinitionRegion::new(0, 0),
            global: DefinitionRegion::new(0, 0),
            locals: Vec::new(),
            active_locals: Vec::new(),
            next_build_serial: 1,
            next_group_serial: 1,
            active_build: None,
            promotions: Vec::new(),
            accounting,
            semantic_identity_enabled: false,
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
        let region = self
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
        self.accounting
            .allocate_shared_dynamic(definition_memory_words(builder.words().len()));
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
            || self.locals.iter().any(|region| !region.headers.is_empty())
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

    fn region(&self, id: u32) -> Option<&DefinitionRegion> {
        match id {
            FORMAT_REGION => Some(&self.format),
            GLOBAL_REGION => Some(&self.global),
            id if id >= 3 => self.locals.get((id - 3) as usize),
            _ => None,
        }
    }

    fn region_mut(&mut self, id: u32) -> Option<&mut DefinitionRegion> {
        match id {
            FORMAT_REGION => Some(&mut self.format),
            GLOBAL_REGION => Some(&mut self.global),
            id if id >= 3 => self.locals.get_mut((id - 3) as usize),
            _ => None,
        }
    }

    pub(crate) fn begin_group(&mut self) -> Result<(), DefinitionAllocationError> {
        self.sweep_retired_regions();
        if self.active_build.is_some() {
            return Err(DefinitionAllocationError::InvalidDefinition);
        }
        let index = u32::try_from(self.locals.len())
            .map_err(|_| DefinitionAllocationError::CapacityOverflow)?;
        self.locals
            .try_reserve(2)
            .map_err(|_| DefinitionAllocationError::AllocationFailed)?;
        let region = index
            .checked_add(3)
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        let parent = self.active_locals.last().copied().unwrap_or(0);
        let group = self.next_group_serial;
        self.next_group_serial = self
            .next_group_serial
            .checked_add(1)
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        self.locals.push(DefinitionRegion::new(parent, group));
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
        let child_group = self
            .region(child)
            .expect("active child definition region exists")
            .group;
        for region in &mut self.locals {
            if region.group == child_group {
                region.retired = true;
            }
        }
        if let Some(previous_parent) = self.active_locals.pop() {
            let previous = self
                .region(previous_parent)
                .expect("active parent definition region exists");
            let parent = previous.parent;
            let group = previous.group;
            let region = u32::try_from(self.locals.len())
                .expect("reserved continuation region count fits u32")
                .checked_add(3)
                .expect("reserved continuation region id fits u32");
            self.locals.push(DefinitionRegion::new(parent, group));
            self.active_locals.push(region);
        }
        self.sweep_retired_regions();
    }

    pub(crate) fn lease(&self, id: DefinitionId<G>) -> DefinitionRegionLease<G> {
        let pin = (id.region() >= 3).then(|| {
            Rc::clone(
                &self
                    .region(id.region())
                    .expect("definition lease names a live region")
                    .pin,
            )
        });
        DefinitionRegionLease {
            pin,
            _brand: PhantomData,
        }
    }

    pub(crate) fn checkpoint_lease(&self) -> DefinitionCheckpointLease<G> {
        let mut regions = smallvec::SmallVec::new();
        for region in &self.locals {
            if self.active_locals.iter().any(|active| {
                self.region(*active)
                    .is_some_and(|active| active.group == region.group)
            }) {
                regions.push(DefinitionRegionLease {
                    pin: Some(Rc::clone(&region.pin)),
                    _brand: PhantomData,
                });
            }
        }
        DefinitionCheckpointLease { regions }
    }

    fn sweep_retired_regions(&mut self) {
        let accounting = self.accounting.clone();
        for region in &mut self.locals {
            if region.retired && Rc::strong_count(&region.pin) == 1 && !region.headers.is_empty() {
                region.truncate_to(0, &accounting);
            }
        }
        let promotions = core::mem::take(&mut self.promotions);
        self.promotions = promotions
            .into_iter()
            .filter(|(source, destination)| {
                self.region(source.region())
                    .is_some_and(|region| source.format_index() < region.headers.len() as u32)
                    && destination.format_index() < self.global.headers.len() as u32
            })
            .collect();
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
            self.region(region)
                .expect("selected definition region exists")
                .words
                .len(),
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
        let region = self
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
        self.accounting
            .allocate_shared_dynamic(definition_memory_words((end - start) as usize));
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
        if let Some((_, promoted)) = self.promotions.iter().find(|(source, _)| *source == id) {
            return Ok(*promoted);
        }
        let source_region = self
            .locals
            .get((id.region() - 3) as usize)
            .ok_or(DefinitionAllocationError::InvalidDefinition)?;
        let source_header = *source_region
            .headers
            .get(id.format_index() as usize)
            .ok_or(DefinitionAllocationError::InvalidDefinition)?;
        let source_words =
            &source_region.words[source_header.start as usize..source_header.end as usize];
        self.global
            .headers
            .try_reserve(1)
            .map_err(|_| DefinitionAllocationError::AllocationFailed)?;
        self.global
            .words
            .try_reserve(source_words.len())
            .map_err(|_| DefinitionAllocationError::AllocationFailed)?;
        self.promotions
            .try_reserve(1)
            .map_err(|_| DefinitionAllocationError::AllocationFailed)?;
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
        self.promotions.push((id, promoted));
        Ok(promoted)
    }

    pub(crate) fn set_origin(
        &mut self,
        id: DefinitionId<G>,
        origin: crate::token::OriginId,
    ) -> Result<(), DefinitionAllocationError> {
        let region = self
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
        let header = &region.headers[id.format_index() as usize];
        DefinitionView {
            header,
            words: &region.words[header.start as usize..header.end as usize],
            _brand: PhantomData,
        }
    }

    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.format.headers.len()
            + self.global.headers.len()
            + self
                .locals
                .iter()
                .map(|region| region.headers.len())
                .sum::<usize>()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.format.headers.is_empty()
            && self.global.headers.is_empty()
            && self.locals.iter().all(|region| region.headers.is_empty())
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
    header: &'a DefinitionHeader,
    words: &'a [TokenWord],
    _brand: PhantomData<fn(&G) -> &G>,
}

impl<'a, G> DefinitionView<'a, G> {
    #[must_use]
    pub fn parameter_pattern(&self) -> MacroParameterPattern {
        MacroParameterPattern::from_words(self.parameter_text())
            .expect("published definitions have a validated parameter program")
    }

    #[must_use]
    pub fn parameter_text(&self) -> &'a [TokenWord] {
        &self.words[..self.header.parameter_len as usize]
    }

    #[must_use]
    pub fn replacement_text(&self) -> &'a [TokenWord] {
        &self.words[self.header.parameter_len as usize..]
    }

    #[must_use]
    pub fn replacement_word(&self, index: usize) -> Option<TokenWord> {
        self.replacement_text().get(index).copied()
    }

    #[cfg(test)]
    pub(crate) fn semantic_identity(&self) -> Option<u64> {
        (self.header.semantic_identity != 0).then_some(self.header.semantic_identity)
    }

    #[must_use]
    pub fn definition_origin(&self) -> crate::token::OriginId {
        self.header.origin
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
