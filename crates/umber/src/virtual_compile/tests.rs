use crate::FontContainer;
use tex_incr::RevisionId;
use tex_state::{Universe, World};

use super::*;

const CMR10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
const CMSY10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmsy10.tfm");
const CMEX10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmex10.tfm");

fn current_render_location(result: Option<RenderedSourceResult>) -> RenderedSourceLocation {
    match result.expect("mapped source") {
        RenderedSourceResult::Current(location) => location,
        other => panic!("expected current rendered source, got {other:?}"),
    }
}

fn session(main: &str) -> VirtualCompileSession {
    let mut session = VirtualCompileSession::new(SessionOptions::default()).expect("session");
    session
        .add_user_file("main.tex", main.as_bytes().to_vec())
        .expect("main file");
    session
}

fn requests(result: CompileAttemptResult) -> Vec<FileRequest> {
    match result {
        CompileAttemptResult::NeedResources(resources) => resources
            .required
            .into_iter()
            .filter_map(|request| match request {
                ResourceRequest::File(request) => Some(request),
                ResourceRequest::Font(_) => None,
            })
            .collect(),
        other => panic!("expected missing files, got {other:#?}"),
    }
}

fn resources(result: CompileAttemptResult) -> Vec<ResourceRequest> {
    match result {
        CompileAttemptResult::NeedResources(resources) => resources.required,
        other => panic!("expected missing resources, got {other:#?}"),
    }
}

fn provide_cmu_font(session: &mut VirtualCompileSession, request: FontRequest) {
    session
        .provide_resolved_font(ResolvedFont {
            request: request.key,
            container: FontContainer::Woff2,
            bytes: include_bytes!("../../../umber-wasm/assets/cmu-serif-500-roman.woff2").to_vec(),
            declared_object_sha256: None,
            declared_program_identity: None,
            provenance: Some("CMU Serif under the SIL OFL".to_owned()),
        })
        .expect("provide OpenType font");
}

fn cmu_response(request: FontRequest) -> ResolvedFont {
    ResolvedFont {
        request: request.key,
        container: FontContainer::Woff2,
        bytes: include_bytes!("../../../umber-wasm/assets/cmu-serif-500-roman.woff2").to_vec(),
        declared_object_sha256: None,
        declared_program_identity: None,
        provenance: Some("CMU Serif under the SIL OFL".to_owned()),
    }
}

fn rendered_text_address(html: &str, code: u8) -> (u32, u32) {
    let marker = format!("data-umber-codes=\"0x{code:02x}");
    let codes = html.find(&marker).expect("text run");
    let page_prefix = "data-umber-page=\"";
    let page_start = html[..codes]
        .rfind(page_prefix)
        .map(|start| start + page_prefix.len())
        .expect("page id");
    let page_end = html[page_start..]
        .find('"')
        .map(|end| page_start + end)
        .expect("page id end");
    let event_prefix = "data-umber-event=\"";
    let event_start = html[..codes]
        .rfind(event_prefix)
        .map(|start| start + event_prefix.len())
        .expect("text event id");
    let event_end = html[event_start..]
        .find('"')
        .map(|end| event_start + end)
        .expect("event id end");
    let page = html[page_start..page_end]
        .parse::<u32>()
        .expect("numeric page id");
    let event = html[event_start..event_end]
        .parse::<u32>()
        .expect("numeric event id");
    (page, event)
}

#[test]
fn virtual_paths_normalize_dots_and_reject_escapes_and_urls() {
    assert_eq!(
        VirtualPath::user("./parts//chapter.tex")
            .expect("path")
            .as_str(),
        "/job/parts/chapter.tex"
    );
    assert_eq!(
        VirtualPath::distribution("/texlive/tex/plain/base/plain.tex")
            .expect("path")
            .as_str(),
        "/texlive/tex/plain/base/plain.tex"
    );
    for rejected in [
        "../secret.tex",
        "/other/file.tex",
        "https://example.test/a.tex",
        "C:/file.tex",
        "dir\\file.tex",
        "bad\0name.tex",
    ] {
        assert!(
            VirtualPath::user(rejected).is_err(),
            "accepted {rejected:?}"
        );
    }
}

#[test]
fn engine_path_rejections_and_missing_main_are_typed() {
    let mut traversal = session("\\input ../secret \\end");
    assert!(matches!(
        traversal.compile_attempt(),
        CompileAttemptResult::Error(CompileError::InvalidRequestedPath { .. })
    ));

    let mut absolute = session("\\input /job/secret \\end");
    assert!(matches!(
        absolute.compile_attempt(),
        CompileAttemptResult::Error(CompileError::UnavailableAbsoluteUserFile(path))
            if path == "/job/secret.tex"
    ));

    let mut missing_main = VirtualCompileSession::new(SessionOptions::default()).expect("session");
    assert!(matches!(
        missing_main.compile_attempt(),
        CompileAttemptResult::Error(CompileError::MissingMainFile(path))
            if path == "/job/main.tex"
    ));
}

#[test]
fn extensions_are_normalized_and_requests_are_deduplicated() {
    let mut session = session("\\input alpha \\input alpha.tex \\end");
    let missing = requests(session.compile_attempt());
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].key().kind(), FileKind::TexInput);
    assert_eq!(missing[0].key().name(), "alpha.tex");
    assert_eq!(missing[0].original_name(), "alpha");
}

#[test]
fn multiple_tfm_misses_and_later_input_are_batched_in_order() {
    let mut session = session("\\font\\a=one\\relax \\font\\b=two\\relax \\input later \\end");
    let missing = requests(session.compile_attempt());
    let keys = missing
        .iter()
        .map(|request| (request.key().kind(), request.key().name()))
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        vec![
            (FileKind::TexInput, "later.tex"),
            (FileKind::Tfm, "one.tfm"),
            (FileKind::Tfm, "two.tfm"),
        ]
    );
}

#[test]
fn retry_requires_progress_and_reaches_completion_after_provision() {
    let mut session = session("\\input remote \\end");
    let missing = requests(session.compile_attempt());
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Error(CompileError::NoProgress)
    ));
    session
        .provide_resolved_file(
            missing[0].key().clone(),
            "/texlive/tex/plain/remote.tex",
            b"\\message{resolved}".to_vec(),
        )
        .expect("provide remote");
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("retry should complete");
    };
    assert!(String::from_utf8_lossy(&output.terminal).contains("resolved"));
    assert_eq!(session.attempts(), 2);
}

#[test]
fn user_files_override_distribution_bindings() {
    let mut session = session("\\input shared \\end");
    session
        .add_user_file("shared.tex", b"\\message{user}".to_vec())
        .expect("user file");
    session
        .provide_resolved_file(
            FileRequestKey::new(FileKind::TexInput, "shared").expect("key"),
            "/texlive/tex/shared.tex",
            b"\\message{distribution}".to_vec(),
        )
        .expect("resolved file");
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("compile should complete");
    };
    let terminal = String::from_utf8_lossy(&output.terminal);
    assert!(terminal.contains("user"));
    assert!(!terminal.contains("distribution"));
}

#[test]
fn attempt_local_effects_do_not_leak_across_fetch_rounds() {
    let mut session = session("\\message{before}\\input remote \\end");
    let missing = requests(session.compile_attempt());
    session
        .provide_resolved_file(
            missing[0].key().clone(),
            "/texlive/tex/remote.tex",
            b"\\message{after}".to_vec(),
        )
        .expect("provide remote");
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("retry should complete");
    };
    let terminal = String::from_utf8_lossy(&output.terminal);
    assert_eq!(terminal.matches("before").count(), 1);
    assert_eq!(terminal.matches("after").count(), 1);
}

#[test]
fn missing_resource_attempt_discards_auxiliary_stage_writes() {
    let mut session = session(concat!(
        "\\immediate\\openout1=attempt.aux ",
        "\\immediate\\write1{complete} ",
        "\\immediate\\closeout1 \\input remote \\end"
    ));
    let missing = requests(session.compile_attempt());
    let output_path = VirtualPath::user("attempt.aux").expect("output path");
    assert!(
        session
            .files
            .snapshot()
            .get(&output_path)
            .expect("live snapshot")
            .is_none(),
        "suspended attempt must not publish its auxiliary file"
    );

    session
        .provide_resolved_file(
            missing[0].key().clone(),
            "/texlive/tex/remote.tex",
            b"\\message{resolved}".to_vec(),
        )
        .expect("provide remote");
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("retry should complete");
    };
    assert_eq!(output.files.len(), 1);
    let snapshot = session.files.snapshot();
    let accepted = snapshot
        .get(&output_path)
        .expect("live snapshot")
        .expect("accepted auxiliary output");
    assert_eq!(accepted.bytes(), output.files[0].bytes);
}

#[test]
fn native_and_vfs_single_pass_outputs_are_byte_identical() {
    let source = concat!(
        "\\immediate\\openout1=shared.aux ",
        "\\immediate\\write1{same} ",
        "\\immediate\\closeout1 \\message{same}\\end"
    );
    let mut stores = Universe::with_world(World::memory());
    prepare_run_stores(&mut stores);
    crate::run_memory_with_stores(source, &mut stores).expect("native memory run");
    let native =
        crate::collect_final_memory_output(&mut stores, &[], 1 << 20).expect("native output");

    let mut virtual_session = VirtualCompileSession::new(SessionOptions {
        job_name: Some("texput".to_owned()),
        ..SessionOptions::default()
    })
    .expect("virtual session");
    virtual_session
        .add_user_file("main.tex", source.as_bytes().to_vec())
        .expect("main source");
    let CompileAttemptResult::Complete(virtual_output) = virtual_session.compile_attempt() else {
        panic!("virtual run should complete");
    };
    assert_eq!(virtual_output, native);
}

#[test]
fn auxiliary_stage_limit_fails_without_publishing_generated_files() {
    let mut session = VirtualCompileSession::new(SessionOptions {
        limits: SessionLimits {
            output_bytes: 32,
            ..SessionLimits::default()
        },
        ..SessionOptions::default()
    })
    .expect("session");
    let source = format!(
        "\\immediate\\openout1=large.aux \\immediate\\write1{{{}}} \\immediate\\closeout1 \\end",
        "x".repeat(64)
    );
    session
        .add_user_file("main.tex", source.into_bytes())
        .expect("main source");
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Error(CompileError::LimitExceeded { limit: 32, .. })
    ));
    assert!(
        session
            .files
            .snapshot()
            .get(&VirtualPath::user("large.aux").expect("path"))
            .expect("live snapshot")
            .is_none()
    );
}

#[test]
fn accepted_patch_publishes_root_generated_files_and_output_together() {
    let source = concat!(
        "\\immediate\\openout1=state.aux ",
        "\\immediate\\write1{old} ",
        "\\immediate\\closeout1 \\end"
    );
    let mut session = session(source);
    let CompileAttemptResult::Complete(old_output) = session.compile_attempt() else {
        panic!("initial revision should complete");
    };
    let old_generation = session.files.snapshot().generation_identity();
    let start = source.find("old").expect("old payload");
    let mut next_source = source.to_owned();
    next_source.replace_range(start..start + 3, "new");

    session
        .apply_patch(SourcePatch {
            next_revision: RevisionId::new(2),
            base_revision: RevisionId::new(1),
            expected_hash: session.content_hash().expect("accepted hash"),
            range: start..start + 3,
            replacement: "new".to_owned(),
        })
        .expect("patch");
    assert_eq!(session.accepted_output.as_ref(), Some(&old_output));
    let CompileAttemptResult::Complete(new_output) = session.compile_attempt() else {
        panic!("patched revision should complete");
    };

    let snapshot = session.files.snapshot();
    assert_ne!(snapshot.generation_identity(), old_generation);
    assert_eq!(
        snapshot
            .get(&session.main_path)
            .expect("root lookup")
            .expect("root")
            .bytes(),
        next_source.as_bytes()
    );
    let auxiliary = snapshot
        .get(&VirtualPath::user("state.aux").expect("aux path"))
        .expect("aux lookup")
        .expect("accepted aux");
    assert!(auxiliary.bytes().windows(3).any(|bytes| bytes == b"new"));
    assert_ne!(new_output, old_output);
    assert_eq!(session.revision(), Some(RevisionId::new(2)));

    let vfs = snapshot.retention();
    let engine = session
        .incremental
        .as_ref()
        .and_then(tex_incr::Session::retention_metrics)
        .expect("engine retention");
    let returned = memory_run_output_bytes(&new_output);
    let retention = session.retention_metrics().expect("session retention");
    assert_eq!(retention.resource_bytes, vfs.input_bytes);
    assert_eq!(
        retention.output_bytes,
        engine.output_bytes + returned + vfs.generated_bytes
    );
}

#[test]
fn failed_patch_restores_the_complete_accepted_build() {
    let source = concat!(
        "\\immediate\\openout1=state.aux ",
        "\\immediate\\write1{old} ",
        "\\immediate\\closeout1 \\end"
    );
    let mut session = VirtualCompileSession::new(SessionOptions {
        limits: SessionLimits {
            output_bytes: 512,
            ..SessionLimits::default()
        },
        ..SessionOptions::default()
    })
    .expect("session");
    session
        .add_user_file("main.tex", source.as_bytes().to_vec())
        .expect("main source");
    let CompileAttemptResult::Complete(old_output) = session.compile_attempt() else {
        panic!("initial revision should fit");
    };
    let old_hash = session.content_hash().expect("accepted hash");
    let old_snapshot = session.files.snapshot();
    let old_generation = old_snapshot.generation_identity();
    let old_root = old_snapshot
        .get(&session.main_path)
        .expect("root lookup")
        .expect("root")
        .bytes()
        .to_vec();
    let old_aux = old_snapshot
        .get(&VirtualPath::user("state.aux").expect("aux path"))
        .expect("aux lookup")
        .expect("aux")
        .bytes()
        .to_vec();
    drop(old_snapshot);
    let start = source.find("old").expect("old payload");

    session
        .apply_patch(SourcePatch {
            next_revision: RevisionId::new(2),
            base_revision: RevisionId::new(1),
            expected_hash: old_hash,
            range: start..start + 3,
            replacement: "x".repeat(1_024),
        })
        .expect("valid patch");
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Error(CompileError::LimitExceeded { .. })
    ));

    let snapshot = session.files.snapshot();
    assert_eq!(snapshot.generation_identity(), old_generation);
    assert_eq!(
        snapshot
            .get(&session.main_path)
            .expect("root lookup")
            .expect("root")
            .bytes(),
        old_root
    );
    assert_eq!(
        snapshot
            .get(&VirtualPath::user("state.aux").expect("aux path"))
            .expect("aux lookup")
            .expect("aux")
            .bytes(),
        old_aux
    );
    assert_eq!(session.revision(), Some(RevisionId::new(1)));
    assert_eq!(session.content_hash(), Some(old_hash));
    assert_eq!(session.accepted_output.as_ref(), Some(&old_output));
    assert!(session.pending_patch.is_none());
    assert_eq!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(old_output)
    );
}

#[test]
fn format_and_fresh_initialization_both_complete() {
    let mut stores = Universe::with_world(World::memory());
    prepare_run_stores(&mut stores);
    let format = stores.dump_format().expect("dump format");
    let mut formatted = VirtualCompileSession::new(SessionOptions {
        format: Some(format),
        ..SessionOptions::default()
    })
    .expect("formatted session");
    formatted
        .add_user_file("main.tex", b"\\end".to_vec())
        .expect("main");
    assert!(matches!(
        formatted.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
    assert!(matches!(
        session("\\end").compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
}

#[test]
fn format_rejection_and_job_clock_are_deterministic() {
    let mut corrupt = VirtualCompileSession::new(SessionOptions {
        format: Some(vec![0; 29]),
        ..SessionOptions::default()
    })
    .expect("format fits configured size");
    corrupt
        .add_user_file("main.tex", b"\\end".to_vec())
        .expect("main");
    assert!(matches!(
        corrupt.compile_attempt(),
        CompileAttemptResult::Error(CompileError::Format(_))
    ));

    let mut clocked = VirtualCompileSession::new(SessionOptions {
        clock: tex_state::JobClock {
            time: 754,
            second: 0,
            day: 13,
            month: 7,
            year: 2042,
        },
        ..SessionOptions::default()
    })
    .expect("clocked session");
    clocked
        .add_user_file("main.tex", b"\\message{year=\\the\\year}\\end".to_vec())
        .expect("main");
    let CompileAttemptResult::Complete(output) = clocked.compile_attempt() else {
        panic!("clocked compile should complete");
    };
    assert!(String::from_utf8_lossy(&output.terminal).contains("year=2042"));
}

#[test]
fn valid_tfm_produces_a_nonempty_dvi() {
    let mut session = session("\\font\\tenrm=cmr10\\relax \\tenrm \\shipout\\hbox{A}\\end");
    let missing = resources(session.compile_attempt());
    assert_eq!(missing.len(), 1);
    let file = missing
        .iter()
        .find_map(|request| match request {
            ResourceRequest::File(request) => Some(request.clone()),
            ResourceRequest::Font(_) => None,
        })
        .expect("TFM request");
    session
        .provide_resolved_file(
            file.key().clone(),
            "/texlive/fonts/tfm/public/cm/cmr10.tfm",
            CMR10.to_vec(),
        )
        .expect("provide tfm");
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("compile should complete");
    };
    assert!(!output.dvi.is_empty());
    assert_eq!(&output.dvi[..2], &[247, 2]);
}

#[test]
fn font_batches_accept_partial_unordered_responses_and_reject_conflicts() {
    let mut session = VirtualCompileSession::new(SessionOptions {
        html: true,
        ..SessionOptions::default()
    })
    .expect("HTML session");
    session
        .add_user_file(
            "main.tex",
            b"\\font\\tenrm=cmr10\\relax \\tenrm \\shipout\\hbox{A}\\end".to_vec(),
        )
        .expect("main source");
    let missing = resources(session.compile_attempt());
    let file = missing
        .iter()
        .find_map(|request| match request {
            ResourceRequest::File(request) => Some(request.clone()),
            ResourceRequest::Font(_) => None,
        })
        .expect("TFM request");
    let font = missing
        .iter()
        .find_map(|request| match request {
            ResourceRequest::Font(request) => Some(request.clone()),
            ResourceRequest::File(_) => None,
        })
        .expect("font request");

    let response = cmu_response(font.clone());
    session
        .provide_resources(vec![ResourceResponse::Font(response.clone())])
        .expect("unordered partial font response");
    session
        .provide_resources(vec![ResourceResponse::Font(response.clone())])
        .expect("identical duplicate");
    let mut conflict = response;
    conflict.provenance = Some("different metadata".to_owned());
    assert!(matches!(
        session.provide_resources(vec![ResourceResponse::Font(conflict)]),
        Err(CompileError::ConflictingResolvedBinding(_))
    ));

    let remaining = resources(session.compile_attempt());
    assert_eq!(remaining, vec![ResourceRequest::File(file.clone())]);
    session
        .provide_resources(vec![ResourceResponse::File(ResolvedFile {
            request: file.key().clone(),
            virtual_path: "/texlive/fonts/tfm/public/cm/cmr10.tfm".to_owned(),
            bytes: CMR10.to_vec(),
            expected_digest: None,
        })])
        .expect("TFM response");
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
}

#[test]
fn invalid_mixed_batch_publishes_nothing() {
    let mut session = VirtualCompileSession::new(SessionOptions {
        html: true,
        ..SessionOptions::default()
    })
    .expect("HTML session");
    session
        .add_user_file("main.tex", b"\\font\\tenrm=cmr10\\relax \\end".to_vec())
        .expect("main source");
    let missing = resources(session.compile_attempt());
    let file = missing
        .iter()
        .find_map(|request| match request {
            ResourceRequest::File(request) => Some(request.clone()),
            ResourceRequest::Font(_) => None,
        })
        .expect("TFM request");
    let font = missing
        .iter()
        .find_map(|request| match request {
            ResourceRequest::Font(request) => Some(request.clone()),
            ResourceRequest::File(_) => None,
        })
        .expect("font request");
    let invalid_font = ResolvedFont {
        request: font.key,
        container: FontContainer::Woff2,
        bytes: b"wOF2".to_vec(),
        declared_object_sha256: None,
        declared_program_identity: None,
        provenance: None,
    };
    assert!(
        session
            .provide_resources(vec![
                ResourceResponse::File(ResolvedFile {
                    request: file.key().clone(),
                    virtual_path: "/texlive/fonts/tfm/public/cm/cmr10.tfm".to_owned(),
                    bytes: CMR10.to_vec(),
                    expected_digest: None,
                }),
                ResourceResponse::Font(invalid_font),
            ])
            .is_err()
    );
    assert_eq!(session.resolved_file_count(), 0);
    assert_eq!(session.cached_file_bytes(), 0);
}

#[test]
fn requested_html_and_dvi_share_one_committed_compile() {
    let mut session = VirtualCompileSession::new(SessionOptions {
        html: true,
        ..SessionOptions::default()
    })
    .expect("HTML session");
    session
        .add_user_file(
            "main.tex",
            b"\\font\\tenrm=cmr10\\relax \\tenrm \\shipout\\hbox{A}\\end".to_vec(),
        )
        .expect("main source");
    let missing = resources(session.compile_attempt());
    let file = missing
        .iter()
        .find_map(|request| match request {
            ResourceRequest::File(request) => Some(request.clone()),
            ResourceRequest::Font(_) => None,
        })
        .expect("TFM request");
    let font = missing
        .iter()
        .find_map(|request| match request {
            ResourceRequest::Font(request) => Some(request.clone()),
            ResourceRequest::File(_) => None,
        })
        .expect("font request");
    session
        .provide_resolved_file(
            file.key().clone(),
            "/texlive/fonts/tfm/public/cm/cmr10.tfm",
            CMR10.to_vec(),
        )
        .expect("TFM");
    provide_cmu_font(&mut session, font);
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("HTML compile should complete");
    };
    assert!(!output.dvi.is_empty());
    let html = String::from_utf8(output.html.expect("HTML output")).expect("HTML UTF-8");
    let output_id = session
        .rendered_output_id()
        .expect("rendered output identity");
    assert!(html.contains("data-umber-page=\"1\" data-umber-revision=\"1\""));
    assert!(html.contains(&format!("data-umber-output=\"{output_id}\"")));
    assert!(html.contains("data-umber-baseline-sp"));
    assert!(html.contains(">A</text>"));
    assert!(output.html_assets.is_empty());

    let (page, event) = rendered_text_address(&html, b'A');
    let retention_before = session.retention_metrics().expect("accepted retention");
    let location = current_render_location(
        session
            .rendered_source_location(page, event, Some(0), output_id, RevisionId::new(1))
            .expect("source query"),
    );
    let retention_after = session.retention_metrics().expect("live retention");
    assert!(retention_after.diagnostic_bytes > retention_before.diagnostic_bytes);
    let source = b"\\font\\tenrm=cmr10\\relax \\tenrm \\shipout\\hbox{A}\\end";
    let start = source.iter().position(|byte| *byte == b'A').expect("A");
    assert_eq!(location.revision, RevisionId::new(1));
    assert_eq!(location.path, "/job/main.tex");
    assert_eq!(location.start as usize, start);
    assert_eq!(location.end as usize, start + 1);
    assert_eq!((location.line, location.column), (1, start as u32 + 1));
    assert!(
        session
            .rendered_source_location(0, event, Some(0), output_id, RevisionId::new(1))
            .expect("invalid page query")
            .is_none()
    );
    assert!(
        session
            .rendered_source_location(page, event, Some(u32::MAX), output_id, RevisionId::new(1),)
            .expect("invalid unit query")
            .is_none()
    );

    session
        .apply_patch(SourcePatch {
            next_revision: RevisionId::new(2),
            base_revision: RevisionId::new(1),
            expected_hash: session.content_hash().expect("accepted hash"),
            range: start..start + 1,
            replacement: "B".to_owned(),
        })
        .expect("glyph patch");
    assert!(
        session
            .rendered_source_location(1, event, Some(0), output_id, RevisionId::new(1))
            .expect("query while patch pending")
            .is_none()
    );
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("patched HTML compile should complete");
    };
    let html = String::from_utf8(output.html.expect("patched HTML output")).expect("HTML UTF-8");
    assert!(html.contains("data-umber-page=\"1\" data-umber-revision=\"2\""));
    assert!(html.contains(&format!("data-umber-output=\"{output_id}\"")));
    let (page, event) = rendered_text_address(&html, b'B');
    assert_eq!(
        session
            .rendered_source_location(page, event, Some(0), output_id, RevisionId::new(1))
            .expect("stale source query"),
        Some(RenderedSourceResult::StaleRevision {
            accepted: RevisionId::new(2),
        })
    );
    let location = current_render_location(
        session
            .rendered_source_location(page, event, Some(0), output_id, RevisionId::new(2))
            .expect("patched source query"),
    );
    assert_eq!(location.revision, RevisionId::new(2));
    assert_eq!(location.path, "/job/main.tex");
    assert_eq!(
        (location.start as usize, location.end as usize),
        (start, start + 1)
    );
}

#[test]
fn accepted_user_tfm_remains_available_across_incremental_patch() {
    let source =
        "\\font\\tenrm=cmr10\\relax\\tenrm %a\n\\shipout\\hbox{\\char65}\\shipout\\hbox{B}\\end";
    let mut session = VirtualCompileSession::new(SessionOptions {
        html: true,
        ..SessionOptions::default()
    })
    .expect("HTML session");
    session
        .add_user_file("cmr10.tfm", CMR10.to_vec())
        .expect("TFM");
    session
        .add_user_file("main.tex", source.as_bytes().to_vec())
        .expect("main source");
    for font in resources(session.compile_attempt())
        .into_iter()
        .filter_map(|request| match request {
            ResourceRequest::Font(request) => Some(request),
            ResourceRequest::File(_) => None,
        })
    {
        provide_cmu_font(&mut session, font);
    }
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));

    let comment = source.find("%a").expect("comment") + 1;
    session
        .apply_patch(SourcePatch {
            next_revision: RevisionId::new(2),
            base_revision: RevisionId::new(1),
            expected_hash: session.content_hash().expect("accepted hash"),
            range: comment..comment + 1,
            replacement: "b".to_owned(),
        })
        .expect("comment patch");
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
}

#[test]
fn rendered_source_location_survives_paragraph_line_breaking() {
    let source = b"\\font\\tenrm=cmr10\\relax \\hsize=12pt \\parindent=0pt \\tenrm A B\\par\\end";
    let mut session = VirtualCompileSession::new(SessionOptions {
        html: true,
        ..SessionOptions::default()
    })
    .expect("HTML session");
    session
        .add_user_file("cmr10.tfm", CMR10.to_vec())
        .expect("TFM");
    session
        .add_user_file("main.tex", source.to_vec())
        .expect("main source");
    let fonts = resources(session.compile_attempt())
        .into_iter()
        .filter_map(|request| match request {
            ResourceRequest::Font(request) => Some(request),
            ResourceRequest::File(_) => None,
        })
        .collect::<Vec<_>>();
    assert!(!fonts.is_empty(), "font requests");
    for font in fonts {
        provide_cmu_font(&mut session, font);
    }

    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("line-broken HTML compile should complete");
    };
    let html = String::from_utf8(output.html.expect("HTML output")).expect("HTML UTF-8");
    assert!(html.matches("class=\"umber-run\"").count() >= 2);
    let (page, event) = rendered_text_address(&html, b'B');
    let output_id = session
        .rendered_output_id()
        .expect("rendered output identity");
    let location = current_render_location(
        session
            .rendered_source_location(page, event, Some(0), output_id, RevisionId::new(1))
            .expect("source query"),
    );
    let start = source.iter().position(|byte| *byte == b'B').expect("B");
    assert_eq!(location.revision, RevisionId::new(1));
    assert_eq!(location.path, "/job/main.tex");
    assert_eq!(
        (location.start as usize, location.end as usize),
        (start, start + 1)
    );
}

#[test]
fn rendered_source_location_survives_math_layout() {
    let source = b"\\font\\tenrm=cmr10 \\font\\sy=cmsy10 \\font\\ex=cmex10 \\textfont0=\\tenrm \\textfont2=\\sy \\scriptfont2=\\sy \\scriptscriptfont2=\\sy \\textfont3=\\ex \\scriptfont3=\\ex \\scriptscriptfont3=\\ex \\mathcode`A=\"0041 \\shipout\\hbox{$A$}\\end";
    let mut session = VirtualCompileSession::new(SessionOptions {
        html: true,
        ..SessionOptions::default()
    })
    .expect("HTML session");
    session
        .add_user_file("cmr10.tfm", CMR10.to_vec())
        .expect("TFM");
    session
        .add_user_file("cmsy10.tfm", CMSY10.to_vec())
        .expect("symbol TFM");
    session
        .add_user_file("cmex10.tfm", CMEX10.to_vec())
        .expect("extension TFM");
    session
        .add_user_file("main.tex", source.to_vec())
        .expect("main source");
    let fonts = resources(session.compile_attempt())
        .into_iter()
        .filter_map(|request| match request {
            ResourceRequest::Font(request) => Some(request),
            ResourceRequest::File(_) => None,
        })
        .collect::<Vec<_>>();
    assert!(!fonts.is_empty(), "font requests");
    for font in fonts {
        provide_cmu_font(&mut session, font);
    }

    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("math HTML compile should complete");
    };
    let html = String::from_utf8(output.html.expect("HTML output")).expect("HTML UTF-8");
    let (page, event) = rendered_text_address(&html, b'A');
    let output_id = session
        .rendered_output_id()
        .expect("rendered output identity");
    let location = current_render_location(
        session
            .rendered_source_location(page, event, Some(0), output_id, RevisionId::new(1))
            .expect("source query"),
    );
    let start = source
        .windows(3)
        .position(|bytes| bytes == b"$A$")
        .map(|start| start + 1)
        .expect("math A");
    assert_eq!(location.revision, RevisionId::new(1));
    assert_eq!(location.path, "/job/main.tex");
    assert_eq!(
        (location.start as usize, location.end as usize),
        (start, start + 1)
    );
}

#[test]
fn user_and_distribution_limits_fail_atomically() {
    let limits = SessionLimits {
        user_files: 1,
        one_file_bytes: 4,
        user_source_bytes: 4,
        resolved_files: 1,
        cached_file_bytes: 4,
        ..SessionLimits::default()
    };
    let mut session = VirtualCompileSession::new(SessionOptions {
        limits,
        ..SessionOptions::default()
    })
    .expect("session");
    assert!(matches!(
        session.add_user_file("large.tex", vec![0; 5]),
        Err(CompileError::LimitExceeded { .. })
    ));
    session
        .add_user_file("first.tex", vec![0; 4])
        .expect("first user file at limit");
    assert!(matches!(
        session.add_user_file("second.tex", Vec::new()),
        Err(CompileError::LimitExceeded {
            resource: "user files",
            limit: 1,
            attempted: 2,
        })
    ));
    session
        .add_user_file("first.tex", vec![1; 4])
        .expect("replacing a user file does not increase count");
    let first = FileRequestKey::new(FileKind::TexInput, "one").expect("key");
    assert!(matches!(
        session.provide_resolved_file(first.clone(), "/texlive/one.tex", vec![0; 5]),
        Err(CompileError::LimitExceeded { .. })
    ));
    assert_eq!(session.resolved_file_count(), 0);
    assert_eq!(session.cached_file_bytes(), 0);
    session
        .provide_resolved_file(first, "/texlive/one.tex", vec![0; 4])
        .expect("at limit");
    let second = FileRequestKey::new(FileKind::TexInput, "two").expect("key");
    assert!(matches!(
        session.provide_resolved_file(second, "/texlive/two.tex", Vec::new()),
        Err(CompileError::LimitExceeded { .. })
    ));
    assert_eq!(session.resolved_file_count(), 1);
    assert_eq!(session.cached_file_bytes(), 4);
}

#[test]
fn default_session_limit_accepts_the_pinned_latex_format() {
    assert!(SessionLimits::default().one_file_bytes >= 74_240_748);
    assert!(SessionLimits::default().validate().is_ok());
}

#[test]
fn canonical_distribution_path_allows_identical_alias_keys() {
    let mut session = session("\\end");
    let short = FileRequestKey::new(FileKind::TexInput, "plain").expect("short key");
    let explicit =
        FileRequestKey::new(FileKind::TexInput, "tex/plain/base/plain").expect("explicit key");
    session
        .provide_resolved_file(short, "/texlive/tex/plain/base/plain.tex", b"same".to_vec())
        .expect("first alias");
    session
        .provide_resolved_file(
            explicit,
            "/texlive/tex/plain/base/plain.tex",
            b"same".to_vec(),
        )
        .expect("matching alias");
    assert_eq!(session.resolved_file_count(), 2);
    assert_eq!(session.cached_file_bytes(), 4);

    let conflict = FileRequestKey::new(FileKind::TexInput, "other").expect("conflict key");
    assert!(matches!(
        session.provide_resolved_file(
            conflict,
            "/texlive/tex/plain/base/plain.tex",
            b"different".to_vec(),
        ),
        Err(CompileError::DistributionPathCollision(_))
    ));
}

#[test]
fn returned_output_limit_remains_a_typed_session_error() {
    let mut session = VirtualCompileSession::new(SessionOptions {
        limits: SessionLimits {
            output_bytes: 1,
            ..SessionLimits::default()
        },
        ..SessionOptions::default()
    })
    .expect("session");
    session
        .add_user_file("main.tex", b"\\message{too-large}\\end".to_vec())
        .expect("main");
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Error(CompileError::LimitExceeded {
            resource: "returned output bytes",
            limit: 1,
            ..
        })
    ));
}

#[test]
fn attempt_and_hard_limits_are_enforced() {
    let mut limited = VirtualCompileSession::new(SessionOptions {
        limits: SessionLimits {
            attempts: 0,
            ..SessionLimits::default()
        },
        ..SessionOptions::default()
    })
    .expect("limited session");
    assert!(matches!(
        limited.compile_attempt(),
        CompileAttemptResult::Error(CompileError::AttemptLimit { limit: 0 })
    ));

    let error = VirtualCompileSession::new(SessionOptions {
        limits: SessionLimits {
            attempts: SessionLimits::HARD_MAX.attempts + 1,
            ..SessionLimits::default()
        },
        ..SessionOptions::default()
    });
    assert!(matches!(error, Err(CompileError::HardLimitExceeded { .. })));
}

#[test]
fn cache_clear_keeps_user_files_and_drops_bindings() {
    let mut session = session("\\input remote \\end");
    let key = FileRequestKey::new(FileKind::TexInput, "remote").expect("key");
    session
        .provide_resolved_file(key, "/texlive/remote.tex", b"done".to_vec())
        .expect("provide");
    session.clear_distribution_cache().expect("clear cache");
    assert_eq!(session.resolved_file_count(), 0);
    assert_eq!(session.cached_file_bytes(), 0);
    assert_eq!(requests(session.compile_attempt()).len(), 1);
}

#[test]
fn cache_clear_preserves_the_latest_accepted_editor_root() {
    let original = "\\message{old}\\end";
    let mut session = session(original);
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
    let old = original.find("old").expect("old message");
    session
        .apply_patch(SourcePatch {
            next_revision: RevisionId::new(2),
            base_revision: RevisionId::new(1),
            expected_hash: session.content_hash().expect("accepted hash"),
            range: old..old + 3,
            replacement: "new".to_owned(),
        })
        .expect("patch");
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));

    session.clear_distribution_cache().expect("clear cache");
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("latest root should compile after clearing the cache");
    };
    let terminal = String::from_utf8(output.terminal).expect("terminal UTF-8");
    assert!(terminal.contains("new"));
    assert!(!terminal.contains("old"));
}

#[test]
fn persistent_session_accepts_multiple_revision_checked_patches() {
    let original = "\\shipout\\vbox{\\hrule height 1pt}\\end";
    let mut session = session(original);
    let CompileAttemptResult::Complete(first) = session.compile_attempt() else {
        panic!("initial revision should complete");
    };
    assert_eq!(session.revision(), Some(RevisionId::new(1)));

    let first_hash = session.content_hash().expect("accepted hash");
    let one = original.find("1pt").expect("height");
    session
        .apply_patch(SourcePatch {
            next_revision: RevisionId::new(2),
            base_revision: RevisionId::new(1),
            expected_hash: first_hash,
            range: one..one + 1,
            replacement: "2".to_owned(),
        })
        .expect("first patch");
    let CompileAttemptResult::Complete(second) = session.compile_attempt() else {
        panic!("second revision should complete");
    };
    assert_ne!(first.dvi, second.dvi);
    assert_eq!(session.revision(), Some(RevisionId::new(2)));

    let second_hash = session.content_hash().expect("second hash");
    session
        .apply_patch(SourcePatch {
            next_revision: RevisionId::new(3),
            base_revision: RevisionId::new(2),
            expected_hash: second_hash,
            range: one..one + 1,
            replacement: "3".to_owned(),
        })
        .expect("second patch");
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
    assert_eq!(session.revision(), Some(RevisionId::new(3)));

    let stale = session.apply_patch(SourcePatch {
        next_revision: RevisionId::new(4),
        base_revision: RevisionId::new(2),
        expected_hash: second_hash,
        range: one..one + 1,
        replacement: "4".to_owned(),
    });
    assert!(
        matches!(stale, Err(CompileError::Incremental(message)) if message.contains("stale revision"))
    );
}

#[test]
fn patch_can_request_and_pin_a_new_resource_before_acceptance() {
    let original = "\\shipout\\hbox{}\\end";
    let mut session = session(original);
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
    let insert = original.find("\\end").expect("end");
    session
        .apply_patch(SourcePatch {
            next_revision: RevisionId::new(2),
            base_revision: RevisionId::new(1),
            expected_hash: session.content_hash().expect("hash"),
            range: insert..insert,
            replacement: "\\input added ".to_owned(),
        })
        .expect("patch");
    let requested = requests(session.compile_attempt());
    assert_eq!(requested.len(), 1);
    session
        .provide_resolved_file(
            requested[0].key().clone(),
            "/texlive/added.tex",
            b"% supplied after patch\n".to_vec(),
        )
        .expect("resource");
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
    assert_eq!(session.revision(), Some(RevisionId::new(2)));
}
