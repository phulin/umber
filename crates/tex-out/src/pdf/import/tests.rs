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
fn imported_number_range_and_precision_are_rejected() {
    assert!(number_value(b"9223372036854775808").is_err());
    assert!(number_value(b"-9223372036854775809").is_err());
    assert!(number_value(b"0.1234567890").is_err());
    assert!(number_value(b"1e-3").is_err());
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
