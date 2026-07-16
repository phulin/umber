use tex_lex::{InputStack, MacroArguments};
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::{FontId, TokenListId};
use tex_state::interner::ControlSequenceKind;
use tex_state::math::MathFontSize;
use tex_state::meaning::{InternalInteger, Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::provenance::SynthesizedOriginKind;
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::{BoxDimension, ExpansionState, PenaltyArrayKind};

use crate::{
    Dispatch, ExpandError, ExpandableOpcode, ExpansionContext, ExpansionMode, ExpansionReplayKind,
    ReadBank, ReadCodeTable, ReadDependency, ReadEngineField, ReadFontField,
    RestrictedExpansionMode, scan_helpers, scan_int,
};

#[allow(dead_code)]
pub(crate) fn expand_the(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    context: TracedTokenWord,
) -> Result<Dispatch, ExpandError> {
    expand_the_with_mode_and_context(
        input,
        stores,
        expansion,
        &mut RestrictedExpansionMode,
        context,
    )
}

pub(crate) fn expand_the_with_mode_and_context(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    cause_context: TracedTokenWord,
) -> Result<Dispatch, ExpandError>
where
{
    let cause_origin = cause_context.origin();
    let Some(token) =
        scan_helpers::next_non_space_x_token_with_mode_and_context(input, stores, expansion, mode)?
    else {
        return Err(ExpandError::MissingTokenAfterPrimitive {
            opcode: ExpandableOpcode::The,
            context: cause_context,
        });
    };
    let semantic = crate::semantic_token(token);
    let mut live_symbol = None;
    let meaning = match semantic {
        Token::Cs(symbol) => {
            live_symbol = Some(symbol);
            let meaning = stores.meaning(symbol);
            expansion.record_meaning(symbol, meaning);
            meaning
        }
        Token::Frozen(_) => stores
            .frozen_primitive_meaning(semantic)
            .ok_or(ExpandError::UnsupportedTheTarget { context: token })?,
        _ => return Err(ExpandError::UnsupportedTheTarget { context: token }),
    };
    record_meaning_value_dependency(expansion, meaning);
    match meaning {
        Meaning::UnexpandablePrimitive(primitive) => match primitive {
            tex_state::meaning::UnexpandablePrimitive::Count => {
                let index = scan_helpers::scan_register_index_with_mode_and_context(
                    input, stores, expansion, mode, token,
                )?;
                crate::record_dependency!(
                    expansion,
                    ReadDependency::Cell {
                        bank: ReadBank::Count,
                        index: u32::from(index),
                    }
                );
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &stores.count(index).to_string(),
                    cause_origin,
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::Dimen => {
                let index = scan_helpers::scan_register_index_with_mode_and_context(
                    input, stores, expansion, mode, token,
                )?;
                crate::record_dependency!(
                    expansion,
                    ReadDependency::Cell {
                        bank: ReadBank::Dimen,
                        index: u32::from(index),
                    }
                );
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &format_scaled(stores.dimen(index)),
                    cause_origin,
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::Skip => {
                let index = scan_helpers::scan_register_index_with_mode_and_context(
                    input, stores, expansion, mode, token,
                )?;
                crate::record_dependency!(
                    expansion,
                    ReadDependency::Cell {
                        bank: ReadBank::Skip,
                        index: u32::from(index),
                    }
                );
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &format_glue(stores.glue(stores.skip(index))),
                    cause_origin,
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::Muskip => {
                let index = scan_helpers::scan_register_index_with_mode_and_context(
                    input, stores, expansion, mode, token,
                )?;
                crate::record_dependency!(
                    expansion,
                    ReadDependency::Cell {
                        bank: ReadBank::Muskip,
                        index: u32::from(index),
                    }
                );
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &format_muglue(stores.glue(stores.muskip(index))),
                    cause_origin,
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::Toks => {
                let index = scan_helpers::scan_register_index_with_mode_and_context(
                    input, stores, expansion, mode, token,
                )?;
                crate::record_dependency!(
                    expansion,
                    ReadDependency::Cell {
                        bank: ReadBank::Toks,
                        index: u32::from(index),
                    }
                );
                let token_list = stores.toks(index);
                let origin_list = crate::expansion_suppressed_origin_list(
                    stores,
                    token_list,
                    tex_state::ids::OriginListId::EMPTY,
                    cause_origin,
                );
                Ok(Dispatch::Push {
                    replay_kind: ExpansionReplayKind::Unexpanded,
                    token_list,
                    origin_list,
                    macro_arguments: MacroArguments::new(),
                    macro_invocation: OriginId::UNKNOWN,
                })
            }
            tex_state::meaning::UnexpandablePrimitive::Font => {
                crate::record_dependency!(
                    expansion,
                    ReadDependency::Cell {
                        bank: ReadBank::CurrentFont,
                        index: 0,
                    }
                );
                let font = stores.current_font();
                crate::record_dependency!(
                    expansion,
                    ReadDependency::Font {
                        field: ReadFontField::Identifier,
                        font: font.raw(),
                        index: 0,
                    }
                );
                let symbol = stores
                    .font_identifier_symbol(font)
                    .ok_or(ExpandError::UnsupportedTheTarget { context: token })?;
                Ok(push_rendered_tokens(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    [Token::Cs(symbol.symbol())],
                    cause_origin,
                ))
            }
            primitive @ (tex_state::meaning::UnexpandablePrimitive::TextFont
            | tex_state::meaning::UnexpandablePrimitive::ScriptFont
            | tex_state::meaning::UnexpandablePrimitive::ScriptScriptFont) => {
                let family = scan_math_family(input, stores, expansion, mode, token)?;
                let size = math_font_size(primitive);
                crate::record_dependency!(
                    expansion,
                    ReadDependency::Cell {
                        bank: ReadBank::MathFamilyFont,
                        index: u32::from(family) + 16 * u32::from(math_font_size_key(size)),
                    }
                );
                let font = stores.math_family_font(size, family);
                crate::record_dependency!(
                    expansion,
                    ReadDependency::Font {
                        field: ReadFontField::Identifier,
                        font: font.raw(),
                        index: 0,
                    }
                );
                let symbol = stores
                    .font_identifier_symbol(font)
                    .ok_or(ExpandError::UnsupportedTheTarget { context: token })?;
                Ok(push_rendered_tokens(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    [Token::Cs(symbol.symbol())],
                    cause_origin,
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::FontDimen => {
                let value = scan_font_dimen(input, stores, expansion, mode, token)?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &format_scaled(value),
                    cause_origin,
                ))
            }
            primitive @ (tex_state::meaning::UnexpandablePrimitive::FontCharWd
            | tex_state::meaning::UnexpandablePrimitive::FontCharHt
            | tex_state::meaning::UnexpandablePrimitive::FontCharDp
            | tex_state::meaning::UnexpandablePrimitive::FontCharIc) => {
                let value =
                    scan_font_char_dimension(input, stores, expansion, mode, token, primitive)?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &format_scaled(value),
                    cause_origin,
                ))
            }
            primitive @ (tex_state::meaning::UnexpandablePrimitive::ParShapeLength
            | tex_state::meaning::UnexpandablePrimitive::ParShapeIndent
            | tex_state::meaning::UnexpandablePrimitive::ParShapeDimen) => {
                let value =
                    scan_parshape_dimension(input, stores, expansion, mode, token, primitive)?;
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &format_scaled(value),
                    cause_origin,
                ))
            }
            primitive @ (tex_state::meaning::UnexpandablePrimitive::InterLinePenalties
            | tex_state::meaning::UnexpandablePrimitive::ClubPenalties
            | tex_state::meaning::UnexpandablePrimitive::WidowPenalties
            | tex_state::meaning::UnexpandablePrimitive::DisplayWidowPenalties) => {
                let index = scan_int::scan_int_with_mode_and_context(
                    input, stores, expansion, mode, token,
                )?
                .value();
                crate::record_dependency!(
                    expansion,
                    ReadDependency::Engine(ReadEngineField::PenaltyArrays)
                );
                let kind = match primitive {
                    tex_state::meaning::UnexpandablePrimitive::InterLinePenalties => {
                        PenaltyArrayKind::InterLine
                    }
                    tex_state::meaning::UnexpandablePrimitive::ClubPenalties => {
                        PenaltyArrayKind::Club
                    }
                    tex_state::meaning::UnexpandablePrimitive::WidowPenalties => {
                        PenaltyArrayKind::Widow
                    }
                    tex_state::meaning::UnexpandablePrimitive::DisplayWidowPenalties => {
                        PenaltyArrayKind::DisplayWidow
                    }
                    _ => unreachable!("outer match restricts primitive"),
                };
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &stores.penalty_array_value(kind, index).to_string(),
                    cause_origin,
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::HyphenChar => {
                let font = scan_font_selector(input, stores, expansion, mode, token)?;
                crate::record_dependency!(
                    expansion,
                    ReadDependency::Font {
                        field: ReadFontField::HyphenChar,
                        font: font.raw(),
                        index: 0,
                    }
                );
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &stores.font_hyphen_char(font).to_string(),
                    cause_origin,
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::SkewChar => {
                let font = scan_font_selector(input, stores, expansion, mode, token)?;
                crate::record_dependency!(
                    expansion,
                    ReadDependency::Font {
                        field: ReadFontField::SkewChar,
                        font: font.raw(),
                        index: 0,
                    }
                );
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &stores.font_skew_char(font).to_string(),
                    cause_origin,
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::Wd
            | tex_state::meaning::UnexpandablePrimitive::Ht
            | tex_state::meaning::UnexpandablePrimitive::Dp => {
                let index = scan_helpers::scan_register_index_with_mode_and_context(
                    input, stores, expansion, mode, token,
                )?;
                let dimension = match primitive {
                    tex_state::meaning::UnexpandablePrimitive::Wd => BoxDimension::Width,
                    tex_state::meaning::UnexpandablePrimitive::Ht => BoxDimension::Height,
                    tex_state::meaning::UnexpandablePrimitive::Dp => BoxDimension::Depth,
                    _ => unreachable!("outer match restricts primitive"),
                };
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &format_scaled(
                        stores
                            .box_dimension(index, dimension)
                            .unwrap_or_else(|| Scaled::from_raw(0)),
                    ),
                    cause_origin,
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::SpaceFactor => Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::TheOutput,
                &expansion.engine.space_factor.to_string(),
                cause_origin,
            )),
            tex_state::meaning::UnexpandablePrimitive::InteractionMode => {
                crate::record_dependency!(
                    expansion,
                    ReadDependency::Engine(ReadEngineField::InteractionMode)
                );
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &stores.interaction_mode_value().to_string(),
                    cause_origin,
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::NumExpr => {
                let value = scan_int::scan_num_expr(input, stores, expansion, mode, token)?.value();
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &value.to_string(),
                    cause_origin,
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::DimExpr => {
                let value =
                    crate::scan_dimen::scan_dim_expr(input, stores, expansion, mode, token)?
                        .value();
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &format_scaled(value),
                    cause_origin,
                ))
            }
            primitive @ (tex_state::meaning::UnexpandablePrimitive::GlueExpr
            | tex_state::meaning::UnexpandablePrimitive::MuExpr) => {
                let mu = primitive == tex_state::meaning::UnexpandablePrimitive::MuExpr;
                let value =
                    crate::scan_glue::scan_glue_expr(input, stores, expansion, mode, mu, token)?;
                let spec = stores.glue(value.id());
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &if mu {
                        format_muglue(spec)
                    } else {
                        format_glue(spec)
                    },
                    cause_origin,
                ))
            }
            primitive @ (tex_state::meaning::UnexpandablePrimitive::GlueStretch
            | tex_state::meaning::UnexpandablePrimitive::GlueShrink) => {
                let scanned = crate::scan_glue::scan_glue_with_mode_and_context(
                    input, stores, expansion, mode, false, token,
                )?;
                let spec = stores.glue(scanned.id());
                let value = if primitive == tex_state::meaning::UnexpandablePrimitive::GlueStretch {
                    spec.stretch
                } else {
                    spec.shrink
                };
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &format_scaled(value),
                    cause_origin,
                ))
            }
            primitive @ (tex_state::meaning::UnexpandablePrimitive::GlueStretchOrder
            | tex_state::meaning::UnexpandablePrimitive::GlueShrinkOrder) => {
                let scanned = crate::scan_glue::scan_glue_with_mode_and_context(
                    input, stores, expansion, mode, false, token,
                )?;
                let spec = stores.glue(scanned.id());
                let order =
                    if primitive == tex_state::meaning::UnexpandablePrimitive::GlueStretchOrder {
                        spec.stretch_order
                    } else {
                        spec.shrink_order
                    };
                let value = match order {
                    tex_state::glue::Order::Normal => 0,
                    tex_state::glue::Order::Fil => 1,
                    tex_state::glue::Order::Fill => 2,
                    tex_state::glue::Order::Filll => 3,
                };
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &value.to_string(),
                    cause_origin,
                ))
            }
            primitive @ (tex_state::meaning::UnexpandablePrimitive::GlueToMu
            | tex_state::meaning::UnexpandablePrimitive::MuToGlue) => {
                let to_mu = primitive == tex_state::meaning::UnexpandablePrimitive::GlueToMu;
                let scanned = crate::scan_glue::scan_glue_with_mode_and_context(
                    input, stores, expansion, mode, !to_mu, token,
                )?;
                let spec = stores.glue(scanned.id());
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &if to_mu {
                        format_muglue(spec)
                    } else {
                        format_glue(spec)
                    },
                    cause_origin,
                ))
            }
            tex_state::meaning::UnexpandablePrimitive::PrevDepth => Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::TheOutput,
                &format_scaled(expansion.engine.prev_depth),
                cause_origin,
            )),
            tex_state::meaning::UnexpandablePrimitive::PrevGraf => Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::TheOutput,
                &expansion.engine.prev_graf.to_string(),
                cause_origin,
            )),
            tex_state::meaning::UnexpandablePrimitive::LastPenalty => Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::TheOutput,
                &expansion.engine.last_penalty.to_string(),
                cause_origin,
            )),
            tex_state::meaning::UnexpandablePrimitive::LastKern => Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::TheOutput,
                &format_scaled(expansion.engine.last_kern),
                cause_origin,
            )),
            tex_state::meaning::UnexpandablePrimitive::LastSkip => Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::TheOutput,
                &format_glue(expansion.engine.last_skip),
                cause_origin,
            )),
            tex_state::meaning::UnexpandablePrimitive::CatCode
            | tex_state::meaning::UnexpandablePrimitive::LcCode
            | tex_state::meaning::UnexpandablePrimitive::UcCode
            | tex_state::meaning::UnexpandablePrimitive::SfCode
            | tex_state::meaning::UnexpandablePrimitive::MathCode
            | tex_state::meaning::UnexpandablePrimitive::DelCode => {
                let ch = scan_code_table_char(input, stores, expansion, mode, token)?;
                crate::record_code_dependency(expansion, code_table_key(primitive), ch);
                let value = match primitive {
                    tex_state::meaning::UnexpandablePrimitive::CatCode => stores.catcode(ch) as i32,
                    tex_state::meaning::UnexpandablePrimitive::LcCode => stores.lccode(ch) as i32,
                    tex_state::meaning::UnexpandablePrimitive::UcCode => stores.uccode(ch) as i32,
                    tex_state::meaning::UnexpandablePrimitive::SfCode => stores.sfcode(ch) as i32,
                    tex_state::meaning::UnexpandablePrimitive::MathCode => {
                        stores.mathcode(ch) as i32
                    }
                    tex_state::meaning::UnexpandablePrimitive::DelCode => stores.delcode(ch),
                    _ => unreachable!("outer match restricts primitive"),
                };
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &value.to_string(),
                    cause_origin,
                ))
            }
            primitive @ (tex_state::meaning::UnexpandablePrimitive::PdfLpCode
            | tex_state::meaning::UnexpandablePrimitive::PdfRpCode
            | tex_state::meaning::UnexpandablePrimitive::PdfEfCode
            | tex_state::meaning::UnexpandablePrimitive::PdfTagCode
            | tex_state::meaning::UnexpandablePrimitive::PdfKnbsCode
            | tex_state::meaning::UnexpandablePrimitive::PdfStbsCode
            | tex_state::meaning::UnexpandablePrimitive::PdfShbsCode
            | tex_state::meaning::UnexpandablePrimitive::PdfKnbcCode
            | tex_state::meaning::UnexpandablePrimitive::PdfKnacCode) => {
                let font = scan_font_selector(input, stores, expansion, mode, token)?;
                let scanned = scan_int::scan_int_with_mode_and_context(
                    input, stores, expansion, mode, token,
                )?;
                let code = u8::try_from(scanned.value())
                    .map_err(
                        |_| crate::scan_int::ScanIntError::RegisterNumberOutOfRange {
                            value: scanned.value(),
                            context: scanned.context(),
                        },
                    )
                    .map_err(ExpandError::from)?;
                let table = pdf_font_code_table(primitive);
                crate::record_dependency!(
                    expansion,
                    ReadDependency::Font {
                        field: ReadFontField::PdfCode,
                        font: font.raw(),
                        index: (table as u32) * 256 + u32::from(code),
                    }
                );
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &stores.pdf_font_code(table, font, code).to_string(),
                    cause_origin,
                ))
            }
            _ => Err(ExpandError::UnsupportedTheTarget { context: token }),
        },
        Meaning::CountRegister(index) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.count(index).to_string(),
            cause_origin,
        )),
        Meaning::DimenRegister(index) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &format_scaled(stores.dimen(index)),
            cause_origin,
        )),
        Meaning::SkipRegister(index) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &format_glue(stores.glue(stores.skip(index))),
            cause_origin,
        )),
        Meaning::MuskipRegister(index) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &format_muglue(stores.glue(stores.muskip(index))),
            cause_origin,
        )),
        Meaning::ToksRegister(index) => {
            let token_list = stores.toks(index);
            let origin_list = crate::expansion_suppressed_origin_list(
                stores,
                token_list,
                tex_state::ids::OriginListId::EMPTY,
                cause_origin,
            );
            Ok(Dispatch::Push {
                replay_kind: ExpansionReplayKind::Unexpanded,
                token_list,
                origin_list,
                macro_arguments: MacroArguments::new(),
                macro_invocation: OriginId::UNKNOWN,
            })
        }
        Meaning::IntParam(index) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.int_param(IntParam::new(index)).to_string(),
            cause_origin,
        )),
        Meaning::InternalInteger(InternalInteger::Badness) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.last_badness().to_string(),
            cause_origin,
        )),
        Meaning::InternalInteger(InternalInteger::InputLineNumber) => {
            let line = input
                .current_source_frame()
                .map_or(0, |frame| frame.line_number().min(i32::MAX as usize) as i32);
            Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::TheOutput,
                &line.to_string(),
                cause_origin,
            ))
        }
        Meaning::InternalInteger(InternalInteger::ETeXVersion) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            "2",
            cause_origin,
        )),
        Meaning::InternalInteger(InternalInteger::PdfTeXVersion) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            "140",
            cause_origin,
        )),
        Meaning::InternalInteger(InternalInteger::PdfElapsedTime) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.pdf_elapsed_time().to_string(),
            cause_origin,
        )),
        Meaning::InternalInteger(InternalInteger::PdfRandomSeed) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.pdf_random_seed().to_string(),
            cause_origin,
        )),
        Meaning::InternalInteger(InternalInteger::PdfShellEscape) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.pdf_shell_escape_status().to_string(),
            cause_origin,
        )),
        Meaning::InternalInteger(InternalInteger::PdfLastObject) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.pdf_last_object().to_string(),
            cause_origin,
        )),
        Meaning::InternalInteger(InternalInteger::PdfLastAnnot) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.pdf_last_annotation().to_string(),
            cause_origin,
        )),
        Meaning::InternalInteger(InternalInteger::PdfLastLink) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.pdf_last_link().to_string(),
            cause_origin,
        )),
        Meaning::InternalInteger(InternalInteger::PdfLastXPos) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.pdf_last_position().0.raw().to_string(),
            cause_origin,
        )),
        Meaning::InternalInteger(InternalInteger::PdfLastYPos) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.pdf_last_position().1.raw().to_string(),
            cause_origin,
        )),
        Meaning::InternalInteger(InternalInteger::PdfLastXForm) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.pdf_last_form().to_string(),
            cause_origin,
        )),
        Meaning::InternalInteger(InternalInteger::PdfLastXImage) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.pdf_last_ximage().to_string(),
            cause_origin,
        )),
        Meaning::InternalInteger(InternalInteger::PdfReturnValue) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.pdf_return_value().to_string(),
            cause_origin,
        )),
        Meaning::InternalInteger(InternalInteger::PdfLastXImagePages) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.pdf_last_ximage_pages().to_string(),
            cause_origin,
        )),
        Meaning::InternalInteger(InternalInteger::PdfLastXImageColorDepth) => {
            Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::TheOutput,
                &stores.pdf_last_ximage_color_depth().to_string(),
                cause_origin,
            ))
        }
        Meaning::InternalInteger(InternalInteger::CurrentGroupLevel) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.execution_group_depth().to_string(),
            cause_origin,
        )),
        Meaning::InternalInteger(InternalInteger::CurrentGroupType) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores
                .current_group_kind()
                .map_or(0, tex_state::GroupKind::etex_code)
                .to_string(),
            cause_origin,
        )),
        Meaning::InternalInteger(InternalInteger::CurrentIfLevel) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &input.condition_depth().to_string(),
            cause_origin,
        )),
        Meaning::InternalInteger(InternalInteger::CurrentIfType) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &scan_int::current_if_type(input).to_string(),
            cause_origin,
        )),
        Meaning::InternalInteger(InternalInteger::CurrentIfBranch) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &scan_int::current_if_branch(input).to_string(),
            cause_origin,
        )),
        Meaning::InternalInteger(InternalInteger::LastNodeType) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &expansion.engine.last_node_type.to_string(),
            cause_origin,
        )),
        Meaning::CharGiven(ch) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &(ch as u32).to_string(),
            cause_origin,
        )),
        Meaning::MathCharGiven(value) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &value.to_string(),
            cause_origin,
        )),
        Meaning::DimenParam(index) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &format_scaled(stores.dimen_param(DimenParam::new(index))),
            cause_origin,
        )),
        Meaning::PageDimension(dimension) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &format_scaled(stores.page_dimension(dimension)),
            cause_origin,
        )),
        Meaning::PageInteger(integer) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.page_integer(integer).to_string(),
            cause_origin,
        )),
        Meaning::GlueParam(index) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &format_glue(stores.glue(stores.glue_param(GlueParam::new(index)))),
            cause_origin,
        )),
        Meaning::MuGlueParam(index) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &format_muglue(stores.glue(stores.glue_param(GlueParam::new(index)))),
            cause_origin,
        )),
        Meaning::TokParam(index) => {
            let token_list = stores.tok_param(TokParam::new(index));
            let origin_list = crate::expansion_suppressed_origin_list(
                stores,
                token_list,
                tex_state::ids::OriginListId::EMPTY,
                cause_origin,
            );
            Ok(Dispatch::Push {
                replay_kind: ExpansionReplayKind::Unexpanded,
                token_list,
                origin_list,
                macro_arguments: MacroArguments::new(),
                macro_invocation: OriginId::UNKNOWN,
            })
        }
        _ => match live_symbol.map_or("", |symbol| stores.resolve(symbol)) {
            "count" => {
                let index = scan_helpers::scan_register_index_with_mode_and_context(
                    input, stores, expansion, mode, token,
                )?;
                crate::record_dependency!(
                    expansion,
                    ReadDependency::Cell {
                        bank: ReadBank::Count,
                        index: u32::from(index),
                    }
                );
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &stores.count(index).to_string(),
                    cause_origin,
                ))
            }
            "dimen" => {
                let index = scan_helpers::scan_register_index_with_mode_and_context(
                    input, stores, expansion, mode, token,
                )?;
                crate::record_dependency!(
                    expansion,
                    ReadDependency::Cell {
                        bank: ReadBank::Dimen,
                        index: u32::from(index),
                    }
                );
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &format_scaled(stores.dimen(index)),
                    cause_origin,
                ))
            }
            "toks" => {
                let index = scan_helpers::scan_register_index_with_mode_and_context(
                    input, stores, expansion, mode, token,
                )?;
                crate::record_dependency!(
                    expansion,
                    ReadDependency::Cell {
                        bank: ReadBank::Toks,
                        index: u32::from(index),
                    }
                );
                let token_list = stores.toks(index);
                let origin_list = crate::expansion_suppressed_origin_list(
                    stores,
                    token_list,
                    tex_state::ids::OriginListId::EMPTY,
                    cause_origin,
                );
                Ok(Dispatch::Push {
                    replay_kind: ExpansionReplayKind::Unexpanded,
                    token_list,
                    origin_list,
                    macro_arguments: MacroArguments::new(),
                    macro_invocation: OriginId::UNKNOWN,
                })
            }
            "endlinechar" => {
                crate::record_dependency!(
                    expansion,
                    ReadDependency::Cell {
                        bank: ReadBank::IntParam,
                        index: u32::from(IntParam::END_LINE_CHAR.raw()),
                    }
                );
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &stores.int_param(IntParam::END_LINE_CHAR).to_string(),
                    cause_origin,
                ))
            }
            "escapechar" => {
                crate::record_dependency!(
                    expansion,
                    ReadDependency::Cell {
                        bank: ReadBank::IntParam,
                        index: u32::from(IntParam::ESCAPE_CHAR.raw()),
                    }
                );
                Ok(push_rendered_text(
                    stores,
                    ExpansionReplayKind::TheOutput,
                    &stores.int_param(IntParam::ESCAPE_CHAR).to_string(),
                    cause_origin,
                ))
            }
            _ => Err(ExpandError::UnsupportedTheTarget { context: token }),
        },
    }
}

fn pdf_font_code_table(
    primitive: tex_state::meaning::UnexpandablePrimitive,
) -> tex_state::PdfFontCode {
    use tex_state::meaning::UnexpandablePrimitive as P;
    match primitive {
        P::PdfLpCode => tex_state::PdfFontCode::Lp,
        P::PdfRpCode => tex_state::PdfFontCode::Rp,
        P::PdfEfCode => tex_state::PdfFontCode::Ef,
        P::PdfTagCode => tex_state::PdfFontCode::Tag,
        P::PdfKnbsCode => tex_state::PdfFontCode::Knbs,
        P::PdfStbsCode => tex_state::PdfFontCode::Stbs,
        P::PdfShbsCode => tex_state::PdfFontCode::Shbs,
        P::PdfKnbcCode => tex_state::PdfFontCode::Knbc,
        P::PdfKnacCode => tex_state::PdfFontCode::Knac,
        _ => unreachable!("caller restricts pdfTeX font-code primitive"),
    }
}

pub(crate) fn record_meaning_value_dependency(
    expansion: &mut ExpansionContext<'_>,
    meaning: Meaning,
) {
    let cell = match meaning {
        Meaning::CountRegister(index) => Some((ReadBank::Count, u32::from(index))),
        Meaning::DimenRegister(index) => Some((ReadBank::Dimen, u32::from(index))),
        Meaning::SkipRegister(index) => Some((ReadBank::Skip, u32::from(index))),
        Meaning::MuskipRegister(index) => Some((ReadBank::Muskip, u32::from(index))),
        Meaning::ToksRegister(index) => Some((ReadBank::Toks, u32::from(index))),
        Meaning::IntParam(index) => Some((ReadBank::IntParam, u32::from(index))),
        Meaning::DimenParam(index) => Some((ReadBank::DimenParam, u32::from(index))),
        Meaning::GlueParam(index) | Meaning::MuGlueParam(index) => {
            Some((ReadBank::GlueParam, u32::from(index)))
        }
        Meaning::TokParam(index) => Some((ReadBank::TokParam, u32::from(index))),
        Meaning::InternalInteger(InternalInteger::Badness) => Some((ReadBank::LastBadness, 0)),
        Meaning::InternalInteger(InternalInteger::InputLineNumber) => {
            crate::record_dependency!(expansion, ReadDependency::InputLine);
            None
        }
        Meaning::InternalInteger(InternalInteger::ETeXVersion | InternalInteger::PdfTeXVersion) => {
            None
        }
        Meaning::InternalInteger(InternalInteger::PdfElapsedTime) => {
            crate::record_dependency!(expansion, ReadDependency::Engine(ReadEngineField::PdfTimer));
            None
        }
        Meaning::InternalInteger(InternalInteger::PdfRandomSeed) => {
            crate::record_dependency!(
                expansion,
                ReadDependency::Engine(ReadEngineField::PdfRandom)
            );
            None
        }
        Meaning::InternalInteger(InternalInteger::PdfShellEscape) => {
            crate::record_dependency!(
                expansion,
                ReadDependency::Engine(ReadEngineField::PdfShellEscape)
            );
            None
        }
        Meaning::InternalInteger(InternalInteger::PdfLastObject) => {
            crate::record_dependency!(
                expansion,
                ReadDependency::Engine(ReadEngineField::PdfObjects)
            );
            None
        }
        Meaning::InternalInteger(InternalInteger::PdfLastAnnot)
        | Meaning::InternalInteger(InternalInteger::PdfLastLink) => {
            crate::record_dependency!(
                expansion,
                ReadDependency::Engine(ReadEngineField::PdfObjects)
            );
            None
        }
        Meaning::InternalInteger(InternalInteger::PdfLastXForm) => {
            crate::record_dependency!(expansion, ReadDependency::Engine(ReadEngineField::PdfForms));
            None
        }
        Meaning::InternalInteger(InternalInteger::PdfLastXImage) => {
            crate::record_dependency!(
                expansion,
                ReadDependency::Engine(ReadEngineField::PdfObjects)
            );
            None
        }
        Meaning::InternalInteger(InternalInteger::PdfReturnValue) => {
            crate::record_dependency!(
                expansion,
                ReadDependency::Engine(ReadEngineField::PdfObjects)
            );
            None
        }
        Meaning::InternalInteger(
            InternalInteger::PdfLastXImagePages | InternalInteger::PdfLastXImageColorDepth,
        ) => {
            crate::record_dependency!(
                expansion,
                ReadDependency::Engine(ReadEngineField::PdfObjects)
            );
            None
        }
        Meaning::InternalInteger(InternalInteger::CurrentGroupLevel) => {
            crate::record_dependency!(
                expansion,
                ReadDependency::Engine(ReadEngineField::GroupLevel)
            );
            None
        }
        Meaning::InternalInteger(InternalInteger::CurrentGroupType) => {
            crate::record_dependency!(
                expansion,
                ReadDependency::Engine(ReadEngineField::GroupType)
            );
            None
        }
        Meaning::InternalInteger(
            InternalInteger::CurrentIfLevel
            | InternalInteger::CurrentIfType
            | InternalInteger::CurrentIfBranch,
        ) => {
            crate::record_dependency!(
                expansion,
                ReadDependency::Engine(ReadEngineField::ConditionStack)
            );
            None
        }
        Meaning::InternalInteger(InternalInteger::LastNodeType) => {
            crate::record_dependency!(
                expansion,
                ReadDependency::Engine(ReadEngineField::LastNodeType)
            );
            None
        }
        Meaning::PageDimension(dimension) => {
            crate::record_dependency!(
                expansion,
                ReadDependency::PageDimension(page_dimension_key(dimension))
            );
            None
        }
        Meaning::PageInteger(integer) => {
            crate::record_dependency!(
                expansion,
                ReadDependency::PageInteger(page_integer_key(integer))
            );
            None
        }
        _ => None,
    };
    if let Some((bank, index)) = cell {
        crate::record_dependency!(expansion, ReadDependency::Cell { bank, index });
    }
}

const fn code_table_key(primitive: tex_state::meaning::UnexpandablePrimitive) -> ReadCodeTable {
    use tex_state::meaning::UnexpandablePrimitive as P;
    match primitive {
        P::CatCode => ReadCodeTable::Catcode,
        P::LcCode => ReadCodeTable::Lccode,
        P::UcCode => ReadCodeTable::Uccode,
        P::SfCode => ReadCodeTable::Sfcode,
        P::MathCode => ReadCodeTable::Mathcode,
        P::DelCode => ReadCodeTable::Delcode,
        _ => unreachable!(),
    }
}

const fn math_font_size_key(size: MathFontSize) -> u8 {
    match size {
        MathFontSize::Text => 0,
        MathFontSize::Script => 1,
        MathFontSize::ScriptScript => 2,
    }
}

const fn page_dimension_key(dimension: tex_state::page::PageDimension) -> u8 {
    use tex_state::page::PageDimension as D;
    match dimension {
        D::Goal => 0,
        D::Total => 1,
        D::Stretch => 2,
        D::FilStretch => 3,
        D::FillStretch => 4,
        D::FilllStretch => 5,
        D::Shrink => 6,
        D::Depth => 7,
    }
}

const fn page_integer_key(integer: tex_state::page::PageInteger) -> u8 {
    use tex_state::page::PageInteger as I;
    match integer {
        I::DeadCycles => 0,
        I::InsertPenalties => 1,
    }
}

pub(crate) fn push_rendered_text(
    stores: &mut tex_state::ExpansionContext<'_>,
    replay_kind: ExpansionReplayKind,
    text: &str,
    parent: OriginId,
) -> Dispatch {
    push_rendered_tokens(stores, replay_kind, text_tokens(text), parent)
}

pub(crate) fn push_rendered_tokens<I>(
    stores: &mut tex_state::ExpansionContext<'_>,
    replay_kind: ExpansionReplayKind,
    tokens: I,
    parent: OriginId,
) -> Dispatch
where
    I: IntoIterator<Item = Token>,
{
    let origin = stores.synthesized_origin(SynthesizedOriginKind::ValueRendering, parent);
    Dispatch::PushTransient {
        replay_kind,
        tokens: tokens
            .into_iter()
            .map(|token| TracedTokenWord::pack(token, origin))
            .collect(),
    }
}

pub(crate) fn string_tokens(stores: &impl ExpansionState, token: Token) -> Vec<Token> {
    match token {
        Token::Char { ch, .. } => vec![rendered_char(ch)],
        Token::Cs(symbol) => {
            let name = stores.resolve(symbol);
            let escape = escapechar(stores);
            let kind = stores.control_sequence_kind(symbol);
            let capacity = match kind {
                ControlSequenceKind::ActiveCharacter => name.chars().count(),
                ControlSequenceKind::Named if name.is_empty() => {
                    "csname".len() + "endcsname".len() + 2 * usize::from(escape.is_some())
                }
                ControlSequenceKind::Named => name.chars().count() + usize::from(escape.is_some()),
            };
            let mut out = Vec::with_capacity(capacity);
            match kind {
                ControlSequenceKind::ActiveCharacter => {
                    out.extend(name.chars().map(rendered_char));
                }
                ControlSequenceKind::Named if name.is_empty() => {
                    append_escaped_text(escape, "csname", &mut out);
                    append_escaped_text(escape, "endcsname", &mut out);
                }
                ControlSequenceKind::Named => {
                    append_escaped_text(escape, name, &mut out);
                }
            }
            out
        }
        Token::Param(slot) => text_tokens(&format!("#{slot}")),
        Token::Frozen(_) => text_tokens("\\endtemplate"),
    }
}

fn append_escaped_text(escape: Option<char>, value: &str, out: &mut Vec<Token>) {
    if let Some(escape) = escape {
        out.push(rendered_char(escape));
    }
    out.extend(value.chars().map(rendered_char));
}

pub fn meaning_text(stores: &impl ExpansionState, token: Token) -> String {
    match token {
        Token::Char {
            ch,
            cat: Catcode::Active,
        } => stores.active_character_symbol(ch).map_or_else(
            || "undefined".to_owned(),
            |symbol| meaning_text(stores, Token::Cs(symbol)),
        ),
        Token::Char {
            ch,
            cat: Catcode::Letter,
        } => format!("the letter {ch}"),
        Token::Char { ch, .. } => format!("the character {ch}"),
        Token::Param(slot) => format!("macro parameter character #{slot}"),
        Token::Frozen(_) => "end of alignment template".to_owned(),
        Token::Cs(symbol) => match stores.meaning(symbol) {
            Meaning::Undefined => "undefined".to_owned(),
            Meaning::Relax => "\\relax".to_owned(),
            Meaning::CharGiven(ch) => format!("the character {ch}"),
            Meaning::CharToken {
                ch,
                cat: Catcode::Letter,
            } => format!("the letter {ch}"),
            Meaning::CharToken { ch, .. } => format!("the character {ch}"),
            Meaning::MathCharGiven(value) => format!("\\mathchar\"{value:X}"),
            Meaning::CountRegister(index) => format!("\\count{index}"),
            Meaning::DimenRegister(index) => format!("\\dimen{index}"),
            Meaning::SkipRegister(index) => format!("\\skip{index}"),
            Meaning::MuskipRegister(index) => format!("\\muskip{index}"),
            Meaning::ToksRegister(index) => format!("\\toks{index}"),
            Meaning::IntParam(_)
            | Meaning::InternalInteger(_)
            | Meaning::DimenParam(_)
            | Meaning::GlueParam(_)
            | Meaning::MuGlueParam(_)
            | Meaning::TokParam(_)
            | Meaning::PageDimension(_)
            | Meaning::PageInteger(_) => {
                format!("\\{}", stores.resolve(symbol))
            }
            Meaning::Font(font) => format!("select font {}", stores.font_name(font)),
            Meaning::ExpandablePrimitive(_) => format!("\\{}", stores.resolve(symbol)),
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Radical) => {
                "\\radical".to_owned()
            }
            Meaning::UnexpandablePrimitive(_) => format!("\\{}", stores.resolve(symbol)),
            Meaning::Macro { flags, definition } => {
                let macro_meaning = stores.macro_definition(definition);
                let mut text = String::new();
                if flags.contains(MeaningFlags::PROTECTED) {
                    text.push_str("\\protected");
                }
                if flags.contains(MeaningFlags::LONG) {
                    text.push_str("\\long");
                }
                if flags.contains(MeaningFlags::OUTER) {
                    text.push_str("\\outer");
                }
                if flags.bits()
                    & (MeaningFlags::PROTECTED | MeaningFlags::LONG | MeaningFlags::OUTER).bits()
                    != 0
                {
                    text.push(' ');
                }
                text.push_str("macro:");
                text.push_str(&token_list_text(stores, macro_meaning.parameter_text()));
                text.push_str("->");
                text.push_str(&token_list_text(stores, macro_meaning.replacement_text()));
                text
            }
            Meaning::Unknown(_) => "unknown".to_owned(),
        },
    }
}

fn token_list_text(stores: &impl ExpansionState, token_list: TokenListId) -> String {
    let mut text = String::new();
    for &token in stores.tokens(token_list) {
        append_token_show_text(stores, token, &mut text);
    }
    text
}

/// Appends the form TeX82's `show_token_list` prints for one token.
///
/// In `tex.web` section 262, `print_cs` always terminates hash-table control
/// sequence names with a space. Direct-address single-character names only
/// receive that space when the character's current catcode is `letter`, and
/// active characters receive neither an escape nor a trailing space.
pub fn append_token_show_text(stores: &impl ExpansionState, token: Token, text: &mut String) {
    if let Token::Char { ch, cat } = token {
        append_tex_print_char(ch, text);
        if cat == Catcode::Parameter {
            append_tex_print_char(ch, text);
        }
    } else {
        text.push_str(&token_text(stores, token));
    }
    let Token::Cs(symbol) = token else {
        return;
    };
    if stores.control_sequence_kind(symbol) == ControlSequenceKind::ActiveCharacter {
        return;
    }

    let name = stores.resolve(symbol);
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) if stores.catcode(ch) != Catcode::Letter => {}
        _ => text.push(' '),
    }
}

/// Appends the token text TeX builds with `selector = new_string`.
///
/// Unlike ordinary diagnostic display, character tokens remain raw; control
/// sequence spelling and its separator still follow `show_token_list`.
pub fn append_token_string_text(stores: &impl ExpansionState, token: Token, text: &mut String) {
    if let Token::Char { ch, cat } = token {
        text.push(ch);
        if cat == Catcode::Parameter {
            text.push(ch);
        }
    } else {
        append_token_show_text(stores, token, text);
    }
}

/// Appends TeX82's printable string for a character code.
///
/// `show_token_list` calls `print(c)`, not `print_char(c)`. The first 256
/// TeX strings therefore render non-printable bytes as `^^A`, `^^?`, or
/// lowercase hexadecimal `^^80` forms (tex.web sections 49 and 262).
fn append_tex_print_char(ch: char, text: &mut String) {
    let code = ch as u32;
    match code {
        0..=31 => {
            text.push_str("^^");
            text.push(char::from_u32(code + 64).expect("ASCII control marker"));
        }
        32..=126 => text.push(ch),
        127 => text.push_str("^^?"),
        128..=255 => {
            use std::fmt::Write as _;
            let _ = write!(text, "^^{code:02x}");
        }
        _ => text.push(ch),
    }
}

pub fn token_text(stores: &impl ExpansionState, token: Token) -> String {
    string_tokens(stores, token)
        .into_iter()
        .filter_map(|token| match token {
            Token::Char { ch, .. } => Some(ch),
            Token::Cs(_) | Token::Param(_) | Token::Frozen(_) => None,
        })
        .collect()
}

pub fn scan_the_text_with_context(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    context: TracedTokenWord,
) -> Result<String, ExpandError> {
    let dispatch = expand_the_with_mode_and_context(
        input,
        stores,
        expansion,
        &mut RestrictedExpansionMode,
        context,
    )?;
    Ok(match dispatch {
        Dispatch::Push { token_list, .. } => token_list_text(stores, token_list),
        Dispatch::PushTransient { tokens, .. } => tokens
            .into_iter()
            .map(|word| token_text(stores, crate::semantic_token(word)))
            .collect(),
        Dispatch::Deliver(token) | Dispatch::DeliverNoExpand(token) => {
            token_text(stores, crate::semantic_token(token))
        }
        Dispatch::Continue => String::new(),
    })
}

fn text_tokens(text: &str) -> Vec<Token> {
    text.chars().map(rendered_char).collect()
}

fn rendered_char(ch: char) -> Token {
    Token::Char {
        ch,
        cat: if ch == ' ' {
            Catcode::Space
        } else {
            Catcode::Other
        },
    }
}

fn escapechar(stores: &impl ExpansionState) -> Option<char> {
    u32::try_from(stores.int_param(IntParam::ESCAPE_CHAR))
        .ok()
        .filter(|&value| value < 256)
        .and_then(char::from_u32)
}

pub(crate) fn roman_numeral(value: i32) -> String {
    if value <= 0 {
        return String::new();
    }
    let mut value = value;
    let mut out = String::new();
    for (amount, text) in [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ] {
        while value >= amount {
            out.push_str(text);
            value -= amount;
        }
    }
    out
}

pub(crate) fn format_scaled(value: Scaled) -> String {
    let mut raw = i64::from(value.raw());
    let mut out = String::new();
    if raw < 0 {
        out.push('-');
        raw = -raw;
    }
    let unity = i64::from(Scaled::UNITY);
    out.push_str(&(raw / unity).to_string());
    out.push('.');
    let mut scaled = 10 * (raw % unity) + 5;
    let mut delta = 10;
    loop {
        if delta > unity {
            scaled += 0o100000 - 50_000;
        }
        out.push(char::from(
            b'0' + u8::try_from(scaled / unity).expect("scaled digit fits u8"),
        ));
        scaled = 10 * (scaled % unity);
        delta *= 10;
        if scaled <= delta {
            break;
        }
    }
    out.push_str("pt");
    out
}

fn format_glue(spec: GlueSpec) -> String {
    format_glue_with_unit(spec, "pt")
}

fn format_muglue(spec: GlueSpec) -> String {
    format_glue_with_unit(spec, "mu")
}

fn format_glue_with_unit(spec: GlueSpec, unit: &str) -> String {
    let mut text = format_scaled(spec.width);
    replace_unit(&mut text, unit);
    if spec.stretch.raw() != 0 {
        text.push_str(" plus ");
        text.push_str(&format_scaled_without_unit(spec.stretch, unit));
        text.push_str(component_unit(spec.stretch_order, unit));
    }
    if spec.shrink.raw() != 0 {
        text.push_str(" minus ");
        text.push_str(&format_scaled_without_unit(spec.shrink, unit));
        text.push_str(component_unit(spec.shrink_order, unit));
    }
    text
}

fn format_scaled_without_unit(value: Scaled, unit: &str) -> String {
    let mut text = format_scaled(value);
    replace_unit(&mut text, unit);
    text.trim_end_matches(unit).to_owned()
}

fn replace_unit(text: &mut String, unit: &str) {
    if unit != "pt" {
        text.truncate(text.len() - "pt".len());
        text.push_str(unit);
    }
}

fn component_unit(order: Order, normal_unit: &str) -> &'static str {
    match order {
        Order::Normal if normal_unit == "mu" => "mu",
        Order::Normal => "pt",
        Order::Fil => "fil",
        Order::Fill => "fill",
        Order::Filll => "filll",
    }
}

fn scan_code_table_char(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
) -> Result<char, ExpandError>
where
{
    let value =
        scan_int::scan_int_with_mode_and_context(input, stores, expansion, mode, context)?.value();
    u32::try_from(value)
        .ok()
        .and_then(char::from_u32)
        .ok_or(ExpandError::UnsupportedTheTarget { context })
}

pub(crate) fn scan_font_dimen(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
) -> Result<tex_state::scaled::Scaled, ExpandError> {
    let number =
        scan_int::scan_int_with_mode_and_context(input, stores, expansion, mode, context)?.value();
    let font = scan_font_selector(input, stores, expansion, mode, context)?;
    crate::record_dependency!(
        expansion,
        ReadDependency::Font {
            field: ReadFontField::ParameterCount,
            font: font.raw(),
            index: 0,
        }
    );
    let available = stores.font_parameter_count(font);
    let number = u32::try_from(number)
        .ok()
        .filter(|number| *number > 0 && *number <= available);
    // TeX.web §578 diagnoses an unavailable parameter but points at its
    // zero-valued dummy font-info cell, so callers still receive a dimension.
    Ok(if let Some(number) = number {
        crate::record_dependency!(
            expansion,
            ReadDependency::Font {
                field: ReadFontField::Parameter,
                font: font.raw(),
                index: number,
            }
        );
        stores.font_dimen(font, number)
    } else {
        tex_state::scaled::Scaled::from_raw(0)
    })
}

pub(crate) fn scan_font_selector(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
) -> Result<FontId, ExpandError>
where
{
    let Some(token) =
        scan_helpers::next_non_space_x_token_with_mode_and_context(input, stores, expansion, mode)?
    else {
        return Err(ExpandError::MissingTokenAfterPrimitive {
            opcode: ExpandableOpcode::FontName,
            context,
        });
    };
    let semantic = crate::semantic_token(token);
    let Token::Cs(symbol) = semantic else {
        return Err(ExpandError::UnsupportedTheTarget { context: token });
    };
    let meaning = stores.meaning(symbol);
    expansion.record_meaning(symbol, meaning);
    match meaning {
        Meaning::Font(id) => Ok(id),
        Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Font) => {
            crate::record_dependency!(
                expansion,
                ReadDependency::Cell {
                    bank: ReadBank::CurrentFont,
                    index: 0,
                }
            );
            Ok(stores.current_font())
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (tex_state::meaning::UnexpandablePrimitive::TextFont
            | tex_state::meaning::UnexpandablePrimitive::ScriptFont
            | tex_state::meaning::UnexpandablePrimitive::ScriptScriptFont),
        ) => {
            let family = scan_math_family(input, stores, expansion, mode, token)?;
            let size = math_font_size(primitive);
            crate::record_dependency!(
                expansion,
                ReadDependency::Cell {
                    bank: ReadBank::MathFamilyFont,
                    index: u32::from(family) + 16 * u32::from(math_font_size_key(size)),
                }
            );
            Ok(stores.math_family_font(size, family))
        }
        _ => {
            // TeX.web's `scan_font_ident` uses `back_error`: the offending
            // token remains available to the following scanner, and the null
            // font supplies the recovered value.
            crate::back_input(input, stores, [token]);
            stores.report_missing_font_identifier();
            Ok(tex_state::font::NULL_FONT)
        }
    }
}

pub(crate) fn scan_font_char_dimension(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
    primitive: tex_state::meaning::UnexpandablePrimitive,
) -> Result<Scaled, ExpandError>
where
{
    let font = scan_font_selector(input, stores, expansion, mode, context)?;
    let code =
        scan_int::scan_int_with_mode_and_context(input, stores, expansion, mode, context)?.value();
    crate::record_dependency!(
        expansion,
        ReadDependency::Font {
            field: ReadFontField::Metrics,
            font: font.raw(),
            index: u32::try_from(code).unwrap_or(u32::MAX),
        }
    );
    let Some(metrics) = u8::try_from(code)
        .ok()
        .and_then(|code| stores.font_char_metrics(font, code))
    else {
        return Ok(Scaled::from_raw(0));
    };
    Ok(match primitive {
        tex_state::meaning::UnexpandablePrimitive::FontCharWd => metrics.width,
        tex_state::meaning::UnexpandablePrimitive::FontCharHt => metrics.height,
        tex_state::meaning::UnexpandablePrimitive::FontCharDp => metrics.depth,
        tex_state::meaning::UnexpandablePrimitive::FontCharIc => metrics.italic_correction,
        _ => unreachable!("caller restricts font character dimension primitive"),
    })
}

pub(crate) fn scan_parshape_dimension(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
    primitive: tex_state::meaning::UnexpandablePrimitive,
) -> Result<Scaled, ExpandError>
where
{
    let number =
        scan_int::scan_int_with_mode_and_context(input, stores, expansion, mode, context)?.value();
    crate::record_dependency!(expansion, ReadDependency::Engine(ReadEngineField::ParShape));
    let (line, width) = match primitive {
        tex_state::meaning::UnexpandablePrimitive::ParShapeLength => (number, true),
        tex_state::meaning::UnexpandablePrimitive::ParShapeIndent => (number, false),
        tex_state::meaning::UnexpandablePrimitive::ParShapeDimen if number > 0 => {
            ((number + 1) / 2, number % 2 == 0)
        }
        tex_state::meaning::UnexpandablePrimitive::ParShapeDimen => (0, false),
        _ => unreachable!("caller restricts primitive"),
    };
    Ok(stores.paragraph_shape_dimension(line, width))
}

fn scan_math_family(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
) -> Result<u8, ExpandError>
where
{
    let scanned =
        scan_int::scan_int_with_mode_and_context(input, stores, expansion, mode, context)?;
    Ok(u8::try_from(scanned.value())
        .ok()
        .filter(|family| *family < 16)
        .unwrap_or(0))
}

fn math_font_size(primitive: tex_state::meaning::UnexpandablePrimitive) -> MathFontSize {
    match primitive {
        tex_state::meaning::UnexpandablePrimitive::TextFont => MathFontSize::Text,
        tex_state::meaning::UnexpandablePrimitive::ScriptFont => MathFontSize::Script,
        tex_state::meaning::UnexpandablePrimitive::ScriptScriptFont => MathFontSize::ScriptScript,
        _ => unreachable!("caller restricts math font primitive"),
    }
}
