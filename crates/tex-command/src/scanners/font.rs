//! Font scanner.
//!
//! Font identifiers are consumed here so replay receives semantic selectors,
//! never a retained current command or an input cursor.

use tex_state::ids::FontId;
use tex_state::meaning::UnexpandablePrimitive;

use crate::scanners::MathFamilySize;
use crate::{CommandError, CommandProcessor};

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
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
        let mut font = None;
        self.consume_expanded_words(&mut crate::terminal_consumer::<Self, G>(
            &mut |processor, spelling, meaning, _code, _, exceptional| {
                if let Some(font_id) = meaning.font_id() {
                    font = Some(font_id);
                    return Ok(std::ops::ControlFlow::Break(()));
                }
                if meaning
                    .character()
                    .is_some_and(|(_, cat)| cat == tex_state::token::Catcode::Space)
                {
                    return Ok(std::ops::ControlFlow::Continue(()));
                }
                font = Some(match meaning.unexpandable_primitive() {
                    Some(UnexpandablePrimitive::Font) => processor.state.current_font(),
                    Some(primitive) if MathFamilySize::of_primitive(primitive).is_some() => {
                        let size = MathFamilySize::of_primitive(primitive)
                            .expect("guard proves def_family");
                        let family = processor
                            .scan_restricted_integer(crate::RestrictedIntegerClass::FourBit)?
                            .value as u8;
                        processor.state.math_family_font(size.into(), family)
                    }
                    _ => {
                        // TeX82 §577 reports before §327's back_error restores
                        // the rejected spelling and §82 renders the input context.
                        let deferred = {
                            let mut report = processor.state.print_err("Missing font identifier");
                            report.help(&[
                                "I was looking for a control sequence whose",
                                "current meaning has been defined by \\font.",
                            ]);
                            report.defer()
                        };
                        processor.back_consumed_word(spelling, exceptional)?;
                        let context = processor.command.output_open_context(processor.state);
                        let mut report = processor.state.resume_error_report(deferred);
                        report.context(context);
                        let outcome = report.error();
                        processor.finish_error_outcome(outcome)?;
                        tex_state::font::NULL_FONT
                    }
                });
                Ok(std::ops::ControlFlow::Break(()))
            },
        ))?;
        font.ok_or_else(CommandError::input_invariant)
    }
}
