use super::*;

fn key_matrix() -> Vec<DependencyKey> {
    let hash = ContentHash::from_bytes(b"dependency");
    vec![
        DependencyKey::Meaning(1),
        DependencyKey::Cell {
            bank: DependencyBank::Count,
            index: 2,
        },
        DependencyKey::Code {
            table: DependencyCodeTable::Catcode,
            scalar: 65,
        },
        DependencyKey::CodeGeneration(DependencyCodeTable::Lccode),
        DependencyKey::Font {
            field: DependencyFontField::Metrics,
            font: 3,
            index: 4,
        },
        DependencyKey::HyphenationPatterns(1),
        DependencyKey::HyphenationExceptions(2),
        DependencyKey::HyphenationCodes(3),
        DependencyKey::InputRecord(hash),
        DependencyKey::PhysicalLine {
            content: hash,
            terminator: 1,
        },
        DependencyKey::InputLine,
        DependencyKey::InputStream(4),
        DependencyKey::InputStack,
        DependencyKey::Engine(DependencyEngineField::Mode),
        DependencyKey::PageDimension(0),
        DependencyKey::PageInteger(1),
        DependencyKey::PageMark(2),
        DependencyKey::PageMarkClass { mark: 3, class: 4 },
        DependencyKey::Page(DependencyPageField::CurrentPage),
        DependencyKey::World {
            field: DependencyWorldField::Rng,
            index: 0,
        },
        DependencyKey::Query {
            domain: 7,
            identity: 8,
        },
    ]
}

#[test]
fn observations_are_read_only_and_mutations_register_stamps() {
    let observed_key = DependencyKey::Meaning(7);
    let changed_key = DependencyKey::Meaning(8);
    let mut tracker = DependencyTracker::default();
    let shared = Arc::clone(&tracker.changed);

    assert_eq!(tracker.track(observed_key), ChangedAt::NEVER);
    let observation = tracker.observe(observed_key, DependencyValue::Absent);
    assert_eq!(observation.changed_at, ChangedAt::NEVER);
    assert!(tracker.changed.is_empty());
    assert!(Arc::ptr_eq(&shared, &tracker.changed));

    let stamp = tracker.mark_changed(changed_key);
    assert!(stamp > ChangedAt::NEVER);
    assert_eq!(tracker.changed_at(changed_key), stamp);
    assert_eq!(tracker.changed.len(), 1);

    let before_global = tracker.changed_at(observed_key);
    tracker.invalidate_all();
    assert!(tracker.changed_at(observed_key) > before_global);
    assert_eq!(tracker.changed.len(), 1);
}

#[test]
fn scalar_code_stamps_share_one_table_generation_entry() {
    let mut tracker = DependencyTracker::default();
    let first = DependencyKey::Code {
        table: DependencyCodeTable::Catcode,
        scalar: 'a' as u32,
    };
    let second = DependencyKey::Code {
        table: DependencyCodeTable::Catcode,
        scalar: 'z' as u32,
    };
    let generation = DependencyKey::CodeGeneration(DependencyCodeTable::Catcode);

    let stamp = tracker.mark_changed(first);
    assert_eq!(tracker.changed_at(first), stamp);
    assert_eq!(tracker.changed_at(second), stamp);
    assert_eq!(tracker.changed_at(generation), stamp);
    assert_eq!(tracker.changed.len(), 1);
}

#[test]
fn every_key_variant_is_independently_invalidated_and_backdated() {
    for key in key_matrix() {
        let unrelated = DependencyKey::Query {
            domain: 99,
            identity: key_matrix().len() as u64,
        };
        let mut tracker = DependencyTracker::default();
        let value = DependencyValue::Projection {
            schema: 1,
            fingerprint: 42,
        };
        let mut observed = tracker.observe(key, value.clone());

        tracker.mark_changed(unrelated);
        let mut semantic_reads = 0;
        assert_eq!(
            tracker.validate(&mut observed, |_| {
                semantic_reads += 1;
                value.clone()
            }),
            DependencyValidation::Unchanged
        );
        assert_eq!(semantic_reads, 0);

        tracker.mark_changed(key);
        assert_eq!(
            tracker.validate(&mut observed, |_| value.clone()),
            DependencyValidation::Backdated
        );
        assert_eq!(
            tracker.validate(&mut observed, |_| panic!("backdated value was reread")),
            DependencyValidation::Unchanged
        );

        tracker.mark_changed(key);
        assert_eq!(
            tracker.validate(&mut observed, |_| DependencyValue::Unsigned(43)),
            DependencyValidation::Changed
        );
    }
}

#[test]
fn region_deduplication_and_nested_query_order_are_deterministic() {
    let mut tracker = DependencyTracker::default();
    let mut region = DependencyRegion::default();
    let parent = DependencyKey::Meaning(12);
    let child = DependencyKey::Query {
        domain: 2,
        identity: 9,
    };
    region.record(tracker.observe(parent, DependencyValue::Integer(1)));
    region.record(tracker.observe(
        child,
        DependencyValue::Content(ContentHash::from_bytes(b"x")),
    ));
    region.record(tracker.observe(parent, DependencyValue::Integer(999)));

    let observations = region.into_observations();
    assert_eq!(observations.len(), 2);
    assert_eq!(observations[0].key, parent);
    assert_eq!(observations[0].value, DependencyValue::Integer(1));
    assert_eq!(observations[1].key, child);
}

#[test]
fn canonical_content_observations_ignore_allocation_identity() {
    let left = Vec::from(&b"same semantic token list"[..]);
    let right = Vec::from(&b"same semantic token list"[..]);
    assert_ne!(left.as_ptr(), right.as_ptr());
    assert_eq!(
        DependencyValue::Content(ContentHash::from_bytes(&left)),
        DependencyValue::Content(ContentHash::from_bytes(&right))
    );
}

#[test]
fn disabled_runtime_does_not_retain_reads_or_allocate_a_region() {
    let mut runtime = DependencyRuntime::default();
    assert!(!runtime.is_recording());
    runtime.record(DependencyKey::Meaning(1), DependencyValue::Integer(2));
    assert_eq!(
        runtime.mark_changed(DependencyKey::Meaning(1)),
        ChangedAt::NEVER
    );
    assert!(runtime.tracker.changed.is_empty());
    assert!(!runtime.is_recording());

    runtime.begin_region();
    runtime.record(DependencyKey::Meaning(1), DependencyValue::Integer(2));
    runtime.record(DependencyKey::Meaning(1), DependencyValue::Integer(2));
    assert_eq!(runtime.finish_region().len(), 1);
    assert!(runtime.mark_changed(DependencyKey::Meaning(1)) > ChangedAt::NEVER);
    assert_eq!(runtime.tracker.changed.len(), 1);
    assert!(!runtime.is_recording());
}

#[test]
fn universe_facade_records_and_invalidates_across_rollback() {
    let key = DependencyKey::World {
        field: DependencyWorldField::Rng,
        index: 0,
    };
    let mut universe = crate::Universe::new();
    universe.begin_dependency_region();
    universe.record_dependency(key, DependencyValue::Unsigned(7));
    let observations = universe.finish_dependency_region();
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].changed_at, ChangedAt::NEVER);

    let snapshot = universe.snapshot();
    universe.mark_dependency_changed(key);
    let after_write = universe.dependency_changed_at(key);
    assert!(after_write > ChangedAt::NEVER);
    universe.rollback(&snapshot);
    assert!(universe.dependency_changed_at(key) > after_write);
}

#[test]
fn group_exit_invalidates_only_restored_facts() {
    let restored = DependencyKey::Cell {
        bank: DependencyBank::Count,
        index: 12,
    };
    let unrelated = DependencyKey::Cell {
        bank: DependencyBank::Count,
        index: 13,
    };
    let mut universe = crate::Universe::new();
    universe.track_dependency(unrelated);
    universe.enter_group();
    universe.set_count(12, 7);
    // Recording after the local assignment is the case that broad group-exit
    // invalidation used to cover and a write-time stamp alone cannot cover.
    let restored_stamp = universe.track_dependency(restored);
    let unrelated_stamp = universe.dependency_changed_at(unrelated);
    let _ = universe.leave_group();
    assert!(universe.dependency_changed_at(restored) > restored_stamp);
    assert_eq!(universe.dependency_changed_at(unrelated), unrelated_stamp);

    let mut restored_observation = ObservedDependency {
        key: restored,
        changed_at: restored_stamp,
        value: DependencyValue::Integer(7),
    };
    assert!(
        !universe.validate_dependencies(std::slice::from_mut(&mut restored_observation), |_| {
            DependencyValue::Integer(0)
        })
    );
}

#[test]
fn rollback_preserves_unrelated_stamps_and_clone_ancestry() {
    let changed = DependencyKey::Meaning(1);
    let unrelated = DependencyKey::Meaning(2);
    let mut universe = crate::Universe::new();
    let changed_before = universe.track_dependency(changed);
    let unrelated_before = universe.track_dependency(unrelated);
    let snapshot = universe.snapshot();
    universe.mark_dependency_changed(changed);
    universe.rollback(&snapshot);
    assert!(universe.dependency_changed_at(changed) > changed_before);
    assert_eq!(universe.dependency_changed_at(unrelated), unrelated_before);

    let fork = universe.clone();
    assert_eq!(
        fork.dependency_changed_at(changed),
        universe.dependency_changed_at(changed)
    );
    assert_eq!(
        fork.dependency_changed_at(unrelated),
        universe.dependency_changed_at(unrelated)
    );
}

#[test]
fn aggregate_region_validates_after_change_and_restore() {
    let key = DependencyKey::Cell {
        bank: DependencyBank::Count,
        index: 12,
    };
    let mut universe = crate::Universe::new();
    universe.begin_dependency_region();
    universe.record_dependency(key, DependencyValue::Integer(0));
    let mut observations = universe.finish_dependency_region();

    universe.set_count(13, 9);
    let mut reads = 0;
    assert!(universe.validate_dependencies(&mut observations, |_| {
        reads += 1;
        DependencyValue::Integer(0)
    }));
    assert_eq!(
        reads, 0,
        "unrelated register write missed the stamp fast path"
    );

    universe.set_count(12, 5);
    assert!(
        !universe.validate_dependencies(&mut observations, |_| { DependencyValue::Integer(5) })
    );

    universe.set_count(12, 0);
    assert!(universe.validate_dependencies(&mut observations, |_| { DependencyValue::Integer(0) }));
    assert_eq!(
        observations[0].changed_at,
        universe.dependency_changed_at(key)
    );
}

#[test]
fn aggregate_mutation_barriers_advance_exact_registered_facts() {
    use crate::page::PageDimension;
    use crate::scaled::Scaled;
    use crate::token::Catcode;

    let count = DependencyKey::Cell {
        bank: DependencyBank::Count,
        index: 7,
    };
    let catcode = DependencyKey::Code {
        table: DependencyCodeTable::Catcode,
        scalar: 'x' as u32,
    };
    let generation = DependencyKey::CodeGeneration(DependencyCodeTable::Catcode);
    let page = DependencyKey::PageDimension(PageDimension::Goal.index());
    let world = DependencyKey::World {
        field: DependencyWorldField::Rng,
        index: 0,
    };
    let mut universe = crate::Universe::new();
    universe.begin_dependency_region();
    for key in [count, catcode, generation, page, world] {
        universe.record_dependency(key, DependencyValue::Absent);
    }
    let _ = universe.finish_dependency_region();

    universe.set_count(8, 1);
    assert_eq!(universe.dependency_changed_at(count), ChangedAt::NEVER);
    universe.set_count(7, 1);
    assert!(universe.dependency_changed_at(count) > ChangedAt::NEVER);

    universe.set_catcode('x', Catcode::Letter);
    assert!(universe.dependency_changed_at(catcode) > ChangedAt::NEVER);
    assert!(universe.dependency_changed_at(generation) > ChangedAt::NEVER);

    universe.set_page_dimension(PageDimension::Goal, Scaled::from_raw(100));
    assert!(universe.dependency_changed_at(page) > ChangedAt::NEVER);

    let before_world = universe.dependency_changed_at(world);
    let _ = universe.world_mut();
    assert!(universe.dependency_changed_at(world) > before_world);
}
