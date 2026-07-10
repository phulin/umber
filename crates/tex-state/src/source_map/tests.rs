use std::mem;
use std::sync::Arc;

use super::*;

fn generated(bytes: &[u8]) -> SourceDescriptor {
    SourceDescriptor::generated(Arc::from(bytes))
}

#[test]
fn regions_reserve_distinct_anchor_positions_and_validate_spans() {
    let mut map = SourceMap::default();
    let first = map
        .register(SourceId::new(0), generated(b"abc"))
        .expect("source-map operation succeeds");
    let empty = map
        .register(SourceId::new(1), generated(b""))
        .expect("source-map operation succeeds");
    let last = map
        .register(SourceId::new(2), generated("é".as_bytes()))
        .expect("source-map operation succeeds");

    assert_eq!(first.0, 0);
    assert_eq!(
        map.position(SourceId::new(0), 3)
            .expect("source-map operation succeeds")
            .0,
        3
    );
    assert_eq!(empty.0, 4);
    assert_eq!(last.0, 5);
    assert!(map.span(first, SourcePos(3)).is_ok());
    assert!(
        map.span(empty, empty)
            .expect("source-map operation succeeds")
            .is_empty()
    );
    assert_eq!(
        map.span(first, empty),
        Err(SourceMapError::SpanCrossesSource)
    );
    assert_eq!(
        map.position(SourceId::new(2), 3),
        Err(SourceMapError::OffsetOutsideSource)
    );
}

#[test]
fn registration_is_idempotent_but_rejects_conflicting_backing() {
    let mut map = SourceMap::default();
    let descriptor = generated(b"same");
    let first = map
        .register(SourceId::new(7), descriptor.clone())
        .expect("source-map operation succeeds");
    assert_eq!(
        map.register(SourceId::new(7), descriptor)
            .expect("source-map operation succeeds"),
        first
    );
    assert_eq!(
        (map.regions.len(), map.generated.len(), map.next_pos),
        (1, 1, 5)
    );
    assert_eq!(
        map.register(SourceId::new(7), generated(b"different")),
        Err(SourceMapError::ConflictingRegistration)
    );
}

#[test]
fn rollback_reuses_source_ids_generated_ids_and_logical_positions_without_aliasing() {
    let mut map = SourceMap::default();
    map.register(SourceId::new(0), generated(b"root"))
        .expect("source-map operation succeeds");
    let mark = map.watermark();
    let discarded = map
        .register(SourceId::new(1), generated(b"discarded"))
        .expect("source-map operation succeeds");
    let discarded_region = map
        .region_for_source(SourceId::new(1))
        .expect("source-map operation succeeds");
    map.truncate_to(mark);

    assert!(map.region_for_source(SourceId::new(1)).is_none());
    assert!(
        map.generated(discarded_region.backing_generated())
            .is_none()
    );
    let reused = map
        .register(SourceId::new(1), generated(b"replacement"))
        .expect("source-map operation succeeds");
    assert_eq!(reused, discarded);
    assert_eq!(map.generated.len(), 2);
    assert_eq!(map.generated[1].bytes(), b"replacement");
}

#[test]
fn checked_registration_rejects_logical_u64_exhaustion_without_mutation() {
    let mut map = SourceMap {
        next_pos: u64::MAX,
        ..SourceMap::default()
    };
    let before = map.watermark();
    assert_eq!(
        map.register(SourceId::new(0), generated(b"")),
        Err(SourceMapError::LogicalPositionExhausted)
    );
    assert_eq!(map.watermark(), before);
}

#[test]
fn snapshot_mark_is_constant_size_independent_of_backing_bytes() {
    let mut map = SourceMap::default();
    let before = mem::size_of_val(&map.watermark());
    map.register(SourceId::new(0), generated(&vec![b'x'; 1024 * 1024]))
        .expect("source-map operation succeeds");
    assert_eq!(mem::size_of_val(&map.watermark()), before);
}

impl SourceRegion {
    fn backing_generated(self) -> GeneratedSourceId {
        match self.backing {
            SourceBacking::Generated(id) => id,
            SourceBacking::World(_) => panic!("expected generated backing"),
        }
    }
}
