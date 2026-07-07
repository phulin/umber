//! Core TeX state layer. See `docs/core_state.md` for the design.

pub mod cell;
pub mod epoch;
pub mod ids;
pub mod interner;
pub mod meaning;
pub mod scaled;

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        assert!(!env!("CARGO_PKG_NAME").is_empty());
    }
}
