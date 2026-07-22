use bib_model::{FieldId, FieldValue, RangeEndpoint};
use umber_vfs::{
    FileProvisioner, FileRequest, FileRequestBatch, ResolvedFile, VfsLimits, VirtualPath,
};

use super::*;
use crate::{BibOptionsBuilder, BibResult, OutputFormat, OutputRequest};

const CONTROL: &[u8] = br#"<bcf:controlfile version="3.11" bltxversion="3.21" xmlns:bcf="https://sourceforge.net/projects/biblatex">
  <bcf:bibdata section="0"><bcf:datasource type="file" datatype="bibtex">https://example.test/data.bib</bcf:datasource></bcf:bibdata>
  <bcf:section number="0"><bcf:citekey>entry</bcf:citekey></bcf:section>
</bcf:controlfile>"#;
const DATA: &[u8] = br#"@article{entry,
  author = {Ada Lovelace},
  date = {1843-03},
  url = {https://example.test/q},
  pages = {23--24, M-1--M-4}
}"#;

#[test]
fn requests_remote_resources_resumes_and_exposes_typed_values() {
    let mut provisioner = FileProvisioner::new(VfsLimits::default()).expect("limits");
    provisioner
        .register_user(
            VirtualPath::user("main.bcf").expect("path"),
            CONTROL.to_vec(),
        )
        .expect("control");
    let output_path = VirtualPath::user("main.bbl").expect("path");
    let mut options = BibOptionsBuilder::new();
    options
        .output(OutputRequest::new(output_path.clone(), OutputFormat::Bbl))
        .expect("output");
    let job = BibJob::new(
        VirtualPath::user("main.bcf").expect("path"),
        options.freeze(),
    );
    let mut session = BibSession::default();
    let needs = match session.process(&job, &provisioner.snapshot()) {
        BibAttempt::NeedResources(needs) => needs,
        attempt => panic!("expected data request, got {attempt:?}"),
    };
    assert_eq!(needs.required.len(), 1);
    assert_eq!(
        needs.required[0].original_name(),
        "https://example.test/data.bib"
    );
    provisioner.expect(&needs);
    provisioner
        .provision(ResolvedFile {
            request: needs.required[0].key().clone(),
            virtual_path: "/texlive/bib/data.bib".into(),
            bytes: DATA.to_vec(),
            expected_digest: None,
        })
        .expect("remote data");
    let result = complete(session.process(&job, &provisioner.snapshot()));
    assert_eq!(result.files().len(), 1);
    assert!(
        provisioner
            .snapshot()
            .get(&output_path)
            .expect("snapshot")
            .is_none(),
        "session outputs remain detached from the VFS"
    );
    let section = result
        .document()
        .section(SectionId::new(0))
        .expect("section");
    let entry = section
        .entry(&EntryId::new("entry").expect("id"))
        .expect("entry");
    assert!(matches!(
        entry.fields().get(&FieldId::new("month").expect("field")),
        Some(FieldValue::Integer(3))
    ));
    assert!(matches!(
        entry.fields().get(&FieldId::new("url").expect("field")),
        Some(FieldValue::UriList(values)) if values[0].as_str() == "https://example.test/q"
    ));
    assert!(matches!(
        entry
            .fields()
            .get(&FieldId::new("pages").expect("field")),
        Some(FieldValue::RangeList(values))
            if matches!(values[0].start(), RangeEndpoint::Integer(23))
                && matches!(values[0].end(), RangeEndpoint::Integer(24))
                && matches!(values[1].start(), RangeEndpoint::Literal(value) if value.as_str() == "M-1")
    ));
    assert_eq!(
        session.accepted_inputs(),
        &[
            crate::BibliographyInput::new(
                VirtualPath::user("main.bcf").expect("control path"),
                FileKind::BibControl,
            ),
            crate::BibliographyInput::new(
                VirtualPath::distribution("/texlive/bib/data.bib").expect("data path"),
                FileKind::BibData,
            ),
        ]
    );
    complete(session.process(&job, &provisioner.snapshot()));
    assert_eq!(
        session.accepted_inputs().len(),
        2,
        "cache reuse retains inputs"
    );
}

#[test]
fn unchanged_missing_batch_is_typed_no_progress() {
    let provisioner = FileProvisioner::new(VfsLimits::default()).expect("limits");
    let job = BibJob::new(
        VirtualPath::user("missing.bcf").expect("path"),
        BibOptionsBuilder::new().freeze(),
    );
    let snapshot = provisioner.snapshot();
    let mut session = BibSession::default();
    assert!(matches!(
        session.process(&job, &snapshot),
        BibAttempt::NeedResources(_)
    ));
    assert!(matches!(
        session.process(&job, &snapshot),
        BibAttempt::Failed(failure) if failure.kind() == BibFailureKind::NoProgress
    ));
}

#[test]
fn cache_disabled_and_enabled_results_are_identical() {
    let mut provisioner = FileProvisioner::new(VfsLimits::default()).expect("limits");
    provisioner
        .register_user(
            VirtualPath::user("main.bcf").expect("path"),
            CONTROL.to_vec(),
        )
        .expect("control");
    let key = request_key(FileKind::BibData, "https://example.test/data.bib");
    provisioner
        .preload(ResolvedFile {
            request: key,
            virtual_path: "/texlive/bib/data.bib".into(),
            bytes: DATA.to_vec(),
            expected_digest: None,
        })
        .expect("data");
    let job = BibJob::new(
        VirtualPath::user("main.bcf").expect("path"),
        BibOptionsBuilder::new().freeze(),
    );
    let snapshot = provisioner.snapshot();
    let mut cached =
        BibSession::new(BibSessionOptions::default().with_cache_entries(1)).expect("session");
    let mut cold = BibSession::new(BibSessionOptions::default().without_caches()).expect("session");
    let cached = complete(cached.process(&job, &snapshot));
    let cold = complete(cold.process(&job, &snapshot));
    assert_eq!(cached, cold);
}

#[test]
fn response_permutation_and_chunking_do_not_change_results() {
    let control_path = VirtualPath::user("main.bcf").expect("path");
    let config_path = VirtualPath::user("settings.conf").expect("path");
    let schema_path = VirtualPath::user("control.rnc").expect("path");
    let mut options = BibOptionsBuilder::new();
    options
        .configuration(config_path)
        .schema(schema_path)
        .expect("schema");
    let job = BibJob::new(control_path.clone(), options.freeze());

    let mut chunked = FileProvisioner::new(VfsLimits::default()).expect("limits");
    chunked
        .register_user(control_path.clone(), CONTROL.to_vec())
        .expect("control");
    let mut session = BibSession::default();
    let first = needs(session.process(&job, &chunked.snapshot()));
    assert_eq!(first.required.len(), 3);
    chunked.expect(&first);
    let schema = first
        .required
        .iter()
        .find(|request| request.key().kind() == FileKind::XmlSchema)
        .expect("schema request");
    chunked
        .provision(response(schema, "/texlive/schema/control.rnc", b"schema"))
        .expect("schema chunk");
    let second = needs(session.process(&job, &chunked.snapshot()));
    assert_eq!(second.required.len(), 2);
    chunked.expect(&second);
    let responses = second
        .required
        .iter()
        .rev()
        .map(|request| match request.key().kind() {
            FileKind::BibConfiguration => {
                response(request, "/texlive/bib/settings.conf", b"<config/>")
            }
            FileKind::BibData => response(request, "/texlive/bib/data.bib", DATA),
            kind => panic!("unexpected kind {kind:?}"),
        })
        .collect::<Vec<_>>();
    chunked.provision_batch(responses).expect("remaining chunk");
    let chunked_result = complete(session.process(&job, &chunked.snapshot()));

    let mut permuted = FileProvisioner::new(VfsLimits::default()).expect("limits");
    permuted
        .register_user(control_path, CONTROL.to_vec())
        .expect("control");
    for request in first.required.iter().rev() {
        let response = match request.key().kind() {
            FileKind::BibConfiguration => {
                response(request, "/texlive/bib/settings.conf", b"<config/>")
            }
            FileKind::XmlSchema => response(request, "/texlive/schema/control.rnc", b"schema"),
            FileKind::BibData => response(request, "/texlive/bib/data.bib", DATA),
            kind => panic!("unexpected kind {kind:?}"),
        };
        permuted.preload(response).expect("preload");
    }
    let permuted_result = complete(BibSession::default().process(&job, &permuted.snapshot()));
    assert_eq!(chunked_result, permuted_result);
}

#[test]
fn stale_snapshot_is_a_typed_resource_conflict() {
    let provisioner = FileProvisioner::new(VfsLimits::default()).expect("limits");
    let snapshot = provisioner.snapshot();
    snapshot.invalidate();
    let job = BibJob::new(
        VirtualPath::user("main.bcf").expect("path"),
        BibOptionsBuilder::new().freeze(),
    );
    assert!(matches!(
        BibSession::default().process(&job, &snapshot),
        BibAttempt::Failed(failure) if failure.kind() == BibFailureKind::ResourceConflict
    ));
}

fn complete(attempt: BibAttempt) -> BibResult {
    match attempt {
        BibAttempt::Complete(result) => result,
        attempt => panic!("expected complete result, got {attempt:?}"),
    }
}

fn needs(attempt: BibAttempt) -> FileRequestBatch {
    match attempt {
        BibAttempt::NeedResources(needs) => needs,
        attempt => panic!("expected resources, got {attempt:?}"),
    }
}

fn response(request: &FileRequest, path: &str, bytes: &[u8]) -> ResolvedFile {
    ResolvedFile {
        request: request.key().clone(),
        virtual_path: path.into(),
        bytes: bytes.to_vec(),
        expected_digest: None,
    }
}
