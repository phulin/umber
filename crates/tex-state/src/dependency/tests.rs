use super::*;
use crate::cell::{BankTag, CellId};

fn cell(bank: BankTag, index: u32) -> DependencyKey {
    DependencyKey::Cell(CellId::new(bank, index))
}

fn meaning(index: u32) -> DependencyKey {
    cell(BankTag::Meaning, index)
}

fn key_matrix() -> Vec<DependencyKey> {
    let hash = ContentHash::from_bytes(b"dependency");
    let mut keys = vec![
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
        DependencyKey::PageDimension(0),
        DependencyKey::PageInteger(1),
        DependencyKey::PageMark(2),
        DependencyKey::PageMarkClass { mark: 3, class: 4 },
        DependencyKey::Query {
            domain: 7,
            identity: 8,
        },
    ];
    for bank in [
        BankTag::Meaning,
        BankTag::Count,
        BankTag::Dimen,
        BankTag::Skip,
        BankTag::Toks,
        BankTag::Box,
        BankTag::IntParam,
        BankTag::DimenParam,
        BankTag::GlueParam,
        BankTag::TokParam,
        BankTag::Muskip,
        BankTag::FontDimen,
        BankTag::FontParamLen,
        BankTag::FontHyphenChar,
        BankTag::FontSkewChar,
        BankTag::CurrentFont,
        BankTag::MathFamilyFont,
        BankTag::PdfLpCode,
        BankTag::PdfRpCode,
        BankTag::PdfEfCode,
        BankTag::PdfTagCode,
        BankTag::PdfKnbsCode,
        BankTag::PdfStbsCode,
        BankTag::PdfShbsCode,
        BankTag::PdfKnbcCode,
        BankTag::PdfKnacCode,
        BankTag::PdfNoLigatures,
    ] {
        keys.push(cell(bank, 0));
    }
    for table in [
        DependencyCodeTable::Catcode,
        DependencyCodeTable::Lccode,
        DependencyCodeTable::Uccode,
        DependencyCodeTable::Sfcode,
        DependencyCodeTable::Mathcode,
        DependencyCodeTable::Delcode,
    ] {
        keys.push(DependencyKey::Code { table, scalar: 65 });
        keys.push(DependencyKey::CodeGeneration(table));
    }
    for field in [
        DependencyFontField::Identifier,
        DependencyFontField::Name,
        DependencyFontField::Parameter,
        DependencyFontField::ParameterCount,
        DependencyFontField::Parameters,
        DependencyFontField::HyphenChar,
        DependencyFontField::SkewChar,
        DependencyFontField::Metrics,
        DependencyFontField::PdfCode,
        DependencyFontField::PdfShaping,
    ] {
        keys.push(DependencyKey::Font {
            field,
            font: 3,
            index: 4,
        });
    }
    for field in [
        DependencyEngineField::Mode,
        DependencyEngineField::InnerMode,
        DependencyEngineField::GroupLevel,
        DependencyEngineField::GroupType,
        DependencyEngineField::ConditionLevel,
        DependencyEngineField::ConditionType,
        DependencyEngineField::ConditionBranch,
        DependencyEngineField::ConditionStack,
        DependencyEngineField::LastNodeType,
        DependencyEngineField::ParShape,
        DependencyEngineField::PenaltyArrays,
        DependencyEngineField::InteractionMode,
        DependencyEngineField::PdfTimer,
        DependencyEngineField::PdfRandom,
        DependencyEngineField::PdfShellEscape,
        DependencyEngineField::PageInsertions,
        DependencyEngineField::PdfExternalImages,
        DependencyEngineField::PdfObjects,
        DependencyEngineField::PdfPositions,
        DependencyEngineField::PdfForms,
        DependencyEngineField::PdfPages,
    ] {
        keys.push(DependencyKey::Engine(field));
    }
    for field in [
        DependencyPageField::Contents,
        DependencyPageField::Contributions,
        DependencyPageField::CurrentPage,
        DependencyPageField::Insertions,
        DependencyPageField::Discards,
        DependencyPageField::SplitDiscards,
        DependencyPageField::BreakState,
        DependencyPageField::FireUp,
    ] {
        keys.push(DependencyKey::Page(field));
    }
    for field in [
        DependencyWorldField::InputResource,
        DependencyWorldField::OutputStream,
        DependencyWorldField::InputStream,
        DependencyWorldField::TerminalInputCursor,
        DependencyWorldField::EffectPolicy,
        DependencyWorldField::ShellEscapePolicy,
        DependencyWorldField::JobClock,
        DependencyWorldField::Rng,
        DependencyWorldField::LoadedResources,
        DependencyWorldField::MaterializationBarrier,
    ] {
        keys.push(DependencyKey::World { field, index: 0 });
    }
    keys
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CoverageFamily {
    Meaning,
    Environment,
    FontSelector,
    Code,
    Font,
    Hyphenation,
    ImmutableInput,
    CommandInput,
    InputStream,
    Engine,
    PageScalar,
    PageRoot,
    World,
    Query,
}

fn coverage_family(key: DependencyKey) -> CoverageFamily {
    match key {
        DependencyKey::Cell(cell) => match cell.bank() {
            BankTag::Meaning => CoverageFamily::Meaning,
            BankTag::Count
            | BankTag::Dimen
            | BankTag::Skip
            | BankTag::Toks
            | BankTag::Box
            | BankTag::IntParam
            | BankTag::DimenParam
            | BankTag::GlueParam
            | BankTag::TokParam
            | BankTag::Muskip => CoverageFamily::Environment,
            BankTag::CurrentFont | BankTag::MathFamilyFont => CoverageFamily::FontSelector,
            BankTag::FontDimen
            | BankTag::FontParamLen
            | BankTag::FontHyphenChar
            | BankTag::FontSkewChar
            | BankTag::PdfLpCode
            | BankTag::PdfRpCode
            | BankTag::PdfEfCode
            | BankTag::PdfTagCode
            | BankTag::PdfKnbsCode
            | BankTag::PdfStbsCode
            | BankTag::PdfShbsCode
            | BankTag::PdfKnbcCode
            | BankTag::PdfKnacCode
            | BankTag::PdfNoLigatures => CoverageFamily::Font,
        },
        DependencyKey::Code { table, .. } | DependencyKey::CodeGeneration(table) => match table {
            DependencyCodeTable::Catcode
            | DependencyCodeTable::Lccode
            | DependencyCodeTable::Uccode
            | DependencyCodeTable::Sfcode
            | DependencyCodeTable::Mathcode
            | DependencyCodeTable::Delcode => CoverageFamily::Code,
        },
        DependencyKey::Font { field, .. } => match field {
            DependencyFontField::Identifier
            | DependencyFontField::Name
            | DependencyFontField::Parameter
            | DependencyFontField::ParameterCount
            | DependencyFontField::Parameters
            | DependencyFontField::HyphenChar
            | DependencyFontField::SkewChar
            | DependencyFontField::Metrics
            | DependencyFontField::PdfCode
            | DependencyFontField::PdfShaping => CoverageFamily::Font,
        },
        DependencyKey::HyphenationPatterns(_)
        | DependencyKey::HyphenationExceptions(_)
        | DependencyKey::HyphenationCodes(_) => CoverageFamily::Hyphenation,
        DependencyKey::InputRecord(_) | DependencyKey::PhysicalLine { .. } => {
            CoverageFamily::ImmutableInput
        }
        DependencyKey::InputLine | DependencyKey::InputStack => CoverageFamily::CommandInput,
        DependencyKey::InputStream(_) => CoverageFamily::InputStream,
        DependencyKey::Engine(field) => match field {
            DependencyEngineField::Mode
            | DependencyEngineField::InnerMode
            | DependencyEngineField::GroupLevel
            | DependencyEngineField::GroupType
            | DependencyEngineField::ConditionLevel
            | DependencyEngineField::ConditionType
            | DependencyEngineField::ConditionBranch
            | DependencyEngineField::ConditionStack
            | DependencyEngineField::LastNodeType
            | DependencyEngineField::ParShape
            | DependencyEngineField::PenaltyArrays
            | DependencyEngineField::InteractionMode
            | DependencyEngineField::PdfTimer
            | DependencyEngineField::PdfRandom
            | DependencyEngineField::PdfShellEscape
            | DependencyEngineField::PageInsertions
            | DependencyEngineField::PdfExternalImages
            | DependencyEngineField::PdfObjects
            | DependencyEngineField::PdfPositions
            | DependencyEngineField::PdfForms
            | DependencyEngineField::PdfPages => CoverageFamily::Engine,
        },
        DependencyKey::PageDimension(_)
        | DependencyKey::PageInteger(_)
        | DependencyKey::PageMark(_)
        | DependencyKey::PageMarkClass { .. } => CoverageFamily::PageScalar,
        DependencyKey::Page(field) => match field {
            DependencyPageField::Contents
            | DependencyPageField::Contributions
            | DependencyPageField::CurrentPage
            | DependencyPageField::Insertions
            | DependencyPageField::Discards
            | DependencyPageField::SplitDiscards
            | DependencyPageField::BreakState
            | DependencyPageField::FireUp => CoverageFamily::PageRoot,
        },
        DependencyKey::World { field, .. } => match field {
            DependencyWorldField::InputResource
            | DependencyWorldField::OutputStream
            | DependencyWorldField::InputStream
            | DependencyWorldField::TerminalInputCursor
            | DependencyWorldField::EffectPolicy
            | DependencyWorldField::ShellEscapePolicy
            | DependencyWorldField::JobClock
            | DependencyWorldField::Rng
            | DependencyWorldField::LoadedResources
            | DependencyWorldField::MaterializationBarrier => CoverageFamily::World,
        },
        DependencyKey::Query { .. } => CoverageFamily::Query,
    }
}

#[test]
fn observations_are_read_only_and_mutations_register_stamps() {
    let observed_key = meaning(7);
    let changed_key = meaning(8);
    let mut tracker = DependencyTracker::default();

    assert_eq!(tracker.track(observed_key), ChangedAt::NEVER);
    let observation = tracker.observe(observed_key, DependencyValue::Absent);
    assert_eq!(observation.changed_at, ChangedAt::NEVER);
    assert!(tracker.changed.is_empty());

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
fn environment_dependencies_strip_assignment_scope() {
    let local = cell(BankTag::Count, 7);
    let global = DependencyKey::Cell(CellId::new_global(BankTag::Count, 7));
    let mut tracker = DependencyTracker::default();

    let stamp = tracker.mark_changed(global);
    assert_eq!(tracker.changed_at(local), stamp);
    assert_eq!(tracker.changed.len(), 1);

    let observed = tracker.observe(global, DependencyValue::Integer(1));
    assert_eq!(observed.key, local);

    let mut region = DependencyRegion::default();
    region.record(observed);
    region.record(ObservedDependency {
        key: global,
        changed_at: stamp,
        value: DependencyValue::Integer(2),
    });
    assert_eq!(region.into_observations().len(), 1);
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
fn aggregate_page_and_pdf_projections_share_family_clocks() {
    let mut tracker = DependencyTracker::default();
    let page_stamp = tracker.mark_changed(DependencyKey::Page(DependencyPageField::FireUp));
    assert_eq!(
        tracker.changed_at(DependencyKey::Page(DependencyPageField::Contributions)),
        page_stamp
    );

    let pdf_stamp = tracker.mark_changed(DependencyKey::Engine(
        DependencyEngineField::PdfExternalImages,
    ));
    assert_eq!(
        tracker.changed_at(DependencyKey::Engine(DependencyEngineField::PdfForms)),
        pdf_stamp
    );
    assert_eq!(tracker.changed.len(), 2);
}

#[test]
fn every_documented_key_variant_is_classified_invalidated_and_backdated() {
    let keys = key_matrix();
    assert_eq!(keys.len(), 101, "coverage inventory lost a documented key");
    for key in keys {
        let _family = coverage_family(key);
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
    let parent = meaning(12);
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
fn canonical_content_observations_match_equal_bytes() {
    let left = Vec::from(&b"same semantic token list"[..]);
    let right = Vec::from(&b"same semantic token list"[..]);
    assert_eq!(
        DependencyValue::Content(ContentHash::from_bytes(&left)),
        DependencyValue::Content(ContentHash::from_bytes(&right))
    );
}

#[test]
fn disabled_runtime_does_not_retain_reads_or_allocate_a_region() {
    let mut runtime = DependencyRuntime::default();
    assert!(!runtime.is_recording());
    runtime.record(meaning(1), DependencyValue::Integer(2));
    assert_eq!(runtime.mark_changed(meaning(1)), ChangedAt::NEVER);
    assert!(runtime.tracker.changed.is_empty());
    assert!(!runtime.is_recording());

    let token = runtime.begin_region().expect("start dependency region");
    runtime.record(meaning(1), DependencyValue::Integer(2));
    runtime.record(meaning(1), DependencyValue::Integer(2));
    assert_eq!(
        runtime
            .finish_region(token)
            .expect("finish dependency region")
            .len(),
        1
    );
    assert!(runtime.mark_changed(meaning(1)) > ChangedAt::NEVER);
    assert_eq!(runtime.tracker.changed.len(), 1);
    assert!(!runtime.is_recording());
}

#[test]
fn poison_is_an_inactive_noop_and_first_reason_fails_closed() {
    let mut runtime = DependencyRuntime::default();
    runtime.poison(TrackedRegionBarrier::UnsupportedCommandState);
    assert!(!runtime.is_recording());

    let token = runtime.begin_region().expect("start dependency region");
    runtime.record(meaning(1), DependencyValue::Integer(2));
    runtime.poison(TrackedRegionBarrier::UnsupportedCommandState);
    runtime.poison(TrackedRegionBarrier::UnsupportedHostCapability);
    assert_eq!(
        runtime.finish_region(token),
        Err(DependencyRegionError::Unsupported(
            TrackedRegionBarrier::UnsupportedCommandState
        ))
    );
    assert!(!runtime.is_recording());

    let next = runtime.begin_region().expect("poison was cleared");
    assert!(runtime.finish_region(next).is_ok());
}

#[test]
fn every_documented_barrier_discards_partial_evidence_and_resets_the_recorder() {
    for barrier in [
        TrackedRegionBarrier::UnsupportedCommandState,
        TrackedRegionBarrier::UnsupportedExecutionState,
        TrackedRegionBarrier::UnsupportedWorldFact,
        TrackedRegionBarrier::IrreversibleEffect,
        TrackedRegionBarrier::UnsupportedHostCapability,
        TrackedRegionBarrier::FatalPartialCommit,
        TrackedRegionBarrier::EnvironmentTimelineChange,
    ] {
        let mut runtime = DependencyRuntime::default();
        let token = runtime.begin_region().expect("start dependency region");
        runtime.record(meaning(1), DependencyValue::Integer(2));
        runtime.poison(barrier);
        assert_eq!(
            runtime.finish_region(token),
            Err(DependencyRegionError::Unsupported(barrier))
        );
        assert!(!runtime.is_recording());

        let clean = runtime.begin_region().expect("start replacement region");
        assert_eq!(
            runtime.finish_region(clean),
            Ok(Vec::new()),
            "partial evidence leaked after {barrier:?}"
        );
    }
}

#[test]
fn tracked_region_orders_reads_and_uses_live_final_write_values() {
    let mut universe = crate::Universe::new();
    let mark = universe
        .begin_tracked_region()
        .expect("start tracked region");

    universe.set_count(7, 1);
    universe.observe_semantic_dependency(cell(BankTag::Count, 7));
    universe.set_count(3, 4);
    universe.set_count(7, 9);

    let record = universe
        .finish_tracked_region(mark)
        .expect("finish tracked region");
    assert_eq!(record.observations().len(), 1);
    assert_eq!(record.observations()[0].value, DependencyValue::Integer(1));
    assert_eq!(record.environment_writes().len(), 2);
    assert_eq!(
        record.environment_writes()[0].cell(),
        CellId::new(BankTag::Count, 3)
    );
    assert_eq!(
        record.environment_writes()[1].cell(),
        CellId::new(BankTag::Count, 7)
    );
    assert_eq!(
        record.environment_writes()[1].value(),
        &DependencyValue::Integer(9),
        "the first-write journal redo word must not determine the final value"
    );
}

#[test]
fn tracked_environment_write_projection_is_release_safe() {
    let mut universe = crate::Universe::new();
    let mark = universe
        .begin_tracked_region()
        .expect("start tracked region");
    universe.set_count(7, 9);

    // This specifically exercises optimized builds: projecting the final
    // value uses an ordinary dependency-aware getter, so the projection guard
    // must suppress recursive observation even when debug assertions are off.
    let record = universe
        .finish_tracked_region(mark)
        .expect("finish tracked environment write");
    assert!(record.observations().is_empty());
    assert_eq!(record.environment_writes().len(), 1);
    assert_eq!(
        record.environment_writes()[0].cell(),
        CellId::new(BankTag::Count, 7)
    );
    assert_eq!(
        record.environment_writes()[0].value(),
        &DependencyValue::Integer(9)
    );
}

#[test]
fn tracked_region_canonicalizes_grouped_assignment_scope() {
    let mut universe = crate::Universe::new();
    universe.enter_group();
    let mark = universe
        .begin_tracked_region()
        .expect("start tracked region");
    universe.set_count(12, 1);
    universe.set_count_global(12, 2);

    let record = universe
        .finish_tracked_region(mark)
        .expect("finish grouped tracked region");
    assert_eq!(record.environment_writes().len(), 1);
    assert_eq!(
        record.environment_writes()[0].cell(),
        CellId::new(BankTag::Count, 12)
    );
    assert_eq!(
        record.environment_writes()[0].value(),
        &DependencyValue::Integer(2)
    );
    let _ = universe.leave_group();
}

#[test]
fn rollback_invalidates_region_and_discards_recorder_atomically() {
    let mut universe = crate::Universe::new();
    let snapshot = universe.snapshot();
    let mark = universe
        .begin_tracked_region()
        .expect("start tracked region");
    universe.set_count(12, 1);
    universe.record_dependency(meaning(1), DependencyValue::Absent);
    universe.rollback(&snapshot);

    assert_eq!(
        universe.finish_tracked_region(mark),
        Err(crate::TrackedRegionError::UnsupportedTimelineChange)
    );
    assert!(!universe.dependency_region_is_active());
    let next = universe
        .begin_tracked_region()
        .expect("recorder was cleared");
    let empty = universe
        .finish_tracked_region(next)
        .expect("finish replacement region");
    assert!(empty.observations().is_empty());
    assert!(empty.environment_writes().is_empty());
}

#[test]
fn group_exit_across_region_mark_fails_closed() {
    let mut universe = crate::Universe::new();
    let mark = universe
        .begin_tracked_region()
        .expect("start tracked region");
    universe.enter_group();
    universe.set_count(12, 1);
    let _ = universe.leave_group();

    assert_eq!(
        universe.finish_tracked_region(mark),
        Err(crate::TrackedRegionError::UnsupportedTimelineChange)
    );
    assert!(!universe.dependency_region_is_active());
}

#[test]
fn nested_begin_and_abandon_are_typed_and_leave_no_observations() {
    let mut universe = crate::Universe::new();
    let mark = universe
        .begin_tracked_region()
        .expect("start tracked region");
    assert!(matches!(
        universe.begin_tracked_region(),
        Err(crate::TrackedRegionError::AlreadyActive)
    ));
    universe.record_dependency(meaning(1), DependencyValue::Absent);
    universe
        .abandon_tracked_region(mark)
        .expect("abandon tracked region");
    assert!(!universe.dependency_region_is_active());

    universe.record_dependency(meaning(2), DependencyValue::Absent);
    let next = universe.begin_tracked_region().expect("start clean region");
    let record = universe
        .finish_tracked_region(next)
        .expect("finish clean region");
    assert!(record.observations().is_empty());
}

#[test]
fn final_write_values_are_allocation_independent() {
    use crate::token::Token;

    let mut left = crate::Universe::new();
    let mut right = crate::Universe::new();
    let _unrelated = left.intern_token_list(&[Token::param(1)]);
    let left_value = left.intern_token_list(&[Token::param(2)]);
    let right_value = right.intern_token_list(&[Token::param(2)]);

    let left_mark = left.begin_tracked_region().expect("start left region");
    let right_mark = right.begin_tracked_region().expect("start right region");
    left.set_toks(4, left_value);
    right.set_toks(4, right_value);
    let left_record = left
        .finish_tracked_region(left_mark)
        .expect("finish left region");
    let right_record = right
        .finish_tracked_region(right_mark)
        .expect("finish right region");

    assert_eq!(
        left_record.environment_writes(),
        right_record.environment_writes()
    );
}

#[test]
fn universe_facade_records_and_invalidates_across_rollback() {
    let key = DependencyKey::World {
        field: DependencyWorldField::Rng,
        index: 0,
    };
    let mut universe = crate::Universe::new();
    let mark = universe
        .begin_tracked_region()
        .expect("start tracked region");
    universe.record_dependency(key, DependencyValue::Unsigned(7));
    let record = universe
        .finish_tracked_region(mark)
        .expect("finish tracked region");
    let observations = record.observations();
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
    let restored = cell(BankTag::Count, 12);
    let unrelated = cell(BankTag::Count, 13);
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
    let changed = meaning(1);
    let unrelated = meaning(2);
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
    let key = cell(BankTag::Count, 12);
    let mut universe = crate::Universe::new();
    let mark = universe
        .begin_tracked_region()
        .expect("start tracked region");
    universe.record_dependency(key, DependencyValue::Integer(0));
    let mut observations = universe
        .finish_tracked_region(mark)
        .expect("finish tracked region")
        .observations()
        .to_vec();

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
fn readonly_region_combines_stamp_fast_path_and_semantic_fallback() {
    let key = cell(BankTag::Count, 12);
    let mut universe = crate::Universe::new();
    let mark = universe
        .begin_tracked_region()
        .expect("start tracked region");
    universe.record_dependency(key, DependencyValue::Integer(0));
    let observations = universe
        .finish_tracked_region(mark)
        .expect("finish tracked region")
        .observations()
        .to_vec();

    let mut reads = 0;
    assert_eq!(
        universe.validate_dependencies_with_failure_readonly(&observations, |_| {
            reads += 1;
            DependencyValue::Integer(0)
        }),
        None
    );
    assert_eq!(reads, 0, "matching stamps must not read semantic state");

    universe.set_count(12, 5);
    assert_eq!(
        universe.validate_dependencies_with_failure_readonly(&observations, |_| {
            reads += 1;
            DependencyValue::Integer(5)
        }),
        Some(key)
    );
    assert_eq!(reads, 1);

    universe.set_count(12, 0);
    assert_eq!(
        universe.validate_dependencies_with_failure_readonly(&observations, |_| {
            reads += 1;
            DependencyValue::Integer(0)
        }),
        None
    );
    assert_eq!(reads, 2);
    assert_ne!(
        observations[0].changed_at,
        universe.dependency_changed_at(key),
        "read-only validation must not backdate shared observations"
    );
}

#[test]
fn aggregate_mutation_barriers_advance_exact_registered_facts() {
    use crate::page::PageDimension;
    use crate::scaled::Scaled;
    use crate::token::Catcode;

    let count = cell(BankTag::Count, 7);
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
    let mark = universe
        .begin_tracked_region()
        .expect("start tracked region");
    for key in [count, catcode, generation, page, world] {
        universe.record_dependency(key, DependencyValue::Absent);
    }
    let _ = universe
        .finish_tracked_region(mark)
        .expect("finish tracked region");

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
    let _ = universe.world_mut().next_random_u64();
    assert!(universe.dependency_changed_at(world) > before_world);
}

#[test]
fn equal_environment_writes_and_restores_do_not_advance_stamps() {
    let count = cell(BankTag::Count, 300);
    let meaning = cell(BankTag::Meaning, 0);
    let box_register = cell(BankTag::Box, 300);
    let current_font = cell(BankTag::CurrentFont, 0);
    let hyphen_char = DependencyKey::Font {
        field: DependencyFontField::HyphenChar,
        font: 0,
        index: 0,
    };
    let mut universe = crate::Universe::new();
    for key in [count, meaning, box_register, current_font, hyphen_char] {
        universe.track_dependency(key);
        assert_eq!(universe.dependency_changed_at(key), ChangedAt::NEVER);
    }

    universe.set_count(300, 0);
    let symbol = universe.intern("receipt-equal-meaning");
    universe.track_dependency(cell(BankTag::Meaning, symbol.symbol().raw()));
    universe.set_meaning(symbol, crate::meaning::Meaning::Undefined);
    universe.clear_box_reg_global(300);
    universe.set_current_font(universe.current_font());
    universe.set_font_hyphen_char(
        crate::font::NULL_FONT,
        universe.font_hyphen_char(crate::font::NULL_FONT),
    );
    for key in [count, box_register, current_font, hyphen_char] {
        assert_eq!(universe.dependency_changed_at(key), ChangedAt::NEVER);
    }
    assert_eq!(
        universe.dependency_changed_at(cell(BankTag::Meaning, symbol.symbol().raw())),
        ChangedAt::NEVER
    );

    universe.enter_group();
    universe.set_count(300, 0);
    let before_exit = universe.dependency_changed_at(count);
    let _ = universe.leave_group();
    assert_eq!(universe.dependency_changed_at(count), before_exit);

    let snapshot = universe.snapshot();
    universe.set_count(300, 0);
    universe.rollback(&snapshot);
    assert_eq!(universe.dependency_changed_at(count), before_exit);
}
