use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::{
    BackedUpToken, BackupTreatment, InputLevelId, PackedTokenSpanHandle, PackedTokenSpanSource,
    ReplayLane, ReplayTrace, RetirementBehavior, StoredReplayReason, TokenBehavior, TokenCursor,
    packed_token_frame,
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
fn transient_payload_is_read_through_its_replay_coordinate() {
    let mut lane = ReplayLane::<()>::default();
    let payload = PackedTokenSpanHandle::<()>::transient([traced('a'), traced('b')])
        .admit(&mut lane)
        .expect("replay admission");
    assert_eq!(payload.frame_len(), 2);
    let PackedTokenSpanHandle::Replay { replay, .. } = payload else {
        panic!("transient replay coordinate")
    };
    assert_eq!(lane.get(replay, 0).map(|entry| entry.0), Some(traced('a')));
    assert_eq!(lane.get(replay, 1).map(|entry| entry.0), Some(traced('b')));
}

#[test]
fn replay_lane_retires_exactly_lifo_and_reuses_its_high_water_segment() {
    let mut lane = ReplayLane::<()>::default();
    let first = PackedTokenSpanHandle::<()>::transient([traced('a')])
        .admit(&mut lane)
        .expect("first replay admission");
    let PackedTokenSpanHandle::Replay { replay: first, .. } = first else {
        panic!("first replay coordinate")
    };
    let high_water = std::sync::Arc::as_ptr(&lane.words.active[0].storage);
    let second = PackedTokenSpanHandle::<()>::transient([traced('b')])
        .admit(&mut lane)
        .expect("second replay admission");
    let PackedTokenSpanHandle::Replay { replay: second, .. } = second else {
        panic!("second replay coordinate")
    };

    assert!(
        lane.release(first).is_err(),
        "non-top replay must stay live"
    );
    assert_eq!(lane.get(first, 0).map(|entry| entry.0), Some(traced('a')));
    lane.release(second).expect("top replay retires");
    lane.release(first).expect("older replay retires after top");

    let warmed = PackedTokenSpanHandle::<()>::transient([traced('c')])
        .admit(&mut lane)
        .expect("warmed replay admission");
    assert_eq!(
        std::sync::Arc::as_ptr(&lane.words.active[0].storage),
        high_water,
        "retired segment storage is reused"
    );
    let PackedTokenSpanHandle::Replay { replay: warmed, .. } = warmed else {
        panic!("warmed replay coordinate")
    };
    assert_eq!(lane.get(warmed, 0).map(|entry| entry.0), Some(traced('c')));
}

#[test]
fn replay_lane_clone_preserves_prior_payload_while_current_reuses_lifo_state() {
    let mut current = ReplayLane::<()>::default();
    let prior_payload = PackedTokenSpanHandle::<()>::backed_up([BackedUpToken {
        spelling: traced('p'),
        source_provenance: None,
    }])
    .admit(&mut current)
    .expect("prior replay admission");
    let PackedTokenSpanHandle::Replay { replay: prior, .. } = prior_payload else {
        panic!("prior replay coordinate")
    };
    let snapshot = current.clone();

    current.release(prior).expect("current replay retires");
    let candidate = PackedTokenSpanHandle::<()>::transient([traced('c')])
        .admit(&mut current)
        .expect("candidate replay admission");
    let PackedTokenSpanHandle::Replay {
        replay: candidate, ..
    } = candidate
    else {
        panic!("candidate replay coordinate")
    };

    assert_eq!(
        snapshot.get(prior, 0).map(|entry| entry.0),
        Some(traced('p'))
    );
    assert_eq!(
        current.get(candidate, 0).map(|entry| entry.0),
        Some(traced('c'))
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
    let mut lane = ReplayLane::default();
    let cursor: TokenCursor<()> = TokenCursor {
        span: PackedTokenSpanHandle::transient([traced('x')])
            .admit(&mut lane)
            .expect("replay admission"),
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
    let mut lane = ReplayLane::<()>::default();
    let stored = PackedTokenSpanHandle::<()>::stored(&[token], [OriginId::UNKNOWN])
        .admit(&mut lane)
        .expect("stored admission");
    let transient =
        PackedTokenSpanHandle::<()>::transient([TracedTokenWord::pack(token, OriginId::UNKNOWN)])
            .admit(&mut lane)
            .expect("transient admission");
    assert_eq!(stored.frame_len(), transient.frame_len());
    let PackedTokenSpanHandle::Replay { replay: stored, .. } = stored else {
        panic!("stored replay payload")
    };
    let PackedTokenSpanHandle::Replay {
        replay: transient, ..
    } = transient
    else {
        panic!("transient replay payload")
    };
    assert_eq!(
        lane.get(stored, 0).map(|entry| entry.0),
        lane.get(transient, 0).map(|entry| entry.0)
    );
}

#[test]
fn replay_coordinates_keep_input_frames_compact() {
    assert_eq!(
        std::mem::size_of::<crate::execution_scratch::MacroFrameId<()>>(),
        8
    );
    assert_eq!(
        std::mem::size_of::<crate::execution_scratch::MacroArgumentRange<()>>(),
        16
    );
    assert_eq!(std::mem::size_of::<PackedTokenSpanHandle<()>>(), 40);
    assert_eq!(std::mem::size_of::<TokenCursor<()>>(), 96);
    assert_eq!(std::mem::size_of::<super::MacroArgumentCursor<()>>(), 56);
    assert_eq!(std::mem::size_of::<super::InputLevel<()>>(), 96);
    assert_eq!(std::mem::size_of::<super::SourceSlotKey>(), 8);
    assert!(std::mem::size_of::<super::SourceLevel<()>>() <= 48);
    assert!(
        std::mem::size_of::<super::SourceLexExecutionState>() <= 64,
        "source lex state is {} bytes",
        std::mem::size_of::<super::SourceLexExecutionState>()
    );
}

#[test]
fn mixed_sources_share_one_cursor_and_restore_exact_nonzero_positions() {
    crate::test_harness::with_universe(|universe| {
        let mut benchmark = super::MixedPackedCursorBenchmark::new(universe);
        let receipt = benchmark.run(8);
        assert_eq!(receipt.calls, 40);
        assert_eq!(receipt.retirements, 10);
        assert_eq!(receipt.rollbacks, 1);
        assert_ne!(receipt.checksum, 0);
    });
}

#[test]
fn long_macro_argument_crosses_fixed_chunks_with_one_scalar_cursor() {
    let mut benchmark = super::LongMacroArgumentCursorBenchmark::<()>::new();
    let receipt = benchmark.run(16_390);
    assert_eq!(receipt.calls, 16_390);
    assert_eq!(receipt.retirements, 1);
    assert_eq!(receipt.rollbacks, 1);
    assert_ne!(receipt.checksum, 0);
}
