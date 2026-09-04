use super::*;
use tex_state::env::AssignmentScope;
use tex_state::meaning::{MeaningFlags, MeaningWord};
use tex_state::token::{Catcode, Token, TokenWord};

use crate::{
    CommandHostCapabilities, CommandObservation, CommandObserver, CommandSemanticDiagnostic,
    DeliveryStatus, InputReason, InputTransition,
};

#[derive(Default)]
struct InsertedRowBalanceObserver {
    pushes: usize,
    retires: usize,
}

impl CommandObserver for InsertedRowBalanceObserver {
    fn committed(&mut self, observation: CommandObservation) {
        let CommandObservation::Input(record) = observation else {
            return;
        };
        match (record.transition, record.reason) {
            (InputTransition::Recovery, InputReason::Recovery) => self.pushes += 1,
            (InputTransition::Retire, InputReason::Recovery) => self.retires += 1,
            _ => {}
        }
    }
}

fn other(ch: char) -> Token {
    Token::Char {
        ch,
        cat: Catcode::Other,
    }
}

fn space() -> Token {
    Token::Char {
        ch: ' ',
        cat: Catcode::Space,
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

fn next_character<G>(processor: &mut CommandProcessor<'_, '_, G>) -> char {
    let mut destination = None;
    assert_eq!(
        processor
            .get_x_token_into(&mut destination)
            .expect("conditional delivery"),
        DeliveryStatus::Command
    );
    match destination
        .expect("character delivery initializes destination")
        .meaning()
    {
        ResolvedMeaning::Static(Meaning::CharToken { ch, .. }) => ch,
        other => panic!("expected a character, found {other:?}"),
    }
}

fn assert_expanded_end<G>(processor: &mut CommandProcessor<'_, '_, G>) {
    let mut destination = None;
    assert_eq!(
        processor
            .get_x_token_into(&mut destination)
            .expect("conditional end delivery"),
        DeliveryStatus::End
    );
    assert!(destination.is_none());
}

fn ifx_branch<G>(universe: &mut tex_state::Universe<G>, first: Token, second: Token) -> char {
    let if_x = install(universe, "ifx", ExpandablePrimitive::IfX);
    let otherwise = install(universe, "else", ExpandablePrimitive::Else);
    let fi = install(universe, "fi", ExpandablePrimitive::Fi);
    let mut command = CommandState::default();
    crate::test_harness::push(
        &mut command,
        [if_x, first, second, other('y'), otherwise, other('n'), fi],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut fuel = crate::CommandFuelLedger::default();
    let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
    let mut context = universe.command_context().expect("command context");
    let mut processor = crate::test_harness::processor(
        &mut command,
        &mut context,
        &mut capabilities,
        &mut fuel,
        &mut diagnostic_effects,
    );

    let branch = next_character(&mut processor);
    assert_expanded_end(&mut processor);
    branch
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
fn active_character_operand_stays_in_the_caller_slot_for_conditional_treatment() {
    crate::test_harness::with_universe(|universe| {
        let no_expand = install(universe, "noexpand", ExpandablePrimitive::NoExpand);
        let active_symbol = universe
            .intern_active_character('~')
            .expect("active character");
        universe
            .assign_meaning(
                active_symbol,
                MeaningWord::from_static(Meaning::ExpandablePrimitive(ExpandablePrimitive::IfTrue)),
                AssignmentScope::Global,
            )
            .expect("active character meaning");
        let active = Token::Char {
            ch: '~',
            cat: Catcode::Active,
        };
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [no_expand, active]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        let mut destination = None;
        processor
            .get_x_token_or_active_char_into(&mut destination)
            .expect("conditional operand delivery");
        let delivered = destination.expect("caller destination");
        assert_eq!(delivered.spelling().semantic_token(), active);
        assert_eq!(delivered.meaning(), ResolvedMeaning::Static(Meaning::Relax));
        assert_eq!(CommandProcessor::if_character_code(&delivered), '~' as u32);
        assert_eq!(
            CommandProcessor::if_category_code(&delivered),
            Some(Catcode::Active)
        );
    });
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
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

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
        assert_expanded_end(&mut processor);
        assert!(processor.command.conditions.current().is_none());
    });
}

#[test]
fn if_and_ifcat_operands_stay_in_the_shared_delivery_lane() {
    crate::test_harness::with_universe(|universe| {
        let if_test = install(universe, "if", ExpandablePrimitive::If);
        let if_cat = install(universe, "ifcat", ExpandablePrimitive::IfCat);
        let otherwise = install(universe, "else", ExpandablePrimitive::Else);
        let fi = install(universe, "fi", ExpandablePrimitive::Fi);
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [
                if_test,
                other('a'),
                other('a'),
                other('T'),
                otherwise,
                other('F'),
                fi,
                if_cat,
                Token::Char {
                    ch: 'a',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: 'b',
                    cat: Catcode::Letter,
                },
                other('N'),
                otherwise,
                other('Y'),
                fi,
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        assert_eq!(next_character(&mut processor), 'T');
        assert_eq!(next_character(&mut processor), 'N');
        assert_expanded_end(&mut processor);
        assert_eq!(processor.command.scratch.driver_continuation_depth(), 0);
    });
}

#[test]
fn if_operand_string_compare_completes_its_exact_parent_once() {
    crate::test_harness::with_universe(|universe| {
        let if_test = install(universe, "if-string-compare", ExpandablePrimitive::If);
        let string_compare = install(
            universe,
            "string-compare-if-operand",
            ExpandablePrimitive::StringCompare,
        );
        let otherwise = install(universe, "else-string-compare", ExpandablePrimitive::Else);
        let fi = install(universe, "fi-string-compare", ExpandablePrimitive::Fi);
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [
                if_test,
                other('0'),
                string_compare,
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                other('a'),
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                other('a'),
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
                other('T'),
                otherwise,
                other('F'),
                fi,
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(256).expect("bounded fuel ledger");
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut observer = InsertedRowBalanceObserver::default();
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        )
        .with_observer(&mut observer);

        assert_eq!(next_character(&mut processor), 'T');
        assert_expanded_end(&mut processor);
        assert!(processor.command.conditions.current().is_none());
        assert_eq!(processor.command.scratch.driver_continuation_depth(), 0);
        assert_eq!(processor.command.input_level_count(), 0);
        drop(processor);
        // The comparison result is one inserted expansion row. Its exact
        // parent consumes that row, and the row retires once rather than
        // entering incomplete-conditional recovery.
        assert_eq!((observer.pushes, observer.retires), (1, 1));
    });
}

#[test]
fn ifnum_literal_operands_stay_in_the_shared_delivery_lane() {
    crate::test_harness::with_universe(|universe| {
        let if_num = install(universe, "ifnum", ExpandablePrimitive::IfNum);
        let otherwise = install(universe, "else", ExpandablePrimitive::Else);
        let fi = install(universe, "fi", ExpandablePrimitive::Fi);
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [
                if_num,
                other('1'),
                other('2'),
                other('<'),
                other('2'),
                other('0'),
                other('Y'),
                otherwise,
                other('N'),
                fi,
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        assert_eq!(next_character(&mut processor), 'Y');
        assert_expanded_end(&mut processor);
        assert_eq!(processor.command.scratch.driver_continuation_depth(), 0);
    });
}

#[test]
fn deeply_nested_ifnum_operands_use_the_shared_control_lane() {
    crate::test_harness::with_universe(|universe| {
        let if_num = install(universe, "ifnum", ExpandablePrimitive::IfNum);
        let fi = install(universe, "fi", ExpandablePrimitive::Fi);
        let mut command = CommandState::default();
        let depth = 1_024;
        let mut input = Vec::with_capacity(depth * 8 + 4);
        input.extend(std::iter::repeat_n(if_num, depth));
        input.extend([
            other('1'),
            space(),
            other('<'),
            space(),
            other('2'),
            space(),
            other('1'),
        ]);
        for _ in 0..depth {
            input.extend([
                fi,
                space(),
                other('<'),
                space(),
                other('2'),
                space(),
                other('1'),
            ]);
        }
        crate::test_harness::push(&mut command, input);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        assert_eq!(next_character(&mut processor), '1');
        let mut destination = None;
        loop {
            match processor
                .get_x_token_into(&mut destination)
                .expect("deep nested ifnum delivery")
            {
                DeliveryStatus::End => break,
                DeliveryStatus::Command => {
                    destination.take().expect("command delivery");
                }
                status => panic!("unexpected delivery status: {status:?}"),
            }
        }
        assert_eq!(processor.command.scratch.driver_continuation_depth(), 0);
        assert_eq!(
            processor
                .command
                .scratch
                .recursive_delivery_entries_with_control(),
            0,
            "nested ifnum operands must not re-enter the delivery loop"
        );
        assert_eq!(
            processor.command.scratch.recursive_delivery_entries(),
            0,
            "the compact ifnum test has no nested delivery call at all"
        );
    });
}

#[test]
fn ifodd_and_ifcase_literal_operands_use_the_numeric_control_lane() {
    crate::test_harness::with_universe(|universe| {
        let if_odd = install(universe, "ifodd", ExpandablePrimitive::IfOdd);
        let if_case = install(universe, "ifcase", ExpandablePrimitive::IfCase);
        let otherwise = install(universe, "else", ExpandablePrimitive::Else);
        let or = install(universe, "or", ExpandablePrimitive::Or);
        let fi = install(universe, "fi", ExpandablePrimitive::Fi);
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [
                if_odd,
                other('3'),
                space(),
                other('Y'),
                otherwise,
                other('N'),
                fi,
                if_case,
                other('1'),
                space(),
                other('A'),
                or,
                other('B'),
                otherwise,
                other('C'),
                fi,
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        assert_eq!(next_character(&mut processor), 'Y');
        assert_eq!(next_character(&mut processor), 'B');
        assert_expanded_end(&mut processor);
        assert_eq!(processor.command.scratch.driver_continuation_depth(), 0);
    });
}

#[test]
fn ifdim_literal_operands_use_the_shared_dimension_control_lane() {
    crate::test_harness::with_universe(|universe| {
        let if_dim = install(universe, "ifdim", ExpandablePrimitive::IfDim);
        let number = install(universe, "number", ExpandablePrimitive::Number);
        let otherwise = install(universe, "else", ExpandablePrimitive::Else);
        let fi = install(universe, "fi", ExpandablePrimitive::Fi);
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [
                if_dim,
                number,
                other('1'),
                other('.'),
                other('5'),
                other('p'),
                other('t'),
                space(),
                other('<'),
                space(),
                other('2'),
                other('p'),
                other('t'),
                space(),
                other('Y'),
                otherwise,
                other('N'),
                fi,
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let mut destination = None;
        let status = processor.get_x_token_into(&mut destination);
        assert!(status.is_ok(), "ifdim delivery failed: {status:?}");
        let command = destination.expect("ifdim result command");
        assert!(
            matches!(
                command.meaning(),
                ResolvedMeaning::Static(Meaning::CharToken { ch: 'Y', .. })
            ),
            "ifdim selected unexpected command: {:?}",
            command.meaning()
        );
        assert_expanded_end(&mut processor);
        assert_eq!(processor.command.scratch.driver_continuation_depth(), 0);
        assert_eq!(
            processor
                .command
                .scratch
                .recursive_delivery_entries_with_control(),
            0,
            "ifdim and its nested number must stay in one delivery loop"
        );
    });
}

#[test]
fn ifpdfabsnum_literal_operands_use_the_shared_number_control_lane() {
    crate::test_harness::with_universe(|universe| {
        let if_abs_num = install(universe, "ifpdfabsnum", ExpandablePrimitive::IfPdfAbsNum);
        let otherwise = install(universe, "else", ExpandablePrimitive::Else);
        let fi = install(universe, "fi", ExpandablePrimitive::Fi);
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [
                if_abs_num,
                other('-'),
                other('3'),
                space(),
                other('>'),
                space(),
                other('2'),
                space(),
                other('Y'),
                otherwise,
                other('N'),
                fi,
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        assert_eq!(next_character(&mut processor), 'Y');
        assert_expanded_end(&mut processor);
        assert_eq!(processor.command.scratch.driver_continuation_depth(), 0);
        assert_eq!(
            processor
                .command
                .scratch
                .recursive_delivery_entries_with_control(),
            0,
            "ifpdfabsnum must stay in the shared delivery loop"
        );
    });
}

#[test]
fn ifpdfabsdim_literal_operands_use_the_shared_dimension_control_lane() {
    crate::test_harness::with_universe(|universe| {
        let if_abs_dim = install(universe, "ifpdfabsdim", ExpandablePrimitive::IfPdfAbsDim);
        let otherwise = install(universe, "else", ExpandablePrimitive::Else);
        let fi = install(universe, "fi", ExpandablePrimitive::Fi);
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [
                if_abs_dim,
                other('-'),
                other('3'),
                other('p'),
                other('t'),
                space(),
                other('>'),
                space(),
                other('2'),
                other('p'),
                other('t'),
                space(),
                other('Y'),
                otherwise,
                other('N'),
                fi,
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        assert_eq!(next_character(&mut processor), 'Y');
        assert_expanded_end(&mut processor);
        assert_eq!(processor.command.scratch.driver_continuation_depth(), 0);
        assert_eq!(
            processor
                .command
                .scratch
                .recursive_delivery_entries_with_control(),
            0,
            "ifpdfabsdim must stay in the shared delivery loop"
        );
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
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        assert_eq!(next_character(&mut processor), 'y');
        assert_expanded_end(&mut processor);
    });
}

#[test]
fn ifx_compares_primitives_active_characters_undefined_controls_and_character_tokens() {
    crate::test_harness::with_universe(|universe| {
        let if_true = install(universe, "iftrue", ExpandablePrimitive::IfTrue);
        let if_false = install(universe, "iffalse", ExpandablePrimitive::IfFalse);
        assert_eq!(ifx_branch(universe, if_true, if_true), 'y');
        assert_eq!(ifx_branch(universe, if_true, if_false), 'n');

        let undefined_left = Token::Cs(
            universe
                .intern("ifx-undefined-left")
                .expect("undefined control sequence")
                .symbol(),
        );
        let undefined_right = Token::Cs(
            universe
                .intern("ifx-undefined-right")
                .expect("undefined control sequence")
                .symbol(),
        );
        assert_eq!(ifx_branch(universe, undefined_left, undefined_right), 'y');

        let active_symbol = universe
            .intern_active_character('~')
            .expect("active character");
        let active_alias = universe.intern("ifx-active-alias").expect("active alias");
        for symbol in [active_symbol, active_alias] {
            universe
                .assign_meaning(
                    symbol,
                    MeaningWord::from_static(Meaning::CharGiven('A')),
                    AssignmentScope::Global,
                )
                .expect("active meaning");
        }
        let active = Token::Char {
            ch: '~',
            cat: Catcode::Active,
        };
        assert_eq!(
            ifx_branch(universe, active, Token::Cs(active_alias.symbol())),
            'y'
        );

        let letter_a = Token::Char {
            ch: 'a',
            cat: Catcode::Letter,
        };
        assert_eq!(ifx_branch(universe, other('a'), other('a')), 'y');
        assert_eq!(ifx_branch(universe, other('a'), other('b')), 'n');
        assert_eq!(ifx_branch(universe, other('a'), letter_a), 'n');
    });
}

#[test]
fn ifx_macro_equality_uses_flags_and_borrowed_token_content() {
    crate::test_harness::with_universe(|universe| {
        let parameter = [TokenWord::pack(other('p'))];
        let replacement = [TokenWord::pack(other('r'))];
        let first = ResolvedMeaning::Macro {
            flags: MeaningFlags::LONG,
            definition: universe
                .allocate_definition(&parameter, &replacement)
                .expect("first definition"),
        };
        let equal = ResolvedMeaning::Macro {
            flags: MeaningFlags::LONG,
            definition: universe
                .allocate_definition(&parameter, &replacement)
                .expect("equal definition with distinct identity"),
        };
        let different_flags = ResolvedMeaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: universe
                .allocate_definition(&parameter, &replacement)
                .expect("definition with different flags"),
        };
        let different_parameter = ResolvedMeaning::Macro {
            flags: MeaningFlags::LONG,
            definition: universe
                .allocate_definition(&[TokenWord::pack(other('q'))], &replacement)
                .expect("definition with different parameter text"),
        };
        let different_replacement = ResolvedMeaning::Macro {
            flags: MeaningFlags::LONG,
            definition: universe
                .allocate_definition(&parameter, &[TokenWord::pack(other('s'))])
                .expect("definition with different replacement text"),
        };

        let mut command = CommandState::default();
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        assert!(processor.ifx_meaning_eq(&first, &equal));
        assert!(!processor.ifx_meaning_eq(&first, &different_flags));
        assert!(!processor.ifx_meaning_eq(&first, &different_parameter));
        assert!(!processor.ifx_meaning_eq(&first, &different_replacement));
        let undefined = ResolvedMeaning::Static(Meaning::Undefined);
        assert!(!processor.ifx_meaning_eq(&undefined, &first));
        assert!(processor.ifx_meaning_eq(&undefined, &undefined));
    });
}

#[test]
fn extra_delimiter_recovery_keeps_following_input_and_owns_its_diagnostic() {
    crate::test_harness::with_universe(|universe| {
        let or = install(universe, "or", ExpandablePrimitive::Or);
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [or, other('t')]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        {
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            assert_eq!(next_character(&mut processor), 't');
        }
        let diagnostics = command.take_semantic_diagnostics();
        assert!(matches!(
            diagnostics.as_slice(),
            [CommandSemanticDiagnostic::Recoverable { message, .. }]
                if message.starts_with("Extra ")
        ));
        assert!(command.take_semantic_diagnostics().is_empty());
    });
}
