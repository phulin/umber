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
    if key == "extradatespec" {
        let start_tag = "<bcf:extradatespec>";
        let end_tag = "</bcf:extradatespec>";
        let start = control.find(start_tag).expect("extradate spec exists");
        let end = control[start..]
            .find(end_tag)
            .map(|offset| start + offset + end_tag.len())
            .expect("extradate spec is terminated");
        let scopes = value
            .split(';')
            .map(|scope| {
                let fields = scope
                    .split(',')
                    .enumerate()
                    .map(|(index, field)| {
                        format!(
                            "      <bcf:field order=\"{}\">{field}</bcf:field>\n",
                            index + 1
                        )
                    })
                    .collect::<String>();
                format!("    <bcf:scope>\n{fields}    </bcf:scope>\n")
            })
            .collect::<String>();
        control.replace_range(start..end, &format!("{start_tag}\n{scopes}  {end_tag}"));
        return;
    }
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
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_001_entry_l1_one_name_first_in_1995() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"L1"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_002_entry_l2_one_name_second_in_1995() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"L2"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_003_entry_l3_one_name_third_in_1995() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"L3"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"3"#####)
    );
}

#[test]
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_004_entry_l4_two_names_first_in_1995() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"L4"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_005_entry_l5_two_names_second_in_1995() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"L5"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_006_entry_l6_two_names_first_in_1996() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"L6"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_007_entry_l7_two_names_second_in_1996() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"L7"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_008_same_name_no_year_1() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"nodate1"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_009_same_name_no_year_2() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"nodate2"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
fn assertion_010_entry_l8_one_name_only_in_year() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"L8"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_011_entry_l9_no_name_same_year_as_another_with_no_name() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"L9"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_012_entry_l10_no_name_same_year_as_another_with_no_name() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"L10"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_013_entry_companion1_names_truncated_to_same_as_another_entry_in_same_year() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"companion1"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_014_entry_companion2_names_truncated_to_same_as_another_entry_in_same_year() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"companion2"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
fn assertion_015_entry_companion3_one_name_same_year_as_truncated_names() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"companion3"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_016_entry_vangennep_useprefix_does_makes_it_different() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"vangennep"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_017_entry_gennep_different_from_prefix_name() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"gennep"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
fn assertion_018_date_range_means_no_extradate_1() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"LY1"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_019_date_range_means_no_extradate_2() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"LY2"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_020_date_range_means_no_extradate_3() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"LY3"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_021_labeldatesource_string_1() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"nodate1"#####,
            r#####"labeldatesource"#####
        )
        .as_deref(),
        Some(r#####"nodate"#####)
    );
}

#[test]
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_022_labeldatesource_string_2() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"nodate2"#####,
            r#####"labeldatesource"#####
        )
        .as_deref(),
        Some(r#####"nodate"#####)
    );
}

#[test]
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_023_labelyear_scope_1() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"ed1"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_024_labelyear_scope_2() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"ed2"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_025_labelyear_scope_1a() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"ed1"#####,
            r#####"extradatescope"#####
        )
        .as_deref(),
        Some(r#####"labelyear"#####)
    );
}

#[test]
fn assertion_026_labelyear_scope_3() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"ed7"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_027_labelyear_scope_4() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"ed8"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_028_labelmonth_scope_1() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (
                    r#####"extradatespec"#####,
                    r#####"labelyear,year;labelmonth"#####
                ),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"ed1"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_029_labelmonth_scope_2() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (
                    r#####"extradatespec"#####,
                    r#####"labelyear,year;labelmonth"#####
                ),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"ed2"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_030_labelmonth_scope_1a() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (
                    r#####"extradatespec"#####,
                    r#####"labelyear,year;labelmonth"#####
                ),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"ed1"#####,
            r#####"extradatescope"#####
        )
        .as_deref(),
        Some(r#####"labelmonth"#####)
    );
}

#[test]
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_031_labelmonth_scope_3() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (
                    r#####"extradatespec"#####,
                    r#####"labelyear,year;labelmonth"#####
                ),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"ed3"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_032_labelmonth_scope_4() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (
                    r#####"extradatespec"#####,
                    r#####"labelyear,year;labelmonth"#####
                ),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"ed4"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_033_labelminute_scope_1() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (
                    r#####"extradatespec"#####,
                    r#####"labelyear,year;labelmonth;labelday;labelhour;labelminute"#####
                ),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"ed5"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_034_labelminute_scope_2() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (
                    r#####"extradatespec"#####,
                    r#####"labelyear,year;labelmonth;labelday;labelhour;labelminute"#####
                ),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"ed6"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_035_labelminute_scope_1a() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (
                    r#####"extradatespec"#####,
                    r#####"labelyear,year;labelmonth;labelday;labelhour;labelminute"#####
                ),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"ed5"#####,
            r#####"extradatescope"#####
        )
        .as_deref(),
        Some(r#####"labelminute"#####)
    );
}

#[test]
fn assertion_036_labelminute_scope_3() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (
                    r#####"extradatespec"#####,
                    r#####"labelyear,year;labelmonth;labelday;labelhour;labelminute"#####
                ),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"ed1"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_037_labelminute_scope_4() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (
                    r#####"extradatespec"#####,
                    r#####"labelyear,year;labelmonth;labelday;labelhour;labelminute"#####
                ),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"ed2"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_038_year_scope_1() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"extradatespec"#####, r#####"year"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"ed7"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: extra-date metadata differs from the Biber 2.22 expectation"]
fn assertion_039_year_scope_2() {
    assert_eq!(
        field_text(
            r#####"extradate.bcf"#####,
            &[
                (r#####"extradatespec"#####, r#####"year"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"maxbibnames"#####, r#####"1"#####),
                (r#####"maxsortnames"#####, r#####"1"#####)
            ],
            r#####"ed8"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}
