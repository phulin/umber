use tex_state::env::AssignmentScope;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags, MeaningWord};
use tex_state::token::{Catcode, OriginId, Token, TokenWord};

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
                    let shifted = crate::test_harness::expect_token_command(&mut processor);
                    assert_eq!(
                        shifted.spelling().semantic_token(),
                        token('A', Catcode::Letter)
                    );
                }
                let opening = crate::test_harness::expect_token_command(&mut processor);
                assert_eq!(
                    opening.spelling().semantic_token(),
                    token('{', Catcode::BeginGroup)
                );
                processor
                    .back_input(opening)
                    .expect("restore measured opening");
            }
            assert!(command.attempt_is_clear());

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
                    let shifted = crate::test_harness::expect_token_command(&mut processor);
                    assert_eq!(
                        shifted.spelling().semantic_token(),
                        token('A', Catcode::Letter)
                    );
                }
                let sentinel = crate::test_harness::expect_token_command(&mut processor);
                assert_eq!(
                    sentinel.spelling().semantic_token(),
                    token('q', Catcode::Letter)
                );
            }
            assert!(
                command.scratch.is_quiescent(),
                "no unfinished scanner builder"
            );
            assert!(!command.roots.input.levels.arguments.is_matching());
            assert!(command.attempt_is_clear());
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
                crate::test_harness::expect_token_command(&mut processor)
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
        assert!(command.attempt_is_clear());
    });
}

fn assert_read_failure_is_fully_cleaned(raw_catcodes: bool) {
    crate::test_harness::with_universe(|universe| {
        universe
            .world_mut()
            .push_memory_terminal_line("read body")
            .expect("terminal line");
        let target = universe.intern("readtarget").expect("read target").symbol();
        let mut command = CommandState::default();
        let before = command.attempt.mark();
        command.alignment.align_state = 37;
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(1).expect("bounded read fuel");
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

        assert!(processor.read_toks(16, target, raw_catcodes).is_err());
        assert_eq!(processor.command.alignment.align_state, 37);
        assert!(processor.command.scanner.is_quiescent());
        assert_eq!(processor.command.attempt.mark(), before);
        assert_eq!(processor.state.meaning(target), before_meaning);
        drop(processor);
        let build = context
            .begin_definition_build(OriginId::UNKNOWN)
            .expect("failed read returned its native writer");
        context.abort_definition_build(build);
        assert!(command.attempt_is_clear());
        assert!(command.attempt.is_clear());
    });
}

#[test]
fn tokenized_read_fuel_failure_restores_scanner_alignment_and_native_writer() {
    assert_read_failure_is_fully_cleaned(false);
}

#[test]
fn verbatim_read_fuel_failure_restores_scanner_alignment_and_native_writer() {
    assert_read_failure_is_fully_cleaned(true);
}

#[test]
fn balanced_collection_freezes_nested_tokens_in_the_attempt_arena() {
    crate::test_harness::with_universe(|universe| {
        reset_runaway_render_count();
        let mut command = CommandState::default();
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
                .attempt_token_words(&scanned.replacement_text)
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
                .attempt_words(&scanned.replacement_text)
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
        processor
            .command
            .attempt
            .discard_token_list(scanned.replacement_text)
            .expect("test consumes scanner result");
        drop(processor);
        assert!(command.attempt_is_clear());
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
            .scan_toks_buffers(ScanToksMode::MacroDefinition { expanded: false })
            .expect("EOF recovery closes the definition");
        assert_eq!(
            runaway_partial(&processor.command.semantic_diagnostics),
            Some(("Runaway definition?", "#1->a#1"))
        );
        assert_eq!(runaway_render_count(), 1);
        drop(processor);
        assert!(command.attempt_is_clear());
    });
}

#[test]
fn outer_recovery_renders_only_the_collected_balanced_prefix() {
    crate::test_harness::with_universe(|universe| {
        reset_runaway_render_count();
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("outer macro definition");
        let definition_publication = definition;
        let outer = universe.intern("outermacro").expect("outer macro name");
        universe
            .assign_meaning(
                outer,
                definition_publication.meaning(MeaningFlags::OUTER),
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
            .expect("outer recovery closes the scan");
        assert_eq!(
            runaway_partial(&processor.command.semantic_diagnostics),
            Some(("Runaway text?", "a"))
        );
        assert_eq!(runaway_render_count(), 1);
        processor
            .command
            .attempt
            .discard_token_list(scanned.replacement_text)
            .expect("test consumes scanner result");
        drop(processor);
        assert!(command.attempt_is_clear());
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

        {
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
                .expect("warm scan");
            processor
                .command
                .discard_attempt_tokens(scanned.replacement_text)
                .expect("fixture consumes scanner result");
        }
        assert!(command.attempt_is_clear());

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
            let scanned = processor
                .scan_toks(ScanToksMode::General { expanded: false })
                .expect("measured scan");
            processor
                .command
                .discard_attempt_tokens(scanned.replacement_text)
                .expect("fixture consumes scanner result");
        }
        let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        assert_eq!(after.calls - before.calls, 0);
        assert_eq!(after.requested_bytes - before.requested_bytes, 0);
        assert_eq!(runaway_render_count(), 0);
        assert!(command.attempt_is_clear());
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
        let scanned = processor
            .scan_toks(ScanToksMode::General { expanded: false })
            .expect("unexpanded general scan");
        processor
            .command
            .discard_attempt_tokens(scanned.replacement_text)
            .expect("fixture consumes scanner result");
        let scanned = processor
            .scan_toks(ScanToksMode::General { expanded: true })
            .expect("expanded general scan");
        processor
            .command
            .discard_attempt_tokens(scanned.replacement_text)
            .expect("fixture consumes scanner result");
        processor
            .scan_toks_buffers(ScanToksMode::MacroDefinition { expanded: false })
            .expect("definition scan");
        let scanned = processor
            .scan_toks(ScanToksMode::GeneralFor {
                expanded: false,
                owner,
            })
            .expect("write-like owned scan");
        processor
            .command
            .discard_attempt_tokens(scanned.replacement_text)
            .expect("fixture consumes scanner result");
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
            assert!(command.attempt_is_clear());

            crate::test_harness::push(&mut command, mixed_scan_workload(rounds));
            command.profile_reset_token_collector_path_counters();
            universe
                .command_context()
                .expect("definition arena reserve context")
                .profile_reserve_definition_arena(rounds)
                .expect("warm definition arena capacity");
            let delivery_owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
            let scratch_owner = tex_state::measurement::HotCoreAllocationOwner::AttemptScratch;
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
            assert_eq!(counters.7, counters.0, "one final publication");
            assert_eq!(counters.8, 0, "whole token-list copies");
            assert_eq!(counters.9, 0, "whole command copies");
            assert_eq!(counters.10, 0, "whole frame copies");
            // Sealing publishes independently owned immutable definition
            // payloads. Those allocations are output storage, not collector
            // scratch or an immediately consumed processing product.
            assert_eq!(scratch_after.calls - scratch_before.calls, 0);
            assert_eq!(
                scratch_after.requested_bytes - scratch_before.requested_bytes,
                0
            );

            assert!(command.attempt_is_clear());
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
        let definition_publication = definition;
        let symbol = universe.intern("scanmacro").expect("macro name");
        universe
            .assign_meaning(
                symbol,
                definition_publication.meaning(MeaningFlags::EMPTY),
                AssignmentScope::Global,
            )
            .expect("macro meaning");
        let mut command = CommandState::default();
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
                .attempt_token_words(&scanned.replacement_text)
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
        let definition_publication = definition;
        let nested = universe.intern("unexpandedpayload").expect("nested macro");
        universe
            .assign_meaning(
                nested,
                definition_publication.meaning(MeaningFlags::EMPTY),
                AssignmentScope::Global,
            )
            .expect("nested macro meaning");

        let mut command = CommandState::default();
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
            .attempt_words(&scanned.replacement_text)
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
            .scan_toks_buffers(ScanToksMode::MacroDefinition { expanded: false })
            .expect("definition scan");
        assert!(!scanned.malformed_parameter);
        let definition = scanned.definition().expect("definition builder result");
        let definition = processor.state.definition(definition.reference());
        assert!(!definition.parameter_text().is_empty());
        assert_eq!(
            definition
                .replacement_text()
                .iter()
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
            .scan_toks_buffers(ScanToksMode::MacroDefinition { expanded: false })
            .expect("hash-brace definition scan");
        let definition = scanned.definition().expect("definition builder result");
        let definition = processor.state.definition(definition.reference());
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
        let nested_definition_publication = nested_definition;
        let nested = universe.intern("nesteddefscan").expect("nested macro name");
        universe
            .assign_meaning(
                nested,
                nested_definition_publication.meaning(MeaningFlags::EMPTY),
                AssignmentScope::Global,
            )
            .expect("nested macro meaning");
        let mut command = CommandState::default();
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
            .scan_toks_buffers(ScanToksMode::MacroDefinition { expanded: true })
            .expect("expanded definition scan");
        let definition = scanned.definition().expect("definition builder result");
        let definition = processor.state.definition(definition.reference());
        assert!(definition.parameter_text().is_empty());
        assert_eq!(definition.replacement_text(), [TokenWord::pack(expansion)]);
    });
}

#[test]
fn expanded_definition_activates_macro_runs_without_temporary_commands() {
    struct Observer;
    impl crate::CommandObserver for Observer {
        fn committed(&mut self, _: crate::CommandObservation) {}
    }
    for observed in [false, true] {
        for length in [1, 4096] {
            crate::test_harness::with_universe(|universe| {
                let publication = universe
                    .allocate_definition(&[], &[])
                    .expect("empty definition");
                let symbol = universe.intern("empty_in_definition").expect("symbol");
                universe
                    .assign_meaning(
                        symbol,
                        publication.meaning(MeaningFlags::EMPTY),
                        AssignmentScope::Global,
                    )
                    .expect("empty macro");
                let mut command = CommandState::default();
                crate::test_harness::push(
                    &mut command,
                    std::iter::once(token('{', Catcode::BeginGroup))
                        .chain(std::iter::repeat_n(Token::Cs(symbol.symbol()), length))
                        .chain(std::iter::once(token('}', Catcode::EndGroup))),
                );
                let mut capabilities = CommandHostCapabilities::default();
                let mut fuel = crate::CommandFuelLedger::default();
                let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
                let mut observer = Observer;
                let mut context = universe.command_context().expect("context");
                let processor = crate::test_harness::processor(
                    &mut command,
                    &mut context,
                    &mut capabilities,
                    &mut fuel,
                    &mut effects,
                );
                let mut processor = if observed {
                    processor.with_observer(&mut observer)
                } else {
                    processor
                };
                let before = crate::command::command_ownership_counters();
                let scanned = processor
                    .scan_toks_buffers(ScanToksMode::MacroDefinition { expanded: true })
                    .expect("expanded definition");
                let after = crate::command::command_ownership_counters();
                let definition = processor
                    .state
                    .definition(scanned.definition().expect("definition").reference());
                assert!(definition.replacement_text().is_empty());
                assert_eq!(
                    after.resolved_writes - before.resolved_writes,
                    if observed { length as u64 + 2 } else { 1 },
                    "only the raw opening scan needs a command; native replacement consumption does not",
                );
                assert_eq!(processor.fuel.burned(), length as u64 + 2);
            });
        }
    }
}

#[test]
fn ordinary_definition_body_constructs_no_per_token_command() {
    for length in [1, 4096] {
        crate::test_harness::with_universe(|universe| {
            let symbol = universe.intern("retained_in_definition").expect("symbol");
            let mut command = CommandState::default();
            let body = [
                token('a', Catcode::Letter),
                Token::Cs(symbol.symbol()),
                token('{', Catcode::BeginGroup),
                token('}', Catcode::EndGroup),
            ];
            let input = std::iter::once(token('{', Catcode::BeginGroup))
                .chain(body.into_iter().cycle().take(length))
                .chain(std::iter::once(token('}', Catcode::EndGroup)));
            crate::test_harness::push(&mut command, input);
            let mut capabilities = CommandHostCapabilities::default();
            let mut fuel = crate::CommandFuelLedger::default();
            let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut context = universe.command_context().expect("context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut effects,
            );
            let before = crate::command::command_ownership_counters();
            let scanned = processor
                .scan_toks_buffers(ScanToksMode::MacroDefinition { expanded: false })
                .expect("definition");
            let after = crate::command::command_ownership_counters();
            let definition = processor
                .state
                .definition(scanned.definition().expect("definition").reference());
            assert_eq!(definition.replacement_text().len(), length);
            assert_eq!(
                after.resolved_writes - before.resolved_writes,
                1,
                "only the opening delimiter uses command delivery, independent of body length"
            );
            assert_eq!(processor.fuel.burned(), length as u64 + 2);
        });
    }
}

#[test]
fn direct_definition_collection_matches_observed_recovery_at_every_fuel_cut() {
    struct Observer;
    impl crate::CommandObserver for Observer {
        fn committed(&mut self, _: crate::CommandObservation) {}
    }
    let run = |observed, limit| {
        crate::test_harness::with_universe(|universe| {
            let retained = universe.intern("retained").expect("symbol");
            let mut command = CommandState::default();
            crate::test_harness::push(
                &mut command,
                [
                    token('#', Catcode::Parameter),
                    token('1', Catcode::Other),
                    token('{', Catcode::BeginGroup),
                    token('{', Catcode::BeginGroup),
                    Token::Cs(retained.symbol()),
                    token('}', Catcode::EndGroup),
                    token('#', Catcode::Parameter),
                    token('1', Catcode::Other),
                    token('#', Catcode::Parameter),
                    token('#', Catcode::Parameter),
                    token('#', Catcode::Parameter),
                    token('2', Catcode::Other), // Illegal parameter: back up this token.
                    token('}', Catcode::EndGroup),
                ],
            );
            let mut capabilities = CommandHostCapabilities::default();
            let mut fuel = crate::CommandFuelLedger::new(limit).expect("fuel");
            let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut observer = Observer;
            let mut context = universe.command_context().expect("context");
            let processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut effects,
            );
            let mut processor = if observed {
                processor.with_observer(&mut observer)
            } else {
                processor
            };
            let result =
                processor.scan_toks_buffers(ScanToksMode::MacroDefinition { expanded: false });
            let body = match result {
                Ok(scanned) => Some(
                    processor
                        .state
                        .definition(scanned.definition().expect("definition").reference())
                        .replacement_text()
                        .iter()
                        .map(|word| word.semantic_token())
                        .collect::<Vec<_>>(),
                ),
                Err(crate::CommandError::FuelExhausted(_)) => None,
                Err(error) => panic!("unexpected scan failure: {error:?}"),
            };
            assert!(matches!(
                processor.command.scanner,
                crate::processor::status::ScannerStatus::Normal
            ));
            (
                body,
                processor.fuel.burned(),
                processor.command.semantic_diagnostics.len(),
                processor.command.roots.alignment.align_state,
            )
        })
    };
    let complete = run(false, 100);
    assert!(complete.0.is_some());
    assert_eq!(
        complete.1, 14,
        "illegal parameter is backed up and read again"
    );
    assert!(
        complete
            .0
            .as_ref()
            .expect("body")
            .ends_with(&[token('#', Catcode::Parameter), token('2', Catcode::Other)])
    );
    assert_eq!(run(true, 100), complete);
    for limit in 1..=complete.1 + 1 {
        assert_eq!(run(false, limit), run(true, limit), "fuel {limit}");
    }
}

/// Unobserved definitions consume plain resident spans as batched runs. The
/// observed scalar path is the reference: both must agree on the stored body,
/// fuel, diagnostics, and `align_state` at every fuel cut, whether the body
/// comes from a macro body or from a substituted macro argument.
#[test]
fn batched_definition_runs_match_scalar_collection_from_resident_sources() {
    struct Observer;
    impl crate::CommandObserver for Observer {
        fn committed(&mut self, _: crate::CommandObservation) {}
    }
    fn install<G>(
        universe: &mut tex_state::Universe<G>,
        name: &str,
        parameters: &[Token],
        replacement: &[Token],
    ) -> Token {
        let pack = |tokens: &[Token]| {
            tokens
                .iter()
                .copied()
                .map(TokenWord::pack)
                .collect::<Vec<_>>()
        };
        let definition = universe
            .allocate_definition(&pack(parameters), &pack(replacement))
            .expect("macro definition");
        let definition_publication = definition;
        let symbol = universe.intern(name).expect("macro name");
        universe
            .assign_meaning(
                symbol,
                definition_publication.meaning(MeaningFlags::EMPTY),
                AssignmentScope::Global,
            )
            .expect("macro meaning");
        Token::Cs(symbol.symbol())
    }
    let run = |from_argument: bool, observed: bool, limit: u64| {
        crate::test_harness::with_universe(|universe| {
            let retained = Token::Cs(universe.intern("retained").expect("symbol").symbol());
            // `x`, then the parameter text `#1`, then a body whose plain
            // spans are interrupted by nested braces, a stored `#1`, a doubled
            // `##`, and an illegal `#2` that is backed up and read again.
            let text = [
                token('x', Catcode::Letter),
                token('#', Catcode::Parameter),
                token('1', Catcode::Other),
                token('{', Catcode::BeginGroup),
                token('a', Catcode::Letter),
                token('{', Catcode::BeginGroup),
                token('b', Catcode::Letter),
                retained,
                token('}', Catcode::EndGroup),
                token('c', Catcode::Letter),
                token('#', Catcode::Parameter),
                token('1', Catcode::Other),
                token('d', Catcode::Letter),
                token('#', Catcode::Parameter),
                token('#', Catcode::Parameter),
                token('e', Catcode::Letter),
                token('#', Catcode::Parameter),
                token('2', Catcode::Other),
                token('f', Catcode::Letter),
                token('}', Catcode::EndGroup),
            ];
            let input = if from_argument {
                let carrier = install(universe, "carrier", &[Token::Param(1)], &[Token::Param(1)]);
                std::iter::once(carrier)
                    .chain(std::iter::once(token('{', Catcode::BeginGroup)))
                    .chain(text)
                    .chain(std::iter::once(token('}', Catcode::EndGroup)))
                    .collect::<Vec<_>>()
            } else {
                vec![install(universe, "carrier", &[], &text)]
            };
            let mut command = CommandState::default();
            crate::test_harness::push(&mut command, input);
            let mut capabilities = CommandHostCapabilities::default();
            let mut fuel = crate::CommandFuelLedger::new(limit).expect("fuel");
            let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut observer = Observer;
            let mut context = universe.command_context().expect("context");
            let processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut effects,
            );
            let mut processor = if observed {
                processor.with_observer(&mut observer)
            } else {
                processor
            };
            // Expanding the carrier leaves its body (or argument) resident.
            let mut destination = None;
            match processor.get_x_token_into(&mut destination) {
                Ok(_) => {}
                Err(crate::CommandError::FuelExhausted(_)) => return None,
                Err(error) => panic!("unexpected expansion failure: {error:?}"),
            }
            let result =
                processor.scan_toks_buffers(ScanToksMode::MacroDefinition { expanded: false });
            let body = match result {
                Ok(scanned) => Some(
                    processor
                        .state
                        .definition(scanned.definition().expect("definition").reference())
                        .replacement_text()
                        .iter()
                        .map(|word| word.semantic_token())
                        .collect::<Vec<_>>(),
                ),
                Err(crate::CommandError::FuelExhausted(_)) => None,
                Err(error) => panic!("unexpected scan failure: {error:?}"),
            };
            Some((
                body,
                processor.fuel.burned(),
                processor.command.semantic_diagnostics.len(),
                processor.command.roots.alignment.align_state,
            ))
        })
    };
    for from_argument in [false, true] {
        let complete = run(from_argument, false, 1_000).expect("carrier expands");
        let body = complete.0.as_ref().expect("body");
        assert_eq!(
            body.len(),
            13,
            "a{{b\\retained}}c#1d#e#2f with a converted #1: {body:?}"
        );
        assert!(body.contains(&Token::Param(1)));
        let fuel = complete.1;
        assert_eq!(run(from_argument, true, 1_000), Some(complete));
        for limit in 1..=fuel + 1 {
            assert_eq!(
                run(from_argument, false, limit),
                run(from_argument, true, limit),
                "argument source {from_argument}, fuel {limit}"
            );
        }
    }
}

#[test]
fn read_setup_failure_preserves_an_existing_native_build_and_restores_scanner() {
    crate::test_harness::with_universe(|universe| {
        let target = universe.intern("readtarget").expect("target").symbol();
        let mut command = CommandState::default();
        command.alignment.align_state = 37;
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("context");
        let existing = context
            .begin_definition_build(OriginId::UNKNOWN)
            .expect("existing build");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut effects,
        );
        assert!(processor.read_toks(16, target, false).is_err());
        assert_eq!(processor.command.alignment.align_state, 37);
        assert!(processor.command.scanner.is_quiescent());
        drop(processor);
        // Setup never owned this build, so failed-read cleanup must preserve it.
        context
            .push_definition_parameter(existing, TokenWord::pack(Token::param(1)))
            .expect("existing build remains live");
        context.abort_definition_build(existing);
    });
}
