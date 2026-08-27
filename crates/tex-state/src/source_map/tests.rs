use std::sync::Arc;

use super::*;

fn generated(bytes: &[u8]) -> SourceDescriptor {
    SourceDescriptor::generated(Arc::from(bytes))
}

#[test]
fn regions_reserve_distinct_anchor_positions_and_validate_spans() {
    let mut map = SourceMap::default();
    map.set_next_position_for_test(0);
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
fn registration_stores_line_starts_and_rollback_discards_them() {
    let mut map = SourceMap::default();
    map.register(SourceId::new(0), generated(b"first\nsecond\n"))
        .expect("source registers");
    let root = map
        .region_for_source(SourceId::new(0))
        .expect("root source remains live");
    assert_eq!(map.line_starts(root), Some(&[0, 6, 13][..]));

    let mark = map.watermark();
    map.register(SourceId::new(1), generated(b"a\nb"))
        .expect("source registers");
    let discarded = map
        .region_for_source(SourceId::new(1))
        .expect("discarded source is live before rollback");
    assert_eq!(map.line_starts(discarded), Some(&[0, 2][..]));

    map.truncate_to(mark);
    assert_eq!(map.line_starts(discarded), None);
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
fn sparse_source_index_tracks_registration_rollback_and_fork() {
    let mut map = SourceMap::default();
    map.register(SourceId::new(40), generated(b"root"))
        .expect("sparse source registers");
    let mark = map.watermark();
    let discarded = map
        .register(SourceId::new(3), generated(b"retry"))
        .expect("out-of-order source registers");

    assert_eq!(map.region_by_source.get(&SourceId::new(40)), Some(&0));
    assert_eq!(map.region_by_source.get(&SourceId::new(3)), Some(&1));
    assert_eq!(map.position(SourceId::new(3), 0), Ok(discarded));

    let fork = map.clone();
    assert_eq!(
        fork.position(SourceId::new(40), 0),
        map.position(SourceId::new(40), 0)
    );
    assert_eq!(fork.position(SourceId::new(3), 0), Ok(discarded));

    let discarded_index = Arc::make_mut(&mut map.region_by_source)
        .remove(&SourceId::new(3))
        .expect("source has a derived index");
    assert_eq!(
        map.position(SourceId::new(3), 0),
        Err(SourceMapError::UnknownSource),
        "source lookup must not retain a linear fallback"
    );
    assert_eq!(
        Arc::make_mut(&mut map.region_by_source).insert(SourceId::new(3), discarded_index),
        None
    );

    map.truncate_to(mark);
    assert_eq!(map.region_by_source.get(&SourceId::new(40)), Some(&0));
    assert!(!map.region_by_source.contains_key(&SourceId::new(3)));
    assert_eq!(
        map.position(SourceId::new(3), 0),
        Err(SourceMapError::UnknownSource)
    );

    let registered_again = map
        .register(SourceId::new(3), generated(b"retry"))
        .expect("source registers after rollback");
    assert!(registered_again > discarded);
    assert_eq!(map.region_by_source.get(&SourceId::new(3)), Some(&1));
}

#[test]
fn existing_registration_distinguishes_absent_identical_and_conflicting_sources() {
    let mut map = SourceMap::default();
    let source = SourceId::new(7);
    let descriptor = generated(b"same\nbacking");
    assert_eq!(
        map.existing_registration(source, &descriptor),
        Ok(None),
        "an unused source identity has no cached registration"
    );

    let position = map
        .register(source, descriptor.clone())
        .expect("source registers");
    assert_eq!(
        map.existing_registration(source, &descriptor),
        Ok(Some(position)),
        "an identical descriptor reuses the registered index"
    );
    assert_eq!(
        map.existing_registration(source, &generated(b"same\nbacking")),
        Ok(Some(position)),
        "equal bytes from a distinct allocation remain semantically identical"
    );
    assert_eq!(
        map.existing_registration(source, &generated(b"different")),
        Err(SourceMapError::ConflictingRegistration)
    );

    let named_source = SourceId::new(8);
    let named = SourceDescriptor::named_generated("first.tex", Arc::from(&b"bytes"[..]));
    map.register(named_source, named)
        .expect("named source registers");
    assert_eq!(
        map.existing_registration(
            named_source,
            &SourceDescriptor::named_generated("other.tex", Arc::from(&b"bytes"[..]))
        ),
        Err(SourceMapError::ConflictingRegistration),
        "logical path remains part of generated source identity"
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
fn registered_source_capability_validates_spans_at_boundaries() {
    let source = RegisteredSource::new(SourcePos(40), 4);
    assert_eq!(
        source.span(0, 0),
        Ok(SourceSpan::new(SourcePos(40), SourcePos(40)))
    );
    assert_eq!(
        source.span(1, 4),
        Ok(SourceSpan::new(SourcePos(41), SourcePos(44)))
    );
    assert_eq!(source.span(3, 2), Err(SourceMapError::OffsetOutsideSource));
    assert_eq!(source.span(0, 5), Err(SourceMapError::OffsetOutsideSource));
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
fn checkpoint_fork_accepts_only_the_marked_prefix_and_opens_a_private_suffix() {
    let mut parent = SourceMap::default();
    let inherited = parent
        .register(SourceId::new(0), generated(b"root"))
        .expect("root registers");
    let mark = parent.watermark();
    let parent_only = parent
        .register(SourceId::new(1), generated(b"parent"))
        .expect("parent source registers");

    let mut child = parent.fork_at(mark);
    assert_eq!(child.position(SourceId::new(0), 0), Ok(inherited));
    assert!(child.region_for_position(parent_only).is_none());
    let child_only = child
        .register(SourceId::new(1), generated(b"child"))
        .expect("child source registers");
    assert_ne!(parent_only, child_only);
    assert!(parent.region_for_position(child_only).is_none());
    assert_eq!(parent.position(SourceId::new(1), 0), Ok(parent_only));
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

impl SourceRegion {
    fn backing_generated(self) -> GeneratedSourceId {
        match self.backing {
            SourceBacking::Generated(id) => id,
            SourceBacking::World(_) => panic!("expected generated backing"),
        }
    }
}
