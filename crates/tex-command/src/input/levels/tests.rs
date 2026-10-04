use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::{
    BackedUpToken, BackupTreatment, InputLevel, InputLevelId, InputStorage, PackedTokenOwnership,
    PackedTokenSpanSource, PendingInsertion, ReplayTrace, RetirementBehavior, StoredReplayReason,
    TokenBehavior, TokenSources, backed_up_replay, packed_token_frame, stored_replay,
    transient_replay,
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
) -> InputStorage<()> {
    let mut attempt = AttemptArena::<()>::default();
    source.admit(
        words,
        TokenSources {
            attempt: &mut attempt,
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
            transient_replay([traced('a'), traced('b')]),
            &mut words,
            &mut pending,
        ),
        InputStorage::List(PackedTokenOwnership::Transient)
    );
    assert_eq!(
        admit(
            backed_up_replay([BackedUpToken {
                spelling: traced('c'),
            }]),
            &mut words,
            &mut pending,
        ),
        InputStorage::List(PackedTokenOwnership::BackedUp)
    );
    let token = traced('d').semantic_token();
    assert_eq!(
        admit(
            stored_replay(&[token], [OriginId::UNKNOWN]),
            &mut words,
            &mut pending,
        ),
        InputStorage::List(PackedTokenOwnership::Stored)
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
        InputStorage::List(PackedTokenOwnership::Transient)
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
    let row = InputLevel::<()> {
        frame,
        storage: InputStorage::List(PackedTokenOwnership::BackedUp),
    };

    assert_eq!(row.position(), 1);
    assert_eq!(row.frame.absolute_position(), 4);
    assert_eq!(row.identity(), InputLevelId(5));
    assert_eq!(row.retirement(), RetirementBehavior::StopAtEnd);
    assert_eq!(
        row.behavior(),
        TokenBehavior::BackedUp(BackupTreatment::SuppressExpandableControlSequence)
    );
}

#[test]
fn packed_omit_template_retains_its_trace_through_v_template_behavior() {
    let behavior = TokenBehavior::VTemplate;
    let retirement = RetirementBehavior::RetainExhaustedVTemplate;
    let trace = ReplayTrace::OmitTemplate;
    let frame = packed_token_frame(InputLevelId(5), 0, 1, &behavior, retirement, &trace);
    let row = InputLevel::<()> {
        frame,
        storage: InputStorage::List(PackedTokenOwnership::Transient),
    };

    assert_eq!(row.behavior(), TokenBehavior::VTemplate);
    assert_eq!(row.trace(), Some(ReplayTrace::OmitTemplate));
}
