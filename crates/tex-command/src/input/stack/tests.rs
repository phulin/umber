use super::{InputRetirementAction, InputRetirementError, InputRetirementReason};
use crate::CommandState;
use crate::input::{
    PackedTokenSpanHandle, RegisteredSourceKind, ReplayTrace, RetirementBehavior, SourceOpenDepths,
    SourceRegistration, StoredReplayReason, TokenBehavior,
};

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
fn source_retirement_moves_its_open_depth_owners() {
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
        let group_owner = open_depths.group_lineages.as_ptr();
        let conditional_owner = open_depths.conditional_identities.as_ptr();
        let (identity, _) = state
            .open_registered_file_with_depths(source, open_depths)
            .expect("source opens");

        let retirement = state
            .retire_exhausted_input(identity)
            .expect("source retires");
        let retired_depths = retirement
            .source_open_depths
            .expect("nested source owns open depths");
        assert_eq!(retired_depths.group_lineages.as_ptr(), group_owner);
        assert_eq!(
            retired_depths.conditional_identities.as_ptr(),
            conditional_owner
        );
    });
}
