use tex_state::env::AssignmentScope;
use tex_state::meaning::{MeaningFlags, MeaningWord};
use tex_state::token::{Catcode, Token, TokenWord};

use super::{ScanToksMode, reset_runaway_render_count, runaway_render_count};
use crate::{CommandHostCapabilities, CommandSemanticDiagnostic, CommandState, DeliveryStatus};

fn token(ch: char, cat: Catcode) -> Token {
    Token::Char { ch, cat }
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
            .scan_toks_buffers(ScanToksMode::MacroDefinition { expanded: false })
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
fn macro_definition_scan_keeps_parameter_and_replacement_lists_separate() {
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
            .scan_toks_buffers(ScanToksMode::MacroDefinition { expanded: false })
            .expect("definition scan");
        assert!(!scanned.malformed_parameter);
        let definition = scanned.definition().expect("definition builder result");
        assert!(
            !processor
                .command
                .attempt
                .arena()
                .definition_parameter_words(definition)
                .expect("parameter words")
                .is_empty()
        );
        assert_eq!(
            processor
                .command
                .attempt
                .arena()
                .definition_replacement_words(definition)
                .expect("replacement words")
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
            .scan_toks_buffers(ScanToksMode::MacroDefinition { expanded: false })
            .expect("hash-brace definition scan");
        let definition = scanned.definition().expect("definition builder result");
        assert_eq!(
            processor
                .command
                .attempt
                .arena()
                .definition_parameter_words(definition)
                .expect("parameter words")
                .iter()
                .map(|word| word.semantic_token())
                .collect::<Vec<_>>(),
            [token('{', Catcode::BeginGroup)]
        );
        assert_eq!(
            processor
                .command
                .attempt
                .arena()
                .definition_replacement_words(definition)
                .expect("replacement words")
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
            .scan_toks_buffers(ScanToksMode::MacroDefinition { expanded: true })
            .expect("expanded definition scan");
        let definition = scanned.definition().expect("definition builder result");
        assert!(
            processor
                .command
                .attempt
                .arena()
                .definition_parameter_words(definition)
                .expect("parameter words")
                .is_empty()
        );
        assert_eq!(
            processor
                .command
                .attempt
                .arena()
                .definition_replacement_words(definition)
                .expect("replacement words"),
            [TokenWord::pack(expansion)]
        );
    });
}
