#[test]
fn expansion_capability_rejects_privileged_apis() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/expansion_ctx_forbidden.rs");
    tests.compile_fail("tests/ui/input_open_ctx_forbidden.rs");
}
