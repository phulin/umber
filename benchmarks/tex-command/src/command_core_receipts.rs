use super::fixtures::{Denominators, OutputToken, workload};
use super::{Case, Delivery, ObserverMode, Options, Receipt, SEMANTIC_SEED, Storage, Workload};
use tex_state::token::Catcode;

fn expected_semantic(output: OutputToken) -> u64 {
    match output {
        OutputToken::Char(ch) => 0x1000_0000 ^ (ch as u64) ^ u64::from(Catcode::Letter as u8),
        OutputToken::ControlSequence => 0x2000_0001,
    }
}

fn expected_semantic_hash(output: OutputToken, iterations: usize) -> u64 {
    let mut hash = SEMANTIC_SEED;
    for _ in 0..iterations {
        hash = hash.rotate_left(7) ^ expected_semantic(output);
    }
    hash
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

pub(super) fn validate_receipt(
    case: Case,
    storage: Storage,
    delivery: Delivery,
    observer_mode: ObserverMode,
    options: Options,
    workload: Workload,
    denominators: Denominators,
    receipt: Receipt,
) {
    if matches!(observer_mode, ObserverMode::Enabled) {
        assert!(
            receipt.observer_records > 0,
            "enabled observer received no records"
        );
    } else {
        assert_eq!(receipt.observer_records, 0);
    }
    let expected_checksum = workload
        .output(delivery)
        .checksum()
        .wrapping_mul(options.iterations as u64);
    assert_eq!(
        receipt.checksum, expected_checksum,
        "matrix checksum does not match the fixture's expected semantic output"
    );
    assert_eq!(
        receipt.semantic_hash,
        expected_semantic_hash(workload.output(delivery), options.iterations),
        "matrix semantic evidence does not match the fixture"
    );
    assert_eq!(
        receipt.work.fuel_charges,
        denominators
            .token_work_per_operation
            .saturating_mul(options.iterations) as u64,
        "fuel work does not match the known token-work denominator"
    );
    assert!(denominators.source_words_per_operation > 0);
    assert!(denominators.stored_words_per_operation > 0);
    assert_eq!(
        denominators.input_tokens_per_operation,
        match storage {
            Storage::Source => denominators.source_words_per_operation,
            Storage::Stored => denominators.stored_words_per_operation,
        }
    );
    assert_eq!(
        denominators.macro_calls_per_operation,
        if matches!(delivery, Delivery::Expanded) {
            workload.macro_calls_per_operation
        } else {
            0
        }
    );
    assert_eq!(
        denominators.body_tokens_per_operation,
        if matches!(delivery, Delivery::Expanded) {
            workload.body_tokens_per_operation
        } else {
            0
        }
    );
    assert_eq!(
        denominators.argument_tokens_per_operation,
        if matches!(delivery, Delivery::Expanded) {
            workload.argument_tokens_per_operation
        } else {
            0
        }
    );
    assert_eq!(
        denominators.delimiter_tokens_per_operation,
        if matches!(delivery, Delivery::Expanded) {
            workload.delimiter_tokens_per_operation
        } else {
            0
        }
    );
    assert_eq!(
        denominators.token_work_per_operation,
        denominators
            .input_tokens_per_operation
            .saturating_add(denominators.body_tokens_per_operation)
    );
    if matches!(case, Case::DelimitedNestedArgument) && matches!(delivery, Delivery::Expanded) {
        assert_eq!(denominators.delimiter_tokens_per_operation, 1);
    }
}

pub(super) fn print_record(
    case: Case,
    storage: Storage,
    delivery: Delivery,
    observer: ObserverMode,
    options: Options,
    profile_name: &str,
    receipt: Receipt,
) {
    let denominators = workload(case).denominators(storage, delivery);
    let work = receipt.work;
    let raw = work.raw_delivery_kinds;
    let total = |per_operation: usize| per_operation.saturating_mul(options.iterations);
    println!(
        "{{\"schema\":\"command-core-matrix-v1\",\"case\":\"{}\",\"storage\":\"{}\",\"delivery\":\"{}\",\"observer\":\"{}\",\"iterations\":{},\"warmups\":{},\"elapsed_ns\":{},\"ns_per_iteration\":{},\"checksum\":{},\"semantic_hash\":{},\"observer_records\":{},\"source_words_per_operation\":{},\"stored_words_per_operation\":{},\"input_tokens_per_operation\":{},\"token_work_per_operation\":{},\"macro_calls_per_operation\":{},\"body_tokens_per_operation\":{},\"argument_tokens_per_operation\":{},\"delimiter_tokens_per_operation\":{},\"source_words\":{},\"stored_words\":{},\"input_tokens\":{},\"token_work\":{},\"macro_calls\":{},\"body_tokens\":{},\"argument_tokens\":{},\"delimiter_tokens\":{},\"fuel_charges\":{},\"token_frame_steps\":{},\"expanded_deliveries\":{},\"meaning_lookups\":{},\"scanner_tokens\":{},\"raw_source\":{},\"raw_stored\":{},\"raw_argument\":{},\"raw_synthetic_end\":{},\"profile\":\"{}\",\"macro_expansions\":{}}}",
        case.name(),
        storage.name(),
        delivery.name(),
        observer.name(),
        options.iterations,
        options.warmups,
        receipt.elapsed_ns,
        receipt.elapsed_ns as f64 / options.iterations as f64,
        receipt.checksum,
        receipt.semantic_hash,
        receipt.observer_records,
        denominators.source_words_per_operation,
        denominators.stored_words_per_operation,
        denominators.input_tokens_per_operation,
        denominators.token_work_per_operation,
        denominators.macro_calls_per_operation,
        denominators.body_tokens_per_operation,
        denominators.argument_tokens_per_operation,
        denominators.delimiter_tokens_per_operation,
        total(denominators.source_words_per_operation),
        total(denominators.stored_words_per_operation),
        total(denominators.input_tokens_per_operation),
        total(denominators.token_work_per_operation),
        total(denominators.macro_calls_per_operation),
        total(denominators.body_tokens_per_operation),
        total(denominators.argument_tokens_per_operation),
        total(denominators.delimiter_tokens_per_operation),
        work.fuel_charges,
        work.token_frame_steps,
        work.expanded_deliveries,
        work.meaning_lookups,
        work.scanner_tokens,
        raw[0],
        raw[1],
        raw[2],
        raw[3],
        profile_name,
        receipt.macro_expansions,
    );
}
