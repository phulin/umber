use super::*;

fn fixture() -> PdfFixture {
    PdfFixture::new("1.7").expect("valid fixture version")
}

#[test]
fn classic_xref_offsets_are_deterministic_and_include_gaps() {
    let mut first = fixture();
    first
        .add_dictionary(1, Dictionary::new().entry("Type", name("Catalog")))
        .expect("fixture construction");
    first
        .add_raw_object(3, b"[1 0 R 3 0 R]")
        .expect("fixture construction");
    first
        .set_trailer_entry("Root", reference(1))
        .expect("fixture construction");
    first
        .set_trailer_entry("Binary", b"(\xff)")
        .expect("fixture construction");
    let first = first.finish().expect("fixture construction");

    let mut second = fixture();
    second
        .add_dictionary(1, Dictionary::new().entry("Type", name("Catalog")))
        .expect("fixture construction");
    second
        .add_raw_object(3, b"[1 0 R 3 0 R]")
        .expect("fixture construction");
    second
        .set_trailer_entry("Root", reference(1))
        .expect("fixture construction");
    second
        .set_trailer_entry("Binary", b"(\xff)")
        .expect("fixture construction");
    let second = second.finish().expect("fixture construction");
    assert_eq!(first, second);

    let text = String::from_utf8_lossy(&first);
    assert!(text.contains("xref\n0 4\n0000000002 65535 f \n"));
    assert!(text.contains("0000000000 00000 f \n"));
    let startxref = text
        .split("startxref\n")
        .nth(1)
        .expect("fixture construction")
        .lines()
        .next()
        .expect("fixture construction")
        .parse::<usize>()
        .expect("fixture construction");
    assert_eq!(&first[startxref..startxref + 5], b"xref\n");
}

#[test]
fn stream_lengths_are_derived_for_raw_and_preencoded_bytes() {
    let mut pdf = fixture();
    pdf.add_stream(1, Dictionary::new(), b"abc\n")
        .expect("fixture construction");
    pdf.add_filtered_stream(
        2,
        Dictionary::new().entry("N", b"3"),
        "FlateDecode",
        [0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01],
    )
    .expect("fixture construction");
    let bytes = pdf.finish().expect("fixture construction");
    assert!(
        bytes
            .windows(b"/Length 4\n".len())
            .any(|part| part == b"/Length 4\n")
    );
    assert!(
        bytes
            .windows(b"/Length 8\n".len())
            .any(|part| part == b"/Length 8\n")
    );
    assert!(
        bytes
            .windows(b"/Filter /FlateDecode\n".len())
            .any(|part| { part == b"/Filter /FlateDecode\n" })
    );

    let error = fixture()
        .add_stream(1, Dictionary::new().entry("Length", b"99"), b"x")
        .expect_err("fixture rejection");
    assert_eq!(error, FixtureError::ReservedStreamKey("Length"));
}

#[test]
fn page_group_and_inherited_resources_fit_the_raw_dictionary_boundary() {
    let mut pdf = PdfFixture::new("1.5").expect("fixture construction");
    pdf.add_dictionary(
        1,
        Dictionary::new()
            .entry("Type", name("Catalog"))
            .entry("Pages", reference(2)),
    )
    .expect("fixture construction");
    pdf.add_dictionary(
        2,
        Dictionary::new()
            .entry("Type", name("Pages"))
            .entry("Kids", array([reference(3)]))
            .entry("Count", b"1")
            .entry("Resources", b"<< /ProcSet [/PDF] >>"),
    )
    .expect("fixture construction");
    pdf.add_dictionary(
        3,
        Dictionary::new()
            .entry("Type", name("Page"))
            .entry("Parent", reference(2))
            .entry("MediaBox", b"[0 0 10 20]")
            .entry("Group", b"<< /S /Transparency /CS /DeviceRGB >>")
            .entry("Contents", reference(4)),
    )
    .expect("fixture construction");
    pdf.add_stream(4, Dictionary::new(), b"0 0 10 20 re f")
        .expect("fixture construction");
    pdf.set_trailer_entry("Root", reference(1))
        .expect("fixture construction");
    let bytes = pdf.finish().expect("fixture construction");
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("/Resources << /ProcSet [/PDF] >>"));
    assert!(text.contains("/Group << /S /Transparency /CS /DeviceRGB >>"));
}

#[test]
fn icc_based_dct_fixture_keeps_encoded_payload_exact() {
    let jpeg = [
        0xff, 0xd8, 0xff, 0xe0, b'U', b'm', b'b', b'e', b'r', 0xff, 0xd9,
    ];
    let mut pdf = PdfFixture::new("1.6").expect("fixture construction");
    pdf.add_stream(
        1,
        Dictionary::new()
            .entry("N", b"3")
            .entry("Alternate", name("DeviceRGB")),
        [b'I'; 32],
    )
    .expect("fixture construction");
    pdf.add_filtered_stream(
        2,
        Dictionary::new()
            .entry("Type", name("XObject"))
            .entry("Subtype", name("Image"))
            .entry("Width", b"1")
            .entry("Height", b"1")
            .entry("BitsPerComponent", b"8")
            .entry("ColorSpace", array([name("ICCBased"), reference(1)])),
        "DCTDecode",
        jpeg,
    )
    .expect("fixture construction");
    let bytes = pdf.finish().expect("fixture construction");
    assert!(bytes.windows(jpeg.len()).any(|part| part == jpeg));
    assert!(
        bytes
            .windows(b"/ColorSpace [/ICCBased 1 0 R]".len())
            .any(|part| { part == b"/ColorSpace [/ICCBased 1 0 R]" })
    );
}

#[test]
fn cycles_deep_values_and_malformed_raw_objects_remain_explicit() {
    let mut pdf = fixture();
    pdf.add_dictionary(1, Dictionary::new().entry("Self", reference(1)))
        .expect("fixture construction");
    pdf.add_dictionary(2, Dictionary::new().entry("Next", reference(3)))
        .expect("fixture construction");
    pdf.add_dictionary(3, Dictionary::new().entry("Next", reference(2)))
        .expect("fixture construction");
    pdf.add_dictionary(
        4,
        Dictionary::new().entry("Deep", nested_array(130, b"null")),
    )
    .expect("fixture construction");
    pdf.add_raw_object(5, b"<< /IntentionallyUnclosed [")
        .expect("fixture construction");
    let bytes = pdf.finish().expect("fixture construction");
    assert!(
        bytes
            .windows(b"/Self 1 0 R".len())
            .any(|part| part == b"/Self 1 0 R")
    );
    let deep = nested_array(130, b"null");
    assert!(bytes.windows(deep.len()).any(|part| part == deep));
    assert!(
        bytes
            .windows(b"/IntentionallyUnclosed [".len())
            .any(|part| { part == b"/IntentionallyUnclosed [" })
    );
}

#[test]
fn rejects_duplicate_objects_and_writer_owned_trailer_size() {
    let mut pdf = fixture();
    pdf.add_raw_object(1, b"null")
        .expect("fixture construction");
    assert_eq!(
        pdf.add_raw_object(1, b"false")
            .expect_err("fixture rejection"),
        FixtureError::DuplicateObject(1)
    );
    assert_eq!(
        pdf.set_trailer_entry("Size", b"99")
            .expect_err("fixture rejection"),
        FixtureError::ReservedTrailerKey("Size")
    );
    let bytes = pdf.finish().expect("original object remains present");
    assert!(
        bytes
            .windows(b"1 0 obj\nnull".len())
            .any(|part| { part == b"1 0 obj\nnull" })
    );
}
