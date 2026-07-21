use std::path::PathBuf;

use test_support::pdf_fixture::{Dictionary, PdfFixture, array, name, nested_array, reference};

use super::*;

#[test]
fn deeply_nested_resource_values_are_rejected() {
    let mut document = PdfFixture::new("1.7").expect("create nested PDF fixture");
    let resources = Dictionary::new().entry(
        "Properties",
        Dictionary::new()
            .entry("Deep", nested_array(MAX_IMPORTED_DEPTH + 1, b"null"))
            .to_bytes(),
    );
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
                .entry("Kids", array([reference(3)]))
                .entry("Count", b"1"),
        )
        .expect("page tree");
    document
        .add_dictionary(
            3,
            Dictionary::new()
                .entry("Type", name("Page"))
                .entry("Parent", reference(2))
                .entry("MediaBox", b"[0 0 10 20]")
                .entry("Resources", resources.to_bytes()),
        )
        .expect("page");
    document
        .set_trailer_entry("Root", reference(1))
        .expect("root");
    let bytes = document.finish().expect("serialize nested PDF");

    let error = import_pdf_page(bytes.into(), 1, &mut 100)
        .err()
        .expect("depth limit");
    assert!(error.contains("nesting exceeds limit"), "{error}");
}

#[test]
#[allow(clippy::disallowed_methods)] // Conditional external-fixture boundary.
fn pinned_arxiv_dct_resources_import_as_encoded_streams_when_available() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("third_party/arxiv-sample-100/sources");
    let cases = [
        "1910.12506/CLAS.pdf",
        "1910.12506/MMPX.pdf",
        "1910.12506/ReacPlane.pdf",
        "1901.02462/Figure1.pdf",
        "1901.02462/Figure2.pdf",
    ];
    let mut tested = 0;
    for relative in cases {
        let path = corpus.join(relative);
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        tested += 1;
        let inspected = inspect_pdf_page(bytes.clone().into(), 1, PdfImagePageBox::Media)
            .unwrap_or_else(|error| panic!("inspect {}: {error}", path.display()));
        assert_eq!(inspected.total_pages, 1, "{}", path.display());
        let mut next_object = 100;
        let imported = import_pdf_page(bytes.into(), 1, &mut next_object)
            .unwrap_or_else(|error| panic!("import {}: {error}", path.display()));
        assert!(
            imported.dependencies.iter().any(|indirect| {
                let PdfObject::EncodedStream { dictionary, data } = &indirect.object else {
                    return false;
                };
                !data.is_empty()
                    && matches!(
                        dictionary.get(b"Filter"),
                        Some(PdfValue::Name(name)) if name.as_bytes() == b"DCTDecode"
                    )
            }),
            "{} has no preserved DCT resource",
            path.display()
        );
    }
    if corpus.exists() {
        assert_eq!(tested, cases.len(), "pinned arXiv corpus is incomplete");
    }
}
