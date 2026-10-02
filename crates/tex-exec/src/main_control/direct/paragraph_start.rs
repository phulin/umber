//! Native packed membership in TeX82 §1090's vertical paragraph-start cases.
use super::codes;

// Both vertical modes use this membership. Spacers, kerns, hrules and unvbox
// remain vertical; §1090's text-direction modifiers share valign's behavior.
pub(in crate::main_control) fn starts_paragraph_in_vertical_mode(code: Option<u64>) -> bool {
    matches!(
        code,
        Some(
            codes::CHAR_LETTER
                | codes::CHAR_OTHER
                | codes::CHAR_MATH_SHIFT
                | codes::CHAR_GIVEN
                | codes::PRIMITIVE_CHAR
                | codes::PRIMITIVE_UN_H_BOX
                | codes::PRIMITIVE_UN_H_COPY
                | codes::PRIMITIVE_V_RULE
                | codes::PRIMITIVE_ACCENT
                | codes::PRIMITIVE_DISCRETIONARY
                | codes::PRIMITIVE_DISCRETIONARY_HYPHEN
                | codes::PRIMITIVE_H_SKIP
                | codes::PRIMITIVE_H_FIL
                | codes::PRIMITIVE_H_FILL
                | codes::PRIMITIVE_H_SS
                | codes::PRIMITIVE_H_FIL_NEG
                | codes::PRIMITIVE_V_ALIGN
                | codes::PRIMITIVE_BEGIN_L
                | codes::PRIMITIVE_END_L
                | codes::PRIMITIVE_BEGIN_R
                | codes::PRIMITIVE_END_R
                | codes::PRIMITIVE_CONTROL_SPACE
                | codes::PRIMITIVE_NO_BOUNDARY
        )
    )
}
