use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::{
    CommandRuntime, CommandState, MeaningCacheEntry, NormalizedLineCacheEntry, TransientState,
};
use crate::conditionals::ConditionStack;
use crate::input::InputState;
use crate::macro_call::ParameterState;
use crate::processor::{AlignmentDeliveryState, ExpansionState, ScannerState};
use crate::{
    AlignmentCellTemplates, AlignmentIdentity, AlignmentLifecycleError, AlignmentRequest,
    AlignmentRequestResult,
};

fn templates() -> AlignmentCellTemplates {
    AlignmentCellTemplates {
        u_template: None,
        v_template: tex_state::input::TracedTokenList::synthetic(
            tex_state::ids::TokenListId::EMPTY,
        ),
    }
}

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
        registered_sources,
        next_level_identity,
        next_source_identity,
    } = input;
    let ParameterState { activations, .. } = parameters;
    let ScannerState { .. } = scanner;
    let ConditionStack {
        frames,
        next_identity,
    } = conditions;
    let AlignmentDeliveryState {
        align_state,
        active_alignment,
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
    let dialect = profile.dialect();
    let characters = profile.character_mode();
    let TransientState {
        builders,
        rollback_roots,
        next_builder_identity,
        active_expansion_depth,
    } = transient;

    drop((
        levels,
        registered_sources,
        next_level_identity,
        next_source_identity,
        activations,
        frames,
        next_identity,
        align_state,
        active_alignment,
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
    assert!(state.input.registered_sources.is_empty());
    assert!(state.parameters.activations.is_empty());
    assert!(state.conditions.frames.is_empty());
    assert!(state.alignment.suspended.is_empty());
    assert!(state.alignment.active_cell.is_none());
    assert!(state.expansion.pending_diagnostics.is_empty());
    assert!(state.transient.builders.is_empty());
    assert_eq!(state.transient.active_expansion_depth, 0);
}

#[test]
fn nested_alignment_suspension_restores_the_outer_cell_identity_and_templates() {
    let mut state = CommandState::default();
    let outer = AlignmentIdentity::new(41);
    let inner = AlignmentIdentity::new(43);
    let outer_templates = templates();

    state.begin_alignment(outer);
    state
        .begin_alignment_cell(outer, outer_templates)
        .expect("outer cell begins");
    state.suspend_alignment(outer).expect("outer cell suspends");
    assert_eq!(
        state.resume_alignment(inner),
        Err(AlignmentLifecycleError::WrongAlignment)
    );

    state.begin_alignment(inner);
    state
        .begin_alignment_cell(inner, templates())
        .expect("inner cell begins");
    state
        .finish_alignment(inner)
        .expect("inner alignment finishes");
    state
        .resume_alignment(outer)
        .expect("outer alignment resumes");
    assert_eq!(
        state
            .alignment
            .active_cell
            .as_ref()
            .expect("outer cell restores")
            .templates,
        outer_templates
    );
}

#[test]
fn typed_requests_preserve_nested_alignment_delivery_without_token_classification() {
    let mut state = CommandState::default();
    let outer = AlignmentIdentity::new(47);
    let inner = AlignmentIdentity::new(53);

    assert_eq!(
        state.apply_alignment_request(AlignmentRequest::Begin(outer)),
        Ok(AlignmentRequestResult::Applied)
    );
    assert_eq!(
        state.apply_alignment_request(AlignmentRequest::BeginCell {
            alignment: outer,
            templates: templates(),
        }),
        Ok(AlignmentRequestResult::Applied)
    );
    assert_eq!(
        state.apply_alignment_request(AlignmentRequest::Suspend(outer)),
        Ok(AlignmentRequestResult::Applied)
    );
    assert_eq!(
        state.apply_alignment_request(AlignmentRequest::Begin(inner)),
        Ok(AlignmentRequestResult::Applied)
    );
    assert_eq!(
        state.apply_alignment_request(AlignmentRequest::Finish(inner)),
        Ok(AlignmentRequestResult::Applied)
    );
    assert_eq!(
        state.apply_alignment_request(AlignmentRequest::Resume(outer)),
        Ok(AlignmentRequestResult::Applied)
    );
    assert_eq!(
        state
            .alignment
            .active_cell
            .as_ref()
            .expect("outer cell resumes")
            .templates,
        templates()
    );
}
