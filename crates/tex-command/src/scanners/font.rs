//! Font scanner.
//!
//! Font identifiers are consumed here so replay receives semantic selectors,
//! never a retained current command or an input cursor.

use tex_state::ids::FontId;
use tex_state::meaning::{Meaning, ResolvedMeaning, UnexpandablePrimitive};

use crate::scanners::MathFamilySize;
use crate::{CommandError, CommandProcessor};

impl<G> CommandProcessor<'_, '_, G> {
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
                static_meaning(command.meaning()),
                Meaning::CharToken {
                    cat: tex_state::token::Catcode::Space,
                    ..
                }
            ) {
                break command;
            }
        };
        match static_meaning(command.meaning()) {
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
                // rejected command. The pinned WEB observer does not assign
                // this text-only error a semantic diagnostic event: only the
                // report channel below carries it. Publishing an additional
                // command event would shift every later canonical event.
                let deferred = {
                    let mut report = self.state.print_err("Missing font identifier");
                    report.help(&[
                        "I was looking for a control sequence whose",
                        "current meaning has been defined by \\font.",
                    ]);
                    report.defer()
                };
                // §577's `back_error` is `back_input; error`: the rejected
                // command is restored before §82 renders §310's display, so
                // §314 names it on its own `<to be read again>␣` line.
                // `scan_font_ident` backs up a non-font command. Its normal
                // main-control delivery is therefore still command-owned.
                self.back_input(command)?;
                let context = self.command.output_open_context(&self.state);
                let mut report = self.state.resume_error_report(deferred);
                report.context(context);
                report.error().jump_out()?;
                Ok(tex_state::font::NULL_FONT)
            }
        }
    }
}

const fn static_meaning<G>(meaning: ResolvedMeaning<G>) -> Meaning {
    match meaning {
        ResolvedMeaning::Static(meaning) => meaning,
        ResolvedMeaning::Macro { .. } => Meaning::Undefined,
    }
}
