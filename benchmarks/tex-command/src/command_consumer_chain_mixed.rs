use std::hint::black_box;
use std::time::Instant;

use tex_command::{
    CommandFuelLedger, CommandHostCapabilities, CommandHostContext, CommandProcessor,
    DeliveryStatus,
};
use tex_state::Universe;
use tex_state::token::{Catcode, Token};

use super::fixtures::{
    self, CHAIN_RESULT_CHARACTER, Evidence, Storage, WorkloadKind, denominators,
    expected_body_checksum, expected_chain, expected_text, validate_body,
};
use super::support::{
    assert_expanded_end, assert_expanded_end_in_processor, assert_main_end_with_processor,
    combine_hash, prepare_command, record_command, structural_delta, subtract_work,
    validate_receipt,
};
use super::{ProfileHooks, Receipt, TextConsumer};
pub(super) fn run_chain<G>(
    universe: &mut Universe<G>,
    storage: Storage,
    iterations: usize,
    warmups: usize,
    symbols: fixtures::ChainSymbols,
    text_chars: usize,
    body_words: usize,
    hooks: ProfileHooks,
) -> Receipt {
    let denominators = denominators(
        WorkloadKind::ParameterizedChain,
        text_chars,
        body_words,
        storage,
    );
    validate_chain_fixture(universe, storage, symbols);
    let mut command = prepare_command(
        universe,
        WorkloadKind::ParameterizedChain,
        storage,
        iterations.saturating_add(warmups),
        text_chars,
        body_words,
    );
    let mut fuel = CommandFuelLedger::default();
    {
        let mut capabilities = CommandHostCapabilities::default();
        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("chain warmup context");
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut context,
            CommandHostContext::new(&mut capabilities),
            fuel.fuel_mut(),
            None,
            &mut effects,
        );
        for _ in 0..warmups {
            let mut destination = None;
            black_box(
                processor
                    .get_x_token_into(&mut destination)
                    .expect("chain warmup delivery"),
            );
        }
    }
    // The warmup processor owns the borrow needed by the fuel snapshot. The
    // command cursor and expansion state remain warm while that wrapper is
    // renewed before timing the measured deliveries.
    let work_before = fuel.work();
    let snapshot_before = (hooks.snapshot)();
    let mut capabilities = CommandHostCapabilities::default();
    let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
    let mut context = universe.command_context().expect("chain context");
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut context,
        CommandHostContext::new(&mut capabilities),
        fuel.fuel_mut(),
        None,
        &mut effects,
    );
    let mut evidence = Evidence::seeded();
    let mut invalid = false;
    let start = Instant::now();
    for _ in 0..iterations {
        let mut destination = None;
        let status = processor
            .get_x_token_into(&mut destination)
            .expect("chain delivery");
        invalid |= !record_command(&mut evidence, status, destination);
    }
    // Stop the timer before processor/context teardown, counter snapshots, or
    // the end-of-input sentinel.
    let elapsed_ns = start.elapsed().as_nanos();
    drop(processor);
    drop(context);
    let work_after = fuel.work();
    let structural = structural_delta(hooks, snapshot_before);
    assert_expanded_end(universe, &mut command, &mut fuel);
    let receipt = Receipt {
        elapsed_ns,
        semantic_hash: evidence.hash,
        text: None,
        macro_output: Some(evidence),
        definition_count: 0,
        definition_body_words: 0,
        definition_body_checksum: None,
        denominators,
        work: subtract_work(work_after, work_before),
        structural,
    };
    assert!(!invalid, "parameterized chain delivery changed");
    validate_receipt(
        receipt,
        WorkloadKind::ParameterizedChain,
        storage,
        iterations,
        0,
        0,
        hooks,
    );
    receipt
}

fn validate_chain_fixture<G>(
    universe: &mut Universe<G>,
    storage: Storage,
    symbols: fixtures::ChainSymbols,
) {
    let mut command = prepare_command(universe, WorkloadKind::ParameterizedChain, storage, 1, 0, 0);
    let mut fuel = CommandFuelLedger::default();
    let mut capabilities = CommandHostCapabilities::default();
    let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
    let mut context = universe
        .command_context()
        .expect("chain validation context");
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut context,
        CommandHostContext::new(&mut capabilities),
        fuel.fuel_mut(),
        None,
        &mut effects,
    );
    let mut destination = None;
    let status = processor
        .get_x_token_into(&mut destination)
        .expect("chain validation delivery");
    assert_eq!(status, DeliveryStatus::Command);
    let command = destination.expect("chain validation command");
    assert_eq!(command.control_sequence(), None);
    assert_eq!(
        command.spelling().semantic_token(),
        Token::Char {
            ch: CHAIN_RESULT_CHARACTER,
            cat: Catcode::Letter,
        }
    );
    assert_expanded_end_in_processor(&mut processor);
    let _ = symbols;
}

pub(super) fn run_mixed<G>(
    universe: &mut Universe<G>,
    storage: Storage,
    iterations: usize,
    warmups: usize,
    chain: fixtures::ChainSymbols,
    definition: fixtures::DefinitionSymbols,
    text_chars: usize,
    body_words: usize,
    hooks: ProfileHooks,
) -> Receipt {
    let denominators = denominators(WorkloadKind::MixedPipeline, text_chars, body_words, storage);
    validate_mixed_fixture(universe, storage, chain, definition, text_chars, body_words);
    let mut command = prepare_command(
        universe,
        WorkloadKind::MixedPipeline,
        storage,
        iterations.saturating_add(warmups),
        text_chars,
        body_words,
    );
    let operation = command.begin_attempt_operation();
    let mut fuel = CommandFuelLedger::default();
    let mut text = TextConsumer::new(text_chars);
    {
        let mut capabilities = CommandHostCapabilities::default();
        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("mixed warmup context");
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut context,
            CommandHostContext::new(&mut capabilities),
            fuel.fuel_mut(),
            None,
            &mut effects,
        );
        for _ in 0..warmups {
            text.reset_operation();
            let mut macro_evidence = Evidence::seeded();
            black_box(mixed_step(
                &mut processor,
                &mut text,
                &mut macro_evidence,
                definition.target,
            ));
        }
    }
    text.reset_evidence();
    let mut macro_evidence = Evidence::seeded();
    let mut definition_count = 0_u64;
    let mut last_definition = None;
    // Recreate the borrowed processor after taking the actual warmup counter
    // snapshot. The attempt, token cursor, semantic state, and reader scratch
    // remain warm; the wrapper construction is before the measured timer.
    let work_before = fuel.work();
    let snapshot_before = (hooks.snapshot)();
    let mut capabilities = CommandHostCapabilities::default();
    let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
    let mut context = universe.command_context().expect("mixed context");
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut context,
        CommandHostContext::new(&mut capabilities),
        fuel.fuel_mut(),
        None,
        &mut effects,
    );
    let mut invalid = false;
    let start = Instant::now();
    for _ in 0..iterations {
        text.reset_operation();
        let step = mixed_step(
            &mut processor,
            &mut text,
            &mut macro_evidence,
            definition.target,
        );
        invalid |= !step.valid;
        definition_count = definition_count.saturating_add(u64::from(step.definition.is_some()));
        last_definition = step.definition;
    }
    // Stop the timer before processor/context teardown, operation settlement,
    // counter snapshots, or any semantic validation.
    let elapsed_ns = start.elapsed().as_nanos();
    drop(processor);
    drop(context);
    command
        .commit_attempt_operation(operation)
        .expect("mixed benchmark operation commit");
    let work_after = fuel.work();
    let structural = structural_delta(hooks, snapshot_before);
    let mut body_checksum = 0;
    let mut sentinel_capabilities = CommandHostCapabilities::default();
    let mut sentinel_effects = tex_state::diagnostic::DiagnosticEffects::new();
    let mut sentinel_context = universe.command_context().expect("mixed sentinel context");
    if let Some(definition_ref) = last_definition {
        body_checksum = validate_body(
            &sentinel_context,
            definition_ref,
            definition.body_control,
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
    assert_main_end_with_processor(&mut sentinel);
    drop(sentinel);
    let receipt = Receipt {
        elapsed_ns,
        semantic_hash: combine_hash(text.evidence.hash, macro_evidence.hash, body_checksum),
        text: Some(text.evidence),
        macro_output: Some(macro_evidence),
        definition_count,
        definition_body_words: body_words,
        definition_body_checksum: Some(body_checksum),
        denominators,
        work: subtract_work(work_after, work_before),
        structural,
    };
    assert!(!invalid, "mixed consumer pipeline changed");
    validate_receipt(
        receipt,
        WorkloadKind::MixedPipeline,
        storage,
        iterations,
        text_chars,
        body_words,
        hooks,
    );
    receipt
}

#[derive(Clone, Copy, Debug)]
struct MixedStep<G> {
    valid: bool,
    definition: Option<tex_state::DefinitionRef<G>>,
}

fn mixed_step<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    text: &mut TextConsumer,
    macro_evidence: &mut Evidence,
    definition_target: tex_state::interner::Symbol,
) -> MixedStep<G> {
    let mut valid = true;
    let mut destination = None;
    let status = processor
        .main_loop_source_step_into(&mut destination, text)
        .expect("mixed text delivery");
    valid &= status == DeliveryStatus::CharacterRun && destination.is_none();

    let mut destination = None;
    let status = processor
        .main_loop_source_step_into(&mut destination, text)
        .expect("mixed boundary delivery");
    valid &= status == DeliveryStatus::CharacterRunBoundary && destination.is_some();
    let status = processor
        .preflight_command_into(&mut destination)
        .expect("mixed macro delivery");
    valid &= record_command(macro_evidence, status, destination);

    let scanned = processor
        .scan_macro_definition(false, false)
        .expect("mixed definition delivery");
    valid &= scanned.target == definition_target;
    MixedStep {
        valid,
        definition: Some(scanned.definition),
    }
}

fn validate_mixed_fixture<G>(
    universe: &mut Universe<G>,
    storage: Storage,
    chain: fixtures::ChainSymbols,
    definition: fixtures::DefinitionSymbols,
    text_chars: usize,
    body_words: usize,
) {
    let mut command = prepare_command(
        universe,
        WorkloadKind::MixedPipeline,
        storage,
        1,
        text_chars,
        body_words,
    );
    let mut fuel = CommandFuelLedger::default();
    let mut text = TextConsumer::new(text_chars);
    let mut macro_evidence = Evidence::seeded();
    let mut capabilities = CommandHostCapabilities::default();
    let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
    let mut context = universe
        .command_context()
        .expect("mixed validation context");
    let operation = command.begin_attempt_operation();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut context,
        CommandHostContext::new(&mut capabilities),
        fuel.fuel_mut(),
        None,
        &mut effects,
    );
    text.reset_operation();
    let step = mixed_step(
        &mut processor,
        &mut text,
        &mut macro_evidence,
        definition.target,
    );
    assert!(step.valid);
    assert_eq!(text.evidence, expected_text(text_chars, 1));
    assert_eq!(macro_evidence, expected_chain(1));
    let definition_ref = step.definition.expect("mixed definition");
    drop(processor);
    command
        .commit_attempt_operation(operation)
        .expect("mixed validation operation commit");
    let body_checksum = validate_body(
        &context,
        definition_ref,
        definition.body_control,
        body_words,
    );
    assert_eq!(body_checksum, expected_body_checksum(body_words));
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut context,
        CommandHostContext::new(&mut capabilities),
        fuel.fuel_mut(),
        None,
        &mut effects,
    );
    assert_main_end_with_processor(&mut processor);
    let _ = chain;
}
