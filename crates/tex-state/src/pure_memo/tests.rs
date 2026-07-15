use super::*;
use crate::{DetachedArtifact, MemoValueError};

fn value(bytes: &[u8]) -> Result<DetachedMemoValue, MemoValueError> {
    DetachedMemoValue::from_artifact(&DetachedArtifact {
        artifact_schema: 1,
        payload: bytes.to_vec(),
    })
}

#[test]
fn forced_candidate_collision_compares_strong_key() {
    let mut runtime = PureMemoRuntime::default();
    runtime.enable(PureMemoConfig::default());
    let left = PureMemoKey::new(1, 7, ContentHash::from_bytes(b"left"));
    let right = PureMemoKey::new(1, 7, ContentHash::from_bytes(b"right"));
    runtime.insert(left, value(b"left-value").expect("left value"));

    assert!(runtime.lookup(right).is_none());
    assert_eq!(
        runtime.lookup(left).expect("verified hit").integrity(),
        value(b"left-value").expect("comparison value").integrity()
    );
}

#[test]
fn eviction_and_disable_release_detached_values() {
    let mut runtime = PureMemoRuntime::default();
    runtime.enable(PureMemoConfig {
        max_entries: 1,
        max_retained_bytes: usize::MAX,
    });
    let first = PureMemoKey::new(1, 1, ContentHash::from_bytes(b"first"));
    let second = PureMemoKey::new(1, 2, ContentHash::from_bytes(b"second"));
    runtime.insert(first, value(b"first").expect("first value"));
    let charged = runtime.stats().retained_bytes;
    assert!(charged > 0);
    runtime.insert(second, value(b"second").expect("second value"));
    assert_eq!(runtime.stats().retained_entries, 1);
    assert_eq!(runtime.stats().evictions, 1);
    runtime.disable();
    assert_eq!(runtime.stats().retained_bytes, 0);
}
