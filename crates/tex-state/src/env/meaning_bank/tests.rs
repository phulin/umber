use super::MeaningBank;

#[test]
fn meaning_bank_keeps_hot_values_and_cold_metadata_in_parallel_storage() {
    let mut bank = MeaningBank::new(4, 0_i32).expect("meaning bank");
    let values_ptr = bank.values.as_ptr();
    let levels_ptr = bank.levels.as_ptr();
    let serials_ptr = bank.save_serials.as_ptr();
    let capacity = bank.capacity();

    bank.admit_through(2).expect("admit rows");
    assert_eq!(bank.len(), 3);
    assert_eq!(bank.capacity(), capacity);
    assert_eq!(bank.values.as_ptr(), values_ptr);
    assert_eq!(bank.levels.as_ptr(), levels_ptr);
    assert_eq!(bank.save_serials.as_ptr(), serials_ptr);
    assert_eq!(bank.get_ref(2), Ok(&0));

    bank.write(2, 7, 3, 11).expect("write row");
    assert_eq!(bank.row(2), Ok((7, 3, 11)));
    let mut alternate = 9;
    let mut alternate_level = 4;
    let mut alternate_serial = 12;
    bank.swap(
        2,
        &mut alternate,
        &mut alternate_level,
        &mut alternate_serial,
    )
    .expect("swap row");
    assert_eq!((alternate, alternate_level, alternate_serial), (7, 3, 11));
    assert_eq!(bank.row(2), Ok((9, 4, 12)));
}
