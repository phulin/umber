use std::sync::Arc;

use tex_state::ids::{OriginListId, TokenListId};
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
    let cursor = TokenCursor {
        payload: TokenPayload::Stored {
            tokens: TokenListId::EMPTY,
            origins: OriginListId::EMPTY,
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
