use super::{
    InsertedOrigin, InsertedOriginKind, MacroInvocationOrigin, ORIGIN_KEY_LEASE_LEN,
    ORIGIN_RECORD_ARCHIVE_CHUNK, OriginKeyRuns, OriginListRef, OriginRecord, OriginRef,
    ProvenanceBudgets, ProvenanceStore, SourceOrigin, SynthesizedOrigin, SynthesizedOriginKind,
    SyntheticOrigin, SyntheticOriginKind, packed_origin_successor,
};
use crate::Universe;
use crate::input::{SourceId, TokenListReplayKind};
use crate::macro_store::MacroMeaning;
use crate::meaning::MeaningFlags;
use crate::source_map::SourceDescriptor;
use crate::token::{Catcode, OriginId, Token};
use std::sync::Arc;
use std::sync::Barrier;

#[test]
fn unknown_origin_and_empty_list_are_preallocated() {
    let store = ProvenanceStore::new();

    assert_eq!(store.get(OriginId::UNKNOWN), OriginRecord::UnknownBootstrap);
    assert_eq!(OriginListRef::empty().id(), crate::ids::OriginListId::EMPTY);
    assert!(store.contains_origin(OriginId::UNKNOWN));
    assert_eq!(store.stats().origin_records(), 0);
}

#[test]
fn packed_arena_origin_namespace_includes_its_last_payload() {
    assert_eq!(packed_origin_successor(0x7fff_fffe), Some(0x7fff_ffff));
    assert_eq!(packed_origin_successor(0x7fff_ffff), Some(0x8000_0000));
    assert_eq!(packed_origin_successor(0x8000_0000), None);
}

#[test]
fn origin_key_runs_map_gaps_and_truncate_partial_runs() {
    let mut keys = OriginKeyRuns::default();
    keys.append(10, 0);
    keys.append(11, 1);
    keys.append(15, 2);
    keys.append(16, 3);

    assert_eq!(keys.slot(10), Some(0));
    assert_eq!(keys.slot(11), Some(1));
    assert_eq!(keys.slot(12), None);
    assert_eq!(keys.slot(15), Some(2));
    assert_eq!(keys.slot(16), Some(3));

    keys.truncate(3);
    assert_eq!(keys.slot(15), Some(2));
    assert_eq!(keys.slot(16), None);
    keys.append(20, 3);
    assert_eq!(keys.slot(20), Some(3));

    keys.truncate(1);
    assert_eq!(keys.slot(10), Some(0));
    assert_eq!(keys.slot(11), None);
    assert_eq!(keys.slot(15), None);
    assert_eq!(keys.slot(20), None);
}

#[test]
fn origin_key_runs_accept_out_of_key_order_fork_imports() {
    let mut keys = OriginKeyRuns::default();
    keys.append(20, 0);
    keys.append(21, 1);
    keys.append(10, 2);
    keys.append(11, 3);

    assert_eq!(keys.slot(10), Some(2));
    assert_eq!(keys.slot(11), Some(3));
    assert_eq!(keys.slot(20), Some(0));
    assert_eq!(keys.slot(21), Some(1));

    keys.truncate(3);
    assert_eq!(keys.slot(10), Some(2));
    assert_eq!(keys.slot(11), None);
    assert_eq!(keys.slot(20), Some(0));
    assert_eq!(keys.slot(21), Some(1));
}

#[test]
fn concurrent_stores_keep_process_global_keys_in_local_affine_runs() {
    const STORES: usize = 4;
    let barrier = Arc::new(Barrier::new(STORES));
    let stores = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..STORES {
            let barrier = Arc::clone(&barrier);
            handles.push(scope.spawn(move || {
                let mut store = ProvenanceStore::new();
                for offset in 0..ORIGIN_KEY_LEASE_LEN {
                    barrier.wait();
                    store.allocate(OriginRecord::Source(SourceOrigin::new(
                        SourceId::new(7),
                        u64::from(offset),
                        1,
                        offset,
                    )));
                }
                store
            }));
        }
        handles
            .into_iter()
            .map(|handle| handle.join().expect("provenance allocator thread"))
            .collect::<Vec<_>>()
    });

    for store in &stores {
        assert_eq!(store.record_keys.runs.len(), 1);
        assert_eq!(
            store.stats().origin_records(),
            ORIGIN_KEY_LEASE_LEN as usize
        );
    }
}

#[test]
fn ordinary_and_occurrence_unique_records_share_one_affine_key_lease() {
    let mut store = ProvenanceStore::new();
    for serial in 0..32_u64 {
        let record = |operand| {
            OriginRecord::MacroInvocation(MacroInvocationOrigin::from_nonowning_operand(
                operand,
                OriginId::UNKNOWN,
                OriginId::UNKNOWN,
                OriginId::UNKNOWN,
            ))
        };
        store.allocate(record(serial * 2));
        store.allocate_unique(record(serial * 2 + 1));
    }

    let stats = store.stats();
    assert_eq!(stats.origin_records(), 64);
    assert_eq!(stats.origin_key_runs(), 1);
}

#[test]
fn structural_origin_records_allocate_and_read_back() {
    let mut store = ProvenanceStore::new();
    let source_record = OriginRecord::Source(SourceOrigin::new(SourceId::new(7), 123, 4, 9));
    let source = store.allocate_rooted(source_record, []);
    let inserted = store.allocate_rooted(
        OriginRecord::Inserted(InsertedOrigin::new(
            InsertedOriginKind::Paragraph,
            Token::Char {
                ch: 'p',
                cat: Catcode::Letter,
            },
            source.id(),
        )),
        [source.clone()],
    );
    assert_ne!(source.id(), inserted.id());
    assert_eq!(source.record(), Some(source_record));
}

#[test]
fn provenance_fork_keeps_inherited_origins_but_separates_new_keys() {
    let mut parent = ProvenanceStore::new();
    let inherited = parent.allocate(OriginRecord::Synthetic(SyntheticOrigin::new(
        SyntheticOriginKind::Test,
    )));
    let mut child = parent.clone();
    assert!(child.contains_origin(inherited));
    let parent_only = parent.allocate(OriginRecord::Synthetic(SyntheticOrigin::new(
        SyntheticOriginKind::Format,
    )));
    let child_only = child.allocate(OriginRecord::Synthetic(SyntheticOrigin::new(
        SyntheticOriginKind::Engine,
    )));
    assert_ne!(parent_only, child_only);
    assert!(!child.contains_origin(parent_only));
    assert!(!parent.contains_origin(child_only));
}

#[test]
fn all_mandatory_origin_record_kinds_round_trip() {
    let mut stores = Universe::new();
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    }]);
    let definition = stores.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, params, body));
    let source = stores.source_origin(SourceId::new(9), 88, 6, 4);
    let invocation = stores.source_origin(SourceId::new(10), 144, 8, 12);
    let macro_origin =
        stores.macro_invocation_origin(definition.id(), invocation, source, OriginId::UNKNOWN);
    let inserted = stores.inserted_origin(
        InsertedOriginKind::TokenListReplay(TokenListReplayKind::MacroBody),
        Token::param(1),
        macro_origin,
    );
    let synthesized = stores.synthesized_origin(SynthesizedOriginKind::ValueRendering, inserted);
    let synthetic = stores.synthetic_origin(SyntheticOriginKind::Test);

    assert_eq!(
        stores.origin(source),
        OriginRecord::Source(SourceOrigin::new(SourceId::new(9), 88, 6, 4))
    );
    assert_eq!(
        stores.origin(macro_origin),
        OriginRecord::MacroInvocation(MacroInvocationOrigin::from_nonowning_operand(
            stores.macro_definition_observation_operand(definition.id()) as u64,
            invocation,
            source,
            OriginId::UNKNOWN,
        ))
    );
    assert_eq!(
        stores.origin(inserted),
        OriginRecord::Inserted(InsertedOrigin::new(
            InsertedOriginKind::TokenListReplay(TokenListReplayKind::MacroBody),
            Token::param(1),
            macro_origin,
        ))
    );
    assert_eq!(
        stores.origin(synthesized),
        OriginRecord::Synthesized(SynthesizedOrigin::new(
            SynthesizedOriginKind::ValueRendering,
            inserted,
        ))
    );
    assert_eq!(
        stores.origin(synthetic),
        OriginRecord::Synthetic(SyntheticOrigin::new(SyntheticOriginKind::Test))
    );
}

#[test]
fn macro_invocation_accounting_tracks_live_parent_chains_and_rollback() {
    let mut stores = Universe::new();
    let empty = stores.intern_token_list(&[]);
    let definition = stores.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, empty));
    let definition_origin = stores.source_origin(SourceId::new(1), 0, 1, 1);
    let invocation_origin = stores.source_origin(SourceId::new(2), 10, 2, 3);
    let snapshot = stores.snapshot();
    let parent = stores.macro_invocation_origin(
        definition.id(),
        invocation_origin,
        definition_origin,
        OriginId::UNKNOWN,
    );
    let child = stores.macro_invocation_origin(
        definition.id(),
        invocation_origin,
        definition_origin,
        parent,
    );

    let stats = stores.macro_invocation_provenance_stats();
    let retention = stores.provenance_stats();
    assert_eq!(stats.invocations(), 2);
    assert!(stats.retained_bytes() > 0);
    assert!(retention.origin_record_slot_bytes() <= 64);
    assert!(
        retention.origin_record_retained_bytes() <= retention.origin_record_layout_budget_bytes()
    );
    assert_eq!(
        stores.origin(child),
        OriginRecord::MacroInvocation(MacroInvocationOrigin::from_nonowning_operand(
            stores.macro_definition_observation_operand(definition.id()) as u64,
            invocation_origin,
            definition_origin,
            parent,
        ))
    );
    assert_eq!(
        stores.macro_invocation_origin(
            definition.id(),
            invocation_origin,
            definition_origin,
            parent,
        ),
        child,
        "the inline recent-frame cache reuses an exact retry without a weak candidate"
    );
    assert_eq!(stores.origin_ref(child).map(|root| root.id()), Some(child));
    let materialized = stores
        .materialize_origin_ref(child)
        .expect("cold publication materializes an archived frame");
    assert_eq!(materialized.id(), child);
    assert_eq!(materialized.record(), Some(stores.origin(child)));
    drop(materialized);
    assert_eq!(stores.origin_ref(child).map(|root| root.id()), Some(child));

    stores.rollback(&snapshot);
    assert_eq!(stores.macro_invocation_provenance_stats().invocations(), 0);
    assert!(!retention.retained_layout_eq(stores.provenance_stats()));
}

#[test]
fn cold_materialized_sidecar_keeps_exact_source_after_archive_rollback() {
    let mut stores = Universe::new();
    let snapshot = stores.snapshot();
    let source_id = SourceId::new(17);
    stores
        .register_source(
            source_id,
            SourceDescriptor::named_generated("rolled-back.tex", Arc::from(&b"abc"[..])),
        )
        .expect("generated source registers");
    let source = stores.source_range_origin(source_id, 0, 3);
    let derived = stores.synthesized_origin(SynthesizedOriginKind::ValueRendering, source);
    let sidecar = stores
        .materialize_origin_ref(derived)
        .expect("cold boundary materializes archived coordinates");

    stores.rollback(&snapshot);
    assert!(stores.origin_if_live(derived).is_none());
    assert_eq!(
        crate::ProvenanceResolver::new(&stores)
            .resolve_origin_ref(&sidecar)
            .expect("sidecar retains its detached source registration"),
        crate::ResolvedSourceLocation {
            path: "rolled-back.tex".to_owned(),
            start: 0,
            end: 3,
            line: 1,
            column: 1,
        }
    );
}

#[test]
fn origin_record_layout_budget_covers_tail_and_chunk_growth() {
    let mut store = ProvenanceStore::new();
    let empty = store.stats();
    assert_eq!(empty.origin_record_archive_chunk_slots(), 1024);
    assert_eq!(empty.origin_key_lease_slots(), 256);
    assert_eq!(empty.origin_record_retained_bytes(), 0);
    assert_eq!(empty.origin_record_layout_budget_bytes(), 0);

    let mark = store.watermark();
    for records in 1..=ORIGIN_RECORD_ARCHIVE_CHUNK * 4 + 1 {
        store.allocate(OriginRecord::Synthetic(SyntheticOrigin::new(
            SyntheticOriginKind::Test,
        )));
        if matches!(
            records,
            1 | 3 | 4 | 5 | 1023 | 1024 | 1025 | 4095 | 4096 | 4097
        ) {
            let stats = store.stats();
            assert!(
                stats.origin_record_retained_bytes() <= stats.origin_record_layout_budget_bytes(),
                "record layout exceeded derived budget at {records}: {stats:?}"
            );
        }
    }

    // Live-length-derived geometry must not excuse capacity retained after a
    // rollback. This is the non-tautological failure mode the budget guards.
    store.truncate_to(mark);
    let rolled_back = store.stats();
    assert_eq!(rolled_back.origin_records(), 0);
    assert!(
        rolled_back.origin_record_retained_bytes()
            > rolled_back.origin_record_layout_budget_bytes()
    );
}

#[test]
fn provenance_capacity_index_guards_reserve_overflow_values() {
    assert_eq!(super::u32_len(u32::MAX as usize), Some(u32::MAX));
    assert_eq!(super::arena_index(0), Some(0));
    assert_eq!(super::arena_index(0x7fff_ffff), Some(0x7fff_ffff));
    assert_eq!(super::arena_index(0x8000_0000), None);
}

#[test]
fn provenance_soft_budget_degrades_excess_records_to_unknown() {
    let mut store = ProvenanceStore::new();
    store.record_limit = 1;

    let retained = store.allocate(OriginRecord::Synthetic(SyntheticOrigin::new(
        SyntheticOriginKind::Engine,
    )));
    let degraded = store.allocate(OriginRecord::Synthetic(SyntheticOrigin::new(
        SyntheticOriginKind::Primitive,
    )));
    assert_ne!(retained, OriginId::UNKNOWN);
    assert_eq!(degraded, OriginId::UNKNOWN);
    let noexpand = store.allocate(OriginRecord::Inserted(InsertedOrigin::new(
        InsertedOriginKind::NoExpand,
        Token::Cs(crate::interner::Symbol::new(1)),
        OriginId::UNKNOWN,
    )));
    assert_eq!(noexpand, OriginId::NOEXPAND_FALLBACK);
    assert_eq!(
        noexpand.decode(),
        crate::token::OriginEncoding::NoExpandFallback
    );
    assert!(store.contains_origin(noexpand));
    assert_eq!(store.stats().origin_records(), 1);
}

#[test]
fn runtime_origin_lists_obey_explicit_region_budgets() {
    let mut universe = Universe::new().with_provenance_config(
        super::ProvenanceDemand::default(),
        ProvenanceBudgets {
            live_atoms: 1,
            live_origin_lists: 1,
            origin_list_entries: 1,
            weak_atom_slots: 1,
            weak_atom_candidate_keys: 0,
            detached_artifact_recipe_bytes: 0,
        },
    );
    let first = OriginRef::unknown();
    let list = universe.allocate_origin_list_ref(std::slice::from_ref(&first));
    let excess_list = universe.allocate_origin_list_ref(std::slice::from_ref(&first));
    assert_eq!(list, excess_list, "exact live value remains reusable");
    let two_entries = universe.allocate_origin_list_ref(&[first.clone(), first]);
    assert_eq!(two_entries, OriginListRef::empty());
    assert_eq!(
        universe.origin_list(list).iter().collect::<Vec<_>>(),
        vec![OriginId::UNKNOWN]
    );
    assert_eq!(universe.provenance_stats().origin_list_spans(), 1);
}

#[test]
fn universe_provenance_stats_measure_rollback_truncation() {
    let mut stores = Universe::new();
    let baseline = stores.provenance_stats();
    let snapshot = stores.snapshot();
    stores
        .register_source(
            SourceId::new(3),
            SourceDescriptor::generated(Arc::from(&b"discarded timeline"[..])),
        )
        .expect("generated source registration");
    let source = stores.source_token_origin(SourceId::new(3), 0, 1);
    let _origins = stores.allocate_origin_list_ref(
        &std::iter::repeat_n(OriginRef::direct(source), 128).collect::<Vec<_>>(),
    );

    let grown = stores.provenance_stats();
    assert_eq!(grown.saturating_sub(baseline).origin_records(), 0);
    assert_eq!(grown.saturating_sub(baseline).origin_list_spans(), 1);
    assert_eq!(grown.saturating_sub(baseline).origin_list_entries(), 128);
    assert_eq!(grown.saturating_sub(baseline).source_regions(), 1);
    assert_eq!(
        grown.saturating_sub(baseline).generated_source_backings(),
        1
    );

    stores.rollback(&snapshot);
    let rolled_back = stores.provenance_stats();
    assert_eq!(rolled_back.origin_records(), baseline.origin_records());
    assert_eq!(
        rolled_back.origin_list_spans(),
        baseline.origin_list_spans()
    );
    assert_eq!(
        rolled_back.origin_list_entries(),
        baseline.origin_list_entries()
    );
    assert_eq!(rolled_back.source_regions(), baseline.source_regions());
    assert_eq!(
        rolled_back.generated_source_backings(),
        baseline.generated_source_backings()
    );
    assert_eq!(rolled_back.estimated_bytes(), baseline.estimated_bytes());
    assert!(rolled_back.retained_bytes() >= baseline.retained_bytes());
    assert!(rolled_back.retained_bytes() > rolled_back.estimated_bytes());
}
