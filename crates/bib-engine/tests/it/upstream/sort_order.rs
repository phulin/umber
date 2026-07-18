//! Native translations of upstream `t/sort-order.t` at commit 74252e6.

use super::maps::{list_keys, try_run_fixture};

#[test]
#[ignore = "xfail: Biber sorting-template mutation/order is not implemented by bib-engine"]
fn assertion_001_sorting_none_and_nocite_second() {
    let result = try_run_fixture("sort-order");
    let keys = result
        .as_ref()
        .ok()
        .map(|result| list_keys(result, 1, "none/global//global/global/global"))
        .unwrap_or_default();
    assert_eq!(
        keys,
        [
            "L2", "L1", "L1A", "L1B", "L3", "L4", "L5", "L6", "L7", "L8", "L9"
        ]
    );
}

#[test]
#[ignore = "xfail: Biber sorting-template mutation/order is not implemented by bib-engine"]
fn assertion_002_sorting_none_and_nocite_first() {
    let result = try_run_fixture("sort-order");
    let keys = result
        .as_ref()
        .ok()
        .map(|result| list_keys(result, 2, "none/global//global/global/global"))
        .unwrap_or_default();
    assert_eq!(
        keys,
        [
            "L1", "L1A", "L1B", "L2", "L3", "L4", "L5", "L6", "L7", "L8", "L9"
        ]
    );
}

#[test]
#[ignore = "xfail: Biber sorting-template mutation/order is not implemented by bib-engine"]
fn assertion_003_citeorder() {
    let result = try_run_fixture("sort-order");
    let keys = result
        .as_ref()
        .ok()
        .map(|result| list_keys(result, 0, "none/global//global/global/global"))
        .unwrap_or_default();
    assert_eq!(
        keys,
        [
            "L2", "L3", "L1B", "L1", "L4", "L5", "L1A", "L7", "L8", "L6", "L9"
        ]
    );
}

#[test]
#[ignore = "xfail: Biber sorting-template mutation/order is not implemented by bib-engine"]
fn assertion_004_nty() {
    let result = try_run_fixture("sort-order");
    let keys = result
        .as_ref()
        .ok()
        .map(|result| list_keys(result, 0, "none/global//global/global/global"))
        .unwrap_or_default();
    assert_eq!(
        keys,
        [
            "L5", "L1A", "L1", "L1B", "L2", "L3", "L4", "L8", "L7", "L6", "L9"
        ]
    );
}

#[test]
#[ignore = "xfail: Biber sorting-template mutation/order is not implemented by bib-engine"]
fn assertion_005_nyt() {
    let result = try_run_fixture("sort-order");
    let keys = result
        .as_ref()
        .ok()
        .map(|result| list_keys(result, 0, "none/global//global/global/global"))
        .unwrap_or_default();
    assert_eq!(
        keys,
        [
            "L5", "L1A", "L1", "L1B", "L2", "L3", "L4", "L8", "L7", "L6", "L9"
        ]
    );
}

#[test]
#[ignore = "xfail: Biber sorting-template mutation/order is not implemented by bib-engine"]
fn assertion_006_nyvt() {
    let result = try_run_fixture("sort-order");
    let keys = result
        .as_ref()
        .ok()
        .map(|result| list_keys(result, 0, "none/global//global/global/global"))
        .unwrap_or_default();
    assert_eq!(
        keys,
        [
            "L5", "L1", "L1A", "L1B", "L2", "L3", "L4", "L8", "L7", "L6", "L9"
        ]
    );
}

#[test]
#[ignore = "xfail: Biber sorting-template mutation/order is not implemented by bib-engine"]
fn assertion_007_nyvt_with_volume_padding() {
    let result = try_run_fixture("sort-order");
    let keys = result
        .as_ref()
        .ok()
        .map(|result| list_keys(result, 0, "none/global//global/global/global"))
        .unwrap_or_default();
    assert_eq!(
        keys,
        [
            "L5", "L1A", "L1", "L1B", "L2", "L3", "L4", "L8", "L7", "L6", "L9"
        ]
    );
}

#[test]
#[ignore = "xfail: Biber sorting-template mutation/order is not implemented by bib-engine"]
fn assertion_008_ynt() {
    let result = try_run_fixture("sort-order");
    let keys = result
        .as_ref()
        .ok()
        .map(|result| list_keys(result, 0, "none/global//global/global/global"))
        .unwrap_or_default();
    assert_eq!(
        keys,
        [
            "L3", "L1B", "L1A", "L1", "L4", "L2", "L8", "L7", "L6", "L9", "L5"
        ]
    );
}

#[test]
#[ignore = "xfail: Biber sorting-template mutation/order is not implemented by bib-engine"]
fn assertion_009_ynt_with_year_substring() {
    let result = try_run_fixture("sort-order");
    let keys = result
        .as_ref()
        .ok()
        .map(|result| list_keys(result, 0, "none/global//global/global/global"))
        .unwrap_or_default();
    assert_eq!(
        keys,
        [
            "L3", "L1B", "L1A", "L1", "L2", "L4", "L8", "L7", "L6", "L9", "L5"
        ]
    );
}

#[test]
#[ignore = "xfail: Biber sorting-template mutation/order is not implemented by bib-engine"]
fn assertion_010_ydnt() {
    let result = try_run_fixture("sort-order");
    let keys = result
        .as_ref()
        .ok()
        .map(|result| list_keys(result, 0, "none/global//global/global/global"))
        .unwrap_or_default();
    assert_eq!(
        keys,
        [
            "L5", "L9", "L6", "L7", "L8", "L2", "L4", "L1A", "L1", "L1B", "L3"
        ]
    );
}

#[test]
#[ignore = "xfail: Biber sorting-template mutation/order is not implemented by bib-engine"]
fn assertion_011_entrytype() {
    let result = try_run_fixture("sort-order");
    let keys = result
        .as_ref()
        .ok()
        .map(|result| list_keys(result, 0, "none/global//global/global/global"))
        .unwrap_or_default();
    assert_eq!(
        keys,
        [
            "L2", "L3", "L1B", "L1", "L1A", "L4", "L5", "L7", "L8", "L6", "L9"
        ]
    );
}

#[test]
#[ignore = "xfail: Biber sorting-template mutation/order is not implemented by bib-engine"]
fn assertion_012_anyt() {
    let result = try_run_fixture("sort-order");
    let keys = result
        .as_ref()
        .ok()
        .map(|result| list_keys(result, 0, "none/global//global/global/global"))
        .unwrap_or_default();
    assert_eq!(
        keys,
        [
            "L1B", "L1A", "L1", "L2", "L3", "L4", "L5", "L8", "L7", "L6", "L9"
        ]
    );
}

#[test]
#[ignore = "xfail: Biber sorting-template mutation/order is not implemented by bib-engine"]
fn assertion_013_anyvt() {
    let result = try_run_fixture("sort-order");
    let keys = result
        .as_ref()
        .ok()
        .map(|result| list_keys(result, 0, "none/global//global/global/global"))
        .unwrap_or_default();
    assert_eq!(
        keys,
        [
            "L1B", "L1", "L1A", "L2", "L3", "L4", "L5", "L8", "L7", "L6", "L9"
        ]
    );
}

#[test]
#[ignore = "xfail: Biber sorting-template mutation/order is not implemented by bib-engine"]
fn assertion_014_nty_with_descending_n() {
    let result = try_run_fixture("sort-order");
    let keys = result
        .as_ref()
        .ok()
        .map(|result| list_keys(result, 0, "none/global//global/global/global"))
        .unwrap_or_default();
    assert_eq!(
        keys,
        [
            "L9", "L6", "L7", "L8", "L5", "L4", "L3", "L2", "L1B", "L1A", "L1"
        ]
    );
}

#[test]
#[ignore = "xfail: Biber sorting-template mutation/order is not implemented by bib-engine"]
fn assertion_015_nosort_1() {
    let result = try_run_fixture("sort-order");
    let keys = result
        .as_ref()
        .ok()
        .map(|result| list_keys(result, 0, "none/global//global/global/global"))
        .unwrap_or_default();
    assert_eq!(
        keys,
        [
            "L1A", "L1", "L1B", "L2", "L3", "L4", "L5", "L7", "L6", "L9", "L8"
        ]
    );
}

#[test]
#[ignore = "xfail: Biber sorting-template mutation/order is not implemented by bib-engine"]
fn assertion_016_sorting_none_year() {
    let result = try_run_fixture("sort-order");
    let keys = result
        .as_ref()
        .ok()
        .map(|result| list_keys(result, 0, "none/global//global/global/global"))
        .unwrap_or_default();
    assert_eq!(
        keys,
        [
            "L3", "L2", "L1B", "L1", "L4", "L5", "L1A", "L7", "L8", "L6", "L9"
        ]
    );
}

#[test]
#[ignore = "xfail: Biber sorting-template mutation/order is not implemented by bib-engine"]
fn assertion_017_citecount_1() {
    let result = try_run_fixture("sort-order");
    let keys = result
        .as_ref()
        .ok()
        .map(|result| list_keys(result, 0, "none/global//global/global/global"))
        .unwrap_or_default();
    assert_eq!(
        keys,
        [
            "L9", "L4", "L6", "L7", "L8", "L5", "L2", "L1", "L1A", "L1B", "L3"
        ]
    );
}

#[test]
#[ignore = "xfail: Biber sorting-template mutation/order is not implemented by bib-engine"]
fn assertion_018_sorting_none_and_allkeys() {
    let result = try_run_fixture("sort-order");
    let keys = result
        .as_ref()
        .ok()
        .map(|result| list_keys(result, 0, "none/global//global/global/global"))
        .unwrap_or_default();
    assert_eq!(
        keys,
        [
            "L1", "L1A", "L1B", "L2", "L3", "L4", "L5", "L6", "L7", "L8", "L9"
        ]
    );
}
