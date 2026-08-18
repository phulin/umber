use core::num::NonZeroU32;

use super::*;

fn capacity(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("test capacity is nonzero")
}

fn append_region<T: Copy>(
    arena: &mut RegionArena<T>,
    values: &[T],
) -> (Vec<RegionCoordinate<T>>, RegionSpan<T>) {
    let reservation = arena
        .reserve(capacity(values.len() as u32))
        .expect("bounded test reservation fits");
    let coordinates = values
        .iter()
        .map(|&value| {
            arena
                .append(reservation, value)
                .expect("reserved append fits")
        })
        .collect();
    let span = arena.freeze(reservation).expect("reservation freezes");
    (coordinates, span)
}

#[test]
fn coordinates_and_marks_have_fixed_compact_layouts() {
    assert_eq!(core::mem::size_of::<ChunkKey>(), 16);
    assert_eq!(core::mem::size_of::<RegionCoordinate<u8>>(), 24);
    assert_eq!(core::mem::size_of::<RegionSpan<u8>>(), 24);
    assert_eq!(core::mem::size_of::<RegionReservation<u8>>(), 24);
    assert_eq!(core::mem::size_of::<RegionArenaMark>(), 24);
}

#[test]
fn exact_coordinates_spans_and_admitted_regions_resolve() {
    let base = AcceptedRegionArena::new(capacity(8));
    let mut arena = base.candidate().expect("namespace remains available");
    let (coordinates, span) = append_region(&mut arena, &[10_u32, 20, 30]);

    assert_eq!(coordinates[0].offset(), 0);
    assert_eq!(coordinates[2].offset(), 2);
    assert_eq!(arena.resolve(coordinates[1]), Ok(&20));
    assert_eq!(arena.resolve_span(span), Ok(&[10, 20, 30][..]));
    assert_eq!(span.len(), 3);
    assert!(!span.is_empty());
    assert_eq!(
        arena.admit_span(span).expect("span admits").values(),
        &[10, 20, 30]
    );
}

#[test]
fn reservation_bounds_and_lifecycle_are_explicit() {
    let base = AcceptedRegionArena::new(capacity(2));
    let mut arena = base.candidate().expect("namespace remains available");
    let reservation = arena.reserve(capacity(1)).expect("reservation fits");

    assert!(matches!(
        arena.reserve(capacity(1)),
        Err(RegionArenaError::ReservationActive)
    ));
    let coordinate = arena.append(reservation, 7_u8).expect("one value fits");
    assert_eq!(
        arena.append(reservation, 8),
        Err(RegionArenaError::InvalidReservation)
    );
    assert_eq!(arena.resolve(coordinate), Ok(&7));
    let span = arena.freeze(reservation).expect("reservation freezes");
    assert_eq!(arena.resolve_span(span), Ok(&[7][..]));
    assert_eq!(
        arena.freeze(reservation),
        Err(RegionArenaError::InvalidReservation)
    );
}

#[test]
fn retired_slots_reuse_physical_storage_under_a_new_generation() {
    let base = AcceptedRegionArena::new(capacity(4));
    let mut arena = base.candidate().expect("namespace remains available");
    let empty = arena.mark().expect("empty mark is valid");
    let (old, _) = append_region(&mut arena, &[1_u32, 2]);
    arena.truncate(empty).expect("ancestor truncates");
    assert_eq!(arena.resolve(old[0]), Err(RegionArenaError::UnknownChunk));

    let (replacement, _) = append_region(&mut arena, &[3_u32, 4]);
    let old_parts = old[0].testing_parts();
    let replacement_parts = replacement[0].testing_parts();
    assert_eq!(old_parts.0, replacement_parts.0);
    assert_eq!(old_parts.1, replacement_parts.1);
    assert_ne!(old_parts.2, replacement_parts.2);
    assert_eq!(
        arena.resolve(old[0]),
        Err(RegionArenaError::StaleGeneration)
    );
    assert_eq!(arena.resolve(replacement[1]), Ok(&4));
}

#[test]
fn sibling_candidate_namespaces_reject_foreign_overlay_coordinates() {
    let base = AcceptedRegionArena::new(capacity(4));
    let mut left = base.candidate().expect("left namespace exists");
    let mut right = base.candidate().expect("right namespace exists");
    let (left_coordinates, _) = append_region(&mut left, &[11_u16]);
    let (right_coordinates, _) = append_region(&mut right, &[22_u16]);

    assert_eq!(
        left.resolve(right_coordinates[0]),
        Err(RegionArenaError::ForeignNamespace)
    );
    assert_eq!(
        right.resolve(left_coordinates[0]),
        Err(RegionArenaError::ForeignNamespace)
    );
    assert_eq!(left.resolve(left_coordinates[0]), Ok(&11));
    assert_eq!(right.resolve(right_coordinates[0]), Ok(&22));
}

#[test]
fn accepted_bases_share_chunks_and_candidates_isolate_overlays() {
    let empty = AcceptedRegionArena::new(capacity(4));
    let mut author = empty.candidate().expect("author namespace exists");
    let (inherited, inherited_span) = append_region(&mut author, &[1_u64, 2]);
    let accepted = author.accept().expect("frozen candidate accepts");
    assert_eq!(accepted.resolve(inherited[1]), Ok(&2));
    assert_eq!(accepted.resolve_span(inherited_span), Ok(&[1, 2][..]));

    let mut left = accepted.candidate().expect("left namespace exists");
    let right = accepted.candidate().expect("right namespace exists");
    assert!(left.base.shares_newest_layer_with(&right.base));
    assert_eq!(left.resolve(inherited[0]), Ok(&1));
    assert_eq!(right.resolve(inherited[0]), Ok(&1));

    let (left_only, _) = append_region(&mut left, &[3_u64]);
    assert_eq!(
        right.resolve(left_only[0]),
        Err(RegionArenaError::ForeignNamespace)
    );
    assert_eq!(right.accounting().logical_values, 2);
    assert_eq!(left.accounting().logical_values, 3);
}

#[test]
fn rollback_truncates_suffix_and_never_revives_old_offsets() {
    let base = AcceptedRegionArena::new(capacity(4));
    let mut arena = base.candidate().expect("namespace remains available");
    let (prefix, prefix_span) = append_region(&mut arena, &[1_u8, 2]);
    let mark = arena.mark().expect("prefix mark is valid");
    let (discarded, _) = append_region(&mut arena, &[3_u8, 4]);
    assert_eq!(discarded[0].testing_parts().1, prefix[0].testing_parts().1);

    arena.truncate(mark).expect("prefix restores");
    assert_eq!(arena.resolve_span(prefix_span), Ok(&[1, 2][..]));
    assert_eq!(
        arena.resolve(discarded[0]),
        Err(RegionArenaError::OffsetOutOfBounds)
    );

    let (replacement, _) = append_region(&mut arena, &[5_u8, 6]);
    assert_ne!(
        replacement[0].testing_parts().1,
        discarded[0].testing_parts().1,
        "a truncated suffix must not be reused in the same generation"
    );
    assert_eq!(arena.resolve(replacement[0]), Ok(&5));
    assert_eq!(
        arena.resolve(discarded[0]),
        Err(RegionArenaError::OffsetOutOfBounds)
    );
}

#[test]
fn foreign_and_non_ancestor_marks_reject_without_mutation() {
    let base = AcceptedRegionArena::new(capacity(2));
    let mut left = base.candidate().expect("left namespace exists");
    let right = base.candidate().expect("right namespace exists");
    let foreign = right.mark().expect("right mark exists");
    let (coordinate, _) = append_region(&mut left, &[9_u8]);

    assert_eq!(left.truncate(foreign), Err(RegionArenaError::InvalidMark));
    assert_eq!(left.resolve(coordinate[0]), Ok(&9));
}

#[test]
fn ten_thousand_dead_regions_plateau_without_arena_owned_heap_growth() {
    let base = AcceptedRegionArena::new(capacity(8));
    let mut arena = base.candidate().expect("namespace remains available");
    let empty = arena.mark().expect("empty mark exists");

    let (_, span) = append_region(&mut arena, &[0_u64, 1, 2, 3]);
    assert_eq!(arena.resolve_span(span), Ok(&[0, 1, 2, 3][..]));
    arena.truncate(empty).expect("warmup truncates");
    let plateau = arena.accounting();
    let growth_events = arena.testing_storage_growth_events();

    for cycle in 0..10_000_u64 {
        let reservation = arena.reserve(capacity(4)).expect("warm chunk reserves");
        let _ = arena.append(reservation, cycle).expect("first append fits");
        let _ = arena
            .append(reservation, cycle + 1)
            .expect("second append fits");
        let _ = arena
            .append(reservation, cycle + 2)
            .expect("third append fits");
        let last = arena
            .append(reservation, cycle + 3)
            .expect("fourth append fits");
        let _ = arena.freeze(reservation).expect("region freezes");
        assert_eq!(arena.resolve(last), Ok(&(cycle + 3)));
        arena.truncate(empty).expect("bounded suffix truncates");
    }

    assert_eq!(arena.accounting(), plateau);
    assert_eq!(plateau.logical_values, 0);
    assert_eq!(plateau.live_chunks, 0);
    assert_eq!(plateau.reusable_chunks, 1);
    assert_eq!(plateau.registry_slots, 1);
    assert_eq!(plateau.retained_payload_values, 8);
    assert_eq!(arena.testing_storage_growth_events(), growth_events);
}

#[test]
fn all_live_growth_has_exact_values_bytes_chunks_and_retained_payload() {
    let base = AcceptedRegionArena::new(capacity(4));
    let mut arena = base.candidate().expect("namespace remains available");
    let (first, _) = append_region(&mut arena, &[1_u32, 2, 3, 4]);
    let (second, _) = append_region(&mut arena, &[5_u32, 6, 7, 8, 9]);
    let candidate = arena.accounting();

    assert_eq!(candidate.logical_values, 9);
    assert_eq!(candidate.logical_value_bytes, 9 * size_of::<u32>());
    assert_eq!(candidate.live_chunks, 2);
    assert_eq!(candidate.reusable_chunks, 0);
    assert_eq!(candidate.registry_slots, 2);
    assert_eq!(candidate.retained_payload_values, 12);
    assert_eq!(candidate.retained_payload_bytes, 12 * size_of::<u32>());

    let accepted = arena.accept().expect("all-live candidate accepts");
    assert_eq!(accepted.accounting().logical_values, 9);
    assert_eq!(
        accepted.accounting().logical_value_bytes,
        9 * size_of::<u32>()
    );
    assert_eq!(accepted.accounting().retained_payload_values, 12);
    assert_eq!(accepted.resolve(first[3]), Ok(&4));
    assert_eq!(accepted.resolve(second[4]), Ok(&9));
}

#[test]
fn accepting_an_empty_overlay_preserves_the_exact_base_owner() {
    let empty = AcceptedRegionArena::<u8>::new(capacity(4));
    let mut author = empty.candidate().expect("author namespace exists");
    let _ = append_region(&mut author, &[1_u8]);
    let accepted = author.accept().expect("author accepts");
    let candidate = accepted.candidate().expect("candidate namespace exists");
    let unchanged = candidate.accept().expect("empty overlay accepts");

    assert!(accepted.shares_newest_layer_with(&unchanged));
    assert_eq!(accepted.accounting(), unchanged.accounting());
}
