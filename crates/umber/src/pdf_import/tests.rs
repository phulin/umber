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
fn missing_named_destination_is_not_treated_as_page_zero() {
    let error = inspect_pdf_page(
        named_destination_pdf(true).into(),
        &tex_exec::PdfImagePageSelection::Named(b"missing".to_vec()),
        PdfImagePageBox::Media,
    )
    .expect_err("missing destination must fail");
    assert_eq!(error, "PDF inclusion: invalid destination <missing>");
}
