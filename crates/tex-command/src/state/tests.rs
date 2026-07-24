use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::{CommandRuntime, CommandState, MeaningCacheEntry, NormalizedLineCacheEntry};

fn semantic_hash(state: &CommandState) -> u64 {
    let mut hasher = DefaultHasher::new();
    state.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn rebuilding_runtime_does_not_change_semantic_state() {
    let state = CommandState::default();
    let original = state.clone();
    let original_hash = semantic_hash(&state);

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
    assert!(runtime.meaning_cache.entries.is_empty());
    assert!(runtime.normalized_lines.entries.is_empty());
    assert!(runtime.transient_pool.buffers.is_empty());
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
}
