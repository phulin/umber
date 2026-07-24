use tex_state::ids::{MacroDefinitionId, OriginListId, TokenListId};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::{MacroArgumentBuildError, MacroArgumentBuilder, MacroParameterEscape, ParameterState};
use crate::CommandState;

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
