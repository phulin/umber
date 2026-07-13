use tex_lex::{InputSource, InputStack, MacroArguments};
use tex_state::meaning::{ExpandablePrimitive, Meaning};
use tex_state::page::PageMark;
use tex_state::provenance::{InsertedOriginKind, SynthesizedOriginKind};
use tex_state::token::{OriginId, Token, TracedTokenWord};
use tex_state::{ExpansionState, InputOpenState};

use crate::{
    Dispatch, DriverExpandNext, EngineMode, ExpandError, ExpandNext, ExpandableOpcode,
    ExpansionHooks, ExpansionReplayKind, NoInputExpandNext, NoopExpansionHooks, ReadRecorder, args,
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

const fn page_mark_key(mark: PageMark) -> u8 {
    match mark {
        PageMark::Top => 0,
        PageMark::First => 1,
        PageMark::Bot => 2,
        PageMark::SplitFirst => 3,
        PageMark::SplitBot => 4,
    }
}

macro_rules! dispatch_match {
    ($token:ident, $call_origin:ident, $input:ident, $stores:ident, $recorder:ident, $hooks:ident, $meaning:ident, $invert:expr, $expander:expr, $input_arm:block) => {{
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
            Meaning::Macro { definition, .. } => {
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
                        input.active_macro_invocation(),
                    ),
                })
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter) => {
                expand_after(input, stores, recorder, hooks, &mut expander, call_context)?;
                Ok(Dispatch::Continue)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand) => {
                // Delivery is classified exactly once by the expansion loop's
                // `DeliverNoExpand` arm below this dispatch boundary.
                let Some(token) = crate::next_unintercepted_raw_token(input, stores)? else {
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
                    token_list: stores.intern_token_list(&[Token::Cs(symbol.symbol())]),
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
            Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate) => {
                Ok(Dispatch::Deliver(TracedTokenWord::pack(
                    stores.frozen_endv_token(),
                    call_origin,
                )))
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::String) => {
                let Some(target) = crate::next_semantic_raw_token(input, stores)? else {
                    return Err(ExpandError::MissingTokenAfterPrimitive {
                        opcode: ExpandableOpcode::String,
                        context: call_context,
                    });
                };
                if matches!(crate::semantic_token(target), Token::Cs(_)) {
                    recorder.record_dependency(crate::ReadDependency::Cell {
                        bank: crate::ReadBank::IntParam,
                        index: u32::from(tex_state::env::banks::IntParam::ESCAPE_CHAR.raw()),
                    });
                }
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
                let Some(target) = crate::next_semantic_raw_token(input, stores)? else {
                    return Err(ExpandError::MissingTokenAfterPrimitive {
                        opcode: ExpandableOpcode::Meaning,
                        context: call_context,
                    });
                };
                if let Token::Cs(symbol) = crate::semantic_token(target) {
                    let meaning = stores.meaning(symbol);
                    recorder.record_meaning(symbol, meaning);
                    crate::values::record_meaning_value_dependency(recorder, meaning);
                }
                recorder.record_dependency(crate::ReadDependency::Cell {
                    bank: crate::ReadBank::IntParam,
                    index: u32::from(tex_state::env::banks::IntParam::ESCAPE_CHAR.raw()),
                });
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
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Unexpanded) => {
                let raw = crate::scan::scan_general_text(input, stores, call_context).map_err(
                    |error| match error {
                        crate::scan::ScanToksError::Lex(error) => ExpandError::Lex(error),
                        crate::scan::ScanToksError::Expand(error) => error,
                        _ => ExpandError::MissingTokenAfterPrimitive {
                            opcode: ExpandableOpcode::Unexpanded,
                            context: call_context,
                        },
                    },
                )?;
                Ok(Dispatch::Push {
                    replay_kind: ExpansionReplayKind::Unexpanded,
                    token_list: raw.token_list(),
                    origin_list: raw.origin_list(),
                    macro_arguments: MacroArguments::new(),
                    macro_invocation: OriginId::UNKNOWN,
                })
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Detokenize) => {
                let raw = crate::scan::scan_general_text(input, stores, call_context).map_err(
                    |error| match error {
                        crate::scan::ScanToksError::Lex(error) => ExpandError::Lex(error),
                        crate::scan::ScanToksError::Expand(error) => error,
                        _ => ExpandError::MissingTokenAfterPrimitive {
                            opcode: ExpandableOpcode::Detokenize,
                            context: call_context,
                        },
                    },
                )?;
                let mut rendered = String::new();
                for &token in stores.tokens(raw.token_list()) {
                    append_token_show_text(stores, token, &mut rendered);
                }
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::NumberOutput,
                    &rendered,
                    call_origin,
                ))
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Unless) => {
                let Some(target) = crate::next_semantic_raw_token(input, stores)? else {
                    return Err(ExpandError::MissingTokenAfterPrimitive {
                        opcode: ExpandableOpcode::Unless,
                        context: call_context,
                    });
                };
                let target_meaning = crate::expandable_symbol(stores, target)
                    .map(|symbol| stores.meaning(symbol));
                if !matches!(
                    target_meaning,
                    Some(Meaning::ExpandablePrimitive(primitive))
                        if is_boolean_conditional(primitive)
                ) {
                    return Err(ExpandError::MissingTokenAfterPrimitive {
                        opcode: ExpandableOpcode::Unless,
                        context: target,
                    });
                }
                expander.dispatch_inverted_raw_token(target, input, stores, recorder, hooks)
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
                recorder.record_dependency(crate::ReadDependency::Font {
                    field: crate::ReadFontField::Name,
                    font: font.raw(),
                    index: 0,
                });
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
            ) => {
                let mark = page_mark_for_primitive(primitive);
                recorder.record_dependency(crate::ReadDependency::PageMark(page_mark_key(mark)));
                Ok(Dispatch::Push {
                    replay_kind: ExpansionReplayKind::Mark,
                    token_list: stores.page_mark(mark),
                    origin_list: tex_state::ids::OriginListId::EMPTY,
                    macro_arguments: MacroArguments::new(),
                    macro_invocation: OriginId::UNKNOWN,
                })
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfTrue) => {
                begin_if(input, stores, recorder, hooks, true ^ $invert, call_context)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfFalse) => {
                begin_if(input, stores, recorder, hooks, false ^ $invert, call_context)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::If) => {
                let frame_token = begin_if_evaluation(input, call_context);
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
                    if_char_equal(left, right) ^ $invert,
                    call_context,
                    frame_token,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfCat) => {
                let frame_token = begin_if_evaluation(input, call_context);
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
                    if_cat_equal(left, right) ^ $invert,
                    call_context,
                    frame_token,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfX) => {
                let frame_token = begin_if_evaluation(input, call_context);
                let Some(left) = crate::next_semantic_raw_token(input, stores)? else {
                    return Err(ExpandError::MissingTokenAfterPrimitive {
                        opcode: ExpandableOpcode::If,
                        context: call_context,
                    });
                };
                let Some(right) = crate::next_semantic_raw_token(input, stores)? else {
                    return Err(ExpandError::MissingTokenAfterPrimitive {
                        opcode: ExpandableOpcode::If,
                        context: call_context,
                    });
                };
                let left = crate::semantic_token(left);
                let right = crate::semantic_token(right);
                for operand in [left, right] {
                    if let Token::Cs(symbol) = operand {
                        let meaning = stores.meaning(symbol);
                        recorder.record_meaning(symbol, meaning);
                    }
                }
                complete_if_evaluation(
                    input,
                    stores,
                    recorder,
                    hooks,
                    ifx_equal(stores, left, right) ^ $invert,
                    call_context,
                    frame_token,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfNum) => {
                let frame_token = begin_if_evaluation(input, call_context);
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
                    compare_ordered(left, relation, right) ^ $invert,
                    call_context,
                    frame_token,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfDim) => {
                let frame_token = begin_if_evaluation(input, call_context);
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
                    compare_ordered(left, relation, right) ^ $invert,
                    call_context,
                    frame_token,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfOdd) => {
                let frame_token = begin_if_evaluation(input, call_context);
                let value = scan_int::scan_int_with_expander_and_hooks(
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
                    (value % 2 != 0) ^ $invert,
                    call_context,
                    frame_token,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfCase) => {
                let frame_token = begin_ifcase_evaluation(input, call_context);
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
                    frame_token,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfVMode) => {
                recorder
                    .record_dependency(crate::ReadDependency::Engine(crate::ReadEngineField::Mode));
                begin_if(
                    input,
                    stores,
                    recorder,
                    hooks,
                    (hooks.mode() == EngineMode::Vertical) ^ $invert,
                    call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfHMode) => {
                recorder
                    .record_dependency(crate::ReadDependency::Engine(crate::ReadEngineField::Mode));
                begin_if(
                    input,
                    stores,
                    recorder,
                    hooks,
                    (hooks.mode() == EngineMode::Horizontal) ^ $invert,
                    call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfMMode) => {
                recorder
                    .record_dependency(crate::ReadDependency::Engine(crate::ReadEngineField::Mode));
                begin_if(
                    input,
                    stores,
                    recorder,
                    hooks,
                    (hooks.mode() == EngineMode::Math) ^ $invert,
                    call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfInner) => {
                recorder.record_dependency(crate::ReadDependency::Engine(
                    crate::ReadEngineField::InnerMode,
                ));
                begin_if(
                    input,
                    stores,
                    recorder,
                    hooks,
                    hooks.is_inner_mode() ^ $invert,
                    call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfVoid) => {
                let frame_token = begin_if_evaluation(input, call_context);
                let index = scan_register_index_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    call_context,
                )?;
                recorder.record_dependency(crate::ReadDependency::Cell {
                    bank: crate::ReadBank::Box,
                    index: u32::from(index),
                });
                complete_if_evaluation(
                    input,
                    stores,
                    recorder,
                    hooks,
                    stores.box_reg(index).is_none() ^ $invert,
                    call_context,
                    frame_token,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfHBox) => {
                let frame_token = begin_if_evaluation(input, call_context);
                let index = scan_register_index_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    call_context,
                )?;
                recorder.record_dependency(crate::ReadDependency::Cell {
                    bank: crate::ReadBank::Box,
                    index: u32::from(index),
                });
                complete_if_evaluation(
                    input,
                    stores,
                    recorder,
                    hooks,
                    box_register_has_kind(stores, index, BoxKind::HBox) ^ $invert,
                    call_context,
                    frame_token,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfVBox) => {
                let frame_token = begin_if_evaluation(input, call_context);
                let index = scan_register_index_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    call_context,
                )?;
                recorder.record_dependency(crate::ReadDependency::Cell {
                    bank: crate::ReadBank::Box,
                    index: u32::from(index),
                });
                complete_if_evaluation(
                    input,
                    stores,
                    recorder,
                    hooks,
                    box_register_has_kind(stores, index, BoxKind::VBox) ^ $invert,
                    call_context,
                    frame_token,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfEof) => {
                let frame_token = begin_if_evaluation(input, call_context);
                let stream = scan_stream_number_with_expander_and_hooks(
                    input,
                    stores,
                    recorder,
                    hooks,
                    &mut expander,
                    call_context,
                )?;
                recorder.record_dependency(crate::ReadDependency::InputStream(stream));
                complete_if_evaluation(
                    input,
                    stores,
                    recorder,
                    hooks,
                    hooks.input_stream_eof(stores, stream) ^ $invert,
                    call_context,
                    frame_token,
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
                    Token::Char { .. } | Token::Param(_) | Token::Frozen(_) => {
                        format!("{token:?}")
                    }
                };
                Err(ExpandError::UndefinedControlSequence {
                    name,
                    context: call_context,
                })
            }
            Meaning::Relax
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
        false,
        DriverExpandNext,
        {
            let context = TracedTokenWord::pack(token, call_origin);
            let name = scan_input_name(input, stores, recorder, hooks, context)?;
            let transfer_endinput = input.take_current_source_end_after_current_line();
            let source = hooks
                .open_input(&mut stores.input_open_context(), &name)
                .map_err(|message| ExpandError::InputOpen {
                    name: name.clone(),
                    message,
                    context,
                })?;
            input.push_source(source);
            if transfer_endinput {
                input.end_current_source_after_current_line();
            }
            Ok(Dispatch::Continue)
        }
    )
}

pub(crate) fn dispatch_with_hooks_inverted<S, R, H>(
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
        true,
        DriverExpandNext,
        { unreachable!("boolean conditionals cannot open input") }
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
        false,
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

pub(crate) fn dispatch_without_input_open_inverted<S, R, H>(
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
        true,
        NoInputExpandNext,
        { unreachable!("boolean conditionals cannot open input") }
    )
}

fn is_boolean_conditional(primitive: ExpandablePrimitive) -> bool {
    matches!(
        primitive,
        ExpandablePrimitive::IfTrue
            | ExpandablePrimitive::IfFalse
            | ExpandablePrimitive::If
            | ExpandablePrimitive::IfCat
            | ExpandablePrimitive::IfX
            | ExpandablePrimitive::IfNum
            | ExpandablePrimitive::IfDim
            | ExpandablePrimitive::IfOdd
            | ExpandablePrimitive::IfVMode
            | ExpandablePrimitive::IfHMode
            | ExpandablePrimitive::IfMMode
            | ExpandablePrimitive::IfInner
            | ExpandablePrimitive::IfVoid
            | ExpandablePrimitive::IfHBox
            | ExpandablePrimitive::IfVBox
            | ExpandablePrimitive::IfEof
    )
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
        | ExpandableOpcode::Unexpanded
        | ExpandableOpcode::Detokenize
        | ExpandableOpcode::Unless
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
