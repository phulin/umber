use std::sync::Arc;

use tex_state::Universe;
use tex_state::ids::TokenListId;
use tex_state::provenance::SyntheticOriginKind;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::{
    BackedUpToken, BackupTreatment, InputLevel, InputLevelId, ReplayTrace, RetirementBehavior,
    SharedBackedUpBuffer, SharedTokenBuffer, StoredReplayReason, TokenBehavior, TokenCursor,
    TokenPayload,
};
use crate::macro_call::MacroArgumentRange;

fn traced(ch: char) -> TracedTokenWord {
    TracedTokenWord::pack(
        Token::Char {
            ch,
            cat: Catcode::Other,
        },
        OriginId::UNKNOWN,
    )
}

#[test]
fn token_cursor_classifies_orthogonal_ownership_domains() {
    let universe = Universe::new();
    let cursor = TokenCursor {
        payload: TokenPayload::Stored {
            tokens: universe.token_list_ref(TokenListId::EMPTY),
            origins: tex_state::provenance::OriginListRef::empty(),
        },
        behavior: TokenBehavior::BackedUp(BackupTreatment::SuppressExpandableControlSequence),
        retirement: RetirementBehavior::StopAtEnd,
        trace: ReplayTrace::Stored(StoredReplayReason::EveryJob),
        index: 3,
        identity: InputLevelId(5),
    };

    let TokenCursor {
        payload,
        behavior,
        retirement,
        trace,
        index,
        identity,
    } = cursor;
    assert!(matches!(payload, TokenPayload::Stored { .. }));
    assert!(matches!(
        behavior,
        TokenBehavior::BackedUp(BackupTreatment::SuppressExpandableControlSequence)
    ));
    assert_eq!(retirement, RetirementBehavior::StopAtEnd);
    assert_eq!(trace, ReplayTrace::Stored(StoredReplayReason::EveryJob));
    assert_eq!(index, 3);
    assert_eq!(identity, InputLevelId(5));
}

#[test]
fn macro_argument_ranges_share_one_contiguous_allocation() {
    let allocation: Arc<[TracedTokenWord]> = vec![traced('a'), traced('b'), traced('c')].into();
    let buffer = SharedTokenBuffer::new(Arc::clone(&allocation));
    let payload = TokenPayload::ArgumentRange {
        buffer: buffer.clone(),
        range: MacroArgumentRange::new(1, 3).expect("valid argument range"),
    };

    drop(allocation);
    assert_eq!(buffer.len(), 3);
    let TokenPayload::ArgumentRange { buffer, range } = payload else {
        panic!("argument payload changed variant");
    };
    assert_eq!(buffer.len(), 3);
    assert_eq!((range.start(), range.end()), (1, 3));
}

#[test]
fn backed_up_buffer_prepends_without_reversing_existing_tokens() {
    let mut buffer = SharedBackedUpBuffer::new(vec![
        BackedUpToken {
            spelling: traced('b'),
            source_provenance: None,
        },
        BackedUpToken {
            spelling: traced('c'),
            source_provenance: None,
        },
    ]);

    buffer.prepend([BackedUpToken {
        spelling: traced('a'),
        source_provenance: None,
    }]);

    assert_eq!(
        (0..3)
            .map(|index| buffer.get(index).expect("token exists").spelling)
            .collect::<Vec<_>>(),
        [traced('a'), traced('b'), traced('c')]
    );
}

#[test]
fn single_token_payload_constructors_select_inline_storage() {
    let transient = TokenPayload::transient([traced('a')]);
    assert!(matches!(transient, TokenPayload::InlineTransient(word) if word == traced('a')));

    let backed_up = TokenPayload::backed_up([BackedUpToken {
        spelling: traced('b'),
        source_provenance: None,
    }]);
    assert!(matches!(
        backed_up,
        TokenPayload::InlineBackedUp(BackedUpToken { spelling, .. }) if spelling == traced('b')
    ));
}

#[test]
fn multi_token_payload_constructors_select_shared_storage() {
    let transient = TokenPayload::transient([traced('a'), traced('b')]);
    assert!(matches!(
        transient,
        TokenPayload::Transient(words) if words.words() == [traced('a'), traced('b')]
    ));

    let backed_up = TokenPayload::backed_up([
        BackedUpToken {
            spelling: traced('a'),
            source_provenance: None,
        },
        BackedUpToken {
            spelling: traced('b'),
            source_provenance: None,
        },
    ]);
    assert!(matches!(
        backed_up,
        TokenPayload::BackedUp(words)
            if words.words().iter().map(|word| word.spelling).collect::<Vec<_>>()
                == [traced('a'), traced('b')]
    ));
}

#[test]
fn prepend_promotes_inline_backup_without_reordering() {
    let mut payload = TokenPayload::backed_up([BackedUpToken {
        spelling: traced('c'),
        source_provenance: None,
    }]);
    payload
        .prepend_backed_up([
            BackedUpToken {
                spelling: traced('a'),
                source_provenance: None,
            },
            BackedUpToken {
                spelling: traced('b'),
                source_provenance: None,
            },
        ])
        .expect("backed-up payload promotes");

    assert!(matches!(payload, TokenPayload::BackedUp(_)));
    assert_eq!(
        payload
            .backed_up_words()
            .expect("backed-up words")
            .iter()
            .map(|word| word.spelling)
            .collect::<Vec<_>>(),
        [traced('a'), traced('b'), traced('c')]
    );
}

#[test]
fn inline_payload_origin_adoption_matches_shared_semantics() {
    let mut universe = tex_state::Universe::new();
    let recorded_origin = universe.synthetic_origin(SyntheticOriginKind::Engine);
    let live_origin = universe.synthetic_origin(SyntheticOriginKind::Primitive);
    let token = Token::Char {
        ch: 'a',
        cat: Catcode::Other,
    };
    let mut recorded = TokenPayload::transient([TracedTokenWord::pack(token, recorded_origin)]);
    let live = TokenPayload::transient([TracedTokenWord::pack(token, live_origin)]);

    recorded
        .adopt_matching_origins(&live)
        .expect("matching inline token adopts live origin");
    assert_eq!(
        recorded.transient_words().expect("transient words")[0].origin(),
        live_origin
    );
}

#[test]
fn inline_backup_rehomes_source_provenance_in_place() {
    let old_source = tex_state::SourceId::new(3);
    let new_source = tex_state::SourceId::new(7);
    let provenance =
        crate::SourceProvenance::from_range(crate::SourceRange::new(old_source, 10, 12));
    let mut payload = TokenPayload::backed_up([BackedUpToken {
        spelling: traced('a'),
        source_provenance: Some(provenance),
    }]);

    payload
        .rehome_backed_up_source(new_source, 5)
        .expect("inline source provenance rehomes");
    let rehomed = payload.backed_up_words().expect("backed-up words")[0]
        .source_provenance
        .expect("source provenance remains present");
    assert_eq!(rehomed.range(), crate::SourceRange::new(new_source, 15, 17));
    assert_eq!(
        rehomed.location(),
        crate::SourceLocation::new(new_source, 16)
    );
}

#[test]
fn the_dense_level_enum_has_only_source_and_token_variants() {
    fn classify(level: &InputLevel) -> &'static str {
        match level {
            InputLevel::Source(_) => "source",
            InputLevel::Tokens(_) => "tokens",
        }
    }

    let level = InputLevel::Tokens(TokenCursor {
        payload: TokenPayload::Transient(SharedTokenBuffer::new(Vec::<TracedTokenWord>::new())),
        behavior: TokenBehavior::Ordinary,
        retirement: RetirementBehavior::Pop,
        trace: ReplayTrace::Inserted,
        index: 0,
        identity: InputLevelId(0),
    });
    assert_eq!(classify(&level), "tokens");
}

#[test]
fn stored_token_encoding_partitions_characters_controls_and_macro_forms() {
    let mut universe = tex_state::Universe::new();
    let control = universe.intern("control").symbol();
    let cases = [
        Token::Char {
            ch: 'A',
            cat: Catcode::Letter,
        },
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        },
        Token::Cs(control),
        Token::param(1),
        Token::param(9),
    ];

    for (index, token) in cases.into_iter().enumerate() {
        let traced = TracedTokenWord::pack(token, OriginId::UNKNOWN);
        assert_eq!(traced.semantic_token(), token, "case {index}");
        assert_eq!(traced.origin(), OriginId::UNKNOWN, "case {index}");
    }
    assert_eq!(core::mem::size_of::<Token>(), 8);
    assert_ne!(cases[0], cases[1]);
    assert_ne!(cases[2], cases[3]);
    assert_ne!(cases[3], cases[4]);
}
