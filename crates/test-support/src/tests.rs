use crate::read_fixture;

#[test]
fn hello_fixture_is_committed() {
    let expected = read_fixture("hello", "hello", "log");
    assert!(
        expected.contains("hello umber"),
        "hello fixture should keep the reference message"
    );
}
