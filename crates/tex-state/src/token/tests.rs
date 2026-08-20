use super::{
    Catcode, OriginId, RootedTracedTokenBuffer, RootedTracedTokenWord, Token, TokenWord,
    TracedTokenWord,
};
use crate::interner::Symbol;
use crate::provenance::{InsertedOriginKind, OriginRef};

#[test]
fn token_word_round_trips_every_category_code() {
    let catcodes = [
        Catcode::Escape,
        Catcode::BeginGroup,
        Catcode::EndGroup,
        Catcode::MathShift,
        Catcode::AlignmentTab,
        Catcode::EndLine,
        Catcode::Parameter,
        Catcode::Superscript,
        Catcode::Subscript,
        Catcode::Ignored,
        Catcode::Space,
        Catcode::Letter,
        Catcode::Other,
        Catcode::Active,
        Catcode::Comment,
        Catcode::Invalid,
    ];
    for cat in catcodes {
        for ch in ['\0', 'x', '\u{10ffff}'] {
            let token = Token::Char { ch, cat };
            let word = TokenWord::pack(token);
            assert_eq!(word.token(), Some(token));
            assert_eq!(word.semantic_token(), token);
        }
    }
}

#[test]
fn token_word_round_trips_every_control_sequence_form() {
    use crate::interner::ControlSequenceKind;

    let mut universe = crate::Universe::new();
    let symbols = [
        universe.intern("").symbol(),
        universe.intern("x").symbol(),
        universe.intern("named").symbol(),
        universe.intern_active_character('~').symbol(),
        universe.intern_internal_control_sequence("frozen").symbol(),
    ];
    let expected_kinds = [
        ControlSequenceKind::Null,
        ControlSequenceKind::SingleCharacter,
        ControlSequenceKind::Named,
        ControlSequenceKind::ActiveCharacter,
        ControlSequenceKind::Internal,
    ];

    for (symbol, expected_kind) in symbols.into_iter().zip(expected_kinds) {
        let token = Token::Cs(symbol);
        assert_eq!(universe.control_sequence_kind(symbol), expected_kind);
        assert_eq!(TokenWord::pack(token).semantic_token(), token);
    }

    let boundary = Token::Cs(Symbol::new((1 << 30) - 1));
    assert_eq!(TokenWord::pack(boundary).token(), Some(boundary));
}

#[test]
fn traced_words_are_exact_token_and_source_coordinate_composition() {
    let tokens = [
        Token::Char {
            ch: '🙂',
            cat: Catcode::Active,
        },
        Token::Cs(Symbol::new((1 << 30) - 1)),
        Token::param(9),
        Token::frozen_endv(),
    ];
    let origin = OriginId::from_raw(0xfeed_beef);

    for token in tokens {
        let word = TokenWord::pack(token);
        let traced = TracedTokenWord::from_parts(word, origin);
        assert_eq!(traced.token_word(), word);
        assert_eq!(traced.semantic_token(), token);
        assert_eq!(traced.origin(), origin);
        assert_eq!(traced, TracedTokenWord::pack(token, origin));
    }
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
    assert_eq!(OriginId::default(), OriginId::UNKNOWN);
}

#[test]
fn origin_encoding_round_trips_direct_and_arena_boundaries() {
    use crate::source_map::SourcePos;

    let first_direct = OriginId::direct_source(SourcePos::from_raw_for_store(0))
        .expect("first direct position must pack");
    let last_direct = OriginId::direct_source(SourcePos::from_raw_for_store(0x7fff_fffe))
        .expect("last direct position must pack");
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
fn frozen_relax_is_outside_the_primitive_index_range() {
    let relax = Token::frozen_relax();
    let Token::Frozen(relax) = relax else {
        panic!("frozen relax must remain inaccessible");
    };
    assert_eq!(relax.primitive_index(), None);

    let last = super::FrozenToken::SENTINEL_BASE - super::FrozenToken::PRIMITIVE_BASE - 1;
    assert_eq!(
        super::FrozenToken::primitive(last).primitive_index(),
        Some(last)
    );
}

#[test]
fn invariant_fast_decode_matches_checked_decode_for_every_token_kind() {
    let tokens = [
        Token::Char {
            ch: '🙂',
            cat: Catcode::Active,
        },
        Token::Cs(Symbol::new((1 << 30) - 1)),
        Token::param(9),
        Token::frozen_endv(),
    ];
    for token in tokens {
        let packed = TracedTokenWord::pack(token, OriginId::from_raw(42));
        assert_eq!(packed.semantic_token(), token);
        assert_eq!(packed.token(), Some(token));
    }
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

    assert_eq!(TokenWord::from_raw((3_u32 << 30) | 0x1_0000).token(), None);
    assert_eq!(TokenWord::from_raw(2_u32 << 30).token(), None);
    assert_eq!(TokenWord::from_raw((2_u32 << 30) | 10).token(), None);
    assert_eq!(TokenWord::from_raw(0x11_0000_u32 << 4).token(), None);

    assert_eq!(bad_frozen.origin(), origin);
    assert_eq!(bad_frozen.unpack(), None);
    assert_eq!(bad_param_zero.unpack(), None);
    assert_eq!(bad_param_ten.unpack(), None);
    assert_eq!(bad_char_scalar.unpack(), None);
}

#[test]
fn rooted_buffer_append_preserves_token_and_origin_order() {
    let mut universe = crate::Universe::new();
    let first_token = Token::Char {
        ch: 'a',
        cat: Catcode::Letter,
    };
    let second_token = Token::Char {
        ch: 'b',
        cat: Catcode::Letter,
    };
    let first = universe.inserted_origin_ref(
        InsertedOriginKind::Unread,
        first_token,
        OriginRef::unknown(),
    );
    let second = universe.inserted_origin_ref(
        InsertedOriginKind::Unread,
        second_token,
        OriginRef::unknown(),
    );
    let mut target =
        RootedTracedTokenBuffer::new([RootedTracedTokenWord::new(first_token, first.clone())]);
    let source = RootedTracedTokenBuffer::new([
        RootedTracedTokenWord::new(first_token, first.clone()),
        RootedTracedTokenWord::new(second_token, second.clone()),
    ]);

    target.append_buffer(source);

    assert_eq!(
        target
            .words()
            .iter()
            .copied()
            .map(TracedTokenWord::semantic_token)
            .collect::<Vec<_>>(),
        [first_token, first_token, second_token]
    );
    assert_eq!(
        target
            .words()
            .iter()
            .copied()
            .map(TracedTokenWord::origin)
            .collect::<Vec<_>>(),
        [first.id(), first.id(), second.id()]
    );
}
