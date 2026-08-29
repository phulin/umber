//! Immutable, generation-branded macro-definition storage.

use core::marker::PhantomData;
use core::num::NonZeroU32;
#[cfg(any(test, feature = "profiling", feature = "testing"))]
use std::cell::Cell;
use thin_dst::ThinRc;

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

/// Attempt-owned reusable semantic definition buffer.
///
/// Layout is always `[parameter words][replacement words]`. The value is
/// mutable only before sealing, carries no generation owner, and is never a
/// checkpoint root. Publication performs one explicit traversal into the
/// immutable contiguous `ThinRc` representation.
#[derive(Debug)]
pub struct DefinitionBuilder {
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

impl DefinitionBuilder {
    #[must_use]
    pub fn new(policy: DefinitionIdentityPolicy) -> Self {
        let mut builder = Self {
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
        builder.reset(policy);
        builder
    }

    pub fn from_slices(
        policy: DefinitionIdentityPolicy,
        parameter_text: &[TokenWord],
        replacement_text: &[TokenWord],
    ) -> Result<Self, DefinitionBuildError> {
        let mut builder = Self::new(policy);
        builder
            .words
            .try_reserve(
                parameter_text
                    .len()
                    .checked_add(replacement_text.len())
                    .ok_or(DefinitionBuildError::CapacityOverflow)?,
            )
            .map_err(|_| DefinitionBuildError::AllocationFailed)?;
        for word in parameter_text.iter().copied() {
            builder.push_parameter(word)?;
        }
        builder.finish_parameters()?;
        for word in replacement_text.iter().copied() {
            builder.push_replacement(word)?;
        }
        builder.seal()?;
        Ok(builder)
    }

    /// Clears one recycled row while retaining its high-water allocation.
    pub fn reset(&mut self, policy: DefinitionIdentityPolicy) {
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

    pub fn push_parameter(&mut self, word: TokenWord) -> Result<(), DefinitionBuildError> {
        if self.phase != DefinitionBuildPhase::OpenParameters {
            return Err(DefinitionBuildError::InvalidPhase);
        }
        let mut pattern = self.pattern;
        pattern.push_parameter(word)?;
        let parameter_len = self
            .parameter_len
            .checked_add(1)
            .ok_or(DefinitionBuildError::CapacityOverflow)?;
        self.reserve_word()?;
        self.parameter_len = parameter_len;
        self.pattern = pattern;
        if let Some(identity) = &mut self.identity {
            identity.u32(word.raw());
        }
        self.words.push(word);
        Ok(())
    }

    pub fn finish_parameters(&mut self) -> Result<(), DefinitionBuildError> {
        if self.phase != DefinitionBuildPhase::OpenParameters {
            return Err(DefinitionBuildError::InvalidPhase);
        }
        if let Some(identity) = &mut self.identity {
            identity.tag(PARAMETER_END);
            identity.u32(self.parameter_len);
            identity.tag(REPLACEMENT_START);
        }
        self.phase = DefinitionBuildPhase::OpenReplacement;
        Ok(())
    }

    pub fn push_replacement(&mut self, word: TokenWord) -> Result<(), DefinitionBuildError> {
        if self.phase != DefinitionBuildPhase::OpenReplacement {
            return Err(DefinitionBuildError::InvalidPhase);
        }
        self.pattern.validate_replacement(word)?;
        let replacement_len = self
            .replacement_len
            .checked_add(1)
            .ok_or(DefinitionBuildError::CapacityOverflow)?;
        self.reserve_word()?;
        if let Some(identity) = &mut self.identity {
            identity.u32(word.raw());
        }
        self.words.push(word);
        self.replacement_len = replacement_len;
        Ok(())
    }

    pub fn seal(&mut self) -> Result<(), DefinitionBuildError> {
        if self.phase != DefinitionBuildPhase::OpenReplacement {
            return Err(DefinitionBuildError::InvalidPhase);
        }
        if let Some(mut identity) = self.identity.take() {
            identity.tag(REPLACEMENT_END);
            identity.u32(self.replacement_len);
            self.sealed_identity = identity.finish().max(1);
        }
        self.phase = DefinitionBuildPhase::Sealed;
        Ok(())
    }

    #[must_use]
    pub const fn phase(&self) -> DefinitionBuildPhase {
        self.phase
    }

    #[must_use]
    pub const fn policy(&self) -> DefinitionIdentityPolicy {
        self.policy
    }

    #[must_use]
    pub fn parameter_text(&self) -> &[TokenWord] {
        &self.words[..self.parameter_len as usize]
    }

    #[must_use]
    pub fn replacement_text(&self) -> &[TokenWord] {
        &self.words[self.parameter_len as usize..]
    }

    #[must_use]
    pub fn words(&self) -> &[TokenWord] {
        &self.words
    }

    #[must_use]
    pub fn capacity(&self) -> usize {
        self.words.capacity()
    }

    fn reserve_word(&mut self) -> Result<(), DefinitionBuildError> {
        #[cfg(test)]
        if core::mem::take(&mut self.fail_next_reserve) {
            return Err(DefinitionBuildError::AllocationFailed);
        }
        self.words
            .try_reserve(1)
            .map_err(|_| DefinitionBuildError::AllocationFailed)
    }

    #[cfg(test)]
    fn force_next_reserve_failure(&mut self) {
        self.fail_next_reserve = true;
    }

    fn metadata(&self) -> Result<CompletedDefinitionMetadata, DefinitionAllocationError> {
        if self.phase != DefinitionBuildPhase::Sealed
            || self.words.len() != self.parameter_len as usize + self.replacement_len as usize
        {
            return Err(DefinitionAllocationError::InvalidDefinition);
        }
        Ok(CompletedDefinitionMetadata {
            parameter_len: self.parameter_len,
            parameters: self.pattern.finish(),
            semantic_identity: self.sealed_identity,
        })
    }
}

#[derive(Clone, Copy)]
struct CompletedDefinitionMetadata {
    parameter_len: u32,
    parameters: MacroParameterPattern,
    semantic_identity: u64,
}

struct DefinitionHeader {
    serial: NonZeroU32,
    parameter_len: u32,
    parameters: MacroParameterPattern,
    accounting: MemoryAccounting,
    memory_words: usize,
    semantic_identity: u64,
}

impl Drop for DefinitionHeader {
    fn drop(&mut self) {
        self.accounting.release_shared_dynamic(self.memory_words);
    }
}

/// Dense coordinate of one immutable macro definition.
///
/// There is deliberately no raw constructor or integer projection. Only a
/// successful `DefinitionArena::allocate` can publish an id.
pub struct DefinitionId<G> {
    allocation: ThinRc<DefinitionHeader, TokenWord>,
    _brand: PhantomData<fn(&G) -> &G>,
}

#[cfg(any(test, feature = "profiling", feature = "testing"))]
thread_local! {
    static DEFINITION_RETAIN_COUNT: Cell<u64> = const { Cell::new(0) };
}

/// Process-local proof counter for non-atomic definition-owner retains.
///
/// This is compiled only in test and profiling resolutions. It is operational
/// evidence, not generation state, and therefore never enters a checkpoint or
/// format image.
#[cfg(any(test, feature = "profiling", feature = "testing"))]
#[must_use]
pub fn definition_retain_count() -> u64 {
    DEFINITION_RETAIN_COUNT.with(Cell::get)
}

impl<G> Clone for DefinitionId<G> {
    fn clone(&self) -> Self {
        #[cfg(any(test, feature = "profiling", feature = "testing"))]
        DEFINITION_RETAIN_COUNT.with(|count| count.set(count.get().saturating_add(1)));
        Self {
            allocation: self.allocation.clone(),
            _brand: PhantomData,
        }
    }
}

impl<G> PartialEq for DefinitionId<G> {
    fn eq(&self, other: &Self) -> bool {
        self.allocation.head.serial == other.allocation.head.serial
    }
}

impl<G> Eq for DefinitionId<G> {}

impl<G> core::hash::Hash for DefinitionId<G> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.allocation.head.serial.hash(state);
    }
}

impl<G> core::fmt::Debug for DefinitionId<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("DefinitionId(..)")
    }
}

impl<G> DefinitionId<G> {
    pub(crate) fn semantic_identity(&self) -> Option<u64> {
        match self.allocation.head.semantic_identity {
            0 => None,
            identity => Some(identity),
        }
    }
    /// Borrows the packed parameter text through this definition's existing
    /// owner.
    #[must_use]
    pub fn parameter_text(&self) -> &[TokenWord] {
        &self.allocation.slice[..self.allocation.head.parameter_len as usize]
    }

    /// Borrows the packed replacement text through this definition's existing
    /// owner.
    #[must_use]
    pub fn replacement_text(&self) -> &[TokenWord] {
        &self.allocation.slice[self.allocation.head.parameter_len as usize..]
    }

    /// Borrows one packed replacement word through this already-owned
    /// definition handle.
    ///
    /// Stored-token delivery keeps the handle in its input span, so this
    /// access neither clones the non-atomic owner nor reconstructs a
    /// [`DefinitionView`].
    #[must_use]
    pub fn replacement_word(&self, index: usize) -> Option<TokenWord> {
        self.allocation.slice[self.allocation.head.parameter_len as usize..]
            .get(index)
            .copied()
    }

    pub(crate) fn format_index(&self) -> u32 {
        self.allocation.head.serial.get() - 1
    }

    pub(crate) fn capture_format(&self) -> crate::format::schema::FormatDefinition {
        crate::format::schema::FormatDefinition {
            parameter_text: self.allocation.slice[..self.allocation.head.parameter_len as usize]
                .iter()
                .map(|word| word.raw())
                .collect(),
            replacement_text: self.allocation.slice[self.allocation.head.parameter_len as usize..]
                .iter()
                .map(|word| word.raw())
                .collect(),
        }
    }

    /// Test-only count of semantic owners for exact lifetime assertions.
    #[cfg(any(test, feature = "testing"))]
    #[doc(hidden)]
    pub fn semantic_owner_count(&self) -> usize {
        let owner: std::rc::Rc<thin_dst::ThinData<DefinitionHeader, TokenWord>> =
            self.allocation.clone().into();
        std::rc::Rc::strong_count(&owner) - 1
    }
}

/// Failure to stage a complete definition row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DefinitionAllocationError {
    CapacityOverflow,
    AllocationFailed,
    InvalidDefinition,
    IdentityPolicyMismatch,
}

/// Publisher for immutable shared definitions.
///
/// Published payloads leave the publisher in their generation-branded owner;
/// this value retains only the monotonic serial used by cold format capture.
pub(crate) struct DefinitionArena<G> {
    next_serial: u32,
    accounting: MemoryAccounting,
    semantic_identity_enabled: bool,
    _brand: PhantomData<fn(&G) -> &G>,
}

impl<G> DefinitionArena<G> {
    pub(crate) const fn cursor(&self) -> u32 {
        self.next_serial
    }

    pub(crate) fn restore_cursor(&mut self, cursor: u32) {
        assert!(
            cursor <= self.next_serial,
            "definition cursor is beyond the publisher"
        );
        self.next_serial = cursor;
    }

    /// Restores the accepted publisher coordinate after a candidate has been
    /// discarded.
    ///
    /// Unlike an ordinary rewind, rejection can observe a lower candidate
    /// coordinate when candidate-local initialization abandoned unpublished
    /// rows. The immutable payloads are owned by their durable handles, so the
    /// publisher has no row storage to replay; restoring the saved accepted
    /// coordinate is the complete forward operation.
    pub(crate) fn restore_accepted_cursor(&mut self, cursor: u32) {
        self.next_serial = cursor;
    }

    pub(super) fn new(
        _token: ArenaToken<G, DefinitionNamespace>,
        accounting: MemoryAccounting,
    ) -> Self {
        Self {
            next_serial: 0,
            accounting,
            semantic_identity_enabled: false,
            _brand: PhantomData,
        }
    }

    /// Atomically publishes one complete definition.
    ///
    /// Both backing vectors reserve before either length changes. The row is
    /// appended only after both token spans have been copied into arena-owned
    /// storage, so no partially initialized definition can be resolved.
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

    /// Atomically publishes one definition from token streams.
    ///
    /// Cold callers use the same checked builder as the scanner before one
    /// explicit publication traversal constructs the contiguous owner.
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
        self.publish(&builder)
    }

    /// Publishes one completely validated attempt builder.
    pub(crate) fn publish(
        &mut self,
        builder: &DefinitionBuilder,
    ) -> Result<DefinitionId<G>, DefinitionAllocationError> {
        if builder.policy() != self.identity_policy() {
            return Err(DefinitionAllocationError::IdentityPolicyMismatch);
        }
        let metadata = builder.metadata()?;
        let final_word_len = builder.words().len();
        let serial = self
            .next_serial
            .checked_add(1)
            .and_then(NonZeroU32::new)
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        u32::try_from(final_word_len).map_err(|_| DefinitionAllocationError::CapacityOverflow)?;
        let memory_words = definition_memory_words(final_word_len);
        let allocation = ThinRc::new(
            DefinitionHeader {
                serial,
                parameter_len: metadata.parameter_len,
                parameters: metadata.parameters,
                accounting: self.accounting.clone(),
                memory_words,
                semantic_identity: metadata.semantic_identity,
            },
            builder.words().iter().copied(),
        );
        self.accounting.allocate_shared_dynamic(memory_words);
        self.next_serial = serial.get();
        Ok(DefinitionId {
            allocation,
            _brand: PhantomData,
        })
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
        if self.next_serial != 0 {
            return false;
        }
        self.semantic_identity_enabled = true;
        true
    }

    /// Reserves a complete promotion batch without changing any logical
    /// arena length. Callers validate the batch's final row and word extents
    /// before invoking this method.
    pub(crate) fn reserve_batch(
        &mut self,
        rows: usize,
        words: usize,
    ) -> Result<(), DefinitionAllocationError> {
        (self.next_serial as usize)
            .checked_add(rows)
            .and_then(|len| u32::try_from(len).ok())
            .ok_or(DefinitionAllocationError::CapacityOverflow)?;
        let _ = words;
        Ok(())
    }

    /// Moves an owner into a direct immutable view.
    #[must_use]
    #[inline(always)]
    pub(crate) fn get(&self, id: DefinitionId<G>) -> DefinitionView<G> {
        DefinitionView { id }
    }

    #[must_use]
    pub(crate) const fn len(&self) -> usize {
        self.next_serial as usize
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn is_empty(&self) -> bool {
        self.next_serial == 0
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
    word_len
        .checked_add(2)
        .and_then(|words| words.checked_add(usize::from(word_len != 0)))
        .expect("validated definition length has a canonical word count")
}

/// Owning view of one complete immutable definition.
pub struct DefinitionView<G> {
    id: DefinitionId<G>,
}

impl<G> DefinitionView<G> {
    #[must_use]
    pub fn parameter_pattern(&self) -> MacroParameterPattern {
        self.id.allocation.head.parameters
    }

    #[must_use]
    pub fn parameter_text(&self) -> &[TokenWord] {
        self.id.parameter_text()
    }

    #[must_use]
    pub fn replacement_text(&self) -> &[TokenWord] {
        self.id.replacement_text()
    }
}
