//! Direct tests for tex.web §54's selector, §73's `print_err`, and §82's
//! `error`.

use super::{ErrorChannel, ErrorContextWidths, ErrorHistory, ErrorOutcome, Printer, Selector};
use crate::universe::{InteractionMode, Universe};
use crate::world::PrintSink;

#[test]
fn error_context_widths_enforce_tex82_section_3_bounds() {
    assert_eq!(
        ErrorContextWidths::new(64, 32),
        Some(ErrorContextWidths {
            error_line: 64,
            half_error_line: 32,
        })
    );
    assert_eq!(ErrorContextWidths::new(64, 29), None);
    assert_eq!(ErrorContextWidths::new(46, 32), None);
    assert_eq!(ErrorContextWidths::new(64, usize::MAX), None);
}

fn terminal_text(universe: &Universe) -> String {
    universe
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            crate::EffectRecord::StreamWrite {
                sink: PrintSink::Terminal | PrintSink::TerminalAndLog | PrintSink::Log,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn sink_text(universe: &Universe, wanted: PrintSink) -> String {
    universe
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            crate::EffectRecord::StreamWrite { sink, text }
                if *sink == wanted || *sink == PrintSink::TerminalAndLog =>
            {
                Some(text.as_str())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn selector_follows_texweb_54_ordinal_arithmetic() {
    assert_eq!(Selector::TermAndLog.decr(), Selector::LogOnly);
    assert_eq!(Selector::LogOnly.decr(), Selector::TermOnly);
    assert_eq!(Selector::TermOnly.decr(), Selector::NoPrint);
    assert_eq!(Selector::NoPrint.decr(), Selector::NoPrint);
    assert_eq!(Selector::NoPrint.incr(), Selector::TermOnly);
    assert_eq!(Selector::LogOnly.incr(), Selector::TermAndLog);
    assert!(Selector::TermOnly.writes_terminal() && !Selector::TermOnly.writes_log());
    assert!(Selector::LogOnly.writes_log() && !Selector::LogOnly.writes_terminal());
    assert_eq!(Selector::NoPrint.sink(), None);
}

#[test]
fn batch_mode_selects_the_transcript_and_every_other_mode_selects_both() {
    assert_eq!(
        Selector::for_interaction(InteractionMode::Batch),
        Selector::LogOnly
    );
    for mode in [
        InteractionMode::Nonstop,
        InteractionMode::Scroll,
        InteractionMode::ErrorStop,
    ] {
        assert_eq!(Selector::for_interaction(mode), Selector::TermAndLog);
    }
}

#[test]
fn print_err_prefixes_the_message_and_error_terminates_it_with_a_period() {
    let mut universe = Universe::new();
    universe.set_interaction_mode(InteractionMode::Nonstop);
    let mut report = universe.print_err("Arithmetic overflow");
    report.help(&["first help line", "second help line"]);
    report.error();

    let output = terminal_text(&universe);
    assert!(output.contains("! Arithmetic overflow."), "{output}");
    // §90 shows help on the transcript, one line per `print_nl`.
    assert!(output.contains("first help line"), "{output}");
    assert!(output.contains("second help line"), "{output}");
}

#[test]
fn int_error_appends_texweb_91_parenthesized_value() {
    let mut universe = Universe::new();
    universe.set_interaction_mode(InteractionMode::Nonstop);
    universe
        .print_err("Illegal magnification has been changed to 1000")
        .int_error(0);

    assert!(
        terminal_text(&universe).contains("! Illegal magnification has been changed to 1000 (0)."),
        "{}",
        terminal_text(&universe)
    );
}

#[test]
fn err_help_replaces_the_builtin_help_lines() {
    let mut universe = Universe::new();
    universe.set_interaction_mode(InteractionMode::Nonstop);
    let mut report = universe.print_err("");
    report.print("bad");
    report.help(&["builtin help"]);
    report.use_err_help("user help".into());
    report.error();

    let output = terminal_text(&universe);
    assert!(output.contains("! bad."), "{output}");
    assert!(output.contains("user help"), "{output}");
    assert!(!output.contains("builtin help"), "{output}");
}

#[test]
fn deferred_error_report_preserves_message_selector_and_help() {
    let mut universe = Universe::new();
    universe.set_interaction_mode(InteractionMode::Nonstop);
    let deferred = {
        let mut report = universe.print_err("Missing { inserted");
        report.help(&["brace help"]);
        report.defer()
    };
    universe.printer().print("intervening recovery observation");
    universe.resume_error_report(deferred).error();

    let output = terminal_text(&universe);
    assert!(
        output.contains("! Missing { insertedintervening recovery observation."),
        "{output}"
    );
    assert!(output.contains("brace help"), "{output}");
}

#[test]
fn error_stop_mode_prompts_and_honors_the_scroll_answer() {
    let mut universe = Universe::new();
    universe
        .world_mut()
        .push_memory_terminal_line("s")
        .expect("memory terminal accepts a line");
    universe.print_err("Something anomalous").error();

    let output = terminal_text(&universe);
    assert!(output.contains("? "), "{output}");
    assert!(output.contains("OK, entering \\scrollmode..."), "{output}");
    assert_eq!(universe.interaction_mode(), InteractionMode::Scroll);
}

#[test]
fn error_stop_mode_skips_the_dialog_when_the_terminal_cannot_answer() {
    // tex.web §71 would `fatal_error` here; Umber cannot end the job, so it
    // asks nothing rather than printing an unanswerable prompt.
    let mut universe = Universe::new();
    universe.print_err("Something anomalous").error();
    let output = terminal_text(&universe);
    assert!(output.contains("! Something anomalous."), "{output}");
    assert!(!output.contains("? "), "{output}");
    assert_eq!(universe.interaction_mode(), InteractionMode::ErrorStop);
}

#[test]
fn print_esc_uses_escapechar_and_omits_it_when_out_of_range() {
    let mut universe = Universe::new();
    Printer::new(&mut universe, Selector::TermAndLog).print_esc("relax");
    assert!(terminal_text(&universe).contains("\\relax"));

    let mut universe = Universe::new();
    universe.set_int_param(crate::env::banks::IntParam::ESCAPE_CHAR, -1);
    Printer::new(&mut universe, Selector::TermAndLog).print_esc("relax");
    assert_eq!(terminal_text(&universe), "relax");
}

#[test]
fn sprint_cs_distinguishes_named_active_and_null_control_sequences() {
    use crate::interner::ControlSequenceKind;

    let mut universe = Universe::new();
    let mut printer = Printer::new(&mut universe, Selector::TermAndLog);
    printer
        .sprint_cs(ControlSequenceKind::Named, "foo")
        .print_char('|')
        .sprint_cs(ControlSequenceKind::ActiveCharacter, "~")
        .print_char('|')
        .sprint_cs(ControlSequenceKind::Named, "");
    assert_eq!(terminal_text(&universe), "\\foo|~|\\csname\\endcsname");
}

#[test]
fn error_channel_counts_scrolled_errors_and_records_the_long_help() {
    let mut channel = ErrorChannel::default();
    assert_eq!(channel.error_count(), 0);
    channel.record_scrolled_error();
    channel.record_scrolled_error();
    assert_eq!(channel.error_count(), 2);
    channel.clear_error_count();
    assert_eq!(channel.error_count(), 0);
    assert!(!channel.take_long_help_seen(true));
    assert!(channel.take_long_help_seen(false));
}

#[test]
fn contextual_int_error_obeys_the_four_interaction_modes() {
    const CONTEXT: &str = "\n<to be read again> \n                   =";
    const HELP: [&str; 2] = [
        "A character number must be between 0 and 255.",
        "I changed this one to zero.",
    ];

    for mode in [
        InteractionMode::Batch,
        InteractionMode::Nonstop,
        InteractionMode::Scroll,
    ] {
        let mut universe = Universe::new();
        universe.set_interaction_mode(mode);
        let mut report = universe.print_err("Bad character code");
        report.help(&HELP).context(CONTEXT.into());
        assert_eq!(report.int_error(256), ErrorOutcome::Continue);

        let terminal = sink_text(&universe, PrintSink::Terminal);
        let log = sink_text(&universe, PrintSink::Log);
        assert_eq!(
            terminal.is_empty(),
            mode == InteractionMode::Batch,
            "{mode:?}: {terminal:?}"
        );
        assert!(
            log.contains("! Bad character code (256).")
                && log.contains(CONTEXT)
                && log.contains(HELP[0])
                && log.contains(HELP[1]),
            "{mode:?}: {log:?}"
        );
        if mode != InteractionMode::Batch {
            assert!(
                terminal.contains("! Bad character code (256).")
                    && terminal.contains(CONTEXT)
                    && !terminal.contains(HELP[0])
                    && !terminal.contains(HELP[1]),
                "{mode:?}: {terminal:?}"
            );
        }
        assert_eq!(universe.world().error_channel().error_count(), 1);
        assert_eq!(
            universe.world().error_channel().history(),
            ErrorHistory::ErrorMessageIssued
        );
    }

    let mut universe = Universe::new();
    universe
        .world_mut()
        .push_memory_terminal_line("h")
        .expect("memory terminal accepts help request");
    universe
        .world_mut()
        .push_memory_terminal_line("")
        .expect("memory terminal accepts return");
    let mut report = universe.print_err("Bad character code");
    report.help(&HELP).context(CONTEXT.into());
    assert_eq!(report.int_error(256), ErrorOutcome::Continue);
    let terminal = sink_text(&universe, PrintSink::Terminal);
    assert!(
        terminal.contains(CONTEXT)
            && terminal.contains("? ")
            && terminal.contains(HELP[0])
            && terminal.contains(HELP[1]),
        "{terminal:?}"
    );
    assert_eq!(universe.world().error_channel().error_count(), 0);
    assert_eq!(
        universe.world().error_channel().history(),
        ErrorHistory::ErrorMessageIssued
    );
}

#[test]
fn the_hundredth_scrolled_error_reaches_tex82_fatal_history_and_limit() {
    let mut universe = Universe::new();
    universe.set_interaction_mode(InteractionMode::Nonstop);
    for index in 1..=100 {
        let outcome = universe.print_err("Repeated error").error();
        assert_eq!(
            outcome,
            if index == 100 {
                ErrorOutcome::FatalErrorLimit
            } else {
                ErrorOutcome::Continue
            }
        );
    }
    assert_eq!(universe.world().error_channel().error_count(), 100);
    assert_eq!(
        universe.world().error_channel().history(),
        ErrorHistory::FatalErrorStop
    );
    assert!(
        sink_text(&universe, PrintSink::Terminal)
            .contains("(That makes 100 errors; please try again.)")
    );
}
