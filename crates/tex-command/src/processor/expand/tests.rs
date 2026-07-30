use std::sync::Arc;

use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
use tex_state::page::PageMark;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::{Universe, World};

use super::*;
use crate::input::{ReplayTrace, RetirementBehavior};
use crate::observation::{
    CommandDeliveryBoundary, CommandObservation, CommandObserver, DiagnosticArgument,
    InputTransition, ObservedToken,
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

#[test]
fn cyclic_macro_exhausts_shared_command_fuel() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let cycle = universe.intern("cycle").symbol();
    let empty = universe.intern_token_list(&[]);
    let replacement = universe.intern_token_list(&[Token::Cs(cycle)]);
    let definition =
        universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, replacement));
    universe.set_meaning(
        cycle,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition,
        },
    );
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![traced(Token::Cs(cycle))])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut fuel = crate::CommandFuelLedger::new(7).expect("valid test limit");
    let error = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    )
    .with_fuel(fuel.fuel_mut())
    .get_x_token()
    .expect_err("cyclic expansion must terminate inside tex-command");
    assert_eq!(
        error,
        crate::CommandError::FuelExhausted {
            limit: 7,
            burned: 7
        }
    );
    assert_eq!(fuel.burned(), 7);
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
fn frozen_end_template_delivers_endv_fresh_and_after_format_load() {
    // TeX82 §§375, 780: both `endtemplate` control sequences are inaccessible
    // frozen slots. Expanding the first delivers the second as `endv`; format
    // loading must preserve that internal meaning without exposing a named
    // primitive to user input.
    let fresh = Universe::new_with_plain_catcodes();
    assert_eq!(fresh.symbol("endtemplate"), None);
    let format = fresh.dump_format().expect("quiescent format");
    let loaded = Universe::from_format(World::default(), &format).expect("load format");

    for mut universe in [fresh, loaded] {
        assert_eq!(universe.symbol("endtemplate"), None);
        assert_eq!(universe.primitive_meaning("endtemplate"), None);
        let frozen_end_template = universe.command_context().frozen_end_template_token();

        let mut command = CommandState::default();
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![traced(frozen_end_template)])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut runtime = CommandRuntime::default();
        let mut capabilities = CommandHostCapabilities::default();
        let delivered = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .get_x_token()
            .expect("end_template expansion succeeds")
            .expect("frozen endv is delivered");

        assert_eq!(delivered.meaning(), Meaning::EndV);
        assert!(delivered.spelling().semantic_token().is_frozen_endv());
        assert_eq!(universe.symbol("endtemplate"), None);
    }
}

#[test]
fn etex_unexpanded_reenters_the_current_expansion_loop() {
    // e-TeX 2.6 etex.ch §27.465 routes `\unexpanded` through
    // `scan_general_text`, then returns its token list through `the_toks`.
    // Outside an expanded token-list collector, that list is ordinary
    // `ins_list` input and the enclosing `get_x_token` expands it normally.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let unexpanded =
        install_expandable(&mut universe, "unexpanded", ExpandablePrimitive::Unexpanded);
    let payload = install_macro(
        &mut universe,
        "payload",
        Token::Char {
            ch: 'X',
            cat: Catcode::Letter,
        },
    );
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(unexpanded)),
            traced(Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            }),
            traced(Token::Cs(payload)),
            traced(Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            }),
            traced(Token::Cs(payload)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut fuel = crate::CommandFuelLedger::new(32).expect("finite test fuel");
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .with_fuel(fuel.fuel_mut());

    let expanded = processor
        .get_x_token()
        .expect("unexpanded scan succeeds")
        .expect("expanded token is returned");
    assert_eq!(
        expanded.spelling().semantic_token(),
        Token::Char {
            ch: 'X',
            cat: Catcode::Letter,
        }
    );
    assert!(!is_expandable_command(&expanded));
    assert_eq!(rendered(&mut processor), "X");
    assert!(fuel.burned() <= 32);
}

#[test]
fn etex_scantokens_retokenizes_balanced_text_as_nested_lines() {
    // e-TeX 2.6 etex.ch §53a: pseudo_start applies token_show, splits at the
    // live \newlinechar, and reads the result under the live catcode table.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    universe.set_int_param(IntParam::NEWLINE_CHAR, i32::from(b'|'));
    universe.set_catcode('a', Catcode::Other);
    let every_eof = universe.intern_token_list(&[Token::Char {
        ch: 'E',
        cat: Catcode::Letter,
    }]);
    universe.set_tok_param(tex_state::env::banks::TokParam::EVERY_EOF, every_eof);
    let scantokens =
        install_expandable(&mut universe, "scantokens", ExpandablePrimitive::Scantokens);
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(scantokens)),
            traced(Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            }),
            traced(Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            }),
            traced(Token::Char {
                ch: '|',
                cat: Catcode::Other,
            }),
            traced(Token::Char {
                ch: 'b',
                cat: Catcode::Letter,
            }),
            traced(Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            }),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut fuel = crate::CommandFuelLedger::new(64).expect("finite test fuel");
    let mut recorder = Recorder::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .with_fuel(fuel.fuel_mut())
        .with_observer(&mut recorder);
    let mut output = Vec::new();
    while let Some(delivery) = processor.get_x_token().expect("scantokens expands") {
        output.push(delivery.spelling().semantic_token());
    }
    assert_eq!(
        output.first(),
        Some(&Token::Char {
            ch: 'a',
            cat: Catcode::Other,
        })
    );
    assert!(output.iter().any(|token| {
        matches!(
            token,
            Token::Char {
                ch: 'b',
                cat: Catcode::Letter
            }
        )
    }));
    assert_eq!(
        output.last(),
        Some(&Token::Char {
            ch: 'E',
            cat: Catcode::Letter,
        }),
        "\\everyeof must replay after the pseudo-file's final line"
    );
    assert!(fuel.burned() <= 64);
    assert!(
        !recorder
            .0
            .iter()
            .any(|event| matches!(event, CommandObservation::ScannerStatus(_))),
        "e-TeX §53a scan_general_text does not publish TeX82 scan_toks status observations"
    );
    assert_eq!(
        recorder
            .0
            .iter()
            .filter(|event| matches!(
                event,
                CommandObservation::TokenList(record)
                    if record.transition == "complete" && record.purpose == "scantokens"
            ))
            .count(),
        1
    );
    assert!(recorder.0.iter().any(|event| matches!(
        event,
        CommandObservation::Input(crate::InputRecord {
            transition: crate::InputTransition::Push,
            reason: crate::InputReason::Source,
            source_name: Some(crate::SourceNameClass::Terminal),
            ..
        })
    )));
    let generated = recorder
        .0
        .iter()
        .find_map(|observation| match observation {
            CommandObservation::GeneratedSource(record) => Some(record),
            _ => None,
        })
        .expect("scantokens backing is observable before its source push");
    assert_eq!(generated.name, "^^R");
    assert_eq!(generated.source.bytes.as_ref(), b"a\nb\n");
}

#[test]
fn etex_scantokens_pseudo_source_name_tracks_tracing() {
    assert_eq!(scantokens_source_name(0), "^^R");
    assert_eq!(scantokens_source_name(-1), "^^R");
    assert_eq!(scantokens_source_name(1), "^^S");
}

#[test]
fn etex_scantokens_null_everyeof_has_no_token_list_retirement() {
    // e-TeX 2.6 etex.ch §24.362 tests `every_eof<>null` before
    // `begin_token_list`. The default null parameter must therefore return
    // directly from the pseudo-file to its enclosing input level.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let scantokens =
        install_expandable(&mut universe, "scantokens", ExpandablePrimitive::Scantokens);
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(scantokens)),
            traced(Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            }),
            traced(Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            }),
            traced(Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            }),
            traced(Token::Char {
                ch: 'Z',
                cat: Catcode::Letter,
            }),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut fuel = crate::CommandFuelLedger::new(64).expect("finite test fuel");
    let mut recorder = Recorder::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .with_fuel(fuel.fuel_mut())
        .with_observer(&mut recorder);
    let mut output = Vec::new();
    while let Some(delivery) = processor.get_x_token().expect("scantokens expands") {
        output.push(delivery.spelling().semantic_token());
    }

    assert!(output.contains(&Token::Char {
        ch: 'a',
        cat: Catcode::Letter,
    }));
    assert_eq!(
        output.last(),
        Some(&Token::Char {
            ch: 'Z',
            cat: Catcode::Letter,
        })
    );
    assert!(!recorder.0.iter().any(|event| matches!(
        event,
        CommandObservation::Input(crate::InputRecord {
            transition: InputTransition::Retire,
            reason: crate::InputReason::Recovery,
            ..
        })
    )));
}

#[test]
fn etex_scantokens_defined_empty_everyeof_pushes_and_retires_before_close() {
    // e-TeX 2.6 etex.ch §24.362 tests pointer presence, not list length.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    universe.set_tok_param(
        tex_state::env::banks::TokParam::EVERY_EOF,
        TokenListId::EMPTY,
    );
    let scantokens =
        install_expandable(&mut universe, "scantokens", ExpandablePrimitive::Scantokens);
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(scantokens)),
            traced(Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            }),
            traced(Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            }),
            traced(Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            }),
            traced(Token::Char {
                ch: 'Z',
                cat: Catcode::Letter,
            }),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut fuel = crate::CommandFuelLedger::new(64).expect("finite test fuel");
    let mut recorder = Recorder::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .with_fuel(fuel.fuel_mut())
        .with_observer(&mut recorder);
    let mut output = Vec::new();
    while let Some(delivery) = processor.get_x_token().expect("scantokens expands") {
        output.push(delivery.spelling().semantic_token());
    }

    assert_eq!(
        output.last(),
        Some(&Token::Char {
            ch: 'Z',
            cat: Catcode::Letter,
        })
    );
    assert_eq!(
        recorder
            .0
            .iter()
            .filter(|event| matches!(
                event,
                CommandObservation::Input(crate::InputRecord {
                    transition: InputTransition::Retire,
                    reason: crate::InputReason::EveryEof,
                    ..
                })
            ))
            .count(),
        1,
        "the present empty everyeof level retires before pseudo_close resumes Z"
    );
    let every_eof_push = recorder
        .0
        .iter()
        .position(|event| {
            matches!(
                event,
                CommandObservation::Input(crate::InputRecord {
                    transition: InputTransition::Push,
                    reason: crate::InputReason::EveryEof,
                    ..
                })
            )
        })
        .expect("present empty everyeof pushes");
    let source_retirement = recorder
        .0
        .iter()
        .position(|event| {
            matches!(
                event,
                CommandObservation::Input(crate::InputRecord {
                    transition: InputTransition::Retire,
                    reason: crate::InputReason::Source,
                    ..
                })
            )
        })
        .expect("pseudo-file retires");
    assert!(
        every_eof_push < source_retirement,
        "etex.ch §24.362 begins everyeof before §329 retires the pseudo-file"
    );
}

#[test]
fn etex_detokenize_projects_token_show_text_without_expansion() {
    // e-TeX 2.6 etex.ch §53a: scan_general_text is unexpanded, token_show
    // separates a control word, and str_toks makes only spaces category 10.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let detokenize =
        install_expandable(&mut universe, "detokenize", ExpandablePrimitive::Detokenize);
    let payload = install_macro(
        &mut universe,
        "payload",
        Token::Char {
            ch: 'X',
            cat: Catcode::Letter,
        },
    );
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(detokenize)),
            traced(Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            }),
            traced(Token::Cs(payload)),
            traced(Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            }),
            traced(Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            }),
            traced(Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            }),
            traced(Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            }),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut fuel = crate::CommandFuelLedger::new(32).expect("finite test fuel");
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .with_fuel(fuel.fuel_mut());
    let mut output = Vec::new();
    while let Some(delivery) = processor.get_x_token().expect("detokenize expands") {
        output.push(delivery.spelling().semantic_token());
    }
    let text = output
        .iter()
        .map(|token| match token {
            Token::Char { ch, .. } => *ch,
            _ => panic!("detokenize returned a non-character token"),
        })
        .collect::<String>();
    assert_eq!(text, "\\payload ##{}");
    assert!(output.iter().all(|token| matches!(
        token,
        Token::Char {
            ch: ' ',
            cat: Catcode::Space
        } | Token::Char {
            cat: Catcode::Other,
            ..
        }
    )));
    assert!(fuel.burned() <= 32);
}

#[test]
fn etex_detokenize_observes_live_escape_and_control_sequence_kinds() {
    // e-TeX §53a delegates spelling to token_show: active characters have no
    // escape, control symbols have no separator, and \csname\endcsname uses
    // the live escape character.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    universe.set_int_param(IntParam::ESCAPE_CHAR, i32::from(b'!'));
    let detokenize =
        install_expandable(&mut universe, "detokenize", ExpandablePrimitive::Detokenize);
    let active = universe.intern_active_character('~').symbol();
    let symbol = universe.intern("@").symbol();
    let empty = universe.intern("").symbol();
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(detokenize)),
            traced(Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            }),
            traced(Token::Cs(active)),
            traced(Token::Cs(symbol)),
            traced(Token::Cs(empty)),
            traced(Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            }),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
    assert_eq!(rendered(&mut processor), "~!@!csname!endcsname ");
}

#[test]
fn etex_detokenize_the_toks_microfixture_matches_fresh_and_loaded_formats() {
    // e-TeX 2.6 etex.ch §§[25.386], [27.465]: a numbered mark enquiry is
    // still unexpanded general text here, while detokenize's converted
    // character list is returned through `the_toks` and joins the enclosing
    // expanded collector directly. The format round trip is a negative
    // control for primitive-table reconstruction.
    let mut fresh = Universe::new_with_plain_catcodes();
    crate::primitives::install_etex_expandable_primitives(&mut fresh);
    let format = fresh.dump_format().expect("quiescent e-TeX format");
    let mut loaded = Universe::from_format(World::default(), &format).expect("format loads");
    crate::primitives::register_etex_expandable_primitives(&mut loaded);

    for mut universe in [fresh, loaded] {
        let mut command = CommandState::new(crate::CommandProfile::ETEX26);
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(
                    include_bytes!("../fixtures/etex-detokenize-the-toks.tex").as_slice(),
                ),
            ))
            .expect("microfixture registers");
        command.open_registered_source(source).expect("source opens");
        let mut runtime = CommandRuntime::default();
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let result = {
            let mut processor =
                processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
                    .with_observer(&mut recorder);
            processor
                .scan_toks(crate::scan_toks::ScanToksMode::General { expanded: true })
                .expect("expanded collection succeeds")
        };

        let rendered = universe
            .tokens(result.replacement_text.token_list())
            .iter()
            .map(|token| match token {
                Token::Char { ch, .. } => *ch,
                _ => panic!("detokenize must return only character tokens"),
            })
            .collect::<String>();
        assert_eq!(rendered, "\\splitfirstmarks 0");
        let token_lists = recorder
            .0
            .iter()
            .filter_map(|record| match record {
                CommandObservation::TokenList(record) => Some(record),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            token_lists
                .iter()
                .map(|record| (record.transition, record.purpose))
                .collect::<Vec<_>>(),
            [
                ("complete", "detokenize"),
                ("splice", "the_toks"),
                ("complete", "expanded_scan_toks"),
            ]
        );
        assert!(token_lists[0]
            .tokens
            .iter()
            .all(|token| matches!(token, ObservedToken::Character { .. })));
        assert_eq!(token_lists[0].tokens, token_lists[1].tokens);
        assert!(!recorder.0.iter().any(|record| matches!(
            record,
            CommandObservation::Recovery(_)
        )));
    }
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
        "inc.tex",
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
        "inc.tex",
        SourceRegistration::new(
            RegisteredSourceKind::World,
            Arc::<[u8]>::from(b"a\\endinput b\nc".as_slice()),
        ),
    );
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    assert_eq!(chars(&mut processor), "ab z ");
}

#[test]
fn child_endinput_retires_true_to_false_before_multiline_parent_resumes() {
    let mut command = CommandState::default();
    let parent = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"\\input{inc}\np".as_slice()),
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
        "inc.tex",
        SourceRegistration::new(
            RegisteredSourceKind::World,
            Arc::<[u8]>::from(b"c\\endinput\nx".as_slice()),
        ),
    );
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    assert_eq!(chars(&mut processor), "c p ");
    drop(processor);
    assert!(
        !command.input.force_eof,
        "TeX82 §362 clears force_eof before retiring the child"
    );
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
fn etex_mark_class_enquiries_share_extended_register_scan_and_recovery() {
    // e-TeX 2.6 `etex.ch` [26.1178]: all five class enquiries use the same
    // `scan_register_num` as `\marks`, including invalid-to-zero recovery.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let primitives = [
        (
            "topmarks",
            ExpandablePrimitive::TopMarks,
            PageMark::Top,
            'A',
        ),
        (
            "firstmarks",
            ExpandablePrimitive::FirstMarks,
            PageMark::First,
            'B',
        ),
        (
            "botmarks",
            ExpandablePrimitive::BotMarks,
            PageMark::Bot,
            'C',
        ),
        (
            "splitfirstmarks",
            ExpandablePrimitive::SplitFirstMarks,
            PageMark::SplitFirst,
            'D',
        ),
        (
            "splitbotmarks",
            ExpandablePrimitive::SplitBotMarks,
            PageMark::SplitBot,
            'E',
        ),
    ];
    let mut input = Vec::new();
    for (name, primitive, mark, value) in primitives {
        let symbol = install_expandable(&mut universe, name, primitive);
        let tokens = universe.intern_token_list(&[Token::Char {
            ch: value,
            cat: Catcode::Letter,
        }]);
        universe.set_page_mark_class(mark, 32_767, tokens);
        input.push(traced(Token::Cs(symbol)));
        input.extend("32767 ".chars().map(|ch| {
            traced(Token::Char {
                ch,
                cat: if ch == ' ' {
                    Catcode::Space
                } else {
                    Catcode::Other
                },
            })
        }));
    }
    let topmarks = universe.intern("topmarks").symbol();
    let zero = universe.intern_token_list(&[Token::Char {
        ch: 'Z',
        cat: Catcode::Letter,
    }]);
    universe.set_page_mark_class(PageMark::Top, 0, zero);
    input.push(traced(Token::Cs(topmarks)));
    input.extend("-1".chars().map(|ch| {
        traced(Token::Char {
            ch,
            cat: Catcode::Other,
        })
    }));
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(input)),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    assert_eq!(rendered(&mut processor), "ABCDEZ");
    assert_eq!(
        processor.take_restricted_integer_recoveries(),
        vec![crate::RestrictedIntegerRecovery {
            class: crate::RestrictedIntegerClass::Register,
            scanned: -1,
            context: String::new(),
        }]
    );
}

#[test]
fn etex_revision_uses_the_canonical_conversion_token_path() {
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let revision = install_expandable(
        &mut universe,
        "eTeXrevision",
        ExpandablePrimitive::ETeXRevision,
    );
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![traced(Token::Cs(revision))])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    assert_eq!(rendered(&mut processor), ".6");
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
fn next_non_blank_x_token_expands_across_levels_and_preserves_the_stopping_delivery() {
    // TeX82 §§406/1045 require `get_x_token`, not raw delivery: spacer
    // commands produced by a macro are skipped even after its replacement
    // level retires. The first non-spacer remains the exact source-attributed
    // delivery that stopped the loop; it is neither backed up nor rebuilt.
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let macro_name = universe.intern("spaces").symbol();
    let empty = universe.intern_token_list(&[]);
    let replacement = universe.intern_token_list(&[
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        },
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        },
    ]);
    let definition =
        universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, replacement));
    universe.set_meaning(
        macro_name,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition,
        },
    );
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"\\spaces X".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    let delivered = processor
        .next_non_blank_x_token()
        .expect("expanded scan succeeds")
        .expect("source character remains");
    assert_eq!(
        delivered.spelling().semantic_token(),
        Token::Char {
            ch: 'X',
            cat: Catcode::Letter,
        }
    );
    assert!(
        delivered.source_location().is_some(),
        "the stopping source token retains its physical provenance"
    );
    assert_eq!(processor.command.expansion.cumulative_expansions, 1);
}

#[test]
fn next_non_blank_x_token_does_not_skip_relax() {
    // §406 differs deliberately from §404: only spacer commands are skipped.
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let relax = universe.intern("relax").symbol();
    universe.set_meaning(relax, Meaning::Relax);
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"  \\relax X".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    let delivered = processor
        .next_non_blank_x_token()
        .expect("expanded scan succeeds")
        .expect("relax remains");
    assert_eq!(delivered.meaning(), Meaning::Relax);
    assert_eq!(delivered.spelling().semantic_token(), Token::Cs(relax));
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

fn command_and_diagnostic_observations(records: &[CommandObservation]) -> Vec<CommandObservation> {
    records
        .iter()
        .filter(|record| {
            matches!(
                record,
                CommandObservation::Command(command)
                    if matches!(command.command.as_str(), "undefined_cs" | "letter")
            ) || matches!(record, CommandObservation::Diagnostic(_))
        })
        .cloned()
        .collect()
}

/// TeX82 §§365/370: a raw fetch with `no_new_control_sequence` frozen maps an
/// unknown multiletter name to §222's dummy `undefined_control_sequence`.
/// Since §207 puts `undefined_cs` above `max_command`, §380 reports it through
/// §370, substitutes nothing, and resumes the same loop at the following
/// source token exactly once.
#[test]
fn frozen_undefined_control_sequence_reports_then_resumes_source_once() {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"\\never A".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .with_observer(&mut recorder);
        let resumed = processor
            .get_x_token()
            .expect("undefined recovery is finite")
            .expect("following source token resumes");
        assert_eq!(
            resumed.spelling().semantic_token(),
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter
            }
        );
        while let Some(command) = processor.get_x_token().expect("source retires") {
            assert_ne!(
                command.spelling().semantic_token(),
                Token::Char {
                    ch: 'A',
                    cat: Catcode::Letter
                },
                "the following source token is delivered only once"
            );
        }
    }
    assert!(universe.symbol("never").is_none());
    assert_eq!(
        command.take_semantic_diagnostics(),
        [crate::CommandSemanticDiagnostic::UndefinedControlSequence]
    );
    assert!(command.take_semantic_diagnostics().is_empty());

    let records = command_and_diagnostic_observations(&recorder.0);
    assert!(matches!(
        records.as_slice(),
        [
            CommandObservation::Command(raw_undefined),
            CommandObservation::Diagnostic(diagnostic),
            CommandObservation::Command(raw_a),
            CommandObservation::Command(expanded_a),
        ] if raw_undefined.boundary == CommandDeliveryBoundary::Raw
            && raw_undefined.command == "undefined_cs"
            && raw_undefined.spelling == ObservedToken::ControlSequence("^^@".into())
            && diagnostic.diagnostic == "undefined_control_sequence"
            && diagnostic.arguments
                == [DiagnosticArgument::Token(ObservedToken::ControlSequence("^^@".into()))]
            && raw_a.boundary == CommandDeliveryBoundary::Raw
            && raw_a.command == "letter"
            && expanded_a.boundary == CommandDeliveryBoundary::Expanded
            && expanded_a.command == "letter"
    ));
}

/// TeX82 §§370/380 still report and discard `undefined_cs` under the e-TeX
/// profile, but the pinned e-TeX 2.6 observer has no diagnostic seam at §370.
/// Its detached stream therefore retires an exhausted macro immediately after
/// the raw undefined command while the command-owned semantic diagnostic
/// remains available to the executor.
#[test]
fn etex_undefined_recovery_retires_macro_without_observer_diagnostic() {
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"A".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let undefined = universe.intern("undefined").symbol();
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![traced(Token::Cs(undefined))])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::MacroReplacement,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .with_observer(&mut recorder);
        let resumed = processor
            .get_x_token()
            .expect("undefined recovery is finite")
            .expect("enclosing source resumes");
        assert_eq!(
            resumed.spelling().semantic_token(),
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter
            }
        );
    }
    assert_eq!(
        command.take_semantic_diagnostics(),
        [crate::CommandSemanticDiagnostic::UndefinedControlSequence]
    );
    assert!(matches!(
        command_and_diagnostic_observations(&recorder.0).as_slice(),
        [
            CommandObservation::Command(raw_undefined),
            CommandObservation::Command(raw_a),
            CommandObservation::Command(expanded_a),
        ] if raw_undefined.boundary == CommandDeliveryBoundary::Raw
            && raw_undefined.command == "undefined_cs"
            && raw_a.boundary == CommandDeliveryBoundary::Raw
            && raw_a.command == "letter"
            && expanded_a.boundary == CommandDeliveryBoundary::Expanded
            && expanded_a.command == "letter"
    ));
    let undefined_position = recorder
        .0
        .iter()
        .position(|record| {
            matches!(
                record,
                CommandObservation::Command(command) if command.command == "undefined_cs"
            )
        })
        .expect("raw undefined command observed");
    assert!(matches!(
        recorder.0.get(undefined_position + 1),
        Some(CommandObservation::Input(record))
            if record.transition == crate::observation::InputTransition::Retire
                && record.reason == crate::observation::InputReason::Macro
    ));
}

/// Bounded source fixture for the e-TeX 2.6 §370 observer boundary. The raw
/// undefined command is visible, recovery remains semantic, and detached
/// observation resumes at the following expanded token with no diagnostic
/// record inserted between them.
#[test]
fn etex_undefined_semantic_microfixture_omits_observer_diagnostic() {
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(
                include_bytes!("../fixtures/etex-undefined-expansion.tex").as_slice(),
            ),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .with_observer(&mut recorder);
        let resumed = processor
            .get_x_token()
            .expect("undefined recovery is finite")
            .expect("fixture resumes");
        assert_eq!(
            resumed.spelling().semantic_token(),
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter
            }
        );
    }
    assert_eq!(
        command.take_semantic_diagnostics(),
        [crate::CommandSemanticDiagnostic::UndefinedControlSequence]
    );
    assert!(matches!(
        command_and_diagnostic_observations(&recorder.0).as_slice(),
        [
            CommandObservation::Command(raw_undefined),
            CommandObservation::Command(raw_a),
            CommandObservation::Command(expanded_a),
        ] if raw_undefined.boundary == CommandDeliveryBoundary::Raw
            && raw_undefined.command == "undefined_cs"
            && raw_a.boundary == CommandDeliveryBoundary::Raw
            && raw_a.command == "letter"
            && expanded_a.boundary == CommandDeliveryBoundary::Expanded
            && expanded_a.command == "letter"
    ));
}

#[test]
fn undefined_semantic_diagnostic_survives_unobserved_execution_and_snapshot_retry() {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"\\undefined A".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let snapshot = command.snapshot();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();

    let run = |command: &mut CommandState,
               runtime: &mut CommandRuntime,
               universe: &mut Universe,
               capabilities: &mut CommandHostCapabilities| {
        let mut processor = processor(command, runtime, universe, capabilities);
        let resumed = processor
            .get_x_token()
            .expect("undefined recovery is finite")
            .expect("following token resumes");
        assert_eq!(
            resumed.spelling().semantic_token(),
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter,
            }
        );
        drop(processor);
        command.take_semantic_diagnostics()
    };

    assert_eq!(
        run(&mut command, &mut runtime, &mut universe, &mut capabilities),
        [crate::CommandSemanticDiagnostic::UndefinedControlSequence]
    );
    assert!(command.take_semantic_diagnostics().is_empty());

    command.rollback(snapshot).expect("rollback succeeds");
    assert_eq!(
        run(&mut command, &mut runtime, &mut universe, &mut capabilities),
        [crate::CommandSemanticDiagnostic::UndefinedControlSequence],
        "rollback replays the command-owned semantic diagnostic exactly once"
    );
}

/// TeX82 §§370/380 are independent of whether `undefined_cs` came from the
/// frozen dummy or an already interned hash entry. The command snapshot owns
/// the entire recovery episode: retry must reproduce the diagnostic ordering,
/// enclosing-input retirement, and following delivery byte-for-byte.
#[test]
fn interned_undefined_recovery_and_enclosing_resume_replay_after_rollback() {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"A".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let undefined = universe.intern("undefined").symbol();
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![traced(Token::Cs(undefined))])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let expected = command.clone();
    let snapshot = command.snapshot();
    let mut capabilities = CommandHostCapabilities::default();

    let run = |command: &mut CommandState,
               runtime: &mut CommandRuntime,
               universe: &mut Universe,
               capabilities: &mut CommandHostCapabilities| {
        let mut recorder = Recorder::default();
        {
            let mut processor =
                processor(command, runtime, universe, capabilities).with_observer(&mut recorder);
            let resumed = processor
                .get_x_token()
                .expect("undefined recovery is finite")
                .expect("enclosing source resumes");
            assert_eq!(
                resumed.spelling().semantic_token(),
                Token::Char {
                    ch: 'A',
                    cat: Catcode::Letter
                }
            );
            while let Some(command) = processor.get_x_token().expect("source retires") {
                assert_ne!(
                    command.spelling().semantic_token(),
                    Token::Char {
                        ch: 'A',
                        cat: Catcode::Letter
                    },
                    "the enclosing source token is delivered only once"
                );
            }
        }
        recorder.0
    };

    let first = run(&mut command, &mut runtime, &mut universe, &mut capabilities);
    command.rollback(snapshot).expect("rollback succeeds");
    assert_eq!(command, expected);
    let replayed = run(&mut command, &mut runtime, &mut universe, &mut capabilities);
    assert_eq!(replayed, first);

    let records = command_and_diagnostic_observations(&first);
    assert!(matches!(
        records.as_slice(),
        [
            CommandObservation::Command(raw_undefined),
            CommandObservation::Diagnostic(diagnostic),
            CommandObservation::Command(raw_a),
            CommandObservation::Command(expanded_a),
        ] if raw_undefined.boundary == CommandDeliveryBoundary::Raw
            && raw_undefined.command == "undefined_cs"
            && raw_undefined.spelling
                == ObservedToken::ControlSequence("undefined".into())
            && diagnostic.diagnostic == "undefined_control_sequence"
            && diagnostic.arguments == [
                DiagnosticArgument::Token(ObservedToken::ControlSequence("undefined".into()))
            ]
            && raw_a.boundary == CommandDeliveryBoundary::Raw
            && raw_a.command == "letter"
            && expanded_a.boundary == CommandDeliveryBoundary::Expanded
            && expanded_a.command == "letter"
    ));
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

/// TeX82 §379 tests `cur_cmd > max_command`, not merely whether a meaning
/// names an expandable primitive or macro. The `undefined_cs` command is in
/// that range too, so `\noexpand` must replay a newly entered undefined name
/// as the one-shot `relax`/`no_expand_flag` command.
#[test]
fn noexpand_suppresses_an_undefined_control_sequence() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let noexpand = universe.intern("noexpand").symbol();
    universe.set_meaning(
        noexpand,
        Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand),
    );
    let undefined = universe.intern("undefined").symbol();
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(noexpand)),
            traced(Token::Cs(undefined)),
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
        .expect("undefined target");
    assert_eq!(delivered.spelling().semantic_token(), Token::Cs(undefined));
    assert_eq!(delivered.meaning(), Meaning::Relax);
    assert_eq!(
        delivered.identity(),
        crate::command::CommandIdentity::NoExpandFrozenRelax
    );
    assert_eq!(
        processor.observed_command_spelling(&delivered),
        crate::observation::ObservedToken::ControlSequence("undefined".into())
    );
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
fn print_cs_delimits_words_but_not_active_characters_or_control_symbols() {
    // TeX82 §§262–263: `print_cs` and `sprint_cs` share spelling, but only
    // `print_cs` appends a delimiter after a named control word. Meaning does
    // not affect that spelling partition.
    let mut universe = Universe::new_with_plain_catcodes();
    let primitive = universe.intern("relax").symbol();
    let macro_name = universe.intern("macro").symbol();
    let undefined = universe.intern("undefined").symbol();
    let active = universe.intern_active_character('~').symbol();
    let symbol = universe.intern("!").symbol();
    let empty = universe.intern_token_list(&[]);
    let definition = universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, empty));
    universe.set_meaning(primitive, Meaning::Relax);
    universe.set_meaning(
        macro_name,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition,
        },
    );

    for (symbol, expected) in [
        (primitive, "\\relax "),
        (macro_name, "\\macro "),
        (undefined, "\\undefined "),
        (active, "~"),
        (symbol, "\\!"),
    ] {
        assert_eq!(
            print_cs_text(&mut universe.command_context(), symbol),
            expected
        );
    }
}

#[test]
fn character_command_renderer_covers_tex82_print_cmd_chr_table() {
    for (cat, ch, expected) in [
        (Catcode::BeginGroup, '{', "begin-group character {"),
        (Catcode::EndGroup, '}', "end-group character }"),
        (Catcode::MathShift, '$', "math shift character $"),
        (Catcode::AlignmentTab, '&', "alignment tab character &"),
        (Catcode::EndLine, '\r', "\\crcr"),
        (Catcode::Parameter, '#', "macro parameter character #"),
        (Catcode::Superscript, '^', "superscript character ^"),
        (Catcode::Subscript, '_', "subscript character _"),
        (Catcode::Space, ' ', "blank space  "),
        (Catcode::Letter, 'a', "the letter a"),
        (Catcode::Other, '7', "the character 7"),
        (Catcode::Escape, '\\', "[uncommandable character \\]"),
        (Catcode::Ignored, '\0', "[uncommandable character \0]"),
        (Catcode::Active, '~', "[uncommandable character ~]"),
        (Catcode::Comment, '%', "[uncommandable character %]"),
        (
            Catcode::Invalid,
            '\u{7f}',
            "[uncommandable character \u{7f}]",
        ),
    ] {
        assert_eq!(character_command_text(ch, cat), expected);
    }
}

#[test]
fn print_cmd_chr_preserves_delivered_command_operands_and_aliases() {
    use tex_state::font::{FontMetrics, LoadedFont};
    use tex_state::scaled::Scaled;

    let mut universe = Universe::new_with_plain_catcodes();
    universe.register_primitive_meaning(
        "advance",
        Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Advance),
    );
    let scaled_font = universe.intern_font(LoadedFont::new(
        "cmr10",
        "cmr10.tfm",
        [0; 32],
        0,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(12 * Scaled::UNITY),
        vec![Scaled::from_raw(0); 7],
        FontMetrics::default(),
    ));
    let design_font = universe.intern_font(LoadedFont::new(
        "cmtt10",
        "cmtt10.tfm",
        [1; 32],
        0,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(10 * Scaled::UNITY),
        vec![Scaled::from_raw(0); 7],
        FontMetrics::default(),
    ));

    for (index, meaning, expected) in [
        (0, Meaning::CharGiven('A'), "\\char\"41"),
        (1, Meaning::MathCharGiven(0x1234), "\\mathchar\"1234"),
        (2, Meaning::Font(scaled_font), "select font cmr10 at 12.0pt"),
        (3, Meaning::Font(design_font), "select font cmtt10"),
        (4, Meaning::EndV, "end of alignment template"),
        (
            5,
            Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Advance),
            "\\advance",
        ),
    ] {
        let alias = universe.intern(&format!("alias{index}")).symbol();
        universe.set_meaning(alias, meaning);
        let command = {
            let mut state = universe.command_context();
            CurrentCommand::resolve(
                traced(Token::Cs(alias)),
                crate::command::DeliveryStamp::new(0, index, 0),
                None,
                false,
                &mut state,
            )
        };
        assert_eq!(
            print_cmd_chr_text(
                &universe.command_context(),
                PrintCommand::from_current(&command),
            ),
            expected
        );
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

/// TeX82 §§471-472 route `font_name_code` through §577's
/// `scan_font_ident`, so an ordinary font control sequence is converted
/// directly and the enclosing expanded delivery resumes after it once.
#[test]
fn fontname_scans_a_valid_font_identifier() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let fontname = install_expandable(&mut universe, "fontname", ExpandablePrimitive::FontName);
    let identifier = universe.intern("selectedfont").symbol();
    let font = universe
        .try_copy_font_with_identifier(tex_state::font::NULL_FONT, identifier)
        .expect("font identity copies");
    universe.set_meaning(identifier, Meaning::Font(font));
    let expected = universe.font_name(font).to_owned();
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(fontname)),
            traced(Token::Cs(identifier)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
    assert_eq!(rendered(&mut processor), expected);
    assert_eq!(processor.command.expansion.cumulative_expansions, 1);
}

/// TeX82 §577 reports one missing-font-identifier error, backs up an invalid
/// command, and selects `nullfont`. §§467/472 then insert the rendered
/// null-font name before the rejected command is reconsidered. A following
/// macro proves that §380's enclosing expansion loop resumes once rather than
/// starting a second driver.
#[test]
fn fontname_invalid_character_and_control_recover_once_then_resume_expansion() {
    for invalid_control in [false, true] {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let fontname = install_expandable(&mut universe, "fontname", ExpandablePrimitive::FontName);
        let continuation = install_macro(
            &mut universe,
            "continue",
            Token::Char {
                ch: '!',
                cat: Catcode::Other,
            },
        );
        let invalid = if invalid_control {
            let relax = universe.intern("relax").symbol();
            universe.set_meaning(relax, Meaning::Relax);
            Token::Cs(relax)
        } else {
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter,
            }
        };
        let null_font_name = universe.font_name(tex_state::font::NULL_FONT).to_owned();
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![
                traced(Token::Cs(fontname)),
                traced(invalid),
                traced(Token::Cs(continuation)),
            ])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let delivered = {
            let mut processor =
                processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
                    .with_observer(&mut recorder);
            let mut delivered = Vec::new();
            while let Some(command) = processor.get_x_token().expect("recovery is finite") {
                delivered.push(command.spelling().semantic_token());
            }
            assert_eq!(processor.command.expansion.cumulative_expansions, 2);
            delivered
        };

        let mut expected = null_font_name
            .chars()
            .map(|ch| Token::Char {
                ch,
                cat: if ch == ' ' {
                    Catcode::Space
                } else {
                    Catcode::Other
                },
            })
            .collect::<Vec<_>>();
        expected.push(invalid);
        expected.push(Token::Char {
            ch: '!',
            cat: Catcode::Other,
        });
        assert_eq!(delivered, expected);

        let diagnostics = recorder
            .0
            .iter()
            .filter_map(|record| match record {
                CommandObservation::Diagnostic(diagnostic)
                    if diagnostic.diagnostic == "missing_font_identifier" =>
                {
                    Some(diagnostic)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].arguments.is_empty());
    }
}
