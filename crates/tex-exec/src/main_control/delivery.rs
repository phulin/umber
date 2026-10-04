//! Command delivery, preflight scanning, and typed retry transitions.

use super::*;

#[derive(Clone, Copy)]
pub(super) enum OffSaveCloser {
    EndGroup,
    MathShift,
    NullRight,
    RightBrace,
}

impl OffSaveCloser {
    pub(super) fn print<G>(self, report: &mut tex_state::print::ErrorReport<'_, G>) {
        match self {
            Self::EndGroup => report.print_esc("endgroup"),
            Self::MathShift => report.print_char('$'),
            Self::NullRight => report.print_esc("right."),
            Self::RightBrace => report.print_char('}'),
        };
    }
}

/// TeX82 §1069's `case cur_group of`: the group opener a stray `}` was
/// probably standing in for.
///
/// This is deliberately not [`OffSaveCloser`]. §1064 inserts a closer and says
/// what it inserted, so its `math_left_group` arm is `\right.` -- a complete
/// command. §1069 deletes the brace and only names what was forgotten, so its
/// arm is the bare `\right`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ForgottenGroupOpener {
    /// `semi_simple_group`.
    EndGroup,
    /// `math_shift_group`.
    MathShift,
    /// `math_left_group`.
    Right,
}

impl ForgottenGroupOpener {
    pub(super) fn print<G>(self, report: &mut tex_state::print::ErrorReport<'_, G>) {
        match self {
            Self::EndGroup => report.print_esc("endgroup"),
            Self::MathShift => report.print_char('$'),
            Self::Right => report.print_esc("right"),
        };
    }
}

pub(super) fn scan_code_table_operands<G, Facts: tex_command::CommandHostFacts<G>>(
    processor: &mut CommandProcessor<'_, '_, G, Facts>,
) -> Result<(char, i32), ExecError> {
    let character =
        match processor.scan_restricted_integer(RestrictedIntegerClass::CharacterCode) {
            Ok(value) => value,
            Err(error) => return Err(command_error(error)),
        }
        .value;
    let character =
        char::from_u32(character as u32).expect("scan_char_num returns a valid character");
    let _ = match processor.scan_optional_equals() {
        Ok(value) => value,
        Err(error) => return Err(command_error(error)),
    };
    let value = match processor.scan_integer() {
        Ok(value) => value,
        Err(error) => return Err(command_error(error)),
    }
    .value;
    Ok((character, value))
}
