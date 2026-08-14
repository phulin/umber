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
fn weak_index_is_not_ownership_authority() {
    let mut pool = ReachableValuePool::new();
    let value = pool.intern(1_u8, 42_u64, u64::eq);
    let identity = value.identity();
    pool.clear_index();

    assert_eq!(pool.resolve(identity).map(|value| *value.value()), Some(42));
    drop(value);
    let replacement = pool.intern(2_u8, 84_u64, u64::eq);
    assert_eq!(*replacement.value(), 84);
    assert!(pool.resolve(identity).is_none());
}

#[test]
fn thousands_of_dead_values_reuse_bounded_slots() {
    let mut pool = ReachableValuePool::with_index_key_budget(8);
    for value in 0..10_000_u64 {
        let root = pool.intern(value, value, u64::eq);
        assert_eq!(*root.value(), value);
        drop(root);
    }
    let retained = pool.intern(u64::MAX, u64::MAX, u64::eq);
    let (slots, capacity, index_keys, index_capacity, bucket_capacity, free) = pool.testing_shape();

    assert_eq!(*retained.value(), u64::MAX);
    assert!(slots <= 1, "dead physical slots did not plateau: {slots}");
    assert!(capacity <= 4, "slot capacity did not plateau: {capacity}");
    assert!(index_keys <= 8, "evictable index exceeded its key budget");
    assert!(
        index_capacity <= 16,
        "evictable index capacity did not plateau: {index_capacity}"
    );
    assert!(bucket_capacity <= 4);
    assert_eq!(free, 0);
}

#[test]
fn one_collision_bucket_has_a_bounded_capacity() {
    let mut pool = ReachableValuePool::new();
    for value in 0..10_000_u64 {
        let root = pool.intern(0_u8, value, u64::eq);
        drop(root);
    }
    let retained = pool.intern(0_u8, u64::MAX, u64::eq);
    let (slots, _, index_keys, _, bucket_capacity, _) = pool.testing_shape();

    assert_eq!(*retained.value(), u64::MAX);
    assert_eq!(slots, 1);
    assert_eq!(index_keys, 1);
    assert!(bucket_capacity <= 64);
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
fn live_prefix_reclamation_work_is_bounded_and_identity_safe() {
    const LIVE_ROOTS: u64 = 2_048;
    let mut pool = ReachableValuePool::with_index_key_budget(LIVE_ROOTS as usize + 2);
    let roots = (0..LIVE_ROOTS)
        .map(|value| pool.intern(value, value, u64::eq))
        .collect::<Vec<_>>();
    let transient = pool.intern(u64::MAX, u64::MAX, u64::eq);
    let transient_identity = transient.identity();
    drop(transient);

    let extent = pool.slots.len();
    let mut visited = 0;
    while pool.identities.contains(transient_identity) {
        let step = pool.reclaim_some_dead_slots(8);
        assert!(step <= 8, "ordinary reclamation must have constant work");
        visited += step;
        assert!(visited <= extent + 8, "one sweep must find the dead slot");
    }

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
