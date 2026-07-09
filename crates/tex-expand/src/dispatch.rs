use tex_lex::{InputSource, InputStack, MacroArguments};
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
use tex_state::page::PageMark;
use tex_state::provenance::{InsertedOriginKind, SynthesizedOriginKind};
use tex_state::token::{OriginId, Token, TracedTokenWord};
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
        OriginId::UNKNOWN,
        input,
        stores,
        recorder,
        &mut NoopExpansionHooks,
        meaning,
    )
}

fn page_mark_for_primitive(primitive: ExpandablePrimitive) -> PageMark {
    match primitive {
        ExpandablePrimitive::TopMark => PageMark::Top,
        ExpandablePrimitive::FirstMark => PageMark::First,
        ExpandablePrimitive::BotMark => PageMark::Bot,
        ExpandablePrimitive::SplitFirstMark => PageMark::SplitFirst,
        ExpandablePrimitive::SplitBotMark => PageMark::SplitBot,
        _ => unreachable!("caller restricts mark-family primitives"),
    }
}

macro_rules! dispatch_match {
    ($token:ident, $call_origin:ident, $input:ident, $stores:ident, $recorder:ident, $hooks:ident, $meaning:ident, $expander:expr, $input_arm:block) => {{
        let token = $token;
        let call_origin = $call_origin;
        let call_context = TracedTokenWord::pack(token, call_origin);
        let input = &mut *$input;
        let stores = &mut *$stores;
        let recorder = &mut *$recorder;
        let hooks = &mut *$hooks;
        let meaning = $meaning;
        let mut expander = $expander;
        match meaning {
            Meaning::Macro { flags, definition } if is_expandable_macro(flags) => {
                let macro_meaning = stores.macro_definition(definition);
                let provenance = stores.macro_definition_provenance(definition);
                let arguments = args::match_macro_call_with_recorder(
                    input,
                    stores,
                    recorder,
                    call_context,
                    macro_meaning,
                )?;
                Ok(Dispatch::Push {
                    replay_kind: ExpansionReplayKind::MacroBody,
                    token_list: macro_meaning.replacement_text(),
                    origin_list: provenance.replacement_origins(),
                    macro_arguments: arguments.as_macro_arguments(),
                    macro_invocation: stores.macro_invocation_origin(
                        definition,
                        call_origin,
                        provenance.definition_origin(),
                    ),
                })
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter) => {
                expand_after(input, stores, recorder, hooks, call_context)?;
                Ok(Dispatch::Continue)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand) => {
                let Some(token) = input.next_traced_token(stores)? else {
                    return Err(ExpandError::MissingTokenAfterPrimitive {
                        opcode: ExpandableOpcode::NoExpand,
                        context: call_context,
                    });
                };
                let semantic = crate::semantic_token(token);
                Ok(Dispatch::DeliverNoExpand(TracedTokenWord::pack(
                    semantic,
                    stores.inserted_origin(InsertedOriginKind::NoExpand, semantic, token.origin()),
                )))
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::CsName) => {
                let name = scan_csname(input, stores, recorder, hooks, call_context)?;
                let symbol = stores.intern_relaxed_control_sequence(&name);
                Ok(Dispatch::Push {
                    replay_kind: ExpansionReplayKind::Inserted,
                    token_list: stores.intern_token_list(&[Token::Cs(symbol)]),
                    origin_list: crate::synthesized_origin_list(
                        stores,
                        1,
                        call_origin,
                        SynthesizedOriginKind::Expansion,
                    ),
                    macro_arguments: MacroArguments::new(),
                    macro_invocation: OriginId::UNKNOWN,
                })
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName) => {
                Ok(Dispatch::Deliver(TracedTokenWord::pack(token, call_origin)))
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::String) => {
                let Some(target) = input.next_traced_token(stores)? else {
                    return Err(ExpandError::MissingTokenAfterPrimitive {
                        opcode: ExpandableOpcode::String,
                        context: call_context,
                    });
                };
                Ok(push_rendered_tokens(
                    stores,
                    ExpansionReplayKind::NumberOutput,
                    string_tokens(stores, crate::semantic_token(target)),
                    call_origin,
                ))
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Number) => {
                let scanned = scan_int::scan_int_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    call_context,
                )?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::NumberOutput,
                    &scanned.value().to_string(),
                    call_origin,
                ))
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::RomanNumeral) => {
                let scanned = scan_int::scan_int_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    call_context,
                )?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::NumberOutput,
                    &roman_numeral(scanned.value()),
                    call_origin,
                ))
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Meaning) => {
                let Some(target) = input.next_traced_token(stores)? else {
                    return Err(ExpandError::MissingTokenAfterPrimitive {
                        opcode: ExpandableOpcode::Meaning,
                        context: call_context,
                    });
                };
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::NumberOutput,
                    &meaning_text(stores, crate::semantic_token(target)),
                    call_origin,
                ))
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::The) => {
                expand_the_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    call_context,
                )
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
                call_origin,
            )),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::FontName) => {
                let font = scan_font_selector(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    call_context,
                )?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::NumberOutput,
                    &stores.font_name(font),
                    call_origin,
                ))
            }
            Meaning::ExpandablePrimitive(
                primitive @ (ExpandablePrimitive::TopMark
                | ExpandablePrimitive::FirstMark
                | ExpandablePrimitive::BotMark
                | ExpandablePrimitive::SplitFirstMark
                | ExpandablePrimitive::SplitBotMark),
            ) => Ok(Dispatch::Push {
                replay_kind: ExpansionReplayKind::Mark,
                token_list: stores.page_mark(page_mark_for_primitive(primitive)),
                origin_list: tex_state::ids::OriginListId::EMPTY,
                macro_arguments: MacroArguments::new(),
                macro_invocation: OriginId::UNKNOWN,
            }),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfTrue) => {
                begin_if(input, stores, recorder, hooks, true, call_context)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfFalse) => {
                begin_if(input, stores, recorder, hooks, false, call_context)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::If) => {
                begin_if_evaluation(input, call_context);
                let left = scan_condition_x_token(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    call_context,
                )?;
                let right = scan_condition_x_token(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    call_context,
                )?;
                complete_if_evaluation(
                    input,
                    stores,
                    recorder,
                    hooks,
                    if_char_equal(left, right),
                    call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfCat) => {
                begin_if_evaluation(input, call_context);
                let left = scan_condition_x_token(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    call_context,
                )?;
                let right = scan_condition_x_token(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    call_context,
                )?;
                complete_if_evaluation(
                    input,
                    stores,
                    recorder,
                    hooks,
                    if_cat_equal(left, right),
                    call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfX) => {
                begin_if_evaluation(input, call_context);
                let Some(left) = input.next_token(stores)? else {
                    return Err(ExpandError::MissingTokenAfterPrimitive {
                        opcode: ExpandableOpcode::If,
                        context: call_context,
                    });
                };
                let Some(right) = input.next_token(stores)? else {
                    return Err(ExpandError::MissingTokenAfterPrimitive {
                        opcode: ExpandableOpcode::If,
                        context: call_context,
                    });
                };
                complete_if_evaluation(
                    input,
                    stores,
                    recorder,
                    hooks,
                    ifx_equal(stores, left, right),
                    call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfNum) => {
                begin_if_evaluation(input, call_context);
                let left = scan_int::scan_int_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    call_context,
                )?
                .value();
                let relation = scan_conditional_relation_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    call_context,
                )?;
                let right = scan_int::scan_int_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    call_context,
                )?
                .value();
                complete_if_evaluation(
                    input,
                    stores,
                    recorder,
                    hooks,
                    compare_ordered(left, relation, right),
                    call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfDim) => {
                begin_if_evaluation(input, call_context);
                let left = scan_dimen::scan_dimen_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    scan_dimen::ScanDimenOptions::STANDARD,
                    call_context,
                )?
                .value();
                let relation = scan_conditional_relation_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    call_context,
                )?;
                let right = scan_dimen::scan_dimen_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    scan_dimen::ScanDimenOptions::STANDARD,
                    call_context,
                )?
                .value();
                complete_if_evaluation(
                    input,
                    stores,
                    recorder,
                    hooks,
                    compare_ordered(left, relation, right),
                    call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfOdd) => {
                begin_if_evaluation(input, call_context);
                let value = scan_int::scan_int_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    call_context,
                )?
                .value();
                complete_if_evaluation(input, stores, recorder, hooks, value % 2 != 0, call_context)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfCase) => {
                begin_ifcase_evaluation(input, call_context);
                let selected_case = scan_int::scan_int_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    call_context,
                )?
                .value();
                complete_ifcase_evaluation(
                    input,
                    stores,
                    recorder,
                    hooks,
                    selected_case,
                    call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfVMode) => begin_if(
                input,
                stores,
                recorder,
                hooks,
                hooks.mode() == EngineMode::Vertical,
                call_context,
            ),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfHMode) => begin_if(
                input,
                stores,
                recorder,
                hooks,
                hooks.mode() == EngineMode::Horizontal,
                call_context,
            ),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfMMode) => begin_if(
                input,
                stores,
                recorder,
                hooks,
                hooks.mode() == EngineMode::Math,
                call_context,
            ),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfInner) => begin_if(
                input,
                stores,
                recorder,
                hooks,
                hooks.is_inner_mode(),
                call_context,
            ),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfVoid) => {
                begin_if_evaluation(input, call_context);
                let index = scan_register_index_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    call_context,
                )?;
                complete_if_evaluation(
                    input,
                    stores,
                    recorder,
                    hooks,
                    stores.box_reg(index).is_none(),
                    call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfHBox) => {
                begin_if_evaluation(input, call_context);
                let index = scan_register_index_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    call_context,
                )?;
                complete_if_evaluation(
                    input,
                    stores,
                    recorder,
                    hooks,
                    box_register_has_kind(stores, index, BoxKind::HBox),
                    call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfVBox) => {
                begin_if_evaluation(input, call_context);
                let index = scan_register_index_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    call_context,
                )?;
                complete_if_evaluation(
                    input,
                    stores,
                    recorder,
                    hooks,
                    box_register_has_kind(stores, index, BoxKind::VBox),
                    call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfEof) => {
                begin_if_evaluation(input, call_context);
                let stream = scan_stream_number_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    call_context,
                )?;
                complete_if_evaluation(
                    input,
                    stores,
                    recorder,
                    hooks,
                    hooks.input_stream_eof(stores, stream),
                    call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Else) => {
                handle_else(token, call_origin, input, stores, recorder, hooks)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Or) => {
                handle_or(token, call_origin, input, stores, recorder, hooks)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Fi) => {
                handle_fi(token, call_origin, input, stores)
            }
            Meaning::Undefined => {
                let name = match token {
                    Token::Cs(symbol) => stores.resolve(symbol).to_owned(),
                    Token::Char {
                        ch,
                        cat: tex_state::token::Catcode::Active,
                    } => {
                        let symbol = stores.intern(&ch.to_string());
                        stores.resolve(symbol).to_owned()
                    }
                    Token::Char { .. } | Token::Param(_) => format!("{token:?}"),
                };
                Err(ExpandError::UndefinedControlSequence {
                    name,
                    context: call_context,
                })
            }
            Meaning::Macro { .. }
            | Meaning::Relax
            | Meaning::CharGiven(_)
            | Meaning::CharToken { .. }
            | Meaning::MathCharGiven(_)
            | Meaning::CountRegister(_)
            | Meaning::DimenRegister(_)
            | Meaning::SkipRegister(_)
            | Meaning::MuskipRegister(_)
            | Meaning::ToksRegister(_)
            | Meaning::IntParam(_)
            | Meaning::InternalInteger(_)
            | Meaning::DimenParam(_)
            | Meaning::GlueParam(_)
            | Meaning::MuGlueParam(_)
            | Meaning::TokParam(_)
            | Meaning::PageDimension(_)
            | Meaning::PageInteger(_)
            | Meaning::Font(_)
            | Meaning::UnexpandablePrimitive(_)
            | Meaning::Unknown(_) => {
                Ok(Dispatch::Deliver(TracedTokenWord::pack(token, call_origin)))
            }
        }
    }};
}

pub fn dispatch_with_hooks<S, R, H>(
    token: Token,
    call_origin: OriginId,
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
        call_origin,
        input,
        stores,
        recorder,
        hooks,
        meaning,
        DriverExpandNext,
        {
            let context = TracedTokenWord::pack(token, call_origin);
            let name = scan_input_name(input, stores, recorder, hooks, context)?;
            let source = hooks
                .open_input(&mut stores.input_open_context(), &name)
                .map_err(|message| ExpandError::InputOpen {
                    name: name.clone(),
                    message,
                    context,
                })?;
            input.push_source(source);
            Ok(Dispatch::Continue)
        }
    )
}

pub(crate) fn dispatch_without_input_open<S, R, H>(
    token: Token,
    call_origin: OriginId,
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
        call_origin,
        input,
        stores,
        recorder,
        hooks,
        meaning,
        NoInputExpandNext,
        {
            let context = TracedTokenWord::pack(token, call_origin);
            Err(ExpandError::InputOpen {
                name: "\\input".to_owned(),
                message: "\\input requires input-open authority".to_owned(),
                context,
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
