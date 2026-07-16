use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use sha2::{Digest, Sha256};
use tex_arith::{Scaled, font_units_to_scaled};
use ttf_parser::{Face, GlyphId, OutlineBuilder, RawFace, Tag};

use super::contract::{
    FONT_PROGRAM_IDENTITY_VERSION, FontContainer, FontLimits, FontObjectIdentity,
    FontProgramIdentity, FontRequest, OpenTypeTag, ResolvedFont, VariationSelection,
};

const IDENTITY_TABLES: &[OpenTypeTag] = &[
    OpenTypeTag::new(*b"avar"),
    OpenTypeTag::new(*b"BASE"),
    OpenTypeTag::new(*b"CBDT"),
    OpenTypeTag::new(*b"CBLC"),
    OpenTypeTag::new(*b"CFF "),
    OpenTypeTag::new(*b"CFF2"),
    OpenTypeTag::new(*b"cmap"),
    OpenTypeTag::new(*b"COLR"),
    OpenTypeTag::new(*b"CPAL"),
    OpenTypeTag::new(*b"cvar"),
    OpenTypeTag::new(*b"fvar"),
    OpenTypeTag::new(*b"gasp"),
    OpenTypeTag::new(*b"GDEF"),
    OpenTypeTag::new(*b"glyf"),
    OpenTypeTag::new(*b"GPOS"),
    OpenTypeTag::new(*b"GSUB"),
    OpenTypeTag::new(*b"gvar"),
    OpenTypeTag::new(*b"head"),
    OpenTypeTag::new(*b"hhea"),
    OpenTypeTag::new(*b"hmtx"),
    OpenTypeTag::new(*b"HVAR"),
    OpenTypeTag::new(*b"JSTF"),
    OpenTypeTag::new(*b"kern"),
    OpenTypeTag::new(*b"loca"),
    OpenTypeTag::new(*b"MATH"),
    OpenTypeTag::new(*b"maxp"),
    OpenTypeTag::new(*b"MVAR"),
    OpenTypeTag::new(*b"OS/2"),
    OpenTypeTag::new(*b"post"),
    OpenTypeTag::new(*b"sbix"),
    OpenTypeTag::new(*b"STAT"),
    OpenTypeTag::new(*b"SVG "),
    OpenTypeTag::new(*b"VORG"),
    OpenTypeTag::new(*b"vhea"),
    OpenTypeTag::new(*b"vmtx"),
    OpenTypeTag::new(*b"VVAR"),
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CharacterMap(BTreeMap<u32, u16>);

impl CharacterMap {
    #[must_use]
    pub fn glyph(&self, scalar: char) -> Option<u16> {
        self.0.get(&(scalar as u32)).copied()
    }

    #[must_use]
    pub fn mappings(&self) -> &BTreeMap<u32, u16> {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenTypeMetrics {
    pub units_per_em: u16,
    pub ascender: i16,
    pub descender: i16,
    pub line_gap: i16,
    pub global_bounds: Option<(i16, i16, i16, i16)>,
    pub horizontal_advances: Vec<u16>,
    pub glyph_bounds: Vec<Option<(i16, i16, i16, i16)>>,
}

impl OpenTypeMetrics {
    /// Converts font units into scaled points using round-half-away-from-zero.
    pub fn units_to_sp(&self, units: i32, size_sp: i32) -> Result<i32, FontParseError> {
        font_units_to_scaled(units, Scaled::from_raw(size_sp), self.units_per_em)
            .map(Scaled::raw)
            .map_err(|_| FontParseError::ArithmeticOverflow)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShapingTables {
    pub gdef: Option<Arc<[u8]>>,
    pub gsub: Option<Arc<[u8]>>,
    pub gpos: Option<Arc<[u8]>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontMetadata {
    pub glyph_count: u16,
    pub is_variable: bool,
    pub is_monospaced: bool,
    pub italic: bool,
}

/// Immutable, fully validated font program plus the retained transport object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenTypeFont {
    pub identity: FontProgramIdentity,
    pub object_identity: FontObjectIdentity,
    pub face_index: u32,
    pub cmap: CharacterMap,
    pub metrics: OpenTypeMetrics,
    pub shaping: ShapingTables,
    pub math: Option<Arc<[u8]>>,
    pub metadata: FontMetadata,
    pub container: FontContainer,
    pub transport_bytes: Arc<[u8]>,
}

impl OpenTypeFont {
    pub fn parse(
        request: &FontRequest,
        response: ResolvedFont,
        limits: FontLimits,
    ) -> Result<Self, FontParseError> {
        validate_limits(limits)?;
        if response.request != request.key {
            return Err(FontParseError::RequestMismatch);
        }
        if !request.accepted_containers.contains(response.container) {
            return Err(FontParseError::ContainerNotAccepted(response.container));
        }
        if response.bytes.len() > limits.max_object_bytes {
            return Err(FontParseError::LimitExceeded {
                resource: "font object bytes",
                limit: limits.max_object_bytes,
                attempted: response.bytes.len(),
            });
        }
        if response
            .provenance
            .as_ref()
            .is_some_and(|value| value.len() > limits.max_provenance_bytes)
        {
            return Err(FontParseError::LimitExceeded {
                resource: "font provenance bytes",
                limit: limits.max_provenance_bytes,
                attempted: response.provenance.as_ref().map_or(0, String::len),
            });
        }
        validate_container_magic(response.container, &response.bytes)?;
        let object_identity = FontObjectIdentity::for_bytes(&response.bytes);
        if response
            .declared_object_sha256
            .is_some_and(|declared| declared != object_identity)
        {
            return Err(FontParseError::ObjectIdentityMismatch);
        }

        let decoded = match response.container {
            FontContainer::Woff2 => {
                woff2_patched::convert_woff2_to_ttf(&mut response.bytes.as_slice())
                    .map_err(|_| FontParseError::InvalidWoff2)?
            }
            _ => response.bytes.clone(),
        };
        if decoded.len() > limits.max_decoded_bytes {
            return Err(FontParseError::LimitExceeded {
                resource: "decoded font bytes",
                limit: limits.max_decoded_bytes,
                attempted: decoded.len(),
            });
        }

        let raw = RawFace::parse(&decoded, request.key.face_index)
            .map_err(|error| FontParseError::InvalidSfnt(error.to_string()))?;
        let table_count = usize::from(raw.table_records.len());
        if table_count > limits.max_tables {
            return Err(FontParseError::LimitExceeded {
                resource: "OpenType tables",
                limit: limits.max_tables,
                attempted: table_count,
            });
        }
        validate_unique_tables(&raw)?;
        let identity = canonical_identity(&raw, request.key.face_index)?;
        if response
            .declared_program_identity
            .is_some_and(|declared| declared != identity)
        {
            return Err(FontParseError::ProgramIdentityMismatch);
        }

        let mut face = Face::parse(&decoded, request.key.face_index)
            .map_err(|error| FontParseError::InvalidSfnt(error.to_string()))?;
        apply_variations(&mut face, &request.key.variation)?;
        let glyph_count = face.number_of_glyphs();
        if usize::from(glyph_count) > limits.max_glyphs {
            return Err(FontParseError::LimitExceeded {
                resource: "glyphs",
                limit: limits.max_glyphs,
                attempted: usize::from(glyph_count),
            });
        }

        let cmap = project_cmap(&face, limits.max_mappings)?;
        let metrics = project_metrics(&face)?;
        validate_outlines(&face)?;
        let shaping = ShapingTables {
            gdef: table_arc(&raw, *b"GDEF"),
            gsub: table_arc(&raw, *b"GSUB"),
            gpos: table_arc(&raw, *b"GPOS"),
        };
        let math = table_arc(&raw, *b"MATH");
        let metadata = FontMetadata {
            glyph_count,
            is_variable: face.is_variable(),
            is_monospaced: face.is_monospaced(),
            italic: face.is_italic(),
        };
        Ok(Self {
            identity,
            object_identity,
            face_index: request.key.face_index,
            cmap,
            metrics,
            shaping,
            math,
            metadata,
            container: response.container,
            transport_bytes: Arc::from(response.bytes),
        })
    }
}

fn validate_limits(limits: FontLimits) -> Result<(), FontParseError> {
    let hard = FontLimits::HARD_MAX;
    for (resource, attempted, maximum) in [
        (
            "font object bytes",
            limits.max_object_bytes,
            hard.max_object_bytes,
        ),
        (
            "decoded font bytes",
            limits.max_decoded_bytes,
            hard.max_decoded_bytes,
        ),
        ("OpenType tables", limits.max_tables, hard.max_tables),
        ("collection faces", limits.max_faces, hard.max_faces),
        ("glyphs", limits.max_glyphs, hard.max_glyphs),
        ("cmap mappings", limits.max_mappings, hard.max_mappings),
        (
            "variation axes",
            limits.max_variation_axes,
            hard.max_variation_axes,
        ),
        ("features", limits.max_features, hard.max_features),
        (
            "logical name bytes",
            limits.max_logical_name_bytes,
            hard.max_logical_name_bytes,
        ),
        (
            "provenance bytes",
            limits.max_provenance_bytes,
            hard.max_provenance_bytes,
        ),
    ] {
        if attempted > maximum {
            return Err(FontParseError::HardLimitExceeded {
                resource,
                hard: maximum,
                attempted,
            });
        }
    }
    Ok(())
}

fn validate_container_magic(container: FontContainer, bytes: &[u8]) -> Result<(), FontParseError> {
    let magic = bytes.get(..4).ok_or(FontParseError::TruncatedContainer)?;
    let valid = match container {
        FontContainer::Woff2 => magic == b"wOF2",
        FontContainer::Collection => magic == b"ttcf",
        FontContainer::OpenType => magic == b"OTTO",
        FontContainer::TrueType => magic == [0, 1, 0, 0] || magic == b"true",
    };
    if valid {
        Ok(())
    } else {
        Err(FontParseError::ContainerTypeMismatch(container))
    }
}

fn validate_unique_tables(raw: &RawFace<'_>) -> Result<(), FontParseError> {
    let mut previous = None;
    for record in raw.table_records {
        let tag = record.tag.to_bytes();
        if previous == Some(tag) {
            return Err(FontParseError::DuplicateTable(OpenTypeTag::new(tag)));
        }
        previous = Some(tag);
        if raw.table(record.tag).is_none() {
            return Err(FontParseError::InvalidTableRange(OpenTypeTag::new(tag)));
        }
    }
    Ok(())
}

fn canonical_identity(
    raw: &RawFace<'_>,
    face_index: u32,
) -> Result<FontProgramIdentity, FontParseError> {
    let mut hash = Sha256::new();
    hash.update(b"umber.font-program");
    hash.update([FONT_PROGRAM_IDENTITY_VERSION]);
    hash.update(face_index.to_be_bytes());
    for tag in IDENTITY_TABLES {
        if let Some(data) = raw.table(Tag::from_bytes(&tag.bytes())) {
            let normalized;
            let data = if tag.bytes() == *b"head" && data.len() >= 12 {
                normalized = {
                    let mut bytes = data.to_vec();
                    bytes[8..12].fill(0); // transport checksum adjustment
                    bytes
                };
                normalized.as_slice()
            } else {
                data
            };
            hash.update(tag.bytes());
            hash.update(
                u32::try_from(data.len())
                    .map_err(|_| FontParseError::ArithmeticOverflow)?
                    .to_be_bytes(),
            );
            hash.update(data);
        }
    }
    Ok(FontProgramIdentity::from_bytes(hash.finalize().into()))
}

fn project_cmap(face: &Face<'_>, limit: usize) -> Result<CharacterMap, FontParseError> {
    let mut map = BTreeMap::new();
    let Some(cmap) = face.tables().cmap else {
        return Err(FontParseError::MissingCmap);
    };
    for subtable in cmap
        .subtables
        .into_iter()
        .filter(|table| table.is_unicode())
    {
        subtable.codepoints(|codepoint| {
            if map.len() <= limit
                && let Some(glyph) = subtable.glyph_index(codepoint)
            {
                map.entry(codepoint).or_insert(glyph.0);
            }
        });
        if map.len() > limit {
            return Err(FontParseError::LimitExceeded {
                resource: "cmap mappings",
                limit,
                attempted: map.len(),
            });
        }
    }
    if map.is_empty() {
        return Err(FontParseError::MissingCmap);
    }
    Ok(CharacterMap(map))
}

fn project_metrics(face: &Face<'_>) -> Result<OpenTypeMetrics, FontParseError> {
    let count = face.number_of_glyphs();
    let mut horizontal_advances = Vec::with_capacity(usize::from(count));
    let mut glyph_bounds = Vec::with_capacity(usize::from(count));
    for value in 0..count {
        let glyph = GlyphId(value);
        horizontal_advances.push(
            face.glyph_hor_advance(glyph)
                .ok_or(FontParseError::MissingAdvance(value))?,
        );
        glyph_bounds.push(
            face.glyph_bounding_box(glyph)
                .map(|bounds| (bounds.x_min, bounds.y_min, bounds.x_max, bounds.y_max)),
        );
    }
    let bounds = face.global_bounding_box();
    Ok(OpenTypeMetrics {
        units_per_em: face.units_per_em(),
        ascender: face.ascender(),
        descender: face.descender(),
        line_gap: face.line_gap(),
        global_bounds: Some((bounds.x_min, bounds.y_min, bounds.x_max, bounds.y_max)),
        horizontal_advances,
        glyph_bounds,
    })
}

fn validate_outlines(face: &Face<'_>) -> Result<(), FontParseError> {
    let mut builder = ValidatingOutline;
    for value in 0..face.number_of_glyphs() {
        let glyph = GlyphId(value);
        if face.glyph_bounding_box(glyph).is_some()
            && face.outline_glyph(glyph, &mut builder).is_none()
        {
            return Err(FontParseError::InvalidOutline(value));
        }
    }
    Ok(())
}

struct ValidatingOutline;
impl OutlineBuilder for ValidatingOutline {
    fn move_to(&mut self, _: f32, _: f32) {}
    fn line_to(&mut self, _: f32, _: f32) {}
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
    fn close(&mut self) {}
}

fn apply_variations(
    face: &mut Face<'_>,
    selection: &VariationSelection,
) -> Result<(), FontParseError> {
    for coordinate in selection.coordinates() {
        let tag = Tag::from_bytes(&coordinate.tag.bytes());
        let value = coordinate.value as f32 / 65_536.0;
        let axis = face
            .variation_axes()
            .into_iter()
            .find(|axis| axis.tag == tag)
            .ok_or(FontParseError::UnknownVariationAxis(coordinate.tag))?;
        if value < axis.min_value || value > axis.max_value {
            return Err(FontParseError::VariationOutOfRange(coordinate.tag));
        }
        face.set_variation(tag, value)
            .ok_or(FontParseError::UnknownVariationAxis(coordinate.tag))?;
    }
    Ok(())
}

fn table_arc(raw: &RawFace<'_>, tag: [u8; 4]) -> Option<Arc<[u8]>> {
    raw.table(Tag::from_bytes(&tag)).map(Arc::from)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FontParseError {
    RequestMismatch,
    ContainerNotAccepted(FontContainer),
    ContainerTypeMismatch(FontContainer),
    TruncatedContainer,
    InvalidWoff2,
    InvalidSfnt(String),
    ObjectIdentityMismatch,
    ProgramIdentityMismatch,
    MissingCmap,
    MissingAdvance(u16),
    InvalidOutline(u16),
    DuplicateTable(OpenTypeTag),
    InvalidTableRange(OpenTypeTag),
    UnknownVariationAxis(OpenTypeTag),
    VariationOutOfRange(OpenTypeTag),
    ArithmeticOverflow,
    LimitExceeded {
        resource: &'static str,
        limit: usize,
        attempted: usize,
    },
    HardLimitExceeded {
        resource: &'static str,
        hard: usize,
        attempted: usize,
    },
}

impl fmt::Display for FontParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OpenType font rejected: {self:?}")
    }
}

impl std::error::Error for FontParseError {}
