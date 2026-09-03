use test_support::pdf_fixture::{Dictionary, ValidPdfFixture, array, name, reference};
use tex_out::pdf::PdfNumber;

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

#[test]
fn named_destination_selects_page_from_legacy_dictionary_and_name_tree() {
    let number = |coefficient, decimal_places| {
        PdfNumber::new(coefficient, decimal_places).expect("valid PDF number")
    };
    for name_tree in [false, true] {
        let inspected = inspect_pdf_page(
            named_destination_pdf(name_tree).into(),
            &tex_exec::PdfImagePageSelection::Named(b"chapter".to_vec()),
            PdfImagePageBox::Media,
        )
        .expect("resolve named destination");
        assert_eq!(inspected.page_number, 2);
        assert_eq!(
            inspected.page_box,
            [number(0, 0), number(0, 0), number(30, 0), number(40, 0)]
        );
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
                .entry("MediaBox", b"[12.345678901 -4.5 42.125 88.75]"),
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
    assert_eq!(
        inspected.page_box,
        [
            PdfNumber::new(12_345_678_901, 9).expect("left"),
            PdfNumber::new(-45, 1).expect("bottom"),
            PdfNumber::new(42_125, 3).expect("right"),
            PdfNumber::new(8875, 2).expect("top"),
        ]
    );
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
