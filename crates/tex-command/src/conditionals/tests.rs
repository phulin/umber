use super::*;

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
