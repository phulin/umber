//! Lexical and expansion dump output and diagnostic contracts.

use super::*;

#[test]
#[allow(clippy::disallowed_methods)] // host-side fixture discovery and expected-output reads.
fn lex_dump_prints_stable_token_format_for_corpus() {
    for case in corpus_cases("lexer") {
        let output = Command::new(env!("CARGO_BIN_EXE_umber"))
            .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
            .arg("lex-dump")
            .arg(case.source_path())
            .output()
            .expect("run umber lex-dump");

        assert!(
            output.status.success(),
            "lex-dump failed for {}:\n{}",
            case.source_path().display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let actual = String::from_utf8(output.stdout).expect("lex-dump output is utf-8");
        assert_matches_fixture("lexer", case.name(), "tokens", &actual);
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side fixture discovery and expected-output reads.
fn expand_dump_prints_stable_token_format_for_corpus() {
    for case in corpus_cases("expand") {
        let output = Command::new(env!("CARGO_BIN_EXE_umber"))
            .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
            .arg("expand-dump")
            .arg(case.source_path())
            .output()
            .expect("run umber expand-dump");

        assert!(
            output.status.success(),
            "expand-dump failed for {}:\n{}",
            case.source_path().display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let actual = String::from_utf8(output.stdout).expect("expand-dump output is utf-8");
        assert_matches_fixture("expand", case.name(), "tokens", &actual);
    }
}

#[test]
fn expand_dump_usage_errors_follow_lex_dump_shape() {
    let missing = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("expand-dump")
        .output()
        .expect("run umber expand-dump without path");
    assert!(!missing.status.success());
    assert_eq!(
        String::from_utf8(missing.stderr).expect("stderr is utf-8"),
        "umber: missing input path for expand-dump\n"
    );

    let extra = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("expand-dump")
        .arg("one.tex")
        .arg("two.tex")
        .output()
        .expect("run umber expand-dump with extra path");
    assert!(!extra.status.success());
    assert_eq!(
        String::from_utf8(extra.stderr).expect("stderr is utf-8"),
        "umber: expand-dump accepts exactly one input path\n"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn expand_dump_expansion_error_renders_primary_source_context() {
    let temp_dir = tempfile::tempdir().expect("create diagnostic temp dir");
    let source = temp_dir.path().join("undefined.tex");
    fs::write(&source, "\\undefined\n").expect("write diagnostic fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("expand-dump")
        .arg(&source)
        .output()
        .expect("run umber expand-dump diagnostic fixture");

    assert!(
        !output.status.success(),
        "undefined expand-dump should fail"
    );
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf-8");
    assert!(stderr.contains("Undefined control sequence \\undefined"));
    assert!(stderr.contains("undefined.tex:1:1"));
    assert!(stderr.contains("  1 | \\undefined"));
    assert!(stderr.contains("    | ^"));
    assert!(!stderr.contains("unknown origin"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn expand_dump_macro_error_renders_bounded_expansion_trace() {
    let temp_dir = tempfile::tempdir().expect("create macro diagnostic temp dir");
    let source = temp_dir.path().join("macro.tex");
    fs::write(&source, "\\def\\a{\\undefined X}\\a\n").expect("write diagnostic fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("expand-dump")
        .arg(&source)
        .output()
        .expect("run umber expand-dump macro diagnostic fixture");

    assert!(!output.status.success(), "macro expand-dump should fail");
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf-8");
    assert!(stderr.contains("Undefined control sequence \\undefined"));
    assert!(stderr.contains("macro.tex:1:8"));
    assert!(stderr.contains("expansion trace:"));
    assert!(stderr.contains("invoked at"));
    assert!(stderr.contains("defined at"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn expand_dump_recovered_execution_error_exits_successfully() {
    let temp_dir = tempfile::tempdir().expect("create execution diagnostic temp dir");
    let source = temp_dir.path().join("prefix.tex");
    fs::write(&source, "\\global X\n").expect("write diagnostic fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("expand-dump")
        .arg(&source)
        .output()
        .expect("run umber expand-dump execution diagnostic fixture");

    assert!(
        output.status.success(),
        "recovered prefix error should succeed"
    );
    assert!(
        output.stderr.is_empty(),
        "recovered error must not reach stderr"
    );
}
