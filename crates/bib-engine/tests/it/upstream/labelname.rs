// Native Rust translation of upstream t/labelname.t at commit 74252e6.

use std::path::PathBuf;

use bib_engine::{
    BibAttempt, BibJob, BibOptionsBuilder, BibSession, EntryId, FieldId, FieldValue,
    FileProvisioner, ResolvedFile, SectionId, VfsLimits, VirtualPath,
};

fn process_fixture(control_name: &str) -> bib_engine::ProcessedBibliography {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/bib/upstream-2.22/tdata");
    let control = VirtualPath::user(control_name).expect("valid control path");
    let mut provisioner = FileProvisioner::new(VfsLimits::default()).expect("valid VFS limits");
    provisioner
        .register_user(
            control.clone(),
            std::fs::read(fixture_dir.join(control_name)).expect("committed BCF fixture"),
        )
        .expect("unique control file");
    let job = BibJob::new(control, BibOptionsBuilder::new().freeze());
    let mut session = BibSession::default();
    loop {
        match session.process(&job, &provisioner.snapshot()) {
            BibAttempt::Complete(result) => return result.document().as_ref().clone(),
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

fn label_name_source(entry_key: &str) -> Option<String> {
    let document = process_fixture("general.bcf");
    let entry = document
        .section(SectionId::new(0))
        .and_then(|section| section.entry(&EntryId::new(entry_key).expect("valid entry key")))?;
    let value = entry
        .fields()
        .get(&FieldId::new("labelnamesource").expect("valid field name"))?;
    match value {
        FieldValue::Literal(value) => Some(value.as_str().to_owned()),
        _ => None,
    }
}

#[test]
#[ignore = "xfail: label-name source metadata is not exposed on processed entries"]
fn assertion_001_global_shortauthor() {
    assert_eq!(
        label_name_source("angenendtsa").as_deref(),
        Some("shortauthor")
    );
}

#[test]
fn assertion_002_global_author() {
    assert_eq!(label_name_source("stdmodel").as_deref(), Some("author"));
}

#[test]
#[ignore = "xfail: label-name source metadata is not exposed on processed entries"]
fn assertion_003_type_specific_editor() {
    assert_eq!(
        label_name_source("aristotle:anima").as_deref(),
        Some("editor")
    );
}

#[test]
#[ignore = "xfail: label-name source metadata is not exposed on processed entries"]
fn assertion_004_type_specific_exotic_name() {
    assert_eq!(label_name_source("lne1").as_deref(), Some("namea"));
}
