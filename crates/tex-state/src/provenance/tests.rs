use super::{
    InsertedOrigin, InsertedOriginKind, MacroInvocationOrigin, OriginKeyRuns, OriginRecord,
    ProvenanceStore, SourceOrigin, SynthesizedOrigin, SynthesizedOriginKind, SyntheticOrigin,
    SyntheticOriginKind, packed_origin_successor,
};
use crate::Universe;
use crate::ids::OriginListId;
use crate::input::{SourceId, TokenListReplayKind};
use crate::macro_store::MacroMeaning;
use crate::meaning::MeaningFlags;
use crate::source_map::SourceDescriptor;
use crate::token::{Catcode, OriginId, Token};
use std::sync::Arc;

#[test]
fn unknown_origin_and_empty_list_are_preallocated() {
    let store = ProvenanceStore::new();

    assert_eq!(store.get(OriginId::UNKNOWN), OriginRecord::UnknownBootstrap);
    assert_eq!(store.list(OriginListId::EMPTY), &[]);
    assert!(store.contains_origin(OriginId::UNKNOWN));
    assert_eq!(store.stats().origin_records(), 0);
    assert!(store.contains_list(OriginListId::EMPTY));
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
fn records_and_origin_lists_allocate_and_read_back() {
    let mut store = ProvenanceStore::new();
    let source = store.allocate(OriginRecord::Source(SourceOrigin::new(
        SourceId::new(7),
        123,
        4,
        9,
    )));
    let inserted = store.allocate(OriginRecord::Inserted(InsertedOrigin::new(
        InsertedOriginKind::Paragraph,
        Token::Char {
            ch: 'p',
            cat: Catcode::Letter,
        },
        source,
    )));
    let list = store.allocate_list(&[source, inserted]);

    assert!(source.raw() & 0x8000_0000 != 0);
    assert!(inserted.raw() & 0x8000_0000 != 0);
    assert_ne!(source, inserted);
    assert_eq!(
        store.get(source),
        OriginRecord::Source(SourceOrigin::new(SourceId::new(7), 123, 4, 9))
    );
    assert_eq!(store.list(list), &[source, inserted]);
}

#[test]
fn record_snapshot_retains_rolled_back_chunks_and_tail() {
    let mut store = ProvenanceStore::new();
    let mark = store.watermark();
    let mut origins = Vec::new();
    for _ in 0..(super::ORIGIN_RECORD_ARCHIVE_CHUNK + 7) {
        origins.push(store.allocate(OriginRecord::Synthetic(SyntheticOrigin::new(
            SyntheticOriginKind::Test,
        ))));
    }
    let snapshot = store.record_snapshot();
    let first = origins[0];
    let last = *origins.last().expect("snapshot has a tail record");

    store.truncate_to(mark);
    assert!(!store.contains_origin(first));
    assert!(!store.contains_origin(last));
    assert!(matches!(
        snapshot.get(first),
        Some(OriginRecord::Synthetic(_))
    ));
    assert!(matches!(
        snapshot.get(last),
        Some(OriginRecord::Synthetic(_))
    ));
    assert!(store.record_snapshot().get(first).is_none());
}

#[test]
fn repeated_origin_lists_allocate_without_extra_records() {
    let mut store = ProvenanceStore::new();
    let source = store.allocate(OriginRecord::Source(SourceOrigin::new(
        SourceId::new(2),
        9,
        1,
        9,
    )));
    let before = store.stats();
    let list = store.allocate_repeated_list(source, 4);
    let after = store.stats();

    assert_eq!(store.list(list), &[source, source, source, source]);
    assert_eq!(after.origin_records(), before.origin_records());
    let growth = after.saturating_sub(before);
    assert_eq!(growth.origin_records(), 0);
    assert_eq!(growth.origin_list_spans(), 1);
    assert_eq!(growth.origin_list_entries(), 4);
    assert!(growth.retained_bytes() >= growth.estimated_bytes());
}

#[test]
fn origin_list_rollback_reuse_invalidates_the_old_identity() {
    let mut store = ProvenanceStore::new();
    let mark = store.watermark();
    let stale = store.allocate_list(&[OriginId::UNKNOWN]);
    store.truncate_to(mark);
    let reused = store.allocate_list(&[OriginId::UNKNOWN]);
    assert_eq!(reused.raw(), stale.raw());
    assert_ne!(reused, stale);
    assert!(!store.contains_list(stale));
    assert_eq!(store.list(reused), &[OriginId::UNKNOWN]);
}

#[test]
fn provenance_fork_keeps_inherited_lists_but_separates_new_ones() {
    let mut parent = ProvenanceStore::new();
    let inherited = parent.allocate_list(&[OriginId::UNKNOWN]);
    let mut child = parent.clone();
    assert_eq!(child.list(inherited), &[OriginId::UNKNOWN]);
    let parent_only = parent.allocate_list(&[OriginId::UNKNOWN; 2]);
    let child_only = child.allocate_list(&[OriginId::UNKNOWN; 3]);
    assert_eq!(parent_only.raw(), child_only.raw());
    assert!(!child.contains_list(parent_only));
    assert!(!parent.contains_list(child_only));
}

#[test]
fn provenance_fork_keeps_inherited_origins_but_separates_new_keys() {
    let mut parent = ProvenanceStore::new();
    let inherited = parent.allocate(OriginRecord::UnknownBootstrap);
    let mut child = parent.clone();
    assert!(child.contains_origin(inherited));
    let parent_only = parent.allocate(OriginRecord::UnknownBootstrap);
    let child_only = child.allocate(OriginRecord::UnknownBootstrap);
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
        stores.macro_invocation_origin(definition, invocation, source, OriginId::UNKNOWN);
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
        OriginRecord::MacroInvocation(MacroInvocationOrigin::new(
            definition,
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
fn provenance_capacity_index_guards_reserve_overflow_values() {
    assert_eq!(super::u32_len(u32::MAX as usize), Some(u32::MAX));
    assert_eq!(super::u32_index(u32::MAX as usize - 1), Some(u32::MAX - 1));
    assert_eq!(super::u32_index(u32::MAX as usize), None);
    assert_eq!(super::arena_index(0), Some(0));
    assert_eq!(super::arena_index(0x7fff_ffff), Some(0x7fff_ffff));
    assert_eq!(super::arena_index(0x8000_0000), None);
}

#[test]
fn rollback_mark_truncates_records_and_lists() {
    let mut store = ProvenanceStore::new();
    let kept = store.allocate(OriginRecord::Synthetic(SyntheticOrigin::new(
        SyntheticOriginKind::Engine,
    )));
    let mark = store.watermark();
    let stale = store.allocate(OriginRecord::Synthetic(SyntheticOrigin::new(
        SyntheticOriginKind::Primitive,
    )));
    let stale_list = store.allocate_list(&[kept, stale]);

    store.truncate_to(mark);
    let reused = store.allocate(OriginRecord::Synthetic(SyntheticOrigin::new(
        SyntheticOriginKind::Format,
    )));
    let reused_list = store.allocate_list(&[reused]);

    assert_ne!(reused.raw(), stale.raw());
    assert!(!store.contains_origin(stale));
    assert_eq!(reused_list.raw(), stale_list.raw());
    assert_ne!(reused_list, stale_list);
    assert_eq!(
        store.get(reused),
        OriginRecord::Synthetic(SyntheticOrigin::new(SyntheticOriginKind::Format))
    );
    assert_eq!(store.list(reused_list), &[reused]);
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
    stores.allocate_repeated_origin_list(source, 128);

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
