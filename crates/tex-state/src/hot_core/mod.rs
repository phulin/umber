//! Compact storage mechanics for the canonical TeX core.
//!
//! This module deliberately contains no command semantics. Its substrates are
//! introduced privately and become aggregate state through later migration
//! stages.

pub(crate) mod journal;
pub(crate) mod stack;
pub(crate) mod state;
