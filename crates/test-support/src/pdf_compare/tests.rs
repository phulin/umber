use crate::pdf_fixture::{Dictionary, ValidPdfFixture};

use super::{first_difference, project_pdf};

fn fixture(content: &[u8], crop_box: &[u8], rotation: i32, font: &str) -> Vec<u8> {
    let mut pdf = ValidPdfFixture::new("1.7").expect("valid PDF fixture");
    pdf.add_raw_object(1, b"<< /Type /Catalog /Pages 2 0 R >>")
        .expect("valid PDF fixture");
    pdf.add_raw_object(2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
        .expect("valid PDF fixture");
    let page = format!(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /CropBox {} /Rotate {rotation} /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
        String::from_utf8_lossy(crop_box)
    );
    pdf.add_raw_object(3, page.as_bytes())
        .expect("valid PDF fixture");
    pdf.add_stream(4, Dictionary::new(), content)
        .expect("valid PDF fixture");
    pdf.add_raw_object(
        5,
        format!("<< /Type /Font /Subtype /Type1 /BaseFont /{font} >>").as_bytes(),
    )
    .expect("valid PDF fixture");
    pdf.set_trailer_entry("Root", b"1 0 R")
        .expect("valid PDF fixture");
    pdf.finish().expect("valid PDF fixture")
}

fn projection(content: &[u8], crop: &[u8], rotation: i32, font: &str) -> String {
    project_pdf(&fixture(content, crop, rotation, font))
        .expect("valid PDF fixture")
        .text
}

#[test]
fn rejects_text_position_graphics_font_and_page_geometry_changes() {
    let base = b"BT /F1 10 Tf 10 20 Td (Hello) Tj ET q 1 0 0 rg 0 0 8 8 re f Q";
    let original = projection(base, b"[0 0 200 300]", 0, "Helvetica");
    for (label, content, crop, rotation, font) in [
        (
            "text",
            b"BT /F1 10 Tf 10 20 Td (Jello) Tj ET q 1 0 0 rg 0 0 8 8 re f Q".as_slice(),
            b"[0 0 200 300]".as_slice(),
            0,
            "Helvetica",
        ),
        (
            "position",
            b"BT /F1 10 Tf 11 20 Td (Hello) Tj ET q 1 0 0 rg 0 0 8 8 re f Q".as_slice(),
            b"[0 0 200 300]".as_slice(),
            0,
            "Helvetica",
        ),
        (
            "graphics",
            b"BT /F1 10 Tf 10 20 Td (Hello) Tj ET q 0 1 0 rg 0 0 8 8 re f Q".as_slice(),
            b"[0 0 200 300]".as_slice(),
            0,
            "Helvetica",
        ),
        ("font", base, b"[0 0 200 300]", 0, "Courier"),
        ("crop", base, b"[0 0 190 300]", 0, "Helvetica"),
        ("rotation", base, b"[0 0 200 300]", 90, "Helvetica"),
    ] {
        let changed = projection(content, crop, rotation, font);
        assert!(first_difference(&original, &changed).is_some(), "{label}");
    }
}

#[test]
fn decoded_stream_digest_attests_lexical_differences_separately() {
    let original =
        project_pdf(&fixture(b"q Q", b"[0 0 200 300]", 0, "Helvetica")).expect("valid PDF fixture");
    let changed = project_pdf(&fixture(
        b"q Q % a trailing comment",
        b"[0 0 200 300]",
        0,
        "Helvetica",
    ))
    .expect("valid PDF fixture");
    assert_eq!(original.text, changed.text);
    assert_ne!(
        original.decoded_content_sha256,
        changed.decoded_content_sha256
    );
}

#[test]
fn rejects_changed_image_resource_samples() {
    fn image_pdf(sample: u8) -> Vec<u8> {
        let mut pdf = ValidPdfFixture::new("1.7").expect("valid PDF fixture");
        pdf.add_raw_object(1, b"<< /Type /Catalog /Pages 2 0 R >>")
            .expect("valid PDF fixture");
        pdf.add_raw_object(2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
            .expect("valid PDF fixture");
        pdf.add_raw_object(
            3,
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Resources << /XObject << /Im1 5 0 R >> >> /Contents 4 0 R >>",
        )
        .expect("valid PDF fixture");
        pdf.add_stream(4, Dictionary::new(), b"q /Im1 Do Q")
            .expect("valid PDF fixture");
        pdf.add_stream(
            5,
            Dictionary::new()
                .entry("Type", b"/XObject")
                .entry("Subtype", b"/Image")
                .entry("Width", b"1")
                .entry("Height", b"1")
                .entry("ColorSpace", b"/DeviceGray")
                .entry("BitsPerComponent", b"8"),
            [sample],
        )
        .expect("valid PDF fixture");
        pdf.set_trailer_entry("Root", b"1 0 R")
            .expect("valid PDF fixture");
        pdf.finish().expect("valid PDF fixture")
    }

    let white = project_pdf(&image_pdf(255)).expect("valid PDF fixture");
    let black = project_pdf(&image_pdf(0)).expect("valid PDF fixture");
    assert!(first_difference(&white.text, &black.text).is_some());
}

#[test]
fn rejects_missing_file_framing_and_truncation() {
    let valid = fixture(b"q Q", b"[0 0 200 300]", 0, "Helvetica");
    assert!(project_pdf(&valid).is_ok());
    assert!(project_pdf(b"arbitrary bytes").is_err());
    assert!(project_pdf(&valid[..valid.len() - 6]).is_err());
}
