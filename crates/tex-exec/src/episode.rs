//! Typed commit and barrier vocabulary for canonical semantic episodes.
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
    slice_limits: u64,
    terminals: u64,
    host_fact_queries: u64,
    effective_tail_traversals: u64,
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
            slice_limits: 0,
            terminals: 0,
            host_fact_queries: 0,
            effective_tail_traversals: 0,
            last_commit: None,
        }
    }
}

impl EpisodeTelemetry {
    pub(crate) fn record_attempt(&mut self) {
        self.attempts = self.attempts.saturating_add(1);
    }

    pub(crate) fn record_host_fact_query(&mut self) {
        self.host_fact_queries = self.host_fact_queries.saturating_add(1);
    }

    pub(crate) fn record_effective_tail_traversal(&mut self, traversed_effective_tail: bool) {
        self.effective_tail_traversals = self
            .effective_tail_traversals
            .saturating_add(u64::from(traversed_effective_tail));
    }

    pub(crate) fn record_commit(&mut self, commit: EpisodeCommit) {
        #[cfg(feature = "profiling")]
        tex_state::measurement::record_hot_core_episode(
            usize::from(commit.operations),
            hot_core_commit_reason(commit.boundary),
        );
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
        }
        self.last_commit = Some(commit);
    }

    #[cfg(feature = "profiling")]
    pub(crate) fn record_rollback(&mut self, barrier: SemanticEpisodeBarrier, operations: usize) {
        tex_state::measurement::record_hot_core_episode(
            operations,
            hot_core_rollback_reason(barrier),
        );
        self.rollbacks = self.rollbacks.saturating_add(1);
        self.record_semantic_barrier(barrier);
    }

    #[cfg(not(feature = "profiling"))]
    pub(crate) fn record_rollback(&mut self, barrier: SemanticEpisodeBarrier) {
        self.rollbacks = self.rollbacks.saturating_add(1);
        self.record_semantic_barrier(barrier);
    }

    pub(crate) fn record_semantic_barrier(&mut self, barrier: SemanticEpisodeBarrier) {
        let index = barrier.index();
        self.semantic_barriers[index] = self.semantic_barriers[index].saturating_add(1);
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
    pub const fn slice_limits(self) -> u64 {
        self.slice_limits
    }

    #[must_use]
    pub const fn terminals(self) -> u64 {
        self.terminals
    }

    /// Number of exact executor facts requested by command processing.
    /// Ordinary delivery and commands without host enquiries leave this zero.
    #[must_use]
    pub const fn host_fact_queries(self) -> u64 {
        self.host_fact_queries
    }

    /// Number of authoritative effective-tail traversals used to prepare host
    /// projections. Pending horizontal characters need no arena traversal.
    #[must_use]
    pub const fn effective_tail_traversals(self) -> u64 {
        self.effective_tail_traversals
    }

    #[must_use]
    pub const fn last_commit(self) -> Option<EpisodeCommit> {
        self.last_commit
    }
}

#[cfg(feature = "profiling")]
fn hot_core_commit_reason(
    boundary: EpisodeCommitBoundary,
) -> tex_state::measurement::HotCoreStopReason {
    use tex_state::measurement::HotCoreStopReason as Reason;

    match boundary {
        EpisodeCommitBoundary::SliceLimit => Reason::SliceLimit,
        EpisodeCommitBoundary::Semantic(barrier) => match barrier {
            SemanticEpisodeBarrier::Resource => Reason::SemanticResource,
            SemanticEpisodeBarrier::Effect => Reason::SemanticEffect,
            SemanticEpisodeBarrier::Observer => Reason::SemanticObserver,
            SemanticEpisodeBarrier::Diagnostic => Reason::SemanticDiagnostic,
            SemanticEpisodeBarrier::Checkpoint => Reason::SemanticCheckpoint,
            SemanticEpisodeBarrier::Format => Reason::SemanticFormat,
            SemanticEpisodeBarrier::Output => Reason::SemanticOutput,
            SemanticEpisodeBarrier::Cancellation => Reason::SemanticCancellation,
            SemanticEpisodeBarrier::Fuel => Reason::SemanticFuel,
            SemanticEpisodeBarrier::StateIdentity => Reason::SemanticStateIdentity,
        },
        EpisodeCommitBoundary::NamedCheckpoint(_) => Reason::NamedCheckpoint,
        EpisodeCommitBoundary::Terminal => Reason::Terminal,
    }
}

#[cfg(feature = "profiling")]
fn hot_core_rollback_reason(
    barrier: SemanticEpisodeBarrier,
) -> tex_state::measurement::HotCoreStopReason {
    use tex_state::measurement::HotCoreStopReason as Reason;

    match barrier {
        SemanticEpisodeBarrier::Resource => Reason::RollbackResource,
        SemanticEpisodeBarrier::Fuel => Reason::RollbackFuel,
        SemanticEpisodeBarrier::Diagnostic => Reason::RollbackDiagnostic,
        SemanticEpisodeBarrier::Effect
        | SemanticEpisodeBarrier::Observer
        | SemanticEpisodeBarrier::Checkpoint
        | SemanticEpisodeBarrier::Format
        | SemanticEpisodeBarrier::Output
        | SemanticEpisodeBarrier::Cancellation
        | SemanticEpisodeBarrier::StateIdentity => Reason::RollbackDiagnostic,
    }
}

#[cfg(test)]
mod tests;
