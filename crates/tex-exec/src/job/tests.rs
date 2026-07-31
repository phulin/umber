//! Direct tests for tex.web's job framing: the start-up banner, §1335's
//! final-cleanup tail, and §1333's DVI/transcript report. See
//! `docs/job_framing.md` for the full account. §537/§362's `(name`/`)`
//! bracketing is tested next to the state it maintains, in
//! `tex_state::file_framing`.

use super::*;

use tex_command::{CommandHostCapabilities, RegisteredSourceKind, SourceRegistration};
use tex_state::{EffectRecord, InteractionMode, JobClock, PrintSink, World};

use crate::{CanonicalMainControl, MainControlStep};

fn channel_text(stores: &Universe, matches_sink: impl Fn(PrintSink) -> bool) -> String {
    stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|record| match record {
            EffectRecord::StreamWrite { sink, text } if matches_sink(*sink) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn terminal_text(stores: &Universe) -> String {
    channel_text(stores, |sink| {
        matches!(sink, PrintSink::Terminal | PrintSink::TerminalAndLog)
    })
}

fn log_text(stores: &Universe) -> String {
    channel_text(stores, |sink| {
        matches!(sink, PrintSink::Log | PrintSink::TerminalAndLog)
    })
}

#[test]
fn format_dump_publication_confirmation_obeys_selector_and_is_one_shot() {
    let cases = [
        (InteractionMode::ErrorStop, true),
        (InteractionMode::Scroll, true),
        (InteractionMode::Nonstop, true),
        (InteractionMode::Batch, false),
    ];
    for (interaction, terminal) in cases {
        let mut stores = Universe::new();
        stores.set_interaction_mode(interaction);
        let mut receipt = FormatDumpReceipt::new("plain".into(), 2026, 7, 30);
        confirm_format_dump_publication(&mut stores, &mut receipt, "published-name.fmt");
        confirm_format_dump_publication(&mut stores, &mut receipt, "duplicate.fmt");
        let expected =
            "Beginning to dump on file published-name.fmt\n (preloaded format=plain 2026.7.30)";
        assert_eq!(terminal_text(&stores), if terminal { expected } else { "" });
        assert_eq!(log_text(&stores), expected);
    }
}

/// Runs a source through a fresh INITEX session to `\end`/end-of-input and
/// returns the resulting `Universe`, for tests that need a real committed
/// page count (`Universe::world().artifact_commits()` is populated only by
/// the engine's own `\shipout` handling, not by any test-visible setter).
fn run_source_to_end(source: &[u8]) -> Universe {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    let registered = control
        .command_mut()
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            source.to_vec(),
        ))
        .expect("source registers");
    control
        .command_mut()
        .open_registered_source(registered)
        .expect("source opens");
    loop {
        match control.step(&mut stores).expect("step executes") {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
    stores
}

#[test]
fn begin_job_prints_the_banner_and_clock_stamped_first_line_on_each_channel() {
    let clock = JobClock {
        time: 13 * 60 + 36,
        second: 7,
        day: 9,
        month: 7,
        year: 2026,
    };
    let mut stores = Universe::with_world(World::memory_with_clock(clock));
    let mut job = JobFraming::default();
    let mut capabilities = CommandHostCapabilities::default();

    begin_job(
        &mut job,
        &mut stores,
        &mut capabilities,
        true,
        None,
        false,
        "show-box.tex",
    );

    // §61: the terminal's banner plus `format_ident` and a trailing newline,
    // no clock.
    assert_eq!(terminal_text(&stores), format!("{BANNER} (INITEX)\n"));
    // §536/§534: the log's banner carries `format_ident` and the clock, then
    // a `**` line with the job's first line and a trailing newline.
    assert_eq!(
        log_text(&stores),
        format!("{BANNER} (INITEX)  9 JUL 2026 13:36\n**show-box.tex\n")
    );
    assert_eq!(capabilities.job_name(), "show-box");
}

#[test]
fn begin_job_prints_entering_extended_mode_on_both_channels_before_the_star_star_line() {
    let clock = JobClock {
        time: 13 * 60 + 36,
        second: 7,
        day: 9,
        month: 7,
        year: 2026,
    };
    let mut stores = Universe::with_world(World::memory_with_clock(clock));
    let mut job = JobFraming::default();
    let mut capabilities = CommandHostCapabilities::default();

    begin_job(
        &mut job,
        &mut stores,
        &mut capabilities,
        true,
        None,
        true,
        "etex.tex",
    );

    assert_eq!(
        terminal_text(&stores),
        format!("{BANNER} (INITEX)\nentering extended mode\n")
    );
    assert_eq!(
        log_text(&stores),
        format!("{BANNER} (INITEX)  9 JUL 2026 13:36\nentering extended mode\n**etex.tex\n")
    );
}

#[test]
fn begin_job_called_twice_prints_the_banner_only_once() {
    let mut stores = Universe::new();
    let mut job = JobFraming::default();
    let mut capabilities = CommandHostCapabilities::default();

    begin_job(
        &mut job,
        &mut stores,
        &mut capabilities,
        true,
        None,
        false,
        "a.tex",
    );
    begin_job(
        &mut job,
        &mut stores,
        &mut capabilities,
        true,
        None,
        false,
        "a.tex",
    );

    assert_eq!(terminal_text(&stores), format!("{BANNER} (INITEX)\n"));
    assert_eq!(
        terminal_text(&stores).matches(BANNER).count(),
        1,
        "begin_job must print the banner only once even when called twice"
    );
}

const HISTORY_NOTE: &str = "(see the transcript file for additional information)";

#[test]
fn history_note_is_silent_when_history_is_spotless() {
    let mut stores = Universe::new();
    print_history_note(&mut stores);
    assert!(terminal_text(&stores).is_empty());
}

#[test]
fn history_note_prints_terminal_only_below_errorstop_mode() {
    let mut stores = Universe::new();
    stores.set_interaction_mode(InteractionMode::Nonstop);
    stores
        .world_mut()
        .error_channel_mut()
        .record_error_history();

    print_history_note(&mut stores);

    assert_eq!(terminal_text(&stores), HISTORY_NOTE);
    assert!(log_text(&stores).is_empty());
}

#[test]
fn history_note_is_silent_in_errorstop_mode_unless_history_is_only_a_warning() {
    // `InteractionMode::default()` is `ErrorStop`.
    let mut raised_error = Universe::new();
    raised_error
        .world_mut()
        .error_channel_mut()
        .record_error_history();
    print_history_note(&mut raised_error);
    assert!(terminal_text(&raised_error).is_empty());

    let mut raised_warning = Universe::new();
    raised_warning
        .world_mut()
        .error_channel_mut()
        .record_warning_history();
    print_history_note(&mut raised_warning);
    assert_eq!(terminal_text(&raised_warning), HISTORY_NOTE);
}

#[test]
fn history_note_is_silent_in_batch_mode_even_when_history_is_raised() {
    // Batch's selector is `log_only`, never `term_and_log`, so tex.web's
    // `selector=term_and_log` guard fails regardless of `history`.
    let mut stores = Universe::new();
    stores.set_interaction_mode(InteractionMode::Batch);
    stores
        .world_mut()
        .error_channel_mut()
        .record_fatal_history();

    print_history_note(&mut stores);

    assert!(terminal_text(&stores).is_empty());
    assert!(log_text(&stores).is_empty());
}

#[test]
fn finish_job_reports_no_pages_of_output_for_a_zero_page_job() {
    let mut stores = Universe::new();

    finish_job(&mut stores, "show-box", None);

    assert_eq!(
        terminal_text(&stores),
        "No pages of output.\nTranscript written on show-box.log.\n"
    );
    // The transcript note is terminal-only.
    assert_eq!(log_text(&stores), "No pages of output.\n");
}

#[test]
fn finish_job_reports_output_written_with_the_singular_page_form() {
    let mut stores = run_source_to_end(br"\shipout\hbox{}\end");
    assert_eq!(stores.world().artifact_commits().len(), 1);

    finish_job(
        &mut stores,
        "doc",
        Some(DviJobOutput {
            file_name: "doc.dvi".into(),
            byte_len: 44,
        }),
    );

    assert!(
        terminal_text(&stores).contains("Output written on doc.dvi (1 page, 44 bytes).\n"),
        "terminal text was: {:?}",
        terminal_text(&stores)
    );
}

#[test]
fn finish_job_reports_output_written_with_the_plural_page_form() {
    let mut stores = run_source_to_end(br"\shipout\hbox{}\shipout\hbox{}\end");
    assert_eq!(stores.world().artifact_commits().len(), 2);

    finish_job(
        &mut stores,
        "doc",
        Some(DviJobOutput {
            file_name: "doc.dvi".into(),
            byte_len: 88,
        }),
    );

    assert!(
        terminal_text(&stores).contains("Output written on doc.dvi (2 pages, 88 bytes).\n"),
        "terminal text was: {:?}",
        terminal_text(&stores)
    );
}

#[test]
#[should_panic(expected = "no `DviJobOutput` was supplied")]
fn finish_job_refuses_to_fabricate_a_byte_count_for_a_shipped_page() {
    let mut stores = run_source_to_end(br"\shipout\hbox{}\end");
    finish_job(&mut stores, "doc", None);
}

#[test]
fn finish_job_transcript_note_is_terminal_only_and_silent_in_batch_mode() {
    let mut stores = Universe::new();
    stores.set_interaction_mode(InteractionMode::Batch);

    finish_job(&mut stores, "show-box", None);

    assert!(!terminal_text(&stores).contains("Transcript written on"));
    assert!(!log_text(&stores).contains("Transcript written on"));
}

/// A loaded-format job prints the format's identity on both sinks, but not
/// the same text: web2c's replacement for §61 prints `dump_name` on the
/// terminal (no dump date, because the banner precedes reading the format
/// file), while §536's `slow_print(format_ident)` prints §1328's dumped
/// string, which carries the date. Both spellings are what the pinned pdfTeX
/// 1.40.27 oracle emits for `-fmt=etex-loaded`.
#[test]
fn begin_job_frames_a_preloaded_format_with_a_dated_log_and_an_undated_terminal() {
    let clock = JobClock {
        time: 13 * 60 + 36,
        second: 7,
        day: 9,
        month: 7,
        year: 2026,
    };
    let mut stores = Universe::with_world(World::memory_with_clock(clock));
    let mut job = JobFraming::default();
    let mut capabilities = CommandHostCapabilities::default();
    let format = PreloadedFormat {
        name: "etex-loaded".to_owned(),
        year: 2026,
        month: 7,
        day: 9,
    };

    begin_job(
        &mut job,
        &mut stores,
        &mut capabilities,
        false,
        Some(&format),
        true,
        "etex-loaded-state-reset.tex",
    );

    assert_eq!(
        terminal_text(&stores),
        format!("{BANNER} (preloaded format=etex-loaded)\nentering extended mode\n")
    );
    assert_eq!(
        log_text(&stores),
        format!(
            "{BANNER} (preloaded format=etex-loaded 2026.7.9)  9 JUL 2026 13:36\n\
             entering extended mode\n**etex-loaded-state-reset.tex\n"
        )
    );
}
