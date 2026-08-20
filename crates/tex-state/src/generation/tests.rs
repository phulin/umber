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
        generation.definitions_mut().allocate(&[], &[]).unwrap();
        generation.token_lists_mut().allocate(&[]).unwrap();
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
        let mut owner = GenerationOwner::new(generation);
        let retained = owner.clone();
        assert!(owner.same_generation(&retained));
        assert!(owner.generation_mut().is_none());
        drop(retained);
        assert!(owner.generation_mut().is_some());
        owner.retire().expect("last coarse owner");
    });
}
