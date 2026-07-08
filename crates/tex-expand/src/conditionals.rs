use tex_lex::{ConditionFrameSummary, ConditionKind, ConditionLimb, InputSource, InputStack};
use tex_state::ExpansionState;
use tex_state::interner::Symbol;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
use tex_state::node::Node;
use tex_state::token::Token;

use crate::{
    Dispatch, ExpandError, ExpandNext, ExpandableOpcode, ExpansionHooks, ReadRecorder,
    scan_helpers, scan_int,
};

pub(crate) fn begin_if<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    hooks: &mut H,
    condition: bool,
) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    input.push_condition(ConditionFrameSummary::new_if(condition));
    if !condition {
        skip_false_limb(input, stores, recorder, hooks)?;
    }
    Ok(Dispatch::Continue)
}

pub(crate) fn begin_ifcase<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    hooks: &mut H,
    selected_case: i32,
) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let take_initial_limb = selected_case == 0;
    input.push_condition(ConditionFrameSummary::new_ifcase(take_initial_limb));
    if !take_initial_limb {
        skip_ifcase_to_selected_limb(input, stores, recorder, hooks, selected_case)?;
    }
    Ok(Dispatch::Continue)
}

pub(crate) fn handle_else<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let frame = input
        .pop_condition()
        .ok_or(ExpandError::ExtraConditionalControl("else"))?;
    if matches!(frame.limb(), ConditionLimb::Else) {
        return Err(ExpandError::ExtraConditionalControl("else"));
    }

    let else_frame = frame.with_else_limb(!frame.any_limb_taken());
    input.push_condition(else_frame);
    if frame.any_limb_taken() {
        skip_to_fi(input, stores, recorder, hooks)?;
    }
    Ok(Dispatch::Continue)
}

pub(crate) fn handle_or<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let frame = input
        .pop_condition()
        .ok_or(ExpandError::ExtraConditionalControl("or"))?;
    if frame.kind() != ConditionKind::IfCase || frame.limb() == ConditionLimb::Else {
        return Err(ExpandError::ExtraConditionalControl("or"));
    }

    let next_or_count = frame.ifcase_or_count().saturating_add(1);
    input.push_condition(frame.with_or_limb(next_or_count, false));
    if frame.any_limb_taken() {
        skip_to_fi(input, stores, recorder, hooks)?;
    }
    Ok(Dispatch::Continue)
}

fn skip_false_limb<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<(), ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    skip_until(input, stores, recorder, hooks, SkipTarget::ElseOrFi)
}

fn skip_to_fi<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<(), ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    skip_until(input, stores, recorder, hooks, SkipTarget::Fi)
}

fn skip_ifcase_to_selected_limb<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    hooks: &mut H,
    selected_case: i32,
) -> Result<(), ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    skip_until(
        input,
        stores,
        recorder,
        hooks,
        SkipTarget::IfCaseSelection(selected_case),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SkipTarget {
    ElseOrFi,
    Fi,
    IfCaseSelection(i32),
}

fn skip_until<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    _hooks: &mut H,
    target: SkipTarget,
) -> Result<(), ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let mut nesting = 0_u32;
    loop {
        let Some(token) = input.next_token(stores)? else {
            return Err(ExpandError::IncompleteIf);
        };
        let Some(primitive) = skipped_conditional_control(stores, token, recorder)? else {
            continue;
        };

        match primitive {
            ConditionalPrimitive::If => {
                nesting = nesting.saturating_add(1);
            }
            ConditionalPrimitive::Else if nesting == 0 && target == SkipTarget::ElseOrFi => {
                move_current_if_to_else(input)?;
                return Ok(());
            }
            ConditionalPrimitive::Else
                if nesting == 0 && matches!(target, SkipTarget::IfCaseSelection(_)) =>
            {
                move_current_ifcase_to_else(input)?;
                return Ok(());
            }
            ConditionalPrimitive::Or
                if nesting == 0
                    && matches!(target, SkipTarget::IfCaseSelection(selected_case) if selected_case >= 0) =>
            {
                if move_current_ifcase_to_next_or(input, target)? {
                    return Ok(());
                }
            }
            ConditionalPrimitive::Fi if nesting == 0 => {
                input
                    .pop_condition()
                    .ok_or(ExpandError::ExtraConditionalControl("fi"))?;
                return Ok(());
            }
            ConditionalPrimitive::Fi => {
                nesting = nesting.saturating_sub(1);
            }
            ConditionalPrimitive::Else | ConditionalPrimitive::Or => {}
        }
    }
}

fn move_current_if_to_else<S>(input: &mut InputStack<S>) -> Result<(), ExpandError> {
    let frame = input
        .pop_condition()
        .ok_or(ExpandError::ExtraConditionalControl("else"))?;
    if frame.kind() != ConditionKind::If || frame.limb() == ConditionLimb::Else {
        return Err(ExpandError::ExtraConditionalControl("else"));
    }
    input.push_condition(frame.with_else_limb(!frame.any_limb_taken()));
    Ok(())
}

fn move_current_ifcase_to_next_or<S>(
    input: &mut InputStack<S>,
    target: SkipTarget,
) -> Result<bool, ExpandError> {
    let frame = input
        .pop_condition()
        .ok_or(ExpandError::ExtraConditionalControl("or"))?;
    if frame.kind() != ConditionKind::IfCase || frame.limb() == ConditionLimb::Else {
        return Err(ExpandError::ExtraConditionalControl("or"));
    }
    let next_or_count = frame.ifcase_or_count().saturating_add(1);
    let current_limb_taken = matches!(
        target,
        SkipTarget::IfCaseSelection(selected_case) if selected_case == next_or_count as i32
    );
    input.push_condition(frame.with_or_limb(next_or_count, current_limb_taken));
    Ok(current_limb_taken)
}

fn move_current_ifcase_to_else<S>(input: &mut InputStack<S>) -> Result<(), ExpandError> {
    let frame = input
        .pop_condition()
        .ok_or(ExpandError::ExtraConditionalControl("else"))?;
    if frame.kind() != ConditionKind::IfCase || frame.limb() == ConditionLimb::Else {
        return Err(ExpandError::ExtraConditionalControl("else"));
    }
    input.push_condition(frame.with_else_limb(!frame.any_limb_taken()));
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConditionalPrimitive {
    If,
    Else,
    Or,
    Fi,
}

fn skipped_conditional_control<R>(
    stores: &impl ExpansionState,
    token: Token,
    recorder: &mut R,
) -> Result<Option<ConditionalPrimitive>, ExpandError>
where
    R: ReadRecorder,
{
    let Token::Cs(symbol) = token else {
        return Ok(None);
    };
    let meaning = stores.meaning(symbol);
    recorder.record_meaning(symbol, meaning);
    match meaning {
        Meaning::ExpandablePrimitive(
            ExpandablePrimitive::IfTrue
            | ExpandablePrimitive::IfFalse
            | ExpandablePrimitive::If
            | ExpandablePrimitive::IfCat
            | ExpandablePrimitive::IfX
            | ExpandablePrimitive::IfNum
            | ExpandablePrimitive::IfDim
            | ExpandablePrimitive::IfOdd
            | ExpandablePrimitive::IfCase
            | ExpandablePrimitive::IfVMode
            | ExpandablePrimitive::IfHMode
            | ExpandablePrimitive::IfMMode
            | ExpandablePrimitive::IfInner
            | ExpandablePrimitive::IfVoid
            | ExpandablePrimitive::IfHBox
            | ExpandablePrimitive::IfVBox
            | ExpandablePrimitive::IfEof,
        ) => Ok(Some(ConditionalPrimitive::If)),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::Else) => {
            Ok(Some(ConditionalPrimitive::Else))
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::Or) => Ok(Some(ConditionalPrimitive::Or)),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::Fi) => Ok(Some(ConditionalPrimitive::Fi)),
        Meaning::Macro { flags, .. } if flags.contains(MeaningFlags::OUTER) => {
            Err(ExpandError::ForbiddenOuterTokenInSkippedConditional {
                name: format!("\\{}", stores.resolve(symbol)),
            })
        }
        _ => Ok(None),
    }
}

pub(crate) fn scan_condition_x_token<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
) -> Result<Token, ExpandError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    expander
        .next_expanded_token(input, stores, recorder, hooks)?
        .ok_or(ExpandError::MissingTokenAfterPrimitive(
            ExpandableOpcode::If,
        ))
}

pub(crate) fn if_char_equal(left: Token, right: Token) -> bool {
    match (left, right) {
        (Token::Char { ch: left, .. }, Token::Char { ch: right, .. }) => left == right,
        _ => false,
    }
}

pub(crate) fn if_cat_equal(left: Token, right: Token) -> bool {
    match (left, right) {
        (Token::Char { cat: left, .. }, Token::Char { cat: right, .. }) => left == right,
        (Token::Cs(_), Token::Cs(_)) => true,
        (Token::Param(_), Token::Param(_)) => true,
        _ => false,
    }
}

pub(crate) fn ifx_equal(stores: &impl ExpansionState, left: Token, right: Token) -> bool {
    match (left, right) {
        (Token::Char { .. } | Token::Param(_), Token::Char { .. } | Token::Param(_)) => {
            left == right
        }
        (Token::Cs(left), Token::Cs(right)) => meaning_words_ifx_equal(stores, left, right),
        _ => false,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConditionalRelation {
    Less,
    Equal,
    Greater,
}

pub(crate) fn scan_conditional_relation<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<ConditionalRelation, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let Some(token) =
        scan_helpers::next_non_space_x_token_with_hooks(input, stores, recorder, hooks)?
    else {
        return Err(ExpandError::MissingTokenAfterPrimitive(
            ExpandableOpcode::If,
        ));
    };
    match token {
        Token::Char { ch: '<', .. } => Ok(ConditionalRelation::Less),
        Token::Char { ch: '=', .. } => Ok(ConditionalRelation::Equal),
        Token::Char { ch: '>', .. } => Ok(ConditionalRelation::Greater),
        _ => Err(ExpandError::InvalidConditionalRelation(token)),
    }
}

pub(crate) fn compare_ordered<T>(left: T, relation: ConditionalRelation, right: T) -> bool
where
    T: Ord,
{
    match relation {
        ConditionalRelation::Less => left < right,
        ConditionalRelation::Equal => left == right,
        ConditionalRelation::Greater => left > right,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BoxKind {
    HBox,
    VBox,
}

pub(crate) fn box_register_has_kind(
    stores: &impl ExpansionState,
    index: u16,
    kind: BoxKind,
) -> bool {
    let Some(list) = stores.box_reg(index) else {
        return false;
    };
    matches!(
        (stores.nodes(list).first(), kind),
        (Some(Node::HList(_)), BoxKind::HBox) | (Some(Node::VList(_)), BoxKind::VBox)
    )
}

pub(crate) fn scan_stream_number<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<u8, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let value = scan_int::scan_int_with_recorder_and_hooks(input, stores, recorder, hooks)?.value();
    Ok(value.clamp(0, 15) as u8)
}

fn meaning_words_ifx_equal(stores: &impl ExpansionState, left: Symbol, right: Symbol) -> bool {
    let left = stores.meaning(left);
    let right = stores.meaning(right);
    match (left, right) {
        (
            Meaning::Macro {
                flags: left_flags,
                definition: left_definition,
            },
            Meaning::Macro {
                flags: right_flags,
                definition: right_definition,
            },
        ) => left_flags == right_flags && left_definition == right_definition,
        (Meaning::Macro { .. }, _) | (_, Meaning::Macro { .. }) => false,
        _ => left == right,
    }
}
