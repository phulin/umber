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
                // A Form's BBox is expressed in form space and is clipped
                // before its Matrix is applied. Preserve the selected page's
                // coordinates here. Imported placement transforms are emitted
                // at the page-content inclusion site because pdf_writer's
                // FormXObject matrix API narrows operands through f32.
                bbox: imported_pdf_form_bbox(page_box)?,
                matrix: None,
            },
        },
        dependencies: imported.dependencies,
        group: imported.group,
    })
}

pub(in crate::pdf::finalize) fn imported_pdf_form_bbox(
    page_box: crate::pdf::PdfPageBoxInput,
) -> Result<[PdfNumber; 4], PdfBuildError> {
    Ok([
        scaled_to_bp_number_checked(page_box.left, 4)?,
        scaled_to_bp_number_checked(page_box.bottom, 4)?,
        scaled_to_bp_number_checked(page_box.right, 4)?,
        scaled_to_bp_number_checked(page_box.top, 4)?,
    ])
}

pub(in crate::pdf::finalize) fn rotation_swaps_axes(rotation: PdfPageRotationInput) -> bool {
    matches!(
        rotation,
        PdfPageRotationInput::Clockwise90 | PdfPageRotationInput::Clockwise270
    )
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
    let (x_offset, y_offset) = match rotation {
        PdfPageRotationInput::None => (
            scaled_product_divide(width, page_box.left, natural_width)?
                .checked_neg()
                .ok_or(PdfBuildError::PageGeometryOverflow)?,
            scaled_product_divide(total_height, page_box.bottom, natural_height)?
                .checked_neg()
                .ok_or(PdfBuildError::PageGeometryOverflow)?,
        ),
        PdfPageRotationInput::Clockwise90 => (
            scaled_product_divide(width, page_box.top, natural_width)?,
            scaled_product_divide(total_height, page_box.left, natural_height)?
                .checked_neg()
                .ok_or(PdfBuildError::PageGeometryOverflow)?,
        ),
        PdfPageRotationInput::UpsideDown => (
            scaled_product_divide(width, page_box.right, natural_width)?,
            scaled_product_divide(total_height, page_box.top, natural_height)?,
        ),
        PdfPageRotationInput::Clockwise270 => (
            scaled_product_divide(width, page_box.bottom, natural_width)?
                .checked_neg()
                .ok_or(PdfBuildError::PageGeometryOverflow)?,
            scaled_product_divide(total_height, page_box.right, natural_height)?,
        ),
    };
    let x = base_x
        .checked_add(x_offset)
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    let y = base_y
        .checked_add(y_offset)
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    let zero = PdfNumber::new(0, 0)?;
    let (a, b, c, d) = match rotation {
        PdfPageRotationInput::None => (width_scale, zero, zero, height_scale),
        PdfPageRotationInput::Clockwise90 => {
            (zero, height_scale, negate_pdf_number(width_scale)?, zero)
        }
        PdfPageRotationInput::UpsideDown => (
            negate_pdf_number(width_scale)?,
            zero,
            zero,
            negate_pdf_number(height_scale)?,
        ),
        PdfPageRotationInput::Clockwise270 => {
            (zero, negate_pdf_number(height_scale)?, width_scale, zero)
        }
    };
    Ok([
        a,
        b,
        c,
        d,
        scaled_to_bp_number_checked(x, decimal_digits)?,
        scaled_to_bp_number_checked(y, decimal_digits)?,
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
