//! Compatibility re-export for TeX fixed-point arithmetic.
//!
//! The implementation lives in `tex-arith` so font parsing and state can share
//! TeX's scaled arithmetic without a state/font dependency cycle.

pub use tex_arith::*;
