use test_support::pdf_fixture::{Dictionary, ValidPdfFixture, array, name, reference};

use super::*;

fn named_destination_pdf(name_tree: bool) -> Vec<u8> {
    let mut document = ValidPdfFixture::new("1.7").expect("create named-destination PDF");
    let catalog = if name_tree {
        document
            .add_dictionary(
                5,
                Dictionary::new().entry("Names", b"[(chapter) [4 0 R /Fit]]"),
            )
            .expect("destination name tree");
        document
            .add_dictionary(6, Dictionary::new().entry("Dests", reference(5)))
            .expect("names dictionary");
        Dictionary::new()
            .entry("Type", name("Catalog"))
            .entry("Pages", reference(2))
            .entry("Names", reference(6))
    } else {
        Dictionary::new()
            .entry("Type", name("Catalog"))
            .entry("Pages", reference(2))
            .entry(
                "Dests",
                Dictionary::new()
                    .entry("chapter", b"[4 0 R /Fit]")
                    .to_bytes(),
            )
    };
    document.add_dictionary(1, catalog).expect("catalog");
    document
        .add_dictionary(
            2,
            Dictionary::new()
                .entry("Type", name("Pages"))
                .entry("Kids", array([reference(3), reference(4)]))
                .entry("Count", b"2"),
        )
        .expect("page tree");
    for (object, media_box) in [(3, b"[0 0 10 20]" as &[u8]), (4, b"[0 0 30 40]")] {
        document
            .add_dictionary(
                object,
                Dictionary::new()
                    .entry("Type", name("Page"))
                    .entry("Parent", reference(2))
                    .entry("MediaBox", media_box),
            )
            .expect("page");
    }
    document
        .set_trailer_entry("Root", reference(1))
        .expect("root");
    document.finish().expect("serialize named-destination PDF")
}

fn single_page_pdf(media_box: &[u8]) -> Vec<u8> {
    let mut document = ValidPdfFixture::new("1.7").expect("create PDF");
    document
        .add_dictionary(
            1,
            Dictionary::new()
                .entry("Type", name("Catalog"))
                .entry("Pages", reference(2)),
        )
        .expect("catalog");
    document
        .add_dictionary(
            2,
            Dictionary::new()
                .entry("Type", name("Pages"))
                .entry("Count", b"1")
                .entry("Kids", array([reference(3)])),
        )
        .expect("pages");
    document
        .add_dictionary(
            3,
            Dictionary::new()
                .entry("Type", name("Page"))
                .entry("Parent", reference(2))
                .entry("MediaBox", media_box),
        )
        .expect("page");
    document
        .set_trailer_entry("Root", reference(1))
        .expect("root");
    document.finish().expect("serialize PDF")
}

#[test]
fn named_destination_selects_page_from_legacy_dictionary_and_name_tree() {
    for name_tree in [false, true] {
        let inspected = inspect_pdf_page(
            named_destination_pdf(name_tree).into(),
            &tex_exec::PdfImagePageSelection::Named(b"chapter".to_vec()),
            PdfImagePageBox::Media,
        )
        .expect("resolve named destination");
        assert_eq!(inspected.page_number, 2);
        assert_eq!(inspected.page_box, [0.0, 0.0, 30.0, 40.0]);
    }
}

#[test]
fn page_box_inspection_preserves_decimal_source_numbers() {
    let mut document = ValidPdfFixture::new("1.7").expect("create PDF");
    document
        .add_dictionary(
            1,
            Dictionary::new()
                .entry("Type", name("Catalog"))
                .entry("Pages", reference(2)),
        )
        .expect("catalog");
    document
        .add_dictionary(
            2,
            Dictionary::new()
                .entry("Type", name("Pages"))
                .entry("Count", b"1")
                .entry("Kids", array([reference(3)])),
        )
        .expect("pages");
    document
        .add_dictionary(
            3,
            Dictionary::new()
                .entry("Type", name("Page"))
                .entry("Parent", reference(2))
                .entry("MediaBox", b"[855.9531003736 391.763125 0 -0]"),
        )
        .expect("page");
    document
        .set_trailer_entry("Root", reference(1))
        .expect("trailer");
    let inspected = inspect_pdf_page(
        document.finish().expect("serialize PDF").into(),
        &tex_exec::PdfImagePageSelection::Number(1),
        PdfImagePageBox::Media,
    )
    .expect("inspect decimal page box");
    let [left, bottom, right, top] = inspected.page_box;
    assert_eq!(left, 0.0);
    assert_eq!(bottom, 0.0);
    assert!((right - 855.9531003736).abs() < 1e-12);
    assert!((top - 391.763125).abs() < 1e-12);
}

#[test]
fn page_box_clamps_each_coordinate_before_ordering() {
    let inspected = inspect_pdf_page(
        single_page_pdf(b"[-1200000000.25 10.0000000001 -2.5 -0]").into(),
        &tex_exec::PdfImagePageSelection::Number(1),
        PdfImagePageBox::Media,
    )
    .expect("inspect clamped page box");
    let [left, bottom, right, top] = inspected.page_box;
    assert_eq!(left, -1e9);
    assert_eq!(right, -2.5);
    assert_eq!(bottom, -0.0);
    assert!((top - 10.0000000001).abs() < 1e-12);
}

#[test]
fn page_box_scaled_conversion_rounds_ties_away_from_zero() {
    let scale = 6_578_176.0 / 100.0;
    let tie = 0.5 / scale;
    assert_eq!(pdf_bp_to_scaled(tie).expect("positive tie").raw(), 1);
    assert_eq!(pdf_bp_to_scaled(-tie).expect("negative tie").raw(), -1);
}

#[test]
fn clamped_huge_page_box_fails_checked_scaled_conversion() {
    let mut huge = b"[".to_vec();
    huge.extend(std::iter::repeat_n(b'1', 400));
    huge.extend_from_slice(b" 0 1 1]");
    let inspected = inspect_pdf_page(
        single_page_pdf(&huge).into(),
        &tex_exec::PdfImagePageSelection::Number(1),
        PdfImagePageBox::Media,
    )
    .expect("valid huge page box syntax");
    assert_eq!(inspected.page_box[2], 1e9);
    assert!(pdf_bp_to_scaled(inspected.page_box[2]).is_err());
}

#[test]
fn malformed_page_box_numbers_are_rejected_at_admission() {
    for media_box in [b"[0 0 1e-3 1]".as_slice(), b"[0 0 NaN 1]", b"[0 0 1junk 1]"] {
        let error = inspect_pdf_page(
            single_page_pdf(media_box).into(),
            &tex_exec::PdfImagePageSelection::Number(1),
            PdfImagePageBox::Media,
        )
        .expect_err("malformed page box must fail");
        assert!(
            !error.is_empty(),
            "malformed page box error must be reported"
        );
    }
}

#[test]
fn missing_named_destination_is_not_treated_as_page_zero() {
    let error = inspect_pdf_page(
        named_destination_pdf(true).into(),
        &tex_exec::PdfImagePageSelection::Named(b"missing".to_vec()),
        PdfImagePageBox::Media,
    )
    .expect_err("missing destination must fail");
    assert_eq!(error, "PDF inclusion: invalid destination <missing>");
}
