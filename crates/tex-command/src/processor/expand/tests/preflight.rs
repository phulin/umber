//! Preflight command-slot reuse and bounded expansion work.

use super::*;

#[test]
#[cfg(feature = "profiling")]
fn one_and_4096_preflight_expansions_reuse_one_slot_with_exact_linear_work() {
    let one = empty_macro_delivery_evidence(1);
    let many = empty_macro_delivery_evidence(4_096);

    for (expansions, evidence) in [(1, one), (4_096, many)] {
        assert_eq!(evidence.slot_initializations, 0);
        assert_eq!(evidence.rich_materializations, 1);
        // Preflight materializes its initial command; subsequent ordinary
        // macro activations consume only their meaning and invocation facts.
        assert_eq!(evidence.resolved_writes, 2);
        assert_eq!(evidence.expanded_classifications, 2);
        assert_eq!(evidence.command_clones, 0);
        assert_eq!(evidence.token_frame_steps, expansions + 1);
        assert_eq!(evidence.meaning_lookups, expansions);
        assert_eq!(evidence.expanded_deliveries, 1);
        #[cfg(feature = "profiling")]
        {
            assert_eq!(evidence.allocations, 0);
            assert_eq!(evidence.allocated_bytes, 0);
        }
    }
}

#[test]
fn expandable_preflight_delivery_uses_one_caller_owned_command_slot() {
    crate::test_harness::with_universe(|universe| {
        let replacement = Token::Char {
            ch: 'A',
            cat: Catcode::Letter,
        };
        let definition = universe
            .allocate_definition(&[], &[TokenWord::pack(replacement)])
            .expect("definition");
        let symbol = universe.intern("m").expect("macro name");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
                AssignmentScope::Global,
            )
            .expect("macro meaning");
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [Token::Cs(symbol.symbol())]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(2).expect("expanded delivery fuel");
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let mut destination = None;
        let ownership_before = crate::command::command_ownership_counters();
        assert_eq!(
            processor
                .preflight_command_into(&mut destination)
                .expect("preflight delivery"),
            crate::DeliveryStatus::Command
        );
        let settled = destination
            .as_ref()
            .expect("expanded delivery occupies the caller destination");
        assert_eq!(settled.spelling().semantic_token(), replacement);
        assert_eq!(
            settled.meaning(),
            Meaning::CharToken {
                ch: 'A',
                cat: Catcode::Letter
            }
        );
        assert_eq!(processor.fuel.burned(), 2);
        let ownership_after = crate::command::command_ownership_counters();
        assert_eq!(ownership_after.clones - ownership_before.clones, 0);
    });
}

#[test]
fn unexpandable_preflight_classifies_once_and_reuses_one_slot() {
    crate::test_harness::with_universe(|universe| {
        let token = Token::Char {
            ch: 'A',
            cat: Catcode::Letter,
        };
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [token]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(1).expect("raw delivery fuel");
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let mut destination = None;
        let ownership_before = crate::command::command_ownership_counters();
        let classifications_before = super::super::expanded_classifications();
        assert_eq!(
            processor
                .preflight_command_into(&mut destination)
                .expect("preflight delivery"),
            crate::DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .as_ref()
                .expect("raw delivery occupies the caller destination")
                .spelling()
                .semantic_token(),
            token
        );
        assert_eq!(processor.fuel.burned(), 1);
        drop(processor);
        let ownership_after = crate::command::command_ownership_counters();
        assert_eq!(
            super::super::expanded_classifications() - classifications_before,
            1
        );
        assert_eq!(
            ownership_after.slot_initializations - ownership_before.slot_initializations,
            0
        );
        assert_eq!(ownership_after.clones - ownership_before.clones, 0);
    });
}

#[test]
#[cfg(feature = "profiling")]
fn raw_main_loop_exit_preserves_the_existing_expanded_work_boundary() {
    crate::test_harness::with_universe(|universe| {
        let token = Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        };
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [token]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let work_before = fuel.work();
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let mut destination = None;

        assert_eq!(
            processor
                .main_loop_lookahead_into(&mut destination)
                .expect("raw main-loop exit"),
            crate::DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .as_ref()
                .expect("main-loop exit occupies its caller destination")
                .spelling()
                .semantic_token(),
            token
        );
        drop(processor);
        assert_eq!(
            fuel.work().expanded_deliveries - work_before.expanded_deliveries,
            0
        );
    });
}
