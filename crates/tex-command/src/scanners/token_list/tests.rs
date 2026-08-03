use std::sync::Arc;

use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
use tex_state::token::{Catcode, Token};

use crate::observation::{CommandDeliveryBoundary, CommandObservation, CommandObserver};
use crate::scan_toks::ScanToksMode;
use crate::{
    CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandRuntime, CommandState,
    RegisteredSourceKind, SourceRegistration,
};

#[derive(Default)]
struct Recorder(Vec<CommandObservation>);

impl CommandObserver for Recorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

fn source(command: &mut CommandState, bytes: &'static [u8]) {
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(bytes),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
}

fn install_macro(universe: &mut tex_state::Universe, name: &str, replacement: Token) {
    let symbol = universe.intern(name).symbol();
    let parameters = universe.intern_token_list(&[]);
    let replacement = universe.intern_token_list(&[replacement]);
    let definition = universe.intern_macro(MacroMeaning::new(
        MeaningFlags::EMPTY,
        parameters,
        replacement,
    ));
    universe.set_meaning(
        symbol,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition,
        },
    );
}

#[test]
fn source_token_list_assignments_preserve_macros_but_expanded_collection_expands_them() {
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    install_macro(
        &mut universe,
        "payload",
        Token::Char {
            ch: 'X',
            cat: Catcode::Letter,
        },
    );
    let payload = universe.intern("payload").symbol();

    let scan = |bytes, output: bool, universe: &mut tex_state::Universe| {
        let mut command = CommandState::default();
        source(&mut command, bytes);
        let mut runtime = CommandRuntime::default();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        );
        if output {
            let owner = processor.state.intern_control_sequence("output");
            processor
                .scan_token_parameter_assignment(tex_state::env::banks::TokParam::OUTPUT, owner)
                .expect("output assignment scans")
        } else {
            let owner = processor.state.intern_control_sequence("toks");
            processor
                .scan_token_register_assignment(owner)
                .expect("token register assignment scans")
                .tokens
        }
    };

    let output = scan(br"={\payload}", true, &mut universe);
    assert_eq!(
        universe.tokens(output.token_list()),
        &[
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(payload),
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ]
    );
    let register = scan(br"0={\payload}", false, &mut universe);
    assert_eq!(
        universe.tokens(register.token_list()),
        &[Token::Cs(payload)]
    );

    let mut command = CommandState::default();
    source(&mut command, br"{\payload}");
    let mut runtime = CommandRuntime::default();
    let mut capabilities = CommandHostCapabilities::default();
    let expanded = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    )
    .scan_toks(ScanToksMode::General { expanded: true })
    .expect("expanded token-list collection scans");
    assert_eq!(
        universe.tokens(expanded.replacement_text.token_list()),
        &[Token::Char {
            ch: 'X',
            cat: Catcode::Letter,
        }]
    );
}

#[test]
fn token_register_runaway_retains_assignment_owner() {
    // TeX82 §§306/336/1227 keep the selected register shorthand in
    // `warning_index` throughout its absorbing scan.
    let mut command = CommandState::default();
    source(&mut command, br"{\outer}");
    let mut runtime = CommandRuntime::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let outer = universe.intern("outer").symbol();
    let empty = universe.intern_token_list(&[]);
    let definition = universe.intern_macro(MacroMeaning::new(MeaningFlags::OUTER, empty, empty));
    universe.set_meaning(
        outer,
        Meaning::Macro {
            flags: MeaningFlags::OUTER,
            definition,
        },
    );
    let owner = universe.intern("tokens").symbol();
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );

    processor
        .scan_token_register_value(owner)
        .expect("outer token recovers");
    let diagnostics = processor.take_semantic_diagnostics();
    assert!(
        diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            crate::CommandSemanticDiagnostic::Recoverable { message, .. }
                if message == "Forbidden control sequence found while scanning text of \\tokens"
        )),
        "{diagnostics:?}"
    );
}

#[test]
fn source_the_output_observes_operand_as_expanded_but_not_the_opener() {
    let mut command = CommandState::default();
    source(&mut command, br"{\the\output}");
    let mut runtime = CommandRuntime::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let the = universe.intern("the").symbol();
    universe.set_meaning(the, Meaning::ExpandablePrimitive(ExpandablePrimitive::The));
    let output = universe.intern("output").symbol();
    universe.set_meaning(
        output,
        Meaning::TokParam(tex_state::env::banks::TokParam::OUTPUT.raw()),
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
    .expect("the output source scans");

    let has = |boundary, command| {
        recorder.0.iter().any(|event| {
            matches!(event, CommandObservation::Command(record)
                if record.boundary == boundary && record.command == command)
        })
    };
    assert!(has(CommandDeliveryBoundary::Raw, "the"));
    assert!(!has(CommandDeliveryBoundary::Expanded, "the"));
    assert!(has(CommandDeliveryBoundary::Raw, "assign_toks"));
    assert!(has(CommandDeliveryBoundary::Expanded, "assign_toks"));
}
