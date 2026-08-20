use tex_state::Universe;
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
use tex_state::token::{OriginId, Token, TracedTokenWord};

use super::*;

#[test]
fn etex_current_if_type_is_one_based_and_preserves_unless_sign() {
    // e-TeX 2.6 `etex.ch` [17.4750--4790] returns `cur_if+1`, or the
    // corresponding negative value after the `unless_code` prefix.
    let cases = [
        (ConditionalKind::If, 1),
        (ConditionalKind::IfCat, 2),
        (ConditionalKind::IfNum, 3),
        (ConditionalKind::IfDim, 4),
        (ConditionalKind::IfOdd, 5),
        (ConditionalKind::IfVMode, 6),
        (ConditionalKind::IfHMode, 7),
        (ConditionalKind::IfMMode, 8),
        (ConditionalKind::IfInner, 9),
        (ConditionalKind::IfVoid, 10),
        (ConditionalKind::IfHBox, 11),
        (ConditionalKind::IfVBox, 12),
        (ConditionalKind::IfX, 13),
        (ConditionalKind::IfEof, 14),
        (ConditionalKind::IfTrue, 15),
        (ConditionalKind::IfFalse, 16),
        (ConditionalKind::IfCase, 17),
        (ConditionalKind::IfDefined, 18),
        (ConditionalKind::IfCsName, 19),
        (ConditionalKind::IfFontChar, 20),
        (ConditionalKind::IfInCsName, 21),
    ];

    assert_eq!(ConditionStack::default().current_etex_values(), (0, 0, 0));
    for (kind, expected) in cases {
        for (inverted, signed) in [(false, expected), (true, -expected)] {
            let mut stack = ConditionStack::default();
            stack.push_with_inversion(kind, 0, inverted);
            assert_eq!(stack.current_etex_values(), (1, signed, 0), "{kind:?}");
        }
    }
}

#[test]
fn etex_current_if_branch_ignores_unless_inversion() {
    // e-TeX 2.6 `etex.ch` [17.4750--4790] derives `\currentifbranch` only
    // from `if_limit`; unlike `\currentiftype`, it does not inspect the
    // `unless_code` carried by `cur_if`.
    for (limit, expected) in [
        (IfLimit::Evaluating, 0),
        (IfLimit::Or, 1),
        (IfLimit::Else, 1),
        (IfLimit::Fi, -1),
    ] {
        for inverted in [false, true] {
            let mut stack = ConditionStack::default();
            let condition = stack.push_with_inversion(ConditionalKind::IfTrue, 0, inverted);
            assert!(stack.change_if_limit(condition, limit));
            assert_eq!(
                stack.current_etex_values(),
                (1, if inverted { -15 } else { 15 }, expected)
            );
        }
    }
}
use crate::input::{
    ReplayTrace, RetirementBehavior, SharedTokenBuffer, TokenBehavior, TokenPayload,
};
use crate::processor::status::{AbsorbingContext, DefinitionContext, TokenBuilderId};
use crate::{
    CommandHostCapabilities, CommandHostContext, CommandObservation, CommandObserver, CommandState,
    ConditionalMode, ConditionalState,
};

#[derive(Default)]
struct Recorder(Vec<CommandObservation>);

impl CommandObserver for Recorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

fn processor<'a>(
    command: &'a mut CommandState,
    universe: &'a mut Universe,
    capabilities: &'a mut CommandHostCapabilities,
) -> CommandProcessor<'a> {
    CommandProcessor::new(
        command,
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

fn macro_token(
    universe: &mut Universe,
    name: &str,
    flags: MeaningFlags,
    parameter_text: &[Token],
    replacement_text: &[Token],
) -> Token {
    let symbol = universe.intern(name).symbol();
    let parameter_text = universe.intern_token_list(parameter_text);
    let replacement_text = universe.intern_token_list(replacement_text);
    universe.set_macro_meaning(
        symbol,
        MacroMeaning::new(flags, parameter_text, replacement_text),
    );
    Token::Cs(symbol)
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
fn final_cleanup_drains_nested_kinds_in_current_first_order_with_saved_lines() {
    let mut stack = ConditionStack::default();
    let outer = stack.push(ConditionalKind::IfTrue, 11);
    assert!(stack.change_if_limit(outer, IfLimit::Else));
    let middle = stack.push(ConditionalKind::IfCase, 23);
    assert!(stack.change_if_limit(middle, IfLimit::Or));
    stack.push(ConditionalKind::IfNum, 37);

    let incomplete = stack.drain_incomplete();

    assert_eq!(
        incomplete
            .iter()
            .map(|condition| (condition.kind_name(), condition.source_line()))
            .collect::<Vec<_>>(),
        [("ifnum", 37), ("ifcase", 23), ("iftrue", 11)]
    );
    assert!(stack.current().is_none());
    assert!(stack.drain_incomplete().is_empty());
}

#[test]
fn ordinary_pop_restores_the_outer_current_kind_line_and_limit() {
    let mut stack = ConditionStack::default();
    let outer = stack.push(ConditionalKind::IfDim, 41);
    assert!(stack.change_if_limit(outer, IfLimit::Else));
    stack.push(ConditionalKind::IfX, 43);

    let popped = stack.pop().expect("inner frame");

    assert_eq!(popped.kind, ConditionalKind::IfX);
    let restored = stack.current().expect("outer frame is current again");
    assert_eq!(restored.kind, ConditionalKind::IfDim);
    assert_eq!(restored.source_line, 41);
    assert_eq!(restored.limit, IfLimit::Else);
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
    let mut universe = crate::test_harness::universe();
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
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

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
    // §331 starts `align_state` at 1000000; the skipped text left one
    // unmatched literal left brace above that running base.
    assert_eq!(
        processor.command.alignment.align_state,
        crate::processor::TOP_LEVEL_ALIGN_STATE + 1
    );
}

#[test]
fn pass_text_only_accepts_or_when_the_frame_limit_allows_it() {
    let mut command = CommandState::default();
    let condition = command.conditions.push(ConditionalKind::IfCase, 1);
    assert!(command.conditions.change_if_limit(condition, IfLimit::Or));
    let mut universe = crate::test_harness::universe();
    let or = install(&mut universe, "or", ExpandablePrimitive::Or);
    push(&mut command, vec![or]);
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(
        processor.pass_text(condition, ScannerWarning(7)),
        Ok(PassTextStop {
            delimiter: ConditionalDelimiter::Or,
            nested_conditions: 0,
        })
    );
}

fn other(ch: char) -> Token {
    Token::Char {
        ch,
        cat: tex_state::token::Catcode::Other,
    }
}

fn next_character(processor: &mut CommandProcessor<'_>) -> char {
    match processor
        .get_x_token()
        .expect("conditional expansion succeeds")
        .expect("conditional selects a limb")
        .meaning()
    {
        Meaning::CharToken { ch, .. } => ch,
        meaning => panic!("expected selected character, got {meaning:?}"),
    }
}

fn chars(text: &str) -> Vec<Token> {
    text.chars().map(other).collect()
}

fn boxed(universe: &mut Universe, vertical: bool) -> tex_state::node_arena::NodeListRef {
    boxed_with_dimensions(
        universe,
        vertical,
        tex_state::scaled::Scaled::from_raw(0),
        tex_state::scaled::Scaled::from_raw(0),
        tex_state::scaled::Scaled::from_raw(0),
    )
}

fn boxed_with_dimensions(
    universe: &mut Universe,
    vertical: bool,
    width: tex_state::scaled::Scaled,
    height: tex_state::scaled::Scaled,
    depth: tex_state::scaled::Scaled,
) -> tex_state::node_arena::NodeListRef {
    use tex_state::glue::Order;
    use tex_state::node::{BoxNode, BoxNodeFields, Node, Sign};
    use tex_state::scaled::GlueSetRatio;

    let children = universe.freeze_node_list(&[]);
    let node = BoxNode::new(BoxNodeFields {
        width,
        height,
        depth,
        shift: tex_state::scaled::Scaled::from_raw(0),
        box_lr: tex_state::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    });
    universe.freeze_node_list(&[if vertical {
        Node::VList(node)
    } else {
        Node::HList(node)
    }])
}

#[test]
fn boolean_condition_skips_false_limb_and_else_skips_true_remainder() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let if_false = install(&mut universe, "iffalse", ExpandablePrimitive::IfFalse);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    push(
        &mut command,
        vec![if_false, other('f'), otherwise, other('t'), fi],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 't');
    let frame = processor
        .command
        .conditions
        .current()
        .expect("else limb retains its condition frame");
    assert_eq!(frame.limit, IfLimit::Fi);
    assert!(processor.get_x_token().expect("fi expands").is_none());
    assert!(processor.command.conditions.current().is_none());
}

#[test]
fn ifx_reads_unexpanded_operands_through_get_token() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let if_x = install(&mut universe, "ifx", ExpandablePrimitive::IfX);
    let if_true = install(&mut universe, "iftrue", ExpandablePrimitive::IfTrue);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    push(
        &mut command,
        vec![
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
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    // If the operands were delivered with get_x_token, either `iftrue` would
    // open a nested condition or the following text would be consumed.
    assert_eq!(next_character(&mut processor), 'y');
}

#[test]
fn ifx_temporarily_normalizes_an_absorbing_scanner() {
    // TeX82 §507 saves `scanner_status`, assigns `normal` across both
    // `get_next` operand deliveries, and restores the saved status. This is
    // observable when an expanded token-list scan encounters `\ifx`.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let if_x = install(&mut universe, "ifx", ExpandablePrimitive::IfX);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    let first = macro_token(
        &mut universe,
        "ifx-absorbing-first",
        MeaningFlags::EMPTY,
        &[],
        &[],
    );
    let second = macro_token(
        &mut universe,
        "ifx-absorbing-second",
        MeaningFlags::EMPTY,
        &[],
        &[],
    );
    push(
        &mut command,
        vec![if_x, first, second, other('y'), otherwise, other('n'), fi],
    );
    let absorbing = ScannerStatus::Absorbing(AbsorbingContext {
        owner: None,
        builder: TokenBuilderId(1),
        warning: ScannerWarning(1),
    });
    let _prior = command.begin_scanner_status(absorbing.clone());
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        assert_eq!(next_character(&mut processor), 'y');
        assert_eq!(processor.command.scanner.status(), &absorbing);
    }
    let transitions = recorder
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::ScannerStatus(record) => Some((record.from, record.to)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        transitions,
        vec![("absorbing", "normal"), ("normal", "absorbing")]
    );
}

#[test]
fn ifx_in_normal_scanner_status_publishes_no_status_transition() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let if_x = install(&mut universe, "ifx", ExpandablePrimitive::IfX);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    push(
        &mut command,
        vec![
            if_x,
            other('q'),
            other('q'),
            other('y'),
            otherwise,
            other('n'),
            fi,
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        assert_eq!(next_character(&mut processor), 'y');
    }
    assert!(
        !recorder
            .0
            .iter()
            .any(|observation| matches!(observation, CommandObservation::ScannerStatus(_)))
    );
}

#[test]
fn true_ifx_fi_observes_branch_before_popping_its_frame() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let if_x = install(&mut universe, "ifx", ExpandablePrimitive::IfX);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    let samea = macro_token(
        &mut universe,
        "samea",
        MeaningFlags::EMPTY,
        &[],
        &[other('s')],
    );
    let sameb = macro_token(
        &mut universe,
        "sameb",
        MeaningFlags::EMPTY,
        &[],
        &[other('s')],
    );
    push(
        &mut command,
        vec![if_x, samea, sameb, other('y'), otherwise, other('n'), fi],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);

        assert_eq!(next_character(&mut processor), 'y');
        assert!(
            processor
                .get_x_token()
                .expect("else skips through its matching fi")
                .is_none()
        );
    }

    assert!(recorder.0.windows(2).any(|pair| {
        matches!(
            &pair,
            [
                CommandObservation::Condition(branch),
                CommandObservation::Condition(pop),
            ] if branch.transition == "branch"
                && branch.condition == "ifx"
                && branch.limit == "else"
                && branch.branch.as_deref() == Some("fi")
                && pop.transition == "pop"
                && pop.condition == "ifx"
                && pop.limit == "else"
        )
    }));
}

#[test]
fn ifx_compares_macro_flags_and_raw_definition_tokens_not_storage_identity() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let if_x = install(&mut universe, "ifx", ExpandablePrimitive::IfX);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    let samea = macro_token(
        &mut universe,
        "samea",
        MeaningFlags::EMPTY,
        &[],
        &[other('s'), other('a'), other('m'), other('e')],
    );
    let sameb = macro_token(
        &mut universe,
        "sameb",
        MeaningFlags::EMPTY,
        &[],
        &[other('s'), other('a'), other('m'), other('e')],
    );
    let long_same = macro_token(
        &mut universe,
        "longsame",
        MeaningFlags::LONG,
        &[],
        &[other('s'), other('a'), other('m'), other('e')],
    );
    push(
        &mut command,
        vec![
            if_x,
            samea,
            sameb,
            other('y'),
            otherwise,
            other('n'),
            fi,
            if_x,
            samea,
            long_same,
            other('y'),
            otherwise,
            other('n'),
            fi,
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 'y');
    assert_eq!(next_character(&mut processor), 'n');
}

#[test]
fn character_and_category_tests_normalize_non_character_operands() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let if_char = install(&mut universe, "if", ExpandablePrimitive::If);
    let if_cat = install(&mut universe, "ifcat", ExpandablePrimitive::IfCat);
    let relax = universe.intern("relax").symbol();
    universe.set_meaning(relax, Meaning::Relax);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    push(
        &mut command,
        vec![
            if_char,
            Token::Cs(relax),
            Token::Cs(relax),
            other('c'),
            otherwise,
            other('x'),
            fi,
            if_cat,
            Token::Cs(relax),
            Token::Cs(relax),
            other('k'),
            otherwise,
            other('x'),
            fi,
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 'c');
    assert_eq!(next_character(&mut processor), 'k');
}

#[test]
fn skipped_text_recovers_extra_delimiters_deterministically() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let if_false = install(&mut universe, "iffalse", ExpandablePrimitive::IfFalse);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let or = install(&mut universe, "or", ExpandablePrimitive::Or);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    push(
        &mut command,
        vec![
            if_false,
            other('f'),
            or,
            other('o'),
            otherwise,
            other('t'),
            fi,
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 't');
    assert_eq!(processor.command.expansion.pending_diagnostics.len(), 1);
    assert_eq!(
        processor.get_x_token().expect("final fi expands"),
        None,
        "the recovery remains confined to the skipped limb"
    );
}

#[test]
fn extra_delimiter_is_observed_at_its_raw_delivery() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let or = install(&mut universe, "or", ExpandablePrimitive::Or);
    push(&mut command, vec![or]);
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        assert_eq!(
            processor.get_x_token().expect("extra delimiter is ignored"),
            None
        );
    }

    assert!(recorder.0.windows(2).any(|pair| {
        matches!(
            pair,
            [
                CommandObservation::Command(raw),
                CommandObservation::Diagnostic(diagnostic),
            ] if raw.boundary == crate::CommandDeliveryBoundary::Raw
                && diagnostic.severity == "error"
                && diagnostic.diagnostic == "conditional_extra_delimiter"
                && diagnostic.arguments.is_empty()
        )
    }));
    assert_eq!(
        command.expansion.pending_diagnostics,
        [EXTRA_DELIMITER_DIAGNOSTIC]
    );
}

#[test]
fn delimiter_during_operand_scan_replays_each_missing_if_operand() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let if_char = install(&mut universe, "if", ExpandablePrimitive::If);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    universe.install_primitive_meaning("relax", Meaning::Relax);
    push(&mut command, vec![if_char, otherwise, other('a'), fi]);
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(
        processor
            .get_x_token()
            .expect("incomplete conditional recovers"),
        None
    );
    // `\if` has two operands. Replaying the delimiter below each inserted
    // frozen relax therefore reports the canonical incomplete-if recovery
    // twice instead of silently losing the second operand boundary.
    assert_eq!(processor.command.expansion.pending_diagnostics.len(), 2);
    assert!(processor.command.conditions.current().is_none());
}

#[test]
fn unless_reuses_boolean_conditional_evaluation_with_inversion() {
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut universe = crate::test_harness::universe();
    let unless = install(&mut universe, "unless", ExpandablePrimitive::Unless);
    let if_false = install(&mut universe, "iffalse", ExpandablePrimitive::IfFalse);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    push(
        &mut command,
        vec![unless, if_false, other('y'), otherwise, other('n'), fi],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        assert_eq!(next_character(&mut processor), 'y');
    }
    let transitions: Vec<_> = recorder
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Condition(record) => Some((
                record.transition,
                record.condition.as_str(),
                record.limit,
                record.branch.as_deref(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        transitions,
        [
            ("push", "unless_iffalse", "evaluating", None),
            ("branch", "iffalse", "evaluating", Some("true")),
            ("limit", "unless_iffalse", "else", None),
        ]
    );
}

#[test]
fn tracingcommands_two_prints_unless_with_its_boolean_operand() {
    // e-TeX 2.6 merged §28.498 carries `unless_code` on the following
    // boolean `if_test`, so §367 prints one combined command before §502's
    // boolean result rather than tracing the prefix independently.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut universe = crate::test_harness::universe();
    universe.set_int_param(tex_state::env::banks::IntParam::TRACING_COMMANDS, 2);
    let unless = install(&mut universe, "unless", ExpandablePrimitive::Unless);
    let if_false = install(&mut universe, "iffalse", ExpandablePrimitive::IfFalse);
    push(&mut command, vec![unless, if_false, other('y')]);
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 'y');
    drop(processor);

    let diagnostics = diagnostic_text(&universe);
    assert!(
        diagnostics.contains("{\\unless}\n{\\unless\\iffalse}\n{true}"),
        "{diagnostics:?}"
    );
}

#[test]
fn malformed_unless_character_is_diagnosed_and_replayed_in_extended_profiles() {
    // e-TeX 2.6's merged change [28.498] accepts only a boolean `if_test`
    // after `\unless`. Its `back_error` path diagnoses the prefix itself,
    // leaves the operand as following input, and creates no condition frame.
    for profile in [
        crate::CommandProfile::ETEX26,
        crate::CommandProfile::PDFTEX14029,
    ] {
        let mut command = CommandState::new(profile);
        let mut universe = crate::test_harness::universe();
        let unless = install(&mut universe, "unless", ExpandablePrimitive::Unless);
        push(&mut command, vec![unless, letter('x'), other('z')]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities)
                .with_observer(&mut recorder);
            assert_eq!(next_character(&mut processor), 'x');
            assert!(processor.command.conditions.current().is_none());
            assert_eq!(next_character(&mut processor), 'z');
        }
        assert_eq!(
            command.expansion.pending_diagnostics,
            [ILLEGAL_UNLESS_OPERAND_DIAGNOSTIC]
        );
        let [
            crate::CommandSemanticDiagnostic::Recoverable {
                identity,
                message,
                help,
                context,
                ..
            },
        ] = command.semantic_diagnostics.as_slice()
        else {
            panic!("expected one recoverable unless diagnostic")
        };
        assert_eq!(*identity, ILLEGAL_UNLESS_OPERAND_DIAGNOSTIC);
        assert_eq!(message, "You can't use `\\unless' before `the letter x'.");
        assert_eq!(*help, ["I'll pretend you didn't say \\unless."]);
        assert!(context.contains("<to be read again>"));
        assert!(context.contains('x'));
        assert!(recorder.0.iter().any(|observation| matches!(
            observation,
            CommandObservation::Diagnostic(diagnostic)
                if diagnostic.diagnostic == "illegal_unless_operand"
                    && diagnostic.arguments
                        == [crate::DiagnosticArgument::Token(ObservedToken::Character {
                            character: 'x',
                            catcode: tex_state::token::Catcode::Letter,
                        })]
        )));
    }
}

#[test]
fn malformed_unless_ifcase_is_replayed_as_an_ordinary_conditional() {
    // The same merged e-TeX change [28.498] singles out `if_case_code`: it is
    // illegal as the prefix operand but is backed up and then executes in its
    // ordinary, non-inverted form.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut universe = crate::test_harness::universe();
    let unless = install(&mut universe, "unless", ExpandablePrimitive::Unless);
    let if_case = install(&mut universe, "ifcase", ExpandablePrimitive::IfCase);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    push(
        &mut command,
        vec![unless, if_case, other('0'), other('a'), fi, other('z')],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 'a');
    assert!(processor.command.conditions.current().is_some());
    assert_eq!(next_character(&mut processor), 'z');
    assert!(processor.command.conditions.current().is_none());
    assert_eq!(
        processor.command.expansion.pending_diagnostics,
        [ILLEGAL_UNLESS_OPERAND_DIAGNOSTIC]
    );
    let [crate::CommandSemanticDiagnostic::Recoverable { message, help, .. }] =
        processor.command.semantic_diagnostics.as_slice()
    else {
        panic!("expected one recoverable unless diagnostic")
    };
    assert_eq!(message, "You can't use `\\unless' before `\\ifcase'.");
    assert_eq!(*help, ["I'll pretend you didn't say \\unless."]);
}

#[test]
fn numeric_and_ifcase_selection_use_the_same_skip_machine() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let if_num = install(&mut universe, "ifnum", ExpandablePrimitive::IfNum);
    let if_case = install(&mut universe, "ifcase", ExpandablePrimitive::IfCase);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let or = install(&mut universe, "or", ExpandablePrimitive::Or);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    push(
        &mut command,
        vec![
            if_num,
            other('2'),
            other('<'),
            other('3'),
            other('y'),
            otherwise,
            other('n'),
            fi,
            if_case,
            other('1'),
            other('z'),
            or,
            other('1'),
            otherwise,
            other('e'),
            fi,
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 'y');
    assert_eq!(next_character(&mut processor), '1');
    assert!(
        processor
            .get_x_token()
            .expect("ifcase else is skipped")
            .is_none()
    );
}

fn diagnostic_text(universe: &Universe) -> String {
    universe
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            tex_state::EffectRecord::StreamWrite { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn tracingcommands_two_reports_true_and_false_boolean_results() {
    // TeX82 §502 prints the predicate value after evaluation and before
    // entering the selected limb. Both outcomes use the shared §245
    // diagnostic channel.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    universe.set_int_param(tex_state::env::banks::IntParam::TRACING_COMMANDS, 2);
    let if_true = install(&mut universe, "iftrue", ExpandablePrimitive::IfTrue);
    let if_false = install(&mut universe, "iffalse", ExpandablePrimitive::IfFalse);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    push(
        &mut command,
        vec![
            if_true,
            other('t'),
            otherwise,
            other('x'),
            fi,
            if_false,
            other('x'),
            otherwise,
            other('f'),
            fi,
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 't');
    assert_eq!(next_character(&mut processor), 'f');
    assert!(processor.get_x_token().expect("input exhausts").is_none());
    drop(processor);

    let diagnostics = diagnostic_text(&universe);
    assert!(
        diagnostics.contains("{\\iftrue}\n{true}"),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics.contains("{\\iffalse}\n{false}"),
        "{diagnostics:?}"
    );
}

#[test]
fn tracingcommands_two_reports_ifcase_selection_before_limb_skips() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    universe.set_int_param(tex_state::env::banks::IntParam::TRACING_COMMANDS, 2);
    let if_case = install(&mut universe, "ifcase", ExpandablePrimitive::IfCase);
    let or = install(&mut universe, "or", ExpandablePrimitive::Or);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    push(
        &mut command,
        vec![
            if_case,
            other('2'),
            other('a'),
            or,
            other('b'),
            or,
            other('c'),
            fi,
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 'c');
    assert!(processor.get_x_token().expect("input exhausts").is_none());
    drop(processor);

    let diagnostics = diagnostic_text(&universe);
    assert!(
        diagnostics.contains("{\\ifcase}\n{case 2}"),
        "{diagnostics:?}"
    );
}

#[test]
fn tracingcommands_one_or_less_omits_boolean_results() {
    // TeX82 §502 uses the strict `tracing_commands>1` threshold.
    for level in [0, 1] {
        let mut command = CommandState::default();
        let mut universe = crate::test_harness::universe();
        universe.set_int_param(tex_state::env::banks::IntParam::TRACING_COMMANDS, level);
        let if_true = install(&mut universe, "iftrue", ExpandablePrimitive::IfTrue);
        let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
        push(&mut command, vec![if_true, other('t'), fi]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);

        assert_eq!(next_character(&mut processor), 't');
        assert!(processor.get_x_token().expect("input exhausts").is_none());
        drop(processor);

        let diagnostics = diagnostic_text(&universe);
        assert!(
            !diagnostics.contains("{true}"),
            "level={level}: {diagnostics:?}"
        );
        assert!(
            !diagnostics.contains("{false}"),
            "level={level}: {diagnostics:?}"
        );
    }
}

#[test]
fn boolean_result_trace_uses_tracingonline_diagnostic_routing() {
    // TeX82 §§245/502: the result uses `begin_diagnostic`, so nonpositive
    // `\tracingonline` redirects it to the log and a positive value retains
    // the terminal-and-log selector.
    for (tracing_online, expected_sink) in [
        (0, tex_state::PrintSink::Log),
        (1, tex_state::PrintSink::TerminalAndLog),
    ] {
        let mut command = CommandState::default();
        let mut universe = crate::test_harness::universe();
        universe.set_int_param(tex_state::env::banks::IntParam::TRACING_COMMANDS, 2);
        universe.set_int_param(
            tex_state::env::banks::IntParam::TRACING_ONLINE,
            tracing_online,
        );
        let if_true = install(&mut universe, "iftrue", ExpandablePrimitive::IfTrue);
        push(&mut command, vec![if_true, other('t')]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);

        assert_eq!(next_character(&mut processor), 't');
        drop(processor);

        assert!(universe.world().effect_records().iter().any(|effect| {
            matches!(
                effect,
                tex_state::EffectRecord::StreamWrite { sink, text }
                    if *sink == expected_sink && text.contains("{true}")
            )
        }));
    }
}

#[test]
fn ifcase_observes_its_limit_only_after_skipping_to_the_selected_limb() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let if_case = install(&mut universe, "ifcase", ExpandablePrimitive::IfCase);
    let or = install(&mut universe, "or", ExpandablePrimitive::Or);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    push(
        &mut command,
        vec![
            if_case,
            other('2'),
            other('a'),
            or,
            other('b'),
            or,
            other('c'),
            fi,
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        assert_eq!(next_character(&mut processor), 'c');
    }

    let conditions = recorder
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Condition(condition) => Some(condition),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(matches!(
        conditions.as_slice(),
        [push, first_or, second_or, limit, branch]
            if push.transition == "push"
                && push.limit == "evaluating"
                && first_or.transition == "branch"
                && first_or.limit == "evaluating"
                && first_or.branch.as_deref() == Some("or")
                && second_or.transition == "branch"
                && second_or.limit == "evaluating"
                && second_or.branch.as_deref() == Some("or")
                && limit.transition == "limit"
                && limit.limit == "or"
                && branch.transition == "branch"
                && branch.limit == "or"
                && branch.branch.as_deref() == Some("case")
    ));
}

#[test]
fn ifdim_uses_typed_units_and_internal_dimensions() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let ifdim = install(&mut universe, "ifdim", ExpandablePrimitive::IfDim);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    let measured = universe.intern("measured").symbol();
    universe.set_meaning(measured, Meaning::DimenRegister(7));
    universe.set_dimen(
        7,
        tex_state::scaled::Scaled::from_raw(2 * tex_state::scaled::Scaled::UNITY),
    );
    let mut tokens = vec![ifdim];
    tokens.extend(chars("1in>72pt"));
    tokens.extend([
        other('y'),
        otherwise,
        other('n'),
        fi,
        ifdim,
        Token::Cs(measured),
    ]);
    tokens.extend(chars("=2pt"));
    tokens.extend([other('i'), otherwise, other('n'), fi]);
    push(&mut command, tokens);
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 'y');
    assert_eq!(next_character(&mut processor), 'i');
}

#[test]
fn ifdim_scans_box_dimensions_as_internal_dimensions() {
    use tex_state::meaning::UnexpandablePrimitive;
    use tex_state::scaled::Scaled;

    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let ifdim = install(&mut universe, "ifdim", ExpandablePrimitive::IfDim);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    let wd = universe.intern("wd").symbol();
    let ht = universe.intern("ht").symbol();
    let dp = universe.intern("dp").symbol();
    universe.set_meaning(
        wd,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Wd),
    );
    universe.set_meaning(
        ht,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Ht),
    );
    universe.set_meaning(
        dp,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dp),
    );
    let box_node = boxed_with_dimensions(
        &mut universe,
        false,
        Scaled::from_raw(Scaled::UNITY),
        Scaled::from_raw(2 * Scaled::UNITY),
        Scaled::from_raw(3 * Scaled::UNITY),
    );
    universe.set_box_reg_ref(3, box_node);

    let mut tokens = Vec::new();
    for (primitive, expected, selected) in [(wd, "1pt", 'w'), (ht, "2pt", 'h'), (dp, "3pt", 'd')] {
        tokens.extend([ifdim, Token::Cs(primitive)]);
        tokens.extend(chars(&format!("3={expected}")));
        tokens.extend([other(selected), otherwise, other('n'), fi]);
    }
    push(&mut command, tokens);
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 'w');
    assert_eq!(next_character(&mut processor), 'h');
    assert_eq!(next_character(&mut processor), 'd');
}

#[test]
fn ifdim_box_dimension_accepts_a_dimension_register_selector() {
    use tex_state::font::NULL_FONT;
    use tex_state::meaning::UnexpandablePrimitive;
    use tex_state::scaled::Scaled;

    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let ifdim = install(&mut universe, "ifdim", ExpandablePrimitive::IfDim);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    let ht = universe.intern("ht").symbol();
    let zero_dimension = universe.intern("z@").symbol();
    universe.set_meaning(
        ht,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Ht),
    );
    universe.set_meaning(zero_dimension, Meaning::DimenRegister(12));
    universe.set_dimen(12, Scaled::from_raw(0));
    universe
        .set_font_dimen(NULL_FONT, 5, Scaled::from_raw(2 * Scaled::UNITY))
        .expect("nullfont has an x-height parameter");
    let box_node = boxed_with_dimensions(
        &mut universe,
        false,
        Scaled::from_raw(0),
        Scaled::from_raw(2 * Scaled::UNITY),
        Scaled::from_raw(0),
    );
    universe.set_box_reg_ref(0, box_node);
    push(
        &mut command,
        vec![
            ifdim,
            Token::Cs(ht),
            Token::Cs(zero_dimension),
            other('='),
            other('1'),
            other('e'),
            other('x'),
            other('y'),
            otherwise,
            other('n'),
            fi,
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 'y');
}

#[test]
fn mode_and_box_predicates_use_host_and_aggregate_queries() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let if_hmode = install(&mut universe, "ifhmode", ExpandablePrimitive::IfHMode);
    let if_inner = install(&mut universe, "ifinner", ExpandablePrimitive::IfInner);
    let if_void = install(&mut universe, "ifvoid", ExpandablePrimitive::IfVoid);
    let if_hbox = install(&mut universe, "ifhbox", ExpandablePrimitive::IfHBox);
    let if_vbox = install(&mut universe, "ifvbox", ExpandablePrimitive::IfVBox);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    let hbox = boxed(&mut universe, false);
    let vbox = boxed(&mut universe, true);
    universe.set_box_reg_ref(1, hbox);
    universe.set_box_reg_ref(2, vbox);
    push(
        &mut command,
        vec![
            if_hmode,
            other('h'),
            otherwise,
            other('x'),
            fi,
            if_inner,
            other('i'),
            otherwise,
            other('x'),
            fi,
            if_void,
            other('0'),
            other('v'),
            otherwise,
            other('x'),
            fi,
            if_hbox,
            other('1'),
            other('h'),
            otherwise,
            other('x'),
            fi,
            if_vbox,
            other('2'),
            other('b'),
            otherwise,
            other('x'),
            fi,
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    capabilities.set_conditional_state(ConditionalState::new(ConditionalMode::Horizontal, true));
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    for expected in ['h', 'i', 'v', 'h', 'b'] {
        assert_eq!(next_character(&mut processor), expected);
    }
    assert!(processor.get_x_token().expect("final fi expands").is_none());
}

#[test]
fn selected_ifcase_limb_skips_remaining_limbs_without_extra_delimiter_errors() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let if_case = install(&mut universe, "ifcase", ExpandablePrimitive::IfCase);
    let or = install(&mut universe, "or", ExpandablePrimitive::Or);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    // `\ifcase0 a\or b\or c\or d\fi e` selects the *first* of four limbs, so
    // TeX.web §510's `while cur_chr<>fi_code do pass_text` passes over two
    // further `\or` delimiters before the matching `\fi`. Neither is a
    // diagnostic: §510 never inspects which delimiter the skip stopped at.
    push(
        &mut command,
        vec![
            if_case,
            other('0'),
            other('a'),
            or,
            other('b'),
            or,
            other('c'),
            or,
            other('d'),
            fi,
            other('e'),
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);

        assert_eq!(next_character(&mut processor), 'a');
        assert_eq!(next_character(&mut processor), 'e');
        assert!(processor.command.conditions.current().is_none());
    }

    assert!(command.expansion.pending_diagnostics.is_empty());
    assert!(!recorder.0.iter().any(|observation| matches!(
        observation,
        CommandObservation::Diagnostic(diagnostic)
            if diagnostic.diagnostic == "conditional_extra_delimiter"
    )));
}

/// Defines `ch` as an active character with an expandable (macro) meaning,
/// returning the active-character token that spells it in a token list.
fn active_macro(universe: &mut Universe, ch: char) -> Token {
    let symbol = universe.intern_active_character(ch).symbol();
    let parameter_text = universe.intern_token_list(&[]);
    let replacement_text = universe.intern_token_list(&[other('!')]);
    universe.set_macro_meaning(
        symbol,
        MacroMeaning::new(MeaningFlags::EMPTY, parameter_text, replacement_text),
    );
    Token::Char {
        ch,
        cat: tex_state::token::Catcode::Active,
    }
}

#[test]
fn noexpand_before_an_active_character_compares_as_that_character() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let if_char = install(&mut universe, "if", ExpandablePrimitive::If);
    let if_cat = install(&mut universe, "ifcat", ExpandablePrimitive::IfCat);
    let no_expand = install(&mut universe, "noexpand", ExpandablePrimitive::NoExpand);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    let tilde = active_macro(&mut universe, '~');
    let at = active_macro(&mut universe, '@');
    let ordinary = macro_token(
        &mut universe,
        "foo",
        MeaningFlags::EMPTY,
        &[],
        &[other('!')],
    );
    // TeX.web §506's `get_x_token_or_active_char` rebuilds `cur_cmd` and
    // `cur_chr` from the retained token, so a `\noexpand`ed active character
    // compares as category 13 with its own character code instead of as the
    // shared `relax`/256 non-character sentinel.
    push(
        &mut command,
        vec![
            // Two distinct active characters share category 13.
            if_cat,
            no_expand,
            tilde,
            no_expand,
            at,
            other('s'),
            otherwise,
            other('d'),
            fi,
            // An active character does not share a category with an ordinary
            // `\noexpand`ed control sequence, which stays the sentinel.
            if_cat,
            no_expand,
            tilde,
            no_expand,
            ordinary,
            other('s'),
            otherwise,
            other('d'),
            fi,
            // Character codes are compared, so two different active
            // characters differ under `\if`.
            if_char,
            no_expand,
            tilde,
            no_expand,
            at,
            other('s'),
            otherwise,
            other('d'),
            fi,
            // The same active character matches itself.
            if_char,
            no_expand,
            tilde,
            no_expand,
            tilde,
            other('s'),
            otherwise,
            other('d'),
            fi,
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 's');
    assert_eq!(next_character(&mut processor), 'd');
    assert_eq!(next_character(&mut processor), 'd');
    assert_eq!(next_character(&mut processor), 's');
    assert!(processor.command.expansion.pending_diagnostics.is_empty());
}

#[test]
fn if_and_ifcat_compare_noexpand_active_characters() {
    noexpand_before_an_active_character_compares_as_that_character();
}

#[test]
fn ifeof_reads_stream_open_state_and_recovers_a_bad_stream_number() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let if_eof = install(&mut universe, "ifeof", ExpandablePrimitive::IfEof);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    universe
        .world_mut()
        .set_memory_file("stream.tex", b"one\n".to_vec())
        .expect("seed stream content");
    universe
        .world_mut()
        .open_in(tex_state::StreamSlot::new(1), "stream.tex")
        .expect("open stream 1");
    push(
        &mut command,
        vec![
            // An open stream is not at end of file.
            if_eof,
            other('1'),
            other('o'),
            otherwise,
            other('x'),
            fi,
            // A stream that was never opened is closed, hence at end of file.
            if_eof,
            other('3'),
            other('c'),
            otherwise,
            other('x'),
            fi,
            // TeX.web §433 recovers an out-of-range stream as stream zero,
            // which is closed here, after reporting "Bad number".
            if_eof,
            other('9'),
            other('9'),
            other('b'),
            otherwise,
            other('x'),
            fi,
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 'x');
    assert_eq!(next_character(&mut processor), 'c');
    assert_eq!(next_character(&mut processor), 'b');
    assert_eq!(
        processor.command.expansion.pending_diagnostics,
        [BAD_NUMBER_DIAGNOSTIC]
    );
}

#[test]
fn redundant_else_inside_a_selected_true_limb_is_skipped_silently() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let if_true = install(&mut universe, "iftrue", ExpandablePrimitive::IfTrue);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    // A second `\else` in the skipped remainder is legal in TeX.web §510: the
    // skip loop swallows it and only `\fi` terminates the conditional.
    push(
        &mut command,
        vec![
            if_true,
            other('t'),
            otherwise,
            other('f'),
            otherwise,
            other('g'),
            fi,
            other('z'),
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 't');
    assert_eq!(next_character(&mut processor), 'z');
    assert!(processor.command.expansion.pending_diagnostics.is_empty());
    assert!(processor.command.conditions.current().is_none());
}

fn letter(ch: char) -> Token {
    Token::Char {
        ch,
        cat: tex_state::token::Catcode::Letter,
    }
}

fn append_boolean_case(
    universe: &mut Universe,
    tokens: &mut Vec<Token>,
    name: &str,
    primitive: ExpandablePrimitive,
    operands: impl IntoIterator<Item = Token>,
) {
    tokens.push(install(universe, name, primitive));
    tokens.extend(operands);
    tokens.push(other('t'));
    tokens.push(install(
        universe,
        &format!("{name}-else"),
        ExpandablePrimitive::Else,
    ));
    tokens.push(other('f'));
    tokens.push(install(
        universe,
        &format!("{name}-fi"),
        ExpandablePrimitive::Fi,
    ));
}

#[test]
fn tex82_predicate_aliases_preserve_classification() {
    for (primitive, kind) in [
        (ExpandablePrimitive::If, ConditionalKind::If),
        (ExpandablePrimitive::IfCat, ConditionalKind::IfCat),
        (ExpandablePrimitive::IfNum, ConditionalKind::IfNum),
        (ExpandablePrimitive::IfDim, ConditionalKind::IfDim),
        (ExpandablePrimitive::IfOdd, ConditionalKind::IfOdd),
        (ExpandablePrimitive::IfVMode, ConditionalKind::IfVMode),
        (ExpandablePrimitive::IfHMode, ConditionalKind::IfHMode),
        (ExpandablePrimitive::IfMMode, ConditionalKind::IfMMode),
        (ExpandablePrimitive::IfInner, ConditionalKind::IfInner),
        (ExpandablePrimitive::IfVoid, ConditionalKind::IfVoid),
        (ExpandablePrimitive::IfHBox, ConditionalKind::IfHBox),
        (ExpandablePrimitive::IfVBox, ConditionalKind::IfVBox),
        (ExpandablePrimitive::IfX, ConditionalKind::IfX),
        (ExpandablePrimitive::IfEof, ConditionalKind::IfEof),
        (ExpandablePrimitive::IfTrue, ConditionalKind::IfTrue),
        (ExpandablePrimitive::IfFalse, ConditionalKind::IfFalse),
        (ExpandablePrimitive::IfCase, ConditionalKind::IfCase),
    ] {
        assert_eq!(ConditionalKind::from_primitive(primitive), Some(kind));
    }
    for (primitive, delimiter) in [
        (ExpandablePrimitive::Or, ConditionalDelimiter::Or),
        (ExpandablePrimitive::Else, ConditionalDelimiter::Else),
        (ExpandablePrimitive::Fi, ConditionalDelimiter::Fi),
    ] {
        assert_eq!(
            ConditionalDelimiter::from_meaning(Meaning::ExpandablePrimitive(primitive)),
            Some(delimiter)
        );
    }

    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let first = install(&mut universe, "truth-alias-a", ExpandablePrimitive::IfTrue);
    let second = install(&mut universe, "truth-alias-b", ExpandablePrimitive::IfTrue);
    universe
        .register_primitive_meaning("fi", Meaning::ExpandablePrimitive(ExpandablePrimitive::Fi));
    let frozen_fi = universe
        .primitive_token("fi")
        .expect("frozen fi is registered");
    let public_fi = universe.intern("fi").symbol();
    universe.set_meaning(
        public_fi,
        Meaning::ExpandablePrimitive(ExpandablePrimitive::IfFalse),
    );
    push(&mut command, vec![first, second, frozen_fi]);
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    for _ in 0..2 {
        let alias = processor
            .get_next()
            .expect("raw token delivery succeeds")
            .expect("raw token remains");
        assert_eq!(
            alias.meaning(),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfTrue)
        );
    }
    assert_eq!(
        processor
            .get_next()
            .expect("raw token delivery succeeds")
            .expect("raw token remains")
            .meaning(),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::Fi)
    );
}

#[test]
fn condition_stack_preserves_lines_limits_and_input_independence() {
    let mut command = CommandState::default();
    let outer = command.conditions.push(ConditionalKind::IfNum, 41);
    let middle = command.conditions.push(ConditionalKind::IfCase, 43);
    let inner = command.conditions.push(ConditionalKind::IfX, 47);
    assert_eq!(
        command
            .conditions
            .frame(outer)
            .expect("outer frame exists")
            .source_line,
        41
    );
    assert_eq!(
        command
            .conditions
            .frame(middle)
            .expect("middle frame exists")
            .source_line,
        43
    );
    assert_eq!(
        command
            .conditions
            .frame(inner)
            .expect("inner frame exists")
            .source_line,
        47
    );

    let saved = command.conditions.frames.clone();
    push(&mut command, vec![other('x')]);
    assert_eq!(command.conditions.frames, saved);
    assert!(command.conditions.change_if_limit(outer, IfLimit::Else));
    assert_eq!(command.conditions.limit(outer), Some(IfLimit::Else));
    assert_eq!(command.conditions.limit(middle), Some(IfLimit::Evaluating));
    assert_eq!(command.conditions.limit(inner), Some(IfLimit::Evaluating));
    assert_eq!(
        command
            .conditions
            .pop()
            .expect("nested frame remains")
            .identity,
        inner
    );
    assert_eq!(
        command
            .conditions
            .pop()
            .expect("middle frame remains")
            .identity,
        middle
    );
    assert_eq!(
        command
            .conditions
            .pop()
            .expect("outer frame remains")
            .identity,
        outer
    );
    assert!(command.conditions.current().is_none());
}

#[test]
fn pass_text_does_not_expand_or_execute_skipped_tokens() {
    let mut command = CommandState::default();
    let condition = command.conditions.push(ConditionalKind::IfFalse, 11);
    assert!(command.conditions.change_if_limit(condition, IfLimit::Fi));
    let mut universe = crate::test_harness::universe();
    let nested_if = install(&mut universe, "skip-nested-if", ExpandablePrimitive::IfTrue);
    let nested_fi = install(&mut universe, "skip-nested-fi", ExpandablePrimitive::Fi);
    let outer_fi = install(&mut universe, "skip-outer-fi", ExpandablePrimitive::Fi);
    let skipped_macro = macro_token(
        &mut universe,
        "skip-macro",
        MeaningFlags::EMPTY,
        &[],
        &[other('!')],
    );
    let right = universe.intern("skip-right-brace").symbol();
    universe.set_meaning(
        right,
        Meaning::CharToken {
            ch: '}',
            cat: tex_state::token::Catcode::EndGroup,
        },
    );
    push(
        &mut command,
        vec![
            skipped_macro,
            other('{'),
            Token::Cs(right),
            nested_if,
            nested_fi,
            outer_fi,
            other('z'),
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(
        processor.pass_text(condition, ScannerWarning(11)),
        Ok(PassTextStop {
            delimiter: ConditionalDelimiter::Fi,
            nested_conditions: 0,
        })
    );
    assert_eq!(next_character(&mut processor), 'z');
    assert_eq!(
        processor.command.alignment.align_state,
        crate::processor::TOP_LEVEL_ALIGN_STATE
    );
    assert!(processor.command.expansion.pending_diagnostics.is_empty());
}

#[test]
fn ifcase_zero_negative_else_and_fi_boundaries() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let ifcase = install(
        &mut universe,
        "case-matrix-ifcase",
        ExpandablePrimitive::IfCase,
    );
    let or = install(&mut universe, "case-matrix-or", ExpandablePrimitive::Or);
    let otherwise = install(&mut universe, "case-matrix-else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "case-matrix-fi", ExpandablePrimitive::Fi);
    let mut tokens = vec![ifcase];
    tokens.extend(chars("0a"));
    tokens.extend([or, other('x'), fi, ifcase]);
    tokens.extend(chars("2a"));
    tokens.extend([
        or,
        other('b'),
        or,
        other('c'),
        otherwise,
        other('x'),
        fi,
        ifcase,
    ]);
    tokens.extend(chars("-1a"));
    tokens.extend([or, other('b'), otherwise, other('e'), fi, ifcase]);
    tokens.extend(chars("4a"));
    tokens.extend([or, other('b'), fi, other('z')]);
    push(&mut command, tokens);
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    for expected in ['a', 'c', 'e', 'z'] {
        assert_eq!(next_character(&mut processor), expected);
    }
    assert!(processor.command.conditions.current().is_none());
    assert!(processor.command.expansion.pending_diagnostics.is_empty());
}

#[test]
fn negative_ifcase_unwinds_condition_opened_while_scanning_its_operand() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    universe.set_int_param(tex_state::env::banks::IntParam::TRACING_COMMANDS, 2);
    let ifcase = install(
        &mut universe,
        "nested-operand-ifcase",
        ExpandablePrimitive::IfCase,
    );
    let iftrue = install(
        &mut universe,
        "nested-operand-iftrue",
        ExpandablePrimitive::IfTrue,
    );
    let otherwise = install(
        &mut universe,
        "nested-operand-else",
        ExpandablePrimitive::Else,
    );
    let fi = install(&mut universe, "nested-operand-fi", ExpandablePrimitive::Fi);
    let mut tokens = vec![ifcase, iftrue];
    tokens.extend(chars("-1a"));
    tokens.extend([otherwise, fi, ifcase]);
    tokens.extend(chars("0"));
    tokens.extend([fi, otherwise, ifcase]);
    tokens.extend(chars("5a"));
    tokens.extend([fi, fi, other('z')]);
    push(&mut command, tokens);
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 'z');
    assert!(processor.get_x_token().expect("input exhausts").is_none());
    assert!(processor.command.conditions.current().is_none());
    drop(processor);

    let diagnostics = diagnostic_text(&universe);
    let case_negative = diagnostics.find("{case -1}").expect("negative case trace");
    let suffix = &diagnostics[case_negative..];
    let case_five = suffix.find("{case 5}").expect("else-branch case trace");
    assert!(!suffix[..case_five].contains("{case 0}"), "{diagnostics:?}");
}

#[test]
fn conditional_delimiter_legality_matrix() {
    for (limit, accepted) in [
        (IfLimit::Evaluating, [false, false, true]),
        (IfLimit::Fi, [false, false, true]),
        (IfLimit::Else, [false, true, true]),
        (IfLimit::Or, [true, true, true]),
    ] {
        for (delimiter, expected) in [
            (ConditionalDelimiter::Or, accepted[0]),
            (ConditionalDelimiter::Else, accepted[1]),
            (ConditionalDelimiter::Fi, accepted[2]),
        ] {
            assert_eq!(limit.accepts_delimiter(delimiter), expected);
        }
    }
    let mut stack = ConditionStack::default();
    let evaluating = stack.push(ConditionalKind::If, 1);
    for delimiter in [
        ConditionalDelimiter::Or,
        ConditionalDelimiter::Else,
        ConditionalDelimiter::Fi,
    ] {
        assert!(
            stack
                .evaluating_delimiter_recovery(evaluating, delimiter)
                .is_some()
        );
    }

    let mut command = CommandState::default();
    let frame = command.conditions.push(ConditionalKind::IfTrue, 7);
    assert!(command.conditions.change_if_limit(frame, IfLimit::Else));
    let mut universe = crate::test_harness::universe();
    let or = install(&mut universe, "legality-or", ExpandablePrimitive::Or);
    let otherwise = install(&mut universe, "legality-else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "legality-fi", ExpandablePrimitive::Fi);
    push(&mut command, vec![or, fi, or, otherwise, fi]);
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);
    assert!(
        processor
            .get_x_token()
            .expect("delimiter recovery finishes")
            .is_none()
    );
    assert!(processor.command.conditions.current().is_none());
    assert_eq!(
        processor.command.expansion.pending_diagnostics,
        [
            EXTRA_DELIMITER_DIAGNOSTIC,
            EXTRA_DELIMITER_DIAGNOSTIC,
            EXTRA_DELIMITER_DIAGNOSTIC,
            EXTRA_DELIMITER_DIAGNOSTIC,
        ]
    );
}

#[test]
fn predicate_dispatch_covers_all_seventeen_kinds_and_state_queries() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let hbox = boxed(&mut universe, false);
    let vbox = boxed(&mut universe, true);
    universe.set_box_reg_ref(1, hbox);
    universe.set_box_reg_ref(2, vbox);
    universe
        .world_mut()
        .set_memory_file("dispatch-stream.tex", b"line\n".to_vec())
        .expect("memory input is installed");
    universe
        .world_mut()
        .open_in(tex_state::StreamSlot::new(1), "dispatch-stream.tex")
        .expect("memory input opens");

    let mut tokens = Vec::new();
    let mut expected = Vec::new();
    for (name, primitive, operands, selected) in [
        (
            "dispatch-if",
            ExpandablePrimitive::If,
            vec![other('a'), other('a')],
            't',
        ),
        (
            "dispatch-ifcat",
            ExpandablePrimitive::IfCat,
            vec![letter('a'), letter('b')],
            't',
        ),
        (
            "dispatch-ifx",
            ExpandablePrimitive::IfX,
            vec![other('q'), other('q')],
            't',
        ),
        (
            "dispatch-ifnum",
            ExpandablePrimitive::IfNum,
            chars("1=1"),
            't',
        ),
        (
            "dispatch-ifdim",
            ExpandablePrimitive::IfDim,
            chars("1pt=1pt"),
            't',
        ),
        (
            "dispatch-ifodd",
            ExpandablePrimitive::IfOdd,
            chars("-3"),
            't',
        ),
        (
            "dispatch-ifvmode",
            ExpandablePrimitive::IfVMode,
            Vec::new(),
            'f',
        ),
        (
            "dispatch-ifhmode",
            ExpandablePrimitive::IfHMode,
            Vec::new(),
            't',
        ),
        (
            "dispatch-ifmmode",
            ExpandablePrimitive::IfMMode,
            Vec::new(),
            'f',
        ),
        (
            "dispatch-ifinner",
            ExpandablePrimitive::IfInner,
            Vec::new(),
            't',
        ),
        (
            "dispatch-ifvoid",
            ExpandablePrimitive::IfVoid,
            chars("0"),
            't',
        ),
        (
            "dispatch-ifhbox",
            ExpandablePrimitive::IfHBox,
            chars("1"),
            't',
        ),
        (
            "dispatch-ifvbox",
            ExpandablePrimitive::IfVBox,
            chars("2"),
            't',
        ),
        (
            "dispatch-ifeof-open",
            ExpandablePrimitive::IfEof,
            chars("1"),
            'f',
        ),
        (
            "dispatch-ifeof-closed",
            ExpandablePrimitive::IfEof,
            chars("3"),
            't',
        ),
        (
            "dispatch-iftrue",
            ExpandablePrimitive::IfTrue,
            Vec::new(),
            't',
        ),
        (
            "dispatch-iffalse",
            ExpandablePrimitive::IfFalse,
            Vec::new(),
            'f',
        ),
    ] {
        append_boolean_case(&mut universe, &mut tokens, name, primitive, operands);
        expected.push(selected);
    }
    let ifcase = install(
        &mut universe,
        "dispatch-ifcase",
        ExpandablePrimitive::IfCase,
    );
    let or = install(&mut universe, "dispatch-ifcase-or", ExpandablePrimitive::Or);
    let fi = install(&mut universe, "dispatch-ifcase-fi", ExpandablePrimitive::Fi);
    tokens.extend([ifcase, other('0'), other('t'), or, other('f'), fi]);
    expected.push('t');
    push(&mut command, tokens);
    let mut capabilities = CommandHostCapabilities::default();
    capabilities.set_conditional_state(ConditionalState::new(ConditionalMode::Horizontal, true));
    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        for selected in expected {
            assert_eq!(next_character(&mut processor), selected);
        }
        assert!(
            processor
                .get_x_token()
                .expect("conditional input is exhausted")
                .is_none()
        );
        assert!(processor.command.conditions.current().is_none());
    }

    for (mode, inner, truth) in [
        (
            ConditionalMode::Vertical,
            false,
            [true, false, false, false],
        ),
        (ConditionalMode::Vertical, true, [true, false, false, true]),
        (
            ConditionalMode::Horizontal,
            false,
            [false, true, false, false],
        ),
        (
            ConditionalMode::Horizontal,
            true,
            [false, true, false, true],
        ),
        (ConditionalMode::Math, false, [false, false, true, false]),
        (ConditionalMode::Math, true, [false, false, true, true]),
    ] {
        capabilities.set_conditional_state(ConditionalState::new(mode, inner));
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        assert_eq!(
            processor.evaluate_boolean(ConditionalKind::IfVMode),
            Ok(truth[0])
        );
        assert_eq!(
            processor.evaluate_boolean(ConditionalKind::IfHMode),
            Ok(truth[1])
        );
        assert_eq!(
            processor.evaluate_boolean(ConditionalKind::IfMMode),
            Ok(truth[2])
        );
        assert_eq!(
            processor.evaluate_boolean(ConditionalKind::IfInner),
            Ok(truth[3])
        );
    }
}

#[test]
fn ifnum_ifdim_relation_and_missing_equals_matrix() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let mut tokens = Vec::new();
    for (index, expression) in ["1<2", "2=2", "3>2"].into_iter().enumerate() {
        append_boolean_case(
            &mut universe,
            &mut tokens,
            &format!("ifnum-relation-{index}"),
            ExpandablePrimitive::IfNum,
            chars(expression),
        );
    }
    for (index, expression) in ["1pt<2pt", "2pt=2pt", "3pt>2pt"].into_iter().enumerate() {
        append_boolean_case(
            &mut universe,
            &mut tokens,
            &format!("ifdim-relation-{index}"),
            ExpandablePrimitive::IfDim,
            chars(expression),
        );
    }
    append_boolean_case(
        &mut universe,
        &mut tokens,
        "ifnum-missing-relation",
        ExpandablePrimitive::IfNum,
        vec![
            other('1'),
            Token::Char {
                ch: ' ',
                cat: tex_state::token::Catcode::Space,
            },
            other('1'),
        ],
    );
    push(&mut command, tokens);
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    for _ in 0..7 {
        assert_eq!(next_character(&mut processor), 't');
    }
    assert!(
        processor
            .get_x_token()
            .expect("conditional input is exhausted")
            .is_none()
    );
    assert_eq!(
        processor.command.expansion.pending_diagnostics,
        [MISSING_RELATION_DIAGNOSTIC]
    );
    assert!(processor.command.conditions.current().is_none());
}

#[test]
fn malformed_ifdim_keeps_nested_conditional_traces_after_its_diagnostic() {
    // TeX82 §§502--503 calls `back_error` for the missing relation before
    // continuing the outer comparison. e-TeX [28.494/28.498] may trace the
    // nested condition and its delimiters during that same expansion call;
    // those later traces must remain behind the synchronous §82 report.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    universe.set_int_param(tex_state::env::banks::IntParam::TRACING_COMMANDS, 2);
    universe.set_int_param(tex_state::env::banks::IntParam::TRACING_IFS, 1);
    let ifdim = install(&mut universe, "ifdim", ExpandablePrimitive::IfDim);
    let iftrue = install(&mut universe, "iftrue", ExpandablePrimitive::IfTrue);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    push(
        &mut command,
        [
            vec![ifdim],
            chars("0pt"),
            vec![iftrue],
            chars("1pt"),
            vec![fi],
            chars("x"),
            vec![otherwise],
            chars("f"),
            vec![fi],
        ]
        .concat(),
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 'f');
    assert!(processor.get_x_token().expect("input exhausts").is_none());
    let diagnostics = processor.take_semantic_diagnostics();
    assert!(matches!(
        diagnostics.first(),
        Some(crate::CommandSemanticDiagnostic::Recoverable {
            identity: MISSING_RELATION_DIAGNOSTIC,
            ..
        })
    ));
    assert!(diagnostics[1..].iter().any(|diagnostic| matches!(
        diagnostic,
        crate::CommandSemanticDiagnostic::Trace { text, .. } if text == "{false}"
    )));
    assert!(diagnostics[1..].iter().any(|diagnostic| matches!(
        diagnostic,
        crate::CommandSemanticDiagnostic::Trace { text, .. } if text.starts_with("{\\fi:")
    )));
    assert!(processor.command.conditions.current().is_none());
    drop(processor);
    let immediate = diagnostic_text(&universe);
    assert!(!immediate.contains("{false}"), "{immediate:?}");
}

#[test]
fn ifodd_signed_parity_and_scanner_recovery_matrix() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let mut tokens = Vec::new();
    for (index, value) in ["0", "2", "-2", "1", "-1"].into_iter().enumerate() {
        append_boolean_case(
            &mut universe,
            &mut tokens,
            &format!("ifodd-parity-{index}"),
            ExpandablePrimitive::IfOdd,
            chars(value),
        );
    }
    append_boolean_case(
        &mut universe,
        &mut tokens,
        "ifodd-missing-number",
        ExpandablePrimitive::IfOdd,
        chars("x"),
    );
    push(&mut command, tokens);
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    for expected in ['f', 'f', 'f', 't', 't', 'f'] {
        assert_eq!(next_character(&mut processor), expected);
    }
    assert!(
        processor
            .get_x_token()
            .expect("conditional input is exhausted")
            .is_none()
    );
    assert!(processor.command.conditions.current().is_none());
}

fn box_with_content(
    universe: &mut Universe,
    vertical: bool,
    nonempty: bool,
) -> tex_state::node_arena::NodeListRef {
    use tex_state::glue::Order;
    use tex_state::node::{BoxNode, BoxNodeFields, Node, Sign};
    use tex_state::scaled::{GlueSetRatio, Scaled};

    let content = if nonempty {
        vec![Node::Penalty(17)]
    } else {
        Vec::new()
    };
    let children = universe.freeze_node_list(&content);
    let node = BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        box_lr: tex_state::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    });
    universe.freeze_node_list(&[if vertical {
        Node::VList(node)
    } else {
        Node::HList(node)
    }])
}

#[test]
fn ifvoid_ifhbox_ifvbox_register_kind_matrix() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let hbox_empty = box_with_content(&mut universe, false, false);
    let hbox_nonempty = box_with_content(&mut universe, false, true);
    let vbox_empty = box_with_content(&mut universe, true, false);
    let vbox_nonempty = box_with_content(&mut universe, true, true);
    // Register zero holds an hbox so that TeX82 §433's recover-to-zero is
    // observable: an out-of-range selector must answer for register zero,
    // not for the absent register the digits spelled.
    let hbox_recovered = box_with_content(&mut universe, false, true);
    universe.set_box_reg_ref(0, hbox_recovered);
    universe.set_box_reg_ref(1, hbox_empty);
    universe.set_box_reg_ref(2, hbox_nonempty);
    universe.set_box_reg_ref(3, vbox_empty);
    universe.set_box_reg_ref(4, vbox_nonempty);

    // The last two selectors are §433's two out-of-range directions, which
    // §505 reaches through `scan_eight_bit_int`: both report "Bad register
    // code" and continue with register zero.
    let selectors = ["5", "1", "2", "3", "4", "256", "-1"];
    let mut tokens = Vec::new();
    let mut expected = Vec::new();
    for (primitive, prefix, truths) in [
        (
            ExpandablePrimitive::IfVoid,
            "ifvoid",
            [true, false, false, false, false, false, false],
        ),
        (
            ExpandablePrimitive::IfHBox,
            "ifhbox",
            [false, true, true, false, false, true, true],
        ),
        (
            ExpandablePrimitive::IfVBox,
            "ifvbox",
            [false, false, false, true, true, false, false],
        ),
    ] {
        for (offset, register) in selectors.into_iter().enumerate() {
            append_boolean_case(
                &mut universe,
                &mut tokens,
                &format!("{prefix}-{register}"),
                primitive,
                chars(register),
            );
            expected.push(if truths[offset] { 't' } else { 'f' });
        }
    }
    push(&mut command, tokens);
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    for selected in expected {
        assert_eq!(next_character(&mut processor), selected);
    }
    assert!(
        processor
            .get_x_token()
            .expect("conditional input is exhausted")
            .is_none()
    );
    assert!(processor.command.conditions.current().is_none());
}

#[test]
fn etex_box_conditionals_read_sparse_register_kinds() {
    // e-TeX 2.6 [28.505] replaces TeX82 §505's eight-bit selector with
    // `scan_register_num; fetch_box(p)` for all three box predicates.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut universe = crate::test_harness::universe();
    let dense_sentinel = box_with_content(&mut universe, true, false);
    let sparse_hbox = box_with_content(&mut universe, false, false);
    let sparse_vbox = box_with_content(&mut universe, true, false);
    universe.set_box_reg_ref(0, dense_sentinel);
    universe.set_box_reg_ref(300, sparse_hbox);
    universe.set_box_reg_ref(301, sparse_vbox);

    let mut tokens = Vec::new();
    for (primitive, register) in [
        (ExpandablePrimitive::IfHBox, "300"),
        (ExpandablePrimitive::IfVBox, "301"),
        (ExpandablePrimitive::IfVoid, "302"),
    ] {
        append_boolean_case(
            &mut universe,
            &mut tokens,
            &format!("etex-box-{register}"),
            primitive,
            chars(register),
        );
    }
    push(&mut command, tokens);
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let mut processor =
        processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);

    assert_eq!(next_character(&mut processor), 't');
    assert_eq!(next_character(&mut processor), 't');
    assert_eq!(next_character(&mut processor), 't');
    drop(processor);
    let branches = recorder
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Condition(record)
                if record.transition == "branch" && record.limit == "evaluating" =>
            {
                record.branch.as_deref()
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(branches, ["true", "true", "true"]);
}

#[test]
fn false_boolean_skip_closes_a_condition_left_by_operand_expansion() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let if_char = install(&mut universe, "if", ExpandablePrimitive::If);
    let no_expand = install(&mut universe, "noexpand", ExpandablePrimitive::NoExpand);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    let sentinel = Token::Cs(universe.intern("sentinel").symbol());

    // TeX.web §500 saves the outer `cond_ptr` before operand expansion. The
    // inner false condition remains above it until its selected branch's
    // `\fi`; that delimiter must pop the inner frame, not end the outer one.
    push(
        &mut command,
        vec![
            if_char,
            other('e'),
            if_char,
            other('E'),
            no_expand,
            sentinel,
            other('e'),
            otherwise,
            no_expand,
            sentinel,
            fi,
            other('t'),
            otherwise,
            other('f'),
            fi,
            other('z'),
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 'f');
    assert_eq!(next_character(&mut processor), 'z');
    assert!(processor.command.conditions.current().is_none());
    assert!(processor.command.expansion.pending_diagnostics.is_empty());
}

#[test]
fn if_ifcat_and_ifx_complete_operand_matrix() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let if_true = install(&mut universe, "operand-iftrue", ExpandablePrimitive::IfTrue);
    let if_false = install(
        &mut universe,
        "operand-iffalse",
        ExpandablePrimitive::IfFalse,
    );
    let no_expand = install(
        &mut universe,
        "operand-noexpand",
        ExpandablePrimitive::NoExpand,
    );
    let active = active_macro(&mut universe, '~');
    let same_a = macro_token(
        &mut universe,
        "operand-same-a",
        MeaningFlags::EMPTY,
        &[Token::param(1)],
        &[Token::param(1), other('x')],
    );
    let same_b = macro_token(
        &mut universe,
        "operand-same-b",
        MeaningFlags::EMPTY,
        &[Token::param(1)],
        &[Token::param(1), other('x')],
    );
    let long_same = macro_token(
        &mut universe,
        "operand-long-same",
        MeaningFlags::LONG,
        &[Token::param(1)],
        &[Token::param(1), other('x')],
    );
    let different_text = macro_token(
        &mut universe,
        "operand-different-text",
        MeaningFlags::EMPTY,
        &[Token::param(1)],
        &[Token::param(1), other('y')],
    );
    let mut tokens = Vec::new();
    for (name, primitive, operands) in [
        (
            "matrix-if-equal",
            ExpandablePrimitive::If,
            vec![other('a'), letter('a')],
        ),
        (
            "matrix-if-different",
            ExpandablePrimitive::If,
            vec![other('a'), other('b')],
        ),
        (
            "matrix-ifcat-equal",
            ExpandablePrimitive::IfCat,
            vec![letter('a'), letter('b')],
        ),
        (
            "matrix-ifcat-different",
            ExpandablePrimitive::IfCat,
            vec![letter('a'), other('a')],
        ),
        (
            "matrix-active",
            ExpandablePrimitive::If,
            vec![no_expand, active, no_expand, active],
        ),
        (
            "matrix-ifx-raw-equal",
            ExpandablePrimitive::IfX,
            vec![if_true, if_true],
        ),
        (
            "matrix-ifx-raw-different",
            ExpandablePrimitive::IfX,
            vec![if_true, if_false],
        ),
        (
            "matrix-ifx-macro-equal",
            ExpandablePrimitive::IfX,
            vec![same_a, same_b],
        ),
        (
            "matrix-ifx-flags",
            ExpandablePrimitive::IfX,
            vec![same_a, long_same],
        ),
        (
            "matrix-ifx-text",
            ExpandablePrimitive::IfX,
            vec![same_a, different_text],
        ),
    ] {
        append_boolean_case(&mut universe, &mut tokens, name, primitive, operands);
    }
    push(&mut command, tokens);
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    for expected in ['t', 'f', 't', 'f', 't', 't', 'f', 't', 'f', 'f'] {
        assert_eq!(next_character(&mut processor), expected);
    }
    assert!(
        processor
            .get_x_token()
            .expect("conditional input is exhausted")
            .is_none()
    );
    assert!(processor.command.conditions.current().is_none());
}

#[test]
fn ifx_compares_macro_tokens_after_candidate_index_churn() {
    // TeX82 §507 compares the parameter and replacement token lists. Their
    // allocator coordinates are deliberately different here: the bounded
    // candidate index is operational metadata, not semantic authority, and
    // must not change `\ifx`'s result after it rolls over.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let first = macro_token(
        &mut universe,
        "domain-first",
        MeaningFlags::EMPTY,
        &[Token::param(1)],
        &[Token::param(1), letter('x')],
    );

    for value in 0..2_048_u32 {
        let ch = char::from_u32(0x1_000 + value).expect("filler is a Unicode scalar");
        let _filler = macro_token(
            &mut universe,
            &format!("ifx-churn-{value}"),
            MeaningFlags::EMPTY,
            &[Token::param(1)],
            &[Token::param(1), other(ch)],
        );
    }
    let equal = macro_token(
        &mut universe,
        "domain-equal",
        MeaningFlags::EMPTY,
        &[Token::param(1)],
        &[Token::param(1), letter('x')],
    );
    let different = macro_token(
        &mut universe,
        "domain-different",
        MeaningFlags::EMPTY,
        &[Token::param(1)],
        &[Token::param(1), letter('y')],
    );

    let replacement = |token| {
        let Token::Cs(symbol) = token else {
            panic!("macro operand is a control sequence");
        };
        let Meaning::Macro { definition, .. } = universe.meaning(symbol) else {
            panic!("macro operand has a macro meaning");
        };
        universe
            .macro_definition(definition)
            .meaning()
            .replacement_text()
    };
    let first_replacement = replacement(first);
    let equal_replacement = replacement(equal);
    assert_ne!(first_replacement, equal_replacement);
    assert_eq!(
        universe.tokens(first_replacement).tokens(),
        universe.tokens(equal_replacement).tokens()
    );

    let mut tokens = Vec::new();
    append_boolean_case(
        &mut universe,
        &mut tokens,
        "domain-equal-case",
        ExpandablePrimitive::IfX,
        vec![first, equal],
    );
    append_boolean_case(
        &mut universe,
        &mut tokens,
        "domain-different-case",
        ExpandablePrimitive::IfX,
        vec![first, different],
    );
    push(&mut command, tokens);
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 't');
    assert_eq!(next_character(&mut processor), 'f');
}

#[test]
fn etex_ifdefined_tests_one_unexpanded_raw_meaning() {
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut universe = crate::test_harness::universe();
    let undefined_symbol = universe.intern("ifdefined-undefined").symbol();
    let macro_operand = macro_token(
        &mut universe,
        "ifdefined-macro",
        MeaningFlags::EMPTY,
        &[],
        &[other('X')],
    );
    let mut tokens = Vec::new();
    for (name, operand) in [
        ("ifdefined-undefined-case", Token::Cs(undefined_symbol)),
        ("ifdefined-macro-case", macro_operand),
        ("ifdefined-character-case", other('q')),
    ] {
        append_boolean_case(
            &mut universe,
            &mut tokens,
            name,
            ExpandablePrimitive::IfDefined,
            [operand],
        );
    }
    push(&mut command, tokens);
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    for expected in ['f', 't', 't'] {
        assert_eq!(next_character(&mut processor), expected);
    }
    assert!(
        processor
            .get_x_token()
            .expect("input is exhausted")
            .is_none()
    );
    assert!(processor.command.conditions.current().is_none());
}

#[test]
fn etex_ifdefined_observes_only_an_actual_scanner_status_change() {
    // e-TeX 2.6 etex.ch [17.4750--4758] saves `scanner_status`, assigns
    // `normal` while `get_next` reads the operand, and restores the saved
    // value. The canonical transition trace records state changes, so an
    // already-normal scan has no synthetic normal-to-normal records.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut universe = crate::test_harness::universe();
    let mut tokens = Vec::new();
    append_boolean_case(
        &mut universe,
        &mut tokens,
        "ifdefined-observation-case",
        ExpandablePrimitive::IfDefined,
        [other('q')],
    );
    push(&mut command, tokens);
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        assert_eq!(next_character(&mut processor), 't');
    }
    assert!(
        !recorder
            .0
            .iter()
            .any(|observation| matches!(observation, CommandObservation::ScannerStatus(_)))
    );
}

#[test]
fn etex_ifdefined_temporarily_allows_an_outer_operand() {
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut universe = crate::test_harness::universe();
    let outer = macro_token(
        &mut universe,
        "ifdefined-outer",
        MeaningFlags::OUTER,
        &[],
        &[],
    );
    let mut tokens = Vec::new();
    append_boolean_case(
        &mut universe,
        &mut tokens,
        "ifdefined-outer-case",
        ExpandablePrimitive::IfDefined,
        [outer],
    );
    push(&mut command, tokens);
    let defining = ScannerStatus::Defining(DefinitionContext {
        target: None,
        builder: TokenBuilderId(1),
        warning: ScannerWarning(1),
    });
    let _prior = command.begin_scanner_status(defining.clone());
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        assert_eq!(next_character(&mut processor), 't');
        assert_eq!(processor.command.scanner.status(), &defining);
        assert!(processor.command.expansion.pending_diagnostics.is_empty());
    }
    let transitions = recorder
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::ScannerStatus(record) => Some((record.from, record.to)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        transitions,
        vec![("defining", "normal"), ("normal", "defining")]
    );
    let defining = command.begin_scanner_status(ScannerStatus::Normal);
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);
    assert!(
        processor
            .get_x_token()
            .expect("input is exhausted")
            .is_none()
    );
    assert!(processor.command.conditions.current().is_none());
    processor.command.restore_scanner_status(defining);
}

#[test]
fn etex_ifcsname_expands_names_without_creating_missing_control_sequences() {
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut universe = crate::test_harness::universe();
    let endcsname = install(
        &mut universe,
        "ifcsname-end",
        ExpandablePrimitive::EndCsName,
    );
    let nested = macro_token(
        &mut universe,
        "ifcsname-name-fragment",
        MeaningFlags::EMPTY,
        &[],
        &[other('é'), other('x')],
    );
    let defined = universe.intern("éx").symbol();
    universe.set_meaning(defined, Meaning::Relax);

    let mut tokens = Vec::new();
    append_boolean_case(
        &mut universe,
        &mut tokens,
        "ifcsname-defined",
        ExpandablePrimitive::IfCsName,
        [nested, endcsname],
    );
    append_boolean_case(
        &mut universe,
        &mut tokens,
        "ifcsname-undefined",
        ExpandablePrimitive::IfCsName,
        [other('m'), other('i'), other('s'), other('s'), endcsname],
    );
    append_boolean_case(
        &mut universe,
        &mut tokens,
        "ifcsname-empty",
        ExpandablePrimitive::IfCsName,
        [endcsname],
    );
    push(&mut command, tokens);
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    for expected in ['t', 'f', 'f'] {
        assert_eq!(next_character(&mut processor), expected);
    }
    assert!(
        processor
            .get_x_token()
            .expect("conditional input is exhausted")
            .is_none()
    );
    assert!(processor.command.conditions.current().is_none());
    assert_eq!(
        processor.state.known_control_sequence("miss"),
        None,
        "etex.ch [17.4765--4779] forbids ifcsname from entering an absent name"
    );
}

#[test]
fn pdftex_ifincsname_tracks_the_dynamic_csname_scan() {
    let mut command = CommandState::new(crate::CommandProfile::PDFTEX14029);
    let mut universe = crate::test_harness::universe();
    let csname = install(
        &mut universe,
        "ifincsname-csname",
        ExpandablePrimitive::CsName,
    );
    let endcsname = install(
        &mut universe,
        "ifincsname-endcsname",
        ExpandablePrimitive::EndCsName,
    );
    let ifincsname = install(
        &mut universe,
        "ifincsname-test",
        ExpandablePrimitive::IfInCsName,
    );
    let else_token = install(&mut universe, "ifincsname-else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "ifincsname-fi", ExpandablePrimitive::Fi);
    let mut tokens = Vec::new();
    append_boolean_case(
        &mut universe,
        &mut tokens,
        "ifincsname-outside",
        ExpandablePrimitive::IfInCsName,
        [],
    );
    tokens.extend([
        csname,
        other('x'),
        ifincsname,
        other('y'),
        else_token,
        other('n'),
        fi,
        endcsname,
    ]);
    let defined = universe.intern("zy").symbol();
    universe.set_meaning(defined, Meaning::Relax);
    append_boolean_case(
        &mut universe,
        &mut tokens,
        "ifincsname-inside-ifcsname",
        ExpandablePrimitive::IfCsName,
        [
            other('z'),
            ifincsname,
            other('y'),
            else_token,
            other('n'),
            fi,
            endcsname,
        ],
    );
    push(&mut command, tokens);
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 'f');
    let named = processor
        .get_x_token()
        .expect("csname expansion succeeds")
        .expect("csname injects its result");
    assert_eq!(
        named
            .control_sequence()
            .map(|symbol| processor.state.resolve(symbol)),
        Some("xy")
    );
    assert_eq!(next_character(&mut processor), 't');
    assert!(!processor.is_in_csname);
}

#[test]
fn etex_ifcsname_uses_csname_boundary_recovery_and_conditional_lifecycle() {
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut universe = crate::test_harness::universe();
    let relax = universe.intern("ifcsname-recovery-relax").symbol();
    universe.set_meaning(relax, Meaning::Relax);
    let endcsname = install(
        &mut universe,
        "ifcsname-recovery-end",
        ExpandablePrimitive::EndCsName,
    );
    let mut tokens = Vec::new();
    append_boolean_case(
        &mut universe,
        &mut tokens,
        "ifcsname-recovery",
        ExpandablePrimitive::IfCsName,
        [other('q'), Token::Cs(relax), endcsname],
    );
    push(&mut command, tokens);
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        assert_eq!(next_character(&mut processor), 'f');
        assert!(
            processor
                .get_x_token()
                .expect("conditional input is exhausted")
                .is_none()
        );
        assert_eq!(
            processor.command.expansion.pending_diagnostics,
            vec![crate::processor::expand::MISSING_ENDCSNAME_DIAGNOSTIC]
        );
        assert!(processor.command.conditions.current().is_none());
    }
    let transitions = recorder
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Condition(record) if record.condition == "ifcsname" => {
                Some((record.transition, record.branch.as_deref()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        transitions,
        [
            ("push", None),
            ("branch", Some("false")),
            ("branch", Some("else")),
            ("limit", None),
            ("pop", None),
        ]
    );
}

#[test]
fn etex_iffontchar_tests_metric_existence_and_unless_inverts_the_same_frame() {
    use tex_state::font::{CharMetrics, CharTag, FontMetrics, LoadedFont};
    use tex_state::scaled::Scaled;

    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut universe = crate::test_harness::universe();
    let iffontchar = install(&mut universe, "iffontchar", ExpandablePrimitive::IfFontChar);
    let unless = install(&mut universe, "unless", ExpandablePrimitive::Unless);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    let font_symbol = universe.intern("metric-font").symbol();
    let mut characters = vec![None; 256];
    characters[0] = Some(CharMetrics {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        italic_correction: Scaled::from_raw(0),
        tag: CharTag::None,
    });
    characters[65] = Some(CharMetrics {
        width: Scaled::from_raw(1),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        italic_correction: Scaled::from_raw(0),
        tag: CharTag::None,
    });
    let font = universe.intern_font_with_identifier(
        LoadedFont::new(
            "metric-font",
            "metric-font.tfm",
            [9; 32],
            0,
            Scaled::from_raw(10 * Scaled::UNITY),
            Scaled::from_raw(10 * Scaled::UNITY),
            vec![Scaled::from_raw(0); 7],
            FontMetrics::new(characters, Vec::new(), None, None, Vec::new()),
        ),
        font_symbol,
    );
    universe.set_meaning(font_symbol, Meaning::Font(font));

    push(
        &mut command,
        vec![
            // Present character.
            iffontchar,
            Token::Cs(font_symbol),
            other('6'),
            other('5'),
            other('p'),
            otherwise,
            other('x'),
            fi,
            // In-range but absent character.
            iffontchar,
            Token::Cs(font_symbol),
            other('6'),
            other('6'),
            other('x'),
            otherwise,
            other('a'),
            fi,
            // `\unless` negates the same predicate rather than nesting a
            // second condition frame.
            unless,
            iffontchar,
            Token::Cs(font_symbol),
            other('6'),
            other('6'),
            other('u'),
            otherwise,
            other('x'),
            fi,
            // §434 recovers 256 as character zero, whose existence is then
            // tested normally.
            iffontchar,
            Token::Cs(font_symbol),
            other('2'),
            other('5'),
            other('6'),
            other('r'),
            otherwise,
            other('x'),
            fi,
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 'p');
    assert_eq!(next_character(&mut processor), 'a');
    assert_eq!(next_character(&mut processor), 'u');
    assert_eq!(next_character(&mut processor), 'r');
    assert!(processor.get_x_token().expect("final fi expands").is_none());
    assert!(processor.command.conditions.current().is_none());
}
