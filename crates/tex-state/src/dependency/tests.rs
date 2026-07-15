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
    assert!(!runtime.is_recording());

    runtime.begin_region();
    runtime.record(DependencyKey::Meaning(1), DependencyValue::Integer(2));
    runtime.record(DependencyKey::Meaning(1), DependencyValue::Integer(2));
    assert_eq!(runtime.finish_region().len(), 1);
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
fn aggregate_region_validates_after_change_and_restore() {
    let key = DependencyKey::Cell {
        bank: DependencyBank::Count,
        index: 12,
    };
    let mut universe = crate::Universe::new();
    universe.begin_dependency_region();
    universe.record_dependency(key, DependencyValue::Integer(5));
    let mut observations = universe.finish_dependency_region();

    universe.mark_dependency_changed(key);
    assert!(universe.validate_dependencies(&mut observations, |_| { DependencyValue::Integer(5) }));
    assert_eq!(
        observations[0].changed_at,
        universe.dependency_changed_at(key)
    );

    universe.mark_dependency_changed(key);
    assert!(
        !universe.validate_dependencies(&mut observations, |_| { DependencyValue::Integer(6) })
    );
}
