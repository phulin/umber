//! Canonical keyed HTML render revisions.
//!
//! This model is downstream of committed page artifacts. It contains only the
//! typed values needed by the HTML driver and is shared by full snapshots and
//! incremental patch planning.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use tex_arith::Scaled;

mod digest;
mod patch;
mod protocol;

use digest::{derive_key, node_value_digest, page_digest, page_match_digest, revision_digest};
pub use patch::{
    PatchApplyError, PatchLimits, PatchOp, PatchPlan, PatchPlanError, RenderPageHeader,
    apply_patch, plan_patch,
};
pub use protocol::{
    PATCH_CAP_TYPED_DOM, PATCH_SCHEMA_VERSION, PatchCounts, PatchDelivery, PatchEnvelope,
    PatchProtocolError, ProtocolLimits, validate_delivery,
};

use super::{
    HtmlError, HtmlFontAssets, HtmlFontKey, HtmlOptions, InterpretedSpecial, ResolvedFont,
    SpecialState, accessible_line, interpret_special, map_text, validate_font,
    write_positioned_html,
};
use crate::positioned::{
    BoxKind, PositionedEvent, PositionedLimits, PositionedPage, TextUnit, lower_page_with_limits,
};
use crate::{MathGlyph, MathOutputEvent, MathRule, MathStart, PageArtifact};

pub const RENDER_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RenderSessionId([u8; 16]);

impl RenderSessionId {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(self) -> [u8; 16] {
        self.0
    }

    #[must_use]
    pub fn hex(self) -> String {
        hex(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RenderKey([u8; 16]);

impl RenderKey {
    pub const ROOT: Self = Self([0; 16]);

    #[must_use]
    pub const fn as_bytes(self) -> [u8; 16] {
        self.0
    }

    #[must_use]
    pub fn hex(self) -> String {
        hex(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RenderDigest([u8; 32]);

impl RenderDigest {
    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }

    #[must_use]
    pub fn hex(self) -> String {
        hex(&self.0)
    }

    #[must_use]
    pub fn parse_hex(value: &str) -> Option<Self> {
        parse_hex(value).map(Self)
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push(char::from(DIGITS[usize::from(byte >> 4)]));
        value.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    value
}

fn parse_hex<const N: usize>(value: &str) -> Option<[u8; N]> {
    if value.len() != N * 2 {
        return None;
    }
    let mut bytes = [0; N];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let offset = index * 2;
        let high = hex_nibble(value.as_bytes()[offset])?;
        let low = hex_nibble(value.as_bytes()[offset + 1])?;
        *byte = (high << 4) | low;
    }
    Some(bytes)
}

const fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderLimits {
    pub max_pages: usize,
    pub max_nodes: usize,
    pub max_resources: usize,
    pub max_resource_bytes: usize,
}

impl Default for RenderLimits {
    fn default() -> Self {
        Self {
            max_pages: 16_384,
            max_nodes: 1_000_000,
            max_resources: 65_536,
            max_resource_bytes: 256 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderRevision {
    pub schema_version: u16,
    pub session_id: RenderSessionId,
    pub revision: u64,
    pub title: String,
    pub language: String,
    pub pages: Vec<RenderPage>,
    pub resources: Vec<RenderResource>,
    pub digest: RenderDigest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderPage {
    pub key: RenderKey,
    pub digest: RenderDigest,
    pub match_digest: RenderDigest,
    pub ordinal: u32,
    pub width: Scaled,
    pub height: Scaled,
    pub origin_x: Scaled,
    pub origin_y: Scaled,
    pub mag: i32,
    pub counts: [i32; 10],
    pub nodes: Vec<RenderNode>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderNode {
    pub key: RenderKey,
    pub digest: RenderDigest,
    pub match_digest: RenderDigest,
    pub value: RenderNodeValue,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderNodeValue {
    Box(RenderBox),
    Rule(RenderRule),
    Text(Box<RenderText>),
    Special(RenderSpecial),
    MathStart(MathStart),
    MathGlyph(MathGlyph),
    MathRule(MathRule),
    MathEnd,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderBox {
    pub id: u32,
    pub kind: BoxKind,
    pub x: Scaled,
    pub y: Scaled,
    pub width: Scaled,
    pub height: Scaled,
    pub baseline: Scaled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderRule {
    pub x: Scaled,
    pub y: Scaled,
    pub width: Scaled,
    pub height: Scaled,
    pub color: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderText {
    pub x: Scaled,
    pub baseline: Scaled,
    pub positions: Vec<Scaled>,
    pub units: Vec<TextUnit>,
    pub text: String,
    pub font: HtmlFontKey,
    pub family: String,
    pub resource: [u8; 32],
    pub direction: RenderDirection,
    pub script: Option<[u8; 4]>,
    pub language: Option<String>,
    pub features: Vec<([u8; 4], u32)>,
    pub variations: Vec<([u8; 4], i32)>,
    pub color: Option<String>,
    pub link: Option<String>,
    pub accessibility_line: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderDirection {
    LeftToRight,
    RightToLeft,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderSpecial {
    pub x: Scaled,
    pub y: Scaled,
    pub class: String,
    pub payload: Vec<u8>,
    pub action: RenderSpecialAction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderSpecialAction {
    ColorPush(String),
    ColorPop,
    LinkStart(String),
    LinkEnd,
    Destination(String),
    Inert,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderResource {
    pub identity: [u8; 32],
    pub bytes: Vec<u8>,
    pub family: String,
    pub provenance: String,
}

#[derive(Debug)]
pub enum RenderBuildError {
    Html(HtmlError),
    RevisionNotMonotonic { previous: u64, target: u64 },
    SessionMismatch,
    TooManyNodes { count: usize, limit: usize },
    TooManyResources { count: usize, limit: usize },
    ResourcesTooLarge { bytes: usize, limit: usize },
}

impl std::fmt::Display for RenderBuildError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Html(error) => error.fmt(formatter),
            Self::RevisionNotMonotonic { previous, target } => write!(
                formatter,
                "render revision {target} does not follow accepted revision {previous}"
            ),
            Self::SessionMismatch => {
                formatter.write_str("render revision belongs to another session")
            }
            Self::TooManyNodes { count, limit } => {
                write!(
                    formatter,
                    "render tree has {count} nodes, exceeding limit {limit}"
                )
            }
            Self::TooManyResources { count, limit } => write!(
                formatter,
                "render tree has {count} resources, exceeding limit {limit}"
            ),
            Self::ResourcesTooLarge { bytes, limit } => write!(
                formatter,
                "render resources require {bytes} bytes, exceeding limit {limit}"
            ),
        }
    }
}

impl std::error::Error for RenderBuildError {}

impl From<HtmlError> for RenderBuildError {
    fn from(value: HtmlError) -> Self {
        Self::Html(value)
    }
}

pub fn build_render_revision<R: HtmlFontAssets>(
    artifacts: &[PageArtifact],
    assets: &R,
    options: &HtmlOptions,
    session_id: RenderSessionId,
    revision: u64,
    previous: Option<&RenderRevision>,
    limits: RenderLimits,
) -> Result<RenderRevision, RenderBuildError> {
    let pages = artifacts
        .iter()
        .enumerate()
        .map(|(index, page)| {
            let page_index = u32::try_from(index + 1).map_err(|_| HtmlError::TooManyPages {
                count: artifacts.len(),
                limit: u32::MAX as usize,
            })?;
            lower_page_with_limits(
                page,
                page_index,
                PositionedLimits {
                    max_events: options.max_positioned_events,
                    max_depth: options.max_positioned_depth,
                    max_run_units: options.max_text_run_units,
                },
            )
            .map_err(HtmlError::from)
        })
        .collect::<Result<Vec<_>, _>>()?;
    build_positioned_render_revision(
        &pages, assets, options, session_id, revision, previous, limits,
    )
}

pub fn build_positioned_render_revision<R: HtmlFontAssets>(
    pages: &[PositionedPage],
    assets: &R,
    options: &HtmlOptions,
    session_id: RenderSessionId,
    revision: u64,
    previous: Option<&RenderRevision>,
    limits: RenderLimits,
) -> Result<RenderRevision, RenderBuildError> {
    // Keep the canonical model and standalone serializer on one validation path.
    let _ = write_positioned_html(pages, assets, options)?;
    if let Some(previous) = previous {
        if previous.session_id != session_id {
            return Err(RenderBuildError::SessionMismatch);
        }
        if revision != previous.revision.saturating_add(1) {
            return Err(RenderBuildError::RevisionNotMonotonic {
                previous: previous.revision,
                target: revision,
            });
        }
    }
    let mut resolved = BTreeMap::<HtmlFontKey, ResolvedFont>::new();
    let mut resources = BTreeMap::<[u8; 32], RenderResource>::new();
    let mut resource_bytes = 0usize;
    for page in pages {
        for font in &page.fonts {
            let key = HtmlFontKey::from(font);
            if resolved.contains_key(&key) {
                continue;
            }
            let web = assets
                .font_asset(font)
                .map_err(|message| HtmlError::FontResolution {
                    font: font.name.clone(),
                    message,
                })?;
            let checked = validate_font(font, web, options)?;
            resource_bytes = resource_bytes.saturating_add(checked.web.woff2.len());
            resources
                .entry(checked.web.sha256)
                .or_insert_with(|| RenderResource {
                    identity: checked.web.sha256,
                    bytes: checked.web.woff2.clone(),
                    family: checked.family.clone(),
                    provenance: checked.web.provenance.clone(),
                });
            resolved.insert(key, checked);
        }
    }
    if resources.len() > limits.max_resources {
        return Err(RenderBuildError::TooManyResources {
            count: resources.len(),
            limit: limits.max_resources,
        });
    }
    if resource_bytes > limits.max_resource_bytes {
        return Err(RenderBuildError::ResourcesTooLarge {
            bytes: resource_bytes,
            limit: limits.max_resource_bytes,
        });
    }
    let mut built = Vec::with_capacity(pages.len());
    let mut node_count = 0usize;
    for page in pages {
        let rendered = render_page(page, &resolved, revision)?;
        node_count = node_count.saturating_add(rendered.nodes.len());
        if node_count > limits.max_nodes {
            return Err(RenderBuildError::TooManyNodes {
                count: node_count,
                limit: limits.max_nodes,
            });
        }
        built.push(rendered);
    }
    reuse_page_keys(previous.map(|old| old.pages.as_slice()), &mut built);
    for page in &mut built {
        for (index, node) in page.nodes.iter_mut().enumerate() {
            node.key = derive_key(page.key, node.digest, index as u64, revision);
        }
        let old = previous.and_then(|revision| {
            revision
                .pages
                .iter()
                .find(|candidate| candidate.key == page.key)
        });
        reuse_node_keys(old.map(|page| page.nodes.as_slice()), &mut page.nodes);
        page.digest = page_digest(page);
    }
    let resources = resources.into_values().collect::<Vec<_>>();
    let digest = revision_digest(&options.title, &options.language, &built, &resources);
    Ok(RenderRevision {
        schema_version: RENDER_SCHEMA_VERSION,
        session_id,
        revision,
        title: options.title.clone(),
        language: options.language.clone(),
        pages: built,
        resources,
        digest,
    })
}

fn render_page(
    page: &PositionedPage,
    fonts: &BTreeMap<HtmlFontKey, ResolvedFont>,
    revision: u64,
) -> Result<RenderPage, HtmlError> {
    let page_fonts = page
        .fonts
        .iter()
        .map(|font| (font.font_id, font))
        .collect::<BTreeMap<_, _>>();
    let mut values = Vec::new();
    let mut special_state = SpecialState::default();
    let mut box_stack = Vec::new();
    for event in &page.events {
        let value = match event {
            PositionedEvent::Box(value) => {
                box_stack.push((value.id, value.kind));
                Some(RenderNodeValue::Box(RenderBox {
                    id: value.id,
                    kind: value.kind,
                    x: value.x,
                    y: value.y,
                    width: value.width,
                    height: value.height,
                    baseline: value.baseline,
                }))
            }
            PositionedEvent::BoxEnd(value) => {
                debug_assert_eq!(box_stack.pop().map(|(id, _)| id), Some(value.id));
                None
            }
            PositionedEvent::Rule(value) => Some(RenderNodeValue::Rule(RenderRule {
                x: value.x,
                y: value.y,
                width: value.width,
                height: value.height,
                color: special_state.color().map(str::to_owned),
            })),
            PositionedEvent::TextRun(value) => {
                let artifact_font =
                    page_fonts
                        .get(&value.font_id)
                        .ok_or(HtmlError::MissingPageFont {
                            page: page.page_index,
                            font_id: value.font_id,
                        })?;
                let font = fonts.get(&HtmlFontKey::from(*artifact_font)).ok_or(
                    HtmlError::MissingPageFont {
                        page: page.page_index,
                        font_id: value.font_id,
                    },
                )?;
                let mapped = artifact_font
                    .opentype
                    .as_ref()
                    .is_none_or(|font| font.encoding_map_version.is_some());
                let text = map_text(&value.units, font, mapped, usize::MAX)?;
                let opentype = artifact_font.opentype.as_ref();
                Some(RenderNodeValue::Text(Box::new(RenderText {
                    x: value.x,
                    baseline: value.baseline,
                    positions: value.positions.clone(),
                    units: value.units.clone(),
                    text,
                    font: HtmlFontKey::from(*artifact_font),
                    family: font.family.clone(),
                    resource: font.web.sha256,
                    direction: if opentype.is_some_and(|font| {
                        font.direction == tex_fonts::WritingDirection::RightToLeft
                    }) {
                        RenderDirection::RightToLeft
                    } else {
                        RenderDirection::LeftToRight
                    },
                    script: opentype.and_then(|font| font.script.map(|tag| tag.bytes())),
                    language: opentype
                        .and_then(|font| font.language.as_ref())
                        .map(|language| language.as_str().to_owned()),
                    features: opentype.map_or_else(Vec::new, |font| {
                        font.features
                            .settings()
                            .iter()
                            .map(|setting| (setting.tag.bytes(), setting.value))
                            .collect()
                    }),
                    variations: opentype.map_or_else(Vec::new, |font| {
                        font.variation
                            .coordinates()
                            .iter()
                            .map(|coordinate| (coordinate.tag.bytes(), coordinate.value))
                            .collect()
                    }),
                    color: special_state.color().map(str::to_owned),
                    link: special_state.link.clone(),
                    accessibility_line: accessible_line(&box_stack),
                })))
            }
            PositionedEvent::Special(value) => {
                let interpreted = interpret_special(value)?;
                let action = match &interpreted {
                    InterpretedSpecial::ColorPush(value) => {
                        RenderSpecialAction::ColorPush(value.clone())
                    }
                    InterpretedSpecial::ColorPop => RenderSpecialAction::ColorPop,
                    InterpretedSpecial::LinkStart(value) => {
                        RenderSpecialAction::LinkStart(value.clone())
                    }
                    InterpretedSpecial::LinkEnd => RenderSpecialAction::LinkEnd,
                    InterpretedSpecial::Destination(value) => {
                        RenderSpecialAction::Destination(value.clone())
                    }
                    InterpretedSpecial::Inert => RenderSpecialAction::Inert,
                };
                special_state.apply(&interpreted)?;
                Some(RenderNodeValue::Special(RenderSpecial {
                    x: value.x,
                    y: value.y,
                    class: value.class.clone(),
                    payload: value.payload.clone(),
                    action,
                }))
            }
            PositionedEvent::PdfAccessibility(_)
            | PositionedEvent::PdfAnnotation(_)
            | PositionedEvent::PdfGraphics(_)
            | PositionedEvent::PdfDestination(_)
            | PositionedEvent::PdfThread(_)
            | PositionedEvent::PdfEndThread { .. } => None,
        };
        if let Some(value) = value {
            values.push(value);
        }
    }
    for event in &page.math_events {
        values.push(match event {
            MathOutputEvent::Start(value) => RenderNodeValue::MathStart(*value),
            MathOutputEvent::Glyph(value) => RenderNodeValue::MathGlyph(*value),
            MathOutputEvent::Rule(value) => RenderNodeValue::MathRule(*value),
            MathOutputEvent::End => RenderNodeValue::MathEnd,
        });
    }
    let nodes = values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let digest = node_value_digest(&value, false);
            let match_digest = node_value_digest(&value, true);
            RenderNode {
                key: derive_key(RenderKey::ROOT, digest, index as u64, revision),
                digest,
                match_digest,
                value,
            }
        })
        .collect::<Vec<_>>();
    let mut rendered = RenderPage {
        key: derive_key(
            RenderKey::ROOT,
            page_match_digest(page, &nodes),
            0,
            revision,
        ),
        digest: RenderDigest([0; 32]),
        match_digest: page_match_digest(page, &nodes),
        ordinal: page.page_index,
        width: page.width,
        height: page.height,
        origin_x: page.page_origin_x,
        origin_y: page.page_origin_y,
        mag: page.mag,
        counts: page.counts,
        nodes,
    };
    rendered.digest = page_digest(&rendered);
    Ok(rendered)
}

fn reuse_page_keys(old: Option<&[RenderPage]>, new: &mut [RenderPage]) {
    let Some(old) = old else { return };
    let old_keys = old.iter().map(|page| page.key).collect::<BTreeSet<_>>();
    reuse_keys(
        old.iter().map(|value| (value.key, value.match_digest)),
        new.iter_mut()
            .map(|value| (&mut value.key, value.match_digest)),
    );
    let reused = new
        .iter()
        .filter_map(|page| old_keys.contains(&page.key).then_some(page.key))
        .collect::<BTreeSet<_>>();
    for (index, page) in new.iter_mut().enumerate() {
        if old_keys.contains(&page.key) {
            continue;
        }
        if let Some(candidate) = old.get(index)
            && !reused.contains(&candidate.key)
        {
            page.key = candidate.key;
        }
    }
}

fn reuse_node_keys(old: Option<&[RenderNode]>, new: &mut [RenderNode]) {
    let Some(old) = old else { return };
    reuse_keys(
        old.iter().map(|value| (value.key, value.match_digest)),
        new.iter_mut()
            .map(|value| (&mut value.key, value.match_digest)),
    );
}

fn reuse_keys<'a>(
    old: impl Iterator<Item = (RenderKey, RenderDigest)>,
    new: impl Iterator<Item = (&'a mut RenderKey, RenderDigest)>,
) {
    let mut by_digest = BTreeMap::<RenderDigest, VecDeque<RenderKey>>::new();
    for (key, digest) in old {
        by_digest.entry(digest).or_default().push_back(key);
    }
    let mut used = BTreeSet::new();
    for (key, digest) in new {
        if let Some(candidate) = by_digest.get_mut(&digest).and_then(VecDeque::pop_front)
            && used.insert(candidate)
        {
            *key = candidate;
        }
    }
}
