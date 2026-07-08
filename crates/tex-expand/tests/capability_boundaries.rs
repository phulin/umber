#[test]
fn scanner_helpers_cannot_open_input() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/scanner_helper_input_open_forbidden.rs");
}
