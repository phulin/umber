//! Native translation of upstream `t/sort-names.t` at commit 74252e6.

use super::maps::{list_keys, run_fixture};

#[test]
#[ignore = "xfail: Biber name sorting is not implemented by bib-engine"]
fn assertion_001_names_order() {
    let result = run_fixture("sort-names");
    assert_eq!(
        list_keys(&result, 0, "none/global//global/global/global"),
        ["N4", "N1", "N2", "N3"]
    );
}
