//! Named-checkpoint publication and command-owner settlement.

use super::*;

impl<G> CommandState<G> {
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

    /// Moves the returned command-root loan into one candidate and restores the
    /// accepted summary's exact logical prefix. Runtime scratch and attempt
    /// lanes begin quiescent.
    #[doc(hidden)]
    pub fn fork_summary(
        mut command: Self,
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
        if !summary
            .generation()
            .addresses(&source_generation, command.timeline.owner)
            || !source_generation.same_generation(&destination_generation)
        {
            return Err(CommandRestoreError::ForeignGeneration);
        }
        let restore = command.resolve_restore(summary.generation(), summary.cursor())?;
        if !restore.attempt.is_empty() {
            return Err(CommandRestoreError::InvalidCursor);
        }
        command
            .profile()
            .validate_fingerprint(
                CommandProfileBoundary::Summary,
                CommandProfileFingerprint::from_u64(summary.profile_fingerprint()),
            )
            .map_err(CommandRestoreError::Profile)?;
        command.begin_prepared_candidate(restore);
        Ok(command)
    }

    /// Reports whether the live command machine can publish one named
    /// boundary without retaining a scanner, delivery, or attempt owner.
    ///
    /// The executor recomputes unreachable attempt suffixes before calling
    /// this projection. It does not allocate a timeline row or mutate state.
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
    /// no input, continuation, timeline, or attempt owner crosses with it.
    pub fn close_format_dump_boundary(&mut self) {
        assert!(
            self.format_dump_is_quiescent(),
            "terminal format closure requires quiescent command state"
        );
        self.roots = crate::state::CommandStateRoots::default();
        self.timeline = CommandTimeline::default();
        self.attempt = crate::CommandAttempt::default();
        self.scratch = crate::execution_scratch::ExecutionScratch::default();
        self.active_attempt_operation = None;
    }

    fn checkpoint_rollback_coordinates(
        &mut self,
    ) -> Result<CommandRollbackCoordinates, CommandSummaryError> {
        let input = self
            .input
            .levels
            .mark()
            .ok_or(CommandSummaryError::TimelineCapacity)?;
        let parameters = self
            .parameters
            .activations
            .mark()
            .ok_or(CommandSummaryError::TimelineCapacity)?;
        let conditions = self
            .conditions
            .frames
            .mark()
            .ok_or(CommandSummaryError::TimelineCapacity)?;
        let groups = self
            .group_payloads
            .mark()
            .ok_or(CommandSummaryError::TimelineCapacity)?;
        let aftergroups = self
            .aftergroup_payloads
            .mark()
            .ok_or(CommandSummaryError::TimelineCapacity)?;
        let alignment = self
            .alignment
            .align_stack
            .mark()
            .ok_or(CommandSummaryError::TimelineCapacity)?;
        let suspended = self
            .alignment
            .suspended
            .mark()
            .ok_or(CommandSummaryError::TimelineCapacity)?;
        Ok(CommandRollbackCoordinates {
            input,
            parameters,
            conditions,
            groups,
            aftergroups,
            alignment,
            suspended_alignment: suspended,
        })
    }

    fn validate_summary_quiescence(&self) -> Result<(), CommandSummaryError> {
        self.validate_format_dump_quiescence()?;
        if self.transient.active_expansion_depth != 0
            || !self.replay_completions.is_empty()
            || !self.pending_replay_completions.is_empty()
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
        match self.scanner.status() {
            ScannerStatus::Normal => {}
            ScannerStatus::Skipping { .. } => return Err(CommandSummaryError::ConditionalSkip),
            ScannerStatus::Defining { .. } => return Err(CommandSummaryError::DefinitionScan),
            ScannerStatus::Matching { .. } => return Err(CommandSummaryError::MacroMatch),
            ScannerStatus::Aligning { .. } => return Err(CommandSummaryError::AlignmentScan),
            ScannerStatus::Absorbing { .. } => return Err(CommandSummaryError::AbsorbingScan),
        }
        if self.scanner.warning().is_some() {
            return Err(CommandSummaryError::ScannerWarningContext);
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
        if self.active_attempt_operation.is_some()
            || !self.attempt.is_empty()
            || !self.scratch.is_quiescent()
        {
            return Err(CommandSummaryError::AttemptSuspended);
        }
        Ok(())
    }

    fn retain_cursor(
        &mut self,
        attempt: AttemptMark,
    ) -> Result<(CommandSnapshotCursor, CommandTimelineMark), CommandSummaryError> {
        let rollback = self.checkpoint_rollback_coordinates()?;
        self.timeline.retain(attempt, rollback)
    }

    fn resolve_restore(
        &self,
        owner: &CommandGenerationOwner<G>,
        cursor: CommandSnapshotCursor,
    ) -> Result<PreparedCommandRestore<G>, CommandRestoreError> {
        if !owner.addresses_cursor(cursor) || owner.timeline_owner != self.timeline.owner {
            return Err(CommandRestoreError::InvalidCursor);
        }
        let rollback = self
            .timeline
            .resolve(cursor, owner.timeline)
            .ok_or(CommandRestoreError::InvalidCursor)?;
        let attempt = owner.attempt;
        let matches = self.input.levels.validates(rollback.input)
            && self.parameters.activations.validates(rollback.parameters)
            && self.conditions.frames.validates(rollback.conditions)
            && self.group_payloads.validates(rollback.groups)
            && self.aftergroup_payloads.validates(rollback.aftergroups)
            && self.alignment.align_stack.validates(rollback.alignment)
            && self
                .alignment
                .suspended
                .validates(rollback.suspended_alignment);
        if !matches {
            return Err(CommandRestoreError::InvalidCursor);
        }
        self.attempt
            .arena()
            .validate_mark(attempt)
            .map_err(|_| CommandRestoreError::InvalidCursor)?;
        Ok(PreparedCommandRestore {
            timeline_owner: self.timeline.owner,
            timeline: owner.timeline,
            rollback,
            attempt,
            brand: PhantomData,
        })
    }

    fn restore_logical_stacks(&mut self, rollback: CommandRollbackCoordinates) -> bool {
        self.input.levels.restore(rollback.input)
            && self.parameters.activations.restore(rollback.parameters)
            && self.conditions.frames.restore(rollback.conditions)
            && self.group_payloads.restore(rollback.groups)
            && self.aftergroup_payloads.restore(rollback.aftergroups)
            && self.alignment.align_stack.restore(rollback.alignment)
            && self
                .alignment
                .suspended
                .restore(rollback.suspended_alignment)
    }

    fn begin_prepared_candidate(&mut self, restore: PreparedCommandRestore<G>) {
        debug_assert_eq!(restore.timeline_owner, self.timeline.owner);
        self.timeline
            .begin_checkpoint_candidate(restore.timeline, &mut self.roots);
        self.input
            .levels
            .begin_checkpoint_candidate(restore.rollback.input);
        self.parameters
            .activations
            .begin_checkpoint_candidate(restore.rollback.parameters);
        self.conditions
            .frames
            .begin_checkpoint_candidate(restore.rollback.conditions);
        self.group_payloads
            .begin_checkpoint_candidate(restore.rollback.groups);
        self.aftergroup_payloads
            .begin_checkpoint_candidate(restore.rollback.aftergroups);
        self.alignment
            .align_stack
            .begin_checkpoint_candidate(restore.rollback.alignment);
        self.alignment
            .suspended
            .begin_checkpoint_candidate(restore.rollback.suspended_alignment);
        self.attempt
            .arena_mut()
            .truncate(restore.attempt)
            .expect("prevalidated command candidate attempt mark");
    }

    /// Consumes the sole physical command owner and opens its prevalidated
    /// accepted/detached-prior/current transaction at `restore`.
    #[doc(hidden)]
    pub fn begin_checkpoint_candidate(mut self, restore: PreparedCommandRestore<G>) -> Self {
        self.begin_prepared_candidate(restore);
        self
    }

    #[doc(hidden)]
    pub fn reject_checkpoint_candidate(&mut self) {
        self.alignment.suspended.reject_checkpoint_candidate();
        self.alignment.align_stack.reject_checkpoint_candidate();
        self.aftergroup_payloads.reject_checkpoint_candidate();
        self.group_payloads.reject_checkpoint_candidate();
        self.conditions.frames.reject_checkpoint_candidate();
        self.parameters.activations.reject_checkpoint_candidate();
        self.input.levels.reject_checkpoint_candidate();
        self.timeline.reject_checkpoint_candidate(&mut self.roots);
    }

    #[doc(hidden)]
    pub fn accept_checkpoint_candidate(&mut self) {
        self.timeline.accept_checkpoint_candidate();
        self.input.levels.accept_checkpoint_candidate();
        self.parameters.activations.accept_checkpoint_candidate();
        self.conditions.frames.accept_checkpoint_candidate();
        self.group_payloads.accept_checkpoint_candidate();
        self.aftergroup_payloads.accept_checkpoint_candidate();
        self.alignment.align_stack.accept_checkpoint_candidate();
        self.alignment.suspended.accept_checkpoint_candidate();
    }

    /// Captures one exact in-session command position as bounded marks. The
    /// live root remains exclusively mutable and no accumulated payload moves.
    pub fn snapshot(
        &mut self,
        universe: &Universe<G>,
    ) -> Result<CommandStateSnapshot<G>, CommandSummaryError> {
        let generation = universe
            .generation_owner()
            .map_err(|_| CommandSummaryError::GenerationUnavailable)?;
        let attempt = self.attempt.arena().mark();
        let (cursor, timeline) = self.retain_cursor(attempt)?;
        Ok(CommandStateSnapshot::new(
            CommandGenerationOwner::new(generation, self.timeline.owner, attempt, timeline),
            cursor,
        ))
    }

    /// Captures a move-only rollback point inside one live operation.
    ///
    /// This is the synchronous nested-episode counterpart to [`Self::snapshot`]:
    /// it may retain an open attempt mark, but neither the mark nor its coarse
    /// generation owner can escape through a clone, summary, or detached DTO.
    pub fn transient_snapshot(
        &mut self,
        universe: &Universe<G>,
    ) -> Result<TransientCommandSnapshot<G>, CommandSummaryError> {
        let generation = universe
            .generation_owner()
            .map_err(|_| CommandSummaryError::GenerationUnavailable)?;
        let attempt = self.attempt.arena().mark();
        let rollback = self.checkpoint_rollback_coordinates()?;
        let (cursor, timeline) = self.timeline.retain_transient(rollback)?;
        let replay = self.input.begin_transient_replay();
        let scratch = self.scratch.begin_transient();
        Ok(TransientCommandSnapshot::new(
            CommandGenerationOwner::new(generation, self.timeline.owner, attempt, timeline),
            cursor,
            replay,
            scratch,
        ))
    }

    /// Publishes a bounded named-boundary summary. Ordinary mutation remains
    /// direct before and after capture; accumulated command roots are not
    /// cloned.
    pub fn publish_summary(
        &mut self,
        universe: &Universe<G>,
    ) -> Result<CommandSummary<G>, CommandSummaryError> {
        self.validate_summary_quiescence()?;
        let generation = universe
            .generation_owner()
            .map_err(|_| CommandSummaryError::GenerationUnavailable)?;
        let attempt = self.attempt.arena().mark();
        let (cursor, timeline) = self.retain_cursor(attempt)?;
        let root_source_anchor = self.input.levels.iter().find_map(|level| {
            let crate::input::InputLevel::Source(source) = level else {
                return None;
            };
            Some(
                self.input
                    .levels
                    .source_level_slot(source)
                    .cursor
                    .next_physical_offset,
            )
        });
        let retained_owner_bytes = self
            .roots
            .retained_bytes()
            .saturating_add(self.timeline.retained_bytes());
        Ok(CommandSummary::new(
            CommandGenerationOwner::new(generation, self.timeline.owner, attempt, timeline),
            cursor,
            self.checkpoint_profile_fingerprint().get(),
            root_source_anchor,
            self.reachable_state_identity_root(),
            retained_owner_bytes,
        ))
    }

    /// Enables bounded command-root publication before incremental execution.
    /// Late selection fails closed because existing command activity may have
    /// crossed mutation barriers without maintaining semantic value metadata.
    #[doc(hidden)]
    pub fn enable_reachable_state_identity(&mut self) -> bool {
        if self.roots.reachable_state_identity_enabled {
            return true;
        }
        if !self.attempt.is_empty()
            || self.active_attempt_operation.is_some()
            || !self.roots.replay_completions.is_empty()
            || !self.roots.pending_replay_completions.is_empty()
        {
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

    /// Validates every command owner, profile, and cursor without mutating
    /// the destination.
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
        if !summary
            .generation()
            .addresses(&generation, self.timeline.owner)
        {
            return Err(CommandRestoreError::ForeignGeneration);
        }
        self.resolve_restore(summary.generation(), summary.cursor())
    }

    /// Revalidates the destination and attempt suffix, then installs one
    /// prepared command root before discarding that suffix.
    pub fn apply_prepared_restore(
        &mut self,
        restore: PreparedCommandRestore<G>,
    ) -> Result<(), CommandRestoreError> {
        if restore.timeline_owner != self.timeline.owner {
            return Err(CommandRestoreError::ForeignGeneration);
        }
        self.attempt
            .arena()
            .validate_mark(restore.attempt)
            .map_err(|_| CommandRestoreError::InvalidCursor)?;
        let restored = self
            .timeline
            .restore_roots(restore.timeline, &mut self.roots);
        if !restored {
            return Err(CommandRestoreError::InvalidCursor);
        }
        assert!(self.restore_logical_stacks(restore.rollback));
        self.attempt
            .arena_mut()
            .truncate(restore.attempt)
            .expect("prepared command restore validated its attempt mark");
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

    /// Validates one aggregate checkpoint-history release against this sole
    /// physical command owner and reports its exact storage effect.
    ///
    /// `oldest_retained` is also validated when present so a stale or foreign
    /// aggregate low-water cannot be published. JobStart is frozen
    /// independently, so the chosen ordinary low-water becomes the journals'
    /// logical base and every whole prefix chunk is returned to its pool.
    #[doc(hidden)]
    pub fn release_checkpoint_summary(
        &mut self,
        released: &CommandSummary<G>,
        oldest_retained: Option<&CommandSummary<G>>,
    ) -> Result<CommandCheckpointReleaseReceipt, CommandRestoreError> {
        if released.generation().timeline_owner != self.timeline.owner
            || oldest_retained
                .is_some_and(|oldest| oldest.generation().timeline_owner != self.timeline.owner)
        {
            return Err(CommandRestoreError::ForeignGeneration);
        }
        let released_restore = self.resolve_restore(released.generation(), released.cursor())?;
        let floor = oldest_retained.unwrap_or(released);
        let floor_restore = if let Some(oldest) = oldest_retained {
            self.resolve_restore(oldest.generation(), oldest.cursor())?
        } else {
            released_restore
        };
        if !self.timeline.release_frame(released.generation().timeline) {
            return Err(CommandRestoreError::InvalidCursor);
        }
        let command_journal_chunks_released = self
            .timeline
            .release_prefix(floor.generation().timeline)
            .ok_or(CommandRestoreError::InvalidCursor)?;
        let rollback = floor_restore.rollback;
        let mut logical_stack_chunks_released = 0usize;
        macro_rules! release_stack_prefix {
            ($stack:expr, $mark:expr) => {
                logical_stack_chunks_released = logical_stack_chunks_released.saturating_add(
                    $stack
                        .release_prefix($mark)
                        .ok_or(CommandRestoreError::InvalidCursor)?,
                );
            };
        }
        logical_stack_chunks_released = logical_stack_chunks_released.saturating_add(
            self.input
                .levels
                .release_prefix(rollback.input)
                .ok_or(CommandRestoreError::InvalidCursor)?,
        );
        release_stack_prefix!(self.parameters.activations, rollback.parameters);
        release_stack_prefix!(self.conditions.frames, rollback.conditions);
        release_stack_prefix!(self.group_payloads, rollback.groups);
        release_stack_prefix!(self.aftergroup_payloads, rollback.aftergroups);
        release_stack_prefix!(self.alignment.align_stack, rollback.alignment);
        release_stack_prefix!(self.alignment.suspended, rollback.suspended_alignment);
        Ok(CommandCheckpointReleaseReceipt {
            timeline_frames_live: self.timeline.live_frame_count(),
            timeline_frame_capacity: self.timeline.frame_capacity(),
            timeline_frames_released: self.timeline.frames_released,
            command_journal_chunks_released,
            logical_stack_chunks_released,
        })
    }

    /// Validates an exact operation snapshot without changing live command
    /// roots or attempt storage.
    pub fn prepare_snapshot_restore(
        &self,
        snapshot: &CommandStateSnapshot<G>,
        universe: &Universe<G>,
    ) -> Result<PreparedCommandRestore<G>, CommandRestoreError> {
        let generation = universe
            .generation_owner()
            .map_err(|_| CommandRestoreError::ForeignGeneration)?;
        if !snapshot.addresses(&generation, self.timeline.owner) {
            return Err(CommandRestoreError::ForeignGeneration);
        }
        let restore = self.resolve_restore(snapshot.generation(), snapshot.cursor())?;
        self.profile()
            .validate_fingerprint(
                CommandProfileBoundary::Snapshot,
                self.expansion.profile.fingerprint(),
            )
            .map_err(CommandRestoreError::Profile)?;
        Ok(restore)
    }

    /// Restores one exact operation snapshot after complete validation.
    pub fn rollback(
        &mut self,
        snapshot: &CommandStateSnapshot<G>,
        universe: &Universe<G>,
    ) -> Result<(), CommandRestoreError> {
        let restore = self.prepare_snapshot_restore(snapshot, universe)?;
        self.apply_prepared_restore(restore)
    }

    /// Consumes and restores one synchronous nested-episode rollback point.
    pub fn rollback_transient(
        &mut self,
        snapshot: TransientCommandSnapshot<G>,
        universe: &Universe<G>,
    ) -> Result<(), CommandRestoreError> {
        let generation = universe
            .generation_owner()
            .map_err(|_| CommandRestoreError::ForeignGeneration)?;
        if !snapshot.addresses(&generation, self.timeline.owner) {
            return Err(CommandRestoreError::ForeignGeneration);
        }
        let restore = self.resolve_restore(&snapshot.generation, snapshot.cursor)?;
        self.profile()
            .validate_fingerprint(
                CommandProfileBoundary::Snapshot,
                self.expansion.profile.fingerprint(),
            )
            .map_err(CommandRestoreError::Profile)?;
        self.apply_prepared_restore(restore)?;
        self.scratch
            .rollback_transient(snapshot.scratch)
            .map_err(|_| CommandRestoreError::InvalidCursor)?;
        self.input
            .rollback_transient_replay(snapshot.replay)
            .map_err(|()| CommandRestoreError::InvalidCursor)
    }

    /// Consumes one synchronous nested-episode rollback point after its
    /// mutations have committed.
    ///
    /// The journal suffix remains authoritative history for any older named
    /// checkpoint. Only the transient frame row is released, so repeated
    /// successful nested episodes reuse one bounded coordinate without
    /// cloning roots or retaining one frame per command.
    pub fn commit_transient(
        &mut self,
        snapshot: TransientCommandSnapshot<G>,
        universe: &Universe<G>,
    ) -> Result<(), CommandRestoreError> {
        let generation = universe
            .generation_owner()
            .map_err(|_| CommandRestoreError::ForeignGeneration)?;
        if !snapshot.addresses(&generation, self.timeline.owner)
            || self
                .timeline
                .resolve(snapshot.cursor, snapshot.generation.timeline)
                .is_none()
        {
            return Err(CommandRestoreError::InvalidCursor);
        }
        self.timeline
            .release_frame(snapshot.generation.timeline)
            .then_some(())
            .ok_or(CommandRestoreError::InvalidCursor)?;
        self.scratch
            .commit_transient(snapshot.scratch)
            .map_err(|_| CommandRestoreError::InvalidCursor)?;
        self.input
            .commit_transient_replay()
            .map_err(|()| CommandRestoreError::InvalidCursor)
    }
}
