//! Driver-neutral exact page geometry derived from committed artifacts.
//!
//! Text events retain browser-shapeable runs together with exact TeX anchors
//! for each source unit. Driver-specific glyph shaping and widths remain absent.

mod traversal;

#[cfg(test)]
mod tests;

use tex_arith::Scaled;

use crate::{FontResource, PageArtifact, PageEffect};

/// Limits applied while lowering one committed page.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PositionedLimits {
    pub max_events: usize,
    pub max_depth: usize,
    pub max_run_units: usize,
}

impl Default for PositionedLimits {
    fn default() -> Self {
        Self {
            max_events: 1_000_000,
            max_depth: 4_096,
            max_run_units: 16_384,
        }
    }
}

/// A complete page of exact positioned events.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PositionedPage {
    pub page_index: u32,
    pub width: Scaled,
    pub height: Scaled,
    pub mag: i32,
    pub counts: [i32; 10],
    pub fonts: Vec<FontResource>,
    pub events: Vec<PositionedEvent>,
    pub diagnostics: Vec<String>,
    pub last_saved_position: Option<(Scaled, Scaled)>,
    pub snap_reference: (Scaled, Scaled),
}

/// Ordered page event. Event order remains significant at equal coordinates.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PositionedEvent {
    Box(PositionedBox),
    BoxEnd(PositionedBoxEnd),
    TextRun(PositionedTextRun),
    Rule(PositionedRule),
    Special(PositionedSpecial),
    PdfAccessibility(PositionedPdfAccessibility),
    PdfAnnotation(PositionedPdfAnnotation),
    PdfGraphics(PositionedPdfGraphics),
    PdfDestination(PositionedPdfDestination),
    PdfThread(PositionedPdfThread),
    PdfEndThread { x: Scaled, y: Scaled },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PositionedPdfGraphics {
    pub x: Scaled,
    pub y: Scaled,
    pub effect: PageEffect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PositionedBox {
    pub id: u32,
    pub depth: u32,
    pub kind: BoxKind,
    pub x: Scaled,
    pub y: Scaled,
    pub width: Scaled,
    pub height: Scaled,
    pub baseline: Scaled,
}

/// Ordered exit from a positioned box entered by [`PositionedEvent::Box`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PositionedBoxEnd {
    pub id: u32,
    pub depth: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoxKind {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PositionedTextRun {
    pub x: Scaled,
    pub baseline: Scaled,
    pub font_id: u32,
    pub units: Vec<TextUnit>,
    /// Exact horizontal TeX coordinates aligned with `units`.
    pub positions: Vec<Scaled>,
    /// Physical PDF/DVI character codes aligned with `units`. A ligature has
    /// one code on its first source unit and `None` on the remaining units.
    pub physical_codes: Vec<Option<u8>>,
    /// Artifact-node addresses aligned with `units`; spaces have no source.
    pub sources: Vec<Option<PositionedSourceRef>>,
}

/// Stable address of one source character within the current page artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PositionedSourceRef {
    pub node_ordinal: u32,
    pub source_index: u16,
}

/// Browser-shapeable source content. Codes are interpreted only by the
/// explicitly resolved web-font encoding; spaces arise from shipped glue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextUnit {
    Code(u8),
    Space,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PositionedRule {
    pub x: Scaled,
    pub y: Scaled,
    pub width: Scaled,
    pub height: Scaled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PositionedSpecial {
    pub x: Scaled,
    pub y: Scaled,
    pub class: String,
    pub payload: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PositionedPdfAccessibility {
    pub x: Scaled,
    pub y: Scaled,
    pub control: crate::PdfAccessibilityEffect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PositionedPdfAnnotation {
    pub x: Scaled,
    pub y: Scaled,
    pub containing_box: u32,
    pub depth: u32,
    pub marker: crate::PdfAnnotationEffect,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PositionedPdfDestination {
    pub x: Scaled,
    pub y: Scaled,
    pub containing_box: u32,
    pub marker: crate::PdfDestinationEffect,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PositionedPdfThread {
    pub x: Scaled,
    pub y: Scaled,
    pub containing_box: u32,
    pub running: bool,
    pub marker: crate::PdfThreadEffect,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PositionedError {
    PositionOverflow,
    InvalidMagnification { mag: i32 },
    MissingEffect { effect_index: u32 },
    CharacterOutOfRange { ch: u32 },
    TooManyEvents { limit: usize },
    NestingTooDeep { limit: usize },
    TextRunTooLong { limit: usize },
    UnmatchedPdfSaves { count: usize },
}

impl std::fmt::Display for PositionedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PositionOverflow => f.write_str("positioned page arithmetic overflowed"),
            Self::InvalidMagnification { mag } => {
                write!(f, "HTML output requires positive magnification, got {mag}")
            }
            Self::MissingEffect { effect_index } => {
                write!(f, "page node references missing effect {effect_index}")
            }
            Self::CharacterOutOfRange { ch } => {
                write!(f, "browser text code {ch} is outside 0..=255")
            }
            Self::TooManyEvents { limit } => {
                write!(f, "positioned page exceeds event limit {limit}")
            }
            Self::NestingTooDeep { limit } => {
                write!(f, "positioned page exceeds nesting limit {limit}")
            }
            Self::TextRunTooLong { limit } => {
                write!(f, "positioned text run exceeds unit limit {limit}")
            }
            Self::UnmatchedPdfSaves { count } => {
                write!(f, "page ended with {count} unmatched \\pdfsave node(s)")
            }
        }
    }
}

impl std::error::Error for PositionedError {}

/// Lowers one validated committed page without consulting live engine state.
pub fn lower_page(page: &PageArtifact, page_index: u32) -> Result<PositionedPage, PositionedError> {
    lower_page_with_limits(page, page_index, PositionedLimits::default())
}

/// Lowers a page for shipout bookkeeping while deferring unmatched PDF graphics
/// saves to the PDF driver, which reports them when the artifact is assembled.
pub fn lower_page_for_shipout(
    page: &PageArtifact,
    page_index: u32,
) -> Result<PositionedPage, PositionedError> {
    traversal::lower(page, page_index, PositionedLimits::default(), false)
}

pub fn lower_page_with_limits(
    page: &PageArtifact,
    page_index: u32,
    limits: PositionedLimits,
) -> Result<PositionedPage, PositionedError> {
    traversal::lower(page, page_index, limits, true)
}
