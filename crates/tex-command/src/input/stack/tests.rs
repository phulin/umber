use super::{InputRetirementAction, InputRetirementError, InputRetirementReason};
use crate::CommandState;
use crate::input::{
    PackedTokenSpanHandle, RegisteredSourceKind, ReplayTrace, RetirementBehavior, SourceOpenDepths,
    SourceRegistration, StoredReplayReason, TokenBehavior,
};

fn load_top_source_line<G>(state: &mut CommandState<G>) {
    state
        .input
        .levels
        .mutate_top_source_cursor(|_, slot| {
            slot.cursor.load_next_line(13).expect("source line loads");
        })
        .expect("source remains on top");
}

#[test]
fn retirement_receipt_is_copy_small_and_owns_no_replay_trace() {
    assert!(std::mem::size_of::<super::InputRetirement>() <= 64);
    assert!(std::mem::size_of::<InputRetirementReason>() <= 2);
    assert!(std::mem::size_of::<ReplayTrace>() <= 2);
    assert!(!std::mem::needs_drop::<ReplayTrace>());
}

#[test]
fn one_exhausted_token_level_retires_once_with_its_semantic_reason() {
    crate::test_harness::with_universe(|_universe| {
        let mut state = CommandState::<()>::default();
        let identity = state.push_token_level(
            PackedTokenSpanHandle::transient([]),
            TokenBehavior::Ordinary,
            RetirementBehavior::StopAtEnd,
            ReplayTrace::Stored(StoredReplayReason::EveryJob),
        );

        let retirement = state
            .retire_exhausted_input(identity)
            .expect("exact level retires");
        assert_eq!(retirement.identity, identity);
        assert_eq!(retirement.action, InputRetirementAction::TerminalStop);
        assert_eq!(
            retirement.reason,
            InputRetirementReason::TokenList(StoredReplayReason::EveryJob)
        );
        assert_eq!(
            state.retire_exhausted_input(identity),
            Err(InputRetirementError::NoInput)
        );
    });
}

#[test]
fn attempt_input_admission_rejects_a_mismatched_extent() {
    let mut state = CommandState::<()>::default();
    let list = state
        .attempt
        .profile_allocate_token_list([crate::test_harness::traced(tex_state::token::Token::Char {
            ch: 'x',
            cat: tex_state::token::Catcode::Other,
        })])
        .expect("attempt token list allocates");

    assert_eq!(
        state.push_attempt_list_level(
            list,
            2,
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        ),
        Err(crate::AttemptError::InvalidCoordinate)
    );
    assert_eq!(state.input_level_count(), 0);
}

#[test]
fn retirement_rejects_a_stale_level_identity_before_mutation() {
    crate::test_harness::with_universe(|_universe| {
        let mut state = CommandState::<()>::default();
        let older = state.push_token_level(
            PackedTokenSpanHandle::transient([]),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let current = state.push_token_level(
            PackedTokenSpanHandle::transient([]),
            TokenBehavior::Recovery,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        );

        assert_eq!(
            state.retire_exhausted_input(older),
            Err(InputRetirementError::LevelChanged {
                expected: older,
                actual: current,
            })
        );
        assert_eq!(state.top_input_level_identity(), Some(current));
    });
}

#[test]
fn canonical_push_keeps_runtime_maximum_across_root_rollback() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let source = state
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                &b""[..],
            ))
            .expect("source registers");
        state.open_registered_source(source).expect("source opens");
        let checkpoint = state.snapshot(universe).expect("command checkpoint");
        state.push_token_level(
            PackedTokenSpanHandle::transient([]),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        );
        state.push_token_level(
            PackedTokenSpanHandle::transient([]),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        );
        assert_eq!(state.stack_usage().input_stack, 2);

        state
            .rollback(&checkpoint, universe)
            .expect("command rollback");
        assert_eq!(state.input_level_count(), 1);
        assert_eq!(state.stack_usage().input_stack, 2);
    });
}

#[test]
fn occupied_source_buffer_slots_stay_exact_at_deep_input_depth() {
    crate::test_harness::with_universe(|_universe| {
        const DEPTH: usize = 4_096;
        let mut state = CommandState::<()>::default();
        for _ in 0..DEPTH {
            let source = state
                .register_source(SourceRegistration::new(
                    RegisteredSourceKind::Generated,
                    &b"x"[..],
                ))
                .expect("source registers");
            state.open_registered_source(source).expect("source opens");
            load_top_source_line(&mut state);
        }

        assert_eq!(state.input.levels.occupied_source_buffer_slots(), DEPTH * 3);
        state.record_csname_buffer_usage(5);
        assert_eq!(state.stack_usage().buffer_stack, DEPTH * 3 + 7);

        for remaining in (0..DEPTH).rev() {
            state
                .input
                .levels
                .pop_project(|_, _| ())
                .expect("source retires");
            assert_eq!(
                state.input.levels.occupied_source_buffer_slots(),
                remaining * 3
            );
        }
    });
}

#[test]
fn unicode_source_read_does_not_recount_the_loaded_line() {
    const SCALARS: usize = 8_192;
    let text = "λ".repeat(SCALARS);
    let mut state = CommandState::<()>::new(crate::CommandProfile::unicode_extended(
        crate::CommandDialect::Tex82,
    ));
    let source = state
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            text.into_bytes(),
        ))
        .expect("Unicode source registers");
    state.open_registered_source(source).expect("source opens");

    crate::input::source::reset_source_buffer_slot_measurement();
    load_top_source_line(&mut state);
    let acquired = crate::input::source::source_buffer_slot_measurement();
    assert_eq!(acquired.unicode_scalars_counted, SCALARS);
    assert_eq!(
        state.input.levels.occupied_source_buffer_slots(),
        SCALARS + 2
    );

    let mut delivered = 0usize;
    while state.next_source_character().is_some() {
        delivered += 1;
    }
    assert_eq!(delivered, SCALARS + 1);
    assert_eq!(
        crate::input::source::source_buffer_slot_measurement(),
        acquired,
        "ordinary Unicode cursor advancement must not recount the retained line"
    );
}

#[test]
fn source_line_replacement_and_candidate_settlement_restore_buffer_slots() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let root = state
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                &b"a\nlonger"[..],
            ))
            .expect("root source registers");
        state
            .open_registered_source(root)
            .expect("root source opens");
        load_top_source_line(&mut state);
        assert_eq!(state.input.levels.occupied_source_buffer_slots(), 3);
        let checkpoint = state.publish_summary(universe).expect("source checkpoint");

        load_top_source_line(&mut state);
        assert_eq!(state.input.levels.occupied_source_buffer_slots(), 8);
        let nested = state
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                &b"xy"[..],
            ))
            .expect("nested source registers");
        state
            .open_registered_source(nested)
            .expect("nested source opens");
        load_top_source_line(&mut state);
        assert_eq!(state.input.levels.occupied_source_buffer_slots(), 12);

        let mut candidate = CommandState::fork_summary(state, &checkpoint, universe, universe)
            .expect("checkpoint candidate opens");
        assert_eq!(candidate.input.levels.occupied_source_buffer_slots(), 3);
        candidate.reject_checkpoint_candidate();
        assert_eq!(candidate.input.levels.occupied_source_buffer_slots(), 12);

        candidate.input.levels.pop_project(|_, _| ());
        assert_eq!(candidate.input.levels.occupied_source_buffer_slots(), 8);
        candidate.input.levels.pop_project(|_, _| ());
        assert_eq!(candidate.input.levels.occupied_source_buffer_slots(), 0);
    });
}

#[test]
fn firmed_line_replacement_owner_survives_candidate_rollback() {
    struct ReplaceLine;

    impl crate::SourceStepQueries for ReplaceLine {
        fn catcode(&mut self, _code: crate::CharacterCode) -> tex_state::token::Catcode {
            unreachable!("firm-up probe never tokenizes")
        }

        fn firm_up_the_line(&mut self, line: &str) -> Option<SourceRegistration> {
            assert_eq!(line, "a");
            Some(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                &b"xy"[..],
            ))
        }
    }

    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let root = state
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                &b"a"[..],
            ))
            .expect("root source registers");
        state
            .open_registered_source(root)
            .expect("root source opens");
        let checkpoint = state.publish_summary(universe).expect("source checkpoint");

        state
            .acquire_input_top_line_with_queries(13, true, false, &mut ReplaceLine)
            .expect("source transition")
            .expect("physical line");
        assert_eq!(state.input.levels.occupied_source_buffer_slots(), 4);
        let (_, slot) = state
            .input
            .levels
            .top_source()
            .expect("source remains live");
        assert_eq!(&*slot.cursor.current_backing().bytes, b"xy");

        let mut candidate = CommandState::fork_summary(state, &checkpoint, universe, universe)
            .expect("checkpoint candidate opens");
        assert_eq!(candidate.input.levels.occupied_source_buffer_slots(), 0);
        candidate.reject_checkpoint_candidate();
        assert_eq!(candidate.input.levels.occupied_source_buffer_slots(), 4);
        let (_, slot) = candidate
            .input
            .levels
            .top_source()
            .expect("replacement owner is restored");
        assert_eq!(&*slot.cursor.current_backing().bytes, b"xy");
    });
}

#[test]
fn source_retirement_returns_only_the_prepared_copy_boundary() {
    crate::test_harness::with_universe(|_universe| {
        let mut state = CommandState::<()>::default();
        let source = state
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                &b""[..],
            ))
            .expect("source registers");
        let open_depths = SourceOpenDepths {
            group_lineages: vec![11, 12].into_boxed_slice(),
            conditional_identities: vec![21, 22].into_boxed_slice(),
        };
        let (identity, _) = state
            .open_registered_file_with_depths(source, open_depths)
            .expect("source opens");

        let retirement = state
            .retire_exhausted_input_with_file_warning(
                identity,
                Some(super::FileWarningBoundary {
                    group_start: 1,
                    condition_start: 2,
                }),
            )
            .expect("source retires");
        assert_eq!(
            retirement.file_warning_boundary,
            Some(super::FileWarningBoundary {
                group_start: 1,
                condition_start: 2,
            })
        );
    });
}
