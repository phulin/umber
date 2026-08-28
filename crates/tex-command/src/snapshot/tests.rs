use core::cell::Cell;
use std::rc::Rc;
use tex_state::token::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};

use super::{
    COMMAND_FRAMES_PER_PAGE, CommandArenaCursors, CommandRestoreError, CommandSnapshotCursor,
    CommandStackCursors, CommandStateSnapshot, CommandSummary, CommandSummaryError,
};
use crate::scalar_journal::PackedJournalMark;

struct Brand;

fn word(ch: char) -> TracedTokenWord {
    TracedTokenWord::pack(
        Token::Char {
            ch,
            cat: Catcode::Other,
        },
        OriginId::UNKNOWN,
    )
}

#[derive(Debug)]
struct CountingOwner {
    clones: Rc<Cell<u32>>,
}

impl Clone for CountingOwner {
    fn clone(&self) -> Self {
        self.clones.set(self.clones.get() + 1);
        Self {
            clones: Rc::clone(&self.clones),
        }
    }
}

fn cursor(seed: u32) -> CommandSnapshotCursor {
    CommandSnapshotCursor::new(
        seed,
        CommandArenaCursors::new(seed + 1, seed + 2, seed + 3, seed + 4, seed + 5),
        CommandStackCursors {
            input_depth: seed + 6,
            parameter_depth: seed + 7,
            condition_depth: seed + 8,
            alignment_depth: seed + 9,
            alignment_undo: PackedJournalMark::synthetic(seed + 15),
            suspended_alignment_depth: seed + 16,
            suspended_alignment_undo: PackedJournalMark::synthetic(seed + 17),
            replay_depth: seed + 10,
            diagnostic_count: seed + 11,
            group_payload_depth: seed + 13,
            aftergroup_payload_count: seed + 14,
            aftergroup_payload_undo: PackedJournalMark::synthetic(seed + 18),
            afterassignment_present: seed.is_multiple_of(2),
        },
    )
}

#[test]
fn snapshot_clone_retains_one_coarse_owner_and_copies_only_cursors() {
    let clones = Rc::new(Cell::new(0));
    let snapshot = CommandStateSnapshot::<Brand, _>::new(
        CountingOwner {
            clones: Rc::clone(&clones),
        },
        cursor(7),
    );

    let cloned = snapshot.clone();

    assert_eq!(clones.get(), 1);
    assert_eq!(cloned.cursor(), cursor(7));
    assert_eq!(cloned.cursor().command_journal(), 7);
    assert_eq!(cloned.cursor().arenas().attempt_rows(), 12);
    assert_eq!(cloned.cursor().stacks().group_payload_depth(), 20);
    assert_eq!(cloned.cursor().stacks().aftergroup_payload_count(), 21);
}

#[test]
fn summary_is_a_coarse_owner_plus_fixed_restart_coordinates() {
    let clones = Rc::new(Cell::new(0));
    let summary = CommandSummary::<Brand, _>::new(
        CountingOwner {
            clones: Rc::clone(&clones),
        },
        cursor(13),
        0xfeed_beef,
        Some(1_024),
        None,
        4_096,
    );
    let cloned = summary.clone();
    let (_owner, restored, profile, anchor, identity, retained_bytes) = cloned.into_parts();

    assert_eq!(clones.get(), 1);
    assert_eq!(restored, cursor(13));
    assert_eq!(profile, 0xfeed_beef);
    assert_eq!(anchor, Some(1_024));
    assert_eq!(identity, None);
    assert_eq!(retained_bytes, 4_096);
}

#[test]
fn cursor_components_are_copy_only_scalar_coordinates() {
    fn assert_copy<T: Copy>() {}

    assert_copy::<CommandArenaCursors>();
    assert_copy::<CommandStackCursors>();
    assert_copy::<CommandSnapshotCursor>();

    let mark = cursor(1);
    assert_eq!(mark.arenas().input_rows(), 2);
    assert_eq!(mark.arenas().input_words(), 3);
    assert_eq!(mark.arenas().parameter_words(), 4);
    assert_eq!(mark.arenas().builder_words(), 5);
    assert_eq!(mark.stacks().input_depth(), 7);
    assert_eq!(mark.stacks().parameter_depth(), 8);
    assert_eq!(mark.stacks().condition_depth(), 9);
    assert_eq!(mark.stacks().alignment_depth(), 10);
    assert_eq!(mark.stacks().replay_depth(), 11);
    assert_eq!(mark.stacks().diagnostic_count(), 12);
}

#[test]
fn summary_rejection_names_suspended_attempts_separately() {
    assert_eq!(
        CommandSummaryError::AttemptSuspended.to_string(),
        "the command attempt is owned by a suspension"
    );
    assert_eq!(
        CommandSummaryError::ResourceSuspension.to_string(),
        "a command resource request is pending"
    );
}

#[test]
fn terminal_format_close_discards_macro_replay_without_weakening_named_boundaries() {
    let mut command = crate::CommandState::<Brand>::default();
    command.transient.active_expansion_depth = 1;

    assert!(!command.named_boundary_is_quiescent());
    assert!(command.format_dump_is_quiescent());

    command.close_format_dump_boundary();

    assert!(command.named_boundary_is_quiescent());
    assert_eq!(command.transient.active_expansion_depth, 0);
}

#[test]
fn retained_summary_restores_the_pre_mutation_command_root() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let summary = command
            .publish_summary(universe)
            .expect("quiescent command state publishes");

        command.begin_file_name().expect("filename guard opens");
        assert!(command.name_in_progress());

        let restore = command
            .prepare_summary_restore(&summary, universe)
            .expect("retained root validates");
        command
            .apply_prepared_restore(restore)
            .expect("prepared restore applies to its destination");

        assert!(!command.name_in_progress());
    });
}

#[test]
fn snapshot_restores_compact_alignment_phase_undo() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        command.alignment.align_state = 37;
        let snapshot = command
            .snapshot(universe)
            .expect("alignment phase snapshots");

        command.record_alignment_phase();
        command.alignment.align_state = -19;
        command
            .rollback(&snapshot, universe)
            .expect("alignment phase rolls back");

        assert_eq!(command.alignment.align_state, 37);
    });
}

#[test]
fn invalid_summary_cursor_leaves_live_command_state_unchanged() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let mut summary = command
            .publish_summary(universe)
            .expect("quiescent command state publishes");
        let captured = summary.cursor;
        let stacks = captured.stacks();
        summary.cursor = CommandSnapshotCursor::new(
            captured.command_journal(),
            captured.arenas(),
            CommandStackCursors {
                input_depth: stacks.input_depth(),
                parameter_depth: stacks.parameter_depth(),
                condition_depth: stacks.condition_depth(),
                alignment_depth: stacks.alignment_depth(),
                alignment_undo: stacks.alignment_undo(),
                suspended_alignment_depth: stacks.suspended_alignment_depth(),
                suspended_alignment_undo: stacks.suspended_alignment_undo(),
                replay_depth: stacks.replay_depth(),
                diagnostic_count: stacks.diagnostic_count() + 1,
                group_payload_depth: stacks.group_payload_depth(),
                aftergroup_payload_count: stacks.aftergroup_payload_count(),
                aftergroup_payload_undo: stacks.aftergroup_payload_undo(),
                afterassignment_present: stacks.afterassignment_present(),
            },
        );
        command.begin_file_name().expect("filename guard opens");

        assert!(matches!(
            command.prepare_summary_restore(&summary, universe),
            Err(CommandRestoreError::InvalidCursor)
        ));
        assert!(command.name_in_progress());
    });
}

#[test]
fn foreign_timeline_rejection_leaves_live_command_state_unchanged() {
    crate::test_harness::with_universe(|universe| {
        let mut source = crate::CommandState::default();
        let summary = source
            .publish_summary(universe)
            .expect("quiescent command state publishes");
        let mut destination = crate::CommandState::default();
        destination.begin_file_name().expect("filename guard opens");

        assert!(matches!(
            destination.prepare_summary_restore(&summary, universe),
            Err(CommandRestoreError::ForeignGeneration)
        ));
        assert!(destination.name_in_progress());
    });
}

#[test]
fn prepared_restore_cannot_be_applied_to_a_foreign_command_machine() {
    crate::test_harness::with_universe(|universe| {
        let mut source = crate::CommandState::default();
        let summary = source
            .publish_summary(universe)
            .expect("quiescent command state publishes");
        let restore = source
            .prepare_summary_restore(&summary, universe)
            .expect("source validates its summary");
        let mut destination = crate::CommandState::default();
        destination.begin_file_name().expect("filename guard opens");

        assert!(matches!(
            destination.apply_prepared_restore(restore),
            Err(CommandRestoreError::ForeignGeneration)
        ));
        assert!(destination.name_in_progress());
    });
}

#[test]
fn timeline_capture_rejects_nonempty_attempts_and_retains_only_empty_marks() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let empty = command.begin_attempt_operation();
        command
            .attempt
            .arena_mut()
            .begin_token_list()
            .expect("live attempt row allocates");
        assert!(matches!(
            command.snapshot(universe),
            Err(CommandSummaryError::AttemptSuspended)
        ));

        command
            .rollback_attempt_operation(empty)
            .expect("empty operation rolls back");
        let snapshot = command
            .snapshot(universe)
            .expect("empty command attempt snapshots");
        let retained = command
            .timeline
            .resolve(snapshot.cursor(), snapshot.generation().timeline)
            .expect("snapshot owner resolves");
        assert!(retained.is_empty());
    });
}

#[test]
fn rollback_restores_replay_lane_coordinates_after_candidate_admission() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        crate::test_harness::push(
            &mut command,
            [Token::Char {
                ch: 'a',
                cat: Catcode::Other,
            }],
        );
        let snapshot = command.snapshot(universe).expect("input state snapshots");
        crate::test_harness::push(
            &mut command,
            [Token::Char {
                ch: 'b',
                cat: Catcode::Other,
            }],
        );

        command
            .rollback(&snapshot, universe)
            .expect("replay coordinate rolls back");
        let mut capabilities = crate::CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut effects,
        );
        assert_eq!(
            processor
                .get_next()
                .expect("restored replay delivers")
                .expect("restored replay is present")
                .spelling(),
            word('a')
        );
    });
}

#[test]
fn rollback_restores_a_mutated_input_frame_cursor() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        crate::test_harness::push(
            &mut command,
            [
                Token::Char {
                    ch: 'a',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: 'b',
                    cat: Catcode::Other,
                },
            ],
        );
        let snapshot = command.snapshot(universe).expect("input frame snapshots");

        let mut capabilities = crate::CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
        {
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut effects,
            );
            assert_eq!(
                processor
                    .get_next()
                    .expect("candidate delivery")
                    .expect("candidate token")
                    .spelling(),
                word('a')
            );
        }

        command
            .rollback(&snapshot, universe)
            .expect("input frame rolls back");
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut effects,
        );
        assert_eq!(
            processor
                .get_next()
                .expect("restored delivery")
                .expect("restored token")
                .spelling(),
            word('a')
        );
    });
}

#[test]
fn dropping_a_summary_never_mutates_its_physical_timeline() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let summary = command
            .publish_summary(universe)
            .expect("quiescent command state publishes");
        assert_eq!(command.timeline.live_frame_count(), 1);

        command
            .begin_file_name()
            .expect("live command root mutates directly");
        drop(summary);
        assert_eq!(command.timeline.live_frame_count(), 1);
    });
}

#[test]
fn checkpoint_release_recycles_its_frame_and_preserves_protected_journals() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let protected = command
            .publish_summary(universe)
            .expect("protected summary publishes");
        let released = command
            .publish_summary(universe)
            .expect("interior summary publishes");
        let receipt = command
            .release_checkpoint_summary(&released, Some(&released))
            .expect("same-owner release validates");

        assert_eq!(receipt.timeline_frames_live(), 1);
        assert!(receipt.timeline_frame_capacity() >= receipt.timeline_frames_live());
        assert_eq!(receipt.timeline_frames_released(), 1);
        assert_eq!(receipt.command_journal_chunks_released(), 0);
        assert_eq!(receipt.logical_stack_chunks_released(), 0);
        assert!(matches!(
            command.release_checkpoint_summary(&released, None),
            Err(CommandRestoreError::InvalidCursor)
        ));

        let mut foreign = crate::CommandState::default();
        let foreign_summary = foreign
            .publish_summary(universe)
            .expect("foreign summary publishes");
        assert!(matches!(
            command.release_checkpoint_summary(&foreign_summary, None),
            Err(CommandRestoreError::ForeignGeneration)
        ));

        command
            .restore_summary(&protected, universe)
            .expect("protected root remains exact after release observation");
    });
}

#[test]
fn released_command_frames_plateau_across_thousands_of_boundaries() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let protected = command
            .publish_summary(universe)
            .expect("protected summary publishes");
        let mut prior = command
            .publish_summary(universe)
            .expect("first ordinary summary publishes");

        for _ in 0..4_096 {
            let next = command
                .publish_summary(universe)
                .expect("next ordinary summary publishes");
            command
                .release_checkpoint_summary(&prior, Some(&next))
                .expect("obsolete command frame releases");
            prior = next;
        }

        assert_eq!(command.timeline.live_frame_count(), 2);
        assert_eq!(command.timeline.frame_capacity(), COMMAND_FRAMES_PER_PAGE);
        assert!(command.timeline.frames_reused >= 4_000);
        command
            .restore_summary(&protected, universe)
            .expect("protected root remains exact");
    });
}

#[test]
fn named_checkpoint_capture_and_warmed_mutation_clone_no_roots() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let summary = command
            .publish_summary(universe)
            .expect("quiescent command state publishes");
        assert_eq!(command.timeline.source_cells_copied(), 0);

        command
            .begin_file_name()
            .expect("post-capture mutation stays direct");
        assert_eq!(command.timeline.source_cells_copied(), 0);

        command.end_file_name();
        command
            .begin_file_name()
            .expect("warmed mutation stays exclusive");
        command.end_file_name();
        assert_eq!(command.timeline.source_cells_copied(), 0);

        drop(summary);
    });
}

#[test]
fn repeated_same_scalar_writes_coalesce_to_first_old_and_final_live_value() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let snapshot = command.snapshot(universe).expect("checkpoint captures");
        let before = command.timeline.packed_journal_counters();

        for _ in 0..1_024 {
            command
                .timeline
                .record_name_in_progress(command.name_in_progress);
            command.name_in_progress = !command.name_in_progress;
        }
        let after = command.timeline.packed_journal_counters();
        assert_eq!(after.records - before.records, 1);
        assert_eq!(after.coalesced_writes - before.coalesced_writes, 1_023);
        assert_eq!(after.descriptor_publications, 0);
        assert!(!command.name_in_progress());

        command
            .rollback(&snapshot, universe)
            .expect("first old value restores");
        assert!(!command.name_in_progress());
    });
}

#[test]
fn token_frame_history_is_one_compact_record_without_payload_clones() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let words = [TokenWord::pack(Token::Char {
            ch: 'x',
            cat: Catcode::Other,
        })];
        let list = universe
            .command_context()
            .expect("command context")
            .allocate_token_list(&words)
            .expect("token list allocates");
        command.push_everypar(&universe.command_context().expect("command context"), list);
        let checkpoint = command.snapshot(universe).expect("checkpoint captures");
        let before = command.input.levels.counters();

        for _ in 0..1_024 {
            let Some(crate::input::InputLevel::Tokens(cursor)) = command.input.levels.last_mut()
            else {
                panic!("fixture token frame remains live");
            };
            cursor.retirement = match cursor.retirement {
                crate::input::RetirementBehavior::Pop => {
                    crate::input::RetirementBehavior::StopAtEnd
                }
                _ => crate::input::RetirementBehavior::Pop,
            };
        }

        let after = command.input.levels.counters();
        assert_eq!(after.payload_admissions, before.payload_admissions);
        assert_eq!(after.full_payload_history_clones, 0);
        assert_eq!(after.undo_records - before.undo_records, 1);
        assert_eq!(
            after.coalesced_mutations - before.coalesced_mutations,
            1_023
        );
        assert!(
            after.undo_record_bytes - before.undo_record_bytes <= 48,
            "token-frame record bytes: {}",
            after.undo_record_bytes - before.undo_record_bytes
        );

        command
            .rollback(&checkpoint, universe)
            .expect("token frame cursor restores exactly");
        let Some(crate::input::InputLevel::Tokens(cursor)) = command.input.levels.last() else {
            panic!("restored token frame remains live");
        };
        assert_eq!(cursor.retirement, crate::input::RetirementBehavior::Pop);
    });
}

#[test]
fn ordered_diagnostic_pushes_remain_noncoalescible_and_restore_in_order() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let checkpoint = command
            .publish_summary(universe)
            .expect("quiescent checkpoint publishes");
        let before = command.timeline.packed_journal_counters();
        for diagnostic in [11, 22, 33] {
            command.timeline.record_expansion_diagnostic_push();
            command.expansion.pending_diagnostics.push(diagnostic);
        }
        let after = command.timeline.packed_journal_counters();
        assert_eq!(after.records - before.records, 3);
        assert_eq!(after.ordered_events - before.ordered_events, 3);

        let mut candidate =
            crate::CommandState::fork_summary(command, &checkpoint, universe, universe)
                .expect("diagnostic suffix forks");
        assert!(candidate.expansion.pending_diagnostics.is_empty());
        candidate.reject_checkpoint_candidate();
        assert_eq!(candidate.expansion.pending_diagnostics, [11, 22, 33]);
    });
}

#[test]
fn surviving_summary_restarts_identically_after_a_newer_summary_is_dropped() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let survivor = command
            .publish_summary(universe)
            .expect("surviving summary publishes");
        command.begin_file_name().expect("later state mutates");
        let discarded = command
            .publish_summary(universe)
            .expect("newer summary publishes");

        drop(discarded);
        command
            .restore_summary(&survivor, universe)
            .expect("surviving summary restores");

        assert!(!command.name_in_progress());
    });
}

#[test]
fn stale_cursor_cannot_resolve_through_a_later_summary_owner() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let stale = command
            .publish_summary(universe)
            .expect("first summary publishes")
            .cursor();
        let mut later = command
            .publish_summary(universe)
            .expect("later summary publishes");
        later.cursor = stale;
        command.begin_file_name().expect("live state mutates");

        assert!(matches!(
            command.prepare_summary_restore(&later, universe),
            Err(CommandRestoreError::InvalidCursor)
        ));
        assert!(command.name_in_progress());
    });
}

#[test]
fn repeated_capture_keeps_one_physical_owner_and_append_only_marks() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let timeline_owner = command.timeline.owner;
        for _ in 0..64 {
            let summary = command
                .publish_summary(universe)
                .expect("summary publishes");
            drop(summary);
        }

        assert_eq!(command.timeline.owner, timeline_owner);
        assert_eq!(command.timeline.next_serial, 64);
        assert_eq!(command.timeline.live_frame_count(), 64);
        assert_eq!(command.timeline.frame_capacity(), 128);
    });
}

#[test]
fn command_fork_reject_and_accept_preserve_prefix_marks_without_copying_history() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let owner = command.timeline.owner;
        let early = command
            .publish_summary(universe)
            .expect("early summary publishes");
        {
            let state = universe.command_context().expect("command context");
            command
                .set_afterassignment(&state, word('x'))
                .expect("accepted suffix mutates");
        }
        let later = command
            .publish_summary(universe)
            .expect("later summary publishes");

        let mut rejected = crate::CommandState::fork_summary(command, &early, universe, universe)
            .expect("early command prefix forks");
        {
            let state = universe.command_context().expect("command context");
            rejected
                .set_afterassignment(&state, word('y'))
                .expect("candidate suffix mutates");
        }
        rejected.reject_checkpoint_candidate();
        assert_eq!(rejected.timeline.owner, owner);
        assert!(rejected.has_afterassignment());
        rejected
            .prepare_summary_restore(&later, universe)
            .expect("rejected accepted suffix mark reattaches");

        let mut accepted = crate::CommandState::fork_summary(rejected, &early, universe, universe)
            .expect("same prefix forks again");
        {
            let state = universe.command_context().expect("command context");
            accepted
                .set_afterassignment(&state, word('z'))
                .expect("replacement suffix mutates");
        }
        accepted.accept_checkpoint_candidate();
        assert_eq!(accepted.timeline.owner, owner);
        accepted
            .prepare_summary_restore(&early, universe)
            .expect("unchanged prefix mark survives acceptance");
        assert!(matches!(
            accepted.prepare_summary_restore(&later, universe),
            Err(CommandRestoreError::InvalidCursor)
        ));
        assert_eq!(accepted.timeline.source_cells_copied(), 0);
    });
}

#[cfg(feature = "profiling")]
#[test]
fn packed_8192_capture_cycles_append_fixed_marks_without_copying_roots() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let warm = command
            .publish_summary(universe)
            .expect("warm summary publishes");
        drop(warm);
        let before_released = command.timeline.frames_released;
        for _ in 0..8_192 {
            let summary = command
                .publish_summary(universe)
                .expect("summary publishes");
            drop(summary);
        }
        assert_eq!(command.timeline.frames_released, before_released);
        assert_eq!(command.timeline.source_cells_copied(), 0);
        assert_eq!(command.timeline.live_frame_count(), 8_193);
    });
}

#[test]
fn invalid_snapshot_validation_does_not_truncate_attempt_or_replace_roots() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let mut snapshot = command
            .snapshot(universe)
            .expect("live command snapshot captures bounded cursors");
        let captured = snapshot.cursor;
        let arenas = captured.arenas();
        snapshot.cursor = CommandSnapshotCursor::new(
            captured.command_journal(),
            CommandArenaCursors::new(
                arenas.input_rows(),
                arenas.input_words(),
                arenas.parameter_words(),
                arenas.builder_words(),
                arenas.attempt_rows() + 1,
            ),
            captured.stacks(),
        );
        command.begin_file_name().expect("filename guard opens");
        command
            .attempt
            .arena_mut()
            .begin_token_list()
            .expect("attempt suffix allocates");
        let live_attempt = command.attempt.arena().mark();

        assert!(matches!(
            command.prepare_snapshot_restore(&snapshot, universe),
            Err(CommandRestoreError::InvalidCursor)
        ));
        assert!(command.name_in_progress());
        assert_eq!(command.attempt.arena().mark(), live_attempt);
    });
}

#[test]
fn snapshot_restores_group_and_assignment_payload_roots() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        {
            let mut state = universe.command_context().expect("admitted state");
            command
                .begin_group(&mut state, tex_state::GroupKind::Simple, 7)
                .expect("group opens");
            command
                .save_aftergroup(&state, word('a'))
                .expect("aftergroup saved");
            command
                .set_afterassignment(&state, word('x'))
                .expect("afterassignment saved");
        }
        let snapshot = command
            .snapshot(universe)
            .expect("snapshot captures payloads");
        {
            let state = universe.command_context().expect("admitted state");
            command
                .save_aftergroup(&state, word('b'))
                .expect("candidate aftergroup");
            command
                .set_afterassignment(&state, word('y'))
                .expect("candidate afterassignment");
        }
        command
            .rollback(&snapshot, universe)
            .expect("snapshot restores payload roots");
        {
            let mut state = universe.command_context().expect("admitted state");
            assert_eq!(
                command.take_afterassignment(&state).expect("take restored"),
                Some(word('x'))
            );
            assert_eq!(
                command
                    .end_group(&mut state, tex_state::GroupKind::Simple)
                    .expect("restored group closes")
                    .into_aftergroup(),
                vec![word('a')]
            );
        }
    });
}

#[test]
fn summary_restores_group_and_assignment_payload_roots() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        {
            let mut state = universe.command_context().expect("admitted state");
            command
                .begin_group(&mut state, tex_state::GroupKind::Simple, 7)
                .expect("group opens");
            command
                .save_aftergroup(&state, word('a'))
                .expect("aftergroup saved");
            command
                .set_afterassignment(&state, word('x'))
                .expect("afterassignment saved");
        }
        let summary = command
            .publish_summary(universe)
            .expect("summary captures payloads");
        {
            let state = universe.command_context().expect("admitted state");
            command
                .save_aftergroup(&state, word('b'))
                .expect("candidate aftergroup");
            command
                .set_afterassignment(&state, word('y'))
                .expect("candidate afterassignment");
        }

        command
            .restore_summary(&summary, universe)
            .expect("summary restores payload roots");
        let mut state = universe.command_context().expect("admitted state");
        assert_eq!(
            command.take_afterassignment(&state).expect("take summary"),
            Some(word('x'))
        );
        assert_eq!(
            command
                .end_group(&mut state, tex_state::GroupKind::Simple)
                .expect("summary group closes")
                .into_aftergroup(),
            vec![word('a')]
        );
    });
}

#[test]
fn invalid_payload_cursor_leaves_live_payloads_unchanged() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let mut summary = command
            .publish_summary(universe)
            .expect("summary publishes");
        let captured = summary.cursor;
        let stacks = captured.stacks();
        summary.cursor = CommandSnapshotCursor::new(
            captured.command_journal(),
            captured.arenas(),
            CommandStackCursors {
                input_depth: stacks.input_depth(),
                parameter_depth: stacks.parameter_depth(),
                condition_depth: stacks.condition_depth(),
                alignment_depth: stacks.alignment_depth(),
                alignment_undo: stacks.alignment_undo(),
                suspended_alignment_depth: stacks.suspended_alignment_depth(),
                suspended_alignment_undo: stacks.suspended_alignment_undo(),
                replay_depth: stacks.replay_depth(),
                diagnostic_count: stacks.diagnostic_count(),
                group_payload_depth: stacks.group_payload_depth(),
                aftergroup_payload_count: stacks.aftergroup_payload_count() + 1,
                aftergroup_payload_undo: stacks.aftergroup_payload_undo(),
                afterassignment_present: stacks.afterassignment_present(),
            },
        );
        {
            let state = universe.command_context().expect("admitted state");
            command
                .set_afterassignment(&state, word('z'))
                .expect("live payload");
        }

        assert!(matches!(
            command.prepare_summary_restore(&summary, universe),
            Err(CommandRestoreError::InvalidCursor)
        ));
        assert!(command.has_afterassignment());
    });
}
