//! Deterministic standalone HTML serialization over positioned pages.

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};
use tex_arith::Scaled;

use crate::positioned::{
    BoxKind, PositionedError, PositionedEvent, PositionedPage, TextUnit, lower_page,
};
use crate::{ContentHash, FontResource, PageArtifact};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct HtmlFontKey {
    pub name: String,
    pub tfm_content_hash: ContentHash,
    pub tfm_checksum: u32,
    pub design_size_raw: i32,
    pub at_size_raw: i32,
}

impl From<&FontResource> for HtmlFontKey {
    fn from(font: &FontResource) -> Self {
        Self {
            name: font.name.clone(),
            tfm_content_hash: font.tfm_content_hash,
            tfm_checksum: font.tfm_checksum,
            design_size_raw: font.design_size.raw(),
            at_size_raw: font.at_size.raw(),
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HtmlOptions {
    pub title: String,
    pub language: String,
    pub asset_mode: AssetMode,
    pub max_pages: usize,
    pub max_html_bytes: usize,
    pub max_asset_bytes: usize,
    pub max_total_asset_bytes: usize,
    pub max_special_bytes: usize,
}

impl Default for HtmlOptions {
    fn default() -> Self {
        Self {
            title: "Umber document".to_owned(),
            language: "und".to_owned(),
            asset_mode: AssetMode::Embedded,
            max_pages: 16_384,
            max_html_bytes: 256 * 1024 * 1024,
            max_asset_bytes: 64 * 1024 * 1024,
            max_total_asset_bytes: 256 * 1024 * 1024,
            max_special_bytes: 4 * 1024,
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
    MissingPageFont { page: u32, font_id: u32 },
    FontResolution { font: String, message: String },
    FontKeyMismatch { font: String },
    InvalidEncodingLength { font: String, count: usize },
    MissingTextMapping { font: String, code: u8 },
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
        }
    }
}

impl std::error::Error for HtmlError {}

impl From<PositionedError> for HtmlError {
    fn from(value: PositionedError) -> Self {
        Self::Positioned(value)
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
            lower_page(page, page_index).map_err(HtmlError::from)
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
    validate_options(options)?;
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
    write_font_css(&mut html, &resolved, options);
    html.push_str(BASE_CSS);
    html.push_str("</style></head><body>\n<main class=\"umber-document\">\n");
    for page in pages {
        write_page(&mut html, page, &resolved, options)?;
        if html.len() > options.max_html_bytes {
            return Err(HtmlError::HtmlTooLarge {
                bytes: html.len(),
                limit: options.max_html_bytes,
            });
        }
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
    ".umber-box{position:absolute;pointer-events:none}\n",
    ".umber-rule{position:absolute;background:currentColor}\n",
    ".umber-run{position:absolute;height:0;line-height:0;white-space:pre;text-wrap:nowrap;unicode-bidi:isolate;font-kerning:normal;font-variant-ligatures:common-ligatures;font-synthesis:none;font-optical-sizing:none}\n",
    ".umber-run-text{line-height:normal;vertical-align:baseline}\n",
    ".umber-baseline{display:inline-block;width:0;height:0;padding:0;margin:0;vertical-align:baseline}\n",
    ".umber-special{position:absolute;width:0;height:0;overflow:hidden;pointer-events:none}\n",
    ".umber-a11y{position:absolute;width:1px;height:1px;padding:0;margin:-1px;overflow:hidden;clip:rect(0,0,0,0);white-space:nowrap;border:0}\n",
    "@media print{.umber-document{background:#fff}.umber-page{break-after:page;margin:0}}\n",
);

#[derive(Clone)]
struct ResolvedFont {
    web: WebFont,
    digest_hex: String,
    family: String,
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
    let digest_hex = hex(&digest);
    let family = format!("umber-font-{}", &digest_hex[..24]);
    Ok(ResolvedFont {
        web,
        digest_hex,
        family,
    })
}

fn build_assets(
    resolved: &BTreeMap<HtmlFontKey, ResolvedFont>,
    options: &HtmlOptions,
) -> Result<Vec<HtmlAsset>, HtmlError> {
    if matches!(options.asset_mode, AssetMode::Embedded) {
        return Ok(Vec::new());
    }
    let mut by_digest = BTreeMap::<String, HtmlAsset>::new();
    let mut total = 0usize;
    for font in resolved.values() {
        if by_digest.contains_key(&font.digest_hex) {
            continue;
        }
        total = total.saturating_add(font.web.woff2.len());
        if total > options.max_total_asset_bytes {
            return Err(HtmlError::AssetsTooLarge {
                bytes: total,
                limit: options.max_total_asset_bytes,
            });
        }
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
    Ok(by_digest.into_values().collect())
}

fn write_font_css(
    out: &mut String,
    fonts: &BTreeMap<HtmlFontKey, ResolvedFont>,
    options: &HtmlOptions,
) {
    for font in fonts.values() {
        out.push_str("@font-face{font-family:'");
        out.push_str(&font.family);
        out.push_str("';src:url('");
        match &options.asset_mode {
            AssetMode::Embedded => {
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
    }
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
    attr_sp(out, "width", page.width);
    attr_sp(out, "height", page.height);
    out.push_str(" data-umber-mag=\"");
    out.push_str(&page.mag.to_string());
    out.push_str("\" style=\"width:");
    css_px(out, page.width, page.mag);
    out.push_str(";height:");
    css_px(out, page.height, page.mag);
    out.push_str("\">\n");
    let page_fonts = page
        .fonts
        .iter()
        .map(|font| (font.font_id, font))
        .collect::<BTreeMap<_, _>>();
    let mut accessible = String::new();
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
                geometry_attrs(out, event.x, event.y, event.width, event.height);
                attr_sp(out, "baseline", event.baseline);
                geometry_style(out, event.x, event.y, event.width, event.height, page.mag);
                out.push_str("\"></div>\n");
            }
            PositionedEvent::Rule(event) => {
                out.push_str("<div class=\"umber-rule\" aria-hidden=\"true\" data-umber-event=\"");
                out.push_str(&ordinal.to_string());
                geometry_attrs(out, event.x, event.y, event.width, event.height);
                geometry_style(out, event.x, event.y, event.width, event.height, page.mag);
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
                let text = map_text(event.units.as_slice(), font)?;
                accessible.push_str(&text);
                out.push_str("<span class=\"umber-run\" aria-hidden=\"true\" dir=\"ltr\" data-umber-event=\"");
                out.push_str(&ordinal.to_string());
                attr_sp(out, "x", event.x);
                attr_sp(out, "baseline", event.baseline);
                out.push_str(" data-umber-font=\"");
                out.push_str(&event.font_id.to_string());
                out.push_str("\" data-umber-codes=\"");
                write_codes(out, &event.units);
                out.push_str("\" style=\"left:");
                css_px(out, event.x, page.mag);
                out.push_str(";top:");
                css_px(out, event.baseline, page.mag);
                out.push_str(";font-family:'");
                out.push_str(&font.family);
                out.push_str("';font-size:");
                css_px(out, Scaled::from_raw(font.web.key.at_size_raw), page.mag);
                out.push_str("\"><span class=\"umber-run-text\">");
                escape_text(&text, out);
                out.push_str("<i class=\"umber-baseline\"></i></span></span>\n");
            }
            PositionedEvent::Special(event) => {
                if event.payload.len() > options.max_special_bytes {
                    return Err(HtmlError::SpecialTooLarge {
                        bytes: event.payload.len(),
                        limit: options.max_special_bytes,
                    });
                }
                out.push_str(
                    "<span class=\"umber-special\" aria-hidden=\"true\" data-umber-event=\"",
                );
                out.push_str(&ordinal.to_string());
                attr_sp(out, "x", event.x);
                attr_sp(out, "y", event.y);
                out.push_str(" data-umber-special-class=\"");
                escape_attr(&event.class, out);
                out.push_str("\" data-umber-special-hex=\"");
                out.push_str(&hex(&event.payload));
                out.push_str("\" style=\"left:");
                css_px(out, event.x, page.mag);
                out.push_str(";top:");
                css_px(out, event.y, page.mag);
                out.push_str("\"></span>\n");
            }
        }
    }
    out.push_str("<div class=\"umber-a11y\">");
    escape_text(&accessible, out);
    out.push_str("</div></section>\n");
    Ok(())
}

fn map_text(units: &[TextUnit], font: &ResolvedFont) -> Result<String, HtmlError> {
    let mut text = String::new();
    for unit in units {
        match unit {
            TextUnit::Space => text.push(' '),
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
                text.push_str(mapping);
            }
        }
    }
    Ok(text)
}

fn validate_options(options: &HtmlOptions) -> Result<(), HtmlError> {
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
    const DENOMINATOR: i128 = 65_536 * 5 * 7_227;
    const PLACES: i128 = 100_000_000;
    let numerator = i128::from(value.raw()) * i128::from(mag) * 48;
    let negative = numerator < 0;
    let magnitude = numerator.abs();
    let mut scaled = magnitude * PLACES / DENOMINATOR;
    let remainder = magnitude * PLACES % DENOMINATOR;
    if remainder * 2 >= DENOMINATOR {
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
    out.push_str("px");
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
