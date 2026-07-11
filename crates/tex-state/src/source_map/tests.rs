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

    let base = first.0;
    assert_eq!(
        map.position(SourceId::new(0), 3)
            .expect("source-map operation succeeds")
            .0,
        base + 3
    );
    assert_eq!(empty.0, base + 4);
    assert_eq!(last.0, base + 5);
    assert!(map.span(first, SourcePos(base + 3)).is_ok());
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
        (1, 1, first.0 + 5)
    );
    assert_eq!(
        map.register(SourceId::new(7), generated(b"different")),
        Err(SourceMapError::ConflictingRegistration)
    );
}

#[test]
fn registered_source_capability_encodes_only_backed_nonempty_direct_ranges() {
    let source = RegisteredSource::new(SourcePos(40), 4);
    let origin = source.direct_origin(1, 3).expect("range is direct");
    assert_eq!(
        origin.decode(),
        crate::token::OriginEncoding::DirectSource(SourcePos(41))
    );
    assert!(source.direct_origin(4, 4).is_none());
    assert!(source.direct_origin(3, 5).is_none());

    let wide = RegisteredSource::new(SourcePos(u64::from(u32::MAX)), 1);
    assert!(wide.direct_origin(0, 1).is_none());
}

#[test]
fn rollback_reuses_source_and_backing_slots_but_not_logical_positions() {
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
    assert_ne!(reused, discarded);
    assert!(map.region_for_position(discarded).is_none());
    assert_eq!(map.generated.len(), 2);
    assert_eq!(map.generated[1].bytes(), b"replacement");
}

#[test]
fn fork_keeps_inherited_regions_and_separates_new_logical_ranges() {
    let mut parent = SourceMap::default();
    let inherited = parent
        .register(SourceId::new(0), generated(b"root"))
        .expect("root registers");
    let mut child = parent.clone();
    assert_eq!(child.position(SourceId::new(0), 0), Ok(inherited));

    let parent_only = parent
        .register(SourceId::new(1), generated(b"parent"))
        .expect("parent source registers");
    let child_only = child
        .register(SourceId::new(1), generated(b"child"))
        .expect("child source registers");
    assert_ne!(parent_only, child_only);
    assert!(child.region_for_position(parent_only).is_none());
    assert!(parent.region_for_position(child_only).is_none());
}

#[test]
fn checked_registration_rejects_logical_u64_exhaustion_without_mutation() {
    let mut map = SourceMap::default();
    map.set_next_position_for_test(u64::MAX);
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
