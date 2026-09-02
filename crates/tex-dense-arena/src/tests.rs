use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

use umber_hot_core_allocator::{HotCoreAllocator, measurement, scope};

use super::*;
use crate::store::BlockId;

#[global_allocator]
static ALLOCATOR: HotCoreAllocator = HotCoreAllocator;

type Wide = [u64; 1_024];

fn value(seed: u64) -> Wide {
    let mut item = [0; 1_024];
    item[0] = seed;
    item[1_023] = !seed;
    item
}

fn push_dense(arena: &mut DenseArena<Wide>, seed: u64) {
    arena
        .push_with(|slot| slot.insert(value(seed)))
        .expect("push wide value");
}

fn push_logical(
    store: &mut BlockStore<Wide>,
    table: &mut AcceptedBlockTable<Wide>,
    seed: u64,
) -> LogicalPosition {
    table
        .push_with(store, |slot| slot.insert(value(seed)))
        .expect("push logical value")
}

#[test]
fn nonforking_direct_indexing_crosses_boundaries_without_descriptors() {
    let mut arena = DenseArena::<Wide>::new();
    assert_eq!(DenseArena::<Wide>::items_per_block(), 8);
    for seed in 0..27 {
        push_dense(&mut arena, seed);
    }
    for index in [0, 7, 8, 15, 16, 26, 13, 21] {
        assert_eq!(
            arena.record_direct_lookup(index).expect("value")[0],
            index as u64
        );
    }
    assert_eq!(arena.metrics().direct_lookups, 8);
    assert_eq!(arena.metrics().descriptor_visits, 0);
    assert_eq!(arena.physical_blocks().len(), 4);
}

#[test]
fn nonforking_cursors_reject_foreign_owners_and_reuse_blocks() {
    let mut first = DenseArena::<Wide>::new();
    let mut second = DenseArena::<Wide>::new();
    for seed in 0..8 {
        push_dense(&mut first, seed);
    }
    let keep = first.cursor();
    let foreign = keep;
    push_dense(&mut first, 8);
    assert_eq!(first.physical_blocks().len(), 2);
    first.truncate(keep).expect("truncate at boundary");
    assert_eq!(first.physical_blocks().len(), 1);
    push_dense(&mut first, 99);
    assert_eq!(first.metrics().superblocks_reused, 1);
    assert_eq!(first.get(8).expect("replacement")[0], 99);
    assert!(matches!(
        second.truncate(foreign),
        Err(ArenaError::InvalidCursor)
    ));
}

#[test]
fn physical_slots_reject_stale_ids_and_fail_reuse_before_mutation() {
    let mut store = BlockStore::<u8>::new();
    let stale = store.allocate().expect("block");
    store.release(stale).expect("release");
    let replacement = store.allocate().expect("reuse");
    assert_ne!(stale, replacement);
    assert!(matches!(
        store.resolve(stale),
        Err(ArenaError::StalePhysicalBlock)
    ));
    store.release(replacement).expect("release replacement");
    store.force_incarnation_exhaustion(replacement);
    assert!(matches!(
        store.allocate(),
        Err(ArenaError::IncarnationExhausted)
    ));
    assert_eq!(store.reusable_blocks(), 1);
    assert_eq!(store.live_blocks(), 0);
}

#[test]
fn logical_coordinates_are_pool_stable_and_reuse_increments_incarnation() {
    let mut store = BlockStore::<Wide>::new();
    let mut table = AcceptedBlockTable::new();
    let mut positions = Vec::new();
    for seed in 0..8 {
        positions.push(push_logical(&mut store, &mut table, seed));
    }
    let keep = table.cursor();
    let stale = push_logical(&mut store, &mut table, 8);
    table.truncate(&mut store, keep).expect("truncate row");
    let replacement = push_logical(&mut store, &mut table, 99);
    assert_eq!(stale.block().ordinal(), replacement.block().ordinal());
    assert_ne!(
        stale.block().incarnation(),
        replacement.block().incarnation()
    );
    let view = table.view(&store);
    assert!(matches!(
        view.get(stale),
        Err(ArenaError::StaleLogicalBlock)
    ));
    assert_eq!(view.get(replacement).expect("replacement")[0], 99);
    assert_eq!(view.get(positions[7]).expect("retained")[0], 7);
    assert_eq!(
        view.get(positions[0].checked_add_offset(7).expect("offset"))
            .expect("advanced")[0],
        7
    );
    assert_eq!(table.metrics().logical_rows_reused, 1);
    assert_eq!(table.metrics().logical_stale_rejections, 1);
    assert_eq!(store.metrics().superblocks_reused, 1);
    assert_eq!(store.metrics().physical_stale_rejections, 0);
}

#[test]
fn one_and_4096_checkpoint_captures_allocate_and_copy_nothing() {
    const OWNER: usize = 0;
    let mut store = BlockStore::<Wide>::new();
    let mut table = AcceptedBlockTable::new();
    for seed in 0..9 {
        push_logical(&mut store, &mut table, seed);
    }
    let before = table.metrics();
    let before_allocations = measurement(OWNER);
    {
        let _scope = scope(OWNER);
        let _one = table.cursor();
        for _ in 0..4_096 {
            let _mark = table.cursor();
        }
    }
    let allocations = measurement(OWNER);
    let after = table.metrics();
    assert_eq!(allocations.calls - before_allocations.calls, 0);
    assert_eq!(
        allocations.requested_bytes - before_allocations.requested_bytes,
        0
    );
    assert_eq!(after.fork_tail_values_copied, 0);
    assert_eq!(after.table_entries_copied, 0);
    assert_eq!(after.cursor_captures - before.cursor_captures, 4_097);
}

fn logical_fork_fixture(
    count: usize,
    checkpoint_len: usize,
) -> (
    BlockStore<Wide>,
    AcceptedBlockTable<Wide>,
    LogicalCursor,
    Vec<LogicalPosition>,
) {
    let mut store = BlockStore::new();
    let mut table = AcceptedBlockTable::new();
    let mut checkpoint = (checkpoint_len == 0).then(|| table.cursor());
    let mut positions = Vec::new();
    for seed in 0..count {
        positions.push(push_logical(&mut store, &mut table, seed as u64));
        if seed + 1 == checkpoint_len {
            checkpoint = Some(table.cursor());
        }
    }
    (store, table, checkpoint.expect("checkpoint"), positions)
}

#[test]
fn exactly_two_views_share_complete_blocks_and_diverge_at_one_tail() {
    let (mut store, table, checkpoint, accepted_positions) = logical_fork_fixture(22, 11);
    let mut fork = table
        .fork(&mut store, checkpoint)
        .unwrap_or_else(|(error, _)| panic!("fork: {error}"));
    assert_eq!(
        fork.shape(),
        ForkShape {
            accepted_blocks: 3,
            candidate_blocks: 2,
            shared_complete_blocks: 1,
            candidate_private_blocks: 1,
        }
    );
    let replacement = fork
        .candidate_push_with(&mut store, |slot| slot.insert(value(90)))
        .expect("replace accepted suffix");
    assert_eq!(replacement, accepted_positions[11]);
    for seed in 91..97 {
        fork.candidate_push_with(&mut store, |slot| slot.insert(value(seed)))
            .expect("candidate suffix");
    }
    let (accepted, candidate) = fork.views(&store);
    assert_eq!(
        accepted.get(accepted_positions[11]).expect("accepted")[0],
        11
    );
    assert_eq!(candidate.get(replacement).expect("candidate")[0], 90);
    assert_eq!(
        accepted.get(accepted_positions[17]).expect("old suffix")[0],
        17
    );
    assert!(matches!(
        candidate.get(accepted_positions[17]),
        Err(ArenaError::StaleLogicalBlock)
    ));
    let metrics = fork.metrics();
    assert_eq!(metrics.fork_tail_values_copied, 3);
    assert_eq!(metrics.fork_tail_bytes_copied, 3 * 8_192);
    assert_eq!(metrics.table_entries_copied, 3);
    assert_eq!(metrics.table_live_entries_copied, 3);
    assert_eq!(
        metrics.table_bytes_copied,
        (3 * AcceptedBlockTable::<Wide>::logical_row_bytes()) as u64
    );
    assert_eq!(metrics.descriptor_visits, 0);
}

#[test]
fn aligned_empty_one_item_and_maximal_tail_forks_obey_the_copy_bound() {
    for checkpoint_len in [0, 8, 9, 15] {
        let (mut store, table, checkpoint, _) = logical_fork_fixture(20, checkpoint_len);
        let fork = table
            .fork(&mut store, checkpoint)
            .unwrap_or_else(|(error, _)| panic!("fork: {error}"));
        let expected = checkpoint_len % 8;
        assert_eq!(fork.metrics().fork_tail_values_copied, expected as u64);
        assert!(fork.metrics().fork_tail_bytes_copied <= 65_536);
        assert_eq!(fork.shape().shared_complete_blocks, checkpoint_len / 8);
        let _ = fork
            .reject(&mut store)
            .unwrap_or_else(|(error, _)| panic!("reject: {error}"));
    }
}

#[test]
fn compact_record_and_annex_word_tails_hit_the_exact_documented_maxima() {
    type Record32 = [u32; 8];
    let mut record_store = BlockStore::new();
    let mut record_table = AcceptedBlockTable::<Record32>::new();
    for seed in 0..2_047 {
        record_table
            .push_with(&mut record_store, |slot| slot.insert([seed; 8]))
            .expect("record");
    }
    let record_checkpoint = record_table.cursor();
    let record_fork = record_table
        .fork(&mut record_store, record_checkpoint)
        .unwrap_or_else(|(error, _)| panic!("record fork: {error}"));
    assert_eq!(record_fork.metrics().fork_tail_values_copied, 2_047);
    assert_eq!(record_fork.metrics().fork_tail_bytes_copied, 65_504);
    let _ = record_fork
        .reject(&mut record_store)
        .unwrap_or_else(|(error, _)| panic!("record reject: {error}"));

    let mut annex_store = BlockStore::new();
    let mut annex_table = AcceptedBlockTable::<u32>::new();
    for word in 0..16_383 {
        annex_table
            .push_with(&mut annex_store, |slot| slot.insert(word))
            .expect("annex word");
    }
    let annex_checkpoint = annex_table.cursor();
    let annex_fork = annex_table
        .fork(&mut annex_store, annex_checkpoint)
        .unwrap_or_else(|(error, _)| panic!("annex fork: {error}"));
    assert_eq!(annex_fork.metrics().fork_tail_values_copied, 16_383);
    assert_eq!(annex_fork.metrics().fork_tail_bytes_copied, 65_532);
    let _ = annex_fork
        .reject(&mut annex_store)
        .unwrap_or_else(|(error, _)| panic!("annex reject: {error}"));
}

#[test]
fn fork_metrics_distinguish_live_and_vacant_logical_rows() {
    let mut store = BlockStore::<Wide>::new();
    let mut table = AcceptedBlockTable::new();
    for seed in 0..8 {
        push_logical(&mut store, &mut table, seed);
    }
    let checkpoint = table.cursor();
    push_logical(&mut store, &mut table, 8);
    table
        .truncate(&mut store, checkpoint)
        .expect("leave one vacant row");
    let checkpoint = table.cursor();
    let fork = table
        .fork(&mut store, checkpoint)
        .unwrap_or_else(|(error, _)| panic!("fork: {error}"));
    assert_eq!(fork.metrics().table_entries_copied, 2);
    assert_eq!(fork.metrics().table_live_entries_copied, 1);
    assert_eq!(fork.metrics().table_vacant_entries_copied, 1);
    let _ = fork
        .reject(&mut store)
        .unwrap_or_else(|(error, _)| panic!("reject: {error}"));
}

#[test]
fn acceptance_moves_candidate_tables_and_releases_old_payload_without_copy() {
    let (mut store, table, checkpoint, accepted_positions) = logical_fork_fixture(19, 10);
    let mut fork = table
        .fork(&mut store, checkpoint)
        .unwrap_or_else(|(error, _)| panic!("fork: {error}"));
    let replacement = fork
        .candidate_push_with(&mut store, |slot| slot.insert(value(90)))
        .expect("candidate append");
    let copied = fork.metrics().fork_tail_values_copied;
    let table = fork
        .accept(&mut store)
        .unwrap_or_else(|(error, _)| panic!("accept: {error}"));
    assert_eq!(table.metrics().fork_tail_values_copied, copied);
    assert_eq!(table.metrics().accepted_payload_copies, 0);
    assert_eq!(
        table.view(&store).get(replacement).expect("candidate")[0],
        90
    );
    assert!(matches!(
        table.view(&store).get(accepted_positions[17]),
        Err(ArenaError::StaleLogicalBlock)
    ));
}

#[test]
fn rejection_restores_the_exact_accepted_mapping_and_values() {
    let (mut store, table, checkpoint, accepted_positions) = logical_fork_fixture(18, 9);
    let mut fork = table
        .fork(&mut store, checkpoint)
        .unwrap_or_else(|(error, _)| panic!("fork: {error}"));
    fork.candidate_push_with(&mut store, |slot| slot.insert(value(77)))
        .expect("candidate append");
    let restored = fork
        .reject(&mut store)
        .unwrap_or_else(|(error, _)| panic!("reject: {error}"));
    assert_eq!(
        restored
            .view(&store)
            .get(accepted_positions[17])
            .expect("accepted suffix")[0],
        17
    );
    assert_eq!(restored.metrics().rejected_payload_copies, 0);
}

fn transferable_suffix() -> (
    BlockStore<Wide>,
    AcceptedBlockTable<Wide>,
    BlockRangeOwner<Wide>,
    Vec<LogicalPosition>,
) {
    let mut store = BlockStore::new();
    let mut table = AcceptedBlockTable::new();
    for seed in 0..3 {
        push_logical(&mut store, &mut table, seed);
    }
    let boundary = table.rotate_tail().expect("rotate");
    let mut positions = Vec::new();
    for seed in 10..19 {
        positions.push(push_logical(&mut store, &mut table, seed));
    }
    let owner = table
        .seal_rotated_suffix(boundary)
        .unwrap_or_else(|(error, _)| panic!("seal: {error}"));
    (store, table, owner, positions)
}

#[test]
fn failed_prepare_returns_the_exact_loan_and_rollback_restores_the_frontier() {
    let (store, table, owner, positions) = transferable_suffix();
    assert_eq!(owner.len(), 2);
    let original_blocks = owner.logical_blocks().to_vec();
    let (source, loan, receipt) = owner
        .detach_suffix(1)
        .unwrap_or_else(|(error, _)| panic!("detach: {error}"));
    let loan_blocks = loan.logical_blocks().to_vec();
    let foreign_table = AcceptedBlockTable::<Wide>::new();
    let foreign_destination = foreign_table.empty_block_owner();
    let failure = match prepare_block_range_transfer(foreign_destination, loan) {
        Ok(_) => panic!("foreign preparation must fail"),
        Err(failure) => failure,
    };
    assert!(matches!(failure.error(), ArenaError::ForeignLogicalSpace));
    let (_, _, loan) = failure.into_parts();
    assert_eq!(loan.logical_blocks(), loan_blocks);
    let restored = receipt
        .rollback(source, loan)
        .unwrap_or_else(|failure| panic!("rollback: {}", failure.error()));
    assert_eq!(restored.logical_blocks(), original_blocks);
    assert_eq!(restored.frontier(), 2);
    assert_eq!(restored.metrics().block_ranges_rolled_back, 1);
    let view = table.view(&store);
    assert_eq!(view.get(positions[8]).expect("mapping unchanged")[0], 18);
}

#[test]
fn prepared_commit_is_infallible_and_moves_only_ownership_metadata() {
    let (store, table, owner, positions) = transferable_suffix();
    let destination = table.empty_block_owner();
    let (_source, loan, _receipt) = owner
        .detach_suffix(0)
        .unwrap_or_else(|(error, _)| panic!("detach: {error}"));
    let prepared = prepare_block_range_transfer(destination, loan)
        .unwrap_or_else(|failure| panic!("prepare: {}", failure.error()));
    let destination = prepared.commit();
    assert_eq!(destination.len(), 2);
    assert_eq!(destination.metrics().block_ranges_prepared, 1);
    assert_eq!(destination.metrics().block_ranges_transferred, 1);
    assert_eq!(
        table.view(&store).get(positions[0]).expect("mapping")[0],
        10
    );
    assert_eq!(table.metrics().boundary_rotations, 1);
    assert_eq!(table.metrics().boundary_slack_values, 5);
}

#[test]
fn an_empty_rotated_build_cancels_without_allocating_a_block() {
    let mut store = BlockStore::<Wide>::new();
    let mut table = AcceptedBlockTable::new();
    for seed in 0..3 {
        push_logical(&mut store, &mut table, seed);
    }
    let before = store.metrics();
    let boundary = table.rotate_tail().expect("rotate");
    table
        .cancel_rotation(boundary)
        .unwrap_or_else(|(error, _)| panic!("cancel: {error}"));
    assert_eq!(
        store.metrics().superblocks_allocated,
        before.superblocks_allocated
    );
    let position = push_logical(&mut store, &mut table, 3);
    assert_eq!(position.block().ordinal(), 0);
}

#[test]
fn rollback_refuses_to_guess_after_the_source_frontier_changes() {
    let (_store, _table, owner, _) = transferable_suffix();
    let (source, loan, receipt) = owner
        .detach_suffix(1)
        .unwrap_or_else(|(error, _)| panic!("detach: {error}"));
    let source_len = source.len();
    let (changed_source, _empty_loan, _empty_receipt) = source
        .detach_suffix(source_len)
        .unwrap_or_else(|(error, _)| panic!("second detach: {error}"));
    let failure = match receipt.rollback(changed_source, loan) {
        Ok(_) => panic!("changed frontier must fail"),
        Err(failure) => failure,
    };
    assert!(matches!(failure.error(), ArenaError::SourceFrontierChanged));
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
fn group_scratch_journal_and_output_remain_nonforking() {
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
    drop(committed);
    drop(journal);
    drop(scratch);
    drop(groups);
    assert_eq!(*drops.lock().expect("drops"), [1, 2, 3, 4, 0]);
}

#[test]
fn logical_table_recovers_after_initializer_panic_without_a_hole() {
    let mut store = BlockStore::<String>::new();
    let mut table = AcceptedBlockTable::new();
    let result = catch_unwind(AssertUnwindSafe(|| {
        let _ = table.push_with(&mut store, |slot| {
            let _guard = slot.insert(String::from("unpublished"));
            panic!("stop")
        });
    }));
    assert!(result.is_err());
    assert_eq!(table.len(), 0);
    let position = table
        .push_with(&mut store, |slot| slot.insert(String::from("first")))
        .expect("recover");
    assert_eq!(
        table.view(&store).get(position).expect("published string"),
        "first"
    );
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
fn truncate_publishes_shorter_logical_prefix_before_drop_panic() {
    let drops = Arc::new(Mutex::new(Vec::new()));
    let mut store = BlockStore::<PanicDrop>::new();
    let mut table = AcceptedBlockTable::new();
    table
        .push_with(&mut store, |slot| {
            slot.insert(PanicDrop {
                id: 0,
                panic: false,
                drops: Arc::clone(&drops),
            })
        })
        .expect("retained");
    let keep = table.cursor();
    let mut removed_positions = Vec::new();
    for (id, panic) in [(1, true), (2, false)] {
        removed_positions.push(
            table
                .push_with(&mut store, |slot| {
                    slot.insert(PanicDrop {
                        id,
                        panic,
                        drops: Arc::clone(&drops),
                    })
                })
                .expect("removed"),
        );
    }
    let result = catch_unwind(AssertUnwindSafe(|| table.truncate(&mut store, keep)));
    assert!(result.is_err());
    assert_eq!(table.len(), 1);
    assert!(matches!(
        table.view(&store).get(removed_positions[0]),
        Err(ArenaError::UninitializedLogicalOffset)
    ));
    assert_eq!(*drops.lock().expect("drops"), [2, 1]);
    drop(table);
    drop(store);
    assert_eq!(*drops.lock().expect("drops"), [2, 1, 0]);
}

#[test]
fn physical_id_is_eight_private_bytes_but_logical_id_is_the_public_coordinate() {
    assert_eq!(core::mem::size_of::<BlockId>(), 8);
    assert_eq!(core::mem::size_of::<LogicalBlockId>(), 12);
    assert_eq!(core::mem::size_of::<LogicalPosition>(), 16);
}
