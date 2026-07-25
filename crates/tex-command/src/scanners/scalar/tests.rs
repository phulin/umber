use tex_state::Universe;
use tex_state::meaning::Meaning;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::*;
use crate::input::{
    ReplayTrace, RetirementBehavior, SharedTokenBuffer, TokenBehavior, TokenPayload,
};
use crate::{
    CommandHostCapabilities, CommandHostContext, CommandObservation, CommandObserver,
    CommandRuntime, CommandState, InputTransition, ObservedToken,
};

#[derive(Default)]
struct Recorder(Vec<CommandObservation>);

impl CommandObserver for Recorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

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
fn integer_radix_prefixes_deliver_digits_before_scanner_completion() {
    let mut command = CommandState::default();
    push(
        &mut command,
        "\"2A '17 42 ".chars().map(char_token).collect(),
    );
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );

    assert_eq!(processor.scan_integer().expect("hex scans").value, 42);
    assert_eq!(processor.scan_integer().expect("octal scans").value, 15);
    assert_eq!(processor.scan_integer().expect("decimal scans").value, 42);
}

#[test]
fn integer_internal_register_scans_its_index_through_command_input() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let count = universe.intern("count").symbol();
    universe.set_meaning(
        count,
        Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Count),
    );
    universe.set_count(0, 42);
    push(&mut command, vec![Token::Cs(count), char_token('0')]);
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );

    assert_eq!(
        processor
            .scan_integer()
            .expect("internal count scans")
            .value,
        42
    );
}

#[test]
fn internal_dimension_register_scans_and_bounds_its_index_through_command_input() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let dimen = universe.intern("dimen").symbol();
    universe.set_meaning(
        dimen,
        Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Dimen),
    );
    universe.set_dimen(20, tex_state::scaled::Scaled::from_raw(42));
    universe.set_dimen(0, tex_state::scaled::Scaled::from_raw(7));
    let mut capabilities = CommandHostCapabilities::default();

    push(
        &mut command,
        vec![Token::Cs(dimen), char_token('2'), char_token('0')],
    );
    {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        );
        assert_eq!(
            processor
                .scan_dimension()
                .expect("internal dimension scans")
                .value
                .raw(),
            42
        );
    }

    push(
        &mut command,
        vec![
            Token::Cs(dimen),
            char_token('2'),
            char_token('5'),
            char_token('6'),
        ],
    );
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );
    assert_eq!(
        processor
            .scan_dimension()
            .expect("out-of-range dimension register recovers")
            .value
            .raw(),
        7
    );
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

#[test]
fn fractional_in_unit_retires_the_final_probe_backup_before_n() {
    let mut command = CommandState::default();
    push(&mut command, "1.25in ".chars().map(char_token).collect());
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    )
    .with_observer(&mut recorder);

    assert_eq!(
        processor
            .scan_dimension()
            .expect("dimension scans")
            .value
            .raw(),
        5_920_358
    );

    let n = recorder
        .0
        .iter()
        .position(|observation| {
            matches!(
                observation,
                CommandObservation::Command(record)
                    if matches!(record.spelling, ObservedToken::Character { character: 'n', .. })
                        && record.boundary == crate::CommandDeliveryBoundary::Raw
            )
        })
        .expect("in suffix n is delivered raw");
    assert!(matches!(
        recorder.0.get(n - 1),
        Some(CommandObservation::Input(record)) if record.transition == InputTransition::Retire
    ));
}
