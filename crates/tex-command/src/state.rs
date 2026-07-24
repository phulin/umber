//! Future-relevant state and discardable runtime ownership.

use tex_state::token::TracedTokenWord;

use crate::conditionals::ConditionStack;
use crate::input::InputState;
use crate::macro_call::ParameterState;
use crate::processor::{AlignmentDeliveryState, ExpansionState, ScannerState};

/// Complete future-relevant state owned by the command machine.
///
/// This is the command half of an executor savepoint. It contains semantic
/// and rollback-coupled provenance state only: host capabilities, aggregate
/// engine state, call-local accumulators, and discardable accelerations are
/// deliberately absent.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct CommandState {
    input: InputState,
    parameters: ParameterState,
    scanner: ScannerState,
    conditions: ConditionStack,
    alignment: AlignmentDeliveryState,
    expansion: ExpansionState,
    transient: TransientState,
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
