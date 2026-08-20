use super::Epoch;

#[test]
fn epoch_starts_at_one_and_bumps() {
    let mut epoch = Epoch::START;

    assert_eq!(epoch, Epoch::START);
    epoch.bump();
    assert!(epoch > Epoch::START);
}
