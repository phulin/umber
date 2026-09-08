use super::fixtures::Evidence;
use super::{ProfileHooks, Receipt, Storage, WorkloadKind};

pub(super) fn print_record(
    kind: WorkloadKind,
    storage: Storage,
    iterations: usize,
    warmups: usize,
    hooks: ProfileHooks,
    receipt: Receipt,
) {
    let denominators = receipt.denominators;
    let work = receipt.work;
    let raw = work.raw_delivery_kinds;
    let total = |per_operation: usize| per_operation.saturating_mul(iterations);
    println!(
        "{{\"schema\":\"consumer-core-v1\",\"workload\":\"{}\",\"storage\":\"{}\",\"profile\":\"{}\",\"iterations\":{},\"warmups\":{},\"elapsed_ns\":{},\"ns_per_iteration\":{},\"timed_evidence\":\"checksum_sink\",\"semantic_hash\":{},\"text_count\":{},\"text_checksum\":{},\"text_hash\":{},\"macro_count\":{},\"macro_checksum\":{},\"macro_hash\":{},\"definition_count\":{},\"definition_body_words\":{},\"definition_body_checksum\":{},\"source_words_per_operation\":{},\"stored_words_per_operation\":{},\"input_tokens_per_operation\":{},\"token_work_per_operation\":{},\"text_characters_per_operation\":{},\"macro_calls_per_operation\":{},\"definition_calls_per_operation\":{},\"definition_body_words_per_operation\":{},\"source_words\":{},\"stored_words\":{},\"input_tokens\":{},\"token_work\":{},\"text_characters\":{},\"macro_calls\":{},\"definition_calls\":{},\"definition_body_words_total\":{},\"fuel_charges\":{},\"token_frame_steps\":{},\"expanded_deliveries\":{},\"meaning_lookups\":{},\"scanner_tokens\":{},\"write_expansions\":{},\"raw_source\":{},\"raw_stored\":{},\"raw_argument\":{},\"raw_synthetic_end\":{},\"macro_expansions\":{},\"definition_direct_stores\":{},\"definition_chunk_transitions\":{},\"definition_episode_admissions\":{}}}",
        kind.name(),
        storage.name(),
        hooks.name,
        iterations,
        warmups,
        receipt.elapsed_ns,
        receipt.elapsed_ns as f64 / iterations as f64,
        receipt.semantic_hash,
        evidence_field(receipt.text, |e| e.count),
        evidence_field(receipt.text, |e| e.checksum),
        evidence_field(receipt.text, |e| e.hash),
        evidence_field(receipt.macro_output, |e| e.count),
        evidence_field(receipt.macro_output, |e| e.checksum),
        evidence_field(receipt.macro_output, |e| e.hash),
        receipt.definition_count,
        receipt.definition_body_words,
        optional_u64(receipt.definition_body_checksum),
        denominators.source_words_per_operation,
        denominators.stored_words_per_operation,
        denominators.input_tokens_per_operation,
        denominators.token_work_per_operation,
        denominators.text_characters_per_operation,
        denominators.macro_calls_per_operation,
        denominators.definition_calls_per_operation,
        denominators.definition_body_words_per_operation,
        total(denominators.source_words_per_operation),
        total(denominators.stored_words_per_operation),
        total(denominators.input_tokens_per_operation),
        total(denominators.token_work_per_operation),
        total(denominators.text_characters_per_operation),
        total(denominators.macro_calls_per_operation),
        total(denominators.definition_calls_per_operation),
        total(denominators.definition_body_words_per_operation),
        work.fuel_charges,
        work.token_frame_steps,
        work.expanded_deliveries,
        work.meaning_lookups,
        work.scanner_tokens,
        work.write_expansions,
        raw[0],
        raw[1],
        raw[2],
        raw[3],
        optional_u64(receipt.structural.macro_expansions),
        optional_u64(receipt.structural.definition_direct_stores),
        optional_u64(receipt.structural.definition_chunk_transitions),
        optional_u64(receipt.structural.definition_episode_admissions),
    );
}

fn evidence_field(evidence: Option<Evidence>, select: impl FnOnce(Evidence) -> u64) -> String {
    evidence.map_or_else(|| "null".to_owned(), |value| select(value).to_string())
}

fn optional_u64(value: Option<u64>) -> String {
    value.map_or_else(|| "null".to_owned(), |value| value.to_string())
}
