use tex_command::{
    CommandFuelLedger, CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandState,
    DeliveryStatus,
};
use tex_state::Universe;
use tex_state::token::{Catcode, Token};

use super::fixtures::{
    CHAIN_RESULT_CHARACTER, Evidence, Storage, WorkloadKind, build_source_text, build_stored_words,
    denominators, expected_body_checksum, expected_chain, expected_text, open_source, push_stored,
};
use super::{ProfileHooks, Receipt, StructuralDelta, StructuralSnapshot, TextConsumer};

const INVALID_COMMAND_CHECKSUM: u64 = 0xDEAD_C001;
pub(super) fn prepare_command<G>(
    universe: &mut Universe<G>,
    kind: WorkloadKind,
    storage: Storage,
    operations: usize,
    text_chars: usize,
    body_words: usize,
) -> CommandState<G> {
    let mut command = CommandState::default();
    match storage {
        Storage::Source => {
            let source = build_source_text(kind, operations, text_chars, body_words);
            open_source(&mut command, &source);
        }
        Storage::Stored => {
            let words = build_stored_words(universe, kind, operations, text_chars, body_words);
            push_stored(universe, &mut command, &words);
        }
    }
    command
}

pub(super) fn main_step<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    consumer: &mut TextConsumer,
) -> bool {
    let mut destination = None;
    let status = processor
        .main_loop_source_step_into(&mut destination, consumer)
        .expect("main consumer delivery");
    status == DeliveryStatus::CharacterRun && destination.is_none()
}

pub(super) fn record_command<G>(
    evidence: &mut Evidence,
    status: DeliveryStatus,
    destination: Option<tex_command::CurrentCommand<G>>,
) -> bool {
    if status != DeliveryStatus::Command {
        evidence.count = evidence.count.saturating_add(1);
        evidence.checksum = evidence.checksum.wrapping_add(INVALID_COMMAND_CHECKSUM);
        evidence.hash = evidence.hash.rotate_left(7) ^ INVALID_COMMAND_CHECKSUM;
        return false;
    }
    let Some(command) = destination else {
        return false;
    };
    match command.spelling().semantic_token() {
        Token::Char { ch, cat } => {
            evidence.absorb_command(ch, cat, command.control_sequence().is_some());
            ch == CHAIN_RESULT_CHARACTER
                && cat == Catcode::Letter
                && command.control_sequence().is_none()
        }
        _ => {
            evidence.count = evidence.count.saturating_add(1);
            evidence.checksum = evidence.checksum.wrapping_add(INVALID_COMMAND_CHECKSUM);
            evidence.hash = evidence.hash.rotate_left(7) ^ INVALID_COMMAND_CHECKSUM;
            false
        }
    }
}

pub(super) fn assert_main_end<G>(
    universe: &mut Universe<G>,
    command: &mut CommandState<G>,
    fuel: &mut CommandFuelLedger,
) {
    let mut capabilities = CommandHostCapabilities::default();
    let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
    let mut context = universe.command_context().expect("main sentinel context");
    let mut processor = CommandProcessor::new(
        command,
        &mut context,
        CommandHostContext::new(&mut capabilities),
        fuel.fuel_mut(),
        None,
        &mut effects,
    );
    assert_main_end_with_processor(&mut processor);
}

pub(super) fn assert_main_end_with_processor<G>(processor: &mut CommandProcessor<'_, '_, G>) {
    let mut destination = None;
    let status = processor
        .main_loop_source_step_into(&mut destination, &mut TextConsumer::new(1))
        .expect("main sentinel delivery");
    assert_eq!(status, DeliveryStatus::End);
    assert!(destination.is_none());
}

pub(super) fn assert_raw_end<G>(processor: &mut CommandProcessor<'_, '_, G>) {
    let mut destination = None;
    let status = processor
        .get_next_into(&mut destination)
        .expect("raw sentinel delivery");
    assert_eq!(status, DeliveryStatus::End);
    assert!(destination.is_none());
}

pub(super) fn assert_expanded_end<G>(
    universe: &mut Universe<G>,
    command: &mut CommandState<G>,
    fuel: &mut CommandFuelLedger,
) {
    let mut capabilities = CommandHostCapabilities::default();
    let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
    let mut context = universe
        .command_context()
        .expect("expanded sentinel context");
    let mut processor = CommandProcessor::new(
        command,
        &mut context,
        CommandHostContext::new(&mut capabilities),
        fuel.fuel_mut(),
        None,
        &mut effects,
    );
    assert_expanded_end_in_processor(&mut processor);
}

pub(super) fn assert_expanded_end_in_processor<G>(processor: &mut CommandProcessor<'_, '_, G>) {
    let mut destination = None;
    let status = processor
        .get_x_token_into(&mut destination)
        .expect("expanded sentinel delivery");
    assert_eq!(status, DeliveryStatus::End);
    assert!(destination.is_none());
}

pub(super) fn combine_hash(text: u64, macro_output: u64, body: u64) -> u64 {
    semantic_combine(text, macro_output, body)
}

const fn semantic_combine(text: u64, macro_output: u64, body: u64) -> u64 {
    text.rotate_left(11) ^ macro_output.rotate_left(23) ^ body
}

pub(super) fn structural_delta(hooks: ProfileHooks, before: StructuralSnapshot) -> StructuralDelta {
    if !hooks.instrumented {
        return StructuralDelta::default();
    }
    let after = (hooks.snapshot)();
    StructuralDelta {
        macro_expansions: Some(
            after
                .macro_expansions
                .saturating_sub(before.macro_expansions),
        ),
        definition_direct_stores: Some(
            after
                .definition_direct_stores
                .saturating_sub(before.definition_direct_stores),
        ),
        definition_chunk_transitions: Some(
            after
                .definition_chunk_transitions
                .saturating_sub(before.definition_chunk_transitions),
        ),
        definition_episode_admissions: Some(
            after
                .definition_episode_admissions
                .saturating_sub(before.definition_episode_admissions),
        ),
    }
}

pub(super) fn subtract_work(
    after: tex_command::CommandWorkCounters,
    before: tex_command::CommandWorkCounters,
) -> tex_command::CommandWorkCounters {
    tex_command::CommandWorkCounters {
        fuel_charges: after.fuel_charges.saturating_sub(before.fuel_charges),
        token_frame_steps: after
            .token_frame_steps
            .saturating_sub(before.token_frame_steps),
        expanded_deliveries: after
            .expanded_deliveries
            .saturating_sub(before.expanded_deliveries),
        meaning_lookups: after.meaning_lookups.saturating_sub(before.meaning_lookups),
        scanner_tokens: after.scanner_tokens.saturating_sub(before.scanner_tokens),
        write_expansions: after
            .write_expansions
            .saturating_sub(before.write_expansions),
        raw_delivery_kinds: std::array::from_fn(|index| {
            after.raw_delivery_kinds[index].saturating_sub(before.raw_delivery_kinds[index])
        }),
    }
}

pub(super) fn work_per_operation(
    kind: WorkloadKind,
    storage: Storage,
    text_chars: usize,
    body_words: usize,
) -> tex_command::CommandWorkCounters {
    let denominator = denominators(kind, text_chars, body_words, storage);
    let mut work = tex_command::CommandWorkCounters {
        fuel_charges: denominator.token_work_per_operation as u64,
        ..tex_command::CommandWorkCounters::default()
    };
    match kind {
        WorkloadKind::LongText => {
            work.token_frame_steps = text_chars as u64;
            work.raw_delivery_kinds[match storage {
                Storage::Source => 0,
                Storage::Stored => 1,
            }] = text_chars as u64;
        }
        WorkloadKind::DefinitionBody => {
            let input = body_words.saturating_add(5) as u64;
            work.token_frame_steps = input;
            work.meaning_lookups = body_words.saturating_div(7).saturating_add(1) as u64;
            work.scanner_tokens = input.saturating_sub(1);
            work.raw_delivery_kinds[match storage {
                Storage::Source => 0,
                Storage::Stored => 1,
            }] = input;
        }
        WorkloadKind::ParameterizedChain => {
            work.token_frame_steps = 13;
            work.expanded_deliveries = 1;
            work.meaning_lookups = 3;
            work.scanner_tokens = 9;
            work.raw_delivery_kinds[match storage {
                Storage::Source => 0,
                Storage::Stored => 1,
            }] = 4;
            work.raw_delivery_kinds[1] = work.raw_delivery_kinds[1].saturating_add(6);
            work.raw_delivery_kinds[2] = 3;
        }
        WorkloadKind::MixedPipeline => {
            let definition_input = body_words.saturating_add(5) as u64;
            work.token_frame_steps = denominator.token_work_per_operation as u64;
            work.expanded_deliveries = 1;
            work.meaning_lookups = body_words.saturating_div(7).saturating_add(4) as u64;
            work.scanner_tokens = body_words.saturating_add(13) as u64;
            work.raw_delivery_kinds[match storage {
                Storage::Source => 0,
                Storage::Stored => 1,
            }] = text_chars as u64 + 4 + definition_input;
            work.raw_delivery_kinds[1] = work.raw_delivery_kinds[1].saturating_add(6);
            work.raw_delivery_kinds[2] = 3;
        }
    }
    work
}

pub(super) fn scale_work(
    work: tex_command::CommandWorkCounters,
    count: usize,
) -> tex_command::CommandWorkCounters {
    let count = count as u64;
    tex_command::CommandWorkCounters {
        fuel_charges: work.fuel_charges.saturating_mul(count),
        token_frame_steps: work.token_frame_steps.saturating_mul(count),
        expanded_deliveries: work.expanded_deliveries.saturating_mul(count),
        meaning_lookups: work.meaning_lookups.saturating_mul(count),
        scanner_tokens: work.scanner_tokens.saturating_mul(count),
        write_expansions: work.write_expansions.saturating_mul(count),
        raw_delivery_kinds: work
            .raw_delivery_kinds
            .map(|value| value.saturating_mul(count)),
    }
}

pub(super) fn validate_receipt(
    receipt: Receipt,
    kind: WorkloadKind,
    storage: Storage,
    iterations: usize,
    text_chars: usize,
    body_words: usize,
    hooks: ProfileHooks,
) {
    let expected = denominators(kind, text_chars, body_words, storage);
    assert_eq!(
        receipt.denominators.token_work_per_operation,
        expected.token_work_per_operation
    );
    assert_eq!(
        receipt.work.fuel_charges,
        receipt
            .denominators
            .token_work_per_operation
            .saturating_mul(iterations) as u64,
        "consumer fuel work differs from the known token-work denominator"
    );
    if hooks.instrumented {
        let expected_work = scale_work(
            work_per_operation(kind, storage, text_chars, body_words),
            iterations,
        );
        assert_eq!(
            receipt.work, expected_work,
            "profiling work counters differ from the independent fixture census"
        );
    }
    match kind {
        WorkloadKind::LongText => {
            assert_eq!(receipt.text, Some(expected_text(text_chars, iterations)));
        }
        WorkloadKind::DefinitionBody => {
            assert_eq!(receipt.definition_count, iterations as u64);
            assert_eq!(
                receipt.definition_body_checksum,
                Some(expected_body_checksum(body_words))
            );
        }
        WorkloadKind::ParameterizedChain => {
            assert_eq!(receipt.macro_output, Some(expected_chain(iterations)));
        }
        WorkloadKind::MixedPipeline => {
            assert_eq!(receipt.text, Some(expected_text(text_chars, iterations)));
            assert_eq!(receipt.macro_output, Some(expected_chain(iterations)));
            assert_eq!(receipt.definition_count, iterations as u64);
            assert_eq!(
                receipt.definition_body_checksum,
                Some(expected_body_checksum(body_words))
            );
            assert_eq!(
                receipt.semantic_hash,
                combine_hash(
                    expected_text(text_chars, iterations).hash,
                    expected_chain(iterations).hash,
                    expected_body_checksum(body_words),
                )
            );
        }
    }
    if hooks.instrumented {
        match kind {
            WorkloadKind::ParameterizedChain | WorkloadKind::MixedPipeline => assert_eq!(
                receipt.structural.macro_expansions,
                Some(
                    receipt
                        .denominators
                        .macro_calls_per_operation
                        .saturating_mul(iterations) as u64,
                )
            ),
            WorkloadKind::LongText | WorkloadKind::DefinitionBody => {}
        }
        if matches!(
            kind,
            WorkloadKind::DefinitionBody | WorkloadKind::MixedPipeline
        ) {
            assert_eq!(
                receipt.structural.definition_direct_stores,
                Some(
                    receipt
                        .denominators
                        .definition_body_words_per_operation
                        .saturating_add(1)
                        .saturating_mul(iterations) as u64,
                )
            );
        }
    }
}
