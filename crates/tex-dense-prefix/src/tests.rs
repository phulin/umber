use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

use super::*;

#[repr(align(256))]
struct Aligned(u8);

#[test]
fn exact_layout_capacity_alignment_and_slack() {
    let block = Superblock::<Aligned>::try_new().expect("aligned block");
    assert_eq!(SUPERBLOCK_BYTES, 65_536);
    assert_eq!(Superblock::<Aligned>::capacity(), 256);
    assert_eq!(Superblock::<[u8; 3]>::capacity(), 21_845);
    assert_eq!(Superblock::<[u8; 3]>::tail_slack_bytes(), 1);
    assert_eq!(
        block.allocation.as_ptr() as usize % align_of::<Aligned>(),
        0
    );
}

#[test]
fn rejects_unsupported_layouts_before_allocation() {
    assert!(matches!(
        Superblock::<()>::try_new(),
        Err(LayoutError::ZeroSizedType)
    ));
    assert!(matches!(
        Superblock::<[u8; SUPERBLOCK_BYTES + 1]>::try_new(),
        Err(LayoutError::TypeTooLarge { .. })
    ));
}

#[test]
fn reports_allocation_failure_without_publishing_an_owner() {
    assert!(matches!(
        Superblock::<u32>::finish_allocation(core::ptr::null_mut()),
        Err(LayoutError::AllocationFailed)
    ));
}

#[test]
fn supports_one_item_blocks_and_dense_checked_access() {
    let mut block = Superblock::<[u8; SUPERBLOCK_BYTES]>::try_new().expect("one-item block");
    assert_eq!(Superblock::<[u8; SUPERBLOCK_BYTES]>::capacity(), 1);
    assert!(block.get(0).is_none());
    block
        .push_with(|slot| slot.insert([7; SUPERBLOCK_BYTES]))
        .expect("first item");
    assert_eq!(block.len(), 1);
    assert_eq!(block.get(0).expect("resident")[0], 7);
    assert!(block.get(1).is_none());
    assert!(matches!(
        block.push_with(|slot| slot.insert([8; SUPERBLOCK_BYTES])),
        Err(CapacityError)
    ));
}

#[test]
fn construction_publishes_only_after_builder_returns() {
    let mut block = Superblock::<String>::try_new().expect("block");
    let before_insert = catch_unwind(AssertUnwindSafe(|| {
        let _ = block.push_with(|_slot| panic!("before insert"));
    }));
    assert!(before_insert.is_err());
    assert_eq!(block.len(), 0);

    let after_insert = catch_unwind(AssertUnwindSafe(|| {
        let _ = block.push_with(|slot| {
            let _initialized = slot.insert(String::from("unpublished"));
            panic!("after insert")
        });
    }));
    assert!(after_insert.is_err());
    assert_eq!(block.len(), 0);

    block
        .push_with(|slot| slot.insert(String::from("published")))
        .expect("publish");
    assert_eq!(block.get(0).map(String::as_str), Some("published"));
}

struct DropRecord {
    id: usize,
    panic_at: Option<usize>,
    dropped: Arc<Mutex<Vec<usize>>>,
}

impl Drop for DropRecord {
    fn drop(&mut self) {
        self.dropped.lock().expect("drop log").push(self.id);
        assert_ne!(self.panic_at, Some(self.id), "requested destructor panic");
    }
}

#[test]
fn truncate_shortens_first_and_drains_once_in_reverse_order() {
    let dropped = Arc::new(Mutex::new(Vec::new()));
    let mut block = Superblock::<DropRecord>::try_new().expect("block");
    for id in 0..5 {
        block
            .push_with(|slot| {
                slot.insert(DropRecord {
                    id,
                    panic_at: None,
                    dropped: Arc::clone(&dropped),
                })
            })
            .expect("push");
    }
    block.truncate(2);
    assert_eq!(block.len(), 2);
    assert_eq!(*dropped.lock().expect("drop log"), [4, 3, 2]);
    drop(block);
    assert_eq!(*dropped.lock().expect("drop log"), [4, 3, 2, 1, 0]);
}

#[test]
fn destructor_panic_continues_draining_without_retry() {
    let dropped = Arc::new(Mutex::new(Vec::new()));
    let mut block = Superblock::<DropRecord>::try_new().expect("block");
    for id in 0..4 {
        block
            .push_with(|slot| {
                slot.insert(DropRecord {
                    id,
                    panic_at: Some(2),
                    dropped: Arc::clone(&dropped),
                })
            })
            .expect("push");
    }
    let result = catch_unwind(AssertUnwindSafe(|| block.truncate(1)));
    assert!(result.is_err());
    assert_eq!(block.len(), 1);
    assert_eq!(*dropped.lock().expect("drop log"), [3, 2, 1]);
    block.get_mut(0).expect("retained").panic_at = None;
    drop(block);
    assert_eq!(*dropped.lock().expect("drop log"), [3, 2, 1, 0]);
}

#[test]
fn mutable_access_is_confined_to_initialized_prefix() {
    let mut block = Superblock::<u32>::try_new().expect("block");
    block.push_with(|slot| slot.insert(3)).expect("push");
    *block.get_mut(0).expect("resident") = 9;
    assert_eq!(block.get(0), Some(&9));
    assert!(block.get_mut(1).is_none());
}

#[test]
fn copy_extension_commits_one_dense_range() {
    let mut block = Superblock::<u32>::try_new().expect("block");
    block
        .extend_copy_from_slice(&[3, 5, 8, 13])
        .expect("copy range");
    assert_eq!(block.initialized(), [3, 5, 8, 13]);
    assert_eq!(block.len(), 4);
}

#[test]
fn counters_publish_exact_requested_allocation_bytes() {
    let before = SubstrateMetrics::snapshot();
    let mut block = Superblock::<u16>::try_new().expect("block");
    block.push_with(|slot| slot.insert(1)).expect("push");
    block.truncate(0);
    drop(block);
    let change = SubstrateMetrics::snapshot() - before;
    // Other unit tests share the process-wide observability counters and may
    // run concurrently. The isolated measurement binary provides exact
    // deltas; the routine suite proves this operation contributes one complete
    // block and one balanced value lifecycle without assuming test ordering.
    assert!(change.allocation_attempts >= 1);
    assert!(change.requested_bytes >= SUPERBLOCK_BYTES as u64);
    assert_eq!(change.requested_bytes % SUPERBLOCK_BYTES as u64, 0);
    assert!(change.superblocks_allocated >= 1);
    assert!(change.superblocks_dropped >= 1);
    assert!(change.superblocks_deallocated >= 1);
    assert!(change.values_constructed >= 1);
    assert!(change.values_dropped >= 1);
}

#[test]
fn checked_coordinates_fit_wasm32_domains() {
    assert!(SUPERBLOCK_BYTES <= u32::MAX as usize);
    assert!(Superblock::<u8>::capacity() <= u32::MAX as usize);
    assert_eq!(u32::try_from(Superblock::<u8>::capacity()), Ok(65_536));
}

#[test]
fn aligned_payload_is_observably_initialized() {
    let mut block = Superblock::<Aligned>::try_new().expect("block");
    block
        .push_with(|slot| slot.insert(Aligned(17)))
        .expect("push");
    assert_eq!(block.get(0).expect("value").0, 17);
}
