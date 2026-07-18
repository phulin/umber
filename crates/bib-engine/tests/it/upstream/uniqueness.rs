// Native Rust translation of the corresponding upstream Biber test at commit 74252e6.

use std::path::PathBuf;

use bib_engine::{
    BibAttempt, BibJob, BibOptionsBuilder, BibSession, EntryId, FieldId, FieldValue,
    FileProvisioner, OutputFormat, OutputRequest, ProcessedBibliography, ResolvedFile, SectionId,
    VfsLimits, VirtualPath,
};

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
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_001_uniquename_requiring_full_name_expansion_1() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un1"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_002_uniquename_requiring_full_name_expansion_2() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un2"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_003_uniquename_requiring_full_name_expansion_3() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un5"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
fn assertion_004_uniquename_requiring_initials_name_expansion_per_namelist_uniquename_1() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un3"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_005_uniquename_requiring_initials_name_expansion_2() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un4"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
fn assertion_006_per_entry_uniquename() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un4a"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_007_namehash_and_fullhash_1() {
    assert_eq!(
        field_text(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un6"#####,
            r#####"namehash"#####
        )
        .as_deref(),
        Some(r#####"f8169a157f8d9209961157b8d23902db"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_008_namehash_and_fullhash_2() {
    assert_eq!(
        field_text(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un6"#####,
            r#####"fullhash"#####
        )
        .as_deref(),
        Some(r#####"f8169a157f8d9209961157b8d23902db"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_009_fullnamehash_ignores_short_names_1() {
    assert_eq!(
        field_text(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un7"#####,
            r#####"namehash"#####
        )
        .as_deref(),
        Some(r#####"b33fbd3f3349d1536dbcc14664f2cbbd"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_010_fullnamehash_ignores_short_names_2() {
    assert_eq!(
        field_text(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un7"#####,
            r#####"fullhash"#####
        )
        .as_deref(),
        Some(r#####"f8169a157f8d9209961157b8d23902db"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_011_namehash_and_fullhash_3() {
    assert_eq!(
        field_text(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"test1"#####,
            r#####"namehash"#####
        )
        .as_deref(),
        Some(r#####"07df5c892ba1452776abee0a867591f2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_012_namehash_and_fullhash_4() {
    assert_eq!(
        field_text(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"test1"#####,
            r#####"fullhash"#####
        )
        .as_deref(),
        Some(r#####"637292dd2997a74c91847f1ec5081a46"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_013_uniquename_with_full_and_repeat_1() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"untf1"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_014_uniquename_with_full_and_repeat_2() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"untf2"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_015_uniquename_with_full_and_repeat_3() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"untf3"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_016_prefix_suffix_1() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"sp1"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_017_prefix_suffix_2() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"sp2"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_018_prefix_suffix_3() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"sp3"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_019_prefix_suffix_4() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"sp4"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_020_prefix_suffix_5() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"sp5"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_021_prefix_suffix_6() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"sp6"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_022_prefix_suffix_7() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"sp7"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_023_prefix_suffix_8() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"sp8"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_024_prefix_suffix_9() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"sp9"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_025_uniquename_with_inits_and_repeat_1() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"unt1"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_026_uniquename_with_inits_and_repeat_2() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"unt2"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_027_uniquename_with_inits_and_repeat_3() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"unt3"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_028_uniquename_with_inits_and_repeat_4() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"unt4"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_029_uniquename_with_inits_and_repeat_5() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"unt5"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_030_namehash_and_fullhash_5() {
    assert_eq!(
        field_text(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"5"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"unall3"#####,
            r#####"namehash"#####
        )
        .as_deref(),
        Some(r#####"f1c5973adbc2e674fa4d98164c9ba5d5"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_031_namehash_and_fullhash_6() {
    assert_eq!(
        field_text(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"5"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"unall3"#####,
            r#####"fullhash"#####
        )
        .as_deref(),
        Some(r#####"f1c5973adbc2e674fa4d98164c9ba5d5"#####)
    );
}

#[test]
fn assertion_032_uniquelist_edgecase_1() {
    assert_eq!(
        field_text(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"5"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"unall3"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_033_uniquelist_edgecase_2() {
    assert_eq!(
        field_text(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"5"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"unall4"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"6"#####)
    );
}

#[test]
fn assertion_034_uniquename_0_due_to_mincitenames_truncation() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"false"#####)
            ],
            r#####"test2"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_035_uniquename_1() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"un8"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_036_uniquename_2() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"un8"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_037_uniquename_3() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"un8"#####,
            3,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_038_uniquename_4() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"un9"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_039_uniquename_5() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"un9"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_040_uniquename_6() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"un9"#####,
            3,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_041_uniquename_7() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"un9"#####,
            4,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_042_uniquename_8() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"un10"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_043_uniquelist_1() {
    assert_eq!(
        field_text(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"un8"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"3"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_044_uniquelist_2() {
    assert_eq!(
        field_text(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"un9"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"3"#####)
    );
}

#[test]
fn assertion_045_uniquelist_3() {
    assert_eq!(
        field_text(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"un10"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_046_uniquelist_4() {
    assert_eq!(
        field_text(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"unapa1"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"3"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_047_uniquelist_5() {
    assert_eq!(
        field_text(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"unapa2"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"3"#####)
    );
}

#[test]
fn assertion_048_uniquelist_6() {
    assert_eq!(
        field_text(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"others1"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_049_uniquelist_7() {
    assert_eq!(
        field_text(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"unall1"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_050_uniquelist_8() {
    assert_eq!(
        field_text(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"unall2"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_051_uniquelist_9() {
    assert_eq!(
        field_text(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"unall5"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"5"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_052_uniquelist_10() {
    assert_eq!(
        field_text(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"unall6"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"5"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_053_uniquelist_11() {
    assert_eq!(
        field_text(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"unall7"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"5"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_054_uniquelist_12() {
    assert_eq!(
        field_text(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"unall8"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"5"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_055_uniquelist_13() {
    assert_eq!(
        field_text(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"unall9"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"5"#####)
    );
}

#[test]
fn assertion_056_per_namelist_uniquelist_1() {
    assert_eq!(
        field_text(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"unall9a"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_057_uniquelist_14() {
    assert_eq!(
        field_text(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"unall10"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"6"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_058_uniquelist_15() {
    assert_eq!(
        field_text(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"unall3"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"5"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_059_uniquelist_16() {
    assert_eq!(
        field_text(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"unall4"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"6"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_060_uniquelist_17() {
    assert_eq!(
        field_text(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"ul01"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"3"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_061_uniquelist_18() {
    assert_eq!(
        field_text(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"ul02"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"3"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_062_uniquelist_19() {
    assert_eq!(
        field_text(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"test3"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_063_uniquename_9() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"test3"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_064_uniquename_10() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"test3"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_065_uniquelist_20() {
    assert_eq!(
        field_text(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"test4"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_066_uniquename_11() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"test4"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_067_uniquename_12() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"test4"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_068_uniquelist_21() {
    assert_eq!(
        field_text(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"test5"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_069_uniquename_13() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"test5"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_070_uniquename_14() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness1.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"test5"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_071_uniquename_sparse_1() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us1"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_072_uniquename_sparse_2() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us1"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_073_uniquename_sparse_3() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us2"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_074_uniquename_sparse_4() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us2"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_075_uniquename_sparse_5() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us3"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_076_uniquename_sparse_6() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us3"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_077_uniquename_sparse_7() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us4"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_078_uniquename_sparse_8() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us4"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_079_uniquename_sparse_9() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us5"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_080_uniquename_sparse_10() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us6"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_081_uniquename_sparse_11() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us6"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_082_uniquename_sparse_12() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us6"#####,
            3,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_083_uniquename_sparse_13() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us7"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_084_uniquename_sparse_14() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us7"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_085_uniquename_sparse_15() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us8"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_086_uniquename_sparse_16() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us8"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_087_uniquename_sparse_17() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us9"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_088_uniquename_sparse_18() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us9"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_089_uniquename_sparse_19() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us10"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_090_uniquename_sparse_20() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us10"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_091_uniquename_sparse_21() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us11"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_092_uniquename_sparse_22() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us11"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_093_uniquename_sparse_23() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us12"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_094_uniquename_sparse_24() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us12"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_095_uniquename_sparse_25() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us13"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_096_uniquename_sparse_26() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us13"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_097_uniquename_sparse_27() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us14"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_098_uniquename_sparse_28() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us14"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_099_uniquename_sparse_29() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us14"#####,
            3,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_100_uniquename_sparse_30() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us15"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_101_uniquename_sparse_31() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us15"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_102_uniquename_sparse_32() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us15"#####,
            3,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_103_uniquename_sparse_33() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us16"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_104_uniquename_sparse_34() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us16"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_105_uniquename_sparse_35() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us16"#####,
            3,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
fn assertion_106_uniquename_sparse_36() {
    assert_eq!(
        field_text(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us16"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_107_uniquename_sparse_37() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us17"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_108_uniquename_sparse_38() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us17"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_109_uniquename_sparse_39() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us17"#####,
            3,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_110_uniquename_sparse_40() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us17"#####,
            4,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_111_uniquename_sparse_41() {
    assert_eq!(
        field_text(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us17"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"4"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_112_uniquename_sparse_42() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us18"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_113_uniquename_sparse_43() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us19"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
fn assertion_114_uniquename_sparse_44() {
    assert_eq!(
        field_text(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us18"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_115_uniquename_sparse_45() {
    assert_eq!(
        field_text(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us19"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"4"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_116_uniquename_sparse_46() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us14"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_117_uniquename_sparse_47() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us14"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_118_uniquename_sparse_48() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us14"#####,
            3,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_119_uniquename_sparse_49() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us15"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_120_uniquename_sparse_50() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us15"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_121_uniquename_sparse_51() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us15"#####,
            3,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_122_uniquename_sparse_52() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us20"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_123_uniquename_sparse_53() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us21"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_124_uniquename_sparse_54() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us22"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_125_uniquename_sparse_55() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us23"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_126_uniquename_sparse_56() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us24"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_127_uniquename_sparse_57() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us25"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_128_uniquename_sparse_58() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us14"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_129_uniquename_sparse_59() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us14"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_130_uniquename_sparse_60() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us14"#####,
            3,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_131_uniquename_sparse_61() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us15"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_132_uniquename_sparse_62() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us15"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_133_uniquename_sparse_63() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us15"#####,
            3,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_134_uniquename_sparse_64() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us26"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_135_uniquename_sparse_65() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us27"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_136_uniquename_sparse_66() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us28"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_137_uniquename_sparse_67() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us29"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_138_uniquename_sparse_68() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness4.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"2"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"minfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"us30"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
fn assertion_139_uniquelist_strict_1() {
    assert_eq!(
        field_text(
            r#####"uniqueness5.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"uls1"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_140_uniquelist_strict_2() {
    assert_eq!(
        field_text(
            r#####"uniqueness5.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"uls2"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_141_uniquelist_strict_3() {
    assert_eq!(
        field_text(
            r#####"uniqueness5.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"uls3"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_142_uniquelist_strict_4() {
    assert_eq!(
        field_text(
            r#####"uniqueness5.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"uls4"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_143_uniquelist_strict_5() {
    assert_eq!(
        field_text(
            r#####"uniqueness5.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"uls5"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_144_uniquelist_strict_6() {
    assert_eq!(
        field_text(
            r#####"uniqueness5.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"uls6"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
fn assertion_145_uniquelist_strict_7() {
    assert_eq!(
        field_text(
            r#####"uniqueness5.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"1"#####),
                (r#####"mincitenames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"uls7"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_146_uniquelist_minyear_1() {
    assert_eq!(
        field_text(
            r#####"uniqueness5.bcf"#####,
            &[
                (r#####"maxnames"#####, r#####"3"#####),
                (r#####"minnames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"labeldateparts"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"ulmy1"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_147_uniquelist_minyear_2() {
    assert_eq!(
        field_text(
            r#####"uniqueness5.bcf"#####,
            &[
                (r#####"maxnames"#####, r#####"3"#####),
                (r#####"minnames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"labeldateparts"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"ulmy2"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_148_uniquelist_minyear_3() {
    assert_eq!(
        field_text(
            r#####"uniqueness5.bcf"#####,
            &[
                (r#####"maxnames"#####, r#####"3"#####),
                (r#####"minnames"#####, r#####"1"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"labeldateparts"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"ulmy3"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_149_uniquelist_strict_8() {
    assert_eq!(
        field_text(
            r#####"uniqueness5.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"2"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"uls8"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_150_uniquelist_strict_9() {
    assert_eq!(
        field_text(
            r#####"uniqueness5.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"2"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"uls9"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_151_uniquelist_strict_10() {
    assert_eq!(
        field_text(
            r#####"uniqueness5.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"2"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"uls1"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_152_uniquelist_strict_11() {
    assert_eq!(
        field_text(
            r#####"uniqueness5.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"2"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"uls10"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"3"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_153_uniquelist_strict_12() {
    assert_eq!(
        field_text(
            r#####"uniqueness5.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"2"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"uls11"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"3"#####)
    );
}

#[test]
fn assertion_154_uniquelist_strict_13() {
    assert_eq!(
        field_text(
            r#####"uniqueness5.bcf"#####,
            &[
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"2"#####),
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"minyear"#####),
                (r#####"singletitle"#####, r#####"0"#####)
            ],
            r#####"uls12"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_155_extrayear_1() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"singletitle"#####, r#####"1"#####)
            ],
            r#####"ey1"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_156_extrayear_2() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"singletitle"#####, r#####"1"#####)
            ],
            r#####"ey2"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_157_extrayear_3() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"singletitle"#####, r#####"1"#####)
            ],
            r#####"ey3"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_158_extrayear_4() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"singletitle"#####, r#####"1"#####)
            ],
            r#####"ey4"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_159_extrayear_5() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"singletitle"#####, r#####"1"#####)
            ],
            r#####"ey5"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_160_extrayear_6() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"singletitle"#####, r#####"1"#####)
            ],
            r#####"ey6"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
fn assertion_161_extrayear_7() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey1"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_162_extrayear_8() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey2"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_163_extrayear_9() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey3"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_164_extrayear_10() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey4"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
fn assertion_165_extrayear_11() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey5"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_166_extrayear_12() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey6"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_167_singletitle_1() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey1"#####,
            r#####"singletitle"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_168_singletitle_2() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey2"#####,
            r#####"singletitle"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
fn assertion_169_singletitle_3() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey3"#####,
            r#####"singletitle"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_170_singletitle_4() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey4"#####,
            r#####"singletitle"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_171_singletitle_5() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey5"#####,
            r#####"singletitle"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_172_singletitle_6() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey6"#####,
            r#####"singletitle"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
fn assertion_173_uniquetitle_1() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey1"#####,
            r#####"uniquetitle"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_174_uniquetitle_2() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey2"#####,
            r#####"uniquetitle"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
fn assertion_175_uniquetitle_3() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey3"#####,
            r#####"uniquetitle"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_176_uniquetitle_4() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey4"#####,
            r#####"uniquetitle"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
fn assertion_177_uniquetitle_5() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey5"#####,
            r#####"uniquetitle"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_178_uniquetitle_6() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey6"#####,
            r#####"uniquetitle"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
fn assertion_179_uniquebaretitle_1() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey7"#####,
            r#####"uniquebaretitle"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_180_uniquebaretitle_2() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey8"#####,
            r#####"uniquebaretitle"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_181_uniquebaretitle_3() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey9"#####,
            r#####"uniquebaretitle"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
fn assertion_182_uniquework_1() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey1"#####,
            r#####"uniquework"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_183_uniquework_2() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey2"#####,
            r#####"uniquework"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_184_uniquework_3() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey3"#####,
            r#####"uniquework"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_185_uniquework_4() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey4"#####,
            r#####"uniquework"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_186_uniquework_5() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey5"#####,
            r#####"uniquework"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_187_uniquework_6() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"full"#####),
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"1"#####),
                (r#####"uniquebaretitle"#####, r#####"1"#####),
                (r#####"uniquework"#####, r#####"1"#####)
            ],
            r#####"ey6"#####,
            r#####"uniquework"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_188_extrayear_13() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"false"#####),
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"0"#####),
                (r#####"uniquework"#####, r#####"0"#####)
            ],
            r#####"ey1"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_189_extrayear_14() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"false"#####),
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"0"#####),
                (r#####"uniquework"#####, r#####"0"#####)
            ],
            r#####"ey2"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_190_extrayear_15() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"false"#####),
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"0"#####),
                (r#####"uniquework"#####, r#####"0"#####)
            ],
            r#####"ey3"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_191_extrayear_16() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"false"#####),
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"0"#####),
                (r#####"uniquework"#####, r#####"0"#####)
            ],
            r#####"ey4"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_192_extrayear_17() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"false"#####),
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"0"#####),
                (r#####"uniquework"#####, r#####"0"#####)
            ],
            r#####"ey5"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_193_extrayear_18() {
    assert_eq!(
        field_text(
            r#####"uniqueness3.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"false"#####),
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"singletitle"#####, r#####"1"#####),
                (r#####"uniquetitle"#####, r#####"0"#####),
                (r#####"uniquework"#####, r#####"0"#####)
            ],
            r#####"ey6"#####,
            r#####"extradate"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_194_forced_init_expansion_1() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"allinit"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un8"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_195_forced_init_expansion_2() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"allinit"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un8"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_196_forced_init_expansion_3() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"allinit"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un8"#####,
            3,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_197_forced_init_expansion_4() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"allinit"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un9"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_198_forced_init_expansion_5() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"allinit"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un9"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_199_forced_init_expansion_6() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"allinit"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un9"#####,
            3,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_200_forced_init_expansion_7() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"allinit"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un9"#####,
            4,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_201_forced_init_expansion_8() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"allinit"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un10"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_202_forced_name_expansion_1() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"allfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un8"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_203_forced_name_expansion_2() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"allfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un8"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_204_forced_name_expansion_3() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"allfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un8"#####,
            3,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_205_forced_name_expansion_4() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"allfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un9"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_206_forced_name_expansion_5() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"allfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un9"#####,
            2,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_207_forced_name_expansion_6() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"allfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un9"#####,
            3,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_208_forced_name_expansion_7() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"allfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un9"#####,
            4,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_209_forced_name_expansion_8() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness2.bcf"#####,
            &[
                (r#####"uniquename"#####, r#####"allfull"#####),
                (r#####"uniquelist"#####, r#####"true"#####)
            ],
            r#####"un10"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_210_uniquelist_duplicates_1() {
    assert_eq!(
        field_text(
            r#####"uniqueness6.bcf"#####,
            &[(r#####"uniquelist"#####, r#####"true"#####)],
            r#####"entry1a"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_211_uniquelist_duplicates_2() {
    assert_eq!(
        field_text(
            r#####"uniqueness6.bcf"#####,
            &[(r#####"uniquelist"#####, r#####"true"#####)],
            r#####"entry1b"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_212_uniquelist_duplicates_3() {
    assert_eq!(
        field_text(
            r#####"uniqueness6.bcf"#####,
            &[(r#####"uniquelist"#####, r#####"true"#####)],
            r#####"entry2a"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_213_uniquelist_duplicates_4() {
    assert_eq!(
        field_text(
            r#####"uniqueness6.bcf"#####,
            &[(r#####"uniquelist"#####, r#####"true"#####)],
            r#####"entry2b"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_214_uniquelist_duplicates_5() {
    assert_eq!(
        field_text(
            r#####"uniqueness6.bcf"#####,
            &[(r#####"uniquelist"#####, r#####"true"#####)],
            r#####"A"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_215_uniquelist_duplicates_6() {
    assert_eq!(
        field_text(
            r#####"uniqueness6.bcf"#####,
            &[(r#####"uniquelist"#####, r#####"true"#####)],
            r#####"B"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_216_uniquelist_duplicates_7() {
    assert_eq!(
        field_text(
            r#####"uniqueness6.bcf"#####,
            &[(r#####"uniquelist"#####, r#####"true"#####)],
            r#####"C"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_217_uniquelist_true_uniquename_false_1() {
    assert_eq!(
        field_text(
            r#####"uniqueness6.bcf"#####,
            &[
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"uniquename"#####, r#####"false"#####)
            ],
            r#####"C"#####,
            r#####"uniquelist"#####
        )
        .as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_218_pluralothers_test_1() {
    assert_eq!(
        field_text(
            r#####"uniqueness7.bcf"#####,
            &[
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"pluralothers"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####)
            ],
            r#####"po1"#####,
            r#####"visiblecite"#####
        )
        .as_deref(),
        Some(r#####"4"#####)
    );
}

#[test]
fn assertion_219_pluralothers_test_2() {
    assert_eq!(
        field_text(
            r#####"uniqueness7.bcf"#####,
            &[
                (r#####"uniquelist"#####, r#####"false"#####),
                (r#####"pluralothers"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"3"#####)
            ],
            r#####"po1"#####,
            r#####"extraname"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_220_pluralothers_test_3() {
    assert_eq!(
        field_text(
            r#####"uniqueness7.bcf"#####,
            &[
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"pluralothers"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"po3"#####,
            r#####"visiblecite"#####
        )
        .as_deref(),
        Some(r#####"4"#####)
    );
}

#[test]
fn assertion_221_pluralothers_test_4() {
    assert_eq!(
        field_text(
            r#####"uniqueness7.bcf"#####,
            &[
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"pluralothers"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"po3"#####,
            r#####"extraname"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_222_pluralothers_test_5() {
    assert_eq!(
        output_entry(
            r#####"uniqueness7.bcf"#####,
            &[
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"uniquename"#####, r#####"init"#####),
                (r#####"pluralothers"#####, r#####"true"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"po3"#####
        ),
        r#####"    \entry{po3}{book}{}{}
      \name{author}{4}{ul=4}{%
        {{un=1,uniquepart=given,hash=c2ab7e2b5663336cc4e65c8bcf1a280d}{%
           family={Abraham},
           familyi={A\bibinitperiod},
           given={A.},
           giveni={A\bibinitperiod},
           givenun=1}}%
        {{un=0,uniquepart=base,hash=1f4cf713d86f6083087eb3085db7815a}{%
           family={Brown},
           familyi={B\bibinitperiod},
           given={B.},
           giveni={B\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=a44def9031aa70c9f458f5b47a34c451}{%
           family={Cuthbert},
           familyi={C\bibinitperiod},
           given={C.},
           giveni={C\bibinitperiod},
           givenun=0}}%
        {{un=1,uniquepart=given,hash=91876a448dc35952ca94dc92cee07f89}{%
           family={Abraham},
           familyi={A\bibinitperiod},
           given={D.},
           giveni={D\bibinitperiod},
           givenun=1}}%
      }
      \strng{namehash}{2f43c72e4c15c6ba3f24e7b6462e60ed}
      \strng{fullhash}{2f43c72e4c15c6ba3f24e7b6462e60ed}
      \strng{fullhashraw}{2f43c72e4c15c6ba3f24e7b6462e60ed}
      \strng{bibnamehash}{2f43c72e4c15c6ba3f24e7b6462e60ed}
      \strng{authorbibnamehash}{2f43c72e4c15c6ba3f24e7b6462e60ed}
      \strng{authornamehash}{2f43c72e4c15c6ba3f24e7b6462e60ed}
      \strng{authorfullhash}{2f43c72e4c15c6ba3f24e7b6462e60ed}
      \strng{authorfullhashraw}{2f43c72e4c15c6ba3f24e7b6462e60ed}
      \field{labelalpha}{Abr\textbf{+}22}
      \field{sortinit}{A}
      \field{sortinithash}{2f401846e2029bad6b3ecc16d50031e2}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{extraalpha}{1}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Title One}
      \field{year}{2022}
      \field{dateera}{ce}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_223_uniquename_minyearinit_1() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness7.bcf"#####,
            &[
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"uniquename"#####, r#####"minyearinit"#####),
                (r#####"pluralothers"#####, r#####"false"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"un1"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_224_uniquename_minyearinit_2() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness7.bcf"#####,
            &[
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"uniquename"#####, r#####"minyearinit"#####),
                (r#####"pluralothers"#####, r#####"false"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"un2"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_225_uniquename_minyearinit_3() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness7.bcf"#####,
            &[
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"uniquename"#####, r#####"minyearinit"#####),
                (r#####"pluralothers"#####, r#####"false"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"un3"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"0"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_226_uniquename_minyearinit_4() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness7.bcf"#####,
            &[
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"uniquename"#####, r#####"minyearinit"#####),
                (r#####"pluralothers"#####, r#####"false"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"un4"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: uniqueness metadata differs from the Biber 2.22 expectation"]
fn assertion_227_uniquename_minyearinit_5() {
    assert_eq!(
        name_assignment(
            r#####"uniqueness7.bcf"#####,
            &[
                (r#####"uniquelist"#####, r#####"true"#####),
                (r#####"uniquename"#####, r#####"minyearinit"#####),
                (r#####"pluralothers"#####, r#####"false"#####),
                (r#####"maxcitenames"#####, r#####"3"#####),
                (r#####"mincitenames"#####, r#####"1"#####)
            ],
            r#####"un5"#####,
            1,
            r#####"un"#####
        )
        .as_deref(),
        Some(r#####"1"#####)
    );
}
