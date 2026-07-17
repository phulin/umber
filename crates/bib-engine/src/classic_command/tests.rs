use super::*;
use crate::{FileKind, ResolvedFile, VfsLimits};

#[test]
fn accepts_a_job_name_and_uses_the_classic_aux_suffix() {
    let command = ClassicBibCommand::parse(["paper"]).expect("command");
    assert_eq!(command.aux_path().as_str(), "/job/paper.aux");
    assert_eq!(
        ClassicBibCommand::parse(["--quiet", "paper"])
            .expect_err("unsupported option")
            .output()
            .status(),
        BibExitStatus::InvalidInvocation
    );
}

#[test]
fn warning_and_recoverable_error_histories_publish_and_fatal_history_is_partial() {
    let command = ClassicBibCommand::parse(["paper"]).expect("command");
    let mut files = FileProvisioner::new(VfsLimits::default()).expect("VFS");
    files
        .register_user(
            command.aux_path().clone(),
            b"\\citation{one}\n\\bibdata{refs}\n\\bibstyle{plain}\n".to_vec(),
        )
        .expect("AUX");
    let warning = command.execute_provisioned(&mut files, |request| {
        let bytes = match request.key().kind() {
            FileKind::ClassicBibData => b"@book{one, title = \"One\"}".to_vec(),
            FileKind::BibStyle => {
                b"ENTRY { title } { } { } FUNCTION {emit} { title write$ } READ ITERATE {emit}"
                    .to_vec()
            }
            kind => panic!("unexpected resource kind: {kind:?}"),
        };
        Some(ResolvedFile {
            request: request.key().clone(),
            virtual_path: format!("/texlive/classic/{}", request.key().name()),
            bytes,
            expected_digest: None,
        })
    });
    assert_eq!(warning.status(), BibExitStatus::Success);
    assert_eq!(
        warning
            .result()
            .expect("result")
            .files()
            .find(|file| file.path().as_str() == "/job/paper.bbl")
            .expect("BBL")
            .bytes(),
        b"One"
    );

    let recoverable = ClassicBibCommand::parse(["recoverable"])
        .expect("command")
        .execute_provisioned(&mut fixture_files("recoverable"), |request| {
            let bytes = match request.key().kind() {
                FileKind::ClassicBibData => b"@book{one}".to_vec(),
                FileKind::BibStyle => {
                    b"ENTRY { } { } { } FUNCTION {bad} { pop$ } READ EXECUTE {bad}".to_vec()
                }
                kind => panic!("unexpected resource kind: {kind:?}"),
            };
            Some(ResolvedFile {
                request: request.key().clone(),
                virtual_path: format!("/texlive/classic/{}", request.key().name()),
                bytes,
                expected_digest: None,
            })
        });
    assert_eq!(recoverable.status(), BibExitStatus::ClassicExecutionError);
    assert!(
        recoverable
            .result()
            .expect("recoverable result")
            .files()
            .any(|file| file.path().as_str() == "/job/recoverable.bbl")
    );

    let fatal = ClassicBibCommand::parse(["fatal"])
        .expect("command")
        .execute_provisioned(&mut fixture_files("fatal"), |request| {
            let bytes = match request.key().kind() {
                FileKind::ClassicBibData => b"@book{one}".to_vec(),
                FileKind::BibStyle => {
                    b"ENTRY { } { } { } FUNCTION {bad} { \"x\" #1 + } READ EXECUTE {bad}".to_vec()
                }
                kind => panic!("unexpected resource kind: {kind:?}"),
            };
            Some(ResolvedFile {
                request: request.key().clone(),
                virtual_path: format!("/texlive/classic/{}", request.key().name()),
                bytes,
                expected_digest: None,
            })
        });
    assert_eq!(fatal.status(), BibExitStatus::OperationalFailure);
    assert!(
        fatal
            .result()
            .expect("fatal result")
            .partial_files()
            .any(|file| file.path().as_str() == "/job/fatal.bbl")
    );
}

#[test]
fn large_style_reallocation_precedes_style_warning_in_the_log_and_not_the_terminal() {
    let command = ClassicBibCommand::parse(["large-style"]).expect("command");
    let mut files = FileProvisioner::new(VfsLimits::default()).expect("VFS");
    files
        .register_user(
            command.aux_path().clone(),
            b"\\citation{one}\n\\bibdata{refs}\n\\bibstyle{large}\n".to_vec(),
        )
        .expect("AUX");
    let mut style = b"ENTRY {} {} {} FUNCTION {emit} { \"style says hi\" warning$ ".to_vec();
    for _ in 0..48 {
        style.extend_from_slice(b"skip$ ");
    }
    style.extend_from_slice(b"} FUNCTION {book} { } READ EXECUTE {emit}");
    let output = command.execute_provisioned(&mut files, |request| {
        let bytes = match request.key().kind() {
            FileKind::ClassicBibData => b"@book{one,}".to_vec(),
            FileKind::BibStyle => style.clone(),
            kind => panic!("unexpected resource kind: {kind:?}"),
        };
        Some(ResolvedFile {
            request: request.key().clone(),
            virtual_path: format!("/texlive/classic/{}", request.key().name()),
            bytes,
            expected_digest: None,
        })
    });
    assert_eq!(output.status(), BibExitStatus::Success, "{output:?}");
    assert_eq!(
        output.terminal(),
        b"This is BibTeX, Version 0.99d (TeX Live 2025)\nThe top-level auxiliary file: large-style.aux\nThe style file: large.bst\nDatabase file #1: refs.bib\nWarning--style says hi\n(There was 1 warning)\n",
    );
    let log = output
        .result()
        .expect("publishable result")
        .files()
        .find(|file| file.path().as_str() == "/job/large-style.blg")
        .expect("BLG")
        .bytes();
    let reallocation = b"Reallocated singl_function (elt_size=4) to 100 items from 50.\n";
    let warning = b"Warning--style says hi\n";
    let reallocation_at = log
        .windows(reallocation.len())
        .position(|window| window == reallocation)
        .expect("Web2C reallocation record");
    let warning_at = log
        .windows(warning.len())
        .position(|window| window == warning)
        .expect("style warning record");
    assert!(reallocation_at < warning_at);
    assert!(
        !output
            .terminal()
            .windows(reallocation.len())
            .any(|window| window == reallocation)
    );
}

fn fixture_files(stem: &str) -> FileProvisioner {
    let mut files = FileProvisioner::new(VfsLimits::default()).expect("VFS");
    files
        .register_user(
            VirtualPath::user(&format!("{stem}.aux")).expect("AUX path"),
            format!("\\citation{{one}}\n\\bibdata{{refs}}\n\\bibstyle{{plain}}\n").into_bytes(),
        )
        .expect("AUX");
    files
}
