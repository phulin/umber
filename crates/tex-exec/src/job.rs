//! TeX's job framing: the start-up banner, the per-file `(name`/`)`
//! bracketing, and the tail every job ends with.
//!
//! See `docs/job_framing.md` for the full account of what a job prints, when,
//! and why the pieces live where they do. In short: printing is
//! `tex-state`'s (`tex_state::print::Printer` over §54's `selector`), file
//! opening is `tex-command`'s (§537's `start_input` is an input-stack
//! operation), and the job lifecycle -- when a job starts and how it ends --
//! is `CanonicalMainControl`'s, because it is the one layer that sees a
//! `Universe` and a driver both. This module is where that lifecycle lives;
//! `canonical_main_control.rs` only exposes it.
//!
//! The per-file `(name`/`)` bracketing is the one piece that is *not* here:
//! §362 prints its `)` from inside `get_next`, ahead of the
//! `check_outer_validity` diagnostic it must precede, so §54's `open_parens`
//! is print-adjacent state on `World` and both layers render through
//! [`tex_state::file_framing`].
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

use tex_command::CommandHostCapabilities;
use tex_state::Universe;
use tex_state::env::banks::IntParam;
use tex_state::print::{ErrorHistory, Printer, Selector};
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

/// Enough job state to make [`begin_job`] a one-shot.
///
/// This is engine state that lives outside `Universe`, alongside
/// `CanonicalMainControl`'s other replay-owned fields
/// (`next_alignment_identity`, `boxes`, ...), so `CanonicalStepSnapshot`
/// captures and restores it exactly as it does those.
///
/// §54's `open_parens` is deliberately *not* here. It is print-adjacent state
/// on `World` -- see [`tex_state::file_framing`] -- because §362 prints its
/// `)` from inside `get_next`, one line ahead of the `check_outer_validity`
/// diagnostic it must precede, where no engine driver is on the stack.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct JobFraming {
    /// Guards [`begin_job`] against printing the banner twice.
    started: bool,
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

/// A job started from a dumped format: web2c's `dump_name` (the `-fmt=`
/// argument) plus the `\year`/`\month`/`\day` that were current when
/// tex.web §1328's `store_fmt_file` built the dumped `format_ident`.
///
/// Both are carried because a real run prints *different* text from them on
/// the two sinks; see [`terminal_format_ident`] and [`log_format_ident`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreloadedFormat {
    /// web2c's `dump_name`, e.g. `etex-loaded` for `-fmt=etex-loaded`.
    pub name: String,
    /// `\year`, `\month`, `\day` at the moment the format was dumped.
    pub year: i32,
    pub month: i32,
    pub day: i32,
}

/// Engine-owned receipt for TeX82 §1328's successful INITEX dump transition.
///
/// The host must not render the announcement until it has successfully
/// created and published the format file. The receipt owns the exact
/// `format_ident` established by the transition; the host supplies only the
/// canonical filename it actually published.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormatDumpReceipt {
    pub format_ident: PreloadedFormat,
    publication_confirmed: bool,
}

impl FormatDumpReceipt {
    #[must_use]
    pub fn new(name: String, year: i32, month: i32, day: i32) -> Self {
        Self {
            format_ident: PreloadedFormat {
                name,
                year,
                month,
                day,
            },
            publication_confirmed: false,
        }
    }
}

/// tex.web §61's `format_ident` as it reaches the *terminal*.
///
/// web2c replaces §61's stock `if format_ident=0 then wterm_ln(' (no format
/// preloaded)')` with `wterm_ln(' (preloaded format=',dump_name,')')`
/// (`texk/web2c/tex.ch`, `@x [5.61]`). That branch is the one a
/// loaded-format run actually takes: web2c prints the banner *before* it
/// reads the format file, so `format_ident` is still `0` here and the name
/// comes from the command line rather than from the dump. It therefore
/// carries **no dump date** -- which is exactly what the pinned pdfTeX
/// 1.40.27 oracle emits.
///
/// INITEX is the other way round: §1337 sets `format_ident:=" (INITEX)"`
/// while its own tables are still loading, so by the time §61 runs the
/// `else` arm's `slow_print(format_ident)` has something to print.
fn terminal_format_ident(format: Option<&PreloadedFormat>, initex: bool) -> String {
    match (format, initex) {
        (Some(format), _) => format!(" (preloaded format={})", format.name),
        (None, true) => " (INITEX)".to_owned(),
        // Unreached in practice: a job is either INITEX or started from a
        // format. tex.web's own remaining honest branch for neither.
        (None, false) => " (no format preloaded)".to_owned(),
    }
}

/// tex.web §61's `format_ident` as it reaches the *log*.
///
/// §536's `open_log_file` does `slow_print(format_ident)`, and by then the
/// format file *has* been read, so this is the dumped string §1328 built:
/// `" (preloaded format=" name " " year "." month "." day ")"`. Hence the
/// dump date appears on the log and not on the terminal.
fn log_format_ident(format: Option<&PreloadedFormat>, initex: bool) -> String {
    match (format, initex) {
        (Some(format), _) => format!(
            " (preloaded format={} {}.{}.{})",
            format.name, format.year, format.month, format.day
        ),
        (None, true) => " (INITEX)".to_owned(),
        (None, false) => " (no format preloaded)".to_owned(),
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
    format: Option<&PreloadedFormat>,
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
    // The banner itself goes out through §54's `wterm`/`wlog`, which do not
    // advance `term_offset`/`file_offset`; it is longer than
    // `max_print_line` and must stay one unbroken line.
    let terminal_banner = format!("{BANNER}{}\n", terminal_format_ident(format, initex));
    stores
        .world_mut()
        .write_text_unmetered(PrintSink::Terminal, &terminal_banner);

    // §536 `open_log_file`: the log's banner additionally carries
    // `format_ident` and the clock, with no trailing newline yet -- §534's
    // `print_nl("**")` below supplies it. Only `wlog(banner)` is unmetered;
    // §536 prints the identity and clock through `slow_print`/`print_int`,
    // so those do advance `file_offset` and are what makes the following
    // `print_nl("**")` break the line.
    stores
        .world_mut()
        .write_text_unmetered(PrintSink::Log, BANNER);
    let log_identity = format!(
        "{}{}",
        log_format_ident(format, initex),
        clock_suffix(stores)
    );
    stores.world_mut().write_text(PrintSink::Log, &log_identity);

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

/// tex.web §1335's `while open_parens>0 do begin print(" )"); decr(open_parens); end`.
pub(crate) fn close_open_parens(stores: &mut Universe) {
    tex_state::file_framing::print_remaining_file_closes(stores);
}

/// tex.web §1335's `if cur_level>level_one then begin print_nl("(");
/// print_esc("end occurred "); print("inside a group at level ");
/// print_int(cur_level-level_one); print_char(")"); end`.
///
/// §1335 spells the escape `\end` whichever of `\end` and `\dump` ended the
/// job, so this takes no dump flag; the sibling report immediately below it
/// (`report_incomplete_conditions`) shares that wording for the same reason.
pub(crate) fn report_unclosed_groups(stores: &mut Universe) {
    let depth = stores.group_depth();
    if depth == 0 {
        return;
    }
    stores
        .printer()
        .print_nl(&format!("(\\end occurred inside a group at level {depth})"));
}

/// Records and prints the retained session's already-opened root source.
///
/// The retained driver selects the root before canonical execution starts,
/// so this opening cannot arrive through [`FileFramingEvent`]. It is still
/// TeX82 §537's `print_char("("); incr(open_parens)`: §1335 must therefore
/// see it when `\end` or `\dump` abandons the still-open root.
pub(crate) fn open_startup_input(stores: &mut Universe, name: &str) {
    tex_state::file_framing::print_startup_file_open(stores, name);
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
    let severity_warrants_note = history == ErrorHistory::WarningIssued
        || interaction != tex_state::InteractionMode::ErrorStop;
    if !severity_warrants_note {
        return;
    }
    if Selector::for_interaction(interaction) != Selector::TermAndLog {
        return;
    }
    Printer::new(stores, Selector::TermOnly)
        .print_nl("(see the transcript file for additional information)");
}

/// tex.web §313's `<*>` context: `show_context`'s rendering for the
/// bottom-of-stack terminal source level.
///
/// [`prompt_for_more_input`] is the only site that has to spell this out.
/// Everywhere else `tex-command`'s `output_open_context` renders context from
/// a live input level, but here input has genuinely run out and Umber has
/// already retired the base level, so the stack it would walk is empty.
///
/// tex.web keeps the base level open for the whole run, so `buffer` still
/// holds whatever was last read into it, and §313 pseudoprints *that* with
/// `loc` past its end: everything before the split, nothing after. Which line
/// that is depends on the interaction mode, and it is the same test §360
/// itself makes:
///
/// - `interaction>nonstop_mode`: §360 ran `first:=start; prompt_input("*")`
///   before the read that failed, and every captured oracle log for that path
///   pseudoprints nothing at all -- including the fixtures whose very first
///   `*` fails, so it is the prompt's own `first:=start`, not a preceding
///   successful read, that empties the pseudoprinted range. This is the run
///   whose help line is `End of file on the terminal!`.
/// - otherwise: §360 went straight to `fatal_error` without touching `first`,
///   `buffer` was never rewritten, and it still holds §331's `**` line naming
///   the root file. This is the run whose help line is `*** (job aborted, no
///   legal \end found)`.
///
/// §31 drops the line's trailing space before setting `limit`, so the
/// terminator the `**` line ends with is not part of `startup_terminal_line`.
fn terminal_exhausted_context(
    stores: &mut Universe,
    startup_terminal_line: &str,
    interactive: bool,
) -> String {
    let line = if interactive {
        ""
    } else {
        startup_terminal_line
    };
    tex_state::print::render_error_context(
        &[tex_state::print::ErrorContextLevel::new("<*> ", line, "")],
        stores.command_context().error_context_widths(),
        stores.int_param(tex_state::env::banks::IntParam::new(54)),
    )
}

/// tex.web §360's `*` prompt loop -- reached when the last input file has
/// closed and the job has not yet seen `\end`/`\dump` -- and §71's
/// `term_input` failing at the end of it.
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
/// §360 prompts exactly once per pass and prints `(Please type a command or
/// say `\end')` only when `limit=start` -- when the line the base terminal
/// level's buffer holds is empty. Nothing else rewrites that buffer: §483's
/// `\read` and §83's `? ` prompt each read into a level of their own, and
/// `end_file_reading` restores `limit` when it pops. So the test is exactly
/// "the previous pass of this loop read an empty line", and before the first
/// pass it is §331's `**` line. The loop repeats because tex.web returns to
/// `get_next` after each accepted line and arrives back here once that line
/// is used up.
///
/// The per-channel asymmetry the corpus shows -- every oracle log puts the
/// message on its own line while every oracle terminal runs it straight onto
/// the preceding `*` -- is §71's, not §360's, and this derives it rather than
/// hard-coding it: `term_input` echoes the accepted line with `selector`
/// decremented to `log_only`, so its closing `print_ln` opens a fresh column
/// in the transcript while leaving the terminal's own column where the user's
/// carriage return put it (`term_offset:=0`). The next pass's `print_nl` then
/// breaks for the transcript and not for the terminal.
///
/// What an accepted line's *tokens* do is not modelled. An empty line
/// contributes only `\endlinechar`, whose `\par` is a no-op in the vertical
/// or restricted horizontal mode §360 is reached in, so the corpus -- which
/// types nothing else at this prompt -- is unaffected; a non-empty line is
/// read, echoed, and then discarded rather than executed.
pub(crate) fn prompt_for_more_input(stores: &mut Universe, startup_terminal_line: &str) {
    // tex.web §360's `else fatal_error("*** (job aborted...)")`.
    if !stores
        .command_context()
        .interaction_permits_terminal_input()
    {
        report_emergency_stop(stores, startup_terminal_line, false);
        return;
    }
    // §360's `limit=start`, carried across passes: §331's `**` line is what
    // the buffer holds until this loop's own first read replaces it.
    let mut buffered_line_is_empty = startup_terminal_line.is_empty();
    loop {
        if buffered_line_is_empty {
            stores
                .printer()
                .print_nl("(Please type a command or say `\\end')");
        }
        stores.printer().print_ln();
        // §71's `prompt_input("*")`: the prompt, `term_input`'s read, and --
        // on success -- its transcript echo, all owned by `input_ln`.
        let line = stores
            .command_context()
            .input_ln(tex_state::CommandLineSource::Terminal { prompt: "*" });
        let Some(line) = line else {
            // §71's `fatal_error("End of file on the terminal!")`.
            report_emergency_stop(stores, startup_terminal_line, true);
            return;
        };
        buffered_line_is_empty = line.is_empty();
    }
}

/// §93's `fatal_error`: `print_err("Emergency stop")` with the caller's one
/// help line, reported through §82's `error` by `succumb`.
fn report_emergency_stop(stores: &mut Universe, startup_terminal_line: &str, interactive: bool) {
    let context = terminal_exhausted_context(stores, startup_terminal_line, interactive);
    let mut report = stores.print_err("Emergency stop");
    report.context(context);
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

/// TeX82 §1328's format-file announcement and newly built `format_ident`.
///
/// The serialized bytes and displayed output name are host concerns. The host
/// calls this only after atomic publication succeeds, matching §1328's
/// successful-open ordering and preventing a failed output from claiming a
/// dump that did not happen.
pub fn confirm_format_dump_publication(
    stores: &mut Universe,
    receipt: &mut FormatDumpReceipt,
    displayed_file_name: &str,
) {
    if std::mem::replace(&mut receipt.publication_confirmed, true) {
        return;
    }
    let ident = &receipt.format_ident;
    let mut printer = stores.printer();
    printer
        .print_nl("Beginning to dump on file ")
        .print(displayed_file_name)
        .print_nl(" (preloaded format=")
        .print(&ident.name)
        .print_char(' ')
        .print_int(ident.year)
        .print_char('.')
        .print_int(ident.month)
        .print_char('.')
        .print_int(ident.day)
        .print_char(')');
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
