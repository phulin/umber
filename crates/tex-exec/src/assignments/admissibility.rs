use tex_state::meaning::UnexpandablePrimitive;

/// Authoritative command-family classification for assignment primitives.
pub(super) fn is_assignment_primitive(primitive: UnexpandablePrimitive) -> bool {
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
            | UnexpandablePrimitive::Font
            | UnexpandablePrimitive::TextFont
            | UnexpandablePrimitive::ScriptFont
            | UnexpandablePrimitive::ScriptScriptFont
            | UnexpandablePrimitive::FontDimen
            | UnexpandablePrimitive::HyphenChar
            | UnexpandablePrimitive::SkewChar
            | UnexpandablePrimitive::Patterns
            | UnexpandablePrimitive::Hyphenation
            | UnexpandablePrimitive::SpaceFactor
            | UnexpandablePrimitive::PrevDepth
            | UnexpandablePrimitive::PrevGraf
            | UnexpandablePrimitive::SetBox
            | UnexpandablePrimitive::Wd
            | UnexpandablePrimitive::Ht
            | UnexpandablePrimitive::Dp
            | UnexpandablePrimitive::OpenIn
            | UnexpandablePrimitive::CloseIn
            | UnexpandablePrimitive::OpenOut
            | UnexpandablePrimitive::CloseOut
            | UnexpandablePrimitive::Immediate
            | UnexpandablePrimitive::Write
            | UnexpandablePrimitive::Read
            | UnexpandablePrimitive::BatchMode
            | UnexpandablePrimitive::NonstopMode
            | UnexpandablePrimitive::ScrollMode
            | UnexpandablePrimitive::ErrorStopMode
    )
}

/// Mode-independent commands that main control may pass through in math mode.
///
/// Assignment-family membership is defined once above. This function records
/// only the small math-mode exception set and the additional non-assignment
/// commands whose ordinary executor semantics are mode independent.
pub(crate) fn math_allows_mode_independent_primitive(primitive: UnexpandablePrimitive) -> bool {
    if is_assignment_primitive(primitive) {
        return !matches!(
            primitive,
            UnexpandablePrimitive::Patterns
                | UnexpandablePrimitive::Hyphenation
                | UnexpandablePrimitive::SpaceFactor
                | UnexpandablePrimitive::PrevDepth
                | UnexpandablePrimitive::OpenIn
                | UnexpandablePrimitive::CloseIn
                | UnexpandablePrimitive::Read
                | UnexpandablePrimitive::BatchMode
                | UnexpandablePrimitive::NonstopMode
                | UnexpandablePrimitive::ScrollMode
                | UnexpandablePrimitive::ErrorStopMode
        );
    }

    matches!(
        primitive,
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
            | UnexpandablePrimitive::Lowercase
            | UnexpandablePrimitive::Uppercase
            | UnexpandablePrimitive::Cr
            | UnexpandablePrimitive::CrCr
            | UnexpandablePrimitive::Span
            | UnexpandablePrimitive::Omit
            | UnexpandablePrimitive::NoAlign
            | UnexpandablePrimitive::Mark
            | UnexpandablePrimitive::Marks
    )
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
    }
}
