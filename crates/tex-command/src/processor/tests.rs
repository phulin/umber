use tex_state::meaning::Meaning;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::{CommandHostCapabilities, CommandState};

#[test]
fn processor_episode_borrows_generation_and_delivers_one_current_command() {
    crate::test_harness::with_universe(|universe| {
        let token = Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        };
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [token]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut processor = crate::test_harness::processor(
            &mut command,
            universe,
            &mut capabilities,
            &mut diagnostic_effects,
        );

        let delivered = processor
            .get_x_token()
            .expect("expanded delivery")
            .expect("one token");
        assert_eq!(delivered.spelling().semantic_token(), token);
        assert_eq!(
            delivered.meaning(),
            Meaning::CharToken {
                ch: 'x',
                cat: Catcode::Letter,
            }
        );
        assert!(processor.get_x_token().expect("end").is_none());
    });
}

#[test]
fn direct_source_command_captures_its_physical_line_before_retirement() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let source = command
            .register_source(
                crate::SourceRegistration::new(crate::RegisteredSourceKind::Generated, &b"\nX"[..])
                    .with_name("two-lines.tex"),
            )
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut processor = crate::test_harness::processor(
            &mut command,
            universe,
            &mut capabilities,
            &mut diagnostic_effects,
        );

        loop {
            let delivered = processor
                .get_next()
                .expect("raw delivery")
                .expect("second-line character");
            if delivered.spelling().semantic_token()
                == (Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                })
            {
                assert_eq!(delivered.direct_source_line_number(), Some(2));
                break;
            }
        }
    });
}

#[test]
fn direct_source_control_sequences_preserve_creation_policy_after_compact_delivery() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &br"\previouslyunseen \previouslyunseen"[..],
            ))
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut processor = crate::test_harness::processor(
            &mut command,
            universe,
            &mut capabilities,
            &mut diagnostic_effects,
        );

        let forbidden = processor
            .get_next()
            .expect("forbidden-creation delivery")
            .expect("first control sequence");
        assert!(
            forbidden
                .spelling()
                .semantic_token()
                .is_undefined_control_sequence()
        );
        assert_eq!(forbidden.control_sequence(), None);

        let allowed = processor
            .get_token()
            .expect("allowed-creation delivery")
            .expect("second control sequence");
        assert!(matches!(allowed.spelling().semantic_token(), Token::Cs(_)));
        assert!(allowed.control_sequence().is_some());
        assert_eq!(allowed.meaning(), Meaning::Undefined);
    });
}

#[test]
fn frozen_macro_primitive_observation_retains_endwrite_identity() {
    crate::test_harness::with_universe(|universe| {
        crate::install_tex82_unexpandable_primitives(universe);
        let endwrite = universe.primitive_token("endwrite").expect("write stopper");
        let mut command = CommandState::default();
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let processor = crate::test_harness::processor(
            &mut command,
            universe,
            &mut capabilities,
            &mut diagnostic_effects,
        );

        assert_eq!(
            processor.observed_token(TracedTokenWord::pack(endwrite, OriginId::UNKNOWN)),
            crate::observation::ObservedToken::FrozenPrimitive("endwrite".into())
        );
    });
}
