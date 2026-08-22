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
