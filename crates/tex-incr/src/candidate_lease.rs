use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::SessionError;

/// Session-owned state for the one current-candidate slot.
///
/// The state is allocated once with the session. Claiming a lease only clones
/// this existing `Arc`; it does not allocate.
pub(crate) struct CandidateLeaseState {
    claimed: AtomicBool,
}

impl CandidateLeaseState {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            claimed: AtomicBool::new(false),
        })
    }

    pub(crate) fn claim(self: &Arc<Self>) -> Result<CandidateLease, SessionError> {
        self.claimed
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| SessionError::CandidateAlreadyLive)?;
        Ok(CandidateLease {
            state: Arc::clone(self),
        })
    }

    pub(crate) fn is_claimed(&self) -> bool {
        self.claimed.load(Ordering::Acquire)
    }
}

/// Move-only ownership of the session's current-candidate slot.
pub(crate) struct CandidateLease {
    state: Arc<CandidateLeaseState>,
}

impl Drop for CandidateLease {
    fn drop(&mut self) {
        let was_claimed = self.state.claimed.swap(false, Ordering::Release);
        debug_assert!(was_claimed, "candidate lease released exactly once");
    }
}

#[cfg(test)]
mod tests;
