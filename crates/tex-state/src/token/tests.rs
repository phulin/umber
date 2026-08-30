use super::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};
use crate::interner::InternerBudget;

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
fn control_sequence_tokens_use_admitted_session_coordinates() {
    use crate::interner::ControlSequenceKind;

    let budget = InternerBudget::new(16, 16, 256).expect("budget");
    crate::with_universe(budget, |universe| {
        let ids = [
            (
                universe.intern("").expect("null"),
                ControlSequenceKind::Null,
            ),
            (
                universe.intern("x").expect("single"),
                ControlSequenceKind::SingleCharacter,
            ),
            (
                universe.intern("named").expect("named"),
                ControlSequenceKind::Named,
            ),
            (
                universe
                    .intern_active_character('~')
                    .expect("active character"),
                ControlSequenceKind::ActiveCharacter,
            ),
        ];

        for (id, expected_kind) in ids {
            let token = Token::Cs(id.symbol());
            assert_eq!(
                universe.control_sequence_kind(id.symbol()),
                Some(expected_kind)
            );
            assert_eq!(TokenWord::pack(token).semantic_token(), token);
        }
    })
    .expect("fresh universe");
}

#[test]
fn traced_words_compose_semantic_tokens_with_source_coordinates() {
    let tokens = [
        Token::Char {
            ch: '🙂',
            cat: Catcode::Active,
        },
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
fn out_parameter_slot_is_a_narrow_packed_projection() {
    assert_eq!(
        TokenWord::pack(Token::Param(7)).out_parameter_slot(),
        Some(7)
    );
    assert_eq!(
        TokenWord::pack(Token::Char {
            ch: '#',
            cat: Catcode::Parameter,
        })
        .out_parameter_slot(),
        None
    );
    assert_eq!(
        TokenWord::pack(Token::frozen_relax()).out_parameter_slot(),
        None
    );
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
    assert_eq!(first_arena.decode(), super::OriginEncoding::Arena(0));
    assert_eq!(
        last_arena.decode(),
        super::OriginEncoding::Arena(0x7fff_fffe)
    );
    assert_eq!(OriginId::UNKNOWN.decode(), super::OriginEncoding::Unknown);
}

#[test]
fn scalar_parameter_and_frozen_tokens_round_trip_with_origins() {
    let origin = OriginId::from_raw(42);
    for token in [
        Token::Char {
            ch: '🙂',
            cat: Catcode::Active,
        },
        Token::param(9),
        Token::frozen_end_template(),
        Token::frozen_endv(),
    ] {
        assert_eq!(
            TracedTokenWord::pack(token, origin).unpack(),
            Some((token, origin))
        );
    }
}

#[test]
fn frozen_token_kinds_remain_distinct() {
    let end_template = Token::frozen_end_template();
    let endv = Token::frozen_endv();
    let relax = Token::frozen_relax();

    assert_ne!(end_template, endv);
    assert_ne!(endv, relax);
    assert!(!matches!(end_template, Token::Cs(_)));
    let Token::Frozen(relax) = relax else {
        panic!("frozen relax must remain inaccessible");
    };
    assert_eq!(relax.primitive_index(), None);
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
    assert_eq!(bad_frozen.unpack(), None);
    assert_eq!(bad_param_zero.unpack(), None);
    assert_eq!(bad_param_ten.unpack(), None);
    assert_eq!(bad_char_scalar.unpack(), None);
}
