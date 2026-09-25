//! Adam7 decoding into ordinary PDF color and soft-mask sample planes.

use super::*;

#[allow(clippy::disallowed_methods)] // Process telemetry; PDF content never observes it.
pub(super) fn streams(
    bytes: &[u8],
    metadata: RasterMetadata,
    telemetry: &mut ImageImportTelemetry,
) -> Result<RasterStreams, PdfBuildError> {
    // PDF predictors consume complete scanlines, never Adam7 pass rows.
    // pdfTeX writepng.c likewise uses png_read_image for interlaced inputs.
    let started = std::time::Instant::now();
    validate_png_crc(bytes)?;
    let mut options = png::DecodeOptions::default();
    options.set_ignore_adler32(false);
    options.set_ignore_crc(false);
    options.set_ignore_text_chunk(true);
    options.set_ignore_iccp_chunk(true);
    options.set_skip_ancillary_crc_failures(false);
    let mut decoder = png::Decoder::new_with_options(std::io::Cursor::new(bytes), options);
    decoder.set_limits(png::Limits {
        bytes: MAX_IMPORTED_PDF_STREAM_BYTES,
    });
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(|_| PdfBuildError::InvalidPng)?;
    let info = reader.info();
    if info.width != metadata.width
        || info.height != metadata.height
        || info.bit_depth as u8 != metadata.bits_per_component
        || Some(info.color_type as u8) != metadata.png_color_type
        || !info.interlaced
    {
        return Err(PdfBuildError::InvalidPng);
    }
    let length = reader
        .output_buffer_size()
        .filter(|length| *length <= MAX_IMPORTED_PDF_STREAM_BYTES)
        .ok_or(PdfBuildError::InvalidPng)?;
    let mut samples = vec![0; length];
    let output = reader
        .next_frame(&mut samples)
        .map_err(|_| PdfBuildError::InvalidPng)?;
    reader.finish().map_err(|_| PdfBuildError::InvalidPng)?;
    if output.width != metadata.width
        || output.height != metadata.height
        || output.buffer_size() != length
    {
        return Err(PdfBuildError::InvalidPng);
    }
    telemetry.decode_ns += started.elapsed().as_nanos();
    telemetry.raw_bytes = telemetry.raw_bytes.saturating_add(length);
    telemetry.rows = telemetry.rows.saturating_add(metadata.height as usize);
    telemetry.pixels = telemetry
        .pixels
        .saturating_add(metadata.width as usize * metadata.height as usize);
    let (color_space, components, has_alpha) = match output.color_type {
        png::ColorType::Grayscale => (PdfImageColorSpace::DeviceGray, 1, false),
        png::ColorType::GrayscaleAlpha => (PdfImageColorSpace::DeviceGray, 1, true),
        png::ColorType::Rgb => (PdfImageColorSpace::DeviceRgb, 3, false),
        png::ColorType::Rgba => (PdfImageColorSpace::DeviceRgb, 3, true),
        png::ColorType::Indexed => return Err(PdfBuildError::InvalidPng),
    };
    let bits = output.bit_depth as u8;
    if !matches!(bits, 8 | 16) {
        return Err(PdfBuildError::InvalidPng);
    }
    let started = std::time::Instant::now();
    let (color, alpha) = if has_alpha {
        let sample_bytes = usize::from(bits / 8);
        let color_bytes = components * sample_bytes;
        let pixel_bytes = color_bytes + sample_bytes;
        let mut color = Vec::with_capacity(length / pixel_bytes * color_bytes);
        let mut alpha = Vec::with_capacity(length / pixel_bytes * sample_bytes);
        for pixel in samples.chunks_exact(pixel_bytes) {
            color.extend_from_slice(&pixel[..color_bytes]);
            alpha.extend_from_slice(&pixel[color_bytes..]);
        }
        (color, Some(alpha))
    } else {
        (samples, None)
    };
    telemetry.transform_ns += started.elapsed().as_nanos();
    telemetry.color_bytes = telemetry.color_bytes.saturating_add(color.len());
    telemetry.alpha_bytes = telemetry
        .alpha_bytes
        .saturating_add(alpha.as_ref().map_or(0, Vec::len));
    let started = std::time::Instant::now();
    let result = (
        zlib(&color)?,
        PdfImageFilter::Flate,
        bits,
        color_space,
        alpha
            .map(|alpha| zlib(&alpha).map(|bytes| (bytes, PdfImageFilter::Flate)))
            .transpose()?,
    );
    telemetry.encode_ns += started.elapsed().as_nanos();
    Ok(result)
}

#[cfg(test)]
mod tests;
