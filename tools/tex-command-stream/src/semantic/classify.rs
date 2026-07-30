//! Classifies a channel's divergence from the pinned oracle by the *shape* of
//! the first oracle line Umber's output does not reproduce, mapping that
//! shape to the Beads bug that already names it.
//!
//! This replaces a guess. Every `xfail` channel's `bug` id used to be chosen
//! by hand at migration time, and an audit found it wrong for 20 of the 48
//! channels then pinned on `umber2-alfh.13` ("Umber raises no error at all"):
//! some were really Umber closing a file's `)` early (`umber2-alfh.25`), and
//! others were Umber raising a *different* error than the oracle, or an
//! error where the oracle raises none at all -- the direction `.13` cannot
//! describe (`umber2-alfh.26`).
//!
//! Two entry points, deliberately different in how much of the divergence
//! they look at:
//!
//! - [`reclassify_no_error_channel`] is the fix for the audited bug: it
//!   re-tests a channel *already* pinned on `.13` using only the one
//!   positional first-line mismatch the corpus already commits for it
//!   (`ChannelMismatch`, the same fingerprint `first_line_difference`
//!   produces). That is deliberate, not a simplification: `.13`'s claim is
//!   specifically about what differs *at that exact position*, and a deeper
//!   scan routinely "explains" the divergence by finding some later,
//!   unrelated line instead -- an already-correctly-labeled `Underfull
//!   \hbox` diagnostic several lines further down, or a `! Emergency stop.`
//!   both sides share at the very end of the transcript -- which names the
//!   wrong bug just as confidently as the guess it replaces. Restricting the
//!   check to the one position `.13` actually pins is what keeps every
//!   already-correct channel (`umber2-alfh.11`/`.14`/`.15`/`.16`/`.17`-`.24`)
//!   untouched: [`command-semantic-channels`](../../bin/command-semantic-channels.rs)
//!   calls this function only when a channel's *currently declared* bug is
//!   `.13`, never for any other already-established label.
//! - [`classify_divergence`] is the general-purpose classifier for a
//!   channel with *no* existing declaration at all (a brand-new divergence).
//!   With no established label at risk of being disturbed, it scans every
//!   line the oracle produced that Umber's did not (mirroring
//!   `difflib.ndiff`'s alignment rather than a positional walk, so a shifted
//!   context block is not misattributed to whatever ordinary text happens to
//!   have moved into that position) and matches the first one that names a
//!   known shape.
//!
//! Both **refuse rather than guess**: neither invents a label when nothing
//! matches, returning `None` instead so the caller preserves whatever is
//! already declared (correct for a genuine one-off,
//! `umber2-alfh.17`-`.21`/`.24`, each a single, narrow, already-hand-diagnosed
//! divergence with no shared shape to generalize) or -- for a genuinely new
//! divergence with nothing declared at all -- still requires a human to
//! decide.
//!
//! The `dvi` channel is never classified: its content is binary, so the
//! line-shape scans below cannot read it, and it no longer has a recurring
//! shape worth a byte-level rule. It used to have exactly one -- the DVI
//! preamble banner comment (`umber2-alfh.22`) -- which turned out not to be
//! a divergence at all but a byte range the repository already held
//! uncomparable, and is now normalized away by
//! `channels::normalize_channel`. Every `dvi` divergence that survives that
//! normalization is a real, individual defect, so it gets the same treatment
//! as any other unclassifiable shape: the declared bug if one exists, and
//! otherwise a human.

use similar::{ChangeTag, TextDiff};

use super::{ChannelMismatch, StreamChannel};

/// One known divergence shape, naming the Beads bug that already describes
/// it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DivergenceClass {
    /// The two profiles built past INITEX (`etex-loaded`, `production`) are
    /// not framed as a real loaded-format job: the oracle's first line is its
    /// `This is pdfTeX` banner and Umber's channel has nothing there at all.
    EtexLoadedFraming,
    /// A `show_context` (tex.web section 313) block the oracle prints is
    /// absent or positioned differently in Umber's own.
    ShowContextAccuracy,
    /// An `Underfull`/`Overfull`/`Loose`/`Tight` `\hbox`/`\vbox` diagnostic
    /// the oracle prints that Umber does not yet.
    BoxDiagnostics,
    /// The oracle wraps a long diagnostic across two physical lines; Umber
    /// prints the same text unwrapped on one line, so Umber's corresponding
    /// line starts with the oracle's wrapped line and keeps going.
    DiagnosticNeverWraps,
    /// The oracle's line is a bare file open (`(./name.tex` with nothing
    /// else on the line, not yet closed); Umber's corresponding line closes
    /// the paren early, with the content that should intervene missing.
    FileParenClosedEarly,
    /// TeX82 section 362's `*` prompt at terminal exhaustion, which the
    /// oracle prints and Umber omits.
    TerminalPromptOmitted,
    /// The oracle raises a `! `-prefixed error and Umber's channel raises no
    /// error anywhere at all.
    UmberRaisesNoError,
    /// Umber raises an error the oracle does not raise at all, or raises a
    /// *different* error than the oracle's -- the direction
    /// [`Self::UmberRaisesNoError`] cannot describe.
    UmberRaisesUnexpectedError,
}

impl DivergenceClass {
    /// The Beads bug this shape is already filed and tracked under.
    #[must_use]
    pub const fn bug(self) -> &'static str {
        match self {
            Self::EtexLoadedFraming => "umber2-alfh.15",
            Self::ShowContextAccuracy => "umber2-alfh.14",
            Self::BoxDiagnostics => "umber2-alfh.16",
            Self::DiagnosticNeverWraps => "umber2-alfh.23",
            Self::FileParenClosedEarly => "umber2-alfh.25",
            Self::TerminalPromptOmitted => "umber2-alfh.11",
            Self::UmberRaisesNoError => "umber2-alfh.13",
            Self::UmberRaisesUnexpectedError => "umber2-alfh.26",
        }
    }
}

/// Re-tests a channel currently declared `xfail` on `umber2-alfh.13` against
/// its own pinned positional mismatch, and returns the class that actually
/// explains it when that differs from `.13`'s own claim.
///
/// Deliberately shallow (see this module's doc): it looks at nothing but the
/// one `expected`/`actual` line pair already committed for this channel, so
/// it can never "explain" the divergence by finding a real but unrelated
/// line elsewhere in the transcript. `.13` claims the oracle raises a `! `
/// error at this exact position and Umber raises none; this function checks
/// that claim directly, plus the one other shape sharing its usual
/// position -- a bare, not-yet-closed file open the oracle printed here,
/// with Umber's line already closing it.
#[must_use]
pub fn reclassify_no_error_channel(mismatch: &ChannelMismatch) -> Option<DivergenceClass> {
    if is_bare_file_open(&mismatch.expected) && mismatch.actual.contains(')') {
        return Some(DivergenceClass::FileParenClosedEarly);
    }
    let oracle_is_error = mismatch.expected.starts_with("! ");
    let actual_is_error = mismatch.actual.starts_with("! ");
    if oracle_is_error || actual_is_error {
        return Some(if actual_is_error {
            DivergenceClass::UmberRaisesUnexpectedError
        } else {
            DivergenceClass::UmberRaisesNoError
        });
    }
    None
}

/// Classifies one channel's divergence from the oracle, or refuses (`None`)
/// when no known shape matches. `oracle` and `umber` are that channel's
/// complete bytes (already clock-normalized for the `log` channel, exactly
/// as committed/captured).
#[must_use]
pub fn classify_divergence(
    channel: StreamChannel,
    oracle: &[u8],
    umber: &[u8],
) -> Option<DivergenceClass> {
    if channel == StreamChannel::Dvi {
        return None;
    }
    let oracle_text = String::from_utf8_lossy(oracle);
    let umber_text = String::from_utf8_lossy(umber);

    // A channel the oracle produced *nothing* for has no oracle-only line
    // for the diff below to find -- there is nothing to delete -- so an
    // unexpected Umber error here must be caught before that scan, not by it.
    if oracle_text.trim().is_empty() && first_line_with_prefix(&umber_text, "! ").is_some() {
        return Some(DivergenceClass::UmberRaisesUnexpectedError);
    }

    let diff = TextDiff::from_lines(oracle_text.as_ref(), umber_text.as_ref());
    let lines = oracle_only_lines(&diff);

    // Priority is by *document order*, not by rule: the first oracle-only
    // line that matches any specific shape wins, exactly as
    // `show_context`'s own block always follows the error it belongs to, so
    // an error on line 1 must be found before a context header on line 2
    // ever gets a vote. `FileParenClosedEarly` is the one exception, checked
    // only afterward and only against the *first* oracle-only line: a
    // missing block (a diagnostic, a context header, an error) pushes every
    // later oracle-only line down, and one of those later, otherwise
    // ordinary lines routinely happens to itself look like a bare,
    // not-yet-closed file open once the real missing block is found and
    // named by the loop above -- so the generic "closes early" shape is only
    // the answer when nothing more specific explains the divergence at all.
    for &(line, paired) in &lines {
        if let Some(class) = classify_specific_line(line, paired) {
            return Some(class);
        }
    }
    if let Some(&(line, paired)) = lines.first()
        && is_bare_file_open(line)
        && paired.is_some_and(|candidate| candidate.contains(')'))
    {
        return Some(DivergenceClass::FileParenClosedEarly);
    }
    None
}

/// Matches one oracle-only line (and its possible paired replacement in
/// Umber's output) against every shape but [`DivergenceClass::FileParenClosedEarly`],
/// which [`classify_divergence`] only ever considers separately, and only
/// against the first oracle-only line. `None` means this line does not name
/// a known class; the caller moves on to the next oracle-only line rather
/// than treating this line as the corpus's final answer.
fn classify_specific_line(oracle_line: &str, paired: Option<&str>) -> Option<DivergenceClass> {
    if is_show_context_header(oracle_line) {
        return Some(DivergenceClass::ShowContextAccuracy);
    }
    if is_box_diagnostic(oracle_line) {
        return Some(DivergenceClass::BoxDiagnostics);
    }
    if oracle_line.starts_with("This is pdfTeX") {
        return Some(DivergenceClass::EtexLoadedFraming);
    }
    if oracle_line.starts_with("! ")
        && paired
            .is_some_and(|line| line.starts_with(oracle_line) && line.len() > oracle_line.len())
    {
        return Some(DivergenceClass::DiagnosticNeverWraps);
    }
    if oracle_line == "*" || paired.is_some_and(|line| line.starts_with("*(")) {
        return Some(DivergenceClass::TerminalPromptOmitted);
    }
    // Whether Umber replaces this exact line with an error of its own --
    // not whether an error appears *anywhere* in Umber's output, which
    // would also match unrelated shared boilerplate (both sides can well
    // share a terminal-exhaustion "! Emergency stop." tail that has nothing
    // to do with this particular missing line).
    if oracle_line.starts_with("! ") || paired.is_some_and(|line| line.starts_with("! ")) {
        return Some(if paired.is_some_and(|line| line.starts_with("! ")) {
            DivergenceClass::UmberRaisesUnexpectedError
        } else {
            DivergenceClass::UmberRaisesNoError
        });
    }
    None
}

/// The first line of `text` starting with `prefix`, if any.
fn first_line_with_prefix<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.lines().find(|line| line.starts_with(prefix))
}

/// Every line present in the oracle's text but not Umber's, in document
/// order, each paired with its positional counterpart in the same "replace"
/// run of Umber's diff (its likely "replacement"), when one exists.
///
/// Uses a real line-alignment diff (`similar`, mirroring `difflib.ndiff`)
/// rather than a positional walk: a context block the oracle prints and
/// Umber omits shifts every later line down by that many positions, so a
/// positional first-differing-line comparison would misattribute the
/// divergence to whatever ordinary text happens to have moved into that
/// slot instead of to the missing block itself. A run of consecutive
/// deletions immediately followed by a run of consecutive insertions (a
/// "replace" in `difflib`'s vocabulary) pairs its `k`-th deletion with the
/// `k`-th insertion when both exist, exactly as `difflib.ndiff`'s own
/// grouping does; `similar` emits every deletion in a replace run before any
/// of its insertions, so pairing by position within each run -- rather than
/// by adjacency in the flat change list -- is what recovers that alignment.
fn oracle_only_lines<'a>(diff: &TextDiff<'a, 'a, '_, str>) -> Vec<(&'a str, Option<&'a str>)> {
    let changes: Vec<_> = diff.iter_all_changes().collect();
    let mut result = Vec::new();
    let mut index = 0;
    while index < changes.len() {
        if changes[index].tag() != ChangeTag::Delete {
            index += 1;
            continue;
        }
        let delete_start = index;
        let mut delete_end = delete_start;
        while delete_end < changes.len() && changes[delete_end].tag() == ChangeTag::Delete {
            delete_end += 1;
        }
        let insert_start = delete_end;
        let mut insert_end = insert_start;
        while insert_end < changes.len() && changes[insert_end].tag() == ChangeTag::Insert {
            insert_end += 1;
        }
        let insert_count = insert_end - insert_start;
        for (offset, change) in changes[delete_start..delete_end].iter().enumerate() {
            let paired = (offset < insert_count).then(|| {
                changes[insert_start + offset]
                    .value()
                    .trim_end_matches(['\n', '\r'])
            });
            result.push((change.value().trim_end_matches(['\n', '\r']), paired));
        }
        index = insert_end;
    }
    result
}

/// tex.web section 313's `show_context` block headers: the pseudo-file
/// `l.<N>` line, every named token-list-source header, and a macro's own
/// `\name ->` replacement-text header.
fn is_show_context_header(line: &str) -> bool {
    const PREFIXES: [&str; 17] = [
        "<recently read>",
        "<to be read again>",
        "<inserted text>",
        "<argument>",
        "<template>",
        "<output>",
        "<everypar>",
        "<everymath>",
        "<everydisplay>",
        "<everyhbox>",
        "<everyvbox>",
        "<everyjob>",
        "<everycr>",
        "<mark>",
        "<write>",
        "<read ",
        "<insert>",
    ];
    if PREFIXES.iter().any(|prefix| line.starts_with(prefix)) || line == "<*>" {
        return true;
    }
    if let Some(rest) = line.strip_prefix("l.")
        && rest.starts_with(|character: char| character.is_ascii_digit())
    {
        return true;
    }
    // A macro replacement-text header: `\name ->...`. `name` itself may not
    // contain a space, so the first ` ->` after the control sequence is it.
    if let Some(name_end) = line.strip_prefix('\\').and_then(|rest| rest.find(" ->")) {
        return !line[1..=name_end].contains(' ');
    }
    false
}

fn is_box_diagnostic(line: &str) -> bool {
    ["Underfull", "Overfull", "Loose", "Tight"]
        .iter()
        .any(|kind| {
            line.starts_with(kind)
                && (line[kind.len()..].starts_with(" \\hbox")
                    || line[kind.len()..].starts_with(" \\vbox"))
        })
}

/// A bare, not-yet-closed file open: `(./name.tex` with no `)` on the line.
fn is_bare_file_open(line: &str) -> bool {
    line.starts_with("(./") && !line.contains(')')
}

#[cfg(test)]
mod tests;
