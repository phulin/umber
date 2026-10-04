//! The sole interpreter switch; no eligibility dispatcher precedes it.
use super::*;
use tex_state::meaning::MeaningWord;

pub(super) fn execute<'state, 'host, G>(
    processor: &mut Processor<'state, 'host, G>,
    spelling: TracedTokenWord,
    meaning: &MeaningWord<G>,
    control_sequence: Option<Symbol>,
    exceptional: Option<&tex_command::CurrentCommand<G>>,
    observation_receipts: &mut Option<Vec<MutationRecord>>,
    prefixes: u8,
) -> Result<MainControlStep, ExecError> {
    let code = match meaning {
        MeaningWord::Static(word) => Meaning::runtime_word_control_code(*word),
        MeaningWord::Font(_) => codes::FONT,
        MeaningWord::Macro { .. } => unreachable!("expanded reader delivers a terminal"),
    };
    if processor.has_pending_reports() {
        report_scan(processor)?;
    }
    trace::command(processor, code, meaning, control_sequence);
    match code {
        codes::PRIMITIVE_GLOBAL
        | codes::PRIMITIVE_LONG
        | codes::PRIMITIVE_OUTER
        | codes::PRIMITIVE_PROTECTED => {
            let flag = match code {
                codes::PRIMITIVE_GLOBAL => prefixes::GLOBAL,
                codes::PRIMITIVE_LONG => MeaningFlags::LONG.bits(),
                codes::PRIMITIVE_OUTER => MeaningFlags::OUTER.bits(),
                _ => MeaningFlags::PROTECTED.bits(),
            };
            prefixes::scan(processor, prefixes | flag, observation_receipts)
        }
        codes::PRIMITIVE_IGNORE_SPACES => {
            characters::ignore_spaces(processor, observation_receipts)
        }
        codes::CHAR_PARAMETER => {
            recovery::illegal_case(processor, spelling, meaning, control_sequence, exceptional)
        }
        codes::END_CS_NAME => {
            processor.with_execution_owners(
                |context, command, effects, _, _, execution, _, _, _| {
                    execution.main_loop_active = false;
                    let display = command.output_open_context(context);
                    report_escaped_error(
                        context,
                        effects,
                        "Extra ",
                        "endcsname",
                        "",
                        &["I'm ignoring this, since I wasn't doing a \\csname."],
                        display,
                    )
                },
            )?;
            recovery::finish_error_dialogue(processor)?;
            Ok(MainControlStep::Continue)
        }
        codes::RELAX => {
            processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
                execution.main_loop_active = false;
                let pending = execution.modes.current_list().pending_hchars().is_some();
                semantic::execute_relax(
                    context,
                    pending.then_some(&mut execution.modes),
                    effects,
                    fuel,
                )
            })
        }
        codes::CHAR_LETTER | codes::CHAR_OTHER | codes::CHAR_GIVEN => {
            let MeaningWord::Static(word) = meaning else {
                unreachable!("scalar character meaning")
            };
            let ch = if code == codes::CHAR_GIVEN {
                char::from_u32(Meaning::runtime_word_operand(*word) as u32)
                    .expect("admitted character definition")
            } else {
                Meaning::runtime_word_character(*word)
                    .expect("admitted literal character")
                    .0
            };
            let origin = processor.active_macro_origin().unwrap_or(spelling.origin());
            let mode = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.modes.current_mode()
            });
            match mode {
                Mode::Math | Mode::DisplayMath => {
                    characters::append_native_math_character(processor, ch, origin)?;
                }
                Mode::Horizontal | Mode::RestrictedHorizontal => {
                    let etex_extended = processor.profile() == CommandProfile::ETEX26;
                    processor.with_execution_owners(
                        |context, _, effects, fuel, _, execution, _, _, _| {
                            characters::append_native_character(
                                context,
                                effects,
                                fuel,
                                execution,
                                ch,
                                origin,
                                etex_extended,
                            )
                        },
                    )?;
                }
                Mode::Vertical | Mode::InternalVertical => {
                    return recovery::start_paragraph_and_reswitch(
                        processor,
                        spelling,
                        meaning,
                        control_sequence,
                        exceptional,
                    );
                }
            }
            Ok(MainControlStep::Continue)
        }
        codes::PRIMITIVE_CHAR => {
            let mode = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.modes.current_mode()
            });
            if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                return recovery::start_paragraph_and_reswitch(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            let origin = processor.active_macro_origin().unwrap_or(spelling.origin());
            let ch = processor.scan_character_number().map_err(command_error)?;
            report_scan(processor)?;
            if matches!(mode, Mode::Math | Mode::DisplayMath) {
                characters::append_native_math_character(processor, ch, origin)?;
            } else {
                let extended = processor.profile() == CommandProfile::ETEX26;
                processor.with_execution_owners(
                    |context, _, effects, fuel, _, execution, _, _, _| {
                        characters::append_native_character(
                            context, effects, fuel, execution, ch, origin, extended,
                        )
                    },
                )?;
            }
            Ok(MainControlStep::Continue)
        }
        codes::CHAR_SPACE => {
            processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
                execution.main_loop_active = false;
                if matches!(
                    execution.modes.current_mode(),
                    Mode::Horizontal | Mode::RestrictedHorizontal
                ) {
                    crate::box_runtime::append_space_with_fuel(
                        &mut execution.modes,
                        context,
                        effects,
                        fuel,
                    )?;
                }
                Ok(MainControlStep::Continue)
            })
        }
        codes::PRIMITIVE_FONT
        | codes::PRIMITIVE_PDF_COPY_FONT
        | codes::PRIMITIVE_LETTERSPACE_FONT => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
            if code == codes::PRIMITIVE_FONT {
                fonts::define(processor, global, observation_receipts)
            } else {
                let kind = if code == codes::PRIMITIVE_PDF_COPY_FONT {
                    tex_command::GeneratedFontKind::Copy
                } else {
                    tex_command::GeneratedFontKind::Letterspace
                };
                fonts::generated(processor, kind, global, observation_receipts)
            }
        }
        codes::PRIMITIVE_FONT_DIMEN => fonts::dimension(processor),
        codes::PRIMITIVE_HYPHEN_CHAR | codes::PRIMITIVE_SKEW_CHAR => {
            fonts::integer(processor, code == codes::PRIMITIVE_SKEW_CHAR)
        }
        codes::PRIMITIVE_PAR_SHAPE
        | codes::PRIMITIVE_INTER_LINE_PENALTIES
        | codes::PRIMITIVE_CLUB_PENALTIES
        | codes::PRIMITIVE_WIDOW_PENALTIES
        | codes::PRIMITIVE_DISPLAY_WIDOW_PENALTIES => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
            if code == codes::PRIMITIVE_PAR_SHAPE {
                assignments::shape(processor, global)
            } else {
                let kind = match code {
                    codes::PRIMITIVE_INTER_LINE_PENALTIES => PenaltyArrayKind::InterLine,
                    codes::PRIMITIVE_CLUB_PENALTIES => PenaltyArrayKind::Club,
                    codes::PRIMITIVE_WIDOW_PENALTIES => PenaltyArrayKind::Widow,
                    _ => PenaltyArrayKind::DisplayWidow,
                };
                assignments::penalties(processor, kind, global, observation_receipts)
            }
        }
        codes::PAGE_DIMENSION => {
            let ResolvedMeaning::Static(Meaning::PageDimension(dimension)) = meaning.resolve()
            else {
                unreachable!("page dimension selector")
            };
            processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_dimension().map_err(command_error)?;
            report_scan(processor)?;
            let step = processor.with_execution_owners(
                |context, _, effects, _, _, execution, _, _, _| {
                    execution.main_loop_active = false;
                    semantic::execute_page_dimension(context, effects, dimension, value)
                },
            )?;
            processor.complete_assignment().map_err(command_error)?;
            Ok(step)
        }
        codes::PAGE_INTEGER => {
            let ResolvedMeaning::Static(Meaning::PageInteger(integer)) = meaning.resolve() else {
                unreachable!("page integer selector")
            };
            processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_integer().map_err(command_error)?.value;
            report_scan(processor)?;
            let step = processor.with_execution_owners(
                |context, _, effects, _, _, execution, _, _, _| {
                    execution.main_loop_active = false;
                    semantic::execute_page_integer(context, effects, integer, value)
                },
            )?;
            processor.complete_assignment().map_err(command_error)?;
            Ok(step)
        }
        codes::PRIMITIVE_BATCH_MODE
        | codes::PRIMITIVE_NONSTOP_MODE
        | codes::PRIMITIVE_SCROLL_MODE
        | codes::PRIMITIVE_ERROR_STOP_MODE
        | codes::PRIMITIVE_INTERACTION_MODE => {
            let scanned = code == codes::PRIMITIVE_INTERACTION_MODE;
            let coordinate = if scanned {
                processor.scan_optional_equals().map_err(command_error)?;
                let value = processor.scan_integer().map_err(command_error)?.value;
                (value, Some(processor.diagnostic_context_coordinate()))
            } else {
                let value = match code {
                    codes::PRIMITIVE_BATCH_MODE => 0,
                    codes::PRIMITIVE_NONSTOP_MODE => 1,
                    codes::PRIMITIVE_SCROLL_MODE => 2,
                    _ => 3,
                };
                (value, None)
            };
            report_scan(processor)?;
            let step = processor.with_execution_owners(
                |context, command, effects, _, _, execution, _, _, _| {
                    execution.main_loop_active = false;
                    semantic::execute_set_interaction_mode(
                        context,
                        command,
                        effects,
                        coordinate.0,
                        coordinate.1,
                    )
                },
            )?;
            recovery::finish_error_dialogue(processor)?;
            processor.complete_assignment().map_err(command_error)?;
            Ok(step)
        }
        codes::PRIMITIVE_PATTERNS | codes::PRIMITIVE_HYPHENATION => {
            fonts::hyphenation(processor, code == codes::PRIMITIVE_PATTERNS)
        }
        codes::PRIMITIVE_BEGIN_L
        | codes::PRIMITIVE_END_L
        | codes::PRIMITIVE_BEGIN_R
        | codes::PRIMITIVE_END_R => {
            let mode = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.modes.current_mode()
            });
            if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                return recovery::start_paragraph_and_reswitch(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            if matches!(mode, Mode::Math | Mode::DisplayMath) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            let direction = match code {
                codes::PRIMITIVE_BEGIN_L => tex_state::node::Direction::BeginL,
                codes::PRIMITIVE_END_L => tex_state::node::Direction::EndL,
                codes::PRIMITIVE_BEGIN_R => tex_state::node::Direction::BeginR,
                _ => tex_state::node::Direction::EndR,
            };
            let enabled = processor.int_param(IntParam::TEX_XET_STATE) > 0;
            report_scan(processor)?;
            let step = processor.with_execution_owners(
                |context, command, effects, fuel, _, execution, _, _, _| {
                    execution.main_loop_active = false;
                    semantic::execute_text_direction(
                        context,
                        &mut execution.modes,
                        command,
                        effects,
                        fuel,
                        direction,
                        enabled,
                    )
                },
            )?;
            recovery::finish_error_dialogue(processor)?;
            Ok(step)
        }
        codes::PRIMITIVE_MARK | codes::PRIMITIVE_MARKS => {
            let class = if code == codes::PRIMITIVE_MARKS {
                processor
                    .scan_extended_register_index()
                    .map_err(command_error)?
            } else {
                0
            };
            let scanned = processor.scan_balanced_text(true).map_err(command_error)?;
            report_scan(processor)?;
            processor.with_execution_owners(
                |context, command, effects, fuel, _, execution, _, _, _| {
                    execution.main_loop_active = false;
                    let tokens = publication::publish_tokens(command, context, scanned.tokens)?;
                    semantic::execute_mark(
                        context,
                        &mut execution.modes,
                        effects,
                        fuel,
                        &mut execution.boxes,
                        class,
                        tokens,
                    )
                },
            )
        }
        codes::PRIMITIVE_PREV_DEPTH => {
            let vertical = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                matches!(
                    execution.modes.current_mode(),
                    Mode::Vertical | Mode::InternalVertical
                )
            });
            if !vertical {
                return recovery::illegal_case(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_dimension().map_err(command_error)?;
            report_scan(processor)?;
            let step = processor.with_execution_owners(
                |context, _, effects, _, _, execution, _, _, _| {
                    execution.main_loop_active = false;
                    semantic::execute_prev_depth(context, &mut execution.modes, effects, value)
                },
            )?;
            processor.complete_assignment().map_err(command_error)?;
            Ok(step)
        }
        codes::PRIMITIVE_SPACE_FACTOR | codes::PRIMITIVE_PREV_GRAF => {
            if code == codes::PRIMITIVE_SPACE_FACTOR {
                let horizontal =
                    processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                        matches!(
                            execution.modes.current_mode(),
                            Mode::Horizontal | Mode::RestrictedHorizontal
                        )
                    });
                if !horizontal {
                    return recovery::illegal_case(
                        processor,
                        spelling,
                        meaning,
                        control_sequence,
                        exceptional,
                    );
                }
            }
            processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_integer().map_err(command_error)?.value;
            report_scan(processor)?;
            let step = processor.with_execution_owners(
                |context, command, effects, _, _, execution, _, _, _| {
                    execution.main_loop_active = false;
                    if code == codes::PRIMITIVE_SPACE_FACTOR {
                        semantic::execute_space_factor(
                            context,
                            &mut execution.modes,
                            command,
                            effects,
                            value,
                        )
                    } else {
                        semantic::execute_prev_graf(
                            context,
                            &mut execution.modes,
                            command,
                            effects,
                            value,
                        )
                    }
                },
            )?;
            recovery::finish_error_dialogue(processor)?;
            processor.complete_assignment().map_err(command_error)?;
            Ok(step)
        }
        codes::PRIMITIVE_QUIT_V_MODE => {
            report_scan(processor)?;
            processor.with_execution_owners(
                |context, command, effects, _, observer, execution, _, _, _| {
                    execution.main_loop_active = false;
                    if matches!(
                        execution.modes.current_mode(),
                        Mode::Vertical | Mode::InternalVertical
                    ) {
                        start_paragraph(command, &mut execution.modes, context, effects, true)?;
                        command.publish_named_token_list_pushes(context, effects, observer);
                    }
                    Ok(MainControlStep::Continue)
                },
            )
        }
        codes::PRIMITIVE_PDF_LP_CODE
        | codes::PRIMITIVE_PDF_RP_CODE
        | codes::PRIMITIVE_PDF_EF_CODE
        | codes::PRIMITIVE_PDF_TAG_CODE
        | codes::PRIMITIVE_PDF_KNBS_CODE
        | codes::PRIMITIVE_PDF_STBS_CODE
        | codes::PRIMITIVE_PDF_SHBS_CODE
        | codes::PRIMITIVE_PDF_KNBC_CODE
        | codes::PRIMITIVE_PDF_KNAC_CODE => {
            let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(primitive)) =
                meaning.resolve()
            else {
                unreachable!("PDF font code selector")
            };
            pdf::font_code(
                processor,
                pdf_font_code_table(primitive),
                observation_receipts,
            )
        }
        codes::PRIMITIVE_PDF_NO_LIGATURES => pdf::no_ligatures(processor, observation_receipts),
        codes::PRIMITIVE_PDF_FONT_EXPAND => pdf::font_expand(processor),
        codes::PRIMITIVE_PDF_FONT_ATTR
        | codes::PRIMITIVE_PDF_INCLUDE_CHARS
        | codes::PRIMITIVE_PDF_NO_BUILTIN_TO_UNICODE
        | codes::PRIMITIVE_PDF_GLYPH_TO_UNICODE
        | codes::PRIMITIVE_PDF_MAP_FILE
        | codes::PRIMITIVE_PDF_MAP_LINE => {
            let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(primitive)) =
                meaning.resolve()
            else {
                unreachable!("PDF font action selector")
            };
            pdf::font_action(processor, primitive)
        }
        codes::PRIMITIVE_PDF_LITERAL
        | codes::PRIMITIVE_PDF_SET_MATRIX
        | codes::PRIMITIVE_PDF_SAVE
        | codes::PRIMITIVE_PDF_RESTORE
        | codes::PRIMITIVE_PDF_COLOR_STACK
        | codes::PRIMITIVE_PDF_SAVE_POS
        | codes::PRIMITIVE_PDF_SNAP_REF_POINT
        | codes::PRIMITIVE_PDF_SNAP_Y
        | codes::PRIMITIVE_PDF_SNAP_Y_COMP => {
            let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(primitive)) =
                meaning.resolve()
            else {
                unreachable!("PDF graphics selector")
            };
            pdf::graphics(processor, primitive, spelling, exceptional)
        }
        codes::PRIMITIVE_PDF_ANNOT
        | codes::PRIMITIVE_PDF_START_LINK
        | codes::PRIMITIVE_PDF_END_LINK
        | codes::PRIMITIVE_PDF_OUTLINE
        | codes::PRIMITIVE_PDF_DEST
        | codes::PRIMITIVE_PDF_THREAD
        | codes::PRIMITIVE_PDF_START_THREAD
        | codes::PRIMITIVE_PDF_END_THREAD => {
            let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(primitive)) =
                meaning.resolve()
            else {
                unreachable!("PDF navigation selector")
            };
            let name = match primitive {
                UnexpandablePrimitive::PdfAnnot => "pdfannot",
                UnexpandablePrimitive::PdfStartLink => "pdfstartlink",
                UnexpandablePrimitive::PdfEndLink => "pdfendlink",
                UnexpandablePrimitive::PdfOutline => "pdfoutline",
                UnexpandablePrimitive::PdfDest => "pdfdest",
                UnexpandablePrimitive::PdfThread => "pdfthread",
                UnexpandablePrimitive::PdfStartThread => "pdfstartthread",
                UnexpandablePrimitive::PdfEndThread => "pdfendthread",
                _ => unreachable!("navigation selector"),
            };
            pdf::require_output(processor, spelling, exceptional, name)?;
            if matches!(
                primitive,
                UnexpandablePrimitive::PdfStartLink | UnexpandablePrimitive::PdfEndLink
            ) {
                let vertical =
                    processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                        matches!(
                            execution.modes.current_mode(),
                            Mode::Vertical | Mode::InternalVertical
                        )
                    });
                if vertical {
                    processor
                        .back_consumed_word(spelling, exceptional)
                        .map_err(command_error)?;
                    return Err(ExecError::PdfLinkInVerticalMode(name));
                }
            }
            pdf::navigation(processor, primitive)
        }
        codes::PRIMITIVE_PDF_X_FORM | codes::PRIMITIVE_PDF_REF_X_FORM => {
            let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(primitive)) =
                meaning.resolve()
            else {
                unreachable!("PDF form selector")
            };
            pdf::require_output(
                processor,
                spelling,
                exceptional,
                if primitive == UnexpandablePrimitive::PdfXForm {
                    "pdfxform"
                } else {
                    "pdfrefxform"
                },
            )?;
            pdf::form(processor, primitive)
        }
        codes::PRIMITIVE_PDF_OBJECT => {
            pdf::require_output(processor, spelling, exceptional, "pdfobj")?;
            pdf::object(processor)
        }
        codes::PRIMITIVE_PDF_INFO
        | codes::PRIMITIVE_PDF_CATALOG
        | codes::PRIMITIVE_PDF_NAMES
        | codes::PRIMITIVE_PDF_TRAILER
        | codes::PRIMITIVE_PDF_TRAILER_ID => {
            let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(primitive)) =
                meaning.resolve()
            else {
                unreachable!("PDF document fragment selector")
            };
            if primitive == UnexpandablePrimitive::PdfNames {
                pdf::require_output(processor, spelling, exceptional, "pdfnames")?;
            }
            pdf::document_fragment(processor, primitive)
        }
        codes::PRIMITIVE_PDF_REF_X_IMAGE | codes::PRIMITIVE_PDF_REFERENCE_OBJECT => {
            let name = if code == codes::PRIMITIVE_PDF_REF_X_IMAGE {
                "pdfrefximage"
            } else {
                "pdfrefobj"
            };
            pdf::require_output(processor, spelling, exceptional, name)?;
            let object = processor.scan_integer().map_err(command_error)?.value;
            report_scan(processor)?;
            processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
                execution.main_loop_active = false;
                if code == codes::PRIMITIVE_PDF_REF_X_IMAGE {
                    semantic::execute_pdf_ref_x_image(
                        context,
                        &mut execution.modes,
                        effects,
                        fuel,
                        object,
                    )
                } else {
                    semantic::execute_pdf_reference_object(
                        context,
                        &mut execution.modes,
                        effects,
                        fuel,
                        object,
                    )
                }
            })
        }
        codes::PRIMITIVE_PDF_INTERWORD_SPACE_ON
        | codes::PRIMITIVE_PDF_INTERWORD_SPACE_OFF
        | codes::PRIMITIVE_PDF_FAKE_SPACE => {
            let control = match code {
                codes::PRIMITIVE_PDF_INTERWORD_SPACE_ON => {
                    tex_state::node::PdfAccessibilityControl::InterwordSpaceOn
                }
                codes::PRIMITIVE_PDF_INTERWORD_SPACE_OFF => {
                    tex_state::node::PdfAccessibilityControl::InterwordSpaceOff
                }
                _ => tex_state::node::PdfAccessibilityControl::FakeSpace,
            };
            processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
                execution.main_loop_active = false;
                semantic::execute_pdf_interword_space(
                    context,
                    &mut execution.modes,
                    effects,
                    fuel,
                    control,
                )
            })
        }
        codes::PRIMITIVE_PDF_RUNNING_LINK_ON | codes::PRIMITIVE_PDF_RUNNING_LINK_OFF => processor
            .with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
                execution.main_loop_active = false;
                semantic::execute_pdf_running_link(
                    context,
                    &mut execution.modes,
                    effects,
                    fuel,
                    code == codes::PRIMITIVE_PDF_RUNNING_LINK_ON,
                )
            }),
        codes::PRIMITIVE_PDF_SET_RANDOM_SEED => {
            let seed = processor
                .scan_integer()
                .map_err(command_error)?
                .value
                .saturating_abs();
            report_scan(processor)?;
            processor.with_execution_owners(|context, _, _, _, _, execution, _, _, _| {
                execution.main_loop_active = false;
                context.set_pdf_random_seed(seed);
            });
            Ok(MainControlStep::Continue)
        }
        codes::PRIMITIVE_PDF_RESET_TIMER => {
            processor.with_execution_owners(|context, _, _, _, _, execution, _, _, _| {
                execution.main_loop_active = false;
                context.reset_pdf_timer();
            });
            Ok(MainControlStep::Continue)
        }
        codes::PRIMITIVE_PDF_SPACE_FONT => {
            pdf::require_output(processor, spelling, exceptional, "pdfspacefont")?;
            pdf::space_font(processor)
        }
        codes::PRIMITIVE_SHOW => diagnostics::show(processor, false),
        codes::PRIMITIVE_SHOW_THE => diagnostics::show(processor, true),
        codes::PRIMITIVE_SHOW_TOKENS => diagnostics::tokens(processor),
        codes::PRIMITIVE_SHOW_LISTS => diagnostics::lists(processor),
        codes::PRIMITIVE_SHOW_IFS => diagnostics::conditions(processor),
        codes::PRIMITIVE_SHOW_GROUPS => diagnostics::groups(processor),
        codes::PRIMITIVE_SHOW_BOX => diagnostics::box_register(processor),
        codes::PRIMITIVE_INSERT | codes::PRIMITIVE_V_ADJUST => {
            let adjust = code == codes::PRIMITIVE_V_ADJUST;
            if adjust {
                let vertical =
                    processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                        matches!(
                            execution.modes.current_mode(),
                            Mode::Vertical | Mode::InternalVertical
                        )
                    });
                if vertical {
                    return recovery::illegal_case(
                        processor,
                        spelling,
                        meaning,
                        control_sequence,
                        exceptional,
                    );
                }
            }
            let construction = processor
                .scan_insert_construction(adjust)
                .map_err(command_error)?;
            report_scan(processor)?;
            let step = processor.with_execution_owners(
                |context, command, effects, fuel, _, execution, _, _, _| {
                    execution.main_loop_active = false;
                    semantic::execute_begin_insert(
                        context,
                        &mut execution.modes,
                        command,
                        effects,
                        fuel,
                        &mut execution.boxes,
                        construction,
                    )
                },
            )?;
            recovery::finish_error_dialogue(processor)?;
            Ok(step)
        }
        codes::PRIMITIVE_WD | codes::PRIMITIVE_HT | codes::PRIMITIVE_DP => {
            let index = processor
                .scan_profile_register_index()
                .map_err(command_error)?;
            processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_dimension().map_err(command_error)?;
            let dimension = match code {
                codes::PRIMITIVE_WD => tex_state::BoxDimension::Width,
                codes::PRIMITIVE_HT => tex_state::BoxDimension::Height,
                _ => tex_state::BoxDimension::Depth,
            };
            report_scan(processor)?;
            let step = processor.with_execution_owners(
                |context, _, effects, _, _, execution, _, _, _| {
                    execution.main_loop_active = false;
                    semantic::execute_box_dimension_assignment(
                        context, effects, index, dimension, value,
                    )
                },
            )?;
            processor.complete_assignment().map_err(command_error)?;
            Ok(step)
        }
        codes::PRIMITIVE_UN_PENALTY | codes::PRIMITIVE_UN_KERN | codes::PRIMITIVE_UN_SKIP => {
            let primitive = match code {
                codes::PRIMITIVE_UN_PENALTY => UnexpandablePrimitive::UnPenalty,
                codes::PRIMITIVE_UN_KERN => UnexpandablePrimitive::UnKern,
                _ => UnexpandablePrimitive::UnSkip,
            };
            let coordinate = processor.diagnostic_context_coordinate();
            report_scan(processor)?;
            let step = processor.with_execution_owners(
                |context, command, effects, fuel, _, execution, _, _, _| {
                    execution.main_loop_active = false;
                    semantic::execute_delete_last(
                        context,
                        &mut execution.modes,
                        command,
                        effects,
                        fuel,
                        primitive,
                        coordinate,
                    )
                },
            )?;
            recovery::finish_error_dialogue(processor)?;
            Ok(step)
        }
        codes::PRIMITIVE_BOX | codes::PRIMITIVE_COPY => boxes::register(
            processor,
            code == codes::PRIMITIVE_COPY,
            false,
            observation_receipts,
        ),
        codes::PRIMITIVE_UN_H_BOX
        | codes::PRIMITIVE_UN_H_COPY
        | codes::PRIMITIVE_UN_V_BOX
        | codes::PRIMITIVE_UN_V_COPY
        | codes::PRIMITIVE_PAGE_DISCARDS
        | codes::PRIMITIVE_SPLIT_DISCARDS => {
            let horizontal = matches!(code, codes::PRIMITIVE_UN_H_BOX | codes::PRIMITIVE_UN_H_COPY);
            let discard = matches!(
                code,
                codes::PRIMITIVE_PAGE_DISCARDS | codes::PRIMITIVE_SPLIT_DISCARDS
            );
            let mode = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.modes.current_mode()
            });
            if horizontal && matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                return recovery::start_paragraph_and_reswitch(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            if !horizontal {
                if mode == Mode::Horizontal {
                    processor
                        .recover_stop_for_vertical_mode(spelling, exceptional)
                        .map_err(command_error)?;
                    return Ok(MainControlStep::Continue);
                }
                if mode == Mode::RestrictedHorizontal {
                    return recovery::off_save(
                        processor,
                        spelling,
                        meaning,
                        control_sequence,
                        exceptional,
                    );
                }
                if discard && matches!(mode, Mode::Math | Mode::DisplayMath) {
                    return recovery::insert_math_shift(
                        processor,
                        spelling,
                        meaning,
                        control_sequence,
                        exceptional,
                    );
                }
            }
            let primitive = match code {
                codes::PRIMITIVE_UN_H_BOX => UnexpandablePrimitive::UnHBox,
                codes::PRIMITIVE_UN_H_COPY => UnexpandablePrimitive::UnHCopy,
                codes::PRIMITIVE_UN_V_BOX => UnexpandablePrimitive::UnVBox,
                codes::PRIMITIVE_UN_V_COPY => UnexpandablePrimitive::UnVCopy,
                codes::PRIMITIVE_PAGE_DISCARDS => UnexpandablePrimitive::PageDiscards,
                _ => UnexpandablePrimitive::SplitDiscards,
            };
            if discard {
                boxes::discards(processor, primitive)
            } else {
                boxes::unbox(processor, primitive)
            }
        }
        codes::PRIMITIVE_V_SPLIT => boxes::split(processor, observation_receipts),
        codes::PRIMITIVE_LAST_BOX => boxes::last(processor, observation_receipts),
        codes::PRIMITIVE_OPEN_IN => streams::open_in(processor),
        codes::PRIMITIVE_CLOSE_IN => streams::close_in(processor),
        codes::PRIMITIVE_READ | codes::PRIMITIVE_READ_LINE => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
            streams::read(
                processor,
                code == codes::PRIMITIVE_READ_LINE,
                global,
                observation_receipts,
            )
        }
        codes::FONT => {
            let MeaningWord::Font(font) = meaning else {
                unreachable!("font capability")
            };
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
            report_scan(processor)?;
            let step = processor.with_execution_owners(
                |context, _, effects, fuel, _, execution, _, _, _| {
                    execution.main_loop_active = false;
                    crate::box_runtime::flush_pending_hchars(
                        &mut execution.modes,
                        context,
                        effects,
                        fuel,
                    )?;
                    semantic::execute_font_select(context, effects, *font, global)
                },
            )?;
            processor.complete_assignment().map_err(command_error)?;
            Ok(step)
        }
        codes::CHAR_BEGIN_GROUP | codes::PRIMITIVE_BEGIN_GROUP => {
            if code == codes::CHAR_BEGIN_GROUP {
                let (output_opening, recovery_opening) =
                    processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                        (
                            execution.boxes.output_routine_opening_pending,
                            execution.boxes.recovery_simple_group_pending,
                        )
                    });
                if output_opening {
                    return processor.with_execution_owners(
                        |context, _, effects, _, _, execution, _, _, _| {
                            execution.main_loop_active = false;
                            crate::paragraph_end::normal_paragraph(
                                &mut execution.modes,
                                context,
                                effects,
                            );
                            execution.boxes.output_routine_opening_pending = false;
                            Ok(MainControlStep::Continue)
                        },
                    );
                }
                if recovery_opening {
                    return processor.with_execution_owners(
                        |context, command, effects, fuel, _, execution, _, _, _| {
                            execution.main_loop_active = false;
                            let step = groups::begin_native_group(
                                context,
                                command,
                                effects,
                                fuel,
                                &mut execution.modes,
                                GroupKind::Simple,
                            )?;
                            execution.boxes.recovery_simple_group_pending = false;
                            execution.boxes.recovery_simple_group_open = true;
                            Ok(step)
                        },
                    );
                }
            }
            if code == codes::CHAR_BEGIN_GROUP && math_mode(processor) {
                processor
                    .back_consumed_word(spelling, exceptional)
                    .map_err(command_error)?;
                return math::nucleus(processor, NoadKind::Normal(NoadClass::Ord));
            }
            let kind = if code == codes::CHAR_BEGIN_GROUP {
                GroupKind::Simple
            } else {
                GroupKind::SemiSimple
            };
            report_scan(processor)?;
            processor.with_execution_owners(
                |context, command, effects, fuel, _, execution, _, _, _| {
                    execution.main_loop_active = false;
                    groups::begin_native_group(
                        context,
                        command,
                        effects,
                        fuel,
                        &mut execution.modes,
                        kind,
                    )
                },
            )
        }
        codes::CHAR_END_GROUP | codes::PRIMITIVE_END_GROUP => groups::close_tex_group(
            processor,
            code,
            spelling,
            meaning,
            control_sequence,
            exceptional,
            observation_receipts,
        ),
        codes::PRIMITIVE_END | codes::PRIMITIVE_DUMP => {
            let mode = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.modes.current_mode()
            });
            match mode {
                Mode::Horizontal => {
                    processor
                        .recover_stop_for_vertical_mode(spelling, exceptional)
                        .map_err(command_error)?;
                    processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                        execution.main_loop_active = false
                    });
                    Ok(MainControlStep::Continue)
                }
                Mode::RestrictedHorizontal => {
                    recovery::off_save(processor, spelling, meaning, control_sequence, exceptional)
                }
                Mode::Math | Mode::DisplayMath => recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                ),
                Mode::InternalVertical => recovery::illegal_case(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                ),
                Mode::Vertical => termination::stop(
                    processor,
                    spelling,
                    exceptional,
                    code == codes::PRIMITIVE_DUMP,
                ),
            }
        }
        codes::PRIMITIVE_MESSAGE | codes::PRIMITIVE_ERR_MESSAGE => {
            streams::message(processor, code == codes::PRIMITIVE_ERR_MESSAGE)
        }
        codes::PRIMITIVE_OPEN_OUT => streams::open_out(processor),
        codes::PRIMITIVE_CLOSE_OUT => streams::close_out(processor),
        codes::PRIMITIVE_WRITE => streams::write(processor),
        codes::PRIMITIVE_SPECIAL => streams::special(processor),
        codes::PRIMITIVE_UPPERCASE | codes::PRIMITIVE_LOWERCASE => {
            processor
                .shift_case(code == codes::PRIMITIVE_UPPERCASE)
                .map_err(command_error)?;
            report_scan(processor)?;
            processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.main_loop_active = false
            });
            Ok(MainControlStep::Continue)
        }
        codes::PRIMITIVE_SET_LANGUAGE => {
            let mode = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.modes.current_mode()
            });
            if !matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal) {
                return recovery::illegal_case(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            let language = processor.scan_integer().map_err(command_error)?.value;
            report_scan(processor)?;
            processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
                execution.main_loop_active = false;
                semantic::execute_set_language(
                    context,
                    &mut execution.modes,
                    effects,
                    fuel,
                    language,
                )
            })
        }
        codes::PRIMITIVE_TOKS | codes::TOKS_REGISTER | codes::TOK_PARAM => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
            let owner = control_sequence.ok_or(ExecError::MissingToken {
                context: "token-list assignment owner",
            })?;
            let index = if code == codes::PRIMITIVE_TOKS {
                processor
                    .scan_profile_register_index()
                    .map_err(command_error)?
            } else {
                operand(meaning) as u16
            };
            assignments::tokens(
                processor,
                owner,
                index,
                code == codes::TOK_PARAM,
                global,
                observation_receipts,
            )
        }
        codes::PRIMITIVE_ADVANCE | codes::PRIMITIVE_MULTIPLY | codes::PRIMITIVE_DIVIDE => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
            let primitive = match code {
                codes::PRIMITIVE_ADVANCE => UnexpandablePrimitive::Advance,
                codes::PRIMITIVE_MULTIPLY => UnexpandablePrimitive::Multiply,
                _ => UnexpandablePrimitive::Divide,
            };
            arithmetic::execute_arithmetic(processor, primitive, global, observation_receipts)
        }
        codes::PRIMITIVE_CAT_CODE
        | codes::PRIMITIVE_LC_CODE
        | codes::PRIMITIVE_UC_CODE
        | codes::PRIMITIVE_SF_CODE
        | codes::PRIMITIVE_MATH_CODE
        | codes::PRIMITIVE_DEL_CODE => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
            let primitive = match code {
                codes::PRIMITIVE_CAT_CODE => UnexpandablePrimitive::CatCode,
                codes::PRIMITIVE_LC_CODE => UnexpandablePrimitive::LcCode,
                codes::PRIMITIVE_UC_CODE => UnexpandablePrimitive::UcCode,
                codes::PRIMITIVE_SF_CODE => UnexpandablePrimitive::SfCode,
                codes::PRIMITIVE_MATH_CODE => UnexpandablePrimitive::MathCode,
                _ => UnexpandablePrimitive::DelCode,
            };
            assignments::code_table(processor, primitive, global, observation_receipts)
        }
        codes::PRIMITIVE_CHAR_DEF | codes::PRIMITIVE_MATH_CHAR_DEF => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
            assignments::character_definition(
                processor,
                if code == codes::PRIMITIVE_CHAR_DEF {
                    UnexpandablePrimitive::CharDef
                } else {
                    UnexpandablePrimitive::MathCharDef
                },
                global,
                observation_receipts,
            )
        }
        codes::PRIMITIVE_COUNT_DEF
        | codes::PRIMITIVE_DIMEN_DEF
        | codes::PRIMITIVE_SKIP_DEF
        | codes::PRIMITIVE_MUSKIP_DEF
        | codes::PRIMITIVE_TOKS_DEF => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
            let primitive = match code {
                codes::PRIMITIVE_COUNT_DEF => UnexpandablePrimitive::CountDef,
                codes::PRIMITIVE_DIMEN_DEF => UnexpandablePrimitive::DimenDef,
                codes::PRIMITIVE_SKIP_DEF => UnexpandablePrimitive::SkipDef,
                codes::PRIMITIVE_MUSKIP_DEF => UnexpandablePrimitive::MuskipDef,
                _ => UnexpandablePrimitive::ToksDef,
            };
            assignments::register_definition(processor, primitive, global, observation_receipts)
        }
        codes::PRIMITIVE_TEXT_FONT
        | codes::PRIMITIVE_SCRIPT_FONT
        | codes::PRIMITIVE_SCRIPT_SCRIPT_FONT => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
            let size = match code {
                codes::PRIMITIVE_TEXT_FONT => MathFontSize::Text,
                codes::PRIMITIVE_SCRIPT_FONT => MathFontSize::Script,
                _ => MathFontSize::ScriptScript,
            };
            assignments::math_font(processor, size, global)
        }
        codes::GLUE_PARAM | codes::MU_GLUE_PARAM => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
            assignments::glue_parameter(
                processor,
                operand(meaning) as u16,
                code == codes::MU_GLUE_PARAM,
                global,
                observation_receipts,
            )
        }
        codes::PRIMITIVE_COUNT | codes::COUNT_REGISTER => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
            let index = if code == codes::COUNT_REGISTER {
                operand(meaning) as u16
            } else {
                processor
                    .scan_profile_register_index()
                    .map_err(command_error)?
            };
            assignments::count(processor, index, global, observation_receipts)
        }
        codes::PRIMITIVE_DIMEN | codes::DIMEN_REGISTER => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
            let index = if code == codes::DIMEN_REGISTER {
                operand(meaning) as u16
            } else {
                processor
                    .scan_profile_register_index()
                    .map_err(command_error)?
            };
            assignments::dimension(processor, index, global, observation_receipts)
        }
        codes::INT_PARAM => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
            assignments::integer_parameter(
                processor,
                operand(meaning) as u16,
                global,
                observation_receipts,
            )
        }
        codes::DIMEN_PARAM => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
            assignments::dimension_parameter(
                processor,
                operand(meaning) as u16,
                global,
                observation_receipts,
            )
        }
        codes::PRIMITIVE_SKIP
        | codes::SKIP_REGISTER
        | codes::PRIMITIVE_MUSKIP
        | codes::MUSKIP_REGISTER => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
            let mu = matches!(code, codes::PRIMITIVE_MUSKIP | codes::MUSKIP_REGISTER);
            let index = if matches!(code, codes::SKIP_REGISTER | codes::MUSKIP_REGISTER) {
                operand(meaning) as u16
            } else {
                processor
                    .scan_profile_register_index()
                    .map_err(command_error)?
            };
            assignments::glue_register(processor, index, mu, global, observation_receipts)
        }
        codes::PRIMITIVE_DEF
        | codes::PRIMITIVE_EDEF
        | codes::PRIMITIVE_GDEF
        | codes::PRIMITIVE_XDEF => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0
                    || matches!(code, codes::PRIMITIVE_GDEF | codes::PRIMITIVE_XDEF),
            );
            let expanded = matches!(code, codes::PRIMITIVE_EDEF | codes::PRIMITIVE_XDEF);
            assignments::definition(
                processor,
                expanded,
                MeaningFlags::from_bits(prefixes & !prefixes::GLOBAL),
                global,
                observation_receipts,
            )
        }
        codes::PRIMITIVE_LET => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
            let (target, meaning) = processor.scan_let_assignment().map_err(command_error)?;
            assignments::bind(processor, target, meaning, global, observation_receipts)
        }
        codes::PRIMITIVE_FUTURE_LET => {
            let global = effective_global(
                processor.int_param(IntParam::GLOBAL_DEFS),
                prefixes & prefixes::GLOBAL != 0,
            );
            let (target, meaning) = processor
                .scan_futurelet_assignment()
                .map_err(command_error)?;
            assignments::bind(processor, target, meaning, global, observation_receipts)
        }
        codes::PRIMITIVE_AFTER_GROUP => {
            let mut token = processor
                .get_token_spelling()
                .map_err(command_error)?
                .ok_or(ExecError::MissingToken {
                    context: "aftergroup operand",
                })?;
            report_scan(processor)?;
            processor.with_execution_owners(|context, command, _, _, _, execution, _, _, _| {
                execution.main_loop_active = false;
                semantic::execute_after_group(context, command, &mut token)
            })
        }
        codes::PRIMITIVE_AFTER_ASSIGNMENT => {
            let mut token = processor
                .get_token_spelling()
                .map_err(command_error)?
                .ok_or(ExecError::MissingToken {
                    context: "afterassignment operand",
                })?;
            report_scan(processor)?;
            processor.with_execution_owners(|context, command, _, _, _, execution, _, _, _| {
                execution.main_loop_active = false;
                semantic::execute_after_assignment(context, command, &mut token)
            })
        }
        codes::MATH_GIVEN => {
            if !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            math::character(processor, operand(meaning) as u32, spelling.origin())
        }
        codes::PRIMITIVE_MATH_CHAR | codes::PRIMITIVE_DELIMITER => {
            if !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            let class = if code == codes::PRIMITIVE_MATH_CHAR {
                RestrictedIntegerClass::FifteenBit
            } else {
                RestrictedIntegerClass::TwentySevenBit
            };
            let scanned = processor
                .scan_restricted_integer(class)
                .map_err(command_error)?;
            let value = if code == codes::PRIMITIVE_DELIMITER {
                scanned.value as u32 >> 12
            } else {
                scanned.value as u32
            };
            math::character(processor, value, scanned.provenance.primary)
        }
        codes::PRIMITIVE_DISPLAY_STYLE
        | codes::PRIMITIVE_TEXT_STYLE
        | codes::PRIMITIVE_SCRIPT_STYLE
        | codes::PRIMITIVE_SCRIPT_SCRIPT_STYLE => {
            if !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            let style = match code {
                codes::PRIMITIVE_DISPLAY_STYLE => MathStyle::Display,
                codes::PRIMITIVE_TEXT_STYLE => MathStyle::Text,
                codes::PRIMITIVE_SCRIPT_STYLE => MathStyle::Script,
                _ => MathStyle::ScriptScript,
            };
            math::style(processor, style)
        }
        codes::PRIMITIVE_MATH_ORD
        | codes::PRIMITIVE_MATH_OP
        | codes::PRIMITIVE_MATH_BIN
        | codes::PRIMITIVE_MATH_REL
        | codes::PRIMITIVE_MATH_OPEN
        | codes::PRIMITIVE_MATH_CLOSE
        | codes::PRIMITIVE_MATH_PUNCT
        | codes::PRIMITIVE_MATH_INNER => {
            if !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            let kind = match code {
                codes::PRIMITIVE_MATH_ORD => NoadKind::Normal(NoadClass::Ord),
                codes::PRIMITIVE_MATH_OP => NoadKind::Normal(NoadClass::Op),
                codes::PRIMITIVE_MATH_BIN => NoadKind::Normal(NoadClass::Bin),
                codes::PRIMITIVE_MATH_REL => NoadKind::Normal(NoadClass::Rel),
                codes::PRIMITIVE_MATH_OPEN => NoadKind::Normal(NoadClass::Open),
                codes::PRIMITIVE_MATH_CLOSE => NoadKind::Normal(NoadClass::Close),
                codes::PRIMITIVE_MATH_PUNCT => NoadKind::Normal(NoadClass::Punct),
                _ => NoadKind::Normal(NoadClass::Inner),
            };
            math::nucleus(processor, kind)
        }
        codes::CHAR_SUPERSCRIPT | codes::CHAR_SUBSCRIPT => {
            if !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            let kind = if code == codes::CHAR_SUPERSCRIPT {
                MathScriptKind::Superscript
            } else {
                MathScriptKind::Subscript
            };
            math::script(processor, kind)
        }
        codes::PRIMITIVE_LIMITS | codes::PRIMITIVE_NO_LIMITS | codes::PRIMITIVE_DISPLAY_LIMITS => {
            if !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            let kind = match code {
                codes::PRIMITIVE_LIMITS => LimitType::Limits,
                codes::PRIMITIVE_NO_LIMITS => LimitType::NoLimits,
                _ => LimitType::DisplayLimits,
            };
            math::limits(processor, kind)
        }
        codes::PRIMITIVE_OVER
        | codes::PRIMITIVE_ATOP
        | codes::PRIMITIVE_ABOVE
        | codes::PRIMITIVE_OVER_WITH_DELIMS
        | codes::PRIMITIVE_ATOP_WITH_DELIMS
        | codes::PRIMITIVE_ABOVE_WITH_DELIMS => {
            if !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            let kind = match code {
                codes::PRIMITIVE_OVER | codes::PRIMITIVE_OVER_WITH_DELIMS => MathFractionKind::Over,
                codes::PRIMITIVE_ATOP | codes::PRIMITIVE_ATOP_WITH_DELIMS => MathFractionKind::Atop,
                _ => MathFractionKind::Above,
            };
            let delimiters = matches!(
                code,
                codes::PRIMITIVE_OVER_WITH_DELIMS
                    | codes::PRIMITIVE_ATOP_WITH_DELIMS
                    | codes::PRIMITIVE_ABOVE_WITH_DELIMS
            );
            math::fraction(processor, kind, delimiters)
        }
        codes::PRIMITIVE_UNDERLINE | codes::PRIMITIVE_OVERLINE => {
            if !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            let kind = if code == codes::PRIMITIVE_UNDERLINE {
                NoadKind::Underline
            } else {
                NoadKind::Overline
            };
            math::nucleus(processor, kind)
        }
        codes::PRIMITIVE_RADICAL => {
            if !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            let delimiter = processor.scan_delimiter(true).map_err(command_error)?;
            math::nucleus(
                processor,
                NoadKind::Radical {
                    delimiter: delimiter.code,
                },
            )
        }
        codes::PRIMITIVE_MATH_ACCENT => {
            if !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            let scanned = processor.scan_math_character().map_err(command_error)?;
            let accent = processor
                .with_execution_owners(|context, _, _, _, _, _, _, _, _| {
                    math_char(context, u32::from(scanned.code), scanned.provenance.primary)
                })?
                .1;
            math::nucleus(processor, NoadKind::Accent { accent })
        }
        codes::PRIMITIVE_DISCRETIONARY | codes::PRIMITIVE_DISCRETIONARY_HYPHEN => {
            let mode = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.modes.current_mode()
            });
            if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                return recovery::start_paragraph_and_reswitch(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            if code == codes::PRIMITIVE_DISCRETIONARY {
                discretionary::begin(processor)
            } else {
                discretionary::hyphen(
                    processor,
                    processor.active_macro_origin().unwrap_or(spelling.origin()),
                )
            }
        }
        codes::PRIMITIVE_MATH_CHOICE => {
            if !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            math::choice(processor)
        }
        codes::PRIMITIVE_H_BOX
        | codes::PRIMITIVE_V_BOX
        | codes::PRIMITIVE_V_TOP
        | codes::PRIMITIVE_V_CENTER => {
            if code == codes::PRIMITIVE_V_CENTER && !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            let primitive = match code {
                codes::PRIMITIVE_H_BOX => UnexpandablePrimitive::HBox,
                codes::PRIMITIVE_V_BOX => UnexpandablePrimitive::VBox,
                codes::PRIMITIVE_V_TOP => UnexpandablePrimitive::VTop,
                _ => UnexpandablePrimitive::VCenter,
            };
            boxes::begin(processor, primitive, observation_receipts)
        }
        codes::PRIMITIVE_H_RULE | codes::PRIMITIVE_V_RULE => {
            let mode = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.modes.current_mode()
            });
            if code == codes::PRIMITIVE_H_RULE {
                match mode {
                    Mode::Math | Mode::DisplayMath => {
                        return recovery::insert_math_shift(
                            processor,
                            spelling,
                            meaning,
                            control_sequence,
                            exceptional,
                        );
                    }
                    Mode::Horizontal => {
                        processor
                            .recover_stop_for_vertical_mode(spelling, exceptional)
                            .map_err(command_error)?;
                        processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                            execution.main_loop_active = false
                        });
                        return Ok(MainControlStep::Continue);
                    }
                    Mode::RestrictedHorizontal => {
                        return recovery::off_save(
                            processor,
                            spelling,
                            meaning,
                            control_sequence,
                            exceptional,
                        );
                    }
                    _ => {}
                }
            } else if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                return recovery::start_paragraph_and_reswitch(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            material::rule(
                processor,
                if code == codes::PRIMITIVE_H_RULE {
                    UnexpandablePrimitive::HRule
                } else {
                    UnexpandablePrimitive::VRule
                },
            )
        }
        codes::PRIMITIVE_CONTROL_SPACE
        | codes::PRIMITIVE_H_FIL
        | codes::PRIMITIVE_H_FILL
        | codes::PRIMITIVE_H_SS
        | codes::PRIMITIVE_H_FIL_NEG => {
            let mode = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.modes.current_mode()
            });
            if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                return recovery::start_paragraph_and_reswitch(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            if code == codes::PRIMITIVE_CONTROL_SPACE {
                return material::control_space(processor);
            }
            let primitive = match code {
                codes::PRIMITIVE_H_FIL => UnexpandablePrimitive::HFil,
                codes::PRIMITIVE_H_FILL => UnexpandablePrimitive::HFill,
                codes::PRIMITIVE_H_SS => UnexpandablePrimitive::HSs,
                _ => UnexpandablePrimitive::HFilNeg,
            };
            material::fixed_glue(processor, primitive)
        }
        codes::PRIMITIVE_V_FIL
        | codes::PRIMITIVE_V_FILL
        | codes::PRIMITIVE_V_SS
        | codes::PRIMITIVE_V_FIL_NEG => {
            let mode = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.modes.current_mode()
            });
            match mode {
                Mode::Math | Mode::DisplayMath => recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                ),
                Mode::Horizontal => {
                    processor
                        .recover_stop_for_vertical_mode(spelling, exceptional)
                        .map_err(command_error)?;
                    processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                        execution.main_loop_active = false
                    });
                    Ok(MainControlStep::Continue)
                }
                Mode::RestrictedHorizontal => {
                    recovery::off_save(processor, spelling, meaning, control_sequence, exceptional)
                }
                _ => {
                    let primitive = match code {
                        codes::PRIMITIVE_V_FIL => UnexpandablePrimitive::VFil,
                        codes::PRIMITIVE_V_FILL => UnexpandablePrimitive::VFill,
                        codes::PRIMITIVE_V_SS => UnexpandablePrimitive::VSs,
                        _ => UnexpandablePrimitive::VFilNeg,
                    };
                    material::fixed_glue(processor, primitive)
                }
            }
        }
        codes::PRIMITIVE_ITALIC_CORRECTION => {
            let mode = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.modes.current_mode()
            });
            if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                recovery::illegal_case(processor, spelling, meaning, control_sequence, exceptional)
            } else {
                material::italic_correction(processor)
            }
        }
        codes::PRIMITIVE_NO_BOUNDARY => {
            let mode = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.modes.current_mode()
            });
            if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                return recovery::start_paragraph_and_reswitch(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            characters::no_boundary(processor, observation_receipts)
        }
        codes::PRIMITIVE_NON_SCRIPT => {
            if !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            material::non_script(processor)
        }
        codes::PRIMITIVE_PAR => {
            let mode = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.modes.current_mode()
            });
            match mode {
                Mode::Math | Mode::DisplayMath => recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                ),
                Mode::RestrictedHorizontal => {
                    processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                        execution.main_loop_active = false
                    });
                    Ok(MainControlStep::Continue)
                }
                _ => {
                    report_scan(processor)?;
                    processor.with_execution_owners(
                        |context, command, effects, fuel, observer, execution, _, _, _| {
                            execution.main_loop_active = false;
                            let line = command
                                .current_file_line_number()
                                .try_into()
                                .unwrap_or(i32::MAX);
                            let outer_paragraph = execution.modes.current_mode()
                                == Mode::Horizontal
                                && execution.modes.depth() == 2;
                            let result = semantic::execute_paragraph(
                                context,
                                &mut execution.modes,
                                command,
                                effects,
                                fuel,
                                observer,
                                execution.boxes.output_routine_active,
                                line,
                            );
                            if result.is_ok()
                                && outer_paragraph
                                && execution.paragraph_checkpoint_demand.is_some()
                                && command.current_file_source_id() == execution.root_main_source
                                && context.execution_group_depth() == 0
                                && execution.modes.current_mode() == Mode::Vertical
                                && execution.modes.depth() == 1
                            {
                                execution.paragraph_checkpoint_cut = true;
                            }
                            result
                        },
                    )
                }
            }
        }
        codes::PRIMITIVE_INDENT | codes::PRIMITIVE_NO_INDENT => {
            report_scan(processor)?;
            processor.with_execution_owners(
                |context, command, effects, fuel, observer, execution, _, _, _| {
                    execution.main_loop_active = false;
                    let result = semantic::execute_paragraph_indent(
                        context,
                        &mut execution.modes,
                        command,
                        effects,
                        fuel,
                        code == codes::PRIMITIVE_INDENT,
                    );
                    command.publish_named_token_list_pushes(context, effects, observer);
                    result
                },
            )
        }
        codes::PRIMITIVE_KERN => material::kern(processor),
        codes::PRIMITIVE_PENALTY => material::penalty(processor),
        codes::PRIMITIVE_H_SKIP => {
            let mode = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.modes.current_mode()
            });
            if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                return recovery::start_paragraph_and_reswitch(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            material::horizontal_glue(processor)
        }
        codes::PRIMITIVE_V_SKIP => {
            let mode = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.modes.current_mode()
            });
            match mode {
                Mode::Math | Mode::DisplayMath => recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                ),
                Mode::Horizontal => {
                    processor
                        .recover_stop_for_vertical_mode(spelling, exceptional)
                        .map_err(command_error)?;
                    processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                        execution.main_loop_active = false
                    });
                    Ok(MainControlStep::Continue)
                }
                Mode::Vertical | Mode::InternalVertical => material::vertical_glue(processor),
                Mode::RestrictedHorizontal => {
                    recovery::off_save(processor, spelling, meaning, control_sequence, exceptional)
                }
            }
        }
        codes::PRIMITIVE_M_SKIP | codes::PRIMITIVE_M_KERN => {
            if !math_mode(processor) {
                return recovery::insert_math_shift(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                );
            }
            if code == codes::PRIMITIVE_M_SKIP {
                material::mu_glue(processor)
            } else {
                material::mu_kern(processor)
            }
        }
        codes::PRIMITIVE_ACCENT => {
            let mode = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.modes.current_mode()
            });
            match mode {
                Mode::Math | Mode::DisplayMath => math::text_accent(processor),
                Mode::Vertical | Mode::InternalVertical => recovery::start_paragraph_and_reswitch(
                    processor,
                    spelling,
                    meaning,
                    control_sequence,
                    exceptional,
                ),
                Mode::Horizontal | Mode::RestrictedHorizontal => {
                    accent::apply(processor, observation_receipts)
                }
            }
        }
        codes::PRIMITIVE_EQ_NO | codes::PRIMITIVE_LEFT_EQ_NO => {
            let side = if code == codes::PRIMITIVE_LEFT_EQ_NO {
                crate::mode::EqNoSide::Left
            } else {
                crate::mode::EqNoSide::Right
            };
            math::equation_number(processor, side, spelling)
        }
        _ => execute_selected_command(
            processor,
            code,
            spelling,
            meaning,
            control_sequence,
            exceptional,
        ),
    }
}

fn operand<G>(meaning: &MeaningWord<G>) -> u64 {
    let MeaningWord::Static(word) = meaning else {
        unreachable!("selected static operand")
    };
    Meaning::runtime_word_operand(*word)
}

fn math_mode<G>(processor: &mut Processor<'_, '_, G>) -> bool {
    processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
        matches!(
            execution.modes.current_mode(),
            Mode::Math | Mode::DisplayMath
        )
    })
}
