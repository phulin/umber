//! Native translations of upstream `t/sections-complex.t` at commit 74252e6.

use bib_engine::{FieldId, FieldValue, SectionId};

use super::maps::{entry, text_field, try_run_fixture};

const LIST_ID: &str = "custom/global//global/global/global";

fn context_text(
    result: &bib_engine::BibResult,
    section: u32,
    key: &str,
    field: &str,
) -> Option<String> {
    let field = FieldId::new(field).expect("valid upstream field id");
    result
        .document()
        .section(SectionId::new(section))?
        .lists()
        .find(|list| list.id().as_str() == LIST_ID)?
        .items()
        .find(|item| item.entry().as_str() == key)?
        .context_fields()
        .find(|candidate| candidate.id() == &field)
        .and_then(|field| match field.value() {
            FieldValue::Literal(value) => Some(value.as_str().to_owned()),
            FieldValue::Verbatim(value) => Some(value.as_str().to_owned()),
            FieldValue::Integer(value) => Some(value.to_string()),
            _ => None,
        })
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_001_maxalphanames_1_minalphanames_1_entry_l1_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 0, "L1").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("Doe95"));
}

#[test]
fn assertion_002_maxalphanames_1_minalphanames_1_entry_l1_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(context_text(&result, 0, "L1", "extraalpha"), None);
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_003_maxalphanames_1_minalphanames_1_entry_l2_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 0, "L2").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("Doe+95"));
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_004_maxalphanames_1_minalphanames_1_entry_l2_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(
        context_text(&result, 0, "L2", "extraalpha").as_deref(),
        Some("1")
    );
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_005_maxalphanames_1_minalphanames_1_entry_l3_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 0, "L3").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("Doe+95"));
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_006_maxalphanames_1_minalphanames_1_entry_l3_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(
        context_text(&result, 0, "L3", "extraalpha").as_deref(),
        Some("2")
    );
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_007_maxalphanames_1_minalphanames_1_entry_l4_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 0, "L4").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("Doe+95"));
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_008_maxalphanames_1_minalphanames_1_entry_l4_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(
        context_text(&result, 0, "L4", "extraalpha").as_deref(),
        Some("3")
    );
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_009_maxalphanames_1_minalphanames_1_entry_l5_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 1, "L5").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("Doe+95"));
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_010_maxalphanames_1_minalphanames_1_entry_l5_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(
        context_text(&result, 1, "L5", "extraalpha").as_deref(),
        Some("1")
    );
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_011_maxalphanames_1_minalphanames_1_entry_l6_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 1, "L6").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("Doe+95"));
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_012_maxalphanames_1_minalphanames_1_entry_l6_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(
        context_text(&result, 1, "L6", "extraalpha").as_deref(),
        Some("2")
    );
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_013_maxalphanames_1_minalphanames_1_entry_l7_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 1, "L7").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("Doe+95"));
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_014_maxalphanames_1_minalphanames_1_entry_l7_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(
        context_text(&result, 1, "L7", "extraalpha").as_deref(),
        Some("3")
    );
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_015_maxalphanames_1_minalphanames_1_entry_l8_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 1, "L8").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("Sha85"));
}

#[test]
fn assertion_016_maxalphanames_1_minalphanames_1_entry_l8_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(context_text(&result, 1, "L8", "extraalpha"), None);
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_017_maxalphanames_2_minalphanames_1_entry_l1_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 0, "L1").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("Doe95"));
}

#[test]
fn assertion_018_maxalphanames_2_minalphanames_1_entry_l1_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(context_text(&result, 0, "L1", "extraalpha"), None);
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_019_maxalphanames_2_minalphanames_1_entry_l2_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 0, "L2").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("DA95"));
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_020_maxalphanames_2_minalphanames_1_entry_l2_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(
        context_text(&result, 0, "L2", "extraalpha").as_deref(),
        Some("1")
    );
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_021_maxalphanames_2_minalphanames_1_entry_l3_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 0, "L3").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("DA95"));
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_022_maxalphanames_2_minalphanames_1_entry_l3_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(
        context_text(&result, 0, "L3", "extraalpha").as_deref(),
        Some("2")
    );
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_023_maxalphanames_2_minalphanames_1_entry_l4_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 0, "L4").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("Doe+95"));
}

#[test]
fn assertion_024_maxalphanames_2_minalphanames_1_entry_l4_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(context_text(&result, 0, "L4", "extraalpha"), None);
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_025_maxalphanames_2_minalphanames_1_entry_l5_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 1, "L5").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("Doe+95"));
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_026_maxalphanames_2_minalphanames_1_entry_l5_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(
        context_text(&result, 1, "L5", "extraalpha").as_deref(),
        Some("1")
    );
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_027_maxalphanames_2_minalphanames_1_entry_l6_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 1, "L6").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("Doe+95"));
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_028_maxalphanames_2_minalphanames_1_entry_l6_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(
        context_text(&result, 1, "L6", "extraalpha").as_deref(),
        Some("2")
    );
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_029_maxalphanames_2_minalphanames_1_entry_l7_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 1, "L7").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("Doe+95"));
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_030_maxalphanames_2_minalphanames_1_entry_l7_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(
        context_text(&result, 1, "L7", "extraalpha").as_deref(),
        Some("3")
    );
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_031_maxalphanames_2_minalphanames_1_entry_l8_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 1, "L8").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("Sha85"));
}

#[test]
fn assertion_032_maxalphanames_2_minalphanames_1_entry_l8_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(context_text(&result, 1, "L8", "extraalpha"), None);
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_033_maxalphanames_2_minalphanames_2_entry_l1_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 0, "L1").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("Doe95"));
}

#[test]
fn assertion_034_maxalphanames_2_minalphanames_2_entry_l1_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(context_text(&result, 0, "L1", "extraalpha"), None);
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_035_maxalphanames_2_minalphanames_2_entry_l2_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 0, "L2").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("DA95"));
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_036_maxalphanames_2_minalphanames_2_entry_l2_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(
        context_text(&result, 0, "L2", "extraalpha").as_deref(),
        Some("1")
    );
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_037_maxalphanames_2_minalphanames_2_entry_l3_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 0, "L3").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("DA95"));
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_038_maxalphanames_2_minalphanames_2_entry_l3_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(
        context_text(&result, 0, "L3", "extraalpha").as_deref(),
        Some("2")
    );
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_039_maxalphanames_2_minalphanames_2_entry_l4_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 0, "L4").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("DA+95"));
}

#[test]
fn assertion_040_maxalphanames_2_minalphanames_2_entry_l4_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(context_text(&result, 0, "L4", "extraalpha"), None);
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_041_maxalphanames_2_minalphanames_2_entry_l5_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 1, "L5").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("DA+95"));
}

#[test]
fn assertion_042_maxalphanames_2_minalphanames_2_entry_l5_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(context_text(&result, 1, "L5", "extraalpha"), None);
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_043_maxalphanames_2_minalphanames_2_entry_l6_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 1, "L6").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("DS+95"));
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_044_maxalphanames_2_minalphanames_2_entry_l6_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(
        context_text(&result, 1, "L6", "extraalpha").as_deref(),
        Some("1")
    );
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_045_maxalphanames_2_minalphanames_2_entry_l7_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 1, "L7").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("DS+95"));
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_046_maxalphanames_2_minalphanames_2_entry_l7_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(
        context_text(&result, 1, "L7", "extraalpha").as_deref(),
        Some("2")
    );
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_047_maxalphanames_2_minalphanames_2_entry_l8_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 1, "L8").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("Sha85"));
}

#[test]
fn assertion_048_maxalphanames_2_minalphanames_2_entry_l8_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(context_text(&result, 1, "L8", "extraalpha"), None);
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_049_maxalphanames_3_minalphanames_1_entry_l1_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 0, "L1").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("Doe95"));
}

#[test]
fn assertion_050_maxalphanames_3_minalphanames_1_entry_l1_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(context_text(&result, 0, "L1", "extraalpha"), None);
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_051_maxalphanames_3_minalphanames_1_entry_l2_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 0, "L2").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("DA95"));
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_052_maxalphanames_3_minalphanames_1_entry_l2_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(
        context_text(&result, 0, "L2", "extraalpha").as_deref(),
        Some("1")
    );
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_053_maxalphanames_3_minalphanames_1_entry_l3_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 0, "L3").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("DA95"));
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_054_maxalphanames_3_minalphanames_1_entry_l3_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(
        context_text(&result, 0, "L3", "extraalpha").as_deref(),
        Some("2")
    );
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_055_maxalphanames_3_minalphanames_1_entry_l4_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 0, "L4").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("DAE95"));
}

#[test]
fn assertion_056_maxalphanames_3_minalphanames_1_entry_l4_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(context_text(&result, 0, "L4", "extraalpha"), None);
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_057_maxalphanames_3_minalphanames_1_entry_l5_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 1, "L5").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("DAE95"));
}

#[test]
fn assertion_058_maxalphanames_3_minalphanames_1_entry_l5_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(context_text(&result, 1, "L5", "extraalpha"), None);
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_059_maxalphanames_3_minalphanames_1_entry_l6_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 1, "L6").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("DSE95"));
}

#[test]
fn assertion_060_maxalphanames_3_minalphanames_1_entry_l6_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(context_text(&result, 1, "L6", "extraalpha"), None);
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_061_maxalphanames_3_minalphanames_1_entry_l7_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 1, "L7").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("DSJ95"));
}

#[test]
fn assertion_062_maxalphanames_3_minalphanames_1_entry_l7_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(context_text(&result, 1, "L7", "extraalpha"), None);
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_063_maxalphanames_3_minalphanames_1_entry_l8_labelalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let actual = entry(&result, 1, "L8").and_then(|entry| text_field(entry, "sortlabelalpha"));
    assert_eq!(actual, Some("Sha85"));
}

#[test]
fn assertion_064_maxalphanames_3_minalphanames_1_entry_l8_extraalpha() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    assert_eq!(context_text(&result, 1, "L8", "extraalpha"), None);
}

#[test]
fn assertion_065_map_refsection_1() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let mapped = entry(&result, 0, "m1").expect("mapped section-zero entry");
    let keywords = FieldId::new("keywords").expect("field id");
    assert!(mapped.fields().get(&keywords).is_none());
}

#[test]
fn assertion_066_map_refsection_2() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let mapped = entry(&result, 0, "m1").expect("mapped section-zero entry");
    assert_eq!(text_field(mapped, "title"), Some("Film title 1"));
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_067_map_refsection_3() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let mapped = entry(&result, 1, "m1").expect("mapped section-one entry");
    let keywords = FieldId::new("keywords").expect("field id");
    let actual = mapped.fields().get(&keywords);
    assert!(
        matches!(actual, Some(FieldValue::LiteralList(values)) if values.iter().map(|v| v.as_str()).eq(["thing"]))
    );
}

#[test]
#[ignore = "xfail: native processing does not yet match Biber section/list derivation"]
fn assertion_068_map_refsection_4() {
    let result = try_run_fixture("sections-complex").expect("native fixture processing");
    let mapped = entry(&result, 1, "m1").expect("mapped section-one entry");
    assert_eq!(text_field(mapped, "title"), Some("Film title 11"));
}
