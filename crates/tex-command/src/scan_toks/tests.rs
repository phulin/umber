use std::sync::Arc;

use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::UnexpandablePrimitive;
use tex_state::{EffectRecord, PrintSink, Universe};

use super::*;
use crate::input::{
    ReplayTrace, RetirementBehavior, SharedTokenBuffer, TokenBehavior, TokenPayload,
};
use crate::{
    CommandHostCapabilities, CommandHostContext, CommandObservation, CommandObserver,
    CommandRuntime, CommandState, InputTransition, ObservedToken, RegisteredSourceKind,
    SourceRegistration,
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

fn push(command: &mut CommandState, tokens: Vec<Token>) {
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(
            tokens.into_iter().map(traced).collect::<Vec<_>>(),
        )),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
}

fn diagnostic_text(universe: &Universe) -> String {
    universe
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite {
                sink: PrintSink::Terminal | PrintSink::TerminalAndLog | PrintSink::Log,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn general_scan_toks_continues_after_section_403_inserted_left_brace() {
    let mut command = CommandState::default();
    push(
        &mut command,
        vec![
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    let scanned = processor
        .scan_toks(ScanToksMode::General { expanded: false })
        .expect("§403 recovery supplies the required opening brace");
    assert_eq!(
        processor
            .state
            .tokens(scanned.replacement_text.token_list()),
        &[Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        }]
    );
    assert_eq!(
        processor.command.alignment.align_state,
        crate::processor::TOP_LEVEL_ALIGN_STATE
    );
    drop(processor);
    assert!(diagnostic_text(&universe).starts_with("! Missing { inserted."));
}

#[test]
fn eof_recovery_restores_defining_status_before_macro_replacement_completes() {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(&b"{DEF"[..]),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    )
    .with_observer(&mut recorder);

    processor
        .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
        .expect("EOF recovery closes the replacement text");

    let close = recorder
        .0
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::Command(command)
            if matches!(command.spelling, ObservedToken::Character {
                character: '}',
                catcode: Catcode::EndGroup,
            }))
        })
        .expect("inserted right brace is delivered");
    let restored = recorder
        .0
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::ScannerStatus(status)
                if status.from == "defining" && status.to == "normal")
        })
        .expect("defining status restores after the inserted right brace");
    assert!(close < restored);
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

#[test]
fn direct_the_toks_splice_is_unexpanded_and_does_not_balance_the_collector() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let the = install_expandable(&mut universe, "the", ExpandablePrimitive::The);
    let macro_symbol = universe.intern("storedmacro").symbol();
    let register = universe.intern("stored").symbol();
    universe.set_meaning(register, Meaning::ToksRegister(3));
    let stored = universe.intern_token_list(&[
        Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        },
        Token::Cs(macro_symbol),
    ]);
    universe.set_toks(3, stored);
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(the),
            Token::Cs(register),
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
            Token::Char {
                ch: 'z',
                cat: Catcode::Letter,
            },
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    let scanned = processor
        .scan_toks(ScanToksMode::General { expanded: true })
        .expect("scan succeeds");
    assert_eq!(
        processor
            .state
            .tokens(scanned.replacement_text.token_list()),
        &[
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(macro_symbol)
        ]
    );
    assert_eq!(
        processor
            .get_next()
            .expect("trailing token delivers")
            .expect("trailing token exists")
            .spelling()
            .semantic_token(),
        Token::Char {
            ch: 'z',
            cat: Catcode::Letter,
        }
    );
}

#[test]
fn unexpanded_expands_scan_general_text_opener_before_copying_raw_body() {
    // e-TeX 2.6 etex.ch [27.465] implements `\unexpanded` through
    // `scan_general_text`. Its opener uses §403's expanded fetch, so the
    // e-TRIP idiom `\unexpanded\expandafter{...}` reaches the brace before
    // switching to raw balanced-text collection.
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let unexpanded =
        install_expandable(&mut universe, "unexpanded", ExpandablePrimitive::Unexpanded);
    let expandafter = install_expandable(
        &mut universe,
        "expandafter",
        ExpandablePrimitive::ExpandAfter,
    );
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(unexpanded),
            Token::Cs(expandafter),
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: 'X',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    let scanned = processor
        .scan_toks(ScanToksMode::General { expanded: true })
        .expect("expanded opener reaches the raw balanced text");
    assert_eq!(
        processor
            .state
            .tokens(scanned.replacement_text.token_list()),
        &[Token::Char {
            ch: 'X',
            cat: Catcode::Letter,
        }]
    );
}

#[test]
fn unexpanded_observes_the_completed_raw_balanced_text_before_its_direct_splice() {
    // e-TeX 2.6 etex.ch [17.3623--3699, 27.465] makes `scan_general_text`
    // construct the raw balanced list before `the_toks` returns it to the
    // enclosing expanded `scan_toks` collector.
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let unexpanded =
        install_expandable(&mut universe, "unexpanded", ExpandablePrimitive::Unexpanded);
    let raw = universe.intern("raw").symbol();
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(unexpanded),
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(raw),
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    )
    .with_observer(&mut recorder)
    .scan_toks(ScanToksMode::General { expanded: true })
    .expect("expanded token-list scan completes");

    let unexpanded = recorder
        .0
        .iter()
        .position(|event| {
            matches!(
                event,
                CommandObservation::TokenList(record)
                    if record.transition == "complete"
                        && record.purpose == "unexpanded"
                        && record.tokens == [ObservedToken::ControlSequence("raw".into())]
            )
        })
        .expect("raw balanced text completion is observed");
    let enclosing = recorder
        .0
        .iter()
        .position(|event| {
            matches!(
                event,
                CommandObservation::TokenList(record)
                    if record.transition == "complete"
                        && record.purpose == "expanded_scan_toks"
            )
        })
        .expect("enclosing expanded scan completion is observed");
    let splice = recorder
        .0
        .iter()
        .position(|event| {
            matches!(
                event,
                CommandObservation::TokenList(record)
                    if record.transition == "splice"
                        && record.purpose == "the_toks"
                        && record.tokens == [ObservedToken::ControlSequence("raw".into())]
            )
        })
        .expect("raw balanced text is observed at the_toks attachment");
    assert!(unexpanded < splice && splice < enclosing);
}

#[test]
fn direct_the_count_scans_the_eight_bit_index_before_its_terminator_backup() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let the = install_expandable(&mut universe, "the", ExpandablePrimitive::The);
    let count = universe.intern("count").symbol();
    universe.set_meaning(
        count,
        Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Count),
    );
    universe.set_count(21, -83);
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(the),
            Token::Cs(count),
            Token::Char {
                ch: '2',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '1',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: ',',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .with_observer(&mut recorder);

    let scanned = processor
        .scan_toks(ScanToksMode::General { expanded: true })
        .expect("expanded collection succeeds");
    assert_eq!(
        processor
            .state
            .tokens(scanned.replacement_text.token_list()),
        &[
            Token::Char {
                ch: '-',
                cat: Catcode::Other
            },
            Token::Char {
                ch: '8',
                cat: Catcode::Other
            },
            Token::Char {
                ch: '3',
                cat: Catcode::Other
            },
            Token::Char {
                ch: ',',
                cat: Catcode::Other
            },
        ]
    );
    let two = recorder
        .0
        .iter()
        .position(|observation| {
            matches!(
                observation,
                CommandObservation::Command(record)
                    if matches!(record.spelling, ObservedToken::Character { character: '2', .. })
            )
        })
        .expect("index digit is delivered");
    let backup = recorder
        .0
        .iter()
        .position(|observation| {
            matches!(
                observation,
                CommandObservation::Input(record) if record.transition == InputTransition::Backup
            )
        })
        .expect("terminator is backed up");
    assert!(two < backup);
}

#[test]
fn completed_direct_splice_scan_rolls_back_to_the_exact_input_state() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let the = install_expandable(&mut universe, "the", ExpandablePrimitive::The);
    let register = universe.intern("stored").symbol();
    universe.set_meaning(register, Meaning::ToksRegister(3));
    let stored = universe.intern_token_list(&[
        Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        },
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
    ]);
    universe.set_toks(3, stored);
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(the),
            Token::Cs(register),
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let expected = command.clone();
    let snapshot = command.snapshot();
    let mut capabilities = CommandHostCapabilities::default();

    let first = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        let scanned = processor
            .scan_toks(ScanToksMode::General { expanded: true })
            .expect("direct splice scan succeeds");
        processor
            .state
            .tokens(scanned.replacement_text.token_list())
            .to_vec()
    };
    command.rollback(snapshot).expect("rollback succeeds");
    assert_eq!(command, expected);

    let replayed = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        let scanned = processor
            .scan_toks(ScanToksMode::General { expanded: true })
            .expect("rolled-back direct splice scan succeeds");
        processor
            .state
            .tokens(scanned.replacement_text.token_list())
            .to_vec()
    };
    assert_eq!(replayed, first);
}

#[test]
fn empty_direct_splice_is_unobserved_across_rollback_and_retry() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let the = install_expandable(&mut universe, "the", ExpandablePrimitive::The);
    let register = universe.intern("empty").symbol();
    universe.set_meaning(register, Meaning::ToksRegister(3));
    let empty = universe.intern_token_list(&[]);
    universe.set_toks(3, empty);
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(the),
            Token::Cs(register),
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let expected = command.clone();
    let mut snapshot = Some(command.snapshot());
    let mut capabilities = CommandHostCapabilities::default();

    for attempt in 0..2 {
        let mut recorder = Recorder::default();
        let scanned = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder)
        .scan_toks(ScanToksMode::General { expanded: true })
        .expect("empty direct splice scan succeeds");
        assert_eq!(
            universe.tokens(scanned.replacement_text.token_list()),
            &[Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            }],
            "empty §478 result changes no collected tokens"
        );
        assert!(
            !recorder.0.iter().any(|event| matches!(
                event,
                CommandObservation::TokenList(record)
                    if record.transition == "splice" && record.purpose == "the_toks"
            )),
            "empty §478 result publishes no splice observation"
        );
        if attempt == 0 {
            command
                .rollback(snapshot.take().expect("first attempt owns snapshot"))
                .expect("rollback succeeds");
            assert_eq!(command, expected);
        }
    }
}

#[test]
fn macro_definition_converts_parameters_and_preserves_doubled_hashes() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '1',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '1',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    let scanned = processor
        .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
        .expect("definition scans");
    assert_eq!(
        processor.state.tokens(scanned.parameter_text.token_list()),
        &[Token::Param(1)]
    );
    assert_eq!(
        processor
            .state
            .tokens(scanned.replacement_text.token_list()),
        &[
            Token::Param(1),
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
        ]
    );
}

/// TeX82 §477 gates the body's parameter-character rule on `macro_def`
/// alone, never on whether the parameter text declared a parameter, so a
/// parameterless definition still collapses `##` to one token. plain.tex's
/// `\m@ketabbox` (`\ialign\bgroup&\t@bbox##\t@bb@x\crcr`) is the canonical
/// witness.
#[test]
fn parameterless_macro_definition_still_collapses_doubled_hashes() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    let scanned = processor
        .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
        .expect("definition scans");
    assert!(
        processor
            .state
            .tokens(scanned.parameter_text.token_list())
            .is_empty()
    );
    assert_eq!(
        processor
            .state
            .tokens(scanned.replacement_text.token_list()),
        &[Token::Char {
            ch: '#',
            cat: Catcode::Parameter,
        }]
    );
}

/// The same rule is `macro_def`-gated: a general text scan (`\message`,
/// `\toks`, e-TeX `\unexpanded`) stores both parameter characters.
#[test]
fn general_text_keeps_both_parameter_characters() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    let scanned = processor
        .scan_toks(ScanToksMode::General { expanded: false })
        .expect("general text scans");
    assert_eq!(
        processor
            .state
            .tokens(scanned.replacement_text.token_list()),
        &[
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
        ]
    );
}

#[test]
fn macro_definition_hash_brace_reuses_the_left_brace_after_the_body() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '1',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: '[',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '1',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: ']',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    let scanned = processor
        .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
        .expect("definition scans");
    assert_eq!(
        processor.state.tokens(scanned.parameter_text.token_list()),
        &[
            Token::Param(1),
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
        ]
    );
    assert_eq!(
        processor
            .state
            .tokens(scanned.replacement_text.token_list()),
        &[
            Token::Char {
                ch: '[',
                cat: Catcode::Other,
            },
            Token::Param(1),
            Token::Char {
                ch: ']',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
        ]
    );
}

#[test]
fn expanded_collection_expands_a_macro_one_step_at_a_time() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let macro_symbol = universe.intern("m").symbol();
    let empty = universe.intern_token_list(&[]);
    let replacement = universe.intern_token_list(&[Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    }]);
    let definition =
        universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, replacement));
    universe.set_meaning(
        macro_symbol,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition,
        },
    );
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(macro_symbol),
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
    let scanned = processor
        .scan_toks(ScanToksMode::General { expanded: true })
        .expect("expanded scan succeeds");
    assert_eq!(
        processor
            .state
            .tokens(scanned.replacement_text.token_list()),
        &[Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        }]
    );
}

#[test]
fn scan_toks_all_parameter_number_success_and_diagnostic_boundaries() {
    for count in 0_u8..=9 {
        let mut tokens = Vec::new();
        for number in 1..=count {
            tokens.push(Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            });
            tokens.push(Token::Char {
                ch: char::from(b'0' + number),
                cat: Catcode::Other,
            });
        }
        tokens.extend([
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ]);
        let mut command = CommandState::default();
        push(&mut command, tokens);
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        let scanned = processor
            .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
            .expect("parameter matrix scans");
        let expected = (1..=count).map(Token::Param).collect::<Vec<_>>();
        assert_eq!(
            processor.state.tokens(scanned.parameter_text.token_list()),
            expected,
            "parameter count {count}"
        );
        assert!(!scanned.malformed_parameter);
    }

    for (tokens, expected_parameters) in [
        (
            vec![
                Token::Char {
                    ch: '#',
                    cat: Catcode::Parameter,
                },
                Token::Char {
                    ch: '2',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
            ],
            vec![
                Token::Param(1),
                Token::Char {
                    ch: '2',
                    cat: Catcode::Other,
                },
            ],
        ),
        (
            {
                let mut tokens = Vec::new();
                for number in 1_u8..=9 {
                    tokens.push(Token::Char {
                        ch: '#',
                        cat: Catcode::Parameter,
                    });
                    tokens.push(Token::Char {
                        ch: char::from(b'0' + number),
                        cat: Catcode::Other,
                    });
                }
                tokens.extend([
                    Token::Char {
                        ch: '#',
                        cat: Catcode::Parameter,
                    },
                    Token::Char {
                        ch: '0',
                        cat: Catcode::Other,
                    },
                    Token::Char {
                        ch: '{',
                        cat: Catcode::BeginGroup,
                    },
                    Token::Char {
                        ch: '}',
                        cat: Catcode::EndGroup,
                    },
                ]);
                tokens
            },
            { (1_u8..=9).map(Token::Param).collect::<Vec<_>>() },
        ),
    ] {
        let mut command = CommandState::default();
        push(&mut command, tokens);
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        let scanned = processor
            .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
            .expect("malformed parameter text recovers");
        assert!(scanned.malformed_parameter);
        assert_eq!(
            processor.state.tokens(scanned.parameter_text.token_list()),
            expected_parameters
        );
    }

    let mut command = CommandState::default();
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
    let scanned = processor
        .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
        .expect("hash-brace definition scans");
    assert_eq!(
        processor.state.tokens(scanned.parameter_text.token_list()),
        &[Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        }]
    );
    assert_eq!(
        processor
            .state
            .tokens(scanned.replacement_text.token_list()),
        &[
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
        ]
    );
}

#[test]
fn scan_toks_raw_expanded_nested_brace_illegal_hash_and_missing_brace_matrix() {
    let mut universe = Universe::new_with_plain_catcodes();
    let macro_symbol = universe.intern("m").symbol();
    let empty = universe.intern_token_list(&[]);
    let replacement = universe.intern_token_list(&[Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    }]);
    universe.set_macro_meaning(
        macro_symbol,
        MacroMeaning::new(MeaningFlags::EMPTY, empty, replacement),
    );
    for (expanded, expected) in [
        (false, vec![Token::Cs(macro_symbol)]),
        (
            true,
            vec![Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            }],
        ),
    ] {
        let mut command = CommandState::default();
        push(
            &mut command,
            vec![
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Cs(macro_symbol),
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
            ],
        );
        let mut runtime = CommandRuntime::default();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        let scanned = processor
            .scan_toks(ScanToksMode::General { expanded })
            .expect("raw/expanded collection scans");
        assert_eq!(
            processor
                .state
                .tokens(scanned.replacement_text.token_list()),
            expected
        );
    }

    let mut command = CommandState::default();
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: 'n',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut runtime = CommandRuntime::default();
    let mut capabilities = CommandHostCapabilities::default();
    let mut nested_processor =
        processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
    let nested = nested_processor
        .scan_toks(ScanToksMode::General { expanded: false })
        .expect("nested raw collection scans");
    assert_eq!(
        nested_processor
            .state
            .tokens(nested.replacement_text.token_list()),
        &[
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: 'n',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ]
    );

    let mut command = CommandState::default();
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '1',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '2',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut hashes_processor =
        processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
    let hashes = hashes_processor
        .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
        .expect("hash recovery scans");
    assert_eq!(
        hashes_processor
            .state
            .tokens(hashes.replacement_text.token_list()),
        &[
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '2',
                cat: Catcode::Other,
            },
        ]
    );

    let mut command = CommandState::default();
    push(
        &mut command,
        vec![Token::Char {
            ch: 'z',
            cat: Catcode::Letter,
        }],
    );
    let mut missing_processor =
        processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
    let recovered = missing_processor
        .scan_toks(ScanToksMode::General { expanded: false })
        .expect("§403 and runaway recovery complete the token list");
    assert_eq!(
        missing_processor
            .state
            .tokens(recovered.replacement_text.token_list()),
        &[Token::Char {
            ch: 'z',
            cat: Catcode::Letter,
        }],
        "the backed-up offender is the first token of the inserted group"
    );

    let the = install_expandable(&mut universe, "the-matrix", ExpandablePrimitive::The);
    let register = universe.intern("matrix-toks").symbol();
    let stored = universe.intern_token_list(&[Token::Cs(macro_symbol)]);
    universe.set_toks(5, stored);
    universe.set_meaning(register, Meaning::ToksRegister(5));
    let mut command = CommandState::default();
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(the),
            Token::Cs(register),
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut direct_processor =
        processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
    let direct = direct_processor
        .scan_toks(ScanToksMode::General { expanded: true })
        .expect("direct the splice scans");
    assert_eq!(
        direct_processor
            .state
            .tokens(direct.replacement_text.token_list()),
        &[Token::Cs(macro_symbol)],
        "direct the output is not recursively expanded"
    );
}

#[test]
fn scan_toks_all_scanner_status_outer_and_eof_recovery() {
    for (mode, active, purpose) in [
        (
            ScanToksMode::MacroDefinition { expanded: false },
            "defining",
            "macro_replacement",
        ),
        (
            ScanToksMode::General { expanded: false },
            "absorbing",
            "scan_toks",
        ),
    ] {
        let mut command = CommandState::default();
        push(
            &mut command,
            vec![
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
            ],
        );
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder)
        .scan_toks(mode)
        .expect("status-scoped scan completes");
        assert!(recorder.0.iter().any(|event| matches!(
            event,
            CommandObservation::ScannerStatus(status)
                if status.from == "normal" && status.to == active
        )));
        assert!(recorder.0.iter().any(|event| matches!(
            event,
            CommandObservation::ScannerStatus(status)
                if status.from == active && status.to == "normal"
        )));
        assert!(recorder.0.iter().any(|event| matches!(
            event,
            CommandObservation::TokenList(record) if record.purpose == purpose
        )));
    }

    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let outer = universe.intern("outer-matrix").symbol();
    let empty = universe.intern_token_list(&[]);
    universe.set_macro_meaning(outer, MacroMeaning::new(MeaningFlags::OUTER, empty, empty));
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(outer),
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
    let recovered = processor
        .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
        .expect("outer validity inserts a right brace");
    assert_eq!(
        processor
            .state
            .tokens(recovered.replacement_text.token_list()),
        &[Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        }],
        "check_outer_validity substitutes the forbidden delivery by its recovery space"
    );
    assert_eq!(
        processor
            .get_token()
            .expect("outer token delivers")
            .expect("outer token remains")
            .control_sequence(),
        Some(outer)
    );

    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(&b"{EOF"[..]),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut recorder = Recorder::default();
    let scanned = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    )
    .with_observer(&mut recorder)
    .scan_toks(ScanToksMode::General { expanded: false })
    .expect("EOF recovery inserts a right brace");
    assert_eq!(
        universe.tokens(scanned.replacement_text.token_list()),
        &[
            Token::Char {
                ch: 'E',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'O',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'F',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
        ]
    );
    assert!(recorder.0.iter().any(|event| matches!(
        event,
        CommandObservation::ScannerStatus(status)
            if status.from == "absorbing" && status.to == "normal"
    )));
}

#[test]
fn expanded_scan_toks_resumes_after_outer_token_aborts_macro_argument() {
    // TeX82 §394 returns from a macro call when §23 changes `long_state` to
    // `outer_call` and inserts frozen `\par`. The enclosing §380
    // get_x_token loop must resume; an expanded scan_toks collector is one
    // such loop and must not surface the internal matcher abort.
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    universe.install_primitive_meaning(
        "par",
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par),
    );
    let caller = universe.intern("caller").symbol();
    let parameter = universe.intern_token_list(&[Token::Param(1)]);
    let empty = universe.intern_token_list(&[]);
    universe.set_macro_meaning(
        caller,
        MacroMeaning::new(MeaningFlags::EMPTY, parameter, empty),
    );
    let outer = universe.intern("outer").symbol();
    universe.set_macro_meaning(outer, MacroMeaning::new(MeaningFlags::OUTER, empty, empty));
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(caller),
            Token::Cs(outer),
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );

    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
    let recovered = processor
        .scan_toks(ScanToksMode::General { expanded: true })
        .expect("§394 outer recovery resumes expanded token collection");

    assert_eq!(
        processor
            .state
            .tokens(recovered.replacement_text.token_list()),
        &[Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        }]
    );
    assert_eq!(
        processor
            .get_token()
            .expect("backed outer token delivers")
            .expect("outer token remains")
            .control_sequence(),
        Some(outer)
    );
}

#[test]
fn expanded_scan_toks_outer_abort_reinstates_saved_collector_status() {
    // TeX82 §§23, 394, and 400: nested outer-token recovery can leave
    // `scanner_status := normal` as the abort unwinds, but scan_toks still
    // owns the saved absorbing episode that must govern backed input.
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
    let collector = ScannerStatus::Absorbing(AbsorbingContext {
        owner: None,
        builder: TokenBuilderId(17),
        warning: ScannerWarning(17),
    });
    processor.command.begin_scanner_status(collector.clone());
    processor
        .command
        .begin_scanner_status(ScannerStatus::Normal);

    processor.restore_collector_status_after_outer_abort(&collector);

    assert_eq!(processor.command.scanner.status(), &collector);
}

#[test]
fn tex82_expansion_macros_observes_raw_expanded_and_direct_splice_scan_toks() {
    let mut universe = Universe::new_with_plain_catcodes();
    let macro_symbol = universe.intern("observed-macro").symbol();
    let empty = universe.intern_token_list(&[]);
    let replacement = universe.intern_token_list(&[Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    }]);
    universe.set_macro_meaning(
        macro_symbol,
        MacroMeaning::new(MeaningFlags::EMPTY, empty, replacement),
    );

    let raw_events = {
        let mut command = CommandState::default();
        push(
            &mut command,
            vec![
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Cs(macro_symbol),
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
            ],
        );
        let mut runtime = CommandRuntime::default();
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let scanned = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder)
        .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
        .expect("ordinary definition scans");
        assert_eq!(
            universe.tokens(scanned.replacement_text.token_list()),
            &[Token::Cs(macro_symbol)]
        );
        recorder.0
    };
    let raw_enter = raw_events
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::ScannerStatus(status)
            if status.from == "normal" && status.to == "defining")
        })
        .expect("definition status begins");
    let raw_restore = raw_events
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::ScannerStatus(status)
            if status.from == "defining" && status.to == "normal")
        })
        .expect("definition status restores");
    let raw_complete = raw_events
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::TokenList(record)
            if record.transition == "complete"
                && record.purpose == "macro_replacement"
                && record.tokens == [ObservedToken::ControlSequence("observed-macro".into())])
        })
        .expect("ordinary definition result is observed");
    assert!(raw_enter < raw_restore && raw_restore < raw_complete);

    let expanded_events = {
        let mut command = CommandState::default();
        push(
            &mut command,
            vec![
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Cs(macro_symbol),
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
            ],
        );
        let mut runtime = CommandRuntime::default();
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder)
        .scan_toks(ScanToksMode::General { expanded: true })
        .expect("expanded definition scans");
        recorder.0
    };
    assert!(expanded_events.iter().any(|event| matches!(
        event,
        CommandObservation::TokenList(record)
            if record.transition == "complete"
                && record.purpose == "expanded_scan_toks"
                && record.tokens == [ObservedToken::Character {
                    character: 'x',
                    catcode: Catcode::Letter,
                }]
    )));

    let direct_events = {
        let mut command = CommandState::default();
        let the = install_expandable(&mut universe, "the-observed", ExpandablePrimitive::The);
        let register = universe.intern("observed-register").symbol();
        let stored = universe.intern_token_list(&[Token::Cs(macro_symbol)]);
        universe.set_toks(5, stored);
        universe.set_meaning(register, Meaning::ToksRegister(5));
        push(
            &mut command,
            vec![
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Cs(the),
                Token::Cs(register),
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
            ],
        );
        let mut runtime = CommandRuntime::default();
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let scanned = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder)
        .scan_toks(ScanToksMode::General { expanded: true })
        .expect("direct the splice scans");
        assert_eq!(
            universe.tokens(scanned.replacement_text.token_list()),
            &[Token::Cs(macro_symbol)],
            "the_toks output is copied without recursive expansion"
        );
        recorder.0
    };
    let splice = direct_events
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::TokenList(record)
            if record.transition == "splice"
                && record.purpose == "the_toks"
                && record.tokens == [ObservedToken::ControlSequence("observed-macro".into())])
        })
        .expect("direct token-list splice is observed");
    let restore = direct_events
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::ScannerStatus(status)
            if status.from == "absorbing" && status.to == "normal")
        })
        .expect("absorbing status restores");
    let complete = direct_events
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::TokenList(record)
            if record.transition == "complete"
                && record.purpose == "expanded_scan_toks"
                && record.tokens == [ObservedToken::ControlSequence("observed-macro".into())])
        })
        .expect("completed direct-splice result is observed");
    assert!(splice < restore && restore < complete);
}

/// TeX82 §403 opens with §404's "Get the next non-blank non-relax
/// non-call token", so a `\relax` before a mandatory `{` is skipped
/// rather than treated as the missing brace.
///
/// §403 states the rule in prose too: "\TeX\ allows \relax to appear
/// before the left_brace". Skipping only spaces made every mandatory
/// group that a `\relax` guards -- the plain-TeX idiom for stopping an
/// unwanted lookahead -- take §403's `back_error` recovery instead
/// (umber2-johp.209).
#[test]
fn a_mandatory_left_brace_scan_skips_relax_as_well_as_spaces() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let relax = universe.intern("relax").symbol();
    universe.set_meaning(relax, Meaning::Relax);
    push(
        &mut command,
        vec![
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Cs(relax),
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Cs(relax),
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
    let scanned = processor
        .scan_toks(ScanToksMode::General { expanded: false })
        .expect("§404 skips the guarding `\\relax`");
    assert_eq!(
        processor
            .state
            .tokens(scanned.replacement_text.token_list()),
        &[Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        }]
    );
}

fn read_stream(universe: &mut Universe, bytes: &[u8]) -> tex_state::world::StreamSlot {
    universe
        .world_mut()
        .set_memory_file("stream.tex", bytes.to_vec())
        .expect("memory world accepts a seeded file");
    let slot = tex_state::world::StreamSlot::new(1);
    universe
        .world_mut()
        .open_in(slot, "stream.tex")
        .expect("stream opens");
    slot
}

fn read_text(processor: &CommandProcessor<'_>, list: &TracedTokenList) -> String {
    processor
        .state
        .tokens(list.token_list())
        .iter()
        .map(|token| match token {
            Token::Char { ch, .. } => *ch,
            _ => '\u{0}',
        })
        .collect()
}

#[test]
fn readline_exact_bytes_nested_in_scantokens_replay_after_rollback() {
    // e-TeX 2.6 etex.ch §53a and §53c retain TeX's eight-bit character
    // domain: `\readline` assigns catcode 12 without requiring the byte to be
    // a Unicode-domain scalar. Its one-line pseudo-file must then retire back
    // to the enclosing `\scantokens` pseudo-file.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    universe.set_int_param(tex_state::env::banks::IntParam::END_LINE_CHAR, -1);
    let empty = TracedTokenList::synthetic(universe.intern_token_list(&[]));
    let scantokens = command
        .open_scantokens(
            SourceRegistration::new(RegisteredSourceKind::Generated, b"q\n".to_vec()),
            empty,
        )
        .expect("scantokens pseudo-file opens");
    let expected = command.clone();
    let mut snapshot = Some(command.snapshot());
    let mut first = None;
    let mut capabilities = CommandHostCapabilities::default();

    for _attempt in 0..2 {
        let mut fuel = crate::CommandFuel::new(16).expect("finite test fuel");
        let collected = {
            let mut processor =
                processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
                    .with_fuel(&mut fuel);
            let line = processor
                .command
                .open_read_line(
                    SourceRegistration::new(RegisteredSourceKind::Generated, vec![0xff]),
                    crate::input::SourceNameClass::ReadStream(1),
                )
                .expect("readline pseudo-file opens");
            let mut tokens = Vec::new();
            processor
                .collect_read_line_verbatim(line, &mut tokens)
                .expect("exact-byte readline collects");
            assert_eq!(
                processor.command.top_input_level_identity(),
                Some(scantokens),
                "readline retirement resumes the enclosing scantokens source"
            );
            tokens
                .into_iter()
                .map(TracedTokenWord::semantic_token)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            collected,
            [Token::Char {
                ch: '\u{ff}',
                cat: Catcode::Other,
            }]
        );
        assert!(fuel.burned() <= 16);
        if let Some(previous) = &first {
            assert_eq!(&collected, previous, "rollback replays the exact byte");
        } else {
            first = Some(collected);
            command
                .rollback(snapshot.take().expect("first attempt owns snapshot"))
                .expect("rollback succeeds");
            assert_eq!(command, expected);
        }
    }
}

#[test]
fn read_toks_collects_balanced_multiline_input_and_appends_one_eof_line() {
    // TeX82 §482: `repeat <input and store one line> until
    // align_state=1000000`, so an unmatched `{` continues onto the next line.
    // §486 closes the stream at end of file and appends one empty line, which
    // §483 tokenizes into the `\par` an active `\endlinechar` produces.
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let slot = read_stream(&mut universe, b"{one\ntwo}\n");
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
    let target = processor.state.intern_control_sequence("line");

    let list = processor
        .read_toks(1, target, false)
        .expect("read collects");

    // The trailing space is line two's own `\endlinechar`, which
    // §483 stores in `buffer[limit]` before tokenizing the line.
    assert_eq!(read_text(&processor, &list), "{one two} ");
    // §482 restores `align_state`, so the collection leaves no alignment
    // state behind for the caller.
    assert_eq!(
        processor.command.alignment.align_state,
        crate::processor::alignment::TOP_LEVEL_ALIGN_STATE
    );
    // §486 appends its empty line only when `input_ln` actually fails, so
    // a `\read` that balanced on the file's last line leaves the stream
    // open for the next one.
    assert!(!processor.state.read_stream_at_eof(slot));

    let second = processor
        .read_toks(1, target, false)
        .expect("read collects the appended empty line");
    // §486: the stream closes and one empty line is appended. §483 still
    // tokenizes it, and an empty line in `state=new_line` is §351's `\par`.
    let par = processor.state.intern_control_sequence("par");
    assert_eq!(
        processor.state.tokens(second.token_list()),
        [Token::Cs(par)]
    );
    assert!(processor.state.read_stream_at_eof(slot));
}

#[test]
fn read_toks_reads_the_terminal_for_a_closed_or_out_of_range_stream() {
    // TeX82 §482: `if (n<0)or(n>15) then m:=16 else m:=n`. Stream 16 is never
    // open, so §483's `read_open[m]=closed` selects §484's terminal branch
    // for every out-of-range number, and for an in-range stream nobody
    // opened. §484 prompts once and then sets `n` negative, so a second line
    // is read with `prompt_input("")`.
    for stream in [-1_i32, 99, 3] {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        universe.set_interaction_mode(tex_state::InteractionMode::ErrorStop);
        for line in ["{first", "second}"] {
            universe
                .world_mut()
                .push_memory_terminal_line(line)
                .expect("terminal input registers");
        }
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        let target = processor.state.intern_control_sequence("line");

        let list = processor
            .read_toks(stream, target, false)
            .expect("terminal read collects");

        assert_eq!(read_text(&processor, &list), "{first second} ", "{stream}");
    }
}

#[test]
fn read_toks_disables_alignment_delimiters_and_restores_scanner_state() {
    // §482: `s:=align_state; align_state:=1000000` for the collection's whole
    // duration, so an alignment tab in the line is stored as an ordinary
    // token instead of ending a cell, and `align_state` and `scanner_status`
    // are both returned to what the caller had.
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let slot = read_stream(&mut universe, b"a&b\n");
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
    let target = processor.state.intern_control_sequence("line");

    let list = processor
        .read_toks(1, target, false)
        .expect("read collects");

    assert_eq!(read_text(&processor, &list), "a&b ");
    assert_eq!(
        processor.command.alignment.align_state,
        crate::processor::alignment::TOP_LEVEL_ALIGN_STATE
    );
    assert!(matches!(
        processor.command.scanner.status(),
        crate::processor::status::ScannerStatus::Normal
    ));
    assert!(!processor.state.read_stream_at_eof(slot));
}
