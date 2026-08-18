use core::mem::{needs_drop, size_of};

use super::*;
use crate::hot_core::journal::{FirstWriteJournal, FirstWriteJournalError};

#[test]
fn dense_coordinates_are_typed_fixed_width_runtime_values() {
    assert_eq!(size_of::<DenseBankOwner>(), 16);
    assert_eq!(size_of::<DenseBankCoordinate<u8>>(), 16);
    assert!(!needs_drop::<DenseBankCoordinate<u8>>());
}

#[test]
fn exact_coordinates_read_direct_dense_values() {
    let bank = DenseBank::filled(4, 17_u32).expect("small bank fits");
    let coordinate = bank.coordinate(2).expect("index is live");

    assert_eq!(coordinate.index(), 2);
    assert_eq!(bank.get(coordinate), Ok(17));
    assert_eq!(bank.len(), 4);
    assert_eq!(bank.coordinate(4), Err(DenseBankError::IndexOutOfBounds));
}

#[test]
fn foreign_and_stale_coordinates_reject() {
    let mut first = DenseBank::filled(2, 1_u16).expect("first bank fits");
    let second = DenseBank::filled(2, 2_u16).expect("second bank fits");
    let old = first.coordinate(0).expect("first coordinate exists");
    let foreign = second.coordinate(0).expect("second coordinate exists");

    assert_eq!(first.get(foreign), Err(DenseBankError::ForeignNamespace));
    first.reset_generation(3).expect("generation advances");
    assert_eq!(first.get(old), Err(DenseBankError::StaleGeneration));
    let fresh = first.coordinate(0).expect("fresh coordinate exists");
    assert_ne!(old.testing_parts().1, fresh.testing_parts().1);
    assert_eq!(first.get(fresh), Ok(3));
}

#[test]
fn inline_and_spilled_bank_accounting_separate_logical_values_from_storage() {
    let inline = DenseBank::filled(12, 0_u64).expect("inline bank fits");
    let spilled = DenseBank::filled(40, 0_u64).expect("spilled bank fits");

    assert_eq!(inline.accounting().logical_cells, 12);
    assert_eq!(
        inline.accounting().logical_value_bytes,
        12 * size_of::<u64>()
    );
    assert_eq!(inline.accounting().retained_heap_cells, 0);
    assert_eq!(spilled.accounting().logical_cells, 40);
    assert!(spilled.accounting().retained_heap_cells >= 40);
    assert_eq!(spilled.accounting().inline_capacity, 32);
}

#[test]
fn nested_first_write_rollback_restores_exact_values() {
    let mut bank = DenseBank::filled(3, 10_i32).expect("bank fits");
    let first = bank.coordinate(0).expect("first cell exists");
    let second = bank.coordinate(1).expect("second cell exists");
    let mut journal = FirstWriteJournal::new(&bank);
    let outer = journal.mark(&bank).expect("outer mark opens");

    journal.write(&mut bank, first, 11).expect("outer write");
    journal
        .write(&mut bank, first, 12)
        .expect("coalesced write");
    let inner = journal.mark(&bank).expect("inner mark opens");
    journal.write(&mut bank, first, 13).expect("inner write");
    journal.write(&mut bank, second, 20).expect("inner write");
    journal.rollback(&mut bank, inner).expect("inner rollback");

    assert_eq!(bank.get(first), Ok(12));
    assert_eq!(bank.get(second), Ok(10));
    journal.rollback(&mut bank, outer).expect("outer rollback");
    assert_eq!(bank.get(first), Ok(10));
    assert_eq!(bank.get(second), Ok(10));
}

#[test]
fn nested_commit_merges_first_write_ownership_into_parent() {
    let mut bank = DenseBank::filled(2, 0_u32).expect("bank fits");
    let first = bank.coordinate(0).expect("first cell exists");
    let second = bank.coordinate(1).expect("second cell exists");
    let mut journal = FirstWriteJournal::new(&bank);
    let outer = journal.mark(&bank).expect("outer mark opens");
    journal.write(&mut bank, first, 1).expect("outer write");
    let inner = journal.mark(&bank).expect("inner mark opens");
    journal
        .write(&mut bank, first, 2)
        .expect("nested overwrite");
    journal
        .write(&mut bank, second, 3)
        .expect("nested first write");
    journal.commit(&mut bank, inner).expect("inner commit");
    journal
        .write(&mut bank, second, 4)
        .expect("parent overwrite");

    assert_eq!(journal.accounting().logical_inverses, 3);
    journal.rollback(&mut bank, outer).expect("outer rollback");
    assert_eq!(bank.get(first), Ok(0));
    assert_eq!(bank.get(second), Ok(0));
}

#[test]
fn root_commit_keeps_values_and_retires_inverse_history() {
    let mut bank = DenseBank::filled(1, 0_u8).expect("bank fits");
    let coordinate = bank.coordinate(0).expect("cell exists");
    let mut journal = FirstWriteJournal::new(&bank);
    let mark = journal.mark(&bank).expect("root mark opens");
    journal
        .write(&mut bank, coordinate, 9)
        .expect("write records");
    journal.commit(&mut bank, mark).expect("root commit");

    assert_eq!(bank.get(coordinate), Ok(9));
    assert_eq!(journal.accounting().logical_inverses, 0);
    assert_eq!(journal.accounting().active_marks, 0);
}

#[test]
fn stale_generation_and_non_top_marks_reject_without_mutation() {
    let mut bank = DenseBank::filled(1, 0_u8).expect("bank fits");
    let coordinate = bank.coordinate(0).expect("cell exists");
    let mut journal = FirstWriteJournal::new(&bank);
    let outer = journal.mark(&bank).expect("outer mark opens");
    let inner = journal.mark(&bank).expect("inner mark opens");
    assert_eq!(
        journal.rollback(&mut bank, outer),
        Err(FirstWriteJournalError::InvalidMark)
    );
    journal
        .write(&mut bank, coordinate, 1)
        .expect("inner write");
    journal.rollback(&mut bank, inner).expect("inner rollback");
    journal.rollback(&mut bank, outer).expect("outer rollback");
    bank.reset_generation(2).expect("generation advances");
    assert_eq!(
        journal.mark(&bank),
        Err(FirstWriteJournalError::ForeignTarget)
    );
    assert_eq!(bank.get(bank.coordinate(0).expect("fresh cell")), Ok(2));
}

#[test]
fn warmed_journal_storage_plateaus_for_ten_thousand_nested_cycles() {
    let mut bank = DenseBank::filled(40, 0_u32).expect("bank fits");
    let coordinates = (0..40)
        .map(|index| bank.coordinate(index).expect("cell exists"))
        .collect::<Vec<_>>();
    let mut journal = FirstWriteJournal::new(&bank);
    let warm = journal.mark(&bank).expect("warm mark opens");
    for (value, &coordinate) in coordinates.iter().enumerate() {
        journal
            .write(&mut bank, coordinate, value as u32)
            .expect("warm write fits");
    }
    journal.rollback(&mut bank, warm).expect("warm rollback");
    let plateau = journal.accounting();

    for cycle in 0_u32..10_000 {
        let mark = journal.mark(&bank).expect("mark reuses storage");
        for (offset, &coordinate) in coordinates.iter().enumerate() {
            journal
                .write(&mut bank, coordinate, cycle + offset as u32)
                .expect("inverse storage is warm");
        }
        journal.rollback(&mut bank, mark).expect("cycle rolls back");
    }

    assert_eq!(journal.accounting(), plateau);
    assert_eq!(plateau.logical_inverses, 0);
    assert!(plateau.retained_inverse_heap_entries >= 40);
}

#[test]
fn journal_marks_and_records_are_plain_fixed_width_values() {
    type Mark = crate::hot_core::journal::FirstWriteMark<DenseBankOwner>;
    assert_eq!(size_of::<Mark>(), 32);
    assert!(!needs_drop::<Mark>());
}
