//! tex.web §§310--318's `show_context` display.
//!
//! §82's `error` prints the location lines (`l.4␣\spacefactor`,
//! `<to be read again>␣`, `<inserted text>␣`) after the message's closing
//! period, from the live input stack. The command core selects exactly the
//! levels §310 displays while it walks that stack; this module owns only the
//! shared §316--§318 pseudoprint arithmetic for each selected level.

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
    before_chars: usize,
    after_chars: usize,
}

impl ErrorContextLevel {
    #[must_use]
    pub fn new(
        label: impl Into<String>,
        before: impl Into<String>,
        after: impl Into<String>,
    ) -> Self {
        let label = label.into();
        let before = before.into();
        let after = after.into();
        let before_chars = before.chars().count();
        let after_chars = after.chars().count();
        Self {
            label,
            before,
            after,
            before_chars,
            after_chars,
        }
    }

    /// Builds one selected level from a bounded pseudoprint projection.
    ///
    /// `before` is the retained tail of a read half whose full character count
    /// is `before_chars`. `after` is the retained head of an unread half whose
    /// capped character count is `after_chars`; callers need only count one
    /// character beyond the largest possible displayed window.
    #[must_use]
    pub fn from_bounded_projection(
        label: impl Into<String>,
        before: impl Into<String>,
        before_chars: usize,
        after: impl Into<String>,
        after_chars: usize,
    ) -> Self {
        Self {
            label: label.into(),
            before: before.into(),
            after: after.into(),
            before_chars,
            after_chars,
        }
    }

    /// §318's `<Print two lines using the tricky pseudoprinted information>`.
    ///
    /// §318 prints the descriptive label *before* §316's `begin_pseudoprint`
    /// starts measuring, so `l` is the label's own width and the `"..."` that
    /// crops line 1 lands after it. The label therefore always survives
    /// truncation: the reference prints `l.11 ...box0=\vbox{`, never
    /// `... \setbox0=`.
    pub fn render_into(&self, widths: ErrorContextWidths, output: &mut String) {
        let label_width = self.label.chars().count();
        let read = self.before_chars;
        // §318's `if l+first_count<=half_error_line`, whose else branch fixes
        // line 1's width `n` at `half_error_line` rather than at whatever the
        // cropped text happens to measure.
        output.push('\n');
        output.push_str(&self.label);
        let indent = if label_width + read <= widths.half_error_line() {
            output.push_str(&self.before);
            label_width + read
        } else {
            let kept = widths
                .half_error_line()
                .saturating_sub(label_width)
                .saturating_sub(3);
            // A bounded projection has already discarded the prefix before
            // `self.before`. Translate the full pseudoprint offset into that
            // retained tail before cropping to §318's final window.
            let retained = self.before.chars().count();
            let omitted = read.saturating_sub(retained);
            let skip = read.saturating_sub(kept).saturating_sub(omitted);
            output.push_str("...");
            output.extend(self.before.chars().skip(skip));
            widths.half_error_line()
        };
        // §318's `if m+n<=error_line then p:=first_count+m else
        // p:=first_count+(error_line-n-3)`, then its trailing `print("...")`.
        let unread = self.after_chars;
        let available = widths.error_line().saturating_sub(indent);
        output.push('\n');
        output.extend(std::iter::repeat_n(' ', indent));
        if unread <= available {
            output.push_str(&self.after);
        } else {
            output.extend(self.after.chars().take(available.saturating_sub(3)));
            output.push_str("...");
        }
    }
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
