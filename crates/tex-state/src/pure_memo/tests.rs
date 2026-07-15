use super::*;
fn plan(position: usize) -> Option<PureBreakPlan> {
    Some(PureBreakPlan {
        breaks: vec![PureBreakDecision {
            position,
            penalty: 0,
            hyphenated: false,
        }],
        demerits: 100,
        last_line_fill: None,
    })
}

#[test]
fn forced_candidate_collision_compares_strong_key() {
    let mut runtime = PureMemoRuntime::default();
    runtime.enable(PureMemoConfig::default());
    let left = PureMemoKey::new(1, 7, ContentHash::from_bytes(b"left"));
    let right = PureMemoKey::new(1, 7, ContentHash::from_bytes(b"right"));
    runtime.insert_pretolerance(left, plan(3));

    assert!(runtime.lookup_pretolerance(right).is_none());
    assert_eq!(runtime.lookup_pretolerance(left), Some(plan(3)));
}

#[test]
fn eviction_and_disable_release_typed_values() {
    let mut runtime = PureMemoRuntime::default();
    runtime.enable(PureMemoConfig {
        max_entries: 1,
        max_retained_bytes: usize::MAX,
    });
    let first = PureMemoKey::new(1, 1, ContentHash::from_bytes(b"first"));
    let second = PureMemoKey::new(1, 2, ContentHash::from_bytes(b"second"));
    runtime.insert_pretolerance(first, plan(1));
    let charged = runtime.stats().retained_bytes;
    assert!(charged > 0);
    runtime.insert_pretolerance(second, plan(2));
    assert_eq!(runtime.stats().retained_entries, 1);
    assert_eq!(runtime.stats().evictions, 1);
    runtime.disable();
    assert_eq!(runtime.stats().retained_bytes, 0);
}
