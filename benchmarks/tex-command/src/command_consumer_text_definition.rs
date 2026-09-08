use std::hint::black_box;
use std::time::Instant;

use tex_command::{
    CommandFuelLedger, CommandHostCapabilities, CommandHostContext, CommandProcessor,
    DeliveryStatus,
};
use tex_state::Universe;
use tex_state::token::Token;

use super::fixtures::{
    self, Storage, WorkloadKind, assert_definition_words, denominators, expected_text,
    validate_body,
};
use super::support::{
    assert_main_end, assert_raw_end, main_step, prepare_command, structural_delta, subtract_work,
    validate_receipt,
};
use super::{ProfileHooks, Receipt, TextConsumer};
pub(super) fn run_long_text<G>(
    universe: &mut Universe<G>,
    storage: Storage,
    iterations: usize,
    warmups: usize,
    text_chars: usize,
    hooks: ProfileHooks,
) -> Receipt {
    let denominators = denominators(WorkloadKind::LongText, text_chars, 0, storage);
    validate_long_text(universe, storage, text_chars);
    let mut command = prepare_command(
        universe,
        WorkloadKind::LongText,
        storage,
        iterations.saturating_add(warmups),
        text_chars,
        0,
    );
    let mut fuel = CommandFuelLedger::default();
    let mut consumer = TextConsumer::new(text_chars);
    {
        let mut capabilities = CommandHostCapabilities::default();
        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe
            .command_context()
            .expect("long text warmup context");
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut context,
            CommandHostContext::new(&mut capabilities),
            fuel.fuel_mut(),
            None,
            &mut effects,
        );
        for _ in 0..warmups {
            consumer.reset_operation();
            black_box(main_step(&mut processor, &mut consumer));
        }
    }
    consumer.reset_evidence();
    // The warmup processor borrows the fuel ledger and command context. Drop
    // that wrapper to take an actual warmup counter snapshot, then renew the
    // wrapper before timing. The command cursor, semantic state, and readers
    // remain warm across this boundary.
    let work_before = fuel.work();
    let snapshot_before = (hooks.snapshot)();
    let mut capabilities = CommandHostCapabilities::default();
    let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
    let mut context = universe.command_context().expect("long text context");
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut context,
        CommandHostContext::new(&mut capabilities),
        fuel.fuel_mut(),
        None,
        &mut effects,
    );
    let mut invalid_status = false;
    let start = Instant::now();
    for _ in 0..iterations {
        consumer.reset_operation();
        if !main_step(&mut processor, &mut consumer) {
            invalid_status = true;
        }
    }
    // Stop the timer before processor/context teardown, operation settlement,
    // counter snapshots, or any semantic validation.
    let elapsed_ns = start.elapsed().as_nanos();
    drop(processor);
    drop(context);
    let work_after = fuel.work();
    let structural = structural_delta(hooks, snapshot_before);
    assert_main_end(universe, &mut command, &mut fuel);
    let receipt = Receipt {
        elapsed_ns,
        semantic_hash: consumer.evidence.hash,
        text: Some(consumer.evidence),
        macro_output: None,
        definition_count: 0,
        definition_body_words: 0,
        definition_body_checksum: None,
        denominators,
        work: subtract_work(work_after, work_before),
        structural,
    };
    assert!(!invalid_status, "long text delivery status changed");
    validate_receipt(
        receipt,
        WorkloadKind::LongText,
        storage,
        iterations,
        text_chars,
        0,
        hooks,
    );
    receipt
}

fn validate_long_text<G>(universe: &mut Universe<G>, storage: Storage, text_chars: usize) {
    let mut command = prepare_command(universe, WorkloadKind::LongText, storage, 1, text_chars, 0);
    let mut fuel = CommandFuelLedger::default();
    let mut consumer = TextConsumer::new(text_chars);
    let mut capabilities = CommandHostCapabilities::default();
    let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
    let mut context = universe
        .command_context()
        .expect("long text validation context");
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut context,
        CommandHostContext::new(&mut capabilities),
        fuel.fuel_mut(),
        None,
        &mut effects,
    );
    assert!(main_step(&mut processor, &mut consumer));
    assert_eq!(consumer.evidence, expected_text(text_chars, 1));
    let mut destination = None;
    let status = processor
        .main_loop_source_step_into(&mut destination, &mut consumer)
        .expect("long text validation EOF");
    assert_eq!(status, DeliveryStatus::End);
    assert!(destination.is_none());
}

pub(super) fn run_definition_body<G>(
    universe: &mut Universe<G>,
    storage: Storage,
    iterations: usize,
    warmups: usize,
    body_words: usize,
    symbols: fixtures::DefinitionSymbols,
    hooks: ProfileHooks,
) -> Receipt {
    let denominators = denominators(WorkloadKind::DefinitionBody, 0, body_words, storage);
    validate_definition_fixture(universe, storage, body_words, symbols);
    let mut command = prepare_command(
        universe,
        WorkloadKind::DefinitionBody,
        storage,
        iterations.saturating_add(warmups),
        0,
        body_words,
    );
    let operation = command.begin_attempt_operation();
    let mut fuel = CommandFuelLedger::default();
    {
        let mut capabilities = CommandHostCapabilities::default();
        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe
            .command_context()
            .expect("definition warmup context");
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut context,
            CommandHostContext::new(&mut capabilities),
            fuel.fuel_mut(),
            None,
            &mut effects,
        );
        for _ in 0..warmups {
            black_box(
                processor
                    .scan_macro_definition(false, false)
                    .expect("definition warmup scan"),
            );
        }
    }
    // Renew the borrowed processor after taking the actual warmup counter
    // snapshot. The command attempt, semantic state, and scanner storage stay
    // warm; only the short-lived borrow wrapper is reconstructed before timing.
    let work_before = fuel.work();
    let snapshot_before = (hooks.snapshot)();
    let mut capabilities = CommandHostCapabilities::default();
    let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
    let mut context = universe.command_context().expect("definition context");
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut context,
        CommandHostContext::new(&mut capabilities),
        fuel.fuel_mut(),
        None,
        &mut effects,
    );
    let mut definitions = 0_u64;
    let mut invalid_target = false;
    let mut last_definition = None;
    let start = Instant::now();
    for _ in 0..iterations {
        let scanned = black_box(
            processor
                .scan_macro_definition(false, false)
                .expect("definition scan"),
        );
        invalid_target |= scanned.target != symbols.target;
        last_definition = Some(scanned.definition);
        definitions = definitions.saturating_add(1);
    }
    // Stop the timer before processor/context teardown, operation settlement,
    // counter snapshots, or any semantic validation.
    let elapsed_ns = start.elapsed().as_nanos();
    drop(processor);
    drop(context);
    command
        .commit_attempt_operation(operation)
        .expect("definition benchmark operation commit");
    let work_after = fuel.work();
    let structural = structural_delta(hooks, snapshot_before);
    let mut body_checksum = 0;
    let mut sentinel_capabilities = CommandHostCapabilities::default();
    let mut sentinel_effects = tex_state::diagnostic::DiagnosticEffects::new();
    let mut sentinel_context = universe
        .command_context()
        .expect("definition sentinel context");
    if let Some(definition) = last_definition {
        body_checksum = validate_body(
            &sentinel_context,
            definition,
            symbols.body_control,
            body_words,
        );
    }
    let mut sentinel = CommandProcessor::new(
        &mut command,
        &mut sentinel_context,
        CommandHostContext::new(&mut sentinel_capabilities),
        fuel.fuel_mut(),
        None,
        &mut sentinel_effects,
    );
    assert_raw_end(&mut sentinel);
    drop(sentinel);
    let receipt = Receipt {
        elapsed_ns,
        semantic_hash: body_checksum,
        text: None,
        macro_output: None,
        definition_count: definitions,
        definition_body_words: body_words,
        definition_body_checksum: Some(body_checksum),
        denominators,
        work: subtract_work(work_after, work_before),
        structural,
    };
    assert!(!invalid_target, "definition target changed");
    validate_receipt(
        receipt,
        WorkloadKind::DefinitionBody,
        storage,
        iterations,
        0,
        body_words,
        hooks,
    );
    receipt
}

fn validate_definition_fixture<G>(
    universe: &mut Universe<G>,
    storage: Storage,
    body_words: usize,
    symbols: fixtures::DefinitionSymbols,
) {
    let mut command = prepare_command(
        universe,
        WorkloadKind::DefinitionBody,
        storage,
        1,
        0,
        body_words,
    );
    let mut fuel = CommandFuelLedger::default();
    let mut capabilities = CommandHostCapabilities::default();
    let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
    let mut context = universe
        .command_context()
        .expect("definition validation context");
    let operation = command.begin_attempt_operation();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut context,
        CommandHostContext::new(&mut capabilities),
        fuel.fuel_mut(),
        None,
        &mut effects,
    );
    let scanned = processor
        .scan_macro_definition(false, false)
        .expect("definition validation scan");
    drop(processor);
    command
        .commit_attempt_operation(operation)
        .expect("definition validation operation commit");
    assert_eq!(scanned.target, symbols.target);
    let expected = fixtures::body_tokens(symbols.body_control, body_words);
    assert_definition_words(&context, scanned.definition, &[Token::Param(1)], &expected);
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut context,
        CommandHostContext::new(&mut capabilities),
        fuel.fuel_mut(),
        None,
        &mut effects,
    );
    assert_raw_end(&mut processor);
}
