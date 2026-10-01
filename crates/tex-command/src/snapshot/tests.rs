use tex_state::Universe;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::{CommandRestoreError, CommandSummary, CommandSummaryError};
use crate::CommandState;

fn word(ch: char) -> TracedTokenWord {
    TracedTokenWord::pack(
        Token::Char {
            ch,
            cat: Catcode::Other,
        },
        OriginId::UNKNOWN,
    )
}

fn other(ch: char) -> Token {
    Token::Char {
        ch,
        cat: Catcode::Other,
    }
}

/// Reads raw commands until the next non-space character token.
fn next_character<G>(command: &mut CommandState<G>, universe: &mut Universe<G>) -> char {
    let mut capabilities = crate::CommandHostCapabilities::default();
    let mut fuel = crate::CommandFuelLedger::default();
    let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
    let mut context = universe.command_context().expect("command context");
    let mut processor = crate::test_harness::processor(
        command,
        &mut context,
        &mut capabilities,
        &mut fuel,
        &mut effects,
    );
    loop {
        match crate::test_harness::expect_raw_command(&mut processor)
            .spelling()
            .semantic_token()
        {
            Token::Char {
                cat: Catcode::Space,
                ..
            } => {}
            Token::Char { ch, .. } => return ch,
            token => panic!("expected a character token, got {token:?}"),
        }
    }
}

fn set_afterassignment<G>(command: &mut CommandState<G>, universe: &mut Universe<G>, ch: char) {
    let state = universe.command_context().expect("admitted state");
    command
        .set_afterassignment(&state, word(ch))
        .expect("afterassignment saved");
}

fn afterassignment<G>(command: &mut CommandState<G>, universe: &mut Universe<G>) -> Option<char> {
    let state = universe.command_context().expect("admitted state");
    let taken = command
        .take_afterassignment(&state)
        .expect("afterassignment taken");
    if let Some(taken) = taken {
        command
            .set_afterassignment(&state, taken)
            .expect("afterassignment restored");
    }
    taken.map(|taken| match taken.semantic_token() {
        Token::Char { ch, .. } => ch,
        token => panic!("expected a character payload, got {token:?}"),
    })
}

fn restore<G>(
    command: &mut CommandState<G>,
    summary: &CommandSummary<G>,
    universe: &Universe<G>,
) -> Result<(), CommandRestoreError> {
    command.restore_summary(summary, universe)
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
    let mut command = CommandState::<()>::default();
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
        let mut command = CommandState::default();
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
fn summary_restores_committed_diagnostic_site_without_changing_future_identity() {
    // §342's last committed file command remains observable after §71 EOF,
    // so restore must reinstall its report site together with input state.
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"A\nB"[..],
            ))
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let first = crate::DiagnosticLocation::new(source, 1, 0);
        let second = crate::DiagnosticLocation::new(source, 2, 0);
        command.set_last_diagnostic_location(Some(first));
        let identity = super::bounded_command_identity(&command.roots);
        let summary = command
            .publish_summary(universe)
            .expect("diagnostic site publishes");

        command.set_last_diagnostic_location(Some(second));
        assert_eq!(super::bounded_command_identity(&command.roots), identity);
        restore(&mut command, &summary, universe).expect("diagnostic site restores");
        assert_eq!(command.last_diagnostic_location(), Some(first));
        assert_eq!(super::bounded_command_identity(&command.roots), identity);
    });
}

#[test]
fn foreign_owner_rejection_leaves_live_command_state_unchanged() {
    crate::test_harness::with_universe(|universe| {
        let mut source = CommandState::default();
        let summary = source
            .publish_summary(universe)
            .expect("quiescent command state publishes");
        let mut destination = CommandState::default();
        destination.begin_file_name().expect("filename guard opens");

        assert!(matches!(
            destination.prepare_summary_restore(&summary, universe),
            Err(CommandRestoreError::ForeignGeneration)
        ));
        assert_eq!(
            destination.release_checkpoint_summary(&summary),
            Err(CommandRestoreError::ForeignGeneration)
        );
        assert!(destination.name_in_progress());
    });
}

#[test]
fn prepared_restore_cannot_be_applied_to_a_foreign_command_machine() {
    crate::test_harness::with_universe(|universe| {
        let mut source = CommandState::default();
        let summary = source
            .publish_summary(universe)
            .expect("quiescent command state publishes");
        let restore = source
            .prepare_summary_restore(&summary, universe)
            .expect("source validates its summary");
        let mut destination = CommandState::default();
        destination.begin_file_name().expect("filename guard opens");

        assert!(matches!(
            destination.apply_prepared_restore(restore),
            Err(CommandRestoreError::ForeignGeneration)
        ));
        assert!(destination.name_in_progress());
    });
}

#[test]
fn restore_drops_newer_captures_and_keeps_the_target_reusable() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let older = command.publish_summary(universe).expect("older publishes");
        set_afterassignment(&mut command, universe, 'x');
        let newer = command.publish_summary(universe).expect("newer publishes");
        set_afterassignment(&mut command, universe, 'y');

        restore(&mut command, &older, universe).expect("older restores");
        assert_eq!(afterassignment(&mut command, universe), None);
        assert_eq!(
            restore(&mut command, &newer, universe),
            Err(CommandRestoreError::InvalidCursor),
            "a capture newer than the restored one leaves the lineage"
        );

        set_afterassignment(&mut command, universe, 'z');
        restore(&mut command, &older, universe).expect("the target restores again");
        assert_eq!(afterassignment(&mut command, universe), None);
    });
}

#[test]
fn release_drops_only_the_named_capture() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let older = command.publish_summary(universe).expect("older publishes");
        set_afterassignment(&mut command, universe, 'x');
        let newer = command.publish_summary(universe).expect("newer publishes");

        command
            .release_checkpoint_summary(&older)
            .expect("older releases");
        assert_eq!(
            command.release_checkpoint_summary(&older),
            Err(CommandRestoreError::InvalidCursor)
        );
        assert_eq!(
            restore(&mut command, &older, universe),
            Err(CommandRestoreError::InvalidCursor)
        );
        set_afterassignment(&mut command, universe, 'y');
        restore(&mut command, &newer, universe).expect("newer survives the release");
        assert_eq!(afterassignment(&mut command, universe), Some('x'));
    });
}

#[test]
fn summary_restores_replay_rows_pushed_before_publication() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [other('a')]);
        let summary = command.publish_summary(universe).expect("input publishes");
        crate::test_harness::push(&mut command, [other('b')]);

        restore(&mut command, &summary, universe).expect("replay rows restore");
        assert_eq!(next_character(&mut command, universe), 'a');
    });
}

#[test]
fn summary_restores_a_replay_row_consumed_after_publication() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [other('a'), other('b')]);
        let summary = command.publish_summary(universe).expect("input publishes");
        assert_eq!(next_character(&mut command, universe), 'a');
        assert_eq!(next_character(&mut command, universe), 'b');

        restore(&mut command, &summary, universe).expect("replay row restores");
        assert_eq!(next_character(&mut command, universe), 'a');
        assert_eq!(next_character(&mut command, universe), 'b');
    });
}

#[test]
fn nested_source_pop_and_summary_restore_keep_authoritative_line_exact() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let parent = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"AB"[..],
            ))
            .expect("parent registration");
        command
            .open_registered_source(parent)
            .expect("parent opening");
        assert_eq!(next_character(&mut command, universe), 'A');
        assert_eq!(command.current_file_line_number(), 1);
        let summary = command.publish_summary(universe).expect("mid-line summary");

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
        assert_eq!(next_character(&mut command, universe), 'B');
        assert_eq!(command.current_file_line_number(), 1);

        restore(&mut command, &summary, universe).expect("line and cursor restore together");
        assert_eq!(command.current_file_line_number(), 1);
        assert_eq!(next_character(&mut command, universe), 'B');
    });
}

#[test]
fn summary_restores_group_and_assignment_payload_roots() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
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

        restore(&mut command, &summary, universe).expect("summary restores payload roots");
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
fn candidate_reject_reinstalls_the_accepted_head_and_detached_captures() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let base = command.publish_summary(universe).expect("base publishes");
        set_afterassignment(&mut command, universe, 'x');
        let detached = command
            .publish_summary(universe)
            .expect("detached publishes");
        set_afterassignment(&mut command, universe, 'y');

        let fork = command
            .prepare_summary_restore(&base, universe)
            .expect("base validates");
        let mut command = command.begin_checkpoint_candidate(fork);
        assert!(command.checkpoint_candidate_active());
        assert_eq!(afterassignment(&mut command, universe), None);
        assert!(matches!(
            command.prepare_summary_restore(&detached, universe),
            Err(CommandRestoreError::InvalidCursor)
        ));
        assert_eq!(
            command.release_checkpoint_summary(&base),
            Err(CommandRestoreError::InvalidCursor),
            "releases are deferred while a candidate is open"
        );

        set_afterassignment(&mut command, universe, 'z');
        let candidate = command
            .publish_summary(universe)
            .expect("candidate publishes");
        set_afterassignment(&mut command, universe, 'w');
        restore(&mut command, &candidate, universe).expect("candidate capture restores");
        assert_eq!(afterassignment(&mut command, universe), Some('z'));
        restore(&mut command, &base, universe).expect("candidate base restores");
        assert_eq!(afterassignment(&mut command, universe), None);
        assert_eq!(
            restore(&mut command, &candidate, universe),
            Err(CommandRestoreError::InvalidCursor)
        );

        command.reject_checkpoint_candidate();
        assert!(!command.checkpoint_candidate_active());
        assert_eq!(afterassignment(&mut command, universe), Some('y'));
        restore(&mut command, &detached, universe).expect("detached capture reattaches");
        assert_eq!(afterassignment(&mut command, universe), Some('x'));
    });
}

#[test]
fn candidate_accept_drops_the_accepted_head_and_detached_captures() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let base = command.publish_summary(universe).expect("base publishes");
        set_afterassignment(&mut command, universe, 'x');
        let detached = command
            .publish_summary(universe)
            .expect("detached publishes");

        let fork = command
            .prepare_summary_restore(&base, universe)
            .expect("base validates");
        let mut command = command.begin_checkpoint_candidate(fork);
        set_afterassignment(&mut command, universe, 'z');
        let candidate = command
            .publish_summary(universe)
            .expect("candidate publishes");
        command.accept_checkpoint_candidate();

        assert!(!command.checkpoint_candidate_active());
        assert_eq!(
            command.release_checkpoint_summary(&detached),
            Err(CommandRestoreError::InvalidCursor)
        );
        restore(&mut command, &base, universe).expect("base stays retained");
        assert_eq!(afterassignment(&mut command, universe), None);
        assert_eq!(
            restore(&mut command, &candidate, universe),
            Err(CommandRestoreError::InvalidCursor),
            "restoring the base dropped the newer candidate capture"
        );
    });
}

#[test]
fn candidate_keeps_pending_source_registrations_on_both_sides() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let base = command.publish_summary(universe).expect("base publishes");
        let pending = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"Q"[..],
            ))
            .expect("pending registration");

        let fork = command
            .prepare_summary_restore(&base, universe)
            .expect("base validates");
        let mut command = command.begin_checkpoint_candidate(fork);
        command
            .open_registered_source(pending)
            .expect("the candidate inherits the registration");
        assert_eq!(next_character(&mut command, universe), 'Q');

        command.reject_checkpoint_candidate();
        command
            .open_registered_source(pending)
            .expect("the accepted head keeps its registration");
        assert_eq!(next_character(&mut command, universe), 'Q');
    });
}

#[test]
fn editor_rehome_rebinds_live_input_and_retained_captures() {
    crate::test_harness::with_universe(|universe| {
        let accepted = b"A\nB\n";
        let mut command = CommandState::default();
        let source = command
            .register_source(
                crate::SourceRegistration::new(
                    crate::RegisteredSourceKind::Generated,
                    &accepted[..],
                )
                .with_name("doc.tex"),
            )
            .expect("editor registration");
        command
            .open_registered_source(source)
            .expect("editor opening");
        assert_eq!(next_character(&mut command, universe), 'A');
        let summary = command
            .publish_summary(universe)
            .expect("summary before the edit");
        assert_eq!(next_character(&mut command, universe), 'B');

        command
            .rehome_generated_editor_source(accepted, std::sync::Arc::from(&b"A\nC\n"[..]), 2, 3, 3)
            .expect("editor backing rehomes");
        restore(&mut command, &summary, universe).expect("rehomed capture restores");
        assert_eq!(next_character(&mut command, universe), 'C');
    });
}
