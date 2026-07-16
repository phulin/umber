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
fn default_policy_records_generation_paragraphs_only() {
    let policy = PureMemoConfig::default().recording;
    assert!(policy.paragraphs);
    assert!(!policy.pretolerance);
    assert!(!policy.pages);
    assert!(!policy.shipouts);
}

#[test]
fn forced_candidate_collision_compares_strong_key() {
    let mut runtime = PureMemoRuntime::default();
    runtime.enable(PureMemoConfig {
        recording: PureMemoRecordingPolicy::all(),
        ..PureMemoConfig::default()
    });
    let left = PureMemoKey::new(1, 7, ContentHash::from_bytes(b"left"));
    let right = PureMemoKey::new(1, 7, ContentHash::from_bytes(b"right"));
    runtime.insert_pretolerance(left, plan(3));

    assert!(runtime.lookup_pretolerance(right).is_none());
    assert_eq!(runtime.lookup_pretolerance(left), Some(plan(3)));
}

#[test]
fn budget_admission_preserves_entries_until_first_reuse_opportunity() {
    let mut runtime = PureMemoRuntime::default();
    runtime.enable(PureMemoConfig {
        max_entries: 1,
        max_retained_bytes: usize::MAX,
        recording: PureMemoRecordingPolicy::all(),
    });
    let first = PureMemoKey::new(1, 1, ContentHash::from_bytes(b"first"));
    let second = PureMemoKey::new(1, 2, ContentHash::from_bytes(b"second"));
    runtime.insert_pretolerance(first, plan(1));
    let charged = runtime.stats().retained_bytes;
    assert!(charged > 0);
    runtime.insert_pretolerance(second, plan(2));
    assert_eq!(runtime.stats().retained_entries, 1);
    assert_eq!(runtime.stats().evictions, 0);
    assert_eq!(runtime.lookup_pretolerance(first), Some(plan(1)));
    assert!(runtime.lookup_pretolerance(second).is_none());
    runtime.disable();
    assert_eq!(runtime.stats().retained_bytes, 0);
}

#[test]
fn deterministic_clock_evicts_only_entries_that_received_a_reuse_opportunity() {
    let mut runtime = PureMemoRuntime::default();
    runtime.enable(PureMemoConfig {
        max_entries: 2,
        max_retained_bytes: usize::MAX,
        recording: PureMemoRecordingPolicy::all(),
    });
    let first = PureMemoKey::new(1, 1, ContentHash::from_bytes(b"first"));
    let second = PureMemoKey::new(1, 2, ContentHash::from_bytes(b"second"));
    let third = PureMemoKey::new(1, 3, ContentHash::from_bytes(b"third"));
    runtime.insert_pretolerance(first, plan(1));
    runtime.insert_pretolerance(second, plan(2));
    assert_eq!(runtime.lookup_pretolerance(first), Some(plan(1)));
    runtime.insert_pretolerance(third, plan(3));

    assert!(runtime.lookup_pretolerance(first).is_none());
    assert_eq!(runtime.lookup_pretolerance(second), Some(plan(2)));
    assert_eq!(runtime.lookup_pretolerance(third), Some(plan(3)));
    let stats = runtime.stats();
    assert_eq!(stats.retained_entries, 2);
    assert_eq!(stats.evictions, 1);
    assert_eq!(stats.pretolerance_evictions, 1);
    assert_eq!(stats.pretolerance_retained_bytes, stats.retained_bytes);
}
