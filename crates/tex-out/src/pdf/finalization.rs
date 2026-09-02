//! Complete detached input boundary for PDF finalization.
//!
//! Engine and host adapters resolve token lists, artifact storage, raw-object
//! files, and font acquisition before constructing this model. Finalization
//! consequently needs neither an engine state handle nor a host I/O callback.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use tex_arith::Scaled;
use tex_fonts::{
    FontSourceIdentity, PdfEncoding, PdfFontMapEntry, PdfPkFont, PdfPkFontRequest,
    PdfTrueTypeProgram, PdfType1Program, TfmFont, VfProgram,
};

use crate::{ContentHash, FontResource};

use super::PdfSerializationOptions;

#[cfg(test)]
mod tests;

/// Largest indirect object number permitted by pdfTeX and the PDF model.
pub const PDF_MAX_OBJECT_ID: u32 = i32::MAX as u32;

/// Every input needed by pure, deterministic PDF finalization.
///
/// Collections whose order affects pdfTeX compatibility remain `Vec`s. Lookup
/// tables use `BTreeMap`, making traversal independent of randomized hashing.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfFinalizationInput {
    pub document: PdfDocumentInput,
    pub pages: Vec<PdfCommittedPageInput>,
    pub forms: BTreeMap<u32, PdfFormInput>,
    pub fonts: BTreeMap<FontSourceIdentity, PdfFontInput>,
    pub virtual_fonts: BTreeMap<Vec<u8>, PdfVirtualFontInput>,
    pub images: BTreeMap<u32, PdfExternalImageInput>,
    pub raw_objects: Vec<PdfRawObjectInput>,
    pub navigation: PdfNavigationInput,
    pub allocation: PdfAllocationInput,
    pub limits: PdfFinalizationLimits,
}

/// Document-wide parameters and already-expanded metadata fragments.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfDocumentInput {
    pub version: (u8, u8),
    pub serialization: PdfSerializationOptions,
    pub decimal_digits: u8,
    pub draft_mode: bool,
    pub inclusion_copy_fonts: bool,
    pub unique_resource_names: bool,
    pub driver_dpi: u32,
    pub image_gamma: PdfImageGammaInput,
    pub pages_entries: Vec<u8>,
    pub form_omit_procset: i32,
    pub suppress_page_group_warning: bool,
    pub metadata: PdfDocumentMetadataInput,
}

/// Frozen raster conversion policy used by PNG finalization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfImageGammaInput {
    pub gamma: i32,
    pub image_gamma: i32,
    pub high_color: bool,
    pub apply_gamma: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PdfDocumentMetadataInput {
    pub include_info_dictionary: bool,
    pub include_dates: bool,
    pub creation_date: Vec<u8>,
    pub ptex_banner_key: Option<Vec<u8>>,
    pub ptex_banner: Vec<u8>,
    pub info_entries: Vec<u8>,
    pub catalog_entries: Vec<u8>,
    pub names_entries: Vec<u8>,
    pub trailer_entries: Vec<u8>,
    pub trailer_id: Vec<u8>,
    pub open_action: Option<PdfIndirectActionInput>,
}

/// One accepted page artifact and its checkpointed pdfTeX page policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfCommittedPageInput {
    pub artifact_hash: ContentHash,
    pub artifact_bytes: Arc<[u8]>,
    pub resources_object: u32,
    pub contents_object: u32,
    pub page_object: u32,
    /// Highest one-based engine font number allocated when the page shipped.
    pub font_watermark: u32,
    pub h_origin: Scaled,
    pub v_origin: Scaled,
    pub width: Scaled,
    pub height: Scaled,
    pub link_margin: Scaled,
    pub page_entries: Vec<u8>,
    pub resource_entries: Vec<u8>,
    pub omit_procset: i32,
    pub space_font_name: Vec<u8>,
}

/// One immutable `\pdfxform` artifact. References in page effects use
/// `object`; resource dictionaries use `resource`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfFormInput {
    pub object: u32,
    pub resource: u32,
    pub artifact_bytes: Arc<[u8]>,
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    pub entries: Vec<u8>,
    pub resource_entries: Vec<u8>,
    pub immediate: bool,
}

/// Output facts for one realized font identity.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfFontInput {
    pub artifact_resource: FontResource,
    pub resource_number: u32,
    pub object_number: u32,
    pub metrics: PdfFontMetricsInput,
    pub included_codes: BTreeSet<u8>,
    pub descriptor_entries: Vec<u8>,
    pub generate_to_unicode: bool,
    pub disable_builtin_to_unicode: bool,
    pub infer_builtin_glyph_unicode: bool,
    pub omit_charset: bool,
    pub glyph_to_unicode: BTreeMap<Vec<u8>, Vec<u32>>,
    pub map_entry: Option<PdfFontMapEntry>,
    pub encoding: Option<PdfEncoding>,
    pub program: PdfFontProgramInput,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfFontMetricsInput {
    pub widths: [Scaled; 256],
    pub heights: [Scaled; 256],
    pub depths: [Scaled; 256],
    pub x_height: Scaled,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PdfFontProgramInput {
    Resident,
    Type1(PdfType1Program),
    TrueType(PdfTrueTypeProgram),
    Pk {
        request: PdfPkFontRequest,
        font: PdfPkFont,
    },
}

/// Host-neutral resources for recursively lowering a virtual-font packet.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfVirtualFontInput {
    pub program: VfProgram,
    /// Exact validated TFM transports indexed by packet-local logical name.
    ///
    /// VF declarations select a size relative to the containing font. Keeping
    /// the transport, rather than only its design-size projection, lets the
    /// detached lowerer ask `tex-fonts` to construct the canonical instance at
    /// that declared size without consulting a host or live engine store.
    pub local_tfms: BTreeMap<Vec<u8>, PdfVirtualLocalTfmInput>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfVirtualLocalTfmInput {
    pub content_hash: tex_fonts::FontContentHash,
    pub bytes: Arc<[u8]>,
    /// Design-size validation receipt. The exact bytes remain authoritative
    /// for every packet-local sized instance.
    pub design_font: TfmFont,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfExternalImageInput {
    pub object: u32,
    /// Image-local resource identity (`pdf_ximage_count` in pdftex.web §1551).
    pub resource: u32,
    pub identity: ContentHash,
    pub metadata: PdfImageMetadataInput,
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    pub color_space_object: Option<u32>,
    pub mask_object: Option<u32>,
    pub bytes: tex_content::SharedBytes,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfImageMetadataInput {
    PdfPage {
        page_box: PdfPageBoxInput,
        rotation: PdfPageRotationInput,
        page: u32,
        total_pages: u32,
        has_page_group: bool,
        version: (u8, u8),
    },
    Raster {
        format: PdfRasterFormatInput,
        width: u32,
        height: u32,
        bits_per_component: u8,
        color_space: PdfRasterColorSpaceInput,
        alpha: bool,
        png_color_type: Option<u8>,
    },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfPageBoxInput {
    pub left: Scaled,
    pub bottom: Scaled,
    pub right: Scaled,
    pub top: Scaled,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfPageRotationInput {
    None,
    Clockwise90,
    UpsideDown,
    Clockwise270,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfRasterFormatInput {
    Jpeg,
    Png,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfRasterColorSpaceInput {
    Gray,
    Rgb,
    Cmyk,
}

/// A raw object after a host adapter has resolved `file` payloads.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfRawObjectInput {
    pub object: u32,
    pub payload: Option<PdfRawObjectPayloadInput>,
    pub immediate: bool,
    pub referenced: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfRawObjectPayloadInput {
    Value(Vec<u8>),
    Stream { entries: Vec<u8>, data: Arc<[u8]> },
}

/// Fully expanded action: no token-list or engine identifiers cross the
/// boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfActionInput {
    User(Vec<u8>),
    GoTo {
        file: Option<Vec<u8>>,
        structure: Option<PdfDestinationIdentityInput>,
        target: PdfActionTargetInput,
        new_window: Option<bool>,
    },
    Thread {
        file: Option<Vec<u8>>,
        structure: Option<PdfDestinationIdentityInput>,
        target: PdfActionTargetInput,
        new_window: Option<bool>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfIndirectActionInput {
    pub object: u32,
    pub target_object: Option<u32>,
    pub structure_object: Option<u32>,
    pub action: PdfActionInput,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfActionTargetInput {
    Page { number: u32, view: Vec<u8> },
    Destination(PdfDestinationIdentityInput),
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PdfDestinationIdentityInput {
    Name(Vec<u8>),
    Number(u32),
    Raw(Vec<u8>),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PdfNavigationInput {
    pub annotations: Vec<PdfAnnotationInput>,
    pub links: Vec<PdfLinkInput>,
    pub destinations: Vec<PdfDestinationInput>,
    pub structure_destinations: Vec<PdfDestinationInput>,
    pub outlines: Vec<PdfOutlineInput>,
    pub threads: Vec<PdfThreadInput>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfAnnotationDimensionsInput {
    pub width: Option<Scaled>,
    pub height: Option<Scaled>,
    pub depth: Option<Scaled>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfAnnotationInput {
    pub object: u32,
    /// `None` preserves a reserved, uninitialized `useobjnum` slot.
    pub data: Option<(PdfAnnotationDimensionsInput, Vec<u8>)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfLinkInput {
    pub object: u32,
    pub dimensions: PdfAnnotationDimensionsInput,
    pub entries: Vec<u8>,
    pub action: PdfActionInput,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfDestinationInput {
    pub identity: PdfDestinationIdentityInput,
    pub object: u32,
    pub structure_object: Option<u32>,
    pub defined: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfOutlineInput {
    pub action_object: u32,
    pub item_object: u32,
    pub title_object: u32,
    pub entries: Vec<u8>,
    pub action: PdfActionInput,
    pub count: i32,
    pub title: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfThreadInput {
    pub identity: PdfDestinationIdentityInput,
    pub object: u32,
    pub beads: Vec<PdfThreadBeadInput>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfThreadBeadInput {
    pub bead_object: u32,
    pub rectangle_object: u32,
}

/// Existing reservations plus the first ID available to finalization-owned
/// deterministic allocations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfAllocationInput {
    pub document: PdfReservedDocumentObjects,
    pub next_object: u32,
}

impl PdfAllocationInput {
    /// Starts the one monotonic allocator used after engine reservations.
    #[must_use]
    pub const fn allocator(self, maximum: u32) -> PdfObjectAllocator {
        PdfObjectAllocator {
            next: self.next_object,
            maximum,
        }
    }
}

/// Deterministic monotonic allocation for objects discovered during lowering.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfObjectAllocator {
    next: u32,
    maximum: u32,
}

impl PdfObjectAllocator {
    #[must_use]
    pub const fn next_object(self) -> u32 {
        self.next
    }

    pub fn allocate(&mut self) -> Result<u32, PdfObjectAllocationError> {
        self.allocate_many(1)
    }

    /// Reserves a consecutive range and returns its first object number.
    pub fn allocate_many(&mut self, count: u32) -> Result<u32, PdfObjectAllocationError> {
        if count == 0 {
            return Ok(self.next);
        }
        let last = self
            .next
            .checked_add(count - 1)
            .filter(|last| *last <= self.maximum)
            .ok_or(PdfObjectAllocationError {
                next: self.next,
                count,
                maximum: self.maximum,
            })?;
        let first = self.next;
        self.next = last
            .checked_add(1)
            .unwrap_or(self.maximum.saturating_add(1));
        Ok(first)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfObjectAllocationError {
    pub next: u32,
    pub count: u32,
    pub maximum: u32,
}

impl std::fmt::Display for PdfObjectAllocationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "cannot reserve {} PDF objects starting at {}; maximum object is {}",
            self.count, self.next, self.maximum
        )
    }
}

impl std::error::Error for PdfObjectAllocationError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfReservedDocumentObjects {
    pub pages: u32,
    pub names: Option<u32>,
    pub catalog: u32,
    pub info: Option<u32>,
}

/// Resource and traversal budgets are data, not hidden process policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfFinalizationLimits {
    pub max_object_id: u32,
    pub max_form_depth: usize,
    pub max_form_work: usize,
    pub max_virtual_font_recursion: usize,
    pub max_virtual_font_stack_depth: usize,
    pub max_virtual_font_packet_commands: usize,
    pub max_virtual_font_output_operations: usize,
    pub max_virtual_font_special_bytes: usize,
    pub max_imported_image_stream_bytes: usize,
    pub max_imported_resource_objects: usize,
    pub max_imported_resource_values: usize,
    pub max_imported_resource_depth: usize,
    pub max_imported_resource_stream_bytes: usize,
}

impl Default for PdfFinalizationLimits {
    fn default() -> Self {
        Self {
            max_object_id: PDF_MAX_OBJECT_ID,
            max_form_depth: 256,
            max_form_work: 1_000_000,
            max_virtual_font_recursion: tex_fonts::PDFTEX_VF_MAX_RECURSION,
            max_virtual_font_stack_depth: 100,
            max_virtual_font_packet_commands: 1_000_000,
            max_virtual_font_output_operations: 1_000_000,
            max_virtual_font_special_bytes: 8 * 1024 * 1024,
            max_imported_image_stream_bytes: 256 * 1024 * 1024,
            max_imported_resource_objects: 100_000,
            max_imported_resource_values: 1_000_000,
            max_imported_resource_depth: 256,
            max_imported_resource_stream_bytes: 1 << 30,
        }
    }
}
