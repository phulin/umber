use super::ShipoutScratchArena;
use crate::node::Node;

#[test]
fn reset_invalidates_ids_without_releasing_warmed_rows() {
    let mut arena = ShipoutScratchArena::<()>::default();
    let mark = arena.mark();
    let first = arena.begin_list();
    for _ in 0..64 {
        arena.push(first, Node::Penalty(1));
    }
    assert_eq!(
        arena.get(first).expect("first scratch row is live").len(),
        64
    );
    arena.reset(mark);
    assert!(arena.get(first).is_none());

    let second = arena.begin_list();
    for _ in 0..64 {
        arena.push(second, Node::Penalty(2));
    }
    assert_eq!(
        arena.get(second).expect("reused scratch row is live").len(),
        64
    );
    assert_ne!(first, second);
    assert_eq!(arena.high_water().0, 1);
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_direct_build_and_wholesale_reset_allocate_nothing() {
    let mut arena = ShipoutScratchArena::<()>::default();
    let mark = arena.mark();
    let warm = arena.begin_list();
    for _ in 0..64 {
        arena.push(warm, Node::Penalty(1));
    }
    arena.reset(mark);

    let owner = crate::measurement::HotCoreAllocationOwner::ArenaGrowth;
    let before = crate::measurement::hot_core_thread_allocation_measurement(owner);
    {
        let _scope = crate::measurement::hot_core_allocation_scope(owner);
        for _ in 0..8_192 {
            let row = arena.begin_list();
            for _ in 0..64 {
                arena.push(row, Node::Penalty(1));
            }
            arena.reset(mark);
        }
    }
    let after = crate::measurement::hot_core_thread_allocation_measurement(owner);
    assert_eq!(after.calls - before.calls, 0);
    assert_eq!(after.requested_bytes - before.requested_bytes, 0);
    assert_eq!(arena.high_water().0, 1);
}
