use tex_state::Universe;
use tex_state::meaning::Meaning;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::*;
use crate::input::{
    ReplayTrace, RetirementBehavior, SharedTokenBuffer, TokenBehavior, TokenPayload,
};
use crate::{CommandHostCapabilities, CommandHostContext, CommandRuntime, CommandState};

fn push(command: &mut CommandState, tokens: Vec<Token>) {
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(
            tokens
                .into_iter()
                .map(|token| TracedTokenWord::pack(token, OriginId::UNKNOWN))
                .collect::<Vec<_>>(),
        )),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
}

fn char_token(ch: char) -> Token {
    Token::Char {
        ch,
        cat: if ch.is_ascii_alphabetic() {
            Catcode::Letter
        } else {
            Catcode::Other
        },
    }
}

#[test]
fn scalar_forms_recovery_and_snapshot_use_only_command_input() {
    let mut command = CommandState::default();
    let snapshot = command.snapshot();
    push(
        &mut command,
        " --12 3.5pt plus 2pt minus 1pt"
            .chars()
            .map(char_token)
            .collect(),
    );
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();
    {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        );

        assert_eq!(processor.scan_integer().expect("integer scans").value, 12);
        let glue = processor.scan_glue(false).expect("glue scans");
        assert_eq!(glue.value.width.raw(), 3 * 65_536 + 32_768);
        assert_eq!(glue.value.stretch.raw(), 2 * 65_536);
        assert_eq!(glue.value.shrink.raw(), 65_536);
        assert_eq!(
            processor
                .scan_integer()
                .expect("EOF recovery scans")
                .recovery,
            ScalarRecovery::InsertedZero
        );
    }
    command.rollback(snapshot).expect("snapshot rolls back");
    assert!(command.publish_summary().is_ok());
}

#[test]
fn internal_values_and_failed_keywords_replay_canonically() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    universe.set_count(17, 41);
    let keyword = "pto".chars().map(char_token).collect::<Vec<_>>();
    push(&mut command, keyword);
    let mut capabilities = CommandHostCapabilities::default();
    {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        );
        assert!(!processor.scan_keyword("plus").expect("keyword scans").value);
        assert!(matches!(
            processor
                .get_x_token()
                .expect("replayed token delivers")
                .expect("replayed token exists")
                .meaning(),
            Meaning::CharToken { ch: 'p', .. }
        ));
        let _ = processor
            .get_x_token()
            .expect("second replay token delivers");
        let _ = processor
            .get_x_token()
            .expect("third replay token delivers");
    }
    push(
        &mut command,
        vec![Token::Cs(universe.intern("count17").symbol())],
    );
    let symbol = universe.intern("count17").symbol();
    universe.set_meaning(symbol, Meaning::CountRegister(17));
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );
    assert_eq!(
        processor
            .scan_internal_value()
            .expect("internal scan succeeds")
            .expect("count register is internal")
            .value,
        InternalValue::Integer(41)
    );
}
