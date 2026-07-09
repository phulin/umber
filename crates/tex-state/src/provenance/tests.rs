use super::{
    InsertedOrigin, InsertedOriginKind, OriginRecord, ProvenanceStore, SourceOrigin,
    SyntheticOrigin, SyntheticOriginKind,
};
use crate::ids::OriginListId;
use crate::input::SourceId;
use crate::token::{Catcode, OriginId, Token};

#[test]
fn unknown_origin_and_empty_list_are_preallocated() {
    let store = ProvenanceStore::new();

    assert_eq!(store.get(OriginId::UNKNOWN), OriginRecord::UnknownBootstrap);
    assert_eq!(store.list(OriginListId::EMPTY), &[]);
    assert!(store.contains_origin(OriginId::UNKNOWN));
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

    assert_eq!(source.raw(), 1);
    assert_eq!(inserted.raw(), 2);
    assert_eq!(
        store.get(source),
        OriginRecord::Source(SourceOrigin::new(SourceId::new(7), 123, 4, 9))
    );
    assert_eq!(store.list(list), &[source, inserted]);
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
