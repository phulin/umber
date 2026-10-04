//! Compile-time keys over the native packed meaning encoding.
use super::*;

#[inline(always)]
pub(super) fn of<G>(meaning: &tex_state::meaning::MeaningWord<G>) -> u64 {
    meaning.control_code()
}

pub(super) const CHAR_BEGIN_GROUP: u64 =
    Meaning::runtime_word_control_code(Meaning::char_token_word('\0', Catcode::BeginGroup));
pub(super) const PRIMITIVE_BEGIN_GROUP: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::BeginGroup).encode(),
);
pub(super) const PRIMITIVE_END_GROUP: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::EndGroup).encode(),
);
pub(super) const PRIMITIVE_CAT_CODE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::CatCode).encode(),
);
pub(super) const PRIMITIVE_DEF: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Def).encode(),
);
pub(super) const PRIMITIVE_EDEF: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Edef).encode(),
);
pub(super) const PRIMITIVE_GDEF: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Gdef).encode(),
);
pub(super) const PRIMITIVE_XDEF: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Xdef).encode(),
);
pub(super) const PRIMITIVE_LET: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Let).encode(),
);
pub(super) const PRIMITIVE_FUTURE_LET: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::FutureLet).encode(),
);
pub(super) const RELAX: u64 = Meaning::runtime_word_control_code(Meaning::Relax.encode());
pub(super) const CHAR_END_GROUP: u64 =
    Meaning::runtime_word_control_code(Meaning::char_token_word('\0', Catcode::EndGroup));
pub(super) const PRIMITIVE_END: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::End).encode(),
);
pub(super) const PRIMITIVE_DUMP: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dump).encode(),
);
pub(super) const PRIMITIVE_COUNT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Count).encode(),
);
pub(super) const COUNT_REGISTER: u64 =
    Meaning::runtime_word_control_code(Meaning::CountRegister(0).encode());
pub(super) const PRIMITIVE_DIMEN: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dimen).encode(),
);
pub(super) const PRIMITIVE_WD: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Wd).encode(),
);
pub(super) const PRIMITIVE_HT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Ht).encode(),
);
pub(super) const PRIMITIVE_DP: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dp).encode(),
);
pub(super) const DIMEN_REGISTER: u64 =
    Meaning::runtime_word_control_code(Meaning::DimenRegister(0).encode());
pub(super) const PRIMITIVE_SKIP: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Skip).encode(),
);
pub(super) const SKIP_REGISTER: u64 =
    Meaning::runtime_word_control_code(Meaning::SkipRegister(0).encode());
pub(super) const PRIMITIVE_MUSKIP: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Muskip).encode(),
);
pub(super) const MUSKIP_REGISTER: u64 =
    Meaning::runtime_word_control_code(Meaning::MuskipRegister(0).encode());
pub(super) const PRIMITIVE_H_SKIP: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HSkip).encode(),
);
pub(super) const PRIMITIVE_KERN: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Kern).encode(),
);
pub(super) const PRIMITIVE_PENALTY: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Penalty).encode(),
);
pub(super) const PRIMITIVE_CONTROL_SPACE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ControlSpace).encode(),
);
pub(super) const PRIMITIVE_PREV_DEPTH: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PrevDepth).encode(),
);
pub(super) const PRIMITIVE_BATCH_MODE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::BatchMode).encode(),
);
pub(super) const PRIMITIVE_NONSTOP_MODE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::NonstopMode).encode(),
);
pub(super) const PRIMITIVE_SCROLL_MODE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ScrollMode).encode(),
);
pub(super) const PRIMITIVE_ERROR_STOP_MODE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ErrorStopMode).encode(),
);
pub(super) const PRIMITIVE_INTERACTION_MODE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::InteractionMode).encode(),
);
pub(super) const PRIMITIVE_SPACE_FACTOR: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SpaceFactor).encode(),
);
pub(super) const PRIMITIVE_PREV_GRAF: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PrevGraf).encode(),
);
pub(super) const PRIMITIVE_CHAR: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char).encode(),
);
pub(super) const PRIMITIVE_ACCENT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Accent).encode(),
);
pub(super) const PRIMITIVE_DISCRETIONARY: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Discretionary).encode(),
);
pub(super) const PRIMITIVE_DISCRETIONARY_HYPHEN: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::DiscretionaryHyphen).encode(),
);
pub(super) const PRIMITIVE_H_FIL: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HFil).encode(),
);
pub(super) const PRIMITIVE_H_FILL: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HFill).encode(),
);
pub(super) const PRIMITIVE_H_SS: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HSs).encode(),
);
pub(super) const PRIMITIVE_H_FIL_NEG: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HFilNeg).encode(),
);
pub(super) const PRIMITIVE_V_SKIP: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VSkip).encode(),
);
pub(super) const PRIMITIVE_V_FIL: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VFil).encode(),
);
pub(super) const PRIMITIVE_V_FILL: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VFill).encode(),
);
pub(super) const PRIMITIVE_V_SS: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VSs).encode(),
);
pub(super) const PRIMITIVE_V_FIL_NEG: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VFilNeg).encode(),
);
pub(super) const PRIMITIVE_INDENT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Indent).encode(),
);
pub(super) const PRIMITIVE_NO_INDENT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::NoIndent).encode(),
);
pub(super) const PRIMITIVE_QUIT_V_MODE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::QuitVMode).encode(),
);
pub(super) const PRIMITIVE_PAR_SHAPE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ParShape).encode(),
);
pub(super) const PRIMITIVE_INTER_LINE_PENALTIES: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::InterLinePenalties).encode(),
);
pub(super) const PRIMITIVE_CLUB_PENALTIES: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ClubPenalties).encode(),
);
pub(super) const PRIMITIVE_WIDOW_PENALTIES: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::WidowPenalties).encode(),
);
pub(super) const PRIMITIVE_DISPLAY_WIDOW_PENALTIES: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::DisplayWidowPenalties).encode(),
);
pub(super) const PRIMITIVE_TOKS: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Toks).encode(),
);
pub(super) const TOKS_REGISTER: u64 =
    Meaning::runtime_word_control_code(Meaning::ToksRegister(0).encode());
pub(super) const INT_PARAM: u64 = Meaning::runtime_word_control_code(Meaning::IntParam(0).encode());
pub(super) const DIMEN_PARAM: u64 =
    Meaning::runtime_word_control_code(Meaning::DimenParam(0).encode());
pub(super) const PAGE_DIMENSION: u64 =
    Meaning::runtime_word_control_code(Meaning::PageDimension(PageDimension::Goal).encode());
pub(super) const PAGE_INTEGER: u64 =
    Meaning::runtime_word_control_code(Meaning::PageInteger(PageInteger::DeadCycles).encode());
pub(super) const TOK_PARAM: u64 = Meaning::runtime_word_control_code(Meaning::TokParam(0).encode());
pub(super) const GLUE_PARAM: u64 =
    Meaning::runtime_word_control_code(Meaning::GlueParam(0).encode());
pub(super) const MU_GLUE_PARAM: u64 =
    Meaning::runtime_word_control_code(Meaning::MuGlueParam(0).encode());
pub(super) const PRIMITIVE_OPEN_IN: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::OpenIn).encode(),
);
pub(super) const PRIMITIVE_CLOSE_IN: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::CloseIn).encode(),
);
pub(super) const PRIMITIVE_READ: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Read).encode(),
);
pub(super) const PRIMITIVE_READ_LINE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ReadLine).encode(),
);
pub(super) const PRIMITIVE_FONT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Font).encode(),
);
pub(super) const PRIMITIVE_PDF_COPY_FONT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfCopyFont).encode(),
);
pub(super) const PRIMITIVE_LETTERSPACE_FONT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::LetterspaceFont).encode(),
);
pub(super) const PRIMITIVE_PDF_X_IMAGE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfXImage).encode(),
);
pub(super) const PRIMITIVE_PDF_REF_X_IMAGE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfRefXImage).encode(),
);
pub(super) const PRIMITIVE_PDF_SET_RANDOM_SEED: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfSetRandomSeed).encode(),
);
pub(super) const PRIMITIVE_PDF_RESET_TIMER: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfResetTimer).encode(),
);
pub(super) const PRIMITIVE_PDF_INTERWORD_SPACE_ON: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfInterwordSpaceOn).encode(),
);
pub(super) const PRIMITIVE_PDF_INTERWORD_SPACE_OFF: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfInterwordSpaceOff).encode(),
);
pub(super) const PRIMITIVE_PDF_FAKE_SPACE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfFakeSpace).encode(),
);
pub(super) const PRIMITIVE_PDF_RUNNING_LINK_ON: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfRunningLinkOn).encode(),
);
pub(super) const PRIMITIVE_PDF_RUNNING_LINK_OFF: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfRunningLinkOff).encode(),
);
pub(super) const PRIMITIVE_PDF_SPACE_FONT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfSpaceFont).encode(),
);
pub(super) const PRIMITIVE_PDF_OBJECT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfObject).encode(),
);
pub(super) const PRIMITIVE_PDF_REFERENCE_OBJECT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfReferenceObject).encode(),
);
pub(super) const PRIMITIVE_PDF_X_FORM: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfXForm).encode(),
);
pub(super) const PRIMITIVE_PDF_REF_X_FORM: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfRefXForm).encode(),
);
pub(super) const PRIMITIVE_PDF_INFO: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfInfo).encode(),
);
pub(super) const PRIMITIVE_PDF_CATALOG: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfCatalog).encode(),
);
pub(super) const PRIMITIVE_PDF_NAMES: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfNames).encode(),
);
pub(super) const PRIMITIVE_PDF_TRAILER: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfTrailer).encode(),
);
pub(super) const PRIMITIVE_PDF_TRAILER_ID: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfTrailerId).encode(),
);
pub(super) const PRIMITIVE_PDF_LITERAL: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfLiteral).encode(),
);
pub(super) const PRIMITIVE_PDF_SET_MATRIX: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfSetMatrix).encode(),
);
pub(super) const PRIMITIVE_PDF_SAVE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfSave).encode(),
);
pub(super) const PRIMITIVE_PDF_RESTORE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfRestore).encode(),
);
pub(super) const PRIMITIVE_PDF_COLOR_STACK: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfColorStack).encode(),
);
pub(super) const PRIMITIVE_PDF_SAVE_POS: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfSavePos).encode(),
);
pub(super) const PRIMITIVE_PDF_SNAP_REF_POINT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfSnapRefPoint).encode(),
);
pub(super) const PRIMITIVE_PDF_SNAP_Y: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfSnapY).encode(),
);
pub(super) const PRIMITIVE_PDF_SNAP_Y_COMP: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfSnapYComp).encode(),
);
pub(super) const PRIMITIVE_PDF_ANNOT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfAnnot).encode(),
);
pub(super) const PRIMITIVE_PDF_START_LINK: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfStartLink).encode(),
);
pub(super) const PRIMITIVE_PDF_END_LINK: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfEndLink).encode(),
);
pub(super) const PRIMITIVE_PDF_OUTLINE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfOutline).encode(),
);
pub(super) const PRIMITIVE_PDF_DEST: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfDest).encode(),
);
pub(super) const PRIMITIVE_PDF_THREAD: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfThread).encode(),
);
pub(super) const PRIMITIVE_PDF_START_THREAD: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfStartThread).encode(),
);
pub(super) const PRIMITIVE_PDF_END_THREAD: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfEndThread).encode(),
);
pub(super) const FONT: u64 =
    Meaning::runtime_word_control_code(Meaning::Font(tex_state::font::NULL_FONT).encode());
pub(super) const PRIMITIVE_FONT_DIMEN: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::FontDimen).encode(),
);
pub(super) const PRIMITIVE_HYPHEN_CHAR: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HyphenChar).encode(),
);
pub(super) const PRIMITIVE_SKEW_CHAR: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SkewChar).encode(),
);
pub(super) const PRIMITIVE_OPEN_OUT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::OpenOut).encode(),
);
pub(super) const PRIMITIVE_CLOSE_OUT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::CloseOut).encode(),
);
pub(super) const PRIMITIVE_PDF_FONT_EXPAND: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfFontExpand).encode(),
);
pub(super) const PRIMITIVE_PDF_FONT_ATTR: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfFontAttr).encode(),
);
pub(super) const PRIMITIVE_PDF_INCLUDE_CHARS: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfIncludeChars).encode(),
);
pub(super) const PRIMITIVE_PDF_MAP_FILE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfMapFile).encode(),
);
pub(super) const PRIMITIVE_PDF_MAP_LINE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfMapLine).encode(),
);
pub(super) const PRIMITIVE_PDF_GLYPH_TO_UNICODE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfGlyphToUnicode).encode(),
);
pub(super) const PRIMITIVE_PDF_NO_BUILTIN_TO_UNICODE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfNoBuiltinToUnicode).encode(),
);
pub(super) const PRIMITIVE_WRITE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Write).encode(),
);
pub(super) const PRIMITIVE_SPECIAL: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Special).encode(),
);
pub(super) const PRIMITIVE_SET_LANGUAGE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SetLanguage).encode(),
);
pub(super) const PRIMITIVE_LC_CODE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::LcCode).encode(),
);
pub(super) const PRIMITIVE_UC_CODE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::UcCode).encode(),
);
pub(super) const PRIMITIVE_SF_CODE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SfCode).encode(),
);
pub(super) const PRIMITIVE_MATH_CODE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::MathCode).encode(),
);
pub(super) const PRIMITIVE_DEL_CODE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::DelCode).encode(),
);
pub(super) const PRIMITIVE_PDF_LP_CODE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfLpCode).encode(),
);
pub(super) const PRIMITIVE_PDF_RP_CODE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfRpCode).encode(),
);
pub(super) const PRIMITIVE_PDF_EF_CODE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfEfCode).encode(),
);
pub(super) const PRIMITIVE_PDF_TAG_CODE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfTagCode).encode(),
);
pub(super) const PRIMITIVE_PDF_KNBS_CODE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfKnbsCode).encode(),
);
pub(super) const PRIMITIVE_PDF_STBS_CODE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfStbsCode).encode(),
);
pub(super) const PRIMITIVE_PDF_SHBS_CODE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfShbsCode).encode(),
);
pub(super) const PRIMITIVE_PDF_KNBC_CODE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfKnbcCode).encode(),
);
pub(super) const PRIMITIVE_PDF_KNAC_CODE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfKnacCode).encode(),
);
pub(super) const PRIMITIVE_PDF_NO_LIGATURES: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfNoLigatures).encode(),
);
pub(super) const PRIMITIVE_ADVANCE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Advance).encode(),
);
pub(super) const PRIMITIVE_MULTIPLY: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Multiply).encode(),
);
pub(super) const PRIMITIVE_DIVIDE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Divide).encode(),
);
pub(super) const PRIMITIVE_CHAR_DEF: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::CharDef).encode(),
);
pub(super) const PRIMITIVE_MATH_CHAR_DEF: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::MathCharDef).encode(),
);
pub(super) const PRIMITIVE_COUNT_DEF: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::CountDef).encode(),
);
pub(super) const PRIMITIVE_DIMEN_DEF: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::DimenDef).encode(),
);
pub(super) const PRIMITIVE_SKIP_DEF: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SkipDef).encode(),
);
pub(super) const PRIMITIVE_MUSKIP_DEF: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::MuskipDef).encode(),
);
pub(super) const PRIMITIVE_TOKS_DEF: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ToksDef).encode(),
);
pub(super) const PRIMITIVE_UPPERCASE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Uppercase).encode(),
);
pub(super) const PRIMITIVE_LOWERCASE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Lowercase).encode(),
);
pub(super) const PRIMITIVE_AFTER_GROUP: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::AfterGroup).encode(),
);
pub(super) const PRIMITIVE_AFTER_ASSIGNMENT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::AfterAssignment).encode(),
);
pub(super) const PRIMITIVE_MESSAGE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Message).encode(),
);
pub(super) const PRIMITIVE_ERR_MESSAGE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ErrMessage).encode(),
);
pub(super) const PRIMITIVE_SHOW: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Show).encode(),
);
pub(super) const PRIMITIVE_SHOW_THE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ShowThe).encode(),
);
pub(super) const PRIMITIVE_SHOW_TOKENS: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ShowTokens).encode(),
);
pub(super) const PRIMITIVE_SHOW_IFS: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ShowIfs).encode(),
);
pub(super) const PRIMITIVE_SHOW_LISTS: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ShowLists).encode(),
);
pub(super) const PRIMITIVE_SHOW_GROUPS: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ShowGroups).encode(),
);
pub(super) const PRIMITIVE_SHOW_BOX: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ShowBox).encode(),
);
pub(super) const PRIMITIVE_IMMEDIATE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Immediate).encode(),
);
pub(super) const PRIMITIVE_H_RULE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HRule).encode(),
);
pub(super) const PRIMITIVE_V_RULE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VRule).encode(),
);
pub(super) const PRIMITIVE_SET_BOX: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SetBox).encode(),
);
pub(super) const PRIMITIVE_V_SPLIT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VSplit).encode(),
);
pub(super) const PRIMITIVE_BOX: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Box).encode(),
);
pub(super) const PRIMITIVE_COPY: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Copy).encode(),
);
pub(super) const PRIMITIVE_UN_V_BOX: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::UnVBox).encode(),
);
pub(super) const PRIMITIVE_UN_V_COPY: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::UnVCopy).encode(),
);
pub(super) const PRIMITIVE_PAGE_DISCARDS: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PageDiscards).encode(),
);
pub(super) const PRIMITIVE_SPLIT_DISCARDS: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SplitDiscards).encode(),
);
pub(super) const PRIMITIVE_UN_H_BOX: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::UnHBox).encode(),
);
pub(super) const PRIMITIVE_UN_H_COPY: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::UnHCopy).encode(),
);
pub(super) const PRIMITIVE_LAST_BOX: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::LastBox).encode(),
);
pub(super) const PRIMITIVE_RAISE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Raise).encode(),
);
pub(super) const PRIMITIVE_LOWER: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Lower).encode(),
);
pub(super) const PRIMITIVE_MOVE_LEFT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::MoveLeft).encode(),
);
pub(super) const PRIMITIVE_MOVE_RIGHT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::MoveRight).encode(),
);
pub(super) const PRIMITIVE_LEADERS: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Leaders).encode(),
);
pub(super) const PRIMITIVE_C_LEADERS: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::CLeaders).encode(),
);
pub(super) const PRIMITIVE_X_LEADERS: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::XLeaders).encode(),
);
pub(super) const PRIMITIVE_SHIPOUT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Shipout).encode(),
);
pub(super) const PRIMITIVE_H_BOX: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HBox).encode(),
);
pub(super) const PRIMITIVE_V_BOX: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VBox).encode(),
);
pub(super) const PRIMITIVE_V_TOP: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VTop).encode(),
);
pub(super) const PRIMITIVE_V_CENTER: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VCenter).encode(),
);
pub(super) const PRIMITIVE_INSERT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Insert).encode(),
);
pub(super) const PRIMITIVE_V_ADJUST: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VAdjust).encode(),
);
pub(super) const PRIMITIVE_MARK: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Mark).encode(),
);
pub(super) const PRIMITIVE_MARKS: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Marks).encode(),
);
pub(super) const PRIMITIVE_H_ALIGN: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HAlign).encode(),
);
pub(super) const PRIMITIVE_V_ALIGN: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VAlign).encode(),
);
pub(super) const PRIMITIVE_PAR: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par).encode(),
);
pub(super) const CHAR_MATH_SHIFT: u64 =
    Meaning::runtime_word_control_code(Meaning::char_token_word('\0', Catcode::MathShift));
pub(super) const CHAR_LETTER: u64 =
    Meaning::runtime_word_control_code(Meaning::char_token_word('\0', Catcode::Letter));
pub(super) const CHAR_OTHER: u64 =
    Meaning::runtime_word_control_code(Meaning::char_token_word('\0', Catcode::Other));
pub(super) const CHAR_SPACE: u64 =
    Meaning::runtime_word_control_code(Meaning::char_token_word('\0', Catcode::Space));
pub(super) const PRIMITIVE_UN_PENALTY: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::UnPenalty).encode(),
);
pub(super) const PRIMITIVE_UN_KERN: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::UnKern).encode(),
);
pub(super) const PRIMITIVE_UN_SKIP: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::UnSkip).encode(),
);
pub(super) const PRIMITIVE_ITALIC_CORRECTION: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ItalicCorrection).encode(),
);
pub(super) const PRIMITIVE_NO_BOUNDARY: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::NoBoundary).encode(),
);
pub(super) const PRIMITIVE_NON_SCRIPT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::NonScript).encode(),
);
pub(super) const PRIMITIVE_PATTERNS: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Patterns).encode(),
);
pub(super) const PRIMITIVE_HYPHENATION: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Hyphenation).encode(),
);
pub(super) const PRIMITIVE_GLOBAL: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Global).encode(),
);

pub(super) const PRIMITIVE_LONG: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Long).encode(),
);

pub(super) const PRIMITIVE_OUTER: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Outer).encode(),
);

pub(super) const PRIMITIVE_PROTECTED: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Protected).encode(),
);

pub(super) const PRIMITIVE_IGNORE_SPACES: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::IgnoreSpaces).encode(),
);

pub(super) const PRIMITIVE_TEXT_FONT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::TextFont).encode(),
);

pub(super) const PRIMITIVE_SCRIPT_FONT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ScriptFont).encode(),
);

pub(super) const PRIMITIVE_SCRIPT_SCRIPT_FONT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ScriptScriptFont).encode(),
);

pub(super) const PRIMITIVE_BEGIN_L: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::BeginL).encode(),
);

pub(super) const PRIMITIVE_END_L: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::EndL).encode(),
);

pub(super) const PRIMITIVE_BEGIN_R: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::BeginR).encode(),
);

pub(super) const PRIMITIVE_END_R: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::EndR).encode(),
);

pub(super) const CHAR_GIVEN: u64 =
    Meaning::runtime_word_control_code(Meaning::CharGiven('\0').encode());

pub(super) const PRIMITIVE_DISPLAY_STYLE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::DisplayStyle).encode(),
);
pub(super) const PRIMITIVE_TEXT_STYLE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::TextStyle).encode(),
);
pub(super) const PRIMITIVE_SCRIPT_STYLE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ScriptStyle).encode(),
);
pub(super) const PRIMITIVE_SCRIPT_SCRIPT_STYLE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ScriptScriptStyle).encode(),
);

pub(super) const MATH_GIVEN: u64 =
    Meaning::runtime_word_control_code(Meaning::MathCharGiven(0).encode());

pub(super) const PRIMITIVE_MATH_CHAR: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::MathChar).encode(),
);
pub(super) const PRIMITIVE_DELIMITER: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Delimiter).encode(),
);

pub(super) const CHAR_SUPERSCRIPT: u64 =
    Meaning::runtime_word_control_code(Meaning::char_token_word('\0', Catcode::Superscript));
pub(super) const CHAR_SUBSCRIPT: u64 =
    Meaning::runtime_word_control_code(Meaning::char_token_word('\0', Catcode::Subscript));

pub(super) const PRIMITIVE_MATH_ORD: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::MathOrd).encode(),
);

pub(super) const PRIMITIVE_MATH_OP: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::MathOp).encode(),
);

pub(super) const PRIMITIVE_MATH_BIN: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::MathBin).encode(),
);

pub(super) const PRIMITIVE_MATH_REL: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::MathRel).encode(),
);

pub(super) const PRIMITIVE_MATH_OPEN: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::MathOpen).encode(),
);

pub(super) const PRIMITIVE_MATH_CLOSE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::MathClose).encode(),
);

pub(super) const PRIMITIVE_MATH_PUNCT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::MathPunct).encode(),
);

pub(super) const PRIMITIVE_MATH_INNER: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::MathInner).encode(),
);

pub(super) const PRIMITIVE_UNDERLINE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Underline).encode(),
);

pub(super) const PRIMITIVE_OVERLINE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Overline).encode(),
);

pub(super) const PRIMITIVE_LIMITS: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Limits).encode(),
);

pub(super) const PRIMITIVE_NO_LIMITS: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::NoLimits).encode(),
);

pub(super) const PRIMITIVE_DISPLAY_LIMITS: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::DisplayLimits).encode(),
);

pub(super) const PRIMITIVE_OVER: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Over).encode(),
);

pub(super) const PRIMITIVE_ATOP: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Atop).encode(),
);

pub(super) const PRIMITIVE_ABOVE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Above).encode(),
);

pub(super) const PRIMITIVE_OVER_WITH_DELIMS: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::OverWithDelims).encode(),
);

pub(super) const PRIMITIVE_ATOP_WITH_DELIMS: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::AtopWithDelims).encode(),
);

pub(super) const PRIMITIVE_ABOVE_WITH_DELIMS: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::AboveWithDelims).encode(),
);

pub(super) const PRIMITIVE_LEFT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Left).encode(),
);

pub(super) const PRIMITIVE_RIGHT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Right).encode(),
);

pub(super) const PRIMITIVE_MIDDLE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Middle).encode(),
);

pub(super) const PRIMITIVE_MATH_CHOICE: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::MathChoice).encode(),
);

pub(super) const PRIMITIVE_RADICAL: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Radical).encode(),
);

pub(super) const PRIMITIVE_MATH_ACCENT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::MathAccent).encode(),
);

pub(super) const PRIMITIVE_M_SKIP: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::MSkip).encode(),
);

pub(super) const PRIMITIVE_M_KERN: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::MKern).encode(),
);

pub(super) const PRIMITIVE_EQ_NO: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::EqNo).encode(),
);

pub(super) const PRIMITIVE_LEFT_EQ_NO: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::LeftEqNo).encode(),
);

pub(super) const CHAR_PARAMETER: u64 =
    Meaning::runtime_word_control_code(Meaning::char_token_word('\0', Catcode::Parameter));
pub(super) const END_CS_NAME: u64 = Meaning::runtime_word_control_code(
    Meaning::ExpandablePrimitive(tex_state::meaning::ExpandablePrimitive::EndCsName).encode(),
);

pub(super) const END_V: u64 = Meaning::runtime_word_control_code(Meaning::END_V_WORD);

pub(super) const CHAR_ALIGN_TAB: u64 =
    Meaning::runtime_word_control_code(Meaning::char_token_word('&', Catcode::AlignmentTab));
pub(super) const PRIMITIVE_CR: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Cr).encode(),
);
pub(super) const PRIMITIVE_CR_CR: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::CrCr).encode(),
);
pub(super) const PRIMITIVE_SPAN: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Span).encode(),
);
pub(super) const PRIMITIVE_OMIT: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Omit).encode(),
);
pub(super) const PRIMITIVE_NO_ALIGN: u64 = Meaning::runtime_word_control_code(
    Meaning::UnexpandablePrimitive(UnexpandablePrimitive::NoAlign).encode(),
);

pub(super) const CHAR_CAR_RET: u64 =
    Meaning::runtime_word_control_code(Meaning::char_token_word('\0', Catcode::EndLine));
