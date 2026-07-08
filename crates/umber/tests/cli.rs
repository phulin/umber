use std::process::Command;

#[test]
fn exits_successfully() {
    let status = Command::new(env!("CARGO_BIN_EXE_umber"))
        .status()
        .expect("failed to run umber binary");

    assert!(status.success());
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side fixture discovery and expected-output reads.
fn lex_dump_prints_stable_token_format_for_corpus() {
    let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/corpus/lexer");
    for entry in std::fs::read_dir(&corpus).expect("read lexer corpus") {
        let path = entry.expect("read corpus entry").path();
        if path.extension().and_then(std::ffi::OsStr::to_str) != Some("tex") {
            continue;
        }
        let stem = path.file_stem().expect("fixture stem");
        let expected = path.with_file_name(format!("{}.expected.tokens", stem.to_string_lossy()));

        let output = Command::new(env!("CARGO_BIN_EXE_umber"))
            .arg("lex-dump")
            .arg(&path)
            .output()
            .expect("run umber lex-dump");

        assert!(
            output.status.success(),
            "lex-dump failed for {}:\n{}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let actual = String::from_utf8(output.stdout).expect("lex-dump output is utf-8");
        let expected = std::fs::read_to_string(&expected).expect("read expected tokens");
        assert_eq!(actual, expected, "fixture mismatch for {}", path.display());
    }
}
