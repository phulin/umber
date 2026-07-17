//! Deterministic standalone HTML serialization over positioned pages.

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write as _;

use sha2::{Digest, Sha256};
use tex_arith::Scaled;

use crate::positioned::{
    BoxKind, PositionedError, PositionedEvent, PositionedLimits, PositionedPage, TextUnit,
    lower_page_with_limits,
};
use crate::{
    ContentHash, FontResource, MathGlyph, MathGlyphSelection, MathOutputEvent, MathStart,
    PageArtifact,
};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct HtmlFontKey {
    pub name: String,
    pub tfm_content_hash: ContentHash,
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
pub struct WebFont {
    pub key: HtmlFontKey,
    pub woff2: Vec<u8>,
    pub sha256: [u8; 32],
    /// Exactly 256 entries. Every used code must have a mapping.
    pub encoding: Vec<Option<String>>,
    pub provenance: String,
    pub embeddable: bool,
}

/// Downstream font acquisition. Implementations must resolve exact keys and
/// must not use platform font fallback.
pub trait HtmlFontResolver {
    fn resolve(&mut self, font: &FontResource) -> Result<WebFont, String>;
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
    pub sha256: [u8; 32],
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
    Coordinate(crate::dvi::coordinates::CoordinateError),
    MissingPageFont { page: u32, font_id: u32 },
    FontResolution { font: String, message: String },
    FontKeyMismatch { font: String },
    InvalidEncodingLength { font: String, count: usize },
    MissingTextMapping { font: String, code: u8 },
    MissingFontGlyph { font: String, code: u8, ch: char },
    MissingMathFontInstance,
    MathGlyphMismatch { glyph_id: u16 },
    MissingMathGlyphOutline { glyph_id: u16 },
    InvalidMathEventSequence,
    UnsafeTextMapping { font: String, code: u8 },
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
            Self::Coordinate(error) => error.fmt(f),
            Self::MissingPageFont { page, font_id } => {
                write!(f, "HTML page {page} references missing font {font_id}")
            }
            Self::FontResolution { font, message } => {
                write!(f, "failed to resolve web font {font}: {message}")
            }
            Self::FontKeyMismatch { font } => {
                write!(
                    f,
                    "web font resolver returned the wrong identity for {font}"
                )
            }
            Self::InvalidEncodingLength { font, count } => {
                write!(
                    f,
                    "web font {font} encoding has {count} entries, expected 256"
                )
            }
            Self::MissingTextMapping { font, code } => {
                write!(f, "web font {font} has no text mapping for code {code}")
            }
            Self::MissingFontGlyph { font, code, ch } => {
                write!(
                    f,
                    "web font {font} has no glyph for code {code} mapping {ch:?}"
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
                write!(f, "web font {font} code {code} maps to unsafe HTML text")
            }
            Self::EmptyFontAsset { font } => write!(f, "web font {font} has no WOFF2 bytes"),
            Self::CorruptFontAsset { font } => {
                write!(f, "web font {font} does not match its SHA-256")
            }
            Self::UnlicensedFont { font } => {
                write!(f, "web font {font} is not licensed for embedding")
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

impl From<crate::dvi::coordinates::CoordinateError> for HtmlError {
    fn from(value: crate::dvi::coordinates::CoordinateError) -> Self {
        Self::Coordinate(value)
    }
}

pub fn write_html<R: HtmlFontResolver>(
    pages: &[PageArtifact],
    resolver: &mut R,
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
    let positioned = pages
        .iter()
        .enumerate()
        .map(|(index, page)| {
            let page_index = u32::try_from(index + 1).map_err(|_| HtmlError::TooManyPages {
                count: pages.len(),
                limit: u32::MAX as usize,
            })?;
            let positioned = lower_page_with_limits(
                page,
                page_index,
                PositionedLimits {
                    max_events: options.max_positioned_events,
                    max_depth: options.max_positioned_depth,
                    max_run_units: options.max_text_run_units,
                },
            )
            .map_err(HtmlError::from)?;
            crate::dvi::coordinates::compare_page(page, &positioned).map_err(HtmlError::from)?;
            Ok::<PositionedPage, HtmlError>(positioned)
        })
        .collect::<Result<Vec<_>, _>>()?;
    write_positioned_html(&positioned, resolver, options)
}

pub fn write_positioned_html<R: HtmlFontResolver>(
    pages: &[PositionedPage],
    resolver: &mut R,
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
    for page in pages {
        if page.events.len() > options.max_positioned_events {
            return Err(HtmlError::Positioned(PositionedError::TooManyEvents {
                limit: options.max_positioned_events,
            }));
        }
        for event in &page.events {
            if let PositionedEvent::TextRun(run) = event
                && run.units.len() > options.max_text_run_units
            {
                return Err(HtmlError::Positioned(PositionedError::TextRunTooLong {
                    limit: options.max_text_run_units,
                }));
            }
        }
    }
    let mut resolved = BTreeMap::<HtmlFontKey, ResolvedFont>::new();
    for page in pages {
        for font in &page.fonts {
            let key = HtmlFontKey::from(font);
            if resolved.contains_key(&key) {
                continue;
            }
            let web = resolver
                .resolve(font)
                .map_err(|message| HtmlError::FontResolution {
                    font: font.name.clone(),
                    message,
                })?;
            let checked = validate_font(font, web, options)?;
            resolved.insert(key, checked);
        }
    }
    let assets = build_assets(&resolved, options)?;
    let mut html = String::new();
    html.push_str("<!doctype html>\n<html lang=\"");
    escape_attr(&options.language, &mut html);
    html.push_str("\"><head><meta charset=\"utf-8\"><meta name=\"generator\" content=\"umber-html/1\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; font-src data: 'self'; style-src 'unsafe-inline'; img-src data:\"><title>");
    escape_text(&options.title, &mut html);
    html.push_str("</title><style>\n");
    check_html_size(&html, options)?;
    write_font_css(&mut html, &resolved, options)?;
    html.push_str(BASE_CSS);
    html.push_str("</style></head><body>\n<main class=\"umber-document\">\n");
    for page in pages {
        write_page(&mut html, page, &resolved, options)?;
        check_html_size(&html, options)?;
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

const BASE_CSS: &str = concat!(
    ".umber-document{margin:0;padding:0;background:#777}\n",
    ".umber-page{position:relative;contain:strict;overflow:hidden;background:#fff;margin:0 auto 1rem;isolation:isolate}\n",
    ".umber-page-content{position:absolute;width:0;height:0;overflow:visible}\n",
    ".umber-box{position:absolute;pointer-events:none}\n",
    ".umber-rule{position:absolute;background:currentColor}\n",
    ".umber-run{position:absolute;left:0;top:0;width:0;height:0;overflow:visible;white-space:pre;unicode-bidi:isolate-override;font-kerning:normal;font-variant-ligatures:common-ligatures;font-synthesis:none;font-optical-sizing:none}\n",
    ".umber-run-text{white-space:pre;fill:currentColor}\n",
    ".umber-baseline{fill:transparent;pointer-events:none}\n",
    ".umber-math{position:absolute;left:0;top:0;width:0;height:0;overflow:visible;color:currentColor}\n",
    ".umber-math-text,.umber-math-outline,.umber-math-rule{fill:currentColor}\n",
    ".umber-math-baseline{fill:transparent;pointer-events:none}\n",
    ".umber-special{position:absolute;width:0;height:0;overflow:hidden;pointer-events:none}\n",
    ".umber-a11y{position:absolute;width:1px;height:1px;padding:0;margin:-1px;overflow:hidden;clip:rect(0,0,0,0);white-space:nowrap;border:0}\n",
    "@media print{.umber-document{background:#fff}.umber-page{break-after:page;margin:0}}\n",
);

#[derive(Clone)]
struct ResolvedFont {
    web: WebFont,
    digest_hex: String,
    family: String,
    sfnt: Vec<u8>,
}

fn validate_font(
    font: &FontResource,
    web: WebFont,
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
    let digest: [u8; 32] = Sha256::digest(&web.woff2).into();
    if digest != web.sha256 {
        return Err(HtmlError::CorruptFontAsset {
            font: font.name.clone(),
        });
    }
    if let Some(opentype) = &font.opentype
        && (opentype.container != tex_fonts::FontContainer::Woff2
            || opentype.object_identity.bytes() != digest)
    {
        return Err(HtmlError::CorruptFontAsset {
            font: font.name.clone(),
        });
    }
    if let Some(opentype) = &font.opentype {
        let key = tex_fonts::FontRequestKey::new(
            "umber-html-validation",
            0,
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
                declared_object_sha256: Some(opentype.object_identity),
                declared_program_identity: Some(opentype.program_identity),
                provenance: None,
            },
            tex_fonts::FontLimits::default(),
        )
        .map_err(|_| HtmlError::CorruptFontAsset {
            font: font.name.clone(),
        })?;
    }
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
    let sfnt = woff2_patched::convert_woff2_to_ttf(&mut web.woff2.as_slice()).map_err(|_| {
        HtmlError::CorruptFontAsset {
            font: font.name.clone(),
        }
    })?;
    let face = ttf_parser::Face::parse(&sfnt, 0).map_err(|_| HtmlError::CorruptFontAsset {
        font: font.name.clone(),
    })?;
    for (code, mapping) in web.encoding.iter().enumerate() {
        for ch in mapping.iter().flat_map(|mapping| mapping.chars()) {
            if face.glyph_index(ch).is_none() {
                return Err(HtmlError::MissingFontGlyph {
                    font: font.name.clone(),
                    code: code as u8,
                    ch,
                });
            }
        }
    }
    let digest_hex = hex(&digest);
    let family_identity = font
        .opentype
        .as_ref()
        .map_or(digest, |font| font.program_identity.bytes());
    let family_hex = hex(&family_identity);
    let family = format!("umber-font-{}", &family_hex[..24]);
    Ok(ResolvedFont {
        web,
        digest_hex,
        family,
        sfnt,
    })
}

fn build_assets(
    resolved: &BTreeMap<HtmlFontKey, ResolvedFont>,
    options: &HtmlOptions,
) -> Result<Vec<HtmlAsset>, HtmlError> {
    let mut by_digest = BTreeMap::<String, HtmlAsset>::new();
    let mut total = 0usize;
    for font in resolved.values() {
        if by_digest.contains_key(&font.digest_hex) {
            continue;
        }
        total = total
            .checked_add(font.web.woff2.len())
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
                    path: format!("sha256-{}.woff2", font.digest_hex),
                    bytes: font.web.woff2.clone(),
                    sha256: font.web.sha256,
                    provenance: font.web.provenance.clone(),
                },
            );
        }
    }
    Ok(by_digest.into_values().collect())
}

fn write_font_css(
    out: &mut String,
    fonts: &BTreeMap<HtmlFontKey, ResolvedFont>,
    options: &HtmlOptions,
) -> Result<(), HtmlError> {
    for font in fonts.values() {
        out.push_str("@font-face{font-family:'");
        out.push_str(&font.family);
        out.push_str("';src:url('");
        match &options.asset_mode {
            AssetMode::Embedded => {
                let encoded = font
                    .web
                    .woff2
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
                base64(&font.web.woff2, out);
            }
            AssetMode::Manifest { relative_directory } => {
                out.push_str(relative_directory);
                if !relative_directory.ends_with('/') {
                    out.push('/');
                }
                out.push_str("sha256-");
                out.push_str(&font.digest_hex);
                out.push_str(".woff2");
            }
        }
        out.push_str("') format('woff2');font-display:block;font-style:normal;font-weight:400}\n");
        check_html_size(out, options)?;
    }
    Ok(())
}

fn write_page(
    out: &mut String,
    page: &PositionedPage,
    fonts: &BTreeMap<HtmlFontKey, ResolvedFont>,
    options: &HtmlOptions,
) -> Result<(), HtmlError> {
    out.push_str("<section class=\"umber-page\" aria-label=\"Page ");
    out.push_str(&page.page_index.to_string());
    out.push_str("\" data-umber-page=\"");
    out.push_str(&page.page_index.to_string());
    out.push_str("\" data-umber-revision=\"");
    out.push_str(&options.revision.to_string());
    out.push_str("\" data-umber-output=\"");
    out.push_str(&options.output_id.to_string());
    out.push('"');
    attr_sp(out, "width", page.width);
    attr_sp(out, "height", page.height);
    attr_sp(out, "origin-x", page.page_origin_x);
    attr_sp(out, "origin-y", page.page_origin_y);
    out.push_str(" data-umber-mag=\"");
    out.push_str(&page.mag.to_string());
    out.push_str("\" style=\"width:");
    css_px(out, page.width, page.mag);
    out.push_str(";height:");
    css_px(out, page.height, page.mag);
    out.push_str("\">\n<div class=\"umber-page-content\" style=\"left:");
    css_px(out, page.page_origin_x, page.mag);
    out.push_str(";top:");
    css_px(out, page.page_origin_y, page.mag);
    out.push_str("\">\n");
    let page_fonts = page
        .fonts
        .iter()
        .map(|font| (font.font_id, font))
        .collect::<BTreeMap<_, _>>();
    let mut accessible = String::new();
    let mut special_state = SpecialState::default();
    for (ordinal, event) in page.events.iter().enumerate() {
        match event {
            PositionedEvent::Box(event) => {
                out.push_str("<div class=\"umber-box\" aria-hidden=\"true\" data-umber-event=\"");
                out.push_str(&ordinal.to_string());
                out.push_str("\" data-umber-kind=\"");
                out.push_str(match event.kind {
                    BoxKind::Horizontal => "hbox",
                    BoxKind::Vertical => "vbox",
                });
                out.push('"');
                geometry_attrs(out, event.x, event.y, event.width, event.height);
                attr_sp(out, "baseline", event.baseline);
                geometry_style(out, event.x, event.y, event.width, event.height, page.mag);
                out.push_str("\"></div>\n");
            }
            PositionedEvent::Rule(event) => {
                out.push_str("<div class=\"umber-rule\" aria-hidden=\"true\" data-umber-event=\"");
                out.push_str(&ordinal.to_string());
                out.push('"');
                geometry_attrs(out, event.x, event.y, event.width, event.height);
                geometry_style(out, event.x, event.y, event.width, event.height, page.mag);
                if let Some(color) = special_state.color() {
                    out.push_str(";color:");
                    out.push_str(color);
                }
                out.push_str("\"></div>\n");
            }
            PositionedEvent::TextRun(event) => {
                let artifact_font =
                    page_fonts
                        .get(&event.font_id)
                        .ok_or(HtmlError::MissingPageFont {
                            page: page.page_index,
                            font_id: event.font_id,
                        })?;
                let font = fonts.get(&HtmlFontKey::from(*artifact_font)).ok_or(
                    HtmlError::MissingPageFont {
                        page: page.page_index,
                        font_id: event.font_id,
                    },
                )?;
                let text_budget = options
                    .max_html_bytes
                    .checked_sub(out.len())
                    .and_then(|remaining| remaining.checked_sub(accessible.len()))
                    .unwrap_or(0)
                    / 6;
                let text = map_text(event.units.as_slice(), font, text_budget)?;
                accessible.push_str(&text);
                out.push_str("<svg class=\"umber-run\" aria-hidden=\"true\" data-umber-event=\"");
                out.push_str(&ordinal.to_string());
                out.push('"');
                attr_sp(out, "x", event.x);
                attr_sp(out, "baseline", event.baseline);
                out.push_str(" data-umber-font=\"");
                out.push_str(&event.font_id.to_string());
                if let Some(opentype) = &artifact_font.opentype {
                    out.push_str("\" data-umber-face-index=\"");
                    out.push_str(&opentype.face_index.to_string());
                    if let Some(script) = opentype.script {
                        out.push_str("\" data-umber-script=\"");
                        escape_attr(&script.to_string(), out);
                    }
                }
                out.push_str("\" data-umber-codes=\"");
                write_codes(out, &event.units);
                out.push_str("\" style=\"font-family:'");
                out.push_str(&font.family);
                out.push_str("';font-size:");
                css_px(out, Scaled::from_raw(font.web.key.at_size_raw), page.mag);
                if let Some(opentype) = &artifact_font.opentype {
                    out.push_str(";font-feature-settings:");
                    write_feature_settings(out, &opentype.features);
                    out.push_str(";font-variation-settings:");
                    write_variation_settings(out, &opentype.variation);
                }
                if let Some(color) = special_state.color() {
                    out.push_str(";color:");
                    out.push_str(color);
                }
                out.push_str("\">");
                out.push_str("<rect class=\"umber-baseline\" x=\"");
                css_px(out, event.x, page.mag);
                out.push_str("\" y=\"");
                css_px(out, event.baseline, page.mag);
                out.push_str("\" width=\"1\" height=\"1\"></rect>");
                if let Some(link) = &special_state.link {
                    out.push_str("<a href=\"");
                    escape_attr(link, out);
                    out.push_str("\" rel=\"noreferrer noopener\">");
                }
                out.push_str("<text class=\"umber-run-text\" direction=\"");
                let direction =
                    artifact_font
                        .opentype
                        .as_ref()
                        .map_or("ltr", |font| match font.direction {
                            tex_fonts::WritingDirection::LeftToRight => "ltr",
                            tex_fonts::WritingDirection::RightToLeft => "rtl",
                        });
                out.push_str(direction);
                if let Some(language) = artifact_font
                    .opentype
                    .as_ref()
                    .and_then(|font| font.language.as_ref())
                {
                    out.push_str("\" lang=\"");
                    escape_attr(language.as_str(), out);
                }
                out.push_str("\" x=\"");
                let exact_character_positions = event.positions.len() == event.units.len()
                    && event.units.iter().all(|unit| match unit {
                        TextUnit::Space => true,
                        TextUnit::Code(code) => font.web.encoding[usize::from(*code)]
                            .as_ref()
                            .is_some_and(|mapping| mapping.chars().count() == 1),
                    });
                if exact_character_positions {
                    for (index, position) in event.positions.iter().enumerate() {
                        if index > 0 {
                            out.push(' ');
                        }
                        css_px(out, *position, page.mag);
                    }
                } else {
                    // A multi-scalar mapping represents one TeX unit. SVG cannot
                    // skip entries in an x-position list, so retain browser
                    // shaping for that exceptional run rather than assigning
                    // later scalars to positions belonging to subsequent units.
                    css_px(out, event.x, page.mag);
                }
                out.push_str("\" y=\"");
                css_px(out, event.baseline, page.mag);
                out.push_str("\">");
                escape_text(&text, out);
                out.push_str("</text>");
                if special_state.link.is_some() {
                    out.push_str("</a>");
                }
                out.push_str("</svg>\n");
            }
            PositionedEvent::Special(event) => {
                if event.payload.len() > options.max_special_bytes {
                    return Err(HtmlError::SpecialTooLarge {
                        bytes: event.payload.len(),
                        limit: options.max_special_bytes,
                    });
                }
                let interpreted = interpret_special(event)?;
                special_state.apply(&interpreted)?;
                out.push_str(
                    "<span class=\"umber-special\" aria-hidden=\"true\" data-umber-event=\"",
                );
                out.push_str(&ordinal.to_string());
                out.push('"');
                attr_sp(out, "x", event.x);
                attr_sp(out, "y", event.y);
                out.push_str(" data-umber-special-class=\"");
                escape_attr(&event.class, out);
                out.push_str("\" data-umber-special-hex=\"");
                out.push_str(&hex(&event.payload));
                match &interpreted {
                    InterpretedSpecial::Destination(id) => {
                        out.push_str("\" id=\"");
                        escape_attr(id, out);
                    }
                    InterpretedSpecial::Inert => {
                        out.push_str("\" data-umber-special-policy=\"inert");
                    }
                    _ => out.push_str("\" data-umber-special-policy=\"applied"),
                }
                out.push_str("\" style=\"left:");
                css_px(out, event.x, page.mag);
                out.push_str(";top:");
                css_px(out, event.y, page.mag);
                out.push_str("\"></span>\n");
            }
            PositionedEvent::PdfAccessibility(_) => {}
            PositionedEvent::PdfAnnotation(_)
            | PositionedEvent::PdfDestination(_)
            | PositionedEvent::PdfGraphics(_) => {}
            PositionedEvent::BoxEnd(_) => {}
            PositionedEvent::PdfThread(_) | PositionedEvent::PdfEndThread { .. } => {}
        }
        check_html_size(out, options)?;
    }
    if !special_state.colors.is_empty() || special_state.link.is_some() {
        return Err(HtmlError::InvalidSpecial {
            message: "unclosed color or link scope at page end".to_owned(),
        });
    }
    write_math(out, page, fonts, options)?;
    out.push_str("</div><div class=\"umber-a11y\">");
    escape_text(&accessible, out);
    out.push_str("</div></section>\n");
    Ok(())
}

fn write_feature_settings(out: &mut String, policy: &tex_fonts::FontFeaturePolicy) {
    if policy.settings().is_empty() {
        out.push_str("normal");
        return;
    }
    for (index, setting) in policy.settings().iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('\'');
        write_css_tag(out, setting.tag);
        out.push_str("' ");
        out.push_str(&setting.value.to_string());
    }
}

fn write_variation_settings(out: &mut String, selection: &tex_fonts::VariationSelection) {
    if selection.coordinates().is_empty() {
        out.push_str("normal");
        return;
    }
    for (index, coordinate) in selection.coordinates().iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('\'');
        write_css_tag(out, coordinate.tag);
        out.push_str("' ");
        let value = f64::from(coordinate.value) / 65_536.0;
        out.push_str(&value.to_string());
    }
}

fn write_css_tag(out: &mut String, tag: tex_fonts::OpenTypeTag) {
    for byte in tag.bytes() {
        match byte {
            b'\'' => out.push_str("\\27 "),
            b'\\' => out.push_str("\\5c "),
            _ => out.push(char::from(byte)),
        }
    }
}

fn write_math(
    out: &mut String,
    page: &PositionedPage,
    fonts: &BTreeMap<HtmlFontKey, ResolvedFont>,
    options: &HtmlOptions,
) -> Result<(), HtmlError> {
    let by_instance = page
        .fonts
        .iter()
        .filter_map(|artifact| {
            let opentype = artifact.opentype.as_ref()?;
            fonts
                .get(&HtmlFontKey::from(artifact))
                .map(|font| (opentype.instance_identity, (font, opentype)))
        })
        .collect::<BTreeMap<_, _>>();
    let mut active: Option<MathStart> = None;
    for (ordinal, event) in page.math_events.iter().enumerate() {
        match event {
            MathOutputEvent::Start(start) if active.is_none() => {
                active = Some(*start);
                out.push_str("<svg class=\"umber-math\" aria-hidden=\"true\" data-umber-math=\"");
                out.push_str(&start.id.to_string());
                out.push('"');
                attr_sp(out, "x", start.x);
                attr_sp(out, "baseline", start.baseline);
                attr_sp(out, "width", start.width);
                attr_sp(out, "height", start.height);
                attr_sp(out, "depth", start.depth);
                out.push_str("><rect class=\"umber-math-baseline\" x=\"");
                css_px(out, start.x, page.mag);
                out.push_str("\" y=\"");
                css_px(out, start.baseline, page.mag);
                out.push_str("\" width=\"1\" height=\"1\"></rect>");
            }
            MathOutputEvent::Glyph(glyph) if active.is_some() => {
                let (font, opentype) = by_instance
                    .get(&glyph.font_instance)
                    .copied()
                    .ok_or(HtmlError::MissingMathFontInstance)?;
                write_math_glyph(out, glyph, font, opentype, page.mag, ordinal)?;
            }
            MathOutputEvent::Rule(rule) if active.is_some() => {
                out.push_str("<rect class=\"umber-math-rule\" data-umber-math-event=\"");
                out.push_str(&ordinal.to_string());
                out.push('"');
                geometry_attrs(out, rule.x, rule.y, rule.width, rule.height);
                out.push_str(" x=\"");
                css_px(out, rule.x, page.mag);
                out.push_str("\" y=\"");
                css_px(out, rule.y, page.mag);
                out.push_str("\" width=\"");
                css_px(out, rule.width, page.mag);
                out.push_str("\" height=\"");
                css_px(out, rule.height, page.mag);
                out.push_str("\"></rect>");
            }
            MathOutputEvent::End if active.take().is_some() => out.push_str("</svg>\n"),
            _ => return Err(HtmlError::InvalidMathEventSequence),
        }
        check_html_size(out, options)?;
    }
    if active.is_some() {
        return Err(HtmlError::InvalidMathEventSequence);
    }
    Ok(())
}

fn write_math_glyph(
    out: &mut String,
    glyph: &MathGlyph,
    font: &ResolvedFont,
    opentype: &crate::OpenTypeFontResource,
    mag: i32,
    ordinal: usize,
) -> Result<(), HtmlError> {
    out.push_str("<g class=\"umber-math-glyph\" data-umber-math-event=\"");
    out.push_str(&ordinal.to_string());
    out.push_str("\" data-umber-glyph-id=\"");
    out.push_str(&glyph.glyph_id.to_string());
    out.push_str("\" data-umber-font-instance=\"");
    out.push_str(&hex(&glyph.font_instance.bytes()));
    out.push_str("\" data-umber-ssty=\"");
    out.push_str(&glyph.ssty.to_string());
    out.push('"');
    attr_sp(out, "x", glyph.x);
    attr_sp(out, "baseline", glyph.baseline);
    attr_sp(out, "width", glyph.width);
    attr_sp(out, "height", glyph.height);
    attr_sp(out, "depth", glyph.depth);
    out.push('>');
    match glyph.selection {
        MathGlyphSelection::Cmap { scalar } => {
            let ch = char::from_u32(scalar).ok_or(HtmlError::MathGlyphMismatch {
                glyph_id: glyph.glyph_id,
            })?;
            if selected_glyph(font, opentype, ch, glyph.ssty) != Some(glyph.glyph_id) {
                return Err(HtmlError::MathGlyphMismatch {
                    glyph_id: glyph.glyph_id,
                });
            }
            out.push_str("<text class=\"umber-math-text\" direction=\"ltr\" x=\"");
            css_px(out, glyph.x, mag);
            out.push_str("\" y=\"");
            css_px(out, glyph.baseline, mag);
            out.push_str("\" style=\"font-family:'");
            out.push_str(&font.family);
            out.push_str("';font-size:");
            css_px(out, Scaled::from_raw(font.web.key.at_size_raw), mag);
            out.push_str(";font-feature-settings:'ssty' ");
            out.push_str(&glyph.ssty.to_string());
            out.push_str(";font-variation-settings:");
            write_variation_settings(out, &opentype.variation);
            out.push_str("\">");
            escape_text(&ch.to_string(), out);
            out.push_str("</text>");
        }
        MathGlyphSelection::OutlineFallback => {
            let (path, units_per_em) = outline_path(font, opentype, glyph.glyph_id)?;
            out.push_str("<path class=\"umber-math-outline\" d=\"");
            out.push_str(&path);
            out.push_str("\" transform=\"translate(");
            css_number(out, glyph.x, mag, 1);
            out.push(' ');
            css_number(out, glyph.baseline, mag, 1);
            out.push_str(") scale(");
            css_number(
                out,
                Scaled::from_raw(font.web.key.at_size_raw),
                mag,
                i128::from(units_per_em),
            );
            out.push(' ');
            css_number(
                out,
                Scaled::from_raw(-font.web.key.at_size_raw),
                mag,
                i128::from(units_per_em),
            );
            out.push_str(")\"></path>");
        }
    }
    out.push_str("</g>");
    Ok(())
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

fn check_html_size(out: &str, options: &HtmlOptions) -> Result<(), HtmlError> {
    if out.len() > options.max_html_bytes {
        Err(HtmlError::HtmlTooLarge {
            bytes: out.len(),
            limit: options.max_html_bytes,
        })
    } else {
        Ok(())
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
                let mapping = font.web.encoding[usize::from(*code)].as_ref().ok_or(
                    HtmlError::MissingTextMapping {
                        font: font.web.key.name.clone(),
                        code: *code,
                    },
                )?;
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

fn geometry_attrs(out: &mut String, x: Scaled, y: Scaled, width: Scaled, height: Scaled) {
    attr_sp(out, "x", x);
    attr_sp(out, "y", y);
    attr_sp(out, "width", width);
    attr_sp(out, "height", height);
}

fn attr_sp(out: &mut String, name: &str, value: Scaled) {
    out.push_str(" data-umber-");
    out.push_str(name);
    out.push_str("-sp=\"");
    out.push_str(&value.raw().to_string());
    out.push('"');
}

fn geometry_style(out: &mut String, x: Scaled, y: Scaled, width: Scaled, height: Scaled, mag: i32) {
    out.push_str(" style=\"left:");
    css_px(out, x, mag);
    out.push_str(";top:");
    css_px(out, y, mag);
    out.push_str(";width:");
    css_px(out, width, mag);
    out.push_str(";height:");
    css_px(out, height, mag);
}

fn css_px(out: &mut String, value: Scaled, mag: i32) {
    css_number(out, value, mag, 1);
    out.push_str("px");
}

fn css_number(out: &mut String, value: Scaled, mag: i32, extra_denominator: i128) {
    const DENOMINATOR: i128 = 65_536 * 5 * 7_227;
    const PLACES: i128 = 100_000_000;
    let numerator = i128::from(value.raw()) * i128::from(mag) * 48;
    let negative = numerator < 0;
    let magnitude = numerator.abs();
    let denominator = DENOMINATOR * extra_denominator;
    let mut scaled = magnitude * PLACES / denominator;
    let remainder = magnitude * PLACES % denominator;
    if remainder * 2 >= denominator {
        scaled += 1;
    }
    if negative && scaled != 0 {
        out.push('-');
    }
    out.push_str(&(scaled / PLACES).to_string());
    out.push('.');
    let fraction = (scaled % PLACES).to_string();
    for _ in fraction.len()..8 {
        out.push('0');
    }
    out.push_str(&fraction);
}

fn write_codes(out: &mut String, units: &[TextUnit]) {
    for (index, unit) in units.iter().enumerate() {
        if index != 0 {
            out.push(',');
        }
        match unit {
            TextUnit::Code(code) => {
                out.push_str("0x");
                out.push_str(&format!("{code:02x}"));
            }
            TextUnit::Space => out.push_str("space"),
        }
    }
}

fn escape_text(value: &str, out: &mut String) {
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
}

fn escape_attr(value: &str, out: &mut String) {
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push(DIGITS[usize::from(byte >> 4)] as char);
        value.push(DIGITS[usize::from(byte & 15)] as char);
    }
    value
}

fn base64(bytes: &[u8], out: &mut String) {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = *chunk.get(1).unwrap_or(&0);
        let c = *chunk.get(2).unwrap_or(&0);
        out.push(TABLE[usize::from(a >> 2)] as char);
        out.push(TABLE[usize::from((a & 3) << 4 | b >> 4)] as char);
        if chunk.len() > 1 {
            out.push(TABLE[usize::from((b & 15) << 2 | c >> 6)] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[usize::from(c & 63)] as char);
        } else {
            out.push('=');
        }
    }
}
