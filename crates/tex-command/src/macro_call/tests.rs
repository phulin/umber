use tex_state::Universe;
use tex_state::ids::{MacroDefinitionId, OriginListId, TokenListId};
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::{
    MacroArgumentBuildError, MacroArgumentBuilder, MacroArguments, MacroParameterEscape,
    ParameterState,
};
use crate::CommandState;
use crate::{
    CommandError, CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandRuntime,
    RegisteredSourceKind, SourceRegistration,
};
use std::sync::Arc;

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
        MacroDefinitionId::testing_new(1),
        MacroArgumentBuilder::default().finish(),
        OriginId::UNKNOWN,
    );
    assert_eq!(first.0, 0);
    parameters.push_activation(
        MacroDefinitionId::testing_new(2),
        MacroArgumentBuilder::default().finish(),
        OriginId::UNKNOWN,
    );
    assert_eq!(parameters.activations.len(), 2);
    assert_eq!(parameters.parent_invocation(), OriginId::UNKNOWN);
}

fn run_macro(
    source: &[u8],
    flags: MeaningFlags,
    parameters: &[Token],
    install_outer: bool,
) -> Result<(CommandState, MacroArguments), CommandError> {
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
    let mut universe = Universe::new();
    let macro_name = universe.intern("m").symbol();
    let parameters = universe.intern_token_list(parameters);
    let replacement = universe.intern_token_list(&[Token::param(1)]);
    let definition = universe.intern_macro(MacroMeaning::new(flags, parameters, replacement));
    universe.set_meaning(macro_name, Meaning::Macro { flags, definition });
    universe.install_primitive_meaning(
        "par",
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par),
    );
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
        let call = processor.get_next()?.ok_or(CommandError::InputInvariant)?;
        processor.macro_call(call)
    };
    result.map(|arguments| (command, arguments))
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
    assert!(command.parameters.activations.is_empty());
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
fn scalar_matcher_rejects_compulsory_prefix_mismatch() {
    assert_eq!(
        run_macro(
            b"\\m(x)",
            MeaningFlags::EMPTY,
            &[
                Token::Char {
                    ch: '[',
                    cat: Catcode::Other
                },
                Token::param(1),
            ],
            false,
        ),
        Err(CommandError::MacroPrefixMismatch)
    );
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
    assert!(command.parameters.activations.is_empty());
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
fn paragraph_is_rejected_only_for_non_long_macros() {
    assert_eq!(
        run_macro(b"\\m\\par", MeaningFlags::EMPTY, &[Token::param(1)], false),
        Err(CommandError::ParagraphInMacroArgument)
    );
    let (_, arguments) = run_macro(b"\\m\\par", MeaningFlags::LONG, &[Token::param(1)], false)
        .expect("long macro accepts paragraph token");
    assert!(matches!(
        arguments
            .buffer
            .get(0)
            .expect("paragraph token")
            .semantic_token(),
        Token::Cs(_)
    ));
}

#[test]
fn outer_argument_token_uses_raw_delivery_recovery_then_aborts_match() {
    assert_eq!(
        run_macro(b"\\m\\outer", MeaningFlags::EMPTY, &[Token::param(1)], true),
        Err(CommandError::OuterInMacroArgument)
    );
}
