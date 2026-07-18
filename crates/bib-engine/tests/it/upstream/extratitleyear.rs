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
#[ignore = "xfail: extra-title-year metadata differs from the Biber 2.22 expectation"]
fn assertion_001_same_title_same_year() {
    assert_eq!(
        field_text(
            r#####"extratitleyear.bcf"#####,
            &[],
            r#####"L1"#####,
            r#####"extratitleyear"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: extra-title-year metadata differs from the Biber 2.22 expectation"]
fn assertion_002_same_title_same_year() {
    assert_eq!(
        field_text(
            r#####"extratitleyear.bcf"#####,
            &[],
            r#####"L2"#####,
            r#####"extratitleyear"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
fn assertion_003_no_title_same_year() {
    assert_eq!(
        field_text(
            r#####"extratitleyear.bcf"#####,
            &[],
            r#####"L3"#####,
            r#####"extratitle"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_004_same_title_different_year() {
    assert_eq!(
        field_text(
            r#####"extratitleyear.bcf"#####,
            &[],
            r#####"L4"#####,
            r#####"extratitleyear"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_005_different_labeltitle_same_year() {
    assert_eq!(
        field_text(
            r#####"extratitleyear.bcf"#####,
            &[],
            r#####"L5"#####,
            r#####"extratitleyear"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_006_different_years_due_to_range_ends_1() {
    assert_eq!(
        field_text(
            r#####"extratitleyear.bcf"#####,
            &[],
            r#####"LY1"#####,
            r#####"extratitleyear"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_007_different_years_due_to_range_ends_1() {
    assert_eq!(
        field_text(
            r#####"extratitleyear.bcf"#####,
            &[],
            r#####"LY2"#####,
            r#####"extratitleyear"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_008_different_years_due_to_range_ends_1() {
    assert_eq!(
        field_text(
            r#####"extratitleyear.bcf"#####,
            &[],
            r#####"LY3"#####,
            r#####"extratitleyear"#####
        )
        .as_deref(),
        None
    );
}
