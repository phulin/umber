use super::*;
use tex_state::env::AssignmentScope;
use tex_state::meaning::MeaningWord;
use tex_state::token::{Catcode, Token};

use crate::{CommandHostCapabilities, CommandSemanticDiagnostic};

fn other(ch: char) -> Token {
    Token::Char {
        ch,
        cat: Catcode::Other,
    }
}

fn install<G>(
    universe: &mut tex_state::Universe<G>,
    name: &str,
    primitive: ExpandablePrimitive,
) -> Token {
    let symbol = universe.intern(name).expect("conditional name");
    universe
        .assign_meaning(
            symbol,
            MeaningWord::from_static(Meaning::ExpandablePrimitive(primitive)),
            AssignmentScope::Global,
        )
        .expect("conditional meaning");
    Token::Cs(symbol.symbol())
}

fn next_character<G>(processor: &mut CommandProcessor<'_, G>) -> char {
    match processor
        .get_x_token()
        .expect("conditional delivery")
        .expect("character after expansion")
        .meaning()
    {
        ResolvedMeaning::Static(Meaning::CharToken { ch, .. }) => ch,
        other => panic!("expected a character, found {other:?}"),
    }
}

#[test]
fn etex_current_if_values_preserve_kind_inversion_and_branch() {
    let cases = [
        (ConditionalKind::If, 1),
        (ConditionalKind::IfNum, 3),
        (ConditionalKind::IfCase, 17),
        (ConditionalKind::IfInCsName, 21),
    ];
    assert_eq!(ConditionStack::default().current_etex_values(), (0, 0, 0));
    for (kind, expected) in cases {
        for (inverted, signed) in [(false, expected), (true, -expected)] {
            let mut stack = ConditionStack::default();
            let condition = stack.push_with_inversion(kind, 37, inverted);
            assert!(stack.change_if_limit(condition, IfLimit::Or));
            assert_eq!(stack.current_etex_values(), (1, signed, 1));
        }
    }
}

#[test]
fn condition_identity_updates_an_outer_frame_without_disturbing_the_inner_frame() {
    let mut stack = ConditionStack::default();
    let outer = stack.push(ConditionalKind::IfNum, 17);
    let inner = stack.push(ConditionalKind::IfX, 19);

    assert!(stack.change_if_limit(outer, IfLimit::Else));
    assert_eq!(stack.limit(outer), Some(IfLimit::Else));
    assert_eq!(stack.limit(inner), Some(IfLimit::Evaluating));
    assert_eq!(stack.current().expect("inner").identity, inner);
}

#[test]
fn cleanup_drains_incomplete_conditions_in_current_first_order() {
    let mut stack = ConditionStack::default();
    stack.push(ConditionalKind::IfTrue, 11);
    stack.push(ConditionalKind::IfCase, 23);
    stack.push(ConditionalKind::IfNum, 37);

    assert_eq!(
        stack
            .drain_incomplete()
            .iter()
            .map(|condition| (condition.kind_name(), condition.source_line()))
            .collect::<Vec<_>>(),
        [("ifnum", 37), ("ifcase", 23), ("iftrue", 11)]
    );
    assert!(stack.current().is_none());
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
fn false_boolean_skips_to_else_and_matching_fi_retires_the_frame() {
    crate::test_harness::with_universe(|universe| {
        let if_false = install(universe, "iffalse", ExpandablePrimitive::IfFalse);
        let otherwise = install(universe, "else", ExpandablePrimitive::Else);
        let fi = install(universe, "fi", ExpandablePrimitive::Fi);
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [if_false, other('f'), otherwise, other('t'), fi],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor =
            crate::test_harness::processor(&mut command, universe, &mut capabilities);

        assert_eq!(next_character(&mut processor), 't');
        assert_eq!(
            processor
                .command
                .conditions
                .current()
                .expect("else frame")
                .limit,
            IfLimit::Fi
        );
        assert!(processor.get_x_token().expect("matching fi").is_none());
        assert!(processor.command.conditions.current().is_none());
    });
}

#[test]
fn ifx_compares_raw_operands_without_expanding_them() {
    crate::test_harness::with_universe(|universe| {
        let if_x = install(universe, "ifx", ExpandablePrimitive::IfX);
        let if_true = install(universe, "iftrue", ExpandablePrimitive::IfTrue);
        let otherwise = install(universe, "else", ExpandablePrimitive::Else);
        let fi = install(universe, "fi", ExpandablePrimitive::Fi);
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [
                if_x,
                if_true,
                if_true,
                other('y'),
                otherwise,
                other('n'),
                fi,
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor =
            crate::test_harness::processor(&mut command, universe, &mut capabilities);

        assert_eq!(next_character(&mut processor), 'y');
        assert!(processor.get_x_token().expect("matching fi").is_none());
    });
}

#[test]
fn extra_delimiter_recovery_keeps_following_input_and_owns_its_diagnostic() {
    crate::test_harness::with_universe(|universe| {
        let or = install(universe, "or", ExpandablePrimitive::Or);
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [or, other('t')]);
        let mut capabilities = CommandHostCapabilities::default();
        {
            let mut processor =
                crate::test_harness::processor(&mut command, universe, &mut capabilities);
            assert_eq!(next_character(&mut processor), 't');
        }
        assert!(
            command
                .take_semantic_diagnostics()
                .iter()
                .any(|diagnostic| {
                    matches!(
                        diagnostic,
                        CommandSemanticDiagnostic::Recoverable { message, .. }
                    if message.starts_with("Extra ")
                    )
                })
        );
    });
}
