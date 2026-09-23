//! Private images lowering for PDF finalization.

use super::*;

mod png_rows;
use png_rows::*;

pub(in crate::pdf::finalize) type RasterStreams = (
    Vec<u8>,
    PdfImageFilter,
    u8,
    PdfImageColorSpace,
    Option<(Vec<u8>, PdfImageFilter)>,
);

#[derive(Default)]
pub(in crate::pdf::finalize) struct ImageImportTelemetry {
    pub(in crate::pdf::finalize) parse_copy_ns: u128,
    pub(in crate::pdf::finalize) decode_ns: u128,
    pub(in crate::pdf::finalize) transform_ns: u128,
    pub(in crate::pdf::finalize) encode_ns: u128,
    pub(in crate::pdf::finalize) cache_hits: usize,
    pub(in crate::pdf::finalize) pixels: usize,
    pub(in crate::pdf::finalize) rows: usize,
    pub(in crate::pdf::finalize) raw_bytes: usize,
    pub(in crate::pdf::finalize) color_bytes: usize,
    pub(in crate::pdf::finalize) alpha_bytes: usize,
    pub(in crate::pdf::finalize) peak_row_bytes: usize,
}

pub(in crate::pdf::finalize) const DERIVED_IMAGE_COMPRESSION_LEVEL: u32 = 1;
pub(in crate::pdf::finalize) const DERIVED_IMAGE_WINDOW_BITS: u8 = 15;

#[derive(Clone, Copy)]
pub(in crate::pdf::finalize) struct RasterMetadata {
    pub(in crate::pdf::finalize) format: PdfRasterFormatInput,
    pub(in crate::pdf::finalize) width: u32,
    pub(in crate::pdf::finalize) height: u32,
    pub(in crate::pdf::finalize) bits_per_component: u8,
    pub(in crate::pdf::finalize) color_space: PdfRasterColorSpaceInput,
    pub(in crate::pdf::finalize) alpha: bool,
    pub(in crate::pdf::finalize) png_color_type: Option<u8>,
}

pub(in crate::pdf::finalize) fn raster_needs_transparency_page_group(
    metadata: PdfImageMetadataInput,
    version: (u8, u8),
) -> bool {
    matches!(
        metadata,
        PdfImageMetadataInput::Raster {
            format: PdfRasterFormatInput::Png,
            alpha: true,
            ..
        }
    ) && (version.0 > 1 || version.1 >= 4)
}

#[allow(clippy::disallowed_methods)] // Process telemetry; PDF content never observes it.
pub(in crate::pdf::finalize) fn raster_image_streams(
    bytes: &[u8],
    metadata: RasterMetadata,
    parameters: PdfImageGammaInput,
    version: (u8, u8),
    telemetry: &mut ImageImportTelemetry,
) -> Result<RasterStreams, PdfBuildError> {
    if metadata.width == 0 || metadata.height == 0 {
        return Err(PdfBuildError::InvalidRasterDimensions);
    }
    if metadata.format == PdfRasterFormatInput::Png {
        validate_png_decoded_size(metadata)?;
    }
    let color_space = match metadata.color_space {
        PdfRasterColorSpaceInput::Gray => PdfImageColorSpace::DeviceGray,
        PdfRasterColorSpaceInput::Rgb => PdfImageColorSpace::DeviceRgb,
        PdfRasterColorSpaceInput::Cmyk => PdfImageColorSpace::DeviceCmyk,
    };
    let streams: Result<RasterStreams, PdfBuildError> = match metadata.format {
        PdfRasterFormatInput::Jpeg => Ok((
            {
                let started = std::time::Instant::now();
                let copy = bytes.to_vec();
                telemetry.parse_copy_ns += started.elapsed().as_nanos();
                copy
            },
            PdfImageFilter::Dct,
            metadata.bits_per_component,
            color_space,
            None,
        )),
        PdfRasterFormatInput::Png if metadata.png_color_type == Some(3) => {
            let (color, alpha) = png_indexed_streams(bytes, metadata, telemetry)?;
            Ok((
                color,
                PdfImageFilter::Flate,
                8,
                PdfImageColorSpace::DeviceRgb,
                alpha.map(|alpha| (alpha, PdfImageFilter::Flate)),
            ))
        }
        PdfRasterFormatInput::Png if metadata.alpha => {
            let (color, color_filter, alpha, alpha_filter) =
                png_alpha_streams(bytes, metadata, telemetry)?;
            Ok((
                color,
                color_filter,
                metadata.bits_per_component,
                color_space,
                Some((alpha, alpha_filter)),
            ))
        }
        PdfRasterFormatInput::Png => Ok((
            {
                let started = std::time::Instant::now();
                let data = png_idat(bytes)?;
                telemetry.parse_copy_ns += started.elapsed().as_nanos();
                data
            },
            PdfImageFilter::FlatePngPredictor {
                colors: raster_color_components(metadata.color_space),
            },
            metadata.bits_per_component,
            color_space,
            None,
        )),
    };
    let mut streams = streams?;
    if metadata.format == PdfRasterFormatInput::Png
        && metadata.bits_per_component == 16
        && (!parameters.high_color || (version.0 == 1 && version.1 < 5))
    {
        let samples = match streams.1 {
            PdfImageFilter::FlatePngPredictor { .. } => png_opaque_samples(bytes, metadata)?,
            PdfImageFilter::Flate => inflate(&streams.0)?,
            PdfImageFilter::Dct => unreachable!("PNG streams do not use DCT"),
        };
        streams.0 = zlib(&strip_png_16(&samples))?;
        streams.1 = PdfImageFilter::Flate;
        streams.2 = 8;
        if let Some((alpha, _)) = streams.4.take() {
            streams.4 = Some((
                zlib(&strip_png_16(&inflate(&alpha)?))?,
                PdfImageFilter::Flate,
            ));
        }
    }
    if metadata.format == PdfRasterFormatInput::Png && parameters.apply_gamma {
        let mut samples = match streams.1 {
            PdfImageFilter::FlatePngPredictor { .. } => png_opaque_samples(bytes, metadata)?,
            PdfImageFilter::Flate => inflate(&streams.0)?,
            PdfImageFilter::Dct => unreachable!("PNG streams do not use DCT"),
        };
        apply_png_gamma(&mut samples, bytes, streams.2, parameters)?;
        streams.0 = zlib(&samples)?;
        streams.1 = PdfImageFilter::Flate;
    }
    Ok(streams)
}

pub(in crate::pdf::finalize) fn validate_png_decoded_size(
    metadata: RasterMetadata,
) -> Result<(), PdfBuildError> {
    let components = match metadata.png_color_type {
        Some(0 | 3) => 1usize,
        Some(2) => 3,
        Some(4) => 2,
        Some(6) => 4,
        _ => return Err(PdfBuildError::InvalidPng),
    };
    let row_bytes = usize::try_from(metadata.width)
        .ok()
        .and_then(|width| width.checked_mul(components))
        .and_then(|samples| samples.checked_mul(usize::from(metadata.bits_per_component)))
        .and_then(|bits| bits.checked_add(7))
        .map(|bits| bits / 8)
        .ok_or(PdfBuildError::InvalidPng)?;
    let height = usize::try_from(metadata.height).map_err(|_| PdfBuildError::InvalidPng)?;
    let decoded_bytes = row_bytes
        .checked_add(1)
        .and_then(|row| row.checked_mul(height))
        .ok_or(PdfBuildError::InvalidPng)?;
    if decoded_bytes > MAX_IMPORTED_PDF_STREAM_BYTES {
        return Err(PdfBuildError::InvalidPng);
    }
    Ok(())
}

pub(in crate::pdf::finalize) fn strip_png_16(samples: &[u8]) -> Vec<u8> {
    samples
        .as_chunks::<2>()
        .0
        .iter()
        .map(|sample| sample[0])
        .collect()
}

pub(in crate::pdf::finalize) fn raster_color_components(
    color_space: PdfRasterColorSpaceInput,
) -> u8 {
    match color_space {
        PdfRasterColorSpaceInput::Gray => 1,
        PdfRasterColorSpaceInput::Rgb => 3,
        PdfRasterColorSpaceInput::Cmyk => 4,
    }
}

pub(in crate::pdf::finalize) fn image_resource_name(
    image: &crate::pdf::PdfExternalImageInput,
    parameters: FinalizationParameters,
) -> Vec<u8> {
    if parameters.unique_resource_names > 0 {
        let prefix = image.identity.hex();
        format!("{}Im{}", &prefix[..6], image.resource).into_bytes()
    } else {
        format!("Im{}", image.resource).into_bytes()
    }
}

pub(in crate::pdf::finalize) fn png_idat(bytes: &[u8]) -> Result<Vec<u8>, PdfBuildError> {
    validate_png_crc(bytes)?;
    let mut cursor = 8usize;
    let mut data = Vec::new();
    while cursor.checked_add(12).is_some_and(|end| end <= bytes.len()) {
        let length = u32::from_be_bytes([
            bytes[cursor],
            bytes[cursor + 1],
            bytes[cursor + 2],
            bytes[cursor + 3],
        ]) as usize;
        let end = cursor
            .checked_add(12)
            .and_then(|value| value.checked_add(length))
            .ok_or(PdfBuildError::InvalidPng)?;
        if end > bytes.len() {
            return Err(PdfBuildError::InvalidPng);
        }
        if &bytes[cursor + 4..cursor + 8] == b"IDAT" {
            data.extend_from_slice(&bytes[cursor + 8..cursor + 8 + length]);
        }
        cursor = end;
    }
    (!data.is_empty())
        .then_some(data)
        .ok_or(PdfBuildError::InvalidPng)
}

pub(in crate::pdf::finalize) fn strict_png_decoder() -> png::StreamingDecoder {
    let mut options = png::DecodeOptions::default();
    options.set_ignore_adler32(false);
    options.set_ignore_crc(false);
    options.set_ignore_text_chunk(true);
    options.set_ignore_iccp_chunk(true);
    options.set_skip_ancillary_crc_failures(false);
    png::StreamingDecoder::new_with_options(options)
}

pub(in crate::pdf::finalize) fn validate_png_crc(bytes: &[u8]) -> Result<(), PdfBuildError> {
    let mut decoder = strict_png_decoder();
    let mut input = bytes;
    let mut saw_iend = false;
    let mut stalled_updates = 0u8;
    while !input.is_empty() && !saw_iend {
        let (consumed, decoded) = decoder
            .update(input, None)
            .map_err(|_| PdfBuildError::InvalidPng)?;
        input = &input[consumed..];
        if let png::Decoded::ChunkBegin(length, _) = decoded
            && usize::try_from(length)
                .ok()
                .is_none_or(|length| length > bytes.len() || length > MAX_IMPORTED_PDF_STREAM_BYTES)
        {
            return Err(PdfBuildError::InvalidPng);
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
    if saw_iend && input.is_empty() {
        Ok(())
    } else {
        Err(PdfBuildError::InvalidPng)
    }
}

pub(in crate::pdf::finalize) fn inflate(bytes: &[u8]) -> Result<Vec<u8>, PdfBuildError> {
    let mut decoder = flate2::read::ZlibDecoder::new(bytes);
    let mut output = Vec::new();
    decoder
        .read_to_end(&mut output)
        .map_err(|_| PdfBuildError::InvalidPng)?;
    Ok(output)
}

pub(in crate::pdf::finalize) fn png_opaque_samples(
    bytes: &[u8],
    metadata: RasterMetadata,
) -> Result<Vec<u8>, PdfBuildError> {
    if !matches!(metadata.bits_per_component, 8 | 16) {
        return Err(PdfBuildError::InvalidPng);
    }
    let component_bytes = usize::from(metadata.bits_per_component / 8);
    let pixel_bytes = usize::from(raster_color_components(metadata.color_space)) * component_bytes;
    let row_bytes = usize::try_from(metadata.width)
        .ok()
        .and_then(|width| width.checked_mul(pixel_bytes))
        .ok_or(PdfBuildError::InvalidPng)?;
    let height = usize::try_from(metadata.height).map_err(|_| PdfBuildError::InvalidPng)?;
    let filtered = inflate(&png_idat(bytes)?)?;
    if filtered.len() != (row_bytes + 1).saturating_mul(height) {
        return Err(PdfBuildError::InvalidPng);
    }
    let mut previous = vec![0u8; row_bytes];
    let mut current = vec![0u8; row_bytes];
    let mut samples = Vec::with_capacity(row_bytes * height);
    for row in filtered.chunks_exact(row_bytes + 1) {
        unfilter_png_row(row[0], &row[1..], &previous, &mut current, pixel_bytes)?;
        samples.extend_from_slice(&current);
        std::mem::swap(&mut previous, &mut current);
    }
    Ok(samples)
}

pub(in crate::pdf::finalize) fn apply_png_gamma(
    samples: &mut [u8],
    png: &[u8],
    bits_per_component: u8,
    parameters: PdfImageGammaInput,
) -> Result<(), PdfBuildError> {
    let file_gamma = png_chunk(png, b"gAMA")
        .and_then(|chunk| <[u8; 4]>::try_from(chunk).ok())
        .map(u32::from_be_bytes)
        .map_or_else(
            || 1_000.0 / f64::from(parameters.image_gamma.max(1)),
            |gamma| f64::from(gamma) / 100_000.0,
        );
    let screen_gamma = f64::from(parameters.gamma.max(1)) / 1_000.0;
    let exponent = 1.0 / (file_gamma * screen_gamma);
    match bits_per_component {
        8 => {
            for sample in samples {
                let normalized = f64::from(*sample) / 255.0;
                *sample = (normalized.powf(exponent) * 255.0).round() as u8;
            }
        }
        16 => {
            for sample in samples.as_chunks_mut::<2>().0 {
                let value = u16::from_be_bytes([sample[0], sample[1]]);
                let normalized = f64::from(value) / 65_535.0;
                let corrected = (normalized.powf(exponent) * 65_535.0).round() as u16;
                sample.copy_from_slice(&corrected.to_be_bytes());
            }
        }
        _ => return Err(PdfBuildError::InvalidPng),
    }
    Ok(())
}
