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

/// TeX82 §1030's `if tracing_commands>0 then show_cur_cmd_chr` at `reswitch`,
/// reached after `big_switch` and after cases such as §1045 `ignore_spaces`
/// fetch a replacement command. §1211's prefix loop does not return to that
/// label, so its internal fetches remain untraced.
pub(super) fn prepare_command_trace<G, Facts: tex_command::CommandHostFacts<G>>(
    processor: &mut CommandProcessor<'_, '_, G, Facts>,
    mode: Mode,
    shown_mode: Option<Mode>,
) {
    // The mode text is owned because expansion may retain it until a later
    // command in this processor episode.  Do not allocate that ownership when
    // neither command nor conditional tracing can consume the prefix: this is
    // the ordinary production path and this boundary is crossed once per
    // main-control operation.
    let tracing_can_consume_prefix = processor.int_param(IntParam::TRACING_COMMANDS) > 0
        || processor.int_param(IntParam::TRACING_IFS) > 0;
    let mode_prefix = (tracing_can_consume_prefix && shown_mode != Some(mode))
        .then(|| mode_text_for_command_trace(mode).into());
    processor.set_command_trace_mode_prefix(mode_prefix);
}
