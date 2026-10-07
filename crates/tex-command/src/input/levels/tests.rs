use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::{
    BackedUpToken, BackupTreatment, InputLevel, InputLevelId, InputStorage, PackedInputFrame,
    PackedTokenOwnership, ReplayTrace, RetirementBehavior, StoredReplayReason, TokenBehavior,
};
use crate::attempt::AttemptArena;
use crate::input::admission::{
    PackedTokenSpanSource, PendingInsertion, TokenSources, backed_up_replay, replay_flags,
    replay_kind, stored_replay, transient_replay,
};
use tex_state::packed_input::{InputFrameFlags, InputFrameKind};

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
    source
        .admit(
            words,
            TokenSources {
                attempt: &mut attempt,
                pending_insertions: pending,
            },
            InputLevelId(1),
            None,
            InputFrameKind::Inserted,
            InputFrameFlags::empty(),
        )
        .storage
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
        InputStorage::List {
            start: 1,
            ownership: PackedTokenOwnership::Transient,
            kind: InputFrameKind::Inserted,
            flags: InputFrameFlags::empty()
        }
    );
    assert_eq!(
        admit(
            backed_up_replay([BackedUpToken {
                spelling: traced('c'),
            }]),
            &mut words,
            &mut pending,
        ),
        InputStorage::List {
            start: 3,
            ownership: PackedTokenOwnership::BackedUp,
            kind: InputFrameKind::Inserted,
            flags: InputFrameFlags::empty()
        }
    );
    let token = traced('d').semantic_token();
    assert_eq!(
        admit(
            stored_replay(&[token], [OriginId::UNKNOWN]),
            &mut words,
            &mut pending,
        ),
        InputStorage::List {
            start: 4,
            ownership: PackedTokenOwnership::Stored,
            kind: InputFrameKind::Inserted,
            flags: InputFrameFlags::empty()
        }
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
        InputStorage::List {
            start: 1,
            ownership: PackedTokenOwnership::Transient,
            kind: InputFrameKind::Inserted,
            flags: InputFrameFlags::empty()
        }
    );
    assert_eq!(words, [traced('s'), traced('x'), traced('y')]);
    assert_eq!(pending, [traced('p')], "the collected suffix is consumed");
}

#[test]
fn packed_cursor_keeps_delivery_retirement_and_trace_orthogonal() {
    let behavior = TokenBehavior::BackedUp(BackupTreatment::SuppressExpandableControlSequence);
    let retirement = RetirementBehavior::StopAtEnd;
    let trace = ReplayTrace::Stored(StoredReplayReason::EveryJob);
    let mut frame = PackedInputFrame::tokens(5, 1, None);
    assert_eq!(frame.position(), 0);
    let _ = frame.advance();
    let row = InputLevel::<()> {
        frame,
        storage: InputStorage::List {
            start: 3,
            ownership: PackedTokenOwnership::BackedUp,
            kind: replay_kind(&behavior, &trace),
            flags: replay_flags(&behavior, retirement),
        },
    };

    assert_eq!(row.position(), 1);
    assert!(matches!(row.storage, InputStorage::List { start: 3, .. }));
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
    let frame = PackedInputFrame::tokens(5, 1, None);
    let row = InputLevel::<()> {
        frame,
        storage: InputStorage::List {
            start: 0,
            ownership: PackedTokenOwnership::Transient,
            kind: replay_kind(&behavior, &trace),
            flags: replay_flags(&behavior, retirement),
        },
    };

    assert_eq!(row.behavior(), TokenBehavior::VTemplate);
    assert_eq!(row.trace(), Some(ReplayTrace::OmitTemplate));
}
