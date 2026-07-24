use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::conditionals::ConditionStack;
use crate::input::InputState;
use crate::macro_call::ParameterState;
use crate::processor::{AlignmentDeliveryState, ExpansionState, ScannerState};
use crate::profile::CommandProfile;

use super::{
    CommandRuntime, CommandState, MeaningCacheEntry, NormalizedLineCacheEntry, TransientState,
};

fn semantic_hash(state: &CommandState) -> u64 {
    let mut hasher = DefaultHasher::new();
    state.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn rebuilding_runtime_does_not_change_semantic_state() {
    let mut state = CommandState::default();
    state.input.next_level_identity = 3;
    state.parameters.activations.reserve(5);
    state.conditions.next_identity = 7;
    state.alignment.align_state = 9;
    state.expansion.cumulative_expansions = 11;
    state.transient.next_builder_identity = 13;
    let original = state.clone();
    let original_hash = semantic_hash(&state);
    let original_summary = state
        .publish_summary()
        .expect("the populated state is quiescent");

    let mut runtime = CommandRuntime::default();
    runtime.meaning_cache.entries.push(MeaningCacheEntry {
        identity: 7,
        generation: 11,
    });
    runtime
        .normalized_lines
        .entries
        .push(NormalizedLineCacheEntry {
            content_identity: 13,
            normalized: b"normalized".to_vec(),
        });
    runtime.transient_pool.buffers.push(Vec::new());
    runtime.profiling.raw_deliveries = 17;
    runtime.profiling.cache_hits = 19;

    runtime = CommandRuntime::default();

    assert_eq!(state, original);
    assert_eq!(semantic_hash(&state), original_hash);
    assert_eq!(
        state
            .publish_summary()
            .expect("runtime replacement cannot change quiescence"),
        original_summary
    );
    assert!(runtime.meaning_cache.entries.is_empty());
    assert!(runtime.normalized_lines.entries.is_empty());
    assert!(runtime.transient_pool.buffers.is_empty());
}

#[test]
fn semantic_ownership_domains_are_exhaustively_classified() {
    let CommandState {
        input,
        parameters,
        scanner,
        conditions,
        alignment,
        expansion,
        transient,
    } = CommandState::default();

    let InputState {
        levels,
        next_level_identity,
        next_source_identity,
    } = input;
    let ParameterState { activations } = parameters;
    let ScannerState {
        status,
        warning_identity,
    } = scanner;
    let ConditionStack {
        frames,
        next_identity,
    } = conditions;
    let AlignmentDeliveryState {
        align_state,
        suspended,
        active_cell,
    } = alignment;
    let ExpansionState {
        cumulative_expansions,
        next_resource_resolution,
        pending_diagnostics,
        observed_dependencies,
        semantic_barriers,
        profile,
    } = expansion;
    let CommandProfile {
        dialect,
        characters,
    } = profile;
    let TransientState {
        builders,
        rollback_roots,
        next_builder_identity,
        active_expansion_depth,
    } = transient;

    drop((
        levels,
        next_level_identity,
        next_source_identity,
        activations,
        status,
        warning_identity,
        frames,
        next_identity,
        align_state,
        suspended,
        active_cell,
        cumulative_expansions,
        next_resource_resolution,
        pending_diagnostics,
        observed_dependencies,
        semantic_barriers,
        dialect,
        characters,
        builders,
        rollback_roots,
        next_builder_identity,
        active_expansion_depth,
    ));
}

#[test]
fn default_state_is_quiescent() {
    let state = CommandState::default();

    assert!(state.input.levels.is_empty());
    assert!(state.parameters.activations.is_empty());
    assert!(state.conditions.frames.is_empty());
    assert!(state.alignment.suspended.is_empty());
    assert!(state.alignment.active_cell.is_none());
    assert!(state.expansion.pending_diagnostics.is_empty());
    assert!(state.transient.builders.is_empty());
    assert_eq!(state.transient.active_expansion_depth, 0);
}
