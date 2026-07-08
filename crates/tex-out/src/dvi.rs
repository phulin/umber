use std::fmt;

use tex_arith::Scaled;

use crate::{BoxNode, FontResource, PageArtifact, PageNode};

#[cfg(test)]
mod tests;

const ID_BYTE: u8 = 2;
const PRE: u8 = 247;
const POST: u8 = 248;
const POST_POST: u8 = 249;
const BOP: u8 = 139;
const EOP: u8 = 140;
const FNT_DEF1: u8 = 243;
const FNT_DEF2: u8 = 244;
const FNT_DEF3: u8 = 245;
const FNT_DEF4: u8 = 246;
const PADDING: u8 = 223;

const NUM: i32 = 25_400_000;
const DEN: i32 = 473_628_672;

/// DVI emission failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DviError {
    NoPages,
    EmptyFontName { font_id: u32 },
    FieldTooLong { field: &'static str, len: usize },
    MissingFont { font_id: u32 },
    InconsistentJobInfo,
    TooManyPages { pages: usize },
    OffsetOverflow { offset: usize },
}

impl fmt::Display for DviError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoPages => f.write_str("cannot write DVI without page artifacts"),
            Self::EmptyFontName { font_id } => {
                write!(f, "font resource {font_id} has an empty DVI font name")
            }
            Self::FieldTooLong { field, len } => {
                write!(f, "DVI {field} length {len} exceeds 255 bytes")
            }
            Self::MissingFont { font_id } => {
                write!(f, "page node references missing font resource {font_id}")
            }
            Self::InconsistentJobInfo => {
                f.write_str("page artifacts disagree on job banner or magnification")
            }
            Self::TooManyPages { pages } => write!(f, "DVI page count {pages} exceeds 65535"),
            Self::OffsetOverflow { offset } => {
                write!(
                    f,
                    "DVI byte offset {offset} exceeds signed 32-bit pointer range"
                )
            }
        }
    }
}

impl std::error::Error for DviError {}

/// Writes a complete DVI file from committed page artifacts.
///
/// The writer is intentionally downstream-only: all DVI preamble data, page
/// counters, dimensions, and font resources come from the artifact stream.
pub fn write_dvi(pages: &[PageArtifact]) -> Result<Vec<u8>, DviError> {
    DviWriter::new(pages)?.finish()
}

struct DviWriter<'a> {
    pages: &'a [PageArtifact],
    bytes: Vec<u8>,
    fonts: Vec<DefinedFont<'a>>,
    previous_bop: i32,
    max_height_depth: i32,
    max_width: i32,
    max_stack_depth: u16,
}

impl<'a> DviWriter<'a> {
    fn new(pages: &'a [PageArtifact]) -> Result<Self, DviError> {
        let Some(first) = pages.first() else {
            return Err(DviError::NoPages);
        };
        for page in pages {
            if page.job != first.job {
                return Err(DviError::InconsistentJobInfo);
            }
        }
        let page_count = u16::try_from(pages.len())
            .map_err(|_| DviError::TooManyPages { pages: pages.len() })?;
        let mut writer = Self {
            pages,
            bytes: Vec::new(),
            fonts: Vec::new(),
            previous_bop: -1,
            max_height_depth: 0,
            max_width: 0,
            max_stack_depth: 0,
        };
        writer.preamble(&first.job.banner, first.job.mag)?;
        debug_assert_eq!(page_count as usize, pages.len());
        Ok(writer)
    }

    fn finish(mut self) -> Result<Vec<u8>, DviError> {
        for page in self.pages {
            self.page(page)?;
        }
        self.postamble()?;
        Ok(self.bytes)
    }

    fn preamble(&mut self, banner: &str, mag: i32) -> Result<(), DviError> {
        let banner = limited_bytes("comment", banner)?;
        self.u8(PRE);
        self.u8(ID_BYTE);
        self.i32(NUM);
        self.i32(DEN);
        self.i32(mag);
        self.u8(banner.len() as u8);
        self.raw(banner);
        Ok(())
    }

    fn page(&mut self, page: &'a PageArtifact) -> Result<(), DviError> {
        let bop_location = self.current_pointer()?;
        self.u8(BOP);
        for count in page.counts {
            self.i32(count);
        }
        self.i32(self.previous_bop);
        self.previous_bop = bop_location;

        let extent = page_extent(&page.root);
        self.max_height_depth = self.max_height_depth.max(extent.height_depth);
        self.max_width = self.max_width.max(extent.width);
        self.emit_page_fonts(page, &page.root)?;
        self.u8(EOP);
        Ok(())
    }

    fn emit_page_fonts(
        &mut self,
        page: &'a PageArtifact,
        node: &'a PageNode,
    ) -> Result<(), DviError> {
        match node {
            PageNode::Char { font_id, .. } | PageNode::Lig { font_id, .. } => {
                let font = page_font(page, *font_id)?;
                self.ensure_font_defined(font)?;
            }
            PageNode::HList(box_node) | PageNode::VList(box_node) => {
                for child in &box_node.children {
                    self.emit_page_fonts(page, child)?;
                }
            }
            PageNode::Kern { .. }
            | PageNode::Glue { .. }
            | PageNode::Penalty(_)
            | PageNode::Rule { .. }
            | PageNode::Unset
            | PageNode::WhatsitAnchor { .. }
            | PageNode::MathOn
            | PageNode::MathOff => {}
        }
        Ok(())
    }

    fn ensure_font_defined(&mut self, font: &'a FontResource) -> Result<u32, DviError> {
        let key = FontKey::from(font);
        if let Some(defined) = self.fonts.iter().find(|defined| defined.key == key) {
            return Ok(defined.number);
        }
        let number = u32::try_from(self.fonts.len()).expect("DVI font count exceeds u32");
        self.fnt_def(number, font)?;
        self.fonts.push(DefinedFont { number, key, font });
        Ok(number)
    }

    fn postamble(&mut self) -> Result<(), DviError> {
        let final_bop = self.previous_bop;
        let post_location = self.current_pointer()?;
        let mag = self.pages[0].job.mag;
        let total_pages = u16::try_from(self.pages.len()).map_err(|_| DviError::TooManyPages {
            pages: self.pages.len(),
        })?;

        self.u8(POST);
        self.i32(final_bop);
        self.i32(NUM);
        self.i32(DEN);
        self.i32(mag);
        self.i32(self.max_height_depth);
        self.i32(self.max_width);
        self.u16(self.max_stack_depth);
        self.u16(total_pages);

        for defined in self.fonts.clone() {
            self.fnt_def(defined.number, defined.font)?;
        }

        self.u8(POST_POST);
        self.i32(post_location);
        self.u8(ID_BYTE);
        for _ in 0..4 {
            self.u8(PADDING);
        }
        while !self.bytes.len().is_multiple_of(4) {
            self.u8(PADDING);
        }
        Ok(())
    }

    fn fnt_def(&mut self, number: u32, font: &FontResource) -> Result<(), DviError> {
        let name = limited_bytes("font name", &font.name)?;
        if name.is_empty() {
            return Err(DviError::EmptyFontName {
                font_id: font.font_id,
            });
        }
        match number {
            0..=0xff => {
                self.u8(FNT_DEF1);
                self.u8(number as u8);
            }
            0x100..=0xffff => {
                self.u8(FNT_DEF2);
                self.u16(number as u16);
            }
            0x1_0000..=0xff_ffff => {
                self.u8(FNT_DEF3);
                self.u24(number);
            }
            _ => {
                self.u8(FNT_DEF4);
                self.u32(number);
            }
        }
        self.u32(font.tfm_checksum);
        self.scaled(font.at_size);
        self.scaled(font.design_size);
        self.u8(0);
        self.u8(name.len() as u8);
        self.raw(name);
        Ok(())
    }

    fn current_pointer(&self) -> Result<i32, DviError> {
        i32::try_from(self.bytes.len()).map_err(|_| DviError::OffsetOverflow {
            offset: self.bytes.len(),
        })
    }

    fn raw(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn u24(&mut self, value: u32) {
        let bytes = value.to_be_bytes();
        self.bytes.extend_from_slice(&bytes[1..]);
    }

    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn i32(&mut self, value: i32) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn scaled(&mut self, value: Scaled) {
        self.i32(value.raw());
    }
}

#[derive(Clone, Debug)]
struct DefinedFont<'a> {
    number: u32,
    key: FontKey,
    font: &'a FontResource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FontKey {
    name: String,
    tfm_checksum: u32,
    design_size: Scaled,
    at_size: Scaled,
}

impl From<&FontResource> for FontKey {
    fn from(font: &FontResource) -> Self {
        Self {
            name: font.name.clone(),
            tfm_checksum: font.tfm_checksum,
            design_size: font.design_size,
            at_size: font.at_size,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PageExtent {
    height_depth: i32,
    width: i32,
}

fn page_extent(node: &PageNode) -> PageExtent {
    match node {
        PageNode::HList(box_node) | PageNode::VList(box_node) => box_extent(box_node),
        PageNode::Rule {
            width,
            height,
            depth,
        } => PageExtent {
            height_depth: optional_raw(*height) + optional_raw(*depth),
            width: optional_raw(*width),
        },
        PageNode::Char { .. }
        | PageNode::Lig { .. }
        | PageNode::Kern { .. }
        | PageNode::Glue { .. }
        | PageNode::Penalty(_)
        | PageNode::Unset
        | PageNode::WhatsitAnchor { .. }
        | PageNode::MathOn
        | PageNode::MathOff => PageExtent::default(),
    }
}

fn box_extent(box_node: &BoxNode) -> PageExtent {
    PageExtent {
        height_depth: box_node.height.raw() + box_node.depth.raw(),
        width: box_node.width.raw(),
    }
}

fn optional_raw(value: Option<Scaled>) -> i32 {
    value.map_or(0, Scaled::raw)
}

fn page_font(page: &PageArtifact, font_id: u32) -> Result<&FontResource, DviError> {
    page.fonts
        .iter()
        .find(|font| font.font_id == font_id)
        .ok_or(DviError::MissingFont { font_id })
}

fn limited_bytes<'a>(field: &'static str, value: &'a str) -> Result<&'a [u8], DviError> {
    let bytes = value.as_bytes();
    if bytes.len() > u8::MAX as usize {
        return Err(DviError::FieldTooLong {
            field,
            len: bytes.len(),
        });
    }
    Ok(bytes)
}
