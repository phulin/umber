//! Direct tests for tex.web §54's selector, §73's `print_err`, and §82's
//! `error`.

use super::{ErrorChannel, Printer, Selector};
use crate::universe::{InteractionMode, Universe};
use crate::world::PrintSink;

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
        terminal_text(&universe)
            .contains("! Illegal magnification has been changed to 1000 (0)."),
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
fn error_stop_mode_returns_when_the_terminal_supplies_no_more_lines() {
    let mut universe = Universe::new();
    universe.print_err("Something anomalous").error();
    assert!(terminal_text(&universe).contains("? "));
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
