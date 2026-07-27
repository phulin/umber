use crate::Universe;
use crate::env::banks::IntParam;
use crate::scaled::Scaled;
use crate::world::{EffectRecord, PrintSink};

/// Concatenates the routed text a diagnostic emitted, per sink.
fn routed(universe: &Universe) -> Vec<(PrintSink, String)> {
    let mut routed: Vec<(PrintSink, String)> = Vec::new();
    for record in universe.world().effect_records() {
        if let EffectRecord::StreamWrite { sink, text } = record {
            match routed.last_mut() {
                Some((last, buffer)) if *last == *sink => buffer.push_str(text),
                _ => routed.push((*sink, text.clone())),
            }
        }
    }
    routed
}

#[test]
fn tracing_online_selects_the_transcript_or_both_channels() {
    for (tracing_online, expected) in [
        (0, PrintSink::Log),
        (-1, PrintSink::Log),
        (1, PrintSink::TerminalAndLog),
    ] {
        let mut universe = Universe::new();
        universe.set_int_param(IntParam::TRACING_ONLINE, tracing_online);
        let mut diagnostic = universe.begin_diagnostic();
        diagnostic.print("trace");
        diagnostic.end(false);
        assert_eq!(
            routed(&universe),
            vec![(expected, "trace\n".to_owned())],
            "\\tracingonline={tracing_online}"
        );
    }
}

#[test]
fn end_diagnostic_closes_the_line_and_optionally_adds_a_blank_one() {
    for (blank_line, expected) in [(false, "trace\n"), (true, "trace\n\n")] {
        let mut universe = Universe::new();
        let mut diagnostic = universe.begin_diagnostic();
        diagnostic.print("trace");
        diagnostic.end(blank_line);
        assert_eq!(
            routed(&universe),
            vec![(PrintSink::Log, expected.to_owned())]
        );
    }
}

#[test]
fn end_diagnostic_on_an_empty_line_emits_nothing_further() {
    let mut universe = Universe::new();
    let mut diagnostic = universe.begin_diagnostic();
    diagnostic.print("trace\n");
    diagnostic.end(false);
    assert_eq!(
        routed(&universe),
        vec![(PrintSink::Log, "trace\n".to_owned())]
    );
}

#[test]
fn print_nl_breaks_only_a_line_the_routed_sink_has_already_started() {
    let mut universe = Universe::new();
    universe.world_mut().write_text(PrintSink::Terminal, "term");
    let mut diagnostic = universe.begin_diagnostic();
    // Routed to the transcript alone, so a partial terminal line is tex.web
    // §62's `odd(selector)` case and must not force a break.
    diagnostic.print_nl("first");
    diagnostic.print_nl("second");
    diagnostic.end(false);
    assert_eq!(
        routed(&universe),
        vec![
            (PrintSink::Terminal, "term".to_owned()),
            (PrintSink::Log, "first\nsecond\n".to_owned()),
        ]
    );
}

#[test]
fn print_nl_breaks_a_terminal_line_when_the_terminal_is_routed() {
    let mut universe = Universe::new();
    universe.set_int_param(IntParam::TRACING_ONLINE, 1);
    universe.world_mut().write_text(PrintSink::Terminal, "term");
    let mut diagnostic = universe.begin_diagnostic();
    diagnostic.print_nl("first");
    diagnostic.end(false);
    assert_eq!(
        routed(&universe),
        vec![
            (PrintSink::Terminal, "term".to_owned()),
            (PrintSink::TerminalAndLog, "\nfirst\n".to_owned()),
        ]
    );
}

#[test]
fn scalar_printing_matches_tex_webs_print_int_and_print_scaled() {
    let mut universe = Universe::new();
    let mut diagnostic = universe.begin_diagnostic();
    diagnostic.print_int(-4168);
    diagnostic.print_char('/');
    diagnostic.print_scaled(Scaled::from_raw(19_516_436));
    diagnostic.print_char('/');
    diagnostic.print_scaled(Scaled::from_raw(-163_840));
    diagnostic.end(false);
    assert_eq!(
        routed(&universe),
        vec![(PrintSink::Log, "-4168/297.79718/-2.5\n".to_owned())]
    );
}
