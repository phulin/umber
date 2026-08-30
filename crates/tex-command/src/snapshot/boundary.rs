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

    fn checkpoint_arenas(
        &mut self,
        attempt: AttemptMark,
    ) -> Result<CommandArenaCursors, CommandSummaryError> {
        let attempt_rows = attempt
            .checked_row_count()
            .ok_or(CommandSummaryError::TimelineCapacity)?;
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
        Ok(CommandArenaCursors::live(
            input.undo,
            parameters.undo,
            conditions.undo,
            groups.undo,
            attempt_rows,
        ))
    }

    fn checkpoint_stacks(&mut self) -> Result<CommandStackCursors, CommandSummaryError> {
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
        Ok(CommandStackCursors {
            input_depth: u32::try_from(self.input.levels.len())
                .map_err(|_| CommandSummaryError::TimelineCapacity)?,
            parameter_depth: u32::try_from(self.parameters.activations.len())
                .map_err(|_| CommandSummaryError::TimelineCapacity)?,
            condition_depth: u32::try_from(self.conditions.frames.len())
                .map_err(|_| CommandSummaryError::TimelineCapacity)?,
            alignment_depth: alignment.top,
            alignment_undo: alignment.undo,
            suspended_alignment_depth: suspended.top,
            suspended_alignment_undo: suspended.undo,
            replay_depth: u32::try_from(self.replay_completions.len())
                .map_err(|_| CommandSummaryError::TimelineCapacity)?,
            diagnostic_count: u32::try_from(self.semantic_diagnostics.len())
                .map_err(|_| CommandSummaryError::TimelineCapacity)?,
            group_payload_depth: u32::try_from(self.group_payloads.len())
                .map_err(|_| CommandSummaryError::TimelineCapacity)?,
            aftergroup_payload_count: aftergroups.top,
            aftergroup_payload_undo: aftergroups.undo,
            afterassignment_present: self.afterassignment.is_some(),
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
        let arenas = self.checkpoint_arenas(attempt)?;
        let stacks = self.checkpoint_stacks()?;
        self.timeline.retain(attempt, arenas, stacks)
    }

    fn resolve_restore(
        &self,
        owner: &CommandGenerationOwner<G>,
        cursor: CommandSnapshotCursor,
    ) -> Result<PreparedCommandRestore<G>, CommandRestoreError> {
        if !owner.addresses_cursor(cursor) || owner.timeline_owner != self.timeline.owner {
            return Err(CommandRestoreError::InvalidCursor);
        }
        let attempt = self
            .timeline
            .resolve(cursor, owner.timeline)
            .ok_or(CommandRestoreError::InvalidCursor)?;
        let attempt_rows = attempt
            .checked_row_count()
            .ok_or(CommandRestoreError::InvalidCursor)?;
        let arenas = cursor.arenas();
        let matches = self.input.levels.validates(crate::input::InputStackMark {
            top: cursor.stacks().input_depth(),
            undo: arenas.input_rows_mark(),
        }) && self.parameters.activations.validates(
            crate::timeline::LogicalStackMark {
                top: cursor.stacks().parameter_depth(),
                undo: arenas.input_words_mark(),
            },
        ) && self
            .conditions
            .frames
            .validates(crate::timeline::LogicalStackMark {
                top: cursor.stacks().condition_depth(),
                undo: arenas.parameter_words_mark(),
            })
            && self
                .group_payloads
                .validates(crate::timeline::LogicalStackMark {
                    top: cursor.stacks().group_payload_depth(),
                    undo: arenas.builder_words_mark(),
                })
            && self
                .aftergroup_payloads
                .validates(crate::timeline::LogicalStackMark {
                    top: cursor.stacks().aftergroup_payload_count(),
                    undo: cursor.stacks().aftergroup_payload_undo(),
                })
            && self
                .alignment
                .align_stack
                .validates(crate::timeline::LogicalStackMark {
                    top: cursor.stacks().alignment_depth(),
                    undo: cursor.stacks().alignment_undo(),
                })
            && self
                .alignment
                .suspended
                .validates(crate::timeline::LogicalStackMark {
                    top: cursor.stacks().suspended_alignment_depth(),
                    undo: cursor.stacks().suspended_alignment_undo(),
                })
            && arenas.attempt_rows() == attempt_rows;
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
            cursor,
            attempt,
            brand: PhantomData,
        })
    }

    fn restore_logical_stacks(&mut self, cursor: CommandSnapshotCursor) -> bool {
        let stacks = cursor.stacks();
        let arenas = cursor.arenas();
        self.input.levels.restore(crate::input::InputStackMark {
            top: stacks.input_depth(),
            undo: arenas.input_rows_mark(),
        }) && self
            .parameters
            .activations
            .restore(crate::timeline::LogicalStackMark {
                top: stacks.parameter_depth(),
                undo: arenas.input_words_mark(),
            })
            && self
                .conditions
                .frames
                .restore(crate::timeline::LogicalStackMark {
                    top: stacks.condition_depth(),
                    undo: arenas.parameter_words_mark(),
                })
            && self
                .group_payloads
                .restore(crate::timeline::LogicalStackMark {
                    top: stacks.group_payload_depth(),
                    undo: arenas.builder_words_mark(),
                })
            && self
                .aftergroup_payloads
                .restore(crate::timeline::LogicalStackMark {
                    top: stacks.aftergroup_payload_count(),
                    undo: stacks.aftergroup_payload_undo(),
                })
            && self
                .alignment
                .align_stack
                .restore(crate::timeline::LogicalStackMark {
                    top: stacks.alignment_depth(),
                    undo: stacks.alignment_undo(),
                })
            && self
                .alignment
                .suspended
                .restore(crate::timeline::LogicalStackMark {
                    top: stacks.suspended_alignment_depth(),
                    undo: stacks.suspended_alignment_undo(),
                })
    }

    fn begin_prepared_candidate(&mut self, restore: PreparedCommandRestore<G>) {
        debug_assert_eq!(restore.timeline_owner, self.timeline.owner);
        let stacks = restore.cursor.stacks();
        let arenas = restore.cursor.arenas();
        self.timeline
            .begin_checkpoint_candidate(restore.timeline, &mut self.roots);
        self.input
            .levels
            .begin_checkpoint_candidate(crate::input::InputStackMark {
                top: stacks.input_depth(),
                undo: arenas.input_rows_mark(),
            });
        self.parameters
            .activations
            .begin_checkpoint_candidate(crate::timeline::LogicalStackMark {
                top: stacks.parameter_depth(),
                undo: arenas.input_words_mark(),
            });
        self.conditions
            .frames
            .begin_checkpoint_candidate(crate::timeline::LogicalStackMark {
                top: stacks.condition_depth(),
                undo: arenas.parameter_words_mark(),
            });
        self.group_payloads
            .begin_checkpoint_candidate(crate::timeline::LogicalStackMark {
                top: stacks.group_payload_depth(),
                undo: arenas.builder_words_mark(),
            });
        self.aftergroup_payloads
            .begin_checkpoint_candidate(crate::timeline::LogicalStackMark {
                top: stacks.aftergroup_payload_count(),
                undo: stacks.aftergroup_payload_undo(),
            });
        self.alignment
            .align_stack
            .begin_checkpoint_candidate(crate::timeline::LogicalStackMark {
                top: stacks.alignment_depth(),
                undo: stacks.alignment_undo(),
            });
        self.alignment
            .suspended
            .begin_checkpoint_candidate(crate::timeline::LogicalStackMark {
                top: stacks.suspended_alignment_depth(),
                undo: stacks.suspended_alignment_undo(),
            });
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
        let arenas = self.checkpoint_arenas(attempt)?;
        let stacks = self.checkpoint_stacks()?;
        let (cursor, timeline) = self.timeline.retain_transient(attempt, arenas, stacks)?;
        Ok(TransientCommandSnapshot::new(
            CommandGenerationOwner::new(generation, self.timeline.owner, attempt, timeline),
            cursor,
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
        assert!(self.restore_logical_stacks(restore.cursor));
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
        self.resolve_restore(released.generation(), released.cursor())?;
        if let Some(oldest) = oldest_retained {
            self.resolve_restore(oldest.generation(), oldest.cursor())?;
        }
        let floor = oldest_retained.unwrap_or(released);
        if !self.timeline.release_frame(released.generation().timeline) {
            return Err(CommandRestoreError::InvalidCursor);
        }
        let command_journal_chunks_released = self
            .timeline
            .release_prefix(floor.generation().timeline)
            .ok_or(CommandRestoreError::InvalidCursor)?;
        let cursor = floor.cursor();
        let arenas = cursor.arenas();
        let stacks = cursor.stacks();
        let mut logical_stack_chunks_released = 0usize;
        macro_rules! release_stack_prefix {
            ($stack:expr, $top:expr, $undo:expr) => {
                logical_stack_chunks_released = logical_stack_chunks_released.saturating_add(
                    $stack
                        .release_prefix(crate::timeline::LogicalStackMark {
                            top: $top,
                            undo: $undo,
                        })
                        .ok_or(CommandRestoreError::InvalidCursor)?,
                );
            };
        }
        logical_stack_chunks_released = logical_stack_chunks_released.saturating_add(
            self.input
                .levels
                .release_prefix(crate::input::InputStackMark {
                    top: stacks.input_depth(),
                    undo: arenas.input_rows_mark(),
                })
                .ok_or(CommandRestoreError::InvalidCursor)?,
        );
        release_stack_prefix!(
            self.parameters.activations,
            stacks.parameter_depth(),
            arenas.input_words_mark()
        );
        release_stack_prefix!(
            self.conditions.frames,
            stacks.condition_depth(),
            arenas.parameter_words_mark()
        );
        release_stack_prefix!(
            self.group_payloads,
            stacks.group_payload_depth(),
            arenas.builder_words_mark()
        );
        release_stack_prefix!(
            self.aftergroup_payloads,
            stacks.aftergroup_payload_count(),
            stacks.aftergroup_payload_undo()
        );
        release_stack_prefix!(
            self.alignment.align_stack,
            stacks.alignment_depth(),
            stacks.alignment_undo()
        );
        release_stack_prefix!(
            self.alignment.suspended,
            stacks.suspended_alignment_depth(),
            stacks.suspended_alignment_undo()
        );
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
        self.apply_prepared_restore(restore)
    }
}
