//! Shared constants for final runtime-state performance gates.

use tex_state::interner::InternerBudget;

pub const DIRECT_READS: usize = 1_000_000;
pub const WARM_WRITES: usize = 16_384;
pub const PAGE_QUEUE_LEN: usize = 16_384;

#[must_use]
pub fn engine_budget() -> InternerBudget {
    InternerBudget::new(65_536, 131_072, 16 * 1024 * 1024).expect("benchmark interner budget")
}
