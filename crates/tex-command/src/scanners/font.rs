//! Font scanner.
//!
//! Font identifiers are consumed here so replay receives semantic selectors,
//! never a retained current command or an input cursor.

use tex_state::ids::FontId;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};

use crate::scanners::MathFamilySize;
use crate::{CommandError, CommandProcessor};

impl CommandProcessor<'_> {
    /// Scans TeX82 §577's `scan_font_ident` through canonical expanded
    /// delivery.
    ///
    /// §577 recognizes exactly three commands, and this is the *only* routine
    /// in TeX that turns a token into a font: `def_font` (`\font`) reads
    /// `cur_font`, `set_font` (any `\font`-defined identifier, and
    /// `\nullfont`) reads its own font, and `def_family` (`\textfont`,
    /// `\scriptfont`, `\scriptscriptfont`) reads §435's `scan_four_bit_int`
    /// family index and then that size bank's font. Anything else is "Missing
    /// font identifier", whose `back_error` leaves the rejected command for
    /// its normal delivery and takes `null_font`.
    pub fn scan_font_selector(&mut self) -> Result<FontId, CommandError> {
        // §577's `@<Get the next non-blank non-call token@>` (§406).
        let command = loop {
            let command = self.get_x_token()?.ok_or(CommandError::input_invariant())?;
            if !matches!(
                command.meaning(),
                Meaning::CharToken {
                    cat: tex_state::token::Catcode::Space,
                    ..
                }
            ) {
                break command;
            }
        };
        match command.meaning() {
            Meaning::Font(font) => Ok(font),
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Font) => {
                Ok(self.state.current_font())
            }
            Meaning::UnexpandablePrimitive(primitive)
                if MathFamilySize::of_primitive(primitive).is_some() =>
            {
                let size = MathFamilySize::of_primitive(primitive)
                    .expect("the guard proved this is `def_family`");
                let family = self.scan_math_family(size)?;
                Ok(self.state.math_family_font(size.into(), family.family))
            }
            _ => {
                // §577 reports before §327's `back_error` backs up the
                // rejected command. The fixed diagnostic has no operand.
                self.observe_error_diagnostic("missing_font_identifier");
                // `scan_font_ident` backs up a non-font command. Its normal
                // main-control delivery is therefore still command-owned.
                self.back_input(command)?;
                Ok(tex_state::font::NULL_FONT)
            }
        }
    }
}
