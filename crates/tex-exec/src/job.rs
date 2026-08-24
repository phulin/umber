//! TeX's job framing: the start-up banner, the per-file `(name`/`)`
//! bracketing, and the tail every job ends with.
//!
//! See `docs/job_framing.md` for the full account of what a job prints, when,
//! and why the pieces live where they do. In short: printing is
//! `tex-state`'s (`tex_state::print::Printer` over §54's `selector`), file
//! opening is `tex-command`'s (§537's `start_input` is an input-stack
//! operation), and the job lifecycle -- when a job starts and how it ends --
//! is `MainControl`'s, because it is the one layer that sees a
//! `Universe` and a driver both. This module is where that lifecycle lives;
//! `main_control.rs` only exposes it.
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

use tex_command::{CommandHostCapabilities, CommandProfile};
use tex_state::env::banks::IntParam;
use tex_state::print::{ErrorHistory, Printer, Selector};
use tex_state::world::PrintSink;
use tex_state::{CommandContext, EngineUsageStatistics, PdfNavigationWarning, Universe};

/// pdftex.web §2's `banner`: the production reference engine's start-up string.
///
/// Byte-for-byte comparison against a pinned reference engine is the whole
/// point of the minifixture corpus this module was built for (see
/// `docs/job_framing.md`), so this is the reference engine's banner, not
/// Umber's name -- the same string `umber::pdf_output` writes as the PDF
/// `PTEX.Fullbanner`/`PTEX_Fullbanner` key.
pub const BANNER: &str = "This is pdfTeX, Version 3.141592653-2.6-1.40.29 (TeX Live 2026)";

/// tex.web §2's TeX82 start-up banner, with the pinned distribution suffix.
pub const TEX82_BANNER: &str = "This is TeX, Version 3.141592653 (TeX Live 2026)";

/// etex.ch §2's e-TeX 2.6 start-up banner, with the pinned distribution suffix.
pub const ETEX26_BANNER: &str = "This is e-TeX, Version 3.141592653-2.6 (TeX Live 2026)";

/// Immutable identity of the executable whose canonical framing is emitted.
///
/// This is deliberately distinct from [`CommandProfile`]: a newer reference
/// binary may execute an older semantic profile for conformance purposes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EngineBinaryIdentity {
    Tex82,
    Etex26,
    Pdftex14029,
}

impl EngineBinaryIdentity {
    pub(crate) const fn for_profile(profile: CommandProfile) -> Self {
        match profile.dialect() {
            tex_command::CommandDialect::Tex82 => Self::Tex82,
            tex_command::CommandDialect::Etex26 => Self::Etex26,
            tex_command::CommandDialect::Pdftex14029 => Self::Pdftex14029,
        }
    }

    fn banner(self) -> &'static str {
        match self {
            Self::Tex82 => TEX82_BANNER,
            Self::Etex26 => ETEX26_BANNER,
            Self::Pdftex14029 => BANNER,
        }
    }

    /// Whether this binary contains the requested semantic command family.
    #[must_use]
    pub const fn supports(self, profile: CommandProfile) -> bool {
        self.command_semantics().supports(profile)
    }

    /// Canonical compiled command semantics supplied by this binary.
    #[must_use]
    pub const fn command_semantics(self) -> tex_command::CommandEngineSemantics {
        match self {
            Self::Tex82 => tex_command::CommandEngineSemantics::Tex82,
            Self::Etex26 => tex_command::CommandEngineSemantics::Etex26,
            Self::Pdftex14029 => tex_command::CommandEngineSemantics::Pdftex14029,
        }
    }

    /// Returns the hash capacity of this pinned executable configuration.
    ///
    /// Web2C `tex.ch` [51.1332] reads `hash_extra` as an executable runtime
    /// bound, independently of the loaded format's command profile, and
    /// [51.1334] renders that value in the usage report. The TeX82/e-TeX
    /// conformance executables use triptrap's default zero extension; the
    /// pinned pdfTeX distribution configuration supplies 600000.
    const fn control_sequence_capacity(self) -> (u32, u32) {
        match self {
            Self::Tex82 | Self::Etex26 => (15_000, 0),
            Self::Pdftex14029 => (15_000, 600_000),
        }
    }

    /// Returns this executable's process-configured `font_info` word bound.
    ///
    /// The value is operational rather than format-owned: TeX82 and e-TeX
    /// use the compiled default, while the pinned Web2C pdfTeX process uses
    /// the distribution's `font_mem_size` setting.
    pub(crate) const fn font_info_capacity(self) -> usize {
        match self {
            Self::Tex82 | Self::Etex26 => tex_state::font::FONT_INFO_CAPACITY,
            Self::Pdftex14029 => tex_state::font::WEB2C_FONT_INFO_CAPACITY,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct JobEngineFraming {
    pub binary: EngineBinaryIdentity,
    pub extended_mode: bool,
}

pub(crate) struct StartupLineFraming<'a> {
    pub first_line: &'a str,
    pub input_name: &'a str,
    pub terminal_banner: bool,
}

/// Enough job state to make [`begin_job_with_terminal_banner`] a one-shot.
///
/// This is engine state that lives outside `Universe`, alongside
/// `MainControl`'s other replay-owned fields
/// (`next_alignment_identity`, `boxes`, ...). Direct command preparation does
/// not mutate it, and committed semantic operations update it in place.
///
/// §54's `open_parens` is deliberately *not* here. It is print-adjacent state
/// on `World` -- see [`tex_state::file_framing`] -- because §362 prints its
/// `)` from inside `get_next`, one line ahead of the `check_outer_validity`
/// diagnostic it must precede, where no engine driver is on the stack.
#[derive(Clone, Debug, Default)]
pub(crate) struct JobFraming {
    /// Guards [`begin_job`] against printing the banner twice.
    started: bool,
    pub(crate) output: crate::job_output::JobOutput,
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

/// Host-supplied facts printed by pdfTeX after its PDF writer is finalized.
///
/// The engine owns profile selection and rendering; the host owns these facts
/// because only the final PDF serializer knows its object-stream and memory
/// totals. The receipt is one-shot so retrying job finalization cannot append
/// a second statistics block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfJobFinalizationReport {
    pub pdf_objects: u32,
    pub compressed_objects: u32,
    pub object_streams: u32,
    pub named_destinations: u32,
    pub extra_memory_words: u32,
    reported: bool,
}

impl PdfJobFinalizationReport {
    #[must_use]
    pub const fn new(
        pdf_objects: u32,
        compressed_objects: u32,
        object_streams: u32,
        named_destinations: u32,
        extra_memory_words: u32,
    ) -> Self {
        Self {
            pdf_objects,
            compressed_objects,
            object_streams,
            named_destinations,
            extra_memory_words,
            reported: false,
        }
    }
}

/// A job started from a dumped format: web2c's `dump_name` (the `-fmt=`
/// argument), the dump job name in tex.web §1328's `format_ident`, and the
/// `\year`/`\month`/`\day` that were current when the format was built.
///
/// Both are carried because a real run prints *different* text from them on
/// the two sinks; see [`terminal_format_ident`] and [`log_format_ident`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreloadedFormat {
    /// web2c's `dump_name`, e.g. `etex-loaded` for `-fmt=etex-loaded`.
    pub dump_name: String,
    /// The dump job name embedded in tex.web §1328's `format_ident`.
    pub format_name: String,
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

/// Exact-once handle-free result of a quiescent INITEX dump transition.
///
/// Serialization succeeds before the engine receipt is consumed. The host
/// therefore observes either the complete image plus announcement receipt or
/// no publication payload at all.
#[derive(Debug)]
pub struct DetachedFormatDump {
    pub image: tex_state::DetachedFormatImage,
    pub receipt: FormatDumpReceipt,
}

/// Aggregate owner which prevented a successful format capture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormatDumpError {
    LiveCommandState,
    LiveExecutorState,
    LiveModeState,
    State(tex_state::FormatError),
}

impl core::fmt::Display for FormatDumpError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::LiveCommandState => formatter.write_str("format dump retains command state"),
            Self::LiveExecutorState => formatter.write_str("format dump retains executor state"),
            Self::LiveModeState => formatter.write_str("format dump retains mode material"),
            Self::State(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for FormatDumpError {}

impl FormatDumpReceipt {
    #[must_use]
    pub fn new(name: String, year: i32, month: i32, day: i32) -> Self {
        Self {
            format_ident: PreloadedFormat {
                dump_name: name.clone(),
                format_name: name,
                year,
                month,
                day,
            },
            publication_confirmed: false,
        }
    }

    pub(crate) fn pool_string(&self) -> String {
        let ident = &self.format_ident;
        format!(
            " (preloaded format={} {}.{}.{})",
            ident.format_name, ident.year, ident.month, ident.day
        )
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
/// 1.40.29 oracle emits.
///
/// INITEX is the other way round: §1337 sets `format_ident:=" (INITEX)"`
/// while its own tables are still loading, so by the time §61 runs the
/// `else` arm's `slow_print(format_ident)` has something to print.
fn terminal_format_ident(format: Option<&PreloadedFormat>, initex: bool) -> String {
    match (format, initex) {
        (Some(format), _) => format!(" (preloaded format={})", format.dump_name),
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
            format.format_name, format.year, format.month, format.day
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

/// pdftex.web §65's `print_two`: the absolute value's final two digits,
/// zero-padded to two digits.
fn print_two(value: i32) -> String {
    format!("{:02}", value.unsigned_abs() % 100)
}

/// §536's clock suffix: `"  " print_int(day) " " month print_int(year) " "
/// print_two(hour) ":" print_two(minute)`.
fn clock_suffix<G>(stores: &Universe<G>) -> String {
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
/// [`crate::MainControl::register_root_source`] or a wrapper over
/// it), so the banner and `**` line precede the root file's own `(`.
#[cfg(test)]
pub(crate) fn begin_job<G>(
    job: &mut JobFraming,
    stores: &mut Universe<G>,
    capabilities: &mut CommandHostCapabilities,
    initex: bool,
    format: Option<&PreloadedFormat>,
    engine: JobEngineFraming,
    first_line: &str,
) {
    begin_job_with_terminal_banner(
        job,
        stores,
        capabilities,
        initex,
        format,
        engine,
        StartupLineFraming {
            first_line,
            input_name: first_line,
            terminal_banner: true,
        },
    );
}

pub(crate) fn begin_job_with_terminal_banner<G>(
    job: &mut JobFraming,
    stores: &mut Universe<G>,
    capabilities: &mut CommandHostCapabilities,
    initex: bool,
    format: Option<&PreloadedFormat>,
    engine: JobEngineFraming,
    startup: StartupLineFraming<'_>,
) {
    if job.started {
        return;
    }
    job.started = true;
    // tex.web §241 runs `fix_date_and_time` for both fresh INITEX and a
    // preloaded/restored format before the first line is tokenized. The
    // restored dense bank remains authoritative except for these four
    // explicitly volatile cells.
    stores
        .refresh_job_clock_parameters()
        .expect("job framing requires a live generation");
    // §537's `a_make_name_string`-derived `\jobname` reuses tex.web's own
    // stem derivation rather than re-deriving it here; see
    // `CommandHostCapabilities::set_startup_job_name`.
    capabilities.set_startup_job_name(startup.input_name);
    // §§534--536 open the transcript as soon as the first input establishes
    // the job name. Ordinary drivers have an available retained target;
    // focused retry paths exercise the structured owner directly.
    let log_name = job
        .output
        .open_log(stores, capabilities.job_name())
        .expect("startup transcript target must be available or retried by the driver")
        .to_owned();
    if format.is_some() {
        // TeX82 §§525/536 retain `a_make_name_string(log_file)` for the
        // loaded job. INITEX construction accounting is sealed separately
        // into the format baseline.
        stores
            .command_context()
            .expect("job framing belongs to a live generation")
            .make_string_pool_string(&log_name);
    } else if initex {
        // TeX82 §§534--537 retain the opened transcript name before INITEX
        // reaches §1328's format dump. The scanned job-name component was
        // already retained at the startup filename seam and is reused here.
        let mut command = stores
            .command_context()
            .expect("job framing belongs to a live generation");
        command.make_string_pool_string(&log_name);
    }

    // §61: the terminal's very first output -- `format_ident` and a
    // terminating `print_ln`, no clock (the clock is §536's log-only
    // addition). Whatever prints next (etex.ch's notice below, or §537's `(`
    // for the root file) starts its own fresh line.
    // The banner itself goes out through §54's `wterm`/`wlog`, which do not
    // advance `term_offset`/`file_offset`; it is longer than
    // `max_print_line` and must stay one unbroken line.
    let banner = engine.binary.banner();
    if startup.terminal_banner {
        let terminal_banner = format!("{banner}{}\n", terminal_format_ident(format, initex));
        stores
            .world_mut()
            .write_text_unmetered(PrintSink::Terminal, &terminal_banner);
    }

    // §536 `open_log_file`: the log's banner additionally carries
    // `format_ident` and the clock, with no trailing newline yet -- §534's
    // `print_nl("**")` below supplies it. Only `wlog(banner)` is unmetered;
    // §536 prints the identity and clock through `slow_print`/`print_int`,
    // so those do advance `file_offset` and are what makes the following
    // `print_nl("**")` break the line.
    stores
        .world_mut()
        .write_text_unmetered(PrintSink::Log, banner);
    let log_identity = format!(
        "{}{}",
        log_format_ident(format, initex),
        clock_suffix(stores)
    );
    stores.world_mut().write_text(PrintSink::Log, &log_identity);

    if engine.extended_mode {
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
        .print(startup.first_line)
        .print_ln();
}

/// tex.web §1335's `while open_parens>0 do begin print(" )"); decr(open_parens); end`.
pub(crate) fn close_open_parens<G>(stores: &mut Universe<G>) {
    tex_state::file_framing::print_remaining_file_closes(stores);
}

/// tex.web §1335's `if cur_level>level_one then begin print_nl("(");
/// print_esc("end occurred "); print("inside a group at level ");
/// print_int(cur_level-level_one); print_char(")"); end`.
///
/// §1335 spells the escape `\end` whichever of `\end` and `\dump` ended the
/// job, so this takes no dump flag; the sibling report immediately below it
/// (`report_incomplete_conditions`) shares that wording for the same reason.
pub(crate) fn report_unclosed_groups<G>(stores: &mut Universe<G>, depth: usize) {
    if depth == 0 {
        return;
    }
    stores
        .printer()
        .print_nl("(")
        .print_esc("end occurred ")
        .print("inside a group at level ")
        .print_int(depth as i32)
        .print_char(')');
}

/// Records and prints the retained session's already-opened root source.
///
/// The retained driver selects the root before canonical execution starts,
/// so this opening cannot arrive through [`FileFramingEvent`]. It is still
/// TeX82 §537's `print_char("("); incr(open_parens)`: §1335 must therefore
/// see it when `\end` or `\dump` abandons the still-open root.
pub(crate) fn open_startup_input<G>(stores: &mut Universe<G>, name: &str) {
    tex_state::file_framing::print_startup_file_open(stores, name);
}

pub(crate) fn open_startup_input_after_log<G>(stores: &mut Universe<G>, name: &str) {
    tex_state::file_framing::print_startup_file_open_after_log(stores, name);
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
pub(crate) fn print_history_note<G>(stores: &mut Universe<G>) {
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
fn terminal_exhausted_context<G>(
    stores: &mut CommandContext<'_, G>,
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
        stores.error_context_widths(),
        stores.int_param(tex_state::env::banks::IntParam::new(54)),
    )
}

/// tex.web §360's `*` prompt loop -- reached when the last input file has
/// closed and the job has not yet seen `\end`/`\dump` -- and §71's
/// `term_input` failing at the end of it.
///
/// A driver calls this the moment a step reports
/// [`crate::MainControlStep::EndOfInput`], mirroring how
/// [`crate::MainControl`]'s own `end_of_job_final_cleanup` runs on
/// [`crate::MainControlStep::End`]. The two are siblings, not the same path:
/// §93's `succumb` sets `interaction` to `scroll_mode`, calls `error` (which
/// is what actually prints everything below), sets `history` to
/// `fatal_error_stop`, and then §81's `jump_out` transfers control straight
/// to §1333's `close_files_and_terminate` ([`finish_job`]) -- skipping
/// §1335's `final_cleanup` ([`crate::MainControl`]'s
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
/// This function performs exactly one acquisition. The owning canonical
/// control installs an accepted line as a real terminal source, so its tokens
/// execute through ordinary command delivery and fuel accounting before a
/// later root-exhaustion step returns here for the next `*` prompt.
pub(crate) enum EndOfInputAction {
    Line(String),
    Fatal(tex_command::FatalError),
}

pub(crate) fn prompt_for_more_input<G>(
    stores: &mut CommandContext<'_, G>,
    startup_terminal_line: &str,
    buffered_line_is_empty: bool,
) -> EndOfInputAction {
    // tex.web §360's `else fatal_error("*** (job aborted...)")`.
    if !stores.interaction_permits_terminal_input() {
        return EndOfInputAction::Fatal(report_emergency_stop(
            stores,
            startup_terminal_line,
            false,
        ));
    }
    // §360's `limit=start`, carried across passes: §331's `**` line is what
    // the buffer holds until this loop's own first read replaces it.
    if buffered_line_is_empty {
        stores
            .printer()
            .print_nl("(Please type a command or say `\\end')");
    }
    stores.printer().print_ln();
    // §71's `prompt_input("*")`: the prompt, `term_input`'s read, and --
    // on success -- its transcript echo, all owned by `input_ln`.
    let line = stores.input_ln(tex_state::CommandLineSource::Terminal { prompt: "*" });
    match line {
        Some(line) => EndOfInputAction::Line(line),
        // §71's `fatal_error("End of file on the terminal!")`.
        None => EndOfInputAction::Fatal(report_emergency_stop(stores, startup_terminal_line, true)),
    }
}

/// §93's `fatal_error`: `print_err("Emergency stop")` with the caller's one
/// help line, completed by `succumb`.
///
/// `succumb`, not §82's `error` directly: `succumb` drops `interaction` to
/// `scroll_mode` before the nested `error` runs, which is what stops an
/// errorstop job from being prompted at §83's `? ` on its way out -- here, of
/// all places, since the reason this report exists is that the terminal has
/// nothing left to answer with.
fn report_emergency_stop<G>(
    stores: &mut CommandContext<'_, G>,
    startup_terminal_line: &str,
    interactive: bool,
) -> tex_command::FatalError {
    let context = terminal_exhausted_context(stores, startup_terminal_line, interactive);
    let mut report = stores.print_err("Emergency stop");
    report.context(context);
    let help = if interactive {
        "End of file on the terminal!"
    } else {
        "*** (job aborted, no legal \\end found)"
    };
    report.help(&[help]);
    report.succumb();
    tex_command::FatalError::emergency_stop(help)
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
pub(crate) fn finish_job<G>(
    stores: &mut Universe<G>,
    profile: CommandProfile,
    binary: EngineBinaryIdentity,
    usage: EngineUsageStatistics,
    job_name: &str,
    dvi: Option<DviJobOutput>,
    pdf: Option<&mut PdfJobFinalizationReport>,
) {
    let statistics_left_file_offset_open = print_usage_statistics(stores, binary, usage);
    print_dvi_report(stores, dvi, statistics_left_file_offset_open);
    print_pdf_report(stores, profile, pdf);
    print_transcript_note(stores, job_name);
    stores.printer().print_ln();
}

/// Completes pdfTeX's `pdf_error` path through §93 `succumb` and the
/// PDF-specific fatal close message before the host receives the unchanged
/// typed execution error.
pub(crate) fn report_pdf_fatal_error<G>(stores: &mut Universe<G>, message: &str) {
    stores
        .world_mut()
        .begin_terminal_publication(tex_state::TerminalPublicationPhase::PdfFatal);
    stores.print_err(message).succumb();
    stores
        .printer()
        .print_nl("!  ==> Fatal error occurred, no output PDF file produced!")
        .print_ln();
    stores.world_mut().commit_terminal_publication();
}

/// Emits pdfTeX's end-of-file warnings for navigation objects which were
/// referenced but never defined.
///
/// pdftex.web §§794--798 checks ordinary and structure destinations before
/// writing the document's remaining objects, while §1600 repairs a referenced
/// article thread whose bead list is empty. Object serialization is a host
/// concern in Umber, but these diagnostics depend only on checkpointed engine
/// state and therefore belong at the engine's finalization boundary.
pub(crate) fn report_pdf_navigation_warnings<G>(
    stores: &mut Universe<G>,
    missing: &[PdfNavigationWarning],
) -> bool {
    if missing.is_empty() {
        return false;
    }

    let mut printer = stores.printer();
    for warning in missing {
        match warning {
            PdfNavigationWarning::Destination(identity) => {
                printer.print_nl("pdfTeX warning (dest): ");
                print_pdf_navigation_identity(&mut printer, identity);
                printer
                    .print(" has been referenced but does not exist, replaced by a fixed one")
                    .print_ln()
                    .print_ln();
            }
            PdfNavigationWarning::StructureDestination(identity) => {
                printer.print("pdfTeX warning (structure dest): ");
                print_pdf_navigation_identity(&mut printer, identity);
                printer
                    .print(" has been referenced but does not exist")
                    .print_ln()
                    .print_ln();
            }
            PdfNavigationWarning::Thread(identity) => {
                printer.print_nl("pdfTeX warning (thread): destination ");
                print_pdf_navigation_identity(&mut printer, identity);
                printer
                    .print(" has been referenced but does not exist, replaced by a fixed one")
                    .print_ln()
                    .print_ln();
            }
        }
    }
    true
}

fn print_pdf_navigation_identity<G>(
    printer: &mut Printer<'_, G>,
    identity: &tex_state::PdfDestinationIdentity,
) {
    match identity {
        tex_state::PdfDestinationIdentity::Name(name) => {
            printer.print("name{");
            for &byte in name {
                printer.print_char(char::from(byte));
            }
            printer.print("}");
        }
        tex_state::PdfDestinationIdentity::Number(number) => {
            printer.print("num").print_int(*number);
        }
    }
}

fn print_pdf_report<G>(
    stores: &mut Universe<G>,
    profile: CommandProfile,
    report: Option<&mut PdfJobFinalizationReport>,
) {
    if !profile.capabilities().supports_pdftex() {
        return;
    }
    let Some(report) = report else { return };
    if std::mem::replace(&mut report.reported, true) {
        return;
    }
    let mut printer = stores.printer();
    printer.print_nl("PDF statistics:").print_nl(" ");
    print_u32(&mut printer, report.pdf_objects);
    printer.print(" PDF objects out of 1000 (max. 8388607)");
    if report.compressed_objects > 0 {
        printer.print_nl(" ");
        print_u32(&mut printer, report.compressed_objects);
        printer.print(" compressed objects within ");
        print_u32(&mut printer, report.object_streams);
        printer.print(" object stream");
        if report.object_streams != 1 {
            printer.print_char('s');
        }
    }
    printer.print_nl(" ");
    print_u32(&mut printer, report.named_destinations);
    printer
        .print(" named destinations out of 1000 (max. 500000)")
        .print_nl(" ");
    print_u32(&mut printer, report.extra_memory_words);
    printer.print(" words of extra memory for PDF output out of 10000 (max. 10000000)");
}

fn print_u32<G>(printer: &mut Printer<'_, G>, value: u32) {
    printer.print_int(i32::try_from(value).unwrap_or(i32::MAX));
}

fn print_usage_statistics<G>(
    stores: &mut Universe<G>,
    binary: EngineBinaryIdentity,
    usage: EngineUsageStatistics,
) -> bool {
    if stores.int_param(IntParam::TRACING_STATS) <= 0 {
        return false;
    }
    let file_offset_was_open = stores.printer().log_offset() > 0;
    // TeX82 §1333 deliberately uses `wlog*` rather than the live selector
    // for this block: statistics belong to the transcript even when ordinary
    // job framing is going to both terminal and log.
    let mut printer = Printer::new(stores, Selector::LogOnly);
    if printer.log_offset() > 0 {
        printer.print_char(' ');
    }
    printer
        .print_nl("Here is how much of TeX's memory you used:")
        .print_nl(" ");
    print_usize(&mut printer, usage.strings);
    printer.print(" strings out of ");
    print_usize(&mut printer, usage.string_capacity);
    printer.print_nl(" ");
    print_usize(&mut printer, usage.string_characters);
    printer.print(" string characters out of ");
    print_usize(&mut printer, usage.string_character_capacity);
    printer.print_nl(" ");
    print_usize(&mut printer, usage.memory_words);
    printer.print(" words of memory out of ");
    print_usize(&mut printer, usage.memory_word_capacity);
    printer.print_nl(" ");
    print_usize(&mut printer, usage.control_sequences);
    let (hash_size, hash_extra) = binary.control_sequence_capacity();
    printer.print(" multiletter control sequences out of ");
    print_u32(&mut printer, hash_size);
    printer.print_char('+');
    print_u32(&mut printer, hash_extra);
    printer.print_nl(" ");
    print_usize(&mut printer, usage.font_info_words);
    printer.print(" words of font info for ");
    print_usize(&mut printer, usage.fonts);
    printer.print(" font");
    if usage.fonts != 1 {
        printer.print_char('s');
    }
    printer.print(", out of 20000 for 75").print_nl(" ");
    print_usize(&mut printer, usage.hyphenation_exceptions);
    printer.print(" hyphenation exception");
    if usage.hyphenation_exceptions != 1 {
        printer.print_char('s');
    }
    printer.print(" out of ");
    print_usize(&mut printer, usage.hyphenation_exception_capacity);
    printer.print_nl(" ");
    print_stack_usage(&mut printer, usage);
    file_offset_was_open
}

fn print_stack_usage<G>(printer: &mut Printer<'_, G>, usage: EngineUsageStatistics) {
    for (value, suffix) in [
        (usage.input_stack, "i,"),
        (usage.nest_stack, "n,"),
        (usage.parameter_stack, "p,"),
        (usage.buffer_stack, "b,"),
        (usage.save_stack, "s"),
    ] {
        print_usize(printer, value);
        printer.print(suffix);
    }
    // TeX82 §1334 ends this final direct-to-log row with `wlog_ln`.
    // Closing the log line independently matters when §642's following
    // `print_nl` sees a still-open terminal line and breaks both sinks.
    printer
        .print(" stack positions out of 200i,40n,60p,500b,600s")
        .print_ln();
}

fn print_usize<G>(printer: &mut Printer<'_, G>, value: usize) {
    printer.print_int(i32::try_from(value).unwrap_or(i32::MAX));
}

/// TeX82 §1328's format-file announcement and newly built `format_ident`.
///
/// The serialized bytes and displayed output name are host concerns. The host
/// calls this only after atomic publication succeeds, matching §1328's
/// successful-open ordering and preventing a failed output from claiming a
/// dump that did not happen.
pub fn confirm_format_dump_publication<G>(
    stores: &mut Universe<G>,
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
        .print(&ident.format_name)
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
fn print_dvi_report<G>(
    stores: &mut Universe<G>,
    dvi: Option<DviJobOutput>,
    statistics_left_file_offset_open: bool,
) {
    // TeX82 §1334 emits its statistics through direct `wlog*` macros. Those
    // writes change the transcript bytes but not §54's `file_offset`, so an
    // offset that was open before the report still satisfies §62's guard at
    // §642. Umber's structured log sink derives its offset from the rendered
    // bytes, so reproduce that one direct-write cursor effect at the owning
    // `print_nl` boundary.
    if statistics_left_file_offset_open {
        stores.printer().print_ln();
    }
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
fn print_transcript_note<G>(stores: &mut Universe<G>, job_name: &str) {
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
