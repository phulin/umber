use std::sync::Arc;

use tex_state::Universe;
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
use tex_state::page::PageMark;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::*;
use crate::input::{ReplayTrace, RetirementBehavior};
use crate::observation::{
    CommandDeliveryBoundary, CommandObservation, CommandObserver, InputTransition,
};
use crate::processor::{DefinitionContext, ScannerStatus, ScannerWarning, TokenBuilderId};
use crate::{
    CommandHostCapabilities, CommandHostContext, CommandRuntime, CommandState,
    RegisteredSourceKind, SourceRegistration,
};

#[derive(Default)]
struct Recorder(Vec<CommandObservation>);

impl CommandObserver for Recorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

fn traced(token: Token) -> TracedTokenWord {
    TracedTokenWord::pack(token, OriginId::UNKNOWN)
}

fn processor<'a>(
    command: &'a mut CommandState,
    runtime: &'a mut CommandRuntime,
    universe: &'a mut Universe,
    capabilities: &'a mut CommandHostCapabilities,
) -> CommandProcessor<'a> {
    CommandProcessor::new(
        command,
        runtime,
        universe.command_context(),
        CommandHostContext::new(capabilities),
    )
}

fn install_macro(
    universe: &mut Universe,
    name: &str,
    replacement: Token,
) -> tex_state::interner::Symbol {
    let name = universe.intern(name).symbol();
    let empty = universe.intern_token_list(&[]);
    let replacement = universe.intern_token_list(&[replacement]);
    let definition =
        universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, replacement));
    universe.set_meaning(
        name,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition,
        },
    );
    name
}

fn install_expandable(
    universe: &mut Universe,
    name: &str,
    primitive: ExpandablePrimitive,
) -> tex_state::interner::Symbol {
    let symbol = universe.intern(name).symbol();
    universe.set_meaning(symbol, Meaning::ExpandablePrimitive(primitive));
    symbol
}

fn rendered(processor: &mut CommandProcessor<'_>) -> String {
    let mut text = String::new();
    while let Some(command) = processor.get_x_token().expect("conversion expands") {
        let Token::Char { ch, .. } = command.spelling().semantic_token() else {
            panic!("expected rendered character")
        };
        text.push(ch);
    }
    text
}

fn chars(processor: &mut CommandProcessor<'_>) -> String {
    let mut text = String::new();
    while let Some(command) = processor.get_x_token().expect("input expands") {
        if let Token::Char { ch, .. } = command.spelling().semantic_token() {
            text.push(ch);
        }
    }
    text
}

fn letters(text: &str) -> Vec<Token> {
    text.chars()
        .map(|ch| Token::Char {
            ch,
            cat: Catcode::Letter,
        })
        .collect()
}

#[test]
fn input_uses_only_capability_registered_backing_and_returns_to_parent() {
    let mut command = CommandState::default();
    let parent = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"\\input{inc}z".as_slice()),
        ))
        .expect("parent registers");
    command
        .open_registered_source(parent)
        .expect("parent opens");
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    install_expandable(&mut universe, "input", ExpandablePrimitive::Input);
    let mut capabilities = CommandHostCapabilities::default();
    capabilities.register_input(
        "inc",
        SourceRegistration::new(
            RegisteredSourceKind::World,
            Arc::<[u8]>::from(b"ab".as_slice()),
        ),
    );
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    assert_eq!(chars(&mut processor), "ab z ");
}

#[test]
fn endinput_keeps_its_line_but_retires_nested_source_before_the_next_line() {
    let mut command = CommandState::default();
    let parent = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"\\input{inc}z".as_slice()),
        ))
        .expect("parent registers");
    command
        .open_registered_source(parent)
        .expect("parent opens");
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    install_expandable(&mut universe, "input", ExpandablePrimitive::Input);
    install_expandable(&mut universe, "endinput", ExpandablePrimitive::EndInput);
    let mut capabilities = CommandHostCapabilities::default();
    capabilities.register_input(
        "inc",
        SourceRegistration::new(
            RegisteredSourceKind::World,
            Arc::<[u8]>::from(b"a\\endinput b\nc".as_slice()),
        ),
    );
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    assert_eq!(chars(&mut processor), "ab z ");
}

#[test]
fn jobname_and_mark_retrieval_replay_deterministic_state_values() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let jobname = install_expandable(&mut universe, "jobname", ExpandablePrimitive::JobName);
    let topmark = install_expandable(&mut universe, "topmark", ExpandablePrimitive::TopMark);
    let mark = universe.intern_token_list(&[Token::Char {
        ch: 'M',
        cat: Catcode::Letter,
    }]);
    universe.set_page_mark(PageMark::Top, mark);
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(jobname)),
            traced(Token::Cs(topmark)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    capabilities.set_job_name("paper");
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    assert_eq!(rendered(&mut processor), "paperM");
}

#[test]
fn scalar_conversions_render_immutable_other_character_tokens() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let number = install_expandable(&mut universe, "number", ExpandablePrimitive::Number);
    let roman = install_expandable(
        &mut universe,
        "romannumeral",
        ExpandablePrimitive::RomanNumeral,
    );
    let string = install_expandable(&mut universe, "string", ExpandablePrimitive::String);
    let target = universe.intern("target").symbol();
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(number)),
            traced(Token::Char {
                ch: '-',
                cat: Catcode::Other,
            }),
            traced(Token::Char {
                ch: '4',
                cat: Catcode::Other,
            }),
            traced(Token::Char {
                ch: '2',
                cat: Catcode::Other,
            }),
            traced(Token::Cs(roman)),
            traced(Token::Char {
                ch: '9',
                cat: Catcode::Other,
            }),
            traced(Token::Cs(string)),
            traced(Token::Cs(target)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
    assert_eq!(rendered(&mut processor), "-42ix\\target");
}

#[test]
fn conversion_rendering_publishes_recovery_input_before_its_first_token() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let number = install_expandable(&mut universe, "number", ExpandablePrimitive::Number);
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(number)),
            traced(Token::Char {
                ch: '-',
                cat: Catcode::Other,
            }),
            traced(Token::Char {
                ch: '4',
                cat: Catcode::Other,
            }),
            traced(Token::Char {
                ch: '2',
                cat: Catcode::Other,
            }),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .with_observer(&mut recorder);
        assert_eq!(rendered(&mut processor), "-42");
    }

    let scanner = recorder
        .0
        .iter()
        .position(|record| matches!(record, CommandObservation::Scanner(scanner) if scanner.kind == "integer" && scanner.value == "-42"))
        .expect("number scanner is observed before conversion output");
    let recovery = recorder
        .0
        .iter()
        .position(|record| matches!(record, CommandObservation::Input(input) if input.transition == InputTransition::Recovery))
        .expect("conversion output creates a recovery input level");
    let inserted = recorder
        .0
        .iter()
        .position(|record| matches!(record, CommandObservation::Recovery(recovery)
            if recovery.kind == RecoveryKind::InsertedToken
                && matches!(recovery.tokens.as_slice(), [crate::ObservedToken::Character { character: '-', catcode: Catcode::Other }, ..])))
        .expect("conversion output reports its inserted minus token");
    let raw = recorder
        .0
        .iter()
        .enumerate()
        .skip(recovery + 1)
        .position(|(_, record)| matches!(record, CommandObservation::Command(command) if command.boundary == CommandDeliveryBoundary::Raw && matches!(command.spelling, crate::ObservedToken::Character { character: '-', catcode: Catcode::Other })))
        .map(|offset| recovery + 1 + offset)
        .expect("rendered minus returns through raw delivery");
    assert!(scanner < recovery && recovery < inserted && inserted < raw);
}

#[test]
fn string_reads_its_target_with_normal_scanner_status_then_restores_definition() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let string = install_expandable(&mut universe, "string", ExpandablePrimitive::String);
    let target = install_macro(
        &mut universe,
        "constructedname",
        Token::Char {
            ch: 'X',
            cat: Catcode::Letter,
        },
    );
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(string)),
            traced(Token::Cs(target)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let _prior = command.begin_scanner_status(ScannerStatus::Defining(DefinitionContext {
        target: None,
        builder: TokenBuilderId(1),
        warning: ScannerWarning(1),
    }));
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .with_observer(&mut recorder);
        for expected in "\\constructedname".chars() {
            let command = processor
                .get_x_token()
                .expect("string conversion expands")
                .expect("string conversion produces a character");
            assert!(
                matches!(command.spelling().semantic_token(), Token::Char { ch, .. } if ch == expected)
            );
        }
    }

    let status_exit = recorder
        .0
        .iter()
        .position(|record| matches!(record, CommandObservation::ScannerStatus(status) if status.from == "defining" && status.to == "normal"))
        .expect("string leaves defining status before its target");
    let target_delivery = recorder
        .0
        .iter()
        .position(|record| matches!(record, CommandObservation::Command(command) if command.boundary == CommandDeliveryBoundary::Raw && matches!(command.spelling, crate::ObservedToken::ControlSequence(ref name) if name == "constructedname")))
        .expect("string target is delivered raw");
    let status_restore = recorder
        .0
        .iter()
        .rposition(|record| matches!(record, CommandObservation::ScannerStatus(status) if status.from == "normal" && status.to == "defining"))
        .expect("string restores defining status after its target");
    let recovery = recorder
        .0
        .iter()
        .position(|record| matches!(record, CommandObservation::Input(input) if input.transition == InputTransition::Recovery))
        .expect("string conversion installs its inserted output");
    assert!(status_exit < target_delivery);
    assert!(target_delivery < status_restore);
    assert!(status_restore < recovery);
    assert!(matches!(
        command.scanner.status(),
        ScannerStatus::Defining(_)
    ));
}

#[test]
fn the_toks_pushes_immutable_stored_input_without_reading_beyond_target() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let the = install_expandable(&mut universe, "the", ExpandablePrimitive::The);
    let register = universe.intern("stored").symbol();
    universe.set_meaning(register, Meaning::ToksRegister(7));
    let stored = universe.intern_token_list(&[Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    }]);
    universe.set_toks(7, stored);
    let trailing = Token::Char {
        ch: 'z',
        cat: Catcode::Letter,
    };
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(the)),
            traced(Token::Cs(register)),
            traced(trailing),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
    let opener = processor.get_next().expect("raw the").expect("the command");
    processor.expand(opener).expect("the inserts stored list");
    assert!(
        matches!(processor.command.input.levels.last(), Some(crate::input::InputLevel::Tokens(cursor))
        if matches!(cursor.payload, TokenPayload::Stored { tokens, .. } if tokens == stored))
    );
    // TeX82 §467 hands §465's copy to `ins_list`, so the level carries
    // §307's `inserted` token type and retires as a recovery, never as an
    // ordinary stored token list.
    assert!(
        matches!(processor.command.input.levels.last(), Some(crate::input::InputLevel::Tokens(cursor))
        if cursor.trace == ReplayTrace::Inserted && cursor.behavior == TokenBehavior::Recovery)
    );
    assert_eq!(
        processor
            .get_x_token()
            .expect("stored token")
            .expect("x")
            .spelling()
            .semantic_token(),
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter
        }
    );
    assert_eq!(
        processor
            .get_x_token()
            .expect("trailing token")
            .expect("z")
            .spelling()
            .semantic_token(),
        trailing
    );
}

/// TeX82 §467's `ins_the_toks` is observed exactly like §470's `conv_toks`.
///
/// Both reach the input stack through §323's `ins_list`, so `\the` of a
/// token parameter must publish the same inserted push and the same
/// first-token recovery record that a rendered conversion does -- and a
/// leading control sequence is §289's `info(p)>=cs_token_flag` case.
#[test]
fn the_toks_publishes_an_inserted_push_naming_its_leading_control_sequence() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let the = install_expandable(&mut universe, "the", ExpandablePrimitive::The);
    // A token *parameter*, not a register: §466 copies both the same way,
    // and the divergence this test pins was `\the\headline`.
    let parameter = universe.intern("everypar").symbol();
    universe.set_meaning(parameter, Meaning::TokParam(1));
    let leading = universe.intern("hfil").symbol();
    let stored = universe.intern_token_list(&[Token::Cs(leading)]);
    universe.set_tok_param(tex_state::env::banks::TokParam::EVERY_PAR, stored);
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(the)),
            traced(Token::Cs(parameter)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .with_observer(&mut recorder);
        let opener = processor.get_next().expect("raw the").expect("the command");
        processor.expand(opener).expect("the inserts its copy");
    }
    let push = recorder
        .0
        .iter()
        .position(|record| {
            matches!(record, CommandObservation::Input(input)
                if input.transition == InputTransition::Recovery
                    && input.reason == crate::observation::InputReason::Recovery)
        })
        .expect("the_toks installs an observed inserted level");
    assert!(matches!(
        &recorder.0[push + 1],
        CommandObservation::Recovery(recovery)
            if recovery.kind == RecoveryKind::InsertedControlSequence
                && recovery.tokens
                    == vec![crate::observation::ObservedToken::ControlSequence("hfil".into())]
    ));
}

#[test]
fn ordinary_loop_expands_macro_body_on_the_canonical_raw_path() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let macro_name = install_macro(
        &mut universe,
        "m",
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
    );
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![traced(Token::Cs(macro_name))])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    let delivered = processor
        .get_x_token()
        .expect("macro expands")
        .expect("body token");
    assert_eq!(
        delivered.spelling().semantic_token(),
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter
        }
    );
    assert_eq!(processor.command.expansion.cumulative_expansions, 1);
    assert_eq!(processor.command.transient.active_expansion_depth, 0);
}

#[test]
fn completed_expansion_rolls_back_to_the_exact_scalar_input_state() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let macro_name = install_macro(
        &mut universe,
        "m",
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
    );
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![traced(Token::Cs(macro_name))])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let expected = command.clone();
    let snapshot = command.snapshot();
    let mut capabilities = CommandHostCapabilities::default();

    let first = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .get_x_token()
            .expect("macro expands")
            .expect("body token")
            .spelling()
            .semantic_token()
    };
    command.rollback(snapshot).expect("rollback succeeds");
    assert_eq!(command, expected);

    let replayed = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .get_x_token()
            .expect("rolled-back macro expands")
            .expect("replayed body token")
            .spelling()
            .semantic_token()
    };
    assert_eq!(replayed, first);
}

#[test]
fn noexpand_suppresses_one_macro_delivery_without_changing_its_spelling() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let noexpand = universe.intern("noexpand").symbol();
    universe.set_meaning(
        noexpand,
        Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand),
    );
    let macro_name = install_macro(
        &mut universe,
        "m",
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
    );
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(noexpand)),
            traced(Token::Cs(macro_name)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    let delivered = processor
        .get_x_token()
        .expect("noexpand completes")
        .expect("target");
    assert_eq!(delivered.spelling().semantic_token(), Token::Cs(macro_name));
    assert_eq!(delivered.meaning(), Meaning::Relax);
    assert_eq!(
        delivered.identity(),
        crate::command::CommandIdentity::NoExpandFrozenRelax
    );
    assert_eq!(
        processor.observed_command_spelling(&delivered),
        crate::observation::ObservedToken::ControlSequence("m".into())
    );
    assert_eq!(processor.command.expansion.cumulative_expansions, 1);
}

#[test]
fn expandafter_expands_second_token_before_replaying_first() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let expandafter = universe.intern("expandafter").symbol();
    universe.set_meaning(
        expandafter,
        Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter),
    );
    let macro_name = install_macro(
        &mut universe,
        "m",
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
    );
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(expandafter)),
            traced(Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            }),
            traced(Token::Cs(macro_name)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let (first, second) = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .with_observer(&mut recorder);

        let first = processor
            .get_x_token()
            .expect("expandafter completes")
            .expect("first token");
        let second = processor
            .get_x_token()
            .expect("macro body follows")
            .expect("body token");
        assert_eq!(processor.command.expansion.cumulative_expansions, 2);
        (first, second)
    };
    assert_eq!(
        first.spelling().semantic_token(),
        Token::Char {
            ch: 'a',
            cat: Catcode::Letter
        }
    );
    assert_eq!(
        second.spelling().semantic_token(),
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter
        }
    );
    assert!(recorder.0.iter().any(|observation| {
        matches!(
            observation,
            CommandObservation::Command(delivery)
                if delivery.boundary == CommandDeliveryBoundary::Raw
                    && delivery.command == "expand_after"
                    && delivery.command_operand == Some(0)
        )
    }));
}

#[test]
fn csname_expands_characters_then_injects_a_relaxed_named_control_sequence() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let csname = install_expandable(&mut universe, "csname", ExpandablePrimitive::CsName);
    let endcsname = install_expandable(&mut universe, "endcsname", ExpandablePrimitive::EndCsName);
    let macro_name = install_macro(
        &mut universe,
        "letter",
        Token::Char {
            ch: 'a',
            cat: Catcode::Other,
        },
    );
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(csname)),
            traced(Token::Cs(macro_name)),
            traced(Token::Char {
                ch: 'b',
                cat: Catcode::Letter,
            }),
            traced(Token::Cs(endcsname)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let delivered = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .get_x_token()
            .expect("csname expands")
            .expect("constructed control sequence")
    };

    let Token::Cs(symbol) = delivered.spelling().semantic_token() else {
        panic!("csname must inject a control sequence");
    };
    assert_eq!(universe.meaning(symbol), Meaning::Relax);
    assert_eq!(
        universe.control_sequence_kind(symbol),
        tex_state::interner::ControlSequenceKind::Named
    );
    assert!(matches!(
        universe.origin(delivered.origin()),
        tex_state::provenance::OriginRecord::Synthesized(origin)
            if origin.kind() == SynthesizedOriginKind::Expansion
    ));
    assert_eq!(command.expansion.cumulative_expansions, 2);
}

#[test]
fn csname_recovers_by_backing_up_a_non_character_before_constructing_the_name() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let csname = install_expandable(&mut universe, "csname", ExpandablePrimitive::CsName);
    let endcsname = install_expandable(&mut universe, "endcsname", ExpandablePrimitive::EndCsName);
    let relax = universe.intern("r").symbol();
    universe.set_meaning(relax, Meaning::Relax);
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(csname)),
            traced(Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            }),
            traced(Token::Cs(relax)),
            traced(Token::Cs(endcsname)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    let constructed = processor
        .get_x_token()
        .expect("csname recovery")
        .expect("constructed name");
    let replayed = processor
        .get_x_token()
        .expect("backed up token")
        .expect("relax");
    assert_eq!(constructed.meaning(), Meaning::Relax);
    assert_eq!(replayed.spelling().semantic_token(), Token::Cs(relax));
    assert_eq!(
        processor.command.expansion.pending_diagnostics,
        vec![MISSING_ENDCSNAME_DIAGNOSTIC]
    );
}

#[test]
fn endcsname_is_an_ordinary_loop_boundary_not_an_expandable_dispatch_error() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let endcsname = install_expandable(&mut universe, "endcsname", ExpandablePrimitive::EndCsName);
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![traced(Token::Cs(endcsname))])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    let boundary = processor
        .get_x_token()
        .expect("boundary delivery")
        .expect("endcsname");
    assert_eq!(
        boundary.meaning(),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName)
    );
}

#[test]
fn macro_activations_allocate_nested_invocation_provenance() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let empty = universe.intern_token_list(&[]);
    let definition = universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, empty));
    let mut capabilities = CommandHostCapabilities::default();
    let outer_invocation;
    let inner_invocation;
    {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        );
        processor.push_macro_activation(
            definition,
            OriginId::UNKNOWN,
            MacroArguments::default(),
            empty,
            tex_state::ids::OriginListId::EMPTY,
        );
        outer_invocation = processor
            .command
            .parameters
            .activations
            .last()
            .expect("outer activation")
            .invocation;
        processor.push_macro_activation(
            definition,
            OriginId::UNKNOWN,
            MacroArguments::default(),
            empty,
            tex_state::ids::OriginListId::EMPTY,
        );
        inner_invocation = processor
            .command
            .parameters
            .activations
            .last()
            .expect("inner activation")
            .invocation;
    }

    assert_ne!(outer_invocation, inner_invocation);
    assert_eq!(command.parameters.activations.len(), 2);
    assert_eq!(
        universe.macro_invocation_provenance_stats().invocations(),
        2
    );
}

#[test]
fn meaning_reads_immutable_replacement_after_nested_macro_retirement() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let meaning = install_expandable(&mut universe, "meaning", ExpandablePrimitive::Meaning);
    let empty = universe.intern_token_list(&[]);
    let expanded = universe.intern_token_list(&letters("EXPANDED"));
    let definition = universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, expanded));
    let target = universe.intern("getxresult").symbol();
    universe.set_meaning(
        target,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition,
        },
    );
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(meaning)),
            traced(Token::Cs(target)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::Inserted,
    );
    command.push_macro_activation(
        definition,
        MacroArguments::default(),
        OriginId::UNKNOWN,
        empty,
        OriginListId::EMPTY,
    );
    command.push_macro_activation(
        definition,
        MacroArguments::default(),
        OriginId::UNKNOWN,
        empty,
        OriginListId::EMPTY,
    );

    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
    assert_eq!(rendered(&mut processor), "macro:->EXPANDED");
    assert!(processor.command.parameters.activations.is_empty());
}

#[test]
fn meaning_separates_a_control_word_from_following_letters() {
    let mut universe = Universe::new_with_plain_catcodes();
    let leaf = universe.intern("leaf").symbol();
    let replacement = universe.intern_token_list(&[
        Token::Cs(leaf),
        Token::Char {
            ch: 'N',
            cat: Catcode::Letter,
        },
    ]);
    let empty = universe.intern_token_list(&[]);
    let definition =
        universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, replacement));
    let macro_name = universe.intern("result").symbol();
    universe.set_meaning(
        macro_name,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition,
        },
    );
    let command = {
        let mut state = universe.command_context();
        CurrentCommand::resolve(
            traced(Token::Cs(macro_name)),
            crate::command::DeliveryStamp::new(0, 0, 0),
            None,
            false,
            &mut state,
        )
    };

    assert_eq!(
        meaning_text(&universe.command_context(), &command),
        "macro:->\\leaf N"
    );
}

#[test]
fn meaning_renders_tex82_long_and_outer_macro_command_identity() {
    let mut universe = Universe::new_with_plain_catcodes();
    let empty = universe.intern_token_list(&[]);
    for (index, (flags, expected)) in [
        (MeaningFlags::EMPTY, "macro:->"),
        (MeaningFlags::LONG, "\\long macro:->"),
        (MeaningFlags::OUTER, "\\outer macro:->"),
        (
            MeaningFlags::LONG | MeaningFlags::OUTER,
            "\\long\\outer macro:->",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let definition = universe.intern_macro(MacroMeaning::new(flags, empty, empty));
        let macro_name = universe.intern(&format!("result{index}")).symbol();
        universe.set_meaning(macro_name, Meaning::Macro { flags, definition });
        let command = {
            let mut state = universe.command_context();
            CurrentCommand::resolve(
                traced(Token::Cs(macro_name)),
                crate::command::DeliveryStamp::new(0, 0, 0),
                None,
                false,
                &mut state,
            )
        };

        assert_eq!(
            meaning_text(&universe.command_context(), &command),
            expected
        );
    }
}

#[test]
fn meaning_macro_token_list_distinguishes_words_symbols_spaces_and_active_chars() {
    let mut universe = Universe::new_with_plain_catcodes();
    let word = universe.intern("word").symbol();
    let symbol = universe.intern("!").symbol();
    let active = universe.intern_active_character('~').symbol();
    let empty = universe.intern_token_list(&[]);
    let replacement = universe.intern_token_list(&[
        Token::Cs(word),
        Token::Cs(symbol),
        Token::Cs(active),
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        },
    ]);
    let definition =
        universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, replacement));
    let macro_name = universe.intern("shown").symbol();
    universe.set_meaning(
        macro_name,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition,
        },
    );
    let command = {
        let mut state = universe.command_context();
        CurrentCommand::resolve(
            traced(Token::Cs(macro_name)),
            crate::command::DeliveryStamp::new(0, 0, 0),
            None,
            false,
            &mut state,
        )
    };

    assert_eq!(
        meaning_text(&universe.command_context(), &command),
        "macro:->\\word \\!~ "
    );
}

#[test]
fn character_command_renderer_covers_tex82_print_cmd_chr_table() {
    for (cat, ch, expected) in [
        (Catcode::BeginGroup, '{', "begin-group character {"),
        (Catcode::EndGroup, '}', "end-group character }"),
        (Catcode::MathShift, '$', "math shift character $"),
        (Catcode::AlignmentTab, '&', "alignment tab character &"),
        (Catcode::EndLine, '\r', "end of line character \r"),
        (Catcode::Parameter, '#', "macro parameter character #"),
        (Catcode::Superscript, '^', "superscript character ^"),
        (Catcode::Subscript, '_', "subscript character _"),
        (Catcode::Space, ' ', "blank space  "),
        (Catcode::Letter, 'a', "the letter a"),
        (Catcode::Other, '7', "the character 7"),
        (Catcode::Escape, '\\', "the character \\"),
        (Catcode::Ignored, '\0', "the character \0"),
        (Catcode::Active, '~', "the character ~"),
        (Catcode::Comment, '%', "the character %"),
        (Catcode::Invalid, '\u{7f}', "the character \u{7f}"),
    ] {
        assert_eq!(character_command_text(ch, cat), expected);
    }
}

#[test]
fn meaning_renderer_covers_register_quantity_and_primitive_families() {
    use tex_state::env::banks::IntParam;
    use tex_state::meaning::{InternalInteger, UnexpandablePrimitive};
    use tex_state::page::{PageDimension, PageInteger};

    let mut universe = Universe::new_with_plain_catcodes();
    for (index, meaning, canonical_name, expected) in [
        (0, Meaning::CountRegister(3), "aliascount", "\\count3"),
        (1, Meaning::DimenRegister(4), "aliasdimen", "\\dimen4"),
        (2, Meaning::SkipRegister(5), "aliasskip", "\\skip5"),
        (3, Meaning::MuskipRegister(6), "aliasmuskip", "\\muskip6"),
        (4, Meaning::ToksRegister(7), "aliastoks", "\\toks7"),
        (
            5,
            Meaning::IntParam(IntParam::ESCAPE_CHAR.raw()),
            "escapechar",
            "\\escapechar",
        ),
        (
            6,
            Meaning::InternalInteger(InternalInteger::Badness),
            "badness",
            "\\badness",
        ),
        (
            7,
            Meaning::PageDimension(PageDimension::Goal),
            "pagegoal",
            "\\pagegoal",
        ),
        (
            8,
            Meaning::PageInteger(PageInteger::DeadCycles),
            "deadcycles",
            "\\deadcycles",
        ),
        (
            9,
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Meaning),
            "meaning",
            "\\meaning",
        ),
        (
            10,
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Show),
            "show",
            "\\show",
        ),
    ] {
        universe.register_primitive_meaning(canonical_name, meaning);
        let alias = universe.intern(&format!("alias{index}")).symbol();
        universe.set_meaning(alias, meaning);
        let command = {
            let mut state = universe.command_context();
            CurrentCommand::resolve(
                traced(Token::Cs(alias)),
                crate::command::DeliveryStamp::new(0, 0, 0),
                None,
                false,
                &mut state,
            )
        };
        let rendered = meaning_text(&universe.command_context(), &command);
        assert_eq!(rendered, expected);
        assert!(!rendered.contains("Register("));
        assert!(!rendered.contains("Primitive("));
    }
}

#[test]
fn expandafter_and_noexpand_preserve_canonical_raw_order() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let expandafter = install_expandable(
        &mut universe,
        "expandafter",
        ExpandablePrimitive::ExpandAfter,
    );
    let noexpand = install_expandable(&mut universe, "noexpand", ExpandablePrimitive::NoExpand);
    let macro_name = install_macro(
        &mut universe,
        "m",
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
    );
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(expandafter)),
            traced(Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            }),
            traced(Token::Cs(macro_name)),
            traced(Token::Cs(noexpand)),
            traced(Token::Cs(macro_name)),
            traced(Token::Cs(macro_name)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let delivered = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .with_observer(&mut recorder);
        let mut delivered = Vec::new();
        for _ in 0..4 {
            let command = processor
                .get_x_token()
                .expect("expanded delivery succeeds")
                .expect("planned command is delivered");
            delivered.push((command.spelling().semantic_token(), command.meaning()));
        }
        assert_eq!(processor.command.expansion.cumulative_expansions, 4);
        delivered
    };

    assert_eq!(
        delivered,
        vec![
            (
                Token::Char {
                    ch: 'a',
                    cat: Catcode::Letter,
                },
                Meaning::CharToken {
                    ch: 'a',
                    cat: Catcode::Letter,
                },
            ),
            (
                Token::Char {
                    ch: 'x',
                    cat: Catcode::Letter,
                },
                Meaning::CharToken {
                    ch: 'x',
                    cat: Catcode::Letter,
                },
            ),
            (Token::Cs(macro_name), Meaning::Relax),
            (
                Token::Char {
                    ch: 'x',
                    cat: Catcode::Letter,
                },
                Meaning::CharToken {
                    ch: 'x',
                    cat: Catcode::Letter,
                },
            ),
        ]
    );
    let raw = recorder
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Command(delivery)
                if delivery.boundary == CommandDeliveryBoundary::Raw =>
            {
                Some(delivery.spelling.clone())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        raw,
        vec![
            crate::ObservedToken::ControlSequence("expandafter".into()),
            crate::ObservedToken::Character {
                character: 'a',
                catcode: Catcode::Letter,
            },
            crate::ObservedToken::ControlSequence("m".into()),
            crate::ObservedToken::Character {
                character: 'a',
                catcode: Catcode::Letter,
            },
            crate::ObservedToken::Character {
                character: 'x',
                catcode: Catcode::Letter,
            },
            crate::ObservedToken::ControlSequence("noexpand".into()),
            crate::ObservedToken::ControlSequence("m".into()),
            crate::ObservedToken::ControlSequence("m".into()),
            crate::ObservedToken::ControlSequence("m".into()),
            crate::ObservedToken::Character {
                character: 'x',
                catcode: Catcode::Letter,
            },
        ]
    );
}

#[test]
fn csname_expands_characters_interns_once_and_requires_endcsname() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let csname = install_expandable(&mut universe, "csname", ExpandablePrimitive::CsName);
    let endcsname = install_expandable(&mut universe, "endcsname", ExpandablePrimitive::EndCsName);
    let letter = install_macro(
        &mut universe,
        "letter",
        Token::Char {
            ch: 'a',
            cat: Catcode::Other,
        },
    );
    let relax = universe.intern("relax").symbol();
    universe.set_meaning(relax, Meaning::Relax);
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(csname)),
            traced(Token::Cs(letter)),
            traced(Token::Char {
                ch: 'b',
                cat: Catcode::Letter,
            }),
            traced(Token::Cs(endcsname)),
            traced(Token::Cs(csname)),
            traced(Token::Char {
                ch: 'a',
                cat: Catcode::Other,
            }),
            traced(Token::Char {
                ch: 'b',
                cat: Catcode::Other,
            }),
            traced(Token::Cs(endcsname)),
            traced(Token::Cs(csname)),
            traced(Token::Char {
                ch: 'q',
                cat: Catcode::Letter,
            }),
            traced(Token::Cs(relax)),
            traced(Token::Cs(endcsname)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    let first = processor
        .get_x_token()
        .expect("first csname expands")
        .expect("first name is injected");
    let second = processor
        .get_x_token()
        .expect("second csname expands")
        .expect("second name is injected");
    let (Token::Cs(first_symbol), Token::Cs(second_symbol)) = (
        first.spelling().semantic_token(),
        second.spelling().semantic_token(),
    ) else {
        panic!("csname must inject control-sequence tokens");
    };
    assert_eq!(first_symbol, second_symbol);
    assert_eq!(
        processor.state.known_control_sequence("ab"),
        Some(first_symbol)
    );
    assert_eq!(first.meaning(), Meaning::Relax);
    assert_eq!(second.meaning(), Meaning::Relax);

    let partial = processor
        .get_x_token()
        .expect("missing endcsname recovers")
        .expect("partial name is injected");
    let backed = processor
        .get_x_token()
        .expect("rejected command is replayed")
        .expect("backed relax is live");
    let boundary = processor
        .get_x_token()
        .expect("original boundary remains")
        .expect("endcsname is not swallowed");
    let Token::Cs(partial_symbol) = partial.spelling().semantic_token() else {
        panic!("partial csname must still create a control sequence");
    };
    assert_eq!(
        processor.state.known_control_sequence("q"),
        Some(partial_symbol)
    );
    assert_eq!(partial.meaning(), Meaning::Relax);
    assert_eq!(backed.spelling().semantic_token(), Token::Cs(relax));
    assert_eq!(
        boundary.meaning(),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName)
    );
    assert_eq!(
        processor.command.expansion.pending_diagnostics,
        vec![MISSING_ENDCSNAME_DIAGNOSTIC]
    );
}

#[test]
fn backup_replays_the_exact_delivered_token_above_expansion() {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"\\expandafter A\\m Z".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    install_expandable(
        &mut universe,
        "expandafter",
        ExpandablePrimitive::ExpandAfter,
    );
    install_macro(
        &mut universe,
        "m",
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    let first = processor
        .get_x_token()
        .expect("expandafter completes")
        .expect("first token is replayed");
    let spelling = first.spelling();
    let source_range = first.source_range();
    let source_location = first.source_location();
    let first_stamp = first.delivery_stamp();
    processor
        .back_input(first)
        .expect("exact delivery backs up");

    let replayed = processor
        .get_x_token()
        .expect("backup replays")
        .expect("backed token is live");
    assert_eq!(replayed.spelling(), spelling);
    assert_eq!(replayed.source_range(), source_range);
    assert_eq!(replayed.source_location(), source_location);
    assert_ne!(replayed.delivery_stamp(), first_stamp);
    assert!(replayed.direct_source_provenance().is_none());
    assert_eq!(
        processor
            .get_x_token()
            .expect("expanded second token remains below backup")
            .expect("macro output is live")
            .spelling()
            .semantic_token(),
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        }
    );
    assert_eq!(
        processor
            .get_x_token()
            .expect("source resumes")
            .expect("following source token is live")
            .spelling()
            .semantic_token(),
        Token::Char {
            ch: 'Z',
            cat: Catcode::Letter,
        }
    );
}

#[test]
fn converted_token_lists_classify_spaces_copy_tokens_and_resume_expansion() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let number = install_expandable(&mut universe, "number", ExpandablePrimitive::Number);
    let roman = install_expandable(
        &mut universe,
        "romannumeral",
        ExpandablePrimitive::RomanNumeral,
    );
    let string = install_expandable(&mut universe, "string", ExpandablePrimitive::String);
    let meaning = install_expandable(&mut universe, "meaning", ExpandablePrimitive::Meaning);
    let fontname = install_expandable(&mut universe, "fontname", ExpandablePrimitive::FontName);
    let jobname = install_expandable(&mut universe, "jobname", ExpandablePrimitive::JobName);
    let string_target = universe.intern("target").symbol();
    let empty = universe.intern_token_list(&[]);
    let long_definition =
        universe.intern_macro(MacroMeaning::new(MeaningFlags::LONG, empty, empty));
    let long_macro = universe.intern("longmacro").symbol();
    universe.set_meaning(
        long_macro,
        Meaning::Macro {
            flags: MeaningFlags::LONG,
            definition: long_definition,
        },
    );
    let font = universe.intern("nullfont-id").symbol();
    let identified_font = universe
        .try_copy_font_with_identifier(tex_state::font::NULL_FONT, font)
        .expect("font identity copies");
    universe.set_meaning(font, Meaning::Font(identified_font));
    let null_font_name = universe.font_name(identified_font).to_owned();
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(number)),
            traced(Token::Char {
                ch: '-',
                cat: Catcode::Other,
            }),
            traced(Token::Char {
                ch: '1',
                cat: Catcode::Other,
            }),
            traced(Token::Char {
                ch: '2',
                cat: Catcode::Other,
            }),
            traced(Token::Cs(roman)),
            traced(Token::Char {
                ch: '9',
                cat: Catcode::Other,
            }),
            traced(Token::Cs(string)),
            traced(Token::Cs(string_target)),
            traced(Token::Cs(meaning)),
            traced(Token::Cs(long_macro)),
            traced(Token::Cs(fontname)),
            traced(Token::Cs(font)),
            traced(Token::Cs(jobname)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    capabilities.set_job_name("paper");
    let rendered_tokens = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        let mut rendered_tokens = Vec::new();
        while let Some(delivery) = processor.get_x_token().expect("conversion expands") {
            rendered_tokens.push(delivery.spelling().semantic_token());
        }
        assert_eq!(processor.command.expansion.cumulative_expansions, 6);
        rendered_tokens
    };
    let rendered = rendered_tokens
        .iter()
        .map(|token| match token {
            Token::Char { ch, .. } => *ch,
            _ => panic!("classic conversion output must be characters"),
        })
        .collect::<String>();
    assert_eq!(
        rendered,
        format!("-12ix\\target\\long macro:->{null_font_name}paper")
    );
    assert!(rendered_tokens.iter().any(|token| matches!(
        token,
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        }
    )));
    assert!(rendered_tokens.iter().all(|token| matches!(
        token,
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        } | Token::Char {
            ch: '!'..='~',
            cat: Catcode::Other,
        }
    )));
    let the = install_expandable(&mut universe, "the", ExpandablePrimitive::The);
    let copied_macro = install_macro(
        &mut universe,
        "copiedmacro",
        Token::Char {
            ch: 'Q',
            cat: Catcode::Letter,
        },
    );
    let register = universe.intern("stored").symbol();
    universe.set_meaning(register, Meaning::ToksRegister(4));
    let stored = universe.intern_token_list(&[
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        },
        Token::Cs(copied_macro),
        Token::Char {
            ch: 'L',
            cat: Catcode::Letter,
        },
    ]);
    universe.set_toks(4, stored);
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(the)),
            traced(Token::Cs(font)),
            traced(Token::Cs(the)),
            traced(Token::Cs(register)),
            traced(Token::Char {
                ch: 'Z',
                cat: Catcode::Letter,
            }),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
    let mut copied = Vec::new();
    while let Some(delivery) = processor.get_x_token().expect("copied list expands") {
        copied.push(delivery.spelling().semantic_token());
    }
    assert_eq!(
        copied,
        vec![
            Token::Cs(font),
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Char {
                ch: 'Q',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'L',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'Z',
                cat: Catcode::Letter,
            },
        ]
    );
}
