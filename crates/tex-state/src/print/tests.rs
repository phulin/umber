//! Direct tests for tex.web §54's selector, §73's `print_err`, and §82's
//! `error`.

use super::{
    ErrorChannel, ErrorContextLevel, ErrorContextWidths, ErrorHistory, ErrorOutcome, JumpOut,
    Printer, Selector, render_error_context,
};
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

#[test]
fn render_error_context_respects_width_and_context_line_limits() {
    let widths = ErrorContextWidths::new(64, 32).expect("valid TeX82 context widths");
    let levels = vec![
        ErrorContextLevel::new(
            "<current> ",
            "before-current-abcdefghijklmnopqrstuvwxyz",
            "after-current-ABCDEFGHIJKLMNOPQRSTUVWXYZ-0123456789",
        ),
        ErrorContextLevel::new(
            "<first> ",
            "before-first-abcdefghijklmnopqrstuvwxyz",
            "after-first-ABCDEFGHIJKLMNOPQRSTUVWXYZ-0123456789",
        ),
        ErrorContextLevel::new(
            "<second> ",
            "before-second-abcdefghijklmnopqrstuvwxyz",
            "after-second-ABCDEFGHIJKLMNOPQRSTUVWXYZ-0123456789",
        ),
        ErrorContextLevel::new(
            "l.99 ",
            "before-bottom-abcdefghijklmnopqrstuvwxyz",
            "after-bottom-ABCDEFGHIJKLMNOPQRSTUVWXYZ-0123456789",
        ),
    ];
    let unchanged = levels.clone();

    let negative = render_error_context(&levels, widths, -1);
    let zero = render_error_context(&levels, widths, 0);
    let one = render_error_context(&levels, widths, 1);

    assert_eq!(
        negative,
        concat!(
            "\n<current> ...hijklmnopqrstuvwxyz",
            "\n                                after-current-ABCDEFGHIJKLMNO...",
            "\nl.99 ...cdefghijklmnopqrstuvwxyz",
            "\n                                after-bottom-ABCDEFGHIJKLMNOP...",
        )
    );
    assert_eq!(
        zero,
        concat!(
            "\n<current> ...hijklmnopqrstuvwxyz",
            "\n                                after-current-ABCDEFGHIJKLMNO...",
            "\n...",
            "\nl.99 ...cdefghijklmnopqrstuvwxyz",
            "\n                                after-bottom-ABCDEFGHIJKLMNOP...",
        )
    );
    assert_eq!(
        one,
        concat!(
            "\n<current> ...hijklmnopqrstuvwxyz",
            "\n                                after-current-ABCDEFGHIJKLMNO...",
            "\n<first> ...fghijklmnopqrstuvwxyz",
            "\n                                after-first-ABCDEFGHIJKLMNOPQ...",
            "\n...",
            "\nl.99 ...cdefghijklmnopqrstuvwxyz",
            "\n                                after-bottom-ABCDEFGHIJKLMNOP...",
        )
    );

    for output in [&negative, &zero, &one] {
        let mut lines = output.lines();
        assert_eq!(lines.next(), Some(""));
        for line in lines.filter(|line| *line != "...") {
            assert!(line.chars().count() <= widths.error_line(), "{line:?}");
        }
    }
    assert_eq!(
        levels, unchanged,
        "rendering must not mutate source context"
    );
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
fn fatal_before_transcript_open_uses_prelog_selector_and_restores_normal_routing() {
    for (mode, prelog_terminal) in [
        (InteractionMode::Batch, false),
        (InteractionMode::Nonstop, true),
        (InteractionMode::Scroll, true),
        (InteractionMode::ErrorStop, true),
    ] {
        let mut universe = Universe::new();
        universe.set_interaction_mode(mode);
        let mut report = universe.print_err_with_transcript_state(
            "TeX capacity exceeded, sorry [input stack size=5000]",
            false,
        );
        report.help(&["If you really absolutely need more capacity,"]);
        report.succumb();

        let terminal = sink_text(&universe, PrintSink::Terminal);
        let log = sink_text(&universe, PrintSink::Log);
        assert_eq!(
            terminal.contains("! TeX capacity exceeded, sorry [input stack size=5000]."),
            prelog_terminal,
            "pre-transcript terminal routing for {mode:?}: {terminal:?}"
        );
        assert!(
            log.is_empty(),
            "a pre-transcript fatal must not create log output for {mode:?}: {log:?}"
        );

        // The pre-log selector belongs only to the report. Once the host has
        // opened the transcript, ordinary printing derives the normal
        // selector (ErrorStop has canonically become Scroll in `succumb`).
        universe.printer().print("after-open");
        let terminal = sink_text(&universe, PrintSink::Terminal);
        let log = sink_text(&universe, PrintSink::Log);
        assert_eq!(
            terminal.ends_with("after-open"),
            mode != InteractionMode::Batch
        );
        assert!(log.ends_with("after-open"));
    }
}

#[test]
fn pdftex_integer_printers_cover_widened_and_negative_boundaries() {
    let mut universe = Universe::new();
    let before_interaction = universe.interaction_mode();
    let before_history = universe.world().error_channel().history();
    let before_error_count = universe.world().error_channel().error_count();

    {
        let mut printer = Printer::new(&mut universe, Selector::TermAndLog);
        for value in [i64::from(i32::MIN), -1, 0, 1, i64::from(i32::MAX), i64::MAX] {
            printer.print_int(value).print_char('|');
        }
        for value in [-1, -9, -10, i32::MIN] {
            printer.print_two(value).print_char('|');
        }
    }

    assert_eq!(
        sink_text(&universe, PrintSink::Terminal),
        "-2147483648|-1|0|1|2147483647|9223372036854775807|01|09|10|48|"
    );
    assert_eq!(universe.interaction_mode(), before_interaction);
    assert_eq!(universe.world().error_channel().history(), before_history);
    assert_eq!(
        universe.world().error_channel().error_count(),
        before_error_count
    );
}

#[test]
fn print_two_and_print_hex_have_exact_wire_text() {
    let mut universe = Universe::new();
    let before_interaction = universe.interaction_mode();
    let before_history = universe.world().error_channel().history();
    let before_error_count = universe.world().error_channel().error_count();

    {
        let mut printer = Printer::new(&mut universe, Selector::TermAndLog);
        assert_eq!(printer.selector(), Selector::TermAndLog);
        for value in [0, 7, 42, 99] {
            printer.print_two(value).print_char('|');
        }
        for value in [0, 15, 255] {
            printer.print_hex(value).print_char('|');
        }
        assert_eq!(printer.selector(), Selector::TermAndLog);
    }

    let expected = "00|07|42|99|'0|'F|'FF|";
    assert_eq!(sink_text(&universe, PrintSink::Terminal), expected);
    assert_eq!(sink_text(&universe, PrintSink::Log), expected);
    assert_eq!(universe.interaction_mode(), before_interaction);
    assert_eq!(universe.world().error_channel().history(), before_history);
    assert_eq!(
        universe.world().error_channel().error_count(),
        before_error_count
    );
}

#[test]
fn pdftex_ignored_error_is_exactly_transcript_only_in_every_interaction_mode() {
    for interaction in [
        InteractionMode::Batch,
        InteractionMode::Nonstop,
        InteractionMode::Scroll,
        InteractionMode::ErrorStop,
    ] {
        let mut universe = Universe::new();
        universe.set_interaction_mode(interaction);
        let before_history = universe.world().error_channel().history();
        let before_error_count = universe.world().error_channel().error_count();

        universe.print_ignored_err("Infinite glue shrinkage found");

        assert_eq!(sink_text(&universe, PrintSink::Terminal), "");
        assert_eq!(
            sink_text(&universe, PrintSink::Log),
            "\nignored: Infinite glue shrinkage found"
        );
        assert_eq!(universe.interaction_mode(), interaction);
        assert_eq!(universe.world().error_channel().history(), before_history);
        assert_eq!(
            universe.world().error_channel().error_count(),
            before_error_count
        );
    }
}

#[test]
fn print_err_prefixes_the_message_and_error_terminates_it_with_a_period() {
    let mut universe = Universe::new();
    universe.set_interaction_mode(InteractionMode::Nonstop);
    let mut report = universe.print_err("Arithmetic overflow");
    report.help(&["first help line", "second help line"]);
    assert_eq!(report.error(), ErrorOutcome::Continue);

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
    assert_eq!(
        universe
            .print_err("Illegal magnification has been changed to 1000")
            .int_error(0),
        ErrorOutcome::Continue
    );

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
    assert_eq!(report.error(), ErrorOutcome::Continue);

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
    assert_eq!(
        universe.resume_error_report(deferred).error(),
        ErrorOutcome::Continue
    );

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
    assert_eq!(
        universe.print_err("Something anomalous").error(),
        ErrorOutcome::Continue
    );

    let output = terminal_text(&universe);
    assert!(output.contains("? "), "{output}");
    assert!(output.contains("OK, entering \\scrollmode..."), "{output}");
    assert_eq!(universe.interaction_mode(), InteractionMode::Scroll);
}

#[test]
fn error_help_preserves_reverse_order_and_menu_text() {
    const MENU: &str = "Type <return> to proceed, S to scroll future error messages,\n\
R to run without stopping, Q to run quietly,\n\
I to insert something,\n\
1 or ... or 9 to ignore the next 1 to 9 tokens of input,\n\
H for help, X to quit.";
    assert_eq!(
        MENU.lines().next().expect("menu has a first line").len(),
        60
    );

    let mut universe = Universe::new();
    for answer in ["h", "z", "h", ""] {
        universe
            .world_mut()
            .push_memory_terminal_line(answer)
            .expect("memory terminal accepts an ErrorStop answer");
    }
    let mut report = universe.print_err("Controlled error");
    report.help(&["first", "second", "third"]);
    assert_eq!(report.error(), ErrorOutcome::Continue);

    assert_eq!(
        sink_text(&universe, PrintSink::Terminal),
        concat!(
            "! Controlled error.\n? first\nsecond\nthird\n\n? ",
            "Type <return> to proceed, S to scroll future error messages,\n",
            "R to run without stopping, Q to run quietly,\n",
            "I to insert something,\n",
            "1 or ... or 9 to ignore the next 1 to 9 tokens of input,\n",
            "H for help, X to quit.\n? ",
            "Sorry, I already gave what help I could...\n",
            "Maybe you should try asking a human?\n",
            "An error might have occurred before I noticed any problems.\n",
            "``If all else fails, read the instructions.''\n\n? "
        )
    );
    let transcript = sink_text(&universe, PrintSink::Log);
    assert_eq!(transcript.matches("first\nsecond\nthird\n").count(), 1);
    assert!(transcript.contains(MENU), "{transcript:?}");
    assert!(transcript.contains("? h\nfirst"), "{transcript:?}");
    assert!(transcript.contains("? z\nType <return>"), "{transcript:?}");
    assert!(
        transcript.contains("? h\nSorry, I already gave"),
        "{transcript:?}"
    );
    assert!(transcript.ends_with("? \n"), "{transcript:?}");
}

/// tex.web §84's `X`: `interaction:=scroll_mode; jump_out`. Unlike §93's
/// `succumb` it prints nothing and leaves `history` where it was -- it is a
/// requested exit, not a diagnosis -- but it ends the job all the same, which
/// Umber could not express before `umber2-er8c`.
#[test]
fn error_stop_mode_x_quits_without_printing_or_raising_history() {
    let mut universe = Universe::new();
    universe
        .world_mut()
        .push_memory_terminal_line("x")
        .expect("memory terminal accepts a line");
    let outcome = universe.print_err("Something anomalous").error();

    assert_eq!(outcome, ErrorOutcome::JumpOut(JumpOut::Quit));
    assert_eq!(universe.interaction_mode(), InteractionMode::Scroll);
    assert_eq!(
        universe.world().error_channel().history(),
        ErrorHistory::ErrorMessageIssued
    );
    let output = terminal_text(&universe);
    assert!(!output.contains("Emergency stop"), "{output}");
    assert!(!output.contains("OK, entering"), "{output}");
}

/// tex.web §82 enters §83's dialog on `interaction=error_stop_mode` alone.
/// §71's `term_input` answers an exhausted terminal with
/// `fatal_error("End of file on the terminal!")`, and §93's `succumb` drops
/// to scroll mode, reports through a nested `error`, and jumps out.
///
/// Umber used to guard the dialog on a terminal line being available, which
/// took the scrolled tail instead: it counted an error tex.web does not
/// count, printed help tex.web does not print, and let the job continue
/// (`umber2-er8c`).
#[test]
fn error_stop_mode_prompting_an_exhausted_terminal_reaches_texweb_93() {
    let mut universe = Universe::new();
    let outcome = universe.print_err("Something anomalous").error();

    assert_eq!(
        outcome,
        ErrorOutcome::JumpOut(JumpOut::EmergencyStop {
            help: "End of file on the terminal!"
        })
    );
    let output = terminal_text(&universe);
    assert!(output.contains("! Something anomalous."), "{output}");
    assert!(output.contains("? "), "{output}");
    assert!(output.contains("! Emergency stop."), "{output}");
    // §93's `if interaction=error_stop_mode then interaction:=scroll_mode`,
    // which is what keeps the nested `error` from prompting again.
    assert_eq!(universe.interaction_mode(), InteractionMode::Scroll);
    assert_eq!(
        universe.world().error_channel().history(),
        ErrorHistory::FatalErrorStop
    );
    // §90 keeps §93's one help line off the terminal in every non-batch
    // mode. `terminal_text` above folds the log in, so this reads the
    // terminal sink itself.
    let terminal = sink_text(&universe, PrintSink::Terminal);
    assert!(
        !terminal.contains("End of file on the terminal!"),
        "{terminal:?}"
    );
    let log = sink_text(&universe, PrintSink::Log);
    assert!(log.contains("End of file on the terminal!"), "{log:?}");
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
fn print_esc_renders_live_eight_bit_escapechar_as_tex_character_string() {
    use crate::env::banks::IntParam;

    for (escape, expected) in [
        (0, "^^@global"),
        (31, "^^_global"),
        (32, " global"),
        (94, "^global"),
        (126, "~global"),
        (127, "^^?global"),
        (128, "^^80global"),
        (255, "^^ffglobal"),
        (-1, "global"),
        (256, "global"),
    ] {
        let mut universe = Universe::new();
        universe.set_int_param(IntParam::ESCAPE_CHAR, escape);
        universe.set_int_param(IntParam::NEWLINE_CHAR, -1);
        Printer::new(&mut universe, Selector::TermAndLog).print_esc("global");
        assert_eq!(terminal_text(&universe), expected, "escapechar={escape}");
    }
}

#[test]
fn character_string_honors_newline_before_caret_rendering_on_active_selectors() {
    use crate::env::banks::IntParam;

    for selector in [Selector::TermOnly, Selector::LogOnly, Selector::TermAndLog] {
        let mut universe = Universe::new();
        universe.set_int_param(IntParam::NEWLINE_CHAR, 127);
        Printer::new(&mut universe, selector)
            .print("prefix")
            .print_character_string('\u{7f}')
            .print_character_string('\0');

        let wanted = match selector {
            Selector::TermOnly => PrintSink::Terminal,
            Selector::LogOnly => PrintSink::Log,
            Selector::TermAndLog => PrintSink::TerminalAndLog,
            Selector::NoPrint => unreachable!(),
        };
        assert_eq!(sink_text(&universe, wanted), "prefix\n^^@");
    }

    let mut universe = Universe::new();
    universe.set_int_param(IntParam::NEWLINE_CHAR, 127);
    Printer::new(&mut universe, Selector::NoPrint).print_character_string('\u{7f}');
    assert!(universe.world().effect_records().is_empty());
}

#[test]
fn content_print_renders_every_character_in_one_atomic_effect() {
    use crate::env::banks::IntParam;

    let mut universe = Universe::new();
    universe.set_int_param(IntParam::NEWLINE_CHAR, 127);
    Printer::new(&mut universe, Selector::TermAndLog).print("A\u{7f}\n\0\u{80}\u{ff}é");

    assert_eq!(terminal_text(&universe), "A\n^^J^^@^^80^^ff^^e9");
    assert_eq!(universe.world().effect_records().len(), 1);
    assert_eq!(
        universe.world().stream_bufs().terminal_partial_line(),
        "^^J^^@^^80^^ff^^e9"
    );
}

#[test]
fn raw_print_char_only_applies_newlinechar_and_never_caret_renders() {
    use crate::env::banks::IntParam;

    let mut universe = Universe::new();
    universe.set_int_param(IntParam::NEWLINE_CHAR, 127);
    Printer::new(&mut universe, Selector::TermAndLog)
        .print_char('\0')
        .print_char('\u{7f}')
        .print_char('\u{80}');

    assert_eq!(terminal_text(&universe).as_bytes(), [0, b'\n', 0xc2, 0x80]);
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
        .sprint_cs(ControlSequenceKind::Null, "");
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
    assert!(
        sink_text(&universe, PrintSink::Terminal)
            .contains("(That makes 100 errors; please try again.)")
    );
}
