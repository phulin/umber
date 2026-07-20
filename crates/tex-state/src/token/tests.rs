use super::{Catcode, OriginId, Token, TracedTokenWord};
use crate::interner::Symbol;

#[test]
fn token_is_one_word() {
    assert_eq!(core::mem::size_of::<Token>(), 8);
    assert_eq!(core::mem::size_of::<OriginId>(), 4);
    assert_eq!(core::mem::size_of::<TracedTokenWord>(), 8);
}

#[test]
fn token_variants_are_copy_and_comparable() {
    let char_token = Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    };
    let cs_token = Token::Cs(Symbol::new(7));
    let param_token = Token::param(3);

    assert_eq!(char_token, char_token);
    assert_eq!(cs_token, Token::Cs(Symbol::new(7)));
    assert_eq!(param_token, Token::Param(3));
}

#[test]
fn origin_zero_is_unknown() {
    assert_eq!(OriginId::UNKNOWN.raw(), 0);
    assert_eq!(OriginId::default(), OriginId::UNKNOWN);
}

#[test]
fn origin_encoding_has_exact_direct_and_arena_boundaries() {
    use crate::source_map::SourcePos;

    let first_direct = OriginId::direct_source(SourcePos::from_raw_for_store(0))
        .expect("first direct position must pack");
    let last_direct = OriginId::direct_source(SourcePos::from_raw_for_store(0x7fff_fffe))
        .expect("last direct position must pack");
    assert_eq!(first_direct.raw(), 1);
    assert_eq!(last_direct.raw(), 0x7fff_ffff);
    assert!(OriginId::direct_source(SourcePos::from_raw_for_store(0x7fff_ffff)).is_none());
    assert_eq!(
        first_direct.decode(),
        super::OriginEncoding::DirectSource(SourcePos::from_raw_for_store(0))
    );
    assert_eq!(
        last_direct.decode(),
        super::OriginEncoding::DirectSource(SourcePos::from_raw_for_store(0x7fff_fffe))
    );

    let first_arena = OriginId::arena(0).expect("first arena index must pack");
    let last_arena = OriginId::arena(0x7fff_fffe).expect("last arena index must pack");
    assert_eq!(first_arena.raw(), 0x8000_0000);
    assert_eq!(last_arena.raw(), 0xffff_fffe);
    assert!(OriginId::arena(0x7fff_ffff).is_none());
    assert!(OriginId::arena(0x8000_0000).is_none());
    assert_eq!(first_arena.decode(), super::OriginEncoding::Arena(0));
    assert_eq!(
        last_arena.decode(),
        super::OriginEncoding::Arena(0x7fff_fffe)
    );
    assert_eq!(OriginId::UNKNOWN.decode(), super::OriginEncoding::Unknown);
    assert_eq!(
        OriginId::NOEXPAND_FALLBACK.decode(),
        super::OriginEncoding::NoExpandFallback
    );
}

#[test]
fn char_token_round_trips_with_origin() {
    let origin = OriginId::from_raw(42);
    let token = Token::Char {
        ch: '🙂',
        cat: Catcode::Active,
    };

    let packed = TracedTokenWord::pack(token, origin);

    assert_eq!(packed.unpack(), Some((token, origin)));
}

#[test]
fn control_sequence_token_round_trips_with_origin() {
    let origin = OriginId::from_raw(u32::MAX);
    let token = Token::Cs(Symbol::new((1 << 30) - 1));

    let packed = TracedTokenWord::pack(token, origin);

    assert_eq!(packed.unpack(), Some((token, origin)));
}

#[test]
fn parameter_token_round_trips_with_origin() {
    let origin = OriginId::from_raw(7);
    let token = Token::param(9);

    let packed = TracedTokenWord::pack(token, origin);

    assert_eq!(packed.unpack(), Some((token, origin)));
}

#[test]
fn frozen_alignment_tokens_round_trip_as_distinct_non_symbol_tokens() {
    let origin = OriginId::from_raw(23);
    let end_template = Token::frozen_end_template();
    let endv = Token::frozen_endv();

    assert_ne!(end_template, endv);
    assert!(!matches!(end_template, Token::Cs(_)));
    assert_eq!(
        TracedTokenWord::pack(end_template, origin).unpack(),
        Some((end_template, origin))
    );
    assert_eq!(
        TracedTokenWord::pack(endv, origin).unpack(),
        Some((endv, origin))
    );
}

#[test]
fn packed_token_decode_rejects_unrepresentable_payloads() {
    let origin = OriginId::from_raw(99);
    let bad_frozen = TracedTokenWord::from_raw(
        (3_u64 << 62) | (u64::from(u16::MAX) + 1) << 32 | u64::from(origin.raw()),
    );
    let bad_param_zero = TracedTokenWord::from_raw(2_u64 << 62);
    let bad_param_ten = TracedTokenWord::from_raw((2_u64 << 62) | (10_u64 << 32));
    let bad_char_scalar = TracedTokenWord::from_raw(0x11_0000_u64 << 36);

    assert_eq!(bad_frozen.origin(), origin);
    assert_eq!(bad_frozen.unpack(), None);
    assert_eq!(bad_param_zero.unpack(), None);
    assert_eq!(bad_param_ten.unpack(), None);
    assert_eq!(bad_char_scalar.unpack(), None);
}
