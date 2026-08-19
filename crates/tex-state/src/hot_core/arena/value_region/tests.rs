use core::mem::{needs_drop, size_of};
use core::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

type TestRoots = AcceptedRuntimeValueRegions<u8, u16, u32, u64, i32, i64>;

fn capacity(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("test capacity is nonzero")
}

#[test]
fn mark_is_fixed_size_copy_only_column_watermark() {
    assert_eq!(size_of::<RuntimeValueRegionMark>(), 56);
    assert!(!needs_drop::<RuntimeValueRegionMark>());
}

#[test]
fn accept_moves_backing_storage_and_reject_makes_suffix_stale() {
    let roots = TestRoots::new(capacity(8));
    let mut candidate = roots.candidate().expect("candidate namespace exists");
    let token = candidate.append_token_word(11).expect("token word appends");
    let before = candidate
        .resolve_token_word(token)
        .expect("candidate token resolves") as *const u8;
    let accepted = candidate.accept().expect("candidate seals");
    let after = accepted
        .resolve_token_word(token)
        .expect("accepted token resolves") as *const u8;
    assert_eq!(
        before, after,
        "sealing must move, not copy, the backing vector"
    );

    let mut retry = accepted.candidate().expect("retry candidate exists");
    let mark = retry.mark().expect("retry mark exists");
    let rejected = retry.append_glue(-3).expect("retry glue appends");
    retry.truncate(mark).expect("retry suffix rejects");
    assert!(retry.resolve_glue(rejected).is_err());
}

#[test]
fn all_six_columns_share_one_region_owner_and_exact_growth() {
    let roots = TestRoots::new(capacity(8));
    let mut candidate = roots.candidate().expect("candidate namespace exists");
    let token_word = candidate.append_token_word(1).expect("word appends");
    let token_list = candidate.append_token_list(2).expect("list appends");
    let macro_record = candidate.append_macro_record(3).expect("macro appends");
    let macro_root = candidate.append_macro_root(4).expect("root appends");
    let glue = candidate.append_glue(5).expect("glue appends");
    let provenance = candidate.append_provenance(6).expect("root appends");

    let owner = token_word.owner();
    assert_eq!(token_list.owner(), owner);
    assert_eq!(macro_record.owner(), owner);
    assert_eq!(macro_root.owner(), owner);
    assert_eq!(glue.owner(), owner);
    assert_eq!(provenance.owner(), owner);

    let accounting = candidate.accounting();
    assert_eq!(accounting.logical_values, 6);
    assert_eq!(
        accounting.logical_bytes,
        size_of::<u8>()
            + size_of::<u16>()
            + size_of::<u32>()
            + size_of::<u64>()
            + size_of::<i32>()
            + size_of::<i64>()
    );
    assert_eq!(accounting.live_regions, 1);
    assert_eq!(accounting.region_owners, 1);
    assert_eq!(accounting.registry_slots, 1);
    assert_eq!(accounting.retained_payload_values, 6 * 8);
}

#[test]
fn old_mark_recovers_sealed_prefix_and_discards_whole_suffix_regions() {
    let roots = TestRoots::new(capacity(2));
    let mut candidate = roots.candidate().expect("candidate namespace exists");
    let old = candidate.append_token_word(1).expect("old word appends");
    let mark = candidate.mark().expect("old mark exists");
    let discarded_same_region = candidate.append_token_word(2).expect("word appends");
    let discarded_next_region = candidate.append_glue(3).expect("glue appends");
    assert_eq!(candidate.accounting().live_regions, 2);

    candidate.truncate(mark).expect("old mark restores");
    assert_eq!(candidate.resolve_token_word(old), Ok(&1));
    assert!(candidate.resolve_token_word(discarded_same_region).is_err());
    assert!(candidate.resolve_glue(discarded_next_region).is_err());

    let replacement = candidate.append_token_word(4).expect("replacement appends");
    assert_ne!(replacement.owner(), discarded_same_region.owner());
    assert_eq!(candidate.resolve_token_word(old), Ok(&1));
    assert_eq!(candidate.resolve_token_word(replacement), Ok(&4));
}

#[test]
fn nested_group_and_global_transfer_keep_only_explicit_region_roots() {
    let drops = Arc::new(AtomicUsize::new(0));
    let empty = AcceptedRuntimeValueRegions::<DropProbe, (), (), (), (), ()>::new(capacity(1));
    let mut outer = empty.candidate().expect("outer candidate exists");
    let saved = outer
        .append_token_word(DropProbe::new(Arc::clone(&drops)))
        .expect("saved value appends");
    let first = outer.accept().expect("outer region seals");

    let mut inner = first.candidate().expect("inner candidate exists");
    let global = inner
        .append_token_word(DropProbe::new(Arc::clone(&drops)))
        .expect("global value appends");
    let second = inner.accept().expect("global region transfers");
    let global_only = second.retain_regions(&[global.owner()]);

    drop(first);
    drop(second);
    assert_eq!(
        drops.load(Ordering::Relaxed),
        1,
        "old saved region releases"
    );
    assert!(global_only.resolve_token_word(saved).is_err());
    assert!(global_only.resolve_token_word(global).is_ok());
    drop(global_only);
    assert_eq!(
        drops.load(Ordering::Relaxed),
        2,
        "global region releases last"
    );
}

#[test]
fn resource_retry_reuses_whole_region_capacity_under_new_generations() {
    let roots = TestRoots::new(capacity(8));
    let mut candidate = roots.candidate().expect("candidate namespace exists");
    let empty = candidate.mark().expect("empty retry mark exists");
    let warm = candidate.append_token_word(0).expect("warm word appends");
    let _ = candidate.append_token_list(0).expect("warm list appends");
    candidate.truncate(empty).expect("warm retry rejects");
    let plateau = candidate.accounting();
    let growth = candidate.testing_storage_growth_events();

    for cycle in 0..10_000_u16 {
        let attempt = candidate.mark().expect("retry mark exists");
        let word = candidate
            .append_token_word((cycle & 0xff) as u8)
            .expect("retry word appends");
        let list = candidate
            .append_token_list(cycle)
            .expect("retry list appends");
        assert_eq!(word.owner(), list.owner());
        candidate.truncate(attempt).expect("retry rejects");
        assert!(candidate.resolve_token_word(word).is_err());
    }

    assert!(candidate.resolve_token_word(warm).is_err());
    assert_eq!(candidate.accounting(), plateau);
    assert_eq!(candidate.testing_storage_growth_events(), growth);
    assert_eq!(plateau.logical_values, 0);
    assert_eq!(plateau.reusable_regions, 1);
    assert_eq!(plateau.registry_slots, 1);
}

struct DropProbe {
    drops: Arc<AtomicUsize>,
}

impl DropProbe {
    fn new(drops: Arc<AtomicUsize>) -> Self {
        Self { drops }
    }
}

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
    }
}
