use tex_state::{Universe, World};

use super::*;

const CMR10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");

fn session(main: &str) -> VirtualCompileSession {
    let mut session = VirtualCompileSession::new(SessionOptions::default()).expect("session");
    session
        .add_user_file("main.tex", main.as_bytes().to_vec())
        .expect("main file");
    session
}

fn requests(result: CompileAttemptResult) -> Vec<FileRequest> {
    match result {
        CompileAttemptResult::NeedFiles(requests) => requests,
        other => panic!("expected missing files, got {other:#?}"),
    }
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
            (FileKind::Tfm, "one.tfm"),
            (FileKind::Tfm, "two.tfm"),
            (FileKind::TexInput, "later.tex"),
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
    let missing = requests(session.compile_attempt());
    assert_eq!(missing.len(), 1);
    session
        .provide_resolved_file(
            missing[0].key().clone(),
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
fn requested_html_and_dvi_share_one_committed_compile() {
    use sha2::{Digest, Sha256};
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
    let woff2 = b"deterministic wasm font".to_vec();
    let mut encoding = vec![None; 256];
    encoding[usize::from(b'A')] = Some("A".to_owned());
    session
        .add_html_font(SessionWebFont {
            name: "cmr10".to_owned(),
            tfm_content_hash_hex: tex_state::ContentHash::from_bytes(CMR10).hex(),
            sha256: Sha256::digest(&woff2).into(),
            woff2,
            encoding,
            provenance: "test embedding license".to_owned(),
            embeddable: true,
        })
        .expect("HTML font");
    let missing = requests(session.compile_attempt());
    session
        .provide_resolved_file(
            missing[0].key().clone(),
            "/texlive/fonts/tfm/public/cm/cmr10.tfm",
            CMR10.to_vec(),
        )
        .expect("TFM");
    let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
        panic!("HTML compile should complete");
    };
    assert!(!output.dvi.is_empty());
    let html = String::from_utf8(output.html.expect("HTML output")).expect("HTML UTF-8");
    assert!(html.contains("data-umber-baseline-sp"));
    assert!(html.contains(">A<i class=\"umber-baseline\""));
    assert!(output.html_assets.is_empty());
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
    session.clear_distribution_cache();
    assert_eq!(session.resolved_file_count(), 0);
    assert_eq!(session.cached_file_bytes(), 0);
    assert_eq!(requests(session.compile_attempt()).len(), 1);
}
