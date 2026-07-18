use super::*;
use crate::{DependencyBank, DependencyValue};

fn observation_region(ordinal: u32) -> RecordedParagraphRegion {
    RecordedParagraphRegion {
        starting_span: None,
        starting_root_span: None,
        starting_input: None,
        starting_input_identity: None,
        ending_span: None,
        consumed_spans: Arc::from([]),
        delivered_tokens: 0,
        dependency_ordinals: Arc::from([ordinal]),
        dependency_observations: None,
        mutation_entry_in_group: false,
        mutations: Arc::from([]),
        effects: Arc::from([]),
        ending_input: crate::InputSummary::default(),
        input_transition_common_frames: 0,
        input_provenance: ParagraphProvenanceRecipe::default(),
        input_origin_list_lengths: Arc::from([]),
        input_suffix_token_lists: Arc::from([]),
        barriers: Arc::from([]),
        break_dependency_ordinals: Arc::from([]),
        break_prev_graf: None,
        lines: None,
        line_count: 0,
        line_last_badness: 0,
        display_active_directions: None,
        line_provenance: ParagraphLineProvenance::Pending,
    }
}

#[test]
fn accepted_paragraphs_keep_generation_local_observation_tables() {
    let key = DependencyKey::Cell {
        bank: DependencyBank::Count,
        index: 7,
    };
    let mut universe = crate::Universe::new();
    let initial_stamp = universe.track_dependency(key);
    let mut runtime = PureMemoRuntime::default();
    runtime.enable(PureMemoConfig::default());

    runtime.begin_paragraph_history(false);
    let initial = runtime.record_paragraph_observation(ObservedDependency {
        key,
        changed_at: initial_stamp,
        value: DependencyValue::Integer(0),
    });
    runtime.record_paragraph_region(observation_region(initial));
    runtime.accept_paragraph_history(universe.paragraph_origin_resolver());
    let carried = runtime.accepted_paragraphs()[0].clone();
    let initial_table = Arc::clone(
        carried
            .dependency_observations
            .as_ref()
            .expect("accepted paragraph has an observation table"),
    );

    universe.set_count(7, 41);
    let changed_stamp = universe.track_dependency(key);
    assert!(changed_stamp > initial_stamp);
    runtime.begin_paragraph_history(true);
    runtime.record_paragraph_region(carried);
    let changed = runtime.record_paragraph_observation(ObservedDependency {
        key,
        changed_at: changed_stamp,
        value: DependencyValue::Integer(41),
    });
    runtime.record_paragraph_region(observation_region(changed));
    runtime.accept_paragraph_history(universe.paragraph_origin_resolver());

    let accepted = runtime.accepted_paragraphs();
    assert_eq!(accepted.len(), 2);
    let carried_table = accepted[0]
        .dependency_observations
        .as_ref()
        .expect("carried paragraph keeps its table");
    let changed_table = accepted[1]
        .dependency_observations
        .as_ref()
        .expect("new paragraph receives the new table");
    assert!(Arc::ptr_eq(&initial_table, carried_table));
    assert!(!Arc::ptr_eq(carried_table, changed_table));
    assert_eq!(
        accepted[0]
            .dependencies()
            .next()
            .expect("carried observation")
            .changed_at,
        initial_stamp
    );
    assert_eq!(
        accepted[1]
            .dependencies()
            .next()
            .expect("new observation")
            .changed_at,
        changed_stamp
    );
}

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
