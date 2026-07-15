use tex_state::meaning::UnexpandablePrimitive;

#[derive(Clone, Copy)]
struct Admissibility {
    assignment: bool,
    math_mode_independent: bool,
}

const ASSIGNMENT_AND_MATH: Admissibility = Admissibility {
    assignment: true,
    math_mode_independent: true,
};
const ASSIGNMENT_ONLY: Admissibility = Admissibility {
    assignment: true,
    math_mode_independent: false,
};
const MATH_ONLY: Admissibility = Admissibility {
    assignment: false,
    math_mode_independent: true,
};
const NEITHER: Admissibility = Admissibility {
    assignment: false,
    math_mode_independent: false,
};

/// The single authoritative mode-admissibility definition for primitives.
/// Each primitive appears in at most one arm, so extending the enum requires
/// one classification decision rather than coordinated allowlists.
const fn admissibility(primitive: UnexpandablePrimitive) -> Admissibility {
    match primitive {
        UnexpandablePrimitive::SpaceFactor | UnexpandablePrimitive::PrevDepth => ASSIGNMENT_ONLY,
        UnexpandablePrimitive::Def
        | UnexpandablePrimitive::Edef
        | UnexpandablePrimitive::Gdef
        | UnexpandablePrimitive::Xdef
        | UnexpandablePrimitive::Let
        | UnexpandablePrimitive::FutureLet
        | UnexpandablePrimitive::GlobalDefs
        | UnexpandablePrimitive::Global
        | UnexpandablePrimitive::BeginGroup
        | UnexpandablePrimitive::EndGroup
        | UnexpandablePrimitive::AfterGroup
        | UnexpandablePrimitive::AfterAssignment
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
        | UnexpandablePrimitive::PdfLpCode
        | UnexpandablePrimitive::PdfRpCode
        | UnexpandablePrimitive::PdfEfCode
        | UnexpandablePrimitive::PdfTagCode
        | UnexpandablePrimitive::PdfKnbsCode
        | UnexpandablePrimitive::PdfStbsCode
        | UnexpandablePrimitive::PdfShbsCode
        | UnexpandablePrimitive::PdfKnbcCode
        | UnexpandablePrimitive::PdfKnacCode
        | UnexpandablePrimitive::Font
        | UnexpandablePrimitive::LetterspaceFont
        | UnexpandablePrimitive::PdfCopyFont
        | UnexpandablePrimitive::PdfFontExpand
        | UnexpandablePrimitive::PdfFontAttr
        | UnexpandablePrimitive::PdfIncludeChars
        | UnexpandablePrimitive::PdfMapFile
        | UnexpandablePrimitive::PdfMapLine
        | UnexpandablePrimitive::PdfGlyphToUnicode
        | UnexpandablePrimitive::PdfNoBuiltinToUnicode
        | UnexpandablePrimitive::TextFont
        | UnexpandablePrimitive::ScriptFont
        | UnexpandablePrimitive::ScriptScriptFont
        | UnexpandablePrimitive::FontDimen
        | UnexpandablePrimitive::HyphenChar
        | UnexpandablePrimitive::SkewChar
        | UnexpandablePrimitive::PrevGraf
        | UnexpandablePrimitive::SetBox
        | UnexpandablePrimitive::Wd
        | UnexpandablePrimitive::Ht
        | UnexpandablePrimitive::Dp
        | UnexpandablePrimitive::Patterns
        | UnexpandablePrimitive::Hyphenation
        | UnexpandablePrimitive::OpenIn
        | UnexpandablePrimitive::CloseIn
        | UnexpandablePrimitive::Read
        | UnexpandablePrimitive::BatchMode
        | UnexpandablePrimitive::NonstopMode
        | UnexpandablePrimitive::ScrollMode
        | UnexpandablePrimitive::ErrorStopMode
        | UnexpandablePrimitive::OpenOut
        | UnexpandablePrimitive::CloseOut
        | UnexpandablePrimitive::Immediate
        | UnexpandablePrimitive::Write => ASSIGNMENT_AND_MATH,
        UnexpandablePrimitive::VAdjust
        | UnexpandablePrimitive::ParShape
        | UnexpandablePrimitive::InterLinePenalties
        | UnexpandablePrimitive::ClubPenalties
        | UnexpandablePrimitive::WidowPenalties
        | UnexpandablePrimitive::DisplayWidowPenalties
        | UnexpandablePrimitive::UnPenalty
        | UnexpandablePrimitive::UnKern
        | UnexpandablePrimitive::UnSkip
        | UnexpandablePrimitive::PageDiscards
        | UnexpandablePrimitive::SplitDiscards
        | UnexpandablePrimitive::Insert
        | UnexpandablePrimitive::Discretionary
        | UnexpandablePrimitive::Show
        | UnexpandablePrimitive::ShowThe
        | UnexpandablePrimitive::ShowTokens
        | UnexpandablePrimitive::ShowGroups
        | UnexpandablePrimitive::ShowIfs
        | UnexpandablePrimitive::ShowLists
        | UnexpandablePrimitive::ShowBox
        | UnexpandablePrimitive::Message
        | UnexpandablePrimitive::ErrMessage
        | UnexpandablePrimitive::Special
        | UnexpandablePrimitive::IgnoreSpaces
        | UnexpandablePrimitive::ControlSpace
        | UnexpandablePrimitive::PdfInterwordSpaceOn
        | UnexpandablePrimitive::PdfInterwordSpaceOff
        | UnexpandablePrimitive::PdfFakeSpace
        | UnexpandablePrimitive::PdfSpaceFont
        | UnexpandablePrimitive::Lowercase
        | UnexpandablePrimitive::Uppercase
        | UnexpandablePrimitive::Cr
        | UnexpandablePrimitive::CrCr
        | UnexpandablePrimitive::Span
        | UnexpandablePrimitive::Omit
        | UnexpandablePrimitive::NoAlign
        | UnexpandablePrimitive::Mark
        | UnexpandablePrimitive::Marks => MATH_ONLY,
        _ => NEITHER,
    }
}

pub(super) const fn is_assignment_primitive(primitive: UnexpandablePrimitive) -> bool {
    admissibility(primitive).assignment
}

pub(crate) const fn math_allows_mode_independent_primitive(
    primitive: UnexpandablePrimitive,
) -> bool {
    admissibility(primitive).math_mode_independent
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn math_admissibility_reuses_assignment_family_with_explicit_exceptions() {
        assert!(is_assignment_primitive(UnexpandablePrimitive::Count));
        assert!(math_allows_mode_independent_primitive(
            UnexpandablePrimitive::Count
        ));
        assert!(is_assignment_primitive(UnexpandablePrimitive::PrevDepth));
        assert!(!math_allows_mode_independent_primitive(
            UnexpandablePrimitive::PrevDepth
        ));
        assert!(!is_assignment_primitive(UnexpandablePrimitive::Mark));
        assert!(math_allows_mode_independent_primitive(
            UnexpandablePrimitive::Mark
        ));
        assert!(math_allows_mode_independent_primitive(
            UnexpandablePrimitive::IgnoreSpaces
        ));
        assert!(math_allows_mode_independent_primitive(
            UnexpandablePrimitive::OpenIn
        ));
    }
}
