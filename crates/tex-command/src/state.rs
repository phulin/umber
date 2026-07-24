//! Future-relevant state and discardable runtime ownership.

use tex_state::token::TracedTokenWord;

use crate::conditionals::ConditionStack;
use crate::input::InputState;
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
