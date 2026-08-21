//! Detached tex.web §245 diagnostic printing.
//!
//! A diagnostic is formatted while the command state is admitted, but it is
//! not published there. The builder records §§57--65 print operations using
//! captured scalar print controls and appends one handle-free effect to an
//! operation-local [`DiagnosticEffects`] collector. The outer operation
//! owner either drops that collector on rollback or gives it to
//! [`crate::World::publish_diagnostic_effects`] after admission has ended.
//!
//! This split is significant for §62 `print_nl`: whether a terminal or log
//! line is open belongs to `World`, and the two offsets can differ. The
//! detached program therefore records the line-start operation rather than
//! consulting either offset early.

use crate::print::Selector;
use crate::scaled::Scaled;
use crate::token_show::append_tex_print_char;
use crate::universe::{InteractionMode, Universe};

/// One print operation whose World-dependent routing is intentionally
/// deferred until the enclosing command operation commits.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum DiagnosticPrintOperation {
    /// Text already rendered through TeX's character-printing rules.
    Rendered(String),
    /// tex.web §62's conditional `print_ln` at the current sink offset.
    EnsureLineStart,
}

/// One logical §245 diagnostic, detached from engine and host handles.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DetachedDiagnosticEffect {
    selector: Selector,
    max_print_line: usize,
    records_warning_history: bool,
    operations: Vec<DiagnosticPrintOperation>,
}

impl DetachedDiagnosticEffect {
    #[must_use]
    pub const fn selector(&self) -> Selector {
        self.selector
    }

    #[must_use]
    pub const fn max_print_line(&self) -> usize {
        self.max_print_line
    }

    #[must_use]
    pub const fn records_warning_history(&self) -> bool {
        self.records_warning_history
    }

    #[must_use]
    pub fn operations(&self) -> &[DiagnosticPrintOperation] {
        &self.operations
    }
}

/// Ordered diagnostics produced by one journalled command operation.
///
/// This is deliberately operation-local rather than state stored in
/// `Universe`. Moving it to World publishes every completed diagnostic in
/// canonical order; dropping it makes a rollback publish none of them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiagnosticEffects {
    effects: Vec<DetachedDiagnosticEffect>,
}

impl DiagnosticEffects {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            effects: Vec::new(),
        }
    }

    pub fn push(&mut self, effect: DetachedDiagnosticEffect) {
        self.effects.push(effect);
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.effects.len()
    }

    pub fn drain(&mut self) -> impl Iterator<Item = DetachedDiagnosticEffect> + '_ {
        self.effects.drain(..)
    }
}

/// An open, detached diagnostic print program.
///
/// The only borrow is the operation-local collector. No World, Printer,
/// output offset, state generation, or runtime identity can escape through
/// this type.
#[must_use = "an open diagnostic must be closed with `Diagnostic::end`"]
pub struct Diagnostic<'a> {
    effects: &'a mut DiagnosticEffects,
    selector: Selector,
    newline_char: i32,
    escape_char: i32,
    max_print_line: usize,
    records_warning_history: bool,
    operations: Vec<DiagnosticPrintOperation>,
}

impl<'a> Diagnostic<'a> {
    pub(crate) fn from_parts(
        effects: &'a mut DiagnosticEffects,
        interaction_mode: InteractionMode,
        max_print_line: usize,
        tracing_online: i32,
        newline_char: i32,
        escape_char: i32,
    ) -> Self {
        // tex.web §245 temporarily decrements term_and_log to log_only.
        let mut selector = Selector::for_interaction(interaction_mode);
        let records_warning_history = tracing_online <= 0 && selector == Selector::TermAndLog;
        if records_warning_history {
            selector = selector.decr();
        }
        Self {
            effects,
            selector,
            newline_char,
            escape_char,
            max_print_line,
            records_warning_history,
            operations: Vec::new(),
        }
    }

    #[must_use]
    pub const fn selector(&self) -> Selector {
        self.selector
    }

    fn append_rendered(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        match self.operations.last_mut() {
            Some(DiagnosticPrintOperation::Rendered(previous)) => previous.push_str(text),
            _ => self
                .operations
                .push(DiagnosticPrintOperation::Rendered(text.to_owned())),
        }
    }

    fn is_newline_character(&self, character: char) -> bool {
        u32::try_from(self.newline_char)
            .ok()
            .and_then(char::from_u32)
            == Some(character)
    }

    /// tex.web §§59--60's content-based `print`/`slow_print` analogue.
    pub fn print(&mut self, text: &str) -> &mut Self {
        let mut rendered = String::with_capacity(text.len());
        for character in text.chars() {
            if self.is_newline_character(character) {
                rendered.push('\n');
            } else {
                append_tex_print_char(character, &mut rendered);
            }
        }
        self.append_rendered(&rendered);
        self
    }

    /// Writes a display already rendered through TeX's print primitives.
    pub fn print_rendered(&mut self, text: &str) -> &mut Self {
        self.append_rendered(text);
        self
    }

    /// tex.web §58's `print_char`.
    pub fn print_char(&mut self, character: char) -> &mut Self {
        if self.is_newline_character(character) {
            return self.print_ln();
        }
        let mut buffer = [0u8; 4];
        self.append_rendered(character.encode_utf8(&mut buffer));
        self
    }

    /// tex.web §68's `print_ASCII`, via the one-character string table.
    pub fn print_ascii(&mut self, character: char) -> &mut Self {
        if self.is_newline_character(character) {
            return self.print_ln();
        }
        let mut rendered = String::new();
        append_tex_print_char(character, &mut rendered);
        self.append_rendered(&rendered);
        self
    }

    /// tex.web §57's `print_ln`.
    pub fn print_ln(&mut self) -> &mut Self {
        self.append_rendered("\n");
        self
    }

    /// tex.web §62's `print_nl`.
    pub fn print_nl(&mut self, text: &str) -> &mut Self {
        self.operations
            .push(DiagnosticPrintOperation::EnsureLineStart);
        self.print(text)
    }

    /// pdftex.web §65's widened `print_int`.
    pub fn print_int(&mut self, value: impl Into<i64>) -> &mut Self {
        self.print(&value.into().to_string())
    }

    /// tex.web §63's `print_esc`.
    pub fn print_esc(&mut self, name: &str) -> &mut Self {
        if (0..256).contains(&self.escape_char)
            && let Some(character) = u32::try_from(self.escape_char)
                .ok()
                .and_then(char::from_u32)
        {
            self.print_ascii(character);
        }
        self.print(name)
    }

    /// tex.web §103's `print_scaled`.
    pub fn print_scaled(&mut self, value: Scaled) -> &mut Self {
        self.print(&crate::scaled::print_scaled(value))
    }

    /// tex.web §245's `end_diagnostic`.
    pub fn end(mut self, blank_line: bool) {
        self.print_nl("");
        if blank_line {
            self.print_ln();
        }
        self.effects.push(DetachedDiagnosticEffect {
            selector: self.selector,
            max_print_line: self.max_print_line,
            records_warning_history: self.records_warning_history,
            operations: self.operations,
        });
    }
}

impl<G> Universe<G> {
    /// Opens §245's detached diagnostic channel.
    pub fn begin_diagnostic<'effects>(
        &self,
        effects: &'effects mut DiagnosticEffects,
    ) -> Diagnostic<'effects> {
        Diagnostic::from_parts(
            effects,
            self.interaction_mode(),
            self.error_context_widths().max_print_line(),
            self.int_param(crate::env::banks::IntParam::TRACING_ONLINE),
            self.int_param(crate::env::banks::IntParam::NEWLINE_CHAR),
            self.int_param(crate::env::banks::IntParam::ESCAPE_CHAR),
        )
    }

    /// Opens §245's detached channel with e-TeX's forced-online routing.
    pub fn begin_online_diagnostic<'effects>(
        &self,
        effects: &'effects mut DiagnosticEffects,
    ) -> Diagnostic<'effects> {
        Diagnostic::from_parts(
            effects,
            self.interaction_mode(),
            self.error_context_widths().max_print_line(),
            1,
            self.int_param(crate::env::banks::IntParam::NEWLINE_CHAR),
            self.int_param(crate::env::banks::IntParam::ESCAPE_CHAR),
        )
    }
}

#[cfg(test)]
mod tests;
