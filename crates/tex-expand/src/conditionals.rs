use tex_lex::{
    ConditionFrameSummary, ConditionFrameToken, ConditionKind, ConditionLimb, InputSource,
    InputStack,
};
use tex_state::ExpansionState;
use tex_state::interner::Symbol;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
use tex_state::provenance::InsertedOriginKind;
use tex_state::token::{OriginId, Token, TracedTokenWord};

use crate::{
    Dispatch, ExpandError, ExpandNext, ExpandableOpcode, ExpansionContext, NoInputExpandNext,
    expandable_symbol, push_inserted_token, scan_helpers, scan_int, semantic_token,
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

pub(crate) fn begin_if_evaluation<S>(
    input: &mut InputStack<S>,
    context: TracedTokenWord,
    metadata: ConditionMetadata,
) -> ConditionFrameToken {
    input.push_condition(metadata.apply(ConditionFrameSummary::evaluating_if(context)))
}

pub(crate) fn begin_ifcase_evaluation<S>(
    input: &mut InputStack<S>,
    context: TracedTokenWord,
    metadata: ConditionMetadata,
) -> ConditionFrameToken {
    input.push_condition(metadata.apply(ConditionFrameSummary::evaluating_ifcase(context)))
}

pub(crate) fn begin_if<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    expansion: &mut ExpansionContext<'_, S>,
    condition: bool,
    metadata: ConditionMetadata,
    context: TracedTokenWord,
) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
{
    let frame_token =
        input.push_condition(metadata.apply(ConditionFrameSummary::new_if(context, condition)));
    if !condition {
        skip_false_limb(input, stores, expansion, frame_token)?;
    }
    Ok(Dispatch::Continue)
}

pub(crate) fn complete_if_evaluation<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    expansion: &mut ExpansionContext<'_, S>,
    condition: bool,
    frame_token: ConditionFrameToken,
) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
{
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

pub(crate) fn complete_ifcase_evaluation<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    expansion: &mut ExpansionContext<'_, S>,
    selected_case: i32,
    frame_token: ConditionFrameToken,
) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
{
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

pub(crate) fn handle_else<S>(
    token: Token,
    origin: OriginId,
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    expansion: &mut ExpansionContext<'_, S>,
) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
{
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

pub(crate) fn handle_or<S>(
    token: Token,
    origin: OriginId,
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    expansion: &mut ExpansionContext<'_, S>,
) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
{
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

pub(crate) fn handle_fi<S>(
    token: Token,
    origin: OriginId,
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
{
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

fn current_condition_is_evaluating<S>(input: &InputStack<S>) -> bool {
    input
        .current_condition()
        .is_some_and(ConditionFrameSummary::evaluating)
}

fn insert_relax_before_token<S>(
    token: Token,
    origin: OriginId,
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
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

fn skip_false_limb<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    expansion: &mut ExpansionContext<'_, S>,
    frame_token: ConditionFrameToken,
) -> Result<(), ExpandError>
where
    S: InputSource,
{
    skip_until(input, stores, expansion, SkipTarget::ElseOrFi, frame_token)
}

fn skip_to_fi<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    expansion: &mut ExpansionContext<'_, S>,
    frame_token: ConditionFrameToken,
) -> Result<(), ExpandError>
where
    S: InputSource,
{
    skip_until(input, stores, expansion, SkipTarget::Fi, frame_token)
}

fn skip_ifcase_to_selected_limb<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    expansion: &mut ExpansionContext<'_, S>,
    selected_case: i32,
    frame_token: ConditionFrameToken,
) -> Result<(), ExpandError>
where
    S: InputSource,
{
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

fn skip_until<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    expansion: &mut ExpansionContext<'_, S>,
    target: SkipTarget,
    frame_token: ConditionFrameToken,
) -> Result<(), ExpandError>
where
    S: InputSource,
{
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

fn move_current_if_to_else<S>(
    input: &mut InputStack<S>,
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

fn move_current_ifcase_to_next_or<S>(
    input: &mut InputStack<S>,
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

fn move_current_ifcase_to_else<S>(
    input: &mut InputStack<S>,
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

fn skipped_conditional_control<S>(
    stores: &mut impl ExpansionState,
    expansion: &mut ExpansionContext<'_, S>,
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
                context: token,
            })
        }
        _ => Ok(None),
    }
}

pub(crate) fn scan_condition_x_token<S, St, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    expansion: &mut ExpansionContext<'_, S>,
    expander: &mut E,
    context: TracedTokenWord,
) -> Result<Token, ExpandError>
where
    S: InputSource,
    St: ExpansionState,
    E: ExpandNext<S, St>,
{
    let token = expander
        .next_expanded_token(input, stores, expansion)?
        .ok_or(ExpandError::MissingTokenAfterPrimitive {
            opcode: ExpandableOpcode::If,
            context,
        })?;
    Ok(semantic_token(token))
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

#[allow(dead_code)]
pub(crate) fn scan_conditional_relation<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    expansion: &mut ExpansionContext<'_, S>,
    context: TracedTokenWord,
) -> Result<ConditionalRelation, ExpandError>
where
    S: InputSource,
{
    scan_conditional_relation_with_expander_and_context(
        input,
        stores,
        expansion,
        &mut NoInputExpandNext,
        context,
    )
}

pub(crate) fn scan_conditional_relation_with_expander_and_context<S, St, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    expansion: &mut ExpansionContext<'_, S>,
    expander: &mut E,
    context: TracedTokenWord,
) -> Result<ConditionalRelation, ExpandError>
where
    S: InputSource,
    St: ExpansionState,
    E: ExpandNext<S, St>,
{
    let Some(token) = scan_helpers::next_non_space_x_token_with_expander_and_context(
        input, stores, expansion, expander,
    )?
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
pub(crate) fn scan_stream_number<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    expansion: &mut ExpansionContext<'_, S>,
    context: TracedTokenWord,
) -> Result<u8, ExpandError>
where
    S: InputSource,
{
    scan_stream_number_with_expander_and_context(
        input,
        stores,
        expansion,
        &mut NoInputExpandNext,
        context,
    )
}

pub(crate) fn scan_stream_number_with_expander_and_context<S, St, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    expansion: &mut ExpansionContext<'_, S>,
    expander: &mut E,
    context: TracedTokenWord,
) -> Result<u8, ExpandError>
where
    S: InputSource,
    St: ExpansionState,
    E: ExpandNext<S, St>,
{
    let value =
        scan_int::scan_int_with_expander_and_context(input, stores, expansion, expander, context)?
            .value();
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
