use super::*;
use crate::cell::{BankTag, CellId};

fn meaning(index: u32) -> DependencyKey {
    DependencyKey::Cell(CellId::new(BankTag::Meaning, index))
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
        keys.push(DependencyKey::Cell(CellId::new(bank, 0)));
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
    let local = DependencyKey::Cell(CellId::new(BankTag::Count, 7));
    let global = DependencyKey::Cell(CellId::new_global(BankTag::Count, 7));
    let mut tracker = DependencyTracker::default();

    let stamp = tracker.mark_changed(global);
    assert_eq!(tracker.changed_at(local), stamp);
    let observed = tracker.observe(global, DependencyValue::Integer(1));
    assert_eq!(observed.key, local);
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
fn validation_uses_stamps_then_semantic_backdating() {
    let key = meaning(12);
    let unrelated = meaning(13);
    let value = DependencyValue::Projection {
        schema: 1,
        fingerprint: 42,
    };
    let mut tracker = DependencyTracker::default();
    let mut observed = tracker.observe(key, value.clone());

    tracker.mark_changed(unrelated);
    let mut reads = 0;
    assert_eq!(
        tracker.validate(&mut observed, |_| {
            reads += 1;
            value.clone()
        }),
        DependencyValidation::Unchanged
    );
    assert_eq!(reads, 0);

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

#[test]
fn every_documented_key_variant_invalidates_and_backdates_semantically() {
    let keys = key_matrix();
    assert_eq!(keys.len(), 101, "coverage inventory lost a documented key");
    for (ordinal, key) in keys.into_iter().enumerate() {
        let unrelated = DependencyKey::Query {
            domain: 99,
            identity: ordinal as u64 + 100,
        };
        let value = DependencyValue::Projection {
            schema: 1,
            fingerprint: 42,
        };
        let mut tracker = DependencyTracker::default();
        let mut observed = tracker.observe(key, value.clone());

        tracker.mark_changed(unrelated);
        assert_eq!(
            tracker.validate(&mut observed, |_| value.clone()),
            DependencyValidation::Unchanged
        );
        tracker.mark_changed(key);
        assert_eq!(
            tracker.validate(&mut observed, |_| value.clone()),
            DependencyValidation::Backdated
        );
        tracker.mark_changed(key);
        assert_eq!(
            tracker.validate(&mut observed, |_| DependencyValue::Unsigned(43)),
            DependencyValidation::Changed
        );
    }
}

#[test]
fn runtime_region_lifecycle_is_typed_and_deduplicated() {
    let mut runtime = DependencyRuntime::default();
    assert!(!runtime.is_recording());
    assert_eq!(runtime.mark_changed(meaning(1)), ChangedAt::NEVER);

    let token = runtime.begin_region().expect("start dependency region");
    assert!(matches!(
        runtime.begin_region(),
        Err(DependencyRegionError::AlreadyActive)
    ));
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
    assert!(!runtime.is_recording());
}

#[test]
fn every_barrier_discards_partial_evidence_and_resets_the_recorder() {
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
        assert_eq!(runtime.finish_region(clean), Ok(Vec::new()));
    }
}
