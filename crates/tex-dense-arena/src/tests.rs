use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

use umber_hot_core_allocator::{HotCoreAllocator, measurement, scope};

use super::*;

#[global_allocator]
static ALLOCATOR: HotCoreAllocator = HotCoreAllocator;

type Wide = [u64; 1_024];

fn value(seed: u64) -> Wide {
    let mut item = [0; 1_024];
    item[0] = seed;
    item[1_023] = !seed;
    item
}

fn push(arena: &mut DenseArena<Wide>, seed: u64) {
    arena
        .push_with(|slot| slot.insert(value(seed)))
        .expect("push wide value");
}

#[test]
fn direct_indexing_crosses_boundaries_without_descriptor_visits() {
    let mut arena = DenseArena::<Wide>::new();
    assert_eq!(DenseArena::<Wide>::items_per_block(), 8);
    for seed in 0..27 {
        push(&mut arena, seed);
    }
    for index in [0, 7, 8, 15, 16, 26, 13, 21] {
        assert_eq!(
            arena.record_direct_lookup(index).expect("value")[0],
            index as u64
        );
    }
    assert_eq!(arena.metrics().direct_lookups, 8);
    assert_eq!(arena.metrics().descriptor_visits, 0);
    assert_eq!(arena.block_ids().len(), 4);
}

#[test]
fn truncation_releases_table_suffix_and_reuses_a_stale_slot() {
    let mut arena = DenseArena::<Wide>::new();
    for seed in 0..9 {
        push(&mut arena, seed);
    }
    let second = arena.block_ids()[1];
    let keep_first = ArenaCursor {
        arena: arena.arena_id,
        len: 8,
        boundary_block: Some(arena.block_ids()[0]),
    };
    arena.truncate(keep_first).expect("truncate at boundary");
    assert!(!arena.is_live_block(second));
    push(&mut arena, 99);
    let replacement = arena.block_ids()[1];
    assert_eq!(replacement.slot, second.slot);
    assert_ne!(replacement.incarnation, second.incarnation);
    assert!(!arena.is_live_block(second));
    assert_eq!(arena.get(8).expect("replacement")[0], 99);
}

#[test]
fn cursors_reject_foreign_owners_and_future_lengths() {
    let mut first = DenseArena::<u32>::new();
    let mut second = DenseArena::<u32>::new();
    first.push_with(|slot| slot.insert(1)).expect("push");
    second.push_with(|slot| slot.insert(2)).expect("push");
    let foreign = first.cursor();
    assert!(matches!(
        second.truncate(foreign),
        Err(ArenaError::InvalidCursor)
    ));
    let future = ArenaCursor {
        arena: second.arena_id,
        len: 2,
        boundary_block: second.block_ids().first().copied(),
    };
    assert!(matches!(
        second.truncate(future),
        Err(ArenaError::InvalidCursor)
    ));
}

#[test]
fn checked_id_domains_fail_before_publication() {
    assert!(u32::try_from(u32::MAX as u64 + 1).is_err());
    let mut store = BlockStore::<u8>::new();
    let mut metrics = ArenaMetrics::default();
    let id = store.allocate(&mut metrics).expect("block");
    store.release(id, &mut metrics).expect("release");
    store.slots[id.slot as usize].incarnation = u32::MAX;
    assert!(matches!(
        store.allocate(&mut metrics),
        Err(ArenaError::IncarnationExhausted)
    ));
    assert!(!store.slots[id.slot as usize].live);
}

#[test]
fn one_and_4096_checkpoint_captures_allocate_and_copy_no_payload() {
    const OWNER: usize = 0;
    let mut arena = DenseArena::<Wide>::new();
    for seed in 0..9 {
        push(&mut arena, seed);
    }
    let before_metrics = arena.metrics();
    let before_allocations = measurement(OWNER);
    {
        let _scope = scope(OWNER);
        let _one = arena.cursor();
        for _ in 0..4_096 {
            let _mark = arena.cursor();
        }
    }
    let after_allocations = measurement(OWNER);
    let after_metrics = arena.metrics();
    assert_eq!(after_allocations.calls - before_allocations.calls, 0);
    assert_eq!(
        after_allocations.requested_bytes - before_allocations.requested_bytes,
        0
    );
    assert_eq!(
        after_metrics.values_constructed,
        before_metrics.values_constructed
    );
    assert_eq!(after_metrics.fork_tail_values_copied, 0);
    assert_eq!(
        after_metrics.cursor_captures - before_metrics.cursor_captures,
        4_097
    );
}

fn generation_with_values(count: usize) -> GenerationArena<Wide> {
    let mut arena = GenerationArena::default();
    for seed in 0..count {
        arena
            .push_with(|slot| slot.insert(value(seed as u64)))
            .expect("push generation value");
    }
    arena
}

fn checkpoint(arena: &GenerationArena<Wide>, len: usize) -> ArenaCursor {
    ArenaCursor {
        arena: arena.0.arena_id,
        len: len as u64,
        boundary_block: len
            .checked_sub(1)
            .and_then(|index| arena.block_ids().get(index / 8))
            .copied(),
    }
}

#[test]
fn fork_shares_complete_blocks_and_copies_only_checkpoint_tail() {
    let accepted = generation_with_values(22);
    let checkpoint = checkpoint(&accepted, 11);
    let mut fork = accepted.fork(checkpoint).expect("fork");
    assert_eq!(
        fork.shape(),
        ForkShape {
            accepted_blocks: 3,
            candidate_blocks: 2,
            shared_complete_blocks: 1,
            candidate_private_blocks: 1,
        }
    );
    assert_eq!(fork.candidate_get(0).expect("shared")[0], 0);
    assert_eq!(fork.candidate_get(10).expect("copied tail")[0], 10);
    assert_eq!(fork.metrics.fork_tail_values_copied, 3);
    assert_eq!(
        fork.metrics.fork_tail_bytes_copied,
        (3 * size_of::<Wide>()) as u64
    );
    assert_eq!(fork.metrics.table_entries_copied, 1);
    assert_eq!(fork.metrics.table_bytes_copied, size_of::<BlockId>() as u64);
    for seed in 100..112 {
        fork.candidate_push(value(seed)).expect("candidate append");
    }
    assert_eq!(fork.metrics.fork_tail_values_copied, 3);
    assert!(fork.shape().candidate_private_blocks >= 2);
}

#[test]
fn acceptance_moves_candidate_table_with_zero_payload_copy() {
    let accepted = generation_with_values(19);
    let prior_tail = accepted.block_ids()[1];
    let checkpoint = checkpoint(&accepted, 10);
    let mut fork = accepted.fork(checkpoint).expect("fork");
    fork.candidate_push(value(90)).expect("append");
    let copied_before = fork.metrics().fork_tail_values_copied;
    let settled = fork.accept().expect("accept");
    assert_eq!(settled.metrics().fork_tail_values_copied, copied_before);
    assert_eq!(settled.metrics().accepted_payload_copies, 0);
    assert_eq!(settled.get(10).expect("candidate value")[0], 90);
    assert!(!settled.is_live_block(prior_tail));
}

#[test]
fn rejection_restores_exact_accepted_values_and_table() {
    let accepted = generation_with_values(18);
    let accepted_ids = accepted.block_ids().to_vec();
    let checkpoint = checkpoint(&accepted, 9);
    let mut fork = accepted.fork(checkpoint).expect("fork");
    fork.candidate_push(value(77)).expect("append");
    let candidate_private = fork.candidate_blocks[1];
    let restored = fork.reject().expect("reject");
    assert_eq!(restored.block_ids(), accepted_ids);
    assert_eq!(restored.get(17).expect("accepted suffix")[0], 17);
    assert!(!restored.is_live_block(candidate_private));
    assert_eq!(restored.metrics().rejected_payload_copies, 0);
}

#[test]
fn empty_boundary_one_item_and_largest_tail_forks_are_bounded() {
    for checkpoint_len in [0, 8, 9, 15] {
        let accepted = generation_with_values(20);
        let mark = checkpoint(&accepted, checkpoint_len);
        let fork = accepted.fork(mark).expect("fork");
        let expected = checkpoint_len % 8;
        assert_eq!(fork.metrics().fork_tail_values_copied, expected as u64);
        assert!(fork.metrics().fork_tail_bytes_copied <= 65_536);
        assert_eq!(fork.shape().shared_complete_blocks, checkpoint_len / 8);
    }
}

struct OwnedDrop {
    id: usize,
    drops: Arc<Mutex<Vec<usize>>>,
}

impl Drop for OwnedDrop {
    fn drop(&mut self) {
        self.drops.lock().expect("drops").push(self.id);
    }
}

fn owned(id: usize, drops: &Arc<Mutex<Vec<usize>>>) -> OwnedDrop {
    OwnedDrop {
        id,
        drops: Arc::clone(drops),
    }
}

#[test]
fn nonforking_wrappers_preserve_distinct_owned_lifetimes() {
    let drops = Arc::new(Mutex::new(Vec::new()));
    let mut groups = GroupStorage::default();
    groups
        .push_with(|slot| slot.insert(owned(0, &drops)))
        .expect("root");
    let group = groups.enter_group();
    groups
        .push_with(|slot| slot.insert(owned(1, &drops)))
        .expect("local");
    groups.leave_group(group).expect("leave");
    assert_eq!(*drops.lock().expect("drops"), [1]);

    let mut scratch = PageAttemptScratch::default();
    let attempt = scratch.begin_attempt();
    scratch
        .push_with(|slot| slot.insert(owned(2, &drops)))
        .expect("scratch");
    scratch.rewind(attempt).expect("rewind");

    let mut journal = CheckpointJournal::default();
    let saved = journal.save();
    journal
        .push_with(|slot| slot.insert(owned(3, &drops)))
        .expect("journal");
    journal.restore(saved).expect("restore");

    let mut output = SpeculativeOutput::default();
    output
        .push_with(|slot| slot.insert(owned(4, &drops)))
        .expect("output");
    let committed = output.commit();
    assert_eq!(committed.len(), 1);
    drop(committed);
    drop(journal);
    drop(scratch);
    drop(groups);
    assert_eq!(*drops.lock().expect("drops"), [1, 2, 3, 4, 0]);
}

#[test]
fn arena_recovers_after_initializer_panic_without_an_interior_hole() {
    let mut arena = DenseArena::<String>::new();
    let result = catch_unwind(AssertUnwindSafe(|| {
        let _ = arena.push_with(|slot| {
            let _guard = slot.insert(String::from("unpublished"));
            panic!("stop")
        });
    }));
    assert!(result.is_err());
    assert_eq!(arena.len(), 0);
    arena
        .push_with(|slot| slot.insert(String::from("first")))
        .expect("recover");
    assert_eq!(arena.get(0).map(String::as_str), Some("first"));
}

struct PanicDrop {
    id: usize,
    panic: bool,
    drops: Arc<Mutex<Vec<usize>>>,
}

impl Drop for PanicDrop {
    fn drop(&mut self) {
        self.drops.lock().expect("drops").push(self.id);
        assert!(!self.panic, "requested arena destructor panic");
    }
}

#[test]
fn truncation_publishes_the_shorter_arena_prefix_before_drop_panic() {
    let drops = Arc::new(Mutex::new(Vec::new()));
    let mut arena = DenseArena::<PanicDrop>::new();
    arena
        .push_with(|slot| {
            slot.insert(PanicDrop {
                id: 0,
                panic: false,
                drops: Arc::clone(&drops),
            })
        })
        .expect("retained");
    let keep = arena.cursor();
    for (id, panic) in [(1, true), (2, false)] {
        arena
            .push_with(|slot| {
                slot.insert(PanicDrop {
                    id,
                    panic,
                    drops: Arc::clone(&drops),
                })
            })
            .expect("removed");
    }
    let result = catch_unwind(AssertUnwindSafe(|| arena.truncate(keep)));
    assert!(result.is_err());
    assert_eq!(arena.len(), 1);
    assert!(arena.get(1).is_none());
    assert_eq!(*drops.lock().expect("drops"), [2, 1]);
    drop(arena);
    assert_eq!(*drops.lock().expect("drops"), [2, 1, 0]);
}
