use super::PackedJournal;

#[test]
fn rollback_and_two_lineage_settlement_swap_exact_values() {
    let mut journal = PackedJournal::<u32, 4>::default();
    journal.warm_first_page();
    let root = journal.mark();
    let mut value = 1;
    journal.append(value);
    value = 2;
    let later = journal.mark();
    journal.append(value);
    value = 3;

    journal.begin_checkpoint_candidate(root, |old| std::mem::swap(old, &mut value));
    assert_eq!(value, 1);
    journal.append(value);
    value = 4;
    journal.reject_checkpoint_candidate(|old| std::mem::swap(old, &mut value));
    assert_eq!(value, 3);
    assert!(journal.validates(later));

    journal.begin_checkpoint_candidate(root, |old| std::mem::swap(old, &mut value));
    journal.append(value);
    value = 5;
    journal.accept_checkpoint_candidate();
    assert_eq!(value, 5);
    assert!(!journal.validates(later));
}

#[test]
fn forward_redo_preserves_noncoalesced_record_order() {
    let mut journal = PackedJournal::<u32, 2>::default();
    let mark = journal.mark();
    journal.append(10);
    journal.append(20);
    journal.append(30);
    let mut rollback = Vec::new();
    journal.begin_checkpoint_candidate(mark, |value| rollback.push(*value));
    assert_eq!(rollback, vec![30, 20, 10]);

    let mut redo = Vec::new();
    journal.reject_checkpoint_candidate(|value| redo.push(*value));
    assert_eq!(redo, vec![10, 20, 30]);
}

#[test]
fn released_prefix_becomes_a_keyless_base_and_reuses_its_chunks() {
    let mut journal = PackedJournal::<u32, 2>::default();
    journal.warm_first_page();
    let root = journal.mark();
    journal.append(10);
    journal.append(20);
    let floor = journal.mark();
    journal.append(30);
    let accepted = journal.mark();

    assert_eq!(journal.release_prefix(floor, drop), Some(1));
    assert!(!journal.validates(root));
    assert!(journal.validates(floor));
    assert!(journal.validates(accepted));

    let reused_before = journal.counters().chunks_reused;
    journal.begin_checkpoint_candidate(floor, |_| {});
    journal.append(40);
    journal.reject_checkpoint_candidate(|_| {});
    assert!(journal.validates(accepted));
    assert!(journal.counters().chunks_reused > reused_before);

    let mut undone = Vec::new();
    assert!(journal.restore(floor, |value| undone.push(*value)));
    assert_eq!(undone, [30]);
    assert!(journal.validates(floor));
}
