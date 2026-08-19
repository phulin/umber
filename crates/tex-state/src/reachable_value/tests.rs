use super::ReachableValuePool;

#[test]
fn exact_equal_values_share_one_live_object() {
    let mut pool = ReachableValuePool::new();
    let first = pool.intern(7_u8, String::from("same"), String::eq);
    let second = pool.intern(7_u8, String::from("same"), String::eq);

    assert!(first.ptr_eq(&second));
    assert_eq!(first.identity(), second.identity());
}

#[test]
fn candidate_collision_cannot_alias_distinct_content() {
    let mut pool = ReachableValuePool::new();
    let first = pool.intern(0_u8, String::from("left"), String::eq);
    let second = pool.intern(0_u8, String::from("right"), String::eq);

    assert!(!first.ptr_eq(&second));
    assert_ne!(first.identity(), second.identity());
    assert_eq!(first.value(), "left");
    assert_eq!(second.value(), "right");
}

#[test]
fn region_membership_remains_authoritative_until_explicit_retirement() {
    let mut pool = ReachableValuePool::new();
    let value = pool.intern(1_u8, 42_u64, u64::eq);
    let identity = value.identity();
    pool.clear_index();

    assert_eq!(pool.resolve(identity).map(|value| *value.value()), Some(42));
    drop(value);
    assert_eq!(pool.resolve(identity).map(|value| *value.value()), Some(42));
    pool.prioritize_reclamation_from(0);
    assert!(pool.resolve(identity).is_none());
}

#[test]
fn bounded_live_values_reuse_region_slots_without_weak_authority() {
    let mut pool = ReachableValuePool::with_index_key_budget(8);
    for value in 0..10_000_u64 {
        let root = pool.intern(value, value, u64::eq);
        assert_eq!(*root.value(), value);
        drop(root);
    }
    let retained = pool.intern(u64::MAX, u64::MAX, u64::eq);
    let (slots, capacity, index_keys, index_capacity, bucket_capacity, free) = pool.testing_shape();

    assert_eq!(*retained.value(), u64::MAX);
    assert!(slots <= 9);
    assert!(capacity >= slots);
    assert_eq!(index_keys, 0);
    assert_eq!(index_capacity, 0);
    assert_eq!(bucket_capacity, 0);
    assert!(free <= slots);
}

#[test]
fn exact_collision_scan_reclaims_unowned_candidates() {
    let mut pool = ReachableValuePool::new();
    for value in 0..10_000_u64 {
        let root = pool.intern(0_u8, value, u64::eq);
        drop(root);
    }
    let retained = pool.intern(0_u8, u64::MAX, u64::eq);
    let (slots, _, index_keys, _, bucket_capacity, _) = pool.testing_shape();

    assert_eq!(*retained.value(), u64::MAX);
    assert!(slots <= 9);
    assert_eq!(index_keys, 0);
    assert_eq!(bucket_capacity, 0);
}

#[test]
fn all_roots_live_negative_control_grows_exactly() {
    let mut pool = ReachableValuePool::new();
    let roots = (0..2_048_u64)
        .map(|value| pool.intern(value, value, u64::eq))
        .collect::<Vec<_>>();
    let (slots, _, _, _, _, free) = pool.testing_shape();

    assert_eq!(slots, roots.len());
    assert_eq!(free, 0);
    assert_eq!(
        roots.iter().map(|root| *root.value()).sum::<u64>(),
        (0..2_048_u64).sum::<u64>()
    );
}

#[test]
fn explicit_suffix_rollback_is_generation_safe() {
    const LIVE_ROOTS: u64 = 2_048;
    let mut pool = ReachableValuePool::with_index_key_budget(LIVE_ROOTS as usize + 2);
    let roots = (0..LIVE_ROOTS)
        .map(|value| pool.intern(value, value, u64::eq))
        .collect::<Vec<_>>();
    let transient = pool.intern(u64::MAX, u64::MAX, u64::eq);
    let transient_identity = transient.identity();
    drop(transient);

    pool.prioritize_reclamation_from(LIVE_ROOTS as usize);

    assert!(pool.resolve(transient_identity).is_none());
    assert!(
        roots
            .iter()
            .all(|root| pool.resolve(root.identity()).is_some())
    );
    let duplicate = pool.intern(LIVE_ROOTS / 2, LIVE_ROOTS / 2, u64::eq);
    assert!(duplicate.ptr_eq(&roots[(LIVE_ROOTS / 2) as usize]));
}

#[test]
fn fork_shares_inherited_payload_but_not_future_slot_identity() {
    let mut parent = ReachableValuePool::new();
    let inherited = parent.intern(1_u8, String::from("inherited"), String::eq);
    let mut child = parent.clone();
    let child_inherited = child
        .resolve(inherited.identity())
        .expect("inherited strong root keeps the shared weak slot live");
    assert!(inherited.ptr_eq(&child_inherited));

    drop(child_inherited);
    let parent_only = parent.intern(2_u8, String::from("parent"), String::eq);
    let child_only = child.intern(3_u8, String::from("child"), String::eq);
    assert_ne!(parent_only.identity(), child_only.identity());
}
