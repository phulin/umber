//! Immutable TeX node model.

use crate::glue::Order;
use crate::ids::{FontId, GlueId, NodeListId, TokenListId};
#[cfg(debug_assertions)]
use crate::math::MathField;
use crate::math::{MathChoice, MathFraction, MathListNode, MathNoad, MathStyle};
use crate::scaled::{GlueSetRatio, Scaled};
use crate::world::{PrintSink, StreamSlot};

/// A frozen TeX node.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Char {
        font: FontId,
        ch: char,
    },
    Lig {
        font: FontId,
        ch: char,
        orig: (char, char),
    },
    Kern {
        amount: Scaled,
        kind: KernKind,
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
    Adjust(NodeListId),
}

#[cfg(feature = "node-stats")]
mod stats {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::Node;

    pub const NAMES: [&str; 22] = [
        "char",
        "lig",
        "kern",
        "glue",
        "penalty",
        "rule",
        "hlist",
        "vlist",
        "unset",
        "disc",
        "mark",
        "ins",
        "whatsit",
        "math_on",
        "math_off",
        "math_noad",
        "fraction_noad",
        "math_style",
        "math_choice",
        "math_list",
        "nonscript",
        "adjust",
    ];
    static COUNTS: [AtomicU64; NAMES.len()] = [const { AtomicU64::new(0) }; NAMES.len()];

    pub fn record(node: &Node) {
        let index = match node {
            Node::Char { .. } => 0,
            Node::Lig { .. } => 1,
            Node::Kern { .. } => 2,
            Node::Glue { .. } => 3,
            Node::Penalty(_) => 4,
            Node::Rule { .. } => 5,
            Node::HList(_) => 6,
            Node::VList(_) => 7,
            Node::Unset(_) => 8,
            Node::Disc { .. } => 9,
            Node::Mark { .. } => 10,
            Node::Ins { .. } => 11,
            Node::Whatsit(_) => 12,
            Node::MathOn(_) => 13,
            Node::MathOff(_) => 14,
            Node::MathNoad(_) => 15,
            Node::FractionNoad(_) => 16,
            Node::MathStyle(_) => 17,
            Node::MathChoice(_) => 18,
            Node::MathList(_) => 19,
            Node::Nonscript => 20,
            Node::Adjust(_) => 21,
        };
        COUNTS[index].fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot() -> Vec<(&'static str, u64)> {
        NAMES
            .iter()
            .zip(&COUNTS)
            .filter_map(|(&name, count)| {
                let count = count.load(Ordering::Relaxed);
                (count != 0).then_some((name, count))
            })
            .collect()
    }
}

/// Returns the process-local node-append histogram used by measurement builds.
///
/// These relaxed counters are diagnostic-only and are not engine state.
#[cfg(feature = "node-stats")]
#[must_use]
pub fn node_append_histogram() -> Vec<(&'static str, u64)> {
    stats::snapshot()
}

#[cfg(feature = "node-stats")]
pub(crate) fn record_node_append(node: &Node) {
    stats::record(node);
}

/// A TeX box node payload shared by hlist and vlist nodes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxNode {
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    /// TeX.web `shift_amount`: positive moves down in an hlist and right in a vlist.
    pub shift: Scaled,
    pub display: bool,
    pub glue_set: GlueSetRatio,
    pub glue_sign: Sign,
    pub glue_order: Order,
    pub children: NodeListId,
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
            display: fields.display,
            glue_set: fields.glue_set,
            glue_sign: fields.glue_sign,
            glue_order: fields.glue_order,
            children: fields.children,
        }
    }
}

/// Construction fields for a TeX box node payload.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxNodeFields {
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    pub shift: Scaled,
    pub display: bool,
    pub glue_set: GlueSetRatio,
    pub glue_sign: Sign,
    pub glue_order: Order,
    pub children: NodeListId,
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
    Accent,
    Mu,
}

/// The source of a glue node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum GlueKind {
    Normal,
    TabSkip,
    BaselineSkip,
    LineSkip,
    TopSkip,
    SplitTopSkip,
    LeftSkip,
    RightSkip,
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

/// An e-TeX text-direction boundary (manual section 3.5, TeX--XeT).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum Direction {
    BeginL,
    EndL,
    BeginR,
    EndR,
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
        slot: StreamSlot,
    },
    DeferredWrite {
        sink: PrintSink,
        tokens: TokenListId,
    },
    Special {
        class: String,
        payload: Vec<u8>,
    },
    Language {
        language: u8,
        left_hyphen_min: u8,
        right_hyphen_min: u8,
    },
}

impl Node {
    /// e-TeX `\lastnodetype` code for this node.
    #[must_use]
    pub const fn etex_type(&self) -> i32 {
        match self {
            Self::Char { .. } => 0,
            Self::HList(_) => 1,
            Self::VList(_) => 2,
            Self::Rule { .. } => 3,
            Self::Ins { .. } => 4,
            Self::Mark { .. } => 5,
            Self::Adjust(_) => 6,
            Self::Lig { .. } => 7,
            Self::Disc { .. } => 8,
            Self::Whatsit(_) => 9,
            Self::MathOn(_) | Self::MathOff(_) | Self::Direction(_) => 10,
            Self::Glue { .. } | Self::Nonscript => 11,
            Self::Kern { .. } => 12,
            Self::Penalty(_) => 13,
            Self::Unset(_) => 14,
            Self::MathNoad(_)
            | Self::FractionNoad(_)
            | Self::MathStyle(_)
            | Self::MathChoice(_)
            | Self::MathList(_) => 15,
        }
    }
    #[cfg(debug_assertions)]
    pub(crate) fn child_lists(&self, out: &mut Vec<NodeListId>) {
        match self {
            Self::HList(box_node) | Self::VList(box_node) => out.push(box_node.children),
            Self::Glue {
                leader: Some(LeaderPayload::HList(box_node) | LeaderPayload::VList(box_node)),
                ..
            } => out.push(box_node.children),
            Self::Unset(unset) => out.push(unset.children),
            Self::Disc {
                pre, post, replace, ..
            } => {
                out.push(*pre);
                out.push(*post);
                out.push(*replace);
            }
            Self::Ins { content, .. } | Self::Adjust(content) => out.push(*content),
            Self::MathNoad(noad) => {
                push_math_field_child(&noad.nucleus, out);
                push_math_field_child(&noad.subscript, out);
                push_math_field_child(&noad.superscript, out);
            }
            Self::FractionNoad(fraction) => {
                out.push(fraction.numerator);
                out.push(fraction.denominator);
            }
            Self::MathChoice(choice) => {
                out.push(choice.display);
                out.push(choice.text);
                out.push(choice.script);
                out.push(choice.script_script);
            }
            Self::MathList(list) => out.push(list.content),
            Self::Char { .. }
            | Self::Lig { .. }
            | Self::Kern { .. }
            | Self::Glue { .. }
            | Self::Penalty(_)
            | Self::Rule { .. }
            | Self::Mark { .. }
            | Self::Whatsit(_)
            | Self::MathOn(_)
            | Self::MathOff(_)
            | Self::Direction(_)
            | Self::MathStyle(_)
            | Self::Nonscript => {}
        }
    }
}

#[cfg(debug_assertions)]
fn push_math_field_child(field: &MathField, out: &mut Vec<NodeListId>) {
    match field {
        MathField::SubBox(list) | MathField::SubMlist(list) => out.push(*list),
        MathField::Empty | MathField::MathChar(_) | MathField::MathTextChar(_) => {}
    }
}
