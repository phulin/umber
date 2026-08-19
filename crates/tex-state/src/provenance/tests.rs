use super::{
    InsertedOrigin, InsertedOriginKind, MacroInvocationOrigin, ORIGIN_KEY_LEASE_LEN,
    ORIGIN_RECORD_ARCHIVE_CHUNK, OriginKeyRuns, OriginListRef, OriginRecord, OriginRef,
    ProvenanceBudgets, ProvenanceStore, SourceOrigin, SynthesizedOrigin, SynthesizedOriginKind,
    SyntheticOrigin, SyntheticOriginKind, packed_origin_successor,
};
use crate::Universe;
use crate::font::NULL_FONT;
use crate::input::{SourceId, TokenListReplayKind};
use crate::macro_store::MacroMeaning;
use crate::meaning::MeaningFlags;
use crate::node::Node;
use crate::node_arena::{NodeListRef, NodeSemanticIdBuilder, SidecarNeeds};
use crate::source_map::SourceDescriptor;
use crate::token::{Catcode, OriginId, Token};
use std::sync::Arc;
use std::sync::Barrier;

fn freeze_test_nodes(nodes: Vec<Node>, semantic: u64) -> NodeListRef {
    let mut needs = SidecarNeeds::default();
    for node in &nodes {
        needs.preflight_and_count(node);
    }
    let mut identity = NodeSemanticIdBuilder::new();
    identity.push(|hasher| hasher.u64(semantic));
    NodeListRef::freeze_builder(nodes, Vec::new(), identity.finish(), needs)
}

#[test]
fn unknown_origin_and_empty_list_are_preallocated() {
    let store = ProvenanceStore::new();

    assert_eq!(store.get(OriginId::UNKNOWN), OriginRecord::UnknownBootstrap);
    assert_eq!(OriginListRef::empty().origins(), &[]);
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
fn records_and_structural_origin_lists_allocate_and_read_back() {
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
    let list = store.allocate_rooted_list(&[source.clone(), inserted.clone()]);

    assert!(source.id().raw() & 0x8000_0000 != 0);
    assert!(inserted.id().raw() & 0x8000_0000 != 0);
    assert_ne!(source.id(), inserted.id());
    assert_eq!(source.record(), Some(source_record));
    assert_eq!(list.origins(), &[source.id(), inserted.id()]);
}

#[test]
fn repeated_structural_origin_lists_share_one_distinct_owner() {
    let mut store = ProvenanceStore::new();
    let source = store.allocate_rooted(
        OriginRecord::Source(SourceOrigin::new(SourceId::new(2), 9, 1, 9)),
        [],
    );
    let before = store.stats();
    let list = store.allocate_rooted_list(&[
        source.clone(),
        source.clone(),
        source.clone(),
        source.clone(),
    ]);
    let after = store.stats();

    assert_eq!(list.origins(), &[source.id(); 4]);
    assert_eq!(list.owned_root_count(), 1);
    assert_eq!(after.origin_records(), before.origin_records());
    let growth = after.saturating_sub(before);
    assert_eq!(growth.origin_records(), 0);
    assert_eq!(growth.origin_list_spans(), 1);
    assert_eq!(growth.origin_list_entries(), 4);
    assert!(growth.retained_bytes() >= growth.estimated_bytes());
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
fn exact_records_and_lists_share_structural_slots() {
    let mut store = ProvenanceStore::new();
    let record = OriginRecord::Source(SourceOrigin::new(SourceId::new(7), 123, 4, 9));
    let first = store.allocate_rooted(record, []);
    let second = store.allocate_rooted(record, []);
    assert_eq!(first, second);
    assert_eq!(store.rooted_record_shape(), (0, 0));

    let first_list = store.allocate_rooted_list(&[first.clone(), OriginRef::unknown()]);
    let second_list = store.allocate_rooted_list(&[first, OriginRef::unknown()]);
    assert_eq!(first_list, second_list);
    assert_eq!(store.stats().origin_list_spans(), 1);
    assert_eq!(store.stats().origin_list_entries(), 2);
}

#[test]
fn exact_structural_hits_skip_unrelated_reclamation_work() {
    let mut store = ProvenanceStore::new();
    let record = OriginRecord::Synthetic(SyntheticOrigin::new(SyntheticOriginKind::Test));
    let root = store.allocate_rooted(record, []);
    let list = store.allocate_rooted_list(std::slice::from_ref(&root));

    let dead = store.allocate_rooted(
        OriginRecord::Synthetic(SyntheticOrigin::new(SyntheticOriginKind::Engine)),
        [],
    );
    let dead_list = store.allocate_rooted_list(std::slice::from_ref(&dead));
    drop(dead_list);
    drop(dead);

    let list_cursor = store.rooted_list_sweep_cursor;
    assert_eq!(store.allocate_rooted(record, []).id(), root.id());
    assert_eq!(
        store.allocate_rooted_list(std::slice::from_ref(&root)).id(),
        list.id()
    );
    assert_eq!(store.rooted_list_sweep_cursor, list_cursor);
}

#[test]
fn expansion_frame_children_fit_the_inline_allocation() {
    let mut store = ProvenanceStore::new();
    let frame = store.allocate_rooted(
        OriginRecord::MacroInvocation(MacroInvocationOrigin::from_nonowning_operand(
            42,
            OriginId::UNKNOWN,
            OriginId::UNKNOWN,
            OriginId::UNKNOWN,
        )),
        [
            OriginRef::unknown(),
            OriginRef::unknown(),
            OriginRef::unknown(),
        ],
    );
    assert_eq!(frame.children().len(), 3);
    assert!(
        !frame
            .value
            .as_ref()
            .expect("frame is structurally rooted")
            .children
            .spilled()
    );
}

#[cfg(feature = "profiling")]
#[test]
fn lifecycle_counters_keep_archive_coordinates_out_of_the_weak_atom_index() {
    let before = crate::measurement::provenance_lifecycle_measurement();
    {
        let mut store = ProvenanceStore::new();
        let atom_record = OriginRecord::Synthetic(SyntheticOrigin::new(SyntheticOriginKind::Test));
        let atom = store.allocate_rooted(atom_record, []);
        let atom_hit = store.allocate_rooted(atom_record, []);
        let frame_record =
            OriginRecord::MacroInvocation(MacroInvocationOrigin::from_nonowning_operand(
                7,
                atom.id(),
                OriginId::UNKNOWN,
                OriginId::UNKNOWN,
            ));
        let frame = store.allocate_rooted(frame_record, [atom.clone()]);
        let frame_hit = store.allocate_rooted(frame_record, [atom.clone()]);
        let list = store.allocate_rooted_list(&[frame.clone(), atom.clone()]);
        let list_hit = store.allocate_rooted_list(&[frame.clone(), atom.clone()]);
        assert!(store.origin_ref(frame.id()).is_some());
        assert_eq!(list.root(0).expect("rooted frame").id(), frame.id());
        drop((list_hit, list, frame_hit, frame, atom_hit, atom));
        let replacement = store.allocate_rooted(
            OriginRecord::Synthetic(SyntheticOrigin::new(SyntheticOriginKind::Engine)),
            [],
        );
        drop(replacement);
    }
    let work = crate::measurement::provenance_lifecycle_measurement().saturating_sub(before);
    assert_eq!(work.atom_intern_calls, 0);
    assert_eq!(work.frame_intern_calls, 0);
    assert!(work.list_intern_hits >= 1);
    assert!(work.list_intern_misses >= 1);
    assert!(work.atom_retains >= 1 && work.atom_releases >= 1);
    assert!(work.frame_retains >= 1 && work.frame_releases >= 1);
    assert!(work.list_retains >= 1 && work.list_releases >= 1);
    assert_eq!(work.atom_reclaim_visits, 0);
    assert_eq!(work.list_reclaim_visits, 0);
    assert!(work.origin_resolutions >= 1);
    assert!(work.list_resolutions >= 1 && work.list_resolution_comparisons >= 1);
}

#[test]
fn repeated_macro_expansion_shares_one_structural_frame() {
    let mut store = ProvenanceStore::new();
    let invocation = OriginId::UNKNOWN;
    let record = OriginRecord::MacroInvocation(MacroInvocationOrigin::from_nonowning_operand(
        42,
        invocation,
        OriginId::UNKNOWN,
        OriginId::UNKNOWN,
    ));
    let first = store.allocate(record);
    for _ in 0..10_000 {
        assert_eq!(store.allocate(record), first);
    }
    assert_eq!(store.stats().origin_records(), 1);
    assert_eq!(store.macro_invocation_stats().invocations(), 1);
}

#[test]
fn origin_list_candidate_hash_collision_still_compares_exact_content() {
    let mut store = ProvenanceStore::new();
    let first = store.allocate_unrooted_origin_ids([OriginId::UNKNOWN].into_iter());
    let second_value = [OriginId::NOEXPAND_FALLBACK];
    let colliding_hash = super::origin_list_hash(&second_value);
    store
        .rooted_list_candidates
        .entry(colliding_hash)
        .or_default()
        .push(Arc::downgrade(first.value.as_ref().expect("nonempty list")));

    let second = store.allocate_unrooted_origin_ids(second_value.into_iter());
    assert_ne!(first, second);
    assert_eq!(first.origins(), &[OriginId::UNKNOWN]);
    assert_eq!(second.origins(), &second_value);
}

#[test]
fn structural_candidate_indexes_are_explicitly_bounded() {
    let mut store = ProvenanceStore::new();
    let mut origins = Vec::new();
    for offset in 0..=super::RECORD_CANDIDATE_KEY_BUDGET {
        origins.push(store.allocate(OriginRecord::Source(SourceOrigin::new(
            SourceId::new(7),
            offset as u64,
            1,
            offset as u32,
        ))));
    }
    assert!(store.record_candidates.len() <= super::RECORD_CANDIDATE_KEY_BUDGET);

    let lists = (1..=super::LIST_CANDIDATE_KEY_BUDGET + 1)
        .map(|raw| store.allocate_unrooted_origin_ids([OriginId::from_raw(raw as u32)].into_iter()))
        .collect::<Vec<_>>();
    assert_eq!(lists.len(), super::LIST_CANDIDATE_KEY_BUDGET + 1);
    assert!(store.rooted_list_candidates.len() <= super::LIST_CANDIDATE_KEY_BUDGET);
}

#[test]
fn rollback_removes_record_candidates_without_touching_owned_lists() {
    let mut store = ProvenanceStore::new();
    let mark = store.watermark();
    let record = OriginRecord::Synthetic(SyntheticOrigin::new(SyntheticOriginKind::Test));
    let stale = store.allocate(record);
    let stale_list = store.allocate_rooted_list(&[OriginRef::direct(stale)]);
    store.truncate_to(mark);

    let replacement = store.allocate(record);
    assert_ne!(stale, replacement);
    assert!(!store.contains_origin(stale));
    assert!(store.contains_origin(replacement));
    assert_eq!(stale_list.origins(), &[stale]);
    assert_eq!(store.rooted_list_shape(), (1, 1, 2));
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
fn provenance_soft_budget_degrades_excess_history_to_unknown_and_empty() {
    let mut store = ProvenanceStore::new();
    store.record_limit = 1;
    store.list_span_limit = 2;
    store.list_entry_limit = 2;

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

    let retained_list = store.allocate_rooted_list(&[
        OriginRef::direct(OriginId::UNKNOWN),
        OriginRef::direct(OriginId::NOEXPAND_FALLBACK),
    ]);
    let degraded_for_entries = store.allocate_rooted_list(&[OriginRef::unknown()]);
    assert_ne!(retained_list, OriginListRef::empty());
    assert_eq!(degraded_for_entries, OriginListRef::empty());

    let mut span_limited = ProvenanceStore::new();
    span_limited.list_span_limit = 0;
    assert_eq!(
        span_limited.allocate_rooted_list(&[OriginRef::unknown()]),
        OriginListRef::empty()
    );
}

#[test]
fn rooted_provenance_obeys_each_explicit_live_and_weak_budget() {
    let mut store = ProvenanceStore::new();
    store.configure_budgets(ProvenanceBudgets {
        live_atoms: 1,
        live_origin_lists: 1,
        origin_list_entries: 1,
        weak_atom_slots: 1,
        weak_atom_candidate_keys: 0,
        weak_list_slots: 1,
        weak_list_candidate_keys: 1,
        detached_artifact_recipe_bytes: 0,
    });
    let first = store.allocate_rooted(
        OriginRecord::Synthetic(SyntheticOrigin::new(SyntheticOriginKind::Engine)),
        [],
    );
    let excess = store.allocate_rooted(
        OriginRecord::Synthetic(SyntheticOrigin::new(SyntheticOriginKind::Primitive)),
        [],
    );
    assert_ne!(first.id(), OriginId::UNKNOWN);
    assert_eq!(excess.id(), OriginId::UNKNOWN);

    let list = store.allocate_rooted_list(std::slice::from_ref(&first));
    let excess_list = store.allocate_rooted_list(std::slice::from_ref(&first));
    assert_eq!(list, excess_list, "exact live value remains reusable");
    let two_entries = store.allocate_rooted_list(&[first.clone(), first.clone()]);
    assert_eq!(two_entries, OriginListRef::empty());

    drop(list);
    drop(excess_list);
    drop(first);
    let replacement = store.allocate_rooted(
        OriginRecord::Synthetic(SyntheticOrigin::new(SyntheticOriginKind::Format)),
        [],
    );
    assert_eq!(replacement.id(), OriginId::UNKNOWN);
    assert_eq!(store.rooted_record_shape(), (0, 0));
    assert_eq!(store.rooted_list_shape(), (0, 0, 2));
}

#[test]
fn rollback_mark_truncates_records_without_invalidating_owned_lists() {
    let mut store = ProvenanceStore::new();
    let kept = store.allocate(OriginRecord::Synthetic(SyntheticOrigin::new(
        SyntheticOriginKind::Engine,
    )));
    let mark = store.watermark();
    let stale = store.allocate(OriginRecord::Synthetic(SyntheticOrigin::new(
        SyntheticOriginKind::Primitive,
    )));
    let stale_list =
        store.allocate_rooted_list(&[OriginRef::direct(kept), OriginRef::direct(stale)]);

    store.truncate_to(mark);
    let reused = store.allocate(OriginRecord::Synthetic(SyntheticOrigin::new(
        SyntheticOriginKind::Format,
    )));

    assert_ne!(reused.raw(), stale.raw());
    assert!(!store.contains_origin(stale));
    assert_eq!(stale_list.origins(), &[kept, stale]);
    assert_eq!(
        store.get(reused),
        OriginRecord::Synthetic(SyntheticOrigin::new(SyntheticOriginKind::Format))
    );
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
    let origins = stores.allocate_origin_list_ref(
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

    drop(origins);
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

#[test]
fn rooted_final_owner_release_leaves_only_the_archived_coordinate() {
    let mut store = ProvenanceStore::new();
    let first = store.allocate_rooted(
        OriginRecord::Synthetic(SyntheticOrigin::new(SyntheticOriginKind::Engine)),
        [],
    );
    let first_id = first.id();
    let retained = first.clone();
    drop(first);
    assert!(store.origin_ref(first_id).is_some());
    drop(retained);

    let second = store.allocate_rooted(
        OriginRecord::Synthetic(SyntheticOrigin::new(SyntheticOriginKind::Format)),
        [],
    );
    assert_ne!(second.id(), first_id);
    assert_eq!(
        store.origin_ref(first_id).map(|root| root.id()),
        Some(first_id)
    );
    assert_eq!(store.rooted_record_shape(), (0, 0));
}

#[test]
fn rooted_origin_lists_release_children_and_reuse_generation_safe_slots() {
    let mut store = ProvenanceStore::new();
    let atom = store.allocate_rooted(
        OriginRecord::Synthetic(SyntheticOrigin::new(SyntheticOriginKind::Test)),
        [],
    );
    let first = store.allocate_rooted_list(std::slice::from_ref(&atom));
    let first_id = first.id();
    drop(atom);
    assert!(store.origin_ref(first.origins()[0]).is_some());
    drop(first);

    let replacement_atom = store.allocate_rooted(
        OriginRecord::Synthetic(SyntheticOrigin::new(SyntheticOriginKind::Primitive)),
        [],
    );
    let second = store.allocate_rooted_list(std::slice::from_ref(&replacement_atom));
    assert_eq!(second.id().raw(), first_id.raw());
    assert_ne!(second.id(), first_id);
    assert_eq!(store.rooted_list_shape(), (1, 1, 2));
}

#[test]
fn rooted_provenance_plateaus_for_dead_work_and_grows_exactly_for_live_work() {
    const OPERATIONS: u64 = 10_000;
    let mut bounded = ProvenanceStore::new();
    for serial in 0..OPERATIONS {
        let root = bounded.allocate_rooted(
            OriginRecord::MacroInvocation(MacroInvocationOrigin::from_nonowning_operand(
                serial,
                OriginId::UNKNOWN,
                OriginId::UNKNOWN,
                OriginId::UNKNOWN,
            )),
            [],
        );
        let list = bounded.allocate_rooted_list(std::slice::from_ref(&root));
        drop(list);
        drop(root);
    }
    assert_eq!(bounded.rooted_record_shape(), (0, 0));
    assert_eq!(bounded.rooted_list_shape(), (0, 0, 2));

    let mut all_live = ProvenanceStore::new();
    let roots = (0..OPERATIONS)
        .map(|serial| {
            all_live.allocate_rooted(
                OriginRecord::MacroInvocation(MacroInvocationOrigin::from_nonowning_operand(
                    serial,
                    OriginId::UNKNOWN,
                    OriginId::UNKNOWN,
                    OriginId::UNKNOWN,
                )),
                [],
            )
        })
        .collect::<Vec<_>>();
    let lists = roots
        .iter()
        .map(|root| all_live.allocate_rooted_list(std::slice::from_ref(root)))
        .collect::<Vec<_>>();
    assert_eq!(all_live.rooted_record_shape(), (0, 0));
    assert_eq!(
        all_live.rooted_list_shape(),
        (
            OPERATIONS as usize,
            OPERATIONS as usize,
            OPERATIONS as usize + 1
        )
    );
    assert_eq!(lists.len(), OPERATIONS as usize);
}

#[test]
fn structural_lists_plateau_for_direct_churn_and_grow_by_distinct_live_roots() {
    const OPERATIONS: usize = 256;
    const WIDTH: usize = 32;

    let mut bounded_direct = ProvenanceStore::new();
    let warm_ids = (1..=WIDTH)
        .map(|raw| OriginId::from_raw(raw as u32))
        .collect::<Vec<_>>();
    let warm = bounded_direct.allocate_unrooted_origin_ids(warm_ids.into_iter());
    drop(warm);
    assert_eq!(bounded_direct.rooted_list_shape(), (0, 0, 2));
    let baseline = bounded_direct.stats();
    for operation in 0..OPERATIONS {
        let first_raw = 1 + operation * WIDTH;
        let list = bounded_direct.allocate_unrooted_origin_ids(
            (first_raw..first_raw + WIDTH).map(|raw| OriginId::from_raw(raw as u32)),
        );
        assert_eq!(list.origins().len(), WIDTH);
        assert_eq!(list.owned_root_count(), 0);
        drop(list);
    }
    assert!(bounded_direct.stats().retained_layout_eq(baseline));
    assert_eq!(bounded_direct.rooted_list_shape(), (0, 0, 2));

    let mut all_live_rooted = ProvenanceStore::new();
    let roots = (0..OPERATIONS as u64)
        .map(|serial| {
            all_live_rooted.allocate_rooted(
                OriginRecord::MacroInvocation(MacroInvocationOrigin::from_nonowning_operand(
                    serial,
                    OriginId::UNKNOWN,
                    OriginId::UNKNOWN,
                    OriginId::UNKNOWN,
                )),
                [],
            )
        })
        .collect::<Vec<_>>();
    let lists = roots
        .iter()
        .map(|root| {
            let repeated: [OriginRef; WIDTH] = std::array::from_fn(|_| root.clone());
            all_live_rooted.allocate_rooted_list(&repeated)
        })
        .collect::<Vec<_>>();
    assert!(lists.iter().all(|list| list.owned_root_count() == 1));
    assert_eq!(
        all_live_rooted.rooted_list_shape(),
        (OPERATIONS, OPERATIONS * WIDTH, OPERATIONS + 1)
    );
    assert_eq!(
        lists
            .iter()
            .map(|list| {
                std::mem::size_of_val(list.origins())
                    + list.owned_root_count() * std::mem::size_of::<OriginRef>()
            })
            .sum::<usize>(),
        OPERATIONS * WIDTH * 4 + OPERATIONS * 24
    );
}

#[test]
fn archived_records_need_no_weak_reclamation_and_preserve_exact_reuse() {
    const LIVE_ROOTS: u64 = 1_024;
    let mut store = ProvenanceStore::new();
    let roots = (0..LIVE_ROOTS)
        .map(|serial| {
            store.allocate_rooted(
                OriginRecord::MacroInvocation(MacroInvocationOrigin::from_nonowning_operand(
                    serial,
                    OriginId::UNKNOWN,
                    OriginId::UNKNOWN,
                    OriginId::UNKNOWN,
                )),
                [],
            )
        })
        .collect::<Vec<_>>();
    let transient = store.allocate_rooted(
        OriginRecord::Synthetic(SyntheticOrigin::new(SyntheticOriginKind::Engine)),
        [],
    );
    let transient_id = transient.id();
    drop(transient);

    assert_eq!(store.rooted_record_shape(), (0, 0));
    assert_eq!(
        store.origin_ref(transient_id).map(|root| root.id()),
        Some(transient_id)
    );
    assert!(
        roots
            .iter()
            .all(|root| store.origin_ref(root.id()).is_some())
    );
    let duplicate = store.allocate_rooted(
        OriginRecord::MacroInvocation(MacroInvocationOrigin::from_nonowning_operand(
            LIVE_ROOTS / 2,
            OriginId::UNKNOWN,
            OriginId::UNKNOWN,
            OriginId::UNKNOWN,
        )),
        [],
    );
    assert_eq!(duplicate.id(), roots[(LIVE_ROOTS / 2) as usize].id());
}

#[test]
fn node_owners_plateau_for_10k_released_roots_and_retain_10k_live_roots() {
    const OPERATIONS: u64 = 10_000;
    let mut bounded = ProvenanceStore::new();
    for serial in 0..OPERATIONS {
        let root = bounded.allocate_rooted(
            OriginRecord::MacroInvocation(MacroInvocationOrigin::from_nonowning_operand(
                serial,
                OriginId::UNKNOWN,
                OriginId::UNKNOWN,
                OriginId::UNKNOWN,
            )),
            [],
        );
        let id = root.id();
        let owner = freeze_test_nodes(
            vec![Node::Char {
                font: NULL_FONT,
                ch: 'x',
                origin: root,
            }],
            serial,
        );
        assert!(bounded.origin_ref(id).is_some());
        drop(owner);
        assert_eq!(bounded.origin_ref(id).map(|root| root.id()), Some(id));
    }
    assert_eq!(bounded.rooted_record_shape(), (0, 0));

    let mut all_live = ProvenanceStore::new();
    let roots = (0..OPERATIONS)
        .map(|serial| {
            all_live.allocate_rooted(
                OriginRecord::MacroInvocation(MacroInvocationOrigin::from_nonowning_operand(
                    serial,
                    OriginId::UNKNOWN,
                    OriginId::UNKNOWN,
                    OriginId::UNKNOWN,
                )),
                [],
            )
        })
        .collect::<Vec<_>>();
    let nodes = roots
        .iter()
        .cloned()
        .map(|origin| Node::Char {
            font: NULL_FONT,
            ch: 'x',
            origin,
        })
        .collect::<Vec<_>>();
    let live_owner = freeze_test_nodes(nodes, OPERATIONS);
    drop(roots);
    assert_eq!(all_live.rooted_record_shape(), (0, 0));
    drop(live_owner);
}

#[test]
fn structural_owner_and_committed_artifact_release_their_exact_node_roots() {
    let mut store = ProvenanceStore::new();
    let root = store.allocate_rooted(
        OriginRecord::Synthetic(SyntheticOrigin::new(SyntheticOriginKind::Test)),
        [],
    );
    let id = root.id();
    let list = freeze_test_nodes(
        vec![Node::Char {
            font: NULL_FONT,
            ch: 'x',
            origin: root,
        }],
        1,
    );
    assert!(store.origin_ref(id).is_some());

    let payload_root = list.nodes().first().expect("owned character");
    let crate::node_arena::NodeRef::Char { origin_root, .. } = payload_root else {
        panic!("survivor character")
    };
    let mut builder = crate::RenderProvenanceBuilder::default();
    builder.push_root(origin_root.clone());
    builder.push_deferred(&crate::OutputProvenanceRecipe::default(), 0..0);
    let verified = crate::VerifiedArtifact::new(b"artifact".to_vec())
        .with_built_render_origins(vec![1], builder);
    let (bytes, render_provenance, occurrences) = verified.into_parts();
    let artifact = crate::CommittedArtifact::new(
        crate::ContentHash::for_domain(crate::ContentDomain::Artifact, &bytes),
        bytes,
        render_provenance,
        occurrences,
    );
    drop(list);
    assert!(store.origin_ref(id).is_some());
    drop(artifact);
    assert_eq!(store.origin_ref(id).map(|root| root.id()), Some(id));
}
