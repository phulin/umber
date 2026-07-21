use std::path::PathBuf;

use lopdf::dictionary;

use super::*;

#[test]
fn deeply_nested_resource_values_are_rejected() {
    let mut document = lopdf::Document::with_version("1.7");
    let pages = document.new_object_id();
    let page = document.new_object_id();
    let mut nested = lopdf::Object::Null;
    for _ in 0..=MAX_IMPORTED_DEPTH {
        nested = lopdf::Object::Array(vec![nested]);
    }
    document.objects.insert(
        page,
        lopdf::dictionary! {
            "Type" => "Page",
            "Parent" => pages,
            "MediaBox" => vec![0.into(), 0.into(), 10.into(), 20.into()],
            "Resources" => lopdf::dictionary! {
                "Properties" => lopdf::dictionary! { "Deep" => nested },
            },
        }
        .into(),
    );
    document.objects.insert(
        pages,
        lopdf::dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page.into()],
            "Count" => 1,
        }
        .into(),
    );
    let catalog = document.add_object(lopdf::dictionary! {
        "Type" => "Catalog",
        "Pages" => pages,
    });
    document.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).expect("serialize nested PDF");

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
