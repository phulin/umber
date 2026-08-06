use super::*;
use crate::pdf_fixture::{Dictionary, PdfFixture, RawPdfFixture, nested_array};

fn classic_fixture() -> Vec<u8> {
    let mut fixture = RawPdfFixture::new("1.7").expect("valid version");
    fixture
        .add_raw_object(
            1,
            b"<< /Type /Catalog /Pages 2 0 R /OpenAction [3 0 R /Fit] /Cycle 9 0 R >>",
        )
        .expect("catalog");
    fixture
        .add_raw_object(
            2,
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] /Rotate 90 /Resources << /Font << /F1 6 0 R >> >> >>",
        )
        .expect("pages");
    fixture
        .add_raw_object(
            3,
            b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Annots [5 0 R] >>",
        )
        .expect("page");
    fixture
        .add_stream(4, Dictionary::new(), b"q 1 0 0 1 2 3 cm (Hi) Tj Q\n")
        .expect("content stream");
    fixture
        .add_raw_object(
            5,
            b"<< /Type /Annot /Subtype /Text /Rect [1 2 3 4] /P 3 0 R /A << /S /URI /URI (https://example.test/) >> /Dest [3 0 R /Fit] >>",
        )
        .expect("annotation");
    fixture
        .add_raw_object(6, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
        .expect("font");
    fixture
        .add_raw_object(7, b"<< /Title (Probe) /Subject (Semantic PDF) >>")
        .expect("info");
    fixture
        .add_raw_object(9, b"<< /Kind /Cycle /Next 9 0 R >>")
        .expect("cycle");
    fixture
        .add_raw_object(10, nested_array(12, b"null"))
        .expect("deep object");
    fixture.add_raw_object(11, b"42").expect("scalar object");
    fixture
        .add_filtered_stream(12, Dictionary::new(), "FlateDecode", b"not deflate")
        .expect("malformed stream");
    fixture
        .add_raw_object(13, b"<< /Missing 99 0 R >>")
        .expect("unresolved reference");
    fixture.set_trailer_entry("Root", b"1 0 R").expect("root");
    fixture.set_trailer_entry("Info", b"7 0 R").expect("info");
    fixture
        .set_trailer_entry("ID", b"[<0102> <0304>]")
        .expect("ID");
    fixture
        .set_trailer_entry("Custom", b"/TrailerValue")
        .expect("custom trailer");
    fixture.finish().expect("fixture")
}

fn probe() -> PdfProbe {
    PdfProbe::new(classic_fixture(), ProbeLimits::default()).expect("parse fixture")
}

#[test]
fn classic_xref_exposes_version_root_trailer_and_ordered_pages() {
    let probe = probe();
    assert_eq!(probe.version(), (1, 7));
    assert_eq!(probe.root_id(), ProbeObjectId::new(1, 0));
    let trailer = probe.trailer().expect("project trailer").expect("trailer");
    assert_eq!(
        trailer.get(b"Root").and_then(|v| v.referenced_id()),
        Some(probe.root_id())
    );
    assert_eq!(
        trailer.get(b"Info").and_then(|v| v.referenced_id()),
        Some(ProbeObjectId::new(7, 0))
    );
    assert_eq!(
        trailer
            .get(b"Custom")
            .and_then(|value| value.name())
            .as_deref(),
        Some(b"TrailerValue".as_slice())
    );
    assert!(trailer.get(b"ID").and_then(|value| value.array()).is_some());

    let pages = probe.pages().expect("project pages");
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].number, 1);
    assert_eq!(pages[0].id, ProbeObjectId::new(3, 0));
    assert_eq!(pages[0].media_box, [0.0, 0.0, 200.0, 300.0]);
    assert_eq!(pages[0].rotation_degrees, 90);
    assert_eq!(pages[0].resources.categories[b"Font".as_slice()].len(), 1);
}

#[test]
fn content_annotations_actions_and_destinations_retain_semantics_and_identity() {
    let pages = probe();
    let page = pages.pages().expect("project pages").remove(0);
    let content = page.content.as_ref().expect("page content");
    assert_eq!(content.decoded.len(), 27);
    assert_eq!(
        content.decoded_sha256,
        <[u8; 32]>::from(Sha256::digest(&content.decoded))
    );
    assert_eq!(
        content
            .operations
            .iter()
            .map(|operation| operation.operator.as_slice())
            .collect::<Vec<_>>(),
        [b"q".as_slice(), b"cm", b"Tj", b"Q"]
    );
    assert_eq!(page.annotations.len(), 1);
    assert_eq!(
        page.annotations[0].referenced_id(),
        Some(ProbeObjectId::new(5, 0))
    );
    let annotation = page.annotations[0]
        .as_dictionary()
        .expect("annotation dictionary");
    assert_eq!(
        annotation
            .get(b"Subtype")
            .and_then(|value| value.name())
            .as_deref(),
        Some(b"Text".as_slice())
    );
    let destination = annotation
        .get(b"Dest")
        .and_then(|value| value.array())
        .expect("destination array");
    assert_eq!(
        destination
            .iter()
            .next()
            .and_then(|value| value.referenced_id()),
        Some(page.id)
    );
}

#[test]
fn streams_expose_raw_decoded_bytes_digest_dictionary_and_operations() {
    let probe = probe();
    let stream = probe
        .stream(ProbeObjectId::new(4, 0))
        .expect("content stream");
    assert_eq!(stream.id, ProbeObjectId::new(4, 0));
    assert_eq!(stream.raw, stream.decoded);
    assert_eq!(
        stream.decoded_sha256,
        <[u8; 32]>::from(Sha256::digest(&stream.decoded))
    );
    assert_eq!(
        stream
            .operations(ProbeLimits::default())
            .expect("operations")[2]
            .operator,
        b"Tj"
    );
    assert!(stream.dictionary.get(b"Length").is_some());
}

#[test]
fn xref_stream_trailer_and_object_stream_objects_are_supported() {
    let bytes = include_bytes!("fixtures/xref-object-stream.pdf").to_vec();
    let probe = PdfProbe::new(bytes, ProbeLimits::default()).expect("parse xref stream fixture");
    assert_eq!(probe.version(), (1, 5));
    assert_eq!(
        probe
            .trailer()
            .expect("trailer query")
            .expect("trailer")
            .get(b"Type")
            .and_then(|value| value.name())
            .as_deref(),
        Some(b"XRef".as_slice())
    );
    assert_eq!(
        probe.pages().expect("compressed pages")[0].id,
        ProbeObjectId::new(3, 0)
    );
    assert_eq!(
        probe
            .dictionary(ProbeObjectId::new(2, 0))
            .expect("compressed dictionary")
            .id(),
        Some(ProbeObjectId::new(2, 0))
    );
}

#[test]
fn cycles_are_shallow_and_all_budgets_fail_closed() {
    let probe = probe();
    let cycle = probe
        .dictionary(ProbeObjectId::new(9, 0))
        .expect("cycle dictionary");
    assert_eq!(
        cycle.get(b"Next").and_then(|value| value.referenced_id()),
        Some(ProbeObjectId::new(9, 0))
    );
    let missing = probe
        .dictionary(ProbeObjectId::new(13, 0))
        .expect("dictionary with unresolved reference")
        .get(b"Missing")
        .expect("unresolved edge");
    assert_eq!(missing.referenced_id(), Some(ProbeObjectId::new(99, 0)));
    assert!(missing.is_unresolved());

    let bytes = classic_fixture();
    for (limits, id, expected) in [
        (
            ProbeLimits {
                max_depth: 3,
                ..ProbeLimits::default()
            },
            ProbeObjectId::new(10, 0),
            "depth budget",
        ),
        (
            ProbeLimits {
                max_objects: 1,
                ..ProbeLimits::default()
            },
            ProbeObjectId::new(1, 0),
            "object budget",
        ),
        (
            ProbeLimits {
                max_values: 1,
                ..ProbeLimits::default()
            },
            ProbeObjectId::new(1, 0),
            "value budget",
        ),
        (
            ProbeLimits {
                max_stream_bytes: 8,
                ..ProbeLimits::default()
            },
            ProbeObjectId::new(4, 0),
            "stream budget",
        ),
    ] {
        let error = PdfProbe::new(bytes.clone(), limits)
            .expect("parse bounded fixture")
            .validate_object(id)
            .expect_err("budget must reject query");
        assert!(error.to_string().contains(expected), "{error:#}");
    }
}

#[test]
fn direct_and_indirect_stream_cycles_obey_object_budgets() {
    let mut fixture = PdfFixture::new("1.7").expect("valid version");
    fixture
        .add_raw_object(1, b"<< /Type /Catalog /Pages 2 0 R >>")
        .expect("catalog");
    fixture
        .add_raw_object(2, b"<< /Type /Pages /Kids [] /Count 0 >>")
        .expect("page tree");
    fixture
        .add_stream(3, Dictionary::new().entry("Loop", b"3 0 R"), [])
        .expect("direct stream cycle");
    fixture
        .add_stream(4, Dictionary::new().entry("Next", b"5 0 R"), [])
        .expect("first indirect stream cycle");
    fixture
        .add_stream(5, Dictionary::new().entry("Next", b"4 0 R"), [])
        .expect("second indirect stream cycle");
    fixture.set_trailer_entry("Root", b"1 0 R").expect("root");
    let bytes = fixture.finish().expect("fixture");

    let one_object = ProbeLimits {
        max_objects: 1,
        ..ProbeLimits::default()
    };
    PdfProbe::new(bytes.clone(), one_object)
        .expect("parse fixture")
        .validate_object(ProbeObjectId::new(3, 0))
        .expect("active direct back-reference consumes no second object");
    let error = PdfProbe::new(bytes.clone(), one_object)
        .expect("parse fixture")
        .validate_object(ProbeObjectId::new(4, 0))
        .expect_err("indirect cycle resolves a second object");
    assert!(error.to_string().contains("object budget"), "{error:#}");

    PdfProbe::new(
        bytes,
        ProbeLimits {
            max_objects: 2,
            ..ProbeLimits::default()
        },
    )
    .expect("parse fixture")
    .validate_object(ProbeObjectId::new(4, 0))
    .expect("active indirect back-reference terminates within two objects");
}

#[test]
fn malformed_files_fail_and_lenient_stream_decoding_remains_observable() {
    let error = PdfProbe::new(b"not a PDF", ProbeLimits::default())
        .err()
        .expect("malformed input must fail");
    assert!(error.to_string().contains("failed to parse PDF"));
    let probe = probe();
    let stream = probe
        .stream(ProbeObjectId::new(12, 0))
        .expect("lenient malformed stream");
    assert_eq!(stream.raw, b"not deflate");
    assert!(stream.decoded.is_empty());
    assert_eq!(stream.decoded_sha256, <[u8; 32]>::from(Sha256::digest([])));
}
