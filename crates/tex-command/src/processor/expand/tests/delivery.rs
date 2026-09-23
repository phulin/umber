//! Resident and macro-body delivery through the shared expansion loop.

use super::*;

#[test]
fn sequential_resident_reads_advance_once_in_optimized_delivery() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [letter('A'), letter('B')]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(2).expect("resident delivery fuel");
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        let first = processor
            .get_next()
            .expect("first resident delivery")
            .expect("first resident command");
        let second = processor
            .get_next()
            .expect("second resident delivery")
            .expect("second resident command");
        assert_eq!(first.spelling().semantic_token(), letter('A'));
        assert_eq!(second.spelling().semantic_token(), letter('B'));
        assert_eq!(first.origin(), OriginId::UNKNOWN);
        assert_eq!(second.origin(), OriginId::UNKNOWN);
        assert_eq!(processor.fuel.burned(), 2);
    });
}

#[test]
fn sequential_macro_body_reads_advance_once_in_optimized_delivery() {
    crate::test_harness::with_universe(|universe| {
        let body_tokens = [letter('A'), letter('B')];
        let definition = universe
            .allocate_definition(
                &[],
                &body_tokens
                    .iter()
                    .copied()
                    .map(TokenWord::pack)
                    .collect::<Vec<_>>(),
            )
            .expect("macro body definition");
        let macro_name = universe
            .intern("sequentialbody")
            .expect("macro name")
            .symbol();
        let body = universe
            .command_context()
            .expect("macro body context")
            .admit_macro_body(definition)
            .expect("resident macro body")
            .2;
        let mut command = CommandState::default();
        command.push_macro_activation(macro_name, body, None, OriginId::UNKNOWN);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(2).expect("macro body delivery fuel");
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        let first = processor
            .get_x_token()
            .expect("first macro-body delivery")
            .expect("first macro-body command");
        let second = processor
            .get_x_token()
            .expect("second macro-body delivery")
            .expect("second macro-body command");
        assert_eq!(first.spelling().semantic_token(), letter('A'));
        assert_eq!(second.spelling().semantic_token(), letter('B'));
        assert_eq!(first.origin(), OriginId::UNKNOWN);
        assert_eq!(second.origin(), OriginId::UNKNOWN);
        assert_eq!(processor.fuel.burned(), 2);
    });
}
