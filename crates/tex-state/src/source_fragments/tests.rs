use std::mem;

use super::*;
use crate::input::SourceId;
use crate::source_map::{SourceDescriptor, SourceMap};

fn append(
    store: &mut FragmentStore,
    bytes: &[u8],
    revision: u64,
) -> (FragmentId, RegisteredSource) {
    store
        .append(Arc::from(bytes), revision)
        .expect("fragment appends")
}

#[test]
fn fragment_and_engine_regions_are_disjoint_and_monotonic() {
    let mut fragments = FragmentStore::new();
    let (_, first) = append(&mut fragments, b"first", 1);
    let first_span = first.span(0, 5).expect("fragment span is valid");

    let mut source_map = SourceMap::default();
    let engine = source_map
        .register(
            SourceId::new(0),
            SourceDescriptor::generated(Arc::from(&b"engine"[..])),
        )
        .expect("engine source registers");
    let (_, last) = append(&mut fragments, b"last", 2);
    let last_span = last.span(0, 4).expect("fragment span is valid");

    assert!(first_span.hi().raw() < engine.raw());
    assert!(engine.raw() < last_span.lo().raw());
    assert!(fragments.fragment_at(engine).is_none());
    assert!(source_map.region_for_position(first_span.lo()).is_none());
    assert!(source_map.region_for_position(last_span.lo()).is_none());
}

#[test]
fn deleted_fragment_position_is_typed_and_never_aliased() {
    let mut fragments = FragmentStore::new();
    let (_, registration) = append(&mut fragments, b"old", 17);
    let origin = registration.direct_origin(1, 2).expect("direct origin");
    let layout = EditorLayout::new("root.tex", LayoutGeneration::new(2), vec![], &fragments)
        .expect("empty layout is valid");

    let span = direct_fragment_span(origin, &fragments).expect("fragment origin resolves");
    assert_eq!(
        resolve_fragment_span(span, &fragments, &layout),
        Some(LayoutResolvedOrigin::Deleted {
            minted_revision: 17
        })
    );
}

#[test]
fn fragment_snapshot_survives_simulated_fork_discard() {
    let mut retained = FragmentStore::new();
    let (id, registration) = append(&mut retained, b"retained", 3);
    let installed_snapshot = retained.clone();
    let mut discarded_fork = installed_snapshot.clone();
    let (_, discarded_registration) = append(&mut discarded_fork, b"discarded", 4);
    drop(discarded_fork);

    let (_, later_registration) = append(&mut retained, b"later", 5);
    assert!(
        discarded_registration
            .span(0, 1)
            .expect("discarded fork registration is valid")
            .lo()
            .raw()
            < later_registration
                .span(0, 1)
                .expect("later retained registration is valid")
                .lo()
                .raw()
    );

    let layout = EditorLayout::new(
        "root.tex",
        LayoutGeneration::new(3),
        vec![Piece::new(id, 0, 8)],
        &installed_snapshot,
    )
    .expect("layout is valid");
    let span = registration.span(0, 1).expect("fragment span is valid");
    assert!(matches!(
        resolve_fragment_span(span, &installed_snapshot, &layout),
        Some(LayoutResolvedOrigin::Current {
            doc_offset_lo: 0,
            ..
        })
    ));
}

#[test]
fn forked_fragment_appends_mint_distinct_handles_at_the_same_dense_slot() {
    let base = FragmentStore::new();
    let mut left = base.clone();
    let mut right = base;
    let (left_id, left_registration) = append(&mut left, b"left", 1);
    let (right_id, right_registration) = append(&mut right, b"right", 1);

    assert_eq!(left_id.raw(), right_id.raw());
    assert_ne!(left_id, right_id);
    assert_eq!(left.bytes(left_id), Some(&b"left"[..]));
    assert_eq!(right.bytes(right_id), Some(&b"right"[..]));
    assert_eq!(left.bytes(right_id), None);
    assert_eq!(right.bytes(left_id), None);
    assert_ne!(
        left_registration.span(0, 1).expect("left span").lo(),
        right_registration.span(0, 1).expect("right span").lo()
    );
}

#[test]
fn cross_store_layout_and_aggregate_installation_are_rejected() {
    let mut first = FragmentStore::new();
    let (first_id, _) = append(&mut first, b"first", 1);
    let layout = EditorLayout::new(
        "root.tex",
        LayoutGeneration::new(1),
        vec![Piece::new(first_id, 0, 5)],
        &first,
    )
    .expect("first-store layout");

    let mut second = FragmentStore::new();
    let (second_id, _) = append(&mut second, b"other", 1);
    assert_eq!(first_id.raw(), second_id.raw());
    assert!(matches!(
        EditorLayout::new(
            "root.tex",
            LayoutGeneration::new(1),
            vec![Piece::new(first_id, 0, 5)],
            &second,
        ),
        Err(EditorLayoutError::UnknownFragment)
    ));

    let mut universe = crate::Universe::new();
    assert_eq!(
        universe.install_editor_fragments(&second, &layout),
        Err(EditorLayoutError::UnknownFragment)
    );
}

#[test]
fn empty_fragments_and_end_anchors_resolve_without_borrowing_a_neighbor() {
    let mut fragments = FragmentStore::new();
    let (empty_id, empty) = append(&mut fragments, b"", 1);
    let (text_id, text) = append(&mut fragments, b"abc", 1);
    let layout = EditorLayout::new(
        "root.tex",
        LayoutGeneration::new(1),
        vec![Piece::new(empty_id, 0, 0), Piece::new(text_id, 0, 3)],
        &fragments,
    )
    .expect("layout is valid");

    let empty_anchor = empty.span(0, 0).expect("empty anchor is valid");
    assert_eq!(
        resolve_fragment_span(empty_anchor, &fragments, &layout),
        Some(LayoutResolvedOrigin::Current {
            path: "root.tex".into(),
            doc_offset_lo: 0,
            doc_offset_hi: 0,
            line: 1,
            column: 1,
        })
    );

    let end_anchor = text.span(3, 3).expect("end anchor is valid");
    assert_eq!(
        resolve_fragment_span(end_anchor, &fragments, &layout),
        Some(LayoutResolvedOrigin::Current {
            path: "root.tex".into(),
            doc_offset_lo: 3,
            doc_offset_hi: 3,
            line: 1,
            column: 4,
        })
    );
}

#[test]
fn repeated_fragment_views_keep_first_covering_piece_semantics() {
    let mut fragments = FragmentStore::new();
    let (filler_id, _) = append(&mut fragments, b"----", 1);
    let (repeated_id, repeated) = append(&mut fragments, b"abcdefghij", 1);
    let layout = EditorLayout::new(
        "root.tex",
        LayoutGeneration::new(1),
        vec![
            Piece::new(filler_id, 0, 4),
            Piece::new(repeated_id, 2, 8),
            Piece::new(repeated_id, 0, 10),
        ],
        &fragments,
    )
    .expect("layout is valid");

    let overlapping = repeated.span(3, 4).expect("overlapping span is valid");
    assert!(matches!(
        resolve_fragment_span(overlapping, &fragments, &layout),
        Some(LayoutResolvedOrigin::Current {
            doc_offset_lo: 5,
            ..
        })
    ));

    let later_only = repeated.span(1, 2).expect("later-only span is valid");
    assert!(matches!(
        resolve_fragment_span(later_only, &fragments, &layout),
        Some(LayoutResolvedOrigin::Current {
            doc_offset_lo: 11,
            ..
        })
    ));
}

#[test]
fn repeated_zero_width_views_resolve_to_first_covering_anchor() {
    let mut fragments = FragmentStore::new();
    let (id, registration) = append(&mut fragments, b"abcdefgh", 1);
    let layout = EditorLayout::new(
        "root.tex",
        LayoutGeneration::new(1),
        vec![Piece::new(id, 4, 4), Piece::new(id, 0, 8)],
        &fragments,
    )
    .expect("layout is valid");
    let anchor = registration.span(4, 4).expect("anchor is valid");

    assert_eq!(
        resolve_fragment_span(anchor, &fragments, &layout),
        Some(LayoutResolvedOrigin::Current {
            path: "root.tex".into(),
            doc_offset_lo: 0,
            doc_offset_hi: 0,
            line: 1,
            column: 1,
        })
    );
}

#[test]
fn fragment_index_matches_linear_first_covering_reference() {
    let mut fragments = FragmentStore::new();
    let (id, _) = append(&mut fragments, b"abcdefgh", 1);
    let layout = EditorLayout::new(
        "root.tex",
        LayoutGeneration::new(1),
        vec![
            Piece::new(id, 4, 4),
            Piece::new(id, 2, 6),
            Piece::new(id, 0, 3),
            Piece::new(id, 5, 8),
            Piece::new(id, 0, 8),
        ],
        &fragments,
    )
    .expect("layout is valid");

    for lo in 0..=8_u64 {
        for hi in lo..=8 {
            let expected = layout
                .pieces()
                .iter()
                .enumerate()
                .find_map(|(index, piece)| {
                    let start = u64::from(piece.start());
                    let end = u64::from(piece.end());
                    let covered = if lo == hi {
                        start <= lo && lo <= end
                    } else {
                        start <= lo && lo < end && hi <= end
                    };
                    covered.then(|| {
                        let doc_lo = layout.doc_starts()[index] + lo - start;
                        (doc_lo, doc_lo + hi - lo)
                    })
                });
            assert_eq!(layout.current_range(id, lo, hi), expected, "{lo}..{hi}");
        }
    }
}

#[test]
fn gaps_between_repeated_fragment_views_remain_deleted() {
    let mut fragments = FragmentStore::new();
    let (id, registration) = append(&mut fragments, b"abcdefgh", 9);
    let layout = EditorLayout::new(
        "root.tex",
        LayoutGeneration::new(1),
        vec![Piece::new(id, 0, 2), Piece::new(id, 6, 8)],
        &fragments,
    )
    .expect("layout is valid");
    let gap = registration.span(3, 4).expect("gap span is valid");

    assert_eq!(
        resolve_fragment_span(gap, &fragments, &layout),
        Some(LayoutResolvedOrigin::Deleted { minted_revision: 9 })
    );
}

#[test]
fn line_index_is_lazy_once_per_layout_generation() {
    let mut fragments = FragmentStore::new();
    let (id, registration) = append(&mut fragments, b"a\nb", 1);
    let first = EditorLayout::new(
        "root.tex",
        LayoutGeneration::new(7),
        vec![Piece::new(id, 0, 3)],
        &fragments,
    )
    .expect("layout is valid");
    let span = registration.span(2, 3).expect("fragment span is valid");
    assert_eq!(first.line_index_build_count(), 0);
    for _ in 0..2 {
        assert!(matches!(
            resolve_fragment_span(span, &fragments, &first),
            Some(LayoutResolvedOrigin::Current {
                line: 2,
                column: 1,
                ..
            })
        ));
    }
    assert_eq!(first.line_index_build_count(), 1);

    let second = EditorLayout::new(
        "root.tex",
        LayoutGeneration::new(8),
        vec![Piece::new(id, 0, 3)],
        &fragments,
    )
    .expect("layout is valid");
    assert!(matches!(
        resolve_fragment_span(span, &fragments, &second),
        Some(LayoutResolvedOrigin::Current { line: 2, .. })
    ));
    assert_eq!(second.line_index_build_count(), 1);
}

#[test]
fn fragment_snapshot_handle_is_constant_size() {
    let mut fragments = FragmentStore::new();
    let before = mem::size_of_val(&fragments.clone());
    append(&mut fragments, &vec![b'x'; 1024 * 1024], 1);
    assert_eq!(mem::size_of_val(&fragments.clone()), before);
}

#[test]
fn pruning_waits_for_checkpoints_and_keeps_deleted_metadata_resolvable() {
    let mut fragments = FragmentStore::new();
    let (id, registration) = append(&mut fragments, "é".as_bytes(), 1);
    let origin = registration
        .direct_origin(0, 2)
        .expect("direct Unicode origin");
    let live = EditorLayout::new(
        "root.tex",
        LayoutGeneration::new(1),
        vec![Piece::new(id, 0, 2)],
        &fragments,
    )
    .expect("live layout");
    let deleted = EditorLayout::new("root.tex", LayoutGeneration::new(2), vec![], &fragments)
        .expect("deleted layout");

    assert_eq!(fragments.prune_for_layout(&live, 1, 1), 0);
    assert_eq!(fragments.prune_for_layout(&deleted, 2, 1), 0);
    assert_eq!(fragments.bytes(id), Some("é".as_bytes()));
    assert_eq!(fragments.prune_for_layout(&deleted, 2, 2), 2);
    assert_eq!(fragments.bytes(id), None);
    let span = direct_fragment_span(origin, &fragments).expect("metadata retains direct origin");
    assert_eq!(
        resolve_fragment_span(span, &fragments, &deleted),
        Some(LayoutResolvedOrigin::Deleted { minted_revision: 1 })
    );
}

#[test]
fn metadata_snapshots_do_not_pin_fragment_source_bytes() {
    let mut fragments = FragmentStore::new();
    let (id, _) = append(&mut fragments, b"source bytes", 1);
    let metadata = fragments.metadata_snapshot();

    assert_eq!(fragments.bytes(id), Some(&b"source bytes"[..]));
    assert_eq!(metadata.bytes(id), None);
    assert_eq!(metadata.source_bytes(), 0);
    assert_eq!(
        fragments.reserved_position_bytes(),
        b"source bytes".len() as u64 + 1
    );
    assert_eq!(
        fragments.retained_bytes(),
        mem::size_of::<FragmentStore>()
            + fragments.metadata_retained_bytes()
            + b"source bytes".len()
    );
}

#[test]
fn metadata_snapshots_are_o1_and_immutable_across_owner_appends() {
    let mut fragments = FragmentStore::new();
    for revision in 0..32 {
        append(&mut fragments, b"x", revision);
    }
    let metadata = fragments.metadata_snapshot();
    assert!(Arc::ptr_eq(
        fragments.fragments.root.as_ref().expect("owner root"),
        metadata.fragments.root.as_ref().expect("snapshot root")
    ));
    assert_eq!(metadata.len(), 32);
    append(&mut fragments, b"later", 33);
    assert_eq!(fragments.len(), 33);
    assert_eq!(metadata.len(), 32);
    assert_eq!(metadata.source_bytes(), 0);
}
