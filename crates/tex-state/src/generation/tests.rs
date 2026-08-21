use super::{GenerationOwner, with_generation};

#[test]
fn a_fresh_generation_starts_with_one_empty_arena_per_namespace() {
    with_generation(|generation| {
        assert!(generation.definitions().is_empty());
        assert!(generation.token_lists().is_empty());
        assert!(generation.glue().is_empty());
        assert!(generation.provenance().is_empty());
    });
}

#[test]
fn retirement_consumes_the_complete_generation_owner() {
    with_generation(|mut generation| {
        generation
            .definitions_mut()
            .allocate(&[], &[])
            .expect("test fixture is valid");
        generation
            .token_lists_mut()
            .allocate(&[])
            .expect("test fixture is valid");
        let retired = generation.retire();
        assert_eq!(retired.definitions, 1);
        assert_eq!(retired.token_lists, 1);
        assert_eq!(retired.glue_values, 0);
        assert_eq!(retired.provenance_records, 0);
    });
}

#[test]
fn coarse_owner_is_the_only_cloneable_generation_lifetime_authority() {
    with_generation(|generation| {
        let owner = GenerationOwner::new(generation);
        let retained = owner.clone();
        assert!(owner.same_generation(&retained));
        owner
            .generation_mut()
            .token_lists_mut()
            .allocate(&[])
            .expect("retained coarse owners do not freeze append-only arenas");
        let owner = owner
            .retire()
            .expect_err("retained owner prevents retirement");
        drop(retained);
        let retired = owner
            .retire()
            .expect("last coarse owner retires generation");
        assert_eq!(retired.token_lists, 1);
    });
}
