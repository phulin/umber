use std::sync::Arc;

use super::CandidateLeaseState;
use crate::SessionError;

#[test]
fn repeated_claim_release_reuses_one_session_allocation() {
    let state = CandidateLeaseState::new();
    let allocation = Arc::as_ptr(&state);

    for _ in 0..8_192 {
        let lease = state.claim().expect("one current lease");
        assert_eq!(Arc::as_ptr(&state), allocation);
        assert_eq!(Arc::strong_count(&state), 2);
        assert!(matches!(
            state.claim(),
            Err(SessionError::CandidateAlreadyLive)
        ));
        drop(lease);
        assert!(!state.is_claimed());
        assert_eq!(Arc::strong_count(&state), 1);
    }
}
