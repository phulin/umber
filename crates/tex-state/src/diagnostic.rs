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
use crate::token::OriginId;
use crate::token_show::append_tex_print_char;
use crate::universe::{InteractionMode, Universe};

/// The observed spelling of a token retained by a cold diagnostic report.
///
/// This transport value deliberately keeps the token kind alongside its
/// spelling. A control-sequence-looking string is not enough to distinguish a
/// character, parameter, macro-match marker, or frozen sentinel after the
/// command that supplied it has been backed up.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum DiagnosticToken {
    Character {
        character: char,
        catcode: crate::token::Catcode,
    },
    ControlSequence(String),
    MacroMatch,
    MacroEndMatch,
    Parameter(u8),
    FrozenEndTemplate,
    FrozenEndV,
    FrozenPrimitive(String),
    FrozenOther,
}

/// A compact, report-time snapshot of the live input/group context.
///
/// The context is intentionally structural rather than rendered text. It is
/// small enough to cross a queued diagnostic or a resource suspension and is
/// independent of the command-generation owner that may be rolled back.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DiagnosticContext {
    pub input_frame_count: usize,
    pub input_frame_tail: Vec<&'static str>,
    pub group_depth: u32,
    pub group_tail: Vec<DiagnosticGroup>,
}

/// One bounded group entry in a [`DiagnosticContext`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DiagnosticGroup {
    pub kind: &'static str,
    pub entered_line: u32,
}

/// Crate-neutral provenance and scanner facts for one recoverable report.
///
/// Every field is frozen at the report-completion seam. In particular, this
/// value never retains a `CurrentCommand`, a command context borrow, or a live
/// input coordinate that could become stale after TeX backs up a token.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DiagnosticSite {
    pub origin: Option<OriginId>,
    pub observed_token: Option<DiagnosticToken>,
    pub command: Option<String>,
    pub command_operand: Option<i64>,
    pub context: Option<DiagnosticContext>,
    pub mode: Option<&'static str>,
    pub scanner_status: &'static str,
    pub interaction: Option<InteractionMode>,
}

/// Maximum amount of text retained for one causal recoverable diagnostic.
///
/// Error reports are cold paths, but a malformed input can supply arbitrarily
/// large rendered context or message fragments.  Keeping the bound here makes
/// the retained run evidence predictable without adding any work to command
/// delivery or ordinary printing.
pub const MAX_RECOVERABLE_DIAGNOSTIC_TEXT: usize = 512;

/// Small, engine-neutral structured argument carried with a deferred error.
///
/// `tex-state` cannot depend on `tex-command`, so the command observation
/// layer converts these owned values to its transport vocabulary when the
/// report reaches the executor boundary.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum RecoverableDiagnosticArgument {
    Token(DiagnosticToken),
    Name(String),
}

/// One recoverable report candidate, owned by an operation until its effects
/// are committed.  The candidate is intentionally independent of World and
/// command-generation handles so rollback can drop it with the rest of the
/// detached diagnostic effects.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RecoverableDiagnostic {
    pub kind: &'static str,
    pub message: Box<str>,
    pub arguments: Vec<RecoverableDiagnosticArgument>,
    pub site: Option<DiagnosticSite>,
    pub interaction: InteractionMode,
}

impl RecoverableDiagnostic {
    pub(crate) fn truncate_text(value: &str) -> String {
        let end = value
            .char_indices()
            .map(|(index, _)| index)
            .chain(std::iter::once(value.len()))
            .take_while(|index| *index <= MAX_RECOVERABLE_DIAGNOSTIC_TEXT)
            .last()
            .unwrap_or(0);
        value[..end].to_owned()
    }

    fn truncate_owned(mut value: String) -> String {
        if value.len() > MAX_RECOVERABLE_DIAGNOSTIC_TEXT {
            let mut end = MAX_RECOVERABLE_DIAGNOSTIC_TEXT;
            while !value.is_char_boundary(end) {
                end -= 1;
            }
            value.truncate(end);
        }
        value
    }

    pub(crate) fn new(
        kind: &'static str,
        message: String,
        arguments: Vec<RecoverableDiagnosticArgument>,
        interaction: InteractionMode,
    ) -> Self {
        Self {
            kind,
            message: Self::truncate_owned(message).into_boxed_str(),
            arguments: arguments
                .into_iter()
                .take(8)
                .map(|argument| match argument {
                    RecoverableDiagnosticArgument::Token(value) => {
                        RecoverableDiagnosticArgument::Token(Self::truncate_diagnostic_token(value))
                    }
                    RecoverableDiagnosticArgument::Name(value) => {
                        RecoverableDiagnosticArgument::Name(Self::truncate_owned(value))
                    }
                })
                .collect(),
            site: None,
            interaction,
        }
    }

    fn truncate_diagnostic_token(token: DiagnosticToken) -> DiagnosticToken {
        match token {
            DiagnosticToken::ControlSequence(value) => {
                DiagnosticToken::ControlSequence(Self::truncate_owned(value))
            }
            DiagnosticToken::FrozenPrimitive(value) => {
                DiagnosticToken::FrozenPrimitive(Self::truncate_owned(value))
            }
            token => token,
        }
    }
}

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
#[derive(Debug, Default, Eq, PartialEq)]
pub struct DiagnosticEffects {
    effects: Vec<DetachedDiagnosticEffect>,
    error_stop_recovery: Option<crate::print::ErrorRecoveryRequest>,
    first_recoverable: Option<RecoverableDiagnostic>,
}

impl DiagnosticEffects {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            effects: Vec::new(),
            error_stop_recovery: None,
            first_recoverable: None,
        }
    }

    pub fn push(&mut self, effect: DetachedDiagnosticEffect) {
        self.effects.push(effect);
    }

    pub(crate) fn push_ordinary_rendered(
        &mut self,
        interaction_mode: InteractionMode,
        max_print_line: usize,
        text: String,
    ) {
        self.effects.push(DetachedDiagnosticEffect {
            selector: Selector::for_interaction(interaction_mode),
            max_print_line,
            records_warning_history: false,
            operations: vec![DiagnosticPrintOperation::Rendered(text)],
        });
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.effects.is_empty() && self.error_stop_recovery.is_none()
    }

    /// Reports whether a cold-path recoverable candidate is waiting for the
    /// enclosing operation's commit seam. This is intentionally separate from
    /// [`Self::is_empty`], which is used by the hot character/command loop and
    /// must retain its pre-evidence cost model.
    #[must_use]
    pub fn has_first_recoverable(&self) -> bool {
        self.first_recoverable.is_some()
    }

    /// Reports whether the operation-local candidate still needs its
    /// report-time site completed. Callers use this to avoid recomputing a
    /// captured site at the outer error-return seam.
    #[must_use]
    pub fn first_recoverable_site_missing(&self) -> bool {
        self.first_recoverable
            .as_ref()
            .is_some_and(|diagnostic| diagnostic.site.is_none())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.effects.len()
    }

    pub fn drain(&mut self) -> impl Iterator<Item = DetachedDiagnosticEffect> + '_ {
        self.effects.drain(..)
    }

    /// Retains the synchronous ErrorStop input action until the enclosing
    /// executor hands control back to the sole command-input owner.
    pub fn request_error_stop_recovery(&mut self, request: crate::print::ErrorRecoveryRequest) {
        assert!(
            self.error_stop_recovery.replace(request).is_none(),
            "one executor transition owns at most one ErrorStop response"
        );
    }

    /// Transfers the synchronous response to the command/input transition.
    pub fn take_error_stop_recovery(&mut self) -> Option<crate::print::ErrorRecoveryRequest> {
        self.error_stop_recovery.take()
    }

    /// Records the first recoverable report produced by this operation.
    /// Later reports cannot replace it, matching TeX's first-cause semantics.
    pub fn record_first_recoverable(&mut self, diagnostic: RecoverableDiagnostic) -> bool {
        if self.first_recoverable.is_some() {
            false
        } else {
            self.first_recoverable = Some(diagnostic);
            true
        }
    }

    /// Freezes the first report's compact provenance at the report-completion
    /// seam. A later report cannot fill or overwrite an earlier site.
    pub fn complete_first_recoverable(&mut self, site: DiagnosticSite) {
        if let Some(diagnostic) = self.first_recoverable.as_mut()
            && diagnostic.site.is_none()
        {
            if let Some(interaction) = site.interaction {
                diagnostic.interaction = interaction;
            }
            diagnostic.site = Some(site);
        }
    }

    /// Moves the operation-local candidate to its enclosing commit owner.
    pub fn take_first_recoverable(&mut self) -> Option<RecoverableDiagnostic> {
        self.first_recoverable.take()
    }

    /// Reinstalls a candidate retained across an intermediate publication.
    pub fn restore_first_recoverable(&mut self, diagnostic: RecoverableDiagnostic) {
        assert!(
            self.first_recoverable.replace(diagnostic).is_none(),
            "one operation owns at most one first recoverable diagnostic"
        );
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
        self.finish();
    }

    /// Commits an ordinary print program while retaining its open line.
    ///
    /// TeX82 §1297's `\showthe`/token display jumps to §1293's common
    /// ending before `end_diagnostic`; §82 supplies the period that closes
    /// the same line. This is that deliberately incomplete print boundary,
    /// not a general substitute for [`Self::end`].
    pub fn end_open(self) {
        self.finish();
    }

    fn finish(self) {
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
