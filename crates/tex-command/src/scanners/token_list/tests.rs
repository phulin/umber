use tex_state::token::{Catcode, Token};

use crate::{CommandHostCapabilities, CommandState};

fn token(ch: char, cat: Catcode) -> Token {
    Token::Char { ch, cat }
}

#[test]
fn token_register_assignment_returns_attempt_local_balanced_text() {
    crate::test_harness::with_universe(|universe| {
        let owner = universe.intern("toks").expect("owner").symbol();
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [
                token('7', Catcode::Other),
                token('=', Catcode::Other),
                token('{', Catcode::BeginGroup),
                token('x', Catcode::Letter),
                token('}', Catcode::EndGroup),
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut processor = crate::test_harness::processor(
            &mut command,
            universe,
            &mut capabilities,
            &mut diagnostic_effects,
        );
        let scanned = processor
            .scan_token_register_assignment(owner)
            .expect("token assignment");
        assert_eq!(scanned.index, 7);
        assert!(scanned.source.is_none());
        assert_eq!(
            processor
                .command
                .attempt_token_words(
                    scanned
                        .tokens
                        .expect("new balanced text has attempt-local storage"),
                )
                .expect("attempt list")
                .iter()
                .map(|word| word.semantic_token())
                .collect::<Vec<_>>(),
            [token('x', Catcode::Letter)]
        );
    });
}
