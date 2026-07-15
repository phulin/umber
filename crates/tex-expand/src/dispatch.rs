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
                let parameter_pattern =
                    stores.macro_definition_parameter_pattern(definition);
                let provenance = stores.macro_definition_provenance(definition);
                let arguments = args::match_macro_call_with_context(
                    input,
                    stores,
                    expansion,
                    call_context,
                    macro_meaning,
                    &parameter_pattern,
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
                crate::pdf_strings::execute_compare(
                    input, stores, expansion, mode, call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfEscapeString) => {
                crate::pdf_strings::execute_conversion(
                    crate::pdf_strings::PdfStringConversion::EscapeString,
                    input, stores, expansion, mode, call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfEscapeName) => {
                crate::pdf_strings::execute_conversion(
                    crate::pdf_strings::PdfStringConversion::EscapeName,
                    input, stores, expansion, mode, call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfEscapeHex) => {
                crate::pdf_strings::execute_conversion(
                    crate::pdf_strings::PdfStringConversion::EscapeHex,
                    input, stores, expansion, mode, call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfUnescapeHex) => {
                crate::pdf_strings::execute_conversion(
                    crate::pdf_strings::PdfStringConversion::UnescapeHex,
                    input, stores, expansion, mode, call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::ShellEscape) => {
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::NumberOutput,
                    "0",
                    call_origin,
                ))
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::CreationDate) => {
                Ok(crate::pdf_files::creation_date(
                    stores,
                    expansion,
                    call_context,
                ))
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfFileModificationDate) => {
                crate::pdf_files::execute(
                    crate::pdf_files::PdfFileEnquiry::ModificationDate,
                    input, stores, expansion, mode, call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfMdFiveSum) => {
                crate::pdf_files::execute(
                    crate::pdf_files::PdfFileEnquiry::MdFiveSum,
                    input, stores, expansion, mode, call_context,
                )
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfFileDump) => {
                crate::pdf_files::execute(
                    crate::pdf_files::PdfFileEnquiry::Dump,
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
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfTeXRevision) => {
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::NumberOutput,
                    ".27",
                    call_origin,
                ))
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfTeXBanner) => {
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::NumberOutput,
                    "This is pdfTeX, Version 3.141592653-2.6-1.40.27 (TeX Live 2025)",
                    call_origin,
                ))
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfFontSize) => {
                let font = scan_font_selector(input, stores, expansion, mode, call_context)?;
                crate::record_dependency!(expansion, crate::ReadDependency::Font {
                    field: crate::ReadFontField::Metrics,
                    font: font.raw(),
                    index: 0,
                });
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::NumberOutput,
                    &crate::values::format_scaled(stores.font_size(font)),
                    call_origin,
                ))
            }
            Meaning::ExpandablePrimitive(
                primitive @ (ExpandablePrimitive::PdfFontName
                | ExpandablePrimitive::PdfFontObjectNumber),
            ) => {
                let font = scan_font_selector(input, stores, expansion, mode, call_context)?;
                if font == tex_state::font::NULL_FONT {
                    return Err(ExpandError::PdfInvalidFontIdentifier { context: call_context });
                }
                crate::record_dependency!(expansion, crate::ReadDependency::Font {
                    field: crate::ReadFontField::Metrics,
                    font: font.raw(),
                    index: 0,
                });
                let record = stores
                    .ensure_pdf_font_resource(font)
                    .map_err(|_| ExpandError::PdfObjectCapacity { context: call_context })?;
                let number = match primitive {
                    ExpandablePrimitive::PdfFontName => record.resource_number(),
                    ExpandablePrimitive::PdfFontObjectNumber => record.object_number(),
                    _ => unreachable!(),
                };
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::NumberOutput,
                    &number.to_string(),
                    call_origin,
                ))
            }
            Meaning::ExpandablePrimitive(
                primitive @ (ExpandablePrimitive::LeftMarginKern
                | ExpandablePrimitive::RightMarginKern),
            ) => {
                let index = scan_register_index_with_mode_and_context(
                    input,
                    stores,
                    expansion,
                    mode,
                    call_context,
                )?;
                crate::record_dependency!(expansion, crate::ReadDependency::Cell {
                    bank: crate::ReadBank::Box,
                    index: u32::from(index),
                });
                let amount = margin_kern_enquiry(
                    stores,
                    index,
                    primitive == ExpandablePrimitive::LeftMarginKern,
                    call_context,
                )?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::NumberOutput,
                    &crate::values::format_scaled(amount),
                    call_origin,
                ))
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfPrimitive) => {
                let Some(target) = crate::next_semantic_raw_token(input, stores)? else {
                    return Err(ExpandError::MissingTokenAfterPrimitive {
                        opcode: ExpandableOpcode::PdfPrimitive,
                        context: call_context,
                    });
                };
                let Token::Cs(symbol) = crate::semantic_token(target) else {
                    return Ok(Dispatch::Continue);
                };
                let name = stores.resolve(symbol).to_owned();
                let Some(original) = stores.primitive_meaning(&name) else {
                    return Ok(Dispatch::Continue);
                };
                if matches!(original, Meaning::ExpandablePrimitive(_)) {
                    mode.dispatch_known_meaning(target, original, input, stores, expansion)
                } else {
                    let frozen = stores
                        .primitive_token(&name)
                        .expect("a registered primitive has a frozen token");
                    Ok(Dispatch::Deliver(TracedTokenWord::pack(
                        frozen,
                        target.origin(),
                    )))
                }
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
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfPdfAbsNum) => {
                let frame_token = begin_if_evaluation(input, call_context, ConditionMetadata::new(22, $invert));
                let left = scan_int::scan_int_with_mode_and_context(
                    input, stores, expansion, mode, call_context,
                )?.value();
                let left = absolute_magnitude(left);
                let relation = scan_conditional_relation_with_mode_and_context(
                    input, stores, expansion, mode, call_context,
                )?;
                let right = scan_int::scan_int_with_mode_and_context(
                    input, stores, expansion, mode, call_context,
                )?.value();
                let right = absolute_magnitude(right);
                complete_if_evaluation(
                    input, stores, expansion,
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
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfPdfAbsDim) => {
                let frame_token = begin_if_evaluation(input, call_context, ConditionMetadata::new(23, $invert));
                let left = scan_dimen::scan_dimen_with_mode_and_context(
                    input, stores, expansion, mode,
                    scan_dimen::ScanDimenOptions::STANDARD, call_context,
                )?.value().raw();
                let left = absolute_magnitude(left);
                let relation = scan_conditional_relation_with_mode_and_context(
                    input, stores, expansion, mode, call_context,
                )?;
                let right = scan_dimen::scan_dimen_with_mode_and_context(
                    input, stores, expansion, mode,
                    scan_dimen::ScanDimenOptions::STANDARD, call_context,
                )?.value().raw();
                let right = absolute_magnitude(right);
                complete_if_evaluation(
                    input, stores, expansion,
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
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfPdfPrimitive) => {
                let frame_token = begin_if_evaluation(input, call_context, ConditionMetadata::new(21, $invert));
                let Some(target) = crate::next_semantic_raw_token(input, stores)? else {
                    return Err(ExpandError::MissingTokenAfterPrimitive {
                        opcode: ExpandableOpcode::IfPdfPrimitive,
                        context: call_context,
                    });
                };
                let primitive = match crate::semantic_token(target) {
                    Token::Cs(symbol) => {
                        let current = stores.meaning(symbol);
                        expansion.record_meaning(symbol, current);
                        stores.primitive_meaning(stores.resolve(symbol)) == Some(current)
                            && current != Meaning::Undefined
                    }
                    _ => false,
                };
                complete_if_evaluation(
                    input, stores, expansion, primitive ^ $invert, frame_token,
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
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfInCsName) => {
                begin_if(
                    input,
                    stores,
                    expansion,
                    (expansion.csname_depth > 0) ^ $invert,
                    ConditionMetadata::new(21, $invert),
                    call_context,
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

fn margin_kern_enquiry(
    stores: &impl ExpansionState,
    index: u16,
    left: bool,
    context: TracedTokenWord,
) -> Result<tex_state::scaled::Scaled, ExpandError> {
    let Some(root) = stores.box_reg(index) else {
        return Err(ExpandError::MarginKernExpectedHBox { context });
    };
    let Some(tex_state::node_arena::NodeRef::HList(box_node)) = stores.nodes(root).first() else {
        return Err(ExpandError::MarginKernExpectedHBox { context });
    };
    let children = stores.nodes(box_node.children);
    let expected = if left {
        tex_state::node::KernKind::LeftMargin
    } else {
        tex_state::node::KernKind::RightMargin
    };
    let found = if left {
        children
            .iter()
            .find(|node| !margin_kern_enquiry_skip(node, true))
    } else {
        children
            .iter()
            .rev()
            .find(|node| !margin_kern_enquiry_skip(node, false))
    };
    Ok(match found {
        Some(tex_state::node_arena::NodeRef::Kern { amount, kind }) if kind == expected => amount,
        _ => tex_state::scaled::Scaled::from_raw(0),
    })
}

fn margin_kern_enquiry_skip(node: &tex_state::node_arena::NodeRef<'_>, left: bool) -> bool {
    matches!(
        node,
        tex_state::node_arena::NodeRef::Penalty(_)
            | tex_state::node_arena::NodeRef::Mark { .. }
            | tex_state::node_arena::NodeRef::Ins { .. }
            | tex_state::node_arena::NodeRef::Whatsit(_)
            | tex_state::node_arena::NodeRef::Direction(_)
            | tex_state::node_arena::NodeRef::Adjust(_)
    ) || matches!(
        node,
        tex_state::node_arena::NodeRef::Glue { kind, .. }
            if (left && *kind == tex_state::node::GlueKind::LeftSkip)
                || (!left && *kind == tex_state::node::GlueKind::RightSkip)
    )
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
            | ExpandablePrimitive::IfInCsName
            | ExpandablePrimitive::IfFontChar
            | ExpandablePrimitive::IfPdfPrimitive
            | ExpandablePrimitive::IfPdfAbsNum
            | ExpandablePrimitive::IfPdfAbsDim
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
        | ExpandableOpcode::ShellEscape
        | ExpandableOpcode::CreationDate
        | ExpandableOpcode::Unexpanded
        | ExpandableOpcode::Detokenize
        | ExpandableOpcode::Unless
        | ExpandableOpcode::Scantokens
        | ExpandableOpcode::ETeXVersion
        | ExpandableOpcode::ETeXRevision
        | ExpandableOpcode::PdfTeXRevision
        | ExpandableOpcode::PdfTeXBanner
        | ExpandableOpcode::PdfFontName
        | ExpandableOpcode::PdfFontObjectNumber
        | ExpandableOpcode::PdfPrimitive
        | ExpandableOpcode::IfPdfPrimitive
        | ExpandableOpcode::IfPdfAbsNum
        | ExpandableOpcode::IfPdfAbsDim
        | ExpandableOpcode::PdfEscapeString
        | ExpandableOpcode::PdfEscapeName
        | ExpandableOpcode::PdfEscapeHex
        | ExpandableOpcode::PdfUnescapeHex
        | ExpandableOpcode::PdfFileModificationDate
        | ExpandableOpcode::PdfMdFiveSum
        | ExpandableOpcode::PdfFileDump
        | ExpandableOpcode::IfDefined
        | ExpandableOpcode::IfCsName
        | ExpandableOpcode::IfInCsName
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
