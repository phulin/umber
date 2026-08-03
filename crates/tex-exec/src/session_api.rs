use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutionBudgets {
    pub steps: u64,
    pub input_frames: u64,
    pub journal_bytes: u64,
    pub effects: u64,
}

impl Default for ExecutionBudgets {
    fn default() -> Self {
        Self {
            steps: u64::MAX,
            input_frames: u64::MAX,
            journal_bytes: u64::MAX,
            effects: u64::MAX,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExecutionBudgetCounters {
    pub committed_steps: u64,
    pub cumulative_fuel: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Clone, Debug, Default)]
pub struct PendingInterrupt(Arc<AtomicBool>);

impl PendingInterrupt {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request(&self) {
        self.0.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_pending(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    pub(crate) fn take(&self) -> bool {
        self.0.swap(false, Ordering::AcqRel)
    }
}
