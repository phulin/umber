use crate::pdf_fixture::{Dictionary, RawPdfFixture, ValidPdfFixture};

use super::normalize_structure;

#[test]
fn normalization_merges_inherited_resources_and_marks_cycles_stably() {
    let mut fixture = RawPdfFixture::new("1.7").expect("valid PDF version");
    fixture
        .add_raw_object(1, b"<< /Type /Catalog /Pages 2 0 R >>")
        .expect("catalog");
    fixture
        .add_raw_object(
            2,
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 20 30] /Resources << /Font << /ParentFont 6 0 R >> /ProcSet [/PDF] >> >>",
        )
        .expect("page tree");
    fixture
        .add_raw_object(
            3,
            b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /PageFont 7 0 R >> >> /Contents 4 0 R >>",
        )
        .expect("page");
    fixture
        .add_stream(4, Dictionary::new(), b"q Q\n")
        .expect("page content");
    fixture
        .add_raw_object(5, b"<< /Kind /Cycle /Next 5 0 R >>")
        .expect("cyclic user object");
    fixture
        .add_raw_object(6, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
        .expect("inherited font");
    fixture
        .add_raw_object(7, b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>")
        .expect("page font");
    fixture.set_trailer_entry("Root", b"1 0 R").expect("root");
    let bytes = fixture.finish().expect("PDF fixture");

    let first = normalize_structure(&bytes).expect("normalize PDF");
    let second = normalize_structure(&bytes).expect("normalize PDF again");
    assert_eq!(first, second, "cycle notation must be stable");
    assert!(first.contains("/ParentFont <</BaseFont /Helvetica"));
    assert!(first.contains("/PageFont <</BaseFont /Courier"));
    assert!(first.contains("/ProcSet [/PDF]"));
    assert!(
        first.contains("object <</Kind /Cycle /Next @0>>"),
        "{first}"
    );
}

#[test]
fn normalization_marks_direct_and_indirect_stream_cycles_stably() {
    let mut fixture = ValidPdfFixture::new("1.7").expect("valid PDF version");
    fixture
        .add_raw_object(
            1,
            b"<< /Type /Catalog /Pages 2 0 R /Names << /Direct 5 0 R /Indirect 6 0 R >> >>",
        )
        .expect("catalog");
    fixture
        .add_raw_object(2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
        .expect("page tree");
    fixture
        .add_raw_object(
            3,
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 20 30] /Resources << /ProcSet [/PDF] >> /Contents 4 0 R >>",
        )
        .expect("page");
    fixture
        .add_stream(4, Dictionary::new(), b"q Q\n")
        .expect("page content");
    fixture
        .add_stream(
            5,
            Dictionary::new()
                .entry("Subtype", b"/Data")
                .entry("Loop", b"5 0 R"),
            [],
        )
        .expect("directly cyclic stream");
    fixture
        .add_stream(
            6,
            Dictionary::new()
                .entry("Subtype", b"/Data")
                .entry("Next", b"7 0 R"),
            [],
        )
        .expect("first indirectly cyclic stream");
    fixture
        .add_stream(
            7,
            Dictionary::new()
                .entry("Subtype", b"/Data")
                .entry("Next", b"6 0 R"),
            [],
        )
        .expect("second indirectly cyclic stream");
    fixture.set_trailer_entry("Root", b"1 0 R").expect("root");
    let bytes = fixture.finish().expect("PDF fixture");

    let first = normalize_structure(&bytes).expect("normalize PDF");
    let second = normalize_structure(&bytes).expect("normalize PDF again");
    assert_eq!(first, second, "stream cycle notation must be stable");
    assert!(
        first.contains("/Direct stream <</Length 0 /Loop @0 /Subtype /Data>>"),
        "{first}"
    );
    assert!(
        first.contains(
            "/Indirect stream <</Length 0 /Next stream <</Length 0 /Next @0 /Subtype /Data>>"
        ),
        "{first}"
    );
}
