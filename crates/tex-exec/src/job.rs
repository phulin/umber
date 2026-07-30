//! TeX's job framing: the start-up banner, the per-file `(name`/`)`
//! bracketing, and the tail every job ends with.
//!
//! See `docs/job_framing.md` for the full account of what a job prints, when,
//! and why the pieces live where they do. In short: printing is
//! `tex-state`'s (`tex_state::print::Printer` over §54's `selector`), file
//! opening is `tex-command`'s (§537's `start_input` is an input-stack
//! operation, queued as a drained [`tex_command::FileFramingEvent`] rather
//! than printed), and the job lifecycle -- when a job starts, when a file's
//! open/close reaches the transcript, and how a job ends -- is
//! `CanonicalMainControl`'s, because it is the one layer that sees a
//! `Universe` and a driver both. This module is where that lifecycle lives;
//! `canonical_main_control.rs` only exposes it.
//!
//! tex.web spreads the pieces this module owns across:
//! - §61 `wterm(banner)` and §536 `open_log_file`: the start-up banner.
//! - etex.ch's patches at tex.web §536 (log) and §1337 (terminal): e-TeX's
//!   "entering extended mode" notice, printed on both channels immediately
//!   after the banner and before §534's `**` line.
//! - §534: the `**` line that echoes the job's first input line.
//! - §537 `start_input` and §362: `(name` and `)` around each opened file.
//! - §1335 `final_cleanup`: closing every still-open paren, reporting
//!   unfinished conditionals, the "(see the transcript file..." note, and
//!   the `\dump`-outside-INITEX note.
//! - §1333 `close_files_and_terminate`: §642's DVI page report and the
//!   "Transcript written on..." note.

use std::sync::Arc;

use tex_command::{CommandHostCapabilities, FileFramingEvent};
use tex_state::Universe;
use tex_state::env::banks::IntParam;
use tex_state::print::{ErrorHistory, MAX_PRINT_LINE, Printer, Selector};
use tex_state::world::PrintSink;

/// tex.web's `banner`: the reference engine's own start-up string.
///
/// Byte-for-byte comparison against a pinned reference engine is the whole
/// point of the minifixture corpus this module was built for (see
/// `docs/job_framing.md`), so this is the reference engine's banner, not
/// Umber's name -- the same string `umber::pdf_output` writes as the PDF
/// `PTEX.Fullbanner`/`PTEX_Fullbanner` key, which is why this is the single
/// place either crate spells it.
pub const BANNER: &str = "This is pdfTeX, Version 3.141592653-2.6-1.40.27 (TeX Live 2025)";

/// tex.web §54's `open_parens`, plus enough to make [`begin_job`] a one-shot.
///
/// Both fields are engine state that lives outside `Universe`, alongside
/// `CanonicalMainControl`'s other replay-owned fields (`next_alignment_identity`,
/// `boxes`, ...): a step that prints `(name` or `)` and then rolls back must
/// have `open_parens` roll back with it, so `CanonicalStepSnapshot` captures
/// and restores this whole value exactly as it does those.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct JobFraming {
    /// Guards [`begin_job`] against printing the banner twice.
    started: bool,
    /// tex.web §54's `open_parens`: how many `(` have been printed with no
    /// matching `)` yet.
    open_parens: u32,
}

/// Immutable facts about a serialized `.dvi` file that only the driver which
/// wrote it can know.
///
/// tex.web's §642 `finish_dvi` prints the current job's total page count
/// alongside the file's name and final byte length. The page count is
/// durable engine state (`Universe::world().artifact_commits()`, incremented
/// at every `\shipout`), so it is not a field here: threading it through
/// this struct would let a caller's wrong number silently override the
/// engine's own count. Only the name and byte length are supplied, because
/// only the caller that finished serializing the file to bytes can know
/// them -- no engine-level API produces a DVI file's length, since nothing
/// below the driver ever serializes one.
#[derive(Clone, Debug)]
pub struct DviJobOutput {
    /// §642's `slow_print(output_file_name)`.
    pub file_name: String,
    /// §642's `dvi_offset+dvi_ptr`: the serialized file's exact byte length.
    pub byte_len: u64,
}

/// tex.web §61's `format_ident`: printed right after the banner both on the
/// terminal (§61's `if format_ident=0 then wterm_ln(' (no format preloaded)')
/// else begin slow_print(format_ident); print_ln end`) and, with the clock
/// appended, on the log (§536's `open_log_file`).
///
/// INITEX sets `format_ident:=" (INITEX)"` while its own tables are still
/// loading, so by the time §61 runs, `format_ident` is never really `0` for
/// an INITEX job -- the only kind [`begin_job`] currently frames
/// (`tools/tex-command-stream`'s `SessionProfile::Initex`/`EtexInitex` both
/// pass `initex: true`; no caller currently reaches the `false` branch at
/// all). A loaded-format job's `format_ident` would instead be `"
/// (preloaded format=" name " " year "." month "." day ")"` (§1336's
/// `store_fmt_file`), which this module has no format name or dump date to
/// spell -- guessing one would look correct and be silently wrong. Rather
/// than refuse outright, an unreached `initex: false` call reports tex.web's
/// *other* honest branch instead: `format_ident=0`'s `" (no format
/// preloaded)"`, true of a job that has loaded no format at all.
fn format_ident(initex: bool) -> &'static str {
    if initex {
        " (INITEX)"
    } else {
        " (no format preloaded)"
    }
}

/// tex.web's month-name table, indexed by `(month-1)*3..(month-1)*3+3`.
const MONTH_NAMES: &[u8] = b"JANFEBMARAPRMAYJUNJULAUGSEPOCTNOVDEC";

fn month_name(month: i32) -> &'static str {
    let index = month.clamp(1, 12) as usize - 1;
    std::str::from_utf8(&MONTH_NAMES[index * 3..index * 3 + 3])
        .expect("MONTH_NAMES is a fixed ASCII table")
}

/// tex.web §65's `print_two`: zero-padded to two digits.
fn print_two(value: i32) -> String {
    format!("{:02}", value.rem_euclid(100))
}

/// §536's clock suffix: `"  " print_int(day) " " month print_int(year) " "
/// print_two(hour) ":" print_two(minute)`.
fn clock_suffix(stores: &Universe) -> String {
    let day = stores.int_param(IntParam::DAY);
    let month = stores.int_param(IntParam::MONTH);
    let year = stores.int_param(IntParam::YEAR);
    let time = stores.int_param(IntParam::TIME);
    format!(
        "  {day} {} {year} {}:{}",
        month_name(month),
        print_two(time.div_euclid(60)),
        print_two(time.rem_euclid(60)),
    )
}

/// tex.web §61's `wterm(banner)` plus `format_ident`, §536's clock-stamped
/// log banner, etex.ch's "entering extended mode" notice, and §534's `**`
/// first line.
///
/// Idempotent: only the first call prints anything, matching tex.web's own
/// one-shot start-up (`job_name=0` guards `open_log_file`). A driver calls
/// this before it opens the root source (via
/// [`crate::CanonicalMainControl::register_root_source`] or a wrapper over
/// it), so the banner and `**` line precede the root file's own `(`.
pub(crate) fn begin_job(
    job: &mut JobFraming,
    stores: &mut Universe,
    capabilities: &mut CommandHostCapabilities,
    initex: bool,
    etex: bool,
    first_line: &str,
) {
    if job.started {
        return;
    }
    job.started = true;
    // §537's `a_make_name_string`-derived `\jobname` reuses tex.web's own
    // stem derivation rather than re-deriving it here; see
    // `CommandHostCapabilities::set_startup_job_name`.
    capabilities.set_startup_job_name(first_line);

    // §61: the terminal's very first output -- `format_ident` and a
    // terminating `print_ln`, no clock (the clock is §536's log-only
    // addition). Whatever prints next (etex.ch's notice below, or §537's `(`
    // for the root file) starts its own fresh line.
    let terminal_banner = format!("{BANNER}{}\n", format_ident(initex));
    stores
        .world_mut()
        .write_text(PrintSink::Terminal, &terminal_banner);

    // §536 `open_log_file`: the log's banner additionally carries
    // `format_ident` and the clock, with no trailing newline yet -- §534's
    // `print_nl("**")` below supplies it.
    let log_banner = format!("{BANNER}{}{}", format_ident(initex), clock_suffix(stores));
    stores.world_mut().write_text(PrintSink::Log, &log_banner);

    if etex {
        // etex.ch's patch at tex.web §1337 (`init_prim`'s caller, run once
        // right after the terminal banner): `wterm_ln('entering extended
        // mode')`, its own line, before anything else reaches the terminal.
        stores
            .world_mut()
            .write_text(PrintSink::Terminal, "entering extended mode\n");
        // etex.ch's patch at tex.web §536 (`open_log_file`, right after the
        // clock): `wlog_cr; wlog('entering extended mode')` -- a fresh line,
        // then the text with no trailing newline; §534's `print_nl("**")`
        // below supplies the line break before `**`.
        stores
            .world_mut()
            .write_text(PrintSink::Log, "\nentering extended mode");
    }

    // §534: `**` plus the job's first line, then `open_log_file`'s own
    // `print_ln` -- log only, since a non-interactive job's terminal never
    // shows the typed `**` prompt line.
    Printer::new(stores, Selector::LogOnly)
        .print_nl("**")
        .print(first_line)
        .print_ln();
}

/// Renders one step's drained §537/§362 file-bracketing queue.
///
/// Every driver that advances the engine (`step_once`, `alignment_step_once`,
/// `step_with_observer_once`) must call this once per step, immediately after
/// it reports the step's other diagnostics -- see `docs/job_framing.md` for
/// why the queue lives on `tex_command::CommandState` rather than here.
pub(crate) fn render_file_framing_events(
    job: &mut JobFraming,
    stores: &mut Universe,
    events: Vec<FileFramingEvent>,
) {
    if events.is_empty() {
        return;
    }
    let mut printer = stores.printer();
    for event in events {
        match event {
            FileFramingEvent::Open { name } => open_paren(job, &mut printer, &name),
            FileFramingEvent::Close => close_paren(job, &mut printer),
        }
    }
}

/// tex.web §537's `(name`.
///
/// The line-break decision tests `term_offset` alone, exactly as tex.web
/// does -- not `file_offset`/`log_offset` -- even though the resulting
/// `print_ln` or space is written through the ambient selector to every
/// channel it routes to. This is tex.web's own asymmetry, not an
/// approximation of it.
fn open_paren(job: &mut JobFraming, printer: &mut Printer<'_>, name: &Arc<str>) {
    let term_offset = printer.terminal_offset();
    if term_offset + name.chars().count() > MAX_PRINT_LINE - 2 {
        printer.print_ln();
    } else if term_offset > 0 || printer.log_offset() > 0 {
        printer.print_char(' ');
    }
    printer.print_char('(');
    job.open_parens = job.open_parens.saturating_add(1);
    printer.print(name);
}

/// tex.web §362's bare `)`.
fn close_paren(job: &mut JobFraming, printer: &mut Printer<'_>) {
    printer.print_char(')');
    job.open_parens = job.open_parens.saturating_sub(1);
}

/// tex.web §1335's `while open_parens>0 do begin print(" )"); decr(open_parens); end`.
pub(crate) fn close_open_parens(job: &mut JobFraming, stores: &mut Universe) {
    let mut printer = stores.printer();
    while job.open_parens > 0 {
        printer.print(" )");
        job.open_parens -= 1;
    }
}

/// tex.web §1335's "(see the transcript file for additional information)"
/// note.
///
/// tex.web's guard is `if history<>spotless then if
/// (history=warning_issued)or(interaction<error_stop_mode) then if
/// selector=term_and_log then ...`. Umber's transcript is "constantly open"
/// (see `tex_state::print`'s module doc), so `selector=term_and_log` reduces
/// to `Selector::for_interaction(interaction) == Selector::TermAndLog`
/// -- i.e. `interaction<>batch_mode` -- rather than a real closed/open log
/// test.
pub(crate) fn print_history_note(stores: &mut Universe) {
    let history = stores.world().error_channel().history();
    if history == ErrorHistory::Spotless {
        return;
    }
    let interaction = stores.interaction_mode();
    let severity_warrants_note =
        history == ErrorHistory::WarningIssued || interaction != tex_state::InteractionMode::ErrorStop;
    if !severity_warrants_note {
        return;
    }
    if Selector::for_interaction(interaction) != Selector::TermAndLog {
        return;
    }
    Printer::new(stores, Selector::TermOnly)
        .print_nl("(see the transcript file for additional information)");
}

/// tex.web §311's `<*>` context: `show_context`'s rendering for the
/// bottom-of-stack terminal source level.
///
/// [`print_terminal_exhausted`] is the only site that has to spell this out.
/// Everywhere else `tex-command`'s `output_open_context` renders context from
/// a live input level, but here input has genuinely run out, so the stack it
/// would walk is empty. §317's two-line form for a level whose label is
/// `<*>` followed by a space and whose consumed and pending text are both
/// empty is that label alone, then an indent of the label's own width with
/// nothing after it.
const TERMINAL_EXHAUSTED_CONTEXT: &str = "\n<*> \n    ";

/// tex.web §360/§362's `*` prompt -- printed when the last input file has
/// closed and the job has not yet seen `\end`/`\dump` -- followed by §71's
/// `term_input` failing and §93's `fatal_error` it raises.
///
/// A driver calls this the moment a step reports
/// [`crate::MainControlStep::EndOfInput`], mirroring how
/// [`crate::CanonicalMainControl`]'s own `end_of_job_final_cleanup` runs on
/// [`crate::MainControlStep::End`]. The two are siblings, not the same path:
/// §93's `succumb` sets `interaction` to `scroll_mode`, calls `error` (which
/// is what actually prints everything below), sets `history` to
/// `fatal_error_stop`, and then §81's `jump_out` transfers control straight
/// to §1333's `close_files_and_terminate` ([`finish_job`]) -- skipping
/// §1335's `final_cleanup` ([`crate::CanonicalMainControl`]'s
/// `end_of_job_final_cleanup`) entirely, so this prints no paren-closing,
/// incomplete-conditions report, or history note of its own; a driver's own
/// unconditional `finish_job` call is what §642's report and the transcript
/// note still reach.
///
/// tex.web's own `get_next` silently accepts one empty terminal line before
/// a second attempt actually fails, and the reference engine's redirected
/// terminal stream needs one such retry before every minifixture's genuine
/// exhaustion in this corpus except a handful with additional pending
/// recoveries (each consuming one further retry) -- a host stdin-reading
/// nuance `tex_state::print`'s module doc already documents as unmodeled
/// here. This reproduces the dominant one-retry (two-`*`, one-message)
/// shape; a fixture needing zero or several retries remains a residual
/// divergence, not a wrong shape for the ordinary case.
///
/// The retry's own message line (`"(Please type...)"`) is a second, smaller
/// case of the same per-channel divergence: every captured oracle log shows
/// it on its own line (`print_nl`'s smart break, column already open from
/// the `*` this function just printed), while every captured oracle
/// *terminal* runs it straight onto the `*`'s line with no break at all.
/// Nothing in §362 conditions that message on the channel, so this treats it
/// as another fact about the reference engine's terminal handling this layer
/// does not model (see `tex_state::print`'s module doc) and reproduces it
/// directly rather than deriving it from one shared smart `print_nl` call.
pub(crate) fn print_terminal_exhausted(stores: &mut Universe) {
    // tex.web §362's `interaction>nonstop_mode`.
    let interactive = !matches!(
        stores.interaction_mode(),
        tex_state::InteractionMode::Batch | tex_state::InteractionMode::Nonstop
    );
    if interactive {
        {
            let mut printer = stores.printer();
            printer.print_ln();
            printer.print_char('*');
        }
        stores
            .world_mut()
            .write_text(PrintSink::Log, "\n(Please type a command or say `\\end')");
        stores.world_mut().write_text(
            PrintSink::Terminal,
            "(Please type a command or say `\\end')",
        );
        {
            let mut printer = stores.printer();
            printer.print_ln();
            printer.print_char('*');
        }
    }
    let mut report = stores.print_err("Emergency stop");
    report.context(TERMINAL_EXHAUSTED_CONTEXT.to_owned());
    report.help(&[if interactive {
        "End of file on the terminal!"
    } else {
        "*** (job aborted, no legal \\end found)"
    }]);
    report.error();
}

/// tex.web §1333's `close_files_and_terminate`, minus §1378's write-stream
/// closing loop.
///
/// §1378 already ran synchronously when the engine applied the `\end`/
/// `\dump` step that ended the job -- closing a write stream is a `World`
/// state effect, not a print, so its position relative to this call (which a
/// driver makes only after that step has already returned
/// [`crate::MainControlStep::End`]) can't reorder anything this function
/// prints. What remains here is exactly §642's DVI report and the
/// transcript-closing note.
pub(crate) fn finish_job(stores: &mut Universe, job_name: &str, dvi: Option<DviJobOutput>) {
    print_dvi_report(stores, dvi);
    print_transcript_note(stores, job_name);
    stores.printer().print_ln();
}

/// tex.web §642's `<Finish the DVI file>`.
///
/// The page count is read from the engine's own durable commit log
/// (`Universe::world().artifact_commits()`), never from `dvi`: a caller
/// cannot make this print "Output written on..." with a fabricated page
/// count, and cannot make it print a byte count when the engine committed no
/// pages, because the zero-page branch never inspects `dvi` at all.
fn print_dvi_report(stores: &mut Universe, dvi: Option<DviJobOutput>) {
    let total_pages = i32::try_from(stores.world().artifact_commits().len()).unwrap_or(i32::MAX);
    if total_pages == 0 {
        stores.printer().print_nl("No pages of output.");
        return;
    }
    let dvi = dvi.unwrap_or_else(|| {
        panic!(
            "job::finish_job: the engine committed {total_pages} page(s) but no `DviJobOutput` \
             was supplied; §642's byte count cannot be fabricated, so the caller must serialize \
             the DVI file and report its name and length before calling `finish_job`"
        )
    });
    let mut printer = stores.printer();
    printer
        .print_nl("Output written on ")
        .print(&dvi.file_name)
        .print(" (")
        .print_int(total_pages)
        .print(" page");
    if total_pages != 1 {
        printer.print_char('s');
    }
    printer
        .print(", ")
        .print_int(i32::try_from(dvi.byte_len).unwrap_or(i32::MAX))
        .print(" bytes).");
}

/// tex.web §1333's `if selector=term_only then begin print_nl("Transcript
/// written on "); slow_print(log_name); print_char("."); end`.
fn print_transcript_note(stores: &mut Universe, job_name: &str) {
    // Real tex.web reaches this only after `a_close(log_file);
    // selector:=selector-2` has just taken `selector` from `term_and_log` to
    // `term_only`. Umber's transcript never really closes (see
    // `tex_state::print`'s module doc), so the honest reading of that
    // transition is: print this note exactly when the ambient selector was
    // `term_and_log`, i.e. the job was writing both channels.
    if Selector::for_interaction(stores.interaction_mode()) != Selector::TermAndLog {
        return;
    }
    Printer::new(stores, Selector::TermOnly)
        .print_nl("Transcript written on ")
        .print(job_name)
        .print(".log")
        .print_char('.');
}

#[cfg(test)]
mod tests;
