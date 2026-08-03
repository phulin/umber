use tex_state::Universe;
use tex_state::ids::{MacroDefinitionId, OriginListId, TokenListId};
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::{
    MacroArgumentBuildError, MacroArgumentBuilder, MacroArguments, MacroCallOutcome,
    MacroParameterEscape, ParameterState,
};
use crate::CommandState;
use crate::processor::{DefinitionContext, ScannerStatus, ScannerWarning, TokenBuilderId};
use crate::{
    CommandError, CommandHostCapabilities, CommandHostContext, CommandObservation, CommandObserver,
    CommandProcessor, CommandRuntime, InputReason, InputTransition, RegisteredSourceKind,
    SourceRegistration,
};
use std::sync::Arc;

#[derive(Default)]
struct Recorder(Vec<CommandObservation>);

impl CommandObserver for Recorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

fn traced(ch: char) -> TracedTokenWord {
    TracedTokenWord::pack(
        Token::Char {
            ch,
            cat: Catcode::Other,
        },
        OriginId::UNKNOWN,
    )
}

#[test]
fn completed_arguments_share_one_buffer_and_preserve_empty_ranges() {
    let mut builder = MacroArgumentBuilder::default();
    builder
        .complete(1, [traced('a'), traced('b')])
        .expect("first completed argument is accepted");
    builder
        .complete(2, std::iter::empty())
        .expect("empty grouped argument still has a range");
    let arguments = builder.finish();

    assert_eq!(arguments.buffer.len(), 2);
    assert_eq!(
        arguments.ranges[0].map(|range| (range.start(), range.end())),
        Some((0, 2))
    );
    assert_eq!(
        arguments.ranges[1].map(|range| (range.start(), range.end())),
        Some((2, 2))
    );
}

#[test]
fn arguments_must_complete_in_canonical_definition_order() {
    let mut builder = MacroArgumentBuilder::default();
    assert_eq!(
        builder.complete(2, std::iter::empty()),
        Err(MacroArgumentBuildError::OutOfOrderSlot {
            expected: 1,
            actual: 2,
        })
    );
    builder
        .complete(1, std::iter::empty())
        .expect("first slot is accepted");
    assert_eq!(
        builder.complete(1, std::iter::empty()),
        Err(MacroArgumentBuildError::OutOfOrderSlot {
            expected: 2,
            actual: 1,
        })
    );
}

#[test]
fn replacement_parameter_forms_remain_compact_and_distinct() {
    assert_eq!(
        MacroParameterEscape::classify(Token::param(7)),
        Some(MacroParameterEscape::OutParameter(7))
    );
    assert_eq!(
        MacroParameterEscape::classify(Token::Char {
            ch: '#',
            cat: Catcode::Parameter,
        }),
        Some(MacroParameterEscape::EscapedParameter)
    );
    assert_eq!(
        MacroParameterEscape::classify(Token::Char {
            ch: '#',
            cat: Catcode::Other,
        }),
        None
    );
}

#[test]
fn activation_boundary_owns_arguments_before_exposing_its_body() {
    let mut builder = MacroArgumentBuilder::default();
    builder
        .complete(1, [traced('x')])
        .expect("argument completes");
    let mut state = CommandState::default();
    let body = state.push_macro_activation(
        tex_state::interner::Symbol::testing_new(1),
        MacroDefinitionId::testing_new(4),
        builder.finish(),
        OriginId::UNKNOWN,
        TokenListId::EMPTY,
        OriginListId::EMPTY,
    );

    let owner = state
        .parameters
        .activations
        .last()
        .expect("activation owner");
    assert_eq!(owner.identity.0, 0);
    assert_eq!(owner.arguments.buffer.len(), 1);
    let crate::input::InputLevel::Tokens(cursor) = state.input.levels.last().expect("body level")
    else {
        panic!("macro activation must push a token body level");
    };
    assert_eq!(cursor.identity, body);
    assert_eq!(
        cursor.behavior,
        crate::input::TokenBehavior::MacroBody(owner.identity)
    );
}

#[test]
fn activation_parent_tracks_the_live_nested_frame() {
    let mut parameters = ParameterState::default();
    let first = parameters.push_activation(
        tex_state::interner::Symbol::testing_new(1),
        MacroDefinitionId::testing_new(1),
        MacroArgumentBuilder::default().finish(),
        OriginId::UNKNOWN,
    );
    assert_eq!(first.0, 0);
    parameters.push_activation(
        tex_state::interner::Symbol::testing_new(2),
        MacroDefinitionId::testing_new(2),
        MacroArgumentBuilder::default().finish(),
        OriginId::UNKNOWN,
    );
    assert_eq!(parameters.activations.len(), 2);
    assert_eq!(parameters.parent_invocation(), OriginId::UNKNOWN);
}

fn run_macro_call(
    source: &[u8],
    flags: MeaningFlags,
    parameters: &[Token],
    install_outer: bool,
) -> Result<(CommandState, MacroCallOutcome), CommandError> {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(source),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    universe.install_primitive_meaning(
        "par",
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par),
    );
    let macro_name = universe.intern("m").symbol();
    let parameters = universe.intern_token_list(parameters);
    let replacement = universe.intern_token_list(&[Token::param(1)]);
    let definition = universe.intern_macro(MacroMeaning::new(flags, parameters, replacement));
    universe.set_meaning(macro_name, Meaning::Macro { flags, definition });
    if install_outer {
        let outer = universe.intern("outer").symbol();
        let empty = universe.intern_token_list(&[]);
        let outer_definition =
            universe.intern_macro(MacroMeaning::new(MeaningFlags::OUTER, empty, empty));
        universe.set_meaning(
            outer,
            Meaning::Macro {
                flags: MeaningFlags::OUTER,
                definition: outer_definition,
            },
        );
    }
    let mut capabilities = CommandHostCapabilities::default();
    let result = {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        );
        let call = processor
            .get_next()?
            .ok_or(CommandError::input_invariant())?;
        processor.macro_call(call)
    };
    result.map(|arguments| (command, arguments))
}

fn run_macro(
    source: &[u8],
    flags: MeaningFlags,
    parameters: &[Token],
    install_outer: bool,
) -> Result<(CommandState, MacroArguments), CommandError> {
    run_macro_call(source, flags, parameters, install_outer).and_then(|(command, outcome)| {
        let MacroCallOutcome::Activated = outcome else {
            return Err(CommandError::input_invariant());
        };
        let arguments = command
            .parameters
            .activations
            .last()
            .ok_or(CommandError::input_invariant())?
            .arguments
            .clone();
        Ok((command, arguments))
    })
}

fn run_observed_macro_call(
    source: &[u8],
    flags: MeaningFlags,
) -> (
    CommandState,
    Result<MacroCallOutcome, CommandError>,
    Recorder,
) {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(source),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    universe.install_primitive_meaning(
        "par",
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par),
    );
    let name = universe.intern("m").symbol();
    let parameters = universe.intern_token_list(&[Token::param(1)]);
    let replacement = universe.intern_token_list(&[Token::param(1)]);
    let definition = universe.intern_macro(MacroMeaning::new(flags, parameters, replacement));
    universe.set_meaning(name, Meaning::Macro { flags, definition });
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let outcome = {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder);
        let call = processor
            .get_next()
            .expect("macro call delivery")
            .expect("macro token");
        processor.macro_call(call)
    };
    (command, outcome, recorder)
}

#[test]
fn scalar_matcher_consumes_compulsory_prefix_before_undelimited_argument() {
    let (command, arguments) = run_macro(
        b"\\m[x]",
        MeaningFlags::EMPTY,
        &[
            Token::Char {
                ch: '[',
                cat: Catcode::Other,
            },
            Token::param(1),
        ],
        false,
    )
    .expect("prefix and argument match");
    assert_eq!(command.parameters.activations.len(), 1);
    assert_eq!(
        arguments
            .buffer
            .get(0)
            .expect("argument token")
            .semantic_token(),
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter
        }
    );
}

#[test]
fn parameterless_macro_pushes_replacement_without_matching_status() {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(&b"\\m"[..]),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let name = universe.intern("m").symbol();
    let empty = universe.intern_token_list(&[]);
    let definition = universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, empty));
    universe.set_meaning(
        name,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition,
        },
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    )
    .with_observer(&mut recorder);

    let call = processor
        .get_next()
        .expect("macro call delivery")
        .expect("macro token");
    processor
        .macro_call(call)
        .expect("parameterless macro activates");

    assert!(recorder.0.iter().any(|observation| {
        matches!(
            observation,
            CommandObservation::Input(record)
                if record.transition == InputTransition::Push && record.reason == InputReason::Macro
        )
    }));
    assert!(recorder.0.iter().all(|observation| !matches!(
        observation,
        CommandObservation::ScannerStatus(status) if status.to == "matching"
    )));
}

#[test]
fn matching_transition_retains_the_enclosing_definition_status() {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(&b"\\m{X}"[..]),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let name = universe.intern("m").symbol();
    let parameters = universe.intern_token_list(&[Token::Param(1)]);
    let replacement = universe.intern_token_list(&[Token::Param(1)]);
    let definition = universe.intern_macro(MacroMeaning::new(
        MeaningFlags::EMPTY,
        parameters,
        replacement,
    ));
    universe.set_meaning(
        name,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition,
        },
    );
    let _prior = command.begin_scanner_status(ScannerStatus::Defining(DefinitionContext {
        target: None,
        builder: TokenBuilderId(7),
        warning: ScannerWarning(7),
    }));
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    )
    .with_observer(&mut recorder);

    let call = processor
        .get_next()
        .expect("macro call delivery")
        .expect("macro token");
    processor.macro_call(call).expect("macro argument matches");

    assert!(recorder.0.iter().any(|observation| matches!(
        observation,
        CommandObservation::ScannerStatus(status)
            if status.from == "defining" && status.to == "matching"
    )));
}

#[test]
fn scalar_matcher_reports_compulsory_prefix_mismatch_without_activating_body() {
    let (mut command, outcome) = run_macro_call(
        b"\\m(x)",
        MeaningFlags::EMPTY,
        &[
            Token::Char {
                ch: '[',
                cat: Catcode::Other,
            },
            Token::param(1),
        ],
        false,
    )
    .expect("TeX82 §391 recovers after reporting the mismatch");

    assert_eq!(outcome, MacroCallOutcome::PrefixMismatchRecovered);
    assert!(command.parameters.activations.is_empty());
    assert!(matches!(
        command.take_semantic_diagnostics().as_slice(),
        [crate::CommandSemanticDiagnostic::MacroPrefixMismatch { .. }]
    ));
}

#[test]
fn outer_recovery_in_compulsory_prefix_still_reports_definition_mismatch() {
    let (command, outcome) = run_macro_call(
        b"\\m\\outer",
        MeaningFlags::EMPTY,
        &[Token::Char {
            ch: 'x',
            cat: Catcode::Other,
        }],
        true,
    )
    .expect("TeX82 §391 recovers the mismatching inserted paragraph");

    assert_eq!(outcome, MacroCallOutcome::PrefixMismatchRecovered);
    assert!(command.parameters.activations.is_empty());
    assert!(matches!(
        command.semantic_diagnostics.as_slice(),
        [
            crate::CommandSemanticDiagnostic::Recoverable { .. },
            crate::CommandSemanticDiagnostic::MacroPrefixMismatch { context, .. },
        ] if context.contains("<inserted text>") && context.contains("\\par")
    ));
}

#[test]
fn later_macro_trace_queues_behind_pending_prefix_mismatch() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    universe.set_int_param(tex_state::env::banks::IntParam::TRACING_MACROS, 1);
    let mismatched = universe.intern("T").symbol();
    let traced = universe.intern("a").symbol();
    let empty = universe.intern_token_list(&[]);
    command
        .semantic_diagnostics
        .push(crate::CommandSemanticDiagnostic::MacroPrefixMismatch {
            macro_name: mismatched,
            context: String::new(),
        });
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );

    processor.trace_macro_invocation(traced, empty, empty);

    assert!(matches!(
        processor.command.semantic_diagnostics.as_slice(),
        [
            crate::CommandSemanticDiagnostic::MacroPrefixMismatch { macro_name: name, .. },
            crate::CommandSemanticDiagnostic::Trace {
                text,
                force_newline: true,
            },
        ] if *name == mismatched && text == "\\a ->"
    ));
}

#[test]
fn undelimited_group_strips_only_its_outer_braces() {
    let (command, arguments) = run_macro(
        b"\\m {a{b}}",
        MeaningFlags::EMPTY,
        &[Token::param(1)],
        false,
    )
    .expect("balanced group matches");
    assert_eq!(command.parameters.activations.len(), 1);
    let buffer = &arguments.buffer;
    assert_eq!(buffer.len(), 4);
    assert!(matches!(
        buffer.get(1).expect("nested begin").semantic_token(),
        Token::Char {
            cat: Catcode::BeginGroup,
            ..
        }
    ));
    assert!(matches!(
        buffer.get(3).expect("nested end").semantic_token(),
        Token::Char {
            cat: Catcode::EndGroup,
            ..
        }
    ));
}

#[test]
fn macro_argument_recovery_emits_exact_extra_brace_and_runaway_reports() {
    let (extra, outcome, observations) = run_observed_macro_call(b"\\m}", MeaningFlags::LONG);
    assert_eq!(outcome, Err(CommandError::ParagraphInMacroArgument));
    assert!(extra.parameters.activations.is_empty());
    let [
        crate::CommandSemanticDiagnostic::Recoverable {
            identity: crate::macro_call::EXTRA_RIGHT_BRACE_ARGUMENT_DIAGNOSTIC,
            message: extra_message,
            help: extra_help,
            context: extra_context,
            ..
        },
        crate::CommandSemanticDiagnostic::Recoverable {
            identity: crate::macro_call::RUNAWAY_ARGUMENT_DIAGNOSTIC,
            message: runaway_message,
            help: runaway_help,
            context: runaway_context,
            ..
        },
    ] = extra.semantic_diagnostics.as_slice()
    else {
        panic!("expected the paired extra-brace and runaway reports")
    };
    assert_eq!(extra_message, "Argument of \\m has an extra }");
    assert_eq!(
        *extra_help,
        [
            "I've run across a `}' that doesn't seem to match anything.",
            "For example, `\\def\\a#1{...}' and `\\a}' would produce",
            "this error. If you simply proceed now, the `\\par' that",
            "I've just inserted will cause me to report a runaway",
            "argument that might be the root of the problem. But if",
            "your `}' was spurious, just type `2' and it will go away.",
        ]
    );
    assert_eq!(runaway_message, "Paragraph ended before \\m was complete");
    assert_eq!(
        *runaway_help,
        [
            "I suspect you've forgotten a `}', causing me to apply this",
            "control sequence to too much text. How can we recover?",
            "My plan is to forget the whole thing and hope for the best.",
        ]
    );
    assert!(
        extra_context.contains("<inserted text>"),
        "{extra_context:?}"
    );
    assert!(extra_context.contains("\\par"), "{extra_context:?}");
    assert!(
        runaway_context.contains("<to be read again>"),
        "{runaway_context:?}"
    );
    assert!(runaway_context.contains("\\par"), "{runaway_context:?}");
    let backups = observations
        .0
        .iter()
        .filter(|observation| {
            matches!(
                observation,
                CommandObservation::Input(record)
                    if record.transition == InputTransition::Backup
                        && record.reason == InputReason::Backup
            )
        })
        .count();
    assert_eq!(
        backups, 2,
        "the brace and inserted paragraph each have one backup owner"
    );

    let (non_long, outcome, observations) =
        run_observed_macro_call(b"\\m\\par", MeaningFlags::EMPTY);
    assert_eq!(outcome, Err(CommandError::ParagraphInMacroArgument));
    assert!(non_long.parameters.activations.is_empty());
    let [
        crate::CommandSemanticDiagnostic::Recoverable {
            identity: crate::macro_call::RUNAWAY_ARGUMENT_DIAGNOSTIC,
            message,
            help,
            context,
            ..
        },
    ] = non_long.semantic_diagnostics.as_slice()
    else {
        panic!("expected one runaway report")
    };
    assert_eq!(message, "Paragraph ended before \\m was complete");
    assert_eq!(*help, *runaway_help);
    assert!(context.contains("<to be read again>"), "{context:?}");
    assert!(context.contains("\\par"), "{context:?}");
    assert_eq!(
        observations
            .0
            .iter()
            .filter(|observation| matches!(
                observation,
                CommandObservation::Input(record)
                    if record.transition == InputTransition::Backup
                        && record.reason == InputReason::Backup
            ))
            .count(),
        1,
        "the rejected paragraph has exactly one backup owner"
    );

    let (long, outcome, observations) = run_observed_macro_call(b"\\m\\par", MeaningFlags::LONG);
    assert_eq!(outcome, Ok(MacroCallOutcome::Activated));
    assert!(long.semantic_diagnostics.is_empty());
    let activation = long.parameters.activations.last().expect("macro activates");
    assert_eq!(activation.arguments.buffer.len(), 1);
    assert!(matches!(
        activation
            .arguments
            .buffer
            .get(0)
            .expect("paragraph argument token")
            .semantic_token(),
        Token::Cs(_)
    ));
    assert!(observations.0.iter().any(|observation| matches!(
        observation,
        CommandObservation::Input(record)
            if record.transition == InputTransition::Push && record.reason == InputReason::Macro
    )));
    assert!(observations.0.iter().all(|observation| !matches!(
        observation,
        CommandObservation::Input(record) if record.transition == InputTransition::Backup
    )));
}

#[test]
fn non_long_paragraph_backs_up_before_restoring_matching_status() {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(&b"\\m\\par"[..]),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    universe.install_primitive_meaning(
        "par",
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par),
    );
    let name = universe.intern("m").symbol();
    let parameters = universe.intern_token_list(&[Token::param(1)]);
    let replacement = universe.intern_token_list(&[]);
    let definition = universe.intern_macro(MacroMeaning::new(
        MeaningFlags::EMPTY,
        parameters,
        replacement,
    ));
    universe.set_meaning(
        name,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition,
        },
    );
    let _prior = command.begin_scanner_status(ScannerStatus::Defining(DefinitionContext {
        target: None,
        builder: TokenBuilderId(9),
        warning: ScannerWarning(9),
    }));
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    )
    .with_observer(&mut recorder);

    let call = processor
        .get_next()
        .expect("macro call delivery")
        .expect("macro token");
    assert_eq!(
        processor.macro_call(call),
        Err(CommandError::ParagraphInMacroArgument)
    );

    let backup = recorder
        .0
        .iter()
        .position(|observation| matches!(
            observation,
            CommandObservation::Input(record)
                if record.transition == InputTransition::Backup && record.reason == InputReason::Backup
        ))
        .expect("paragraph is backed up while matching");
    let restored = recorder
        .0
        .iter()
        .position(|observation| {
            matches!(
                observation,
                CommandObservation::ScannerStatus(status)
                    if status.from == "matching" && status.to == "defining"
            )
        })
        .expect("matching restores enclosing definition status");
    assert!(backup < restored);
}

#[test]
fn outer_argument_token_uses_raw_delivery_recovery_then_aborts_match() {
    assert_eq!(
        run_macro(b"\\m\\outer", MeaningFlags::EMPTY, &[Token::param(1)], true),
        Err(CommandError::OuterInMacroArgument)
    );
}

#[test]
fn matching_outer_recovery_reports_before_backing_up_the_forbidden_control_sequence() {
    // TeX82 §23's `check_outer_validity` is instrumented at entry, before
    // its `back_list` call; §394 owns the live matching episode.
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(&b"\\m\\outer"[..]),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    universe.install_primitive_meaning(
        "par",
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par),
    );
    let macro_name = universe.intern("m").symbol();
    let parameters = universe.intern_token_list(&[Token::param(1)]);
    let replacement = universe.intern_token_list(&[]);
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
    let outer_name = universe.intern("outer").symbol();
    let empty = universe.intern_token_list(&[]);
    let outer_definition =
        universe.intern_macro(MacroMeaning::new(MeaningFlags::OUTER, empty, empty));
    universe.set_meaning(
        outer_name,
        Meaning::Macro {
            flags: MeaningFlags::OUTER,
            definition: outer_definition,
        },
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    )
    .with_observer(&mut recorder);

    let call = processor
        .get_next()
        .expect("macro call delivery")
        .expect("macro token");
    assert_eq!(
        processor.macro_call(call),
        Err(CommandError::OuterInMacroArgument)
    );

    let diagnostic = recorder
        .0
        .iter()
        .position(|observation| matches!(
            observation,
            CommandObservation::Diagnostic(diagnostic)
                if diagnostic.diagnostic == "outer_validity_control_sequence"
                    && diagnostic.arguments == [crate::DiagnosticArgument::Name("matching".into())]
        ))
        .expect("matching outer diagnostic");
    let backup = recorder
        .0
        .iter()
        .position(|observation| matches!(
            observation,
            CommandObservation::Input(record)
                if record.transition == InputTransition::Backup && record.reason == InputReason::Backup
        ))
        .expect("outer control sequence backup");
    assert!(diagnostic < backup, "diagnostic precedes raw backup");
}

#[test]
fn successful_call_activates_canonical_replacement_and_replays_parameter_range() {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(&b"\\m{x}"[..]),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let name = universe.intern("m").symbol();
    let parameters = universe.intern_token_list(&[Token::param(1)]);
    let replacement = universe.intern_token_list(&[Token::param(1), other('!')]);
    let definition = universe.intern_macro(MacroMeaning::new(
        MeaningFlags::EMPTY,
        parameters,
        replacement,
    ));
    universe.set_meaning(
        name,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition,
        },
    );
    let mut capabilities = CommandHostCapabilities::default();
    let snapshot = command.snapshot();
    let activated;
    {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        );

        let call = processor
            .get_next()
            .expect("macro call delivery")
            .expect("macro token");
        let MacroCallOutcome::Activated = processor.macro_call(call).expect("macro matches") else {
            panic!("matching macro must activate");
        };
        let arguments = &processor
            .command
            .parameters
            .activations
            .last()
            .expect("activation")
            .arguments;
        assert_eq!(arguments.buffer.len(), 1);
        assert_eq!(processor.command.parameters.activations.len(), 1);
        assert!(matches!(
            processor.command.input.levels.last(),
            Some(crate::input::InputLevel::Tokens(cursor))
                if matches!(cursor.behavior, crate::input::TokenBehavior::MacroBody(_))
        ));
        activated = processor.command.clone();

        assert_eq!(
            processor
                .get_token()
                .expect("parameter replay")
                .expect("argument token")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter
            }
        );
        assert_eq!(
            processor
                .get_token()
                .expect("literal replay")
                .expect("literal token")
                .spelling()
                .semantic_token(),
            other('!')
        );
        assert!(
            processor
                .get_token()
                .expect("replacement retires")
                .is_some()
        );
        assert!(processor.command.parameters.activations.is_empty());
        assert!(processor.get_token().expect("source retires").is_none());
    }

    command
        .rollback(snapshot)
        .expect("macro-call snapshot restores the untouched input cursor");
    {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        );
        let call = processor
            .get_next()
            .expect("rolled-back macro call delivery")
            .expect("rolled-back macro token");
        processor
            .macro_call(call)
            .expect("rolled-back macro call matches again");
        assert_eq!(processor.command.input, activated.input);
        assert_eq!(processor.command.scanner, activated.scanner);
        assert_eq!(processor.command.transient, activated.transient);
        let replayed = processor
            .command
            .parameters
            .activations
            .last()
            .expect("rolled-back call installs one activation");
        let expected = activated
            .parameters
            .activations
            .last()
            .expect("original call installs one activation");
        assert_eq!(replayed.identity, expected.identity);
        assert_eq!(replayed.definition, expected.definition);
        assert_eq!(replayed.arguments, expected.arguments);
    }
    assert_eq!(
        universe.macro_invocation_provenance_stats().invocations(),
        2,
        "each replayed successful call creates exactly one fresh invocation origin"
    );
}

#[test]
fn nested_macro_activation_retires_the_exhausted_caller_before_pushing_the_callee() {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(&b"\\outer"[..]),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let outer = universe.intern("outer").symbol();
    let inner = universe.intern("inner").symbol();
    let empty = universe.intern_token_list(&[]);
    let outer_replacement = universe.intern_token_list(&[Token::Cs(inner)]);
    let outer_definition = universe.intern_macro(MacroMeaning::new(
        MeaningFlags::EMPTY,
        empty,
        outer_replacement,
    ));
    let inner_definition =
        universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, empty));
    universe.set_meaning(
        outer,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: outer_definition,
        },
    );
    universe.set_meaning(
        inner,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: inner_definition,
        },
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    )
    .with_observer(&mut recorder);

    assert!(
        processor
            .get_x_token()
            .expect("nested macro expansion succeeds")
            .is_none()
    );

    let macro_lifecycle: Vec<_> = recorder
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Input(record) if record.reason == InputReason::Macro => {
                Some(record.transition)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        macro_lifecycle,
        vec![
            InputTransition::Push,
            InputTransition::Retire,
            InputTransition::Push,
            InputTransition::Retire,
        ],
        "TeX82 §390 retires the depleted caller before the callee body is pushed"
    );
}

#[test]
fn nested_calls_keep_out_parameter_ownership_and_invocation_provenance() {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(&b"\\outer{x}"[..]),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let outer = universe.intern("outer").symbol();
    let inner = universe.intern("inner").symbol();
    let one_parameter = universe.intern_token_list(&[Token::param(1)]);
    let outer_replacement = universe.intern_token_list(&[Token::Cs(inner), Token::param(1)]);
    let escaped_hash = Token::Char {
        ch: '#',
        cat: Catcode::Parameter,
    };
    let inner_replacement = universe.intern_token_list(&[escaped_hash, Token::param(1)]);
    let outer_definition = universe.intern_macro(MacroMeaning::new(
        MeaningFlags::EMPTY,
        one_parameter,
        outer_replacement,
    ));
    let inner_definition = universe.intern_macro(MacroMeaning::new(
        MeaningFlags::EMPTY,
        one_parameter,
        inner_replacement,
    ));
    universe.set_meaning(
        outer,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: outer_definition,
        },
    );
    universe.set_meaning(
        inner,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: inner_definition,
        },
    );
    let mut capabilities = CommandHostCapabilities::default();
    {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        );

        let outer_call = processor
            .get_next()
            .expect("outer delivery")
            .expect("outer token");
        processor.macro_call(outer_call).expect("outer matches");
        let inner_call = processor
            .get_next()
            .expect("inner delivery")
            .expect("inner token");
        processor.macro_call(inner_call).expect("inner matches");
        // TeX82 §390 drains every depleted token list before the callee body
        // is pushed. Matching `\inner`'s argument exhausts both the replayed
        // parameter level and the caller's macro body, so `end_token_list`
        // pops the body and flushes its `param_stack` entries: this tail call
        // leaves exactly one live activation, the callee's.
        assert_eq!(processor.command.parameters.activations.len(), 1);
        assert_eq!(
            processor
                .get_token()
                .expect("escaped hash")
                .expect("hash token")
                .spelling()
                .semantic_token(),
            escaped_hash
        );
        assert_eq!(
            processor
                .get_token()
                .expect("nested parameter replay")
                .expect("argument token")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter
            }
        );
        assert!(
            processor
                .get_token()
                .expect("nested replay retires")
                .is_some()
        );
        assert!(processor.command.parameters.activations.is_empty());
        assert!(processor.get_token().expect("source retires").is_none());
    }
    assert_eq!(
        universe.macro_invocation_provenance_stats().invocations(),
        2
    );
}

fn other(ch: char) -> Token {
    Token::Char {
        ch,
        cat: Catcode::Other,
    }
}

fn argument_tokens(arguments: &MacroArguments) -> Vec<Token> {
    (0..arguments.buffer.len())
        .map(|index| {
            arguments
                .buffer
                .get(index)
                .expect("argument token")
                .semantic_token()
        })
        .collect()
}

#[test]
fn delimited_argument_stops_at_its_literal_token_sequence() {
    let (_, arguments) = run_macro(
        b"\\m x,",
        MeaningFlags::EMPTY,
        &[Token::param(1), other(',')],
        false,
    )
    .expect("delimiter terminates the argument");
    assert_eq!(
        argument_tokens(&arguments),
        vec![Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        }]
    );
}

#[test]
fn delimited_argument_retries_a_mismatch_as_an_overlapping_prefix() {
    let (_, arguments) = run_macro(
        b"\\m aaab",
        MeaningFlags::EMPTY,
        &[
            Token::param(1),
            Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'b',
                cat: Catcode::Letter,
            },
        ],
        false,
    )
    .expect("the final two a tokens begin the overlapping delimiter");
    assert_eq!(
        argument_tokens(&arguments),
        vec![Token::Char {
            ch: 'a',
            cat: Catcode::Letter,
        }]
    );
}

#[test]
fn delimited_argument_commits_a_partial_prefix_before_continuing() {
    let (_, arguments) = run_macro(
        b"\\m!x!?",
        MeaningFlags::EMPTY,
        &[Token::param(1), other('!'), other('?')],
        false,
    )
    .expect("partial prefix is argument material");
    assert_eq!(
        argument_tokens(&arguments),
        vec![
            Token::Char {
                ch: '!',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
        ]
    );
}

#[test]
fn delimiter_inside_literal_braces_remains_argument_material() {
    let (_, arguments) = run_macro(
        b"\\m{a,b}c,",
        MeaningFlags::EMPTY,
        &[Token::param(1), other(',')],
        false,
    )
    .expect("nested delimiter does not terminate the argument");
    assert_eq!(
        argument_tokens(&arguments),
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            },
            other(','),
            Token::Char {
                ch: 'b',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
            Token::Char {
                ch: 'c',
                cat: Catcode::Letter,
            },
        ]
    );
}

#[test]
fn delimited_argument_strips_one_complete_outer_group() {
    let (_, arguments) = run_macro(
        b"\\m{{a}},",
        MeaningFlags::EMPTY,
        &[Token::param(1), other(',')],
        false,
    )
    .expect("complete outer group is stripped");
    assert_eq!(
        argument_tokens(&arguments),
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ]
    );
}

#[test]
fn non_long_delimited_argument_allows_a_recovered_paragraph_prefix() {
    let mut universe = Universe::new_with_plain_catcodes();
    let par = universe.intern("par").symbol();
    drop(universe);
    let (_, arguments) = run_macro(
        b"\\m\\par?\\par!",
        MeaningFlags::EMPTY,
        &[Token::param(1), Token::Cs(par), other('!')],
        false,
    )
    .expect("failed delimiter prefix is accepted literally");
    assert_eq!(arguments.buffer.len(), 2);
    assert!(matches!(
        arguments
            .buffer
            .get(0)
            .expect("recovered prefix")
            .semantic_token(),
        Token::Cs(_)
    ));
    assert_eq!(
        arguments.buffer.get(1).expect("question").semantic_token(),
        other('?')
    );
}

#[test]
fn non_long_delimited_argument_rejects_a_paragraph_mismatch() {
    assert_eq!(
        run_macro(
            b"\\m a\\par",
            MeaningFlags::EMPTY,
            &[
                Token::param(1),
                Token::Char {
                    ch: 'a',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: 'b',
                    cat: Catcode::Letter,
                },
            ],
            false,
        ),
        Err(CommandError::ParagraphInMacroArgument)
    );
}

#[test]
fn delimiter_opening_brace_does_not_leave_literal_brace_accounting() {
    let (command, arguments) = run_macro(
        b"\\m{",
        MeaningFlags::EMPTY,
        &[
            Token::param(1),
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
        ],
        false,
    )
    .expect("opening brace may be a parameter delimiter");
    assert_eq!(arguments.buffer.len(), 0);
    // The delimiter brace's `get_next` increment and `back_input`
    // decrement cancel, leaving §331's running base untouched.
    assert_eq!(
        command.alignment.align_state,
        crate::processor::TOP_LEVEL_ALIGN_STATE
    );
}

#[test]
fn delimited_argument_eof_consumes_matching_recovery_before_failing() {
    assert_eq!(
        run_macro(
            b"\\m x",
            MeaningFlags::EMPTY,
            &[Token::param(1), other(',')],
            false,
        ),
        Err(CommandError::ParagraphInMacroArgument)
    );
}
