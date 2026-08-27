use std::sync::Arc;

use tex_state::env::{AssignmentScope, CodeTableKind};
use tex_state::meaning::Meaning;
use tex_state::token::Catcode;

use crate::{
    CommandHostCapabilities, CommandSemanticDiagnostic, CommandState, RegisteredSourceKind,
    SourceRegistration,
};

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
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
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
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
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
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        {
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
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
