use super::*;

// PNG specification: seven Adam7 passes, each with its own packed scanlines.
// Construct the transport independently of the production decoder.
fn adam7_png(width: u32, height: u32, bits: u8, color: u8, pixels: &[u8]) -> Vec<u8> {
    let components = match color {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => unreachable!(),
    };
    let pixel_bits = components * usize::from(bits);
    let row_bytes = (width as usize * pixel_bits).div_ceil(8);
    assert_eq!(pixels.len(), row_bytes * height as usize);
    let mut filtered = Vec::new();
    for (x, y, dx, dy) in [
        (0, 0, 8, 8),
        (4, 0, 8, 8),
        (0, 4, 4, 8),
        (2, 0, 4, 4),
        (0, 2, 2, 4),
        (1, 0, 2, 2),
        (0, 1, 1, 2),
    ] {
        if x >= width as usize || y >= height as usize {
            continue;
        }
        for row in (y..height as usize).step_by(dy) {
            filtered.push(0); // independent, unfiltered pass row
            let mut pass = Vec::new();
            let mut bit_count = 0;
            for column in (x..width as usize).step_by(dx) {
                for bit in 0..pixel_bits {
                    let source_bit = column * pixel_bits + bit;
                    let value =
                        (pixels[row * row_bytes + source_bit / 8] >> (7 - source_bit % 8)) & 1;
                    if bit_count % 8 == 0 {
                        pass.push(0);
                    }
                    *pass.last_mut().expect("pass byte was just allocated") |=
                        value << (7 - bit_count % 8);
                    bit_count += 1;
                }
            }
            filtered.extend(pass);
        }
    }
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = width.to_be_bytes().to_vec();
    header.extend(height.to_be_bytes());
    header.extend([bits, color, 0, 0, 1]);
    chunk(&mut png, b"IHDR", &header);
    if color == 3 {
        chunk(&mut png, b"PLTE", &[10, 20, 30, 40, 50, 60]);
        chunk(&mut png, b"tRNS", &[0, 123]);
    }
    chunk(
        &mut png,
        b"IDAT",
        &zlib(&filtered).expect("compress Adam7 fixture"),
    );
    chunk(&mut png, b"IEND", &[]);
    png
}

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

fn metadata(width: u32, height: u32, bits: u8, color: u8) -> RasterMetadata {
    RasterMetadata {
        format: PdfRasterFormatInput::Png,
        width,
        height,
        bits_per_component: bits,
        color_space: if matches!(color, 0 | 4) {
            PdfRasterColorSpaceInput::Gray
        } else {
            PdfRasterColorSpaceInput::Rgb
        },
        alpha: matches!(color, 4 | 6),
        png_color_type: Some(color),
    }
}

fn lower(png: &[u8], metadata: RasterMetadata) -> Result<RasterStreams, PdfBuildError> {
    raster_image_streams(
        png,
        metadata,
        PdfImageGammaInput {
            gamma: 1000,
            image_gamma: 1000,
            high_color: true,
            apply_gamma: false,
        },
        (1, 7),
        &mut ImageImportTelemetry::default(),
    )
}

#[test]
fn adam7_color_and_alpha_samples_preserve_coordinates_and_precision() {
    for (width, height) in [(1, 1), (1, 9), (9, 1), (9, 9)] {
        for bits in [8, 16] {
            for (color_type, components, alpha) in
                [(0, 1, false), (2, 3, false), (4, 1, true), (6, 3, true)]
            {
                let sample_bytes = usize::from(bits / 8);
                let color_bytes = components * sample_bytes;
                let pixel_bytes = color_bytes + usize::from(alpha) * sample_bytes;
                let pixels: Vec<u8> = (0..width * height * pixel_bytes as u32)
                    .map(|index| (index.wrapping_mul(37) % 256) as u8)
                    .collect();
                let png = adam7_png(width, height, bits, color_type, &pixels);
                let result = lower(&png, metadata(width, height, bits, color_type))
                    .expect("valid PNG sample stream");
                assert_eq!(result.2, bits);
                let expected_color: Vec<u8> = pixels
                    .chunks_exact(pixel_bytes)
                    .flat_map(|pixel| pixel[..color_bytes].iter().copied())
                    .collect();
                assert_eq!(
                    inflate(&result.0).expect("valid PNG sample stream"),
                    expected_color
                );
                if alpha {
                    let expected_alpha: Vec<u8> = pixels
                        .chunks_exact(pixel_bytes)
                        .flat_map(|pixel| pixel[color_bytes..].iter().copied())
                        .collect();
                    assert_eq!(
                        inflate(&result.4.expect("valid PNG sample stream").0)
                            .expect("valid PNG sample stream"),
                        expected_alpha
                    );
                } else {
                    assert!(result.4.is_none());
                }
            }
        }
    }
}

#[test]
fn adam7_packed_palette_expands_color_and_transparency() {
    let png = adam7_png(9, 2, 1, 3, &[0b0101_0101, 0, 0b1010_1010, 128]);
    let result = lower(&png, metadata(9, 2, 1, 3)).expect("valid PNG sample stream");
    let indices = (0..18).map(|index| (index % 9 + index / 9) % 2);
    let color: Vec<u8> = indices
        .clone()
        .flat_map(|index| {
            if index == 0 {
                [10, 20, 30]
            } else {
                [40, 50, 60]
            }
        })
        .collect();
    let alpha: Vec<u8> = indices
        .map(|index| if index == 0 { 0 } else { 123 })
        .collect();
    assert_eq!(result.2, 8);
    assert_eq!(inflate(&result.0).expect("valid PNG sample stream"), color);
    assert_eq!(
        inflate(&result.4.expect("valid PNG sample stream").0).expect("valid PNG sample stream"),
        alpha
    );
}

#[test]
fn adam7_high_color_policy_strips_both_color_and_alpha_low_bytes() {
    let png = adam7_png(1, 1, 16, 4, &[42, 99, 123, 10]);
    let result = raster_image_streams(
        &png,
        metadata(1, 1, 16, 4),
        PdfImageGammaInput {
            gamma: 1000,
            image_gamma: 1000,
            high_color: true,
            apply_gamma: true,
        },
        (1, 4),
        &mut ImageImportTelemetry::default(),
    )
    .expect("valid PNG sample stream");
    assert_eq!(result.2, 8);
    assert_eq!(inflate(&result.0).expect("valid PNG sample stream"), [42]);
    assert_eq!(
        inflate(&result.4.expect("valid PNG sample stream").0).expect("valid PNG sample stream"),
        [123]
    );
}

#[test]
fn adam7_rejects_truncation_crc_damage_and_metadata_mismatch() {
    let png = adam7_png(1, 1, 8, 6, &[1, 2, 3, 4]);
    for length in [0, 8, png.len() - 1, png.len() - 12] {
        assert!(lower(&png[..length], metadata(1, 1, 8, 6)).is_err());
    }
    let mut damaged = png.clone();
    damaged[29] ^= 1;
    assert!(lower(&damaged, metadata(1, 1, 8, 6)).is_err());
    assert!(lower(&png, metadata(2, 1, 8, 6)).is_err());
    assert!(lower(&png, metadata(1, 1, 8, 4)).is_err());
    assert!(lower(&png, metadata(u32::MAX, u32::MAX, 8, 6)).is_err());
}
