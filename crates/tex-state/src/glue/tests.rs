use super::{GlueSpec, GlueStore, Order};
use crate::frozen_lookup::FrozenLookup;
use crate::ids::GlueId;
use crate::patch_domain::PatchAllocationDomain;
use crate::scaled::Scaled;

#[test]
fn zero_is_an_explicit_immortal_root() {
    let store = GlueStore::new();
    let zero = store.owner(GlueId::ZERO).expect("zero owner");
    assert_eq!(zero.spec(), GlueSpec::ZERO);
    assert_eq!(zero.id(), GlueId::ZERO);
    assert_eq!(zero.strong_count(), 2, "store and caller own zero");
}

#[test]
fn exact_content_deduplicates_and_collision_candidates_do_not_alias() {
    let mut store = GlueStore::new();
    let left = spec(10);
    let right = spec(11);
    let stretch_order = GlueSpec {
        stretch_order: Order::Filll,
        ..left
    };
    let shrink_order = GlueSpec {
        shrink_order: Order::Normal,
        ..left
    };
    let first = store.testing_intern_with_key(left, 0);
    let equal = store.testing_intern_with_key(left, 0);
    let collision = store.testing_intern_with_key(right, 0);
    let stretch_collision = store.testing_intern_with_key(stretch_order, 0);
    let shrink_collision = store.testing_intern_with_key(shrink_order, 0);

    assert!(first.ptr_eq(&equal));
    assert_ne!(first.id(), collision.id());
    assert_ne!(first.id(), stretch_collision.id());
    assert_ne!(first.id(), shrink_collision.id());
    assert_eq!(collision.spec(), right);
    assert_eq!(stretch_collision.spec(), stretch_order);
    assert_eq!(shrink_collision.spec(), shrink_order);
}

#[test]
fn next_allocation_retires_unrooted_region_slot_generation_safely() {
    let mut store = GlueStore::new();
    let stale = store.intern_owned(spec(1), None);
    let stale_id = stale.id();
    drop(stale);
    assert!(store.contains(stale_id));

    let replacement = store.intern_owned(spec(2), None);
    assert_eq!(replacement.id().raw(), stale_id.raw());
    assert_ne!(replacement.id(), stale_id);
    assert!(!store.contains(stale_id));
}

#[test]
fn loaded_base_is_explicit_and_runtime_region_retires_at_mutation() {
    let mut store = GlueStore::from_frozen(vec![GlueSpec::ZERO, spec(40)], FrozenLookup::empty())
        .expect("valid frozen glue");
    let frozen = store.stored_slot(1);
    assert_eq!(frozen.spec(), spec(40));

    let dynamic = store.intern_owned(spec(41), None);
    let dynamic_id = dynamic.id();
    drop(dynamic);
    assert!(store.contains(dynamic_id));
    let _replacement = store.intern_owned(spec(42), None);
    assert!(!store.contains(dynamic_id));
    assert_eq!(store.stored_slot(1).spec(), spec(40));
}

#[test]
fn private_rollback_and_selected_acceptance_follow_typed_leases() {
    let mut store = GlueStore::new();
    let mut domain = PatchAllocationDomain::new();
    let first_mark = domain.begin_operation().expect("operation");
    let retained = store.intern_owned(spec(50), Some(&mut domain));
    domain.commit_operation(first_mark).expect("commit");

    let failed_mark = domain.begin_operation().expect("retry operation");
    let store_mark = store.watermark();
    let failed = store.intern_owned(spec(51), Some(&mut domain));
    drop(failed);
    domain.rollback_operation(failed_mark).expect("rollback");
    store.truncate_to(store_mark);

    let roots = store.selected_patch_roots(&domain);
    assert_eq!(roots.len(), 1);
    let accepted = domain.accept(roots).expect("selected acceptance");
    assert_eq!(accepted.len(), 1);
    store.clear_patch_allocations();
    drop(accepted);
    assert_eq!(retained.spec(), spec(50));
}

#[test]
fn rejected_private_domain_cannot_keep_an_unrooted_slot_live() {
    let mut store = GlueStore::new();
    let mut domain = PatchAllocationDomain::new();
    let operation = domain.begin_operation().expect("operation");
    let rejected = store.intern_owned(spec(52), Some(&mut domain));
    let rejected_id = rejected.id();
    drop(rejected);
    domain.commit_operation(operation).expect("commit");
    drop(domain);
    store.clear_patch_allocations();

    assert!(!store.contains(rejected_id));
}

#[test]
fn ten_thousand_bounded_live_redefinitions_plateau() {
    let mut store = GlueStore::new();
    let mut current = store.intern_owned(spec(0), None);
    for raw in 1..=10_000 {
        current = store.intern_owned(spec(raw), None);
    }
    assert_eq!(current.spec(), spec(10_000));
    let shape = store.testing_pool_shape();
    let totals = store.testing_live_totals();
    assert!(
        shape.0 <= 3,
        "region slots should track bounded live roots: {shape:?}"
    );
    assert_eq!(totals.0, 3, "zero plus current and one retired-at-next-mutation value remain");
    assert!(shape.2 <= 1_024);
    assert!(shape.4 <= 64);
}

#[test]
fn all_roots_live_grows_exactly() {
    const VALUES: usize = 2_048;
    let mut store = GlueStore::new();
    let roots = (1..=VALUES)
        .map(|raw| store.intern_owned(spec(raw as i32), None))
        .collect::<Vec<_>>();
    let (objects, bytes) = store.testing_live_totals();
    assert_eq!(objects, VALUES + 1);
    assert_eq!(bytes, (VALUES + 1) * core::mem::size_of::<GlueSpec>());
    assert_eq!(roots.len(), VALUES);
}

fn spec(width: i32) -> GlueSpec {
    GlueSpec {
        width: Scaled::from_raw(width),
        stretch: Scaled::from_raw(width.wrapping_mul(3)),
        stretch_order: Order::Fil,
        shrink: Scaled::from_raw(width.wrapping_mul(5)),
        shrink_order: Order::Fill,
    }
}
