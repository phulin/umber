use std::fs;
use std::process::Command;

use sha2::{Digest, Sha256};
use test_support::{
    corpus_cases, corpus_root, pdf::normalize_structure, read_binary_fixture, read_fixture,
};
use tex_state::Universe;

const PINNED_SOURCE_DATE_EPOCH: &str = "1783604160";

#[test]
#[allow(clippy::disallowed_methods)] // Hermetic CLI fixture boundary.
fn committed_pdftex_fixtures_match_structure_and_bytes() {
    for case in corpus_cases("pdf") {
        assert_committed_case(case.name());
    }
}

#[allow(clippy::disallowed_methods)] // Hermetic CLI fixture boundary.
fn assert_committed_case(case: &str) {
    let temp = tempfile::tempdir().expect("create PDF parity directory");
    let actual_path = temp.path().join(format!("{case}.pdf"));
    let source = corpus_root().join("pdf").join(format!("{case}.tex"));
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
    let expected_umber = read_binary_fixture("pdf", case, "umber.pdf");
    assert_eq!(
        actual, expected_umber,
        "deterministic Umber PDF bytes changed"
    );

    let reference = read_binary_fixture("pdf", case, "ref.pdf");
    let expected_structure = read_fixture("pdf", case, "structure");
    assert_eq!(
        normalize_structure(&reference).expect("normalize reference PDF"),
        expected_structure
    );
    assert_eq!(
        normalize_structure(&actual).expect("normalize current Umber PDF"),
        expected_structure
    );

    let raster = read_binary_fixture("pdf", case, "pgm");
    assert!(
        raster.starts_with(b"P5\n") && raster.windows(5).any(|bytes| bytes == b"\n255\n"),
        "unexpected raster header for pdf/{case}"
    );
    let expected_attestation = format!(
        "pdf-render-v1\nrenderer pdftoppm version 25.08.0\narguments -r 72 -gray -singlefile\ncomparison exact-gray-pixels\nreference-pdf-sha256 {}\number-pdf-sha256 {}\npgm-sha256 {}\n",
        digest(&reference),
        digest(&expected_umber),
        digest(&raster),
    );
    assert_eq!(
        read_fixture("pdf", case, "render"),
        expected_attestation,
        "committed renderer attestation is stale for pdf/{case}"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Committed corpus fixture boundary.
fn object_dictionary_pdf_replays_to_identical_bytes_and_state() {
    let source = fs::read_to_string(corpus_root().join("pdf/object_dictionaries.tex"))
        .expect("read object dictionary parity source");
    let mut stores = Universe::default();
    umber::prepare_pdftex_run_stores(&mut stores);
    stores
        .begin_retained_session()
        .expect("retained replay session starts");
    let checkpoint = stores.snapshot();

    umber::run_memory_with_stores(&source, &mut stores).expect("first PDF execution");
    let first_artifacts = stores.world().committed_artifacts().to_vec();
    let first = umber::pdf_from_committed_artifacts(&mut stores, &first_artifacts)
        .expect("first PDF finalization");
    let first_hash = stores.snapshot().state_hash();

    stores.rollback(&checkpoint);
    umber::run_memory_with_stores(&source, &mut stores).expect("replayed PDF execution");
    let replayed_artifacts = stores.world().committed_artifacts().to_vec();
    let replayed = umber::pdf_from_committed_artifacts(&mut stores, &replayed_artifacts)
        .expect("replayed PDF finalization");

    assert_eq!(replayed, first, "rollback replay changed final PDF bytes");
    assert_eq!(
        stores.snapshot().state_hash(),
        first_hash,
        "rollback replay changed the finalized PDF ledger hash"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Hermetic CLI fixture boundary.
fn committed_embedded_font_fixtures_match_bytes_structure_and_attestations() {
    for case in [
        "embedded_type1",
        "embedded_truetype",
        "embedded_subset_type1",
        "embedded_subset_truetype",
        "embedded_subset_omit",
        "embedded_subset_controls_negative",
        "pk_bitmap_300",
        "pk_bitmap_600",
    ] {
        check_embedded_font_case(case);
    }
}

#[allow(clippy::disallowed_methods)]
fn check_embedded_font_case(case: &str) {
    let temp = tempfile::tempdir().expect("create embedded-font parity directory");
    let source_name = format!("{case}.tex");
    fs::copy(
        corpus_root().join("pdf").join(&source_name),
        temp.path().join(&source_name),
    )
    .expect("stage embedded-font source");
    fs::copy(
        corpus_root().join("../../crates/tex-fonts/tests/fixtures/cm/cmr10.tfm"),
        temp.path().join("cmr10.tfm"),
    )
    .expect("stage cmr10 TFM");
    if case.starts_with("pk_bitmap_") {
        let dpi = case.trim_start_matches("pk_bitmap_");
        fs::copy(
            corpus_root().join("pdf").join(format!("cmr10.{dpi}pk")),
            temp.path().join(format!("cmr10.{dpi}pk")),
        )
        .expect("stage committed PK program");
    } else if matches!(
        case,
        "embedded_type1"
            | "embedded_subset_type1"
            | "embedded_subset_omit"
            | "embedded_subset_controls_negative"
    ) {
        fs::copy(
            corpus_root().join("pdf/embedded_type1.pfb"),
            temp.path().join("cmr10.pfb"),
        )
        .expect("stage committed Type1 program");
    } else {
        let woff2 = include_bytes!("../../../umber-wasm/assets/cmu-serif-500-roman.woff2");
        let program = tex_fonts::PdfTrueTypeProgram::from_woff2(woff2)
            .expect("decode committed TrueType fixture");
        fs::write(temp.path().join("cmu-serif.ttf"), program.bytes())
            .expect("stage decoded TrueType program");
        if case == "embedded_subset_truetype" {
            fs::copy(
                corpus_root().join("pdf/fixture.enc"),
                temp.path().join("fixture.enc"),
            )
            .expect("stage subset encoding");
        }
    }

    let actual_path = temp.path().join(format!("{case}.umber.pdf"));
    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .args(["run", "--pdftex", "--pdf"])
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .env("TEXFONTS", temp.path())
        .arg(&actual_path)
        .arg(temp.path().join(&source_name))
        .output()
        .expect("run embedded-font PDF fixture");
    assert!(
        output.status.success(),
        "{case} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let actual = fs::read(actual_path).expect("read embedded-font PDF");
    let expected_umber = read_binary_fixture("pdf", case, "umber.pdf");
    assert_eq!(actual, expected_umber, "deterministic {case} bytes changed");
    assert_eq!(
        normalize_structure(&actual).expect("normalize embedded-font PDF"),
        read_fixture("pdf", case, "umber.structure")
    );
    let reference = read_binary_fixture("pdf", case, "ref.pdf");
    assert_eq!(
        normalize_structure(&reference).expect("normalize reference font PDF"),
        read_fixture("pdf", case, "ref.structure")
    );
    let actual_structure = normalize_structure(&actual).expect("normalize embedded-font PDF");
    let reference_structure =
        normalize_structure(&reference).expect("normalize reference font PDF");
    match case {
        "embedded_subset_type1" => {
            assert!(actual_structure.contains("/ToUnicode"));
            assert!(actual_structure.contains("/CharSet"));
            assert!(reference_structure.contains("/ToUnicode"));
            assert!(reference_structure.contains("/CharSet"));
        }
        "embedded_subset_omit" => {
            assert!(!actual_structure.contains("/CharSet"));
            assert!(!reference_structure.contains("/CharSet"));
        }
        "embedded_subset_controls_negative" => {
            assert!(!actual_structure.contains("/ToUnicode"));
            assert!(!actual_structure.contains("/CharSet"));
            assert!(!reference_structure.contains("/ToUnicode"));
            assert!(!reference_structure.contains("/CharSet"));
        }
        "pk_bitmap_300" => {
            assert!(actual_structure.contains("/Subtype /Type3"));
            assert!(actual_structure.contains("/FontMatrix [0.024 0 0 0.024 0 0]"));
            assert!(reference_structure.contains("/Subtype /Type3"));
            assert!(reference_structure.contains("/FontMatrix [0.024 0 0 0.024 0 0]"));
        }
        "pk_bitmap_600" => {
            assert!(actual_structure.contains("/Subtype /Type3"));
            assert!(actual_structure.contains("/FontMatrix [0.012 0 0 0.012 0 0]"));
            assert!(reference_structure.contains("/Subtype /Type3"));
            assert!(reference_structure.contains("/FontMatrix [0.012 0 0 0.012 0 0]"));
        }
        _ => {}
    }
    let expected_extract = read_binary_fixture("pdf", case, "extract");
    if case.starts_with("embedded_subset_") {
        assert!(
            !expected_extract.trim_ascii().is_empty(),
            "pinned Poppler extraction for {case} is empty"
        );
    } else {
        let extracted = lopdf::Document::load_mem(&actual)
            .expect("parse embedded-font PDF")
            .extract_text(&[1])
            .expect("extract embedded-font text");
        assert_eq!(
            extracted.trim().as_bytes(),
            expected_extract.trim_ascii(),
            "lopdf extraction drift for {case}"
        );
    }

    let raster = read_binary_fixture("pdf", case, "pgm");
    let expected_attestation = format!(
        "pdf-render-v2\nrenderer pdftoppm version 25.08.0\narguments -r 72 -gray -singlefile\ncomparison max-gray-delta 2\nextractor pdftotext version 25.08.0\nextraction exact-utf8\nreference-pdf-sha256 {}\number-pdf-sha256 {}\npgm-sha256 {}\nextract-sha256 {}\n",
        digest(&reference),
        digest(&expected_umber),
        digest(&raster),
        digest(&expected_extract),
    );
    assert_eq!(read_fixture("pdf", case, "render"), expected_attestation);
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
