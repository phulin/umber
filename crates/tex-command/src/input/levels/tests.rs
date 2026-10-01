use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::{
    BackedUpToken, BackupTreatment, InputLevelId, PackedTokenOwnership, PackedTokenSpanHandle,
    PackedTokenSpanSource, PendingInsertion, ReplayTrace, ResidentTokenRow, ResidentTokenStorage,
    RetirementBehavior, StoredReplayReason, TokenBehavior, TokenRowHeader, TokenSources,
    packed_token_frame,
};
use crate::attempt::AttemptArena;

fn traced(ch: char) -> TracedTokenWord {
    TracedTokenWord::pack(
        Token::Char {
            ch,
            cat: Catcode::Other,
        },
        OriginId::UNKNOWN,
    )
}

fn admit(
    source: impl PackedTokenSpanSource<()>,
    words: &mut Vec<TracedTokenWord>,
    pending: &mut Vec<TracedTokenWord>,
) -> ResidentTokenStorage<()> {
    let attempt = AttemptArena::<()>::default();
    source.admit(
        words,
        TokenSources {
            attempt: &attempt,
            pending_insertions: pending,
        },
    )
}

#[test]
fn every_seed_appends_its_words_to_the_token_stack() {
    let mut words = vec![traced('s')];
    let mut pending = Vec::new();
    assert_eq!(
        admit(
            PackedTokenSpanHandle::transient([traced('a'), traced('b')]),
            &mut words,
            &mut pending,
        ),
        ResidentTokenStorage::List(PackedTokenOwnership::Transient)
    );
    assert_eq!(
        admit(
            PackedTokenSpanHandle::backed_up([BackedUpToken {
                spelling: traced('c'),
            }]),
            &mut words,
            &mut pending,
        ),
        ResidentTokenStorage::List(PackedTokenOwnership::BackedUp)
    );
    let token = traced('d').semantic_token();
    assert_eq!(
        admit(
            PackedTokenSpanHandle::stored(&[token], [OriginId::UNKNOWN]),
            &mut words,
            &mut pending,
        ),
        ResidentTokenStorage::List(PackedTokenOwnership::Stored)
    );
    assert_eq!(
        words,
        [
            traced('s'),
            traced('a'),
            traced('b'),
            traced('c'),
            traced('d')
        ]
    );
}

#[test]
fn pending_insertion_moves_its_suffix_onto_the_stack() {
    let mut words = vec![traced('s')];
    let mut pending = vec![traced('p'), traced('x'), traced('y')];
    let ownership = admit(PendingInsertion { start: 1 }, &mut words, &mut pending);
    assert_eq!(
        ownership,
        ResidentTokenStorage::List(PackedTokenOwnership::Transient)
    );
    assert_eq!(words, [traced('s'), traced('x'), traced('y')]);
    assert_eq!(pending, [traced('p')], "the collected suffix is consumed");
}

#[test]
fn packed_cursor_keeps_delivery_retirement_and_trace_orthogonal() {
    let behavior = TokenBehavior::BackedUp(BackupTreatment::SuppressExpandableControlSequence);
    let retirement = RetirementBehavior::StopAtEnd;
    let trace = ReplayTrace::Stored(StoredReplayReason::EveryJob);
    let mut frame = packed_token_frame(InputLevelId(5), 3, 1, &behavior, retirement, &trace);
    assert_eq!(frame.position(), 0);
    assert_eq!(frame.start(), 3);
    let _ = frame.advance();
    let row = ResidentTokenRow::<()> {
        header: TokenRowHeader::new(frame),
        storage: ResidentTokenStorage::List(PackedTokenOwnership::BackedUp),
    };

    assert_eq!(row.header.position(), 1);
    assert_eq!(row.header.frame.absolute_position(), 4);
    assert_eq!(row.header.identity(), InputLevelId(5));
    assert_eq!(row.header.retirement(), RetirementBehavior::StopAtEnd);
    assert_eq!(
        row.header.behavior(),
        TokenBehavior::BackedUp(BackupTreatment::SuppressExpandableControlSequence)
    );
}

#[test]
fn packed_omit_template_retains_its_trace_through_v_template_behavior() {
    let behavior = TokenBehavior::VTemplate;
    let retirement = RetirementBehavior::RetainExhaustedVTemplate;
    let trace = ReplayTrace::OmitTemplate;
    let frame = packed_token_frame(InputLevelId(5), 0, 1, &behavior, retirement, &trace);
    let row = ResidentTokenRow::<()> {
        header: TokenRowHeader::new(frame),
        storage: ResidentTokenStorage::List(PackedTokenOwnership::Transient),
    };

    assert_eq!(row.header.behavior(), TokenBehavior::VTemplate);
    assert_eq!(row.trace(), ReplayTrace::OmitTemplate);
}

#[test]
fn token_rows_include_their_immutable_span_owners() {
    assert_eq!(std::mem::size_of::<super::ArgumentSpan>(), 8);
    assert_eq!(std::mem::size_of::<super::MacroBodyCursor<()>>(), 48);
    assert_eq!(std::mem::size_of::<TokenRowHeader>(), 32);
    assert_eq!(std::mem::size_of::<ResidentTokenRow<()>>(), 88);
}
