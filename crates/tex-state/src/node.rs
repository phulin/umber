//! Immutable TeX node model.

use crate::glue::Order;
use crate::ids::{FontId, GlueId, NodeListId, TokenListId};
use crate::math::{MathChoice, MathFraction, MathListNode, MathNoad, MathStyle};
use crate::scaled::{GlueSetRatio, Scaled};
use crate::token::OriginId;
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
        spec: GlueId,
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
        tokens: TokenListId,
    },
    Ins {
        class: u16,
        size: Scaled,
        split_top_skip: GlueId,
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
        match (self, other) {
            (
                Self::Char {
                    font: left_font,
                    ch: left_ch,
                    ..
                },
                Self::Char {
                    font: right_font,
                    ch: right_ch,
                    ..
                },
            ) => left_font == right_font && left_ch == right_ch,
            (
                Self::Lig {
                    font: left_font,
                    ch: left_ch,
                    orig: left_orig,
                    left_hit: left_left_hit,
                    right_hit: left_right_hit,
                    ..
                },
                Self::Lig {
                    font: right_font,
                    ch: right_ch,
                    orig: right_orig,
                    left_hit: right_left_hit,
                    right_hit: right_right_hit,
                    ..
                },
            ) => {
                left_font == right_font
                    && left_ch == right_ch
                    && left_orig == right_orig
                    && left_left_hit == right_left_hit
                    && left_right_hit == right_right_hit
            }
            (
                Self::Kern {
                    amount: left_amount,
                    kind: left_kind,
                },
                Self::Kern {
                    amount: right_amount,
                    kind: right_kind,
                },
            ) => left_amount == right_amount && left_kind == right_kind,
            (
                Self::MarginKern {
                    amount: left_amount,
                    side: left_side,
                    font: left_font,
                    ch: left_ch,
                },
                Self::MarginKern {
                    amount: right_amount,
                    side: right_side,
                    font: right_font,
                    ch: right_ch,
                },
            ) => {
                left_amount == right_amount
                    && left_side == right_side
                    && left_font == right_font
                    && left_ch == right_ch
            }
            (
                Self::Glue {
                    spec: left_spec,
                    kind: left_kind,
                    leader: left_leader,
                },
                Self::Glue {
                    spec: right_spec,
                    kind: right_kind,
                    leader: right_leader,
                },
            ) => left_spec == right_spec && left_kind == right_kind && left_leader == right_leader,
            (Self::Penalty(left), Self::Penalty(right)) => left == right,
            (
                Self::Rule {
                    width: left_width,
                    height: left_height,
                    depth: left_depth,
                },
                Self::Rule {
                    width: right_width,
                    height: right_height,
                    depth: right_depth,
                },
            ) => {
                left_width == right_width
                    && left_height == right_height
                    && left_depth == right_depth
            }
            (Self::HList(left), Self::HList(right)) | (Self::VList(left), Self::VList(right)) => {
                left == right
            }
            (Self::Unset(left), Self::Unset(right)) => left == right,
            (
                Self::Disc {
                    kind: left_kind,
                    pre: left_pre,
                    post: left_post,
                    replace: left_replace,
                    ..
                },
                Self::Disc {
                    kind: right_kind,
                    pre: right_pre,
                    post: right_post,
                    replace: right_replace,
                    ..
                },
            ) => {
                left_kind == right_kind
                    && left_pre == right_pre
                    && left_post == right_post
                    && left_replace == right_replace
            }
            (
                Self::Mark {
                    class: left_class,
                    tokens: left_tokens,
                },
                Self::Mark {
                    class: right_class,
                    tokens: right_tokens,
                },
            ) => left_class == right_class && left_tokens == right_tokens,
            (
                Self::Ins {
                    class: left_class,
                    size: left_size,
                    split_top_skip: left_split_top_skip,
                    split_max_depth: left_split_max_depth,
                    floating_penalty: left_floating_penalty,
                    content: left_content,
                },
                Self::Ins {
                    class: right_class,
                    size: right_size,
                    split_top_skip: right_split_top_skip,
                    split_max_depth: right_split_max_depth,
                    floating_penalty: right_floating_penalty,
                    content: right_content,
                },
            ) => {
                left_class == right_class
                    && left_size == right_size
                    && left_split_top_skip == right_split_top_skip
                    && left_split_max_depth == right_split_max_depth
                    && left_floating_penalty == right_floating_penalty
                    && left_content == right_content
            }
            (Self::Whatsit(left), Self::Whatsit(right)) => left == right,
            (Self::MathOn(left), Self::MathOn(right))
            | (Self::MathOff(left), Self::MathOff(right)) => left == right,
            (Self::Direction(left), Self::Direction(right)) => left == right,
            (Self::MathNoad(left), Self::MathNoad(right)) => left == right,
            (Self::FractionNoad(left), Self::FractionNoad(right)) => left == right,
            (Self::MathStyle(left), Self::MathStyle(right)) => left == right,
            (Self::MathChoice(left), Self::MathChoice(right)) => left == right,
            (Self::MathList(left), Self::MathList(right)) => left == right,
            (Self::Nonscript, Self::Nonscript) => true,
            (Self::Adjust(left), Self::Adjust(right)) => left == right,
            _ => false,
        }
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
        tokens: TokenListId,
    },
    Special {
        class: String,
        payload: Vec<u8>,
    },
    DeferredSpecial {
        class: String,
        tokens: TokenListId,
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
        tokens: TokenListId,
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
        glue: GlueId,
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
    pub attributes: TokenListId,
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
    #[cfg(debug_assertions)]
    pub(crate) fn child_lists(&self, out: &mut Vec<NodeListId>) {
        out.extend(crate::node_arena::NodeRef::from(self).children());
    }
}
