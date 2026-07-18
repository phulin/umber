//! Native translations of upstream `t/sort-case.t` at commit 74252e6.

use super::maps::{list_keys, run_fixture};

#[test]
#[ignore = "xfail: per-run Biber sorting-template overrides are not exposed by bib-engine"]
fn assertion_001_u_c_case_1() {
    let result = run_fixture("sort-case");
    assert_eq!(
        list_keys(&result, 0, "custom/global//global/global/global"),
        ["CS1", "CS3", "CS2"]
    );
}

#[test]
#[ignore = "xfail: Biber case-sensitive sorting is not implemented by bib-engine"]
fn assertion_002_u_c_case_2() {
    let result = run_fixture("sort-case");
    assert_eq!(
        list_keys(&result, 0, "custom/global//global/global/global"),
        ["CS3", "CS2", "CS1"]
    );
}
