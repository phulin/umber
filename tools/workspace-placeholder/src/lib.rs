//! Placeholder package keeping the `tools/*` workspace glob resolvable.

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        assert!(!env!("CARGO_PKG_NAME").is_empty());
    }
}
