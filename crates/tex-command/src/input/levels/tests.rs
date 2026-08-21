use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::{
    BackupTreatment, InputLevelId, ReplayTrace, RetirementBehavior, StoredReplayReason,
    TokenBehavior, TokenCursor, TokenPayload, packed_token_frame,
};

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
fn transient_payload_is_one_owned_packed_chunk() {
    let payload = TokenPayload::<()>::transient([traced('a'), traced('b')]);
    assert_eq!(payload.frame_len(), 2);
    assert_eq!(
        payload.transient_words(),
        Some(&[traced('a'), traced('b')][..])
    );
}

#[test]
fn packed_cursor_keeps_delivery_retirement_and_trace_orthogonal() {
    let behavior = TokenBehavior::BackedUp(BackupTreatment::SuppressExpandableControlSequence);
    let retirement = RetirementBehavior::StopAtEnd;
    let trace = ReplayTrace::Stored(StoredReplayReason::EveryJob);
    let mut frame = packed_token_frame(InputLevelId(5), 1, &behavior, retirement, &trace);
    assert_eq!(frame.position(), 0);
    let _ = frame.advance();
    let cursor: TokenCursor<()> = TokenCursor {
        payload: TokenPayload::transient([traced('x')]),
        behavior,
        retirement,
        trace,
        frame,
    };

    assert_eq!(cursor.frame.position(), 1);
    assert_eq!(cursor.frame.identity(), 5);
    assert_eq!(cursor.retirement, RetirementBehavior::StopAtEnd);
    assert!(matches!(
        cursor.behavior,
        TokenBehavior::BackedUp(BackupTreatment::SuppressExpandableControlSequence)
    ));
}

#[test]
fn stored_and_transient_payloads_have_the_same_semantic_words() {
    let token = Token::Char {
        ch: 'q',
        cat: Catcode::Other,
    };
    let stored = TokenPayload::<()>::stored(&[token], [OriginId::UNKNOWN]);
    let transient =
        TokenPayload::<()>::transient([TracedTokenWord::pack(token, OriginId::UNKNOWN)]);
    assert_eq!(stored.frame_len(), transient.frame_len());
    let TokenPayload::Packed(stored) = stored else {
        panic!("stored packed payload")
    };
    let TokenPayload::Packed(transient) = transient else {
        panic!("transient packed payload")
    };
    assert_eq!(
        stored.get(0).map(|entry| entry.0),
        transient.get(0).map(|entry| entry.0)
    );
}
