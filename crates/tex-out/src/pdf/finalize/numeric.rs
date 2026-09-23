//! Private numeric lowering for PDF finalization.

use super::*;

pub(super) fn scaled_ratio_number(
    value: Scaled,
    divisor: Scaled,
) -> Result<PdfNumber, PdfBuildError> {
    let denominator = i128::from(divisor.raw());
    if denominator <= 0 {
        return Err(PdfBuildError::PageGeometryOverflow);
    }
    let numerator = i128::from(value.raw())
        .checked_mul(1_000_000)
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    let coefficient = round_divide_away_from_zero(numerator, denominator)?;
    let coefficient =
        i64::try_from(coefficient).map_err(|_| PdfBuildError::PageGeometryOverflow)?;
    PdfNumber::new(coefficient, 6).map_err(Into::into)
}

pub(super) fn scaled_product_divide(
    value: Scaled,
    factor: Scaled,
    divisor: Scaled,
) -> Result<Scaled, PdfBuildError> {
    let denominator = i128::from(divisor.raw());
    if denominator <= 0 {
        return Err(PdfBuildError::PageGeometryOverflow);
    }
    let numerator = i128::from(value.raw())
        .checked_mul(i128::from(factor.raw()))
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    let result = round_divide_away_from_zero(numerator, denominator)?;
    let result = i64::try_from(result).map_err(|_| PdfBuildError::PageGeometryOverflow)?;
    let result = i32::try_from(result).map_err(|_| PdfBuildError::PageGeometryOverflow)?;
    Ok(Scaled::from_raw(result))
}

pub(super) fn round_divide_away_from_zero(
    numerator: i128,
    denominator: i128,
) -> Result<i128, PdfBuildError> {
    if denominator <= 0 {
        return Err(PdfBuildError::PageGeometryOverflow);
    }
    let half = denominator / 2;
    let adjusted = if numerator >= 0 {
        numerator
            .checked_add(half)
            .ok_or(PdfBuildError::PageGeometryOverflow)?
    } else {
        numerator
            .checked_sub(half)
            .ok_or(PdfBuildError::PageGeometryOverflow)?
    };
    Ok(adjusted / denominator)
}

pub(super) fn scaled_to_bp_number_checked(
    value: Scaled,
    decimal_digits: i32,
) -> Result<PdfNumber, PdfBuildError> {
    let decimal_digits =
        u32::try_from(decimal_digits).map_err(|_| PdfBuildError::PageGeometryOverflow)?;
    if decimal_digits > 9 {
        return Err(PdfBuildError::Model(
            PdfModelError::NumberPrecisionTooLarge(u8::try_from(decimal_digits).unwrap_or(u8::MAX)),
        ));
    }
    let scale = 10_i128
        .checked_pow(decimal_digits)
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    const NUMERATOR: i128 = 7_200;
    const DENOMINATOR: i128 = 7_227 * 65_536;
    let numerator = i128::from(value.raw())
        .checked_mul(NUMERATOR)
        .and_then(|value| value.checked_mul(scale))
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    let coefficient = round_divide_away_from_zero(numerator, DENOMINATOR)?;
    let coefficient =
        i64::try_from(coefficient).map_err(|_| PdfBuildError::PageGeometryOverflow)?;
    PdfNumber::new(coefficient, decimal_digits as u8).map_err(Into::into)
}

pub(super) fn object_id(raw: u32) -> Result<PdfObjectId, PdfBuildError> {
    PdfObjectId::new(raw).ok_or(PdfBuildError::InvalidObjectId(raw))
}

pub(super) fn indirect_dictionary(id: PdfObjectId, dictionary: PdfDictionary) -> PdfIndirectObject {
    PdfIndirectObject {
        id,
        object: PdfObject::Value(PdfValue::Dictionary(dictionary)),
    }
}

pub(super) fn pdf_page_extents(
    artifact: &crate::PageArtifact,
    record: &super::super::PdfCommittedPageInput,
) -> Result<(Scaled, Scaled), PdfBuildError> {
    let root = match &artifact.root {
        PageNode::HList(root) | PageNode::VList(root) => root,
        _ => unreachable!("validated artifact root is a box"),
    };
    let h_offset = record
        .h_origin()
        .checked_add(artifact.job.h_offset)
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    let v_offset = record
        .v_origin()
        .checked_add(artifact.job.v_offset)
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    let width = if record.width().raw() == 0 {
        root.width
            .checked_add(h_offset)
            .and_then(|value| value.checked_add(h_offset))
            .ok_or(PdfBuildError::PageGeometryOverflow)?
    } else {
        record.width()
    };
    let height = if record.height().raw() == 0 {
        root.height
            .checked_add(root.depth)
            .and_then(|value| value.checked_add(v_offset))
            .and_then(|value| value.checked_add(v_offset))
            .ok_or(PdfBuildError::PageGeometryOverflow)?
    } else {
        record.height()
    };
    Ok((width, height))
}

pub(super) fn scaled_to_bp_f32(value: Scaled, decimal_digits: i32) -> f32 {
    let scale = 10_f32.powi(decimal_digits);
    scaled_to_bp_coefficient(value, decimal_digits) as f32 / scale
}

pub(super) fn scaled_to_bp_f64(value: Scaled, decimal_digits: i32) -> f64 {
    let scale = 10_f64.powi(decimal_digits);
    scaled_to_bp_coefficient(value, decimal_digits) as f64 / scale
}

pub(in crate::pdf) fn pdftex_font_size(value: Scaled) -> f32 {
    // pdftex.web §690 (`pdf_set_font`, `pdf_use_font`, and
    // `adv_char_width`) uses one four-place font-size raster for both the
    // serialized `Tf` operand and cumulative character-width accounting,
    // independently of `\pdfdecimaldigits`.
    scaled_to_bp_f32(value, 4)
}

pub(super) fn pdftex_font_size_f64(value: Scaled) -> f64 {
    scaled_to_bp_coefficient(value, 4) as f64 / 10_000.0
}

pub(in crate::pdf) fn pdftex_scalable_width_tenths(
    width: Scaled,
    font_size: Scaled,
) -> Option<i64> {
    // pdftex.web §690 and writefont.c `create_charwidth_array`: first retain
    // the six-place font-size raster selected by `pdf_use_font`, then divide
    // each TFM width onto the 1/10000 raster consumed by `adv_char_width`.
    // `/Widths` prints that coefficient with one fractional decimal place.
    let font_size_raster = pdftex_font_size_raster(font_size)?;
    let (coefficient, _) =
        pdftex_divide_scaled_positive(i64::from(width.raw()), font_size_raster, 4)?;
    Some(coefficient)
}

pub(super) fn pdftex_font_size_raster(font_size: Scaled) -> Option<i64> {
    const ONE_HUNDRED_BP: i64 = 6_578_176;
    pdftex_divide_scaled_positive(i64::from(font_size.raw()), ONE_HUNDRED_BP, 6)
        .map(|(_, scaled_out)| scaled_out)
}

pub(super) fn pdftex_divide_scaled_positive(
    value: i64,
    divisor: i64,
    decimal_digits: u32,
) -> Option<(i64, i64)> {
    if value < 0 || divisor <= 0 {
        return None;
    }
    let scale = 10_i128.checked_pow(decimal_digits)?;
    let numerator = i128::from(value).checked_mul(scale)?;
    let divisor = i128::from(divisor);
    let quotient = (numerator + divisor / 2) / divisor;
    let remainder = numerator - quotient * divisor;
    let scaled_out = i128::from(value) - remainder / scale;
    Some((
        i64::try_from(quotient).ok()?,
        i64::try_from(scaled_out).ok()?,
    ))
}

pub(super) fn scaled_to_bp_unrounded_f64(value: Scaled) -> f64 {
    f64::from(value.raw()) * 7_200.0 / (7_227.0 * 65_536.0)
}

pub(super) fn scaled_to_bp_number(
    value: Scaled,
    decimal_digits: i32,
) -> Result<PdfNumber, PdfModelError> {
    PdfNumber::new(
        scaled_to_bp_coefficient(value, decimal_digits),
        decimal_digits as u8,
    )
}

pub(super) fn scaled_to_bp_coefficient(value: Scaled, decimal_digits: i32) -> i64 {
    let scale = 10_i128.pow(decimal_digits as u32);
    const NUMERATOR: i128 = 7_200;
    const DENOMINATOR: i128 = 7_227 * 65_536;
    let numerator = i128::from(value.raw()) * NUMERATOR * scale;
    let rounded = if numerator >= 0 {
        (numerator + DENOMINATOR / 2) / DENOMINATOR
    } else {
        (numerator - DENOMINATOR / 2) / DENOMINATOR
    };
    rounded as i64
}
