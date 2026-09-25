use super::*;
use test_support::pdf_fixture::{Dictionary, ValidPdfFixture, name, reference};

fn number(coefficient: i64, decimal_places: u8) -> PdfNumber {
    PdfNumber::new(coefficient, decimal_places).expect("valid fixed number")
}

#[test]
fn imported_numbers_keep_short_decimal_digits_and_integer_values() {
    assert_eq!(
        number_value(b"-891.018"),
        Ok(PdfValue::Number(number(-891018, 3)))
    );
    assert_eq!(number_value(b".125"), Ok(PdfValue::Number(number(125, 3))));
    assert_eq!(
        number_value(b"9223372036854775807"),
        Ok(PdfValue::Number(number(i64::MAX, 0)))
    );
    assert_eq!(
        number_value(b"-9223372036854775808"),
        Ok(PdfValue::Number(number(i64::MIN, 0)))
    );
}

#[test]
fn imported_real_rounding_matches_pdftex_epsilon_boundaries() {
    let cases: &[(&[u8], PdfNumber)] = &[
        (b"0.0000004", number(0, 0)),
        (b"0.0000005", number(1, 6)),
        (b"0.0000006", number(1, 6)),
        (b"-0.0000004", number(0, 0)),
        (b"-0.0000005", number(-1, 6)),
        (b"-0.0000006", number(-1, 6)),
        (b"1.2345674", number(1_234_567, 6)),
        (b"1.2345675", number(1_234_568, 6)),
        (b"-1.2345675", number(-1_234_568, 6)),
        (b"9.9999994", number(9_999_999, 6)),
        (b"9.9999995", number(10, 0)),
        (b"-9.9999995", number(-10, 0)),
        (b"0001.2000000", number(12, 1)),
    ];
    for &(source, expected) in cases {
        assert_eq!(
            number_value(source),
            Ok(PdfValue::Number(expected)),
            "imported spelling {source:?}"
        );
    }
}

#[test]
fn imported_number_range_and_invalid_syntax_are_rejected() {
    assert!(number_value(b"9223372036854775808").is_err());
    assert!(number_value(b"-9223372036854775809").is_err());
    assert_eq!(
        number_value(b"0.1234567890"),
        Ok(PdfValue::Number(number(123457, 6)))
    );
    assert!(number_value(b"1e-3").is_err());
}

#[test]
fn imported_numbers_reject_nonfinite_and_trailing_tokens() {
    for source in [b"NaN".as_slice(), b"Inf", b"-Inf", b"1e-3", b"1.2junk"] {
        assert!(number_value(source).is_err(), "invalid spelling {source:?}");
    }
}

#[test]
fn imported_dictionary_and_array_numbers_use_the_same_real_rule() {
    let entries = raw_dictionary_entries(
        b"<< /Matrix [0.123456789 -0.25] /Name /Example /Nested << /Scale 1.5 >> >>",
    )
    .expect("valid dictionary");
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].name, b"Matrix");
    let array = Array::from_bytes(entries[0].value).expect("valid array");
    let values = raw_array_values(array.data()).expect("valid array values");
    assert_eq!(
        values
            .into_iter()
            .map(number_value)
            .collect::<Result<Vec<_>, _>>()
            .expect("valid numbers"),
        vec![
            PdfValue::Number(number(123457, 6)),
            PdfValue::Number(number(-25, 2)),
        ]
    );
}

fn imported_ext_g_state_pdf() -> Vec<u8> {
    let mut document = ValidPdfFixture::new("1.7").expect("create ExtGState PDF");
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
                .entry("Kids", b"[3 0 R]"),
        )
        .expect("pages");
    document
        .add_dictionary(
            3,
            Dictionary::new()
                .entry("Type", name("Page"))
                .entry("Parent", reference(2))
                .entry("MediaBox", b"[0 0 1 1]")
                .entry(
                    "Resources",
                    b"<< /ExtGState << /GS1 << /CA .2509804 /CA2 .14901962 /ca .7019608 /Other .6 >> >> >>",
                )
                .entry("Contents", reference(4)),
        )
        .expect("page");
    document
        .add_stream(4, Dictionary::new(), b"q Q")
        .expect("content stream");
    document
        .set_trailer_entry("Root", reference(1))
        .expect("trailer");
    document.finish().expect("serialize ExtGState PDF")
}

fn import_fixture(bytes: Vec<u8>) -> ImportedPdfPage {
    let mut next_object = 100;
    import_pdf_page(
        bytes.into(),
        1,
        &mut next_object,
        super::super::PdfFinalizationLimits::default(),
    )
    .expect("import PDF fixture")
}

#[test]
fn indirect_lookup_uses_real_object_eleven_after_object_one_hundred_eleven() {
    let mut document = ValidPdfFixture::new("1.7").expect("create PDF");
    document
        .add_raw_object(111, b"<< /Type /StructElem /K 999 0 R >>")
        .expect("unused object 111");
    document
        .add_dictionary(
            11,
            Dictionary::new()
                .entry("Type", name("Font"))
                .entry("Subtype", name("Type1"))
                .entry("BaseFont", name("Helvetica")),
        )
        .expect("font object 11");
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
                .entry("Kids", b"[3 0 R]"),
        )
        .expect("pages");
    document
        .add_dictionary(
            3,
            Dictionary::new()
                .entry("Type", name("Page"))
                .entry("Parent", reference(2))
                .entry("MediaBox", b"[0 0 1 1]")
                .entry("Resources", b"<< /Font << /F1 11 0 R >> >>")
                .entry("Contents", reference(4)),
        )
        .expect("page");
    document
        .add_stream(4, Dictionary::new(), b"q Q")
        .expect("contents");
    document
        .set_trailer_entry("Root", reference(1))
        .expect("trailer");
    let bytes = document.finish().expect("serialize PDF");
    let first = bytes
        .windows(b"111 0 obj".len())
        .position(|s| s == b"111 0 obj");
    let second = bytes
        .windows(b"11 0 obj".len())
        .position(|s| s == b"11 0 obj");
    assert_eq!(
        first.map(|offset| offset + 1),
        second,
        "the old byte scan would see 11 inside 111"
    );

    let imported = import_fixture(bytes);
    assert_eq!(imported.dependencies.len(), 1);
    let PdfObject::Value(PdfValue::Dictionary(font)) = &imported.dependencies[0].object else {
        panic!("font dictionary");
    };
    assert_eq!(
        font.get(b"BaseFont"),
        Some(&PdfValue::Name(PdfName::new(b"Helvetica")))
    );
}

fn pdf_with_compressed_stem_v() -> Vec<u8> {
    let mut pdf = b"%PDF-1.5\n".to_vec();
    let mut offsets = [0usize; 27];
    let objects: &[(usize, &[u8])] = &[
        (1, b"<< /Type /Catalog /Pages 2 0 R >>"),
        (2, b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>"),
        (3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1 1] /Resources << /Font << /F1 12 0 R >> >> /Contents 4 0 R >>"),
        (4, b"<< /Length 3 >>\nstream\nq Q\nendstream"),
        (5, b"<< /Type /FontDescriptor /FontName /Helvetica /StemV 8 0 R /MissingWidth 9 0 R /ItalicAngle 10 0 R >>"),
        (12, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /FontDescriptor 5 0 R >>"),
        (25, b"<< /Type /ObjStm /N 3 /First 14 /Length 45 >>\nstream\n8 0 9 4 10 21 813 9007199254740993 .772891500\nendstream"),
    ];
    for &(id, body) in objects {
        offsets[id] = pdf.len();
        pdf.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
        pdf.extend_from_slice(body);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    offsets[26] = pdf.len();
    let mut xref = Vec::with_capacity(27 * 7);
    for (id, &offset) in offsets.iter().enumerate() {
        let (kind, location, generation) = if (8..=10).contains(&id) {
            (2u8, 25u32, u16::try_from(id - 8).expect("small fixture"))
        } else if offset != 0 {
            (1u8, u32::try_from(offset).expect("small fixture"), 0u16)
        } else if id == 0 {
            (0u8, 0u32, 65535u16)
        } else {
            (0u8, 0u32, 0u16)
        };
        xref.push(kind);
        xref.extend_from_slice(&location.to_be_bytes());
        xref.extend_from_slice(&generation.to_be_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "26 0 obj\n<< /Type /XRef /Size 27 /W [1 4 2] /Root 1 0 R /Length {} >>\nstream\n",
            xref.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(&xref);
    pdf.extend_from_slice(
        format!("\nendstream\nendobj\nstartxref\n{}\n%%EOF\n", offsets[26]).as_bytes(),
    );
    pdf
}

#[test]
fn compressed_indirect_number_is_imported_from_resolved_object_stream() {
    let imported = import_fixture(pdf_with_compressed_stem_v());
    assert_eq!(imported.dependencies.len(), 5);
    assert!(
        imported
            .dependencies
            .iter()
            .any(|entry| { entry.object == PdfObject::Value(PdfValue::Number(number(813, 0))) })
    );
    assert!(imported.dependencies.iter().any(|entry| {
        entry.object == PdfObject::Value(PdfValue::Number(number(9_007_199_254_740_993, 0)))
    }));
    assert!(
        imported
            .dependencies
            .iter()
            .any(|entry| { entry.object == PdfObject::Value(PdfValue::Number(number(772892, 6))) })
    );
}

#[test]
fn incremental_number_redefinition_uses_active_xref_spelling() {
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
                .entry("Kids", b"[3 0 R]"),
        )
        .expect("pages");
    document
        .add_dictionary(
            3,
            Dictionary::new()
                .entry("Type", name("Page"))
                .entry("Parent", reference(2))
                .entry("MediaBox", b"[0 0 1 1]")
                .entry("Resources", b"<< /Font << /F1 12 0 R >> >>")
                .entry("Contents", reference(4)),
        )
        .expect("page");
    document
        .add_stream(4, Dictionary::new(), b"q Q")
        .expect("contents");
    document
        .add_dictionary(5, Dictionary::new().entry("StemV", reference(8)))
        .expect("descriptor");
    document
        .add_raw_object(8, b".772891499")
        .expect("old number spelling");
    document
        .add_dictionary(
            12,
            Dictionary::new()
                .entry("Type", name("Font"))
                .entry("Subtype", name("Type1"))
                .entry("BaseFont", name("Helvetica"))
                .entry("FontDescriptor", reference(5)),
        )
        .expect("font");
    document
        .set_trailer_entry("Root", reference(1))
        .expect("trailer");
    let mut pdf = document.finish().expect("serialize PDF");
    let startxref = pdf
        .windows(b"startxref\n".len())
        .rposition(|window| window == b"startxref\n")
        .expect("first xref")
        + b"startxref\n".len();
    let previous = std::str::from_utf8(&pdf[startxref..])
        .expect("ASCII xref")
        .lines()
        .next()
        .expect("xref offset")
        .parse::<usize>()
        .expect("decimal xref offset");
    let replacement = pdf.len();
    pdf.extend_from_slice(b"\n8 0 obj\n.772891500\nendobj\n");
    let xref = pdf.len();
    pdf.extend_from_slice(format!("xref\n8 1\n{replacement:010} 00000 n \r\ntrailer\n<< /Size 13 /Root 1 0 R /Prev {previous} >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());

    let imported = import_fixture(pdf);
    assert!(
        imported
            .dependencies
            .iter()
            .any(|entry| { entry.object == PdfObject::Value(PdfValue::Number(number(772892, 6))) })
    );
}

#[test]
fn imported_ext_g_state_values_are_quantized_at_admission() {
    let mut next_object = 100;
    let imported = import_pdf_page(
        imported_ext_g_state_pdf().into(),
        1,
        &mut next_object,
        super::super::PdfFinalizationLimits::default(),
    )
    .expect("import ExtGState page");
    let Some(PdfValue::Dictionary(ext_g_states)) = imported.resources.get(b"ExtGState") else {
        panic!("imported ExtGState resource dictionary");
    };
    let Some(PdfValue::Dictionary(state)) = ext_g_states.get(b"GS1") else {
        panic!("imported GS1 dictionary");
    };
    let cases = [
        (b"CA".as_slice(), number(25098, 5)),
        (b"CA2".as_slice(), number(14902, 5)),
        (b"ca".as_slice(), number(701961, 6)),
        (b"Other".as_slice(), number(6, 1)),
    ];
    for (key, expected) in cases {
        assert_eq!(
            state.get(key),
            Some(&PdfValue::Number(expected)),
            "ExtGState key {key:?}"
        );
    }
}

#[test]
fn fixed_formatter_handles_signs_and_i64_minimum() {
    let mut buffer = [0_u8; 32];
    assert_eq!(
        super::super::fixed_number_bytes(number(-891018, 3), &mut buffer),
        b"-891.018"
    );
    assert_eq!(
        super::super::fixed_number_bytes(number(-5, 2), &mut buffer),
        b"-0.05"
    );
    assert_eq!(
        super::super::fixed_number_bytes(number(1200, 3), &mut buffer),
        b"1.2"
    );
    assert_eq!(
        super::super::fixed_number_bytes(number(i64::MIN, 0), &mut buffer),
        b"-9223372036854775808"
    );
}

#[test]
fn imported_page_with_empty_resource_categories_remains_valid() {
    let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/pdf/external_pdf_page/minimal_rule.expected.ref.pdf"
    ));
    let mut next_object = 100;
    let imported = import_pdf_page(
        bytes.into(),
        1,
        &mut next_object,
        super::super::PdfFinalizationLimits::default(),
    )
    .expect("minimal imported page");
    assert_eq!(imported.resources.len(), 1);
    assert!(imported.resources.get(b"ProcSet").is_some());
}
