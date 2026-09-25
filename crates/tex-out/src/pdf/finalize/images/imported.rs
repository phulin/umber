//! Imported PDF page image geometry and dependencies.

use super::*;

pub(in crate::pdf::finalize) struct ImportedPdfPage {
    pub(in crate::pdf::finalize) form: PdfIndirectObject,
    pub(in crate::pdf::finalize) dependencies: Vec<PdfIndirectObject>,
    pub(in crate::pdf::finalize) group: Option<PdfObjectId>,
}

// Imported page resources are attacker-controlled input. Keep a per-stream
// ceiling below the detached document's aggregate 1 GiB stream budget so a
// single pass-through image cannot consume the whole finalization allowance.
pub(in crate::pdf::finalize) const MAX_IMPORTED_PDF_STREAM_BYTES: usize = 256 * 1024 * 1024;

pub(in crate::pdf::finalize) fn import_pdf_page(
    image: &crate::pdf::PdfExternalImageInput,
    page: u32,
    page_box: crate::pdf::PdfPageBoxInput,
    next_object: &mut u32,
    limits: crate::pdf::PdfFinalizationLimits,
) -> Result<ImportedPdfPage, PdfBuildError> {
    let imported =
        crate::pdf::import::import_pdf_page(image.bytes.clone(), page, next_object, limits)
            .map_err(PdfBuildError::InvalidPdfPage)?;
    let mut dictionary = PdfDictionary::new();
    // pdftex.web §776 emits image attributes before delegating to the
    // backend that writes the imported page's Form XObject entries.
    dictionary.set_raw_entries(image.attributes.clone());
    dictionary.insert("FormType", PdfValue::Integer(1))?;
    dictionary.insert("Resources", PdfValue::Dictionary(imported.resources))?;
    if let Some(group) = imported.group {
        dictionary.insert("Group", PdfValue::Reference(group))?;
    }
    let width = page_box
        .right
        .checked_sub(page_box.left)
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    let height = page_box
        .top
        .checked_sub(page_box.bottom)
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    if width.raw() <= 0 || height.raw() <= 0 {
        return Err(PdfBuildError::InvalidPdfPage(
            "selected page box is empty".to_owned(),
        ));
    }
    Ok(ImportedPdfPage {
        form: PdfIndirectObject {
            id: object_id(image.object)?,
            object: PdfObject::FormXObject {
                dictionary,
                data: imported.data,
                // pdfTeX's pdftoepdf.cc writes the original page box and the
                // rotation on the Form, before page-content placement.
                bbox: imported_pdf_form_bbox(page_box)?,
                matrix: imported_pdf_form_matrix(page_box, image.metadata)?,
            },
        },
        dependencies: imported.dependencies,
        group: imported.group,
    })
}

pub(in crate::pdf::finalize) fn imported_pdf_form_matrix(
    page_box: crate::pdf::PdfPageBoxInput,
    metadata: PdfImageMetadataInput,
) -> Result<Option<[PdfNumber; 6]>, PdfBuildError> {
    let PdfImageMetadataInput::PdfPage { rotation, .. } = metadata else {
        return Err(PdfBuildError::InvalidPdfPage(
            "expected PDF page metadata".to_owned(),
        ));
    };
    let [left, bottom, right, top] = page_box.source.map(f64::from_bits);
    let (linear, offset) = match rotation {
        PdfPageRotationInput::None => return Ok(None),
        PdfPageRotationInput::Clockwise90 => ([0, -1, 1, 0], [left - bottom, bottom + right]),
        PdfPageRotationInput::UpsideDown => ([-1, 0, 0, -1], [left + right, bottom + top]),
        PdfPageRotationInput::Clockwise270 => ([0, 1, -1, 0], [left + top, bottom - left]),
    };
    let mut matrix = [PdfNumber::new(0, 0)?; 6];
    for (index, coefficient) in linear.into_iter().enumerate() {
        matrix[index] = PdfNumber::new(coefficient, 0)?;
    }
    for (index, coordinate) in offset.into_iter().enumerate() {
        matrix[index + 4] = pdftex_source_number(coordinate)?;
    }
    Ok(Some(matrix))
}

pub(in crate::pdf::finalize) fn imported_pdf_form_bbox(
    page_box: crate::pdf::PdfPageBoxInput,
) -> Result<[PdfNumber; 4], PdfBuildError> {
    Ok([
        pdftex_source_number(f64::from_bits(page_box.source[0]))?,
        pdftex_source_number(f64::from_bits(page_box.source[1]))?,
        pdftex_source_number(f64::from_bits(page_box.source[2]))?,
        pdftex_source_number(f64::from_bits(page_box.source[3]))?,
    ])
}

/// `pdftoepdf.cc::write_epdf` formats each final source-space operand with
/// `%.8f`. Format after double-precision box arithmetic, not before it.
fn pdftex_source_number(value: f64) -> Result<PdfNumber, PdfBuildError> {
    if !value.is_finite() {
        return Err(PdfBuildError::PageGeometryOverflow);
    }
    let decimal = format!("{value:.8}");
    let negative = decimal.starts_with('-');
    let mut coefficient = 0_i64;
    for digit in decimal.bytes().filter(|digit| digit.is_ascii_digit()) {
        coefficient = coefficient
            .checked_mul(10)
            .and_then(|value| value.checked_add(i64::from(digit - b'0')))
            .ok_or(PdfBuildError::PageGeometryOverflow)?;
    }
    if negative {
        coefficient = coefficient
            .checked_neg()
            .ok_or(PdfBuildError::PageGeometryOverflow)?;
    }
    PdfNumber::new(coefficient, 8).map_err(Into::into)
}

pub(in crate::pdf::finalize) fn rotation_swaps_axes(rotation: PdfPageRotationInput) -> bool {
    matches!(
        rotation,
        PdfPageRotationInput::Clockwise90 | PdfPageRotationInput::Clockwise270
    )
}

pub(in crate::pdf::finalize) fn imported_pdf_page_origin(
    page_box: crate::pdf::PdfPageBoxInput,
    decimal_digits: i32,
) -> Result<[PdfNumber; 2], PdfBuildError> {
    Ok([
        negate_pdf_number(scaled_to_bp_number_checked(page_box.left, decimal_digits)?)?,
        negate_pdf_number(scaled_to_bp_number_checked(
            page_box.bottom,
            decimal_digits,
        )?)?,
    ])
}

pub(in crate::pdf::finalize) fn imported_pdf_page_matrix(
    base_x: Scaled,
    base_y: Scaled,
    width: Scaled,
    total_height: Scaled,
    page_box: crate::pdf::PdfPageBoxInput,
    rotation: PdfPageRotationInput,
    decimal_digits: i32,
) -> Result<[PdfNumber; 6], PdfBuildError> {
    let box_width = page_box
        .right
        .checked_sub(page_box.left)
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    let box_height = page_box
        .top
        .checked_sub(page_box.bottom)
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    if box_width.raw() <= 0 || box_height.raw() <= 0 {
        return Err(PdfBuildError::InvalidPdfPage(
            "selected page box is empty".to_owned(),
        ));
    }
    let (natural_width, natural_height) = if rotation_swaps_axes(rotation) {
        (box_height, box_width)
    } else {
        (box_width, box_height)
    };
    let width_scale = scaled_ratio_number(width, natural_width)?;
    let height_scale = scaled_ratio_number(total_height, natural_height)?;
    // pdfTeX places the selected box using a scale and a subsequent origin
    // translation. Its page rotation remains on the imported Form object.
    let zero = PdfNumber::new(0, 0)?;
    Ok([
        width_scale,
        zero,
        zero,
        height_scale,
        scaled_to_bp_number_checked(base_x, decimal_digits)?,
        scaled_to_bp_number_checked(base_y, decimal_digits)?,
    ])
}

pub(in crate::pdf::finalize) fn negate_pdf_number(
    value: PdfNumber,
) -> Result<PdfNumber, PdfBuildError> {
    PdfNumber::new(
        value
            .coefficient()
            .checked_neg()
            .ok_or(PdfBuildError::PageGeometryOverflow)?,
        value.decimal_places(),
    )
    .map_err(Into::into)
}
