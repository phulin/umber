use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::{PatchAllocationDomain, PatchDomainError};

#[derive(Debug)]
struct DropSpy {
    bytes: Box<[u8]>,
    drops: Arc<AtomicUsize>,
}

impl Drop for DropSpy {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn allocate(
    domain: &mut PatchAllocationDomain,
    bytes: usize,
    drops: &Arc<AtomicUsize>,
) -> super::PatchHandle<DropSpy> {
    domain
        .allocate(
            DropSpy {
                bytes: vec![0x5a; bytes].into_boxed_slice(),
                drops: Arc::clone(drops),
            },
            bytes,
        )
        .expect("active operation owns allocation")
}

#[test]
fn failed_operations_restore_exact_suffix_and_metadata_capacity() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut domain = PatchAllocationDomain::new();
    let baseline = domain.stats();
    for attempt in 0..1_024 {
        let mark = domain.begin_operation().expect("operation begins");
        let bytes = 1 + attempt % 257;
        let _ = allocate(&mut domain, bytes, &drops);
        let _ = allocate(&mut domain, bytes * 2, &drops);
        domain
            .rollback_operation(mark)
            .expect("operation rolls back");
        assert_eq!(domain.stats(), baseline);
    }
    assert_eq!(drops.load(Ordering::SeqCst), 2_048);
}

#[test]
fn retry_keeps_earlier_private_work_once_and_drops_only_failed_suffix() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut domain = PatchAllocationDomain::new();
    let first = domain.begin_operation().expect("first operation begins");
    let retained = allocate(&mut domain, 64, &drops);
    domain
        .commit_operation(first)
        .expect("first operation commits");
    let committed = domain.stats();
    assert_eq!(domain.stats(), committed);

    for _ in 0..1_024 {
        let retry = domain.begin_operation().expect("retry operation begins");
        let abandoned = allocate(&mut domain, 4_096, &drops);
        assert_eq!(
            domain
                .get(&abandoned)
                .expect("attempt allocation is live")
                .bytes
                .len(),
            4_096
        );
        domain
            .rollback_operation(retry)
            .expect("blocked operation rolls back");
        assert_eq!(domain.stats(), committed);
        assert_eq!(
            domain
                .get(&retained)
                .expect("earlier private root remains")
                .bytes
                .len(),
            64
        );
    }
    assert_eq!(drops.load(Ordering::SeqCst), 1_024);
}

#[test]
fn acceptance_keeps_only_distinct_explicit_roots_without_the_domain() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut domain = PatchAllocationDomain::new();
    let operation = domain.begin_operation().expect("operation begins");
    let accepted = allocate(&mut domain, 32, &drops);
    let accepted_handle = accepted.clone();
    let rejected = allocate(&mut domain, 8_192, &drops);
    domain
        .commit_operation(operation)
        .expect("operation commits");
    let root = domain.root(&accepted).expect("accepted root is live");
    let rejected_root = domain.root(&rejected).expect("rejected root is live");
    drop(rejected_root);
    let accepted = domain
        .accept(vec![root.clone(), root])
        .expect("root transfer succeeds");
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted.logical_bytes(), 32);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(matches!(
        accepted.get(&rejected),
        Err(PatchDomainError::StaleRoot)
    ));
    assert_eq!(
        accepted
            .get(&accepted_handle)
            .expect("accepted payload remains independently owned")
            .bytes
            .len(),
        32
    );
    drop(accepted);
    assert_eq!(drops.load(Ordering::SeqCst), 2);
}

#[test]
fn rejection_drops_complete_domain_and_handles_retain_only_a_raw_coordinate() {
    let drops = Arc::new(AtomicUsize::new(0));
    let handle = {
        let mut domain = PatchAllocationDomain::new();
        let operation = domain.begin_operation().expect("operation begins");
        let handle = allocate(&mut domain, 1_024, &drops);
        domain
            .commit_operation(operation)
            .expect("operation commits");
        handle
    };
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_ne!(handle.owner, 0);
}

#[test]
fn repeated_accept_and_reject_retention_follows_roots_not_patch_count() {
    let drops = Arc::new(AtomicUsize::new(0));
    for revision in 0..1_024 {
        let mut domain = PatchAllocationDomain::new();
        let operation = domain.begin_operation().expect("operation begins");
        let live = allocate(&mut domain, 128, &drops);
        let _dead = allocate(&mut domain, 16_384, &drops);
        domain
            .commit_operation(operation)
            .expect("operation commits");
        if revision % 2 == 0 {
            let root = domain.root(&live).expect("live root is valid");
            let accepted = domain.accept(vec![root]).expect("revision accepts");
            assert_eq!(accepted.len(), 1);
            assert_eq!(accepted.logical_bytes(), 128);
            assert_eq!(drops.load(Ordering::SeqCst), revision * 2 + 1);
            drop(accepted);
        } else {
            drop(domain);
        }
        assert_eq!(drops.load(Ordering::SeqCst), (revision + 1) * 2);
    }
}

#[test]
fn foreign_stale_and_out_of_order_authority_fails_closed() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut first = PatchAllocationDomain::new();
    let first_operation = first.begin_operation().expect("operation begins");
    assert!(matches!(
        first.begin_operation(),
        Err(PatchDomainError::OperationAlreadyActive)
    ));
    let stale = allocate(&mut first, 8, &drops);
    first
        .rollback_operation(first_operation)
        .expect("operation rolls back");
    assert!(matches!(
        first.get(&stale),
        Err(PatchDomainError::StaleRoot)
    ));

    let second = PatchAllocationDomain::new();
    assert!(matches!(
        second.get(&stale),
        Err(PatchDomainError::ForeignRoot)
    ));
}
