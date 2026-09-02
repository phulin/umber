use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::{
    BackedUpToken, BackupTreatment, InputLevelId, PackedTokenSpanHandle, PackedTokenSpanSource,
    ReplayLane, ReplayTokenRow, ReplayTrace, RetirementBehavior, StoredReplayReason, TokenBehavior,
    TokenRowHeader, packed_token_frame,
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
    assert_eq!(lane.indexed_get_cold(replay, 0), Some(traced('a')));
    assert_eq!(lane.indexed_get_cold(replay, 1), Some(traced('b')));
}

#[test]
fn escaping_input_builder_keeps_final_ownership_across_a_snapshot() {
    let mut lane = ReplayLane::<()>::default();
    let builder = lane.begin_input_builder().expect("escaping input owner");
    lane.push_input_builder_word(builder, traced('a'))
        .expect("first final-owner word");
    let mut snapshot = lane.clone();
    lane.push_input_builder_word(builder, traced('b'))
        .expect("post-snapshot final-owner word");

    let payload = lane
        .finish_input_builder(builder)
        .expect("escaping input coordinate");
    assert_eq!(payload.frame_len(), 2);
    let PackedTokenSpanHandle::Replay { replay, .. } = payload else {
        panic!("escaping input replay coordinate")
    };
    assert_eq!(lane.indexed_get_cold(replay, 0), Some(traced('a')));
    assert_eq!(lane.indexed_get_cold(replay, 1), Some(traced('b')));

    let snapshot_payload = snapshot
        .finish_input_builder(builder)
        .expect("snapshot keeps its admitted prefix");
    let PackedTokenSpanHandle::Replay {
        replay: snapshot_replay,
        ..
    } = snapshot_payload
    else {
        panic!("snapshot replay coordinate")
    };
    assert_eq!(
        snapshot.indexed_get_cold(snapshot_replay, 0),
        Some(traced('a'))
    );
    assert_eq!(snapshot.indexed_get_cold(snapshot_replay, 1), None);
    assert!(
        lane.words.active.is_empty(),
        "escaping words are written only into their final replay entry"
    );
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
    assert_eq!(lane.indexed_get_cold(first, 0), Some(traced('a')));
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
    assert_eq!(lane.indexed_get_cold(warmed, 0), Some(traced('c')));
}

#[test]
fn replay_lane_clone_preserves_prior_payload_while_current_reuses_lifo_state() {
    let mut current = ReplayLane::<()>::default();
    let prior_payload = PackedTokenSpanHandle::<()>::backed_up([BackedUpToken {
        spelling: traced('p'),
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

    assert_eq!(snapshot.indexed_get_cold(prior, 0), Some(traced('p')));
    assert_eq!(current.indexed_get_cold(candidate, 0), Some(traced('c')));
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
    let payload = PackedTokenSpanHandle::transient([traced('x')])
        .admit(&mut lane)
        .expect("replay admission");
    let PackedTokenSpanHandle::Replay { replay, .. } = payload else {
        unreachable!("transient input is replay-owned")
    };
    let cursor: ReplayTokenRow<()> = ReplayTokenRow {
        replay,
        resident: lane
            .resident_cursor(replay)
            .expect("resident replay cursor"),
        header: TokenRowHeader::new(behavior, retirement, trace, frame),
    };

    assert_eq!(cursor.header.frame.position(), 1);
    assert_eq!(cursor.header.frame.identity(), 5);
    assert_eq!(cursor.header.retirement, RetirementBehavior::StopAtEnd);
    assert!(matches!(
        cursor.header.behavior,
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
        lane.indexed_get_cold(stored, 0),
        lane.indexed_get_cold(transient, 0)
    );
}

#[test]
fn replay_coordinates_keep_input_frames_compact() {
    assert_eq!(
        std::mem::size_of::<crate::execution_scratch::ArgumentSetId<()>>(),
        8
    );
    assert_eq!(
        std::mem::size_of::<crate::execution_scratch::MacroArgumentRange<()>>(),
        16
    );
    assert_eq!(std::mem::size_of::<PackedTokenSpanHandle<()>>(), 40);
    assert_eq!(std::mem::size_of::<TokenRowHeader<()>>(), 48);
    assert_eq!(std::mem::size_of::<tex_state::ResidentMacroBodyCursor>(), 4);
    assert_eq!(std::mem::size_of::<super::RowRollbackMarker>(), 8);
    assert_eq!(std::mem::size_of::<super::MacroBodyCursor<()>>(), 72);
    assert_eq!(std::mem::size_of::<super::MacroArgumentCursor<()>>(), 56);
    assert_eq!(std::mem::size_of::<super::ReplayTokenRow<()>>(), 72);
    assert_eq!(std::mem::size_of::<super::DurableTokenRow<()>>(), 80);
    assert_eq!(std::mem::size_of::<super::AttemptTokenRow<()>>(), 72);
    assert_eq!(std::mem::size_of::<super::InputLevel<()>>(), 88);
    assert_eq!(std::mem::size_of::<super::SourceSlotKey>(), 8);
    assert_eq!(std::mem::size_of::<super::SourceLevel<()>>(), 48);
    assert!(
        std::mem::size_of::<super::SourceLexExecutionState>() <= 64,
        "source lex state is {} bytes",
        std::mem::size_of::<super::SourceLexExecutionState>()
    );
}

#[test]
fn sequential_replay_inspects_only_crossed_segment_boundaries() {
    for segments in [1_usize, 64, 4_096] {
        let words = segments * super::REPLAY_SEGMENT_ITEMS;
        let mut lane = ReplayLane::<()>::default();
        let payload =
            PackedTokenSpanHandle::<()>::transient(std::iter::repeat_n(traced('x'), words))
                .admit(&mut lane)
                .expect("adversarial replay span admits");
        let PackedTokenSpanHandle::Replay { replay, .. } = payload else {
            unreachable!("transient span is replay-owned")
        };
        let mut cursor = lane
            .resident_cursor(replay)
            .expect("adversarial span has a resident cursor");
        let mut inspections = 0;
        let mut run_transitions = 0;
        for _ in 0..words {
            assert_eq!(
                lane.advance_sequential(
                    replay,
                    &mut cursor,
                    &mut inspections,
                    &mut run_transitions,
                ),
                Some(traced('x'))
            );
        }
        assert_eq!(
            lane.advance_sequential(replay, &mut cursor, &mut inspections, &mut run_transitions,),
            None
        );
        assert_eq!(inspections, (segments - 1) as u64);
        assert_eq!(run_transitions, 0);
    }
}

#[test]
fn sequential_replay_exhaustion_stops_before_a_mid_segment_read() {
    let mut lane = ReplayLane::<()>::default();
    let payload = PackedTokenSpanHandle::<()>::transient(['a', 'b', 'c'].into_iter().map(traced))
        .admit(&mut lane)
        .expect("short replay span admits");
    let PackedTokenSpanHandle::Replay { replay, .. } = payload else {
        unreachable!("transient span is replay-owned")
    };
    PackedTokenSpanHandle::<()>::transient(['x', 'y'].into_iter().map(traced))
        .admit(&mut lane)
        .expect("following span shares the active segment");
    let mut cursor = lane.resident_cursor(replay).expect("short resident cursor");
    let mut inspections = 0;
    let mut run_transitions = 0;
    for expected in ['a', 'b', 'c'].map(traced) {
        assert_eq!(
            lane.advance_sequential(replay, &mut cursor, &mut inspections, &mut run_transitions),
            Some(expected)
        );
    }
    assert_eq!(cursor.remaining, 0);
    assert_ne!(cursor.offset, cursor.segment_end);
    assert_eq!(
        lane.advance_sequential(replay, &mut cursor, &mut inspections, &mut run_transitions),
        None
    );
    assert_eq!(inspections, 0);
    assert_eq!(run_transitions, 0);
}

#[test]
fn sequential_replay_exhaustion_guard_is_present_in_every_build() {
    let source = include_str!("../levels.rs");
    let body = source
        .split_once(
            "fn advance_sequential(\n        &self,\n        cursor: &mut ResidentReplayCursor,",
        )
        .expect("locate segmented resident replay advance")
        .1
        .split_once("/// Crosses the uncommon exhausted-span")
        .expect("locate resident replay advance boundary")
        .0;
    let exhaustion = body
        .find("if cursor.remaining == 0")
        .expect("logical-span exhaustion check");
    let function_body = body
        .find(") -> Option<&T> {")
        .map(|offset| offset + ") -> Option<&T> {".len())
        .expect("segmented resident replay function body");
    let read = body
        .find("let segment = self.active.get")
        .expect("direct segment read");
    assert!(exhaustion < read);
    assert!(!body[function_body..exhaustion].contains("#[cfg(test)]"));
}

#[test]
fn sequential_replay_crosses_prefix_body_and_owned_runs_exactly() {
    let mut lane = ReplayLane::<()>::default();
    let payload = PackedTokenSpanHandle::<()>::backed_up(std::iter::repeat_n(
        BackedUpToken {
            spelling: traced('b'),
        },
        300,
    ))
    .admit(&mut lane)
    .expect("backed-up body admits");
    let PackedTokenSpanHandle::Replay { replay, .. } = payload else {
        unreachable!("backed-up span is replay-owned")
    };
    lane.prepend_backed_up(
        replay,
        std::iter::repeat_n(
            BackedUpToken {
                spelling: traced('p'),
            },
            300,
        ),
    )
    .expect("prefix admits");
    let mut cursor = lane.resident_cursor(replay).expect("prefixed cursor");
    let mut inspections = 0;
    let mut run_transitions = 0;
    for expected in
        std::iter::repeat_n(traced('p'), 300).chain(std::iter::repeat_n(traced('b'), 300))
    {
        assert_eq!(
            lane.advance_sequential(replay, &mut cursor, &mut inspections, &mut run_transitions,),
            Some(expected)
        );
    }
    assert_eq!(inspections, 3);
    assert_eq!(run_transitions, 1);

    let builder = lane.begin_input_builder().expect("owned builder");
    for _ in 0..300 {
        lane.push_input_builder_word(builder, traced('o'))
            .expect("owned word");
    }
    let owned = lane.finish_input_builder(builder).expect("owned replay");
    let PackedTokenSpanHandle::Replay { replay, .. } = owned else {
        unreachable!("owned span is replay-owned")
    };
    let mut cursor = lane.resident_cursor(replay).expect("owned cursor");
    let mut inspections = 0;
    let mut run_transitions = 0;
    for _ in 0..300 {
        assert_eq!(
            lane.advance_sequential(replay, &mut cursor, &mut inspections, &mut run_transitions,),
            Some(traced('o'))
        );
    }
    assert_eq!(inspections, 1);
    assert_eq!(run_transitions, 0);
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

#[test]
fn long_macro_argument_default_matches_new() {
    let mut constructed = super::LongMacroArgumentCursorBenchmark::<()>::new();
    let mut defaulted = super::LongMacroArgumentCursorBenchmark::<()>::default();
    assert_eq!(defaulted.run(16_390), constructed.run(16_390));
}

#[test]
fn resident_macro_cursor_layout_stays_compact_and_wrapper_free() {
    let body = std::mem::size_of::<super::MacroBodyCursor<()>>();
    let argument = std::mem::size_of::<super::MacroArgumentCursor<()>>();
    eprintln!("body={body} argument={argument}");
    assert!(body <= 72, "macro body cursor is {body} bytes");
    assert!(argument <= 56, "macro argument cursor is {argument} bytes");
}
