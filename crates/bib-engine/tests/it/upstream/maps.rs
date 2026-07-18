//! Native translations of upstream `t/maps.t` at commit 74252e6.

use std::fs;
use std::path::{Path, PathBuf};

use bib_engine::{
    BibAttempt, BibFailure, BibJob, BibOptionsBuilder, BibResult, BibSession, Entry, EntryId,
    FieldId, FieldValue, FileProvisioner, OutputFormat, OutputRequest, ResolvedFile, SectionId,
    VfsLimits, VirtualPath,
};

pub(super) fn run_fixture(stem: &str) -> BibResult {
    try_run_fixture(stem)
        .unwrap_or_else(|failure| panic!("native fixture processing failed: {failure:?}"))
}

pub(super) fn try_run_fixture(stem: &str) -> Result<BibResult, BibFailure> {
    let corpus =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/bib/upstream-2.22/tdata");
    let control_name = format!("{stem}.bcf");
    let control_path = VirtualPath::user(&control_name).expect("valid fixture control path");
    let mut provisioner = FileProvisioner::new(VfsLimits::default()).expect("fixture limits");
    provisioner
        .register_user(
            control_path.clone(),
            fs::read(corpus.join(&control_name)).expect("committed upstream control fixture"),
        )
        .expect("register control fixture");

    let output_name = format!("{stem}.bbl");
    let output_path = VirtualPath::user(&output_name).expect("valid output path");
    let mut options = BibOptionsBuilder::new();
    options
        .output(OutputRequest::new(output_path, OutputFormat::Bbl))
        .expect("unique output path");
    let job = BibJob::new(control_path, options.freeze());
    let mut session = BibSession::default();

    for _ in 0..16 {
        match session.process(&job, &provisioner.snapshot()) {
            BibAttempt::Complete(result) => return Ok(result),
            BibAttempt::NeedResources(batch) => {
                provisioner.expect(&batch);
                for request in &batch.required {
                    let fixture = fixture_for_request(&corpus, request.original_name());
                    provisioner
                        .provision(ResolvedFile {
                            request: request.key().clone(),
                            virtual_path: format!(
                                "/texlive/bib/{}",
                                fixture.file_name().unwrap().to_string_lossy()
                            ),
                            bytes: fs::read(&fixture).unwrap_or_else(|error| {
                                panic!("read requested fixture {}: {error}", fixture.display())
                            }),
                            expected_digest: None,
                        })
                        .expect("provision committed fixture");
                }
            }
            BibAttempt::Failed(failure) => return Err(failure),
        }
    }
    panic!("native fixture processing did not converge")
}

fn fixture_for_request(corpus: &Path, original_name: &str) -> PathBuf {
    let without_query = original_name.split('?').next().unwrap_or(original_name);
    let name = without_query
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(without_query);
    let direct = corpus.join(name);
    if direct.is_file() {
        return direct;
    }
    for extension in ["bib", "bcf", "conf", "dbx", "rnc", "bltxml"] {
        let candidate = corpus.join(format!("{name}.{extension}"));
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!("no committed fixture satisfies native request {original_name:?}")
}

pub(super) fn entry<'a>(result: &'a BibResult, section: u32, key: &str) -> Option<&'a Entry> {
    let id = EntryId::new(key).expect("valid upstream entry id");
    result
        .document()
        .section(SectionId::new(section))
        .and_then(|section| section.entry(&id))
}

pub(super) fn text_field<'a>(entry: &'a Entry, field: &str) -> Option<&'a str> {
    let id = FieldId::new(field).expect("valid upstream field id");
    match entry.fields().get(&id) {
        Some(FieldValue::Literal(value)) => Some(value.as_str()),
        Some(FieldValue::Verbatim(value)) => Some(value.as_str()),
        _ => None,
    }
}

pub(super) fn list_keys<'a>(result: &'a BibResult, section: u32, list_id: &str) -> Vec<&'a str> {
    result
        .document()
        .section(SectionId::new(section))
        .expect("upstream section")
        .lists()
        .find(|list| list.id().as_str() == list_id)
        .map(|list| list.entries().map(EntryId::as_str).collect())
        .unwrap_or_default()
}

pub(super) fn output_entry(result: &BibResult, key: &str) -> Option<String> {
    output_entry_nth(result, key, 0)
}

pub(super) fn output_text(result: &BibResult) -> &str {
    let bytes = result
        .files()
        .find(|file| file.path().as_str().ends_with(".bbl"))
        .expect("native BBL output")
        .bytes();
    std::str::from_utf8(bytes).expect("native BBL is UTF-8")
}

pub(super) fn output_entry_nth(result: &BibResult, key: &str, occurrence: usize) -> Option<String> {
    let bytes = result
        .files()
        .find(|file| file.path().as_str().ends_with(".bbl"))?
        .bytes();
    let output = std::str::from_utf8(bytes).expect("native BBL is UTF-8");
    let marker = format!("    \\entry{{{key}}}");
    let start = output.match_indices(&marker).nth(occurrence)?.0;
    let relative_end = output[start..].find("    \\endentry\n")?;
    let end = start + relative_end + "    \\endentry\n".len();
    Some(output[start..end].to_owned())
}

pub(super) fn section_entry_keys(result: &BibResult, section: u32) -> Vec<&str> {
    result
        .document()
        .section(SectionId::new(section))
        .expect("upstream section")
        .entries()
        .map(|entry| entry.id().as_str())
        .collect()
}

#[test]
fn assertion_001_maps_test_1() {
    let result = run_fixture("maps");
    assert!(entry(&result, 0, "maps1").is_some());
}

#[test]
#[ignore = "xfail: source-map deletion is not implemented by bib-engine"]
fn assertion_002_maps_test_2() {
    let result = run_fixture("maps");
    assert!(entry(&result, 0, "maps2").is_none());
}

#[test]
fn assertion_003_maps_test_3() {
    let result = run_fixture("maps");
    assert!(entry(&result, 0, "maps3").is_some());
}

#[test]
#[ignore = "xfail: source-map deletion is not implemented by bib-engine"]
fn assertion_004_maps_test_4() {
    let result = run_fixture("maps");
    assert!(entry(&result, 0, "maps4").is_none());
}

#[test]
#[ignore = "xfail: source-map field assignment is not implemented by bib-engine"]
fn assertion_005_maps_test_5() {
    let result = run_fixture("maps");
    let mapped = entry(&result, 0, "maps1").expect("explicitly cited entry");
    assert_eq!(text_field(mapped, "verba"), Some("somevalue"));
}

#[test]
fn assertion_006_maps_test_6() {
    let result = run_fixture("maps");
    let unmapped = entry(&result, 0, "maps3").expect("collection entry");
    assert_eq!(text_field(unmapped, "verba"), None);
}

#[test]
#[ignore = "xfail: source-map field assignment is not implemented by bib-engine"]
fn assertion_007_maps_test_7() {
    let result = run_fixture("maps");
    let mapped = entry(&result, 0, "maps1").expect("explicitly cited entry");
    assert_eq!(text_field(mapped, "verbb"), Some("somevalue1"));
}

#[test]
fn assertion_008_maps_test_8() {
    let result = run_fixture("maps");
    let unmapped = entry(&result, 0, "maps3").expect("collection entry");
    assert_eq!(text_field(unmapped, "verbb"), None);
}
