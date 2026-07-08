use super::Epoch;

#[test]
fn epoch_starts_at_one_and_bumps() {
    let mut epoch = Epoch::START;

    assert_eq!(epoch.raw(), 1);
    epoch.bump();
    assert_eq!(epoch.raw(), 2);
}
