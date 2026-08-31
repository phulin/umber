//! Deterministic standalone HTML serialization over positioned pages.

#[cfg(test)]
mod tests;

pub mod incremental;
mod markup;

use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write as _;
use std::sync::Arc;

use umber_hash::{AHash64, HashDomain};

use crate::positioned::{BoxKind, PositionedError, PositionedPage, TextUnit};
use crate::{FontResource, PageArtifact};

use markup::{base64, check_html_size, escape_attr, escape_text, hex};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct HtmlFontKey {
    pub name: String,
    pub tfm_content_hash: tex_fonts::FontContentHash,
    pub tfm_checksum: u32,
    pub design_size_raw: i32,
    pub at_size_raw: i32,
    pub opentype_program_identity: Option<tex_fonts::FontProgramIdentity>,
    pub opentype_instance_identity: Option<tex_fonts::FontInstanceIdentity>,
}

impl From<&FontResource> for HtmlFontKey {
    fn from(font: &FontResource) -> Self {
        Self {
            name: font.name.clone(),
            tfm_content_hash: font.tfm_content_hash,
            tfm_checksum: font.tfm_checksum,
            design_size_raw: font.design_size.raw(),
            at_size_raw: font.at_size.raw(),
            opentype_program_identity: font.opentype.as_ref().map(|font| font.program_identity),
            opentype_instance_identity: font.opentype.as_ref().map(|font| font.instance_identity),
        }
    }
}

/// A fully explicit browser font and TeX-code mapping.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HtmlFontAsset {
    pub key: HtmlFontKey,
    pub woff2: Vec<u8>,
    pub ahash64: [u8; 8],
    /// Exactly 256 entries. Every used code must have a mapping.
    pub encoding: Vec<Option<String>>,
    pub provenance: String,
    pub embeddable: bool,
}

/// Downstream font acquisition. Implementations must resolve exact keys and
/// must not use platform font fallback.
pub trait HtmlFontAssets {
    /// Returns an already-retained, already-selected asset; this must not acquire resources.
    fn font_asset(&self, font: &FontResource) -> Result<HtmlFontAsset, String>;

    /// Returns the validated program selected during layout when the output
    /// closure retains it. The default keeps existing resolver implementations
    /// source-compatible; their asset is decoded once during validation.
    fn realized_opentype(&self, _font: &FontResource) -> Option<&tex_fonts::OpenTypeFont> {
        None
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssetMode {
    Embedded,
    /// Content-addressed files returned separately and referenced below this
    /// validated relative directory.
    Manifest {
        relative_directory: String,
    },
}

/// Opaque identity binding rendered HTML to the session that produced it.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RenderedOutputId([u8; 16]);

impl RenderedOutputId {
    pub const ZERO: Self = Self([0; 16]);

    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(self) -> [u8; 16] {
        self.0
    }

    pub fn parse_hex(value: &str) -> Option<Self> {
        let value = value.as_bytes();
        if value.len() != 32 {
            return None;
        }
        let mut bytes = [0; 16];
        for (index, byte) in bytes.iter_mut().enumerate() {
            let start = index * 2;
            let high = hex_nibble(value[start])?;
            let low = hex_nibble(value[start + 1])?;
            *byte = (high << 4) | low;
        }
        Some(Self(bytes))
    }
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

impl fmt::Display for RenderedOutputId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HtmlOptions {
    pub title: String,
    pub language: String,
    /// Accepted editor revision whose page/event ordinals this HTML describes.
    pub revision: u64,
    /// Producing session identity paired with `revision` for source queries.
    pub output_id: RenderedOutputId,
    pub asset_mode: AssetMode,
    pub max_pages: usize,
    pub max_html_bytes: usize,
    pub max_asset_bytes: usize,
    pub max_total_asset_bytes: usize,
    pub max_special_bytes: usize,
    pub max_positioned_events: usize,
    pub max_positioned_depth: usize,
    pub max_text_run_units: usize,
}

impl Default for HtmlOptions {
    fn default() -> Self {
        Self {
            title: "Umber document".to_owned(),
            language: "und".to_owned(),
            revision: 1,
            output_id: RenderedOutputId::ZERO,
            asset_mode: AssetMode::Embedded,
            max_pages: 16_384,
            max_html_bytes: 256 * 1024 * 1024,
            max_asset_bytes: 64 * 1024 * 1024,
            max_total_asset_bytes: 256 * 1024 * 1024,
            max_special_bytes: 4 * 1024,
            max_positioned_events: 1_000_000,
            max_positioned_depth: 4_096,
            max_text_run_units: 16_384,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HtmlAsset {
    pub path: String,
    pub bytes: Vec<u8>,
    pub ahash64: [u8; 8],
    pub provenance: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HtmlOutput {
    pub html: Vec<u8>,
    pub assets: Vec<HtmlAsset>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HtmlError {
    NoPages,
    TooManyPages { count: usize, limit: usize },
    Positioned(PositionedError),
    MissingPageFont { page: u32, font_id: u32 },
    FontResolution { font: String, message: String },
    FontKeyMismatch { font: String },
    InvalidEncodingLength { font: String, count: usize },
    MissingTextMapping { font: String, code: u32 },
    MissingFontGlyph { font: String, code: u8, ch: char },
    MissingMathFontInstance,
    MathGlyphMismatch { glyph_id: u16 },
    MissingMathGlyphOutline { glyph_id: u16 },
    InvalidMathEventSequence,
    UnsafeTextMapping { font: String, code: u32 },
    EmptyFontAsset { font: String },
    CorruptFontAsset { font: String },
    UnlicensedFont { font: String },
    AssetTooLarge { bytes: usize, limit: usize },
    AssetsTooLarge { bytes: usize, limit: usize },
    HtmlTooLarge { bytes: usize, limit: usize },
    InvalidAssetDirectory,
    InvalidLanguage,
    SpecialTooLarge { bytes: usize, limit: usize },
    InconsistentFont { font: String },
    InvalidSpecial { message: String },
    SpecialNestingTooDeep { limit: usize },
}

impl std::fmt::Display for HtmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPages => f.write_str("cannot write HTML without page artifacts"),
            Self::TooManyPages { count, limit } => {
                write!(f, "HTML page count {count} exceeds limit {limit}")
            }
            Self::Positioned(error) => error.fmt(f),
            Self::MissingPageFont { page, font_id } => {
                write!(f, "HTML page {page} references missing font {font_id}")
            }
            Self::FontResolution { font, message } => {
                write!(
                    f,
                    "retained HTML font asset {font} is unavailable: {message}"
                )
            }
            Self::FontKeyMismatch { font } => {
                write!(
                    f,
                    "retained HTML font asset has the wrong identity for {font}"
                )
            }
            Self::InvalidEncodingLength { font, count } => {
                write!(
                    f,
                    "HTML font asset {font} mapping has {count} entries, expected 256"
                )
            }
            Self::MissingTextMapping { font, code } => {
                write!(
                    f,
                    "HTML font asset {font} has no text mapping for code {code}"
                )
            }
            Self::MissingFontGlyph { font, code, ch } => {
                write!(
                    f,
                    "HTML font asset {font} has no glyph for code {code} mapping {ch:?}"
                )
            }
            Self::MissingMathFontInstance => {
                f.write_str("HTML math references an unavailable OpenType font instance")
            }
            Self::MathGlyphMismatch { glyph_id } => write!(
                f,
                "HTML math cmap and ssty selection does not reproduce glyph {glyph_id}"
            ),
            Self::MissingMathGlyphOutline { glyph_id } => {
                write!(f, "HTML math glyph {glyph_id} has no validated outline")
            }
            Self::InvalidMathEventSequence => {
                f.write_str("HTML math event stream is not properly nested")
            }
            Self::UnsafeTextMapping { font, code } => {
                write!(
                    f,
                    "HTML font asset {font} code {code} maps to unsafe HTML text"
                )
            }
            Self::EmptyFontAsset { font } => write!(f, "HTML font asset {font} has no WOFF2 bytes"),
            Self::CorruptFontAsset { font } => {
                write!(
                    f,
                    "HTML font asset {font} does not match its aHash64 identity"
                )
            }
            Self::UnlicensedFont { font } => {
                write!(f, "HTML font asset {font} is not licensed for embedding")
            }
            Self::AssetTooLarge { bytes, limit } => {
                write!(
                    f,
                    "HTML font asset requires {bytes} bytes, exceeding limit {limit}"
                )
            }
            Self::AssetsTooLarge { bytes, limit } => {
                write!(
                    f,
                    "HTML font assets require {bytes} bytes, exceeding limit {limit}"
                )
            }
            Self::HtmlTooLarge { bytes, limit } => {
                write!(f, "HTML requires {bytes} bytes, exceeding limit {limit}")
            }
            Self::InvalidAssetDirectory => {
                f.write_str("HTML asset directory must be a safe relative path")
            }
            Self::InvalidLanguage => f.write_str("HTML language must be a simple BCP-47 token"),
            Self::SpecialTooLarge { bytes, limit } => {
                write!(
                    f,
                    "HTML special requires {bytes} bytes, exceeding limit {limit}"
                )
            }
            Self::InconsistentFont { font } => {
                write!(f, "HTML pages resolve font {font} inconsistently")
            }
            Self::InvalidSpecial { message } => write!(f, "invalid HTML special: {message}"),
            Self::SpecialNestingTooDeep { limit } => {
                write!(f, "HTML special nesting exceeds limit {limit}")
            }
        }
    }
}

impl std::error::Error for HtmlError {}

impl From<PositionedError> for HtmlError {
    fn from(value: PositionedError) -> Self {
        Self::Positioned(value)
    }
}

pub fn write_html<R: HtmlFontAssets>(
    pages: &[PageArtifact],
    assets: &R,
    options: &HtmlOptions,
) -> Result<HtmlOutput, HtmlError> {
    if pages.is_empty() {
        return Err(HtmlError::NoPages);
    }
    if pages.len() > options.max_pages {
        return Err(HtmlError::TooManyPages {
            count: pages.len(),
            limit: options.max_pages,
        });
    }
    validate_options(options)?;
    let document = incremental::build_render_document(
        pages,
        assets,
        options,
        incremental::RenderSessionId::from_bytes(options.output_id.as_bytes()),
        options.revision,
        None,
        standalone_render_limits(options),
    )
    .map_err(standalone_render_error)?;
    write_render_document(&document, options)
}

pub fn write_positioned_html<R: HtmlFontAssets>(
    pages: &[PositionedPage],
    assets: &R,
    options: &HtmlOptions,
) -> Result<HtmlOutput, HtmlError> {
    let document = incremental::build_positioned_render_document(
        pages,
        assets,
        options,
        incremental::RenderSessionId::from_bytes(options.output_id.as_bytes()),
        options.revision,
        None,
        standalone_render_limits(options),
    )
    .map_err(standalone_render_error)?;
    write_render_document(&document, options)
}

fn standalone_render_limits(options: &HtmlOptions) -> incremental::RenderLimits {
    incremental::RenderLimits {
        max_pages: options.max_pages,
        max_nodes: usize::MAX,
        max_resources: usize::MAX,
        max_resource_bytes: usize::MAX,
    }
}

fn standalone_render_error(error: incremental::RenderBuildError) -> HtmlError {
    match error {
        incremental::RenderBuildError::Html(error) => error,
        incremental::RenderBuildError::TooManyNodes { .. }
        | incremental::RenderBuildError::ResourcesTooLarge { .. }
        | incremental::RenderBuildError::TooManyResources { .. }
        | incremental::RenderBuildError::RevisionNotMonotonic { .. }
        | incremental::RenderBuildError::SessionMismatch => {
            unreachable!("standalone render has no previous revision or resource-count limit")
        }
    }
}

/// Serializes a detached producer model without consulting fonts, artifacts,
/// engine state, or any host capability.
pub fn write_render_document(
    document: &incremental::RenderDocument,
    options: &HtmlOptions,
) -> Result<HtmlOutput, HtmlError> {
    let mut options = options.clone();
    options.revision = document.revision.revision;
    options.output_id = RenderedOutputId::from_bytes(document.revision.session_id.as_bytes());
    validate_options(&options)?;
    let resources = document
        .revision
        .resources
        .iter()
        .map(|resource| (resource.identity, resource))
        .collect::<BTreeMap<_, _>>();
    let assets = build_render_assets(document, &resources, &options)?;
    let mut html = String::new();
    html.push_str("<!doctype html>\n<html lang=\"");
    escape_attr(&document.revision.language, &mut html);
    html.push_str("\"><head><meta charset=\"utf-8\"><meta name=\"generator\" content=\"umber-html/1\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; font-src data: 'self'; style-src 'unsafe-inline'; img-src data:\"><title>");
    escape_text(&document.revision.title, &mut html);
    html.push_str("</title><style>\n");
    check_html_size(&html, options.max_html_bytes)?;
    write_render_font_css(&mut html, document, &resources, &options)?;
    html.push_str(markup::BASE_CSS);
    html.push_str("</style></head><body>\n<main class=\"umber-document\">\n");
    for page in &document.revision.pages {
        markup::write_render_page(&mut html, page, &options)?;
    }
    html.push_str("</main></body></html>\n");
    if html.len() > options.max_html_bytes {
        return Err(HtmlError::HtmlTooLarge {
            bytes: html.len(),
            limit: options.max_html_bytes,
        });
    }
    Ok(HtmlOutput {
        html: html.into_bytes(),
        assets,
    })
}

fn build_render_assets(
    document: &incremental::RenderDocument,
    resources: &BTreeMap<[u8; 8], &incremental::RenderResource>,
    options: &HtmlOptions,
) -> Result<Vec<HtmlAsset>, HtmlError> {
    let mut by_digest = BTreeMap::new();
    let mut total = 0usize;
    for font in &document.fonts {
        if by_digest.contains_key(&font.digest_hex) {
            continue;
        }
        let resource = resources
            .get(&font.identity)
            .expect("render font resource validated during construction");
        total = total
            .checked_add(resource.bytes.len())
            .ok_or(HtmlError::AssetsTooLarge {
                bytes: usize::MAX,
                limit: options.max_total_asset_bytes,
            })?;
        if total > options.max_total_asset_bytes {
            return Err(HtmlError::AssetsTooLarge {
                bytes: total,
                limit: options.max_total_asset_bytes,
            });
        }
        if matches!(options.asset_mode, AssetMode::Manifest { .. }) {
            by_digest.insert(
                font.digest_hex.clone(),
                HtmlAsset {
                    path: format!("ahash64-v1-{}.woff2", font.digest_hex),
                    bytes: resource.bytes.clone(),
                    ahash64: resource.identity,
                    provenance: resource.provenance.clone(),
                },
            );
        }
    }
    Ok(by_digest.into_values().collect())
}

fn write_render_font_css(
    out: &mut String,
    document: &incremental::RenderDocument,
    resources: &BTreeMap<[u8; 8], &incremental::RenderResource>,
    options: &HtmlOptions,
) -> Result<(), HtmlError> {
    for font in &document.fonts {
        let resource = resources
            .get(&font.identity)
            .expect("render font resource validated during construction");
        out.push_str("@font-face{font-family:'");
        out.push_str(&font.family);
        out.push_str("';src:url('");
        match &options.asset_mode {
            AssetMode::Embedded => {
                let encoded = resource
                    .bytes
                    .len()
                    .checked_add(2)
                    .and_then(|len| (len / 3).checked_mul(4))
                    .ok_or(HtmlError::HtmlTooLarge {
                        bytes: usize::MAX,
                        limit: options.max_html_bytes,
                    })?;
                let projected = out
                    .len()
                    .checked_add(encoded)
                    .ok_or(HtmlError::HtmlTooLarge {
                        bytes: usize::MAX,
                        limit: options.max_html_bytes,
                    })?;
                if projected > options.max_html_bytes {
                    return Err(HtmlError::HtmlTooLarge {
                        bytes: projected,
                        limit: options.max_html_bytes,
                    });
                }
                out.push_str("data:font/woff2;base64,");
                base64(&resource.bytes, out);
            }
            AssetMode::Manifest { relative_directory } => {
                out.push_str(relative_directory);
                if !relative_directory.ends_with('/') {
                    out.push('/');
                }
                out.push_str("ahash64-v1-");
                out.push_str(&font.digest_hex);
                out.push_str(".woff2");
            }
        }
        out.push_str("') format('woff2');font-display:block;font-style:normal;font-weight:400}\n");
        check_html_size(out, options.max_html_bytes)?;
    }
    Ok(())
}

#[derive(Clone)]
struct ResolvedFont {
    web: HtmlFontAsset,
    digest_hex: String,
    family: String,
    sfnt: Arc<[u8]>,
}

fn validate_font(
    font: &FontResource,
    web: HtmlFontAsset,
    realized: Option<&tex_fonts::OpenTypeFont>,
    options: &HtmlOptions,
) -> Result<ResolvedFont, HtmlError> {
    let key = HtmlFontKey::from(font);
    if web.key != key {
        return Err(HtmlError::FontKeyMismatch {
            font: font.name.clone(),
        });
    }
    if web.woff2.is_empty() {
        return Err(HtmlError::EmptyFontAsset {
            font: font.name.clone(),
        });
    }
    if !web.woff2.starts_with(b"wOF2") {
        return Err(HtmlError::CorruptFontAsset {
            font: font.name.clone(),
        });
    }
    if web.woff2.len() > options.max_asset_bytes {
        return Err(HtmlError::AssetTooLarge {
            bytes: web.woff2.len(),
            limit: options.max_asset_bytes,
        });
    }
    if web.encoding.len() != 256 {
        return Err(HtmlError::InvalidEncodingLength {
            font: font.name.clone(),
            count: web.encoding.len(),
        });
    }
    if !web.embeddable {
        return Err(HtmlError::UnlicensedFont {
            font: font.name.clone(),
        });
    }
    let digest = AHash64::for_bytes(HashDomain::HtmlResource, &web.woff2).to_le_bytes();
    let object_identity = tex_fonts::FontObjectIdentity::for_bytes(&web.woff2);
    if digest != web.ahash64 {
        return Err(HtmlError::CorruptFontAsset {
            font: font.name.clone(),
        });
    }
    if let Some(opentype) = &font.opentype
        && (opentype.container != tex_fonts::FontContainer::Woff2
            || opentype.object_identity != object_identity)
    {
        return Err(HtmlError::CorruptFontAsset {
            font: font.name.clone(),
        });
    }
    let sfnt = if let Some(opentype) = &font.opentype {
        if let Some(realized) = realized {
            if realized.identity != opentype.program_identity
                || realized.object_identity != opentype.object_identity
                || realized.container != opentype.container
                || realized.face_index != opentype.face_index
                || realized.instance_identity(font.at_size) != opentype.instance_identity
                || realized.transport_bytes.as_ref() != web.woff2.as_slice()
            {
                return Err(HtmlError::CorruptFontAsset {
                    font: font.name.clone(),
                });
            }
            realized.decoded_bytes()
        } else {
            let key = tex_fonts::FontRequestKey::new(
                &font.name,
                opentype.face_index,
                tex_fonts::VariationSelection::default(),
                tex_fonts::FontFeaturePolicy::default(),
            )
            .map_err(|_| HtmlError::CorruptFontAsset {
                font: font.name.clone(),
            })?;
            let request = tex_fonts::FontRequest {
                key: key.clone(),
                accepted_containers: tex_fonts::AcceptedFontContainers::WASM,
                purposes: tex_fonts::FontPurposes::LAYOUT_AND_HTML,
            };
            tex_fonts::OpenTypeFont::parse(
                &request,
                tex_fonts::ResolvedFont {
                    request: key,
                    container: tex_fonts::FontContainer::Woff2,
                    bytes: web.woff2.clone(),
                    declared_object_ahash64: Some(opentype.object_identity),
                    declared_program_identity: Some(opentype.program_identity),
                    provenance: None,
                    legacy_mapping: None,
                },
                tex_fonts::FontLimits::default(),
            )
            .map_err(|_| HtmlError::CorruptFontAsset {
                font: font.name.clone(),
            })?
            .decoded_bytes()
        }
    } else if let Some(realized) = realized {
        if realized.container != tex_fonts::FontContainer::Woff2
            || realized.object_identity != object_identity
            || realized.transport_bytes.as_ref() != web.woff2.as_slice()
        {
            return Err(HtmlError::CorruptFontAsset {
                font: font.name.clone(),
            });
        }
        realized.decoded_bytes()
    } else {
        Arc::from(
            woff2_patched::convert_woff2_to_ttf(&mut web.woff2.as_slice()).map_err(|_| {
                HtmlError::CorruptFontAsset {
                    font: font.name.clone(),
                }
            })?,
        )
    };
    let declared_size = web
        .woff2
        .get(16..20)
        .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
        .map(u32::from_be_bytes)
        .and_then(|size| usize::try_from(size).ok())
        .ok_or_else(|| HtmlError::CorruptFontAsset {
            font: font.name.clone(),
        })?;
    if declared_size > options.max_asset_bytes {
        return Err(HtmlError::AssetTooLarge {
            bytes: declared_size,
            limit: options.max_asset_bytes,
        });
    }
    let face_index = realized.map_or_else(
        || font.opentype.as_ref().map_or(0, |font| font.face_index),
        |font| font.face_index,
    );
    validate_mapped_glyphs(&sfnt, face_index, &web.encoding, &font.name)?;
    let digest_hex = hex(&digest);
    let family_identity = font
        .opentype
        .as_ref()
        .map_or(digest, |font| font.program_identity.bytes());
    let family_hex = hex(&family_identity);
    let family = format!("umber-font-{family_hex}");
    Ok(ResolvedFont {
        web,
        digest_hex,
        family,
        sfnt,
    })
}

fn validate_mapped_glyphs(
    sfnt: &[u8],
    face_index: u32,
    encoding: &[Option<String>],
    font_name: &str,
) -> Result<(), HtmlError> {
    let face =
        ttf_parser::Face::parse(sfnt, face_index).map_err(|_| HtmlError::CorruptFontAsset {
            font: font_name.to_owned(),
        })?;
    for (code, mapping) in encoding.iter().enumerate() {
        for ch in mapping.iter().flat_map(|mapping| mapping.chars()) {
            if face.glyph_index(ch).is_none() {
                return Err(HtmlError::MissingFontGlyph {
                    font: font_name.to_owned(),
                    code: code as u8,
                    ch,
                });
            }
        }
    }
    Ok(())
}

fn accessible_line(box_stack: &[(u32, BoxKind)]) -> Option<u32> {
    box_stack
        .iter()
        .enumerate()
        .rev()
        .find(|(index, (_, kind))| {
            *kind == BoxKind::Horizontal
                && (*index == 0 || box_stack[*index - 1].1 == BoxKind::Vertical)
        })
        .map(|(_, (id, _))| *id)
}

fn selected_glyph(
    font: &ResolvedFont,
    opentype: &crate::OpenTypeFontResource,
    ch: char,
    ssty: u8,
) -> Option<u16> {
    if ssty == 0 {
        return ttf_parser::Face::parse(&font.sfnt, opentype.face_index)
            .ok()?
            .glyph_index(ch)
            .map(|glyph| glyph.0);
    }
    let mut face = rustybuzz::Face::from_slice(&font.sfnt, opentype.face_index)?;
    let variations = opentype
        .variation
        .coordinates()
        .iter()
        .map(|coordinate| rustybuzz::Variation {
            tag: rustybuzz::ttf_parser::Tag::from_bytes(&coordinate.tag.bytes()),
            value: coordinate.value as f32 / 65_536.0,
        })
        .collect::<Vec<_>>();
    face.set_variations(&variations);
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    let mut encoded = [0; 4];
    buffer.push_str(ch.encode_utf8(&mut encoded));
    let feature = rustybuzz::Feature::new(
        rustybuzz::ttf_parser::Tag::from_bytes(b"ssty"),
        u32::from(ssty),
        ..,
    );
    let shaped = rustybuzz::shape(&face, &[feature], buffer);
    let infos = shaped.glyph_infos();
    (infos.len() == 1)
        .then(|| u16::try_from(infos[0].glyph_id).ok())
        .flatten()
}

fn outline_path(
    font: &ResolvedFont,
    opentype: &crate::OpenTypeFontResource,
    glyph_id: u16,
) -> Result<(String, u16), HtmlError> {
    let mut face = ttf_parser::Face::parse(&font.sfnt, opentype.face_index).map_err(|_| {
        HtmlError::CorruptFontAsset {
            font: font.web.key.name.clone(),
        }
    })?;
    for coordinate in opentype.variation.coordinates() {
        let tag = ttf_parser::Tag::from_bytes(&coordinate.tag.bytes());
        let value = coordinate.value as f32 / 65_536.0;
        face.set_variation(tag, value)
            .ok_or_else(|| HtmlError::CorruptFontAsset {
                font: font.web.key.name.clone(),
            })?;
    }
    let mut builder = SvgOutline::default();
    if face
        .outline_glyph(ttf_parser::GlyphId(glyph_id), &mut builder)
        .is_none()
        || builder.invalid
        || builder.path.is_empty()
    {
        return Err(HtmlError::MissingMathGlyphOutline { glyph_id });
    }
    Ok((builder.path, face.units_per_em()))
}

#[derive(Default)]
struct SvgOutline {
    path: String,
    invalid: bool,
}

impl SvgOutline {
    fn point(&mut self, command: char, values: &[f32]) {
        if values.iter().any(|value| !value.is_finite()) {
            self.invalid = true;
            return;
        }
        self.path.push(command);
        for value in values {
            self.path.push(' ');
            let _ = write!(self.path, "{value}");
        }
    }
}

impl ttf_parser::OutlineBuilder for SvgOutline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.point('M', &[x, y]);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.point('L', &[x, y]);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.point('Q', &[x1, y1, x, y]);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.point('C', &[x1, y1, x2, y2, x, y]);
    }
    fn close(&mut self) {
        self.path.push('Z');
    }
}

#[derive(Default)]
struct SpecialState {
    colors: Vec<String>,
    link: Option<String>,
}

impl SpecialState {
    fn color(&self) -> Option<&str> {
        self.colors.last().map(String::as_str)
    }

    fn apply(&mut self, special: &InterpretedSpecial) -> Result<(), HtmlError> {
        const LIMIT: usize = 256;
        match special {
            InterpretedSpecial::ColorPush(color) => {
                if self.colors.len() >= LIMIT {
                    return Err(HtmlError::SpecialNestingTooDeep { limit: LIMIT });
                }
                self.colors.push(color.clone());
            }
            InterpretedSpecial::ColorPop => {
                self.colors.pop().ok_or_else(|| HtmlError::InvalidSpecial {
                    message: "color pop without push".to_owned(),
                })?;
            }
            InterpretedSpecial::LinkStart(link) => {
                if self.link.is_some() {
                    return Err(HtmlError::InvalidSpecial {
                        message: "nested links are not supported".to_owned(),
                    });
                }
                self.link = Some(link.clone());
            }
            InterpretedSpecial::LinkEnd => {
                self.link.take().ok_or_else(|| HtmlError::InvalidSpecial {
                    message: "link end without start".to_owned(),
                })?;
            }
            InterpretedSpecial::Destination(_) | InterpretedSpecial::Inert => {}
        }
        Ok(())
    }
}

enum InterpretedSpecial {
    ColorPush(String),
    ColorPop,
    LinkStart(String),
    LinkEnd,
    Destination(String),
    Inert,
}

fn interpret_special(
    event: &crate::positioned::PositionedSpecial,
) -> Result<InterpretedSpecial, HtmlError> {
    if event.class != "html" {
        return Ok(InterpretedSpecial::Inert);
    }
    let payload = std::str::from_utf8(&event.payload).map_err(|_| HtmlError::InvalidSpecial {
        message: "payload is not UTF-8".to_owned(),
    })?;
    if payload == "color pop" {
        return Ok(InterpretedSpecial::ColorPop);
    }
    if let Some(color) = payload.strip_prefix("color push ") {
        return canonical_color(color)
            .map(InterpretedSpecial::ColorPush)
            .ok_or_else(|| HtmlError::InvalidSpecial {
                message: format!("unsupported color {color:?}"),
            });
    }
    if payload == "endlink" {
        return Ok(InterpretedSpecial::LinkEnd);
    }
    if let Some(link) = payload.strip_prefix("link ") {
        if safe_link(link) {
            return Ok(InterpretedSpecial::LinkStart(link.to_owned()));
        }
        return Err(HtmlError::InvalidSpecial {
            message: format!("unsafe link {link:?}"),
        });
    }
    if let Some(id) = payload.strip_prefix("dest ") {
        if safe_identifier(id) {
            return Ok(InterpretedSpecial::Destination(format!("umber-dest-{id}")));
        }
        return Err(HtmlError::InvalidSpecial {
            message: format!("unsafe destination {id:?}"),
        });
    }
    Ok(InterpretedSpecial::Inert)
}

fn canonical_color(color: &str) -> Option<String> {
    match color {
        "black" | "red" | "green" | "blue" | "cyan" | "magenta" | "yellow" | "gray" => {
            Some(color.to_owned())
        }
        _ if color.len() == 7
            && color.starts_with('#')
            && color[1..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()) =>
        {
            Some(color.to_owned())
        }
        _ => None,
    }
}

fn safe_link(link: &str) -> bool {
    (link.starts_with('#') && safe_identifier(&link[1..]))
        || (link.starts_with("https://")
            && !link
                .chars()
                .any(|ch| ch.is_control() || matches!(ch, '"' | '\'' | '<' | '>' | '\\')))
}

fn safe_identifier(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn map_text(
    units: &[TextUnit],
    font: &ResolvedFont,
    mapped_encoding: bool,
    max_bytes: usize,
) -> Result<String, HtmlError> {
    let mut text = String::new();
    for unit in units {
        match unit {
            TextUnit::Space => {
                let projected = text.len().checked_add(1).ok_or(HtmlError::HtmlTooLarge {
                    bytes: usize::MAX,
                    limit: max_bytes,
                })?;
                if text.len() >= max_bytes {
                    return Err(HtmlError::HtmlTooLarge {
                        bytes: projected,
                        limit: max_bytes,
                    });
                }
                text.push(' ');
            }
            TextUnit::Code(code) => {
                let direct;
                let mapping = if mapped_encoding {
                    usize::try_from(*code)
                        .ok()
                        .and_then(|code| font.web.encoding.get(code))
                        .and_then(Option::as_ref)
                        .ok_or_else(|| HtmlError::MissingTextMapping {
                            font: font.web.key.name.clone(),
                            code: *code,
                        })?
                } else {
                    direct = char::from_u32(*code)
                        .ok_or_else(|| HtmlError::MissingTextMapping {
                            font: font.web.key.name.clone(),
                            code: *code,
                        })?
                        .to_string();
                    &direct
                };
                if mapping
                    .chars()
                    .any(|ch| ch == '\0' || (ch.is_control() && ch != '\t'))
                {
                    return Err(HtmlError::UnsafeTextMapping {
                        font: font.web.key.name.clone(),
                        code: *code,
                    });
                }
                let projected =
                    text.len()
                        .checked_add(mapping.len())
                        .ok_or(HtmlError::HtmlTooLarge {
                            bytes: usize::MAX,
                            limit: max_bytes,
                        })?;
                if projected > max_bytes {
                    return Err(HtmlError::HtmlTooLarge {
                        bytes: projected,
                        limit: max_bytes,
                    });
                }
                text.push_str(mapping);
            }
        }
    }
    Ok(text)
}

fn validate_options(options: &HtmlOptions) -> Result<(), HtmlError> {
    let title_bytes = options
        .title
        .len()
        .checked_mul(6)
        .ok_or(HtmlError::HtmlTooLarge {
            bytes: usize::MAX,
            limit: options.max_html_bytes,
        })?;
    let language_bytes = options
        .language
        .len()
        .checked_mul(6)
        .ok_or(HtmlError::HtmlTooLarge {
            bytes: usize::MAX,
            limit: options.max_html_bytes,
        })?;
    if title_bytes > options.max_html_bytes || language_bytes > options.max_html_bytes {
        return Err(HtmlError::HtmlTooLarge {
            bytes: title_bytes.max(language_bytes),
            limit: options.max_html_bytes,
        });
    }
    if options.language.is_empty()
        || !options
            .language
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(HtmlError::InvalidLanguage);
    }
    if let AssetMode::Manifest { relative_directory } = &options.asset_mode
        && (relative_directory.is_empty()
            || relative_directory.starts_with('/')
            || relative_directory.contains("..")
            || relative_directory.contains('\\')
            || relative_directory.contains(':')
            || !relative_directory
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'/')))
    {
        return Err(HtmlError::InvalidAssetDirectory);
    }
    Ok(())
}
