use super::*;
use proptest::prelude::*;

fn allocator(namespace: u64) -> IdentityAllocator {
    IdentityAllocator::with_namespace(
        1,
        NonZeroU64::new(namespace).expect("test namespace is nonzero"),
    )
}

#[test]
fn runtime_identity_is_two_words_and_not_a_serialized_dense_id() {
    assert_eq!(core::mem::size_of::<HandleIdentity>(), 16);
    assert_eq!(HandleIdentity::builtin(0).slot(), 0);
}

#[test]
fn rollback_exhaustion_is_atomic_and_never_wraps() {
    let mut identities = allocator(2);
    let mark = identities.watermark();
    let stale = identities.allocate().expect("test allocation fits");
    identities.active.generation =
        NonZeroU32::new(u32::MAX).expect("maximum generation is nonzero");

    assert_eq!(
        identities.rollback(mark),
        Err(IdentityError::GenerationExhausted)
    );
    assert!(identities.contains(stale));
}

#[test]
fn forks_share_ancestry_but_reject_each_others_new_handles() {
    let mut parent = allocator(2);
    let inherited = parent.allocate().expect("test allocation fits");
    let mut child = parent.fork();
    let parent_only = parent.allocate().expect("test allocation fits");
    let child_only = child.allocate().expect("test allocation fits");

    assert!(parent.contains(inherited));
    assert!(child.contains(inherited));
    assert!(!parent.contains(child_only));
    assert!(!child.contains(parent_only));
}

#[test]
fn a_snapshot_from_a_discarded_branch_is_invalidated() {
    let mut identities = allocator(2);
    identities.allocate().expect("test allocation fits");
    let old_branch = identities.watermark();
    let ancestor = IdentityMark {
        len: 1,
        frontier: Some(HandleIdentity::builtin(0).tag()),
    };
    identities
        .rollback(ancestor)
        .expect("ancestor mark remains valid");
    identities.allocate().expect("test allocation fits");

    assert_eq!(
        identities.rollback(old_branch),
        Err(IdentityError::InvalidatedMark)
    );
}

proptest! {
    #[test]
    fn arbitrary_rollback_and_reallocation_never_revives_stale_or_foreign_handles(
        cycles in prop::collection::vec(1_usize..64, 1..128)
    ) {
        let mut local = allocator(2);
        let foreign = allocator(3);
        let builtin = HandleIdentity::builtin(0);
        prop_assert!(local.contains(builtin));
        prop_assert!(foreign.contains(builtin));

        for width in cycles {
            let mark = local.watermark();
            let stale: Vec<_> = (0..width)
                .map(|_| local.allocate().expect("bounded test allocation fits"))
                .collect();
            for id in &stale {
                prop_assert!(local.contains(*id));
                prop_assert!(!foreign.contains(*id));
            }

            local.rollback(mark).expect("fresh ancestor mark is valid");
            let replacements: Vec<_> = (0..width)
                .map(|_| local.allocate().expect("bounded test allocation fits"))
                .collect();
            for (old, new) in stale.into_iter().zip(replacements) {
                prop_assert_eq!(old.slot(), new.slot());
                prop_assert!(!local.contains(old));
                prop_assert!(local.contains(new));
            }
        }
    }
}
