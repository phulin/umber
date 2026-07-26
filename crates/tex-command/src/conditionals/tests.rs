use tex_state::Universe;
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
use tex_state::token::{OriginId, Token, TracedTokenWord};

use super::*;
use crate::input::{
    ReplayTrace, RetirementBehavior, SharedTokenBuffer, TokenBehavior, TokenPayload,
};
use crate::{
    CommandHostCapabilities, CommandHostContext, CommandObservation, CommandObserver,
    CommandRuntime, CommandState, ConditionalMode, ConditionalState,
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

fn boxed(universe: &mut Universe, vertical: bool) -> tex_state::ids::NodeListId {
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
) -> tex_state::ids::NodeListId {
    use tex_state::glue::Order;
    use tex_state::node::{BoxNode, BoxNodeFields, Node, Sign};
    use tex_state::scaled::GlueSetRatio;

    let children = universe.freeze_node_list(&[]);
    let node = BoxNode::new(BoxNodeFields {
        width,
        height,
        depth,
        shift: tex_state::scaled::Scaled::from_raw(0),
        display: false,
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let if_false = install(&mut universe, "iffalse", ExpandablePrimitive::IfFalse);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    push(
        &mut command,
        vec![if_false, other('f'), otherwise, other('t'), fi],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
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
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    // If the operands were delivered with get_x_token, either `iftrue` would
    // open a nested condition or the following text would be consumed.
    assert_eq!(next_character(&mut processor), 'y');
}

#[test]
fn true_ifx_fi_observes_branch_before_popping_its_frame() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .with_observer(&mut recorder);

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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
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
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 'y');
    assert_eq!(next_character(&mut processor), 'n');
}

#[test]
fn character_and_category_tests_normalize_non_character_operands() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
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
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 'c');
    assert_eq!(next_character(&mut processor), 'k');
}

#[test]
fn skipped_text_recovers_extra_delimiters_deterministically() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
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
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let or = install(&mut universe, "or", ExpandablePrimitive::Or);
    push(&mut command, vec![or]);
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .with_observer(&mut recorder);
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let if_char = install(&mut universe, "if", ExpandablePrimitive::If);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    universe.install_primitive_meaning("relax", Meaning::Relax);
    push(&mut command, vec![if_char, otherwise, other('a'), fi]);
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

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
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let unless = install(&mut universe, "unless", ExpandablePrimitive::Unless);
    let if_false = install(&mut universe, "iffalse", ExpandablePrimitive::IfFalse);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    push(
        &mut command,
        vec![unless, if_false, other('y'), otherwise, other('n'), fi],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 'y');
}

#[test]
fn numeric_and_ifcase_selection_use_the_same_skip_machine() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
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
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 'y');
    assert_eq!(next_character(&mut processor), '1');
    assert!(
        processor
            .get_x_token()
            .expect("ifcase else is skipped")
            .is_none()
    );
}

#[test]
fn ifcase_observes_its_limit_only_after_skipping_to_the_selected_limb() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .with_observer(&mut recorder);
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
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
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 'y');
    assert_eq!(next_character(&mut processor), 'i');
}

#[test]
fn ifdim_scans_box_dimensions_as_internal_dimensions() {
    use tex_state::meaning::UnexpandablePrimitive;
    use tex_state::scaled::Scaled;

    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
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
    universe.set_box_reg(3, box_node);

    let mut tokens = Vec::new();
    for (primitive, expected, selected) in [(wd, "1pt", 'w'), (ht, "2pt", 'h'), (dp, "3pt", 'd')] {
        tokens.extend([ifdim, Token::Cs(primitive)]);
        tokens.extend(chars(&format!("3={expected}")));
        tokens.extend([other(selected), otherwise, other('n'), fi]);
    }
    push(&mut command, tokens);
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
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
    universe.set_box_reg(0, box_node);
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
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 'y');
}

#[test]
fn mode_and_box_predicates_use_host_and_aggregate_queries() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let if_hmode = install(&mut universe, "ifhmode", ExpandablePrimitive::IfHMode);
    let if_inner = install(&mut universe, "ifinner", ExpandablePrimitive::IfInner);
    let if_void = install(&mut universe, "ifvoid", ExpandablePrimitive::IfVoid);
    let if_hbox = install(&mut universe, "ifhbox", ExpandablePrimitive::IfHBox);
    let if_vbox = install(&mut universe, "ifvbox", ExpandablePrimitive::IfVBox);
    let otherwise = install(&mut universe, "else", ExpandablePrimitive::Else);
    let fi = install(&mut universe, "fi", ExpandablePrimitive::Fi);
    let hbox = boxed(&mut universe, false);
    let vbox = boxed(&mut universe, true);
    universe.set_box_reg(1, hbox);
    universe.set_box_reg(2, vbox);
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
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    for expected in ['h', 'i', 'v', 'h', 'b'] {
        assert_eq!(next_character(&mut processor), expected);
    }
    assert!(processor.get_x_token().expect("final fi expands").is_none());
}

#[test]
fn selected_ifcase_limb_skips_remaining_limbs_without_extra_delimiter_errors() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .with_observer(&mut recorder);

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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
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
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 's');
    assert_eq!(next_character(&mut processor), 'd');
    assert_eq!(next_character(&mut processor), 'd');
    assert_eq!(next_character(&mut processor), 's');
    assert!(processor.command.expansion.pending_diagnostics.is_empty());
}

#[test]
fn ifeof_reads_stream_open_state_and_recovers_a_bad_stream_number() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
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
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
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
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

    assert_eq!(next_character(&mut processor), 't');
    assert_eq!(next_character(&mut processor), 'z');
    assert!(processor.command.expansion.pending_diagnostics.is_empty());
    assert!(processor.command.conditions.current().is_none());
}
