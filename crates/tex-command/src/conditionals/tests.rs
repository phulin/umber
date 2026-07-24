use tex_state::Universe;
use tex_state::meaning::{ExpandablePrimitive, Meaning};
use tex_state::token::{OriginId, Token, TracedTokenWord};

use super::*;
use crate::input::{
    ReplayTrace, RetirementBehavior, SharedTokenBuffer, TokenBehavior, TokenPayload,
};
use crate::{CommandHostCapabilities, CommandHostContext, CommandRuntime, CommandState};

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

fn install(universe: &mut Universe, name: &str, primitive: ExpandablePrimitive) -> Token {
    let symbol = universe.intern(name).symbol();
    universe.set_meaning(symbol, Meaning::ExpandablePrimitive(primitive));
    Token::Cs(symbol)
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

#[test]
fn condition_identity_updates_outer_frame_after_nested_push() {
    let mut stack = ConditionStack::default();
    let outer = stack.push(ConditionalKind::IfNum, 17);
    let inner = stack.push(ConditionalKind::IfX, 19);

    assert!(stack.change_if_limit(outer, IfLimit::Else));
    assert_eq!(stack.limit(outer), Some(IfLimit::Else));
    assert_eq!(stack.limit(inner), Some(IfLimit::Evaluating));
    assert_eq!(
        stack.current().expect("inner remains current").identity,
        inner
    );
}

#[test]
fn evaluating_delimiter_recovery_is_typed_and_frame_specific() {
    let mut stack = ConditionStack::default();
    let evaluating = stack.push(ConditionalKind::If, 3);
    let completed = stack.push(ConditionalKind::IfCase, 5);
    assert!(stack.change_if_limit(completed, IfLimit::Or));

    assert_eq!(
        stack.evaluating_delimiter_recovery(evaluating, ConditionalDelimiter::Else),
        Some(EvaluatingDelimiterRecovery {
            condition: evaluating,
            delimiter: ConditionalDelimiter::Else,
        })
    );
    assert_eq!(
        stack.evaluating_delimiter_recovery(completed, ConditionalDelimiter::Else),
        None
    );
}

#[test]
fn pass_text_uses_get_next_and_skips_nested_conditionals() {
    let mut command = CommandState::default();
    let condition = command.conditions.push(ConditionalKind::IfFalse, 1);
    assert!(command.conditions.change_if_limit(condition, IfLimit::Fi));
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let nested_if = install(&mut universe, "nestedif", ExpandablePrimitive::IfTrue);
    let nested_fi = install(&mut universe, "nestedfi", ExpandablePrimitive::Fi);
    let outer_fi = install(&mut universe, "outerfi", ExpandablePrimitive::Fi);
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: tex_state::token::Catcode::BeginGroup,
            },
            nested_if,
            nested_fi,
            outer_fi,
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    assert_eq!(
        processor.pass_text(condition, ScannerWarning(41)),
        Ok(PassTextStop {
            delimiter: ConditionalDelimiter::Fi,
            nested_conditions: 0,
        })
    );
    assert!(matches!(
        processor.command.scanner.status(),
        ScannerStatus::Normal
    ));
    assert_eq!(processor.command.alignment.align_state, 1);
}

#[test]
fn pass_text_only_accepts_or_when_the_frame_limit_allows_it() {
    let mut command = CommandState::default();
    let condition = command.conditions.push(ConditionalKind::IfCase, 1);
    assert!(command.conditions.change_if_limit(condition, IfLimit::Or));
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let or = install(&mut universe, "or", ExpandablePrimitive::Or);
    push(&mut command, vec![or]);
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    assert_eq!(
        processor.pass_text(condition, ScannerWarning(7)),
        Ok(PassTextStop {
            delimiter: ConditionalDelimiter::Or,
            nested_conditions: 0,
        })
    );
}
