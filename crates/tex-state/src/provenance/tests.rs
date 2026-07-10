use super::{
    InsertedOrigin, InsertedOriginKind, MacroInvocationOrigin, OriginRecord, ProvenanceStore,
    SourceOrigin, SynthesizedOrigin, SynthesizedOriginKind, SyntheticOrigin, SyntheticOriginKind,
};
use crate::Universe;
use crate::ids::OriginListId;
use crate::input::{SourceId, TokenListReplayKind};
use crate::macro_store::MacroMeaning;
use crate::meaning::MeaningFlags;
use crate::token::{Catcode, OriginId, Token};

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

    assert_eq!(source.raw(), 0x8000_0000);
    assert_eq!(inserted.raw(), 0x8000_0001);
    assert_eq!(
        store.get(source),
        OriginRecord::Source(SourceOrigin::new(SourceId::new(7), 123, 4, 9))
    );
    assert_eq!(store.list(list), &[source, inserted]);
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
    let macro_origin = stores.macro_invocation_origin(definition, invocation, source);
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
        OriginRecord::MacroInvocation(MacroInvocationOrigin::new(definition, invocation, source))
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

    assert_eq!(reused.raw(), stale.raw());
    assert_eq!(reused_list.raw(), stale_list.raw());
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
    let source = stores.source_origin(SourceId::new(3), 30, 3, 1);
    stores.allocate_repeated_origin_list(source, 128);

    let grown = stores.provenance_stats();
    assert_eq!(grown.saturating_sub(baseline).origin_records(), 1);
    assert_eq!(grown.saturating_sub(baseline).origin_list_spans(), 1);
    assert_eq!(grown.saturating_sub(baseline).origin_list_entries(), 128);

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
    assert!(rolled_back.retained_bytes() >= baseline.retained_bytes());
}
