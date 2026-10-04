//! Actual current semantic level, suspended owners and admitted checkpoints.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Explicit stack of TeX mode levels.
pub(super) struct ModeNestStorage {
    pub(super) current: ModeLevelSummary,
    pub(super) suspended: Vec<ModeLevelSummary>,
    pub(super) scratch: HorizontalModeScratch,
}

impl ModeNestStorage {
    fn iter(&self) -> impl DoubleEndedIterator<Item = &ModeLevelSummary> {
        self.suspended.iter().chain(std::iter::once(&self.current))
    }
    fn level(&self, index: usize) -> &ModeLevelSummary {
        if index == self.suspended.len() {
            &self.current
        } else {
            &self.suspended[index]
        }
    }
    fn enclosing_vertical_index(&self) -> usize {
        if matches!(self.current.mode(), Mode::Vertical | Mode::InternalVertical) {
            return self.suspended.len();
        }
        self.suspended
            .iter()
            .rposition(|level| matches!(level.mode(), Mode::Vertical | Mode::InternalVertical))
            .expect("base vertical level is always present")
    }

    fn recycle_level_pending_sources(&mut self) {
        let scratch = &mut self.scratch;
        for level in self
            .suspended
            .iter_mut()
            .chain(std::iter::once(&mut self.current))
        {
            if let Some(value) = level.list.pending_hchars.take() {
                scratch.recycle_pending_source(value.source);
            }
        }
    }
}

static NEXT_MODE_CHECKPOINT_OWNER: AtomicUsize = AtomicUsize::new(1);

/// Opaque bounded root of one mode timeline position.
pub(crate) struct ModeCheckpoint {
    owner: usize,
    pub(super) outer: ModeLevelSummary,
    reachable_state_identity_root: Option<u64>,
}

pub(crate) enum PreparedModeReplayRestore {
    Candidate { nest: usize, checkpoint: usize },
    Independent { nest: usize, checkpoint: usize },
}

impl Clone for ModeCheckpoint {
    fn clone(&self) -> Self {
        Self {
            owner: self.owner,
            outer: self.outer.clone_rootless(),
            reachable_state_identity_root: self.reachable_state_identity_root,
        }
    }
}

impl std::fmt::Debug for ModeCheckpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ModeCheckpoint")
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

impl ModeCheckpoint {
    #[cfg(feature = "profiling")]
    pub(crate) fn replay_work(&self) -> u64 {
        0
    }

    pub(crate) fn retention_owner_address(&self) -> usize {
        self.owner
    }

    pub(crate) const fn reachable_state_identity_root(&self) -> Option<u64> {
        self.reachable_state_identity_root
    }

    pub(crate) fn retained_owner_bytes(&self) -> usize {
        std::mem::size_of::<ModeLevelSummary>()
    }

    pub(crate) fn retains_page_node_handles(&self) -> bool {
        false
    }

    pub(crate) fn summary(&self) -> ModeNestSummary {
        ModeNestSummary {
            levels: vec![self.outer.clone_rootless()],
        }
    }
}

pub struct ModeNest {
    pub(super) storage: ModeNestStorage,
    lifecycle: ModeNestLifecycle,
    /// TeX82 §216's maximum pre-push `nest_ptr`. This runtime diagnostic is
    /// intentionally absent from summaries, semantic equality, and hashes.
    max_nest_stack: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ModeNestLifecycle {
    Independent,
    CheckpointCandidate,
}

impl Drop for ModeNest {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        assert_eq!(
            self.lifecycle,
            ModeNestLifecycle::Independent,
            "checkpoint candidate mode owner requires explicit accept or reject"
        );
    }
}

impl std::fmt::Debug for ModeNest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ModeNest")
            .field("suspended", &self.storage.suspended)
            .field("current", &self.storage.current)
            .finish()
    }
}

impl PartialEq for ModeNest {
    fn eq(&self, other: &Self) -> bool {
        self.storage.current == other.storage.current
            && self.storage.suspended == other.storage.suspended
    }
}

impl Default for ModeNest {
    fn default() -> Self {
        Self::new()
    }
}

impl ModeNest {
    pub(crate) const fn is_checkpoint_candidate(&self) -> bool {
        matches!(self.lifecycle, ModeNestLifecycle::CheckpointCandidate)
    }

    /// Promotes the current mode owner through the aggregate candidate
    /// barrier. Consuming the candidate makes a second disposition impossible.
    pub(crate) fn accept_checkpoint_candidate(mut self) -> Self {
        self.accept_checkpoint_candidate_in_place();
        self
    }

    /// Promotes the current mode owner without replacing its bounded stack
    /// storage. This is the production `MainControl` settlement path: the
    /// stack allocation remains owned by the accepted control instead of
    /// constructing a default nest merely to move it out and back.
    pub(crate) fn accept_checkpoint_candidate_in_place(&mut self) {
        assert_eq!(
            self.lifecycle,
            ModeNestLifecycle::CheckpointCandidate,
            "only a rooted mode candidate can be accepted"
        );
        self.lifecycle = ModeNestLifecycle::Independent;
    }

    /// Rejects the candidate-only mode suffix after destination owners have
    /// returned their roots. Consuming the candidate prevents later use.
    pub(crate) fn reject_checkpoint_candidate(mut self) {
        assert_eq!(
            self.lifecycle,
            ModeNestLifecycle::CheckpointCandidate,
            "only a rooted mode candidate can be rejected"
        );
        self.lifecycle = ModeNestLifecycle::Independent;
    }

    /// TeX82 §11's maximum number of simultaneously saved semantic levels.
    pub(super) const TEX82_NEST_SIZE: usize = 40;
    const MAX_LIVE_LEVELS: usize = Self::TEX82_NEST_SIZE + 1;

    /// Creates the outer main vertical nest level.
    #[must_use]
    pub fn new() -> Self {
        // Allocate the bounded suspended stack once, as in TeX82 §216.
        let suspended = Vec::with_capacity(Self::TEX82_NEST_SIZE);
        Self {
            storage: ModeNestStorage {
                current: ModeLevelSummary::new(Mode::Vertical),
                suspended,
                scratch: HorizontalModeScratch::default(),
            },
            lifecycle: ModeNestLifecycle::Independent,
            max_nest_stack: 0,
        }
    }

    /// Rehydrates a nest from snapshot summary state.
    pub fn from_summary(mut summary: ModeNestSummary) -> Result<Self, ExecError> {
        if summary.levels.is_empty() {
            return Err(ExecError::EmptyModeNestSummary);
        }
        if summary.levels.len() > Self::MAX_LIVE_LEVELS {
            return Err(ExecError::Fatal(tex_command::FatalError::overflow(
                "semantic nest size",
                Self::TEX82_NEST_SIZE as i32,
            )));
        }
        Ok(Self {
            storage: ModeNestStorage {
                current: summary.levels.pop().expect("nonempty summary"),
                suspended: summary.levels,
                scratch: HorizontalModeScratch::default(),
            },
            lifecycle: ModeNestLifecycle::Independent,
            max_nest_stack: 0,
        })
    }

    #[must_use]
    pub fn summary(&self) -> ModeNestSummary {
        ModeNestSummary {
            levels: self
                .storage
                .iter()
                .map(|level| {
                    #[cfg(test)]
                    {
                        level.clone_operation_projection()
                    }
                    #[cfg(not(test))]
                    {
                        level.clone_rootless()
                    }
                })
                .collect(),
        }
    }

    pub(crate) fn iter_levels(&self) -> impl DoubleEndedIterator<Item = &ModeLevelSummary> {
        self.storage.iter()
    }

    pub(crate) fn semantic_fingerprint<G>(&self, universe: &Universe<G>) -> u64 {
        semantic_fingerprint_levels(self.depth(), self.storage.iter(), universe)
    }

    pub(crate) fn checkpoint_with_identity(
        &mut self,
        wants_reachable_state_identity: bool,
    ) -> ModeCheckpoint {
        assert!(
            self.restart_checkpoint_is_quiescent(),
            "restart checkpoint requires one quiescent empty outer vertical mode"
        );
        self.storage.scratch.clear();
        let reachable_state_identity_root = wants_reachable_state_identity
            .then(|| mode_nest_semantic_identity(self.depth(), self.storage.iter()));
        let owner = NEXT_MODE_CHECKPOINT_OWNER.fetch_add(1, Ordering::Relaxed);
        assert_ne!(owner, 0, "mode checkpoint owner identity exhausted");
        ModeCheckpoint {
            owner,
            outer: self.storage.current.clone_rootless(),
            reachable_state_identity_root,
        }
    }

    /// Reports whether the complete mode owner can cross a named restart
    /// boundary. TeX82 §1096 may finish an outer paragraph while command
    /// input still owns a macro argument; if consuming that argument starts a
    /// new paragraph, the delayed command boundary must wait for outer
    /// vertical mode again instead of capturing the intervening horizontal
    /// list.
    pub(crate) fn restart_checkpoint_is_quiescent(&self) -> bool {
        self.storage.suspended.is_empty()
            && self.storage.current.mode == Mode::Vertical
            && self.storage.current.list.is_checkpoint_rootless()
    }

    /// Reports whether any live mode payload retains page-arena coordinates.
    ///
    /// Shipout uses this borrowed projection before releasing a rootless page
    /// suffix.  It deliberately stays on the live owner instead of cloning a
    /// [`ModeNestSummary`] and every retained list merely to answer the
    /// lifetime question.
    pub(crate) fn retains_page_node_handles(&self) -> bool {
        self.storage.iter().any(|level| level.list.has_node_roots())
    }

    /// Preflights the mode half of page succession without cloning a mode
    /// level or scanning arena payload. Every level is checked against the
    /// admitted current region; succession is permitted only after the exact
    /// live and rollback-restorable mode-list closures have become rootless.
    pub(crate) fn preflight_page_region_succession<G>(
        &self,
        stores: &CommandContext<'_, G>,
    ) -> Option<tex_state::page::ModeListRegionPreflight> {
        self.storage
            .iter()
            .all(|level| level.list.validate_page_region(stores))
            .then_some(())?;
        (!self.retains_page_node_handles()).then(|| stores.seal_mode_list_region_preflight())
    }

    pub(crate) fn restore_checkpoint(
        &mut self,
        checkpoint: &ModeCheckpoint,
    ) -> Result<(), ExecError> {
        assert_eq!(
            self.lifecycle,
            ModeNestLifecycle::Independent,
            "candidate settlement precedes same-owner restoration"
        );
        self.storage.recycle_level_pending_sources();
        self.storage.suspended.clear();
        self.storage.current = checkpoint.outer.clone_rootless();
        self.storage.scratch.clear();
        Ok(())
    }

    pub(crate) fn prepare_checkpoint_restore_after_replay(
        &self,
        checkpoint: &ModeCheckpoint,
    ) -> PreparedModeReplayRestore {
        let nest = std::ptr::from_ref(self) as usize;
        let checkpoint = checkpoint.owner;
        if self.is_checkpoint_candidate() {
            PreparedModeReplayRestore::Candidate { nest, checkpoint }
        } else {
            PreparedModeReplayRestore::Independent { nest, checkpoint }
        }
    }

    pub(crate) fn apply_prepared_checkpoint_restore_after_replay(
        &mut self,
        checkpoint: &ModeCheckpoint,
        prepared: PreparedModeReplayRestore,
    ) -> Result<(), ExecError> {
        match prepared {
            PreparedModeReplayRestore::Candidate {
                nest,
                checkpoint: owner,
            } if nest == std::ptr::from_ref(self) as usize
                && owner == checkpoint.owner
                && self.is_checkpoint_candidate() =>
            {
                self.storage.recycle_level_pending_sources();
                self.storage.suspended.clear();
                self.storage.current = checkpoint.outer.clone_rootless();
                self.storage.scratch.clear();
                Ok(())
            }
            PreparedModeReplayRestore::Independent {
                nest,
                checkpoint: owner,
            } if nest == std::ptr::from_ref(self) as usize
                && owner == checkpoint.owner
                && !self.is_checkpoint_candidate() =>
            {
                self.restore_checkpoint(checkpoint)
            }
            _ => Err(ExecError::Halted),
        }
    }

    pub(crate) fn fork_checkpoint(checkpoint: &ModeCheckpoint) -> Result<Self, ExecError> {
        let suspended = Vec::with_capacity(Self::TEX82_NEST_SIZE);
        Ok(Self {
            storage: ModeNestStorage {
                current: checkpoint.outer.clone_rootless(),
                suspended,
                scratch: HorizontalModeScratch::default(),
            },
            lifecycle: ModeNestLifecycle::CheckpointCandidate,
            max_nest_stack: 0,
        })
    }

    #[must_use]
    pub fn depth(&self) -> usize {
        self.storage.suspended.len() + 1
    }

    /// TeX82 §216's maximum `nest_ptr` observed before a semantic push.
    #[must_use]
    pub const fn maximum_saved_depth(&self) -> usize {
        self.max_nest_stack
    }

    /// Retains a job's operational high-water mark across restoration of an
    /// earlier semantic mode summary. The summary itself remains free of
    /// diagnostic accounting, so fresh format/session materialization still
    /// starts from zero.
    pub(crate) fn retain_maximum_saved_depth(&mut self, maximum: usize) {
        self.max_nest_stack = self.max_nest_stack.max(maximum);
    }

    #[must_use]
    pub fn current_mode(&self) -> Mode {
        self.storage.current.mode()
    }

    /// Saves the current level and enters a new empty semantic level.
    ///
    /// TeX82 §216 checks `nest_size` before copying `cur_list` into the
    /// semantic stack. Accordingly, a rejected push leaves every live level
    /// unchanged.
    pub fn push(&mut self, mode: Mode) -> Result<(), ExecError> {
        self.push_at_line(mode, 0)
    }

    /// Enters a semantic level while retaining TeX's `mode_line` diagnostic
    /// context. A negative line identifies the output-routine level.
    pub(crate) fn push_at_line(&mut self, mode: Mode, entry_line: i32) -> Result<(), ExecError> {
        let depth = self.depth();
        if depth > Self::TEX82_NEST_SIZE {
            return Err(ExecError::Fatal(tex_command::FatalError::overflow(
                "semantic nest size",
                Self::TEX82_NEST_SIZE as i32,
            )));
        }
        self.max_nest_stack = self.max_nest_stack.max(depth.saturating_sub(1));
        let mut level = ModeLevelSummary::new(mode);
        level.set_entry_line(entry_line);
        if matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal) {
            level.mutate_list(|list| list.set_space_factor(1000));
        }
        let previous = std::mem::replace(&mut self.storage.current, level);
        self.storage.suspended.push(previous);
        Ok(())
    }

    pub fn pop(&mut self) -> Result<ModeLevelSummary, ExecError> {
        if self.storage.suspended.is_empty() {
            return Err(ExecError::CannotPopBaseMode);
        }
        if self.current_list().pending_hchars().is_some() {
            return Err(ExecError::UncommittedPendingHchars);
        }
        let parent = self.storage.suspended.pop().expect("base pop rejected");
        let popped = std::mem::replace(&mut self.storage.current, parent);
        self.storage.scratch.clear();
        Ok(popped)
    }

    #[inline]
    pub fn current_list(&self) -> &ModeList {
        self.storage.current.list()
    }

    pub(crate) fn horizontal_mode_scratch_mut(&mut self) -> &mut HorizontalModeScratch {
        &mut self.storage.scratch
    }

    pub(crate) fn reshape_open_type_runs_list<G>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
        source: tex_state::page_node_arena::PageListId,
    ) -> tex_state::page_node_arena::PageListId {
        self.storage
            .scratch
            .reshape_open_type_runs_list(stores, source)
    }

    /// Appends one owned node to the current mode list.
    pub fn push_current_node<G>(&mut self, stores: &mut CommandContext<'_, G>, node: Node) {
        self.current_list_mutation().push(stores, node);
    }

    pub(crate) fn current_list_mutation(&mut self) -> ModeListMutation<'_> {
        let storage = &mut self.storage;
        let list = &mut storage.current.list;
        ModeListMutation {
            list,
            scratch: Some(&mut storage.scratch),
        }
    }

    pub(crate) fn list_mutation(&mut self, index: usize) -> Option<ModeListMutation<'_>> {
        let storage = &mut self.storage;
        let level = if index == storage.suspended.len() {
            &mut storage.current
        } else {
            storage.suspended.get_mut(index)?
        };
        Some(ModeListMutation {
            list: &mut level.list,
            scratch: Some(&mut storage.scratch),
        })
    }

    #[must_use]
    pub fn enclosing_vertical_prev_graf(&self) -> i32 {
        let storage = &self.storage;
        let index = storage.enclosing_vertical_index();
        storage.level(index).list().prev_graf()
    }

    #[must_use]
    pub fn enclosing_vertical_prev_depth(&self) -> Option<Scaled> {
        let storage = &self.storage;
        let index = storage.enclosing_vertical_index();
        storage.level(index).list().prev_depth()
    }

    pub fn set_enclosing_vertical_prev_graf(&mut self, lines: i32) {
        let index = self.storage.enclosing_vertical_index();
        self.list_mutation(index)
            .expect("enclosing vertical level exists")
            .set_prev_graf(lines);
    }
}
