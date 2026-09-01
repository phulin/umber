use core::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use tex_state::token::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};

use super::{
    COMMAND_FRAMES_PER_PAGE, CommandRestoreError, CommandSnapshotCursor, CommandStateSnapshot,
    CommandSummary, CommandSummaryError,
};

struct Brand;

fn top_source_slot<G>(command: &crate::CommandState<G>) -> &crate::input::SourceSlot<G> {
    command
        .input
        .levels
        .top_source()
        .expect("source row is live")
        .1
}

fn top_source_key<G>(command: &crate::CommandState<G>) -> crate::input::SourceSlotKey {
    command
        .input
        .levels
        .top_source()
        .expect("source row is live")
        .0
        .slot
}

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
    CommandSnapshotCursor::new(seed)
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
fn cursor_is_only_one_copy_small_timeline_identity() {
    fn assert_copy<T: Copy>() {}

    assert_copy::<CommandSnapshotCursor>();
    assert_eq!(std::mem::size_of::<CommandSnapshotCursor>(), 4);

    let mark = cursor(1);
    assert_eq!(mark.command_journal(), 1);
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
        summary.cursor = CommandSnapshotCursor::new(captured.command_journal() + 1);
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
        let _rollback = command
            .timeline
            .resolve(snapshot.cursor(), snapshot.generation().timeline)
            .expect("snapshot owner resolves");
        assert!(snapshot.generation().attempt.is_empty());
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
fn checkpoint_release_recycles_its_frame_at_the_current_journal_floor() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let root = command
            .publish_summary(universe)
            .expect("root summary publishes");
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
            .restore_summary(&root, universe)
            .expect("unreleased root remains exact after release observation");
    });
}

#[test]
fn released_command_frames_plateau_across_thousands_of_boundaries() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let job_start = command
            .publish_summary(universe)
            .expect("JobStart summary publishes");
        command
            .release_checkpoint_summary(&job_start, None)
            .expect("frozen JobStart releases its live frame");
        let mut prior = command
            .publish_summary(universe)
            .expect("first ordinary summary publishes");
        let mut command_chunks_released = 0usize;

        for _ in 0..4_096 {
            command.begin_file_name().expect("toggle command scalar");
            command.end_file_name();
            let next = command
                .publish_summary(universe)
                .expect("next ordinary summary publishes");
            let receipt = command
                .release_checkpoint_summary(&prior, Some(&next))
                .expect("obsolete command frame releases");
            command_chunks_released =
                command_chunks_released.saturating_add(receipt.command_journal_chunks_released());
            prior = next;
        }

        assert_eq!(command.timeline.live_frame_count(), 1);
        assert_eq!(command.timeline.frame_capacity(), COMMAND_FRAMES_PER_PAGE);
        assert!(command.timeline.frames_reused >= 4_000);
        assert_eq!(command_chunks_released, 4_096);
        assert!(matches!(
            command.restore_summary(&job_start, universe),
            Err(CommandRestoreError::InvalidCursor)
        ));
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
        assert_eq!(after.descriptor_publications, 0);
        assert!(!command.name_in_progress());

        command
            .rollback(&snapshot, universe)
            .expect("first old value restores");
        assert!(!command.name_in_progress());
    });
}

#[test]
fn nested_source_pop_and_snapshot_restore_keep_authoritative_line_exact() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let parent = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"AB"[..],
            ))
            .expect("parent registration");
        command
            .open_registered_source(parent)
            .expect("parent opening");
        let mut capabilities = crate::CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        {
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            let first = processor
                .get_next()
                .expect("parent delivery")
                .expect("parent character");
            assert_eq!(
                first.spelling().semantic_token(),
                Token::Char {
                    ch: 'A',
                    cat: Catcode::Letter,
                }
            );
            assert_eq!(processor.current_file_line_number(), 1);
        }
        let snapshot = command.snapshot(universe).expect("mid-line snapshot");

        let nested = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b""[..],
            ))
            .expect("nested registration");
        command
            .open_registered_source(nested)
            .expect("nested opening");
        assert_eq!(command.current_file_line_number(), 0);
        {
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            let resumed_parent = processor
                .get_next()
                .expect("nested EOF resumes parent")
                .expect("parent second token");
            assert_eq!(
                resumed_parent.spelling().semantic_token(),
                Token::Char {
                    ch: 'B',
                    cat: Catcode::Letter,
                }
            );
            assert_eq!(processor.current_file_line_number(), 1);
        }

        command
            .rollback(&snapshot, universe)
            .expect("line and cursor restore together");
        assert_eq!(command.current_file_line_number(), 1);
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let replayed = processor
            .get_next()
            .expect("restored parent delivery")
            .expect("restored second token");
        assert_eq!(
            replayed.spelling().semantic_token(),
            Token::Char {
                ch: 'B',
                cat: Catcode::Letter,
            }
        );
    });
}

#[test]
fn source_first_touch_moves_cold_owners_and_restores_one_stable_slot() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"AB"[..],
            ))
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let slot = std::ptr::from_ref(top_source_slot(&command));
        let snapshot = command.snapshot(universe).expect("source checkpoint");
        let before = command.input.levels.counters();
        let mut capabilities = crate::CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        {
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            let first = processor
                .get_next()
                .expect("source delivery")
                .expect("source token");
            assert_eq!(
                first.spelling().semantic_token(),
                Token::Char {
                    ch: 'A',
                    cat: Catcode::Letter,
                }
            );
        }
        let after = command.input.levels.counters();
        assert_eq!(after.full_payload_history_clones, 0);
        assert_eq!(after.owner_swaps - before.owner_swaps, 1);
        assert_eq!(
            after.stored_state_captures - before.stored_state_captures,
            1
        );

        command
            .rollback(&snapshot, universe)
            .expect("source owner and lexer restore");
        let restored = top_source_slot(&command);
        assert_eq!(std::ptr::from_ref(restored), slot);
        assert!(restored.cursor.line.is_none());
        assert_eq!(restored.cursor.next_physical_offset, 0);
        assert!(!restored.cursor.backing_registered);
    });
}

#[test]
fn retired_source_slot_releases_owners_and_rejects_its_stale_generation() {
    crate::test_harness::with_universe(|_| {
        let first_bytes = Arc::<[u8]>::from(&b"first"[..]);
        let second_bytes = Arc::<[u8]>::from(&b"second"[..]);
        let mut command = crate::CommandState::<Brand>::default();
        let first = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                Arc::clone(&first_bytes),
            ))
            .expect("first source registers");
        command
            .open_registered_source(first)
            .expect("first source opens");
        let stale = top_source_key(&command);

        command
            .pop_input_level_at_end_of_job()
            .expect("unmarked source retires");
        assert_eq!(Arc::strong_count(&first_bytes), 1);

        let second = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                Arc::clone(&second_bytes),
            ))
            .expect("second source registers");
        command
            .open_registered_source(second)
            .expect("second source opens");
        let current = top_source_key(&command);
        assert_eq!(current.0.slot, stale.0.slot);
        assert_ne!(current.0.generation, stale.0.generation);
        assert_eq!(
            top_source_slot(&command).cursor.current_backing().id,
            second
        );
    });
}

#[test]
fn source_owner_swap_candidate_reject_redoes_prior_and_accept_promotes_current() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"AB"[..],
            ))
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let early = command.publish_summary(universe).expect("pre-line summary");
        let mut capabilities = crate::CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();

        macro_rules! deliver {
            ($command:expr) => {{
                let mut context = universe.command_context().expect("command context");
                crate::test_harness::processor(
                    $command,
                    &mut context,
                    &mut capabilities,
                    &mut fuel,
                    &mut diagnostic_effects,
                )
                .get_next()
                .expect("source delivery")
                .expect("source token")
                .spelling()
                .semantic_token()
            }};
        }

        assert_eq!(
            deliver!(&mut command),
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter,
            }
        );
        let accepted_slot = std::ptr::from_ref(top_source_slot(&command));

        let mut rejected = crate::CommandState::fork_summary(command, &early, universe, universe)
            .expect("source prefix forks");
        assert_eq!(
            deliver!(&mut rejected),
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter,
            }
        );
        rejected.reject_checkpoint_candidate();
        let rejected_source = top_source_slot(&rejected);
        assert_eq!(
            rejected_source
                .cursor
                .line
                .as_ref()
                .map(|line| line.cursor.byte_cursor),
            Some(1)
        );
        let restored_slot = std::ptr::from_ref(rejected_source);
        assert_eq!(restored_slot, accepted_slot);
        assert_eq!(
            deliver!(&mut rejected),
            Token::Char {
                ch: 'B',
                cat: Catcode::Letter,
            }
        );

        let mut accepted = crate::CommandState::fork_summary(rejected, &early, universe, universe)
            .expect("source prefix forks again");
        assert_eq!(
            deliver!(&mut accepted),
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter,
            }
        );
        accepted.accept_checkpoint_candidate();
        assert_eq!(
            deliver!(&mut accepted),
            Token::Char {
                ch: 'B',
                cat: Catcode::Letter,
            }
        );
        assert_eq!(
            accepted.input.levels.counters().full_payload_history_clones,
            0
        );
    });
}

#[test]
fn repeated_source_owner_swaps_coalesce_and_candidate_reject_redoes_the_final_owner() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"a\nb\nc"[..],
            ))
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let early = command.publish_summary(universe).expect("pre-line summary");
        let before = command.input.levels.counters();

        for expected_line in 1..=3 {
            command
                .input
                .levels
                .mutate_top_source(|source, slot| {
                    let stored = crate::input::SourceLevelExecutionState::cursor(source, slot);
                    let line = slot.cursor.load_next_line(13).expect("next line loads");
                    assert_eq!(line.physical.number(), expected_line);
                    (stored, ())
                })
                .expect("source owner transition records");
        }
        let after = command.input.levels.counters();
        assert_eq!(after.owner_swaps - before.owner_swaps, 1);
        assert_eq!(after.undo_records - before.undo_records, 1);

        let mut candidate = crate::CommandState::fork_summary(command, &early, universe, universe)
            .expect("source prefix forks");
        assert!(top_source_slot(&candidate).cursor.line.is_none());

        candidate.reject_checkpoint_candidate();
        assert_eq!(
            top_source_slot(&candidate)
                .cursor
                .line
                .as_ref()
                .map(|line| line.physical.number()),
            Some(3)
        );
    });
}

#[test]
fn source_owner_swaps_on_an_interval_local_row_need_no_inverse() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let empty = command.snapshot(universe).expect("empty snapshot");
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"a\nb"[..],
            ))
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let before = command.input.levels.counters();

        for _ in 0..2 {
            command.input.levels.mutate_top_source(|source, slot| {
                let stored = crate::input::SourceLevelExecutionState::cursor(source, slot);
                slot.cursor.load_next_line(13).expect("next line loads");
                (stored, ())
            });
        }
        let after = command.input.levels.counters();
        assert_eq!(after.owner_swaps, before.owner_swaps);
        assert_eq!(after.undo_records, before.undo_records);

        command
            .rollback(&empty, universe)
            .expect("interval-local row disappears at rollback");
        assert!(command.input.levels.is_empty());
    });
}

#[test]
fn compact_source_touch_then_token_row_reuse_restores_the_source_incarnation() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"ab"[..],
            ))
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        command
            .input
            .levels
            .mutate_top_source(|source, slot| {
                let stored = crate::input::SourceLevelExecutionState::cursor(source, slot);
                slot.cursor.load_next_line(13).expect("fixture line loads");
                (stored, ())
            })
            .expect("source row is live");
        let source_slot = top_source_key(&command);
        let snapshot = command.snapshot(universe).expect("source checkpoint");

        command
            .input
            .levels
            .mutate_top_source_lex(|_, slot| {
                slot.cursor
                    .line
                    .as_mut()
                    .expect("line remains loaded")
                    .cursor
                    .byte_cursor = 1;
            })
            .expect("source row remains live");
        command
            .input
            .levels
            .pop_project(|_, _| ())
            .expect("source pops");

        let words = [TokenWord::pack(Token::Char {
            ch: 'x',
            cat: Catcode::Other,
        })];
        let list = universe
            .command_context()
            .expect("command context")
            .allocate_token_list(&words)
            .expect("token list allocates");
        {
            let stores = universe.command_context().expect("command context");
            command.push_token_level(
                crate::input::PackedTokenSpanHandle::durable(stores.token_list(list)),
                crate::input::TokenBehavior::Ordinary,
                crate::input::RetirementBehavior::Pop,
                crate::input::ReplayTrace::Stored(crate::input::StoredReplayReason::EveryPar),
            );
        }

        command
            .rollback(&snapshot, universe)
            .expect("source incarnation restores before its compact inverse");
        assert_eq!(top_source_key(&command), source_slot);
        assert_eq!(
            top_source_slot(&command)
                .cursor
                .line
                .as_ref()
                .map(|line| line.cursor.byte_cursor),
            Some(0)
        );
    });
}

#[test]
fn cold_source_owner_swap_then_source_row_reuse_restores_in_order() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let original = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"a\nb"[..],
            ))
            .expect("original registration");
        let replacement = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"z"[..],
            ))
            .expect("replacement registration");
        command
            .open_registered_source(original)
            .expect("original opening");
        command
            .input
            .levels
            .mutate_top_source(|source, slot| {
                let stored = crate::input::SourceLevelExecutionState::cursor(source, slot);
                slot.cursor.load_next_line(13).expect("first line loads");
                (stored, ())
            })
            .expect("original source is live");
        let original_slot = top_source_key(&command);
        let snapshot = command.snapshot(universe).expect("source checkpoint");

        command
            .input
            .levels
            .mutate_top_source(|source, slot| {
                let stored = crate::input::SourceLevelExecutionState::cursor(source, slot);
                slot.cursor.load_next_line(13).expect("second line loads");
                (stored, ())
            })
            .expect("cold owner swap records");
        command
            .input
            .levels
            .pop_project(|_, _| ())
            .expect("original source pops");
        command
            .open_registered_source(replacement)
            .expect("replacement source opens");

        command
            .rollback(&snapshot, universe)
            .expect("replacement reverses before the cold owner inverse");
        assert_eq!(top_source_key(&command), original_slot);
        assert_eq!(
            top_source_slot(&command).cursor.current_backing().id,
            original
        );
        assert_eq!(
            top_source_slot(&command)
                .cursor
                .line
                .as_ref()
                .map(|line| line.physical.number()),
            Some(1)
        );
    });
}

#[test]
fn candidate_source_reuse_with_the_same_input_id_rejects_and_redoes_by_incarnation() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let root_source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"r"[..],
            ))
            .expect("root registration");
        let prior_source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"a"[..],
            ))
            .expect("prior registration");
        let candidate_source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"c"[..],
            ))
            .expect("candidate registration");
        command
            .open_registered_source(root_source)
            .expect("root source opens");
        let root = command.publish_summary(universe).expect("root summary");

        command.input.levels.pop_project(|_, _| ());
        command
            .open_registered_source(prior_source)
            .expect("prior source opens");
        let prior_identity = match command.input.levels.last() {
            Some(crate::input::InputLevel::Source(source)) => source.identity(),
            _ => panic!("prior source is live"),
        };
        command.input.levels.mutate_top_source(|source, slot| {
            let stored = crate::input::SourceLevelExecutionState::cursor(source, slot);
            slot.cursor.load_next_line(13).expect("prior line loads");
            (stored, ())
        });
        let prior_slot = top_source_key(&command);

        let mut candidate = crate::CommandState::fork_summary(command, &root, universe, universe)
            .expect("source prefix forks");
        candidate.input.levels.pop_project(|_, _| ());
        candidate
            .open_registered_source(candidate_source)
            .expect("candidate source opens");
        let candidate_identity = match candidate.input.levels.last() {
            Some(crate::input::InputLevel::Source(source)) => source.identity(),
            _ => panic!("candidate source is live"),
        };
        candidate.input.levels.mutate_top_source(|source, slot| {
            let stored = crate::input::SourceLevelExecutionState::cursor(source, slot);
            slot.cursor
                .load_next_line(13)
                .expect("candidate line loads");
            (stored, ())
        });
        let candidate_slot = top_source_key(&candidate);
        assert_eq!(candidate_identity, prior_identity);
        assert_ne!(candidate_slot, prior_slot);

        candidate.reject_checkpoint_candidate();
        assert_eq!(
            candidate
                .input
                .levels
                .top_source()
                .expect("prior source redoes")
                .0
                .identity(),
            prior_identity
        );
        assert_eq!(top_source_key(&candidate), prior_slot);
        assert_eq!(
            top_source_slot(&candidate).cursor.current_backing().id,
            prior_source
        );
        assert!(top_source_slot(&candidate).cursor.line.is_some());
    });
}

#[test]
fn source_accept_and_prefix_release_drop_each_obsolete_backing_owner() {
    crate::test_harness::with_universe(|universe| {
        let root_bytes = Arc::<[u8]>::from(&b"r"[..]);
        let prior_bytes = Arc::<[u8]>::from(&b"a"[..]);
        let current_bytes = Arc::<[u8]>::from(&b"c"[..]);
        let mut command = crate::CommandState::default();
        let root_source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                Arc::clone(&root_bytes),
            ))
            .expect("root registration");
        let prior_source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                Arc::clone(&prior_bytes),
            ))
            .expect("prior registration");
        let current_source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                Arc::clone(&current_bytes),
            ))
            .expect("current registration");
        command
            .open_registered_source(root_source)
            .expect("root source opens");
        let root = command.publish_summary(universe).expect("root summary");

        command.input.levels.pop_project(|_, _| ());
        command
            .open_registered_source(prior_source)
            .expect("prior source opens");
        let mut candidate = crate::CommandState::fork_summary(command, &root, universe, universe)
            .expect("source prefix forks");
        candidate.input.levels.pop_project(|_, _| ());
        candidate
            .open_registered_source(current_source)
            .expect("current source opens");
        candidate.accept_checkpoint_candidate();
        assert_eq!(Arc::strong_count(&prior_bytes), 1);
        let retained_root_owners = Arc::strong_count(&root_bytes);
        let retained_current_owners = Arc::strong_count(&current_bytes);
        assert_eq!(retained_root_owners, 3);
        assert_eq!(retained_current_owners, 3);

        let floor = candidate
            .publish_summary(universe)
            .expect("current source summary");
        candidate
            .release_checkpoint_summary(&root, Some(&floor))
            .expect("obsolete source prefix releases");
        assert_eq!(Arc::strong_count(&root_bytes), retained_root_owners - 2);
        assert_eq!(Arc::strong_count(&prior_bytes), 1);
        assert_eq!(Arc::strong_count(&current_bytes), retained_current_owners);
        assert_eq!(candidate.input.levels.counters().displaced_payloads, 0);
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
            let mutated = command.input.levels.toggle_top_token_retirement();
            assert!(mutated, "fixture token frame remains live");
        }

        let after = command.input.levels.counters();
        assert_eq!(after.payload_admissions, before.payload_admissions);
        assert_eq!(after.full_payload_history_clones, 0);
        assert_eq!(after.undo_records - before.undo_records, 1);
        assert!(
            after.undo_record_bytes - before.undo_record_bytes <= 48,
            "token-frame record bytes: {}",
            after.undo_record_bytes - before.undo_record_bytes
        );

        command
            .rollback(&checkpoint, universe)
            .expect("token frame cursor restores exactly");
        let Some(level) = command.input.levels.last() else {
            panic!("restored token frame remains live");
        };
        let cursor = level
            .stored_common()
            .expect("restored token frame remains live");
        assert_eq!(cursor.retirement, crate::input::RetirementBehavior::Pop);
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

#[test]
fn far_from_head_command_fork_preserves_rejected_siblings_and_accepted_candidate_marks() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let root = command
            .publish_summary(universe)
            .expect("root summary publishes");
        let mut accepted = Vec::new();
        for _ in 0..64 {
            command.begin_file_name().expect("accepted scalar mutates");
            let summary = command
                .publish_summary(universe)
                .expect("accepted summary publishes");
            command.end_file_name();
            accepted.push(summary);
        }
        let selected = accepted[7].clone();
        let later_sibling = accepted[47].clone();

        let before_reject = command.timeline.packed_journal_counters();
        let mut rejected =
            crate::CommandState::fork_summary(command, &selected, universe, universe)
                .expect("far accepted suffix detaches");
        rejected.end_file_name();
        let rejected_candidate_mark = rejected
            .publish_summary(universe)
            .expect("candidate mark publishes");
        rejected
            .begin_file_name()
            .expect("candidate scalar mutates");
        let before_reject_settle = rejected.timeline.packed_journal_counters();
        rejected.reject_checkpoint_candidate();
        let after_reject = rejected.timeline.packed_journal_counters();

        rejected
            .prepare_summary_restore(&later_sibling, universe)
            .expect("later accepted sibling reattaches after rejection");
        assert!(matches!(
            rejected.prepare_summary_restore(&rejected_candidate_mark, universe),
            Err(CommandRestoreError::InvalidCursor)
        ));
        assert!(after_reject.selected_rewind_records > before_reject.selected_rewind_records);
        assert!(after_reject.candidate_reject_records > before_reject.candidate_reject_records);
        assert_eq!(
            after_reject.accepted_redo_records - before_reject.accepted_redo_records,
            after_reject.selected_rewind_records - before_reject.selected_rewind_records
        );
        assert!(after_reject.frame_chain_transfers > before_reject_settle.frame_chain_transfers);
        assert_eq!(
            after_reject.frame_reuse_link_visits - before_reject_settle.frame_reuse_link_visits,
            0
        );

        let before_accept = rejected.timeline.packed_journal_counters();
        let mut accepted_command =
            crate::CommandState::fork_summary(rejected, &selected, universe, universe)
                .expect("same far prefix forks after rejection");
        accepted_command.end_file_name();
        let accepted_candidate_mark = accepted_command
            .publish_summary(universe)
            .expect("replacement candidate mark publishes");
        let before_accept_settle = accepted_command.timeline.packed_journal_counters();
        accepted_command.accept_checkpoint_candidate();
        let after_accept = accepted_command.timeline.packed_journal_counters();

        accepted_command
            .prepare_summary_restore(&root, universe)
            .expect("unchanged prefix root remains valid");
        accepted_command
            .prepare_summary_restore(&accepted_candidate_mark, universe)
            .expect("post-accept candidate mark remains valid");
        assert!(matches!(
            accepted_command.prepare_summary_restore(&later_sibling, universe),
            Err(CommandRestoreError::InvalidCursor)
        ));
        assert!(after_accept.accepted_chunks_released > before_accept.accepted_chunks_released);
        assert!(after_accept.frame_chain_transfers > before_accept_settle.frame_chain_transfers);
        assert_eq!(
            after_accept.frame_reuse_link_visits - before_accept_settle.frame_reuse_link_visits,
            0
        );
    });
}

#[test]
fn retired_command_frame_rows_receive_a_fresh_incarnation_only_when_reused() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let selected = command
            .publish_summary(universe)
            .expect("selected summary publishes");
        let stale = command
            .publish_summary(universe)
            .expect("obsolete sibling publishes");
        let stale_key = stale.generation.timeline.frame;

        let mut command = crate::CommandState::fork_summary(command, &selected, universe, universe)
            .expect("selected prefix forks");
        let candidate = command
            .publish_summary(universe)
            .expect("candidate summary publishes");
        command.accept_checkpoint_candidate();

        let after_settlement = command.timeline.packed_journal_counters();
        assert_eq!(after_settlement.frame_chain_transfers, 1);
        assert_eq!(after_settlement.frame_reuse_visits, 0);
        assert_eq!(after_settlement.frame_reuse_incarnations, 0);

        let reused = command
            .publish_summary(universe)
            .expect("retired row is reused lazily");
        let reused_key = reused.generation.timeline.frame;
        let after_reuse = command.timeline.packed_journal_counters();
        assert_eq!(reused_key.slot, stale_key.slot);
        assert_ne!(reused_key.generation, stale_key.generation);
        assert_eq!(after_reuse.frame_reuse_visits, 1);
        assert_eq!(after_reuse.frame_reuse_incarnations, 1);
        assert!(command.timeline.frame(stale_key).is_none());
        assert!(matches!(
            command.prepare_summary_restore(&stale, universe),
            Err(CommandRestoreError::InvalidCursor)
        ));
        command
            .prepare_summary_restore(&candidate, universe)
            .expect("accepted candidate mark remains valid");
    });
}

#[test]
fn rejected_candidate_frame_row_reuse_rejects_its_aba_stale_mark() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let selected = command
            .publish_summary(universe)
            .expect("selected summary publishes");
        let mut command = crate::CommandState::fork_summary(command, &selected, universe, universe)
            .expect("selected prefix forks");
        let stale = command
            .publish_summary(universe)
            .expect("candidate summary publishes");
        let stale_key = stale.generation.timeline.frame;
        command.reject_checkpoint_candidate();

        let settled = command.timeline.packed_journal_counters();
        assert_eq!(settled.frame_chain_transfers, 1);
        assert_eq!(settled.frame_reuse_incarnations, 0);
        let reused = command
            .publish_summary(universe)
            .expect("rejected candidate row reuses lazily");
        let reused_key = reused.generation.timeline.frame;
        assert_eq!(reused_key.slot, stale_key.slot);
        assert_ne!(reused_key.generation, stale_key.generation);
        assert!(matches!(
            command.prepare_summary_restore(&stale, universe),
            Err(CommandRestoreError::InvalidCursor)
        ));
    });
}

#[test]
fn reusable_frame_chain_owner_drains_newer_then_older_retired_chains() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let root = command
            .publish_summary(universe)
            .expect("root summary publishes");
        let obsolete = (0..3)
            .map(|_| {
                command
                    .publish_summary(universe)
                    .expect("obsolete summary publishes")
            })
            .collect::<Vec<_>>();
        let obsolete_keys = obsolete
            .iter()
            .map(|summary| summary.generation.timeline.frame)
            .collect::<Vec<_>>();
        let capacity = command.timeline.frame_capacity();

        let mut command = crate::CommandState::fork_summary(command, &root, universe, universe)
            .expect("root prefix forks");
        let accepted = command
            .publish_summary(universe)
            .expect("replacement summary publishes");
        command.accept_checkpoint_candidate();
        let first_old_reuse = command
            .publish_summary(universe)
            .expect("first old row reuses");
        assert_eq!(
            first_old_reuse.generation.timeline.frame.slot,
            obsolete_keys[0].slot
        );

        let mut command = crate::CommandState::fork_summary(command, &accepted, universe, universe)
            .expect("replacement prefix forks");
        let rejected = command
            .publish_summary(universe)
            .expect("candidate consumes the second old row");
        let rejected_key = rejected.generation.timeline.frame;
        assert_eq!(rejected_key.slot, obsolete_keys[1].slot);
        command.reject_checkpoint_candidate();

        let newer = command
            .publish_summary(universe)
            .expect("newer retired chain drains first");
        let older = command
            .publish_summary(universe)
            .expect("older retired chain remains linked");
        assert_eq!(newer.generation.timeline.frame.slot, rejected_key.slot);
        assert_ne!(
            newer.generation.timeline.frame.generation,
            rejected_key.generation
        );
        assert_eq!(older.generation.timeline.frame.slot, obsolete_keys[2].slot);
        assert_ne!(
            older.generation.timeline.frame.generation,
            obsolete_keys[2].generation
        );
        assert_eq!(command.timeline.frame_capacity(), capacity);
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
fn mismatched_snapshot_identity_does_not_truncate_attempt_or_replace_roots() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let mut snapshot = command
            .snapshot(universe)
            .expect("live command snapshot captures bounded cursors");
        let captured = snapshot.cursor;
        snapshot.cursor = CommandSnapshotCursor::new(captured.command_journal() + 1);
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
fn mismatched_payload_snapshot_identity_leaves_live_payloads_unchanged() {
    crate::test_harness::with_universe(|universe| {
        let mut command = crate::CommandState::default();
        let mut summary = command
            .publish_summary(universe)
            .expect("summary publishes");
        let captured = summary.cursor;
        summary.cursor = CommandSnapshotCursor::new(captured.command_journal() + 1);
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
