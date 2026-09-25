use super::{dimensions, resolution};
use crate::pdf_import::PdfImageSourceKey;
use tex_command::{PdfImagePageBox, PdfImagePageSelection};
use tex_state::World;

fn segment(marker: u8, data: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0xff, marker];
    bytes.extend_from_slice(
        &u16::try_from(data.len() + 2)
            .expect("small segment")
            .to_be_bytes(),
    );
    bytes.extend_from_slice(data);
    bytes
}

fn jfif(unit: u8, x: u16, y: u16) -> Vec<u8> {
    let mut data = b"JFIF\0\x01\x01".to_vec();
    data.push(unit);
    data.extend_from_slice(&x.to_be_bytes());
    data.extend_from_slice(&y.to_be_bytes());
    data.extend_from_slice(&[0, 0]);
    let mut bytes = vec![0xff, 0xd8];
    bytes.extend(segment(0xe0, &data));
    bytes
}

#[test]
fn jfif_units_and_missing_axis_follow_pdftex() {
    for (unit, x, y, expected) in [
        (1, 300, 600, (300, 600)),
        (1, 0, 300, (300, 300)),
        (1, 300, 0, (300, 300)),
        (1, 0, 0, (0, 0)),
        (2, 118, 237, (299, 601)),
        (0, 300, 600, (0, 0)),
    ] {
        assert_eq!(resolution(&jfif(unit, x, y)), Some(expected));
    }
}

#[test]
fn jpeg_natural_size_uses_intrinsic_density_before_live_fallback() {
    let mut bytes = jfif(1, 300, 300);
    bytes.extend(segment(
        0xc0,
        &[8, 1, 44, 2, 88, 3, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0],
    ));
    assert_eq!(dimensions(&bytes), Ok((600, 300, 8, 3)));
    let mut world = World::default();
    world
        .set_memory_file("density.jpg", bytes)
        .expect("seed JPEG");
    let content = world.read_file("density.jpg").expect("JPEG bytes");
    let source = super::super::parse_image(
        &content,
        &PdfImageSourceKey {
            name: "density.jpg".to_owned(),
            page: PdfImagePageSelection::Number(1),
            page_box: PdfImagePageBox::Media,
            resolution: 72,
        },
    )
    .expect("JPEG metadata");
    // 600/300 inches * 72.27 TeX points/inch, rounded to scaled points.
    assert_eq!(source.natural_width.raw(), 9_472_573);
    assert_eq!(source.natural_height.raw(), 4_736_287);
}

fn exif(big: bool) -> Vec<u8> {
    let short = |v: u16| {
        if big {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        }
    };
    let long = |v: u32| {
        if big {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        }
    };
    let mut tiff = if big { b"MM".to_vec() } else { b"II".to_vec() };
    tiff.extend(short(42));
    tiff.extend(long(8));
    tiff.extend(short(3));
    for (tag, offset) in [(282, 50), (283, 58)] {
        tiff.extend(short(tag));
        tiff.extend(short(5));
        tiff.extend(long(1));
        tiff.extend(long(offset));
    }
    tiff.extend(short(296));
    tiff.extend(short(3));
    tiff.extend(long(1));
    tiff.extend(short(3));
    tiff.extend(short(0));
    tiff.extend(long(0));
    for value in [237, 2, 475, 2] {
        tiff.extend(long(value));
    }
    let mut data = b"Exif\0\0".to_vec();
    data.extend(tiff);
    let mut bytes = vec![0xff, 0xd8];
    bytes.extend(segment(0xe1, &data));
    bytes
}

#[test]
fn exif_endianness_and_integer_rationals_follow_pdftex() {
    for big in [false, true] {
        // writejpg.c truncates 237/2 and 475/2 before centimetre conversion.
        assert_eq!(resolution(&exif(big)), Some((299, 601)));
    }
}

#[test]
fn only_the_first_marker_supplies_density_and_truncated_headers_are_safe() {
    let header = jfif(1, 300, 300);
    let mut bytes = vec![0xff, 0xd8];
    bytes.extend(segment(0xfe, b"comment"));
    bytes.extend_from_slice(&header[2..]);
    assert_eq!(resolution(&bytes), None);
    for bytes in [header, exif(false), exif(true)] {
        for end in 0..bytes.len() {
            assert_eq!(resolution(&bytes[..end]), None);
        }
    }
    assert!(dimensions(&[0xff, 0xd8, 0xff, 0xc0, 0, 7, 8, 0, 1, 0, 1]).is_err());
}
