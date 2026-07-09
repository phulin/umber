//! Immutable TeX node model.

use crate::glue::Order;
use crate::ids::{FontId, GlueId, NodeListId, TokenListId};
#[cfg(debug_assertions)]
use crate::math::MathField;
use crate::math::{MathChoice, MathFraction, MathListNode, MathNoad, MathStyle};
use crate::scaled::{GlueSetRatio, Scaled};
use crate::world::PrintSink;

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
    MathNoad(MathNoad),
    FractionNoad(MathFraction),
    MathStyle(MathStyle),
    MathChoice(MathChoice),
    MathList(MathListNode),
    Nonscript,
    Adjust(NodeListId),
}

/// A TeX box node payload shared by hlist and vlist nodes.
#[derive(Clone, Debug, PartialEq)]
pub struct BoxNode {
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

/// A TeX unset box used while alignments are being measured and resolved.
#[derive(Clone, Debug, PartialEq)]
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
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum UnsetKind {
    HBox,
    VBox,
}

/// The source of a kern node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum KernKind {
    Explicit,
    Font,
    Accent,
    Mu,
}

/// The source of a glue node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DiscKind {
    Discretionary,
    ExplicitHyphen,
    AutomaticHyphen,
}

/// The sign of box glue adjustment.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Sign {
    Normal,
    Stretching,
    Shrinking,
}

/// Extension nodes whose effects are interpreted by later subsystems.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Whatsit {
    DeferredWrite {
        sink: PrintSink,
        tokens: TokenListId,
    },
    Special {
        class: String,
        payload: Vec<u8>,
    },
}

impl Node {
    #[cfg(debug_assertions)]
    pub(crate) fn child_lists(&self, out: &mut Vec<NodeListId>) {
        match self {
            Self::HList(box_node) | Self::VList(box_node) => out.push(box_node.children),
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
