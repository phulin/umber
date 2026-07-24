use tex_state::Universe;
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
use tex_state::token::{OriginId, Token, TracedTokenWord};

use super::*;
use crate::input::{
    ReplayTrace, RetirementBehavior, SharedTokenBuffer, TokenBehavior, TokenPayload,
};
use crate::{
    CommandHostCapabilities, CommandHostContext, CommandRuntime, CommandState, ConditionalMode,
    ConditionalState,
};

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
    use tex_state::glue::Order;
    use tex_state::node::{BoxNode, BoxNodeFields, Node, Sign};
    use tex_state::scaled::{GlueSetRatio, Scaled};

    let children = universe.freeze_node_list(&[]);
    let node = BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
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
fn delimiter_during_operand_scan_inserts_frozen_relax() {
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
    assert_eq!(processor.command.expansion.pending_diagnostics.len(), 1);
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
