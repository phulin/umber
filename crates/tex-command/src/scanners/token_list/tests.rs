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
        let index = processor
            .scan_profile_register_index_retained()
            .into_result()
            .expect("token register index");
        let (tokens, source) = processor
            .scan_token_list_value(owner, false)
            .expect("token assignment");
        assert_eq!(index, 7);
        assert!(source.is_none());
        assert_eq!(
            processor
                .command
                .attempt_token_words(&tokens.expect("new balanced text has attempt-local storage"),)
                .expect("attempt list")
                .iter()
                .map(|word| word.semantic_token())
                .collect::<Vec<_>>(),
            [token('x', Catcode::Letter)]
        );
    });
}
