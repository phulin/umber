//! Direct tests for tex.web's job framing: the start-up banner, §1335's
//! final-cleanup tail, and §1333's DVI/transcript report. See
//! `docs/job_framing.md` for the full account. §537/§362's `(name`/`)`
//! bracketing is tested next to the state it maintains, in
//! `tex_state::file_framing`.

use super::*;

use tex_command::{CommandHostCapabilities, RegisteredSourceKind, SourceRegistration};
use tex_state::{EffectRecord, InteractionMode, JobClock, PrintSink, World};

#[test]
fn print_two_uses_absolute_last_two_digits() {
    for (value, expected) in [(-1, "01"), (-9, "09"), (-10, "10"), (101, "01")] {
        assert_eq!(super::print_two(value), expected);
    }
}

#[test]
fn unclosed_group_report_uses_live_escapechar_and_interaction_selector() {
    // TeX82 §§63/1335: final_cleanup constructs this report with print_esc,
    // so an out-of-range escape character contributes no prefix. The active
    // interaction selector still decides whether the report reaches the
    // terminal as well as the transcript.
    for (interaction, terminal_visible) in [
        (InteractionMode::Nonstop, true),
        (InteractionMode::Batch, false),
    ] {
        for (escape, expected) in [
            (i32::from(b'@'), "(@end occurred inside a group at level 1)"),
            (256, "(end occurred inside a group at level 1)"),
        ] {
            let mut stores = Universe::new();
            stores.set_interaction_mode(interaction);
            stores.set_int_param(IntParam::ESCAPE_CHAR, escape);
            stores.enter_group();

            report_unclosed_groups(&mut stores);

            assert_eq!(log_text(&stores), expected);
            assert_eq!(
                terminal_text(&stores),
                if terminal_visible { expected } else { "" }
            );
        }
    }
}

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
        JobEngineFraming {
            binary: EngineBinaryIdentity::Pdftex14029,
            extended_mode: false,
        },
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
        JobEngineFraming {
            binary: EngineBinaryIdentity::Etex26,
            extended_mode: true,
        },
        "etex.tex",
    );

    assert_eq!(
        terminal_text(&stores),
        format!("{ETEX26_BANNER} (INITEX)\nentering extended mode\n")
    );
    assert_eq!(
        log_text(&stores),
        format!("{ETEX26_BANNER} (INITEX)  9 JUL 2026 13:36\nentering extended mode\n**etex.tex\n")
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
        JobEngineFraming {
            binary: EngineBinaryIdentity::Pdftex14029,
            extended_mode: false,
        },
        "a.tex",
    );
    begin_job(
        &mut job,
        &mut stores,
        &mut capabilities,
        true,
        None,
        JobEngineFraming {
            binary: EngineBinaryIdentity::Pdftex14029,
            extended_mode: false,
        },
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

    finish_job(&mut stores, CommandProfile::TEX82, "show-box", None, None);

    assert_eq!(
        terminal_text(&stores),
        "No pages of output.\nTranscript written on show-box.log.\n"
    );
    // The transcript note is terminal-only.
    assert_eq!(log_text(&stores), "No pages of output.\n");
}

#[test]
fn finish_job_suppresses_usage_report_when_tracingstats_is_zero() {
    let mut stores = Universe::new();
    stores.set_int_param_global(IntParam::TRACING_STATS, 0);
    finish_job(&mut stores, CommandProfile::TEX82, "stats", None, None);
    assert!(!terminal_text(&stores).contains("Here is how much"));
    assert!(!log_text(&stores).contains("Here is how much"));
}

#[test]
fn finish_job_prints_tex82_usage_report_only_to_log_before_dvi_tail() {
    for (interaction, terminal) in [
        (InteractionMode::ErrorStop, true),
        (InteractionMode::Batch, false),
    ] {
        let mut stores = Universe::new();
        stores.set_interaction_mode(interaction);
        stores.set_int_param_global(IntParam::TRACING_STATS, 1);
        finish_job(&mut stores, CommandProfile::TEX82, "stats", None, None);
        let log = log_text(&stores);
        let report = "Here is how much of TeX's memory you used:\n";
        assert!(log.starts_with(report));
        assert!(log.contains(" strings out of 13973\n"));
        assert!(log.contains(" string characters out of 18159\n"));
        assert!(log.contains(" words of memory out of 250000\n"));
        assert!(log.contains(" multiletter control sequences out of 15000+0\n"));
        assert!(log.contains(" words of font info for 0 fonts, out of 20000 for 75\n"));
        assert!(log.contains(" hyphenation exceptions out of 307\n"));
        assert!(log.contains(
            "0i,0n,0p,0b,0s stack positions out of 200i,40n,60p,500b,600s\nNo pages of output."
        ));
        assert!(!terminal_text(&stores).contains(report));
        assert_eq!(
            terminal_text(&stores).contains("No pages of output."),
            terminal
        );
    }
}

#[test]
fn usage_report_separates_a_partial_final_cleanup_line_before_breaking() {
    // TeX82 §1333's log-only usage report preserves the separator at the
    // final-cleanup column before its first `wlog_cr`-style line break.
    let mut stores = Universe::new();
    stores.set_int_param_global(IntParam::TRACING_STATS, 1);
    Printer::new(&mut stores, Selector::LogOnly).print("unfinished)");

    finish_job(&mut stores, CommandProfile::TEX82, "stats", None, None);

    assert!(
        log_text(&stores).starts_with("unfinished) \nHere is how much of TeX's memory you used:\n")
    );
}

#[test]
fn finish_job_keeps_log_only_statistics_before_the_committed_page_report() {
    let mut stores = run_source_to_end(br"\shipout\hbox{}\end");
    stores.set_int_param_global(IntParam::TRACING_STATS, 1);

    finish_job(
        &mut stores,
        CommandProfile::TEX82,
        "doc",
        Some(DviJobOutput {
            file_name: "doc.dvi".into(),
            byte_len: 44,
        }),
        None,
    );

    let report = "Here is how much of TeX's memory you used:";
    let output = "Output written on doc.dvi (1 page, 44 bytes).";
    assert!(!terminal_text(&stores).contains(report));
    assert!(terminal_text(&stores).contains(output));
    let log = log_text(&stores);
    assert!(log.find(report).expect("statistics") < log.find(output).expect("DVI report"));
}

#[test]
fn finish_job_reports_output_written_with_the_singular_page_form() {
    let mut stores = run_source_to_end(br"\shipout\hbox{}\end");
    assert_eq!(stores.world().artifact_commits().len(), 1);

    finish_job(
        &mut stores,
        CommandProfile::TEX82,
        "doc",
        Some(DviJobOutput {
            file_name: "doc.dvi".into(),
            byte_len: 44,
        }),
        None,
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
        CommandProfile::TEX82,
        "doc",
        Some(DviJobOutput {
            file_name: "doc.dvi".into(),
            byte_len: 88,
        }),
        None,
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
    finish_job(&mut stores, CommandProfile::TEX82, "doc", None, None);
}

#[test]
fn finish_job_transcript_note_is_terminal_only_and_silent_in_batch_mode() {
    let mut stores = Universe::new();
    stores.set_interaction_mode(InteractionMode::Batch);

    finish_job(&mut stores, CommandProfile::TEX82, "show-box", None, None);

    assert!(!terminal_text(&stores).contains("Transcript written on"));
    assert!(!log_text(&stores).contains("Transcript written on"));
}

#[test]
fn pdf_finalization_report_is_profile_aware_exact_and_one_shot() {
    let mut stores = Universe::new();
    let mut report = PdfJobFinalizationReport::new(17, 6, 2, 3, 41);
    finish_job(
        &mut stores,
        CommandProfile::PDFTEX14029,
        "doc",
        None,
        Some(&mut report),
    );
    finish_job(
        &mut stores,
        CommandProfile::PDFTEX14029,
        "doc",
        None,
        Some(&mut report),
    );
    let terminal = terminal_text(&stores);
    let expected = "PDF statistics:\n 17 PDF objects out of 1000 (max. 8388607)\n 6 compressed objects within 2 object streams\n 3 named destinations out of 1000 (max. 500000)\n 41 words of extra memory for PDF output out of 10000 (max. 10000000)";
    assert_eq!(terminal.matches("PDF statistics:").count(), 1);
    assert!(
        terminal.contains(expected),
        "terminal text was: {terminal:?}"
    );
}

#[test]
fn tex_and_etex_profiles_never_render_a_pdf_finalization_report() {
    for profile in [CommandProfile::TEX82, CommandProfile::ETEX26] {
        let mut stores = Universe::new();
        let mut report = PdfJobFinalizationReport::new(1, 0, 0, 0, 1);
        finish_job(&mut stores, profile, "doc", None, Some(&mut report));
        assert!(!terminal_text(&stores).contains("PDF statistics:"));
    }
}

/// A loaded-format job prints the format's identity on both sinks, but not
/// the same text: web2c's replacement for §61 prints `dump_name` on the
/// terminal (no dump date, because the banner precedes reading the format
/// file), while §536's `slow_print(format_ident)` prints §1328's dumped
/// string, which carries the date. Both spellings are what the pinned pdfTeX
/// 1.40.29 oracle emits for `-fmt=etex-loaded`.
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
        dump_name: "etex-loaded".to_owned(),
        format_name: "etex-loaded".to_owned(),
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
        JobEngineFraming {
            binary: EngineBinaryIdentity::Etex26,
            extended_mode: true,
        },
        "etex-loaded-state-reset.tex",
    );

    assert_eq!(
        terminal_text(&stores),
        format!("{ETEX26_BANNER} (preloaded format=etex-loaded)\nentering extended mode\n")
    );
    assert_eq!(
        log_text(&stores),
        format!(
            "{ETEX26_BANNER} (preloaded format=etex-loaded 2026.7.9)  9 JUL 2026 13:36\n\
             entering extended mode\n**etex-loaded-state-reset.tex\n"
        )
    );
}

#[test]
fn startup_selector_is_echoed_without_becoming_the_job_name() {
    let mut stores = Universe::new();
    let mut job = JobFraming::default();
    let mut capabilities = CommandHostCapabilities::default();

    begin_job_with_terminal_banner(
        &mut job,
        &mut stores,
        &mut capabilities,
        false,
        None,
        JobEngineFraming {
            binary: EngineBinaryIdentity::Tex82,
            extended_mode: false,
        },
        StartupLineFraming {
            first_line: "&trip inputs/trip.tex",
            input_name: "inputs/trip.tex",
            terminal_banner: true,
        },
    );

    // TeX82 §534 echoes the complete terminal buffer, while §§528--529
    // select the filename's name component for `job_name`.
    assert!(log_text(&stores).contains("**&trip inputs/trip.tex\n"));
    assert_eq!(capabilities.job_name(), "trip");
}

#[test]
fn loaded_tex82_banner_is_selected_by_runtime_profile_without_etex_or_pdftex_text() {
    let mut stores = Universe::new();
    let mut job = JobFraming::default();
    let mut capabilities = CommandHostCapabilities::default();
    let format = PreloadedFormat {
        dump_name: "umber-tex82-oracle".to_owned(),
        format_name: "trip".to_owned(),
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
        JobEngineFraming {
            binary: EngineBinaryIdentity::Tex82,
            extended_mode: false,
        },
        "trip.tex",
    );

    let terminal = terminal_text(&stores);
    assert_eq!(
        terminal,
        format!("{TEX82_BANNER} (preloaded format=umber-tex82-oracle)\n")
    );
    assert!(!terminal.contains("pdfTeX"));
    assert!(!terminal.contains("e-TeX"));
    let log = log_text(&stores);
    assert!(log.starts_with(TEX82_BANNER));
    assert!(log.contains("(preloaded format=trip 2026.7.9)"));
}

#[test]
fn engine_binary_compatibility_is_a_superset_relation_not_a_dialect_alias() {
    assert!(EngineBinaryIdentity::Tex82.supports(tex_command::CommandProfile::TEX82));
    assert!(!EngineBinaryIdentity::Tex82.supports(tex_command::CommandProfile::ETEX26));
    assert!(EngineBinaryIdentity::Etex26.supports(tex_command::CommandProfile::TEX82));
    assert!(EngineBinaryIdentity::Etex26.supports(tex_command::CommandProfile::ETEX26));
    assert!(!EngineBinaryIdentity::Etex26.supports(tex_command::CommandProfile::PDFTEX14029));
    assert!(EngineBinaryIdentity::Pdftex14029.supports(tex_command::CommandProfile::TEX82));
    assert!(EngineBinaryIdentity::Pdftex14029.supports(tex_command::CommandProfile::ETEX26));
    assert!(EngineBinaryIdentity::Pdftex14029.supports(tex_command::CommandProfile::PDFTEX14029));
}
