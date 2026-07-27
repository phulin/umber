use std::sync::Arc;

use tex_state::ids::{OriginListId, TokenListId};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::{
    BackupTreatment, InputLevel, InputLevelId, ReplayTrace, RetirementBehavior, SharedTokenBuffer,
    StoredReplayReason, TokenBehavior, TokenCursor, TokenPayload,
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
