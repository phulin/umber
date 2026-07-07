//! Core TeX state layer. See `docs/core_state.md` for the design.

pub mod cell;
pub mod env;
pub mod epoch;
pub mod glue;
pub mod ids;
pub mod interner;
pub(crate) mod journal;
pub mod meaning;
pub mod node;
pub mod node_arena;
pub mod scaled;
pub mod stores;
pub mod token;
pub mod token_store;

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        assert!(!env!("CARGO_PKG_NAME").is_empty());
    }
}
