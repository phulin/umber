use super::{InputRetirementAction, InputRetirementError, InputRetirementReason};
use crate::CommandState;
use crate::input::{
    PackedTokenSpanHandle, ReplayTrace, RetirementBehavior, StoredReplayReason, TokenBehavior,
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
