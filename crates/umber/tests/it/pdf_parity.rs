use std::fs;
use std::process::Command;

use sha2::{Digest, Sha256};
use test_support::{
    closed_case::FixtureCase,
    corpus_root,
    pdf::normalize_structure,
    pdf_query::{PdfQuery, QueryLimits},
    read_binary_fixture, read_fixture,
};
use tex_state::{DetachedPdfCompletion, Universe};
use umber_distribution::{ManifestShard, pack_shard};

const PINNED_SOURCE_DATE_EPOCH: &str = "1783604160";
const PDF_PARITY_CASES: &[PdfParityCase] = &[
    PdfParityCase::new("annotations_running", &[]),
    PdfParityCase::new("external_pdf_page", &["minimal_rule.expected.ref.pdf"]),
    PdfParityCase::new("form_xobjects", &[]),
    PdfParityCase::new("minimal_rule", &[]),
    PdfParityCase::new("navigation_structures", &[]),
    PdfParityCase::new("object_dictionaries", &[]),
];

#[derive(Clone, Copy)]
struct PdfParityCase {
    name: &'static str,
    owned_inputs: &'static [&'static str],
}

impl PdfParityCase {
    const fn new(name: &'static str, owned_inputs: &'static [&'static str]) -> Self {
        Self { name, owned_inputs }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PdfParityChannel {
    ExactBytes,
    ReferenceStructure,
    UmberStructure,
    Raster,
    RenderAttestation,
}

#[derive(Debug, Eq, PartialEq)]
struct PdfParityCaseSummary {
    identity: &'static str,
    assertions: [PdfParityChannel; 5],
}

#[test]
#[allow(clippy::disallowed_methods)] // Hermetic CLI fixture boundary.
fn committed_pdftex_fixtures_match_structure_and_bytes() {
    let summary = run_committed_pdf_parity();
    assert_eq!(
        summary,
        [
            expected_pdf_parity_summary("annotations_running"),
            expected_pdf_parity_summary("external_pdf_page"),
            expected_pdf_parity_summary("form_xobjects"),
            expected_pdf_parity_summary("minimal_rule"),
            expected_pdf_parity_summary("navigation_structures"),
            expected_pdf_parity_summary("object_dictionaries"),
        ],
        "committed PDF parity consumer did not execute the exact case/channel matrix"
    );
    assert_eq!(
        summary
            .iter()
            .map(|case| case.assertions.len())
            .sum::<usize>(),
        30,
        "committed PDF parity consumer did not execute 30 channel assertions"
    );
}

fn run_committed_pdf_parity() -> Vec<PdfParityCaseSummary> {
    PDF_PARITY_CASES
        .iter()
        .copied()
        .map(assert_committed_case)
        .collect()
}

fn expected_pdf_parity_summary(identity: &'static str) -> PdfParityCaseSummary {
    PdfParityCaseSummary {
        identity,
        assertions: [
            PdfParityChannel::ExactBytes,
            PdfParityChannel::ReferenceStructure,
            PdfParityChannel::UmberStructure,
            PdfParityChannel::Raster,
            PdfParityChannel::RenderAttestation,
        ],
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

#[derive(Debug, PartialEq)]
struct AnnotationProjection {
    rectangle: Vec<f64>,
    subtype: Vec<u8>,
    action_subtype: Option<Vec<u8>>,
}

fn annotation_projection(bytes: &[u8]) -> Vec<Vec<AnnotationProjection>> {
    let document = PdfQuery::new(bytes, QueryLimits::default()).expect("parse annotation fixture");
    let mut owned = std::collections::BTreeSet::new();
    document
        .pages()
        .expect("ordered pages")
        .into_iter()
        .map(|page| {
            page.annotations
                .iter()
                .map(|entry| {
                    let id = entry.referenced_id().expect("indirect annotation");
                    assert!(owned.insert(id), "annotation object is shared by pages");
                    let annotation = entry.as_dictionary().expect("annotation dictionary");
                    if let Some(owner) = annotation.get(b"P") {
                        assert_eq!(
                            owner.referenced_id(),
                            Some(page.id),
                            "annotation /P does not reference its owning page"
                        );
                    }
                    assert_eq!(
                        annotation
                            .get(b"Type")
                            .and_then(|value| value.name())
                            .expect("annotation Type is a name")
                            .as_ref(),
                        b"Annot"
                    );
                    let rect = annotation
                        .get(b"Rect")
                        .and_then(|value| value.array())
                        .expect("annotation rectangle")
                        .iter()
                        .map(|number| {
                            number
                                .number()
                                .expect("annotation rectangle value is numeric")
                        })
                        .collect();
                    let subtype = annotation
                        .get(b"Subtype")
                        .and_then(|value| value.name())
                        .expect("annotation subtype is a name")
                        .as_ref()
                        .to_vec();
                    let action_subtype = annotation
                        .get(b"A")
                        .and_then(|value| value.as_dictionary())
                        .and_then(|action| action.get(b"S"))
                        .map(|value| {
                            value
                                .name()
                                .expect("annotation action subtype is a name")
                                .as_ref()
                                .to_vec()
                        });
                    AnnotationProjection {
                        rectangle: rect,
                        subtype,
                        action_subtype,
                    }
                })
                .collect()
        })
        .collect()
}

#[allow(clippy::disallowed_methods)] // Hermetic CLI fixture boundary.
fn assert_committed_case(declaration: PdfParityCase) -> PdfParityCaseSummary {
    let case = declaration.name;
    let closed = closed_pdf_case(case);
    let source = closed
        .payload_path("source.tex")
        .expect("resolve declared PDF source role");
    for input in declaration.owned_inputs {
        closed.payload_path(input).unwrap_or_else(|error| {
            panic!("resolve owned input role {input} for {case}: {error:#}")
        });
    }
    let expected_umber = closed
        .read("expected.umber.pdf")
        .expect("read declared Umber PDF role");
    let reference = closed
        .read("expected.ref.pdf")
        .expect("read declared reference PDF role");
    let expected_structure = closed
        .read_to_string("expected.structure")
        .expect("read declared structure role");
    let raster = closed
        .read("expected.pgm")
        .expect("read declared raster role");
    let render = closed
        .read_to_string("expected.render")
        .expect("read declared render-attestation role");

    let temp = tempfile::tempdir().expect("create PDF parity directory");
    let actual_path = temp.path().join(format!("{case}.pdf"));
    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .args(["run", "--pdftex", "--pdf"])
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .env("UMBER_ENGINE_FUEL", "10000000")
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
    assert_eq!(
        actual, expected_umber,
        "deterministic Umber PDF bytes changed"
    );

    assert_eq!(
        normalize_structure(&reference).expect("normalize reference PDF"),
        expected_structure
    );
    assert_eq!(
        normalize_structure(&actual).expect("normalize current Umber PDF"),
        expected_structure
    );

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
        render, expected_attestation,
        "committed renderer attestation is stale for pdf/{case}"
    );
    PdfParityCaseSummary {
        identity: case,
        assertions: [
            PdfParityChannel::ExactBytes,
            PdfParityChannel::ReferenceStructure,
            PdfParityChannel::UmberStructure,
            PdfParityChannel::Raster,
            PdfParityChannel::RenderAttestation,
        ],
    }
}

fn closed_pdf_case(case: &str) -> FixtureCase {
    FixtureCase::discover(format!("tests/corpus/pdf/{case}"), "source.tex", "pdf")
        .unwrap_or_else(|error| panic!("pdf/{case} is not a typed closed fixture case: {error:#}"))
}

fn detach_pdf_run<G>(stores: &mut Universe<G>, source: &str) -> DetachedPdfCompletion {
    umber::run_memory_with_stores_and_profile(
        source,
        stores,
        tex_command::CommandProfile::PDFTEX14029,
        false,
    )
    .expect("PDF execution");
    stores
        .command_context()
        .expect("admit terminal PDF completion")
        .detach_pdf_completion()
        .expect("detach terminal PDF completion")
}

fn finalize_detached_pdf(completion: &DetachedPdfCompletion) -> Vec<u8> {
    umber::pdf_from_accepted_artifacts_with_virtual_fonts(
        completion,
        &umber::PdfVirtualFontResources::default(),
        &umber::PdfRawObjectFileReceipt::default(),
    )
    .expect("detached PDF finalization")
}

#[test]
#[allow(clippy::disallowed_methods)] // Committed corpus fixture boundary.
fn object_dictionary_pdf_replays_to_identical_bytes_and_state() {
    let source = fs::read_to_string(corpus_root().join("pdf/object_dictionaries/source.tex"))
        .expect("read object dictionary parity source");
    umber::with_engine_universe(|stores| {
        umber::prepare_pdftex_run_stores(stores);
        stores
            .begin_retained_session()
            .expect("retained replay session starts");
        let checkpoint = stores.runtime_checkpoint().expect("PDF replay checkpoint");

        let first_completion = detach_pdf_run(stores, &source);
        let raw_objects = first_completion.raw_objects();
        assert_eq!(raw_objects.len(), 2);
        assert_eq!(raw_objects[0].object, 1);
        assert!(raw_objects[0].referenced);
        assert_eq!(raw_objects[1].object, 2);
        assert!(raw_objects[1].immediate);
        let action = first_completion
            .document()
            .open_action
            .as_ref()
            .expect("fixture installs its catalog action");
        assert_eq!(action.id, 3);
        assert_eq!(action.target_object, Some(4));
        let page = &first_completion.pages()[0];
        assert_eq!(
            (
                page.resources_object,
                page.contents_object,
                page.page_object
            ),
            (5, 6, 4)
        );
        let first = finalize_detached_pdf(&first_completion);

        stores
            .restore_runtime_checkpoint_with_roots(&checkpoint, || {})
            .expect("restore PDF replay checkpoint");
        let replayed_completion = detach_pdf_run(stores, &source);
        let replayed = finalize_detached_pdf(&replayed_completion);
        assert_eq!(replayed_completion, first_completion);
        assert_eq!(replayed, first, "rollback replay changed final PDF bytes");
    })
    .expect("fresh object-dictionary replay universe");
}

#[test]
#[allow(clippy::disallowed_methods)] // Committed corpus fixture boundary.
fn navigation_fixture_replays_graph_bytes_and_state() {
    let source = fs::read_to_string(corpus_root().join("pdf/navigation_structures/source.tex"))
        .expect("read navigation parity source");
    umber::with_engine_universe(|stores| {
        umber::prepare_pdftex_run_stores(stores);
        stores
            .begin_retained_session()
            .expect("retained navigation replay session starts");
        let checkpoint = stores.runtime_checkpoint().expect("navigation checkpoint");
        let first_completion = detach_pdf_run(stores, &source);
        let first = finalize_detached_pdf(&first_completion);
        let structure = normalize_structure(&first).expect("normalize navigation graph");
        for marker in ["names ", "outlines ", "threads ", "beads "] {
            assert!(structure.contains(marker), "missing {marker} projection");
        }

        stores
            .restore_runtime_checkpoint_with_roots(&checkpoint, || {})
            .expect("restore navigation checkpoint");
        let replayed_completion = detach_pdf_run(stores, &source);
        let replayed = finalize_detached_pdf(&replayed_completion);
        assert_eq!(replayed_completion, first_completion);
        assert_eq!(replayed, first, "navigation rollback changed PDF bytes");
    })
    .expect("fresh navigation replay universe");
}

#[test]
#[allow(clippy::disallowed_methods)] // Committed corpus fixture boundary.
fn form_xobject_fixture_replays_bytes_artifacts_positions_and_state() {
    let source = fs::read_to_string(corpus_root().join("pdf/form_xobjects/source.tex"))
        .expect("read Form XObject parity source");
    let source = format!(
        "\\font\\sym=cmsy10 \\font\\ext=cmex10 \
         \\textfont2=\\sym \\scriptfont2=\\sym \\scriptscriptfont2=\\sym \
         \\textfont3=\\ext \\scriptfont3=\\ext \\scriptscriptfont3=\\ext {source}"
    );
    umber::with_engine_universe(|stores| {
        umber::prepare_pdftex_run_stores(stores);
        stores
            .world_mut()
            .set_memory_file(
                "cmsy10.tfm",
                include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmsy10.tfm").to_vec(),
            )
            .expect("seed symbol font fixture");
        stores
            .world_mut()
            .set_memory_file(
                "cmex10.tfm",
                include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmex10.tfm").to_vec(),
            )
            .expect("seed extension font fixture");
        stores
            .begin_retained_session()
            .expect("retained form replay session starts");
        let checkpoint = stores.runtime_checkpoint().expect("form replay checkpoint");
        let first_completion = detach_pdf_run(stores, &source);
        assert_eq!(
            first_completion
                .forms()
                .iter()
                .map(|form| (form.object, form.resource))
                .collect::<Vec<_>>(),
            [(1, 1), (3, 2), (5, 3)]
        );
        let first = finalize_detached_pdf(&first_completion);

        stores
            .restore_runtime_checkpoint_with_roots(&checkpoint, || {})
            .expect("restore form replay checkpoint");
        let replayed_completion = detach_pdf_run(stores, &source);
        let replayed = finalize_detached_pdf(&replayed_completion);
        assert_eq!(replayed, first, "form rollback replay changed PDF bytes");
        assert_eq!(replayed_completion, first_completion);
    })
    .expect("fresh form replay universe");
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
    let (distribution, distribution_ahash64) = write_empty_distribution(temp.path());
    let source_name = format!("{case}.tex");
    fs::copy(
        corpus_root().join("pdf").join(case).join("source.tex"),
        temp.path().join(&source_name),
    )
    .expect("stage embedded-font source");
    fs::copy(
        corpus_root().join("pdf").join(case).join("cmr10.tfm"),
        temp.path().join("cmr10.tfm"),
    )
    .expect("stage cmr10 TFM");
    if case.starts_with("pk_bitmap_") {
        let dpi = case.trim_start_matches("pk_bitmap_");
        fs::copy(
            corpus_root()
                .join("pdf")
                .join(case)
                .join(format!("cmr10.{dpi}pk")),
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
            corpus_root().join("pdf").join(case).join("cmr10.pfb"),
            temp.path().join("cmr10.pfb"),
        )
        .expect("stage committed Type1 program");
        if case == "embedded_tagged_spacing" {
            fs::copy(
                corpus_root()
                    .join("pdf")
                    .join(case)
                    .join("tagged_spacing.enc"),
                temp.path().join("tagged_spacing.enc"),
            )
            .expect("stage tagged-spacing encoding");
            // Umber's fallback is its pdf_writer-built Type-3 space font, but
            // the explicit reference map line still participates in resource
            // discovery. Any valid staged Type-1 program satisfies that
            // discovery without changing the generated fallback object.
            fs::copy(
                corpus_root().join("pdf").join(case).join("pdftexspace.pfb"),
                temp.path().join("pdftexspace.pfb"),
            )
            .expect("stage fallback map resource");
        }
    } else {
        fs::copy(
            corpus_root().join("pdf").join(case).join("cmu-serif.ttf"),
            temp.path().join("cmu-serif.ttf"),
        )
        .expect("stage closed-case TrueType program");
        if case == "embedded_subset_truetype" {
            fs::copy(
                corpus_root().join("pdf").join(case).join("fixture.enc"),
                temp.path().join("fixture.enc"),
            )
            .expect("stage subset encoding");
        }
    }

    fs::write(temp.path().join("pdftex.map"), b"").expect("stage empty default PDF map");
    let actual_path = temp.path().join(format!("{case}.umber.pdf"));
    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .args(["run", "--pdftex"])
        .arg("--distribution")
        .arg(&distribution)
        .args(["--distribution-ahash64", &distribution_ahash64])
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
    assert_pdf_bytes_eq(case, &actual, &expected_umber);
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
    if case.starts_with("embedded_") {
        // pdftex.web §32e delegates scalable dictionaries to
        // writefont.c, which omits Type-3's resource-name entry.
        assert!(!actual_structure.contains("/Name /F1"));
        assert!(!reference_structure.contains("/Name /F1"));
    }
    match case {
        "embedded_type1" => {
            // pdftex.web §690: mapped scalable widths share the one-decimal
            // text-space raster used for character-advance accounting.
            assert!(
                actual_structure.contains("/Widths [625 833.3 777.8 694.4"),
                "Umber Type1 widths lost the pdfTeX one-decimal raster"
            );
            assert!(
                reference_structure.contains("/Widths [750 708.3 722.2]"),
                "pinned pdfTeX Type1 width witness changed"
            );
        }
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
            assert!(actual_structure.contains("/Name /F1"));
            assert!(actual_structure.contains("/FontMatrix [0.024 0 0 0.024 0 0]"));
            assert!(reference_structure.contains("/Subtype /Type3"));
            assert!(reference_structure.contains("/Name /F1"));
            assert!(reference_structure.contains("/FontMatrix [0.024 0 0 0.024 0 0]"));
        }
        "pk_bitmap_600" => {
            assert!(actual_structure.contains("/Subtype /Type3"));
            assert!(actual_structure.contains("/Name /F1"));
            assert!(actual_structure.contains("/FontMatrix [0.012 0 0 0.012 0 0]"));
            assert!(reference_structure.contains("/Subtype /Type3"));
            assert!(reference_structure.contains("/Name /F1"));
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

fn assert_pdf_bytes_eq(case: &str, actual: &[u8], expected: &[u8]) {
    if actual == expected {
        return;
    }
    let first_difference = actual
        .iter()
        .zip(expected)
        .position(|(actual, expected)| actual != expected)
        .unwrap_or_else(|| actual.len().min(expected.len()));
    panic!(
        "deterministic {case} bytes changed at offset {first_difference}: actual {} bytes sha256 {}, expected {} bytes sha256 {}",
        actual.len(),
        digest(actual),
        expected.len(),
        digest(expected),
    );
}

#[allow(clippy::disallowed_methods)] // Hermetic host-side distribution fixture.
fn write_empty_distribution(root: &std::path::Path) -> (std::path::PathBuf, String) {
    let distribution = root.join("distribution");
    let objects = distribution.join("objects");
    fs::create_dir_all(&objects).expect("create empty distribution");
    let shard = pack_shard(
        &ManifestShard::parse(
            "{\"schema\":3,\"distribution\":\"pdf-fixture\",\"index\":0,\"files\":{}}\n",
        )
        .expect("typed empty shard"),
    )
    .expect("packed empty shard");
    let shard_digest = distribution_digest(&shard);
    fs::write(objects.join(format!("ahash64-v1-{shard_digest}")), shard)
        .expect("write empty distribution shard");
    let root = format!(
        "{{\"schema\":8,\"distribution\":\"pdf-fixture\",\"objectsBaseUrl\":\"https://example.invalid/objects/\",\"shardBits\":0,\"shardCount\":1,\"shards\":[\"{shard_digest}\"]}}\n"
    );
    let root_digest = distribution_digest(root.as_bytes());
    fs::write(distribution.join("manifest-v8.json"), root).expect("write empty distribution root");
    (distribution, root_digest)
}

fn distribution_digest(bytes: &[u8]) -> String {
    umber_hash::AHash64::for_bytes(umber_hash::HashDomain::DistributionContent, bytes).hex()
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
