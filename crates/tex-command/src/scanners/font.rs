//! Font scanner.
//!
//! Font identifiers are consumed here so replay receives semantic selectors,
//! never a retained current command or an input cursor.

use tex_state::ids::FontId;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};

use crate::{CommandError, CommandProcessor};

impl CommandProcessor<'_> {
    /// Scans TeX82's `scan_font_ident` through canonical expanded delivery.
    pub fn scan_font_selector(&mut self) -> Result<FontId, CommandError> {
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
            _ => {
                // `scan_font_ident` backs up a non-font command. Its normal
                // main-control delivery is therefore still command-owned.
                self.back_input(command)?;
                Ok(tex_state::font::NULL_FONT)
            }
        }
    }
}
