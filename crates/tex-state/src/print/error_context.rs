//! tex.web §§310--318's `show_context` display.
//!
//! §82's `error` prints the location lines (`l.4␣\spacefactor`,
//! `<to be read again>␣`, `<inserted text>␣`) after the message's closing
//! period, from the live input stack. Two crates own an input stack -- the
//! canonical command core and the gullet's replay stack -- so each projects
//! its own levels into [`ErrorContextLevel`], and the pseudoprint arithmetic
//! §316--§318 describes lives here exactly once.
//!
//! What a projection owes this module:
//!
//! - Levels ordered innermost first, so `levels[0]` is §312's `base_ptr =
//!   input_ptr` current level and the last entry is §310's `bottom_line`.
//!   §310 stops at the first real file level, so a projection that can see
//!   file identity must truncate there rather than pass the whole stack.
//! - §312's omission already applied: a `backed_up` list that is not the
//!   current level and has been read to its end is not displayed at all.
//!   [`token_list_replay_label`] still spells the current one
//!   `<recently read>␣`.

use super::ErrorContextWidths;
use crate::input::TokenListReplayKind;

/// One §312 "Display the current context" entry.
///
/// `before` and `after` are the pseudoprinted halves §316 gathers on either
/// side of the level's read position; `label` is §313/§314's descriptive
/// prefix, including the trailing space both sections print.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ErrorContextLevel {
    label: String,
    before: String,
    after: String,
}

impl ErrorContextLevel {
    #[must_use]
    pub fn new(
        label: impl Into<String>,
        before: impl Into<String>,
        after: impl Into<String>,
    ) -> Self {
        Self {
            label: label.into(),
            before: before.into(),
            after: after.into(),
        }
    }

    /// §318's `<Print two lines using the tricky pseudoprinted information>`.
    ///
    /// §318 prints the descriptive label *before* §316's `begin_pseudoprint`
    /// starts measuring, so `l` is the label's own width and the `"..."` that
    /// crops line 1 lands after it. The label therefore always survives
    /// truncation: the reference prints `l.11 ...box0=\vbox{`, never
    /// `... \setbox0=`.
    fn render(&self, widths: ErrorContextWidths) -> String {
        let label_width = self.label.chars().count();
        let read = self.before.chars().count();
        // §318's `if l+first_count<=half_error_line`, whose else branch fixes
        // line 1's width `n` at `half_error_line` rather than at whatever the
        // cropped text happens to measure.
        let (line, indent) = if label_width + read <= widths.half_error_line() {
            (format!("{}{}", self.label, self.before), label_width + read)
        } else {
            let kept = widths
                .half_error_line()
                .saturating_sub(label_width)
                .saturating_sub(3);
            (
                format!(
                    "{}...{}",
                    self.label,
                    self.before
                        .chars()
                        .skip(read.saturating_sub(kept))
                        .collect::<String>()
                ),
                widths.half_error_line(),
            )
        };
        // §318's `if m+n<=error_line then p:=first_count+m else
        // p:=first_count+(error_line-n-3)`, then its trailing `print("...")`.
        let unread = self.after.chars().count();
        let available = widths.error_line().saturating_sub(indent);
        let rest = if unread <= available {
            self.after.clone()
        } else {
            format!(
                "{}...",
                self.after
                    .chars()
                    .take(available.saturating_sub(3))
                    .collect::<String>()
            )
        };
        format!("\n{line}\n{}{rest}", " ".repeat(indent))
    }
}

/// §310's `show_context` loop over already-projected levels.
///
/// `error_context_lines` is `\errorcontextlines` unclamped: §310 omits its
/// `...` elision marker entirely when the parameter is negative, which a
/// `usize` conversion would silently turn into "elide everything".
#[must_use]
pub fn render_error_context(
    levels: &[ErrorContextLevel],
    widths: ErrorContextWidths,
    error_context_lines: i32,
) -> String {
    let bottom = levels.len().saturating_sub(1);
    // §310's `nn`, which counts only the levels actually displayed, so an
    // omitted level never consumes part of the `\errorcontextlines` budget.
    let mut shown: i32 = -1;
    let mut output = String::new();
    for (index, level) in levels.iter().enumerate() {
        if index == 0 || index == bottom || shown < error_context_lines {
            output.push_str(&level.render(widths));
            shown = shown.saturating_add(1);
        } else if shown == error_context_lines {
            output.push_str("\n...");
            shown = shown.saturating_add(1);
        }
    }
    output
}

/// §314's `<Print type of token list>`.
///
/// `exhausted` is §314's `loc=null` test, which only the backed-up family
/// consults: `back_input`'s list reads `<to be read again>␣` while the token
/// it holds is still unread and `<recently read>␣` once it has been consumed.
/// Every other `token_type` has a single spelling.
#[must_use]
pub const fn token_list_replay_label(kind: TokenListReplayKind, exhausted: bool) -> &'static str {
    match kind {
        TokenListReplayKind::MacroBody => "<macro> ",
        TokenListReplayKind::MacroArgument => "<argument> ",
        // §369's `\noexpand` level is a `backed_up` list too, so both
        // spellings apply to it exactly as they do to `back_input`'s.
        TokenListReplayKind::NoExpand | TokenListReplayKind::BackedUp => {
            if exhausted {
                "<recently read> "
            } else {
                "<to be read again> "
            }
        }
        TokenListReplayKind::Unexpanded => "<unexpanded> ",
        TokenListReplayKind::EveryPar => "<everypar> ",
        TokenListReplayKind::EveryHBox => "<everyhbox> ",
        TokenListReplayKind::EveryVBox => "<everyvbox> ",
        TokenListReplayKind::EveryJob => "<everyjob> ",
        TokenListReplayKind::EveryCr => "<everycr> ",
        TokenListReplayKind::Mark => "<mark> ",
        TokenListReplayKind::OutputRoutine => "<output> ",
        TokenListReplayKind::Inserted => "<inserted text> ",
        TokenListReplayKind::ScantokensEveryEof => "<everyeof> ",
        TokenListReplayKind::AlignmentUTemplate | TokenListReplayKind::AlignmentVTemplate => {
            "<template> "
        }
    }
}
