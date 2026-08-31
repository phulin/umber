use std::sync::Arc;

use tex_state::env::{AssignmentScope, CodeTableKind};
use tex_state::meaning::Meaning;
use tex_state::token::Catcode;

use crate::{
    CommandHostCapabilities, CommandSemanticDiagnostic, CommandState, RegisteredSourceKind,
    SourceRegistration,
};

use super::ErrorContextSelection;

#[test]
fn diagnostic_coordinate_allocates_only_when_published_and_rejects_stale_input() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        command.set_terminal_context_line("allocation-free coordinate");
        super::reset_diagnostic_context_measurement();

        let coordinate = command.diagnostic_context_coordinate();
        let foreign = CommandState::<()>::default().diagnostic_context_coordinate();
        assert_eq!(
            super::diagnostic_context_measurement(),
            super::DiagnosticContextMeasurement::default()
        );

        let stores = universe.command_context().expect("command context");
        assert_eq!(
            command.render_diagnostic_context(foreign, &stores),
            Err(crate::StaleDiagnosticContext)
        );
        let rendered = command
            .render_diagnostic_context(coordinate, &stores)
            .expect("live coordinate renders");
        assert!(rendered.contains("allocation-free coordinate"));
        let published = super::diagnostic_context_measurement();
        assert_eq!(published.renders, 1);
        assert_eq!(published.owned_allocations, 1);
        assert!(published.owned_bytes >= rendered.len());

        command.set_terminal_context_line("replacement context");
        assert_eq!(
            command.render_diagnostic_context(coordinate, &stores),
            Err(crate::StaleDiagnosticContext)
        );
        assert_eq!(super::diagnostic_context_measurement(), published);
    });
}

#[test]
fn stored_token_advance_invalidates_a_diagnostic_coordinate() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [tex_state::token::Token::Char {
                ch: 'x',
                cat: Catcode::Other,
            }],
        );
        let coordinate = command.diagnostic_context_coordinate();
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let mut destination = None;
        assert_eq!(
            processor
                .get_next_into(&mut destination)
                .expect("stored token delivers"),
            crate::DeliveryStatus::Command
        );
        drop(processor);
        drop(context);
        let stores = universe.command_context().expect("diagnostic context");
        assert_eq!(
            command.render_diagnostic_context(coordinate, &stores),
            Err(crate::StaleDiagnosticContext)
        );
    });
}

#[test]
fn source_advance_invalidates_a_diagnostic_coordinate() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(&b"A"[..]),
            ))
            .expect("source registration");
        command.open_registered_source(source).expect("source open");
        let coordinate = command.diagnostic_context_coordinate();
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        assert!(processor.get_next().expect("source delivers").is_some());
        drop(processor);
        drop(context);
        let stores = universe.command_context().expect("diagnostic context");
        assert_eq!(
            command.render_diagnostic_context(coordinate, &stores),
            Err(crate::StaleDiagnosticContext)
        );
    });
}

#[test]
fn push_pop_aba_and_context_scalars_reject_old_coordinates() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [tex_state::token::Token::Char {
                ch: 'x',
                cat: Catcode::Other,
            }],
        );
        let before_push = command.diagnostic_context_coordinate();
        crate::test_harness::push(
            &mut command,
            [tex_state::token::Token::Char {
                ch: 'y',
                cat: Catcode::Other,
            }],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        assert!(
            processor
                .get_next()
                .expect("nested token delivers")
                .is_some()
        );
        assert!(
            processor
                .get_next()
                .expect("nested level retires")
                .is_some()
        );
        drop(processor);
        drop(context);
        let stores = universe.command_context().expect("diagnostic context");
        assert_eq!(
            command.render_diagnostic_context(before_push, &stores),
            Err(crate::StaleDiagnosticContext)
        );

        let before_scalar = command.diagnostic_context_coordinate();
        command.set_retained_file_line_number(42);
        assert_eq!(
            command.render_diagnostic_context(before_scalar, &stores),
            Err(crate::StaleDiagnosticContext)
        );

        let before_force = command.diagnostic_context_coordinate();
        command.input.force_eof = true;
        assert_eq!(
            command.render_diagnostic_context(before_force, &stores),
            Err(crate::StaleDiagnosticContext)
        );

        let before_pending = command.diagnostic_context_coordinate();
        command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(&b"pending"[..]),
            ))
            .expect("pending source registration");
        assert_eq!(
            command.render_diagnostic_context(before_pending, &stores),
            Err(crate::StaleDiagnosticContext)
        );
    });
}

fn selected_context_levels(level_count: usize, error_context_lines: i32) -> Vec<Option<usize>> {
    let mut selection = ErrorContextSelection::new(error_context_lines);
    let mut selected = Vec::new();
    let mut deferred_bottom = None;
    for index in 0..level_count {
        if selection.display_immediately() {
            selected.push(Some(index));
        } else {
            deferred_bottom = Some(index);
        }
    }
    if selection.displays_elision_marker() {
        selected.push(None);
    }
    if selection.has_deferred_bottom() {
        selected.push(Some(
            deferred_bottom.expect("a deferred count has a bottom candidate"),
        ));
    }
    selected
}

#[test]
fn error_context_selection_matches_tex310_omission_matrix() {
    for level_count in 0_usize..=8 {
        for error_context_lines in -3..=6 {
            let bottom = level_count.saturating_sub(1);
            let mut shown = -1_i32;
            let mut expected = Vec::new();
            for index in 0..level_count {
                if index == 0 || index == bottom || shown < error_context_lines {
                    expected.push(Some(index));
                    shown = shown.saturating_add(1);
                } else if shown == error_context_lines {
                    expected.push(None);
                    shown = shown.saturating_add(1);
                }
            }
            assert_eq!(
                selected_context_levels(level_count, error_context_lines),
                expected,
                "level_count={level_count}, error_context_lines={error_context_lines}",
            );
        }
    }
}

#[test]
fn registered_source_delivers_through_the_generation_typed_processor() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(&b"A"[..]),
            ))
            .expect("source registration");
        command.open_registered_source(source).expect("source open");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        assert!(matches!(
            processor
                .get_next()
                .expect("delivery")
                .expect("first command")
                .meaning(),
            tex_state::ResolvedMeaning::Static(Meaning::CharToken { ch: 'A', .. })
        ));
    });
}

#[test]
fn transient_replay_preserves_authored_token_categories() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [tex_state::token::Token::Char {
                ch: 'x',
                cat: tex_state::token::Catcode::Letter,
            }],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        assert_eq!(
            processor
                .get_next()
                .expect("delivery")
                .expect("command")
                .meaning(),
            Meaning::CharToken {
                ch: 'x',
                cat: tex_state::token::Catcode::Letter,
            }
        );
    });
}

#[test]
fn invalid_source_character_is_reported_once_and_delivery_restarts() {
    crate::test_harness::with_universe(|universe| {
        universe
            .assign_code(
                CodeTableKind::Catcode,
                '!',
                Catcode::Invalid as i64,
                AssignmentScope::Global,
            )
            .expect("invalid catcode");
        let mut command = CommandState::default();
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(&b"!A"[..]),
            ))
            .expect("source registration");
        command.open_registered_source(source).expect("source open");
        let mut capabilities = CommandHostCapabilities::default();
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
            assert!(matches!(
                processor
                    .get_next()
                    .expect("delivery restarts")
                    .expect("following command")
                    .meaning(),
                tex_state::ResolvedMeaning::Static(Meaning::CharToken { ch: 'A', .. })
            ));
            assert!(processor.get_next().expect("source retirement").is_none());
        }

        let diagnostics = command.take_semantic_diagnostics();
        assert_eq!(diagnostics.len(), 1);
        assert!(matches!(
            &diagnostics[0],
            CommandSemanticDiagnostic::Recoverable { message, .. }
                if message == "Text line contains an invalid character"
        ));
        assert_eq!(command.input_level_count(), 0);
    });
}
