//! Immutable TeX node model.

use crate::glue::{GlueSpecRef, Order};
use crate::ids::{FontId, NodeListId};
use crate::math::{MathChoice, MathFraction, MathListNode, MathNoad, MathStyle};
use crate::scaled::{GlueSetRatio, Scaled};
use crate::token::OriginId;
use crate::token_store::TokenListRef;
use crate::world::{PrintSink, StreamSlot};

/// Stable logical node kinds shared by owned and compact node views.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum NodeKind {
    Char,
    Lig,
    Kern,
    MarginKern,
    Glue,
    Penalty,
    Rule,
    HList,
    VList,
    Unset,
    Disc,
    Mark,
    Ins,
    Whatsit,
    MathOn,
    MathOff,
    Direction,
    MathNoad,
    FractionNoad,
    MathStyle,
    MathChoice,
    MathList,
    Nonscript,
    Adjust,
}

impl NodeKind {
    pub const ALL: [Self; 24] = [
        Self::Char,
        Self::Lig,
        Self::Kern,
        Self::MarginKern,
        Self::Glue,
        Self::Penalty,
        Self::Rule,
        Self::HList,
        Self::VList,
        Self::Unset,
        Self::Disc,
        Self::Mark,
        Self::Ins,
        Self::Whatsit,
        Self::MathOn,
        Self::MathOff,
        Self::Direction,
        Self::MathNoad,
        Self::FractionNoad,
        Self::MathStyle,
        Self::MathChoice,
        Self::MathList,
        Self::Nonscript,
        Self::Adjust,
    ];

    #[must_use]
    pub const fn etex_type(self) -> i32 {
        self.descriptor().etex_type
    }
}

/// A frozen TeX node.
#[derive(Clone, Debug)]
pub enum Node {
    Char {
        font: FontId,
        ch: char,
        /// Diagnostic-only source provenance; excluded from semantic identity.
        origin: OriginId,
    },
    Lig {
        font: FontId,
        ch: char,
        orig: Vec<char>,
        left_hit: bool,
        right_hit: bool,
        /// One origin per original character consumed by the ligature.
        origins: Vec<OriginId>,
    },
    Kern {
        amount: Scaled,
        kind: KernKind,
    },
    /// pdfTeX character protrusion retaining the exact contributing glyph.
    MarginKern {
        amount: Scaled,
        side: MarginKernSide,
        font: FontId,
        ch: u8,
    },
    Glue {
        spec: GlueSpecRef,
        kind: GlueKind,
        leader: Option<LeaderPayload>,
    },
    Penalty(i32),
    Rule {
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
    },
    HList(BoxNode),
    VList(BoxNode),
    Unset(UnsetNode),
    Disc {
        kind: DiscKind,
        pre: NodeListId,
        post: NodeListId,
        replace: NodeListId,
        /// TeX's physical `replace_count`, retained only for diagnostics.
        physical_replace_count: u8,
    },
    Mark {
        class: u16,
        tokens: TokenListRef,
    },
    Ins {
        class: u16,
        size: Scaled,
        split_top_skip: GlueSpecRef,
        split_max_depth: Scaled,
        floating_penalty: i32,
        content: NodeListId,
    },
    Whatsit(Whatsit),
    MathOn(Scaled),
    MathOff(Scaled),
    Direction(Direction),
    MathNoad(MathNoad),
    FractionNoad(MathFraction),
    MathStyle(MathStyle),
    MathChoice(MathChoice),
    MathList(MathListNode),
    Nonscript,
    Adjust(AdjustNode),
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        crate::node_arena::NodeRef::from(self) == crate::node_arena::NodeRef::from(other)
    }
}

#[cfg(feature = "profiling")]
mod stats {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::Node;

    static COUNTS: [AtomicU64; 24] = [const { AtomicU64::new(0) }; 24];

    pub fn record(node: &Node) {
        let index = node.kind() as usize;
        COUNTS[index].fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot() -> Vec<(&'static str, u64)> {
        super::NodeKind::ALL
            .iter()
            .zip(&COUNTS)
            .filter_map(|(kind, count)| {
                let count = count.load(Ordering::Relaxed);
                (count != 0).then_some((kind.descriptor().name, count))
            })
            .collect()
    }
}

/// Returns the process-local node-append histogram used by measurement builds.
///
/// These relaxed counters are diagnostic-only and are not engine state.
#[cfg(feature = "profiling")]
#[must_use]
pub fn node_append_histogram() -> Vec<(&'static str, u64)> {
    stats::snapshot()
}

#[cfg(feature = "profiling")]
pub(crate) fn record_node_append(node: &Node) {
    stats::record(node);
}

/// A pdfTeX adjustment node payload.
///
/// Ordinary TeX adjustments migrate after their containing horizontal box;
/// pdfTeX's `pre` form migrates before it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdjustNode {
    pub content: NodeListId,
    pub pre: bool,
}

impl AdjustNode {
    #[must_use]
    pub const fn ordinary(content: NodeListId) -> Self {
        Self {
            content,
            pre: false,
        }
    }
}

/// A TeX box node payload shared by hlist and vlist nodes.
#[derive(Clone, Copy, Debug)]
pub struct BoxNode {
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    /// TeX.web `shift_amount`: positive moves down in an hlist and right in a vlist.
    pub shift: Scaled,
    /// Merged e-TeX WEB §53a's `box_lr` subtype.
    pub box_lr: BoxLr,
    pub glue_set: GlueSetRatio,
    pub glue_sign: Sign,
    pub glue_order: Order,
    pub children: NodeListId,
    pub diagnostic_children: Option<NodeListId>,
}

impl BoxNode {
    /// Creates a box payload.
    #[must_use]
    pub fn new(fields: BoxNodeFields) -> Self {
        Self {
            width: fields.width,
            height: fields.height,
            depth: fields.depth,
            shift: fields.shift,
            box_lr: fields.box_lr,
            glue_set: fields.glue_set,
            glue_sign: fields.glue_sign,
            glue_order: fields.glue_order,
            children: fields.children,
            diagnostic_children: None,
        }
    }
}

impl PartialEq for BoxNode {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.depth == other.depth
            && self.shift == other.shift
            && self.box_lr == other.box_lr
            && self.glue_set == other.glue_set
            && self.glue_sign == other.glue_sign
            && self.glue_order == other.glue_order
            && self.children == other.children
    }
}

/// Construction fields for a TeX box node payload.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxNodeFields {
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    pub shift: Scaled,
    pub box_lr: BoxLr,
    pub glue_set: GlueSetRatio,
    pub glue_sign: Sign,
    pub glue_order: Order,
    pub children: NodeListId,
}

/// Direction/reversal identity carried by an e-TeX horizontal box.
///
/// The numeric values are the canonical hlist subtypes from merged e-TeX WEB
/// §53a. Vertical boxes retain [`BoxLr::Normal`].
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize,
)]
#[repr(u8)]
pub enum BoxLr {
    #[default]
    Normal = 0,
    Reversed = 1,
    DList = 2,
}

/// Repeated material attached to a leader glue node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LeaderPayload {
    HList(BoxNode),
    VList(BoxNode),
    Rule {
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
    },
}

/// A TeX unset box used while alignments are being measured and resolved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnsetNode {
    pub kind: UnsetKind,
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    /// TeX82 §796's zero-based quarterword encoding (columns minus one).
    pub span_count: u16,
    pub stretch: Scaled,
    pub stretch_order: Order,
    pub shrink: Scaled,
    pub shrink_order: Order,
    pub children: NodeListId,
}

impl UnsetNode {
    /// Creates an unset box payload.
    #[must_use]
    pub fn new(fields: UnsetNodeFields) -> Self {
        Self {
            kind: fields.kind,
            width: fields.width,
            height: fields.height,
            depth: fields.depth,
            span_count: fields.span_count,
            stretch: fields.stretch,
            stretch_order: fields.stretch_order,
            shrink: fields.shrink,
            shrink_order: fields.shrink_order,
            children: fields.children,
        }
    }
}

/// Construction fields for an unset alignment box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnsetNodeFields {
    pub kind: UnsetKind,
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    /// TeX82 §796's zero-based quarterword encoding (columns minus one).
    pub span_count: u16,
    pub stretch: Scaled,
    pub stretch_order: Order,
    pub shrink: Scaled,
    pub shrink_order: Order,
    pub children: NodeListId,
}

/// Whether an unset node was packaged with horizontal or vertical metrics.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum UnsetKind {
    HBox,
    VBox,
}

/// The source of a kern node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum KernKind {
    Explicit,
    Font,
    /// Automatic kern from pdfTeX's `knbc`/`knac` character-code tables.
    Auto,
    Accent,
    Mu,
    /// pdfTeX character protrusion at the left edge of a finalized line.
    LeftMargin,
    /// pdfTeX character protrusion at the right edge of a finalized line.
    RightMargin,
}

/// The edge at which pdfTeX materialized a character-protrusion kern.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum MarginKernSide {
    Left,
    Right,
}

/// The source of a glue node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum GlueKind {
    Normal,
    SpaceSkip,
    XSpaceSkip,
    TabSkip,
    BaselineSkip,
    LineSkip,
    TopSkip,
    SplitTopSkip,
    LeftSkip,
    RightSkip,
    ParSkip,
    ParFillSkip,
    AboveDisplaySkip,
    BelowDisplaySkip,
    AboveDisplayShortSkip,
    BelowDisplayShortSkip,
    Leaders,
    Cleaders,
    Xleaders,
    MuSkip,
    ThinMuSkip,
    MedMuSkip,
    ThickMuSkip,
    NonScript,
}

/// The source of a discretionary node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum DiscKind {
    Discretionary,
    ExplicitHyphen,
    AutomaticHyphen,
}

/// An e-TeX M/L/R math boundary (merged e-TeX WEB §12).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
#[repr(u8)]
pub enum MathBoundary {
    BeginM = 4,
    EndM = 5,
    BeginL = 0,
    EndL = 1,
    BeginR = 2,
    EndR = 3,
}

/// Compatibility name for the L/R users of [`MathBoundary`].
pub type Direction = MathBoundary;

/// Merged e-TeX WEB §53a's split LR anomaly counter.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct LrAnomalies {
    pub missing: u32,
    pub extra: u32,
}

/// Canonical M/L/R nesting state used by list-boundary algorithms.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MathBoundaryStack {
    open: Vec<MathBoundary>,
    anomalies: LrAnomalies,
}

impl MathBoundaryStack {
    pub fn observe(&mut self, boundary: MathBoundary) {
        match boundary {
            MathBoundary::BeginM | MathBoundary::BeginL | MathBoundary::BeginR => {
                self.open.push(boundary)
            }
            MathBoundary::EndM | MathBoundary::EndL | MathBoundary::EndR => {
                if self
                    .open
                    .last()
                    .copied()
                    .is_some_and(|open| open.matches(boundary))
                {
                    self.open.pop();
                } else {
                    self.anomalies.extra = self.anomalies.extra.saturating_add(1);
                }
            }
        }
    }

    #[must_use]
    pub fn finish(mut self) -> LrAnomalies {
        self.anomalies.missing = self
            .anomalies
            .missing
            .saturating_add(u32::try_from(self.open.len()).unwrap_or(u32::MAX));
        self.anomalies
    }
}

impl MathBoundary {
    const fn matches(self, end: Self) -> bool {
        matches!(
            (self, end),
            (Self::BeginM, Self::EndM) | (Self::BeginL, Self::EndL) | (Self::BeginR, Self::EndR)
        )
    }
}

/// The sign of box glue adjustment.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum Sign {
    Normal,
    Stretching,
    Shrinking,
}

/// Extension nodes whose effects are interpreted by later subsystems.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Whatsit {
    OpenOut {
        slot: StreamSlot,
        path: String,
    },
    CloseOut {
        /// `None` is §1342's permanently closed normalized slot 16 or 17.
        slot: Option<StreamSlot>,
    },
    DeferredWrite {
        sink: PrintSink,
        tokens: TokenListRef,
    },
    Special {
        class: String,
        payload: Vec<u8>,
    },
    DeferredSpecial {
        class: String,
        tokens: TokenListRef,
    },
    PdfReferenceObject {
        object: u32,
    },
    PdfAccessibility(PdfAccessibilityControl),
    PdfAnnotation {
        object: u32,
    },
    PdfLinkStart {
        object: u32,
    },
    PdfLinkEnd {
        object: u32,
    },
    PdfRunningLink(bool),
    PdfLiteral {
        mode: PdfLiteralMode,
        payload: Vec<u8>,
    },
    DeferredPdfLiteral {
        mode: PdfLiteralMode,
        tokens: TokenListRef,
    },
    PdfSetMatrix {
        payload: Vec<u8>,
    },
    PdfSave,
    PdfRestore,
    PdfColorStack {
        id: u32,
        action: crate::PdfColorStackAction,
    },
    PdfSavePos,
    PdfSnapRefPoint,
    PdfSnapY {
        glue: GlueSpecRef,
    },
    PdfSnapYComp {
        ratio: u16,
    },
    PdfRefXForm {
        object: u32,
        width: Scaled,
        height: Scaled,
        depth: Scaled,
    },
    PdfRefXImage {
        object: u32,
        width: Scaled,
        height: Scaled,
        depth: Scaled,
    },
    PdfDestination(Box<PdfDestinationNode>),
    PdfThread(Box<PdfThreadNode>),
    PdfEndThread,
    Language {
        language: u8,
        left_hyphen_min: u8,
        right_hyphen_min: u8,
    },
}

/// Rare article-thread marker kept out of the hot inline node representation.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PdfThreadNode {
    pub identifier: crate::PdfActionIdentifier,
    pub dimensions: crate::PdfAnnotationDimensions,
    pub attributes: TokenListRef,
    pub running: bool,
}

/// Rare destination marker kept out of the hot inline node representation.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PdfDestinationNode {
    pub identifier: crate::PdfActionIdentifier,
    pub structure: Option<u32>,
    pub kind: PdfDestinationKind,
}

/// A page destination view, retained until final traversal resolves geometry.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfDestinationKind {
    Xyz { zoom: Option<i32> },
    FitBoundingBoxHorizontal,
    FitBoundingBoxVertical,
    FitBoundingBox,
    FitHorizontal,
    FitVertical,
    FitRectangle(crate::PdfAnnotationDimensions),
    Fit,
}
/// Ordered PDF text-accessibility controls interpreted during page traversal.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum PdfAccessibilityControl {
    InterwordSpaceOn,
    InterwordSpaceOff,
    FakeSpace,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfLiteralMode {
    Origin,
    Page,
    Direct,
}

impl Node {
    /// Returns this node's source-independent logical kind.
    #[must_use]
    pub fn kind(&self) -> NodeKind {
        crate::node_arena::NodeRef::from(self).kind()
    }

    /// e-TeX `\lastnodetype` code for this node.
    #[must_use]
    pub fn etex_type(&self) -> i32 {
        self.kind().etex_type()
    }
}
