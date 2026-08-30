use crate::FontContainer;
use std::path::Path;
use tex_incr::RevisionId;
use tex_state::World;
use tex_state::meaning::{ExpandablePrimitive, Meaning};

use super::*;

#[test]
fn every_supported_runtime_names_its_command_owned_engine_entry() {
    let sources = [
        (
            "CLI",
            include_str!("../main.rs"),
            "cli_resource::run_for_finalization",
        ),
        (
            "retained session",
            include_str!("../engine_session.rs"),
            "control: MainControl",
        ),
        (
            "direct library",
            include_str!("../lib.rs"),
            "EngineSession::new",
        ),
        (
            "loaded format",
            include_str!("../format_fixture.rs"),
            "EngineSession::new",
        ),
        (
            "virtual compile",
            include_str!("../virtual_compile.rs"),
            "drive_with_resource_resolvers",
        ),
        (
            "editor",
            include_str!("../editor_session.rs"),
            "VirtualCompileSession",
        ),
        (
            "incremental",
            include_str!("../../../tex-incr/src/lib.rs"),
            "candidate_control(",
        ),
        (
            "WASM",
            include_str!("../../../umber-wasm/src/lib.rs"),
            "VirtualCompileSession",
        ),
    ];
    for (runtime, source, command_entry) in sources {
        assert!(
            source.contains(command_entry),
            "{runtime} must enter through its command-owned engine boundary `{command_entry}`"
        );
    }
}

const CMR10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
const CMSY10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmsy10.tfm");
const CMEX10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmex10.tfm");

#[test]
fn resource_batch_ceiling_deduplicates_without_reordering_protocol_vectors() {
    let first = ResourceRequest::File(FileRequest::new(
        FileRequestKey::new(FileKind::TexInput, "first.tex").expect("first key"),
        "first.tex",
    ));
    let second = ResourceRequest::File(FileRequest::new(
        FileRequestKey::new(FileKind::TexInput, "second.tex").expect("second key"),
        "second.tex",
    ));
    let required = vec![second.clone(), first.clone()];
    let probes = vec![first.clone()];
    let prefetch_hints = vec![second.clone()];

    check_resource_batch_limit(&required, &probes, &prefetch_hints, 2)
        .expect("duplicate requests count once");
    assert_eq!(required, [second, first]);
    assert!(matches!(
        check_resource_batch_limit(&required, &probes, &[], 1),
        Err(CompileError::LimitExceeded {
            resource: "resource request batch",
            limit: 1,
            attempted: 2,
        })
    ));
}

fn current_render_location(result: Option<RenderedSourceResult>) -> RenderedSourceLocation {
    match result.expect("mapped source") {
        RenderedSourceResult::Current(location) => location,
        other => panic!("expected current rendered source, got {other:?}"),
    }
}

fn session(main: &str) -> VirtualCompileSession<'static> {
    let mut session = VirtualCompileSession::new(SessionOptions {
        font_layout_policy: tex_fonts::FontLayoutPolicy::ClassicTfmExact,
        ..SessionOptions::default()
    })
    .expect("session");
    session
        .add_user_file("main.tex", main.as_bytes().to_vec())
        .expect("main file");
    session
}

fn construct_test_format(mode: EngineMode, source: &str) -> tex_state::DetachedFormatImage {
    crate::with_engine_world(World::memory(), |stores| {
        mode.prepare_initex(stores);
        let source = format!("\\catcode123=1 \\catcode125=2 \\catcode35=6 {source}");
        crate::run_memory_collecting_initex_artifacts_with_profile(
            &source,
            stores,
            mode.command_profile(),
        )
        .expect("test format construction")
        .format_dump
        .expect("test format dump")
        .image
    })
    .expect("fresh format-construction universe")
}

#[test]
fn tex_etex_pdftex_fresh_and_twice_loaded_format_matrix() {
    let mut images = Vec::new();

    for mode in [EngineMode::Tex82, EngineMode::ETex, EngineMode::PdfTex] {
        let image = construct_test_format(mode, "\\count37=1403\\dump");
        let mut loaded = VirtualCompileSession::new(SessionOptions {
            format: Some(image.as_bytes().to_vec()),
            engine: mode,
            ..SessionOptions::default()
        })
        .expect("loaded profile session");
        loaded
            .add_user_file("main.tex", b"\\message{count=\\the\\count37}\\end".to_vec())
            .expect("loaded profile main");
        let CompileAttemptResult::Complete(output) = loaded.compile_attempt() else {
            panic!("loaded profile completes in {mode:?}");
        };
        assert!(
            String::from_utf8_lossy(&output.terminal).contains("count=1403"),
            "ordinary state restores in {mode:?}"
        );
        images.push(image.into_bytes());
    }

    assert_ne!(images[0], images[1]);
    assert_ne!(images[1], images[2]);
    assert_ne!(images[0], images[2]);
}

#[test]
fn profile_format_dump_and_malformed_load_fail_exactly() {
    let bytes = construct_test_format(EngineMode::PdfTex, "\\dump").into_bytes();
    for (label, malformed, expected) in [
        (
            "truncated",
            bytes[..bytes.len() - 1].to_vec(),
            "truncated Umber format file",
        ),
        (
            "bad magic",
            {
                let mut malformed = bytes.clone();
                malformed[0] ^= 0xff;
                malformed
            },
            "not an Umber format file",
        ),
    ] {
        let error = tex_state::DetachedFormatImage::try_from_bytes(malformed)
            .expect_err("malformed format must fail closed");
        assert_eq!(error.to_string(), expected, "{label}");
    }
}

fn compile_diagnostic(session: &mut VirtualCompileSession) -> CompileDiagnostic {
    match session.compile_attempt() {
        CompileAttemptResult::Error(CompileError::Diagnostic(diagnostic)) => diagnostic,
        other => panic!("expected engine diagnostic, got {other:#?}"),
    }
}

#[test]
fn engine_diagnostic_preserves_atomic_root_utf8_location() {
    let source = "é\n  \\input absent\n";
    let mut session = VirtualCompileSession::new(SessionOptions {
        authored_root_name: Some("texput".to_owned()),
        font_layout_policy: tex_fonts::FontLayoutPolicy::ClassicTfmExact,
        ..SessionOptions::default()
    })
    .expect("session");
    session
        .add_user_file("main.tex", source.as_bytes().to_vec())
        .expect("main file");
    let [request] = requests(session.compile_attempt())
        .try_into()
        .expect("one input");
    session
        .provide_resources(vec![ResourceResponse::FileUnavailable(
            request.key().clone(),
        )])
        .expect("unavailable input");
    let expected = Some(CompileSourceLocation {
        file: "/job/main.tex".into(),
        byte_start: 5,
        byte_end: 11,
        line: 2,
        column: 3,
        excerpt: "  \\input absent".into(),
    });
    let diagnostic = compile_diagnostic(&mut session);
    assert_eq!(diagnostic.location, expected);
}

#[test]
fn engine_diagnostic_preserves_included_file_location() {
    let mut session = session("\\input child \\end");
    session
        .add_user_file("child.tex", b"x\\input absent\n".to_vec())
        .expect("included file");
    let [request] = requests(session.compile_attempt())
        .try_into()
        .expect("one input");
    session
        .provide_resources(vec![ResourceResponse::FileUnavailable(
            request.key().clone(),
        )])
        .expect("unavailable input");
    let expected = Some(CompileSourceLocation {
        file: "/job/child.tex".into(),
        byte_start: 1,
        byte_end: 7,
        line: 1,
        column: 2,
        excerpt: "x\\input absent".into(),
    });
    let diagnostic = compile_diagnostic(&mut session);
    assert_eq!(diagnostic.location, expected);
}

#[test]
fn engine_diagnostic_captures_first_bounded_expansion_and_group_context() {
    let mut session = session("\\begingroup\\def\\a#2{x}\\input absent\\endgroup\\end");
    let [request] = requests(session.compile_attempt())
        .try_into()
        .expect("one input");
    session
        .provide_resources(vec![ResourceResponse::FileUnavailable(
            request.key().clone(),
        )])
        .expect("unavailable input");

    let diagnostic = compile_diagnostic(&mut session);
    let context = diagnostic
        .context
        .unwrap_or_else(|| panic!("candidate diagnostic context: {}", diagnostic.message));
    assert!(context.input_frame_count >= context.input_frame_tail.len());
    assert!(context.input_frame_tail.len() <= 8);
    assert_eq!(context.cause_kind, "command-error");
    assert!(context.input_frame_tail.contains(&"source"));
    assert_eq!(context.group_depth, 1);
    assert_eq!(context.group_tail.len(), 1);
    assert_eq!(context.group_tail[0].kind, "semi-simple");
}

#[test]
fn accepted_finalization_transfers_uncommitted_engine_state() {
    let mut session = session("\\message{accepted-finalization}\\end");
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("revision should complete");
    };

    let finalization = session
        .into_accepted_finalization()
        .expect("accepted finalization");
    assert!(!finalization.completion.effects().is_empty());
    let mut destination = World::memory();
    let publication = finalization
        .completion
        .into_publication()
        .expect("prepare accepted publication");
    let plan = crate::PlannedFinalization::new(publication, Vec::new()).expect("empty driver plan");
    plan.commit_effects(&mut destination)
        .expect("commit accepted effects");
    assert_eq!(
        destination.memory_terminal_output(),
        Some(output.terminal.as_slice())
    );
    assert!(finalization.format_dump.is_none());
}

#[test]
fn accepted_finalization_keeps_openout_page_unpublished() {
    let mut session = session(
        "\\setbox0=\\hbox{\\openout2=original.out \\write2{x}\\closeout2}\
         \\shipout\\copy0\\end",
    );
    let CompileAttemptResult::Complete(_) = session.compile_attempt() else {
        panic!("revision should complete");
    };

    let finalization = session
        .into_accepted_finalization()
        .expect("accepted finalization");
    assert_eq!(finalization.completion.pages().len(), 1);
    let page =
        tex_out::PageArtifact::from_bytes(finalization.completion.pages()[0].artifact().bytes())
            .expect("prepared artifact parses");
    assert!(page.effects.iter().any(|effect| {
        matches!(
            effect,
            tex_out::PageEffect::OpenOut { stream: 2, path }
                if path == "original.out"
        )
    }));
}

#[test]
#[allow(clippy::disallowed_methods)] // Exercises authoritative real open failure and artifact CAS.
fn replacement_openout_name_becomes_published_artifact_identity() {
    let temp = tempfile::Builder::new()
        .prefix("openout.")
        .tempdir_in(".")
        .expect("temporary output root");
    let failed = Path::new(temp.path().file_name().expect("relative temporary name"));
    let replacement = failed.join("replacement.out");
    let source = format!(
        "\\setbox0=\\hbox{{\\openout2={} \\write2{{x}}\\closeout2}}\
         \\shipout\\copy0\\end",
        failed.display()
    );
    let mut session = session(&source);
    let attempt = session.compile_attempt();
    assert!(
        matches!(attempt, CompileAttemptResult::Complete(_)),
        "{attempt:#?}"
    );
    let finalization = session
        .into_accepted_finalization()
        .expect("accepted finalization");
    let original_hash = finalization.completion.pages()[0].artifact().hash();
    let publication = finalization
        .completion
        .into_publication()
        .expect("prepare accepted publication");
    let mut destination = World::real_with_artifact_dir(temp.path().join("artifacts"));
    let mut plan =
        crate::PlannedFinalization::new(publication, Vec::new()).expect("empty driver plan");
    let crate::FinalizationCommit::Retry {
        plan: retained,
        failure,
    } = plan
        .commit_effects_retryable(&mut destination)
        .expect("retry-safe open failure")
    else {
        panic!("directory open must retain finalization");
    };
    assert!(destination.committed_artifacts().is_empty());
    let context_lines = failure.message().lines().collect::<Vec<_>>();
    assert!(context_lines.len() >= 2);
    let context_lines = &context_lines[context_lines.len() - 2..];
    // §318 prints the location descriptor before §316 starts measuring, so
    // §317's `...` lands after it rather than replacing it.
    assert!(
        context_lines[0].starts_with("l.1 ..."),
        "{:?}",
        failure.message()
    );
    assert!(context_lines[0].ends_with("\\shipout\\copy0\\end"));
    assert_eq!(context_lines[0].chars().count(), 50);
    assert_eq!(context_lines[1].chars().count(), 50);
    plan = *retained;
    plan.retarget_stream_open(&failure, &replacement)
        .expect("retarget prepared page");
    assert!(matches!(
        plan.commit_effects_retryable(&mut destination)
            .expect("replacement succeeds"),
        crate::FinalizationCommit::Committed(_)
    ));

    let [published] = destination.committed_artifacts() else {
        panic!("one page publishes after the open succeeds");
    };
    assert_ne!(published.hash(), original_hash);
    let page = tex_out::PageArtifact::from_bytes(published.bytes()).expect("published page parses");
    assert!(page.effects.iter().any(|effect| {
        matches!(
            effect,
            tex_out::PageEffect::OpenOut { stream: 2, path }
                if path == &replacement.to_string_lossy()
        )
    }));
}

#[test]
#[allow(clippy::disallowed_methods)] // Exercises authoritative finalization retry context.
fn finalization_retry_context_traverses_nested_sources_with_limit() {
    let temp = tempfile::Builder::new()
        .prefix("openout-context.")
        .tempdir_in(".")
        .expect("temporary output root");
    let failed = Path::new(temp.path().file_name().expect("relative temporary name"));
    let root = "\\errorcontextlines=0 \\input middle ROOT-CONTEXT\\end";
    let leaf = format!(
        "\\setbox0=\\hbox{{\\openout2={} }}\\shipout\\copy0 LEAF-CONTEXT",
        failed.display()
    );
    let mut session = session(root);
    session
        .add_user_file("middle.tex", br"\input leaf MIDDLE-CONTEXT".to_vec())
        .expect("middle input");
    session
        .add_user_file("leaf.tex", leaf.into_bytes())
        .expect("leaf input");
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
    let finalization = session
        .into_accepted_finalization()
        .expect("accepted finalization");
    let publication = finalization
        .completion
        .into_publication()
        .expect("prepare accepted publication");
    let mut destination = World::real_with_artifact_dir(temp.path().join("artifacts"));
    let plan = crate::PlannedFinalization::new(publication, Vec::new()).expect("empty driver plan");
    let crate::FinalizationCommit::Retry { failure, .. } = plan
        .commit_effects_retryable(&mut destination)
        .expect("retry-safe open failure")
    else {
        panic!("directory open must retain finalization");
    };
    let context = failure.message();
    assert!(context.contains("LEAF-CONTEXT"), "{context:?}");
    // tex.web §310's `bottom_line` stops at the first real-file level. The
    // enclosing files therefore do not consume the `\errorcontextlines`
    // budget and cannot cause an omission marker. This is the finalization
    // counterpart of tex-exec's canonical
    // `out_what_retry_show_context_stops_at_the_innermost_open_file` test.
    assert!(!context.contains("MIDDLE-CONTEXT"), "{context:?}");
    assert!(!context.contains("ROOT-CONTEXT"), "{context:?}");
    assert!(!context.contains("\n..."), "{context:?}");
}

#[test]
#[allow(clippy::disallowed_methods)] // Exercises ordered real-backend occurrence identity.
fn retry_retargets_exact_cross_page_open_occurrence_atomically() {
    let temp = tempfile::Builder::new()
        .prefix("openout-duplicate.")
        .tempdir_in(".")
        .expect("temporary output root");
    let target =
        Path::new(temp.path().file_name().expect("relative temporary name")).join("same.out");
    let blocked =
        Path::new(temp.path().file_name().expect("relative temporary name")).join("blocked.out");
    std::fs::create_dir(&blocked).expect("second output target is unavailable");
    let replacement = Path::new(temp.path().file_name().expect("relative temporary name"))
        .join("replacement.out");
    let source = format!(
        "\\shipout\\hbox{{A\\openout1={0} }}\
         \\shipout\\hbox{{B\\openout1={1} \\openout1={1}\
         \\write16{{same}}\\write1{{same}}\\write1{{second}}}}\\end",
        target.display(),
        blocked.display()
    );
    let mut session = session(&source);
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
    let finalization = session
        .into_accepted_finalization()
        .expect("accepted finalization");
    assert_eq!(finalization.completion.pages().len(), 2);
    let publication = finalization
        .completion
        .into_publication()
        .expect("prepare accepted publication");
    let mut destination = World::real_with_artifact_dir(temp.path().join("artifacts"));
    let mut plan =
        crate::PlannedFinalization::new(publication, Vec::new()).expect("empty driver plan");
    let crate::FinalizationCommit::Retry {
        plan: retained,
        failure,
    } = plan
        .commit_effects_retryable(&mut destination)
        .expect("second occurrence suspends")
    else {
        panic!("second occurrence must fail");
    };
    assert_eq!(failure.slot(), Some(tex_state::StreamSlot::new(1)));
    assert_eq!(failure.path(), Some(blocked.as_path()));
    plan = *retained;
    plan.retarget_stream_open(&failure, &replacement)
        .expect("exact occurrence retargets");

    assert_eq!(plan.pages().len(), 2);
    let opens = plan
        .pages()
        .iter()
        .map(|prepared| {
            let page = tex_out::PageArtifact::from_bytes(prepared.artifact().bytes())
                .expect("prepared page parses");
            page.effects
                .iter()
                .filter_map(|effect| match effect {
                    tex_out::PageEffect::OpenOut { stream, path } => Some((*stream, path.clone())),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        opens,
        [
            vec![(1, target.to_string_lossy().into_owned())],
            vec![
                (1, replacement.to_string_lossy().into_owned()),
                (1, blocked.to_string_lossy().into_owned()),
            ],
        ]
    );
    let second_page = tex_out::PageArtifact::from_bytes(plan.pages()[1].artifact().bytes())
        .expect("second prepared page parses");
    let same_writes = second_page
        .effects
        .iter()
        .filter_map(|effect| match effect {
            tex_out::PageEffect::Write { sink, text } if text == "same\n" => Some(*sink),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        same_writes,
        [
            tex_out::EffectSink::TerminalAndLog,
            tex_out::EffectSink::Stream(1),
        ],
        "equal write text with different sinks cannot influence OpenOut identity"
    );
    let retargeted_hashes = plan
        .pages()
        .iter()
        .map(|page| page.artifact().hash())
        .collect::<Vec<_>>();
    assert!(
        plan.retarget_stream_open(&failure, Path::new("stale.out"))
            .is_err(),
        "stale occurrence identity must fail"
    );
    assert_eq!(
        plan.pages()
            .iter()
            .map(|page| page.artifact().hash())
            .collect::<Vec<_>>(),
        retargeted_hashes,
        "stale failure is atomic for prepared history"
    );
    let crate::FinalizationCommit::Retry {
        plan: retained,
        failure: second_failure,
    } = plan
        .commit_effects_retryable(&mut destination)
        .expect("the other blocked occurrence remains retryable")
    else {
        panic!("the other blocked occurrence must retain finalization");
    };
    assert_eq!(second_failure.slot(), Some(tex_state::StreamSlot::new(1)));
    assert_eq!(second_failure.path(), Some(blocked.as_path()));
    assert_ne!(second_failure.ordinal(), failure.ordinal());
    plan = *retained;
    plan.retarget_stream_open(&second_failure, &replacement)
        .expect("second exact occurrence retargets");
    plan.commit_effects(&mut destination)
        .expect("both retargeted occurrences publish");
    assert_eq!(destination.committed_artifacts().len(), 2);
}

#[test]
#[allow(clippy::disallowed_methods)] // Exercises a real failure after an incremental edit.
fn incremental_effect_prefix_edit_preserves_openout_retry_identity() {
    let temp = tempfile::Builder::new()
        .prefix("openout-splice.")
        .tempdir_in(".")
        .expect("temporary output root");
    let target =
        Path::new(temp.path().file_name().expect("relative temporary name")).join("blocked.out");
    std::fs::create_dir(&target).expect("unavailable output target");
    let mut original = "\\message{prefix}".to_owned();
    for page in 0..20 {
        original.push_str(&format!("\\shipout\\hbox{{stable-{page}}}"));
    }
    original.push_str(&format!(
        "\\shipout\\hbox{{B\\openout2={0} \\openout2={0}}}\\end",
        target.display()
    ));
    let mut session = session(&original);
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
    apply_text_replacement(
        &mut session,
        2,
        &original,
        "\\message{prefix}",
        "\\message{prefix}\\message{inserted}",
    );
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
    assert_eq!(session.revision(), Some(RevisionId::new(2)));

    let finalization = session
        .into_accepted_finalization()
        .expect("accepted finalization");
    let hashes_before = finalization
        .completion
        .pages()
        .iter()
        .map(|page| page.artifact().hash())
        .collect::<Vec<_>>();
    let publication = finalization
        .completion
        .into_publication()
        .expect("prepare accepted publication");
    let mut destination = World::real_with_artifact_dir(temp.path().join("artifacts"));
    let plan = crate::PlannedFinalization::new(publication, Vec::new()).expect("empty driver plan");
    let crate::FinalizationCommit::Retry { mut plan, failure } = plan
        .commit_effects_retryable(&mut destination)
        .expect("unavailable adopted OpenOut suspends")
    else {
        panic!("unavailable adopted OpenOut must retry");
    };
    let remaining_before = plan.remaining_effects().len();
    let replacement = Path::new(temp.path().file_name().expect("relative temporary name"))
        .join("replacement.out");
    plan.retarget_stream_open(&failure, &replacement)
        .expect("rebased exact occurrence retargets");
    assert_eq!(
        plan.remaining_effects().len(),
        remaining_before,
        "retry retarget preserves effect history length"
    );
    assert_ne!(
        plan.pages()
            .iter()
            .map(|page| page.artifact().hash())
            .collect::<Vec<_>>(),
        hashes_before,
        "only successful retarget changes prepared artifact identity"
    );
    (*plan)
        .commit_effects(&mut destination)
        .expect("replacement finalization succeeds");
}

#[test]
fn finalization_rejects_a_session_without_accepted_output() {
    let error = session("\\end")
        .into_accepted_finalization()
        .err()
        .expect("unfinished session must not finalize");
    assert!(error.to_string().contains("no completed accepted output"));
}

#[test]
fn initial_main_file_accepts_legacy_8_bit_bytes() {
    let mut session = VirtualCompileSession::new(SessionOptions::default()).expect("session");
    let mut source = b"\\catcode237=12 \\message{legacy:".to_vec();
    source.push(0xed);
    source.extend_from_slice(b"}\\end");
    session
        .add_user_file("main.tex", source)
        .expect("main file");

    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("legacy-byte main file should compile");
    };
    assert!(String::from_utf8_lossy(&output.terminal).contains("legacy:"));
}

#[test]
fn expanded_definition_classifies_noexpand_across_editor_fragment_origins() {
    let source = "\\def\\a{A}\n\\toks0={T}\n\\edef\\e{\\noexpand\\a:\\the\\toks0:\\number7}\n\\show\\e\n\\end\n";
    let CompileAttemptResult::Complete(output) = session(source).compile_attempt() else {
        panic!("expanded definition should compile in a retained virtual session");
    };

    assert!(String::from_utf8_lossy(&output.terminal).contains("->\\a :T:7."));
}

fn requests(result: CompileAttemptResult) -> Vec<FileRequest> {
    match result {
        CompileAttemptResult::NeedResources(resources) => resources
            .required
            .into_iter()
            .filter_map(|request| match request {
                ResourceRequest::File(request) => Some(request),
                ResourceRequest::Font(_) | ResourceRequest::PkFont(_) => None,
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

fn probes(result: CompileAttemptResult) -> Vec<FileRequest> {
    match result {
        CompileAttemptResult::NeedResources(resources) => {
            assert!(
                resources.required.is_empty(),
                "probe was promoted to required"
            );
            resources
                .probes
                .into_iter()
                .filter_map(|request| match request {
                    ResourceRequest::File(request) => Some(request),
                    ResourceRequest::Font(_) | ResourceRequest::PkFont(_) => None,
                })
                .collect()
        }
        other => panic!("expected missing file probes, got {other:#?}"),
    }
}

fn apply_text_replacement(
    session: &mut VirtualCompileSession,
    revision: u64,
    source: &str,
    needle: &str,
    replacement: &str,
) -> String {
    let start = source.find(needle).expect("replacement text");
    let mut next = source.to_owned();
    next.replace_range(start..start + needle.len(), replacement);
    session
        .apply_patch(SourcePatch {
            next_revision: RevisionId::new(revision),
            base_revision: session.revision().expect("accepted revision"),
            expected_hash: session.content_hash().expect("accepted source hash"),
            range: start..start + needle.len(),
            replacement: replacement.to_owned(),
        })
        .expect("valid replacement patch");
    next
}

fn answer_single_file(result: CompileAttemptResult, bytes: Option<&[u8]>) -> ResourceResponse {
    let resources = match result {
        CompileAttemptResult::NeedResources(resources) => resources,
        other => panic!("expected one file resource, got {other:#?}"),
    };
    let mut requests = resources.required;
    requests.extend(resources.probes);
    let [ResourceRequest::File(request)] = requests.as_slice() else {
        panic!("expected exactly one file resource: {requests:#?}");
    };
    bytes.map_or_else(
        || ResourceResponse::FileUnavailable(request.key().clone()),
        |bytes| {
            ResourceResponse::File(ResolvedFile {
                request: request.key().clone(),
                virtual_path: format!("/texlive/{}", request.key().name()),
                bytes: bytes.to_vec(),
                expected_digest: None,
            })
        },
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AcceptedSessionState {
    revision: RevisionId,
    content_hash: ContentHash,
    output: MemoryRunOutput,
    generated: Option<Vec<u8>>,
    history: Vec<(
        RevisionId,
        tex_incr::BoundaryKey,
        usize,
        usize,
        Option<tex_exec::ReachableStateIdentity>,
    )>,
    observations: crate::AcceptedInputObservationLedger,
}

fn accepted_session_state(session: &VirtualCompileSession) -> AcceptedSessionState {
    let snapshot = session.workspace.snapshot();
    let generated_path = VirtualPath::user("state.aux").expect("generated path");
    AcceptedSessionState {
        revision: session.revision().expect("accepted revision"),
        content_hash: session.content_hash().expect("accepted content hash"),
        output: session
            .accepted_output
            .clone()
            .expect("accepted session output"),
        generated: snapshot
            .get(&generated_path)
            .expect("generated lookup")
            .map(|file| file.bytes().to_vec()),
        history: session
            .incremental
            .as_ref()
            .expect("accepted incremental session")
            .history()
            .iter()
            .map(|record| {
                (
                    record.revision(),
                    record.key(),
                    record.effect_prefix(),
                    record.artifact_prefix(),
                    record.reachable_state_identity(),
                )
            })
            .collect(),
        observations: session
            .accepted_input_observations()
            .expect("accepted observations"),
    }
}

fn generated_input_fallback_session() -> (VirtualCompileSession<'static>, String) {
    let source = concat!(
        "\\input state.aux ",
        "\\immediate\\openout1=state.aux ",
        "\\immediate\\write1{\\string\\message{new-input}} ",
        "\\immediate\\closeout1 \\message{accepted}\\end"
    );
    let mut session = session(source);
    session
        .add_user_file("state.aux", b"\\message{old-input}\n".to_vec())
        .expect("old incoming generated input");
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
    (session, source.to_owned())
}

#[test]
fn accepted_dependencies_record_required_positive_and_shadowing_negative_paths() {
    let mut session = session("\\input generated.aux \\end");
    let requested = requests(session.compile_attempt());
    let [request] = requested.as_slice() else {
        panic!("expected one generated-input request");
    };
    let request = request.key().clone();
    assert!(session.accepted_input_dependencies().next().is_none());

    session
        .provide_resources(vec![ResourceResponse::File(ResolvedFile {
            request,
            virtual_path: "/texlive/generated.aux".to_owned(),
            bytes: b"\\relax".to_vec(),
            expected_digest: None,
        })])
        .expect("provide generated-input fallback");
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));

    let dependencies = session.accepted_input_dependencies().collect::<Vec<_>>();
    assert_eq!(dependencies.len(), 2);
    assert_eq!(dependencies[0].path(), Path::new("/job/generated.aux"));
    assert_eq!(
        dependencies[0].outcome(),
        tex_state::InputDependencyOutcome::Missing
    );
    assert_eq!(
        dependencies[0].access(),
        tex_state::InputDependencyAccess::RequiredRead
    );
    assert_eq!(dependencies[1].path(), Path::new("/texlive/generated.aux"));
    assert!(matches!(
        dependencies[1].outcome(),
        tex_state::InputDependencyOutcome::Present(_)
    ));
    assert_eq!(
        dependencies[1].access(),
        tex_state::InputDependencyAccess::RequiredRead
    );

    let ledger = session
        .accepted_input_observations()
        .expect("accepted observation ledger");
    assert_eq!(
        ledger.schema_version(),
        crate::ACCEPTED_INPUT_OBSERVATION_SCHEMA_VERSION
    );
    assert_eq!(ledger.revision(), RevisionId::new(1));
    assert_eq!(ledger.observations().len(), 3);
    assert_eq!(
        ledger.observations()[0].namespace(),
        crate::InputObservationNamespace::Authored
    );
    assert_eq!(
        ledger.observations()[2].namespace(),
        crate::InputObservationNamespace::Distribution
    );
    assert_eq!(ledger.observations()[2].resource_kind(), FileKind::TexInput);
    assert_eq!(
        ledger.observations()[2].phase(),
        crate::InputObservationPhase::Tex
    );
    assert_eq!(ledger.observations()[2].project_pass(), None);
}

#[test]
fn accepted_dependencies_record_authoritative_probe_but_not_its_resource_wait() {
    let mut session = session("\\openin0=generated.aux \\ifeof0 \\message{missing}\\fi \\end");
    let requested = probes(session.compile_attempt());
    let [request] = requested.as_slice() else {
        panic!("expected one generated-input probe");
    };
    let request = request.key().clone();
    assert!(session.accepted_input_dependencies().next().is_none());

    session
        .provide_resources(vec![ResourceResponse::FileUnavailable(request)])
        .expect("provide authoritative absence");
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));

    let dependencies = session.accepted_input_dependencies().collect::<Vec<_>>();
    assert_eq!(dependencies.len(), 1);
    assert_eq!(dependencies[0].path(), Path::new("/job/generated.aux"));
    assert_eq!(
        dependencies[0].outcome(),
        tex_state::InputDependencyOutcome::Missing
    );
    assert_eq!(
        dependencies[0].access(),
        tex_state::InputDependencyAccess::AuthoritativeProbe
    );
}

#[test]
fn generated_probe_missing_to_present_restarts_from_job_start_and_matches_cold() {
    let source = concat!(
        "\\openin0=state.aux ",
        "\\ifeof0 \\message{missing}\\else \\message{present}\\fi \\closein0 ",
        "\\immediate\\openout1=state.aux \\immediate\\write1{generated} ",
        "\\immediate\\closeout1 \\message{old-tail}\\end"
    );
    let mut incremental = session(source);
    let response = answer_single_file(incremental.compile_attempt(), None);
    incremental
        .provide_resources(vec![response])
        .expect("authoritative initial absence");
    assert!(matches!(
        incremental.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
    let generated_path = VirtualPath::user("state.aux").expect("generated path");
    let generated = incremental
        .workspace
        .snapshot()
        .get(&generated_path)
        .expect("live accepted snapshot")
        .expect("generated file")
        .bytes()
        .to_vec();

    let next = apply_text_replacement(&mut incremental, 2, source, "old-tail", "new-tail");
    let CompileAttemptResult::Complete(actual) = incremental.compile_attempt() else {
        panic!("generated file must satisfy the next probe");
    };
    assert_eq!(
        incremental
            .reuse_metrics()
            .expect("replacement metrics")
            .reexecuted_bytes,
        next.len()
    );
    let restart = incremental
        .reuse_metrics()
        .expect("replacement metrics")
        .restart_boundary
        .expect("generated-input change selects the retained startup root");
    assert_eq!(restart.position, 0);
    assert_eq!(restart.boundary, tex_exec::EngineBoundary::JobStart);

    let mut cold = session(&next);
    cold.add_user_file("state.aux", generated)
        .expect("incoming generated snapshot");
    let CompileAttemptResult::Complete(expected) = cold.compile_attempt() else {
        panic!("cold comparison must complete");
    };
    assert_eq!(actual, expected);
}

#[test]
fn changed_required_generated_input_retries_one_job_start_candidate_and_matches_cold() {
    let source = concat!(
        "\\input state.aux ",
        "\\immediate\\openout1=state.aux ",
        "\\immediate\\write1{\\string\\message{new-input}} ",
        "\\immediate\\closeout1 \\message{old-tail}\\end"
    );
    let mut incremental = session(source);
    incremental
        .add_user_file("state.aux", b"\\message{old-input}\n".to_vec())
        .expect("old incoming input");
    assert!(matches!(
        incremental.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
    let generated_path = VirtualPath::user("state.aux").expect("generated path");
    let generated = incremental
        .workspace
        .snapshot()
        .get(&generated_path)
        .expect("live accepted snapshot")
        .expect("changed generated file")
        .bytes()
        .to_vec();

    let next = apply_text_replacement(
        &mut incremental,
        2,
        source,
        "\\message{old-tail}",
        "\\input later \\message{new-tail}",
    );
    let response = answer_single_file(incremental.compile_attempt(), Some(b"\\relax\n"));
    assert_eq!(
        incremental
            .candidate
            .as_ref()
            .expect("suspended private candidate")
            .suspension_serial,
        1
    );
    incremental
        .provide_resources(vec![response])
        .expect("resume resource");
    let CompileAttemptResult::Complete(actual) = incremental.compile_attempt() else {
        panic!("resumed candidate must complete");
    };
    assert_eq!(
        incremental
            .reuse_metrics()
            .expect("replacement metrics")
            .reexecuted_bytes,
        next.len()
    );
    let restart = incremental
        .reuse_metrics()
        .expect("replacement metrics")
        .restart_boundary
        .expect("required-input change selects the retained startup root");
    assert_eq!(restart.position, 0);
    assert_eq!(restart.boundary, tex_exec::EngineBoundary::JobStart);

    let mut cold = session(&next);
    cold.add_user_file("state.aux", generated)
        .expect("incoming generated snapshot");
    let response = answer_single_file(cold.compile_attempt(), Some(b"\\relax\n"));
    cold.provide_resources(vec![response])
        .expect("cold comparison resource");
    let CompileAttemptResult::Complete(expected) = cold.compile_attempt() else {
        panic!("cold comparison must complete");
    };
    assert_eq!(actual, expected);
}

#[test]
fn job_start_fallback_failure_and_output_limit_preserve_every_accepted_root() {
    let (mut failed, source) = generated_input_fallback_session();
    let accepted = accepted_session_state(&failed);
    apply_text_replacement(
        &mut failed,
        2,
        &source,
        "\\message{accepted}",
        "\\input candidate-failed",
    );
    let request = requests(failed.compile_attempt());
    assert_eq!(accepted_session_state(&failed), accepted);
    failed
        .provide_resources(vec![ResourceResponse::FileUnavailable(
            request[0].key().clone(),
        )])
        .expect("authoritative candidate failure");
    assert!(matches!(
        failed.compile_attempt(),
        CompileAttemptResult::Error(CompileError::Diagnostic(_))
    ));
    assert_eq!(accepted_session_state(&failed), accepted);
    assert_eq!(
        failed.compile_attempt(),
        CompileAttemptResult::Complete(accepted.output.clone())
    );

    let (mut limited, source) = generated_input_fallback_session();
    let accepted = accepted_session_state(&limited);
    limited.limits.output_bytes = memory_run_output_bytes(&accepted.output).saturating_add(32);
    let replacement = format!("\\message{{{}}}", "x".repeat(512));
    apply_text_replacement(
        &mut limited,
        2,
        &source,
        "\\message{accepted}",
        &replacement,
    );
    assert!(matches!(
        limited.compile_attempt(),
        CompileAttemptResult::Error(CompileError::LimitExceeded { .. })
    ));
    assert_eq!(accepted_session_state(&limited), accepted);
}

#[test]
fn job_start_fallback_suspension_no_progress_and_cancellation_preserve_accepted_state() {
    let (mut no_progress, source) = generated_input_fallback_session();
    let accepted = accepted_session_state(&no_progress);
    apply_text_replacement(
        &mut no_progress,
        2,
        &source,
        "\\message{accepted}",
        "\\input later \\message{candidate}",
    );
    let first_request = requests(no_progress.compile_attempt());
    assert_eq!(accepted_session_state(&no_progress), accepted);
    assert_eq!(no_progress.revision(), Some(RevisionId::new(1)));
    assert!(matches!(
        no_progress.compile_attempt(),
        CompileAttemptResult::Error(CompileError::NoProgress)
    ));
    assert_eq!(accepted_session_state(&no_progress), accepted);

    let (mut cancelled, source) = generated_input_fallback_session();
    let accepted = accepted_session_state(&cancelled);
    apply_text_replacement(
        &mut cancelled,
        2,
        &source,
        "\\message{accepted}",
        "\\input later \\message{candidate}",
    );
    let request = requests(cancelled.compile_attempt());
    assert_eq!(request, first_request);
    assert_eq!(accepted_session_state(&cancelled), accepted);
    assert!(cancelled.discard_suspended_candidate());
    assert_eq!(accepted_session_state(&cancelled), accepted);
    assert_eq!(requests(cancelled.compile_attempt()), request);
    assert!(cancelled.cancel_pending_patch());
    assert_eq!(accepted_session_state(&cancelled), accepted);
    assert_eq!(
        cancelled.compile_attempt(),
        CompileAttemptResult::Complete(accepted.output.clone())
    );
}

#[test]
fn resumed_job_start_fallback_publishes_root_stage_and_revision_together() {
    let (mut session, source) = generated_input_fallback_session();
    let accepted = accepted_session_state(&session);
    let next = apply_text_replacement(
        &mut session,
        2,
        &source,
        "\\message{accepted}",
        "\\input later \\message{candidate}",
    );
    let response = answer_single_file(session.compile_attempt(), Some(b"\\relax\n"));
    assert_eq!(accepted_session_state(&session), accepted);
    session
        .provide_resources(vec![response])
        .expect("resume fallback candidate");
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("resumed fallback must complete");
    };
    assert_ne!(accepted_session_state(&session), accepted);
    assert_eq!(session.revision(), Some(RevisionId::new(2)));
    assert_eq!(
        session
            .workspace
            .snapshot()
            .get(&session.main_path)
            .expect("root lookup")
            .expect("accepted root")
            .bytes(),
        next.as_bytes()
    );
    assert_eq!(session.accepted_output.as_ref(), Some(&output));
    let history = session
        .incremental
        .as_ref()
        .expect("incremental session")
        .history();
    assert_eq!(history[0].revision(), RevisionId::new(1));
    assert_eq!(
        history[0].key().boundary,
        tex_exec::EngineBoundary::JobStart
    );
    assert!(
        history[1..]
            .iter()
            .all(|record| record.revision() == RevisionId::new(2))
    );
    assert_eq!(
        session
            .accepted_input_observations()
            .expect("accepted observations")
            .revision(),
        RevisionId::new(2)
    );
}

#[test]
fn resumed_job_start_fallback_preserves_legacy_root_byte_representation() {
    let mut source = concat!(
        "\\input state.aux ",
        "\\immediate\\openout1=state.aux ",
        "\\immediate\\write1{\\string\\message{new-input}} ",
        "\\immediate\\closeout1 %"
    )
    .as_bytes()
    .to_vec();
    source.push(0xff);
    source.extend_from_slice(b"\n\\message{old}\\end");
    let mut session = VirtualCompileSession::new(SessionOptions::default()).expect("session");
    session
        .add_user_file("main.tex", source.clone())
        .expect("legacy root");
    session
        .add_user_file("state.aux", b"\\message{old-input}\n".to_vec())
        .expect("old incoming generated input");
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));

    let old = b"\\message{old}";
    let physical_start = source
        .windows(old.len())
        .position(|window| window == old)
        .expect("old message");
    let editor_start = session
        .incremental
        .as_ref()
        .expect("accepted incremental session")
        .source()
        .find("\\message{old}")
        .expect("editor message");
    let replacement = "\\input later \\message{new}";
    session
        .apply_patch(SourcePatch {
            next_revision: RevisionId::new(2),
            base_revision: RevisionId::new(1),
            expected_hash: session.content_hash().expect("accepted hash"),
            range: editor_start..editor_start + old.len(),
            replacement: replacement.to_owned(),
        })
        .expect("legacy patch");
    source.splice(
        physical_start..physical_start + old.len(),
        replacement.bytes(),
    );
    let response = answer_single_file(session.compile_attempt(), Some(b"\\relax\n"));
    session
        .provide_resources(vec![response])
        .expect("resume legacy candidate");
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
    assert_eq!(
        session
            .workspace
            .snapshot()
            .get(&session.main_path)
            .expect("root lookup")
            .expect("accepted root")
            .bytes(),
        source
    );
}

#[test]
fn generated_probe_present_to_missing_restarts_from_job_start_and_matches_cold() {
    let producing = concat!(
        "\\immediate\\openout1=state.aux \\immediate\\write1{present} ",
        "\\immediate\\closeout1 \\message{one}\\end"
    );
    let consuming = concat!(
        "\\openin0=state.aux ",
        "\\ifeof0 \\message{missing}\\else \\message{present}\\fi \\closein0 ",
        "\\message{two}\\end"
    );
    let mut incremental = session(producing);
    assert!(matches!(
        incremental.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
    incremental
        .apply_patch(SourcePatch {
            next_revision: RevisionId::new(2),
            base_revision: RevisionId::new(1),
            expected_hash: incremental.content_hash().expect("accepted source hash"),
            range: 0..producing.len(),
            replacement: consuming.to_owned(),
        })
        .expect("replace producer with consumer");
    assert!(matches!(
        incremental.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
    assert!(
        incremental
            .workspace
            .snapshot()
            .get(&VirtualPath::user("state.aux").expect("generated path"))
            .expect("live accepted snapshot")
            .is_none()
    );

    let next = apply_text_replacement(&mut incremental, 3, consuming, "two", "three");
    let response = answer_single_file(incremental.compile_attempt(), None);
    incremental
        .provide_resources(vec![response])
        .expect("authoritative current absence");
    let CompileAttemptResult::Complete(actual) = incremental.compile_attempt() else {
        panic!("missing-input candidate must complete");
    };
    assert_eq!(
        incremental
            .reuse_metrics()
            .expect("replacement metrics")
            .reexecuted_bytes,
        next.len()
    );
    let restart = incremental
        .reuse_metrics()
        .expect("replacement metrics")
        .restart_boundary
        .expect("missing generated input selects the retained startup root");
    assert_eq!(restart.position, 0);
    assert_eq!(restart.boundary, tex_exec::EngineBoundary::JobStart);

    let mut cold = session(&next);
    let response = answer_single_file(cold.compile_attempt(), None);
    cold.provide_resources(vec![response])
        .expect("cold authoritative absence");
    let CompileAttemptResult::Complete(expected) = cold.compile_attempt() else {
        panic!("cold comparison must complete");
    };
    assert_eq!(actual, expected);
}

#[test]
fn unchanged_consumed_input_and_changed_unconsumed_output_use_cold_recompute() {
    let source = concat!(
        "\\input stable.tex \\font\\f=cmr10 \\f reusable paragraph\\par ",
        "\\immediate\\openout1=unused.aux \\immediate\\write1{old-output} ",
        "\\immediate\\closeout1 \\message{tail}\\end"
    );
    let mut session = session(source);
    session
        .add_user_file("stable.tex", b"\\relax\n".to_vec())
        .expect("stable consumed input");
    session
        .add_user_file("cmr10.tfm", CMR10.to_vec())
        .expect("paragraph font");
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));

    let _next = apply_text_replacement(&mut session, 2, source, "old-output", "new-output");
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("edited output candidate must complete");
    };
    assert!(
        output
            .files
            .iter()
            .any(|file| { file.path.ends_with("unused.aux") && file.bytes == b"new-output\n" })
    );
    let reuse = session.reuse_metrics().expect("incremental reuse metrics");
    assert_eq!(
        reuse.execution_path,
        tex_incr::RevisionExecutionPath::SlowEdit
    );
    let restart = reuse
        .restart_boundary
        .expect("slow edit forks the retained startup checkpoint");
    assert_eq!(restart.position, 0);
    assert_eq!(restart.boundary, tex_exec::EngineBoundary::JobStart);
}

#[test]
fn discarded_candidate_cannot_change_accepted_dependencies() {
    let mut session = session("\\end");
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
    let base_revision = session.revision().expect("accepted revision");
    let expected_hash = session.content_hash().expect("accepted source hash");
    session
        .apply_patch(SourcePatch {
            next_revision: RevisionId::new(base_revision.raw() + 1),
            base_revision,
            expected_hash,
            range: 0..4,
            replacement: "\\openin0=generated.aux \\ifeof0 \\fi \\end".to_owned(),
        })
        .expect("start candidate revision");
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::NeedResources(_)
    ));
    assert!(session.accepted_input_dependencies().next().is_none());
    assert!(session.discard_suspended_candidate());
    assert!(session.accepted_input_dependencies().next().is_none());
    let ledger = session
        .accepted_input_observations()
        .expect("accepted ledger");
    assert_eq!(ledger.observations().len(), 1);
    assert_eq!(ledger.observations()[0].path().as_str(), "/job/main.tex");
}

fn minimal_vf_with_local(name: &[u8]) -> Vec<u8> {
    let mut bytes = vec![247, 202, 0];
    bytes.extend_from_slice(&0_u32.to_be_bytes());
    bytes.extend_from_slice(&(10_i32 << 20).to_be_bytes());
    bytes.extend_from_slice(&[243, 0]);
    bytes.extend_from_slice(&0_u32.to_be_bytes());
    bytes.extend_from_slice(&(1_i32 << 20).to_be_bytes());
    bytes.extend_from_slice(&(10_i32 << 20).to_be_bytes());
    bytes.push(0);
    bytes.push(u8::try_from(name.len()).expect("short fixture font name"));
    bytes.extend_from_slice(name);
    bytes.push(248);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(248);
    }
    bytes
}

fn fixture_encoding() -> Vec<u8> {
    let mut bytes = b"/FixtureEncoding [".to_vec();
    for _ in 0..256 {
        bytes.extend_from_slice(b" /.notdef");
    }
    bytes.extend_from_slice(b" ] def\n");
    bytes
}

fn fixture_pfb() -> Vec<u8> {
    let mut bytes = vec![0x80, 0x01];
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.push(b'a');
    bytes.extend_from_slice(&[0x80, 0x02]);
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&[0x80, 0x03]);
    bytes
}

fn pdf_raw_object_file_session() -> VirtualCompileSession<'static> {
    let mut session = VirtualCompileSession::new(SessionOptions {
        engine: EngineMode::PdfTex,
        outputs: OutputCapabilitySet::PDF,
        ..SessionOptions::default()
    })
    .expect("PDF session");
    session
        .add_user_file(
            "main.tex",
            concat!(
                "\\pdfoutput=1 ",
                "\\pdfcompresslevel=0 ",
                "\\pdfobjcompresslevel=0 ",
                "\\immediate\\pdfobj stream file {t1.cmap} ",
                "\\shipout\\hbox{}\\end",
            )
            .as_bytes()
            .to_vec(),
        )
        .expect("main source");
    session
}

#[test]
fn pdf_raw_object_file_uses_an_identity_pinned_accepted_payload() {
    const T1_CMAP: &[u8] =
        b"/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\nendcmap\nend\nend\n";
    let mut session = pdf_raw_object_file_session();
    let requested = requests(session.compile_attempt());
    let [request] = requested.as_slice() else {
        panic!("expected one raw-object file request, got {requested:?}");
    };
    assert_eq!(request.key().kind(), FileKind::TexInput);
    assert_eq!(request.key().name(), "t1.cmap");
    assert_eq!(request.original_name(), "t1.cmap");
    session
        .provide_resolved_file(
            request.key().clone(),
            "/texlive/tex/latex/cmap/t1.cmap",
            T1_CMAP.to_vec(),
        )
        .expect("typed cmap-like payload");
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));

    let accepted = session
        .into_accepted_finalization()
        .expect("accepted PDF finalization");
    let entry = accepted
        .pdf_raw_object_file_receipt
        .entries
        .values()
        .next()
        .expect("raw-object file receipt");
    assert_eq!(entry.source_name, "t1.cmap");
    assert_eq!(entry.virtual_path, "/texlive/tex/latex/cmap/t1.cmap");
    assert_eq!(entry.content_id, FileContentId::for_bytes(T1_CMAP));
    assert_eq!(entry.bytes.as_slice(), T1_CMAP);
    assert!(matches!(
        &entry.source,
        PdfRawObjectFileSource::Resolved(key) if key == request.key()
    ));

    let pdf_completion = accepted.completion.pdf().expect("accepted PDF completion");
    let pdf = crate::pdf_from_accepted_artifacts_with_virtual_fonts(
        pdf_completion,
        &accepted.virtual_font_resources,
        &accepted.pdf_raw_object_file_receipt,
    )
    .expect("detached finalization uses accepted bytes");
    assert!(
        pdf.windows(T1_CMAP.len()).any(|window| window == T1_CMAP),
        "uncompressed PDF must contain the exact accepted cmap-like payload"
    );
}

#[test]
fn unavailable_pdf_raw_object_file_blocks_acceptance() {
    let mut session = pdf_raw_object_file_session();
    let requested = requests(session.compile_attempt());
    let [request] = requested.as_slice() else {
        panic!("expected one raw-object file request");
    };
    session
        .provide_resources(vec![ResourceResponse::FileUnavailable(
            request.key().clone(),
        )])
        .expect("authoritative unavailable payload");
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Error(CompileError::OutputCapability {
            capability: OutputCapability::Pdf,
            ref message,
        }) if message.contains("t1.cmap") && message.contains("unavailable")
    ));
    assert_eq!(
        session.revision(),
        None,
        "failed output closure is not accepted"
    );
    assert!(
        session.into_accepted_finalization().is_err(),
        "failed raw-object closure cannot cross the effect boundary"
    );
}

#[test]
fn detached_pdf_rejects_a_stale_raw_object_payload_identity() {
    let mut session = pdf_raw_object_file_session();
    let requested = requests(session.compile_attempt());
    let request = requested.first().expect("raw-object request");
    session
        .provide_resolved_file(
            request.key().clone(),
            "/texlive/t1.cmap",
            b"accepted cmap bytes".to_vec(),
        )
        .expect("raw-object payload");
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
    let mut accepted = session
        .into_accepted_finalization()
        .expect("accepted finalization");
    accepted
        .pdf_raw_object_file_receipt
        .entries
        .values_mut()
        .next()
        .expect("receipt entry")
        .bytes = b"changed after acceptance".to_vec();
    let error = crate::pdf_from_accepted_artifacts_with_virtual_fonts(
        accepted.completion.pdf().expect("accepted PDF completion"),
        &accepted.virtual_font_resources,
        &accepted.pdf_raw_object_file_receipt,
    )
    .expect_err("stale payload identity must fail before serialization");
    assert!(matches!(
        error,
        crate::PdfBuildError::RawObjectFilePayloadMismatch(_)
    ));
}

#[test]
fn pdf_virtual_font_closure_uses_typed_bounded_retries() {
    let mut session = VirtualCompileSession::new(SessionOptions {
        engine: EngineMode::PdfTex,
        outputs: OutputCapabilitySet::PDF,
        font_layout_policy: tex_fonts::FontLayoutPolicy::ClassicTfmExact,
        ..SessionOptions::default()
    })
    .expect("PDF session");
    session
        .add_user_file(
            "main.tex",
            b"\\pdfoutput=1 \\font\\root=cmr10\\relax \\root \\shipout\\hbox{A}\\end".to_vec(),
        )
        .expect("main source");

    let root_requests = requests(session.compile_attempt());
    let [root_tfm] = root_requests.as_slice() else {
        panic!("expected root TFM request");
    };
    assert_eq!(root_tfm.key().kind(), FileKind::Tfm);
    session
        .provide_resolved_file(root_tfm.key().clone(), "/texlive/cmr10.tfm", CMR10.to_vec())
        .expect("root TFM");
    let first_attempt = session.compile_attempt();
    let CompileAttemptResult::NeedResources(first) = first_attempt else {
        panic!("completed engine should discover PDF resources: {first_attempt:?}");
    };
    let vf = first
        .probes
        .iter()
        .find_map(|request| match request {
            ResourceRequest::File(request) if request.key().kind() == FileKind::VirtualFont => {
                Some(request.clone())
            }
            _ => None,
        })
        .expect("typed VF probe");
    assert_eq!(vf.key().name(), "cmr10.vf");
    assert_eq!(vf.original_name(), "cmr10.vf");
    assert!(first.required.is_empty());
    let vf_bytes = minimal_vf_with_local(b"cmsy10");
    session
        .provide_resolved_file(vf.key().clone(), "/texlive/cmr10.vf", vf_bytes.clone())
        .expect("virtual font");

    let local_requests = requests(session.compile_attempt());
    let [local_tfm] = local_requests.as_slice() else {
        panic!("expected local TFM request");
    };
    assert_eq!(local_tfm.key().kind(), FileKind::Tfm);
    assert_eq!(local_tfm.key().name(), "cmsy10.tfm");
    assert_eq!(local_tfm.original_name(), "cmsy10.tfm");
    session
        .provide_resolved_file(
            local_tfm.key().clone(),
            "/texlive/cmsy10.tfm",
            CMSY10.to_vec(),
        )
        .expect("local TFM");

    let local_probes = probes(session.compile_attempt());
    let [local_vf] = local_probes.as_slice() else {
        panic!("expected recursive VF probe");
    };
    assert_eq!(local_vf.key().name(), "cmsy10.vf");
    session
        .provide_resources(vec![ResourceResponse::FileUnavailable(
            local_vf.key().clone(),
        )])
        .expect("authoritative non-VF response");

    let map_requests = requests(session.compile_attempt());
    let [map] = map_requests.as_slice() else {
        panic!("expected default map request");
    };
    assert_eq!(map.key().kind(), FileKind::PdfFontMap);
    session
        .provide_resolved_file(
            map.key().clone(),
            "/texlive/pdftex.map",
            b"cmsy10 FixturePS <[fixture.enc <fixture.pfb\n".to_vec(),
        )
        .expect("font map");

    let resources = requests(session.compile_attempt());
    assert_eq!(resources.len(), 2);
    let encoding = resources
        .iter()
        .find(|request| request.key().kind() == FileKind::PdfEncoding)
        .expect("typed encoding request");
    let program = resources
        .iter()
        .find(|request| request.key().kind() == FileKind::PdfFontProgram)
        .expect("typed program request");
    session
        .provide_resolved_file(
            encoding.key().clone(),
            "/texlive/fixture.enc",
            fixture_encoding(),
        )
        .expect("encoding");
    session
        .provide_resolved_file(program.key().clone(), "/texlive/fixture.pfb", fixture_pfb())
        .expect("font program");

    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("PDF closure should complete");
    };
    assert_eq!(output.outputs, OutputCapabilitySet::PDF);
    assert!(output.dvi.is_empty());
    assert!(session.attempts() <= 7);
    let finalization = session
        .into_accepted_finalization()
        .expect("accepted resources");
    let cached = finalization
        .virtual_font_resources
        .virtual_fonts
        .get("cmr10")
        .expect("VF retained by logical identity");
    assert_eq!(
        cached.content_id,
        umber_vfs::FileContentId::for_bytes(&vf_bytes)
    );
    assert!(
        finalization
            .virtual_font_resources
            .local_tfms
            .contains_key("cmsy10")
    );
    assert!(
        finalization
            .virtual_font_resources
            .encodings
            .contains_key(b"fixture.enc".as_slice())
    );
    assert!(
        finalization
            .virtual_font_resources
            .type1_programs
            .contains_key(b"fixture.pfb".as_slice())
    );
    let receipt = &finalization.pdf_font_closure_receipt.entries;
    for (kind, name, resolved) in [
        (FileKind::VirtualFont, "cmr10.vf", true),
        (FileKind::Tfm, "cmsy10.tfm", true),
        (FileKind::VirtualFont, "cmsy10.vf", false),
        (FileKind::PdfFontMap, "pdftex.map", true),
        (FileKind::PdfEncoding, "fixture.enc", true),
        (FileKind::PdfFontProgram, "fixture.pfb", true),
    ] {
        assert!(receipt.iter().any(|entry| matches!(
            entry,
            PdfFontClosureReceiptEntry::File { request, outcome }
                if request.kind() == kind
                    && request.name() == name
                    && matches!(outcome, PdfFontClosureResourceOutcome::Resolved { .. }) == resolved
        )), "missing {kind:?} receipt for {name}");
    }
}

#[test]
fn pdf_bitmap_fallback_crosses_the_typed_session_boundary() {
    let mut session = VirtualCompileSession::new(SessionOptions {
        engine: EngineMode::PdfTex,
        outputs: OutputCapabilitySet::PDF,
        font_layout_policy: tex_fonts::FontLayoutPolicy::ClassicTfmExact,
        ..SessionOptions::default()
    })
    .expect("PDF session");
    session
        .add_user_file("cmr10.tfm", CMR10.to_vec())
        .expect("TFM");
    session
        .add_user_file(
            "main.tex",
            b"\\pdfoutput=1 \\font\\tenrm=cmr10\\relax \\tenrm \\shipout\\hbox{A}\\end".to_vec(),
        )
        .expect("source");

    let vf = probes(session.compile_attempt()).pop().expect("VF probe");
    session
        .provide_resources(vec![ResourceResponse::FileUnavailable(vf.key().clone())])
        .expect("non-virtual font");
    let map = requests(session.compile_attempt())
        .pop()
        .expect("map request");
    session
        .provide_resolved_file(
            map.key().clone(),
            "/texlive/fonts/map/pdftex.map",
            Vec::new(),
        )
        .expect("empty authoritative map");

    let pk_resources = resources(session.compile_attempt());
    let [ResourceRequest::PkFont(request)] = pk_resources.as_slice() else {
        panic!("expected one typed PK request");
    };
    assert_eq!(request.tex_name(), b"cmr10");
    assert_eq!(request.dpi(), 600);
    assert_eq!(request.logical_name(), b"cmr10.600pk");
    let bytes = include_bytes!("../../../../tests/corpus/pdf/pk_bitmap_600/cmr10.600pk").to_vec();
    let expected_ahash64 = Some(
        umber_hash::AHash64::for_bytes(umber_hash::HashDomain::PkProgram, &bytes).to_le_bytes(),
    );
    session
        .provide_resources(vec![ResourceResponse::PkFont(ResolvedPkFont {
            request: request.clone(),
            virtual_path: "/texlive/fonts/pk/ljfour/public/cm/cmr10.600pk".into(),
            bytes,
            expected_ahash64,
        })])
        .expect("typed PK response");
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("PK-backed PDF session should complete");
    };
    assert_eq!(output.outputs, OutputCapabilitySet::PDF);
}

#[test]
fn dvi_only_pdftex_session_skips_the_pdf_font_closure() {
    let mut session = VirtualCompileSession::new(SessionOptions {
        engine: EngineMode::PdfTex,
        outputs: OutputCapabilitySet::DVI,
        font_layout_policy: tex_fonts::FontLayoutPolicy::ClassicTfmExact,
        ..SessionOptions::default()
    })
    .expect("DVI-only pdfTeX session");
    session
        .add_user_file("cmr10.tfm", CMR10.to_vec())
        .expect("TFM");
    session
        .add_user_file(
            "main.tex",
            br"\font\tenrm=cmr10\relax\tenrm\shipout\hbox{A}\end".to_vec(),
        )
        .expect("source");
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("DVI-only pdfTeX session must not request VF or PDF assets");
    };
    assert_eq!(output.outputs, OutputCapabilitySet::DVI);
    assert!(!output.dvi.is_empty());
}

fn provide_cmu_font(session: &mut VirtualCompileSession, request: FontRequest) {
    let legacy_mapping = legacy_mapping_for(request.key.logical_name());
    session
        .provide_resources(vec![ResourceResponse::Font(ResolvedFont {
            request: request.key,
            container: FontContainer::Woff2,
            bytes: include_bytes!("../../../umber-wasm/assets/cmu-serif-500-roman.woff2").to_vec(),
            declared_object_ahash64: None,
            declared_program_identity: None,
            provenance: Some("CMU Serif under the SIL OFL".to_owned()),
            legacy_mapping,
        })])
        .expect("provide OpenType font");
}

fn legacy_mapping_for(name: &str) -> Option<tex_fonts::LegacyFontMapping> {
    let tfm = match name {
        "cmr10" => CMR10,
        "cmsy10" => CMSY10,
        "cmex10" => CMEX10,
        _ => return None,
    };
    let mut encoding = vec![None; 256];
    for code in 32_u8..=126 {
        encoding[usize::from(code)] = Some(char::from(code).to_string());
    }
    encoding[0] = Some("Γ".to_owned());
    Some(tex_fonts::LegacyFontMapping {
        tfm_ahash64: tex_fonts::font_content_hash(tfm),
        encoding,
        embeddable: true,
    })
}

#[test]
fn mapped_tfm_text_uses_one_opentype_authority_for_layout_and_html() {
    let source = b"\\font\\tenrm=cmr10\\relax \\tenrm \\textfont0=\\tenrm \\shipout\\hbox{\\char0 AV office}\\end";
    let mut modern = VirtualCompileSession::new(SessionOptions {
        outputs: OutputCapabilitySet::DVI.with(OutputCapability::Html),
        font_layout_policy: tex_fonts::FontLayoutPolicy::OpenTypePreferred,
        font_mapping_fallback: tex_fonts::FontMappingFallbackPolicy::Error,
        ..SessionOptions::default()
    })
    .expect("modern session");
    modern
        .add_user_file("cmr10.tfm", CMR10.to_vec())
        .expect("TFM");
    modern
        .add_user_file("main.tex", source.to_vec())
        .expect("source");
    let requested = resources(modern.compile_attempt());
    let [ResourceRequest::Font(request)] = requested.as_slice() else {
        panic!("mapped selection should request its exact OpenType program: {requested:?}");
    };
    provide_cmu_font(&mut modern, request.clone());
    let CompileAttemptResult::Complete(modern_output) = modern.compile_attempt() else {
        panic!("mapped compile should complete");
    };
    let html = String::from_utf8(modern_output.html.clone().expect("HTML")).expect("UTF-8");
    assert!(html.contains(">ΓAV office</text>"));
    assert!(html.contains("data:font/woff2;base64,"));

    let mut classic = VirtualCompileSession::new(SessionOptions {
        font_layout_policy: tex_fonts::FontLayoutPolicy::ClassicTfmExact,
        ..SessionOptions::default()
    })
    .expect("classic");
    classic
        .add_user_file("cmr10.tfm", CMR10.to_vec())
        .expect("TFM");
    classic
        .add_user_file("main.tex", source.to_vec())
        .expect("source");
    let CompileAttemptResult::Complete(classic_output) = classic.compile_attempt() else {
        panic!("classic compile should complete");
    };
    assert_ne!(
        modern_output.dvi, classic_output.dvi,
        "mapped advances must affect layout"
    );
}

#[test]
fn classic_tfm_html_acquires_exact_paint_resource_without_changing_dvi() {
    let source = b"\\font\\tenrm=cmr10\\relax \\tenrm \\shipout\\hbox{Classic A}\\end";
    let mut dvi_only = VirtualCompileSession::new(SessionOptions {
        outputs: OutputCapabilitySet::DVI,
        font_layout_policy: tex_fonts::FontLayoutPolicy::ClassicTfmExact,
        ..SessionOptions::default()
    })
    .expect("DVI session");
    dvi_only
        .add_user_file("cmr10.tfm", CMR10.to_vec())
        .expect("TFM");
    dvi_only
        .add_user_file("main.tex", source.to_vec())
        .expect("source");
    let CompileAttemptResult::Complete(dvi_output) = dvi_only.compile_attempt() else {
        panic!("DVI-only classic compile should complete without a paint request");
    };

    let mut html = VirtualCompileSession::new(SessionOptions {
        outputs: OutputCapabilitySet::DVI.with(OutputCapability::Html),
        font_layout_policy: tex_fonts::FontLayoutPolicy::ClassicTfmExact,
        ..SessionOptions::default()
    })
    .expect("HTML session");
    html.add_user_file("cmr10.tfm", CMR10.to_vec())
        .expect("TFM");
    html.add_user_file("main.tex", source.to_vec())
        .expect("source");
    let required = resources(html.compile_attempt());
    let [ResourceRequest::Font(request)] = required.as_slice() else {
        panic!("classic HTML should request one exact paint resource: {required:?}");
    };
    assert_eq!(request.key.logical_name(), "cmr10");
    assert_eq!(request.purposes, tex_fonts::FontPurposes::HTML);
    provide_cmu_font(&mut html, request.clone());
    let CompileAttemptResult::Complete(html_output) = html.compile_attempt() else {
        panic!("classic HTML compile should complete with its exact paint resource");
    };
    assert_eq!(html_output.dvi, dvi_output.dvi);
    let rendered = String::from_utf8(html_output.html.expect("HTML")).expect("UTF-8");
    assert!(rendered.contains(">Classic A</text>"));
    assert!(rendered.contains("data:font/woff2;base64,"));
}

#[test]
fn accepted_html_revisions_publish_snapshot_then_acknowledged_patch_and_resync() {
    let source = "\\shipout\\hbox{\\vrule width 1pt height 1pt}\\end";
    let mut session = VirtualCompileSession::new(SessionOptions {
        outputs: OutputCapabilitySet::HTML,
        ..SessionOptions::default()
    })
    .expect("HTML session");
    session
        .add_user_file("main.tex", source.as_bytes().to_vec())
        .expect("source");
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("initial HTML revision should complete");
    };
    let RenderUpdate::Snapshot(snapshot) = session.render_update().expect("initial snapshot")
    else {
        panic!("initial update must be a snapshot");
    };
    let serialized =
        tex_out::html::write_render_document(snapshot, &tex_out::html::HtmlOptions::default())
            .expect("retained render document serializes");
    assert_eq!(output.html.as_deref(), Some(serialized.html.as_slice()));
    assert_eq!(output.html_assets.len(), serialized.assets.len());
    let first_digest = snapshot.revision.digest;
    session
        .acknowledge_render_update(1, first_digest)
        .expect("snapshot acknowledgement");
    assert!(session.render_update().is_none());
    let Some(RenderUpdate::Snapshot(resync)) = session.render_resync() else {
        panic!("explicit resync should return the accepted snapshot");
    };
    assert_eq!(resync.revision.revision, 1);
    assert_eq!(resync.revision.digest, first_digest);

    let _next_source = apply_text_replacement(&mut session, 2, source, "1pt", "2pt");
    let CompileAttemptResult::Complete(_) = session.compile_attempt() else {
        panic!("edited HTML revision should complete");
    };
    let RenderUpdate::Patch(patch) = session.render_update().expect("incremental patch") else {
        panic!("acknowledged base should produce a patch");
    };
    assert_eq!(patch.base_revision, 1);
    assert_eq!(patch.target_revision, 2);
    let second_digest = patch.after_digest;
    session
        .acknowledge_render_update(2, second_digest)
        .expect("patch acknowledgement");
    let Some(RenderUpdate::Snapshot(resync)) = session.render_resync() else {
        panic!("resync after patch should return the latest accepted snapshot");
    };
    assert_eq!(resync.revision.revision, 2);
    assert_eq!(resync.revision.digest, second_digest);
}

#[test]
fn classic_html_font_names_bind_one_tfm_identity() {
    let key = FontRequestKey::new(
        "cmr10",
        0,
        tex_fonts::VariationSelection::default(),
        tex_fonts::FontFeaturePolicy::default(),
    )
    .expect("font key");
    let first = tex_fonts::font_content_hash(b"first TFM");
    let conflicting = tex_fonts::font_content_hash(b"conflicting TFM");
    let mut fonts = BTreeMap::new();

    register_classic_html_paint_font(&mut fonts, key.clone(), "cmr10", first)
        .expect("initial binding");
    register_classic_html_paint_font(&mut fonts, key.clone(), "cmr10", first)
        .expect("byte-identical duplicate");
    assert_eq!(fonts.len(), 1);

    let error = register_classic_html_paint_font(&mut fonts, key, "cmr10", conflicting)
        .expect_err("different TFM identity must conflict");
    assert_eq!(
        error,
        CompileError::ConflictingHtmlFontBinding {
            name: "cmr10".to_owned(),
            expected_tfm_identity: first,
            conflicting_tfm_identity: conflicting,
        }
    );
    let message = error.to_string();
    assert!(message.contains("cmr10"));
    assert!(message.contains(&hex_ahash64(first)));
    assert!(message.contains(&hex_ahash64(conflicting)));
}

#[test]
fn classic_tfm_html_reports_unsupported_exact_mapping() {
    let mut session = VirtualCompileSession::new(SessionOptions {
        outputs: OutputCapabilitySet::HTML,
        font_layout_policy: tex_fonts::FontLayoutPolicy::ClassicTfmExact,
        ..SessionOptions::default()
    })
    .expect("HTML session");
    session
        .add_user_file("cmr10.tfm", CMR10.to_vec())
        .expect("TFM");
    session
        .add_user_file(
            "main.tex",
            b"\\font\\tenrm=cmr10\\relax \\tenrm \\shipout\\hbox{A}\\end".to_vec(),
        )
        .expect("source");
    let required = resources(session.compile_attempt());
    let [ResourceRequest::Font(request)] = required.as_slice() else {
        panic!("classic HTML paint request");
    };
    session
        .provide_resources(vec![ResourceResponse::FontUnavailable(request.key.clone())])
        .expect("typed absence");
    let CompileAttemptResult::Error(CompileError::OutputCapability {
        capability: OutputCapability::Html,
        message,
    }) = session.compile_attempt()
    else {
        panic!("unsupported classic mapping should be an HTML capability error");
    };
    assert!(message.contains("unsupported HTML legacy mapping"));
    assert!(message.contains("cmr10"));
}

#[test]
fn mapped_tfm_policy_rejects_missing_and_conflicting_exact_bundles() {
    let mut missing = VirtualCompileSession::new(SessionOptions {
        font_layout_policy: tex_fonts::FontLayoutPolicy::OpenTypePreferred,
        font_mapping_fallback: tex_fonts::FontMappingFallbackPolicy::Error,
        ..SessionOptions::default()
    })
    .expect("strict modern session");
    missing
        .add_user_file("cmr10.tfm", CMR10.to_vec())
        .expect("TFM");
    missing
        .add_user_file("main.tex", b"\\font\\tenrm=cmr10\\relax\\end".to_vec())
        .expect("source");
    let requested = resources(missing.compile_attempt());
    let [ResourceRequest::Font(request)] = requested.as_slice() else {
        panic!("font request");
    };
    missing
        .provide_resources(vec![ResourceResponse::FontUnavailable(request.key.clone())])
        .expect("unavailable");
    assert!(matches!(
        missing.compile_attempt(),
        CompileAttemptResult::Error(_)
    ));
}

fn cmu_response(request: FontRequest) -> ResolvedFont {
    let legacy_mapping = legacy_mapping_for(request.key.logical_name());
    ResolvedFont {
        request: request.key,
        container: FontContainer::Woff2,
        bytes: include_bytes!("../../../umber-wasm/assets/cmu-serif-500-roman.woff2").to_vec(),
        declared_object_ahash64: None,
        declared_program_identity: None,
        provenance: Some("CMU Serif under the SIL OFL".to_owned()),
        legacy_mapping,
    }
}

#[test]
fn typed_font_response_obeys_the_per_resource_byte_limit_atomically() {
    let font_bytes = include_bytes!("../../../umber-wasm/assets/cmu-serif-500-roman.woff2");
    let mut session = VirtualCompileSession::new(SessionOptions {
        outputs: OutputCapabilitySet::DVI.with(OutputCapability::Html),
        limits: SessionLimits {
            one_file_bytes: font_bytes.len() - 1,
            ..SessionLimits::default()
        },
        ..SessionOptions::default()
    })
    .expect("bounded session");
    session
        .add_user_file(
            "main.tex",
            b"\\font\\ot=opentype:cmu-serif-roman at 10pt \\ot A\\end".to_vec(),
        )
        .expect("source");
    let request = resources(session.compile_attempt())
        .into_iter()
        .find_map(|request| match request {
            ResourceRequest::Font(request) => Some(request),
            ResourceRequest::File(_) | ResourceRequest::PkFont(_) => None,
        })
        .expect("font request");
    assert!(matches!(
        session.provide_resources(vec![ResourceResponse::Font(cmu_response(request))]),
        Err(CompileError::LimitExceeded {
            resource: "one font resource bytes",
            limit,
            attempted,
        }) if limit == font_bytes.len() - 1 && attempted == font_bytes.len()
    ));
    assert!(session.resolved_fonts.is_empty());
    assert_eq!(session.cached_file_bytes(), 0);
}

#[test]
fn dvi_only_mapping_does_not_require_embedding_permission() {
    let mut session = VirtualCompileSession::new(SessionOptions {
        font_layout_policy: tex_fonts::FontLayoutPolicy::OpenTypePreferred,
        font_mapping_fallback: tex_fonts::FontMappingFallbackPolicy::Error,
        ..SessionOptions::default()
    })
    .expect("DVI session");
    session
        .add_user_file("cmr10.tfm", CMR10.to_vec())
        .expect("TFM");
    session
        .add_user_file(
            "main.tex",
            b"\\font\\tenrm=cmr10\\relax \\tenrm \\shipout\\hbox{A}\\end".to_vec(),
        )
        .expect("source");
    let request = resources(session.compile_attempt())
        .into_iter()
        .find_map(|request| match request {
            ResourceRequest::Font(request) => Some(request),
            ResourceRequest::File(_) | ResourceRequest::PkFont(_) => None,
        })
        .expect("font request");
    let mut response = cmu_response(request);
    response
        .legacy_mapping
        .as_mut()
        .expect("mapped response")
        .embeddable = false;
    session
        .provide_resources(vec![ResourceResponse::Font(response)])
        .expect("non-embedding layout response");
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("DVI-only layout must not require embedding permission");
    };
    assert!(!output.dvi.is_empty());
}

fn rendered_text_address(html: &str, code: u32) -> (u32, u32) {
    let marker = format!("data-umber-codes=\"0x{code:x}");
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
fn parent_relative_paths_are_opaque_requests_and_missing_main_is_typed() {
    let mut traversal = session("\\input ../secret \\input ../secret.tex \\end");
    let missing = requests(traversal.compile_attempt());
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].original_name(), "../secret");
    assert!(missing[0].key().name().starts_with(".host-path/"));
    assert!(!missing[0].key().name().contains(".."));
    traversal
        .provide_resolved_file(
            missing[0].key().clone(),
            "/texlive/local/secret.tex",
            b"% host-relative input\n".to_vec(),
        )
        .expect("host-relative resource");
    assert!(matches!(
        traversal.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));

    let mut incomplete = session("\\input .. \\end");
    assert!(matches!(
        incomplete.compile_attempt(),
        CompileAttemptResult::Error(CompileError::InvalidRequestedPath { .. })
    ));

    let mut foreign_platform_area = session("\\input :texsys.aux \\end");
    assert!(matches!(
        foreign_platform_area.compile_attempt(),
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
fn unavailable_required_input_becomes_an_engine_open_error() {
    let mut session = session("\\input absent \\end");
    let requested = requests(session.compile_attempt());
    let [request] = requested.as_slice() else {
        panic!("expected one required input request");
    };
    session
        .provide_resources(vec![ResourceResponse::FileUnavailable(
            request.key().clone(),
        )])
        .expect("authoritative negative input response");
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Error(CompileError::Diagnostic(ref diagnostic))
            if diagnostic.message.contains("emergency-stop")
    ));
}

#[test]
fn unavailable_probe_retries_through_dump_instead_of_accepting_end_of_input() {
    let mut session = session(
        "\\openin0=optional.cfg \\ifeof0 \\message{OPTIONAL-MISSING}\\else \\errmessage{unexpected optional file}\\fi \\dump\\endinput",
    );
    let missing = probes(session.compile_attempt());
    assert_eq!(missing.len(), 1);
    session
        .provide_resources(vec![ResourceResponse::FileUnavailable(
            missing[0].key().clone(),
        )])
        .expect("authoritative negative probe response");
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
    assert!(
        session
            .into_accepted_finalization()
            .expect("accepted format finalization")
            .format_dump
            .is_some()
    );
}

#[test]
fn positive_probe_can_promote_to_required_input_before_dump() {
    let mut session = session(
        "\\openin0=optional.cfg \\ifeof0 \\errmessage{missing optional file}\\else \\closein0 \\input optional.cfg \\fi \\dump",
    );
    let missing = probes(session.compile_attempt());
    assert_eq!(missing.len(), 1);
    session
        .provide_resources(vec![ResourceResponse::File(ResolvedFile {
            request: missing[0].key().clone(),
            virtual_path: "/texlive/optional.cfg".to_owned(),
            bytes: b"\\message{OPTIONAL-PRESENT}\\endinput".to_vec(),
            expected_digest: None,
        })])
        .expect("positive probe response");
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("positive probe should permit the guarded input and dump");
    };
    assert!(
        output
            .terminal
            .windows(16)
            .any(|window| window == b"OPTIONAL-PRESENT")
    );
    assert!(session.accepted_input_dependencies().any(|dependency| {
        dependency.path() == Path::new("/texlive/optional.cfg")
            && dependency.access() == tex_state::InputDependencyAccess::RequiredRead
    }));
    assert!(
        session
            .into_accepted_finalization()
            .expect("accepted format finalization")
            .format_dump
            .is_some()
    );
}

#[test]
fn hinted_file_is_provided_only_after_a_typed_request() {
    let mut session = session("\\input hinted \\end");
    let requested = resources(session.compile_attempt());
    let [ResourceRequest::File(request)] = requested.as_slice() else {
        panic!("file request");
    };
    session
        .provide_resources(vec![ResourceResponse::File(ResolvedFile {
            request: request.key().clone(),
            virtual_path: "/texlive/tex/latex/example/hinted.tex".to_owned(),
            bytes: b"\\message{PREFETCHED}".to_vec(),
            expected_digest: None,
        })])
        .expect("typed response");

    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("provided input should complete");
    };
    assert!(String::from_utf8_lossy(&output.terminal).contains("PREFETCHED"));
    assert_eq!(session.attempts(), 2);
}

#[test]
fn resolved_nested_probe_retries_through_endinput_to_root_dump() {
    let mut session = session("\\input wrapper \\dump");
    let missing = resources(session.compile_attempt());
    assert_eq!(missing.len(), 1);
    let ResourceRequest::File(wrapper) = &missing[0] else {
        unreachable!();
    };
    session
        .provide_resources(vec![ResourceResponse::File(ResolvedFile {
            request: wrapper.key().clone(),
            virtual_path: "/texlive/wrapper.tex".into(),
            bytes: b"\\openin0=optional.dfu \\ifeof0 \\else \\input optional.dfu \\fi \\endinput"
                .to_vec(),
            expected_digest: None,
        })])
        .expect("wrapper response");

    let probed = probes(session.compile_attempt());
    assert_eq!(probed.len(), 1);
    session
        .provide_resources(vec![ResourceResponse::File(ResolvedFile {
            request: probed[0].key().clone(),
            virtual_path: "/texlive/optional.dfu".into(),
            bytes: b"\\endinput".to_vec(),
            expected_digest: None,
        })])
        .expect("positive probe response");

    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
    assert!(
        session
            .into_accepted_finalization()
            .expect("accepted format finalization")
            .format_dump
            .is_some()
    );
}

#[test]
fn unavailable_nested_probe_retries_through_endinput_to_root_dump() {
    let mut session = session("\\input wrapper \\dump");
    let missing = resources(session.compile_attempt());
    let ResourceRequest::File(wrapper) = &missing[0] else {
        unreachable!();
    };
    session
        .provide_resources(vec![ResourceResponse::File(ResolvedFile {
            request: wrapper.key().clone(),
            virtual_path: "/texlive/wrapper.tex".into(),
            bytes: b"\\openin0=optional.dfu \\ifeof0 \\else \\input optional.dfu \\fi \\endinput"
                .to_vec(),
            expected_digest: None,
        })])
        .expect("wrapper response");

    let probed = probes(session.compile_attempt());
    session
        .provide_resources(vec![ResourceResponse::FileUnavailable(
            probed[0].key().clone(),
        )])
        .expect("negative probe response");

    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
    assert!(
        session
            .into_accepted_finalization()
            .expect("accepted format finalization")
            .format_dump
            .is_some()
    );
}

#[test]
fn required_resources_suspend_at_the_first_unavailable_dependency() {
    let mut session = session("\\font\\a=one\\relax \\font\\b=two\\relax \\input later \\end");
    for expected in ["one.tfm", "two.tfm"] {
        let requested = requests(session.compile_attempt());
        let [request] = requested.as_slice() else {
            panic!("expected one ordered TFM dependency");
        };
        assert_eq!(request.key().kind(), FileKind::Tfm);
        assert_eq!(request.key().name(), expected);
        session
            .provide_resolved_file(
                request.key().clone(),
                if expected == "one.tfm" {
                    "/texlive/one.tfm"
                } else {
                    "/texlive/two.tfm"
                },
                CMR10.to_vec(),
            )
            .expect("TFM response");
    }
    let requested = requests(session.compile_attempt());
    let [request] = requested.as_slice() else {
        panic!("expected the later input after both font dependencies");
    };
    assert_eq!(request.key().kind(), FileKind::TexInput);
    assert_eq!(request.key().name(), "later.tex");
}

#[test]
fn initial_candidate_and_committed_prefix_survive_sequential_resource_batches() {
    let mut session = session("\\input first \\input second \\end");
    let first = requests(session.compile_attempt());
    assert_eq!(first[0].key().name(), "first.tex");
    assert_eq!(
        session
            .candidate
            .as_ref()
            .expect("retained first suspension")
            .suspension_serial,
        1
    );
    assert!(session.retention_metrics().is_some());
    session
        .provide_resolved_file(
            first[0].key().clone(),
            "/texlive/first.tex",
            b"\\message{first}".to_vec(),
        )
        .expect("first response");

    let second = requests(session.compile_attempt());
    assert_eq!(second[0].key().name(), "second.tex");
    assert_eq!(
        session
            .candidate
            .as_ref()
            .expect("retained second suspension")
            .suspension_serial,
        2,
        "the same execution run must advance its suspension serial"
    );
    session
        .provide_resolved_file(
            second[0].key().clone(),
            "/texlive/second.tex",
            b"\\message{second}".to_vec(),
        )
        .expect("second response");
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("retained candidate should complete");
    };
    let terminal = String::from_utf8_lossy(&output.terminal);
    assert!(terminal.contains("first"));
    assert!(terminal.contains("second"));
    assert!(session.candidate.is_none());
}

fn register_equivalence_resources(session: &mut VirtualCompileSession) {
    session
        .provide_resolved_file(
            FileRequestKey::new(FileKind::Tfm, "cmr10.tfm").expect("TFM key"),
            "/texlive/fonts/tfm/public/cm/cmr10.tfm",
            CMR10.to_vec(),
        )
        .expect("preloaded TFM");
    session
        .provide_resolved_file(
            FileRequestKey::new(FileKind::TexInput, "remote.tex").expect("input key"),
            "/texlive/tex/remote.tex",
            b"\\message{REMOTE}\\endinput".to_vec(),
        )
        .expect("preloaded input");
}

#[test]
fn preloaded_and_partitioned_positive_negative_resources_are_exactly_equivalent() {
    let source = r"\font\tenrm=cmr10 \openin0=optional.cfg \ifeof0 \message{ABSENT}\else \errmessage{unexpected optional}\fi \closein0 \input remote \tenrm \shipout\hbox{A}\end";

    let mut preloaded = session(source);
    register_equivalence_resources(&mut preloaded);
    let optional = FileRequestKey::new(FileKind::TexInput, "optional.cfg").expect("probe key");
    preloaded.workspace.expect(&FileRequestBatch::with_probes(
        std::iter::empty(),
        [FileRequest::new(optional.clone(), "optional.cfg")],
        std::iter::empty(),
    ));
    preloaded
        .workspace
        .provision_unavailable(optional.clone())
        .expect("preloaded authoritative absence");
    let CompileAttemptResult::Complete(preloaded_output) = preloaded.compile_attempt() else {
        panic!("preloaded run must complete uninterrupted");
    };
    let preloaded_telemetry = preloaded.compile_telemetry();

    let mut partitioned = session(source);
    loop {
        match partitioned.compile_attempt() {
            CompileAttemptResult::NeedResources(needs) => {
                let request = needs
                    .required
                    .into_iter()
                    .chain(needs.probes)
                    .next()
                    .expect("one deliberately partitioned dependency");
                match request {
                    ResourceRequest::File(request) if request.key().kind() == FileKind::Tfm => {
                        partitioned
                            .provide_resolved_file(
                                request.key().clone(),
                                "/texlive/fonts/tfm/public/cm/cmr10.tfm",
                                CMR10.to_vec(),
                            )
                            .expect("partitioned TFM");
                    }
                    ResourceRequest::File(request) if request.key() == &optional => {
                        partitioned
                            .provide_resources(vec![ResourceResponse::FileUnavailable(
                                optional.clone(),
                            )])
                            .expect("partitioned authoritative absence");
                    }
                    ResourceRequest::File(request) => {
                        assert_eq!(request.key().name(), "remote.tex");
                        partitioned
                            .provide_resolved_file(
                                request.key().clone(),
                                "/texlive/tex/remote.tex",
                                b"\\message{REMOTE}\\endinput".to_vec(),
                            )
                            .expect("partitioned input");
                    }
                    ResourceRequest::Font(_) | ResourceRequest::PkFont(_) => {
                        panic!("classic DVI run requests no output font")
                    }
                }
            }
            CompileAttemptResult::Complete(partitioned_output) => {
                assert_eq!(partitioned_output, preloaded_output);
                break;
            }
            other => panic!("partitioned run failed: {other:?}"),
        }
    }
    let partitioned_telemetry = partitioned.compile_telemetry();
    assert_eq!(preloaded_telemetry.execution.cold_starts, 1);
    assert_eq!(preloaded_telemetry.execution.suspensions, 0);
    assert_eq!(preloaded_telemetry.execution.local_step_retries, 0);
    assert_eq!(preloaded_telemetry.execution.replayed_delivered_tokens, 0);
    assert_eq!(preloaded_telemetry.execution.replayed_dispatches, 0);
    assert_eq!(partitioned_telemetry.execution.cold_starts, 1);
    assert_eq!(partitioned_telemetry.execution.suspensions, 3);
    assert_eq!(
        (
            partitioned_telemetry.execution.local_step_retries,
            partitioned_telemetry.execution.replayed_delivered_tokens,
            partitioned_telemetry.execution.replayed_dispatches,
        ),
        (3, 0, 0),
        "typed resource continuations retry locally without replaying deliveries"
    );
    assert_eq!(partitioned.attempts(), 4);
    assert!(
        partitioned_telemetry.execution.cumulative_fuel
            >= preloaded_telemetry.execution.cumulative_fuel
    );
}

#[test]
fn cumulative_engine_fuel_is_terminal_across_step_boundaries() {
    let mut limited = VirtualCompileSession::new(SessionOptions {
        limits: SessionLimits {
            engine_fuel: 1,
            ..SessionLimits::default()
        },
        ..SessionOptions::default()
    })
    .expect("limited session");
    limited
        .add_user_file("main.tex", b"\\def\\a{A}\\a\\a\\a\\end".to_vec())
        .expect("source");
    assert!(matches!(
        limited.compile_attempt(),
        CompileAttemptResult::Error(CompileError::Diagnostic(ref diagnostic))
            if diagnostic.message.contains("canonical command fuel exhausted")
                && diagnostic.message.contains("limit 1")
    ));
    assert_eq!(limited.compile_telemetry().execution.cumulative_fuel, 1);
}

#[test]
fn configured_executor_step_budget_fails_actionably() {
    let mut limited = VirtualCompileSession::new(SessionOptions {
        limits: SessionLimits {
            engine_steps: 0,
            ..SessionLimits::default()
        },
        ..SessionOptions::default()
    })
    .expect("limited session");
    limited
        .add_user_file("main.tex", b"\\end".to_vec())
        .expect("source");
    let result = limited.compile_attempt();
    assert!(
        matches!(
            result,
            CompileAttemptResult::Error(CompileError::Diagnostic(ref diagnostic))
                if diagnostic.message.contains("execution steps budget 0 exceeded at 1")
        ),
        "{result:?}"
    );
}

#[test]
fn configured_input_frame_budget_fails_actionably() {
    let mut limited = VirtualCompileSession::new(SessionOptions {
        limits: SessionLimits {
            input_frames: 0,
            ..SessionLimits::default()
        },
        ..SessionOptions::default()
    })
    .expect("limited session");
    limited
        .add_user_file("main.tex", b"\\end".to_vec())
        .expect("source");
    let result = limited.compile_attempt();
    assert!(
        matches!(
            result,
            CompileAttemptResult::Error(CompileError::Diagnostic(ref diagnostic))
                if diagnostic.message.contains("live input frames budget 0 exceeded at 1")
        ),
        "{result:?}"
    );
    assert!(limited.candidate.is_none());
}

#[test]
fn configured_journal_budget_fails_actionably() {
    let mut limited = VirtualCompileSession::new(SessionOptions {
        limits: SessionLimits {
            journal_bytes: 0,
            ..SessionLimits::default()
        },
        ..SessionOptions::default()
    })
    .expect("limited session");
    limited
        .add_user_file("main.tex", b"\\count0=1\\end".to_vec())
        .expect("source");
    assert!(matches!(
        limited.compile_attempt(),
        CompileAttemptResult::Error(CompileError::Diagnostic(diagnostic))
            if diagnostic.message.contains("environment journal bytes budget 0 exceeded")
    ));
    assert!(limited.candidate.is_none());
}

#[test]
fn configured_pending_effect_budget_fails_actionably() {
    let mut limited = VirtualCompileSession::new(SessionOptions {
        limits: SessionLimits {
            effects: 0,
            ..SessionLimits::default()
        },
        ..SessionOptions::default()
    })
    .expect("limited session");
    limited
        .add_user_file("main.tex", b"\\message{effect}\\end".to_vec())
        .expect("source");
    let result = limited.compile_attempt();
    assert!(
        matches!(
            result,
            CompileAttemptResult::Error(CompileError::Diagnostic(ref diagnostic))
                if diagnostic.message.contains("pending effects budget 0 exceeded at 9")
        ),
        "{result:?}"
    );
    assert!(limited.candidate.is_none());
}

#[test]
fn retained_resource_retry_charges_live_input_frames_once() {
    let mut limited = VirtualCompileSession::new(SessionOptions {
        limits: SessionLimits {
            input_frames: 2,
            ..SessionLimits::default()
        },
        ..SessionOptions::default()
    })
    .expect("limited session");
    limited
        .add_user_file("main.tex", b"\\input remote \\end".to_vec())
        .expect("source");
    let request = requests(limited.compile_attempt())
        .into_iter()
        .next()
        .expect("remote input request")
        .key()
        .clone();
    limited
        .provide_resources(vec![ResourceResponse::File(ResolvedFile {
            request,
            virtual_path: "/texlive/remote.tex".to_owned(),
            bytes: b"\\relax".to_vec(),
            expected_digest: None,
        })])
        .expect("remote input response");
    assert!(matches!(
        limited.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
}

#[test]
fn discarded_suspension_cannot_resume_and_releases_candidate_retention() {
    let mut session = session("\\input remote \\end");
    let first = requests(session.compile_attempt());
    assert!(session.candidate.is_some());
    assert!(session.discard_suspended_candidate());
    assert!(session.candidate.is_none());
    assert!(session.awaiting.is_none());

    let restarted = requests(session.compile_attempt());
    assert_eq!(restarted, first);
    assert_eq!(
        session
            .candidate
            .as_ref()
            .expect("fresh candidate after cancellation")
            .suspension_serial,
        1,
        "the cancelled run's suspension serial must not be resumed"
    );
}

#[test]
fn cancelled_edit_drops_its_run_but_keeps_accepted_output_and_late_bytes_cache_only() {
    let source = "\\message{accepted}\\end";
    let mut session = session(source);
    let CompileAttemptResult::Complete(accepted) = session.compile_attempt() else {
        panic!("initial revision should complete");
    };
    let end = source.find("\\end").expect("end marker");
    session
        .apply_patch(SourcePatch {
            next_revision: RevisionId::new(2),
            base_revision: RevisionId::new(1),
            expected_hash: session.content_hash().expect("accepted hash"),
            range: end..end,
            replacement: "\\input late ".to_owned(),
        })
        .expect("patch");
    let late = requests(session.compile_attempt());
    assert!(session.candidate.is_some());
    assert!(session.cancel_pending_patch());
    assert!(session.candidate.is_none());
    session
        .provide_resources(vec![ResourceResponse::File(ResolvedFile {
            request: late[0].key().clone(),
            virtual_path: "/texlive/late.tex".to_owned(),
            bytes: b"verified late bytes".to_vec(),
            expected_digest: None,
        })])
        .expect("late immutable response may warm the cache");
    assert!(session.candidate.is_none());
    assert_eq!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(accepted)
    );
    assert_eq!(session.revision(), Some(RevisionId::new(1)));
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
fn successful_resource_progress_is_not_mistaken_for_an_attempt_loop() {
    let mut session = session("\\input remote-0 \\end");

    for index in 0..SessionLimits::HARD_MAX.attempts {
        let missing = requests(session.compile_attempt());
        assert_eq!(missing.len(), 1, "resource round {index}");
        let contents = if index + 1 == SessionLimits::HARD_MAX.attempts {
            b"\\message{resolved-chain}".to_vec()
        } else {
            format!("\\input remote-{}", index + 1).into_bytes()
        };
        session
            .provide_resolved_file(
                missing[0].key().clone(),
                &format!("/texlive/remote-{index}.tex"),
                contents,
            )
            .expect("provide the newly requested resource");
    }

    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("a fully progressing resource chain must complete");
    };
    assert!(String::from_utf8_lossy(&output.terminal).contains("resolved-chain"));
}

#[test]
fn latex_sessions_preserve_legacy_high_bytes_in_resolved_inputs() {
    let mut options = SessionOptions {
        engine: EngineMode::Latex,
        ..SessionOptions::default()
    };
    options.main_path = "main.tex".to_owned();
    let mut session = VirtualCompileSession::new(options).expect("LaTeX session");
    session
        .add_user_file("main.tex", b"\\input legacy \\end".to_vec())
        .expect("main file");

    let missing = requests(session.compile_attempt());
    assert_eq!(missing.len(), 1);
    session
        .provide_resolved_file(
            missing[0].key().clone(),
            "/texlive/tex/legacy.tex",
            vec![b'%', b' ', 0x96, b'\n'],
        )
        .expect("legacy input");

    let result = session.compile_attempt();
    assert!(
        matches!(result, CompileAttemptResult::Complete(_)),
        "legacy-byte LaTeX compile did not complete: {result:?}"
    );
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
    let mut session = VirtualCompileSession::new(SessionOptions {
        job_name: Some("texput".to_owned()),
        authored_root_name: Some("texput".to_owned()),
        font_layout_policy: tex_fonts::FontLayoutPolicy::ClassicTfmExact,
        ..SessionOptions::default()
    })
    .expect("session");
    session
        .add_user_file(
            "main.tex",
            b"\\message{before}\\input remote \\end".to_vec(),
        )
        .expect("main source");
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
    assert_eq!(terminal.matches("(texput").count(), 1);
    assert_eq!(terminal.matches("before").count(), 1);
    assert_eq!(terminal.matches("after").count(), 1);
    assert!(terminal.ends_with(" )"));
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
            .workspace
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
    let snapshot = session.workspace.snapshot();
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
    let native = crate::with_engine_world(World::memory(), |stores| {
        prepare_run_stores(stores);
        crate::run_memory_with_stores(source, stores).expect("native memory run");
        crate::collect_final_memory_output(stores, &[], 1 << 20).expect("native output")
    })
    .expect("fresh native-output universe");

    let mut virtual_session = VirtualCompileSession::new(SessionOptions {
        job_name: Some("texput".to_owned()),
        authored_root_name: Some("texput".to_owned()),
        ..SessionOptions::default()
    })
    .expect("virtual session");
    virtual_session
        .add_user_file("main.tex", source.as_bytes().to_vec())
        .expect("main source");
    let CompileAttemptResult::Complete(virtual_output) = virtual_session.compile_attempt() else {
        panic!("virtual run should complete");
    };
    assert_eq!(
        virtual_output
            .terminal
            .windows(b"(texput".len())
            .filter(|window| *window == b"(texput")
            .count(),
        1,
        "the authored root frame is emitted exactly once"
    );
    for transcript in [&virtual_output.terminal, &virtual_output.log] {
        assert!(
            !transcript
                .windows(b"/job/main.tex".len())
                .any(|window| window == b"/job/main.tex"),
            "the provenance path must not become a duplicate transcript frame: {:?}",
            String::from_utf8_lossy(transcript),
        );
    }
    assert_eq!(virtual_output.outputs, native.outputs);
    assert_eq!(virtual_output.dvi, native.dvi);
    assert_eq!(virtual_output.html, native.html);
    assert_eq!(virtual_output.html_assets, native.html_assets);
    assert_eq!(virtual_output.files, native.files);
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
            .workspace
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

    let snapshot = session.workspace.snapshot();
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
        .and_then(|session| session.retention_metrics())
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
    let old_snapshot = session.workspace.snapshot();
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

    let snapshot = session.workspace.snapshot();
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
fn every_engine_mode_has_source_and_schema_12_format_artifact_equivalence() {
    let source = b"\\catcode123=1 \\catcode125=2 \\shipout\\hbox{}\\end";
    for engine in [
        EngineMode::Tex82,
        EngineMode::ETex,
        EngineMode::PdfTex,
        EngineMode::Latex,
        EngineMode::PdfLatex,
    ] {
        let format = construct_test_format(engine, "\\dump");
        assert_eq!(
            u32::from_le_bytes(format.as_bytes()[8..12].try_into().expect("schema bytes")),
            12
        );

        let mut formatted = VirtualCompileSession::new(SessionOptions {
            format: Some(format.into_bytes()),
            engine,
            ..SessionOptions::default()
        })
        .expect("formatted session");
        formatted
            .add_user_file("main.tex", source.to_vec())
            .expect("formatted main");

        let mut fresh = VirtualCompileSession::new(SessionOptions {
            engine,
            ..SessionOptions::default()
        })
        .expect("fresh session");
        fresh
            .add_user_file("main.tex", source.to_vec())
            .expect("fresh main");

        let CompileAttemptResult::Complete(formatted) = formatted.compile_attempt() else {
            panic!("{} formatted session did not complete", engine.name());
        };
        let CompileAttemptResult::Complete(fresh) = fresh.compile_attempt() else {
            panic!("{} fresh session did not complete", engine.name());
        };
        assert_eq!(
            formatted.outputs,
            fresh.outputs,
            "{} outputs differ",
            engine.name()
        );
        assert_eq!(formatted.dvi, fresh.dvi, "{} DVI differs", engine.name());
        assert_eq!(formatted.html, fresh.html, "{} HTML differs", engine.name());
        assert_eq!(
            formatted.html_assets,
            fresh.html_assets,
            "{} HTML assets differ",
            engine.name()
        );
        assert_eq!(
            formatted.files,
            fresh.files,
            "{} files differ",
            engine.name()
        );

        let formatted_terminal = String::from_utf8_lossy(&formatted.terminal);
        let fresh_terminal = String::from_utf8_lossy(&fresh.terminal);
        assert!(
            formatted_terminal.contains(" (no format preloaded)\n"),
            "{} loaded startup identity: {formatted_terminal}",
            engine.name()
        );
        assert!(
            fresh_terminal.contains(" (INITEX)\n"),
            "{} fresh startup identity: {fresh_terminal}",
            engine.name()
        );
        assert_eq!(
            formatted_terminal.split_once('\n').map(|(_, rest)| rest),
            fresh_terminal.split_once('\n').map(|(_, rest)| rest),
            "{} post-banner terminal differs",
            engine.name()
        );
        let formatted_log = String::from_utf8_lossy(&formatted.log);
        let fresh_log = String::from_utf8_lossy(&fresh.log);
        assert_eq!(
            formatted_log.split_once('\n').map(|(_, rest)| rest),
            fresh_log.split_once('\n').map(|(_, rest)| rest),
            "{} post-banner log differs",
            engine.name()
        );
        assert!(
            !formatted.dvi.is_empty(),
            "{} emitted no DVI",
            engine.name()
        );
    }
}

#[test]
fn pdftex_output_mode_override_runs_after_format_loading() {
    // pdftex.web §1515 applies the process-selected output format after the
    // format image is loaded and before the first source token. The override
    // is independent from the downstream DVI receipt capability.
    for (stored, override_mode, expected) in [
        (0, None, 0),
        (1, Some(PdfOutputMode::Dvi), 0),
        (0, Some(PdfOutputMode::Pdf), 1),
    ] {
        let format =
            construct_test_format(EngineMode::PdfTex, &format!("\\pdfoutput={stored}\\dump"));
        let mut session = VirtualCompileSession::new(SessionOptions {
            format: Some(format.into_bytes()),
            engine: EngineMode::PdfTex,
            pdf_output_mode: override_mode,
            outputs: OutputCapabilitySet::DVI,
            ..SessionOptions::default()
        })
        .expect("pdfTeX loaded-format session");
        session
            .add_user_file(
                "main.tex",
                br"\message{PDFOUTPUT=\the\pdfoutput}\end".to_vec(),
            )
            .expect("output-mode probe");

        let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
            panic!("pdfTeX output-mode probe did not complete");
        };
        let terminal = String::from_utf8_lossy(&output.terminal);
        assert!(
            terminal.contains(&format!("PDFOUTPUT={expected}")),
            "stored={stored}, override={override_mode:?}: {terminal}"
        );
    }
}

#[test]
fn virtual_initex_installs_the_canonical_profile_registry() {
    for engine in [
        EngineMode::Tex82,
        EngineMode::ETex,
        EngineMode::PdfTex,
        EngineMode::Latex,
        EngineMode::PdfLatex,
    ] {
        crate::with_engine_world(World::memory(), |stores| {
            engine.prepare_initex(stores);

            let iftrue = stores.intern("iftrue").expect("iftrue symbol");
            let iftrue_meaning = stores
                .command_context()
                .expect("admit INITEX registry")
                .meaning(iftrue.symbol());
            assert_eq!(
                iftrue_meaning,
                Meaning::ExpandablePrimitive(ExpandablePrimitive::IfTrue),
                "{} TeX82 registry",
                engine.name()
            );
            assert_eq!(
                stores.primitive_meaning("ifdefined").is_some(),
                !matches!(engine, EngineMode::Tex82),
                "{} e-TeX registry",
                engine.name()
            );
            assert_eq!(
                stores.primitive_meaning("pdfprimitive").is_some(),
                matches!(engine, EngineMode::PdfTex | EngineMode::PdfLatex),
                "{} pdfTeX registry",
                engine.name()
            );
        })
        .expect("fresh INITEX registry universe");
    }
}

#[test]
fn latex_session_keeps_pdftex_identity_out_of_the_etex_compatibility_profile() {
    assert_eq!(
        EngineMode::Latex.command_profile(),
        tex_command::CommandProfile::ETEX26
    );
    let mut session = VirtualCompileSession::new(SessionOptions {
        engine: EngineMode::Latex,
        ..SessionOptions::default()
    })
    .expect("LaTeX session");
    session
        .add_user_file(
            "main.tex",
            br"\catcode123=1 \catcode125=2
               \ifdefined\pdftexversion\message{PDFTEX}\else\message{ETEX}\fi
               \end"
                .to_vec(),
        )
        .expect("LaTeX source");

    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("LaTeX compatibility source did not complete");
    };
    let terminal = String::from_utf8_lossy(&output.terminal);
    assert!(terminal.contains("ETEX"), "{terminal}");
    assert!(!terminal.contains("PDFTEX"), "{terminal}");
}

#[test]
fn virtual_format_registry_preserves_live_meanings_and_profile_distinctions() {
    for engine in [
        EngineMode::Tex82,
        EngineMode::ETex,
        EngineMode::PdfTex,
        EngineMode::Latex,
        EngineMode::PdfLatex,
    ] {
        crate::with_engine_world(World::memory(), |stores| {
            let names = ["iftrue", "ifdefined", "pdfprimitive"];
            for name in names {
                let symbol = stores.intern(name).expect("shadow symbol");
                stores
                    .assign_meaning(
                        symbol,
                        tex_state::MeaningWord::from_static(Meaning::Relax),
                        tex_state::AssignmentScope::Local,
                    )
                    .expect("shadow primitive");
            }

            engine.install_after_format(stores);

            for name in names {
                let symbol = stores.intern(name).expect("restored symbol");
                let meaning = stores
                    .command_context()
                    .expect("admit restored registry")
                    .meaning(symbol.symbol());
                assert_eq!(
                    meaning,
                    Meaning::Relax,
                    "{} must preserve restored \\{name}",
                    engine.name()
                );
            }
            assert!(stores.primitive_meaning("iftrue").is_some());
            assert_eq!(
                stores.primitive_meaning("ifdefined").is_some(),
                !matches!(engine, EngineMode::Tex82),
                "{} e-TeX restored registry",
                engine.name()
            );
            assert_eq!(
                stores.primitive_meaning("pdfprimitive").is_some(),
                matches!(engine, EngineMode::PdfTex | EngineMode::PdfLatex),
                "{} pdfTeX restored registry",
                engine.name()
            );
        })
        .expect("fresh format-registry universe");
    }
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
fn formatted_session_starts_with_fresh_clock_everyjob_and_checkpoint_state() {
    let format = construct_test_format(
        EngineMode::Tex82,
        "\\everyjob{\\count0=41\\message{everyjob}}\\dump",
    );
    let clock = tex_state::JobClock {
        time: 754,
        second: 56,
        day: 13,
        month: 7,
        year: 2042,
    };
    let mut formatted = VirtualCompileSession::new(SessionOptions {
        format: Some(format.into_bytes()),
        clock,
        ..SessionOptions::default()
    })
    .expect("formatted session");
    formatted
        .add_user_file(
            "main.tex",
            b"\\message{root=\\the\\count0,\\the\\time,\\the\\day,\\the\\month,\\the\\year}\\end"
                .to_vec(),
        )
        .expect("main");

    let CompileAttemptResult::Complete(output) = formatted.compile_attempt() else {
        panic!("formatted compile should complete");
    };
    let terminal = String::from_utf8_lossy(&output.terminal);
    let every_job = terminal.find("everyjob").expect("everyjob output");
    let root = terminal
        .find("root=41,754,13,7,2042")
        .expect("fresh job clock and everyjob mutation");
    assert!(every_job < root, "{terminal}");
    let history = formatted
        .incremental
        .as_ref()
        .expect("incremental session")
        .history();
    assert_eq!(
        history[0].key().boundary,
        tex_exec::EngineBoundary::JobStart
    );
    assert_eq!(history[0].key().position, 0);
}

#[test]
fn modern_policy_rejects_classic_preloaded_format_fonts_before_execution() {
    let format = crate::with_engine_world(World::memory(), |stores| {
        EngineMode::Tex82.prepare_initex(stores);
        let font = stores
            .command_context()
            .expect("admit classic format font")
            .intern_font(tex_fonts::LoadedFont::new(
                "classic-format-font",
                "classic-format-font.tfm",
                [1; 8],
                0,
                tex_state::scaled::Scaled::from_raw(10 << 16),
                tex_state::scaled::Scaled::from_raw(10 << 16),
                vec![tex_state::scaled::Scaled::from_raw(0); 7],
                tex_fonts::FontMetrics::default(),
            ));
        stores
            .assign_current_font(font, tex_state::AssignmentScope::Global)
            .expect("retain classic format font");
        crate::run_memory_collecting_initex_artifacts_with_profile(
            "\\dump",
            stores,
            tex_command::CommandProfile::TEX82,
        )
        .expect("dump classic font format")
        .format_dump
        .expect("classic font format dump")
        .image
    })
    .expect("fresh classic-font format universe");
    let mut session = VirtualCompileSession::new(SessionOptions {
        format: Some(format.into_bytes()),
        font_layout_policy: tex_fonts::FontLayoutPolicy::OpenTypePreferred,
        ..SessionOptions::default()
    })
    .expect("session construction remains side-effect free");
    session
        .add_user_file("main.tex", b"\\message{must-not-run}\\end".to_vec())
        .expect("source");
    let CompileAttemptResult::Error(CompileError::Font(message)) = session.compile_attempt() else {
        panic!("classic preloaded format must be rejected");
    };
    assert!(message.contains("selected through typed resources before layout"));
}

#[test]
fn source_session_installs_positive_prefetch_responses_for_the_next_attempt() {
    let request = |name: &str| {
        ResourceRequest::File(FileRequest::new(
            FileRequestKey::new(FileKind::TexInput, name).expect("request key"),
            name,
        ))
    };
    let mut formatted = VirtualCompileSession::new(SessionOptions {
        initial_prefetch_hints: Some(
            vec![
                request("remote.tex"),
                request("required.tex"),
                request("local.tex"),
                request("remote.tex"),
            ]
            .into_boxed_slice(),
        ),
        ..SessionOptions::default()
    })
    .expect("formatted session");
    formatted
        .add_user_file("main.tex", b"\\input required \\end".to_vec())
        .expect("main");
    formatted
        .add_user_file("local.tex", b"local".to_vec())
        .expect("local closure override");

    let CompileAttemptResult::NeedResources(first) = formatted.compile_attempt() else {
        panic!("first format miss should request resources");
    };
    assert_eq!(first.required.len(), 1);
    let ResourceRequest::File(required) = &first.required[0] else {
        unreachable!();
    };
    assert_eq!(required.key().name(), "required.tex");
    assert_eq!(required.original_name(), "required");
    assert_eq!(first.prefetch_hints, vec![request("remote.tex")]);

    let ResourceRequest::File(required) = first.required[0].clone() else {
        unreachable!();
    };
    let ResourceRequest::File(remote) = first.prefetch_hints[0].clone() else {
        unreachable!();
    };
    assert!(matches!(
        formatted.provide_resources(vec![ResourceResponse::FileUnavailable(
            remote.key().clone()
        )]),
        Err(CompileError::UnexpectedResourceResponse(name)) if name == "remote.tex"
    ));
    formatted
        .provide_resources(vec![
            ResourceResponse::File(ResolvedFile {
                request: required.key().clone(),
                virtual_path: "/texlive/required.tex".into(),
                bytes: b"\\input remote \\endinput".to_vec(),
                expected_digest: None,
            }),
            ResourceResponse::File(ResolvedFile {
                request: remote.key().clone(),
                virtual_path: "/texlive/remote.tex".into(),
                bytes: b"prefetched".to_vec(),
                expected_digest: None,
            }),
        ])
        .expect("required response");
    let CompileAttemptResult::Complete(_) = formatted.compile_attempt() else {
        panic!("the prefetched closure should complete on attempt two");
    };
    assert_eq!(formatted.attempts(), 2);
}

#[test]
fn formatted_session_reports_unsupported_schema_version() {
    let mut format = construct_test_format(EngineMode::Tex82, "\\dump").into_bytes();
    format[8..12].copy_from_slice(&9_u32.to_le_bytes());
    let mut session = VirtualCompileSession::new(SessionOptions {
        format: Some(format),
        ..SessionOptions::default()
    })
    .expect("format fits limits");
    session
        .add_user_file("main.tex", b"\\end".to_vec())
        .expect("main");

    let CompileAttemptResult::Error(CompileError::Format(message)) = session.compile_attempt()
    else {
        panic!("schema 9 must be rejected as a format error");
    };
    assert!(
        message.contains("unsupported Umber format version 9"),
        "{message}"
    );
}

#[test]
fn format_images_have_a_separate_size_ceiling_from_vfs_files() {
    assert!(check_format_image_bytes(SessionLimits::default().one_file_bytes + 1).is_ok());
    assert!(matches!(
        check_format_image_bytes(SessionLimits::FORMAT_IMAGE_BYTES + 1),
        Err(CompileError::LimitExceeded {
            resource: "format image bytes",
            limit: SessionLimits::FORMAT_IMAGE_BYTES,
            attempted,
        }) if attempted == SessionLimits::FORMAT_IMAGE_BYTES + 1
    ));
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
            ResourceRequest::Font(_) | ResourceRequest::PkFont(_) => None,
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
fn opentype_only_font_needs_no_tfm_and_exposes_synthesized_fontdimens() {
    let mut session = VirtualCompileSession::new(SessionOptions {
        outputs: OutputCapabilitySet::DVI.with(OutputCapability::Html),
        ..SessionOptions::default()
    })
    .expect("HTML session");
    session
        .add_user_file(
            "main.tex",
            br"\font\ot=opentype:cmu-serif-roman at 10pt \ot
               \message{space=\the\fontdimen2\ot,stretch=\the\fontdimen3\ot,shrink=\the\fontdimen4\ot,quad=\the\fontdimen6\ot}
               \shipout\hbox{A B}\end"
                .to_vec(),
        )
        .expect("main source");

    let missing = resources(session.compile_attempt());
    assert_eq!(missing.len(), 1);
    let font = match missing.into_iter().next().expect("font request") {
        ResourceRequest::Font(font) => font,
        ResourceRequest::File(file) => panic!("unexpected TFM request: {file:?}"),
        ResourceRequest::PkFont(pk) => panic!("unexpected PK request: {pk:?}"),
    };
    provide_cmu_font(&mut session, font);
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("OpenType-only compile should complete");
    };
    let terminal = String::from_utf8(output.terminal).expect("terminal UTF-8");
    assert!(terminal.contains("space="));
    assert!(terminal.contains("stretch="));
    assert!(terminal.contains("shrink="));
    assert!(terminal.contains("quad=10.0pt"));
    let html = String::from_utf8(output.html.expect("HTML output")).expect("HTML UTF-8");
    assert!(html.contains(">A B</text>"));
}

#[test]
fn html_only_opentype_retains_unicode_while_requested_dvi_rejects_it() {
    const SOURCE: &str =
        "\\font\\ot=opentype:cmu-serif-roman at 10pt \\ot \\shipout\\hbox{αЖ}\\end";
    let mut html_only = VirtualCompileSession::new(SessionOptions {
        engine: EngineMode::PdfTex,
        outputs: OutputCapabilitySet::HTML,
        ..SessionOptions::default()
    })
    .expect("HTML-only session");
    html_only
        .add_user_file("main.tex", SOURCE.as_bytes().to_vec())
        .expect("Unicode source");
    let font = resources(html_only.compile_attempt())
        .into_iter()
        .find_map(|request| match request {
            ResourceRequest::Font(font) => Some(font),
            ResourceRequest::File(_) | ResourceRequest::PkFont(_) => None,
        })
        .expect("OpenType request");
    provide_cmu_font(&mut html_only, font);
    let attempt = html_only.compile_attempt();
    let CompileAttemptResult::Complete(output) = attempt else {
        panic!("HTML-only Unicode compile should complete: {attempt:?}");
    };
    assert!(output.dvi.is_empty());
    let html = String::from_utf8(output.html.expect("HTML output")).expect("HTML UTF-8");
    assert!(html.contains(">αЖ</text>"), "{html}");
    assert!(html.contains("data-umber-codes=\"0x3b1,0x416\""));

    let output_id = html_only.rendered_output_id().expect("output identity");
    let (page, event) = rendered_text_address(&html, u32::from('α'));
    let alpha = current_render_location(
        html_only
            .rendered_source_location(page, event, Some(0), output_id, RevisionId::new(1))
            .expect("alpha source query"),
    );
    let zhe = current_render_location(
        html_only
            .rendered_source_location(page, event, Some(1), output_id, RevisionId::new(1))
            .expect("Cyrillic source query"),
    );
    let alpha_start = SOURCE.find('α').expect("alpha source offset");
    let zhe_start = SOURCE.find('Ж').expect("Cyrillic source offset");
    assert_eq!(
        (alpha.start as usize, alpha.end as usize),
        (alpha_start, alpha_start + 2)
    );
    assert_eq!(
        (zhe.start as usize, zhe.end as usize),
        (zhe_start, zhe_start + 2)
    );

    let mut joint = VirtualCompileSession::new(SessionOptions {
        engine: EngineMode::PdfTex,
        outputs: OutputCapabilitySet::DVI.with(OutputCapability::Html),
        ..SessionOptions::default()
    })
    .expect("joint session");
    joint
        .add_user_file("main.tex", SOURCE.as_bytes().to_vec())
        .expect("Unicode source");
    let font = resources(joint.compile_attempt())
        .into_iter()
        .find_map(|request| match request {
            ResourceRequest::Font(font) => Some(font),
            ResourceRequest::File(_) | ResourceRequest::PkFont(_) => None,
        })
        .expect("OpenType request");
    provide_cmu_font(&mut joint, font);
    let CompileAttemptResult::Error(error) = joint.compile_attempt() else {
        panic!("joint DVI/HTML Unicode compile must reject TeX82 DVI");
    };
    assert!(
        error
            .to_string()
            .contains("DVI TeX82 character code 945 is outside 0..=255"),
        "{error}"
    );
}

#[test]
fn html_only_is_independent_from_every_engine_compatibility_contract() {
    for engine in [
        EngineMode::Tex82,
        EngineMode::ETex,
        EngineMode::PdfTex,
        EngineMode::Latex,
        EngineMode::PdfLatex,
    ] {
        let mut session = VirtualCompileSession::new(SessionOptions {
            engine,
            outputs: OutputCapabilitySet::HTML,
            ..SessionOptions::default()
        })
        .unwrap_or_else(|error| panic!("{} HTML session: {error}", engine.name()));
        session
            .add_user_file("main.tex", br"\shipout\hbox{}\end".to_vec())
            .expect("main source");
        let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
            panic!("{} HTML-only session did not complete", engine.name());
        };
        assert!(output.dvi.is_empty());
        assert!(output.html.is_some());
    }
}

#[test]
fn pdf_capability_requires_a_pdftex_compatible_engine() {
    for engine in [EngineMode::Tex82, EngineMode::ETex, EngineMode::Latex] {
        let error = VirtualCompileSession::new(SessionOptions {
            engine,
            outputs: OutputCapabilitySet::PDF,
            ..SessionOptions::default()
        })
        .err()
        .expect("PDF capability must be rejected");
        assert!(matches!(
            error,
            CompileError::OutputCapability {
                capability: OutputCapability::Pdf,
                ..
            }
        ));
    }
}

#[test]
fn opentype_only_font_rejects_classic_math_family_assignment() {
    let mut session = VirtualCompileSession::new(SessionOptions {
        outputs: OutputCapabilitySet::DVI.with(OutputCapability::Html),
        ..SessionOptions::default()
    })
    .expect("HTML session");
    session
        .add_user_file(
            "main.tex",
            br"\font\ot=opentype:cmu-serif-roman \textfont0=\ot\end".to_vec(),
        )
        .expect("main source");
    let font = resources(session.compile_attempt())
        .into_iter()
        .find_map(|request| match request {
            ResourceRequest::Font(font) => Some(font),
            ResourceRequest::File(_) | ResourceRequest::PkFont(_) => None,
        })
        .expect("font request");
    provide_cmu_font(&mut session, font);
    let CompileAttemptResult::Error(error) = session.compile_attempt() else {
        panic!("classic math assignment should fail");
    };
    assert!(
        error
            .to_string()
            .contains("OpenType-only fonts cannot be assigned")
    );
}

#[test]
fn font_batches_accept_partial_unordered_responses_and_reject_conflicts() {
    let mut session = VirtualCompileSession::new(SessionOptions {
        outputs: OutputCapabilitySet::DVI.with(OutputCapability::Html),
        ..SessionOptions::default()
    })
    .expect("HTML session");
    session
        .add_user_file("cmr10.tfm", CMR10.to_vec())
        .expect("TFM");
    session
        .add_user_file(
            "main.tex",
            b"\\font\\tenrm=cmr10\\relax \\tenrm \\shipout\\hbox{A}\\end".to_vec(),
        )
        .expect("main source");
    let missing = resources(session.compile_attempt());
    let font = missing
        .iter()
        .find_map(|request| match request {
            ResourceRequest::Font(request) => Some(request.clone()),
            ResourceRequest::File(_) | ResourceRequest::PkFont(_) => None,
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

    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
}

#[test]
fn unavailable_openin_retries_into_tex_missing_file_semantics() {
    let mut session = session(
        "\\openin0=optional.cfg \\ifeof0 \\message{OPTIONAL-MISSING}\\else \\errmessage{unexpected optional file}\\fi \\end",
    );
    let missing = probes(session.compile_attempt());
    assert_eq!(missing.len(), 1);
    let key = missing[0].key().clone();
    session
        .provide_resources(vec![ResourceResponse::FileUnavailable(key.clone())])
        .expect("negative file response");
    session
        .provide_resources(vec![ResourceResponse::FileUnavailable(key.clone())])
        .expect("duplicate negative file response");
    assert!(matches!(
        session.provide_resources(vec![ResourceResponse::File(ResolvedFile {
            request: key,
            virtual_path: "/texlive/optional.cfg".to_owned(),
            bytes: Vec::new(),
            expected_digest: None,
        })]),
        Err(CompileError::ConflictingResolvedBinding(_))
    ));
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("negative response should allow optional probe to complete");
    };
    assert!(
        output
            .terminal
            .windows(b"OPTIONAL-MISSING".len())
            .any(|window| window == b"OPTIONAL-MISSING")
    );
}

#[test]
fn format_macro_reads_same_run_output_after_an_authoritative_missing_probe() {
    let format = construct_test_format(
        EngineMode::Tex82,
        "\\long\\def\\GenerateAfterProbe#1{\\openin1=\"#1\" \\ifeof1 \\message{OPTIONAL-MISSING}\\else \\errmessage{unexpected existing input}\\fi \\immediate\\openout1=#1 \\immediate\\write1{\\string\\message{GENERATED-READ}}\\immediate\\closeout1 \\input #1}\\dump",
    );
    let mut session = VirtualCompileSession::new(SessionOptions {
        format: Some(format.into_bytes()),
        ..SessionOptions::default()
    })
    .expect("formatted session");
    session
        .add_user_file(
            "main.tex",
            b"\\GenerateAfterProbe{roundtrip.cfg}\\end".to_vec(),
        )
        .expect("main source");

    let missing = probes(session.compile_attempt());
    assert_eq!(missing.len(), 1);
    session
        .provide_resources(vec![ResourceResponse::FileUnavailable(
            missing[0].key().clone(),
        )])
        .expect("negative file response");
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("same-run output should override an earlier negative acquisition binding");
    };
    assert!(
        output
            .terminal
            .windows(b"GENERATED-READ".len())
            .any(|window| window == b"GENERATED-READ")
    );
}

#[test]
fn generated_output_reopens_after_a_negative_probe_and_unrelated_suspension() {
    let format = construct_test_format(
        EngineMode::Tex82,
        "\\long\\def\\GenerateAfterProbe#1{\\openin1=\"#1\" \\ifeof1 \\message{OPTIONAL-MISSING}\\else \\errmessage{unexpected existing input}\\fi \\immediate\\openout15=#1 \\immediate\\write15{SNR, AWGN}\\immediate\\closeout15}\\dump",
    );
    let mut session = VirtualCompileSession::new(SessionOptions {
        format: Some(format.into_bytes()),
        ..SessionOptions::default()
    })
    .expect("formatted session");
    session
        .add_user_file(
            "main.tex",
            concat!(
                "\\GenerateAfterProbe{roundtrip.csv}",
                "\\input unrelated.cfg ",
                "\\openin0=roundtrip.csv ",
                "\\ifeof0 \\message{BARE-REOPEN-EOF}",
                "\\else \\read0 to\\bareline \\message{BARE-FIRST-LINE:[\\meaning\\bareline]}\\fi ",
                "\\closein0 ",
                "\\openin0=./roundtrip.csv ",
                "\\ifeof0 \\message{DOT-REOPEN-EOF}",
                "\\else \\read0 to\\dotline \\message{DOT-FIRST-LINE:[\\meaning\\dotline]}\\fi ",
                "\\closein0\\end"
            )
            .as_bytes()
            .to_vec(),
        )
        .expect("main source");

    let missing = probes(session.compile_attempt());
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].key().name(), "roundtrip.csv");
    session
        .provide_resources(vec![ResourceResponse::FileUnavailable(
            missing[0].key().clone(),
        )])
        .expect("negative file response");

    let unrelated = requests(session.compile_attempt());
    assert_eq!(unrelated.len(), 1);
    assert_eq!(unrelated[0].key().name(), "unrelated.cfg");
    session
        .provide_resources(vec![ResourceResponse::File(ResolvedFile {
            request: unrelated[0].key().clone(),
            virtual_path: "/texlive/unrelated.cfg".to_owned(),
            bytes: b"\\endinput".to_vec(),
            expected_digest: None,
        })])
        .expect("unrelated resource response");

    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("same-run output should reopen after the unrelated resource suspension");
    };
    assert!(
        output
            .terminal
            .windows(b"BARE-FIRST-LINE:[macro:->SNR, AWGN ]".len())
            .any(|window| window == b"BARE-FIRST-LINE:[macro:->SNR, AWGN ]"),
        "same-run output must override the earlier authoritative probe absence: {}",
        String::from_utf8_lossy(&output.terminal)
    );
    assert!(
        output
            .terminal
            .windows(b"DOT-FIRST-LINE:[macro:->SNR, AWGN ]".len())
            .any(|window| window == b"DOT-FIRST-LINE:[macro:->SNR, AWGN ]"),
        "the equivalent current-directory spelling must read the same output: {}",
        String::from_utf8_lossy(&output.terminal)
    );
}

#[test]
fn legacy_platform_openin_probes_are_not_found_without_weakening_vfs_paths() {
    let mut bracket = session(
        "\\openin0=[]texsys.aux \\ifeof0 \\message{BRACKET-MISSING}\\else \\errmessage{unexpected bracket file}\\fi \\end",
    );
    let missing = probes(bracket.compile_attempt());
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].original_name(), "[]texsys.aux");
    bracket
        .provide_resources(vec![ResourceResponse::FileUnavailable(
            missing[0].key().clone(),
        )])
        .expect("negative bracket-area response");
    let CompileAttemptResult::Complete(output) = bracket.compile_attempt() else {
        panic!("negative bracket-area probe should leave the stream closed");
    };
    assert!(String::from_utf8_lossy(&output.terminal).contains("BRACKET-MISSING"));

    let mut colon = session(
        "\\openin0=:texsys.aux \\ifeof0 \\message{COLON-MISSING}\\else \\errmessage{unexpected colon file}\\fi \\end",
    );
    let CompileAttemptResult::Complete(output) = colon.compile_attempt() else {
        panic!("foreign-platform colon probe should be an immediate miss");
    };
    assert!(String::from_utf8_lossy(&output.terminal).contains("COLON-MISSING"));
}

#[test]
fn unavailable_file_size_enquiry_is_a_probe_and_reaches_dump() {
    let mut session = VirtualCompileSession::new(SessionOptions {
        engine: EngineMode::PdfTex,
        ..SessionOptions::default()
    })
    .expect("session");
    session
        .add_user_file(
            "main.tex",
            b"\\message{SIZE=[\\pdffilesize{optional.cfg}]}\\dump".to_vec(),
        )
        .expect("main source");
    let missing = probes(session.compile_attempt());
    assert_eq!(missing.len(), 1);
    session
        .provide_resources(vec![ResourceResponse::FileUnavailable(
            missing[0].key().clone(),
        )])
        .expect("authoritative negative enquiry response");
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("negative file-size probe should resume through dump");
    };
    assert!(
        output
            .terminal
            .windows(7)
            .any(|window| window == b"SIZE=[]")
    );
    assert!(
        session
            .into_accepted_finalization()
            .expect("accepted format finalization")
            .format_dump
            .is_some()
    );
}

#[test]
fn pdf_file_enquiries_share_typed_virtual_file_responses() {
    let mut session = VirtualCompileSession::new(SessionOptions {
        engine: EngineMode::PdfTex,
        ..SessionOptions::default()
    })
    .expect("session");
    session
        .add_user_file("asset.bin", b"abc".to_vec())
        .expect("asset");
    session
        .add_user_file(
            "main.tex",
            br"\message{S=[\pdffilesize{asset.bin}]}\message{M=[\pdfmdfivesum file {asset.bin}]}\message{D=[\pdffiledump offset 1 length 2 {asset.bin}]}\message{T=[\pdffilemoddate{asset.bin}]}\end".to_vec(),
        )
        .expect("main source");

    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("registered virtual files should answer every enquiry without suspension");
    };
    let terminal = String::from_utf8_lossy(&output.terminal);
    assert!(terminal.contains("S=[3]"), "{terminal}");
    assert!(
        terminal.contains("M=[900150983CD24FB0D6963F7D28E17F72]"),
        "{terminal}"
    );
    assert!(terminal.contains("D=[6263]"), "{terminal}");
    // In-memory authored files deliberately carry no filesystem timestamp;
    // pdfTeX expands an available file with unavailable date metadata to
    // nothing, rather than confusing absence of metadata with a missing file.
    assert!(terminal.contains("T=[]"), "{terminal}");
}

#[test]
fn invalid_and_absolute_file_enquiries_are_missing_without_host_access() {
    let mut session = VirtualCompileSession::new(SessionOptions {
        engine: EngineMode::PdfTex,
        ..SessionOptions::default()
    })
    .expect("pdfTeX session");
    session
        .add_user_file(
            "main.tex",
            b"\\message{COLON=[\\pdffilesize{nul:}]}\\message{ABS=[\\pdffilesize{/dev/null}]}\\end"
                .to_vec(),
        )
        .expect("main source");
    let attempt = session.compile_attempt();
    let CompileAttemptResult::Complete(output) = attempt else {
        panic!(
            "file enquiries should treat invalid and unavailable host paths as missing: {attempt:?}"
        );
    };
    for expected in [b"COLON=[]".as_slice(), b"ABS=[]".as_slice()] {
        assert!(
            output
                .terminal
                .windows(expected.len())
                .any(|window| window == expected)
        );
    }
}

#[test]
fn invalid_legacy_platform_filesize_probe_expands_to_nothing() {
    let format = construct_test_format(EngineMode::Latex, "\\dump");
    let mut session = VirtualCompileSession::new(SessionOptions {
        format: Some(format.into_bytes()),
        // The loaded format records the TeX Live process capacities selected
        // by its LaTeX producer; do not execute it as the compact TeX82 binary.
        engine: EngineMode::Latex,
        ..SessionOptions::default()
    })
    .expect("formatted session");
    session
        .add_user_file(
            "main.tex",
            b"\\message{COLON-SIZE=[\\filesize{:texsys.aux}]}\\end".to_vec(),
        )
        .expect("main source");

    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("foreign-platform file-size probe should be an immediate miss");
    };
    let terminal = String::from_utf8_lossy(&output.terminal);
    assert!(terminal.contains("COLON-SIZE=[]"), "{terminal}");
}

#[test]
fn unavailable_font_answers_are_idempotent_and_conflict_with_bytes() {
    let mut session = VirtualCompileSession::new(SessionOptions {
        outputs: OutputCapabilitySet::DVI.with(OutputCapability::Html),
        ..SessionOptions::default()
    })
    .expect("HTML session");
    session
        .add_user_file("cmr10.tfm", CMR10.to_vec())
        .expect("TFM");
    session
        .add_user_file("main.tex", b"\\font\\tenrm=cmr10\\relax \\end".to_vec())
        .expect("main source");
    let missing = resources(session.compile_attempt());
    let font = missing
        .iter()
        .find_map(|request| match request {
            ResourceRequest::Font(request) => Some(request.clone()),
            ResourceRequest::File(_) | ResourceRequest::PkFont(_) => None,
        })
        .expect("font request");
    session
        .provide_resources(vec![ResourceResponse::FontUnavailable(font.key.clone())])
        .expect("negative font response");
    session
        .provide_resources(vec![ResourceResponse::FontUnavailable(font.key.clone())])
        .expect("duplicate negative font response");
    assert!(matches!(
        session.provide_resources(vec![ResourceResponse::Font(cmu_response(font))]),
        Err(CompileError::ConflictingResolvedBinding(_))
    ));
    if let CompileAttemptResult::NeedResources(resources) = session.compile_attempt() {
        assert!(
            resources
                .required
                .iter()
                .all(|request| matches!(request, ResourceRequest::File(_)))
        );
    }
}

#[test]
fn unavailable_font_and_tfm_answers_use_tex_missing_font_semantics() {
    let mut classic = session("\\font\\missing=absent \\message{FONT=[\\fontname\\missing]} \\end");
    let requested = requests(classic.compile_attempt());
    let [tfm] = requested.as_slice() else {
        panic!("expected one TFM request");
    };
    assert_eq!(tfm.key().kind(), FileKind::Tfm);
    classic
        .provide_resources(vec![ResourceResponse::FileUnavailable(tfm.key().clone())])
        .expect("authoritative negative TFM response");
    let CompileAttemptResult::Complete(output) = classic.compile_attempt() else {
        panic!("an unavailable TFM should use TeX's recoverable null-font behavior");
    };
    let terminal = String::from_utf8(output.terminal).expect("terminal UTF-8");
    assert!(
        terminal.contains("Metric (TFM) file not found"),
        "{terminal}"
    );

    let mut modern = VirtualCompileSession::new(SessionOptions {
        outputs: OutputCapabilitySet::DVI.with(OutputCapability::Html),
        ..SessionOptions::default()
    })
    .expect("HTML session");
    modern
        .add_user_file(
            "main.tex",
            b"\\font\\missing=opentype:absent \\message{FONT=[\\fontname\\missing]} \\shipout\\hbox{\\vrule width1pt height1pt} \\end".to_vec(),
        )
        .expect("main source");
    let requested = resources(modern.compile_attempt());
    let [ResourceRequest::Font(font)] = requested.as_slice() else {
        panic!("expected one OpenType font request");
    };
    modern
        .provide_resources(vec![ResourceResponse::FontUnavailable(font.key.clone())])
        .expect("authoritative negative OpenType response");
    let attempt = modern.compile_attempt();
    let CompileAttemptResult::Complete(output) = attempt else {
        panic!(
            "an unavailable OpenType font should use recoverable null-font behavior: {attempt:?}"
        );
    };
    let terminal = String::from_utf8(output.terminal).expect("terminal UTF-8");
    assert!(
        terminal.contains("OpenType resource not found"),
        "{terminal}"
    );
}

#[test]
fn active_font_definition_uses_tex82_synthetic_recorded_identifier() {
    // TeX82 §1257 synthesizes `FONT<char>` for an active font target and
    // records it at §1257's `common_ending`; §267's font rendering must use
    // that identifier rather than the bare active character.
    let mut session = session(r"\catcode`\?=13 \font?=cmr10 ?\showthe\font \end");
    session
        .add_user_file("cmr10.tfm", CMR10.to_vec())
        .expect("TFM fixture");
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("the bundled TFM should compile");
    };
    let terminal = String::from_utf8(output.terminal).expect("terminal UTF-8");
    assert!(terminal.contains("> \\FONT? ."), "{terminal}");
}

#[test]
fn malformed_tfm_bytes_use_recoverable_null_font_behavior() {
    let mut session = session("\\font\\broken=broken \\message{FONT=[\\fontname\\broken]} \\end");
    let requested = requests(session.compile_attempt());
    let [tfm] = requested.as_slice() else {
        panic!("expected one TFM request");
    };
    session
        .provide_resolved_file(
            tfm.key().clone(),
            "/texlive/broken.tfm",
            b"not a TFM".to_vec(),
        )
        .expect("malformed TFM bytes are provisioned before parsing");
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("malformed TFM should recover through nullfont")
    };
    let terminal = String::from_utf8(output.terminal).expect("terminal UTF-8");
    assert!(terminal.contains("Bad metric (TFM) file"), "{terminal}");
    assert!(terminal.contains("FONT=[nullfont]"), "{terminal}");
}

#[test]
fn invalid_mixed_batch_publishes_nothing() {
    let mut session = VirtualCompileSession::new(SessionOptions {
        outputs: OutputCapabilitySet::DVI.with(OutputCapability::Html),
        ..SessionOptions::default()
    })
    .expect("HTML session");
    session
        .add_user_file("cmr10.tfm", CMR10.to_vec())
        .expect("TFM");
    session
        .add_user_file("main.tex", b"\\font\\tenrm=cmr10\\relax \\end".to_vec())
        .expect("main source");
    let missing = resources(session.compile_attempt());
    let font = missing
        .iter()
        .find_map(|request| match request {
            ResourceRequest::Font(request) => Some(request.clone()),
            ResourceRequest::File(_) | ResourceRequest::PkFont(_) => None,
        })
        .expect("font request");
    let invalid_font = ResolvedFont {
        request: font.key,
        container: FontContainer::Woff2,
        bytes: b"wOF2".to_vec(),
        declared_object_ahash64: None,
        declared_program_identity: None,
        provenance: None,
        legacy_mapping: None,
    };
    assert!(
        session
            .provide_resources(vec![ResourceResponse::Font(invalid_font)])
            .is_err()
    );
    assert_eq!(session.resolved_file_count(), 0);
    assert_eq!(session.cached_file_bytes(), 0);
}

#[test]
fn requested_html_and_dvi_share_one_committed_compile() {
    const SOURCE: &[u8] = b"\\font\\tenrm=cmr10\\relax \\tenrm \\shipout\\hbox{A}\\end";
    let mut session = VirtualCompileSession::new(SessionOptions {
        outputs: OutputCapabilitySet::DVI.with(OutputCapability::Html),
        ..SessionOptions::default()
    })
    .expect("HTML session");
    session
        .add_user_file("cmr10.tfm", CMR10.to_vec())
        .expect("TFM");
    session
        .add_user_file("main.tex", SOURCE.to_vec())
        .expect("main source");
    let missing = resources(session.compile_attempt());
    let font = missing
        .iter()
        .find_map(|request| match request {
            ResourceRequest::Font(request) => Some(request.clone()),
            ResourceRequest::File(_) | ResourceRequest::PkFont(_) => None,
        })
        .expect("font request");
    provide_cmu_font(&mut session, font);
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("HTML compile should complete");
    };
    assert!(!output.dvi.is_empty());
    let joint_dvi = output.dvi.clone();
    let html = String::from_utf8(output.html.expect("HTML output")).expect("HTML UTF-8");
    let output_id = session
        .rendered_output_id()
        .expect("rendered output identity");
    assert!(html.contains("data-umber-page=\"1\" data-umber-revision=\"1\""));
    assert!(html.contains(&format!("data-umber-output=\"{output_id}\"")));
    assert!(html.contains("data-umber-baseline-sp"));
    assert!(html.contains(">A</text>"));
    assert!(output.html_assets.is_empty());

    let (page, event) = rendered_text_address(&html, u32::from(b'A'));
    let location = current_render_location(
        session
            .rendered_source_location(page, event, Some(0), output_id, RevisionId::new(1))
            .expect("source query"),
    );
    let retention_before = session.retention_metrics().expect("accepted retention");
    let repeated_location = current_render_location(
        session
            .rendered_source_location(page, event, Some(0), output_id, RevisionId::new(1))
            .expect("repeated source query"),
    );
    let retention_after = session.retention_metrics().expect("live retention");
    assert_eq!(retention_after, retention_before);
    assert_eq!(repeated_location, location);
    let start = SOURCE.iter().position(|byte| *byte == b'A').expect("A");
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

    let mut dvi_only =
        VirtualCompileSession::new(SessionOptions::default()).expect("DVI-only session");
    dvi_only
        .add_user_file("cmr10.tfm", CMR10.to_vec())
        .expect("DVI-only TFM");
    dvi_only
        .add_user_file("main.tex", SOURCE.to_vec())
        .expect("DVI-only main source");
    let font = resources(dvi_only.compile_attempt())
        .into_iter()
        .find_map(|request| match request {
            ResourceRequest::Font(request) => Some(request),
            ResourceRequest::File(_) | ResourceRequest::PkFont(_) => None,
        })
        .expect("DVI-only font request");
    provide_cmu_font(&mut dvi_only, font);
    let CompileAttemptResult::Complete(dvi_only_output) = dvi_only.compile_attempt() else {
        panic!("DVI-only compile should complete");
    };
    assert_eq!(joint_dvi, dvi_only_output.dvi);

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
    let (page, event) = rendered_text_address(&html, u32::from(b'B'));
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
        outputs: OutputCapabilitySet::DVI.with(OutputCapability::Html),
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
            ResourceRequest::File(_) | ResourceRequest::PkFont(_) => None,
        })
    {
        provide_cmu_font(&mut session, font);
    }
    assert!(matches!(
        session.compile_attempt(),
        CompileAttemptResult::Complete(_)
    ));
    assert!(matches!(
        session.add_user_file("cmr10.tfm", CMSY10.to_vec()),
        Err(CompileError::SessionAlreadyStarted)
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
        outputs: OutputCapabilitySet::DVI.with(OutputCapability::Html),
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
            ResourceRequest::File(_) | ResourceRequest::PkFont(_) => None,
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
    let (page, event) = rendered_text_address(&html, u32::from(b'B'));
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
fn modern_math_session_html_is_canonical_and_scriptless() {
    let source = b"\\font\\tenrm=cmr10 \\font\\sy=cmsy10 \\font\\ex=cmex10 \\textfont0=\\tenrm \\scriptfont0=\\tenrm \\scriptscriptfont0=\\tenrm \\textfont2=\\sy \\scriptfont2=\\sy \\scriptscriptfont2=\\sy \\textfont3=\\ex \\scriptfont3=\\ex \\scriptscriptfont3=\\ex \\mathcode`A=\"0041 \\tenrm \\shipout\\hbox{X${A\\over A}$}\\end";
    let mut session = VirtualCompileSession::new(SessionOptions {
        outputs: OutputCapabilitySet::DVI.with(OutputCapability::Html),
        font_layout_policy: tex_fonts::FontLayoutPolicy::OpenTypePreferred,
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
    let output = loop {
        match session.compile_attempt() {
            CompileAttemptResult::NeedResources(needs) => {
                let fonts = needs
                    .required
                    .into_iter()
                    .filter_map(|request| match request {
                        ResourceRequest::Font(request) => Some(request),
                        ResourceRequest::File(_) | ResourceRequest::PkFont(_) => None,
                    })
                    .collect::<Vec<_>>();
                assert_eq!(fonts.len(), 1, "one font dependency per suspension");
                provide_cmu_font(&mut session, fonts.into_iter().next().expect("font"));
            }
            CompileAttemptResult::Complete(output) => break output,
            other => panic!("math HTML compile should complete: {other:?}"),
        }
    };
    let html = String::from_utf8(output.html.expect("HTML output")).expect("HTML UTF-8");
    assert!(html.contains("class=\"umber-page\""));
    assert!(html.contains("class=\"umber-run-text\""));
    assert!(html.contains(">X</text>"));
    assert!(html.contains("A</text>"));
    assert!(html.contains("class=\"umber-rule\""));
    assert!(!html.contains("<script"));
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
fn command_fuel_configuration_uses_the_canonical_hard_maximum() {
    assert_eq!(
        SessionLimits::HARD_MAX.engine_fuel,
        tex_command::MAX_COMMAND_FUEL_LIMIT
    );
    assert_eq!(tex_command::MAX_COMMAND_FUEL_LIMIT, 100_000_000_000);

    VirtualCompileSession::new(SessionOptions {
        limits: SessionLimits {
            engine_fuel: tex_command::MAX_COMMAND_FUEL_LIMIT,
            ..SessionLimits::default()
        },
        ..SessionOptions::default()
    })
    .expect("canonical hard maximum is valid");

    for invalid in [0, tex_command::MAX_COMMAND_FUEL_LIMIT + 1, u64::MAX] {
        let result = VirtualCompileSession::new(SessionOptions {
            limits: SessionLimits {
                engine_fuel: invalid,
                ..SessionLimits::default()
            },
            ..SessionOptions::default()
        });
        let Err(CompileError::InvalidCommandFuelLimit(error)) = result else {
            panic!("expected typed command-fuel error for {invalid}");
        };
        assert_eq!(error.requested(), invalid);
        assert_eq!(
            error.to_string(),
            format!(
                "canonical command fuel limit {invalid} is outside 1..={}",
                tex_command::MAX_COMMAND_FUEL_LIMIT
            )
        );
    }
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
    let mut incremental = session(original);
    let CompileAttemptResult::Complete(first) = incremental.compile_attempt() else {
        panic!("initial revision should complete");
    };
    assert_eq!(incremental.revision(), Some(RevisionId::new(1)));

    let first_hash = incremental.content_hash().expect("accepted hash");
    let one = original.find("1pt").expect("height");
    incremental
        .apply_patch(SourcePatch {
            next_revision: RevisionId::new(2),
            base_revision: RevisionId::new(1),
            expected_hash: first_hash,
            range: one..one + 1,
            replacement: "2".to_owned(),
        })
        .expect("first patch");
    let CompileAttemptResult::Complete(second) = incremental.compile_attempt() else {
        panic!("second revision should complete");
    };
    assert_ne!(first.dvi, second.dvi);
    assert_eq!(incremental.revision(), Some(RevisionId::new(2)));

    let second_hash = incremental.content_hash().expect("second hash");
    incremental
        .apply_patch(SourcePatch {
            next_revision: RevisionId::new(3),
            base_revision: RevisionId::new(2),
            expected_hash: second_hash,
            range: one..one + 1,
            replacement: "3".to_owned(),
        })
        .expect("second patch");
    let CompileAttemptResult::Complete(third) = incremental.compile_attempt() else {
        panic!("third revision should complete");
    };
    assert_eq!(incremental.revision(), Some(RevisionId::new(3)));

    let final_source = original.replacen("1pt", "3pt", 1);
    let mut cold_session = session(&final_source);
    let CompileAttemptResult::Complete(cold) = cold_session.compile_attempt() else {
        panic!("cold final revision should complete");
    };
    assert_eq!(
        third, cold,
        "retained multi-run output must equal cold output"
    );

    let stale = incremental.apply_patch(SourcePatch {
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

#[test]
fn pdfximage_uses_typed_image_retry_and_accepts_png_metadata() {
    let source = "\\pdfoutput=1 \\
        \\message{INITIAL=\\the\\pdflastximagepages/\\the\\pdflastximagecolordepth} \\
        \\pdfximage width 20pt height 10pt depth 2pt \"figure.png\" \\
        \\message{IMAGE=\\the\\pdflastximage/\\the\\pdflastximagepages/\\the\\pdflastximagecolordepth} \\
        \\pdfrefximage\\pdflastximage \\
        \\message{REUSE=\\the\\pdflastximagepages/\\the\\pdflastximagecolordepth} \\
        \\end";
    let mut session = VirtualCompileSession::new(SessionOptions {
        engine: EngineMode::PdfTex,
        outputs: OutputCapabilitySet::PDF,
        ..SessionOptions::default()
    })
    .expect("session");
    session
        .add_user_file("main.tex", source.as_bytes().to_vec())
        .expect("main file");

    let requested = requests(session.compile_attempt());
    assert_eq!(requested.len(), 1);
    assert_eq!(requested[0].key().kind(), FileKind::Image);
    assert_eq!(requested[0].key().name(), "figure.png");

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(&13_u32.to_be_bytes());
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&40_u32.to_be_bytes());
    png.extend_from_slice(&20_u32.to_be_bytes());
    png.extend_from_slice(&[8, 2, 0, 0, 0]);
    session
        .provide_resolved_file(requested[0].key().clone(), "/texlive/figure.png", png)
        .expect("provide PNG");
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("retried image compile should complete");
    };
    let terminal = String::from_utf8_lossy(&output.terminal);
    assert!(terminal.contains("INITIAL=0/0"), "{terminal}");
    assert!(terminal.contains("IMAGE=1/1/8"), "{terminal}");
    assert!(terminal.contains("REUSE=1/8"), "{terminal}");

    let end = source.find("\\end").expect("end marker");
    session
        .apply_patch(SourcePatch {
            next_revision: RevisionId::new(2),
            base_revision: RevisionId::new(1),
            expected_hash: session.content_hash().expect("accepted hash"),
            range: end..end,
            replacement: "% retained replay\n".to_owned(),
        })
        .expect("comment-only patch");
    let CompileAttemptResult::Complete(replayed) = session.compile_attempt() else {
        panic!("retained image compile should complete without another request");
    };
    assert_eq!(replayed.terminal, output.terminal);
    assert_eq!(session.revision(), Some(RevisionId::new(2)));
}

#[test]
fn pdfximage_expands_macro_page_box_without_leaking_it_into_the_file_name() {
    let source = concat!(
        "\\pdfoutput=1 ",
        "\\def\\empty{}",
        "\\def\\page{}",
        "\\def\\decode{}",
        "\\let\\ifinterpolate\\iffalse",
        "\\let\\iftransgroup\\iftrue",
        "\\def\\pagebox{cropbox}",
        "\\pdfximage",
        "\\ifnum0",
        "\\ifx\\decode\\empty\\else1\\fi",
        "\\ifinterpolate1\\fi",
        "\\iftransgroup1\\fi",
        ">0 attr{",
        "\\ifx\\decode\\empty\\else/Decode[\\decode]\\fi",
        "\\iftransgroup/Group<</S/Transparency/K false/I false>>\\fi",
        "\\ifinterpolate/Interpolate true\\fi",
        "}\\fi",
        "\\ifx\\page\\empty\\else page \\page\\fi",
        "\\pagebox{figure.png} ",
        "\\end",
    );
    let mut session = VirtualCompileSession::new(SessionOptions {
        engine: EngineMode::PdfTex,
        limits: SessionLimits {
            engine_fuel: 1_000_000,
            ..SessionLimits::default()
        },
        ..SessionOptions::default()
    })
    .expect("session");
    session
        .add_user_file("main.tex", source.as_bytes().to_vec())
        .expect("main file");

    let requested = requests(session.compile_attempt());
    let [request] = requested.as_slice() else {
        panic!("expected one image request, got {requested:?}");
    };
    assert_eq!(request.key().kind(), FileKind::Image);
    assert_eq!(request.key().name(), "figure.png");

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(&13_u32.to_be_bytes());
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&40_u32.to_be_bytes());
    png.extend_from_slice(&20_u32.to_be_bytes());
    png.extend_from_slice(&[8, 2, 0, 0, 0]);
    session
        .provide_resolved_file(request.key().clone(), "/texlive/figure.png", png)
        .expect("provide PNG");
    assert!(
        matches!(session.compile_attempt(), CompileAttemptResult::Complete(_)),
        "resumed image scan must retain the complete page-box token sequence"
    );
}

#[test]
fn pdfximage_distinguishes_unavailable_and_malformed_resources() {
    let source = "\\pdfoutput=1 \\pdfximage figure.png \\end";

    let mut unavailable = VirtualCompileSession::new(SessionOptions {
        engine: EngineMode::PdfTex,
        ..SessionOptions::default()
    })
    .expect("PDF session");
    unavailable
        .add_user_file("main.tex", source.as_bytes().to_vec())
        .expect("main file");
    let requested = requests(unavailable.compile_attempt());
    let [request] = requested.as_slice() else {
        panic!("expected one image request");
    };
    unavailable
        .provide_resources(vec![ResourceResponse::FileUnavailable(
            request.key().clone(),
        )])
        .expect("authoritative negative image response");
    assert!(matches!(
        unavailable.compile_attempt(),
        CompileAttemptResult::Error(CompileError::Diagnostic(diagnostic))
            if diagnostic.message.contains("image is unavailable")
    ));

    let mut malformed = VirtualCompileSession::new(SessionOptions {
        engine: EngineMode::PdfTex,
        ..SessionOptions::default()
    })
    .expect("PDF session");
    malformed
        .add_user_file("main.tex", source.as_bytes().to_vec())
        .expect("main file");
    let requested = requests(malformed.compile_attempt());
    let [request] = requested.as_slice() else {
        panic!("expected one image request");
    };
    malformed
        .provide_resolved_file(
            request.key().clone(),
            "/texlive/figure.png",
            b"not an image".to_vec(),
        )
        .expect("malformed image bytes are provisioned before parsing");
    match malformed.compile_attempt() {
        CompileAttemptResult::Error(CompileError::Diagnostic(diagnostic)) => assert!(
            diagnostic
                .message
                .contains("image type is not PDF, PNG, or JPEG"),
            "{}",
            diagnostic.message
        ),
        other => panic!("malformed image must be a fatal engine error: {other:?}"),
    }
}
