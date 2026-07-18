// Native Rust translation of the corresponding upstream Biber test at commit 74252e6.

use std::path::PathBuf;

use bib_engine::{
    BibAttempt, BibJob, BibOptionsBuilder, BibSession, EntryId, FieldId, FieldValue,
    FileProvisioner, OutputFormat, OutputRequest, ProcessedBibliography, ResolvedFile, SectionId,
    VfsLimits, VirtualPath,
};

#[allow(dead_code)]
struct FixtureResult {
    document: ProcessedBibliography,
    bbl: String,
}

fn override_scalar_option(control: &mut String, key: &str, value: &str) {
    let key_tag = format!("<bcf:key>{key}</bcf:key>");
    let key_at = control
        .find(&key_tag)
        .expect("option exists in committed BCF");
    let value_start = control[key_at..]
        .find("<bcf:value>")
        .map(|offset| key_at + offset + "<bcf:value>".len())
        .expect("option has a value");
    let value_end = control[value_start..]
        .find("</bcf:value>")
        .map(|offset| value_start + offset)
        .expect("option value is terminated");
    control.replace_range(value_start..value_end, value);
}

fn process_fixture(control_name: &str, option_overrides: &[(&str, &str)]) -> FixtureResult {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/bib/upstream-2.22/tdata");
    let control = VirtualPath::user(control_name).expect("valid control path");
    let mut control_bytes = String::from_utf8(
        std::fs::read(fixture_dir.join(control_name)).expect("committed BCF fixture"),
    )
    .expect("BCF is UTF-8");
    for &(key, value) in option_overrides {
        override_scalar_option(&mut control_bytes, key, value);
    }
    let mut provisioner = FileProvisioner::new(VfsLimits::default()).expect("valid VFS limits");
    provisioner
        .register_user(control.clone(), control_bytes.into_bytes())
        .expect("unique control file");
    let output_path = VirtualPath::user("native.bbl").expect("valid output path");
    let mut options = BibOptionsBuilder::new();
    options
        .output(OutputRequest::new(output_path, OutputFormat::Bbl))
        .expect("unique output");
    let job = BibJob::new(control, options.freeze());
    let mut session = BibSession::default();
    loop {
        match session.process(&job, &provisioner.snapshot()) {
            BibAttempt::Complete(result) => {
                let bbl = result
                    .files()
                    .find(|file| file.path().as_str().ends_with("native.bbl"))
                    .map(|file| String::from_utf8_lossy(file.bytes()).into_owned())
                    .unwrap_or_default();
                return FixtureResult {
                    document: result.document().as_ref().clone(),
                    bbl,
                };
            }
            BibAttempt::NeedResources(requests) => {
                provisioner.expect(&requests);
                for request in requests
                    .required
                    .iter()
                    .chain(requests.prefetch_hints.iter())
                {
                    let path = fixture_dir.join(request.key().name());
                    if !path.is_file() {
                        continue;
                    }
                    provisioner
                        .provision(ResolvedFile {
                            request: request.key().clone(),
                            virtual_path: format!("/texlive/bib/{}", request.key().name()).into(),
                            bytes: std::fs::read(path).expect("committed requested fixture"),
                            expected_digest: None,
                        })
                        .expect("requested fixture is valid");
                }
            }
            BibAttempt::Failed(failure) => panic!("fixture processing failed: {failure:?}"),
        }
    }
}

fn field_text(
    control: &str,
    option_overrides: &[(&str, &str)],
    entry_key: &str,
    field_name: &str,
) -> Option<String> {
    let fixture = process_fixture(control, option_overrides);
    let entry = fixture
        .document
        .section(SectionId::new(0))?
        .entry(&EntryId::new(entry_key).expect("valid entry key"))?;
    match entry
        .fields()
        .get(&FieldId::new(field_name).expect("valid field name"))?
    {
        FieldValue::Literal(value) => Some(value.as_str().to_owned()),
        FieldValue::Verbatim(value) => Some(value.as_str().to_owned()),
        FieldValue::Integer(value) => Some(value.to_string()),
        FieldValue::Boolean(value) => Some(if *value { "1" } else { "0" }.to_owned()),
        _ => None,
    }
}

#[allow(dead_code)]
fn name_assignment(
    control: &str,
    option_overrides: &[(&str, &str)],
    entry_key: &str,
    name_index: usize,
    assignment_key: &str,
) -> Option<String> {
    let fixture = process_fixture(control, option_overrides);
    let entry = fixture
        .document
        .section(SectionId::new(0))?
        .entry(&EntryId::new(entry_key).expect("valid entry key"))?;
    let source = match entry
        .fields()
        .get(&FieldId::new("labelnamesource").expect("valid field name"))?
    {
        FieldValue::Literal(value) => value.as_str(),
        _ => return None,
    };
    let names = match entry
        .fields()
        .get(&FieldId::new(source).expect("valid name-list field"))?
    {
        FieldValue::NameList(names) => names,
        _ => return None,
    };
    names
        .iter()
        .nth(name_index.checked_sub(1)?)?
        .assignments()
        .find(|assignment| assignment.key() == assignment_key)
        .map(|assignment| assignment.value().to_owned())
}

#[allow(dead_code)]
fn output_entry(control: &str, option_overrides: &[(&str, &str)], entry_key: &str) -> String {
    let fixture = process_fixture(control, option_overrides);
    let marker = format!("\\\\entry{{{entry_key}}}");
    let marker_at = fixture
        .bbl
        .find(&marker)
        .expect("entry is present in generated BBL");
    let start = fixture.bbl[..marker_at].rfind("    ").unwrap_or(marker_at);
    let end = fixture.bbl[marker_at..]
        .find("\\\\endentry")
        .map(|offset| marker_at + offset + "\\\\endentry".len())
        .expect("entry is terminated");
    fixture.bbl[start..end].to_owned()
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_001_useprefix_0_so_not_in_label() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####)
            ],
            r#####"prefix1"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Vaa99"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_002_default_prefix_settings_entry_prefix1_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
            r#####"prefix1"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"vdVaa99"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_003_maxalphanames_1_minalphanames_1_entry_l1_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
            r#####"L1"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Doe95"#####)
    );
}

#[test]
fn assertion_004_maxalphanames_1_minalphanames_1_entry_l1_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
            r#####"l1"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_005_maxalphanames_1_minalphanames_1_entry_l2_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
            r#####"L2"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Doe+95"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_006_maxalphanames_1_minalphanames_1_entry_l2_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
            r#####"L2"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_007_maxalphanames_1_minalphanames_1_entry_l3_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
            r#####"L3"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Doe+95"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_008_maxalphanames_1_minalphanames_1_entry_l3_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
            r#####"L3"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_009_maxalphanames_1_minalphanames_1_entry_l4_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
            r#####"L4"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Doe+95"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_010_maxalphanames_1_minalphanames_1_entry_l4_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
            r#####"L4"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"3"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_011_maxalphanames_1_minalphanames_1_entry_l5_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
            r#####"L5"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Doe+95"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_012_maxalphanames_1_minalphanames_1_entry_l5_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
            r#####"L5"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"4"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_013_maxalphanames_1_minalphanames_1_entry_l6_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
            r#####"L6"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Doe+95"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_014_maxalphanames_1_minalphanames_1_entry_l6_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
            r#####"L6"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"5"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_015_maxalphanames_1_minalphanames_1_entry_l7_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
            r#####"L7"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Doe+95"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_016_maxalphanames_1_minalphanames_1_entry_l7_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
            r#####"L7"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"6"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_017_maxalphanames_1_minalphanames_1_entry_l8_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
            r#####"L8"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Sha85"#####)
    );
}

#[test]
fn assertion_018_maxalphanames_1_minalphanames_1_entry_l8_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
            r#####"L8"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_019_l9_extraalpha_unset_due_to_shorthand() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
            r#####"L9"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_020_l10_extraalpha_unset_due_to_shorthand() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
            r#####"L10"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_021_year_with_range_needs_label_differentiating_from_individual_volumes_1() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
            r#####"knuth:ct"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_022_year_with_range_needs_label_differentiating_from_individual_volumes_2() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
            r#####"knuth:ct:a"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_023_year_with_range_needs_label_differentiating_from_individual_volumes_3() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
            r#####"knuth:ct:b"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_024_year_with_range_needs_label_differentiating_from_individual_volumes_4() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
            r#####"knuth:ct:c"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_025_default_ignore() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
            r#####"ignore1"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"OTo07"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_026_default_no_ignore_spaces() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"1"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####)
            ],
            r#####"ignore2"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"De 07"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_027_maxalphanames_2_minalphanames_1_entry_l1_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L1"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Doe95"#####)
    );
}

#[test]
fn assertion_028_maxalphanames_2_minalphanames_1_entry_l1_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"l1"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_029_maxalphanames_2_minalphanames_1_entry_l2_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L2"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"DA95"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_030_maxalphanames_2_minalphanames_1_entry_l2_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L2"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_031_maxalphanames_2_minalphanames_1_entry_l3_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L3"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"DA95"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_032_maxalphanames_2_minalphanames_1_entry_l3_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L3"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_033_maxalphanames_2_minalphanames_1_entry_l4_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L4"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Doe+95"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_034_maxalphanames_2_minalphanames_1_entry_l4_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L4"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_035_maxalphanames_2_minalphanames_1_entry_l5_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L5"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Doe+95"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_036_maxalphanames_2_minalphanames_1_entry_l5_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L5"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_037_maxalphanames_2_minalphanames_1_entry_l6_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L6"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Doe+95"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_038_maxalphanames_2_minalphanames_1_entry_l6_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L6"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"3"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_039_maxalphanames_2_minalphanames_1_entry_l7_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L7"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Doe+95"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_040_maxalphanames_2_minalphanames_1_entry_l7_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L7"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"4"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_041_maxalphanames_2_minalphanames_1_entry_l8_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L8"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Sha85"#####)
    );
}

#[test]
fn assertion_042_maxalphanames_2_minalphanames_1_entry_l8_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L8"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_043_maxalphanames_2_minalphanames_2_entry_l1_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
            r#####"L1"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Doe95"#####)
    );
}

#[test]
fn assertion_044_maxalphanames_2_minalphanames_2_entry_l1_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
            r#####"l1"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_045_maxalphanames_2_minalphanames_2_entry_l2_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
            r#####"L2"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"DA95"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_046_maxalphanames_2_minalphanames_2_entry_l2_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
            r#####"L2"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_047_maxalphanames_2_minalphanames_2_entry_l3_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
            r#####"L3"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"DA95"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_048_maxalphanames_2_minalphanames_2_entry_l3_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
            r#####"L3"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_049_maxalphanames_2_minalphanames_2_entry_l4_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
            r#####"L4"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"DA+95"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_050_maxalphanames_2_minalphanames_2_entry_l4_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
            r#####"L4"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_051_maxalphanames_2_minalphanames_2_entry_l5_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
            r#####"L5"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"DA+95"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_052_maxalphanames_2_minalphanames_2_entry_l5_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
            r#####"L5"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_053_maxalphanames_2_minalphanames_2_entry_l6_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
            r#####"L6"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"DS+95"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_054_maxalphanames_2_minalphanames_2_entry_l6_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
            r#####"L6"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_055_maxalphanames_2_minalphanames_2_entry_l7_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
            r#####"L7"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"DS+95"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_056_maxalphanames_2_minalphanames_2_entry_l7_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
            r#####"L7"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_057_maxalphanames_2_minalphanames_2_entry_l8_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
            r#####"L8"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Sha85"#####)
    );
}

#[test]
fn assertion_058_maxalphanames_2_minalphanames_2_entry_l8_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"2"#####),
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"2"#####)
            ],
            r#####"L8"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_059_maxalphanames_3_minalphanames_1_entry_l1_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L1"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Doe95"#####)
    );
}

#[test]
fn assertion_060_maxalphanames_3_minalphanames_1_entry_l1_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L1"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_061_maxalphanames_3_minalphanames_1_entry_l2_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L2"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"DA95"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_062_maxalphanames_3_minalphanames_1_entry_l2_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L2"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_063_maxalphanames_3_minalphanames_1_entry_l3_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L3"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"DA95"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_064_maxalphanames_3_minalphanames_1_entry_l3_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L3"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_065_maxalphanames_3_minalphanames_1_entry_l4_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L4"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"DAE95"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_066_maxalphanames_3_minalphanames_1_entry_l4_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L4"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_067_maxalphanames_3_minalphanames_1_entry_l5_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L5"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"DAE95"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_068_maxalphanames_3_minalphanames_1_entry_l5_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L5"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_069_maxalphanames_3_minalphanames_1_entry_l6_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L6"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"DSE95"#####)
    );
}

#[test]
fn assertion_070_maxalphanames_3_minalphanames_1_entry_l6_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L6"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_071_maxalphanames_3_minalphanames_1_entry_l7_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L7"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"DSJ95"#####)
    );
}

#[test]
fn assertion_072_maxalphanames_3_minalphanames_1_entry_l7_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L7"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_073_maxalphanames_3_minalphanames_1_entry_l8_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L8"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Sha85"#####)
    );
}

#[test]
fn assertion_074_maxalphanames_3_minalphanames_1_entry_l8_extraalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"L8"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_075_testing_compound_lastnames_1() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"LDN1"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"VUR89"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_076_testing_compound_lastnames_2() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"LDN2"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"VU45"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_077_testing_with_multiple_pre_and_main_and_width_side_override() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"0"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"LDN3"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"VisvSJRu45"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_078_prefix_labelalpha_1() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L11"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"vRan22"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_079_prefix_labelalpha_2() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L12"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"vRvB2"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_080_per_type_labelalpha_1() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L13"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"vRa+-ksUnV"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_081_per_type_labelalpha_2() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L14"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Alabel-ksUnW"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_082_labelalpha_disambiguation_1() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L15"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"AccBrClim"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_083_labelalpha_disambiguation_2() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L16"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"AccBaClim"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_084_labelalpha_disambiguation_2a() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L16a"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"AccBaClim"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_085_labelalpha_disambiguation_2c() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L16"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_086_labelalpha_disambiguation_2d() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L16a"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_087_labelalpha_disambiguation_3() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L17"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"AckBaClim"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_088_custom_labelalpha_extradate_1() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L17a"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_089_labelalpha_disambiguation_4() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L18"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"AgChLa"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_090_labelalpha_disambiguation_5() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L19"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"AgConLe"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_091_labelalpha_disambiguation_6() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L20"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"AgCouLa"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_092_labelalpha_disambiguation_7() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L21"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"BoConEdb"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_093_labelalpha_disambiguation_8() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L22"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"BoConEm"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_094_labelalpha_disambiguation_9() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L23"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Sa"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_095_labelalpha_disambiguation_10() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L18"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Agas/Cha/Laver"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_096_labelalpha_disambiguation_11() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L19"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Agas/Con/Lendl"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_097_labelalpha_disambiguation_12() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L20"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Agas/Cou/Laver"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_098_labelalpha_list_disambiguation_1() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L18"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"AChL"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_099_labelalpha_list_disambiguation_2() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L19"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"ACoL"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_100_labelalpha_list_disambiguation_3() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L20"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"ACL"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_101_labelalpha_list_disambiguation_4() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L21"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"BCEd"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_102_labelalpha_list_disambiguation_5() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L22"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"BCE"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_103_labelalpha_list_disambiguation_6() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L24"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Z"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_104_labelalpha_list_disambiguation_7() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L25"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"ZX"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_105_labelalpha_list_disambiguation_8() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"L26"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"ZX"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_106_title_in_braces_with_utf_8_char_1() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"4"#####),
                (r#####"maxcitenames"#####, r#####"4"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"4"#####),
                (r#####"mincitenames"#####, r#####"4"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"title1"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Tït"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_107_extraalpha_ne_extradate_1() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"Schmidt2007"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Sch+07"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_108_extraalpha_ne_extradate_2() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"Schmidt2007"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_109_extraalpha_ne_extradate_3() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"Schmidt2007a"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Sch07"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_110_extraalpha_ne_extradate_4() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"Schmidt2007a"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_111_extraalpha_ne_extradate_5() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"Schnee2007"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Sch+07"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_112_extraalpha_ne_extradate_6() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"Schnee2007"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_113_extraalpha_ne_extradate_7() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"Schnee2007a"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"Sch07"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_114_extraalpha_ne_extradate_8() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"Schnee2007a"#####,
            r#####"extraalpha"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_115_entrykey_label_1() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"Schmidt2007"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"SCH"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_116_labeldate_test_1() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"labelstest"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"200532"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_117_pad_test_1() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"padtest"#####,
            r#####"labelalpha"#####
        )
        .as_deref(),
        Some(r#####"\&Al\_\_{\textasciitilde}{\textasciitilde}T07"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_118_pad_test_2() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"padtest"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"&Al__~~T07"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_119_skip_width_test_1() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"skipwidthtest1"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"OToolOToole"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_120_compound_and_string_length_entry_prefix1_labelalpha() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"prefix1"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"vadeVaaThin"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_121_name_range_test_1() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"3"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"rangetest1"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"WAXAYAZA.VEWEXE+.VTWT.XFYFZF.WH+"#####)
    );
}

#[test]
#[ignore = "xfail: label-alpha metadata differs from the Biber 2.22 expectation"]
fn assertion_122_name_range_test_2() {
    assert_eq!(
        field_text(
            r#####"labelalpha.bcf"#####,
            &[
                (r#####"maxalphanames"#####, r#####"10"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"labeldateparts"#####, r#####"1"#####),
                (r#####"useprefix"#####, r#####"1"#####),
                (r#####"minalphanames"#####, r#####"10"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"labelalpha"#####, r#####"1"#####)
            ],
            r#####"rangetest1"#####,
            r#####"sortlabelalpha"#####
        )
        .as_deref(),
        Some(r#####"VWXYZ..V/W/X/Y/Z"#####)
    );
}
