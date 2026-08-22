use tex_state::AssignmentScope;
use tex_state::diagnostic::DiagnosticEffects;
use tex_state::env::banks::IntParam;
use tex_state::world::{EffectRecord, PrintSink};

use super::ExecutionDiagnosticContext;

fn writes<G>(universe: &tex_state::Universe<G>) -> Vec<(PrintSink, String)> {
    universe
        .world()
        .effect_records()
        .iter()
        .filter_map(|record| match record {
            EffectRecord::StreamWrite { sink, text } => Some((*sink, text.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn extended_missing_character_forces_online_routing_inside_admission() {
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let before = universe.world().effect_records().len();
        let mut command = universe
            .command_context()
            .expect("admitted diagnostic state");
        command
            .assign_int_param(IntParam::TRACING_LOST_CHARS, 2, AssignmentScope::Global)
            .expect("set tracinglostchars");
        let font = command.current_font();
        let mut diagnostic_effects = DiagnosticEffects::new();
        super::report_missing_character_warning(
            &mut command,
            &mut diagnostic_effects,
            font,
            '?',
            true,
        );
        assert_eq!(command.int_param(IntParam::TRACING_LOST_CHARS), 2);
        drop(command);
        universe
            .world_mut()
            .publish_diagnostic_effects(diagnostic_effects);
        let sequences = universe.world().effect_sequences();
        assert!(!sequences[before..].is_empty());
        assert!(
            sequences[before..]
                .windows(2)
                .all(|pair| pair[0] == pair[1]),
            "one missing-character diagnostic must retain one logical batch"
        );

        assert!(writes(universe).iter().any(|(sink, text)| {
            *sink == PrintSink::TerminalAndLog
                && text.contains("Missing character: There is no ? in font nullfont!")
        }));
    });
}

#[test]
fn forced_online_missing_character_uses_the_joint_print_nl_offset_test() {
    // e-TeX change 17.516 temporarily routes a level-two warning to both
    // sinks. tex.web §62 tests the selected offsets jointly, so an open
    // terminal line makes the ensuing newline visible in the already-closed
    // transcript too. This is the blank line between e-TRIP's `b` and `c`
    // nullfont warnings.
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        universe
            .world_mut()
            .write_text(PrintSink::TerminalAndLog, "(");
        let font = universe.current_font().expect("live nullfont identity");

        let mut first = DiagnosticEffects::new();
        {
            let mut command = universe.command_context().expect("first admission");
            command
                .assign_int_param(IntParam::TRACING_LOST_CHARS, 1, AssignmentScope::Global)
                .expect("enable ordinary warning");
            super::report_missing_character_warning(&mut command, &mut first, font, 'b', true);
        }
        universe.world_mut().publish_diagnostic_effects(first);

        let mut second = DiagnosticEffects::new();
        {
            let mut command = universe.command_context().expect("second admission");
            command
                .assign_int_param(IntParam::TRACING_LOST_CHARS, 2, AssignmentScope::Global)
                .expect("enable forced-online warning");
            super::report_missing_character_warning(&mut command, &mut second, font, 'c', true);
        }
        universe.world_mut().publish_diagnostic_effects(second);

        let log = universe
            .world()
            .effect_records()
            .iter()
            .filter_map(|record| match record {
                EffectRecord::StreamWrite {
                    sink: PrintSink::Log | PrintSink::TerminalAndLog,
                    text,
                } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert!(
            log.contains(
                "Missing character: There is no b in font nullfont!\n\nMissing character: There is no c in font nullfont!"
            ),
            "{log:?}"
        );
    });
}

#[test]
fn infinite_shrink_report_uses_only_detached_output_context() {
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let context =
            ExecutionDiagnosticContext::new(41, 37, true, "\nl.41 detached diagnostic context\n");
        let mut command = universe
            .command_context()
            .expect("admitted diagnostic state");
        super::report_page_infinite_shrinkage(&mut command, &context)
            .expect("nonstop mode recovers from the diagnostic");
        drop(command);

        let text = writes(universe)
            .into_iter()
            .map(|(_, text)| text)
            .collect::<String>();
        assert!(text.contains("Infinite glue shrinkage found on current page"));
        assert!(text.contains("l.41 detached diagnostic context"));
    });
}

#[test]
fn ignored_split_error_preserves_forced_online_text() {
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let context = ExecutionDiagnosticContext::source_free("unused");
        let before = universe.world().effect_records().len();
        let mut command = universe
            .command_context()
            .expect("admitted diagnostic state");
        command
            .assign_int_param(IntParam::IGNORE_PRIMITIVE_ERROR, 1, AssignmentScope::Global)
            .expect("set ignore primitive error");
        let mut diagnostic_effects = DiagnosticEffects::new();
        super::report_split_infinite_shrinkage(&mut command, &mut diagnostic_effects, &context)
            .expect("ignored error recovers");
        drop(command);
        universe
            .world_mut()
            .publish_diagnostic_effects(diagnostic_effects);
        let sequences = universe.world().effect_sequences();
        assert!(!sequences[before..].is_empty());
        assert!(
            sequences[before..]
                .windows(2)
                .all(|pair| pair[0] == pair[1]),
            "one ignored split diagnostic must retain one logical batch"
        );

        assert_eq!(
            writes(universe),
            vec![(
                PrintSink::TerminalAndLog,
                "\nignored error: Infinite glue shrinkage found in box being split\n".to_owned()
            )]
        );
    });
}
