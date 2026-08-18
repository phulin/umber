use core::mem::size_of;
use core::num::NonZeroU32;

use crate::interner::Symbol;
use crate::token::{Catcode, OriginId, Token, TokenWord};

use super::*;
use crate::hot_core::arena::{AcceptedRegionArena, RegionArena, RegionArenaError};

fn capacity(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("test capacity is nonzero")
}

fn append_tokens(arena: &mut RegionArena<TokenWord>, tokens: &[Token]) -> TokenSpan {
    let reservation = arena
        .reserve(capacity(tokens.len() as u32))
        .expect("bounded token reservation fits");
    for token in tokens {
        let _ = arena
            .append(reservation, TokenWord::pack(*token))
            .expect("reserved token append fits");
    }
    TokenSpan::from_region(arena.freeze(reservation).expect("token span freezes"))
}

#[test]
fn packed_runtime_values_have_fixed_documented_layouts() {
    assert_eq!(size_of::<TokenWord>(), 4);
    assert_eq!(size_of::<SourceCoordinate>(), 4);
    assert_eq!(size_of::<ChunkOwner>(), 16);
    assert_eq!(size_of::<TokenSpan>(), 24);
    assert_eq!(size_of::<InputFrameKind>(), 1);
    assert_eq!(size_of::<InputFrameFlags>(), 1);
    assert_eq!(size_of::<InputFrame>(), 40);
}

#[test]
fn frame_classification_retains_tex_token_type_values() {
    let kinds = [
        InputFrameKind::Parameter,
        InputFrameKind::AlignmentUTemplate,
        InputFrameKind::AlignmentVTemplate,
        InputFrameKind::BackedUp,
        InputFrameKind::Inserted,
        InputFrameKind::Macro,
        InputFrameKind::OutputRoutine,
        InputFrameKind::EveryPar,
        InputFrameKind::EveryMath,
        InputFrameKind::EveryDisplay,
        InputFrameKind::EveryHBox,
        InputFrameKind::EveryVBox,
        InputFrameKind::EveryJob,
        InputFrameKind::EveryCr,
        InputFrameKind::Mark,
        InputFrameKind::Write,
    ];
    for (raw, kind) in kinds.into_iter().enumerate() {
        assert_eq!(kind as usize, raw);
    }
    assert_eq!(InputFrameKind::EveryEof as u8, 16);
    assert_eq!(InputFrameKind::Source as u8, 17);
    assert_eq!(InputFrameKind::UmberReplay as u8, 18);
}

#[test]
fn admitted_frame_traversal_preserves_exact_words_and_coordinates() {
    let base = AcceptedRegionArena::new(capacity(4));
    let mut arena = base.candidate().expect("token namespace remains available");
    let tokens = [
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
        Token::Cs(Symbol::testing_new(27)),
        Token::param(9),
        Token::frozen_relax(),
    ];
    let span = append_tokens(&mut arena, &tokens);
    let trace = SourceCoordinate::from_origin(OriginId::UNKNOWN);
    let flags = InputFrameFlags::EXPAND.union(InputFrameFlags::STOP_AT_END);
    let mut frame = InputFrame::new(span, InputFrameKind::Macro, flags, 9, trace);
    let admitted = arena
        .admit_span(frame.complete_span().region())
        .expect("frame span admits once");

    assert_eq!(frame.len(), 4);
    assert_eq!(frame.position(), 0);
    assert_eq!(frame.kind(), InputFrameKind::Macro);
    assert_eq!(frame.auxiliary(), 9);
    assert_eq!(frame.trace().origin(), OriginId::UNKNOWN);
    assert!(frame.flags().contains(InputFrameFlags::EXPAND));
    assert!(
        !frame
            .flags()
            .contains(InputFrameFlags::SUPPRESS_EXPANDABLE_CONTROL_SEQUENCE)
    );

    for (index, expected) in tokens.into_iter().enumerate() {
        let coordinate = frame.next_coordinate().expect("frame has next token");
        assert_eq!(arena.resolve(coordinate), Ok(&admitted.values()[index]));
        assert_eq!(admitted.values()[index].semantic_token(), expected);
    }
    assert!(frame.is_exhausted());
    assert!(frame.remaining_span().is_empty());
    assert_eq!(frame.next_coordinate(), None);
}

#[test]
fn token_spans_reject_foreign_and_stale_chunk_owners() {
    let base = AcceptedRegionArena::new(capacity(2));
    let mut left = base.candidate().expect("left namespace remains available");
    let right = base.candidate().expect("right namespace remains available");
    let empty = left.mark().expect("empty left mark exists");
    let stale = append_tokens(
        &mut left,
        &[Token::Char {
            ch: 'a',
            cat: Catcode::Letter,
        }],
    );

    assert_eq!(
        right.resolve_span(stale.region()),
        Err(RegionArenaError::ForeignNamespace)
    );
    left.truncate(empty).expect("left suffix truncates");
    let replacement = append_tokens(
        &mut left,
        &[Token::Char {
            ch: 'b',
            cat: Catcode::Letter,
        }],
    );
    assert_eq!(stale.owner().namespace, replacement.owner().namespace);
    assert_eq!(stale.owner().slot, replacement.owner().slot);
    assert_ne!(stale.owner().generation, replacement.owner().generation);
    assert_eq!(
        left.resolve_span(stale.region()),
        Err(RegionArenaError::StaleGeneration)
    );
    assert_eq!(
        left.resolve_span(replacement.region())
            .expect("replacement span resolves")[0]
            .semantic_token(),
        Token::Char {
            ch: 'b',
            cat: Catcode::Letter,
        }
    );
}

#[test]
fn warmed_frame_construction_and_traversal_do_not_grow_storage() {
    let base = AcceptedRegionArena::new(capacity(8));
    let mut arena = base.candidate().expect("token namespace remains available");
    let empty = arena.mark().expect("empty mark exists");
    let tokens = [
        Token::Char {
            ch: 'a',
            cat: Catcode::Letter,
        },
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        },
    ];
    let warm = append_tokens(&mut arena, &tokens);
    let mut frame = InputFrame::new(
        warm,
        InputFrameKind::BackedUp,
        InputFrameFlags::empty(),
        0,
        SourceCoordinate::UNKNOWN,
    );
    while frame.next_coordinate().is_some() {}
    arena.truncate(empty).expect("warm suffix truncates");
    let plateau = arena.accounting();
    let growth_events = arena.testing_storage_growth_events();

    for _ in 0..10_000 {
        let span = append_tokens(&mut arena, &tokens);
        let mut frame = InputFrame::new(
            span,
            InputFrameKind::BackedUp,
            InputFrameFlags::RETAIN_AT_END,
            0,
            SourceCoordinate::UNKNOWN,
        );
        let admitted = arena
            .admit_span(frame.complete_span().region())
            .expect("warmed span admits");
        let mut visited = 0;
        while frame.next_coordinate().is_some() {
            let _ = admitted.values()[visited].semantic_token();
            visited += 1;
        }
        assert_eq!(visited, tokens.len());
        arena.truncate(empty).expect("bounded suffix truncates");
    }

    assert_eq!(arena.accounting(), plateau);
    assert_eq!(arena.testing_storage_growth_events(), growth_events);
}
