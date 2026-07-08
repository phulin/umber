use tex_lex::{InputSource, InputStack};
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
use tex_state::token::Token;
use tex_state::{ExpansionState, InputOpenState};

use crate::{
    Dispatch, DriverExpandNext, EngineMode, ExpandError, ExpandableOpcode, ExpansionHooks,
    ExpansionReplayKind, NoInputExpandNext, NoopExpansionHooks, ReadRecorder, args,
    conditionals::*, primitives::*, scan_dimen, scan_helpers::*, scan_int, values::*,
};

/// Dispatches one token/meaning pair.
pub fn dispatch<S, R>(
    token: Token,
    input: &mut InputStack<S>,
    stores: &mut (impl ExpansionState + InputOpenState),
    recorder: &mut R,
    meaning: Meaning,
) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
{
    dispatch_with_hooks(
        token,
        input,
        stores,
        recorder,
        &mut NoopExpansionHooks,
        meaning,
    )
}

macro_rules! dispatch_match {
    ($token:ident, $input:ident, $stores:ident, $recorder:ident, $hooks:ident, $meaning:ident, $expander:expr, $input_arm:block) => {{
        let token = $token;
        let input = &mut *$input;
        let stores = &mut *$stores;
        let recorder = &mut *$recorder;
        let hooks = &mut *$hooks;
        let meaning = $meaning;
        let mut expander = $expander;
        match meaning {
            Meaning::Macro { flags, definition } if is_expandable_macro(flags) => {
                let macro_meaning = stores.macro_definition(definition);
                let arguments = args::match_macro_call_with_recorder(
                    input,
                    stores,
                    recorder,
                    token,
                    macro_meaning,
                )?;
                Ok(Dispatch::Push {
                    replay_kind: ExpansionReplayKind::MacroBody,
                    token_list: macro_meaning.replacement_text(),
                    macro_arguments: arguments.as_macro_arguments(),
                })
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter) => {
                expand_after(input, stores, recorder, hooks)?;
                Ok(Dispatch::Continue)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand) => {
                let Some(token) = input.next_token(stores)? else {
                    return Err(ExpandError::MissingTokenAfterPrimitive(
                        ExpandableOpcode::NoExpand,
                    ));
                };
                Ok(Dispatch::DeliverNoExpand(token))
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::CsName) => {
                let name = scan_csname(input, stores, recorder, hooks)?;
                let symbol = stores.intern_relaxed_control_sequence(&name);
                Ok(Dispatch::Deliver(Token::Cs(symbol)))
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName) => {
                Ok(Dispatch::Deliver(token))
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::String) => {
                let Some(target) = input.next_token(stores)? else {
                    return Err(ExpandError::MissingTokenAfterPrimitive(
                        ExpandableOpcode::String,
                    ));
                };
                Ok(push_rendered_tokens(
                    stores,
                    ExpansionReplayKind::NumberOutput,
                    string_tokens(stores, target),
                ))
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Number) => {
                let scanned = scan_int::scan_int_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                )?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::NumberOutput,
                    &scanned.value().to_string(),
                ))
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::RomanNumeral) => {
                let scanned = scan_int::scan_int_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                )?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::NumberOutput,
                    &roman_numeral(scanned.value()),
                ))
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Meaning) => {
                let Some(target) = input.next_token(stores)? else {
                    return Err(ExpandError::MissingTokenAfterPrimitive(
                        ExpandableOpcode::Meaning,
                    ));
                };
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::NumberOutput,
                    &meaning_text(stores, target),
                ))
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::The) => {
                expand_the_with_expander_and_hooks(input, stores, recorder, hooks, &mut expander)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Input) => $input_arm,
            Meaning::ExpandablePrimitive(ExpandablePrimitive::EndInput) => {
                input.end_current_source_after_current_line();
                Ok(Dispatch::Continue)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::JobName) => Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::JobName,
                hooks.job_name(),
            )),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::FontName) => {
                let font = scan_font_selector(input, stores, recorder, hooks, &mut expander)?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::NumberOutput,
                    &stores.font_name(font),
                ))
            }
            Meaning::ExpandablePrimitive(
                ExpandablePrimitive::TopMark
                | ExpandablePrimitive::FirstMark
                | ExpandablePrimitive::BotMark
                | ExpandablePrimitive::SplitFirstMark
                | ExpandablePrimitive::SplitBotMark,
            ) => {
                // TODO(umber2-page): return the page builder's stored mark token
                // lists once mark nodes and page splitting exist.
                Ok(push_rendered_text(stores, ExpansionReplayKind::Mark, ""))
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfTrue) => {
                begin_if(input, stores, recorder, hooks, true)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfFalse) => {
                begin_if(input, stores, recorder, hooks, false)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::If) => {
                let left = scan_condition_x_token(input, stores, recorder, hooks, &mut expander)?;
                let right = scan_condition_x_token(input, stores, recorder, hooks, &mut expander)?;
                begin_if(input, stores, recorder, hooks, if_char_equal(left, right))
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfCat) => {
                let left = scan_condition_x_token(input, stores, recorder, hooks, &mut expander)?;
                let right = scan_condition_x_token(input, stores, recorder, hooks, &mut expander)?;
                begin_if(input, stores, recorder, hooks, if_cat_equal(left, right))
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfX) => {
                let Some(left) = input.next_token(stores)? else {
                    return Err(ExpandError::MissingTokenAfterPrimitive(
                        ExpandableOpcode::If,
                    ));
                };
                let Some(right) = input.next_token(stores)? else {
                    return Err(ExpandError::MissingTokenAfterPrimitive(
                        ExpandableOpcode::If,
                    ));
                };
                begin_if(
                    input,
                    stores,
                    recorder,
                    hooks,
                    ifx_equal(stores, left, right),
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfNum) => {
                let left = scan_int::scan_int_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                )?
                .value();
                let relation = scan_conditional_relation_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                )?;
                let right = scan_int::scan_int_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                )?
                .value();
                begin_if(
                    input,
                    stores,
                    recorder,
                    hooks,
                    compare_ordered(left, relation, right),
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfDim) => {
                let left = scan_dimen::scan_dimen_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    scan_dimen::ScanDimenOptions::STANDARD,
                )?
                .value();
                let relation = scan_conditional_relation_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                )?;
                let right = scan_dimen::scan_dimen_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    scan_dimen::ScanDimenOptions::STANDARD,
                )?
                .value();
                begin_if(
                    input,
                    stores,
                    recorder,
                    hooks,
                    compare_ordered(left, relation, right),
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfOdd) => {
                let value = scan_int::scan_int_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                )?
                .value();
                begin_if(input, stores, recorder, hooks, value % 2 != 0)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfCase) => {
                let selected_case = scan_int::scan_int_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                )?
                .value();
                begin_ifcase(input, stores, recorder, hooks, selected_case)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfVMode) => begin_if(
                input,
                stores,
                recorder,
                hooks,
                hooks.mode() == EngineMode::Vertical,
            ),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfHMode) => begin_if(
                input,
                stores,
                recorder,
                hooks,
                hooks.mode() == EngineMode::Horizontal,
            ),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfMMode) => begin_if(
                input,
                stores,
                recorder,
                hooks,
                hooks.mode() == EngineMode::Math,
            ),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfInner) => {
                begin_if(input, stores, recorder, hooks, hooks.is_inner_mode())
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfVoid) => {
                let index = scan_register_index_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                )?;
                begin_if(
                    input,
                    stores,
                    recorder,
                    hooks,
                    stores.box_reg(index).is_none(),
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfHBox) => {
                let index = scan_register_index_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                )?;
                begin_if(
                    input,
                    stores,
                    recorder,
                    hooks,
                    box_register_has_kind(stores, index, BoxKind::HBox),
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfVBox) => {
                let index = scan_register_index_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                )?;
                begin_if(
                    input,
                    stores,
                    recorder,
                    hooks,
                    box_register_has_kind(stores, index, BoxKind::VBox),
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfEof) => {
                let stream = scan_stream_number_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                )?;
                begin_if(
                    input,
                    stores,
                    recorder,
                    hooks,
                    hooks.input_stream_eof(stores, stream),
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Else) => {
                handle_else(input, stores, recorder, hooks)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Or) => {
                handle_or(input, stores, recorder, hooks)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Fi) => {
                input
                    .pop_condition()
                    .ok_or(ExpandError::ExtraConditionalControl("fi"))?;
                Ok(Dispatch::Continue)
            }
            Meaning::Macro { .. }
            | Meaning::Undefined
            | Meaning::Relax
            | Meaning::CharGiven(_)
            | Meaning::MathCharGiven(_)
            | Meaning::CountRegister(_)
            | Meaning::DimenRegister(_)
            | Meaning::SkipRegister(_)
            | Meaning::MuskipRegister(_)
            | Meaning::ToksRegister(_)
            | Meaning::IntParam(_)
            | Meaning::DimenParam(_)
            | Meaning::GlueParam(_)
            | Meaning::TokParam(_)
            | Meaning::PageDimension(_)
            | Meaning::PageInteger(_)
            | Meaning::Font(_)
            | Meaning::UnexpandablePrimitive(_)
            | Meaning::Unknown(_) => Ok(Dispatch::Deliver(token)),
        }
    }};
}

pub fn dispatch_with_hooks<S, R, H>(
    token: Token,
    input: &mut InputStack<S>,
    stores: &mut (impl ExpansionState + InputOpenState),
    recorder: &mut R,
    hooks: &mut H,
    meaning: Meaning,
) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    dispatch_match!(
        token,
        input,
        stores,
        recorder,
        hooks,
        meaning,
        DriverExpandNext,
        {
            let name = scan_input_name(input, stores, recorder, hooks)?;
            let source = hooks
                .open_input(&mut stores.input_open_context(), &name)
                .map_err(|message| ExpandError::InputOpen {
                    name: name.clone(),
                    message,
                })?;
            input.push_source(source);
            Ok(Dispatch::Continue)
        }
    )
}

pub(crate) fn dispatch_without_input_open<S, R, H>(
    token: Token,
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    hooks: &mut H,
    meaning: Meaning,
) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    dispatch_match!(
        token,
        input,
        stores,
        recorder,
        hooks,
        meaning,
        NoInputExpandNext,
        {
            Err(ExpandError::InputOpen {
                name: "\\input".to_owned(),
                message: "\\input requires input-open authority".to_owned(),
            })
        }
    )
}

const fn is_expandable_macro(flags: MeaningFlags) -> bool {
    !flags.contains(MeaningFlags::PROTECTED)
}

/// Skeleton dispatch table for all expandable opcode families in this epic.
pub fn dispatch_expandable_opcode(opcode: ExpandableOpcode) -> Result<(), ExpandError> {
    match opcode {
        ExpandableOpcode::Macro => Ok(()),
        ExpandableOpcode::ExpandAfter
        | ExpandableOpcode::NoExpand
        | ExpandableOpcode::CsName
        | ExpandableOpcode::EndCsName
        | ExpandableOpcode::String
        | ExpandableOpcode::Number
        | ExpandableOpcode::RomanNumeral
        | ExpandableOpcode::Meaning
        | ExpandableOpcode::The
        | ExpandableOpcode::Input
        | ExpandableOpcode::EndInput
        | ExpandableOpcode::JobName
        | ExpandableOpcode::FontName
        | ExpandableOpcode::Mark
        | ExpandableOpcode::If
        | ExpandableOpcode::Else
        | ExpandableOpcode::Or
        | ExpandableOpcode::Fi => Ok(()),
    }
}
