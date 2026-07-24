use std::sync::Arc;

use tex_state::Universe;
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{Meaning, MeaningFlags};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::*;
use crate::input::{
    ReplayTrace, RetirementBehavior, SharedTokenBuffer, TokenBehavior, TokenPayload,
};
use crate::{
    CommandHostCapabilities, CommandHostContext, CommandObservation, CommandObserver,
    CommandRuntime, CommandState, RegisteredSourceKind, SourceRegistration,
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

fn push(command: &mut CommandState, tokens: impl IntoIterator<Item = Token>) {
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(
            tokens.into_iter().map(traced).collect::<Vec<_>>(),
        )),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
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

#[test]
fn balanced_text_and_macro_definition_freeze_typed_lists_with_provenance() {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"{xy}".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();
    let snapshot = command.snapshot();
    let balanced = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .scan_balanced_text(false)
            .expect("balanced text scans")
    };
    let provenance = balanced.provenance;
    assert_eq!(
        universe.tokens(balanced.tokens.token_list()),
        &[
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter
            },
            Token::Char {
                ch: 'y',
                cat: Catcode::Letter
            }
        ]
    );
    command
        .rollback(snapshot)
        .expect("balanced scan rolls back exactly");
    let replayed = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .scan_balanced_text(false)
            .expect("balanced replay scans")
    };
    assert_eq!(replayed.provenance, provenance);

    push(
        &mut command,
        [
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
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let definition = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .scan_macro_definition(false)
            .expect("definition scans")
    };
    assert_eq!(
        universe.tokens(definition.parameter_text.token_list()),
        &[Token::Param(1)]
    );
    assert_eq!(
        universe.tokens(definition.replacement_text.token_list()),
        &[Token::Param(1)]
    );
}

#[test]
fn balanced_text_enters_absorbing_before_its_opening_brace() {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"{x}".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();

    processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .with_observer(&mut recorder)
        .scan_balanced_text(true)
        .expect("balanced text scans");

    assert!(matches!(
        recorder.0.as_slice(),
        [
            CommandObservation::ScannerStatus(status),
            CommandObservation::Command(opening),
            ..
        ] if status.from.starts_with("Normal")
            && status.to.starts_with("Absorbing")
            && matches!(opening.spelling, crate::ObservedToken::Character {
                character: '{', catcode: Catcode::BeginGroup
            })
    ));
}

#[test]
fn expanded_balanced_text_uses_canonical_macro_argument_matching() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let macro_name = universe.intern("arg").symbol();
    let parameters = universe.intern_token_list(&[Token::Param(1)]);
    let replacement = universe.intern_token_list(&[Token::Param(1)]);
    let definition = universe.intern_macro(MacroMeaning::new(
        MeaningFlags::EMPTY,
        parameters,
        replacement,
    ));
    universe.set_meaning(
        macro_name,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition,
        },
    );
    push(
        &mut command,
        [
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(macro_name),
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: 'q',
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
    let scanned = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .scan_balanced_text(true)
            .expect("macro argument expands")
    };
    assert_eq!(
        universe.tokens(scanned.tokens.token_list()),
        &[Token::Char {
            ch: 'q',
            cat: Catcode::Letter
        }]
    );
}

#[test]
fn rule_spec_scans_expanded_keywords_and_dimensions() {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"width1pt height2pt depth0pt!".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();
    let spec = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .scan_rule_spec(UnexpandablePrimitive::VRule)
        .expect("rule spec scans");

    assert_eq!(spec.width.map(Scaled::raw), Some(Scaled::UNITY));
    assert_eq!(spec.height.map(Scaled::raw), Some(2 * Scaled::UNITY));
    assert_eq!(spec.depth.map(Scaled::raw), Some(0));
    let terminator = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .get_x_token()
        .expect("terminator delivers")
        .expect("terminator exists");
    assert!(matches!(
        terminator.meaning(),
        Meaning::CharToken { ch: '!', .. }
    ));
}

#[test]
fn rule_spec_starts_v_template_when_scalar_lookahead_hits_cell_delimiters() {
    for (name, primitive, expected) in [
        ("tab", None, crate::AlignmentCellDelimiter::Tab),
        (
            "span",
            Some(UnexpandablePrimitive::Span),
            crate::AlignmentCellDelimiter::Span,
        ),
        (
            "cr",
            Some(UnexpandablePrimitive::Cr),
            crate::AlignmentCellDelimiter::Row,
        ),
    ] {
        let mut command = CommandState::default();
        let alignment = crate::AlignmentIdentity::new(1);
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new();
        let mut capabilities = CommandHostCapabilities::default();
        let delimiter = if let Some(primitive) = primitive {
            let symbol = universe.intern(name).symbol();
            universe.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
            Token::Cs(symbol)
        } else {
            Token::Char {
                ch: '&',
                cat: Catcode::AlignmentTab,
            }
        };
        let v_template =
            tex_state::input::TracedTokenList::synthetic(universe.intern_token_list(&[
                Token::Char {
                    ch: 'v',
                    cat: Catcode::Letter,
                },
            ]));
        command.begin_alignment(alignment);
        command
            .begin_alignment_cell(
                alignment,
                crate::AlignmentCellTemplates {
                    u_template: None,
                    v_template,
                },
            )
            .expect("cell begins");
        command
            .install_alignment_cell_template(alignment)
            .expect("omit-style cell has no u-template input");
        let mut tokens = b"width1pt height2pt depth0pt"
            .iter()
            .map(|byte| Token::Char {
                ch: char::from(*byte),
                cat: if byte.is_ascii_alphabetic() {
                    Catcode::Letter
                } else {
                    Catcode::Other
                },
            })
            .collect::<Vec<_>>();
        tokens.push(delimiter);
        push(&mut command, tokens);

        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        let spec = processor
            .scan_rule_spec(UnexpandablePrimitive::VRule)
            .unwrap_or_else(|error| panic!("{name} rule scan succeeds: {error}"));
        assert_eq!(spec.depth.map(Scaled::raw), Some(0));
        let v = processor
            .get_x_token()
            .unwrap_or_else(|error| panic!("{name} v-template delivery succeeds: {error}"))
            .expect("v-template token is live");
        assert!(matches!(v.meaning(), Meaning::CharToken { ch: 'v', .. }));
        let endv = processor
            .get_x_token()
            .unwrap_or_else(|error| panic!("{name} end-template delivery succeeds: {error}"))
            .expect("retained v-template emits endv");
        assert!(matches!(endv.meaning(), Meaning::EndV));
        let finished = processor
            .command
            .finish_alignment_cell(alignment)
            .expect("only exhausted v-template completes the cell");
        assert_eq!(finished.delimiter, expected, "{name} delimiter is retained");
    }
}

#[test]
fn alignment_preamble_discards_leading_spaces_from_each_u_template_only() {
    let mut command = CommandState::default();
    let alignment = crate::AlignmentIdentity::new(1);
    command.begin_alignment(alignment);
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let hfil = universe.intern("hfil").symbol();
    let cr = universe.intern("cr").symbol();
    universe.set_meaning(
        hfil,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HFil),
    );
    universe.set_meaning(
        cr,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Cr),
    );
    push(
        &mut command,
        [
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Cs(hfil),
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Cs(hfil),
            Token::Char {
                ch: '&',
                cat: Catcode::AlignmentTab,
            },
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Cs(hfil),
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Cs(cr),
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .scan_alignment_preamble_opening()
            .expect("opening brace validates and backs up");
        processor
            .replay_alignment_preamble_opening()
            .expect("opening brace replays before preamble collection");
        processor
            .begin_alignment_preamble_scan()
            .expect("preamble scans");
    }

    let preamble = command
        .take_completed_alignment_preamble(alignment)
        .expect("frozen preamble is available");
    assert_eq!(preamble.columns.len(), 2);
    for column in &preamble.columns {
        let template = column.u_template.expect("u-template remains nonempty");
        assert_eq!(universe.tokens(template.token_list()), &[Token::Cs(hfil)]);
    }
    assert_eq!(
        universe.tokens(preamble.columns[0].v_template.token_list()),
        &[
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Cs(hfil),
        ]
    );
}

#[test]
fn alignment_preamble_missing_parameter_before_tab_replays_the_delimiter_into_v_template() {
    assert_missing_preamble_parameter(
        [
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: 'l',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '&',
                cat: Catcode::AlignmentTab,
            },
            Token::Char {
                ch: 'r',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
        ],
        2,
    );
}

#[test]
fn alignment_preamble_missing_parameter_before_cr_replays_the_delimiter_into_v_template() {
    assert_missing_preamble_parameter(
        [
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: 'l',
                cat: Catcode::Letter,
            },
        ],
        1,
    );
}

fn assert_missing_preamble_parameter(
    prefix: impl IntoIterator<Item = Token>,
    expected_columns: usize,
) {
    let mut command = CommandState::default();
    let alignment = crate::AlignmentIdentity::new(1);
    command.begin_alignment(alignment);
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let cr = universe.intern("cr").symbol();
    universe.set_meaning(
        cr,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Cr),
    );
    let mut tokens = prefix.into_iter().collect::<Vec<_>>();
    tokens.push(Token::Cs(cr));
    push(&mut command, tokens);
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .with_observer(&mut recorder);
        processor
            .scan_alignment_preamble_opening()
            .expect("opening brace validates and backs up");
        processor
            .replay_alignment_preamble_opening()
            .expect("opening brace replays before preamble collection");
        processor
            .begin_alignment_preamble_scan()
            .expect("missing parameter recovers through the v-template");
    }
    let preamble = command
        .take_completed_alignment_preamble(alignment)
        .expect("frozen preamble is available");
    assert_eq!(preamble.columns.len(), expected_columns);
    let recovery = recorder
        .0
        .iter()
        .position(|observation| {
            matches!(
                observation,
                CommandObservation::Alignment(record) if record.transition == "missing_parameter"
            )
        })
        .expect("TeX82 missing-parameter recovery is observed");
    let backup = recorder
        .0
        .iter()
        .enumerate()
        .skip(recovery + 1)
        .find_map(|(index, observation)| {
            matches!(
            observation,
            CommandObservation::Input(record) if record.transition == crate::InputTransition::Backup
        ).then_some(index)
        })
        .expect("back_error pushes the delimiter back into command input");
    assert!(
        recovery < backup,
        "recovery is selected before back_error input backup"
    );
}

#[test]
fn filename_registered_input_recovery_and_rollback_stay_command_owned() {
    let mut command = CommandState::default();
    push(
        &mut command,
        [
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: 'i',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'n',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'c',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let snapshot = command.snapshot();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();
    capabilities.register_input(
        "inc",
        SourceRegistration::new(
            RegisteredSourceKind::World,
            Arc::<[u8]>::from(b"z".as_slice()),
        ),
    );
    let input = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .open_registered_input()
            .expect("registered input opens")
    };
    assert_eq!(input.file_name.name, "inc");
    assert_eq!(input.file_name.termination, FileNameTermination::Group);
    command
        .rollback(snapshot)
        .expect("input opening rolls back");

    push(
        &mut command,
        [Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        }],
    );
    let error = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .open_registered_input()
            .expect_err("unregistered input is structured recovery")
    };
    assert_eq!(error, CommandError::MissingInput);
}
