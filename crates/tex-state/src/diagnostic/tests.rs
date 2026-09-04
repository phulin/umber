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
            let mut effects = super::DiagnosticEffects::new();
            let mut diagnostic = universe.begin_diagnostic(&mut effects);
            diagnostic.print("trace");
            diagnostic.end(false);
            universe.world_mut().publish_diagnostic_effects(effects);
            assert_eq!(
                routed(universe),
                vec![(expected, "trace\n".to_owned())],
                "\\tracingonline={tracing_online}"
            );
        });
    }
}

#[test]
fn eager_publication_materializes_log_only_and_term_only_batches_to_exact_sinks() {
    with_test_universe(|universe| {
        let mut effects = super::DiagnosticEffects::new();
        effects.push(super::DetachedDiagnosticEffect {
            selector: crate::print::Selector::LogOnly,
            max_print_line: 79,
            records_warning_history: false,
            operations: vec![super::DiagnosticPrintOperation::Rendered(
                "log-only\n".into(),
            )],
        });
        effects.push(super::DetachedDiagnosticEffect {
            selector: crate::print::Selector::TermOnly,
            max_print_line: 79,
            records_warning_history: false,
            operations: vec![super::DiagnosticPrintOperation::Rendered(
                "term-only\n".into(),
            )],
        });

        universe.world_mut().publish_diagnostic_effects(effects);
        universe
            .publish_effect_prefix(universe.world().effect_pos())
            .expect("eager diagnostic publication commits");

        assert_eq!(
            universe.world().memory_log_output(),
            Some(&b"log-only\n"[..])
        );
        assert_eq!(
            universe.world().memory_terminal_output(),
            Some(&b"term-only\n"[..])
        );
        assert!(universe.world().effect_records().is_empty());
    });
}

#[test]
fn end_diagnostic_closes_the_line_and_optional_blank_line() {
    for (blank_line, expected) in [(false, "trace\n"), (true, "trace\n\n")] {
        with_test_universe(|universe| {
            let mut effects = super::DiagnosticEffects::new();
            let mut diagnostic = universe.begin_diagnostic(&mut effects);
            diagnostic.print("trace");
            diagnostic.end(blank_line);
            universe.world_mut().publish_diagnostic_effects(effects);
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
        let mut effects = super::DiagnosticEffects::new();
        let mut diagnostic = universe.begin_diagnostic(&mut effects);
        diagnostic.print_nl("first").print_nl("second");
        diagnostic.end(false);
        universe.world_mut().publish_diagnostic_effects(effects);
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
        let before = universe.world().effect_records().len();
        let mut effects = super::DiagnosticEffects::new();
        let mut diagnostic = universe.begin_diagnostic(&mut effects);
        diagnostic.print_nl("first");
        diagnostic.end(false);
        universe.world_mut().publish_diagnostic_effects(effects);
        assert_eq!(universe.world().effect_records().len(), before + 1);
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
fn admitted_online_diagnostic_overrides_tracingonline_without_assigning_it() {
    with_test_universe(|universe| {
        universe
            .assign_int_param(IntParam::TRACING_ONLINE, 0, AssignmentScope::Global)
            .expect("set tracingonline");
        let mut effects = super::DiagnosticEffects::new();
        {
            let command = universe
                .command_context()
                .expect("admitted command context");
            let mut diagnostic = command.begin_online_diagnostic(&mut effects);
            diagnostic.print("missing character");
            diagnostic.end(false);
        }
        universe.world_mut().publish_diagnostic_effects(effects);
        assert_eq!(
            universe.int_param(IntParam::TRACING_ONLINE),
            0,
            "routing override must not write eqtb"
        );
        assert_eq!(
            routed(universe),
            vec![(PrintSink::TerminalAndLog, "missing character\n".to_owned())]
        );
    });
}

#[test]
fn scalar_printing_matches_tex_web() {
    with_test_universe(|universe| {
        let mut effects = super::DiagnosticEffects::new();
        let mut diagnostic = universe.begin_diagnostic(&mut effects);
        diagnostic
            .print_int(-4168)
            .print_char('/')
            .print_scaled(Scaled::from_raw(19_516_436))
            .print_char('/')
            .print_scaled(Scaled::from_raw(-163_840));
        diagnostic.end(false);
        universe.world_mut().publish_diagnostic_effects(effects);
        assert_eq!(
            routed(universe),
            vec![(PrintSink::Log, "-4168/297.79718/-2.5\n".to_owned())]
        );
    });
}

#[test]
fn detached_publication_combines_equal_payloads_when_sink_offsets_diverge() {
    with_test_universe(|universe| {
        universe
            .assign_int_param(IntParam::TRACING_ONLINE, 1, AssignmentScope::Global)
            .expect("set tracingonline");
        universe
            .world_mut()
            .write_text(PrintSink::Terminal, "terminal-prefix");
        universe.world_mut().write_text(PrintSink::Log, "log\n");
        let before = universe.world().effect_records().len();

        let mut effects = super::DiagnosticEffects::new();
        let mut diagnostic = universe.begin_diagnostic(&mut effects);
        diagnostic.print_nl("trace");
        diagnostic.end(false);

        assert_eq!(universe.world().effect_records().len(), before);
        universe.world_mut().publish_diagnostic_effects(effects);
        assert_eq!(universe.world().effect_records().len(), before + 1);
        assert_eq!(
            &universe.world().effect_records()[before],
            &EffectRecord::StreamWrite {
                sink: PrintSink::TerminalAndLog,
                text: "\ntrace\n".to_owned(),
            }
        );
    });
}

#[test]
fn dropping_operation_collector_publishes_nothing() {
    with_test_universe(|universe| {
        let before = universe.world().effect_records().len();
        let mut effects = super::DiagnosticEffects::new();
        let mut diagnostic = universe.begin_diagnostic(&mut effects);
        diagnostic.print("rolled back");
        diagnostic.end(false);
        drop(effects);
        assert_eq!(universe.world().effect_records().len(), before);
    });
}

#[test]
fn first_recoverable_candidate_is_bounded_and_never_replaced() {
    let mut effects = super::DiagnosticEffects::new();
    let first = super::RecoverableDiagnostic::new(
        "missing-number",
        "界".repeat(171),
        vec![
            super::RecoverableDiagnosticArgument::Name("first".into()),
            super::RecoverableDiagnosticArgument::Name("second".into()),
        ],
        crate::InteractionMode::Nonstop,
    );
    effects.record_first_recoverable(first);
    effects.record_first_recoverable(super::RecoverableDiagnostic::new(
        "later",
        "later".into(),
        Vec::new(),
        crate::InteractionMode::Nonstop,
    ));

    let retained = effects
        .take_first_recoverable()
        .expect("first candidate retained");
    assert_eq!(retained.kind, "missing-number");
    assert!(retained.message.len() <= super::MAX_RECOVERABLE_DIAGNOSTIC_TEXT);
    assert!(retained.message.is_char_boundary(retained.message.len()));
    assert_eq!(retained.arguments.len(), 2);
    assert!(retained.site.is_none());
    assert!(!effects.has_first_recoverable());
}

#[test]
fn first_recoverable_provenance_is_frozen_after_its_first_report() {
    let mut effects = super::DiagnosticEffects::new();
    effects.record_first_recoverable(super::RecoverableDiagnostic::new(
        "first",
        "first".into(),
        Vec::new(),
        crate::InteractionMode::Nonstop,
    ));

    effects.complete_first_recoverable(super::DiagnosticSite {
        origin: Some(crate::token::OriginId::from_raw(7)),
        observed_token: None,
        command: None,
        command_operand: None,
        context: None,
        mode: Some("vertical mode"),
        scanner_status: "scalar",
        interaction: Some(crate::InteractionMode::Nonstop),
    });
    effects.complete_first_recoverable(super::DiagnosticSite {
        origin: Some(crate::token::OriginId::from_raw(8)),
        observed_token: None,
        command: Some("later".into()),
        command_operand: None,
        context: None,
        mode: Some("horizontal mode"),
        scanner_status: "font",
        interaction: Some(crate::InteractionMode::Batch),
    });

    let retained = effects
        .take_first_recoverable()
        .expect("first candidate retained");
    assert_eq!(
        retained.site.as_ref().and_then(|site| site.origin),
        Some(crate::token::OriginId::from_raw(7))
    );
    assert_eq!(
        retained.site.as_ref().map(|site| site.scanner_status),
        Some("scalar")
    );
}

#[test]
fn diagnostic_publication_leaves_the_candidate_for_commit() {
    with_test_universe(|universe| {
        let mut effects = super::DiagnosticEffects::new();
        effects.record_first_recoverable(super::RecoverableDiagnostic::new(
            "command-recoverable",
            "first".into(),
            Vec::new(),
            crate::InteractionMode::Nonstop,
        ));
        effects.push_ordinary_rendered(crate::InteractionMode::Nonstop, 79, "trace".into());

        universe
            .world_mut()
            .publish_diagnostic_effects_preserving(&mut effects);

        assert!(effects.has_first_recoverable());
        assert_eq!(
            effects
                .take_first_recoverable()
                .expect("first recoverable candidate")
                .message
                .as_ref(),
            "first"
        );
        assert!(effects.is_empty());
    });
}
