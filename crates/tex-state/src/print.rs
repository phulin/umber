//! tex.web §54's print `selector`, §§57-65's print primitives, §73's
//! `print_err`, and §82's `error`.
//!
//! Every message TeX shows a user -- ordinary `\message` text, `\show`
//! output, and every recoverable-error report -- is produced by the same
//! print primitives routed through one global `selector`. This module owns
//! that channel so no caller formats or routes its own output.
//!
//! [`crate::diagnostic`] is the `\tracing*` half of the same machinery:
//! tex.web §245's `begin_diagnostic` is defined entirely as a temporary
//! `decr(selector)`, so it is built on [`Printer`] here rather than
//! duplicating the primitives.
//!
//! # What Umber models differently, and what it does not model
//!
//! - **The live transcript is constantly open.** Umber's
//!   [`crate::world::World`] always accepts [`PrintSink::Log`] writes -- there
//!   is no `open_log_file` moment -- so live callers use
//!   [`Selector::for_interaction`]. [`Selector::for_interaction_and_log`]
//!   nevertheless exposes tex.web §§1262--1265's complete initialization
//!   table, including the pre-transcript `no_print`/`term_only` states.
//! - **`show_context` is caller-supplied.** tex.web §82 shows the live input
//!   stack after every error. The command core captures that display while it
//!   owns the stack and supplies it through [`ErrorReport::context`].
//! - **`jump_out` is reported to the caller.** tex.web §81's `jump_out` is a
//!   non-local `goto` out of whatever was in progress. Umber cannot perform
//!   one from this layer, so every site that reaches it returns a
//!   [`JumpOut`] through [`ErrorOutcome`] and the caller propagates it as a
//!   fatal error. All three of §82's 100-error abort, §84's `X`, and §71's
//!   terminal EOF inside §83's dialog are modelled; the reports each one
//!   prints on the way out are printed here, exactly where tex.web prints
//!   them.
//! - §83's dialog records deletion and insertion requests in the error
//!   channel. The canonical command processor applies them at its next raw
//!   input demand, where it exclusively owns the suspended input stack.

mod error_context;

pub use error_context::{ErrorContextLevel, token_list_replay_label};

/// Driver-selected widths for TeX82 §79's pseudoprinted error context.
///
/// Web2C exposes the WEB constants as process configuration. They are
/// operational output policy: formats and engine snapshots do not own them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ErrorContextWidths {
    error_line: usize,
    half_error_line: usize,
    max_print_line: usize,
}

impl ErrorContextWidths {
    /// TeX82 §3 requires `half_error_line` to be between 30 and
    /// `error_line - 15`.
    #[must_use]
    pub const fn new(error_line: usize, half_error_line: usize) -> Option<Self> {
        if half_error_line >= 30 && half_error_line <= error_line.saturating_sub(15) {
            Some(Self {
                error_line,
                half_error_line,
                max_print_line: MAX_PRINT_LINE,
            })
        } else {
            None
        }
    }

    #[must_use]
    pub const fn error_line(self) -> usize {
        self.error_line
    }

    #[must_use]
    pub const fn half_error_line(self) -> usize {
        self.half_error_line
    }

    /// Selects tex.web §3's `max_print_line` independently of the two
    /// pseudoprint widths. TRIP's canonical build uses 72 while ordinary
    /// Web2C jobs use 79.
    #[must_use]
    pub const fn with_max_print_line(mut self, max_print_line: usize) -> Option<Self> {
        if max_print_line >= 60 {
            self.max_print_line = max_print_line;
            Some(self)
        } else {
            None
        }
    }

    #[must_use]
    pub const fn max_print_line(self) -> usize {
        self.max_print_line
    }
}

impl Default for ErrorContextWidths {
    fn default() -> Self {
        Self::new(79, 50).expect("Web2C context widths are valid")
    }
}

use crate::env::banks::IntParam;
use crate::interner::ControlSequenceKind;
use crate::scaled::Scaled;
use crate::token_show::append_tex_print_char;
use crate::universe::{InteractionMode, Universe};
use crate::world::PrintSink;

/// tex.web §54's `max_print_line`.
pub const MAX_PRINT_LINE: usize = 79;

/// Removes the line breaks §58 inserted at [`MAX_PRINT_LINE`], leaving every
/// break the printer itself asked for.
///
/// §58 breaks a line the instant its column reaches the limit, wherever that
/// lands -- routinely mid-word. A caller comparing a message's *content*
/// rather than its layout reads printed text through this, so a wording
/// change that shifts the break point cannot silently turn a substring test
/// into a false negative. A caller whose subject is the layout reads the
/// printed bytes directly.
#[must_use]
pub fn without_line_breaks(text: &str) -> String {
    let mut unbroken = String::with_capacity(text.len());
    let mut column = 0usize;
    for character in text.chars() {
        if character == '\n' {
            if column != MAX_PRINT_LINE {
                unbroken.push('\n');
            }
            column = 0;
            continue;
        }
        unbroken.push(character);
        column += 1;
    }
    unbroken
}

/// tex.web §54's `selector` values that ordinary printing can hold.
///
/// The discriminants are tex.web's own, so `decr`/`incr` on a selector and
/// the `odd(selector)` and `selector>=log_only` tests are the module's
/// arithmetic rather than a re-derivation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum Selector {
    /// §54's `no_print=16`.
    NoPrint = 16,
    /// §54's `term_only=17`.
    TermOnly = 17,
    /// §54's `log_only=18`.
    LogOnly = 18,
    /// §54's `term_and_log=19`.
    TermAndLog = 19,
}

impl Selector {
    /// tex.web §§1262--1265's complete interaction/log-state matrix.
    #[must_use]
    pub const fn for_interaction_and_log(interaction: InteractionMode, log_opened: bool) -> Self {
        match (interaction, log_opened) {
            (InteractionMode::Batch, false) => Self::NoPrint,
            (InteractionMode::Batch, true) => Self::LogOnly,
            (_, false) => Self::TermOnly,
            (_, true) => Self::TermAndLog,
        }
    }

    /// tex.web §75's `<Initialize the print selector based on interaction>`
    /// followed by §1265's `if log_opened then selector:=selector+2`.
    /// Umber's transcript is always open, so the `+2` always applies.
    #[must_use]
    pub const fn for_interaction(interaction: InteractionMode) -> Self {
        Self::for_interaction_and_log(interaction, true)
    }

    /// The routed sink, or `None` for §54's `no_print`.
    #[must_use]
    pub const fn sink(self) -> Option<PrintSink> {
        match self {
            Self::NoPrint => None,
            Self::TermOnly => Some(PrintSink::Terminal),
            Self::LogOnly => Some(PrintSink::Log),
            Self::TermAndLog => Some(PrintSink::TerminalAndLog),
        }
    }

    /// tex.web's `decr(selector)`, saturating at §54's `no_print`.
    #[must_use]
    pub const fn decr(self) -> Self {
        match self {
            Self::NoPrint | Self::TermOnly => Self::NoPrint,
            Self::LogOnly => Self::TermOnly,
            Self::TermAndLog => Self::LogOnly,
        }
    }

    /// tex.web's `incr(selector)`, saturating at §54's `term_and_log`.
    #[must_use]
    pub const fn incr(self) -> Self {
        match self {
            Self::NoPrint => Self::TermOnly,
            Self::TermOnly => Self::LogOnly,
            Self::LogOnly | Self::TermAndLog => Self::TermAndLog,
        }
    }

    /// tex.web §62's `odd(selector)`.
    #[must_use]
    pub const fn writes_terminal(self) -> bool {
        matches!(self, Self::TermOnly | Self::TermAndLog)
    }

    /// tex.web §62's `selector>=log_only`.
    #[must_use]
    pub const fn writes_log(self) -> bool {
        matches!(self, Self::LogOnly | Self::TermAndLog)
    }
}

/// tex.web §§57-65's print primitives over one [`Selector`].
pub struct Printer<'a, G> {
    world: &'a mut crate::World,
    interaction_mode: &'a mut InteractionMode,
    newline_char: i32,
    escape_char: i32,
    max_print_line: usize,
    selector: Selector,
    generation: core::marker::PhantomData<fn(G) -> G>,
}

impl<'a, G> Printer<'a, G> {
    /// Opens a print scope routed by `selector`.
    pub fn new(universe: &'a mut Universe<G>, selector: Selector) -> Self {
        let newline_char = universe.int_param(IntParam::NEWLINE_CHAR);
        let escape_char = universe.int_param(IntParam::ESCAPE_CHAR);
        let max_print_line = universe.error_context_widths().max_print_line();
        Self::from_parts(
            &mut universe.world,
            &mut universe.interaction_mode,
            newline_char,
            escape_char,
            max_print_line,
            selector,
        )
    }

    pub(crate) const fn from_parts(
        world: &'a mut crate::World,
        interaction_mode: &'a mut InteractionMode,
        newline_char: i32,
        escape_char: i32,
        max_print_line: usize,
        selector: Selector,
    ) -> Self {
        Self {
            world,
            interaction_mode,
            newline_char,
            escape_char,
            max_print_line,
            selector,
            generation: core::marker::PhantomData,
        }
    }

    fn interaction_mode(&self) -> InteractionMode {
        *self.interaction_mode
    }

    fn set_interaction_mode(&mut self, mode: InteractionMode) {
        *self.interaction_mode = mode;
    }

    fn world_mut(&mut self) -> &mut crate::World {
        self.world
    }

    #[must_use]
    pub const fn selector(&self) -> Selector {
        self.selector
    }

    /// tex.web's `selector:=#`, for the scoped redirections in §71, §86,
    /// §90, §245, and §1298.
    pub const fn set_selector(&mut self, selector: Selector) {
        self.selector = selector;
    }

    /// tex.web §§59--60's content-based `print`/`slow_print` analogue.
    ///
    /// Umber carries string contents rather than string-pool numbers. Each
    /// eight-bit character therefore crosses §59's one-character-string
    /// path, as §60 `slow_print` does, while larger Unicode scalars pass
    /// through unchanged.
    pub fn print(&mut self, text: &str) -> &mut Self {
        let mut rendered = String::with_capacity(text.len());
        for character in text.chars() {
            if self.is_newline_character(character) {
                rendered.push('\n');
            } else {
                append_tex_print_char(character, &mut rendered);
            }
        }
        self.write_raw(&rendered)
    }

    /// tex.web §58's `print_char`.
    pub fn print_char(&mut self, character: char) -> &mut Self {
        if self.is_newline_character(character) {
            return self.print_ln();
        }
        let mut buffer = [0u8; 4];
        self.write_raw(character.encode_utf8(&mut buffer))
    }

    /// tex.web §59's `print(c)` for a one-character string.
    ///
    /// This differs deliberately from §58's [`Self::print_char`]: an active
    /// selector expands non-printable eight-bit character strings to TeX's
    /// `^^` spelling. The new-line test happens first, and the characters in
    /// the expanded spelling are then emitted with `new_line_char` disabled.
    pub fn print_character_string(&mut self, character: char) -> &mut Self {
        if self.is_newline_character(character) {
            return self.print_ln();
        }
        let mut rendered = String::new();
        append_tex_print_char(character, &mut rendered);
        self.write_raw(&rendered)
    }

    /// Writes text whose characters have already crossed TeX's print
    /// primitives, such as a completed `show_context` or token display.
    /// Embedded line feeds are physical `print_ln` results and must not be
    /// interpreted again through the live `new_line_char`.
    pub fn print_rendered(&mut self, text: &str) -> &mut Self {
        self.write_raw(text)
    }

    /// Writes bytes already encoded by the active command profile.
    pub fn print_encoded_bytes(&mut self, bytes: &[u8]) -> &mut Self {
        if let Some(sink) = self.selector.sink() {
            let max_print_line = self.max_print_line();
            self.world
                .write_encoded_bytes_with_line_limit(sink, bytes, max_print_line);
        }
        self
    }

    fn is_newline_character(&self, character: char) -> bool {
        u32::try_from(self.newline_char)
            .ok()
            .and_then(char::from_u32)
            == Some(character)
    }

    fn write_raw(&mut self, text: &str) -> &mut Self {
        if let Some(sink) = self.selector.sink() {
            let max_print_line = self.max_print_line();
            self.world
                .write_text_with_line_limit(sink, text, max_print_line);
        }
        self
    }

    /// tex.web §57's `print_ln`.
    pub fn print_ln(&mut self) -> &mut Self {
        self.write_raw("\n")
    }

    /// tex.web §62's `print_nl`: prints `text` at the start of a line.
    pub fn print_nl(&mut self, text: &str) -> &mut Self {
        if self.line_is_open() {
            self.print_ln();
        }
        self.print(text)
    }

    /// pdftex.web §65's `print_int` over Web2C's widened `longinteger`.
    pub fn print_int(&mut self, value: impl Into<i64>) -> &mut Self {
        self.print(&value.into().to_string())
    }

    /// pdftex.web §65's `print_two`: the last two digits of the absolute
    /// value, padded with a leading zero.
    pub fn print_two(&mut self, value: i32) -> &mut Self {
        let digits = value.unsigned_abs() % 100;
        self.print_char(char::from(b'0' + (digits / 10) as u8));
        self.print_char(char::from(b'0' + (digits % 10) as u8))
    }

    /// tex.web §64's `print_hex`: a quote followed by uppercase hexadecimal.
    pub fn print_hex(&mut self, value: u32) -> &mut Self {
        self.print_char('\'').print(&format!("{value:X}"))
    }

    /// tex.web §103's `print_scaled`. The unit, if any, is the caller's.
    pub fn print_scaled(&mut self, value: Scaled) -> &mut Self {
        self.print(&crate::scaled::print_scaled(value))
    }

    /// tex.web §63's `print_esc`: `\escapechar` followed by the name, with
    /// the escape omitted when `\escapechar` is outside `0..255`.
    pub fn print_esc(&mut self, name: &str) -> &mut Self {
        let escape = self.escape_char;
        if (0..256).contains(&escape)
            && let Some(character) = u32::try_from(escape).ok().and_then(char::from_u32)
        {
            self.print_character_string(character);
        }
        self.print(name)
    }

    /// TeX82 §263's `sprint_cs`.
    ///
    /// Active-character control sequences print as their character, escaped
    /// names use the live `\escapechar`, and §222's `null_cs` is the empty
    /// named spelling represented by `\csname\endcsname`.
    pub fn sprint_cs(&mut self, kind: ControlSequenceKind, name: &str) -> &mut Self {
        match (kind, name) {
            (ControlSequenceKind::ActiveCharacter, _) => self.print(name),
            (ControlSequenceKind::Null, _) => self.print_esc("csname").print_esc("endcsname"),
            (
                ControlSequenceKind::SingleCharacter
                | ControlSequenceKind::Named
                | ControlSequenceKind::Internal,
                _,
            ) => self.print_esc(name),
        }
    }

    /// tex.web §54's `term_offset`.
    #[must_use]
    pub fn terminal_offset(&self) -> usize {
        self.world
            .stream_bufs()
            .terminal_partial_line()
            .chars()
            .count()
    }

    /// tex.web §54's `file_offset`.
    #[must_use]
    pub fn log_offset(&self) -> usize {
        self.world.stream_bufs().log_partial_line().chars().count()
    }

    /// The process-selected tex.web §3 `max_print_line`.
    #[must_use]
    pub const fn max_print_line(&self) -> usize {
        self.max_print_line
    }

    /// tex.web §62's guard, `(term_offset>0)and(odd(selector))` or
    /// `(file_offset>0)and(selector>=log_only)`.
    fn line_is_open(&self) -> bool {
        (self.selector.writes_terminal() && self.terminal_offset() > 0)
            || (self.selector.writes_log() && self.log_offset() > 0)
    }
}

/// A recoverable error being reported: tex.web §73's `print_err` through
/// §82's `error`.
///
/// The message text is printed as the report is built; §79's help lines and
/// §1283's `use_err_help` are accumulated and shown by [`Self::error`].
#[must_use = "an opened error report must be completed with `error` or `int_error`"]
pub struct ErrorReport<'a, G> {
    printer: Printer<'a, G>,
    help: Vec<String>,
    err_help: Option<String>,
    context: Option<String>,
}

/// An error report paused between tex.web's message/help setup and §82's
/// `error`, allowing `back_error` to restore input in between.
pub struct DeferredErrorReport {
    selector: Selector,
    help: Vec<String>,
    err_help: Option<String>,
    context: Option<String>,
}

/// tex.web §81's `jump_out`: the non-local exit that abandons whatever was in
/// progress and runs `close_files_and_terminate` without `final_cleanup`.
///
/// The variants are the three sites that reach it through the error channel.
/// Each names what tex.web has *already printed* by the time it jumps, so a
/// caller propagating one must not print a second report for it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JumpOut {
    /// §82's `error_count=100`, after its
    /// `(That makes 100 errors; please try again.)` notice.
    TooManyErrors,
    /// §93's `fatal_error(s)`, reached from §71's `term_input` when §83's
    /// dialog prompts a terminal that has no line left. `help` is §93's `s`,
    /// the single help line its `Emergency stop` report carries.
    EmergencyStop { help: &'static str },
    /// §84's `X`, which prints nothing at all on its way out. `interaction`
    /// is already `scroll_mode` when this is returned.
    Quit,
}

/// The control-flow consequence of completing tex.web §82's `error`.
///
/// `#[must_use]` because the whole point of the value is the branch it
/// forces: §82's `exit` and its `jump_out` are not interchangeable, and a
/// dropped verdict silently turns the second into the first. That is what
/// left 55 of Umber's 58 error sites unable to end a job (`umber2-er8c`).
#[derive(Clone, Debug, Eq, PartialEq)]
#[must_use = "§82's `jump_out` branch must be propagated, not dropped"]
pub enum ErrorOutcome {
    Continue,
    Recovery(ErrorRecoveryRequest),
    JumpOut(JumpOut),
}

/// Input mutation requested by tex.web §§84/87's ErrorStop dialog.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ErrorRecoveryRequest {
    Delete(u8),
    Insert(String),
}

impl ErrorOutcome {
    /// Completes an executor-side report without giving the executor a second
    /// route to the command input stack.
    pub fn defer_recovery(
        self,
        effects: &mut crate::diagnostic::DiagnosticEffects,
    ) -> Result<(), JumpOut> {
        match self {
            Self::Continue => Ok(()),
            Self::Recovery(request) => {
                effects.request_error_stop_recovery(request);
                Ok(())
            }
            Self::JumpOut(jump) => Err(jump),
        }
    }
}

impl<'a, G> ErrorReport<'a, G> {
    fn begin(universe: &'a mut Universe<G>, text: &str) -> Self {
        let selector = Selector::for_interaction(universe.interaction_mode());
        Self::begin_with_selector(universe, text, selector)
    }

    fn begin_with_selector(universe: &'a mut Universe<G>, text: &str, selector: Selector) -> Self {
        // tex.web §73: `print_nl("! "); print(#)`.
        let mut printer = Printer::new(universe, selector);
        printer.print_nl("! ").print(text);
        Self {
            printer,
            help: Vec::new(),
            err_help: None,
            context: None,
        }
    }

    pub(crate) fn begin_from_parts(
        world: &'a mut crate::World,
        interaction_mode_slot: &'a mut InteractionMode,
        widths: ErrorContextWidths,
        newline_char: i32,
        escape_char: i32,
        text: &str,
    ) -> Self {
        let selector = Selector::for_interaction(*interaction_mode_slot);
        let mut printer = Printer::from_parts(
            world,
            interaction_mode_slot,
            newline_char,
            escape_char,
            widths.max_print_line(),
            selector,
        );
        printer.print_nl("! ").print(text);
        Self {
            printer,
            help: Vec::new(),
            err_help: None,
            context: None,
        }
    }

    pub(crate) fn bare_from_parts(
        world: &'a mut crate::World,
        interaction_mode_slot: &'a mut InteractionMode,
        widths: ErrorContextWidths,
        newline_char: i32,
        escape_char: i32,
    ) -> Self {
        let selector = Selector::for_interaction(*interaction_mode_slot);
        Self {
            printer: Printer::from_parts(
                world,
                interaction_mode_slot,
                newline_char,
                escape_char,
                widths.max_print_line(),
                selector,
            ),
            help: Vec::new(),
            err_help: None,
            context: None,
        }
    }

    pub(crate) fn resume_from_parts(
        world: &'a mut crate::World,
        interaction_mode_slot: &'a mut InteractionMode,
        widths: ErrorContextWidths,
        newline_char: i32,
        escape_char: i32,
        deferred: DeferredErrorReport,
    ) -> Self {
        Self {
            printer: Printer::from_parts(
                world,
                interaction_mode_slot,
                newline_char,
                escape_char,
                widths.max_print_line(),
                deferred.selector,
            ),
            help: deferred.help,
            err_help: deferred.err_help,
            context: deferred.context,
        }
    }

    pub(crate) fn continue_from_parts(
        world: &'a mut crate::World,
        interaction_mode_slot: &'a mut InteractionMode,
        widths: ErrorContextWidths,
        newline_char: i32,
        escape_char: i32,
        context: &str,
    ) -> ErrorOutcome {
        let selector = Selector::for_interaction(*interaction_mode_slot);
        let mut report = Self {
            printer: Printer::from_parts(
                world,
                interaction_mode_slot,
                newline_char,
                escape_char,
                widths.max_print_line(),
                selector,
            ),
            help: Vec::new(),
            err_help: None,
            context: None,
        };
        report.users_advice(Some(context))
    }

    /// tex.web §59's `print`, continuing the message text.
    pub fn print(&mut self, text: &str) -> &mut Self {
        self.printer.print(text);
        self
    }

    /// Writes text already rendered through TeX's print primitives.
    pub fn print_rendered(&mut self, text: &str) -> &mut Self {
        self.printer.print_rendered(text);
        self
    }

    /// tex.web §58's `print_char`.
    pub fn print_char(&mut self, character: char) -> &mut Self {
        self.printer.print_char(character);
        self
    }

    /// tex.web §68's `print_ASCII`, via the one-character string table.
    /// Eight-bit codes receive TeX's `^^` spelling when required; Unicode
    /// extension characters outside that table pass through unchanged.
    pub fn print_ascii(&mut self, character: char) -> &mut Self {
        self.printer.print_character_string(character);
        self
    }

    /// tex.web §62's `print_nl`, for a report whose message text spans more
    /// than one line (§288's `prepare_mag` is one).
    pub fn print_nl(&mut self, text: &str) -> &mut Self {
        self.printer.print_nl(text);
        self
    }

    /// tex.web §65's `print_int`.
    pub fn print_int(&mut self, value: i32) -> &mut Self {
        self.printer.print_int(value);
        self
    }

    /// tex.web §103's `print_scaled`.
    pub fn print_scaled(&mut self, value: Scaled) -> &mut Self {
        self.printer.print_scaled(value);
        self
    }

    /// tex.web §63's `print_esc`.
    pub fn print_esc(&mut self, name: &str) -> &mut Self {
        self.printer.print_esc(name);
        self
    }

    /// TeX82 §263's typed control-sequence renderer.
    pub fn sprint_cs(&mut self, kind: ControlSequenceKind, name: &str) -> &mut Self {
        self.printer.sprint_cs(kind, name);
        self
    }

    /// The selector this report prints through, for the scoped redirections
    /// tex.web performs mid-message (§1298's is the only one).
    #[must_use]
    pub const fn selector(&self) -> Selector {
        self.printer.selector()
    }

    /// tex.web's `selector:=#` inside an open report.
    pub const fn set_selector(&mut self, selector: Selector) {
        self.printer.set_selector(selector);
    }

    /// tex.web §79's `help1`..`help6`, in the order the lines are printed.
    pub fn help(&mut self, lines: &[&str]) -> &mut Self {
        self.help = lines.iter().map(|line| (*line).to_owned()).collect();
        self
    }

    /// tex.web §1283's `use_err_help:=true`, carrying the rendered
    /// `\errhelp` list §1284's `give_err_help` shows. Token rendering is the
    /// caller's, so this layer never inspects a token list.
    pub fn use_err_help(&mut self, rendered: String) -> &mut Self {
        self.err_help = Some(rendered);
        self
    }

    /// Supplies tex.web §82's already-rendered `show_context` display.
    pub fn context(&mut self, rendered: String) -> &mut Self {
        self.context = Some(rendered);
        self
    }

    /// Releases the live [`Universe`] borrow while retaining the report state
    /// needed by §82's `error`.
    #[must_use = "a deferred error report must be resumed and completed"]
    pub fn defer(self) -> DeferredErrorReport {
        DeferredErrorReport {
            selector: self.printer.selector(),
            help: self.help,
            err_help: self.err_help,
            context: self.context,
        }
    }

    /// tex.web §91's `int_error`.
    pub fn int_error(mut self, value: i32) -> ErrorOutcome {
        self.print(" (").print_int(value).print_char(')');
        self.error()
    }

    /// tex.web §82's `error`.
    pub fn error(mut self) -> ErrorOutcome {
        self.printer
            .world_mut()
            .error_channel_mut()
            .record_error_history();
        self.printer.print_char('.');
        // §82 prints `show_context` once here, and §93's `succumb` prints it
        // again from the nested `error` of an `Emergency stop` raised inside
        // the dialog below, so the rendering outlives this display.
        let context = self.context.take();
        if let Some(context) = &context {
            self.printer.print_rendered(context);
        }
        // §82: `if interaction=error_stop_mode then <Get user's advice and
        // return>`. There is no second conjunct. Umber used to require a
        // terminal line to be available too, which made an errorstop job with
        // an exhausted terminal fall through to the scrolled tail -- counting
        // an error tex.web does not count, printing help tex.web does not
        // print, and continuing past the point tex.web ends the job
        // (`umber2-er8c`).
        if self.printer.interaction_mode() == InteractionMode::ErrorStop {
            return self.users_advice(context.as_deref());
        }
        let error_count = self
            .printer
            .world_mut()
            .error_channel_mut()
            .record_scrolled_error();
        if error_count == 100 {
            self.printer
                .print_nl("(That makes 100 errors; please try again.)");
            self.printer
                .world_mut()
                .error_channel_mut()
                .record_fatal_history();
            return ErrorOutcome::JumpOut(JumpOut::TooManyErrors);
        }
        self.help_on_transcript();
        ErrorOutcome::Continue
    }

    /// tex.web §90's `<Put help message on the transcript file>`.
    fn help_on_transcript(&mut self) {
        let restore = self.printer.selector();
        let redirect = self.printer.interaction_mode() != InteractionMode::Batch;
        if redirect {
            self.printer.set_selector(restore.decr());
        }
        if let Some(rendered) = self.err_help.clone() {
            self.printer.print_ln().print_rendered(&rendered);
        } else {
            let lines = std::mem::take(&mut self.help);
            for line in &lines {
                self.printer.print_nl(line);
            }
        }
        self.printer.print_ln();
        if redirect {
            self.printer.set_selector(restore);
        }
        self.printer.print_ln();
    }

    /// tex.web §83's `<Get user's advice and return>` over the subset of
    /// §84's options that need no input stack.
    ///
    /// `context` is §82's already-rendered `show_context` display, kept so
    /// that an `Emergency stop` raised here can show the same stack §82 just
    /// showed, which is what tex.web's second `show_context` inside
    /// `succumb`'s nested `error` produces.
    ///
    /// Returns §81's `jump_out` when the dialog ends the job rather than
    /// returning to `error`'s caller.
    fn users_advice(&mut self, context: Option<&str>) -> ErrorOutcome {
        loop {
            if self.printer.interaction_mode() != InteractionMode::ErrorStop {
                return ErrorOutcome::Continue;
            }
            // §330's `clear_for_error_prompt`. Its `clear_terminal` flushes
            // pending terminal input, which Umber's line-oriented terminal
            // source has none of; its closing `print_ln` is unconditional
            // and is what separates the context display §82 just printed
            // from the prompt.
            self.printer.print_ln();
            self.printer.print("? ");
            // §71's `term_input`: `if not input_ln(term_in,true) then
            // fatal_error("End of file on the terminal!")`.
            let Some(line) = self.terminal_input() else {
                return ErrorOutcome::JumpOut(
                    self.fatal_error("End of file on the terminal!", context),
                );
            };
            let Some(first) = line.bytes().next() else {
                return ErrorOutcome::Continue;
            };
            let code = first.to_ascii_uppercase();
            match code {
                b'0'..=b'9' => {
                    let mut count = code - b'0';
                    if let Some(second @ b'0'..=b'9') = line.as_bytes().get(1).copied() {
                        count = count * 10 + second - b'0';
                    }
                    return ErrorOutcome::Recovery(ErrorRecoveryRequest::Delete(count));
                }
                b'I' => {
                    let insertion = if line.len() > 1 {
                        line[1..].to_owned()
                    } else {
                        self.printer.print("insert> ");
                        let Some(line) = self.terminal_input() else {
                            return ErrorOutcome::JumpOut(
                                self.fatal_error("End of file on the terminal!", context),
                            );
                        };
                        line
                    };
                    return ErrorOutcome::Recovery(ErrorRecoveryRequest::Insert(insertion));
                }
                // §89's `<Print the help information and goto continue>`.
                b'H' => self.show_help(),
                // §86's `<Change the interaction level and return>`.
                b'Q' | b'R' | b'S' => {
                    self.change_interaction(code);
                    return ErrorOutcome::Continue;
                }
                // §84's `X`: `interaction:=scroll_mode; jump_out`. It prints
                // nothing on the way out.
                b'X' => {
                    self.printer.set_interaction_mode(InteractionMode::Scroll);
                    return ErrorOutcome::JumpOut(JumpOut::Quit);
                }
                // §84's `othercases do_nothing`, then §85's menu.
                _ => self.show_menu(),
            }
        }
    }

    /// tex.web §93's `fatal_error(s)`, raised from inside §83's dialog:
    ///
    ///   normalize_selector; print_err("Emergency stop"); help1(s); succumb;
    fn fatal_error(&mut self, help: &'static str, context: Option<&str>) -> JumpOut {
        // §72's `normalize_selector`. `log_opened` is constantly true here
        // (see this module's header), so this only re-derives the selector
        // from the interaction mode still in force.
        let selector = Selector::for_interaction(self.printer.interaction_mode());
        self.printer.set_selector(selector);
        self.printer.print_nl("! ").print("Emergency stop");
        self.help = vec![help.to_owned()];
        self.err_help = None;
        self.succumb_with_context(context);
        JumpOut::EmergencyStop { help }
    }

    /// tex.web §93's `succumb`, completing a report that `fatal_error`,
    /// `overflow`, or `confusion` has already composed:
    ///
    ///   if interaction=error_stop_mode then interaction:=scroll_mode;
    ///   if log_opened then error; history:=fatal_error_stop; jump_out
    ///
    /// The nested `error` is what puts a second `show_context` display and
    /// the transcript-only help line in the transcript. Dropping to scroll
    /// mode *first* is what keeps that nested `error` from re-entering §83's
    /// dialog and prompting a user the job is in the middle of abandoning.
    ///
    /// There is no return value because there is no branch: `succumb` always
    /// reaches §81's `jump_out`. The caller already knows which terminal
    /// state it is raising and names it in its own error type.
    pub fn succumb(mut self) {
        let context = self.context.take();
        self.succumb_with_context(context.as_deref());
    }

    fn succumb_with_context(&mut self, context: Option<&str>) {
        // §93: `if interaction=error_stop_mode then interaction:=scroll_mode`.
        // Only from errorstop -- a batch or nonstop job keeps the mode it was
        // given, which is what §1335's own note then branches on.
        if self.printer.interaction_mode() == InteractionMode::ErrorStop {
            self.printer.set_interaction_mode(InteractionMode::Scroll);
        }
        self.printer
            .world_mut()
            .error_channel_mut()
            .record_error_history();
        self.printer.print_char('.');
        if let Some(context) = context {
            self.printer.print_rendered(context);
        }
        // §93 reaches `error` in scroll mode, so §82 takes its scrolled tail:
        // `incr(error_count)` and §90's transcript-only help. Whether that
        // increment happens to be the hundredth does not matter: both
        // branches end the job here, and §93's own history wins below.
        let _ = self
            .printer
            .world_mut()
            .error_channel_mut()
            .record_scrolled_error();
        self.help_on_transcript();
        self.printer
            .world_mut()
            .error_channel_mut()
            .record_fatal_history();
    }

    /// tex.web §71's `term_input`, including its echo to the transcript.
    fn terminal_input(&mut self) -> Option<String> {
        let line = self
            .printer
            .world_mut()
            .read_terminal_line()
            .ok()
            .flatten()?;
        let restore = self.printer.selector();
        self.printer.set_selector(restore.decr());
        self.printer.print(&line).print_ln();
        self.printer.set_selector(restore);
        Some(line)
    }

    /// tex.web §89's `<Print the help information and goto continue>`.
    fn show_help(&mut self) {
        if let Some(rendered) = self.err_help.take() {
            self.printer.print_rendered(&rendered).print_ln();
            return;
        }
        if self.help.is_empty() {
            self.help = vec![
                "Sorry, I don't know how to help in this situation.".into(),
                "Maybe you should try asking a human?".into(),
            ];
        }
        let lines = std::mem::take(&mut self.help);
        for line in &lines {
            self.printer.print(line).print_ln();
        }
        self.help = vec![
            "Sorry, I already gave what help I could...".into(),
            "Maybe you should try asking a human?".into(),
            "An error might have occurred before I noticed any problems.".into(),
            "``If all else fails, read the instructions.''".into(),
        ];
    }

    /// tex.web §86's `<Change the interaction level and return>`.
    fn change_interaction(&mut self, code: u8) {
        let (mode, name) = match code {
            b'Q' => (InteractionMode::Batch, "batchmode"),
            b'R' => (InteractionMode::Nonstop, "nonstopmode"),
            _ => (InteractionMode::Scroll, "scrollmode"),
        };
        self.printer
            .world_mut()
            .error_channel_mut()
            .clear_error_count();
        self.printer.set_interaction_mode(mode);
        self.printer.print("OK, entering ").print_esc(name);
        if mode == InteractionMode::Batch {
            let selector = self.printer.selector().decr();
            self.printer.set_selector(selector);
        }
        self.printer.print("...").print_ln();
    }

    /// tex.web §85's `<Print the menu of available options>`. Deletion and
    /// insertion are supported; only the editor handoff's `E` line is absent.
    fn show_menu(&mut self) {
        self.printer
            .print("Type <return> to proceed, S to scroll future error messages,")
            .print_nl("R to run without stopping, Q to run quietly,")
            .print_nl("I to insert something,")
            .print_nl("1 or ... or 9 to ignore the next 1 to 9 tokens of input,")
            .print_nl("H for help, X to quit.");
    }
}

/// Mutable state tex.web keeps for the error channel across errors.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ErrorChannel {
    error_count: i32,
    long_help_seen: bool,
    history: ErrorHistory,
}

/// tex.web §76's ordered `history` severities.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub enum ErrorHistory {
    #[default]
    Spotless,
    WarningIssued,
    ErrorMessageIssued,
    FatalErrorStop,
}

impl ErrorChannel {
    pub(crate) fn reachable_state_identity(&self) -> u64 {
        crate::state_hash::semantic_scalar_root(0x6572_726f_725f_6368, |hasher| {
            hasher.i32(self.error_count);
            hasher.bool(self.long_help_seen);
            hasher.u8(match self.history {
                ErrorHistory::Spotless => 0,
                ErrorHistory::WarningIssued => 1,
                ErrorHistory::ErrorMessageIssued => 2,
                ErrorHistory::FatalErrorStop => 3,
            });
        })
    }

    /// tex.web §82's `incr(error_count)`, returning the incremented count so
    /// the report owner can perform the 100-error terminal transition.
    pub const fn record_scrolled_error(&mut self) -> i32 {
        self.error_count = self.error_count.saturating_add(1);
        self.error_count
    }

    /// tex.web §82's `history:=error_message_issued` monotonic transition.
    pub const fn record_error_history(&mut self) {
        if matches!(
            self.history,
            ErrorHistory::Spotless | ErrorHistory::WarningIssued
        ) {
            self.history = ErrorHistory::ErrorMessageIssued;
        }
    }

    /// tex.web §82's 100-error `history:=fatal_error_stop`.
    pub const fn record_fatal_history(&mut self) {
        self.history = ErrorHistory::FatalErrorStop;
    }

    /// tex.web §76's `history:=warning_issued`, raised by the non-error
    /// warnings (§660's overfull-box reports, §1298's `\show` family under
    /// `batch_mode`) rather than by `error`.
    ///
    /// No engine site raises it yet, so `history` reaches this level only
    /// when a caller asks for it. It is declared here because §1335's
    /// end-of-job note (`tex_exec::job`'s `print_history_note`) branches on
    /// `history=warning_issued` specifically, and a transition the model
    /// cannot represent is a branch nothing can exercise.
    pub const fn record_warning_history(&mut self) {
        if matches!(self.history, ErrorHistory::Spotless) {
            self.history = ErrorHistory::WarningIssued;
        }
    }

    #[must_use]
    pub const fn history(&self) -> ErrorHistory {
        self.history
    }

    /// tex.web §86's `error_count:=0`, also §1054's paragraph reset.
    pub const fn clear_error_count(&mut self) {
        self.error_count = 0;
    }

    /// tex.web §76's `error_count`.
    #[must_use]
    pub const fn error_count(&self) -> i32 {
        self.error_count
    }

    /// tex.web §1281's `long_help_seen`, returning whether the long
    /// `\errmessage` help has already been given and marking it seen.
    pub const fn take_long_help_seen(&mut self, mark: bool) -> bool {
        let seen = self.long_help_seen;
        if mark {
            self.long_help_seen = true;
        }
        seen
    }
}

impl<G> Universe<G> {
    /// pdftex.web §73's `print_ignored_err`: an original-TeX error that
    /// pdfTeX deliberately records only in the transcript, without the word
    /// `error` that humans and tooling treat specially.
    pub fn print_ignored_err(&mut self, text: &str) {
        Printer::new(self, Selector::LogOnly)
            .print_ln()
            .print("ignored: ")
            .print(text);
    }

    /// tex.web §73's `print_err`, opening a recoverable-error report.
    pub fn print_err(&mut self, text: &str) -> ErrorReport<'_, G> {
        ErrorReport::begin(self, text)
    }

    /// Opens §73's report with the selector implied by the transcript
    /// lifecycle as well as the interaction mode.
    ///
    /// Canonical startup owns the short interval before §1335 opens the log.
    /// Keeping that lifecycle explicit here lets it report a fatal startup
    /// error without pretending the transcript already exists; the selector
    /// remains scoped to the returned report and later ordinary printing uses
    /// the normal live-transcript selector again.
    pub fn print_err_with_transcript_state(
        &mut self,
        text: &str,
        transcript_open: bool,
    ) -> ErrorReport<'_, G> {
        let selector = Selector::for_interaction_and_log(self.interaction_mode(), transcript_open);
        ErrorReport::begin_with_selector(self, text, selector)
    }

    /// tex.web §82's `error` for a message printed *without* §73's
    /// `print_err`.
    ///
    /// §1293's `\show` completion is the one such site: §1294 and §1297 print
    /// their `>␣` line through the ordinary print routines and then reach
    /// `common_ending`'s bare `error`.
    pub fn error_report(&mut self) -> ErrorReport<'_, G> {
        let selector = Selector::for_interaction(self.interaction_mode());
        ErrorReport {
            printer: Printer::new(self, selector),
            help: Vec::new(),
            err_help: None,
            context: None,
        }
    }

    /// Resumes a report paused by [`ErrorReport::defer`].
    pub fn resume_error_report(&mut self, deferred: DeferredErrorReport) -> ErrorReport<'_, G> {
        ErrorReport {
            printer: Printer::new(self, deferred.selector),
            help: deferred.help,
            err_help: deferred.err_help,
            context: deferred.context,
        }
    }

    /// Re-enters tex.web §83 after §84 deleted tokens and displayed the new
    /// input context.
    pub fn continue_error_stop_dialog(&mut self, context: &str) -> ErrorOutcome {
        let selector = Selector::for_interaction(self.interaction_mode());
        let mut report = ErrorReport {
            printer: Printer::new(self, selector),
            help: Vec::new(),
            err_help: None,
            context: None,
        };
        report.users_advice(Some(context))
    }

    /// An ordinary print scope at the §75 selector the current interaction
    /// mode implies.
    pub fn printer(&mut self) -> Printer<'_, G> {
        let selector = Selector::for_interaction(self.interaction_mode());
        Printer::new(self, selector)
    }
}

#[cfg(test)]
mod tests;
