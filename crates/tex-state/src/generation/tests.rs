use super::with_generation;

#[test]
fn a_fresh_generation_starts_with_one_empty_arena_per_namespace() {
    with_generation(|generation| {
        assert!(generation.definitions().is_empty());
        assert!(generation.token_lists().is_empty());
        assert!(generation.glue().is_empty());
        assert!(generation.provenance().is_empty());
    });
}
