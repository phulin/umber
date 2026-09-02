//! Lightweight host-side inspection for external PDF page requests.

use std::sync::Arc;

use hayro_syntax::object::{Array, Dict, Name, Number, Object, Rect, String as PdfString};
use hayro_syntax::page::Page;
use hayro_syntax::{Pdf, PdfVersion};
use tex_exec::PdfImagePageBox;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug)]
pub(crate) struct InspectedPdfPage {
    pub(crate) page_box: [f64; 4],
    pub(crate) rotation: tex_state::PdfPageRotation,
    pub(crate) total_pages: u32,
    pub(crate) has_page_group: bool,
    pub(crate) pdf_version: (u8, u8),
    pub(crate) page_number: u32,
}

pub(crate) fn inspect_pdf_page(
    bytes: tex_state::SharedBytes,
    selection: &tex_exec::PdfImagePageSelection,
    page_box: PdfImagePageBox,
) -> Result<InspectedPdfPage, String> {
    let pdf = load_pdf(bytes)?;
    let page_number = selected_page_number(&pdf, selection)?;
    let page = selected_page(&pdf, page_number)?;
    let keys: &[&[u8]] = match page_box {
        PdfImagePageBox::Media => &[b"MediaBox"],
        PdfImagePageBox::Crop => &[b"CropBox", b"MediaBox"],
        PdfImagePageBox::Bleed => &[b"BleedBox", b"CropBox", b"MediaBox"],
        PdfImagePageBox::Trim => &[b"TrimBox", b"CropBox", b"MediaBox"],
        PdfImagePageBox::Art => &[b"ArtBox", b"CropBox", b"MediaBox"],
    };
    let rect = keys
        .iter()
        .find_map(|key| inherited_rect(page, key))
        .ok_or_else(|| "selected PDF page box is missing".to_owned())?;
    Ok(InspectedPdfPage {
        page_box: [rect.x0, rect.y0, rect.x1, rect.y1],
        rotation: inherited_rotation(page)?,
        total_pages: u32::try_from(pdf.pages().len())
            .map_err(|_| "external PDF page count exceeds u32".to_owned())?,
        has_page_group: page.raw().contains_key(b"Group"),
        pdf_version: version_pair(pdf.version()),
        page_number,
    })
}

fn selected_page_number(
    pdf: &Pdf,
    selection: &tex_exec::PdfImagePageSelection,
) -> Result<u32, String> {
    match selection {
        tex_exec::PdfImagePageSelection::Number(page) => Ok(*page),
        tex_exec::PdfImagePageSelection::Named(name) => named_destination_page(pdf, name),
    }
}

fn named_destination_page(pdf: &Pdf, wanted: &[u8]) -> Result<u32, String> {
    let display = String::from_utf8_lossy(wanted);
    let catalog = pdf.objects().into_iter().find_map(|object| {
        let dict = object.into_dict()?;
        (dict.get::<Name<'_>>(b"Type")?.as_ref() == b"Catalog").then_some(dict)
    });
    let Some(catalog) = catalog else {
        return Err(format!("PDF inclusion: invalid destination <{display}>"));
    };
    let destination = catalog
        .get::<Dict<'_>>(b"Dests")
        .and_then(|dests| dests.get::<Object<'_>>(wanted))
        .or_else(|| {
            let names = catalog.get::<Dict<'_>>(b"Names")?;
            let root = names.get::<Dict<'_>>(b"Dests")?;
            name_tree_destination(&root, wanted, 0)
        });
    let Some(destination) = destination else {
        return Err(format!("PDF inclusion: invalid destination <{display}>"));
    };
    let destination = match destination {
        Object::Dict(dict) => dict.get::<Array<'_>>(b"D"),
        Object::Array(array) => Some(array),
        _ => None,
    };
    let Some(destination) = destination else {
        return Err(format!(
            "PDF inclusion: destination is not a page <{display}>"
        ));
    };
    let mut items = destination.flex_iter();
    let Some(page) = items.next::<Dict<'_>>() else {
        return Err(format!(
            "PDF inclusion: destination is not a page <{display}>"
        ));
    };
    pdf.pages()
        .iter()
        .position(|candidate| candidate.raw() == &page)
        .and_then(|index| u32::try_from(index + 1).ok())
        .ok_or_else(|| format!("PDF inclusion: destination is not a page <{display}>"))
}

const MAX_NAME_TREE_DEPTH: usize = 256;

fn name_tree_destination<'a>(node: &Dict<'a>, wanted: &[u8], depth: usize) -> Option<Object<'a>> {
    if depth >= MAX_NAME_TREE_DEPTH {
        return None;
    }
    if let Some(names) = node.get::<Array<'a>>(b"Names") {
        let mut entries = names.flex_iter();
        while let Some(name) = entries.next::<PdfString<'a>>() {
            let value = entries.next::<Object<'a>>()?;
            if name.as_bytes() == wanted {
                return Some(value);
            }
        }
    }
    let kids = node.get::<Array<'a>>(b"Kids")?;
    kids.iter::<Dict<'a>>()
        .find_map(|kid| name_tree_destination(&kid, wanted, depth + 1))
}

fn load_pdf(bytes: tex_state::SharedBytes) -> Result<Pdf, String> {
    Pdf::new(Arc::new(bytes)).map_err(|error| format!("{error:?}"))
}

fn selected_page(pdf: &Pdf, page_number: u32) -> Result<&Page<'_>, String> {
    let index = page_number
        .checked_sub(1)
        .and_then(|page| usize::try_from(page).ok())
        .ok_or_else(|| format!("page {page_number} does not exist"))?;
    pdf.pages()
        .get(index)
        .ok_or_else(|| format!("page {page_number} does not exist"))
}

fn inherited_rect(page: &Page<'_>, key: &[u8]) -> Option<Rect> {
    let mut dictionary = page.raw().clone();
    loop {
        if let Some(rect) = dictionary.get::<Rect>(key) {
            return Some(rect);
        }
        let parent = dictionary.get_ref(b"Parent")?;
        dictionary = page.xref().get(parent.into())?;
    }
}

fn inherited_rotation(page: &Page<'_>) -> Result<tex_state::PdfPageRotation, String> {
    let mut dictionary = page.raw().clone();
    let rotation = loop {
        if let Some(rotation) = dictionary.get::<Number>(b"Rotate") {
            break rotation.as_i64().rem_euclid(360);
        }
        let Some(parent) = dictionary.get_ref(b"Parent") else {
            break 0;
        };
        dictionary = page
            .xref()
            .get(parent.into())
            .ok_or_else(|| "PDF page Parent does not exist".to_owned())?;
    };
    match rotation {
        0 => Ok(tex_state::PdfPageRotation::None),
        90 => Ok(tex_state::PdfPageRotation::Clockwise90),
        180 => Ok(tex_state::PdfPageRotation::UpsideDown),
        270 => Ok(tex_state::PdfPageRotation::Clockwise270),
        rotation => Err(format!(
            "PDF page rotation {rotation} is not a multiple of 90"
        )),
    }
}

fn version_pair(version: PdfVersion) -> (u8, u8) {
    match version {
        PdfVersion::Pdf10 => (1, 0),
        PdfVersion::Pdf11 => (1, 1),
        PdfVersion::Pdf12 => (1, 2),
        PdfVersion::Pdf13 => (1, 3),
        PdfVersion::Pdf14 => (1, 4),
        PdfVersion::Pdf15 => (1, 5),
        PdfVersion::Pdf16 => (1, 6),
        PdfVersion::Pdf17 => (1, 7),
        PdfVersion::Pdf20 => (2, 0),
    }
}
