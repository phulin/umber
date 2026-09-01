use super::*;
use std::mem::size_of;

#[cfg(not(feature = "profiling"))]
#[global_allocator]
static STRING_POOL_TEST_ALLOCATOR: umber_hot_core_allocator::HotCoreAllocator =
    umber_hot_core_allocator::HotCoreAllocator;

#[test]
fn empty_pool_owns_no_backing_allocations() {
    let pool = RecycledStringPool::default();
    assert_eq!(pool.bytes.capacity(), 0);
    assert_eq!(pool.ends.capacity(), 0);
    assert_eq!(pool.buckets.capacity(), 0);
    assert!(pool.to_format_strings().is_empty());
}

#[test]
fn unique_spellings_have_one_compact_dense_owner() {
    let mut pool = RecycledStringPool::default();
    assert!(pool.insert("alpha"));
    assert!(pool.insert("beta"));
    assert!(!pool.insert("alpha"));

    assert_eq!(pool.len(), 2);
    assert_eq!(pool.character_len(), 9);
    assert_eq!(
        pool.to_format_strings(),
        BTreeSet::from(["alpha".to_owned(), "beta".to_owned()])
    );
    assert_eq!(
        size_of::<RecycledStringPool>(),
        size_of::<Vec<u8>>() + size_of::<Vec<u32>>() * 2
    );
    assert_eq!(size_of::<u32>(), 4, "each dense boundary/index is one word");
}

#[test]
fn cold_format_projection_roundtrips_exact_sorted_membership() {
    let format = BTreeSet::from([
        "zeta".to_owned(),
        "alpha".to_owned(),
        String::new(),
        "βeta".to_owned(),
    ]);
    let pool = RecycledStringPool::from_format_strings(&format);
    assert_eq!(pool.to_format_strings(), format);
}

#[test]
fn reserved_append_and_owner_move_reuse_all_backing_allocations() {
    let values: Vec<_> = (0..128).map(|index| format!("cs-{index:03}")).collect();
    let characters = values.iter().map(String::len).sum();
    let mut pool = RecycledStringPool::default();
    pool.reserve(values.len(), characters);
    let storage = (
        pool.bytes.as_ptr(),
        pool.ends.as_ptr(),
        pool.buckets.as_ptr(),
    );

    for value in &values {
        assert!(pool.insert(value));
    }
    assert_eq!(
        (
            pool.bytes.as_ptr(),
            pool.ends.as_ptr(),
            pool.buckets.as_ptr(),
        ),
        storage,
        "warmed capacity appends without reallocating any owner or index"
    );

    let moved = pool;
    assert_eq!(
        (
            moved.bytes.as_ptr(),
            moved.ends.as_ptr(),
            moved.buckets.as_ptr(),
        ),
        storage,
        "moving the pool moves only three vector headers"
    );
}

#[cfg(not(feature = "profiling"))]
#[test]
fn warmed_borrowed_hits_allocate_nothing_and_move_no_storage() {
    let mut pool = RecycledStringPool::default();
    pool.reserve(64, 4 * 1024);
    assert!(pool.insert("retained-control-sequence-name"));
    let storage = (
        pool.bytes.as_ptr(),
        pool.ends.as_ptr(),
        pool.buckets.as_ptr(),
    );
    let capacities = (
        pool.bytes.capacity(),
        pool.ends.capacity(),
        pool.buckets.capacity(),
    );
    const OWNER: usize = 15;
    let before = umber_hot_core_allocator::thread_measurement(OWNER);
    let hits = {
        let _scope = umber_hot_core_allocator::scope(OWNER);
        (0..4_096).all(|_| {
            !std::hint::black_box(
                pool.insert(std::hint::black_box("retained-control-sequence-name")),
            )
        })
    };
    let after = umber_hot_core_allocator::thread_measurement(OWNER);

    assert!(hits);
    assert_eq!(after.calls - before.calls, 0);
    assert_eq!(after.requested_bytes - before.requested_bytes, 0);
    assert_eq!(
        (
            pool.bytes.as_ptr(),
            pool.ends.as_ptr(),
            pool.buckets.as_ptr(),
        ),
        storage
    );
    assert_eq!(
        (
            pool.bytes.capacity(),
            pool.ends.capacity(),
            pool.buckets.capacity(),
        ),
        capacities
    );
}

#[test]
fn colliding_buckets_still_use_exact_spelling_equality() {
    let mut pool = RecycledStringPool::default();
    pool.reserve(4, 64);
    let target_bucket = string_hash(b"collision-0") as usize & (pool.buckets.len() - 1);
    let mut values = Vec::new();
    for candidate in 0..10_000 {
        let value = format!("collision-{candidate}");
        if string_hash(value.as_bytes()) as usize & (pool.buckets.len() - 1) == target_bucket {
            values.push(value);
            if values.len() == 4 {
                break;
            }
        }
    }
    assert_eq!(values.len(), 4);
    for value in &values {
        assert!(pool.insert(value));
    }
    for value in &values {
        assert!(!pool.insert(value));
    }
    assert_eq!(pool.len(), values.len());
}

#[test]
fn candidate_prefix_rejection_restores_the_detached_live_suffix() {
    let mut pool = RecycledStringPool::default();
    pool.reserve(8, 128);
    assert!(pool.insert("retained"));
    let checkpoint = pool.mark();
    assert!(pool.insert("accepted-later"));

    let suffix = pool.detach_suffix(checkpoint);
    assert!(!pool.insert("retained"));
    assert!(pool.insert("candidate-only"));
    pool.restore_suffix(checkpoint, suffix);

    assert!(!pool.insert("retained"));
    assert!(!pool.insert("accepted-later"));
    assert!(pool.insert("candidate-only"));
    assert_eq!(pool.len(), 3);
    assert_eq!(
        pool.character_len(),
        "retainedaccepted-latercandidate-only".len()
    );
}

#[test]
fn suffix_rollback_restores_membership_without_moving_warmed_storage() {
    let mut pool = RecycledStringPool::default();
    pool.reserve(8, 128);
    assert!(pool.insert("retained"));
    let storage = (
        pool.bytes.as_ptr(),
        pool.ends.as_ptr(),
        pool.buckets.as_ptr(),
    );
    let capacities = (
        pool.bytes.capacity(),
        pool.ends.capacity(),
        pool.buckets.capacity(),
    );
    let outer = pool.mark();
    assert!(pool.insert("outer-speculative"));
    let inner = pool.mark();
    assert!(pool.insert("inner-speculative"));

    pool.rollback_to(inner);
    assert!(!pool.insert("outer-speculative"));
    assert!(pool.insert("inner-speculative"));
    pool.rollback_to(outer);
    assert!(!pool.insert("retained"));
    assert!(pool.insert("outer-speculative"));
    assert!(pool.insert("inner-speculative"));
    assert_eq!(
        (
            pool.bytes.as_ptr(),
            pool.ends.as_ptr(),
            pool.buckets.as_ptr(),
        ),
        storage
    );
    assert_eq!(
        (
            pool.bytes.capacity(),
            pool.ends.capacity(),
            pool.buckets.capacity(),
        ),
        capacities
    );
}
