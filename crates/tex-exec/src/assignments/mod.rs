//! Authoritative typed assignment commits and primitive registration.
//!
//! This module owns the canonical portion that can act on an already
//! classified assignment without either legacy token/scanner front.

pub(crate) mod committer;
mod primitives;
pub(crate) mod tracing;
pub use primitives::{
    install_etex_unexpandable_primitives, install_unexpandable_primitives,
    register_etex_unexpandable_primitives, register_unexpandable_primitives,
};

#[cfg(test)]
mod tests;
