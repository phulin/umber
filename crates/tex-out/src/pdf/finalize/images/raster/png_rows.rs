//! PNG row and alpha stream processing.

use super::*;

#[allow(clippy::disallowed_methods)] // Process telemetry; PDF content never observes it.
pub(super) fn png_alpha_streams(
    bytes: &[u8],
    metadata: RasterMetadata,
    telemetry: &mut ImageImportTelemetry,
) -> Result<(Vec<u8>, PdfImageFilter, Vec<u8>, PdfImageFilter), PdfBuildError> {
    if !matches!(metadata.bits_per_component, 8 | 16) {
        return Err(PdfBuildError::InvalidPng);
    }
    let color_components = usize::from(raster_color_components(metadata.color_space));
    let component_bytes = usize::from(metadata.bits_per_component / 8);
    let pixel_bytes = (color_components + 1) * component_bytes;
    let width = usize::try_from(metadata.width).map_err(|_| PdfBuildError::InvalidPng)?;
    let row_bytes = width
        .checked_mul(pixel_bytes)
        .ok_or(PdfBuildError::InvalidPng)?;
    let height = usize::try_from(metadata.height).map_err(|_| PdfBuildError::InvalidPng)?;
    let pixels = width.checked_mul(height).ok_or(PdfBuildError::InvalidPng)?;
    telemetry.pixels = telemetry.pixels.saturating_add(pixels);
    telemetry.rows = telemetry.rows.saturating_add(height);
    telemetry.raw_bytes = telemetry
        .raw_bytes
        .saturating_add(row_bytes.saturating_mul(height));
    if metadata.bits_per_component == 8 {
        return png_alpha_streams_filtered(
            bytes,
            metadata,
            width,
            height,
            row_bytes,
            pixel_bytes,
            telemetry,
        );
    }
    let started = std::time::Instant::now();
    let compressed = png_idat(bytes)?;
    telemetry.parse_copy_ns += started.elapsed().as_nanos();
    let started = std::time::Instant::now();
    let mut decoder = flate2::read::ZlibDecoder::new(compressed.as_slice());
    let mut filtered = Vec::new();
    decoder
        .read_to_end(&mut filtered)
        .map_err(|_| PdfBuildError::InvalidPng)?;
    telemetry.decode_ns += started.elapsed().as_nanos();
    if filtered.len() != (row_bytes + 1).saturating_mul(height) {
        return Err(PdfBuildError::InvalidPng);
    }
    let started = std::time::Instant::now();
    let mut previous = vec![0u8; row_bytes];
    let mut current = vec![0u8; row_bytes];
    let mut color = Vec::with_capacity(row_bytes * height);
    let mut alpha = Vec::with_capacity(width * component_bytes * height);
    telemetry.color_bytes = telemetry.color_bytes.saturating_add(
        width
            .saturating_mul(color_components * component_bytes)
            .saturating_mul(height),
    );
    telemetry.alpha_bytes = telemetry
        .alpha_bytes
        .saturating_add(width.saturating_mul(component_bytes).saturating_mul(height));
    for row in filtered.chunks_exact(row_bytes + 1) {
        unfilter_png_row(row[0], &row[1..], &previous, &mut current, pixel_bytes)?;
        for pixel in current.chunks_exact(pixel_bytes) {
            color.extend_from_slice(&pixel[..color_components * component_bytes]);
            alpha.extend_from_slice(&pixel[color_components * component_bytes..]);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    telemetry.transform_ns += started.elapsed().as_nanos();
    let started = std::time::Instant::now();
    let streams = (zlib(&color)?, zlib(&alpha)?);
    telemetry.encode_ns += started.elapsed().as_nanos();
    Ok((
        streams.0,
        PdfImageFilter::Flate,
        streams.1,
        PdfImageFilter::Flate,
    ))
}

#[allow(clippy::disallowed_methods)] // Process telemetry; PDF content never observes it.
pub(super) fn png_alpha_streams_filtered(
    png_bytes: &[u8],
    metadata: RasterMetadata,
    width: usize,
    height: usize,
    row_bytes: usize,
    pixel_bytes: usize,
    telemetry: &mut ImageImportTelemetry,
) -> Result<(Vec<u8>, PdfImageFilter, Vec<u8>, PdfImageFilter), PdfBuildError> {
    let color_space = metadata.color_space;
    let color_components = usize::from(raster_color_components(color_space));
    let color_row_bytes = width
        .checked_mul(color_components)
        .ok_or(PdfBuildError::InvalidPng)?;
    telemetry.color_bytes = telemetry
        .color_bytes
        .saturating_add((color_row_bytes + 1).saturating_mul(height));
    telemetry.alpha_bytes = telemetry
        .alpha_bytes
        .saturating_add((width + 1).saturating_mul(height));
    let filtered_row_bytes = row_bytes.checked_add(1).ok_or(PdfBuildError::InvalidPng)?;
    let decoder_buffer_bytes = (32 * 1024usize)
        .checked_add(8 * 1024)
        .and_then(|size| size.checked_add(filtered_row_bytes.checked_mul(2)?))
        .ok_or(PdfBuildError::InvalidPng)?;
    telemetry.peak_row_bytes = telemetry.peak_row_bytes.max(
        decoder_buffer_bytes
            .saturating_add(color_row_bytes + 1)
            .saturating_add(width + 1),
    );
    let mut decoder = strict_png_decoder();
    let mut decoder_buffer = vec![0; decoder_buffer_bytes];
    let mut decoder_region = png::UnfilterRegion::default();
    let mut color_encoder = flate2::write::ZlibEncoder::new(
        Vec::new(),
        flate2::Compression::new(DERIVED_IMAGE_COMPRESSION_LEVEL),
    );
    let mut alpha_encoder = flate2::write::ZlibEncoder::new(
        Vec::new(),
        flate2::Compression::new(DERIVED_IMAGE_COMPRESSION_LEVEL),
    );
    let mut color_row = vec![0; color_row_bytes + 1];
    let mut alpha_row = vec![0; width + 1];
    let mut input = png_bytes;
    let mut rows = 0usize;
    let mut saw_iend = false;
    let mut stalled_updates = 0u8;
    while !input.is_empty() && !saw_iend {
        let started = std::time::Instant::now();
        let (consumed, decoded) = decoder
            .update(input, Some(&mut decoder_region.as_buf(&mut decoder_buffer)))
            .map_err(|_| PdfBuildError::InvalidPng)?;
        input = &input[consumed..];
        telemetry.decode_ns += started.elapsed().as_nanos();
        if let png::Decoded::ChunkBegin(length, _) = decoded
            && usize::try_from(length).ok().is_none_or(|length| {
                length > png_bytes.len() || length > MAX_IMPORTED_PDF_STREAM_BYTES
            })
        {
            return Err(PdfBuildError::InvalidPng);
        }
        if let Some(info) = decoder.info()
            && (info.width != metadata.width
                || info.height != metadata.height
                || info.bit_depth != png::BitDepth::Eight
                || info.color_type
                    != match metadata.png_color_type {
                        Some(4) => png::ColorType::GrayscaleAlpha,
                        Some(6) => png::ColorType::Rgba,
                        _ => return Err(PdfBuildError::InvalidPng),
                    }
                || info.interlaced)
        {
            return Err(PdfBuildError::InvalidPng);
        }
        rows = rows
            .checked_add(split_available_png_rows(
                &mut decoder_buffer,
                &mut decoder_region,
                filtered_row_bytes,
                pixel_bytes,
                color_components,
                &mut color_row,
                &mut alpha_row,
                &mut color_encoder,
                &mut alpha_encoder,
                telemetry,
            )?)
            .ok_or(PdfBuildError::InvalidPng)?;
        if matches!(decoded, png::Decoded::ImageDataFlushed) {
            decoder_region.available = decoder_region.filled;
            rows = rows
                .checked_add(split_available_png_rows(
                    &mut decoder_buffer,
                    &mut decoder_region,
                    filtered_row_bytes,
                    pixel_bytes,
                    color_components,
                    &mut color_row,
                    &mut alpha_row,
                    &mut color_encoder,
                    &mut alpha_encoder,
                    telemetry,
                )?)
                .ok_or(PdfBuildError::InvalidPng)?;
        }
        saw_iend = matches!(decoded, png::Decoded::ChunkComplete(kind) if kind == png::chunk::IEND);
        if consumed == 0 {
            stalled_updates = stalled_updates.saturating_add(1);
            if stalled_updates > 8 {
                return Err(PdfBuildError::InvalidPng);
            }
        } else {
            stalled_updates = 0;
        }
    }
    if !saw_iend || !input.is_empty() || rows != height || decoder_region.filled != 0 {
        return Err(PdfBuildError::InvalidPng);
    }
    let started = std::time::Instant::now();
    let color = color_encoder
        .finish()
        .map_err(|_| PdfBuildError::InvalidPng)?;
    let alpha = alpha_encoder
        .finish()
        .map_err(|_| PdfBuildError::InvalidPng)?;
    telemetry.encode_ns += started.elapsed().as_nanos();
    Ok((
        color,
        PdfImageFilter::FlatePngPredictor {
            colors: raster_color_components(color_space),
        },
        alpha,
        PdfImageFilter::FlatePngPredictor { colors: 1 },
    ))
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::disallowed_methods)] // Process telemetry; PDF content never observes it.
pub(super) fn split_available_png_rows(
    decoder_buffer: &mut [u8],
    decoder_region: &mut png::UnfilterRegion,
    filtered_row_bytes: usize,
    pixel_bytes: usize,
    color_components: usize,
    color_row: &mut [u8],
    alpha_row: &mut [u8],
    color_encoder: &mut flate2::write::ZlibEncoder<Vec<u8>>,
    alpha_encoder: &mut flate2::write::ZlibEncoder<Vec<u8>>,
    telemetry: &mut ImageImportTelemetry,
) -> Result<usize, PdfBuildError> {
    let rows = decoder_region.available / filtered_row_bytes;
    for row in decoder_buffer[..rows * filtered_row_bytes].chunks_exact(filtered_row_bytes) {
        let started = std::time::Instant::now();
        if row[0] > 4 {
            return Err(PdfBuildError::InvalidPng);
        }
        color_row[0] = row[0];
        alpha_row[0] = row[0];
        for (index, pixel) in row[1..].chunks_exact(pixel_bytes).enumerate() {
            let color_start = 1 + index * color_components;
            color_row[color_start..color_start + color_components]
                .copy_from_slice(&pixel[..color_components]);
            alpha_row[index + 1] = pixel[color_components];
        }
        telemetry.transform_ns += started.elapsed().as_nanos();

        let started = std::time::Instant::now();
        color_encoder
            .write_all(color_row)
            .map_err(|_| PdfBuildError::InvalidPng)?;
        alpha_encoder
            .write_all(alpha_row)
            .map_err(|_| PdfBuildError::InvalidPng)?;
        telemetry.encode_ns += started.elapsed().as_nanos();
    }
    let consumed = rows * filtered_row_bytes;
    if consumed != 0 {
        decoder_buffer.copy_within(consumed..decoder_region.filled, 0);
        decoder_region.available -= consumed;
        decoder_region.filled -= consumed;
    }
    Ok(rows)
}

#[allow(clippy::disallowed_methods)] // Process telemetry; PDF content never observes it.
pub(super) fn png_indexed_streams(
    bytes: &[u8],
    metadata: RasterMetadata,
    telemetry: &mut ImageImportTelemetry,
) -> Result<(Vec<u8>, Option<Vec<u8>>), PdfBuildError> {
    let palette = png_chunk(bytes, b"PLTE").ok_or(PdfBuildError::InvalidPng)?;
    if palette.len() % 3 != 0 || !matches!(metadata.bits_per_component, 1 | 2 | 4 | 8) {
        return Err(PdfBuildError::InvalidPng);
    }
    let transparency = png_chunk(bytes, b"tRNS");
    let width = usize::try_from(metadata.width).map_err(|_| PdfBuildError::InvalidPng)?;
    let height = usize::try_from(metadata.height).map_err(|_| PdfBuildError::InvalidPng)?;
    let row_bytes = width
        .checked_mul(usize::from(metadata.bits_per_component))
        .and_then(|bits| bits.checked_add(7))
        .map(|bits| bits / 8)
        .ok_or(PdfBuildError::InvalidPng)?;
    let started = std::time::Instant::now();
    let compressed = png_idat(bytes)?;
    telemetry.parse_copy_ns += started.elapsed().as_nanos();
    let started = std::time::Instant::now();
    let mut decoder = flate2::read::ZlibDecoder::new(compressed.as_slice());
    let mut filtered = Vec::new();
    decoder
        .read_to_end(&mut filtered)
        .map_err(|_| PdfBuildError::InvalidPng)?;
    telemetry.decode_ns += started.elapsed().as_nanos();
    if filtered.len() != (row_bytes + 1).saturating_mul(height) {
        return Err(PdfBuildError::InvalidPng);
    }
    let started = std::time::Instant::now();
    let mut previous = vec![0u8; row_bytes];
    let mut current = vec![0u8; row_bytes];
    let mut color = Vec::with_capacity(width * height * 3);
    let mut alpha = transparency.map(|_| Vec::with_capacity(width * height));
    let bits = metadata.bits_per_component;
    let mask = (1u16 << bits) - 1;
    for row in filtered.chunks_exact(row_bytes + 1) {
        unfilter_png_row(row[0], &row[1..], &previous, &mut current, 1)?;
        for pixel in 0..width {
            let bit = pixel * usize::from(bits);
            let shift = 8 - usize::from(bits) - (bit % 8);
            let index = usize::from((u16::from(current[bit / 8]) >> shift) & mask);
            let start = index.checked_mul(3).ok_or(PdfBuildError::InvalidPng)?;
            color.extend_from_slice(
                palette
                    .get(start..start + 3)
                    .ok_or(PdfBuildError::InvalidPng)?,
            );
            if let Some(alpha) = &mut alpha {
                alpha.push(
                    transparency
                        .and_then(|values| values.get(index))
                        .copied()
                        .unwrap_or(255),
                );
            }
        }
        std::mem::swap(&mut previous, &mut current);
    }
    telemetry.transform_ns += started.elapsed().as_nanos();
    let started = std::time::Instant::now();
    let streams = (zlib(&color)?, alpha.map(|data| zlib(&data)).transpose()?);
    telemetry.encode_ns += started.elapsed().as_nanos();
    Ok(streams)
}

pub(super) fn png_chunk<'a>(bytes: &'a [u8], wanted: &[u8; 4]) -> Option<&'a [u8]> {
    let mut cursor = 8usize;
    while cursor + 12 <= bytes.len() {
        let length = u32::from_be_bytes([
            bytes[cursor],
            bytes[cursor + 1],
            bytes[cursor + 2],
            bytes[cursor + 3],
        ]) as usize;
        let end = cursor.checked_add(length + 12)?;
        if end > bytes.len() {
            return None;
        }
        if &bytes[cursor + 4..cursor + 8] == wanted {
            return Some(&bytes[cursor + 8..cursor + 8 + length]);
        }
        cursor = end;
    }
    None
}

pub(super) fn unfilter_png_row(
    filter: u8,
    source: &[u8],
    previous: &[u8],
    target: &mut [u8],
    bytes_per_pixel: usize,
) -> Result<(), PdfBuildError> {
    for index in 0..source.len() {
        let left = index.checked_sub(bytes_per_pixel).map_or(0, |i| target[i]);
        let up = previous[index];
        let upper_left = index
            .checked_sub(bytes_per_pixel)
            .map_or(0, |i| previous[i]);
        target[index] = source[index].wrapping_add(match filter {
            0 => 0,
            1 => left,
            2 => up,
            3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
            4 => paeth(left, up, upper_left),
            _ => return Err(PdfBuildError::InvalidPng),
        });
    }
    Ok(())
}

pub(super) fn paeth(left: u8, up: u8, upper_left: u8) -> u8 {
    let left = i32::from(left);
    let up = i32::from(up);
    let upper_left = i32::from(upper_left);
    let estimate = left + up - upper_left;
    let left_distance = (estimate - left).abs();
    let up_distance = (estimate - up).abs();
    let upper_left_distance = (estimate - upper_left).abs();
    if left_distance <= up_distance && left_distance <= upper_left_distance {
        left as u8
    } else if up_distance <= upper_left_distance {
        up as u8
    } else {
        upper_left as u8
    }
}

pub(super) fn zlib(bytes: &[u8]) -> Result<Vec<u8>, PdfBuildError> {
    // Generated image planes retain PNG prediction, so fast deflate bounds
    // finalization latency without discarding useful source compression structure.
    let mut encoder = flate2::write::ZlibEncoder::new(
        Vec::new(),
        flate2::Compression::new(DERIVED_IMAGE_COMPRESSION_LEVEL),
    );
    encoder
        .write_all(bytes)
        .map_err(|_| PdfBuildError::InvalidPng)?;
    encoder.finish().map_err(|_| PdfBuildError::InvalidPng)
}
