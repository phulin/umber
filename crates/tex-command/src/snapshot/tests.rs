use core::cell::Cell;
use std::rc::Rc;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::{
    CommandArenaCursors, CommandRestoreError, CommandSnapshotCursor, CommandStackCursors,
    CommandStateSnapshot, CommandSummary, CommandSummaryError,
};

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
        CommandStackCursors::new(
            seed + 6,
            seed + 7,
            seed + 8,
            seed + 9,
            seed + 10,
            seed + 11,
            seed + 12,
            seed + 13,
            seed + 14,
            seed % 2 == 0,
        ),
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
    assert_eq!(cloned.cursor().stacks().framing_event_count(), 19);
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
    );
    let cloned = summary.clone();
    let (_owner, restored, profile, anchor) = cloned.into_parts();

    assert_eq!(clones.get(), 1);
    assert_eq!(restored, cursor(13));
    assert_eq!(profile, 0xfeed_beef);
    assert_eq!(anchor, Some(1_024));
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
            CommandStackCursors::new(
                stacks.input_depth(),
                stacks.parameter_depth(),
                stacks.condition_depth(),
                stacks.alignment_depth(),
                stacks.replay_depth(),
                stacks.diagnostic_count() + 1,
                stacks.framing_event_count(),
                stacks.group_payload_depth(),
                stacks.aftergroup_payload_count(),
                stacks.afterassignment_present(),
            ),
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
        let source = crate::CommandState::default();
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
        let source = crate::CommandState::default();
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

        command.discard_attempt_operation(empty);
        let snapshot = command
            .snapshot(universe)
            .expect("empty command attempt snapshots");
        let (_, retained) = command
            .timeline
            .resolve(snapshot.cursor())
            .expect("timeline row resolves");
        assert!(retained.is_empty());
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
            CommandStackCursors::new(
                stacks.input_depth(),
                stacks.parameter_depth(),
                stacks.condition_depth(),
                stacks.alignment_depth(),
                stacks.replay_depth(),
                stacks.diagnostic_count(),
                stacks.framing_event_count(),
                stacks.group_payload_depth(),
                stacks.aftergroup_payload_count() + 1,
                stacks.afterassignment_present(),
            ),
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
