use core::mem::{needs_drop, size_of};

use super::*;

const ARENAS: [HotArenaKind; 6] = [
    HotArenaKind::TokenWord,
    HotArenaKind::TokenList,
    HotArenaKind::MacroRecord,
    HotArenaKind::MacroRoot,
    HotArenaKind::Glue,
    HotArenaKind::Provenance,
];
const STACKS: [HotStackKind; 6] = [
    HotStackKind::Input,
    HotStackKind::Parameter,
    HotStackKind::Condition,
    HotStackKind::Group,
    HotStackKind::Save,
    HotStackKind::Mode,
];

fn capacity(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("test capacity is nonzero")
}

fn base() -> AcceptedHotCore {
    AcceptedHotCore::new(capacity(8)).expect("hot-core base identity exists")
}

fn candidate(base: &AcceptedHotCore) -> HotCore {
    base.candidate(40, 0).expect("hot-core candidate exists")
}

fn mutate_suffix(core: &mut HotCore, seed: u64) {
    for (offset, kind) in ARENAS.into_iter().enumerate() {
        let _ = core
            .append_arena_word(kind, seed + offset as u64)
            .expect("arena suffix appends");
    }
    for kind in STACKS {
        for offset in 0..16_u64 {
            core.push_stack(kind, seed + offset)
                .expect("warmed stack suffix appends");
        }
    }
    for index in 0..40 {
        core.write_state(index, seed + index as u64)
            .expect("journaled dense write succeeds");
    }
    core.advance_external_journals(1)
        .expect("external cursors advance");
}

#[test]
fn hot_snapshot_is_a_plain_fixed_size_runtime_mark() {
    assert_eq!(size_of::<HotSnapshot>(), 152);
    assert!(!needs_drop::<HotSnapshot>());

    let mut core = candidate(&base());
    let snapshot = core.snapshot().expect("snapshot opens");
    assert_eq!(snapshot.retained_bytes(), 0);
    core.rollback(snapshot).expect("snapshot closes");
}

#[test]
fn snapshot_size_and_retention_do_not_depend_on_live_state_size() {
    let accepted = base();
    let mut small = candidate(&accepted);
    let mut large = candidate(&accepted);
    for value in 0..4_096_u64 {
        for kind in ARENAS {
            let _ = large
                .append_arena_word(kind, value)
                .expect("large live arena appends");
        }
    }
    for kind in STACKS {
        for value in 0..4_096_u64 {
            large
                .push_stack(kind, value)
                .expect("large live stack appends");
        }
    }
    assert!(large.accounting().retained_bytes > small.accounting().retained_bytes);

    let small_mark = small.snapshot().expect("small snapshot opens");
    let large_mark = large.snapshot().expect("large snapshot opens");
    assert_eq!(size_of_val(&small_mark), size_of_val(&large_mark));
    assert_eq!(small_mark.retained_bytes(), large_mark.retained_bytes());
    small.commit(small_mark).expect("small snapshot commits");
    large.commit(large_mark).expect("large snapshot commits");
}

#[test]
fn rollback_restores_every_composed_component_exactly() {
    let mut core = candidate(&base());
    let before = core.accounting();
    let snapshot = core.snapshot().expect("snapshot opens");
    let token = core
        .append_arena_word(HotArenaKind::TokenWord, 91)
        .expect("token word appends");
    mutate_suffix(&mut core, 100);
    assert_eq!(core.resolve(HotArenaKind::TokenWord, token), Ok(&91));
    assert_eq!(core.state_value(7), Ok(107));
    assert_eq!(core.stack_len(HotStackKind::Mode), 16);

    core.rollback(snapshot)
        .expect("aggregate suffix rolls back");
    assert!(core.resolve(HotArenaKind::TokenWord, token).is_err());
    assert_eq!(core.state_value(7), Ok(0));
    assert_eq!(core.stack_len(HotStackKind::Mode), 0);
    assert_eq!(
        core.accounting().arena_logical_values,
        before.arena_logical_values
    );
    assert_eq!(
        core.accounting().stack_logical_entries,
        before.stack_logical_entries
    );
    assert_eq!(core.accounting().journal_logical_inverses, 0);
    assert_eq!(core.accounting().active_snapshots, 0);
}

#[test]
fn accepted_bases_remain_readable_across_accept_reject_and_retry() {
    let empty = base();
    let mut author = candidate(&empty);
    let inherited = author
        .append_arena_word(HotArenaKind::TokenWord, 11)
        .expect("accepted token appends");
    let accepted = author.accept().expect("author overlay accepts");
    assert_eq!(
        accepted.resolve(HotArenaKind::TokenWord, inherited),
        Ok(&11)
    );

    let mut rejected = candidate(&accepted);
    let rejected_word = rejected
        .append_arena_word(HotArenaKind::TokenWord, 12)
        .expect("rejected token appends");
    assert_eq!(
        rejected.resolve(HotArenaKind::TokenWord, inherited),
        Ok(&11)
    );
    drop(rejected);

    let mut retry = candidate(&accepted);
    assert_eq!(retry.resolve(HotArenaKind::TokenWord, inherited), Ok(&11));
    assert!(
        retry
            .resolve(HotArenaKind::TokenWord, rejected_word)
            .is_err()
    );
    let attempt = retry.snapshot().expect("retry attempt opens");
    let discarded = retry
        .append_arena_word(HotArenaKind::TokenWord, 13)
        .expect("retry suffix appends");
    retry.rollback(attempt).expect("retry attempt rejects");
    assert!(retry.resolve(HotArenaKind::TokenWord, discarded).is_err());
    let replacement = retry
        .append_arena_word(HotArenaKind::TokenWord, 14)
        .expect("retry replacement appends");
    let next = retry.accept().expect("retry candidate accepts");

    assert_eq!(next.resolve(HotArenaKind::TokenWord, inherited), Ok(&11));
    assert_eq!(next.resolve(HotArenaKind::TokenWord, replacement), Ok(&14));
    assert!(
        next.resolve(HotArenaKind::TokenWord, rejected_word)
            .is_err()
    );
}

#[test]
fn nested_commit_transfers_dense_inverses_to_the_parent_snapshot() {
    let mut core = candidate(&base());
    let outer = core.snapshot().expect("outer snapshot opens");
    core.write_state(0, 1).expect("outer state write succeeds");
    let inner = core.snapshot().expect("inner snapshot opens");
    core.write_state(0, 2).expect("inner state write succeeds");
    core.push_stack(HotStackKind::Input, 7)
        .expect("inner stack write succeeds");
    core.commit(inner).expect("inner snapshot commits");

    assert_eq!(core.state_value(0), Ok(2));
    assert_eq!(core.stack_len(HotStackKind::Input), 1);
    core.rollback(outer).expect("outer snapshot rolls back");
    assert_eq!(core.state_value(0), Ok(0));
    assert_eq!(core.stack_len(HotStackKind::Input), 0);
}

#[test]
fn stale_and_foreign_snapshots_reject_before_any_component_mutates() {
    let accepted = base();
    let mut left = candidate(&accepted);
    let mut right = candidate(&accepted);
    let left_mark = left.snapshot().expect("left snapshot opens");
    mutate_suffix(&mut left, 10);
    let right_mark = right.snapshot().expect("right snapshot opens");
    mutate_suffix(&mut right, 20);
    let right_before = right.accounting();
    let right_value = right.state_value(3);

    assert_eq!(right.rollback(left_mark), Err(HotCoreError::ForeignCore));
    assert_eq!(right.accounting(), right_before);
    assert_eq!(right.state_value(3), right_value);
    right.commit(right_mark).expect("right snapshot commits");
    let committed = right.accounting();
    let committed_value = right.state_value(3);
    assert!(matches!(
        right.rollback(right_mark),
        Err(HotCoreError::MutationJournal(
            FirstWriteJournalError::InvalidMark
        ))
    ));
    assert_eq!(right.accounting(), committed);
    assert_eq!(right.state_value(3), committed_value);

    left.rollback(left_mark)
        .expect("left snapshot remains valid");
}

#[test]
fn snapshots_cannot_cross_an_accepted_generation() {
    let accepted = base();
    let mut first = candidate(&accepted);
    let old = first.snapshot().expect("first snapshot opens");
    first.commit(old).expect("first snapshot commits");
    let next_base = first.accept().expect("first candidate accepts");
    let mut next = candidate(&next_base);
    let before = next.accounting();

    assert_eq!(next.rollback(old), Err(HotCoreError::ForeignCore));
    assert_eq!(next.accounting(), before);
}

#[test]
fn all_live_aggregate_growth_has_exact_logical_accounting() {
    let mut core = candidate(&base());
    let snapshot = core.snapshot().expect("all-live snapshot opens");
    for kind in ARENAS {
        for value in 0..9_u64 {
            let _ = core
                .append_arena_word(kind, value)
                .expect("all-live arena word appends");
        }
    }
    for kind in STACKS {
        for value in 0..13_u64 {
            core.push_stack(kind, value)
                .expect("all-live stack word appends");
        }
    }
    for index in 0..40 {
        core.write_state(index, index as u64 + 1)
            .expect("all-live dense value writes");
    }
    let live = core.accounting();
    assert_eq!(live.arena_logical_values, 6 * 9);
    assert_eq!(live.arena_logical_bytes, 6 * 9 * size_of::<u64>());
    assert_eq!(live.stack_logical_entries, 6 * 13);
    assert_eq!(live.stack_logical_bytes, 6 * 13 * size_of::<u64>());
    assert_eq!(live.dense_logical_cells, 40);
    assert_eq!(live.dense_logical_value_bytes, 40 * size_of::<u64>());
    assert_eq!(live.journal_logical_inverses, 40);
    assert_eq!(live.active_snapshots, 1);

    core.commit(snapshot).expect("all-live snapshot commits");
    let committed = core.accounting();
    assert_eq!(committed.arena_logical_values, live.arena_logical_values);
    assert_eq!(committed.stack_logical_entries, live.stack_logical_entries);
    assert_eq!(committed.journal_logical_inverses, 0);
    assert_eq!(committed.active_snapshots, 0);
    assert!(committed.retained_bytes > 0);
}

#[test]
fn ten_thousand_aggregate_accept_reject_retry_cycles_plateau_exactly() {
    let mut core = candidate(&base());
    let warm = core.snapshot().expect("warm snapshot opens");
    mutate_suffix(&mut core, 0);
    core.rollback(warm).expect("warm suffix rolls back");
    let plateau = core.accounting();

    for cycle in 0..10_000_u64 {
        let accepted = core.snapshot().expect("empty accepted attempt opens");
        core.commit(accepted).expect("empty attempt accepts");

        let rejected = core.snapshot().expect("rejected attempt opens");
        mutate_suffix(&mut core, cycle);
        core.rollback(rejected).expect("attempt rejects");

        let retry = core.snapshot().expect("retry attempt opens");
        mutate_suffix(&mut core, cycle + 1);
        core.rollback(retry).expect("retry rolls back");
    }

    assert_eq!(core.accounting(), plateau);
    assert_eq!(plateau.arena_logical_values, 0);
    assert_eq!(plateau.stack_logical_entries, 0);
    assert_eq!(plateau.journal_logical_inverses, 0);
    assert_eq!(plateau.active_snapshots, 0);
}
