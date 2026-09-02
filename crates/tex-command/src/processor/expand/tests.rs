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

fn collect_expanded_characters<G>(
    universe: &mut tex_state::Universe<G>,
    command: &mut CommandState<G>,
) -> String {
    let mut capabilities = CommandHostCapabilities::default();
    let mut fuel = crate::CommandFuelLedger::default();
    let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
    let mut context = universe.command_context().expect("command context");
    let mut processor = crate::test_harness::processor(
        command,
        &mut context,
        &mut capabilities,
        &mut fuel,
        &mut diagnostic_effects,
    );
    let mut output = String::new();
    while let Some(command) = processor.get_x_token().expect("expanded delivery") {
        match command.meaning() {
            tex_state::meaning::ResolvedMeaning::Static(Meaning::CharToken { ch, .. }) => {
                output.push(ch);
            }
            other => panic!("expected expanded character, found {other:?}"),
        }
    }
    output
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

#[cfg(feature = "profiling")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OrdinaryDeliveryEvidence {
    slot_initializations: u64,
    resolved_writes: u64,
    expanded_classifications: u64,
    command_clones: u64,
    token_frame_steps: u64,
    meaning_lookups: u64,
    expanded_deliveries: u64,
    #[cfg(feature = "profiling")]
    allocations: u64,
    #[cfg(feature = "profiling")]
    allocated_bytes: u64,
}

#[cfg(feature = "profiling")]
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
            resolved_writes: after_ownership.resolved_writes - before_ownership.resolved_writes,
            expanded_classifications: super::expanded_classifications() - classifications_before,
            command_clones: after_ownership.clones - before_ownership.clones,
            token_frame_steps: work.token_frame_steps - work_before.token_frame_steps,
            meaning_lookups: work.meaning_lookups - work_before.meaning_lookups,
            expanded_deliveries: work.expanded_deliveries - work_before.expanded_deliveries,
            #[cfg(feature = "profiling")]
            allocations: after_allocations.calls - before_allocations.calls,
            #[cfg(feature = "profiling")]
            allocated_bytes: after_allocations.requested_bytes - before_allocations.requested_bytes,
        }
    })
}

#[test]
#[cfg(feature = "profiling")]
fn one_and_4096_preflight_expansions_reuse_one_slot_with_exact_linear_work() {
    let one = empty_macro_delivery_evidence(1);
    let many = empty_macro_delivery_evidence(4_096);

    for (expansions, evidence) in [(1, one), (4_096, many)] {
        assert_eq!(evidence.slot_initializations, 1);
        assert_eq!(evidence.resolved_writes, expansions + 1);
        assert_eq!(evidence.expanded_classifications, expansions + 1);
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
fn destination_owned_expansion_result_excludes_suspension_payload() {
    struct FormerSuspendedExpansion<G> {
        _resume: crate::state::PendingExpansionResume,
        _child: Option<
            crate::execution_scratch::ChildContinuation<
                G,
                crate::state::PendingExpansionChildDestination,
            >,
        >,
    }
    struct FormerExpansionFailure<G> {
        _error: crate::CommandError,
        _suspended: Option<FormerSuspendedExpansion<G>>,
    }

    let current = core::mem::size_of::<Result<(), crate::CommandError>>();
    let former = core::mem::size_of::<FormerExpansionFailure<()>>();
    assert!(current < former, "current={current}, former={former}");
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
fn unexpandable_preflight_classifies_once_and_reuses_one_slot() {
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
        let ownership_after = crate::command::command_ownership_counters();
        assert_eq!(
            super::expanded_classifications() - classifications_before,
            1
        );
        assert_eq!(
            ownership_after.slot_initializations - ownership_before.slot_initializations,
            1
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
fn input_suspension_retains_delivery_expansion_and_rollback_replays_the_same_prefix() {
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
        #[cfg(feature = "profiling")]
        {
            command.profile_reset_delivery_loop_counters();
            command.profile_reset_stored_token_advance_counters();
        }
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
            let error = match processor.next_alignment_lookahead() {
                Err(error) => error,
                Ok(_) => panic!("unresolved input must suspend"),
            };
            assert!(matches!(
                error,
                crate::CommandError::MissingInput { ref name, .. } if name == "child.tex"
            ));
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
            let error = match processor.next_alignment_lookahead() {
                Err(error) => error,
                Ok(_) => panic!("unfulfilled retry must resuspend"),
            };
            assert!(matches!(
                error,
                crate::CommandError::MissingInput { ref name, .. } if name == "child.tex"
            ));
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
            let lookahead = processor
                .next_alignment_lookahead()
                .expect("resource-backed resume")
                .expect("resumed source command");
            assert!(matches!(
                lookahead,
                crate::AlignmentLookahead::PendingExpanded(_)
            ));
            let delivered = processor.commit_alignment_lookahead_delivery(lookahead);
            assert_eq!(
                delivered.spelling().semantic_token(),
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
        #[cfg(feature = "profiling")]
        {
            let (warm, _cold, intermediate) = processor.command.profile_delivery_loop_counters();
            assert!(
                warm > 0,
                "suspend/resume/rollback must retain scalar delivery"
            );
            assert_eq!(intermediate, 0);
            let (
                _selections,
                loads,
                advances,
                writes,
                lookups,
                _parameters,
                relays,
                _segment_inspections,
                _run_transitions,
            ) = processor.command.profile_stored_token_advance_counters();
            assert!(loads > 0, "suspension fixture must traverse stored input");
            assert_eq!(advances, loads);
            assert_eq!(writes, loads);
            assert!(
                lookups > 0,
                "the restored input primitive must resolve once"
            );
            assert_eq!(relays, 0);
        }
    });
}

#[test]
fn nested_expandafter_suspension_parks_each_command_once() {
    crate::test_harness::with_universe(|universe| {
        let expandafter = install_static(
            universe,
            "expandafter",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter),
        );
        let input = install_static(
            universe,
            "input",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Input),
        );
        let first = Token::Char {
            ch: 'A',
            cat: Catcode::Letter,
        };
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [expandafter, first, input]
                .into_iter()
                .chain("child".chars().map(|ch| Token::Char {
                    ch,
                    cat: Catcode::Letter,
                }))
                .chain([Token::Char {
                    ch: ' ',
                    cat: Catcode::Space,
                }]),
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let ownership_before = crate::command::command_ownership_counters();

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
                .expect_err("nested input expansion suspends");
            assert!(matches!(
                error,
                crate::CommandError::MissingInput { ref name, .. } if name == "child.tex"
            ));
            assert!(destination.is_none());
            (
                processor
                    .take_pending_expansion_work()
                    .expect("outer expansion owns the parked command chain"),
                processor.delivery_cursor(),
            )
        };
        let ownership_after_suspend = crate::command::command_ownership_counters();
        assert_eq!(
            ownership_after_suspend.clones - ownership_before.clones,
            0,
            "nested callers must retain only child edges, not cloned commands"
        );
        assert_eq!(
            ownership_after_suspend.expansion_moves_in - ownership_before.expansion_moves_in,
            2,
            "the input command and its expandafter parent each park once"
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
                    .expect("nested expansion resumes"),
                crate::DeliveryStatus::Command
            );
            assert_eq!(
                destination
                    .expect("expandafter replays its first token")
                    .spelling()
                    .semantic_token(),
                first
            );
        }
        let ownership_after_resume = crate::command::command_ownership_counters();
        assert_eq!(ownership_after_resume.clones - ownership_before.clones, 0);
        assert_eq!(
            ownership_after_resume.expansion_moves_out - ownership_before.expansion_moves_out,
            2
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

#[test]
fn pdf_insert_height_queries_live_state_and_distinguishes_missing_from_zero() {
    crate::test_harness::with_universe(|universe| {
        let pdf_insert_height = install_static(
            universe,
            "pdfinsertht",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfInsertHeight),
        );
        let class = [Token::Char {
            ch: '7',
            cat: Catcode::Other,
        }];

        let mut missing = CommandState::new(CommandProfile::PDFTEX14029);
        crate::test_harness::push(
            &mut missing,
            std::iter::once(pdf_insert_height).chain(class),
        );
        assert_eq!(collect_expanded_characters(universe, &mut missing), "0pt");

        universe
            .command_context()
            .expect("command context")
            .upsert_page_insertion(tex_state::page::PageInsertion::new(
                7,
                tex_state::scaled::Scaled::from_raw(0),
            ));
        let mut present = CommandState::new(CommandProfile::PDFTEX14029);
        crate::test_harness::push(
            &mut present,
            std::iter::once(pdf_insert_height).chain(class),
        );
        assert_eq!(collect_expanded_characters(universe, &mut present), "0.0pt");
    });
}
