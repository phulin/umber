//! Native translations of upstream `t/sort-uc.t` at commit 74252e6.

use super::maps::{list_keys, run_fixture};

#[test]
#[ignore = "xfail: Biber case-tailored sorting is not implemented by bib-engine"]
fn assertion_001_u_c_tailoring_1() {
    let result = run_fixture("sort-uc");
    assert_eq!(
        list_keys(&result, 0, "nty/global//global/global/global"),
        ["LS6", "LS5", "LS2", "LS1", "LS3", "LS4"]
    );
}

#[test]
#[ignore = "xfail: Biber case-tailored sorting is not implemented by bib-engine"]
fn assertion_002_u_c_tailoring_2() {
    let result = run_fixture("sort-uc");
    assert_eq!(
        list_keys(&result, 0, "shorthand/global//global/global/global"),
        ["LS3", "LS4", "LS2", "LS1"]
    );
}

#[test]
#[ignore = "xfail: Biber case-tailored sorting is not implemented by bib-engine"]
fn assertion_003_u_c_tailoring_3() {
    let result = run_fixture("sort-uc");
    assert_eq!(
        list_keys(&result, 0, "shorthand/global//global/global/global"),
        ["LS2", "LS1", "LS3", "LS4"]
    );
}

#[test]
#[ignore = "xfail: Biber case-tailored sorting is not implemented by bib-engine"]
fn assertion_004_u_c_tailoring_descending_1() {
    let result = run_fixture("sort-uc");
    assert_eq!(
        list_keys(&result, 0, "nty/global//global/global/global"),
        ["LS3", "LS4", "LS1", "LS2", "LS5", "LS6"]
    );
}

#[test]
#[ignore = "xfail: Biber case-tailored sorting is not implemented by bib-engine"]
fn assertion_005_upper_before_lower_locally_false() {
    let result = run_fixture("sort-uc");
    assert_eq!(
        list_keys(&result, 0, "nty/global//global/global/global"),
        ["LS5", "LS6", "LS4", "LS3", "LS2", "LS1"]
    );
}

#[test]
#[ignore = "xfail: Biber case-tailored sorting is not implemented by bib-engine"]
fn assertion_006_sortcase_locally_false_upper_before_lower_locally_false() {
    let result = run_fixture("sort-uc");
    assert_eq!(
        list_keys(&result, 0, "nty/global//global/global/global"),
        ["LS5", "LS6", "LS3", "LS4", "LS2", "LS1"]
    );
}
