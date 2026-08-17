//! Typed commit, barrier, and coverage-fallback vocabulary for semantic episodes.
//!
//! These values are operational evidence. They describe why the one live
//! `MainControl` transaction returned to its session owner; they are not
//! semantic state and therefore never enter formats or checkpoints.

use crate::EngineBoundary;

/// A boundary required by the externally observable engine contract.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum SemanticEpisodeBarrier {
    Resource,
    Effect,
    Observer,
    Diagnostic,
    Checkpoint,
    Format,
    Output,
    Cancellation,
    Fuel,
    StateIdentity,
}

impl SemanticEpisodeBarrier {
    const COUNT: usize = 10;

    const fn index(self) -> usize {
        self as usize
    }
}

/// Temporary reason an episode cannot absorb the following scalar work.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum EpisodeCoverageFamily {
    CharacterProfile,
    SourceTokenization,
    CommandVocabulary,
    ScannerOrExpansion,
    GroupLineage,
    RollbackLineage,
}

impl EpisodeCoverageFamily {
    const COUNT: usize = 6;

    const fn index(self) -> usize {
        self as usize
    }
}

/// Proof that canonical scalar execution may safely resume after fallback.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CoverageFallbackSafety {
    /// Admission published no mutation, allocation, effect, or observation.
    MutationFreeAdmission,
    /// The complete command/execution/Universe aggregate was restored first.
    ExactAggregateRollback,
}

/// One typed temporary fallback with its mandatory safety proof.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct EpisodeCoverageFallback {
    family: EpisodeCoverageFamily,
    safety: CoverageFallbackSafety,
}

impl EpisodeCoverageFallback {
    #[must_use]
    pub const fn mutation_free(family: EpisodeCoverageFamily) -> Self {
        Self {
            family,
            safety: CoverageFallbackSafety::MutationFreeAdmission,
        }
    }

    #[must_use]
    pub const fn rolled_back(family: EpisodeCoverageFamily) -> Self {
        Self {
            family,
            safety: CoverageFallbackSafety::ExactAggregateRollback,
        }
    }

    #[must_use]
    pub const fn family(self) -> EpisodeCoverageFamily {
        self.family
    }

    #[must_use]
    pub const fn safety(self) -> CoverageFallbackSafety {
        self.safety
    }
}

/// Why a successfully frozen episode returned to its owner.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EpisodeCommitBoundary {
    /// The configured bounded slice ended without another barrier.
    SliceLimit,
    /// An externally required semantic barrier was reached.
    Semantic(SemanticEpisodeBarrier),
    /// A named incremental checkpoint was committed.
    NamedCheckpoint(EngineBoundary),
    /// The job reached its canonical terminal or fragment boundary.
    Terminal,
    /// Temporary coverage stopped batching after a committed atomic action.
    /// This is not scalar fallback and carries no fallback-safety proof.
    CoverageBoundary(EpisodeCoverageFamily),
}

/// Receipt for one committed bounded episode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EpisodeCommit {
    operations: u16,
    boundary: EpisodeCommitBoundary,
}

impl EpisodeCommit {
    #[must_use]
    pub const fn new(operations: u16, boundary: EpisodeCommitBoundary) -> Self {
        Self {
            operations,
            boundary,
        }
    }

    #[must_use]
    pub const fn operations(self) -> u16 {
        self.operations
    }

    #[must_use]
    pub const fn boundary(self) -> EpisodeCommitBoundary {
        self.boundary
    }
}

/// Monotonic operational counters for episode admission and publication.
///
/// Rollback never refunds these fixed-size, allocation-free values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EpisodeTelemetry {
    attempts: u64,
    commits: u64,
    rollbacks: u64,
    operations: u64,
    semantic_barriers: [u64; SemanticEpisodeBarrier::COUNT],
    coverage_boundaries: [u64; EpisodeCoverageFamily::COUNT],
    coverage_fallbacks: [u64; EpisodeCoverageFamily::COUNT],
    slice_limits: u64,
    terminals: u64,
    last_commit: Option<EpisodeCommit>,
}

impl Default for EpisodeTelemetry {
    fn default() -> Self {
        Self {
            attempts: 0,
            commits: 0,
            rollbacks: 0,
            operations: 0,
            semantic_barriers: [0; SemanticEpisodeBarrier::COUNT],
            coverage_boundaries: [0; EpisodeCoverageFamily::COUNT],
            coverage_fallbacks: [0; EpisodeCoverageFamily::COUNT],
            slice_limits: 0,
            terminals: 0,
            last_commit: None,
        }
    }
}

impl EpisodeTelemetry {
    pub(crate) fn record_attempt(&mut self) {
        self.attempts = self.attempts.saturating_add(1);
    }

    pub(crate) fn record_commit(&mut self, commit: EpisodeCommit) {
        self.commits = self.commits.saturating_add(1);
        self.operations = self.operations.saturating_add(u64::from(commit.operations));
        match commit.boundary {
            EpisodeCommitBoundary::SliceLimit => {
                self.slice_limits = self.slice_limits.saturating_add(1);
            }
            EpisodeCommitBoundary::Semantic(barrier) => {
                self.record_semantic_barrier(barrier);
            }
            EpisodeCommitBoundary::NamedCheckpoint(_) => {
                self.record_semantic_barrier(SemanticEpisodeBarrier::Checkpoint);
            }
            EpisodeCommitBoundary::Terminal => {
                self.terminals = self.terminals.saturating_add(1);
            }
            EpisodeCommitBoundary::CoverageBoundary(family) => {
                let index = family.index();
                self.coverage_boundaries[index] = self.coverage_boundaries[index].saturating_add(1);
            }
        }
        self.last_commit = Some(commit);
    }

    pub(crate) fn record_rollback(&mut self, barrier: SemanticEpisodeBarrier) {
        self.rollbacks = self.rollbacks.saturating_add(1);
        self.record_semantic_barrier(barrier);
    }

    pub(crate) fn record_semantic_barrier(&mut self, barrier: SemanticEpisodeBarrier) {
        let index = barrier.index();
        self.semantic_barriers[index] = self.semantic_barriers[index].saturating_add(1);
    }

    pub(crate) fn record_fallback(&mut self, fallback: EpisodeCoverageFallback) {
        let index = fallback.family.index();
        self.coverage_fallbacks[index] = self.coverage_fallbacks[index].saturating_add(1);
    }

    #[must_use]
    pub const fn attempts(self) -> u64 {
        self.attempts
    }

    #[must_use]
    pub const fn commits(self) -> u64 {
        self.commits
    }

    #[must_use]
    pub const fn rollbacks(self) -> u64 {
        self.rollbacks
    }

    #[must_use]
    pub const fn operations(self) -> u64 {
        self.operations
    }

    #[must_use]
    pub const fn semantic_barriers(self, barrier: SemanticEpisodeBarrier) -> u64 {
        self.semantic_barriers[barrier.index()]
    }

    #[must_use]
    pub const fn coverage_boundaries(self, family: EpisodeCoverageFamily) -> u64 {
        self.coverage_boundaries[family.index()]
    }

    #[must_use]
    pub const fn coverage_fallbacks(self, family: EpisodeCoverageFamily) -> u64 {
        self.coverage_fallbacks[family.index()]
    }

    #[must_use]
    pub const fn slice_limits(self) -> u64 {
        self.slice_limits
    }

    #[must_use]
    pub const fn terminals(self) -> u64 {
        self.terminals
    }

    #[must_use]
    pub const fn last_commit(self) -> Option<EpisodeCommit> {
        self.last_commit
    }
}

#[cfg(test)]
mod tests;
