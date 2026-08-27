use std::sync::Arc;

use tex_state::env::{AssignmentScope, CodeTableKind};
use tex_state::meaning::Meaning;
use tex_state::token::Catcode;

use crate::{
    CommandHostCapabilities, CommandSemanticDiagnostic, CommandState, RegisteredSourceKind,
    SourceRegistration,
};

use super::ErrorContextSelection;

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
