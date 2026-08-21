//! Direct tests for tex.web's job framing: the start-up banner, §1335's
//! final-cleanup tail, and §1333's DVI/transcript report. See
//! `docs/job_framing.md` for the full account. §537/§362's `(name`/`)`
//! bracketing is tested next to the state it maintains, in
//! `tex_state::file_framing`.

use super::*;

use tex_command::{CommandHostCapabilities, RegisteredSourceKind, SourceRegistration};
use tex_state::{EffectRecord, InteractionMode, JobClock, PrintSink, Universe, World};

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
            crate::test_harness::with_universe(|universe| {
                universe.set_interaction_mode(interaction);
                crate::test_harness::assign_int_param(
                    universe,
                    IntParam::ESCAPE_CHAR,
                    escape,
                    tex_state::AssignmentScope::Global,
                )
                .expect("escape character assignment");
                crate::test_harness::begin_group(universe, tex_state::GroupKind::Simple, 0)
                    .expect("test group opens");

                report_unclosed_groups(universe, 1);

                assert_eq!(log_text(universe), expected);
                assert_eq!(
                    terminal_text(universe),
                    if terminal_visible { expected } else { "" }
                );
            });
        }
    }
}

use crate::{MainControl, MainControlStep};

fn channel_text<G>(universe: &Universe<G>, matches_sink: impl Fn(PrintSink) -> bool) -> String {
    universe
        .world()
        .effect_records()
        .iter()
        .filter_map(|record| match record {
            EffectRecord::StreamWrite { sink, text } if matches_sink(*sink) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn terminal_text<G>(universe: &Universe<G>) -> String {
    channel_text(universe, |sink| {
        matches!(sink, PrintSink::Terminal | PrintSink::TerminalAndLog)
    })
}

fn log_text<G>(universe: &Universe<G>) -> String {
    channel_text(universe, |sink| {
        matches!(sink, PrintSink::Log | PrintSink::TerminalAndLog)
    })
}

fn finish_test_job<G>(
    universe: &mut Universe<G>,
    profile: CommandProfile,
    binary: EngineBinaryIdentity,
    job_name: &str,
    dvi: Option<DviJobOutput>,
    pdf: Option<&mut PdfJobFinalizationReport>,
) {
    let usage = crate::test_harness::with_admitted(universe, |context| {
        context.detach_engine_usage_statistics()
    });
    finish_job(universe, profile, binary, usage, job_name, dvi, pdf);
}

fn assign_global_int<G>(universe: &mut Universe<G>, parameter: IntParam, value: i32) {
    crate::test_harness::assign_int_param(
        universe,
        parameter,
        value,
        tex_state::AssignmentScope::Global,
    )
    .expect("job fixture integer assignment");
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
        crate::test_harness::with_universe(|universe| {
            universe.set_interaction_mode(interaction);
            let mut receipt = FormatDumpReceipt::new("plain".into(), 2026, 7, 30);
            confirm_format_dump_publication(universe, &mut receipt, "published-name.fmt");
            confirm_format_dump_publication(universe, &mut receipt, "duplicate.fmt");
            let expected =
                "Beginning to dump on file published-name.fmt\n (preloaded format=plain 2026.7.30)";
            assert_eq!(
                terminal_text(universe),
                if terminal { expected } else { "" }
            );
            assert_eq!(log_text(universe), expected);
        });
    }
}

/// Runs a source through a fresh INITEX session to `\end`/end-of-input and
/// Runs a complete source through a fresh branded INITEX session, then lends
/// the still-live engine to assertions that need committed page evidence.
fn with_source_to_end<R>(
    source: &[u8],
    test: impl for<'id> FnOnce(
        &mut MainControl<tex_state::GenerationBrand<'id>>,
        &mut Universe<tex_state::GenerationBrand<'id>>,
    ) -> R,
) -> R {
    crate::test_harness::with_plain_universe(|universe| {
        let mut control = MainControl::tex82_initex(universe);
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
            match control.step(universe).expect("step executes") {
                MainControlStep::End | MainControlStep::EndOfInput => break,
                MainControlStep::Continue => {}
            }
        }
        test(&mut control, universe)
    })
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
    crate::test_harness::with_world_universe(World::memory_with_clock(clock), |universe| {
        let mut job = JobFraming::default();
        let mut capabilities = CommandHostCapabilities::default();

        begin_job(
            &mut job,
            universe,
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
        assert_eq!(terminal_text(universe), format!("{BANNER} (INITEX)\n"));
        // §536/§534: the log's banner carries `format_ident` and the clock, then
        // a `**` line with the job's first line and a trailing newline.
        assert_eq!(
            log_text(universe),
            format!("{BANNER} (INITEX)  9 JUL 2026 13:36\n**show-box.tex\n")
        );
        assert_eq!(capabilities.job_name(), "show-box");
    });
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
    crate::test_harness::with_world_universe(World::memory_with_clock(clock), |universe| {
        let mut job = JobFraming::default();
        let mut capabilities = CommandHostCapabilities::default();

        begin_job(
            &mut job,
            universe,
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
            terminal_text(universe),
            format!("{ETEX26_BANNER} (INITEX)\nentering extended mode\n")
        );
        assert_eq!(
            log_text(universe),
            format!(
                "{ETEX26_BANNER} (INITEX)  9 JUL 2026 13:36\nentering extended mode\n**etex.tex\n"
            )
        );
    });
}

#[test]
fn begin_job_called_twice_prints_the_banner_only_once() {
    crate::test_harness::with_universe(|universe| {
        let mut job = JobFraming::default();
        let mut capabilities = CommandHostCapabilities::default();

        begin_job(
            &mut job,
            universe,
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
            universe,
            &mut capabilities,
            true,
            None,
            JobEngineFraming {
                binary: EngineBinaryIdentity::Pdftex14029,
                extended_mode: false,
            },
            "a.tex",
        );

        assert_eq!(terminal_text(universe), format!("{BANNER} (INITEX)\n"));
        assert_eq!(
            terminal_text(universe).matches(BANNER).count(),
            1,
            "begin_job must print the banner only once even when called twice"
        );
    });
}

const HISTORY_NOTE: &str = "(see the transcript file for additional information)";

#[test]
fn history_note_is_silent_when_history_is_spotless() {
    crate::test_harness::with_universe(|universe| {
        print_history_note(universe);
        assert!(terminal_text(universe).is_empty());
    });
}

#[test]
fn history_note_prints_terminal_only_below_errorstop_mode() {
    crate::test_harness::with_universe(|universe| {
        universe.set_interaction_mode(InteractionMode::Nonstop);
        universe
            .world_mut()
            .error_channel_mut()
            .record_error_history();

        print_history_note(universe);

        assert_eq!(terminal_text(universe), HISTORY_NOTE);
        assert!(log_text(universe).is_empty());
    });
}

#[test]
fn history_note_is_silent_in_errorstop_mode_unless_history_is_only_a_warning() {
    crate::test_harness::with_universe(|universe| {
        universe.set_interaction_mode(InteractionMode::ErrorStop);
        universe
            .world_mut()
            .error_channel_mut()
            .record_error_history();
        print_history_note(universe);
        assert!(terminal_text(universe).is_empty());
    });

    crate::test_harness::with_universe(|universe| {
        universe.set_interaction_mode(InteractionMode::ErrorStop);
        universe
            .world_mut()
            .error_channel_mut()
            .record_warning_history();
        print_history_note(universe);
        assert_eq!(terminal_text(universe), HISTORY_NOTE);
    });
}

#[test]
fn history_note_is_silent_in_batch_mode_even_when_history_is_raised() {
    // Batch's selector is `log_only`, never `term_and_log`, so tex.web's
    // `selector=term_and_log` guard fails regardless of `history`.
    crate::test_harness::with_universe(|universe| {
        universe.set_interaction_mode(InteractionMode::Batch);
        universe
            .world_mut()
            .error_channel_mut()
            .record_fatal_history();

        print_history_note(universe);

        assert!(terminal_text(universe).is_empty());
        assert!(log_text(universe).is_empty());
    });
}

#[test]
fn finish_job_reports_no_pages_of_output_for_a_zero_page_job() {
    crate::test_harness::with_universe(|universe| {
        finish_test_job(
            universe,
            CommandProfile::TEX82,
            EngineBinaryIdentity::Tex82,
            "show-box",
            None,
            None,
        );

        assert_eq!(
            terminal_text(universe),
            "No pages of output.\nTranscript written on show-box.log.\n"
        );
        // The transcript note is terminal-only.
        assert_eq!(log_text(universe), "No pages of output.\n");
    });
}

#[test]
fn finish_job_suppresses_usage_report_when_tracingstats_is_zero() {
    crate::test_harness::with_universe(|universe| {
        assign_global_int(universe, IntParam::TRACING_STATS, 0);
        finish_test_job(
            universe,
            CommandProfile::TEX82,
            EngineBinaryIdentity::Tex82,
            "stats",
            None,
            None,
        );
        assert!(!terminal_text(universe).contains("Here is how much"));
        assert!(!log_text(universe).contains("Here is how much"));
    });
}

#[test]
fn finish_job_prints_tex82_usage_report_only_to_log_before_dvi_tail() {
    for (interaction, terminal) in [
        (InteractionMode::ErrorStop, true),
        (InteractionMode::Batch, false),
    ] {
        crate::test_harness::with_universe(|universe| {
            universe.set_interaction_mode(interaction);
            assign_global_int(universe, IntParam::TRACING_STATS, 1);
            finish_test_job(
                universe,
                CommandProfile::TEX82,
                EngineBinaryIdentity::Tex82,
                "stats",
                None,
                None,
            );
            let log = log_text(universe);
            let report = "Here is how much of TeX's memory you used:\n";
            assert!(log.starts_with(report));
            assert!(log.contains(" strings out of 13973\n"));
            assert!(log.contains(" string characters out of 18192\n"));
            assert!(log.contains(" words of memory out of 250000\n"));
            assert!(log.contains(" multiletter control sequences out of 15000+0\n"));
            assert!(log.contains(" words of font info for 0 fonts, out of 20000 for 75\n"));
            assert!(log.contains(" hyphenation exceptions out of 307\n"));
            assert!(log.contains(
                "0i,0n,0p,0b,0s stack positions out of 200i,40n,60p,500b,600s\nNo pages of output."
            ));
            assert!(!terminal_text(universe).contains(report));
            assert_eq!(
                terminal_text(universe).contains("No pages of output."),
                terminal
            );
        });
    }
}

#[test]
fn usage_report_hash_capacity_belongs_to_the_executing_binary() {
    // Web2C tex.ch [51.1332] owns `hash_extra` as executable runtime
    // configuration, and [51.1334] prints it independently of the loaded
    // format's command family. The older profile is the negative control.
    for (profile, binary, expected) in [
        (
            CommandProfile::ETEX26,
            EngineBinaryIdentity::Etex26,
            "15000+0",
        ),
        (
            CommandProfile::TEX82,
            EngineBinaryIdentity::Pdftex14029,
            "15000+600000",
        ),
    ] {
        crate::test_harness::with_universe(|universe| {
            assign_global_int(universe, IntParam::TRACING_STATS, 1);
            finish_test_job(universe, profile, binary, "stats", None, None);

            assert!(
                log_text(universe).contains(&format!(
                    "multiletter control sequences out of {expected}\n"
                )),
                "unexpected usage report: {:?}",
                log_text(universe)
            );
        });
    }
}

#[test]
fn usage_report_separates_a_partial_final_cleanup_line_before_breaking() {
    // TeX82 §1333's log-only usage report preserves the separator at the
    // final-cleanup column before its first `wlog_cr`-style line break.
    crate::test_harness::with_universe(|universe| {
        assign_global_int(universe, IntParam::TRACING_STATS, 1);
        crate::test_harness::with_admitted(universe, |context| {
            context
                .printer()
                .set_selector(Selector::LogOnly)
                .print("unfinished)");
        });

        finish_test_job(
            universe,
            CommandProfile::TEX82,
            EngineBinaryIdentity::Tex82,
            "stats",
            None,
            None,
        );

        assert!(
            log_text(universe)
                .starts_with("unfinished) \nHere is how much of TeX's memory you used:\n")
        );
    });
}

#[test]
fn usage_report_closes_log_before_shared_dvi_line_break() {
    // TeX82 §1334 closes its last `wlog_ln` row independently. When the
    // terminal remains mid-line, §642's `print_nl` then breaks both sinks:
    // one terminal newline, but a second newline in the already-closed log.
    crate::test_harness::with_universe(|universe| {
        assign_global_int(universe, IntParam::TRACING_STATS, 1);
        crate::test_harness::with_admitted(universe, |context| {
            context
                .printer()
                .set_selector(Selector::TermOnly)
                .print("terminal tail");
        });

        finish_test_job(
            universe,
            CommandProfile::TEX82,
            EngineBinaryIdentity::Tex82,
            "stats",
            None,
            None,
        );

        assert!(terminal_text(universe).starts_with("terminal tail\nNo pages of output."));
        assert!(log_text(universe).contains(
            "0i,0n,0p,0b,0s stack positions out of 200i,40n,60p,500b,600s\n\nNo pages of output."
        ));
    });
}

#[test]
fn direct_usage_report_preserves_the_open_log_cursor_for_the_dvi_break() {
    // TeX82 §§54/62/1334/642: §1334's direct `wlog*` writes do not
    // update `file_offset`. A line open before the statistics therefore
    // still makes §642's `print_nl` emit one shared line break after the
    // final statistics row. Batch mode is the negative control for a
    // terminal offset: only the stale log cursor can cause this blank line.
    with_source_to_end(br"\shipout\hbox{}\end", |_, universe| {
        universe.set_interaction_mode(InteractionMode::Batch);
        assign_global_int(universe, IntParam::TRACING_STATS, 1);
        crate::test_harness::with_admitted(universe, |context| {
            context
                .printer()
                .set_selector(Selector::LogOnly)
                .print(" )");
        });

        finish_test_job(
            universe,
            CommandProfile::TEX82,
            EngineBinaryIdentity::Tex82,
            "doc",
            Some(DviJobOutput {
                file_name: "doc.dvi".into(),
                byte_len: 44,
            }),
            None,
        );

        assert!(terminal_text(universe).is_empty());
        assert!(log_text(universe).contains(
            "0i,0n,0p,0b,0s stack positions out of 200i,40n,60p,500b,600s\n\n\
             Output written on doc.dvi (1 page, 44 bytes)."
        ));
    });
}

#[test]
fn finish_job_keeps_log_only_statistics_before_the_committed_page_report() {
    with_source_to_end(br"\shipout\hbox{}\end", |_, universe| {
        assign_global_int(universe, IntParam::TRACING_STATS, 1);

        finish_test_job(
            universe,
            CommandProfile::TEX82,
            EngineBinaryIdentity::Tex82,
            "doc",
            Some(DviJobOutput {
                file_name: "doc.dvi".into(),
                byte_len: 44,
            }),
            None,
        );

        let report = "Here is how much of TeX's memory you used:";
        let output = "Output written on doc.dvi (1 page, 44 bytes).";
        assert!(!terminal_text(universe).contains(report));
        assert!(terminal_text(universe).contains(output));
        let log = log_text(universe);
        assert!(log.find(report).expect("statistics") < log.find(output).expect("DVI report"));
    });
}

#[test]
fn finish_job_reports_output_written_with_the_singular_page_form() {
    with_source_to_end(br"\shipout\hbox{}\end", |_, universe| {
        assert_eq!(universe.world().artifact_commits().len(), 1);

        finish_test_job(
            universe,
            CommandProfile::TEX82,
            EngineBinaryIdentity::Tex82,
            "doc",
            Some(DviJobOutput {
                file_name: "doc.dvi".into(),
                byte_len: 44,
            }),
            None,
        );

        assert!(
            terminal_text(universe).contains("Output written on doc.dvi (1 page, 44 bytes).\n"),
            "terminal text was: {:?}",
            terminal_text(universe)
        );
    });
}

#[test]
fn finish_job_reports_output_written_with_the_plural_page_form() {
    with_source_to_end(br"\shipout\hbox{}\shipout\hbox{}\end", |_, universe| {
        assert_eq!(universe.world().artifact_commits().len(), 2);

        finish_test_job(
            universe,
            CommandProfile::TEX82,
            EngineBinaryIdentity::Tex82,
            "doc",
            Some(DviJobOutput {
                file_name: "doc.dvi".into(),
                byte_len: 88,
            }),
            None,
        );

        assert!(
            terminal_text(universe).contains("Output written on doc.dvi (2 pages, 88 bytes).\n"),
            "terminal text was: {:?}",
            terminal_text(universe)
        );
    });
}

#[test]
#[should_panic(expected = "no `DviJobOutput` was supplied")]
fn finish_job_refuses_to_fabricate_a_byte_count_for_a_shipped_page() {
    with_source_to_end(br"\shipout\hbox{}\end", |_, universe| {
        finish_test_job(
            universe,
            CommandProfile::TEX82,
            EngineBinaryIdentity::Tex82,
            "doc",
            None,
            None,
        );
    });
}

#[test]
fn finish_job_transcript_note_is_terminal_only_and_silent_in_batch_mode() {
    crate::test_harness::with_universe(|universe| {
        universe.set_interaction_mode(InteractionMode::Batch);

        finish_test_job(
            universe,
            CommandProfile::TEX82,
            EngineBinaryIdentity::Tex82,
            "show-box",
            None,
            None,
        );

        assert!(!terminal_text(universe).contains("Transcript written on"));
        assert!(!log_text(universe).contains("Transcript written on"));
    });
}

#[test]
fn pdf_finalization_report_is_profile_aware_exact_and_one_shot() {
    crate::test_harness::with_universe(|universe| {
        let mut report = PdfJobFinalizationReport::new(17, 6, 2, 3, 41);
        finish_test_job(
            universe,
            CommandProfile::PDFTEX14029,
            EngineBinaryIdentity::Pdftex14029,
            "doc",
            None,
            Some(&mut report),
        );
        finish_test_job(
            universe,
            CommandProfile::PDFTEX14029,
            EngineBinaryIdentity::Pdftex14029,
            "doc",
            None,
            Some(&mut report),
        );
        let terminal = terminal_text(universe);
        let expected = "PDF statistics:\n 17 PDF objects out of 1000 (max. 8388607)\n 6 compressed objects within 2 object streams\n 3 named destinations out of 1000 (max. 500000)\n 41 words of extra memory for PDF output out of 10000 (max. 10000000)";
        assert_eq!(terminal.matches("PDF statistics:").count(), 1);
        assert!(
            terminal.contains(expected),
            "terminal text was: {terminal:?}"
        );
    });
}

#[test]
fn pdf_fatal_error_has_pdftex_channel_asymmetry() {
    crate::test_harness::with_universe(|universe| {
        universe.set_interaction_mode(InteractionMode::Nonstop);
        report_pdf_fatal_error(
            universe,
            "pdfTeX error (ext1): num identifier must be positive",
        );

        assert_eq!(
            terminal_text(universe),
            "! pdfTeX error (ext1): num identifier must be positive.\n!  ==> Fatal error occurred, no output PDF file produced!\n"
        );
        assert_eq!(
            log_text(universe),
            "! pdfTeX error (ext1): num identifier must be positive.\n\n!  ==> Fatal error occurred, no output PDF file produced!\n"
        );
        assert_eq!(
            universe.world().error_channel().history(),
            ErrorHistory::FatalErrorStop
        );
    });
}

#[test]
fn tex_and_etex_profiles_never_render_a_pdf_finalization_report() {
    for profile in [CommandProfile::TEX82, CommandProfile::ETEX26] {
        crate::test_harness::with_universe(|universe| {
            let mut report = PdfJobFinalizationReport::new(1, 0, 0, 0, 1);
            finish_test_job(
                universe,
                profile,
                EngineBinaryIdentity::for_profile(profile),
                "doc",
                None,
                Some(&mut report),
            );
            assert!(!terminal_text(universe).contains("PDF statistics:"));
        });
    }
}

#[test]
fn pdf_navigation_finalization_reports_only_unresolved_objects_in_source_order() {
    use tex_state::PdfDestinationIdentity::{Name, Number};

    crate::test_harness::with_universe(|universe| {
        let missing = [
            PdfNavigationWarning::Destination(Name(b"missing-regular".to_vec())),
            PdfNavigationWarning::StructureDestination(Name(b"missing-structure".to_vec())),
            PdfNavigationWarning::Thread(Name(b"missing-thread".to_vec())),
        ];

        assert!(report_pdf_navigation_warnings(universe, &missing));

        let expected = concat!(
            "pdfTeX warning (dest): name{missing-regular} has been referenced but does not e\n",
            "xist, replaced by a fixed one\n\n",
            "pdfTeX warning (structure dest): name{missing-structure} has been referenced bu\n",
            "t does not exist\n\n",
            "pdfTeX warning (thread): destination name{missing-thread} has been referenced b\n",
            "ut does not exist, replaced by a fixed one\n\n",
        );
        assert_eq!(terminal_text(universe), expected);
        assert_eq!(log_text(universe), expected);
        assert!(!terminal_text(universe).contains("num7"));
        assert!(!terminal_text(universe).contains("num23"));
    });
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
    crate::test_harness::with_world_universe(World::memory_with_clock(clock), |universe| {
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
            universe,
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
            terminal_text(universe),
            format!("{ETEX26_BANNER} (preloaded format=etex-loaded)\nentering extended mode\n")
        );
        assert_eq!(
            log_text(universe),
            format!(
                "{ETEX26_BANNER} (preloaded format=etex-loaded 2026.7.9)  9 JUL 2026 13:36\n\
             entering extended mode\n**etex-loaded-state-reset.tex\n"
            )
        );
    });
}

#[test]
fn startup_selector_is_echoed_without_becoming_the_job_name() {
    crate::test_harness::with_universe(|universe| {
        let mut job = JobFraming::default();
        let mut capabilities = CommandHostCapabilities::default();

        begin_job_with_terminal_banner(
            &mut job,
            universe,
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
        assert!(log_text(universe).contains("**&trip inputs/trip.tex\n"));
        assert_eq!(capabilities.job_name(), "trip");
    });
}

#[test]
fn loaded_tex82_banner_is_selected_by_runtime_profile_without_etex_or_pdftex_text() {
    crate::test_harness::with_universe(|universe| {
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
            universe,
            &mut capabilities,
            false,
            Some(&format),
            JobEngineFraming {
                binary: EngineBinaryIdentity::Tex82,
                extended_mode: false,
            },
            "trip.tex",
        );

        let terminal = terminal_text(universe);
        assert_eq!(
            terminal,
            format!("{TEX82_BANNER} (preloaded format=umber-tex82-oracle)\n")
        );
        assert!(!terminal.contains("pdfTeX"));
        assert!(!terminal.contains("e-TeX"));
        let log = log_text(universe);
        assert!(log.starts_with(TEX82_BANNER));
        assert!(log.contains("(preloaded format=trip 2026.7.9)"));
    });
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
