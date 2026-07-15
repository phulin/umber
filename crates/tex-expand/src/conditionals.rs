use tex_lex::{
    ConditionFrameSummary, ConditionFrameToken, ConditionKind, ConditionLimb, InputStack,
};
use tex_state::ExpansionState;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
use tex_state::provenance::InsertedOriginKind;
use tex_state::token::{OriginId, Token, TracedTokenWord};

use crate::{
    Dispatch, ExpandError, ExpandableOpcode, ExpansionContext, ExpansionMode,
    RestrictedExpansionMode, expandable_symbol, push_inserted_token, scan_helpers, scan_int,
    semantic_token,
};

#[derive(Clone, Copy)]
pub(crate) struct ConditionMetadata {
    if_type: u8,
    inverted: bool,
}

impl ConditionMetadata {
    pub(crate) const fn new(if_type: u8, inverted: bool) -> Self {
        Self { if_type, inverted }
    }

    const fn apply(self, frame: ConditionFrameSummary) -> ConditionFrameSummary {
        frame
            .with_if_type(self.if_type)
            .with_inverted(self.inverted)
    }
}

pub(crate) fn begin_if_evaluation(
    input: &mut InputStack,
    context: TracedTokenWord,
    metadata: ConditionMetadata,
) -> ConditionFrameToken {
    input.push_condition(metadata.apply(ConditionFrameSummary::evaluating_if(context)))
}

pub(crate) fn begin_ifcase_evaluation(
    input: &mut InputStack,
    context: TracedTokenWord,
    metadata: ConditionMetadata,
) -> ConditionFrameToken {
    input.push_condition(metadata.apply(ConditionFrameSummary::evaluating_ifcase(context)))
}

pub(crate) fn begin_if(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    condition: bool,
    metadata: ConditionMetadata,
    context: TracedTokenWord,
) -> Result<Dispatch, ExpandError> {
    let frame_token =
        input.push_condition(metadata.apply(ConditionFrameSummary::new_if(context, condition)));
    if !condition {
        skip_false_limb(input, stores, expansion, frame_token)?;
    }
    Ok(Dispatch::Continue)
}

pub(crate) fn complete_if_evaluation(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    condition: bool,
    frame_token: ConditionFrameToken,
) -> Result<Dispatch, ExpandError> {
    let current = input
        .current_condition()
        .expect("the evaluating conditional frame remains current");
    let context = current.context();
    let metadata = ConditionMetadata::new(current.if_type(), current.inverted());
    let previous = input
        .update_condition(
            frame_token,
            metadata.apply(ConditionFrameSummary::new_if(context, condition)),
        )
        .ok_or(ExpandError::IncompleteIf { context })?;
    debug_assert!(previous.evaluating());
    if !condition {
        skip_false_limb(input, stores, expansion, frame_token)?;
    }
    Ok(Dispatch::Continue)
}

pub(crate) fn complete_ifcase_evaluation(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    selected_case: i32,
    frame_token: ConditionFrameToken,
) -> Result<Dispatch, ExpandError> {
    let current = input
        .current_condition()
        .expect("the evaluating ifcase frame remains current");
    let context = current.context();
    let take_initial_limb = selected_case == 0;
    let previous = input
        .update_condition(
            frame_token,
            ConditionMetadata::new(17, false).apply(ConditionFrameSummary::new_ifcase(
                context,
                take_initial_limb,
            )),
        )
        .ok_or(ExpandError::IncompleteIf { context })?;
    debug_assert!(previous.evaluating());
    if !take_initial_limb {
        skip_ifcase_to_selected_limb(input, stores, expansion, selected_case, frame_token)?;
    }
    Ok(Dispatch::Continue)
}

pub(crate) fn handle_else(
    token: Token,
    origin: OriginId,
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
) -> Result<Dispatch, ExpandError> {
    let context = TracedTokenWord::pack(token, origin);
    if current_condition_is_evaluating(input) {
        insert_relax_before_token(token, origin, input, stores);
        return Ok(Dispatch::Continue);
    }

    let frame_token =
        input
            .current_condition_token()
            .ok_or(ExpandError::ExtraConditionalControl {
                name: "else",
                context,
            })?;
    let frame = input
        .current_condition()
        .expect("a current condition token identifies a condition frame");
    if matches!(frame.limb(), ConditionLimb::Else) {
        return Err(ExpandError::ExtraConditionalControl {
            name: "else",
            context,
        });
    }

    let else_frame = frame.with_else_limb(!frame.any_limb_taken());
    input
        .update_condition(frame_token, else_frame)
        .expect("the current condition frame remains live");
    if frame.any_limb_taken() {
        skip_to_fi(input, stores, expansion, frame_token)?;
    }
    Ok(Dispatch::Continue)
}

pub(crate) fn handle_or(
    token: Token,
    origin: OriginId,
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
) -> Result<Dispatch, ExpandError> {
    let context = TracedTokenWord::pack(token, origin);
    if current_condition_is_evaluating(input) {
        insert_relax_before_token(token, origin, input, stores);
        return Ok(Dispatch::Continue);
    }

    let frame_token =
        input
            .current_condition_token()
            .ok_or(ExpandError::ExtraConditionalControl {
                name: "or",
                context,
            })?;
    let frame = input
        .current_condition()
        .expect("a current condition token identifies a condition frame");
    if frame.kind() != ConditionKind::IfCase || frame.limb() == ConditionLimb::Else {
        return Err(ExpandError::ExtraConditionalControl {
            name: "or",
            context,
        });
    }

    let next_or_count = frame.ifcase_or_count().saturating_add(1);
    input
        .update_condition(frame_token, frame.with_or_limb(next_or_count, false))
        .expect("the current condition frame remains live");
    if frame.any_limb_taken() {
        skip_to_fi(input, stores, expansion, frame_token)?;
    }
    Ok(Dispatch::Continue)
}

pub(crate) fn handle_fi(
    token: Token,
    origin: OriginId,
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
) -> Result<Dispatch, ExpandError> {
    let context = TracedTokenWord::pack(token, origin);
    if current_condition_is_evaluating(input) {
        insert_relax_before_token(token, origin, input, stores);
        return Ok(Dispatch::Continue);
    }

    input
        .pop_condition()
        .ok_or(ExpandError::ExtraConditionalControl {
            name: "fi",
            context,
        })?;
    Ok(Dispatch::Continue)
}

fn current_condition_is_evaluating(input: &InputStack) -> bool {
    input
        .current_condition()
        .is_some_and(ConditionFrameSummary::evaluating)
}

fn insert_relax_before_token(
    token: Token,
    origin: OriginId,
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
) {
    let relax = stores.intern_relaxed_control_sequence("relax");
    push_inserted_token(
        input,
        stores,
        TracedTokenWord::pack(token, origin),
        InsertedOriginKind::Unread,
    );
    push_inserted_token(
        input,
        stores,
        TracedTokenWord::pack(Token::Cs(relax.symbol()), origin),
        InsertedOriginKind::ErrorRecovery,
    );
}

fn skip_false_limb(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    frame_token: ConditionFrameToken,
) -> Result<(), ExpandError> {
    skip_until(input, stores, expansion, SkipTarget::ElseOrFi, frame_token)
}

fn skip_to_fi(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    frame_token: ConditionFrameToken,
) -> Result<(), ExpandError> {
    skip_until(input, stores, expansion, SkipTarget::Fi, frame_token)
}

fn skip_ifcase_to_selected_limb(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    selected_case: i32,
    frame_token: ConditionFrameToken,
) -> Result<(), ExpandError> {
    skip_until(
        input,
        stores,
        expansion,
        SkipTarget::IfCaseSelection(selected_case),
        frame_token,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SkipTarget {
    ElseOrFi,
    Fi,
    IfCaseSelection(i32),
}

fn skip_until(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    target: SkipTarget,
    frame_token: ConditionFrameToken,
) -> Result<(), ExpandError> {
    let mut nesting = 0_u32;
    loop {
        let Some(token) = crate::next_semantic_raw_token(input, stores)? else {
            let context = input
                .current_condition()
                .expect("conditional skipping requires an open condition frame")
                .context();
            return Err(ExpandError::IncompleteIf { context });
        };
        let primitive = match skipped_conditional_control(stores, expansion, token) {
            Ok(Some(primitive)) => primitive,
            Ok(None) => continue,
            Err(ExpandError::ForbiddenOuterTokenInSkippedConditional { .. }) => {
                // TeX.web §336 backs up the outer token and inserts a
                // frozen \fi. Skipped nested conditions are represented only
                // by `nesting`, so this ultimately closes the one live frame.
                push_inserted_token(input, stores, token, InsertedOriginKind::Unread);
                input
                    .pop_condition()
                    .expect("conditional skipping retains its live frame");
                return Ok(());
            }
            Err(error) => return Err(error),
        };

        match primitive {
            ConditionalPrimitive::If => {
                nesting = nesting.saturating_add(1);
            }
            ConditionalPrimitive::Else
                if nesting == 0
                    && input.current_condition_token() == Some(frame_token)
                    && target == SkipTarget::ElseOrFi =>
            {
                move_current_if_to_else(input, token)?;
                return Ok(());
            }
            ConditionalPrimitive::Else
                if nesting == 0
                    && input.current_condition_token() == Some(frame_token)
                    && matches!(target, SkipTarget::IfCaseSelection(_)) =>
            {
                move_current_ifcase_to_else(input, token)?;
                return Ok(());
            }
            ConditionalPrimitive::Or
                if nesting == 0
                    && input.current_condition_token() == Some(frame_token)
                    && matches!(target, SkipTarget::IfCaseSelection(selected_case) if selected_case >= 0) =>
            {
                if move_current_ifcase_to_next_or(input, target, token)? {
                    return Ok(());
                }
            }
            ConditionalPrimitive::Fi
                if nesting == 0 && input.current_condition_token() == Some(frame_token) =>
            {
                input
                    .pop_condition()
                    .ok_or(ExpandError::ExtraConditionalControl {
                        name: "fi",
                        context: token,
                    })?;
                return Ok(());
            }
            ConditionalPrimitive::Fi if nesting == 0 => {
                input
                    .pop_condition()
                    .ok_or(ExpandError::ExtraConditionalControl {
                        name: "fi",
                        context: token,
                    })?;
            }
            ConditionalPrimitive::Fi => {
                nesting = nesting.saturating_sub(1);
            }
            ConditionalPrimitive::Else | ConditionalPrimitive::Or => {}
        }
    }
}

fn move_current_if_to_else(
    input: &mut InputStack,
    context: TracedTokenWord,
) -> Result<(), ExpandError> {
    let frame_token =
        input
            .current_condition_token()
            .ok_or(ExpandError::ExtraConditionalControl {
                name: "else",
                context,
            })?;
    let frame = input
        .current_condition()
        .expect("a current condition token identifies a condition frame");
    if frame.kind() != ConditionKind::If || frame.limb() == ConditionLimb::Else {
        return Err(ExpandError::ExtraConditionalControl {
            name: "else",
            context,
        });
    }
    input
        .update_condition(frame_token, frame.with_else_limb(!frame.any_limb_taken()))
        .expect("the current condition frame remains live");
    Ok(())
}

fn move_current_ifcase_to_next_or(
    input: &mut InputStack,
    target: SkipTarget,
    context: TracedTokenWord,
) -> Result<bool, ExpandError> {
    let frame_token =
        input
            .current_condition_token()
            .ok_or(ExpandError::ExtraConditionalControl {
                name: "or",
                context,
            })?;
    let frame = input
        .current_condition()
        .expect("a current condition token identifies a condition frame");
    if frame.kind() != ConditionKind::IfCase || frame.limb() == ConditionLimb::Else {
        return Err(ExpandError::ExtraConditionalControl {
            name: "or",
            context,
        });
    }
    let next_or_count = frame.ifcase_or_count().saturating_add(1);
    let current_limb_taken = matches!(
        target,
        SkipTarget::IfCaseSelection(selected_case) if selected_case == next_or_count as i32
    );
    input
        .update_condition(
            frame_token,
            frame.with_or_limb(next_or_count, current_limb_taken),
        )
        .expect("the current condition frame remains live");
    Ok(current_limb_taken)
}

fn move_current_ifcase_to_else(
    input: &mut InputStack,
    context: TracedTokenWord,
) -> Result<(), ExpandError> {
    let frame_token =
        input
            .current_condition_token()
            .ok_or(ExpandError::ExtraConditionalControl {
                name: "else",
                context,
            })?;
    let frame = input
        .current_condition()
        .expect("a current condition token identifies a condition frame");
    if frame.kind() != ConditionKind::IfCase || frame.limb() == ConditionLimb::Else {
        return Err(ExpandError::ExtraConditionalControl {
            name: "else",
            context,
        });
    }
    input
        .update_condition(frame_token, frame.with_else_limb(!frame.any_limb_taken()))
        .expect("the current condition frame remains live");
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConditionalPrimitive {
    If,
    Else,
    Or,
    Fi,
}

fn skipped_conditional_control(
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    token: TracedTokenWord,
) -> Result<Option<ConditionalPrimitive>, ExpandError> {
    let Some(symbol) = expandable_symbol(stores, token) else {
        return Ok(None);
    };
    let meaning = stores.meaning(symbol);
    expansion.record_meaning(symbol, meaning);
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
            | ExpandablePrimitive::IfEof
            | ExpandablePrimitive::IfDefined
            | ExpandablePrimitive::IfCsName
            | ExpandablePrimitive::IfFontChar,
        ) => Ok(Some(ConditionalPrimitive::If)),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::Else) => {
            Ok(Some(ConditionalPrimitive::Else))
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::Or) => Ok(Some(ConditionalPrimitive::Or)),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::Fi) => Ok(Some(ConditionalPrimitive::Fi)),
        Meaning::Macro { flags, .. } if flags.contains(MeaningFlags::OUTER) => {
            Err(ExpandError::ForbiddenOuterTokenInSkippedConditional {
                name: format!("\\{}", stores.resolve(symbol)),
                context: token,
            })
        }
        _ => Ok(None),
    }
}

pub(crate) fn scan_condition_x_token(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
) -> Result<Token, ExpandError>
where
{
    let token = mode.next_expanded_token(input, stores, expansion)?.ok_or(
        ExpandError::MissingTokenAfterPrimitive {
            opcode: ExpandableOpcode::If,
            context,
        },
    )?;
    let token = semantic_token(token);
    Ok(match token {
        Token::Cs(symbol) => match stores.meaning(symbol) {
            Meaning::CharToken { ch, cat } => Token::Char { ch, cat },
            _ => token,
        },
        _ => token,
    })
}

pub(crate) fn if_char_equal(left: Token, right: Token) -> bool {
    match (left, right) {
        (Token::Char { ch: left, .. }, Token::Char { ch: right, .. }) => left == right,
        (Token::Cs(_), Token::Cs(_)) => true,
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

#[derive(Clone, Copy)]
pub(crate) struct IfxOperand {
    token: Token,
    meaning: Option<Meaning>,
}

pub(crate) fn scan_ifx_operand(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
) -> Result<IfxOperand, ExpandError> {
    loop {
        let Some(read) = input.next_traced_expansion_token(stores)? else {
            return Err(ExpandError::MissingTokenAfterPrimitive {
                opcode: ExpandableOpcode::If,
                context,
            });
        };
        expansion.observe_read(read);
        let token = read.token();
        let traced = read.traced_token();
        let Some(symbol) = crate::expandable_symbol(stores, traced) else {
            return Ok(IfxOperand {
                token,
                meaning: None,
            });
        };
        let meaning = expansion.resolve_meaning(input, stores, symbol);
        expansion.record_meaning(symbol, meaning);
        if !read.suppress_expansion()
            && meaning == Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand)
        {
            let dispatch = mode.dispatch_raw_token(traced, input, stores, expansion)?;
            crate::push_dispatch_result(input, stores, dispatch);
            continue;
        }
        let meaning = if read.suppress_expansion()
            && matches!(
                meaning,
                Meaning::Undefined | Meaning::Macro { .. } | Meaning::ExpandablePrimitive(_)
            ) {
            Meaning::Relax
        } else {
            meaning
        };
        return Ok(IfxOperand {
            token,
            meaning: Some(meaning),
        });
    }
}

pub(crate) fn ifx_operands_equal(
    stores: &impl ExpansionState,
    left: IfxOperand,
    right: IfxOperand,
) -> bool {
    match (left.meaning, right.meaning) {
        (Some(left), Some(right)) => meanings_ifx_equal(stores, left, right),
        (Some(left), None) => raw_token_ifx_meaning(right.token)
            .is_some_and(|right| meanings_ifx_equal(stores, left, right)),
        (None, Some(right)) => raw_token_ifx_meaning(left.token)
            .is_some_and(|left| meanings_ifx_equal(stores, left, right)),
        (None, None) => left.token == right.token,
    }
}

/// Returns the command-and-character meaning TeX82 uses for an unbound token.
///
/// In `tex.web`'s `ifx` branch, TeX first compares `cur_cmd` and then compares
/// `cur_chr` for non-macros. Consequently a control sequence `\let` to a
/// character compares equal to the raw character token; token provenance is
/// irrelevant once both command meanings have been obtained.
fn raw_token_ifx_meaning(token: Token) -> Option<Meaning> {
    match token {
        Token::Char { ch, cat } => Some(Meaning::CharToken { ch, cat }),
        Token::Cs(_) | Token::Param(_) | Token::Frozen(_) => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConditionalRelation {
    Less,
    Equal,
    Greater,
}

#[allow(dead_code)]
pub(crate) fn scan_conditional_relation(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    context: TracedTokenWord,
) -> Result<ConditionalRelation, ExpandError> {
    scan_conditional_relation_with_mode_and_context(
        input,
        stores,
        expansion,
        &mut RestrictedExpansionMode,
        context,
    )
}

pub(crate) fn scan_conditional_relation_with_mode_and_context(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
) -> Result<ConditionalRelation, ExpandError>
where
{
    let Some(token) =
        scan_helpers::next_non_space_x_token_with_mode_and_context(input, stores, expansion, mode)?
    else {
        return Err(ExpandError::MissingTokenAfterPrimitive {
            opcode: ExpandableOpcode::If,
            context,
        });
    };
    match semantic_token(token) {
        Token::Char { ch: '<', .. } => Ok(ConditionalRelation::Less),
        Token::Char { ch: '=', .. } => Ok(ConditionalRelation::Equal),
        Token::Char { ch: '>', .. } => Ok(ConditionalRelation::Greater),
        _ => {
            // TeX.web §500 uses `back_error`: the offending token is the
            // first token of the right operand, and `=` is assumed.
            push_inserted_token(input, stores, token, InsertedOriginKind::Unread);
            Ok(ConditionalRelation::Equal)
        }
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
        (
            Some(tex_state::node_arena::NodeRef::HList(_)),
            BoxKind::HBox
        ) | (
            Some(tex_state::node_arena::NodeRef::VList(_)),
            BoxKind::VBox
        )
    )
}

#[allow(dead_code)]
pub(crate) fn scan_stream_number(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    context: TracedTokenWord,
) -> Result<u8, ExpandError> {
    scan_stream_number_with_mode_and_context(
        input,
        stores,
        expansion,
        &mut RestrictedExpansionMode,
        context,
    )
}

pub(crate) fn scan_stream_number_with_mode_and_context(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
) -> Result<u8, ExpandError>
where
{
    let value =
        scan_int::scan_int_with_mode_and_context(input, stores, expansion, mode, context)?.value();
    Ok(value.clamp(0, 15) as u8)
}

fn meanings_ifx_equal(stores: &impl ExpansionState, left: Meaning, right: Meaning) -> bool {
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
        ) => {
            left_flags == right_flags
                && stores
                    .macro_definition(left_definition)
                    .semantic_eq(stores.macro_definition(right_definition))
        }
        (Meaning::Macro { .. }, _) | (_, Meaning::Macro { .. }) => false,
        _ => left == right,
    }
}
