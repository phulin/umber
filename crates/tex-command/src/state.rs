//! Future-relevant state and discardable runtime ownership.

use tex_state::token::TracedTokenWord;

use crate::conditionals::ConditionStack;
use crate::input::InputState;
use crate::input::{
    InputLevel, PhysicalLine, RegisteredSource, SourceCharacter, SourceCursor, SourceRegistration,
    SourceRegistrationError, SourceTokenizationStep,
};
use crate::macro_call::ParameterState;
use crate::processor::{AlignmentDeliveryState, ExpansionState, ScannerState};
use crate::profile::{
    CommandProfile, CommandProfileBoundary, CommandProfileFingerprint, CommandProfileMismatch,
};

/// Complete future-relevant state owned by the command machine.
///
/// This is the command half of an executor savepoint. It contains semantic
/// and rollback-coupled provenance state only: host capabilities, aggregate
/// engine state, call-local accumulators, and discardable accelerations are
/// deliberately absent.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct CommandState {
    pub(crate) input: InputState,
    pub(crate) parameters: ParameterState,
    pub(crate) scanner: ScannerState,
    pub(crate) conditions: ConditionStack,
    pub(crate) alignment: AlignmentDeliveryState,
    pub(crate) expansion: ExpansionState,
    pub(crate) transient: TransientState,
}

impl CommandState {
    /// Creates a fresh command job with an immutable semantic profile.
    ///
    /// No API changes the profile after construction. Snapshot, summary,
    /// format, and checkpoint restoration validate their recorded profile
    /// identity against this value.
    #[must_use]
    pub fn new(profile: CommandProfile) -> Self {
        Self {
            expansion: ExpansionState {
                profile,
                ..ExpansionState::default()
            },
            ..Self::default()
        }
    }

    /// Registers complete immutable backing without consulting host policy.
    ///
    /// Registration validates Unicode before allocating an identity. It does
    /// not open an input level or perform any tokenization.
    pub fn register_source(
        &mut self,
        registration: SourceRegistration,
    ) -> Result<tex_state::SourceId, SourceRegistrationError> {
        let raw = u32::try_from(self.input.next_source_identity)
            .map_err(|_| SourceRegistrationError::SourceIdentityExhausted)?;
        let id = tex_state::SourceId::new(raw);
        let source = RegisteredSource::register(id, self.profile(), registration)?;
        self.input.next_source_identity += 1;
        self.input.registered_sources.push(source);
        Ok(id)
    }

    /// Opens an already registered source as a future input level.
    ///
    /// This operation only clones retained immutable backing. It cannot search
    /// for files, invoke a host callback, or diagnose text encoding.
    pub fn open_registered_source(
        &mut self,
        source: tex_state::SourceId,
    ) -> Result<(), UnknownRegisteredSource> {
        let registered = self
            .input
            .registered_sources
            .iter()
            .find(|registered| registered.id == source)
            .cloned()
            .ok_or(UnknownRegisteredSource(source))?;
        let identity = self.input.next_level_identity;
        self.input.next_level_identity = self.input.next_level_identity.wrapping_add(1);
        self.input.levels.push(InputLevel {
            identity,
            source: SourceCursor::new(registered),
        });
        Ok(())
    }

    /// Splits and normalizes the next physical line on the active source.
    ///
    /// LF, CR, and CRLF are retained as distinct physical metadata. TeX
    /// trailing spaces are removed and the current `endlinechar` is captured
    /// for this line without tokenizing any characters.
    pub fn load_next_source_line(&mut self, endlinechar: i32) -> Option<PhysicalLine> {
        let level = self.input.levels.last_mut()?;
        level
            .source
            .load_next_line(endlinechar)
            .map(|line| line.physical)
    }

    /// Reads one byte-domain character or decoded Unicode scalar from the
    /// active normalized line with its exact physical range.
    pub fn next_source_character(&mut self) -> Option<SourceCharacter> {
        let level = self.input.levels.last_mut()?;
        let mode = level.source.backing.mode;
        let bytes = std::sync::Arc::clone(&level.source.backing.bytes);
        level.source.line.as_mut()?.next_character(mode, &bytes)
    }

    /// Retires the active normalized line so the next physical line may load.
    pub fn finish_source_line(&mut self) {
        if let Some(level) = self.input.levels.last_mut() {
            level.source.finish_line();
        }
    }

    /// Tokenizes one exact-byte source step using the caller's live catcodes.
    ///
    /// The callback is queried independently for every classified character;
    /// it is not retained or cached across tokens. Invalid characters are
    /// returned as recoverable steps after their complete spelling is
    /// consumed.
    ///
    /// # Panics
    ///
    /// Panics when called for the separately implemented Unicode character
    /// profile.
    pub fn next_exact_source_step(
        &mut self,
        endlinechar: i32,
        catcode: impl FnMut(crate::CharacterCode) -> tex_state::token::Catcode,
    ) -> SourceTokenizationStep {
        assert_eq!(
            self.profile().character_mode(),
            crate::CharacterMode::EightBitExact,
            "exact-byte tokenization requires an exact-byte command profile"
        );
        let Some(level) = self.input.levels.last_mut() else {
            return SourceTokenizationStep::End;
        };
        level.source.next_exact_byte_step(endlinechar, catcode)
    }

    /// Returns the immutable profile selected when this job was created.
    #[must_use]
    pub const fn profile(&self) -> CommandProfile {
        self.expansion.profile
    }

    /// Returns the profile component required in portable format identity.
    #[must_use]
    pub fn format_profile_fingerprint(&self) -> CommandProfileFingerprint {
        self.profile().fingerprint()
    }

    /// Rejects a format image produced for a different command profile.
    pub fn validate_format_profile(
        &self,
        found: CommandProfileFingerprint,
    ) -> Result<(), CommandProfileMismatch> {
        self.profile()
            .validate_fingerprint(CommandProfileBoundary::Format, found)
    }

    /// Returns the profile component required in incremental checkpoint identity.
    #[must_use]
    pub fn checkpoint_profile_fingerprint(&self) -> CommandProfileFingerprint {
        self.profile().fingerprint()
    }

    /// Rejects a checkpoint produced for a different command profile.
    pub fn validate_checkpoint_profile(
        &self,
        found: CommandProfileFingerprint,
    ) -> Result<(), CommandProfileMismatch> {
        self.profile()
            .validate_fingerprint(CommandProfileBoundary::Checkpoint, found)
    }
}

/// An input level referred to a source absent from retained registration.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct UnknownRegisteredSource(tex_state::SourceId);

impl UnknownRegisteredSource {
    /// Returns the missing source identity.
    #[must_use]
    pub const fn source(self) -> tex_state::SourceId {
        self.0
    }
}

impl std::fmt::Display for UnknownRegisteredSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "source identity {} is not registered",
            self.0.raw()
        )
    }
}

impl std::error::Error for UnknownRegisteredSource {}

/// Live temporary data referenced by persistent command state.
///
/// Builder contents and rollback roots are semantic while live. Spare
/// capacity and reusable empty buffers instead belong to [`CommandRuntime`].
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct TransientState {
    pub(crate) builders: Vec<LiveTokenBuilder>,
    pub(crate) rollback_roots: Vec<u64>,
    pub(crate) next_builder_identity: u64,
    /// Nesting of the call-local expansion episode currently borrowing the
    /// command machine. This records only quiescence, never a continuation,
    /// accumulator, fuel scope, host capability, or processor borrow.
    pub(crate) active_expansion_depth: u32,
}

/// One semantic token builder named by a scanner-status variant.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct LiveTokenBuilder {
    pub(crate) identity: u64,
    pub(crate) tokens: Vec<TracedTokenWord>,
}

/// Discardable command-processing acceleration and measurements.
///
/// Replacing this value with [`CommandRuntime::default`] at any point cannot
/// change semantic events, diagnostics, effects, output, or `CommandState`.
/// It intentionally implements neither equality nor hashing, preventing it
/// from becoming part of semantic state comparisons by convenience.
#[derive(Debug, Default)]
#[allow(dead_code)] // caches are populated when command semantics are implemented
pub struct CommandRuntime {
    meaning_cache: MeaningCache,
    normalized_lines: LineNormalizationCache,
    transient_pool: TokenBufferPool,
    profiling: CommandProfiling,
}

#[derive(Debug, Default)]
#[allow(dead_code)] // ownership shell
struct MeaningCache {
    entries: Vec<MeaningCacheEntry>,
}

#[derive(Debug)]
#[allow(dead_code)] // ownership shell
struct MeaningCacheEntry {
    identity: u64,
    generation: u64,
}

#[derive(Debug, Default)]
#[allow(dead_code)] // ownership shell
struct LineNormalizationCache {
    entries: Vec<NormalizedLineCacheEntry>,
}

#[derive(Debug)]
#[allow(dead_code)] // ownership shell
struct NormalizedLineCacheEntry {
    content_identity: u64,
    normalized: Vec<u8>,
}

#[derive(Debug, Default)]
#[allow(dead_code)] // ownership shell
struct TokenBufferPool {
    buffers: Vec<Vec<TracedTokenWord>>,
}

#[derive(Debug, Default)]
#[allow(dead_code)] // ownership shell
struct CommandProfiling {
    raw_deliveries: u64,
    cache_hits: u64,
}

#[cfg(test)]
mod tests;
