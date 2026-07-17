use std::fmt::Debug;

#[track_caller]
pub fn xfail_string(assertion: &str, expected: &str, actual: &str) {
    require_failure(assertion, "exact string", expected == actual);
}

#[track_caller]
pub fn xfail_bytes(assertion: &str, expected: &[u8], actual: &[u8]) {
    require_failure(assertion, "exact bytes", expected == actual);
}

#[track_caller]
pub fn xfail_deep<T>(assertion: &str, expected: &T, actual: &T)
where
    T: Debug + PartialEq + ?Sized,
{
    require_failure(assertion, "deep value", expected == actual);
}

#[track_caller]
pub fn xfail_diagnostics<T>(
    assertion: &str,
    expected: &[T],
    actual: &[T],
    expected_rendered: &str,
    actual_rendered: &str,
) where
    T: Debug + PartialEq,
{
    require_failure(
        assertion,
        "structured diagnostics and rendered text",
        expected == actual && expected_rendered == actual_rendered,
    );
}

#[track_caller]
fn require_failure(assertion: &str, comparison: &str, passed: bool) {
    assert!(
        !passed,
        "XPASS: upstream assertion `{assertion}` unexpectedly passed its {comparison} comparison"
    );
}
