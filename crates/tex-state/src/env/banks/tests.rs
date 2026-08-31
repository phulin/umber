use super::{BankCell, BankError, DenseBank, LEVEL_ONE, PagedDenseBank, RegisterBank};

fn identity(index: u32) -> u32 {
    index
}

fn zero(_: u32) -> i32 {
    0
}

#[test]
fn dense_bank_reads_one_validated_slot() {
    let mut bank = DenseBank::fixed(4, 0_i32, LEVEL_ONE).expect("dense allocation");
    bank.write(
        2,
        BankCell {
            value: 9,
            level: 3,
            save_serial: 7,
        },
    )
    .expect("write");
    assert_eq!(
        bank.get(2).expect("read"),
        BankCell {
            value: 9,
            level: 3,
            save_serial: 7,
        }
    );
    assert_eq!(bank.get(4), Err(BankError::IndexOutOfBounds));
}

#[test]
fn absent_paged_values_are_direct_algorithmic_defaults() {
    let mut bank = PagedDenseBank::new(1024, identity, LEVEL_ONE).expect("directory");
    assert_eq!(bank.allocated_pages(), 0);
    assert_eq!(bank.get(700).expect("default"), BankCell::level_one(700));
    bank.write(
        700,
        BankCell {
            value: 4,
            level: 2,
            save_serial: 8,
        },
    )
    .expect("write");
    assert_eq!(bank.allocated_pages(), 1);
    assert_eq!(
        bank.get(700).expect("written"),
        BankCell {
            value: 4,
            level: 2,
            save_serial: 8,
        }
    );
    assert_eq!(
        bank.get(701).expect("same page default"),
        BankCell::level_one(701)
    );
}

#[test]
fn register_bank_keeps_classic_prefix_inline_and_pages_only_overflow() {
    let mut bank = RegisterBank::new(zero).expect("register bank");
    bank.write(
        255,
        BankCell {
            value: 1,
            level: 1,
            save_serial: 1,
        },
    )
    .expect("dense write");
    assert_eq!(bank.allocated_overflow_pages(), 0);
    bank.write(
        256,
        BankCell {
            value: 2,
            level: 1,
            save_serial: 2,
        },
    )
    .expect("overflow write");
    assert_eq!(bank.allocated_overflow_pages(), 1);
    assert_eq!(bank.get(255).expect("dense read").value, 1);
    assert_eq!(bank.get(256).expect("overflow read").value, 2);
}
