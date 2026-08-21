use std::sync::Arc;

use tex_state::meaning::Meaning;

use crate::{CommandHostCapabilities, CommandState, RegisteredSourceKind, SourceRegistration};

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
        let mut processor =
            crate::test_harness::processor(&mut command, universe, &mut capabilities);

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
        let mut processor =
            crate::test_harness::processor(&mut command, universe, &mut capabilities);
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
