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
struct DefinitionPublication {
    serial: NonZeroU32,
    accounting: MemoryAccounting,
    memory_words: usize,
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
    publication: Option<DefinitionPublication>,
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
            publication: None,
            #[cfg(test)]
            fail_next_reserve: false,
        };
        data.reset(policy);
        data
    }

    fn reset(&mut self, policy: DefinitionIdentityPolicy) {
        debug_assert!(self.publication.is_none());
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
        self.publication = None;
        #[cfg(test)]
        {
            self.fail_next_reserve = false;
        }
    }

    fn release_accounting(&self) {
        if let Some(publication) = &self.publication {
            publication
                .accounting
                .release_shared_dynamic(publication.memory_words);
        }
    }
}

impl Drop for DefinitionData {
    fn drop(&mut self) {
        self.release_accounting();
    }
}

/// Attempt-owned reusable semantic definition buffer.
///
/// Layout is always `[parameter words][replacement words]`. The builder is
/// the first owner of the allocation which later becomes the immutable
/// definition. Publication transfers that allocation into its first semantic
/// owner without allocating or copying the checked words. The now-vacant
/// builder allocates a fresh resident owner when the next definition begins.
#[derive(Debug)]
pub struct DefinitionBuilder {
    data: Option<Rc<DefinitionData>>,
}

impl DefinitionBuilder {
    #[must_use]
    pub fn new(policy: DefinitionIdentityPolicy) -> Self {
        Self {
            data: Some(Rc::new(DefinitionData::new(policy))),
        }
    }

    /// Clears one recycled row.
    ///
    /// A builder whose allocation was published is deliberately vacant, so
    /// resetting it creates a new attempt-owned allocation rather than
    /// retaining an alias to the immutable definition.
    pub fn reset(&mut self, policy: DefinitionIdentityPolicy) {
        let Some(data) = &mut self.data else {
            self.data = Some(Rc::new(DefinitionData::new(policy)));
            return;
        };
        Rc::get_mut(data)
            .expect("an unpublished builder is the sole allocation owner")
            .reset(policy);
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
        self.data
            .as_deref()
            .expect("vacant definition builder must be reset before use")
    }

    fn data_mut(&mut self) -> &mut DefinitionData {
        Rc::get_mut(
            self.data
                .as_mut()
                .expect("vacant definition builder must be reset before use"),
        )
        .expect("a mutable builder has no published semantic owner")
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
        let data = self
            .data
            .as_deref()
            .ok_or(DefinitionAllocationError::InvalidDefinition)?;
        if data.phase != DefinitionBuildPhase::Sealed
            || data.words.len() != data.parameter_len as usize + data.replacement_len as usize
        {
            return Err(DefinitionAllocationError::InvalidDefinition);
        }
        Ok(())
    }

    fn take_data(&mut self) -> Rc<DefinitionData> {
        self.data
            .take()
            .expect("prevalidated definition builder is occupied")
    }
}

/// Dense coordinate of one immutable macro definition.
///
/// There is deliberately no raw constructor or integer projection. Only a
/// successful `DefinitionArena::allocate` can publish an id.
pub struct DefinitionId<G> {
    data: Rc<DefinitionData>,
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
            data: self.data.clone(),
            _brand: PhantomData,
        }
    }
}

impl<G> PartialEq for DefinitionId<G> {
    fn eq(&self, other: &Self) -> bool {
        self.serial() == other.serial()
    }
}

impl<G> Eq for DefinitionId<G> {}

impl<G> core::hash::Hash for DefinitionId<G> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.serial().hash(state);
    }
}

impl<G> core::fmt::Debug for DefinitionId<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("DefinitionId(..)")
    }
}

impl<G> DefinitionId<G> {
    fn serial(&self) -> NonZeroU32 {
        self.data
            .publication
            .as_ref()
            .expect("definition owner has a publication identity")
            .serial
    }

    pub(crate) fn semantic_identity(&self) -> Option<u64> {
        match self.data.sealed_identity {
            0 => None,
            identity => Some(identity),
        }
    }
    /// Borrows the packed parameter text through this definition's existing
    /// owner.
    #[must_use]
    pub fn parameter_text(&self) -> &[TokenWord] {
        &self.data.words[..self.data.parameter_len as usize]
    }

    /// Borrows the packed replacement text through this definition's existing
    /// owner.
    #[must_use]
    pub fn replacement_text(&self) -> &[TokenWord] {
        &self.data.words[self.data.parameter_len as usize..]
    }

    /// Borrows one packed replacement word through this already-owned
    /// definition handle.
    ///
    /// Macro replacement delivery reaches this handle through its live
    /// activation, so the input span neither clones the non-atomic owner nor
    /// reconstructs a [`DefinitionView`].
    #[must_use]
    pub fn replacement_word(&self, index: usize) -> Option<TokenWord> {
        self.data.words[self.data.parameter_len as usize..]
            .get(index)
            .copied()
    }

    pub(crate) fn format_index(&self) -> u32 {
        self.serial().get() - 1
    }

    pub(crate) fn capture_format(&self) -> crate::format::schema::FormatDefinition {
        crate::format::schema::FormatDefinition {
            parameter_text: self.data.words[..self.data.parameter_len as usize]
                .iter()
                .map(|word| word.raw())
                .collect(),
            replacement_text: self.data.words[self.data.parameter_len as usize..]
                .iter()
                .map(|word| word.raw())
                .collect(),
        }
    }

    /// Test-only count of semantic owners for exact lifetime assertions.
    #[cfg(any(test, feature = "testing"))]
    #[doc(hidden)]
    pub fn semantic_owner_count(&self) -> usize {
        Rc::strong_count(&self.data)
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
    /// Cold callers fill one checked builder allocation before publication,
    /// so no partially initialized definition can be resolved.
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
    /// Cold callers use the same checked builder as the scanner. Publication
    /// retains that allocation without traversing or copying its words.
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

    /// Publishes one completely validated attempt builder.
    ///
    /// The builder already owns the final allocation. Publication assigns its
    /// destination serial and accounting charge through unique access, then
    /// transfers that allocation into the first semantic owner.
    pub(crate) fn publish(
        &mut self,
        builder: &mut DefinitionBuilder,
    ) -> Result<DefinitionId<G>, DefinitionAllocationError> {
        self.validate_builder(builder)?;
        self.reserve_batch(1, builder.words().len())?;
        Ok(self.publish_prevalidated(builder))
    }

    /// Moves one builder allocation after complete destination preflight.
    ///
    /// This is intentionally infallible: callers validate every builder and
    /// reserve the complete batch before the first builder becomes vacant.
    pub(crate) fn publish_prevalidated(
        &mut self,
        builder: &mut DefinitionBuilder,
    ) -> DefinitionId<G> {
        let serial = NonZeroU32::new(
            self.next_serial
                .checked_add(1)
                .expect("batch preflight reserved a definition serial"),
        )
        .expect("definition serial zero is not publishable");
        let memory_words = definition_memory_words(builder.words().len());
        let mut data = builder.take_data();
        Rc::get_mut(&mut data)
            .expect("publication receives the unique builder allocation")
            .publication = Some(DefinitionPublication {
            serial,
            accounting: self.accounting.clone(),
            memory_words,
        });
        self.accounting.allocate_shared_dynamic(memory_words);
        self.next_serial = serial.get();
        DefinitionId {
            data,
            _brand: PhantomData,
        }
    }

    /// Validates every fallible destination-specific property without
    /// changing publisher serial or memory accounting.
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
        self.id.data.pattern.finish()
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
