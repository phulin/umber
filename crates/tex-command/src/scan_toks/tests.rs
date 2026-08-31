use tex_state::env::AssignmentScope;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags, MeaningWord};
use tex_state::token::{Catcode, Token, TokenWord};

use super::{ScanToksMode, reset_runaway_render_count, runaway_render_count};
use crate::{CommandHostCapabilities, CommandSemanticDiagnostic, CommandState, DeliveryStatus};

fn token(ch: char, cat: Catcode) -> Token {
    Token::Char { ch, cat }
}

#[cfg(feature = "profiling")]
fn case_shift_input(body_len: usize) -> Vec<Token> {
    let mut input = Vec::with_capacity(body_len.saturating_mul(2).saturating_add(5));
    for _ in 0..2 {
        input.push(token('{', Catcode::BeginGroup));
        input.extend(std::iter::repeat_n(token('a', Catcode::Letter), body_len));
        input.push(token('}', Catcode::EndGroup));
    }
    input.push(token('q', Catcode::Letter));
    input
}

#[cfg(feature = "profiling")]
#[test]
fn case_shift_one_64_and_4096_write_once_without_copy_or_warmed_allocation() {
    use super::{case_shift_path_counters, reset_case_shift_path_counters};

    for body_len in [1_usize, 64, 4096] {
        crate::test_harness::with_universe(|universe| {
            let mut command = CommandState::default();
            crate::test_harness::push(&mut command, case_shift_input(body_len));
            let mut capabilities = CommandHostCapabilities::default();
            let mut fuel = crate::CommandFuelLedger::default();
            let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();

            let warm_operation = command.begin_attempt_operation();
            {
                let mut context = universe.command_context().expect("command context");
                let mut processor = crate::test_harness::processor(
                    &mut command,
                    &mut context,
                    &mut capabilities,
                    &mut fuel,
                    &mut diagnostic_effects,
                );
                processor.shift_case(true).expect("warm case shift");
                for _ in 0..body_len {
                    let shifted = processor
                        .get_token()
                        .expect("warm shifted delivery")
                        .expect("warm shifted token");
                    assert_eq!(
                        shifted.spelling().semantic_token(),
                        token('A', Catcode::Letter)
                    );
                }
                let opening = processor
                    .get_token()
                    .expect("next opening delivery")
                    .expect("next opening token");
                assert_eq!(
                    opening.spelling().semantic_token(),
                    token('{', Catcode::BeginGroup)
                );
                processor
                    .back_input(opening)
                    .expect("restore measured opening");
            }
            command
                .commit_attempt_operation(warm_operation)
                .expect("warm operation commit");

            let measured_operation = command.begin_attempt_operation();
            command.profile_reset_token_collector_path_counters();
            reset_case_shift_path_counters();
            let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
            let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            {
                let mut context = universe.command_context().expect("command context");
                let mut processor = crate::test_harness::processor(
                    &mut command,
                    &mut context,
                    &mut capabilities,
                    &mut fuel,
                    &mut diagnostic_effects,
                );
                let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
                processor.shift_case(true).expect("measured case shift");
            }
            let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            let path = case_shift_path_counters();
            let collector = command.profile_token_collector_path_counters();
            assert_eq!(path.final_writes, body_len as u64);
            assert_eq!(path.table_lookups, body_len as u64);
            assert_eq!(path.source_payload_copies, 0);
            assert_eq!(path.second_traversals, 0);
            assert_eq!(collector.8, 0, "no whole-list copy");
            assert_eq!(after.calls - before.calls, 0, "warmed allocations");
            assert_eq!(
                after.requested_bytes - before.requested_bytes,
                0,
                "warmed allocation bytes"
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
                for _ in 0..body_len {
                    let shifted = processor
                        .get_token()
                        .expect("measured shifted delivery")
                        .expect("measured shifted token");
                    assert_eq!(
                        shifted.spelling().semantic_token(),
                        token('A', Catcode::Letter)
                    );
                }
                let sentinel = processor
                    .get_token()
                    .expect("sentinel delivery")
                    .expect("sentinel token");
                assert_eq!(
                    sentinel.spelling().semantic_token(),
                    token('q', Catcode::Letter)
                );
            }
            let storage = command.roots.input.replay.input_builder_storage_counts();
            assert_eq!(storage.0, 0, "no unfinished builder");
            assert_eq!(storage.1, 0, "no live escaped replay entry");
            assert_eq!(storage.2, 1, "one recycled builder lane");
            assert_eq!(storage.3, 0, "retirement truncates all active chunks");
            assert_eq!(storage.4, body_len.div_ceil(256), "chunks remain reusable");
            command
                .commit_attempt_operation(measured_operation)
                .expect("measured operation commit");
        });
    }
}

#[test]
fn case_shift_empty_and_nested_groups_replay_the_final_span_directly() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [
                token('{', Catcode::BeginGroup),
                token('}', Catcode::EndGroup),
                token('{', Catcode::BeginGroup),
                token('{', Catcode::BeginGroup),
                token('a', Catcode::Letter),
                token('}', Catcode::EndGroup),
                token('b', Catcode::Letter),
                token('}', Catcode::EndGroup),
            ],
        );
        let operation = command.begin_attempt_operation();
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

        processor.shift_case(true).expect("empty case shift");
        processor.shift_case(true).expect("nested case shift");
        let mut shifted = Vec::new();
        for _ in 0..4 {
            shifted.push(
                processor
                    .get_token()
                    .expect("shifted delivery")
                    .expect("shifted token")
                    .spelling()
                    .semantic_token(),
            );
        }
        assert_eq!(
            shifted,
            [
                token('{', Catcode::BeginGroup),
                token('A', Catcode::Letter),
                token('}', Catcode::EndGroup),
                token('B', Catcode::Letter),
            ]
        );
        drop(processor);
        command
            .commit_attempt_operation(operation)
            .expect("case-shift operation commit");
    });
}

fn assert_read_failure_is_fully_cleaned(finalize: bool) {
    crate::test_harness::with_universe(|universe| {
        universe
            .world_mut()
            .push_memory_terminal_line("read body")
            .expect("terminal line");
        let target = universe.intern("readtarget").expect("read target").symbol();
        let mut command = CommandState::default();
        let operation = command.begin_attempt_operation();
        let before = command.attempt.arena().mark();
        command.alignment.align_state = 37;
        if finalize {
            command
                .attempt
                .arena_mut()
                .inject_definition_finalization_failure();
        } else {
            command
                .attempt
                .arena_mut()
                .inject_definition_builder_allocation_failure();
        }
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let before_meaning = context.meaning(target);
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        assert!(processor.read_toks(16, target, finalize).is_err());
        assert_eq!(processor.command.alignment.align_state, 37);
        assert!(processor.command.scanner.is_quiescent());
        assert_eq!(processor.command.attempt.arena().mark(), before);
        assert_eq!(processor.state.meaning(target), before_meaning);
        drop(processor);
        command
            .rollback_attempt_operation(operation)
            .expect("failed read leaves an exact operation scope");
        assert!(command.attempt.is_empty());
    });
}

fn assert_suspended_scan_store_failure_is_transactional(
    failure: crate::execution_scratch::InjectedScannerFrameStoreFailure,
) {
    crate::test_harness::with_universe(|universe| {
        let file_size = universe.intern("filesize").expect("file-size primitive");
        universe
            .assign_meaning(
                file_size,
                MeaningWord::from_static(Meaning::ExpandablePrimitive(
                    ExpandablePrimitive::FileSize,
                )),
                AssignmentScope::Global,
            )
            .expect("file-size meaning");
        let mut command = CommandState::default();
        let operation = command.begin_attempt_operation();
        let before = command.attempt.arena().mark();
        command.scratch.inject_scanner_frame_store_failure(failure);
        crate::test_harness::push(
            &mut command,
            [
                token('{', Catcode::BeginGroup),
                Token::Cs(file_size.symbol()),
                token('{', Catcode::BeginGroup),
                token('x', Catcode::Letter),
                token('}', Catcode::EndGroup),
                token('}', Catcode::EndGroup),
            ],
        );
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

        assert!(matches!(
            processor.scan_toks_buffers(ScanToksMode::General { expanded: true }),
            Err(crate::CommandError::Fatal(_))
        ));
        assert!(processor.command.scanner.is_quiescent());
        assert!(processor.scanner_resume.is_none());
        assert_eq!(processor.command.attempt.arena().mark(), before);
        let (slots, free, live) = processor.command.scratch.scanner_resume_storage_counts();
        assert_eq!(live, 0);
        assert_eq!(free, slots);
        drop(processor);
        command
            .rollback_attempt_operation(operation)
            .expect("failed scanner publication leaves the operation exact");
        assert!(command.attempt.is_empty());
    });
}

#[test]
fn suspended_scan_store_failures_restore_all_moved_owners() {
    use crate::execution_scratch::InjectedScannerFrameStoreFailure as Failure;

    for failure in [Failure::Allocation, Failure::Capacity, Failure::Serial] {
        assert_suspended_scan_store_failure_is_transactional(failure);
    }
}

#[test]
fn suspended_scan_publication_collision_restores_the_displaced_key() {
    crate::test_harness::with_universe(|universe| {
        let file_size = universe.intern("filesize").expect("file-size primitive");
        universe
            .assign_meaning(
                file_size,
                MeaningWord::from_static(Meaning::ExpandablePrimitive(
                    ExpandablePrimitive::FileSize,
                )),
                AssignmentScope::Global,
            )
            .expect("file-size meaning");
        let mut command = CommandState::default();
        let operation = command.begin_attempt_operation();
        let before = command.attempt.arena().mark();
        command.scratch.inject_scan_toks_publication_collision();
        crate::test_harness::push(
            &mut command,
            [
                token('{', Catcode::BeginGroup),
                Token::Cs(file_size.symbol()),
                token('{', Catcode::BeginGroup),
                token('x', Catcode::Letter),
                token('}', Catcode::EndGroup),
                token('}', Catcode::EndGroup),
            ],
        );
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

        assert!(matches!(
            processor.scan_toks_buffers(ScanToksMode::General { expanded: true }),
            Err(crate::CommandError::InputInvariant(_))
        ));
        assert!(processor.command.scanner.is_quiescent());
        assert_eq!(processor.command.attempt.arena().mark(), before);
        assert!(
            processor
                .scanner_resume
                .take()
                .is_some_and(|key| key.is_injected_scan_toks_publication_collision())
        );
        let (slots, free, live) = processor.command.scratch.scanner_resume_storage_counts();
        assert_eq!(live, 0);
        assert_eq!(free, slots);
        drop(processor);
        command
            .rollback_attempt_operation(operation)
            .expect("collision cleanup leaves the operation exact");
        assert!(command.attempt.is_empty());
    });
}

#[test]
fn read_builder_setup_failure_restores_scanner_alignment_and_attempt_scope() {
    assert_read_failure_is_fully_cleaned(false);
}

#[test]
fn read_builder_finalize_failure_restores_scanner_alignment_and_attempt_scope() {
    assert_read_failure_is_fully_cleaned(true);
}

#[test]
fn balanced_collection_freezes_nested_tokens_in_the_attempt_arena() {
    crate::test_harness::with_universe(|universe| {
        reset_runaway_render_count();
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                token('{', Catcode::BeginGroup),
                token('a', Catcode::Letter),
                token('{', Catcode::BeginGroup),
                token('b', Catcode::Letter),
                token('}', Catcode::EndGroup),
                token('}', Catcode::EndGroup),
                token('X', Catcode::Letter),
            ],
        );
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
        let scanned = processor
            .scan_toks(ScanToksMode::General { expanded: false })
            .expect("balanced scan");
        assert_eq!(runaway_render_count(), 0);
        assert_eq!(
            processor
                .command
                .attempt_token_words(scanned.replacement_text)
                .expect("attempt words")
                .iter()
                .map(|word| word.semantic_token())
                .collect::<Vec<_>>(),
            [
                token('a', Catcode::Letter),
                token('{', Catcode::BeginGroup),
                token('b', Catcode::Letter),
                token('}', Catcode::EndGroup),
            ]
        );
        let mut destination = None;
        assert_eq!(
            processor
                .get_x_token_into(&mut destination)
                .expect("following delivery"),
            DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .take()
                .expect("following token")
                .spelling()
                .semantic_token(),
            token('X', Catcode::Letter)
        );
    });
}

fn runaway_partial(diagnostics: &[CommandSemanticDiagnostic]) -> Option<(&'static str, &str)> {
    diagnostics.iter().find_map(|diagnostic| match diagnostic {
        CommandSemanticDiagnostic::Recoverable {
            runaway: Some(runaway),
            ..
        } => Some((runaway.heading, runaway.partial.as_str())),
        _ => None,
    })
}

#[test]
fn eof_recovery_renders_balanced_text_only_after_the_runaway_exists() {
    crate::test_harness::with_universe(|universe| {
        reset_runaway_render_count();
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [
                token('{', Catcode::BeginGroup),
                token('a', Catcode::Letter),
                token('b', Catcode::Letter),
                token('c', Catcode::Letter),
            ],
        );
        let operation = command.begin_attempt_operation();
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

        let scanned = processor
            .scan_toks(ScanToksMode::General { expanded: false })
            .expect("EOF recovery closes the balanced scan");
        assert_eq!(
            processor
                .attempt_words(scanned.replacement_text)
                .expect("replacement text")
                .iter()
                .map(|word| word.semantic_token())
                .collect::<Vec<_>>(),
            [
                token('a', Catcode::Letter),
                token('b', Catcode::Letter),
                token('c', Catcode::Letter),
            ]
        );
        assert_eq!(
            runaway_partial(&processor.command.semantic_diagnostics),
            Some(("Runaway text?", "abc"))
        );
        assert_eq!(runaway_render_count(), 1);
        drop(processor);
        command
            .commit_attempt_operation(operation)
            .expect("operation commit");
    });
}

#[test]
fn eof_recovery_streams_macro_parameter_and_replacement_slices_once() {
    crate::test_harness::with_universe(|universe| {
        reset_runaway_render_count();
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [
                token('#', Catcode::Parameter),
                token('1', Catcode::Other),
                token('{', Catcode::BeginGroup),
                token('a', Catcode::Letter),
                token('#', Catcode::Parameter),
                token('1', Catcode::Other),
            ],
        );
        let operation = command.begin_attempt_operation();
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

        processor
            .scan_toks_buffers(ScanToksMode::MacroDefinition {
                expanded: false,
                global: false,
            })
            .expect("EOF recovery closes the definition");
        assert_eq!(
            runaway_partial(&processor.command.semantic_diagnostics),
            Some(("Runaway definition?", "#1->a#1"))
        );
        assert_eq!(runaway_render_count(), 1);
        drop(processor);
        command
            .commit_attempt_operation(operation)
            .expect("operation commit");
    });
}

#[test]
fn outer_recovery_renders_only_the_collected_balanced_prefix() {
    crate::test_harness::with_universe(|universe| {
        reset_runaway_render_count();
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("outer macro definition");
        let outer = universe.intern("outermacro").expect("outer macro name");
        universe
            .assign_meaning(
                outer,
                MeaningWord::macro_definition(MeaningFlags::OUTER, definition),
                AssignmentScope::Global,
            )
            .expect("outer macro meaning");
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [
                token('{', Catcode::BeginGroup),
                token('a', Catcode::Letter),
                Token::Cs(outer.symbol()),
            ],
        );
        let operation = command.begin_attempt_operation();
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

        processor
            .scan_toks(ScanToksMode::General { expanded: false })
            .expect("outer recovery closes the scan");
        assert_eq!(
            runaway_partial(&processor.command.semantic_diagnostics),
            Some(("Runaway text?", "a"))
        );
        assert_eq!(runaway_render_count(), 1);
        drop(processor);
        command
            .commit_attempt_operation(operation)
            .expect("operation commit");
    });
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_successful_scan_builds_no_runaway_and_allocates_nothing() {
    crate::test_harness::with_universe(|universe| {
        reset_runaway_render_count();
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [
                token('{', Catcode::BeginGroup),
                token('a', Catcode::Letter),
                token('}', Catcode::EndGroup),
                token('{', Catcode::BeginGroup),
                token('a', Catcode::Letter),
                token('}', Catcode::EndGroup),
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();

        let warm_operation = command.begin_attempt_operation();
        {
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            processor
                .scan_toks(ScanToksMode::General { expanded: false })
                .expect("warm scan");
        }
        command
            .commit_attempt_operation(warm_operation)
            .expect("warm operation commit");

        let measured_operation = command.begin_attempt_operation();
        let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
        let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        {
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            processor
                .scan_toks(ScanToksMode::General { expanded: false })
                .expect("measured scan");
        }
        let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        assert_eq!(after.calls - before.calls, 0);
        assert_eq!(after.requested_bytes - before.requested_bytes, 0);
        assert_eq!(runaway_render_count(), 0);
        command
            .commit_attempt_operation(measured_operation)
            .expect("measured operation commit");
    });
}

#[cfg(feature = "profiling")]
fn mixed_scan_workload(rounds: usize) -> Vec<Token> {
    let mut input = Vec::with_capacity(rounds * 15);
    for _ in 0..rounds {
        input.extend([
            token('{', Catcode::BeginGroup),
            token('u', Catcode::Letter),
            token('}', Catcode::EndGroup),
            token('{', Catcode::BeginGroup),
            token('e', Catcode::Letter),
            token('}', Catcode::EndGroup),
            token('#', Catcode::Parameter),
            token('1', Catcode::Other),
            token('{', Catcode::BeginGroup),
            token('#', Catcode::Parameter),
            token('1', Catcode::Other),
            token('}', Catcode::EndGroup),
            token('{', Catcode::BeginGroup),
            token('w', Catcode::Letter),
            token('}', Catcode::EndGroup),
        ]);
    }
    input
}

#[cfg(feature = "profiling")]
fn run_mixed_scan_workload<G>(
    processor: &mut crate::CommandProcessor<'_, '_, G>,
    owner: tex_state::interner::Symbol,
    rounds: usize,
) {
    for _ in 0..rounds {
        processor
            .scan_toks(ScanToksMode::General { expanded: false })
            .expect("unexpanded general scan");
        processor
            .scan_toks(ScanToksMode::General { expanded: true })
            .expect("expanded general scan");
        processor
            .scan_toks_buffers(ScanToksMode::MacroDefinition {
                expanded: false,
                global: false,
            })
            .expect("definition scan");
        processor
            .scan_toks(ScanToksMode::GeneralFor {
                expanded: false,
                owner,
            })
            .expect("write-like owned scan");
    }
}

#[cfg(feature = "profiling")]
#[test]
fn one_and_4096_mixed_scans_use_one_resident_collector_write_per_token() {
    for rounds in [1, 4096] {
        crate::test_harness::with_universe(|universe| {
            let owner = universe.intern("mixedscanowner").expect("owner").symbol();
            let mut command = CommandState::default();
            let mut capabilities = CommandHostCapabilities::default();
            let mut fuel = crate::CommandFuelLedger::default();
            let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();

            crate::test_harness::push(&mut command, mixed_scan_workload(rounds));
            let warm_operation = command.begin_attempt_operation();
            {
                let mut context = universe.command_context().expect("command context");
                let mut processor = crate::test_harness::processor(
                    &mut command,
                    &mut context,
                    &mut capabilities,
                    &mut fuel,
                    &mut diagnostic_effects,
                );
                run_mixed_scan_workload(&mut processor, owner, rounds);
            }
            command
                .rollback_attempt_operation(warm_operation)
                .expect("warm scanner scratch rollback");

            crate::test_harness::push(&mut command, mixed_scan_workload(rounds));
            let measured_operation = command.begin_attempt_operation();
            command.profile_reset_token_collector_path_counters();
            universe
                .command_context()
                .expect("definition arena reserve context")
                .profile_reserve_definition_arena(rounds, 4 * rounds)
                .expect("warm definition arena capacity");
            let delivery_owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
            let scratch_owner = tex_state::measurement::HotCoreAllocationOwner::AttemptScratch;
            let delivery_before =
                tex_state::measurement::hot_core_thread_allocation_measurement(delivery_owner);
            let scratch_before =
                tex_state::measurement::hot_core_thread_allocation_measurement(scratch_owner);
            {
                let mut context = universe.command_context().expect("command context");
                let mut processor = crate::test_harness::processor(
                    &mut command,
                    &mut context,
                    &mut capabilities,
                    &mut fuel,
                    &mut diagnostic_effects,
                );
                let _scope = tex_state::measurement::hot_core_allocation_scope(delivery_owner);
                run_mixed_scan_workload(&mut processor, owner, rounds);
            }
            let delivery_after =
                tex_state::measurement::hot_core_thread_allocation_measurement(delivery_owner);
            let scratch_after =
                tex_state::measurement::hot_core_thread_allocation_measurement(scratch_owner);
            let counters = command.profile_token_collector_path_counters();

            assert_eq!(counters.0, 4 * rounds as u64, "collectors");
            assert_eq!(counters.1, 12 * rounds as u64, "raw classifications");
            assert_eq!(counters.2, 5 * rounds as u64, "appends");
            assert_eq!(counters.3, 7 * rounds as u64, "in-place state updates");
            assert_eq!(counters.4, counters.0, "one monotonic phase transition");
            assert_eq!(counters.5, 0, "duplicate phase dispatches");
            assert_eq!(counters.6, 0, "fact rescans");
            assert_eq!(counters.7, counters.0, "one final settlement");
            assert_eq!(counters.8, 0, "whole token-list copies");
            assert_eq!(counters.9, 0, "whole command copies");
            assert_eq!(counters.10, 0, "whole frame copies");
            assert_eq!(delivery_after.calls - delivery_before.calls, 0);
            assert_eq!(
                delivery_after.requested_bytes - delivery_before.requested_bytes,
                0
            );
            assert_eq!(scratch_after.calls - scratch_before.calls, 0);
            assert_eq!(
                scratch_after.requested_bytes - scratch_before.requested_bytes,
                0
            );

            command
                .rollback_attempt_operation(measured_operation)
                .expect("measured scanner scratch rollback");
        });
    }
}

#[test]
fn expanded_collection_keeps_its_builder_live_across_nested_macro_retirement() {
    crate::test_harness::with_universe(|universe| {
        let replacement = token('a', Catcode::Letter);
        let definition = universe
            .allocate_definition(&[], &[TokenWord::pack(replacement)])
            .expect("macro definition");
        let symbol = universe.intern("scanmacro").expect("macro name");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
                AssignmentScope::Global,
            )
            .expect("macro meaning");
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                token('{', Catcode::BeginGroup),
                Token::Cs(symbol.symbol()),
                token('}', Catcode::EndGroup),
                token('X', Catcode::Letter),
            ],
        );
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

        let scanned = processor
            .scan_toks(ScanToksMode::General { expanded: true })
            .expect("expanded scan");
        assert_eq!(
            processor
                .command
                .attempt_token_words(scanned.replacement_text)
                .expect("replacement survives nested retirement")
                .iter()
                .map(|word| word.semantic_token())
                .collect::<Vec<_>>(),
            [replacement]
        );
        let mut destination = None;
        assert_eq!(
            processor
                .get_x_token_into(&mut destination)
                .expect("following delivery"),
            DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .take()
                .expect("following token")
                .spelling()
                .semantic_token(),
            token('X', Catcode::Letter)
        );
    });
}

#[test]
fn expanded_scan_adopts_unexpanded_child_tokens_without_recursive_expansion() {
    crate::test_harness::with_universe(|universe| {
        let unexpanded = universe.intern("unexpanded").expect("unexpanded primitive");
        universe
            .assign_meaning(
                unexpanded,
                MeaningWord::from_static(Meaning::ExpandablePrimitive(
                    ExpandablePrimitive::Unexpanded,
                )),
                AssignmentScope::Global,
            )
            .expect("unexpanded meaning");
        let replacement = token('y', Catcode::Letter);
        let definition = universe
            .allocate_definition(&[], &[TokenWord::pack(replacement)])
            .expect("nested macro definition");
        let nested = universe.intern("unexpandedpayload").expect("nested macro");
        universe
            .assign_meaning(
                nested,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
                AssignmentScope::Global,
            )
            .expect("nested macro meaning");

        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                token('{', Catcode::BeginGroup),
                Token::Cs(unexpanded.symbol()),
                token('{', Catcode::BeginGroup),
                Token::Cs(nested.symbol()),
                token('{', Catcode::BeginGroup),
                token('z', Catcode::Letter),
                token('}', Catcode::EndGroup),
                token('}', Catcode::EndGroup),
                token('}', Catcode::EndGroup),
            ],
        );
        command.profile_reset_token_collector_path_counters();
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

        let scanned = processor
            .scan_toks(ScanToksMode::General { expanded: true })
            .expect("expanded parent scan");
        let words = processor
            .attempt_words(scanned.replacement_text)
            .expect("adopted parent words");
        assert_eq!(
            words
                .iter()
                .map(|word| word.semantic_token())
                .collect::<Vec<_>>(),
            [
                Token::Cs(nested.symbol()),
                token('{', Catcode::BeginGroup),
                token('z', Catcode::Letter),
                token('}', Catcode::EndGroup),
            ]
        );
        let counters = processor.command.profile_token_collector_path_counters();
        assert_eq!(counters.0, 2, "parent and child collectors");
        assert_eq!(counters.6, 0, "no classified-token rescan");
        assert_eq!(counters.8, 0, "no whole-list copy");
    });
}

#[test]
fn macro_definition_scan_shares_one_checked_builder_coordinate() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                token('#', Catcode::Parameter),
                token('1', Catcode::Other),
                token('{', Catcode::BeginGroup),
                token('#', Catcode::Parameter),
                token('1', Catcode::Other),
                token('}', Catcode::EndGroup),
            ],
        );
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
        let scanned = processor
            .scan_toks_buffers(ScanToksMode::MacroDefinition {
                expanded: false,
                global: false,
            })
            .expect("definition scan");
        assert!(!scanned.malformed_parameter);
        let definition = scanned.definition().expect("definition builder result");
        let definition = processor.state.definition(definition);
        assert!(!definition.parameter_text().is_empty());
        assert_eq!(
            definition
                .replacement_text()
                .last()
                .map(|word| word.semantic_token()),
            Some(Token::Param(1))
        );
    });
}

#[test]
fn macro_definition_hash_brace_shares_one_checked_builder_boundary() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                token('#', Catcode::Parameter),
                token('{', Catcode::BeginGroup),
                token('x', Catcode::Letter),
                token('}', Catcode::EndGroup),
            ],
        );
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

        let scanned = processor
            .scan_toks_buffers(ScanToksMode::MacroDefinition {
                expanded: false,
                global: false,
            })
            .expect("hash-brace definition scan");
        let definition = scanned.definition().expect("definition builder result");
        let definition = processor.state.definition(definition);
        assert_eq!(
            definition
                .parameter_text()
                .iter()
                .map(|word| word.semantic_token())
                .collect::<Vec<_>>(),
            [token('{', Catcode::BeginGroup)]
        );
        assert_eq!(
            definition
                .replacement_text()
                .iter()
                .map(|word| word.semantic_token())
                .collect::<Vec<_>>(),
            [token('x', Catcode::Letter), token('{', Catcode::BeginGroup),]
        );
    });
}

#[test]
fn expanded_macro_definition_keeps_its_builder_across_nested_macro_retirement() {
    crate::test_harness::with_universe(|universe| {
        let expansion = token('a', Catcode::Letter);
        let nested_definition = universe
            .allocate_definition(&[], &[TokenWord::pack(expansion)])
            .expect("nested macro definition");
        let nested = universe.intern("nesteddefscan").expect("nested macro name");
        universe
            .assign_meaning(
                nested,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, nested_definition),
                AssignmentScope::Global,
            )
            .expect("nested macro meaning");
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                token('{', Catcode::BeginGroup),
                Token::Cs(nested.symbol()),
                token('}', Catcode::EndGroup),
            ],
        );
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

        let scanned = processor
            .scan_toks_buffers(ScanToksMode::MacroDefinition {
                expanded: true,
                global: false,
            })
            .expect("expanded definition scan");
        let definition = scanned.definition().expect("definition builder result");
        let definition = processor.state.definition(definition);
        assert!(definition.parameter_text().is_empty());
        assert_eq!(definition.replacement_text(), [TokenWord::pack(expansion)]);
    });
}
