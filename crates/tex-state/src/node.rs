//! Immutable TeX node model.

use crate::glue::Order;
use crate::ids::{FontId, GlueId, NodeListId, TokenListId};
use crate::scaled::Scaled;

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
    Unset,
    Disc {
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
        content: NodeListId,
    },
    Whatsit(Whatsit),
    MathOn,
    MathOff,
    Adjust(NodeListId),
}

/// A TeX box node payload shared by hlist and vlist nodes.
#[derive(Clone, Debug, PartialEq)]
pub struct BoxNode {
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    pub shift: Scaled,
    pub glue_set: f64,
    pub glue_sign: Sign,
    pub glue_order: Order,
    pub children: NodeListId,
}

impl BoxNode {
    /// Creates a box payload, normalizing `-0.0` glue ratios to `0.0`.
    #[must_use]
    pub fn new(fields: BoxNodeFields) -> Self {
        Self {
            width: fields.width,
            height: fields.height,
            depth: fields.depth,
            shift: fields.shift,
            // TeX's glue_ratio is a float; normalize the negative zero spelling
            // so future content hashing is independent of arithmetic history.
            glue_set: if fields.glue_set == 0.0 {
                0.0
            } else {
                fields.glue_set
            },
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
    pub glue_set: f64,
    pub glue_sign: Sign,
    pub glue_order: Order,
    pub children: NodeListId,
}

/// The source of a kern node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum KernKind {
    Explicit,
    Font,
    Accent,
}

/// The source of a glue node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GlueKind {
    Normal,
    Leaders,
    Cleaders,
    Xleaders,
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
    DeferredWrite { stream: u8, tokens: TokenListId },
}

impl Node {
    #[cfg(debug_assertions)]
    pub(crate) fn child_lists(&self, out: &mut Vec<NodeListId>) {
        match self {
            Self::HList(box_node) | Self::VList(box_node) => out.push(box_node.children),
            Self::Disc { pre, post, replace } => {
                out.push(*pre);
                out.push(*post);
                out.push(*replace);
            }
            Self::Ins { content, .. } | Self::Adjust(content) => out.push(*content),
            Self::Char { .. }
            | Self::Lig { .. }
            | Self::Kern { .. }
            | Self::Glue { .. }
            | Self::Penalty(_)
            | Self::Rule { .. }
            | Self::Unset
            | Self::Mark { .. }
            | Self::Whatsit(_)
            | Self::MathOn
            | Self::MathOff => {}
        }
    }
}
