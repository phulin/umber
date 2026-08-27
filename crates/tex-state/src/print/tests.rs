//! Direct semantic tests for tex.web's printer and error channel.

use super::{
    ErrorChannel, ErrorContextLevel, ErrorContextWidths, ErrorHistory, ErrorOutcome,
    ErrorRecoveryRequest, JumpOut, Printer, Selector, render_error_context,
};
use crate::env::AssignmentScope;
use crate::env::banks::IntParam;
use crate::interner::{ControlSequenceKind, InternerBudget};
use crate::universe::{InteractionMode, Universe};
use crate::world::PrintSink;

fn with_test_universe<R>(
    use_universe: impl for<'id> FnOnce(&mut Universe<crate::GenerationBrand<'id>>) -> R,
) -> R {
    let budget = InternerBudget::new(32, 32, 1024).expect("budget");
    crate::with_universe(budget, use_universe).expect("fresh universe")
}

fn sink_text<G>(universe: &Universe<G>, wanted: PrintSink) -> String {
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

fn terminal_text<G>(universe: &Universe<G>) -> String {
    sink_text(universe, PrintSink::Terminal)
}

fn set_int<G>(universe: &mut Universe<G>, parameter: IntParam, value: i32) {
    universe
        .assign_int_param(parameter, value, AssignmentScope::Global)
        .expect("assign integer parameter");
}

#[test]
fn error_context_widths_enforce_tex82_bounds() {
    assert_eq!(
        ErrorContextWidths::new(64, 32),
        Some(ErrorContextWidths {
            error_line: 64,
            half_error_line: 32,
            max_print_line: 79,
        })
    );
    assert_eq!(ErrorContextWidths::new(64, 29), None);
    assert_eq!(ErrorContextWidths::new(46, 32), None);
    assert_eq!(ErrorContextWidths::new(64, usize::MAX), None);
}

#[test]
fn process_print_line_limit_wraps_terminal_and_log_independently() {
    with_test_universe(|universe| {
        universe.set_error_context_widths(
            ErrorContextWidths::new(64, 32)
                .and_then(|widths| widths.with_max_print_line(72))
                .expect("valid TRIP print widths"),
        );

        Printer::new(universe, Selector::TermOnly).print(&"t".repeat(70));
        Printer::new(universe, Selector::LogOnly).print(&"l".repeat(69));
        Printer::new(universe, Selector::TermAndLog).print("xyz");

        assert_eq!(
            sink_text(universe, PrintSink::Terminal),
            format!("{}xy\nz", "t".repeat(70))
        );
        assert_eq!(
            sink_text(universe, PrintSink::Log),
            format!("{}xyz\n", "l".repeat(69))
        );
    });
}

#[test]
fn error_context_projection_is_bounded_without_mutating_inputs() {
    let widths = ErrorContextWidths::new(64, 32).expect("valid widths");
    let levels = vec![
        ErrorContextLevel::new(
            "<current> ",
            "before-current-abcdefghijklmnopqrstuvwxyz",
            "after-current-ABCDEFGHIJKLMNOPQRSTUVWXYZ-0123456789",
        ),
        ErrorContextLevel::new(
            "l.99 ",
            "before-bottom-abcdefghijklmnopqrstuvwxyz",
            "after-bottom-ABCDEFGHIJKLMNOPQRSTUVWXYZ-0123456789",
        ),
    ];
    let unchanged = levels.clone();
    let rendered = render_error_context(&levels, widths, -1);

    for line in rendered.lines().filter(|line| !line.is_empty()) {
        assert!(line.chars().count() <= widths.error_line(), "{line:?}");
    }
    assert!(rendered.contains("<current>"));
    assert!(rendered.contains("l.99"));
    assert_eq!(levels, unchanged);
}

#[test]
fn selector_follows_texweb_ordinal_arithmetic_and_interaction() {
    assert_eq!(Selector::TermAndLog.decr(), Selector::LogOnly);
    assert_eq!(Selector::LogOnly.decr(), Selector::TermOnly);
    assert_eq!(Selector::TermOnly.decr(), Selector::NoPrint);
    assert_eq!(Selector::NoPrint.incr(), Selector::TermOnly);
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
fn integer_two_digit_and_hex_printers_have_exact_wire_text() {
    with_test_universe(|universe| {
        let mut printer = Printer::new(universe, Selector::TermAndLog);
        for value in [i64::from(i32::MIN), -1, 0, i64::MAX] {
            printer.print_int(value).print_char('|');
        }
        for value in [0, 7, 42, 99] {
            printer.print_two(value).print_char('|');
        }
        for value in [0, 15, 255] {
            printer.print_hex(value).print_char('|');
        }

        assert_eq!(
            terminal_text(universe),
            "-2147483648|-1|0|9223372036854775807|00|07|42|99|'0|'F|'FF|"
        );
    });
}

#[test]
fn print_err_formats_message_help_and_history() {
    with_test_universe(|universe| {
        universe.set_interaction_mode(InteractionMode::Nonstop);
        let mut report = universe.print_err("Arithmetic overflow");
        report.help(&["first help line", "second help line"]);
        assert_eq!(report.error(), ErrorOutcome::Continue);

        let terminal = terminal_text(universe);
        let log = sink_text(universe, PrintSink::Log);
        assert!(terminal.contains("! Arithmetic overflow."), "{terminal}");
        assert!(log.contains("first help line"), "{log}");
        assert!(log.contains("second help line"), "{log}");
        assert_eq!(
            universe.world().error_channel().history(),
            ErrorHistory::ErrorMessageIssued
        );
    });
}

#[test]
fn error_stop_answers_resume_or_quit_with_typed_outcomes() {
    with_test_universe(|universe| {
        universe
            .world_mut()
            .push_memory_terminal_line("12")
            .expect("memory terminal line");
        assert_eq!(
            universe.print_err("Something anomalous").error(),
            ErrorOutcome::Recovery(ErrorRecoveryRequest::Delete(12))
        );
    });

    with_test_universe(|universe| {
        universe
            .world_mut()
            .push_memory_terminal_line("I\\count0=17")
            .expect("memory terminal line");
        assert_eq!(
            universe.print_err("Something anomalous").error(),
            ErrorOutcome::Recovery(ErrorRecoveryRequest::Insert("\\count0=17".to_owned()))
        );
    });

    with_test_universe(|universe| {
        universe
            .world_mut()
            .push_memory_terminal_line("s")
            .expect("memory terminal line");
        assert_eq!(
            universe.print_err("Something anomalous").error(),
            ErrorOutcome::Continue
        );
        assert_eq!(universe.interaction_mode(), InteractionMode::Scroll);
    });

    with_test_universe(|universe| {
        universe
            .world_mut()
            .push_memory_terminal_line("x")
            .expect("memory terminal line");
        assert_eq!(
            universe.print_err("Something anomalous").error(),
            ErrorOutcome::JumpOut(JumpOut::Quit)
        );
    });
}

#[test]
fn exhausted_error_stop_terminal_reaches_emergency_stop() {
    with_test_universe(|universe| {
        assert_eq!(
            universe.print_err("Something anomalous").error(),
            ErrorOutcome::JumpOut(JumpOut::EmergencyStop {
                help: "End of file on the terminal!",
            })
        );
        assert_eq!(universe.interaction_mode(), InteractionMode::Scroll);
        assert_eq!(
            universe.world().error_channel().history(),
            ErrorHistory::FatalErrorStop
        );
    });
}

#[test]
fn print_esc_and_sprint_cs_use_semantic_control_sequence_forms() {
    with_test_universe(|universe| {
        set_int(universe, IntParam::ESCAPE_CHAR, '\\' as i32);
        set_int(universe, IntParam::NEWLINE_CHAR, -1);
        Printer::new(universe, Selector::TermAndLog)
            .print_esc("relax")
            .print_char('|')
            .sprint_cs(ControlSequenceKind::Named, "foo")
            .print_char('|')
            .sprint_cs(ControlSequenceKind::ActiveCharacter, "~")
            .print_char('|')
            .sprint_cs(ControlSequenceKind::Null, "");
        assert_eq!(
            terminal_text(universe),
            "\\relax|\\foo|~|\\csname\\endcsname"
        );
    });

    with_test_universe(|universe| {
        set_int(universe, IntParam::ESCAPE_CHAR, -1);
        Printer::new(universe, Selector::TermAndLog).print_esc("relax");
        assert_eq!(terminal_text(universe), "relax");
    });
}

#[test]
fn content_and_character_printing_keep_their_distinct_encoding_contracts() {
    with_test_universe(|universe| {
        set_int(universe, IntParam::NEWLINE_CHAR, 127);
        Printer::new(universe, Selector::TermAndLog).print("A\u{7f}\n\0\u{80}\u{ff}é");
        assert_eq!(terminal_text(universe), "A\n^^J^^@^^80^^ff^^e9");
        assert_eq!(universe.world().effect_records().len(), 1);
    });

    with_test_universe(|universe| {
        set_int(universe, IntParam::NEWLINE_CHAR, 127);
        Printer::new(universe, Selector::TermAndLog)
            .print_char('\0')
            .print_char('\u{7f}')
            .print_char('\u{80}');
        assert_eq!(terminal_text(universe).as_bytes(), [0, b'\n', 0xc2, 0x80]);
    });
}

#[test]
fn error_channel_counts_scrolled_errors_and_records_long_help() {
    let mut channel = ErrorChannel::default();
    channel.record_scrolled_error();
    channel.record_scrolled_error();
    assert_eq!(channel.error_count(), 2);
    channel.clear_error_count();
    assert_eq!(channel.error_count(), 0);
    assert!(!channel.take_long_help_seen(true));
    assert!(channel.take_long_help_seen(false));
}

#[test]
fn hundredth_scrolled_error_reaches_tex82_fatal_limit() {
    with_test_universe(|universe| {
        universe.set_interaction_mode(InteractionMode::Nonstop);
        for index in 1..=100 {
            assert_eq!(
                universe.print_err("Repeated error").error(),
                if index == 100 {
                    ErrorOutcome::JumpOut(JumpOut::TooManyErrors)
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
    });
}
