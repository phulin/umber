//! Native translations of upstream `t/dm-constraints.t` at commit 74252e6.

use bib_engine::FieldId;

use super::maps::{entry, run_fixture};

const WARNINGS_C1: &[&str] = &[
    r#"Datamodel: badtype entry 'c1' (dm-constraints.bib): Invalid entry type 'badtype' - defaulting to 'misc'"#,
];

const WARNINGS_C2: &[&str] = &[
    r#"Datamodel: eta entry 'c2' (dm-constraints.bib): Field 'badfield' invalid in data model - ignoring"#,
    r#"Datamodel: eta entry 'c2' (dm-constraints.bib): Invalid field 'journaltitle' for entrytype 'eta'"#,
    r#"Datamodel: eta entry 'c2' (dm-constraints.bib): Missing mandatory field 'author'"#,
];

const WARNINGS_C3: &[&str] = &[
    r#"Datamodel: etb entry 'c3' (dm-constraints.bib): Invalid value of field 'month' must be datatype 'datepart' - ignoring field"#,
    r#"Datamodel: etb entry 'c3' (dm-constraints.bib): Invalid value (pattern match fails) for field 'gender'"#,
];

const WARNINGS_C4: &[&str] = &[
    r#"Datamodel: etb entry 'c4' (dm-constraints.bib): Invalid value of field 'month' must be '<=12' - ignoring field"#,
    r#"Datamodel: etb entry 'c4' (dm-constraints.bib): Invalid value of field 'field1' must be '>=5' - ignoring field"#,
];

const WARNINGS_C5: &[&str] = &[
    r#"Overwriting field 'year' with year value from field 'date' for entry 'c5'"#,
    r#"Datamodel: etb entry 'c5' (dm-constraints.bib): Constraint violation - none of fields (field5, field6) must exist when all of fields (field2, field3, field4) exist. Ignoring them."#,
];

const WARNINGS_C6: &[&str] = &[
    r#"Datamodel: etb entry 'c6' (dm-constraints.bib): Constraint violation - one of fields (field7, field8) must exist when all of fields (field1, field2) exist"#,
    r#"Datamodel: etb entry 'c6' (dm-constraints.bib): Constraint violation - all of fields (field9, field10) must exist when all of fields (field5, field6) exist"#,
];

const WARNINGS_C7: &[&str] = &[
    r#"Datamodel: etc entry 'c7' (dm-constraints.bib): Missing mandatory field - one of 'fielda, fieldb' must be defined"#,
    r#"Datamodel: etc entry 'c7' (dm-constraints.bib): Constraint violation - none of fields (field7) must exist when one of fields (field5, field6) exist. Ignoring them."#,
];

const WARNINGS_C8: &[&str] = &[
    r#"Datamodel: etd entry 'c8' (dm-constraints.bib): Constraint violation - none of fields (field4) must exist when none of fields (field2, field3) exist. Ignoring them."#,
    r#"Datamodel: etd entry 'c8' (dm-constraints.bib): Constraint violation - one of fields (field10, field11) must exist when none of fields (field8, field9) exist"#,
    r#"Datamodel: etd entry 'c8' (dm-constraints.bib): Constraint violation - all of fields (field12, field13) must exist when none of fields (field6) exist"#,
];

const WARNINGS_C10: &[&str] = &[
    r#"Datamodel: misc entry 'c10' (dm-constraints.bib): Invalid ISBN in value of field 'isbn'"#,
    r#"Datamodel: misc entry 'c10' (dm-constraints.bib): Invalid ISSN in value of field 'issn'"#,
];

fn warnings_for<'a>(result: &'a bib_engine::BibResult, key: &str) -> Vec<&'a str> {
    result
        .diagnostics()
        .filter(|diagnostic| {
            diagnostic
                .entry()
                .is_some_and(|entry| entry.as_str() == key)
        })
        .map(|diagnostic| diagnostic.message())
        .collect()
}

fn field_missing(result: &bib_engine::BibResult, key: &str, field: &str) -> bool {
    entry(result, 0, key)
        .is_some_and(|entry| entry.fields().get(&FieldId::new(field).unwrap()).is_none())
}

#[test]
#[ignore = "xfail: Biber data-model diagnostics are not implemented by bib-engine"]
fn assertion_001_constraints_test_1() {
    let result = run_fixture("dm-constraints");
    assert_eq!(warnings_for(&result, "c1"), WARNINGS_C1);
}

#[test]
#[ignore = "xfail: Biber data-model diagnostics are not implemented by bib-engine"]
fn assertion_002_constraints_test_2() {
    let result = run_fixture("dm-constraints");
    assert_eq!(warnings_for(&result, "c2"), WARNINGS_C2);
}

#[test]
#[ignore = "xfail: Biber data-model diagnostics are not implemented by bib-engine"]
fn assertion_003_constraints_test_3a() {
    let result = run_fixture("dm-constraints");
    assert_eq!(warnings_for(&result, "c3"), WARNINGS_C3);
}

#[test]
#[ignore = "xfail: Biber data-model invalid-field removal is not implemented by bib-engine"]
fn assertion_004_constraints_test_3b() {
    let result = run_fixture("dm-constraints");
    assert!(field_missing(&result, "c3", "month"));
}

#[test]
#[ignore = "xfail: Biber data-model diagnostics are not implemented by bib-engine"]
fn assertion_005_constraints_test_4a() {
    let result = run_fixture("dm-constraints");
    assert_eq!(warnings_for(&result, "c4"), WARNINGS_C4);
}

#[test]
#[ignore = "xfail: Biber data-model invalid-field removal is not implemented by bib-engine"]
fn assertion_006_constraints_test_4b() {
    let result = run_fixture("dm-constraints");
    assert!(field_missing(&result, "c4", "month"));
}

#[test]
#[ignore = "xfail: Biber data-model diagnostics are not implemented by bib-engine"]
fn assertion_007_constraints_test_5a() {
    let result = run_fixture("dm-constraints");
    assert_eq!(warnings_for(&result, "c5"), WARNINGS_C5);
}

#[test]
#[ignore = "xfail: Biber data-model invalid-field removal is not implemented by bib-engine"]
fn assertion_008_constraints_test_5b() {
    let result = run_fixture("dm-constraints");
    assert!(field_missing(&result, "c5", "field5"));
}

#[test]
#[ignore = "xfail: Biber data-model invalid-field removal is not implemented by bib-engine"]
fn assertion_009_constraints_test_5c() {
    let result = run_fixture("dm-constraints");
    assert!(field_missing(&result, "c5", "field6"));
}

#[test]
#[ignore = "xfail: Biber data-model diagnostics are not implemented by bib-engine"]
fn assertion_010_constraints_test_6() {
    let result = run_fixture("dm-constraints");
    assert_eq!(warnings_for(&result, "c6"), WARNINGS_C6);
}

#[test]
#[ignore = "xfail: Biber data-model diagnostics are not implemented by bib-engine"]
fn assertion_011_constraints_test_7a() {
    let result = run_fixture("dm-constraints");
    assert_eq!(warnings_for(&result, "c7"), WARNINGS_C7);
}

#[test]
#[ignore = "xfail: Biber data-model invalid-field removal is not implemented by bib-engine"]
fn assertion_012_constraints_test_7b() {
    let result = run_fixture("dm-constraints");
    assert!(field_missing(&result, "c7", "field7"));
}

#[test]
#[ignore = "xfail: Biber data-model diagnostics are not implemented by bib-engine"]
fn assertion_013_constraints_test_8a() {
    let result = run_fixture("dm-constraints");
    assert_eq!(warnings_for(&result, "c8"), WARNINGS_C8);
}

#[test]
#[ignore = "xfail: Biber data-model invalid-field removal is not implemented by bib-engine"]
fn assertion_014_constraints_test_8b() {
    let result = run_fixture("dm-constraints");
    assert!(field_missing(&result, "c8", "field4"));
}

#[test]
fn assertion_015_constraints_test_9() {
    let result = run_fixture("dm-constraints");
    assert!(warnings_for(&result, "c9").is_empty());
}

#[test]
#[ignore = "xfail: Biber data-model diagnostics are not implemented by bib-engine"]
fn assertion_016_constraints_test_10() {
    let result = run_fixture("dm-constraints");
    assert_eq!(warnings_for(&result, "c10"), WARNINGS_C10);
}
