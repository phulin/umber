use tex_lex::{InputStack, MacroArguments};
use tex_state::meaning::{ExpandablePrimitive, Meaning};
use tex_state::page::PageMark;
use tex_state::provenance::{InsertedOriginKind, SynthesizedOriginKind};
use tex_state::token::{OriginId, Token, TracedTokenWord};
use tex_state::{ExpansionState, InputOpenState};

use crate::{
    Dispatch, DriverExpansionMode, EngineMode, ExpandError, ExpandableOpcode, ExpansionContext,
    ExpansionMode, ExpansionReplayKind, RestrictedExpansionMode, args, conditionals::*,
    primitives::*, scan_dimen, scan_helpers::*, scan_int, values::*,
};

/// Dispatches one token/meaning pair.
pub fn dispatch(
    token: Token,
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    meaning: Meaning,
) -> Result<Dispatch, ExpandError> {
    dispatch_with_context(
        token,
        OriginId::UNKNOWN,
        input,
        stores,
        &mut ExpansionContext::new("texput"),
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
        ExpandablePrimitive::TopMarks => PageMark::Top,
        ExpandablePrimitive::FirstMarks => PageMark::First,
        ExpandablePrimitive::BotMarks => PageMark::Bot,
        ExpandablePrimitive::SplitFirstMarks => PageMark::SplitFirst,
        ExpandablePrimitive::SplitBotMarks => PageMark::SplitBot,
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
    ($token:ident, $call_origin:ident, $input:ident, $stores:ident, $context:ident, $meaning:ident, $invert:expr, $mode:expr, $input_arm:block, $filesize_arm:block) => {{
        let token = $token;
        let call_origin = $call_origin;
        let call_context = TracedTokenWord::pack(token, call_origin);
        let input = &mut *$input;
        let stores = &mut *$stores;
        let expansion = &mut *$context;
        let meaning = $meaning;
        let mode = &mut *$mode;
        match meaning {
            Meaning::Macro { definition, .. } => {
                let macro_meaning = stores.macro_definition(definition);
                let provenance = stores.macro_definition_provenance(definition);
                let arguments = args::match_macro_call_with_context(
                    input,
                    stores,
                    expansion,
                    call_context,
                    macro_meaning,
                )?;
                Ok(Dispatch::Push {
                    replay_kind: ExpansionReplayKind::MacroBody,
                    token_list: macro_meaning.replacement_text(),
                    origin_list: provenance.replacement_origins(),
                    macro_arguments: arguments.into_macro_arguments(),
                    macro_invocation: stores.macro_invocation_origin(
                        definition,
                        call_origin,
                        provenance.definition_origin(),
                        input.active_macro_invocation(),
                    ),
                })
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter) => {
                expand_after(input, stores, expansion, mode, call_context)?;
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
                let name = scan_csname(input, stores, expansion, call_context)?;
                let symbol = stores.intern_relaxed_control_sequence(&name);
                let origin = stores.synthesized_origin(
                    SynthesizedOriginKind::Expansion,
                    call_origin,
                );
                Ok(Dispatch::PushTransient {
                    replay_kind: ExpansionReplayKind::Inserted,
                    tokens: vec![TracedTokenWord::pack(Token::Cs(symbol.symbol()), origin)],
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
                    crate::record_dependency!(expansion, crate::ReadDependency::Cell {
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
                let scanned = scan_int::scan_int_with_mode_and_context(
                    input,
                    stores, expansion,
                    mode,
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
                let scanned = scan_int::scan_int_with_mode_and_context(
                    input,
                    stores, expansion,
                    mode,
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
                    expansion.record_meaning(symbol, meaning);
                    crate::values::record_meaning_value_dependency(expansion, meaning);
                }
                crate::record_dependency!(expansion, crate::ReadDependency::Cell {
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
                expand_the_with_mode_and_context(
                    input,
                    stores, expansion,
                    mode,
                    call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Expanded) => {
                let expanded = crate::scan::scan_general_text_expanded_with_expanded_open(
                    input,
                    stores,
                    expansion,
                    mode,
                    call_context,
                )
                .map_err(|error| match error {
                    crate::scan::ScanToksError::Lex(error) => ExpandError::Lex(error),
                    crate::scan::ScanToksError::Expand(error) => error,
                    _ => ExpandError::MissingTokenAfterPrimitive {
                        opcode: ExpandableOpcode::Expanded,
                        context: call_context,
                    },
                })?;
                Ok(Dispatch::Push {
                    replay_kind: ExpansionReplayKind::Inserted,
                    token_list: expanded.token_list(),
                    origin_list: expanded.origin_list(),
                    macro_arguments: MacroArguments::new(),
                    macro_invocation: OriginId::UNKNOWN,
                })
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::FileSize) => $filesize_arm,
            Meaning::ExpandablePrimitive(ExpandablePrimitive::StringCompare) => {
                execute_string_compare_primitive(
                    input, stores, expansion, mode, call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Unexpanded) => {
                let raw = crate::scan::scan_general_text_with_expanded_open(
                    input, stores, expansion, mode, call_context,
                ).map_err(
                    |error| match error {
                        crate::scan::ScanToksError::Lex(error) => ExpandError::Lex(error),
                        crate::scan::ScanToksError::Expand(error) => error,
                        _ => ExpandError::MissingTokenAfterPrimitive {
                            opcode: ExpandableOpcode::Unexpanded,
                            context: call_context,
                        },
                    },
                )?;
                let origin_list = crate::expansion_suppressed_origin_list(
                    stores,
                    raw.token_list(),
                    raw.origin_list(),
                    OriginId::UNKNOWN,
                );
                Ok(Dispatch::Push {
                    replay_kind: ExpansionReplayKind::Unexpanded,
                    token_list: raw.token_list(),
                    origin_list,
                    macro_arguments: MacroArguments::new(),
                    macro_invocation: OriginId::UNKNOWN,
                })
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Detokenize) => {
                let raw = crate::scan::scan_general_text_with_expanded_open(
                    input, stores, expansion, mode, call_context,
                ).map_err(
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
                    crate::append_token_string_text(stores, token, &mut rendered);
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
                mode.dispatch_inverted_raw_token(target, input, stores, expansion)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Scantokens) => {
                let raw = crate::scan::scan_general_text_with_expanded_open(
                    input, stores, expansion, mode, call_context,
                )?;
                let mut text = String::new();
                for &token in stores.tokens(raw.token_list()) {
                    crate::append_token_string_text(stores, token, &mut text);
                }
                text.push('\n');
                let source = tex_lex::MemoryInput::scantokens(text);
                stores.trace_scantokens_boundary(true);
                input.push_source(source);
                Ok(Dispatch::Continue)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::ETeXVersion) => {
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::NumberOutput,
                    "2",
                    call_origin,
                ))
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::ETeXRevision) => {
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::NumberOutput,
                    ".6",
                    call_origin,
                ))
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Input) => $input_arm,
            Meaning::ExpandablePrimitive(ExpandablePrimitive::EndInput) => {
                input.end_current_source_after_current_line();
                Ok(Dispatch::Continue)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::JobName) => Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::JobName,
                expansion.job_name,
                call_origin,
            )),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::FontName) => {
                let font = scan_font_selector(
                    input,
                    stores, expansion,
                    mode,
                    call_context,
                )?;
                crate::record_dependency!(expansion, crate::ReadDependency::Font {
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
                crate::record_dependency!(expansion, crate::ReadDependency::PageMark(page_mark_key(mark)));
                Ok(Dispatch::Push {
                    replay_kind: ExpansionReplayKind::Mark,
                    token_list: stores.page_mark(mark),
                    origin_list: tex_state::ids::OriginListId::EMPTY,
                    macro_arguments: MacroArguments::new(),
                    macro_invocation: OriginId::UNKNOWN,
                })
            }
            Meaning::ExpandablePrimitive(
                primitive @ (ExpandablePrimitive::TopMarks
                | ExpandablePrimitive::FirstMarks
                | ExpandablePrimitive::BotMarks
                | ExpandablePrimitive::SplitFirstMarks
                | ExpandablePrimitive::SplitBotMarks),
            ) => {
                let mark = page_mark_for_primitive(primitive);
                let scanned = scan_int::scan_int_with_mode_and_context(
                    input,
                    stores, expansion,
                    mode,
                    call_context,
                )?;
                let class = if (0..=32_767).contains(&scanned.value()) {
                    scanned.value() as u16
                } else {
                    stores.report_bad_register_code(scanned.value(), 32_767);
                    0
                };
                crate::record_dependency!(expansion, crate::ReadDependency::PageMarkClass {
                    mark: page_mark_key(mark),
                    class,
                });
                Ok(Dispatch::Push {
                    replay_kind: ExpansionReplayKind::Mark,
                    token_list: stores.page_mark_class(mark, class),
                    origin_list: tex_state::ids::OriginListId::EMPTY,
                    macro_arguments: MacroArguments::new(),
                    macro_invocation: OriginId::UNKNOWN,
                })
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfTrue) => {
                begin_if(
                    input,
                    stores, expansion,
                    true ^ $invert,
                    ConditionMetadata::new(15, $invert),
                    call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfFalse) => {
                begin_if(
                    input,
                    stores, expansion,
                    false ^ $invert,
                    ConditionMetadata::new(16, $invert),
                    call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::If) => {
                let frame_token = begin_if_evaluation(
                    input,
                    call_context,
                    ConditionMetadata::new(1, $invert),
                );
                let left = scan_condition_x_token(
                    input,
                    stores, expansion,
                    mode,
                    call_context,
                )?;
                let right = scan_condition_x_token(
                    input,
                    stores, expansion,
                    mode,
                    call_context,
                )?;
                complete_if_evaluation(
                    input,
                    stores, expansion,
                    if_char_equal(left, right) ^ $invert,
                    frame_token,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfCat) => {
                let frame_token = begin_if_evaluation(input, call_context, ConditionMetadata::new(2, $invert));
                let left = scan_condition_x_token(
                    input,
                    stores, expansion,
                    mode,
                    call_context,
                )?;
                let right = scan_condition_x_token(
                    input,
                    stores, expansion,
                    mode,
                    call_context,
                )?;
                complete_if_evaluation(
                    input,
                    stores, expansion,
                    if_cat_equal(left, right) ^ $invert,
                    frame_token,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfX) => {
                let frame_token = begin_if_evaluation(input, call_context, ConditionMetadata::new(13, $invert));
                let left = scan_ifx_operand(
                    input, stores, expansion, mode, call_context,
                )?;
                let right = scan_ifx_operand(
                    input, stores, expansion, mode, call_context,
                )?;
                complete_if_evaluation(
                    input,
                    stores, expansion,
                    ifx_operands_equal(stores, left, right) ^ $invert,
                    frame_token,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfNum) => {
                let frame_token = begin_if_evaluation(input, call_context, ConditionMetadata::new(3, $invert));
                let left = scan_int::scan_int_with_mode_and_context(
                    input,
                    stores, expansion,
                    mode,
                    call_context,
                )?
                .value();
                let relation = scan_conditional_relation_with_mode_and_context(
                    input,
                    stores, expansion,
                    mode,
                    call_context,
                )?;
                let right = scan_int::scan_int_with_mode_and_context(
                    input,
                    stores, expansion,
                    mode,
                    call_context,
                )?
                .value();
                complete_if_evaluation(
                    input,
                    stores, expansion,
                    compare_ordered(left, relation, right) ^ $invert,
                    frame_token,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfDim) => {
                let frame_token = begin_if_evaluation(input, call_context, ConditionMetadata::new(4, $invert));
                let left = scan_dimen::scan_dimen_with_mode_and_context(
                    input,
                    stores, expansion,
                    mode,
                    scan_dimen::ScanDimenOptions::STANDARD,
                    call_context,
                )?
                .value();
                let relation = scan_conditional_relation_with_mode_and_context(
                    input,
                    stores, expansion,
                    mode,
                    call_context,
                )?;
                let right = scan_dimen::scan_dimen_with_mode_and_context(
                    input,
                    stores, expansion,
                    mode,
                    scan_dimen::ScanDimenOptions::STANDARD,
                    call_context,
                )?
                .value();
                complete_if_evaluation(
                    input,
                    stores, expansion,
                    compare_ordered(left, relation, right) ^ $invert,
                    frame_token,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfOdd) => {
                let frame_token = begin_if_evaluation(input, call_context, ConditionMetadata::new(5, $invert));
                let value = scan_int::scan_int_with_mode_and_context(
                    input,
                    stores, expansion,
                    mode,
                    call_context,
                )?
                .value();
                complete_if_evaluation(
                    input,
                    stores, expansion,
                    (value % 2 != 0) ^ $invert,
                    frame_token,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfCase) => {
                let frame_token = begin_ifcase_evaluation(input, call_context, ConditionMetadata::new(17, false));
                let selected_case = scan_int::scan_int_with_mode_and_context(
                    input,
                    stores, expansion,
                    mode,
                    call_context,
                )?
                .value();
                complete_ifcase_evaluation(
                    input,
                    stores, expansion,
                    selected_case,
                    frame_token,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfVMode) => {
                crate::record_dependency!(expansion, crate::ReadDependency::Engine(crate::ReadEngineField::Mode));
                begin_if(
                    input,
                    stores, expansion,
                    (expansion.engine.mode == EngineMode::Vertical) ^ $invert,
                    ConditionMetadata::new(6, $invert),
                    call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfHMode) => {
                crate::record_dependency!(expansion, crate::ReadDependency::Engine(crate::ReadEngineField::Mode));
                begin_if(
                    input,
                    stores, expansion,
                    (expansion.engine.mode == EngineMode::Horizontal) ^ $invert,
                    ConditionMetadata::new(7, $invert),
                    call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfMMode) => {
                crate::record_dependency!(expansion, crate::ReadDependency::Engine(crate::ReadEngineField::Mode));
                begin_if(
                    input,
                    stores, expansion,
                    (expansion.engine.mode == EngineMode::Math) ^ $invert,
                    ConditionMetadata::new(8, $invert),
                    call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfInner) => {
                crate::record_dependency!(expansion, crate::ReadDependency::Engine(
                    crate::ReadEngineField::InnerMode,
                ));
                begin_if(
                    input,
                    stores, expansion,
                    expansion.engine.is_inner_mode ^ $invert,
                    ConditionMetadata::new(9, $invert),
                    call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfVoid) => {
                let frame_token = begin_if_evaluation(input, call_context, ConditionMetadata::new(10, $invert));
                let index = scan_register_index_with_mode_and_context(
                    input,
                    stores, expansion,
                    mode,
                    call_context,
                )?;
                crate::record_dependency!(expansion, crate::ReadDependency::Cell {
                    bank: crate::ReadBank::Box,
                    index: u32::from(index),
                });
                complete_if_evaluation(
                    input,
                    stores, expansion,
                    stores.box_reg(index).is_none() ^ $invert,
                    frame_token,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfHBox) => {
                let frame_token = begin_if_evaluation(input, call_context, ConditionMetadata::new(11, $invert));
                let index = scan_register_index_with_mode_and_context(
                    input,
                    stores, expansion,
                    mode,
                    call_context,
                )?;
                crate::record_dependency!(expansion, crate::ReadDependency::Cell {
                    bank: crate::ReadBank::Box,
                    index: u32::from(index),
                });
                complete_if_evaluation(
                    input,
                    stores, expansion,
                    box_register_has_kind(stores, index, BoxKind::HBox) ^ $invert,
                    frame_token,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfVBox) => {
                let frame_token = begin_if_evaluation(input, call_context, ConditionMetadata::new(12, $invert));
                let index = scan_register_index_with_mode_and_context(
                    input,
                    stores, expansion,
                    mode,
                    call_context,
                )?;
                crate::record_dependency!(expansion, crate::ReadDependency::Cell {
                    bank: crate::ReadBank::Box,
                    index: u32::from(index),
                });
                complete_if_evaluation(
                    input,
                    stores, expansion,
                    box_register_has_kind(stores, index, BoxKind::VBox) ^ $invert,
                    frame_token,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfEof) => {
                let frame_token = begin_if_evaluation(input, call_context, ConditionMetadata::new(14, $invert));
                let stream = scan_stream_number_with_mode_and_context(
                    input,
                    stores, expansion,
                    mode,
                    call_context,
                )?;
                crate::record_dependency!(expansion, crate::ReadDependency::InputStream(stream));
                complete_if_evaluation(
                    input,
                    stores, expansion,
                    (stream >= tex_state::world::STREAM_SLOT_COUNT as u8 || stores.input_stream_eof(tex_state::StreamSlot::new(stream))) ^ $invert,
                    frame_token,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfDefined) => {
                let frame_token = begin_if_evaluation(input, call_context, ConditionMetadata::new(18, $invert));
                let Some(target) = crate::next_semantic_raw_token(input, stores)? else {
                    return Err(ExpandError::MissingTokenAfterPrimitive {
                        opcode: ExpandableOpcode::IfDefined,
                        context: call_context,
                    });
                };
                let defined = match crate::semantic_token(target) {
                    Token::Cs(symbol) => {
                        let meaning = stores.meaning(symbol);
                        expansion.record_meaning(symbol, meaning);
                        meaning != Meaning::Undefined
                    }
                    Token::Char {
                        ch,
                        cat: tex_state::token::Catcode::Active,
                    } => stores.active_character_symbol(ch).is_some_and(|symbol| {
                        let meaning = stores.meaning(symbol);
                        expansion.record_meaning(symbol, meaning);
                        meaning != Meaning::Undefined
                    }),
                    Token::Char { .. } | Token::Param(_) | Token::Frozen(_) => true,
                };
                complete_if_evaluation(
                    input,
                    stores, expansion,
                    defined ^ $invert,
                    frame_token,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfCsName) => {
                let frame_token = begin_if_evaluation(input, call_context, ConditionMetadata::new(19, $invert));
                let name = scan_csname(input, stores, expansion, call_context)?;
                let defined = stores.symbol(&name).is_some_and(|symbol| {
                    let meaning = stores.meaning(symbol);
                    expansion.record_meaning(symbol, meaning);
                    meaning != Meaning::Undefined
                });
                complete_if_evaluation(
                    input,
                    stores, expansion,
                    defined ^ $invert,
                    frame_token,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfFontChar) => {
                let frame_token = begin_if_evaluation(
                    input,
                    call_context,
                    ConditionMetadata::new(20, $invert),
                );
                let font = scan_font_selector(
                    input,
                    stores, expansion,
                    mode,
                    call_context,
                )?;
                let code = scan_int::scan_int_with_mode_and_context(
                    input,
                    stores, expansion,
                    mode,
                    call_context,
                )?
                .value();
                crate::record_dependency!(expansion, crate::ReadDependency::Font {
                    field: crate::ReadFontField::Metrics,
                    font: font.raw(),
                    index: u32::try_from(code).unwrap_or(u32::MAX),
                });
                let exists = u8::try_from(code)
                    .ok()
                    .is_some_and(|code| stores.font_char_metrics(font, code).is_some());
                complete_if_evaluation(
                    input,
                    stores, expansion,
                    exists ^ $invert,
                    frame_token,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Else) => {
                handle_else(token, call_origin, input, stores, expansion)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Or) => {
                handle_or(token, call_origin, input, stores, expansion)
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

type InputOpenOperation = fn(
    &mut InputStack,
    &mut tex_state::ExpansionContext<'_>,
    &mut ExpansionContext<'_>,
    TracedTokenWord,
) -> Result<Dispatch, ExpandError>;

fn execute_input_primitive(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    context: TracedTokenWord,
) -> Result<Dispatch, ExpandError>
where
{
    let name = scan_input_name(input, stores, expansion, context)?;
    let transfer_endinput = input.take_current_source_end_after_current_line();
    let source = expansion
        .open_input(&mut stores.input_open_context(), &name)
        .map_err(|message| ExpandError::InputOpen {
            name: name.clone(),
            message,
            context,
        })?;
    input.push_boxed_source(source);
    if transfer_endinput {
        input.end_current_source_after_current_line();
    }
    Ok(Dispatch::Continue)
}

fn execute_filesize_primitive(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    context: TracedTokenWord,
) -> Result<Dispatch, ExpandError> {
    let expanded = crate::scan::scan_general_text_expanded_with_expanded_open(
        input,
        stores,
        expansion,
        &mut DriverExpansionMode,
        context,
    )
    .map_err(|error| match error {
        crate::scan::ScanToksError::Lex(error) => ExpandError::Lex(error),
        crate::scan::ScanToksError::Expand(error) => error,
        _ => ExpandError::MissingTokenAfterPrimitive {
            opcode: ExpandableOpcode::FileSize,
            context,
        },
    })?;
    let mut name = String::new();
    for &token in stores.tokens(expanded.token_list()) {
        crate::append_token_string_text(stores, token, &mut name);
    }
    match expansion.input_file_size(&mut stores.input_open_context(), &name) {
        Ok(Some(size)) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::NumberOutput,
            &size.to_string(),
            context.origin(),
        )),
        Ok(None) => Ok(Dispatch::Continue),
        Err(message) => Err(ExpandError::InputOpen {
            name,
            message,
            context,
        }),
    }
}

fn execute_string_compare_primitive(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
) -> Result<Dispatch, ExpandError> {
    let mut strings = [String::new(), String::new()];
    for string in &mut strings {
        let text = crate::scan::scan_general_text_expanded_with_expanded_open(
            input, stores, expansion, mode, context,
        )?;
        for &token in stores.tokens(text.token_list()) {
            crate::append_token_string_text(stores, token, string);
        }
    }
    let result = match strings[0].as_bytes().cmp(strings[1].as_bytes()) {
        std::cmp::Ordering::Less => "-1",
        std::cmp::Ordering::Equal => "0",
        std::cmp::Ordering::Greater => "1",
    };
    Ok(push_rendered_text(
        stores,
        ExpansionReplayKind::NumberOutput,
        result,
        context.origin(),
    ))
}

#[allow(clippy::too_many_arguments)]
fn dispatch_core(
    token: Token,
    call_origin: OriginId,
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    meaning: Meaning,
    mode: &mut dyn ExpansionMode,
    invert: bool,
    input_open: Option<InputOpenOperation>,
) -> Result<Dispatch, ExpandError>
where
{
    dispatch_match!(
        token,
        call_origin,
        input,
        stores,
        expansion,
        meaning,
        invert,
        mode,
        {
            let context = TracedTokenWord::pack(token, call_origin);
            if let Some(open_input) = input_open {
                open_input(input, stores, expansion, context)
            } else {
                Err(ExpandError::InputOpen {
                    name: "\\input".to_owned(),
                    message: "\\input requires input-open authority".to_owned(),
                    context,
                })
            }
        },
        {
            let context = TracedTokenWord::pack(token, call_origin);
            execute_filesize_primitive(input, stores, expansion, context)
        }
    )
}

pub fn dispatch_with_context(
    token: Token,
    call_origin: OriginId,
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    meaning: Meaning,
) -> Result<Dispatch, ExpandError> {
    dispatch_core(
        token,
        call_origin,
        input,
        stores,
        expansion,
        meaning,
        &mut DriverExpansionMode,
        false,
        Some(execute_input_primitive),
    )
}

pub(crate) fn dispatch_with_context_inverted(
    token: Token,
    call_origin: OriginId,
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    meaning: Meaning,
) -> Result<Dispatch, ExpandError> {
    dispatch_core(
        token,
        call_origin,
        input,
        stores,
        expansion,
        meaning,
        &mut DriverExpansionMode,
        true,
        None,
    )
}

pub(crate) fn dispatch_without_input_open(
    token: Token,
    call_origin: OriginId,
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    meaning: Meaning,
) -> Result<Dispatch, ExpandError> {
    dispatch_core(
        token,
        call_origin,
        input,
        stores,
        expansion,
        meaning,
        &mut RestrictedExpansionMode,
        false,
        None,
    )
}

pub(crate) fn dispatch_without_input_open_inverted(
    token: Token,
    call_origin: OriginId,
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    meaning: Meaning,
) -> Result<Dispatch, ExpandError> {
    dispatch_core(
        token,
        call_origin,
        input,
        stores,
        expansion,
        meaning,
        &mut RestrictedExpansionMode,
        true,
        None,
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
            | ExpandablePrimitive::IfDefined
            | ExpandablePrimitive::IfCsName
            | ExpandablePrimitive::IfFontChar
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
        | ExpandableOpcode::Expanded
        | ExpandableOpcode::FileSize
        | ExpandableOpcode::StringCompare
        | ExpandableOpcode::Unexpanded
        | ExpandableOpcode::Detokenize
        | ExpandableOpcode::Unless
        | ExpandableOpcode::Scantokens
        | ExpandableOpcode::ETeXVersion
        | ExpandableOpcode::ETeXRevision
        | ExpandableOpcode::IfDefined
        | ExpandableOpcode::IfCsName
        | ExpandableOpcode::IfFontChar
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
