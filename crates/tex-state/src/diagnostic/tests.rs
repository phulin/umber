use crate::env::AssignmentScope;
use crate::env::banks::IntParam;
use crate::interner::InternerBudget;
use crate::scaled::Scaled;
use crate::world::{EffectRecord, PrintSink};

fn routed<G>(universe: &crate::Universe<G>) -> Vec<(PrintSink, String)> {
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

fn with_test_universe<R>(
    use_universe: impl for<'id> FnOnce(&mut crate::Universe<crate::GenerationBrand<'id>>) -> R,
) -> R {
    let budget = InternerBudget::new(16, 16, 256).expect("budget");
    crate::with_universe(budget, use_universe).expect("fresh universe")
}

#[test]
fn tracing_online_selects_the_transcript_or_both_channels() {
    for (tracing_online, expected) in [
        (0, PrintSink::Log),
        (-1, PrintSink::Log),
        (1, PrintSink::TerminalAndLog),
    ] {
        with_test_universe(|universe| {
            universe
                .assign_int_param(
                    IntParam::TRACING_ONLINE,
                    tracing_online,
                    AssignmentScope::Global,
                )
                .expect("set tracingonline");
            let mut diagnostic = universe.begin_diagnostic();
            diagnostic.print("trace");
            diagnostic.end(false);
            assert_eq!(
                routed(universe),
                vec![(expected, "trace\n".to_owned())],
                "\\tracingonline={tracing_online}"
            );
        });
    }
}

#[test]
fn end_diagnostic_closes_the_line_and_optional_blank_line() {
    for (blank_line, expected) in [(false, "trace\n"), (true, "trace\n\n")] {
        with_test_universe(|universe| {
            let mut diagnostic = universe.begin_diagnostic();
            diagnostic.print("trace");
            diagnostic.end(blank_line);
            assert_eq!(
                routed(universe),
                vec![(PrintSink::Log, expected.to_owned())]
            );
        });
    }
}

#[test]
fn print_nl_breaks_only_a_routed_partial_line() {
    with_test_universe(|universe| {
        universe.world_mut().write_text(PrintSink::Terminal, "term");
        let mut diagnostic = universe.begin_diagnostic();
        diagnostic.print_nl("first").print_nl("second");
        diagnostic.end(false);
        assert_eq!(
            routed(universe),
            vec![
                (PrintSink::Terminal, "term".to_owned()),
                (PrintSink::Log, "first\nsecond\n".to_owned()),
            ]
        );
    });
}

#[test]
fn online_print_nl_breaks_a_terminal_partial_line() {
    with_test_universe(|universe| {
        universe
            .assign_int_param(IntParam::TRACING_ONLINE, 1, AssignmentScope::Global)
            .expect("set tracingonline");
        universe.world_mut().write_text(PrintSink::Terminal, "term");
        let mut diagnostic = universe.begin_diagnostic();
        diagnostic.print_nl("first");
        diagnostic.end(false);
        assert_eq!(
            routed(universe),
            vec![
                (PrintSink::Terminal, "term".to_owned()),
                (PrintSink::TerminalAndLog, "\nfirst\n".to_owned()),
            ]
        );
    });
}

#[test]
fn scalar_printing_matches_tex_web() {
    with_test_universe(|universe| {
        let mut diagnostic = universe.begin_diagnostic();
        diagnostic
            .print_int(-4168)
            .print_char('/')
            .print_scaled(Scaled::from_raw(19_516_436))
            .print_char('/')
            .print_scaled(Scaled::from_raw(-163_840));
        diagnostic.end(false);
        assert_eq!(
            routed(universe),
            vec![(PrintSink::Log, "-4168/297.79718/-2.5\n".to_owned())]
        );
    });
}
