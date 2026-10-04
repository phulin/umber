//! Named-checkpoint publication, restore, and candidate settlement.

use super::*;
use crate::processor::ScannerStatus;

impl<G> CommandStateRoots<G> {
    fn capture_checkpoint(&self) -> CommandCapture<G> {
        CommandCapture {
            input: self.input.checkpoint_copy(),
            conditions: self.conditions.frames.clone(),
            afterassignment: self.afterassignment,
            alignment: self.alignment.checkpoint_copy(),
            name_in_progress: self.name_in_progress,
            last_diagnostic_location: self.last_diagnostic_location,
        }
    }

    /// Installs a copy of `capture`. Every root a capture omits because it is
    /// empty at a quiescent boundary is reset; job configuration and session
    /// accounting are left live.
    fn install_capture(&mut self, capture: &CommandCapture<G>) {
        self.input.install_checkpoint(&capture.input);
        self.scanner = ScannerStatus::Normal;
        self.conditions.frames.clone_from(&capture.conditions);
        self.alignment = capture.alignment.checkpoint_copy();
        self.transient.builders.clear();
        self.transient.rollback_roots.clear();
        self.transient.active_expansion_depth = 0;
        self.replay_completions.clear();
        self.semantic_diagnostics.clear();
        self.last_diagnostic_location = capture.last_diagnostic_location;
        self.afterassignment = capture.afterassignment;
        self.name_in_progress = capture.name_in_progress;
        self.pending_input_open = None;
    }

    /// Empty roots for a candidate forked from `self`. Job configuration and
    /// session registrations are copied; a capture installs everything else.
    fn candidate_session(&self) -> Self {
        let mut input = crate::input::InputState {
            terminal_context_line: self.input.terminal_context_line.clone(),
            pending_sources: self.input.pending_sources.clone(),
            ..crate::input::InputState::default()
        };
        input.levels.continue_context_epoch(&self.input.levels);
        Self {
            reachable_state_identity_enabled: self.reachable_state_identity_enabled,
            engine_semantics: self.engine_semantics,
            profile: self.profile,
            input,
            conditions: crate::conditionals::ConditionStack {
                frames: Vec::new(),
                next_identity: self.conditions.next_identity,
            },
            transient: crate::state::TransientState {
                next_builder_identity: self.transient.next_builder_identity,
                ..crate::state::TransientState::default()
            },
            ..Self::default()
        }
    }
}

impl<G> CommandState<G> {
    #[doc(hidden)]
    pub fn checkpoint_candidate_active(&self) -> bool {
        self.checkpoints.candidate_active()
    }

    /// Runs only the command-family fork for its standalone allocation gate.
    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    pub fn profile_fork_summary(
        self,
        summary: &CommandSummary<G>,
        universe: &Universe<G>,
    ) -> Result<Self, CommandRestoreError> {
        Self::fork_summary(self, summary, universe, universe)
    }

    /// Moves the live command roots aside as the accepted head of one
    /// candidate and installs the accepted summary's capture. Runtime scratch
    /// and attempt lanes begin quiescent.
    #[doc(hidden)]
    pub fn fork_summary(
        command: Self,
        summary: &CommandSummary<G>,
        source: &Universe<G>,
        destination: &Universe<G>,
    ) -> Result<Self, CommandRestoreError> {
        let source_generation = source
            .generation_owner()
            .map_err(|_| CommandRestoreError::ForeignGeneration)?;
        let destination_generation = destination
            .generation_owner()
            .map_err(|_| CommandRestoreError::ForeignGeneration)?;
        if !source_generation.same_generation(&destination_generation) {
            return Err(CommandRestoreError::ForeignGeneration);
        }
        let restore = command.prepare_summary_restore(summary, source)?;
        Ok(command.begin_checkpoint_candidate(restore))
    }

    /// Reports whether the live command machine can publish one named
    /// boundary without retaining a scanner, delivery, or attempt owner.
    ///
    /// The executor recomputes unreachable attempt suffixes before calling
    /// this projection. It does not capture or mutate state.
    #[must_use]
    pub fn named_boundary_is_quiescent(&self) -> bool {
        self.validate_summary_quiescence().is_ok()
    }

    /// Reports whether the terminal format boundary can discard the remaining
    /// command-only state without abandoning an active scanner or attempt.
    ///
    /// TeX's terminal `\dump` can leave unread input or macro levels behind;
    /// those levels are not part of a fresh-job format image. They are dropped
    /// only after aggregate image capture has succeeded.
    #[must_use]
    pub fn format_dump_is_quiescent(&self) -> bool {
        self.validate_format_dump_quiescence().is_ok()
    }

    /// Drops the command-only remainder of one successfully captured terminal
    /// format boundary. Engine semantics live in the format's Universe state;
    /// no input, continuation, capture, or attempt owner crosses with it.
    pub fn close_format_dump_boundary(&mut self) {
        assert!(
            self.format_dump_is_quiescent(),
            "terminal format closure requires quiescent command state"
        );
        self.roots = CommandStateRoots::default();
        self.checkpoints = CommandCheckpoints::default();
        self.attempt = crate::attempt::AttemptArena::default();
        self.scratch = crate::execution_scratch::ExecutionScratch::default();
        self.synchronize_delivery_mode_roots();
    }

    fn validate_summary_quiescence(&self) -> Result<(), CommandSummaryError> {
        self.validate_format_dump_quiescence()?;
        if self.transient.active_expansion_depth != 0
            || self.input.levels.has_macro_activation()
            || !self.replay_completions.is_empty()
        {
            return Err(CommandSummaryError::ExpansionActive);
        }
        Ok(())
    }

    /// Validates state that cannot be discarded even at a terminal format
    /// boundary. Input and macro replay levels are intentionally omitted:
    /// TeX's `\dump` may be delivered through either, and a successful format
    /// capture closes those command-only levels rather than serializing them.
    fn validate_format_dump_quiescence(&self) -> Result<(), CommandSummaryError> {
        match self.scanner {
            ScannerStatus::Normal => {}
            ScannerStatus::Skipping { .. } => return Err(CommandSummaryError::ConditionalSkip),
            ScannerStatus::Defining { .. } => return Err(CommandSummaryError::DefinitionScan),
            ScannerStatus::Matching { .. } => return Err(CommandSummaryError::MacroMatch),
            ScannerStatus::Aligning { .. } => return Err(CommandSummaryError::AlignmentScan),
            ScannerStatus::Absorbing { .. } => return Err(CommandSummaryError::AbsorbingScan),
        }
        if !self.semantic_diagnostics.is_empty() {
            return Err(CommandSummaryError::PendingSemanticDiagnostic);
        }
        if self.pending_input_open.is_some() {
            return Err(CommandSummaryError::ResourceSuspension);
        }
        if self.alignment.active_alignment.is_some() || self.alignment.active_cell.is_some() {
            return Err(CommandSummaryError::AlignmentTemplateActive);
        }
        if !self.alignment.suspended.is_empty() || !self.alignment.align_stack.is_empty() {
            return Err(CommandSummaryError::SuspendedAlignment);
        }
        if !self.transient.builders.is_empty() {
            return Err(CommandSummaryError::LiveTokenBuilder);
        }
        if !self.transient.rollback_roots.is_empty() {
            return Err(CommandSummaryError::LiveRollbackRoot);
        }
        if !self.attempt.is_clear()
            || !self.scratch.is_quiescent()
            || self.input.levels.arguments.is_matching()
        {
            return Err(CommandSummaryError::AttemptSuspended);
        }
        Ok(())
    }

    /// Consumes the sole live command owner and opens one candidate at the
    /// prevalidated `restore`: the live roots become the accepted head and
    /// every newer capture is detached until settlement.
    #[doc(hidden)]
    pub fn begin_checkpoint_candidate(mut self, restore: PreparedCommandRestore<G>) -> Self {
        assert_eq!(
            restore.owner, self.checkpoints.owner,
            "command candidate was prepared against this owner"
        );
        assert!(
            !self.checkpoints.candidate_active(),
            "one command candidate may be open at a time"
        );
        let index = self
            .checkpoints
            .position(restore.serial)
            .expect("command candidate base was prevalidated");
        self.abandon_checkpoint_attempt();
        let detached = self.checkpoints.captures.split_off(index + 1);
        let candidate = self.roots.candidate_session();
        let accepted = core::mem::replace(&mut self.roots, candidate);
        self.roots
            .install_capture(&self.checkpoints.captures[index].1);
        self.checkpoints.fork = Some(CheckpointFork {
            base: restore.serial,
            detached,
            accepted: Box::new(accepted),
        });
        self.synchronize_delivery_mode_roots();
        self
    }

    /// Discards the candidate and reinstalls the accepted head and its
    /// detached captures by move. A candidate is discarded whole, so an
    /// operation it halted mid-flight is dropped with it.
    #[doc(hidden)]
    pub fn reject_checkpoint_candidate(&mut self) {
        let fork = self
            .checkpoints
            .fork
            .take()
            .expect("command rejection requires a candidate fork");
        self.abandon_checkpoint_attempt();
        let base = self
            .checkpoints
            .position(fork.base)
            .expect("a candidate's base stays retained until settlement");
        self.checkpoints.captures.truncate(base + 1);
        self.checkpoints.captures.extend(fork.detached);
        let mut accepted = *fork.accepted;
        accepted
            .input
            .levels
            .continue_context_epoch(&self.roots.input.levels);
        self.roots = accepted;
        self.synchronize_delivery_mode_roots();
    }

    /// Keeps the candidate and drops the accepted head with its detached
    /// captures.
    #[doc(hidden)]
    pub fn accept_checkpoint_candidate(&mut self) {
        let fork = self
            .checkpoints
            .fork
            .take()
            .expect("command acceptance requires a candidate fork");
        drop(fork);
    }

    /// Publishes a named-boundary summary by copying the quiescent command
    /// remainder.
    pub fn publish_summary(
        &mut self,
        universe: &Universe<G>,
    ) -> Result<CommandSummary<G>, CommandSummaryError> {
        self.preflight_summary(universe)?;
        let generation = universe
            .generation_owner()
            .expect("command capture generation was preflighted");
        let root_source_anchor = self.input.levels.iter().find_map(|level| {
            let source @ crate::input::InputLevel {
                storage: crate::input::InputStorage::Source(_),
                ..
            } = level
            else {
                return None;
            };
            Some(
                source
                    .source_slot()
                    .expect("source input")
                    .cursor
                    .next_physical_offset,
            )
        });
        let capture = self.roots.capture_checkpoint();
        let retained_owner_bytes = capture.retained_bytes();
        let profile_fingerprint = self.checkpoint_profile_fingerprint().get();
        let reachable_state_identity_root = self.reachable_state_identity_root();
        let serial = self
            .checkpoints
            .push(capture)
            .expect("command publication serial was preflighted");
        Ok(CommandSummary {
            generation,
            owner: self.checkpoints.owner,
            serial,
            profile_fingerprint,
            root_source_anchor,
            reachable_state_identity_root,
            retained_owner_bytes,
        })
    }

    /// Checks named capture admission without publishing a command row.
    #[doc(hidden)]
    pub fn preflight_summary(&self, universe: &Universe<G>) -> Result<(), CommandSummaryError> {
        self.validate_summary_quiescence()?;
        if !universe.checkpoint_eligible() {
            return Err(CommandSummaryError::CheckpointIneligible);
        }
        universe
            .generation_owner()
            .map_err(|_| CommandSummaryError::GenerationUnavailable)?;
        self.checkpoints
            .next_serial
            .checked_add(1)
            .ok_or(CommandSummaryError::TimelineCapacity)?;
        Ok(())
    }

    /// Enables bounded command-root publication before incremental execution.
    /// Late selection fails closed because existing command activity may have
    /// crossed mutation barriers without maintaining semantic value metadata.
    #[doc(hidden)]
    pub fn enable_reachable_state_identity(&mut self) -> bool {
        if self.roots.reachable_state_identity_enabled {
            return true;
        }
        if !self.attempt.is_clear() || !self.roots.replay_completions.is_empty() {
            return false;
        }
        self.roots.reachable_state_identity_enabled = true;
        true
    }

    fn reachable_state_identity_root(&self) -> Option<u64> {
        self.roots
            .reachable_state_identity_enabled
            .then(|| bounded_command_identity(&self.roots))
    }

    /// Validates the profile, owner, and capture without mutating the
    /// destination. Within an open candidate only the candidate's base and
    /// its own later captures are restorable.
    pub fn prepare_summary_restore(
        &self,
        summary: &CommandSummary<G>,
        universe: &Universe<G>,
    ) -> Result<PreparedCommandRestore<G>, CommandRestoreError> {
        self.profile()
            .validate_fingerprint(
                CommandProfileBoundary::Summary,
                CommandProfileFingerprint::from_u64(summary.profile_fingerprint()),
            )
            .map_err(CommandRestoreError::Profile)?;
        let generation = universe
            .generation_owner()
            .map_err(|_| CommandRestoreError::ForeignGeneration)?;
        if !summary.addresses(&generation, self.checkpoints.owner) {
            return Err(CommandRestoreError::ForeignGeneration);
        }
        if !self.checkpoints.restorable(summary.serial) {
            return Err(CommandRestoreError::InvalidCursor);
        }
        Ok(PreparedCommandRestore {
            owner: self.checkpoints.owner,
            serial: summary.serial,
            brand: core::marker::PhantomData,
        })
    }

    /// Installs one prepared capture, dropping every newer capture on the
    /// current lineage and abandoning the live operation.
    pub fn apply_prepared_restore(
        &mut self,
        restore: PreparedCommandRestore<G>,
    ) -> Result<(), CommandRestoreError> {
        if restore.owner != self.checkpoints.owner {
            return Err(CommandRestoreError::ForeignGeneration);
        }
        if !self.checkpoints.restorable(restore.serial) {
            return Err(CommandRestoreError::InvalidCursor);
        }
        self.abandon_checkpoint_attempt();
        let capture = self.checkpoints.truncate_after(restore.serial);
        self.roots.install_capture(capture);
        self.synchronize_delivery_mode_roots();
        Ok(())
    }

    /// Validates and restores one named-boundary summary atomically.
    pub fn restore_summary(
        &mut self,
        summary: &CommandSummary<G>,
        universe: &Universe<G>,
    ) -> Result<(), CommandRestoreError> {
        let restore = self.prepare_summary_restore(summary, universe)?;
        self.apply_prepared_restore(restore)
    }

    /// Drops the capture named by one released checkpoint-history summary.
    /// Releases arriving during a candidate are deferred by the executor.
    #[doc(hidden)]
    pub fn release_checkpoint_summary(
        &mut self,
        released: &CommandSummary<G>,
    ) -> Result<(), CommandRestoreError> {
        if released.owner != self.checkpoints.owner {
            return Err(CommandRestoreError::ForeignGeneration);
        }
        if self.checkpoints.candidate_active() || !self.checkpoints.release(released.serial) {
            return Err(CommandRestoreError::InvalidCursor);
        }
        Ok(())
    }

    /// Rehomes the accepted editor backing after convergence, in the live
    /// stack and in every retained capture. This runs only after the current
    /// candidate has been rejected and creates no other generation. When no
    /// live row or capture still reads the accepted revision there is nothing
    /// to move.
    #[doc(hidden)]
    pub fn rehome_generated_editor_source(
        &mut self,
        accepted: &[u8],
        bytes: std::sync::Arc<[u8]>,
        old_start: usize,
        old_end: usize,
        new_end: usize,
    ) -> Result<(), crate::SourceRegistrationError> {
        let Some(backing) = self.input.levels.editor_backing(accepted).or_else(|| {
            self.checkpoints
                .captures
                .iter()
                .find_map(|(_, capture)| capture.input.levels.editor_backing(accepted))
        }) else {
            return Ok(());
        };
        let replacement = backing.rehome_generated(bytes.into())?;
        let offsets = crate::input::SourceOffsetMap::new(old_start, old_end, new_end);
        self.input
            .levels
            .rehome_generated_source(accepted, &replacement, offsets);
        for (_, capture) in &mut self.checkpoints.captures {
            capture
                .input
                .levels
                .rehome_generated_source(accepted, &replacement, offsets);
        }
        Ok(())
    }
}
