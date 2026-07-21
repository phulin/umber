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
        let expected_structure = corpus_root()
            .join("pdf")
            .join(format!("{}.expected.structure", case.name()));
        if expected_structure.exists() {
            assert_committed_case(case.name());
        }
    }
}

#[test]
fn annotation_fixture_matches_page_ownership_and_rectangles() {
    let reference = read_binary_fixture("pdf", "annotations_running", "ref.pdf");
    let umber = read_binary_fixture("pdf", "annotations_running", "umber.pdf");
    let reference = annotation_projection(&reference);
    let umber = annotation_projection(&umber);
    assert_eq!(umber, reference, "annotation rectangle projection drifted");
    assert_eq!(umber.iter().map(Vec::len).collect::<Vec<_>>(), [2, 1]);
}

fn annotation_projection(bytes: &[u8]) -> Vec<Vec<(Vec<f64>, Vec<u8>)>> {
    let document = lopdf::Document::load_mem(bytes).expect("parse annotation fixture");
    let mut owned = std::collections::BTreeSet::new();
    document
        .get_pages()
        .into_values()
        .map(|page_id| {
            let page = document
                .get_object(page_id)
                .and_then(lopdf::Object::as_dict)
                .expect("page dictionary");
            page.get(b"Annots")
                .and_then(lopdf::Object::as_array)
                .expect("page annotation array")
                .iter()
                .map(|entry| {
                    let id = entry.as_reference().expect("indirect annotation");
                    assert!(owned.insert(id), "annotation object is shared by pages");
                    let annotation = document
                        .get_object(id)
                        .and_then(lopdf::Object::as_dict)
                        .expect("annotation dictionary");
                    assert_eq!(
                        annotation
                            .get(b"Type")
                            .and_then(lopdf::Object::as_name)
                            .expect("annotation type"),
                        b"Annot"
                    );
                    let rect = annotation
                        .get(b"Rect")
                        .and_then(lopdf::Object::as_array)
                        .expect("annotation rectangle")
                        .iter()
                        .map(|number| match number {
                            lopdf::Object::Integer(value) => *value as f64,
                            lopdf::Object::Real(value) => f64::from(*value),
                            _ => panic!("annotation rectangle value is numeric"),
                        })
                        .collect();
                    let subtype = annotation
                        .get(b"Subtype")
                        .and_then(lopdf::Object::as_name)
                        .expect("annotation subtype")
                        .to_vec();
                    (rect, subtype)
                })
                .collect()
        })
        .collect()
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
    let raw_objects = stores.pdf_raw_objects();
    assert_eq!(raw_objects.len(), 2);
    assert_eq!(raw_objects[0].id().raw(), 1);
    assert!(raw_objects[0].is_referenced());
    assert_eq!(raw_objects[1].id().raw(), 2);
    assert!(raw_objects[1].is_immediate());
    let action = stores
        .pdf_catalog_open_action()
        .expect("fixture installs its catalog action");
    assert_eq!(action.id(), 3);
    assert_eq!(action.target_object(), Some(4));
    assert_eq!(stores.pdf_pages()[0].resources_object(), 5);
    assert_eq!(stores.pdf_pages()[0].contents_object(), 6);
    assert_eq!(stores.pdf_pages()[0].page_object(), 4);
    let first_artifacts = stores.world().committed_artifacts().to_vec();
    let first = umber::pdf_from_committed_artifacts(&mut stores, &first_artifacts)
        .expect("first PDF finalization");
    let document_ids = stores
        .finalize_pdf_document_objects(true)
        .expect("document identities remain idempotent");
    assert_eq!(document_ids.pages(), Some(7));
    assert_eq!(document_ids.names(), Some(8));
    assert_eq!(document_ids.catalog(), Some(9));
    assert_eq!(document_ids.info(), Some(10));
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
#[allow(clippy::disallowed_methods)] // Committed corpus fixture boundary.
fn navigation_fixture_replays_graph_bytes_and_state() {
    let source = fs::read_to_string(corpus_root().join("pdf/navigation_structures.tex"))
        .expect("read navigation parity source");
    let mut stores = Universe::default();
    umber::prepare_pdftex_run_stores(&mut stores);
    stores
        .begin_retained_session()
        .expect("retained navigation replay session starts");
    let checkpoint = stores.snapshot();

    umber::run_memory_with_stores(&source, &mut stores).expect("first navigation execution");
    let first_artifacts = stores.world().committed_artifacts().to_vec();
    let first = umber::pdf_from_committed_artifacts(&mut stores, &first_artifacts)
        .expect("first navigation PDF finalization");
    let first_hash = stores.snapshot().state_hash();
    let structure = normalize_structure(&first).expect("normalize navigation graph");
    for marker in ["names ", "outlines ", "threads ", "beads "] {
        assert!(structure.contains(marker), "missing {marker} projection");
    }

    stores.rollback(&checkpoint);
    umber::run_memory_with_stores(&source, &mut stores).expect("replayed navigation execution");
    let replayed_artifacts = stores.world().committed_artifacts().to_vec();
    let replayed = umber::pdf_from_committed_artifacts(&mut stores, &replayed_artifacts)
        .expect("replayed navigation PDF finalization");
    assert_eq!(replayed, first, "navigation rollback changed PDF bytes");
    assert_eq!(stores.snapshot().state_hash(), first_hash);
}

#[test]
#[allow(clippy::disallowed_methods)] // Committed corpus fixture boundary.
fn form_xobject_fixture_replays_bytes_artifacts_positions_and_state() {
    let source = fs::read_to_string(corpus_root().join("pdf/form_xobjects.tex"))
        .expect("read Form XObject parity source");
    let mut stores = Universe::default();
    umber::prepare_pdftex_run_stores(&mut stores);
    stores
        .begin_retained_session()
        .expect("retained form replay session starts");
    let checkpoint = stores.snapshot();

    umber::run_memory_with_stores(&source, &mut stores).expect("first form execution");
    assert_eq!(
        stores
            .pdf_forms()
            .map(|form| (form.object(), form.resource()))
            .collect::<Vec<_>>(),
        [(1, 1), (3, 2), (5, 3)]
    );
    let first_artifacts = [1, 3, 5].map(|object| {
        stores
            .pdf_form_artifact(object)
            .expect("referenced form was traversed")
            .clone()
    });
    assert_eq!(
        first_artifacts[0].last_position(),
        Some((tex_state::scaled::Scaled::from_raw(0), pt(2)))
    );
    assert_eq!(
        first_artifacts[1].last_position(),
        Some((tex_state::scaled::Scaled::from_raw(0), pt(6)))
    );
    assert_eq!(first_artifacts[1].snap_reference(), (pt(0), pt(10)));
    assert_eq!(first_artifacts[2].last_position(), Some((pt(1), pt(2))));
    assert_eq!(stores.pdf_snap_reference(), (pt(0), pt(5)));
    let first_pages = stores.world().committed_artifacts().to_vec();
    let first = umber::pdf_from_committed_artifacts(&mut stores, &first_pages)
        .expect("first form PDF finalization");
    let first_hash = stores.snapshot().state_hash();

    stores.rollback(&checkpoint);
    umber::run_memory_with_stores(&source, &mut stores).expect("replayed form execution");
    let replay_pages = stores.world().committed_artifacts().to_vec();
    let replayed = umber::pdf_from_committed_artifacts(&mut stores, &replay_pages)
        .expect("replayed form PDF finalization");
    assert_eq!(replayed, first, "form rollback replay changed PDF bytes");
    for (object, expected) in [1, 3, 5].into_iter().zip(first_artifacts) {
        let actual = stores
            .pdf_form_artifact(object)
            .expect("replayed form artifact exists");
        assert_eq!(actual.bytes(), expected.bytes());
        assert_eq!(actual.last_position(), expected.last_position());
        assert_eq!(actual.snap_reference(), expected.snap_reference());
    }
    assert_eq!(stores.snapshot().state_hash(), first_hash);
}

fn pt(value: i32) -> tex_state::scaled::Scaled {
    tex_state::scaled::Scaled::from_raw(value * 65_536)
}

#[test]
#[allow(clippy::disallowed_methods)] // Hermetic CLI fixture boundary.
fn committed_embedded_font_fixtures_match_bytes_structure_and_attestations() {
    for case in [
        "embedded_type1",
        "embedded_tagged_spacing",
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
    let distribution = write_empty_distribution(temp.path());
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
            | "embedded_tagged_spacing"
            | "embedded_subset_type1"
            | "embedded_subset_omit"
            | "embedded_subset_controls_negative"
    ) {
        fs::copy(
            corpus_root().join("pdf/embedded_type1.pfb"),
            temp.path().join("cmr10.pfb"),
        )
        .expect("stage committed Type1 program");
        if case == "embedded_tagged_spacing" {
            fs::copy(
                corpus_root().join("pdf/tagged_spacing.enc"),
                temp.path().join("tagged_spacing.enc"),
            )
            .expect("stage tagged-spacing encoding");
            // Umber's fallback is its pdf_writer-built Type-3 space font, but
            // the explicit reference map line still participates in resource
            // discovery. Any valid staged Type-1 program satisfies that
            // discovery without changing the generated fallback object.
            fs::copy(
                corpus_root().join("pdf/embedded_type1.pfb"),
                temp.path().join("pdftexspace.pfb"),
            )
            .expect("stage fallback map resource");
        }
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
        .args(["run", "--pdftex"])
        .arg("--distribution")
        .arg(&distribution)
        .arg("--pdf")
        .arg(&actual_path)
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .env("XDG_CACHE_HOME", temp.path().join("cache"))
        .env("TEXFONTS", temp.path())
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
        "embedded_tagged_spacing" => {
            assert!(actual_structure.contains("/Subtype /Type3"));
            assert!(actual_structure.contains("/Name /customspace"));
            assert!(actual_structure.contains("/Differences [32 /space]"));
            assert!(actual_structure.contains("content /UmberSpace 10 Tf"));
            assert!(actual_structure.contains("content <0b> Tj"));
            assert!(reference_structure.contains("PdfTeX-Space"));
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
    if case.starts_with("embedded_subset_") || case == "embedded_tagged_spacing" {
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

#[allow(clippy::disallowed_methods)] // Hermetic host-side distribution fixture.
fn write_empty_distribution(root: &std::path::Path) -> std::path::PathBuf {
    let distribution = root.join("distribution");
    let objects = distribution.join("objects");
    fs::create_dir_all(&objects).expect("create empty distribution");
    let shard = b"{\"schema\":1,\"distribution\":\"pdf-fixture\",\"index\":0,\"files\":{}}\n";
    let shard_digest = digest(shard);
    fs::write(objects.join(format!("sha256-{shard_digest}")), shard)
        .expect("write empty distribution shard");
    let root = format!(
        "{{\"schema\":2,\"distribution\":\"pdf-fixture\",\"objectsBaseUrl\":\"https://example.invalid/objects/\",\"shardBits\":0,\"shardCount\":1,\"shards\":[\"{shard_digest}\"]}}\n"
    );
    fs::write(distribution.join("manifest-v2.json"), root).expect("write empty distribution root");
    distribution
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
