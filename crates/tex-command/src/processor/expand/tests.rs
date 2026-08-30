use tex_state::env::AssignmentScope;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags, MeaningWord};
use tex_state::token::{Catcode, Token, TokenWord};

use crate::{CommandHostCapabilities, CommandProfile, CommandState};

fn install_static<G>(universe: &mut tex_state::Universe<G>, name: &str, meaning: Meaning) -> Token {
    let symbol = universe.intern(name).expect("intern primitive");
    universe
        .assign_meaning(
            symbol,
            MeaningWord::from_static(meaning),
            AssignmentScope::Global,
        )
        .expect("install primitive");
    Token::Cs(symbol.symbol())
}

#[test]
fn parameterless_macro_expands_from_a_generation_typed_definition() {
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
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(&mut command, [Token::Cs(symbol.symbol())]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        let expanded = processor
            .get_x_token()
            .expect("macro expansion")
            .expect("replacement command");
        assert_eq!(expanded.spelling().semantic_token(), replacement);
        assert_eq!(
            expanded.meaning(),
            Meaning::CharToken {
                ch: 'A',
                cat: Catcode::Letter,
            }
        );
        assert!(processor.get_x_token().expect("end").is_none());
    });
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OrdinaryDeliveryEvidence {
    slot_initializations: u64,
    raw_writes: u64,
    expanded_classifications: u64,
    command_clones: u64,
    token_frame_steps: u64,
    meaning_lookups: u64,
    expanded_deliveries: u64,
    expansions: u64,
    #[cfg(feature = "profiling")]
    allocations: u64,
    #[cfg(feature = "profiling")]
    allocated_bytes: u64,
}

fn empty_macro_delivery_evidence(expansions: usize) -> OrdinaryDeliveryEvidence {
    crate::test_harness::with_universe(|universe| {
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("empty definition");
        let symbol = universe.intern("m").expect("macro name");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
                AssignmentScope::Global,
            )
            .expect("macro meaning");
        let terminal = Token::Char {
            ch: 'Z',
            cat: Catcode::Letter,
        };
        let mut input = Vec::with_capacity((expansions + 1) * 2);
        for _ in 0..2 {
            input.resize(input.len() + expansions, Token::Cs(symbol.symbol()));
            input.push(terminal);
        }

        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(&mut command, input);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut destination = None;

        {
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            assert_eq!(
                processor
                    .get_x_token_into(&mut destination)
                    .expect("warm ordinary expanded delivery"),
                crate::DeliveryStatus::Command
            );
        }
        assert_eq!(
            destination
                .take()
                .expect("warm terminal command")
                .spelling()
                .semantic_token(),
            terminal
        );
        let before_ownership = crate::command::command_ownership_counters();
        let classifications_before = super::expanded_classifications();
        let expansions_before = command.expansion.cumulative_expansions;
        let work_before = fuel.work();

        #[cfg(feature = "profiling")]
        let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
        #[cfg(feature = "profiling")]
        let before_allocations =
            tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        {
            #[cfg(feature = "profiling")]
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            assert_eq!(
                processor
                    .preflight_command_into(&mut destination)
                    .expect("preflight expansion delivery"),
                crate::DeliveryStatus::Command
            );
        }
        #[cfg(feature = "profiling")]
        let after_allocations =
            tex_state::measurement::hot_core_thread_allocation_measurement(owner);

        let delivered = destination.expect("terminal command");
        assert_eq!(delivered.spelling().semantic_token(), terminal);
        assert_eq!(
            delivered.meaning(),
            Meaning::CharToken {
                ch: 'Z',
                cat: Catcode::Letter,
            }
        );
        let after_ownership = crate::command::command_ownership_counters();
        let work = fuel.work();
        OrdinaryDeliveryEvidence {
            slot_initializations: after_ownership.slot_initializations
                - before_ownership.slot_initializations,
            raw_writes: after_ownership.raw_writes - before_ownership.raw_writes,
            expanded_classifications: super::expanded_classifications() - classifications_before,
            command_clones: after_ownership.clones - before_ownership.clones,
            token_frame_steps: work.token_frame_steps - work_before.token_frame_steps,
            meaning_lookups: work.meaning_lookups - work_before.meaning_lookups,
            expanded_deliveries: work.expanded_deliveries - work_before.expanded_deliveries,
            expansions: command.expansion.cumulative_expansions - expansions_before,
            #[cfg(feature = "profiling")]
            allocations: after_allocations.calls - before_allocations.calls,
            #[cfg(feature = "profiling")]
            allocated_bytes: after_allocations.requested_bytes - before_allocations.requested_bytes,
        }
    })
}

#[test]
fn one_and_4096_preflight_expansions_reuse_one_slot_with_exact_linear_work() {
    let one = empty_macro_delivery_evidence(1);
    let many = empty_macro_delivery_evidence(4_096);

    for (expansions, evidence) in [(1, one), (4_096, many)] {
        assert_eq!(evidence.slot_initializations, 1);
        assert_eq!(evidence.raw_writes, expansions + 1);
        assert_eq!(evidence.expanded_classifications, expansions + 1);
        assert_eq!(evidence.command_clones, 0);
        assert_eq!(evidence.token_frame_steps, expansions + 1);
        assert_eq!(evidence.meaning_lookups, expansions);
        assert_eq!(evidence.expanded_deliveries, 1);
        assert_eq!(evidence.expansions, expansions);
        #[cfg(feature = "profiling")]
        {
            assert_eq!(evidence.allocations, 0);
            assert_eq!(evidence.allocated_bytes, 0);
        }
    }
}

#[test]
fn internal_delivery_result_does_not_carry_the_rich_error_envelope() {
    let internal = size_of::<Result<crate::DeliveryStatus, crate::processor::DeliveryFailed>>();
    let public = size_of::<Result<crate::DeliveryStatus, crate::CommandError>>();

    assert!(internal < public, "internal={internal}, public={public}");
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
        let mut fuel = crate::CommandFuelLedger::default();
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
        let ownership_after = crate::command::command_ownership_counters();
        assert_eq!(ownership_after.clones - ownership_before.clones, 0);
    });
}

#[test]
fn unexpandable_preflight_classifies_once_without_a_second_driver_completion() {
    crate::test_harness::with_universe(|universe| {
        let token = Token::Char {
            ch: 'A',
            cat: Catcode::Letter,
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
        let ownership_before = crate::command::command_ownership_counters();
        let classifications_before = super::expanded_classifications();
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
        drop(processor);
        let work_after = fuel.work();
        let ownership_after = crate::command::command_ownership_counters();
        assert_eq!(
            super::expanded_classifications() - classifications_before,
            1
        );
        assert_eq!(
            work_after.expanded_deliveries - work_before.expanded_deliveries,
            0
        );
        assert_eq!(
            ownership_after.slot_initializations - ownership_before.slot_initializations,
            1
        );
        assert_eq!(ownership_after.clones - ownership_before.clones, 0);
    });
}

#[test]
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

#[test]
fn noexpand_suppresses_exactly_one_expandable_delivery() {
    crate::test_harness::with_universe(|universe| {
        let noexpand = install_static(
            universe,
            "noexpand",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand),
        );
        let replacement = Token::Char {
            ch: 'B',
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
        let macro_token = Token::Cs(symbol.symbol());
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [noexpand, macro_token, macro_token]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        let suppressed = processor
            .get_x_token()
            .expect("suppressed delivery")
            .expect("suppressed command");
        assert_eq!(suppressed.spelling().semantic_token(), macro_token);
        assert_eq!(suppressed.meaning(), Meaning::Relax);
        assert_eq!(
            processor
                .get_x_token()
                .expect("second delivery")
                .expect("replacement")
                .spelling()
                .semantic_token(),
            replacement
        );
    });
}

#[test]
fn input_suspension_moves_the_command_once_and_rollback_replays_the_same_prefix() {
    crate::test_harness::with_universe(|universe| {
        let ownership_before = crate::command::command_ownership_counters();
        let input = install_static(
            universe,
            "input",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Input),
        );
        let filename = "child".chars().map(|ch| Token::Char {
            ch,
            cat: Catcode::Letter,
        });
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            std::iter::once(input)
                .chain(filename)
                .chain(std::iter::once(Token::Char {
                    ch: ' ',
                    cat: Catcode::Space,
                })),
        );
        let snapshot = command.snapshot(universe).expect("input prefix snapshots");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();

        let (resume, delivery_cursor) = {
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            let mut destination = None;
            let error = processor
                .get_x_token_into(&mut destination)
                .expect_err("unresolved input suspends");
            assert!(matches!(
                error,
                crate::CommandError::MissingInput { ref name, .. } if name == "child.tex"
            ));
            assert!(destination.is_none());
            let delivery_cursor = processor.delivery_cursor();
            let resume = processor
                .take_pending_expansion_work()
                .expect("typed parked expansion suspension");
            assert!(processor.scanner_resume.is_none());
            (resume, delivery_cursor)
        };
        let ownership_after_first = crate::command::command_ownership_counters();
        assert_eq!(ownership_after_first.clones - ownership_before.clones, 0);
        assert_eq!(
            ownership_after_first.expansion_moves_in - ownership_before.expansion_moves_in,
            1
        );
        assert_eq!(
            ownership_after_first.expansion_moves_out - ownership_before.expansion_moves_out,
            0
        );

        let resume = {
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            processor.resume_delivery_cursor(delivery_cursor);
            processor.install_expansion_resume(resume);
            let mut destination = None;
            let error = processor
                .get_x_token_into(&mut destination)
                .expect_err("unfulfilled retry resuspends");
            assert!(matches!(
                error,
                crate::CommandError::MissingInput { ref name, .. } if name == "child.tex"
            ));
            assert!(destination.is_none());
            processor
                .take_pending_expansion_work()
                .expect("second suspension parks the same sole owner")
        };
        let ownership_after_second = crate::command::command_ownership_counters();
        assert_eq!(ownership_after_second.clones - ownership_before.clones, 0);
        assert_eq!(
            ownership_after_second.expansion_moves_in - ownership_before.expansion_moves_in,
            2
        );
        assert_eq!(
            ownership_after_second.expansion_moves_out - ownership_before.expansion_moves_out,
            1
        );
        capabilities.register_input(
            "child.tex",
            crate::SourceRegistration::new(crate::RegisteredSourceKind::Generated, &b"Q"[..])
                .with_name("child.tex"),
        );
        {
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            processor.resume_delivery_cursor(delivery_cursor);
            processor.install_expansion_resume(resume);
            let mut destination = None;
            assert_eq!(
                processor
                    .get_x_token_into(&mut destination)
                    .expect("resource-backed resume"),
                crate::DeliveryStatus::Command
            );
            assert_eq!(
                destination
                    .take()
                    .expect("resumed source command")
                    .spelling()
                    .semantic_token(),
                Token::Char {
                    ch: 'Q',
                    cat: Catcode::Letter,
                }
            );
            assert!(processor.scanner_resume.is_none());
        }
        let ownership_after_resume = crate::command::command_ownership_counters();
        assert_eq!(ownership_after_resume.clones - ownership_before.clones, 0);
        assert_eq!(
            ownership_after_resume.expansion_moves_in - ownership_before.expansion_moves_in,
            2
        );
        assert_eq!(
            ownership_after_resume.expansion_moves_out - ownership_before.expansion_moves_out,
            2
        );

        command
            .rollback(&snapshot, universe)
            .expect("resumed input prefix rolls back");
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        assert_eq!(
            processor
                .get_x_token()
                .expect("restored prefix expands")
                .expect("restored child source command")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: 'Q',
                cat: Catcode::Letter,
            }
        );
    });
}

#[test]
fn protected_replay_delivery_writes_the_terminal_macro_into_its_caller_slot() {
    crate::test_harness::with_universe(|universe| {
        let replacement = Token::Char {
            ch: 'P',
            cat: Catcode::Letter,
        };
        let definition = universe
            .allocate_definition(&[], &[TokenWord::pack(replacement)])
            .expect("definition");
        let symbol = universe.intern("protected").expect("macro name");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::PROTECTED, definition),
                AssignmentScope::Global,
            )
            .expect("macro meaning");
        let macro_token = Token::Cs(symbol.symbol());
        let mut command = CommandState::new(CommandProfile::ETEX26);
        crate::test_harness::push(&mut command, [macro_token]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
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
        assert_eq!(
            processor
                .get_x_or_protected_with_replay_completion_into(&mut destination)
                .expect("protected delivery"),
            super::DeliveryStatus::Command
        );
        let delivered = destination.expect("caller destination");
        assert_eq!(delivered.spelling().semantic_token(), macro_token);
        assert!(matches!(
            delivered.meaning(),
            tex_state::meaning::ResolvedMeaning::Macro { flags, .. }
                if flags.contains(MeaningFlags::PROTECTED)
        ));
    });
}

#[test]
fn csname_relaxes_an_already_interned_undefined_name() {
    crate::test_harness::with_universe(|universe| {
        let csname = install_static(
            universe,
            "csname",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::CsName),
        );
        let endcsname = install_static(
            universe,
            "endcsname",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName),
        );
        let latent = universe.intern("latent").expect("pre-intern name");
        let mut input = vec![csname];
        input.extend("latent".chars().map(|ch| Token::Char {
            ch,
            cat: Catcode::Letter,
        }));
        input.push(endcsname);
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, input);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        let expanded = processor
            .get_x_token()
            .expect("csname expansion")
            .expect("named control sequence");
        assert_eq!(
            expanded.spelling().semantic_token(),
            Token::Cs(latent.symbol())
        );
        assert_eq!(expanded.meaning(), Meaning::Relax);
        assert!(processor.get_x_token().expect("end").is_none());
    });
}
