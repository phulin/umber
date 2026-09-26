use super::*;

fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], bytes: &[u8]) {
    png.extend((bytes.len() as u32).to_be_bytes());
    png.extend(kind);
    png.extend(bytes);
    let mut crc = u32::MAX;
    for byte in kind.iter().chain(bytes) {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    png.extend((!crc).to_be_bytes());
}

fn indexed_png(bits: u8, row: &[u8], transparency: Option<&[u8]>) -> Vec<u8> {
    let width = if bits == 1 { 9u32 } else { 3u32 };
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = width.to_be_bytes().to_vec();
    header.extend(1u32.to_be_bytes());
    header.extend([bits, 3, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &header);
    chunk(&mut png, b"PLTE", &[10, 20, 30, 40, 50, 60]);
    if let Some(transparency) = transparency {
        chunk(&mut png, b"tRNS", transparency);
    }
    let mut filtered = vec![0];
    filtered.extend(row);
    chunk(
        &mut png,
        b"IDAT",
        &zlib(&filtered).expect("compress fixture"),
    );
    chunk(&mut png, b"IEND", &[]);
    png
}

fn lower(png: &[u8], width: u32, bits: u8) -> RasterStreams {
    raster_image_streams(
        png,
        RasterMetadata {
            format: PdfRasterFormatInput::Png,
            width,
            height: 1,
            bits_per_component: bits,
            color_space: PdfRasterColorSpaceInput::Rgb,
            alpha: false,
            png_color_type: Some(3),
        },
        PdfImageGammaInput {
            gamma: 1000,
            image_gamma: 1000,
            high_color: true,
            apply_gamma: false,
        },
        (1, 7),
        &mut ImageImportTelemetry::default(),
    )
    .expect("valid indexed PNG")
}

#[test]
fn opaque_indexed_png_keeps_palette_and_packed_samples() {
    for (bits, width, row) in [(1, 9, &[0b0101_0101, 0][..]), (8, 3, &[0, 1, 0][..])] {
        let result = lower(&indexed_png(bits, row, None), width, bits);
        assert_eq!(result.2, bits);
        assert_eq!(
            result.3,
            PdfImageColorSpace::IndexedRgb(vec![10, 20, 30, 40, 50, 60])
        );
        assert_eq!(inflate(&result.0).expect("valid image stream"), row);
        assert!(result.4.is_none());
    }
}

#[test]
fn transparent_indexed_png_expands_color_and_alpha() {
    let result = lower(&indexed_png(8, &[0, 1, 0], Some(&[0, 128])), 3, 8);
    assert_eq!(result.3, PdfImageColorSpace::DeviceRgb);
    assert_eq!(result.2, 8);
    assert_eq!(
        inflate(&result.0).expect("valid color stream"),
        [10, 20, 30, 40, 50, 60, 10, 20, 30]
    );
    assert_eq!(
        inflate(&result.4.expect("alpha mask").0).expect("valid alpha stream"),
        [0, 128, 0]
    );
}

#[test]
fn gamma_corrects_the_palette_without_changing_indices() {
    let png = indexed_png(8, &[0, 1, 0], None);
    let result = raster_image_streams(
        &png,
        RasterMetadata {
            format: PdfRasterFormatInput::Png,
            width: 3,
            height: 1,
            bits_per_component: 8,
            color_space: PdfRasterColorSpaceInput::Rgb,
            alpha: false,
            png_color_type: Some(3),
        },
        PdfImageGammaInput {
            gamma: 2000,
            image_gamma: 1000,
            high_color: true,
            apply_gamma: true,
        },
        (1, 7),
        &mut ImageImportTelemetry::default(),
    )
    .expect("valid indexed PNG");
    assert_eq!(inflate(&result.0).expect("valid index stream"), [0, 1, 0]);
    assert_eq!(
        result.3,
        PdfImageColorSpace::IndexedRgb(vec![50, 71, 87, 101, 113, 124])
    );
}
