//! Checkpointed pdfTeX document allocation ledger.

use crate::ContentHash;
use crate::ids::{FontId, TokenListId};
use crate::scaled::Scaled;
use crate::state_hash::{StateHashFragment, StateHasher};
use std::collections::BTreeMap;

const PDF_STATE_DOMAIN: u64 = 0x7064_665f_7374_6174;
const PDF_PAGE_DOMAIN: u64 = 0x7064_665f_7061_6765;
const PDF_FONT_DOMAIN: u64 = 0x7064_665f_666f_6e74;
pub const PDF_CATALOG_OBJECT_ID: u32 = 1;
pub const PDF_PAGES_OBJECT_ID: u32 = 2;
const FIRST_DYNAMIC_OBJECT: u32 = 3;
const OBJECTS_PER_PAGE: u32 = 3;
const MAX_OBJECT_ID: u32 = i32::MAX as u32;

/// The PDF object ledger cannot reserve another indirect object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfObjectCapacityError;

impl std::fmt::Display for PdfObjectCapacityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PDF object number exceeds 2147483647")
    }
}

impl std::error::Error for PdfObjectCapacityError {}

/// Stable page resource and indirect-object identities for one PDF font.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfFontResourceRecord {
    font: FontId,
    source_identity: tex_fonts::FontSourceIdentity,
    resource_number: u32,
    object_number: u32,
    tfm_content_hash: [u8; 32],
    program_identity: Option<[u8; 32]>,
}

impl PdfFontResourceRecord {
    #[must_use]
    pub const fn font(self) -> FontId {
        self.font
    }
    #[must_use]
    pub const fn resource_number(self) -> u32 {
        self.resource_number
    }
    #[must_use]
    pub const fn object_number(self) -> u32 {
        self.object_number
    }
}

/// A host-neutral font-map mutation recorded by a pdfTeX action primitive.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfFontMapOperation {
    File(tex_fonts::PdfFontMapFile),
    Line(tex_fonts::PdfFontMapEntry),
}

/// One validated `\pdfglyphtounicode` mapping. A `tfm:` prefix scopes the
/// mapping to one TeX metric name; otherwise it is global across fonts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfGlyphToUnicode {
    pub tfm_name: Option<Vec<u8>>,
    pub glyph_name: Vec<u8>,
    pub unicode: Vec<u32>,
}

/// An append-only font-output mutation. The log makes snapshots cheap and
/// ensures rollback discards the exact suffix produced after a checkpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
enum PdfFontOperation {
    Map(PdfFontMapOperation),
    Attribute {
        font: FontId,
        bytes: Vec<u8>,
    },
    IncludeChars {
        font: FontId,
        chars: Vec<u8>,
    },
    GlyphToUnicode(PdfGlyphToUnicode),
    NoBuiltinToUnicode {
        font: FontId,
    },
    Type1Program {
        logical_name: Vec<u8>,
        program: tex_fonts::PdfType1Program,
    },
    Encoding {
        logical_name: Vec<u8>,
        encoding: tex_fonts::PdfEncoding,
    },
    TrueTypeProgram {
        logical_name: Vec<u8>,
        program: tex_fonts::PdfTrueTypeProgram,
    },
}

/// Live pdfTeX microtype and font-output controls.
///
/// The raw values remain ordinary grouped integer parameters in `Env`; this
/// projection gives downstream paragraph and font backends one typed,
/// host-neutral contract without introducing shadow state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfFontConfiguration {
    pub adjust_spacing: i32,
    pub protrude_chars: i32,
    pub tracing_fonts: i32,
    pub adjust_interword_glue: i32,
    pub prepend_kern: i32,
    pub append_kern: i32,
    pub generate_to_unicode: i32,
    pub pk_resolution: i32,
    pub omit_charset: i32,
}

impl PdfFontConfiguration {
    /// Enables expansion while final line boxes are packed.
    #[must_use]
    pub const fn adjusts_spacing(self) -> bool {
        self.adjust_spacing > 0
    }

    /// Enables expansion-aware line-breaking passes 7 and 8.
    #[must_use]
    pub const fn adjusts_line_breaking(self) -> bool {
        self.adjust_spacing > 1
    }

    /// Enables margin-kern insertion in materialized lines.
    #[must_use]
    pub const fn protrudes_chars(self) -> bool {
        self.protrude_chars > 0
    }

    /// Enables protrusion-aware line-breaking width calculations.
    #[must_use]
    pub const fn protrudes_during_line_breaking(self) -> bool {
        self.protrude_chars > 1
    }

    #[must_use]
    pub const fn traces_fonts(self) -> bool {
        self.tracing_fonts > 0
    }

    #[must_use]
    pub const fn adjusts_interword_glue(self) -> bool {
        self.adjust_interword_glue > 0
    }

    #[must_use]
    pub const fn prepends_kerns(self) -> bool {
        self.prepend_kern > 0
    }

    #[must_use]
    pub const fn appends_kerns(self) -> bool {
        self.append_kern > 0
    }

    #[must_use]
    pub const fn generates_to_unicode(self) -> bool {
        self.generate_to_unicode > 0
    }

    #[must_use]
    pub const fn omits_charset(self) -> bool {
        self.omit_charset != 0
    }

    /// Resolves pdfTeX's zero sentinel against driver configuration, then
    /// applies the engine's `72..=8000` DPI output-time clamp.
    #[must_use]
    pub const fn resolved_pk_resolution(self, driver_dpi: i32) -> i32 {
        let dpi = if self.pk_resolution == 0 {
            driver_dpi
        } else {
            self.pk_resolution
        };
        if dpi < 72 {
            72
        } else if dpi > 8_000 {
            8_000
        } else {
            dpi
        }
    }
}

/// pdfTeX output controls frozen by the first shipped page.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfOutputParameters {
    pub output: i32,
    pub major_version: i32,
    pub minor_version: i32,
    pub compress_level: i32,
    pub object_compress_level: i32,
    pub decimal_digits: i32,
    /// Gamma controls fixed when PDF output is initialized.
    pub gamma: i32,
    pub image_gamma: i32,
    pub image_hicolor: i32,
    pub image_apply_gamma: i32,
    /// Raw draft value fixed by the first output write; positive enables it.
    pub draft_mode: i32,
    pub inclusion_copy_fonts: i32,
    /// PK resolution remains zero until a driver supplies its configured DPI.
    pub pk_resolution: i32,
    /// Normalized boolean controlling document-wide resource-name prefixes.
    pub unique_resource_names: i32,
}

impl PdfOutputParameters {
    /// Applies pdfTeX's first-PDF-write recovery and clamping policy.
    #[must_use]
    pub fn normalized(self) -> Self {
        let major_version = self.major_version.max(1);
        let minor_version = if (0..=9).contains(&self.minor_version) {
            self.minor_version
        } else {
            4
        };
        let mut object_compress_level = self.object_compress_level.clamp(0, 3);
        if major_version == 1 && minor_version < 5 {
            object_compress_level = 0;
        }
        Self {
            major_version,
            minor_version,
            object_compress_level,
            decimal_digits: self.decimal_digits.clamp(0, 4),
            gamma: self.gamma.clamp(0, 1_000_000),
            image_gamma: self.image_gamma.clamp(0, 1_000_000),
            image_hicolor: self.image_hicolor.clamp(0, 1),
            image_apply_gamma: self.image_apply_gamma.clamp(0, 1),
            inclusion_copy_fonts: self.inclusion_copy_fonts.clamp(0, 1),
            pk_resolution: if self.pk_resolution == 0 {
                0
            } else {
                self.pk_resolution.clamp(72, 8_000)
            },
            unique_resource_names: i32::from(self.unique_resource_names > 0),
            ..self
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PdfTokenParameter {
    pub(crate) tokens: TokenListId,
    pub(crate) semantic_id: u64,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PdfPageParameters {
    pub(crate) h_origin: Scaled,
    pub(crate) v_origin: Scaled,
    pub(crate) width: Scaled,
    pub(crate) height: Scaled,
    pub(crate) page_attr: PdfTokenParameter,
    pub(crate) resources: PdfTokenParameter,
    /// Raw `\pdfomitprocset` value captured when this page is shipped.
    pub(crate) omit_procset: i32,
}

/// Stable object identities assigned to one committed PDF page.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfPageRecord {
    artifact: ContentHash,
    resources_object: u32,
    contents_object: u32,
    page_object: u32,
    parameters: PdfPageParameters,
}

impl PdfPageRecord {
    #[must_use]
    pub const fn artifact(self) -> ContentHash {
        self.artifact
    }
    #[must_use]
    pub const fn resources_object(self) -> u32 {
        self.resources_object
    }
    #[must_use]
    pub const fn contents_object(self) -> u32 {
        self.contents_object
    }
    #[must_use]
    pub const fn page_object(self) -> u32 {
        self.page_object
    }
    #[must_use]
    pub const fn h_origin(self) -> Scaled {
        self.parameters.h_origin
    }
    #[must_use]
    pub const fn v_origin(self) -> Scaled {
        self.parameters.v_origin
    }
    #[must_use]
    pub const fn width(self) -> Scaled {
        self.parameters.width
    }
    #[must_use]
    pub const fn height(self) -> Scaled {
        self.parameters.height
    }
    #[must_use]
    pub const fn page_attr(self) -> TokenListId {
        self.parameters.page_attr.tokens
    }
    #[must_use]
    pub const fn resources(self) -> TokenListId {
        self.parameters.resources.tokens
    }
    #[must_use]
    pub const fn omit_procset(self) -> i32 {
        self.parameters.omit_procset
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PdfStateCursor {
    enabled: bool,
    next_object: u32,
    page_count: usize,
    output_parameters: Option<PdfOutputParameters>,
    pk_mode: Option<PdfTokenParameter>,
    font_operation_count: usize,
    font_resource_count: usize,
    fingerprint: u64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PdfStateSnapshot(PdfStateCursor);

/// Live append-only PDF allocation state owned by one Universe timeline.
#[derive(Clone, Debug)]
pub(crate) struct PdfState {
    enabled: bool,
    next_object: u32,
    pages: Vec<PdfPageRecord>,
    output_parameters: Option<PdfOutputParameters>,
    pk_mode: Option<PdfTokenParameter>,
    font_operations: Vec<PdfFontOperation>,
    font_resources: Vec<PdfFontResourceRecord>,
    fingerprint: u64,
}

impl Default for PdfState {
    fn default() -> Self {
        Self {
            enabled: false,
            next_object: FIRST_DYNAMIC_OBJECT,
            pages: Vec::new(),
            output_parameters: None,
            pk_mode: None,
            font_operations: Vec::new(),
            font_resources: Vec::new(),
            fingerprint: base_fingerprint(false),
        }
    }
}

impl PdfState {
    pub(crate) fn enable(&mut self) {
        if self.enabled {
            return;
        }
        debug_assert!(self.pages.is_empty());
        self.enabled = true;
        self.next_object = FIRST_DYNAMIC_OBJECT;
        self.fingerprint = base_fingerprint(true);
    }

    #[must_use]
    pub(crate) const fn enabled(&self) -> bool {
        self.enabled
    }
    #[must_use]
    pub(crate) fn pages(&self) -> &[PdfPageRecord] {
        &self.pages
    }
    #[must_use]
    pub(crate) const fn next_object(&self) -> u32 {
        self.next_object
    }
    #[must_use]
    pub(crate) const fn is_format_empty(&self) -> bool {
        self.pages.is_empty()
            && self.next_object == FIRST_DYNAMIC_OBJECT
            && self.output_parameters.is_none()
            && self.pk_mode.is_none()
            && self.font_operations.is_empty()
            && self.font_resources.is_empty()
    }

    pub(crate) fn ensure_page_capacity(&self, parameters: PdfOutputParameters) -> Result<(), ()> {
        if !self.enabled || self.output_parameters.unwrap_or(parameters).output <= 0 {
            return Ok(());
        }
        let last = self
            .next_object
            .checked_add(OBJECTS_PER_PAGE - 1)
            .ok_or(())?;
        (last <= MAX_OBJECT_ID).then_some(()).ok_or(())
    }

    pub(crate) fn commit_page(
        &mut self,
        artifact: ContentHash,
        output: PdfOutputParameters,
        page: PdfPageParameters,
        pk_mode: PdfTokenParameter,
    ) {
        if !self.enabled {
            return;
        }
        let output = match self.output_parameters {
            Some(parameters) => parameters,
            None => {
                self.output_parameters = Some(output);
                self.fingerprint = freeze_fingerprint(self.fingerprint, output);
                output
            }
        };
        if output.output <= 0 {
            return;
        }
        if self.pk_mode.is_none() {
            self.pk_mode = Some(pk_mode);
            self.fingerprint = freeze_pk_mode_fingerprint(self.fingerprint, pk_mode);
        }
        self.ensure_page_capacity(output)
            .expect("PDF page object capacity was preflighted");
        let record = PdfPageRecord {
            artifact,
            resources_object: self.next_object,
            contents_object: self.next_object + 1,
            page_object: self.next_object + 2,
            parameters: page,
        };
        self.next_object += OBJECTS_PER_PAGE;
        self.pages.push(record);
        self.fingerprint = append_fingerprint(self.fingerprint, record);
    }

    #[must_use]
    pub(crate) const fn output_parameters(&self) -> Option<PdfOutputParameters> {
        self.output_parameters
    }

    #[must_use]
    pub(crate) const fn pk_mode(&self) -> Option<TokenListId> {
        match self.pk_mode {
            Some(mode) => Some(mode.tokens),
            None => None,
        }
    }

    pub(crate) fn push_font_map(&mut self, operation: PdfFontMapOperation) {
        self.push_font_operation(PdfFontOperation::Map(operation));
    }

    pub(crate) fn set_font_attribute(&mut self, font: FontId, bytes: Vec<u8>) {
        self.push_font_operation(PdfFontOperation::Attribute { font, bytes });
    }

    pub(crate) fn include_font_chars(&mut self, font: FontId, chars: Vec<u8>) {
        self.push_font_operation(PdfFontOperation::IncludeChars { font, chars });
    }

    pub(crate) fn set_glyph_to_unicode(&mut self, mapping: PdfGlyphToUnicode) {
        self.push_font_operation(PdfFontOperation::GlyphToUnicode(mapping));
    }

    pub(crate) fn disable_builtin_to_unicode(&mut self, font: FontId) {
        self.push_font_operation(PdfFontOperation::NoBuiltinToUnicode { font });
    }

    pub(crate) fn provide_type1_program(
        &mut self,
        logical_name: Vec<u8>,
        program: tex_fonts::PdfType1Program,
    ) {
        self.push_font_operation(PdfFontOperation::Type1Program {
            logical_name,
            program,
        });
    }

    pub(crate) fn ensure_font_resource(
        &mut self,
        font: FontId,
        source_identity: tex_fonts::FontSourceIdentity,
        tfm_content_hash: [u8; 32],
        program_identity: Option<[u8; 32]>,
    ) -> Result<PdfFontResourceRecord, PdfObjectCapacityError> {
        if let Some(record) = self
            .font_resources
            .iter()
            .copied()
            .find(|record| record.font == font)
        {
            return Ok(record);
        }
        if let Some(record) = self.font_resources.iter().copied().find(|record| {
            record.tfm_content_hash == tfm_content_hash
                && record.program_identity == program_identity
        }) {
            let alias = PdfFontResourceRecord {
                font,
                source_identity,
                ..record
            };
            self.font_resources.push(alias);
            self.fingerprint = append_font_resource_fingerprint(self.fingerprint, alias);
            return Ok(alias);
        }
        if self.next_object > MAX_OBJECT_ID {
            return Err(PdfObjectCapacityError);
        }
        let record = PdfFontResourceRecord {
            font,
            source_identity,
            resource_number: font.raw(),
            object_number: self.next_object,
            tfm_content_hash,
            program_identity,
        };
        self.next_object += 1;
        self.font_resources.push(record);
        self.fingerprint = append_font_resource_fingerprint(self.fingerprint, record);
        Ok(record)
    }

    pub(crate) fn font_resource(&self, font: FontId) -> Option<PdfFontResourceRecord> {
        self.font_resources
            .iter()
            .copied()
            .find(|record| record.font == font)
    }

    pub(crate) fn font_resource_by_identity(
        &self,
        identity: tex_fonts::FontSourceIdentity,
    ) -> Option<PdfFontResourceRecord> {
        self.font_resources
            .iter()
            .copied()
            .find(|record| record.source_identity == identity)
    }

    pub(crate) fn font_resources(&self) -> impl Iterator<Item = PdfFontResourceRecord> + '_ {
        self.font_resources
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(index, record)| {
                (!self.font_resources[..index]
                    .iter()
                    .any(|prior| prior.object_number == record.object_number))
                .then_some(record)
            })
    }

    #[must_use]
    pub(crate) fn type1_program(&self, logical_name: &[u8]) -> Option<&tex_fonts::PdfType1Program> {
        self.font_operations
            .iter()
            .rev()
            .find_map(|operation| match operation {
                PdfFontOperation::Type1Program {
                    logical_name: candidate,
                    program,
                } if candidate == logical_name => Some(program),
                _ => None,
            })
    }

    pub(crate) fn provide_encoding(
        &mut self,
        logical_name: Vec<u8>,
        encoding: tex_fonts::PdfEncoding,
    ) {
        self.push_font_operation(PdfFontOperation::Encoding {
            logical_name,
            encoding,
        });
    }

    pub(crate) fn encoding(&self, logical_name: &[u8]) -> Option<&tex_fonts::PdfEncoding> {
        self.font_operations
            .iter()
            .rev()
            .find_map(|operation| match operation {
                PdfFontOperation::Encoding {
                    logical_name: candidate,
                    encoding,
                } if candidate == logical_name => Some(encoding),
                _ => None,
            })
    }

    pub(crate) fn provide_truetype_program(
        &mut self,
        logical_name: Vec<u8>,
        program: tex_fonts::PdfTrueTypeProgram,
    ) {
        self.push_font_operation(PdfFontOperation::TrueTypeProgram {
            logical_name,
            program,
        });
    }

    pub(crate) fn truetype_program(
        &self,
        logical_name: &[u8],
    ) -> Option<&tex_fonts::PdfTrueTypeProgram> {
        self.font_operations
            .iter()
            .rev()
            .find_map(|operation| match operation {
                PdfFontOperation::TrueTypeProgram {
                    logical_name: candidate,
                    program,
                } if candidate == logical_name => Some(program),
                _ => None,
            })
    }

    fn push_font_operation(&mut self, operation: PdfFontOperation) {
        self.fingerprint = append_font_fingerprint(self.fingerprint, &operation);
        self.font_operations.push(operation);
    }

    pub(crate) fn font_maps(&self) -> impl Iterator<Item = &PdfFontMapOperation> {
        self.font_operations
            .iter()
            .filter_map(|operation| match operation {
                PdfFontOperation::Map(map) => Some(map),
                PdfFontOperation::Attribute { .. }
                | PdfFontOperation::IncludeChars { .. }
                | PdfFontOperation::GlyphToUnicode(_)
                | PdfFontOperation::NoBuiltinToUnicode { .. }
                | PdfFontOperation::Type1Program { .. }
                | PdfFontOperation::Encoding { .. }
                | PdfFontOperation::TrueTypeProgram { .. } => None,
            })
    }

    #[must_use]
    pub(crate) fn resolved_font_map_lines(&self) -> Vec<tex_fonts::PdfFontMapEntry> {
        self.resolve_font_map_lines().0.into_values().collect()
    }

    #[must_use]
    pub(crate) fn font_map_duplicate_names(&self) -> Vec<Vec<u8>> {
        self.resolve_font_map_lines().1
    }

    fn resolve_font_map_lines(
        &self,
    ) -> (BTreeMap<Vec<u8>, tex_fonts::PdfFontMapEntry>, Vec<Vec<u8>>) {
        let mut entries = BTreeMap::new();
        let mut duplicates = Vec::new();
        for operation in self.font_maps() {
            let PdfFontMapOperation::Line(entry) = operation else {
                continue;
            };
            match entry.directive {
                tex_fonts::PdfFontMapDirective::Default | tex_fonts::PdfFontMapDirective::Add => {
                    if entries.contains_key(&entry.tex_name) {
                        duplicates.push(entry.tex_name.clone());
                    } else {
                        entries.insert(entry.tex_name.clone(), entry.clone());
                    }
                }
                tex_fonts::PdfFontMapDirective::Replace => {
                    entries.insert(entry.tex_name.clone(), entry.clone());
                }
                tex_fonts::PdfFontMapDirective::Remove => {
                    entries.remove(&entry.tex_name);
                }
            }
        }
        (entries, duplicates)
    }

    #[must_use]
    pub(crate) fn font_attribute(&self, font: FontId) -> &[u8] {
        self.font_operations
            .iter()
            .rev()
            .find_map(|operation| match operation {
                PdfFontOperation::Attribute {
                    font: candidate,
                    bytes,
                } if *candidate == font => Some(bytes.as_slice()),
                _ => None,
            })
            .unwrap_or_default()
    }

    #[must_use]
    pub(crate) fn included_font_chars(&self, font: FontId) -> Vec<u8> {
        let mut included = [false; 256];
        for operation in &self.font_operations {
            if let PdfFontOperation::IncludeChars {
                font: candidate,
                chars,
            } = operation
                && *candidate == font
            {
                for &character in chars {
                    included[usize::from(character)] = true;
                }
            }
        }
        included
            .into_iter()
            .enumerate()
            .filter_map(|(character, present)| present.then_some(character as u8))
            .collect()
    }

    #[must_use]
    pub(crate) fn builtin_to_unicode_disabled(&self, font: FontId) -> bool {
        self.font_operations.iter().any(|operation| {
            matches!(operation, PdfFontOperation::NoBuiltinToUnicode { font: candidate } if *candidate == font)
        })
    }

    #[must_use]
    pub(crate) fn has_glyph_to_unicode_mappings(&self) -> bool {
        self.font_operations
            .iter()
            .any(|operation| matches!(operation, PdfFontOperation::GlyphToUnicode(_)))
    }

    #[must_use]
    pub(crate) fn glyph_to_unicode(&self, tfm_name: &[u8], glyph_name: &[u8]) -> Option<&[u32]> {
        let glyph_name = glyph_name
            .split(|byte| *byte == b'.')
            .next()
            .unwrap_or(glyph_name);
        for scoped in [true, false] {
            if let Some(mapping) = self.font_operations.iter().rev().find_map(|operation| {
                let PdfFontOperation::GlyphToUnicode(mapping) = operation else {
                    return None;
                };
                let scope_matches = if scoped {
                    mapping.tfm_name.as_deref() == Some(tfm_name)
                } else {
                    mapping.tfm_name.is_none()
                };
                (scope_matches && mapping.glyph_name == glyph_name).then_some(mapping)
            }) {
                return Some(&mapping.unicode);
            }
        }
        None
    }

    #[must_use]
    pub(crate) const fn cursor(&self) -> PdfStateCursor {
        PdfStateCursor {
            enabled: self.enabled,
            next_object: self.next_object,
            page_count: self.pages.len(),
            output_parameters: self.output_parameters,
            pk_mode: self.pk_mode,
            font_operation_count: self.font_operations.len(),
            font_resource_count: self.font_resources.len(),
            fingerprint: self.fingerprint,
        }
    }
    #[must_use]
    pub(crate) const fn snapshot(&self) -> PdfStateSnapshot {
        PdfStateSnapshot(self.cursor())
    }

    pub(crate) fn rollback(&mut self, snapshot: PdfStateSnapshot) {
        let cursor = snapshot.0;
        assert!(
            cursor.page_count <= self.pages.len(),
            "PDF snapshot suffix was discarded"
        );
        self.pages.truncate(cursor.page_count);
        self.enabled = cursor.enabled;
        self.next_object = cursor.next_object;
        self.output_parameters = cursor.output_parameters;
        self.pk_mode = cursor.pk_mode;
        self.font_operations.truncate(cursor.font_operation_count);
        self.font_resources.truncate(cursor.font_resource_count);
        self.fingerprint = cursor.fingerprint;
    }

    #[must_use]
    pub(crate) fn hash_fragment(&self) -> StateHashFragment {
        let cursor = self.cursor();
        StateHashFragment::from_builder(PDF_STATE_DOMAIN, |hasher| {
            hasher.bool(cursor.enabled);
            hasher.u32(cursor.next_object);
            hasher.usize(cursor.page_count);
            hash_output_parameters(hasher, cursor.output_parameters);
            hasher.usize(cursor.font_operation_count);
            hasher.usize(cursor.font_resource_count);
            hasher.u64(cursor.fingerprint);
        })
    }
}

fn append_font_resource_fingerprint(previous: u64, record: PdfFontResourceRecord) -> u64 {
    let mut hasher = StateHasher::new(PDF_FONT_DOMAIN);
    hasher.u64(previous);
    hasher.tag(5);
    hasher.u32(record.font.raw());
    hasher.bytes(&record.source_identity.bytes());
    hasher.u32(record.resource_number);
    hasher.u32(record.object_number);
    hasher.bytes(&record.tfm_content_hash);
    hasher.bool(record.program_identity.is_some());
    if let Some(identity) = record.program_identity {
        hasher.bytes(&identity);
    }
    hasher.finish()
}

fn append_font_fingerprint(previous: u64, operation: &PdfFontOperation) -> u64 {
    let mut hasher = StateHasher::new(PDF_FONT_DOMAIN);
    hasher.u64(previous);
    match operation {
        PdfFontOperation::Map(PdfFontMapOperation::File(file)) => {
            hasher.tag(0);
            hasher.tag(file.directive as u8);
            hasher.bytes(&file.logical_name);
        }
        PdfFontOperation::Map(PdfFontMapOperation::Line(line)) => {
            hasher.tag(1);
            hasher.tag(line.directive as u8);
            hasher.bytes(&line.tex_name);
            hasher.bool(line.postscript_name.is_some());
            if let Some(name) = &line.postscript_name {
                hasher.bytes(name);
            }
            for instruction in &line.special_instructions {
                hasher.bytes(instruction);
            }
            for encoding in &line.encoding_files {
                hasher.bytes(encoding);
            }
            hasher.bool(line.font_file.is_some());
            if let Some(file) = &line.font_file {
                hasher.bytes(file);
            }
            hasher.tag(line.program as u8);
        }
        PdfFontOperation::Attribute { font, bytes } => {
            hasher.tag(2);
            hasher.u32(font.raw());
            hasher.bytes(bytes);
        }
        PdfFontOperation::IncludeChars { font, chars } => {
            hasher.tag(3);
            hasher.u32(font.raw());
            hasher.bytes(chars);
        }
        PdfFontOperation::GlyphToUnicode(mapping) => {
            hasher.tag(8);
            hasher.bool(mapping.tfm_name.is_some());
            if let Some(name) = &mapping.tfm_name {
                hasher.bytes(name);
            }
            hasher.bytes(&mapping.glyph_name);
            for value in &mapping.unicode {
                hasher.u32(*value);
            }
        }
        PdfFontOperation::NoBuiltinToUnicode { font } => {
            hasher.tag(9);
            hasher.u32(font.raw());
        }
        PdfFontOperation::Type1Program {
            logical_name,
            program,
        } => {
            hasher.tag(4);
            hasher.bytes(logical_name);
            hasher.bytes(&program.identity().bytes());
        }
        PdfFontOperation::Encoding {
            logical_name,
            encoding,
        } => {
            hasher.tag(6);
            hasher.bytes(logical_name);
            hasher.bytes(encoding.name());
            for name in encoding.glyph_names() {
                hasher.bytes(name);
            }
        }
        PdfFontOperation::TrueTypeProgram {
            logical_name,
            program,
        } => {
            hasher.tag(7);
            hasher.bytes(logical_name);
            hasher.bytes(&program.identity().bytes());
        }
    }
    hasher.finish()
}

fn base_fingerprint(enabled: bool) -> u64 {
    let mut hasher = StateHasher::new(PDF_STATE_DOMAIN);
    hasher.bool(enabled);
    hasher.u32(FIRST_DYNAMIC_OBJECT);
    hasher.finish()
}

fn freeze_fingerprint(previous: u64, parameters: PdfOutputParameters) -> u64 {
    let mut hasher = StateHasher::new(PDF_PAGE_DOMAIN);
    hasher.u64(previous);
    hash_output_parameters(&mut hasher, Some(parameters));
    hasher.finish()
}

fn append_fingerprint(previous: u64, record: PdfPageRecord) -> u64 {
    let mut hasher = StateHasher::new(PDF_PAGE_DOMAIN);
    hasher.u64(previous);
    hasher.bytes(&record.artifact.bytes());
    hasher.u32(record.resources_object);
    hasher.u32(record.contents_object);
    hasher.u32(record.page_object);
    hasher.i32(record.parameters.h_origin.raw());
    hasher.i32(record.parameters.v_origin.raw());
    hasher.i32(record.parameters.width.raw());
    hasher.i32(record.parameters.height.raw());
    hasher.u64(record.parameters.page_attr.semantic_id);
    hasher.u64(record.parameters.resources.semantic_id);
    hasher.i32(record.parameters.omit_procset);
    hasher.finish()
}

fn freeze_pk_mode_fingerprint(previous: u64, mode: PdfTokenParameter) -> u64 {
    let mut hasher = StateHasher::new(PDF_PAGE_DOMAIN);
    hasher.u64(previous);
    hasher.u64(mode.semantic_id);
    hasher.finish()
}

fn hash_output_parameters(hasher: &mut StateHasher, parameters: Option<PdfOutputParameters>) {
    hasher.bool(parameters.is_some());
    if let Some(parameters) = parameters {
        hasher.i32(parameters.output);
        hasher.i32(parameters.major_version);
        hasher.i32(parameters.minor_version);
        hasher.i32(parameters.compress_level);
        hasher.i32(parameters.object_compress_level);
        hasher.i32(parameters.decimal_digits);
        hasher.i32(parameters.gamma);
        hasher.i32(parameters.image_gamma);
        hasher.i32(parameters.image_hicolor);
        hasher.i32(parameters.image_apply_gamma);
        hasher.i32(parameters.draft_mode);
        hasher.i32(parameters.inclusion_copy_fonts);
        hasher.i32(parameters.pk_resolution);
        hasher.i32(parameters.unique_resource_names);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_configuration_preserves_pdftex_thresholds_and_pk_resolution() {
        let mut configuration = PdfFontConfiguration {
            adjust_spacing: 1,
            protrude_chars: 1,
            tracing_fonts: 1,
            adjust_interword_glue: 1,
            prepend_kern: 1,
            append_kern: 1,
            generate_to_unicode: 1,
            pk_resolution: 0,
            omit_charset: 1,
        };
        assert!(configuration.adjusts_spacing());
        assert!(!configuration.adjusts_line_breaking());
        assert!(configuration.protrudes_chars());
        assert!(!configuration.protrudes_during_line_breaking());
        assert!(configuration.traces_fonts());
        assert!(configuration.adjusts_interword_glue());
        assert!(configuration.prepends_kerns());
        assert!(configuration.appends_kerns());
        assert!(configuration.generates_to_unicode());
        assert!(configuration.omits_charset());
        assert_eq!(configuration.resolved_pk_resolution(600), 600);

        configuration.adjust_spacing = 2;
        configuration.protrude_chars = 2;
        configuration.pk_resolution = 9_000;
        assert!(configuration.adjusts_line_breaking());
        assert!(configuration.protrudes_during_line_breaking());
        assert_eq!(configuration.resolved_pk_resolution(600), 8_000);

        configuration.pk_resolution = -1;
        configuration.omit_charset = -1;
        assert_eq!(configuration.resolved_pk_resolution(600), 72);
        assert!(configuration.omits_charset());
    }

    #[test]
    fn image_output_controls_use_pdftex_consumer_ranges() {
        let parameters = PdfOutputParameters {
            output: 1,
            major_version: 1,
            minor_version: 4,
            compress_level: 9,
            object_compress_level: 0,
            decimal_digits: 3,
            gamma: -1,
            image_gamma: 1_000_001,
            image_hicolor: 2,
            image_apply_gamma: -1,
            draft_mode: 2,
            inclusion_copy_fonts: -1,
            pk_resolution: 9_000,
            unique_resource_names: -2,
        }
        .normalized();
        assert_eq!(parameters.gamma, 0);
        assert_eq!(parameters.image_gamma, 1_000_000);
        assert_eq!(parameters.image_hicolor, 1);
        assert_eq!(parameters.image_apply_gamma, 0);
        assert_eq!(parameters.draft_mode, 2);
        assert_eq!(parameters.inclusion_copy_fonts, 0);
        assert_eq!(parameters.pk_resolution, 8_000);
        assert_eq!(parameters.unique_resource_names, 0);
    }

    #[test]
    fn rollback_reuses_page_object_suffix_and_fingerprint() {
        let mut state = PdfState::default();
        state.enable();
        let snapshot = state.snapshot();
        let hash = ContentHash::new([7; 32]);
        let parameters = PdfOutputParameters {
            output: 1,
            major_version: 1,
            minor_version: 4,
            compress_level: 9,
            object_compress_level: 0,
            decimal_digits: 3,
            gamma: 1_000,
            image_gamma: 2_200,
            image_hicolor: 1,
            image_apply_gamma: 0,
            draft_mode: 0,
            inclusion_copy_fonts: 0,
            pk_resolution: 0,
            unique_resource_names: 0,
        };
        let token = PdfTokenParameter {
            tokens: TokenListId::EMPTY,
            semantic_id: 0,
        };
        let page = PdfPageParameters {
            h_origin: Scaled::from_raw(10),
            v_origin: Scaled::from_raw(20),
            width: Scaled::from_raw(30),
            height: Scaled::from_raw(40),
            page_attr: token,
            resources: token,
            omit_procset: 0,
        };
        state.commit_page(hash, parameters, page, token);
        let first = (state.pages()[0], state.cursor());
        state.rollback(snapshot);
        state.commit_page(hash, parameters, page, token);
        assert_eq!((state.pages()[0], state.cursor()), first);
    }

    #[test]
    fn font_output_log_rolls_back_and_projects_last_attribute_and_char_union() {
        let mut state = PdfState::default();
        state.enable();
        let font = crate::font::NULL_FONT;
        state.set_font_attribute(font, b"/StemV 70".to_vec());
        state.include_font_chars(font, vec![b'B', b'A', b'B']);
        state.set_glyph_to_unicode(PdfGlyphToUnicode {
            tfm_name: None,
            glyph_name: b"A".to_vec(),
            unicode: vec![0x41],
        });
        let checkpoint = state.snapshot();
        let checkpoint_hash = state.hash_fragment();

        state.set_font_attribute(font, b"/StemV 80".to_vec());
        state.include_font_chars(font, vec![b'C']);
        state.disable_builtin_to_unicode(font);
        state.set_glyph_to_unicode(PdfGlyphToUnicode {
            tfm_name: None,
            glyph_name: b"A".to_vec(),
            unicode: vec![0x391],
        });
        state.push_font_map(PdfFontMapOperation::Line(
            tex_fonts::PdfFontMapEntry::parse(b"cmr10 CMR10 <cmr10.pfb").expect("valid map entry"),
        ));
        assert_eq!(state.font_attribute(font), b"/StemV 80");
        assert_eq!(state.included_font_chars(font), b"ABC");
        assert_eq!(state.font_maps().count(), 1);
        assert_eq!(
            state.glyph_to_unicode(b"cmr10", b"A"),
            Some([0x391].as_slice())
        );
        assert!(state.builtin_to_unicode_disabled(font));

        state.rollback(checkpoint);
        assert_eq!(state.font_attribute(font), b"/StemV 70");
        assert_eq!(state.included_font_chars(font), b"AB");
        assert_eq!(state.font_maps().count(), 0);
        assert_eq!(
            state.glyph_to_unicode(b"cmr10", b"A"),
            Some([0x41].as_slice())
        );
        assert!(!state.builtin_to_unicode_disabled(font));
        assert_eq!(state.hash_fragment(), checkpoint_hash);
    }

    #[test]
    fn map_line_resolution_keeps_first_duplicate_and_honors_replace_and_remove() {
        let mut state = PdfState::default();
        for line in [
            b"cmr10 First <cmr10.pfb".as_slice(),
            b"+cmr10 Ignored <ignored.pfb",
            b"=cmr10 Replacement <replacement.pfb",
            b"-cmr10",
            b"cmtt10 CMTT10 <cmtt10.pfb",
        ] {
            state.push_font_map(PdfFontMapOperation::Line(
                tex_fonts::PdfFontMapEntry::parse(line).expect("valid map entry"),
            ));
        }
        assert_eq!(state.font_map_duplicate_names(), [b"cmr10"]);
        let entries = state.resolved_font_map_lines();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].tex_name, b"cmtt10");
    }
}
