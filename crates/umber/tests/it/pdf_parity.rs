use std::fs;
use std::process::Command;

use sha2::{Digest, Sha256};
use test_support::{corpus_root, pdf::normalize_structure, read_binary_fixture, read_fixture};

const PINNED_SOURCE_DATE_EPOCH: &str = "1783604160";

#[test]
#[allow(clippy::disallowed_methods)] // Hermetic CLI fixture boundary.
fn committed_pdftex_fixture_matches_structure_and_bytes() {
    let temp = tempfile::tempdir().expect("create PDF parity directory");
    let actual_path = temp.path().join("minimal_rule.pdf");
    let source = corpus_root().join("pdf/minimal_rule.tex");
    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .args(["run", "--pdftex", "--pdf"])
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg(&actual_path)
        .arg(source)
        .output()
        .expect("run committed PDF fixture");
    assert!(
        output.status.success(),
        "PDF fixture failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let actual = fs::read(actual_path).expect("read current Umber PDF");
    let expected_umber = read_binary_fixture("pdf", "minimal_rule", "umber.pdf");
    assert_eq!(
        actual, expected_umber,
        "deterministic Umber PDF bytes changed"
    );

    let reference = read_binary_fixture("pdf", "minimal_rule", "ref.pdf");
    let expected_structure = read_fixture("pdf", "minimal_rule", "structure");
    assert_eq!(
        normalize_structure(&reference).expect("normalize reference PDF"),
        expected_structure
    );
    assert_eq!(
        normalize_structure(&actual).expect("normalize current Umber PDF"),
        expected_structure
    );

    let raster = read_binary_fixture("pdf", "minimal_rule", "pgm");
    assert!(
        raster.starts_with(b"P5\n10 5\n255\n"),
        "unexpected raster dimensions"
    );
    let expected_attestation = format!(
        "pdf-render-v1\nrenderer pdftoppm version 25.08.0\narguments -r 72 -gray -singlefile\ncomparison exact-gray-pixels\nreference-pdf-sha256 {}\number-pdf-sha256 {}\npgm-sha256 {}\n",
        digest(&reference),
        digest(&expected_umber),
        digest(&raster),
    );
    assert_eq!(
        read_fixture("pdf", "minimal_rule", "render"),
        expected_attestation,
        "committed renderer attestation is stale"
    );
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
