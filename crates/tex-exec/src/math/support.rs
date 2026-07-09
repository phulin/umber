use tex_state::math::{LimitType, MathStyle, NoadClass, NoadKind};
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::{PrintSink, Universe};

pub(super) fn noad_kind_for_constructor(primitive: UnexpandablePrimitive) -> NoadKind {
    match primitive {
        UnexpandablePrimitive::MathOrd => NoadKind::Normal(NoadClass::Ord),
        UnexpandablePrimitive::MathOp => NoadKind::Operator(LimitType::DisplayLimits),
        UnexpandablePrimitive::MathBin => NoadKind::Normal(NoadClass::Bin),
        UnexpandablePrimitive::MathRel => NoadKind::Normal(NoadClass::Rel),
        UnexpandablePrimitive::MathOpen => NoadKind::Normal(NoadClass::Open),
        UnexpandablePrimitive::MathClose => NoadKind::Normal(NoadClass::Close),
        UnexpandablePrimitive::MathPunct => NoadKind::Normal(NoadClass::Punct),
        UnexpandablePrimitive::MathInner => NoadKind::Normal(NoadClass::Inner),
        _ => unreachable!("caller restricts constructor primitive"),
    }
}

pub(super) fn style_for_primitive(primitive: UnexpandablePrimitive) -> MathStyle {
    match primitive {
        UnexpandablePrimitive::DisplayStyle => MathStyle::Display,
        UnexpandablePrimitive::TextStyle => MathStyle::Text,
        UnexpandablePrimitive::ScriptStyle => MathStyle::Script,
        UnexpandablePrimitive::ScriptScriptStyle => MathStyle::ScriptScript,
        _ => unreachable!("caller restricts style primitive"),
    }
}

pub(super) fn math_allows_assignment_primitive(primitive: UnexpandablePrimitive) -> bool {
    matches!(
        primitive,
        UnexpandablePrimitive::Def
            | UnexpandablePrimitive::Edef
            | UnexpandablePrimitive::Gdef
            | UnexpandablePrimitive::Xdef
            | UnexpandablePrimitive::Let
            | UnexpandablePrimitive::FutureLet
            | UnexpandablePrimitive::GlobalDefs
            | UnexpandablePrimitive::Global
            | UnexpandablePrimitive::Long
            | UnexpandablePrimitive::Outer
            | UnexpandablePrimitive::Protected
            | UnexpandablePrimitive::Count
            | UnexpandablePrimitive::Dimen
            | UnexpandablePrimitive::Skip
            | UnexpandablePrimitive::Muskip
            | UnexpandablePrimitive::Toks
            | UnexpandablePrimitive::CountDef
            | UnexpandablePrimitive::DimenDef
            | UnexpandablePrimitive::SkipDef
            | UnexpandablePrimitive::MuskipDef
            | UnexpandablePrimitive::ToksDef
            | UnexpandablePrimitive::CharDef
            | UnexpandablePrimitive::MathCharDef
            | UnexpandablePrimitive::Advance
            | UnexpandablePrimitive::Multiply
            | UnexpandablePrimitive::Divide
            | UnexpandablePrimitive::CatCode
            | UnexpandablePrimitive::LcCode
            | UnexpandablePrimitive::UcCode
            | UnexpandablePrimitive::SfCode
            | UnexpandablePrimitive::MathCode
            | UnexpandablePrimitive::DelCode
            | UnexpandablePrimitive::Font
            | UnexpandablePrimitive::TextFont
            | UnexpandablePrimitive::ScriptFont
            | UnexpandablePrimitive::ScriptScriptFont
            | UnexpandablePrimitive::FontDimen
            | UnexpandablePrimitive::HyphenChar
            | UnexpandablePrimitive::SkewChar
            | UnexpandablePrimitive::AfterGroup
            | UnexpandablePrimitive::AfterAssignment
            | UnexpandablePrimitive::Show
            | UnexpandablePrimitive::ShowThe
            | UnexpandablePrimitive::ShowTokens
            | UnexpandablePrimitive::ShowLists
            | UnexpandablePrimitive::Message
            | UnexpandablePrimitive::ErrMessage
    )
}

pub(super) fn math_allows_assignment_meaning(meaning: Meaning) -> bool {
    matches!(
        meaning,
        Meaning::CountRegister(_)
            | Meaning::DimenRegister(_)
            | Meaning::SkipRegister(_)
            | Meaning::MuskipRegister(_)
            | Meaning::ToksRegister(_)
            | Meaning::IntParam(_)
            | Meaning::DimenParam(_)
            | Meaning::GlueParam(_)
            | Meaning::MuGlueParam(_)
            | Meaning::TokParam(_)
            | Meaning::PageDimension(_)
            | Meaning::PageInteger(_)
    )
}

pub(super) fn report_math_error(stores: &mut Universe, text: &str) {
    stores
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, &format!("\n! {text}.\n"));
}
