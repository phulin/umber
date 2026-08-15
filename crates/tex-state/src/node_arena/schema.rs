//! Executable logical schema shared by owned and compact nodes.
//!
//! The compact arena deliberately keeps its specialized words and sidecars.
//! This module describes their decoded meaning instead: every [`NodeRef`]
//! enters through one exhaustive match and reports its portable handles,
//! diagnostic origins, and ordered child edges without allocating.

use super::NodeRef;
use crate::ids::{FontId, GlueId, NodeListId, TokenListId};
use crate::math::{MathChar, MathField, NoadKind};
use crate::node::{LeaderPayload, NodeKind, Whatsit};
use crate::token::OriginId;

/// Stable semantic tag used by source-independent node operations.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum NodeTag {
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

/// Whether a declared field contributes to source-independent node identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FieldPolicy {
    Semantic,
    Diagnostic,
}

/// One field declared by the logical schema.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeField {
    pub name: &'static str,
    pub policy: FieldPolicy,
}

/// Static metadata for one node variant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeDescriptor {
    pub tag: NodeTag,
    pub name: &'static str,
    pub etex_type: i32,
    pub fields: &'static [NodeField],
}

const fn semantic(name: &'static str) -> NodeField {
    NodeField {
        name,
        policy: FieldPolicy::Semantic,
    }
}

const fn diagnostic(name: &'static str) -> NodeField {
    NodeField {
        name,
        policy: FieldPolicy::Diagnostic,
    }
}

macro_rules! fields {
    ($($policy:ident($name:literal)),* $(,)?) => { &[$($policy($name)),*] };
}

const DESCRIPTORS: [NodeDescriptor; 24] = [
    NodeDescriptor {
        tag: NodeTag::Char,
        name: "char",
        etex_type: 0,
        fields: fields![semantic("font"), semantic("ch"), diagnostic("origin")],
    },
    NodeDescriptor {
        tag: NodeTag::Lig,
        name: "lig",
        etex_type: 7,
        fields: fields![
            semantic("font"),
            semantic("ch"),
            semantic("orig"),
            semantic("left_hit"),
            semantic("right_hit"),
            diagnostic("origins")
        ],
    },
    NodeDescriptor {
        tag: NodeTag::Kern,
        name: "kern",
        etex_type: 12,
        fields: fields![semantic("amount"), semantic("kind")],
    },
    NodeDescriptor {
        tag: NodeTag::MarginKern,
        name: "margin_kern",
        etex_type: 12,
        fields: fields![
            semantic("amount"),
            semantic("side"),
            semantic("font"),
            semantic("ch")
        ],
    },
    NodeDescriptor {
        tag: NodeTag::Glue,
        name: "glue",
        etex_type: 11,
        fields: fields![semantic("spec"), semantic("kind"), semantic("leader")],
    },
    NodeDescriptor {
        tag: NodeTag::Penalty,
        name: "penalty",
        etex_type: 13,
        fields: fields![semantic("penalty")],
    },
    NodeDescriptor {
        tag: NodeTag::Rule,
        name: "rule",
        etex_type: 3,
        fields: fields![semantic("width"), semantic("height"), semantic("depth")],
    },
    NodeDescriptor {
        tag: NodeTag::HList,
        name: "hlist",
        etex_type: 1,
        fields: fields![semantic("box"), diagnostic("diagnostic_children")],
    },
    NodeDescriptor {
        tag: NodeTag::VList,
        name: "vlist",
        etex_type: 2,
        fields: fields![semantic("box"), diagnostic("diagnostic_children")],
    },
    NodeDescriptor {
        tag: NodeTag::Unset,
        name: "unset",
        etex_type: 14,
        fields: fields![semantic("unset")],
    },
    NodeDescriptor {
        tag: NodeTag::Disc,
        name: "disc",
        etex_type: 8,
        fields: fields![
            semantic("kind"),
            semantic("pre"),
            semantic("post"),
            semantic("replace"),
            diagnostic("physical_replace_count")
        ],
    },
    NodeDescriptor {
        tag: NodeTag::Mark,
        name: "mark",
        etex_type: 5,
        fields: fields![semantic("class"), semantic("tokens")],
    },
    NodeDescriptor {
        tag: NodeTag::Ins,
        name: "ins",
        etex_type: 4,
        fields: fields![
            semantic("class"),
            semantic("size"),
            semantic("split_top_skip"),
            semantic("split_max_depth"),
            semantic("floating_penalty"),
            semantic("content")
        ],
    },
    NodeDescriptor {
        tag: NodeTag::Whatsit,
        name: "whatsit",
        etex_type: 9,
        fields: fields![semantic("payload")],
    },
    NodeDescriptor {
        tag: NodeTag::MathOn,
        name: "math_on",
        etex_type: 10,
        fields: fields![semantic("width")],
    },
    NodeDescriptor {
        tag: NodeTag::MathOff,
        name: "math_off",
        etex_type: 10,
        fields: fields![semantic("width")],
    },
    NodeDescriptor {
        tag: NodeTag::Direction,
        name: "direction",
        etex_type: 10,
        fields: fields![semantic("boundary")],
    },
    NodeDescriptor {
        tag: NodeTag::MathNoad,
        name: "math_noad",
        etex_type: 15,
        fields: fields![
            semantic("kind"),
            semantic("nucleus"),
            semantic("subscript"),
            semantic("superscript")
        ],
    },
    NodeDescriptor {
        tag: NodeTag::FractionNoad,
        name: "fraction_noad",
        etex_type: 15,
        fields: fields![
            semantic("numerator"),
            semantic("denominator"),
            semantic("thickness"),
            semantic("left_delimiter"),
            semantic("right_delimiter")
        ],
    },
    NodeDescriptor {
        tag: NodeTag::MathStyle,
        name: "math_style",
        etex_type: 15,
        fields: fields![semantic("style")],
    },
    NodeDescriptor {
        tag: NodeTag::MathChoice,
        name: "math_choice",
        etex_type: 15,
        fields: fields![
            semantic("display"),
            semantic("text"),
            semantic("script"),
            semantic("script_script")
        ],
    },
    NodeDescriptor {
        tag: NodeTag::MathList,
        name: "math_list",
        etex_type: 15,
        fields: fields![semantic("display"), semantic("content")],
    },
    NodeDescriptor {
        tag: NodeTag::Nonscript,
        name: "nonscript",
        etex_type: 11,
        fields: fields![],
    },
    NodeDescriptor {
        tag: NodeTag::Adjust,
        name: "adjust",
        etex_type: 6,
        fields: fields![semantic("content"), semantic("pre")],
    },
];

/// Runtime handle class. Portable encoders remap each class independently.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeHandleKind {
    Font,
    Glue,
    TokenList,
    NodeList,
    Origin,
}

/// Policy attached to every handle-bearing field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeHandlePolicy {
    /// Resolve content and encode a portable content key, never raw bits.
    Content,
    /// Follow in the reported order and encode a portable list key.
    Child,
    /// Excluded from semantics; preserve only for diagnostic overlays.
    Diagnostic,
}

/// Role of a portable or diagnostic handle within its node.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeHandleRole {
    Font,
    GlueSpec,
    SplitTopSkip,
    Tokens,
    Attributes,
    Identifier,
    Child(NodeChildRole),
    CharOrigin,
    LigatureOrigins,
    MathOrigin,
    SnapGlue,
}

/// Typed borrowed handle reported by the schema.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NodeHandle<'a> {
    Font(FontId),
    Glue(GlueId),
    TokenList(TokenListId),
    NodeList(NodeListId),
    Origin(OriginId),
    Origins(&'a [OriginId]),
    OriginRefs(&'a [crate::provenance::OriginRef]),
}

impl NodeHandle<'_> {
    #[must_use]
    pub const fn kind(self) -> NodeHandleKind {
        match self {
            Self::Font(_) => NodeHandleKind::Font,
            Self::Glue(_) => NodeHandleKind::Glue,
            Self::TokenList(_) => NodeHandleKind::TokenList,
            Self::NodeList(_) => NodeHandleKind::NodeList,
            Self::Origin(_) | Self::Origins(_) | Self::OriginRefs(_) => NodeHandleKind::Origin,
        }
    }
}

/// Canonical role of an ordered child edge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeChildRole {
    Box,
    DiagnosticBox,
    Leader,
    Unset,
    DiscPre,
    DiscPost,
    DiscReplace,
    Insertion,
    Nucleus,
    Subscript,
    Superscript,
    Numerator,
    Denominator,
    Display,
    Text,
    Script,
    ScriptScript,
    MathList,
    Adjustment,
}

/// One schema event delivered without allocating.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeHandleEvent<'a> {
    pub role: NodeHandleRole,
    pub policy: NodeHandlePolicy,
    pub handle: NodeHandle<'a>,
}

/// Exhaustive visitor for the executable schema.
pub trait NodeSchemaVisitor {
    fn descriptor(&mut self, descriptor: &'static NodeDescriptor);
    fn handle(&mut self, handle: NodeHandleEvent<'_>);
}

impl NodeKind {
    /// Returns this kind's stable executable-schema descriptor.
    #[must_use]
    pub const fn descriptor(self) -> &'static NodeDescriptor {
        &DESCRIPTORS[self as usize]
    }
}

impl NodeRef<'_> {
    /// Visits this decoded node's schema without materializing or allocating.
    pub fn visit_schema(&self, visitor: &mut impl NodeSchemaVisitor) {
        visitor.descriptor(self.kind().descriptor());
        match self {
            Self::Char {
                font, origin_root, ..
            } => {
                content(visitor, NodeHandleRole::Font, NodeHandle::Font(*font));
                diagnostic_handle(
                    visitor,
                    NodeHandleRole::CharOrigin,
                    NodeHandle::Origin(origin_root.id()),
                );
            }
            Self::Lig {
                font, origin_roots, ..
            } => {
                content(visitor, NodeHandleRole::Font, NodeHandle::Font(*font));
                diagnostic_handle(
                    visitor,
                    NodeHandleRole::LigatureOrigins,
                    NodeHandle::OriginRefs(origin_roots),
                );
            }
            Self::MarginKern { font, .. } => {
                content(visitor, NodeHandleRole::Font, NodeHandle::Font(*font))
            }
            Self::Glue { spec, leader, .. } => {
                content(
                    visitor,
                    NodeHandleRole::GlueSpec,
                    NodeHandle::Glue(spec.id()),
                );
                if let Some(LeaderPayload::HList(node) | LeaderPayload::VList(node)) = leader {
                    child(visitor, NodeChildRole::Leader, node.children, false);
                    if let Some(id) = node.diagnostic_children {
                        child(visitor, NodeChildRole::DiagnosticBox, id, true);
                    }
                }
            }
            Self::HList(node) | Self::VList(node) => {
                child(visitor, NodeChildRole::Box, node.children, false);
                if let Some(id) = node.diagnostic_children {
                    child(visitor, NodeChildRole::DiagnosticBox, id, true);
                }
            }
            Self::Unset(node) => child(visitor, NodeChildRole::Unset, node.children, false),
            Self::Disc {
                pre, post, replace, ..
            } => {
                child(visitor, NodeChildRole::DiscPre, *pre, false);
                child(visitor, NodeChildRole::DiscPost, *post, false);
                child(visitor, NodeChildRole::DiscReplace, *replace, false);
            }
            Self::Mark { tokens, .. } => content(
                visitor,
                NodeHandleRole::Tokens,
                NodeHandle::TokenList(tokens.id()),
            ),
            Self::Ins {
                split_top_skip,
                content: id,
                ..
            } => {
                content(
                    visitor,
                    NodeHandleRole::SplitTopSkip,
                    NodeHandle::Glue(split_top_skip.id()),
                );
                child(visitor, NodeChildRole::Insertion, *id, false);
            }
            Self::Whatsit(value) => visit_whatsit(value, visitor),
            Self::MathNoad(noad) => {
                visit_noad_kind(&noad.kind, visitor);
                visit_math_field(&noad.nucleus, NodeChildRole::Nucleus, visitor);
                visit_math_field(&noad.subscript, NodeChildRole::Subscript, visitor);
                visit_math_field(&noad.superscript, NodeChildRole::Superscript, visitor);
            }
            Self::FractionNoad(value) => {
                child(visitor, NodeChildRole::Numerator, value.numerator, false);
                child(
                    visitor,
                    NodeChildRole::Denominator,
                    value.denominator,
                    false,
                );
            }
            Self::MathChoice(value) => {
                child(visitor, NodeChildRole::Display, value.display, false);
                child(visitor, NodeChildRole::Text, value.text, false);
                child(visitor, NodeChildRole::Script, value.script, false);
                child(
                    visitor,
                    NodeChildRole::ScriptScript,
                    value.script_script,
                    false,
                );
            }
            Self::MathList(value) => child(visitor, NodeChildRole::MathList, value.content, false),
            Self::Adjust(value) => child(visitor, NodeChildRole::Adjustment, value.content, false),
            Self::Kern { .. }
            | Self::Penalty(_)
            | Self::Rule { .. }
            | Self::MathOn(_)
            | Self::MathOff(_)
            | Self::Direction(_)
            | Self::MathStyle(_)
            | Self::Nonscript => {}
        }
    }
}

/// Compares the semantic fields declared by the logical schema. Diagnostic
/// provenance and physical-only discretionary metadata are intentionally
/// excluded for both owned and compact views.
pub(super) fn semantic_eq(left: &NodeRef<'_>, right: &NodeRef<'_>) -> bool {
    match (left, right) {
        (NodeRef::Char { font: a, ch: b, .. }, NodeRef::Char { font: c, ch: d, .. }) => {
            a == c && b == d
        }
        (
            NodeRef::Lig {
                font: a,
                ch: b,
                orig: c,
                left_hit: d,
                right_hit: e,
                ..
            },
            NodeRef::Lig {
                font: f,
                ch: g,
                orig: h,
                left_hit: i,
                right_hit: j,
                ..
            },
        ) => a == f && b == g && c == h && d == i && e == j,
        (NodeRef::Kern { amount: a, kind: b }, NodeRef::Kern { amount: c, kind: d }) => {
            a == c && b == d
        }
        (
            NodeRef::MarginKern {
                amount: a,
                side: b,
                font: c,
                ch: d,
            },
            NodeRef::MarginKern {
                amount: e,
                side: f,
                font: g,
                ch: h,
            },
        ) => a == e && b == f && c == g && d == h,
        (
            NodeRef::Glue {
                spec: a,
                kind: b,
                leader: c,
            },
            NodeRef::Glue {
                spec: d,
                kind: e,
                leader: f,
            },
        ) => a == d && b == e && c == f,
        (NodeRef::Penalty(a), NodeRef::Penalty(b)) => a == b,
        (
            NodeRef::Rule {
                width: a,
                height: b,
                depth: c,
            },
            NodeRef::Rule {
                width: d,
                height: e,
                depth: f,
            },
        ) => a == d && b == e && c == f,
        (NodeRef::HList(a), NodeRef::HList(b)) | (NodeRef::VList(a), NodeRef::VList(b)) => a == b,
        (NodeRef::Unset(a), NodeRef::Unset(b)) => a == b,
        (
            NodeRef::Disc {
                kind: a,
                pre: b,
                post: c,
                replace: d,
                ..
            },
            NodeRef::Disc {
                kind: e,
                pre: f,
                post: g,
                replace: h,
                ..
            },
        ) => a == e && b == f && c == g && d == h,
        (
            NodeRef::Mark {
                class: a,
                tokens: b,
            },
            NodeRef::Mark {
                class: c,
                tokens: d,
            },
        ) => a == c && b == d,
        (
            NodeRef::Ins {
                class: a,
                size: b,
                split_top_skip: c,
                split_max_depth: d,
                floating_penalty: e,
                content: f,
            },
            NodeRef::Ins {
                class: g,
                size: h,
                split_top_skip: i,
                split_max_depth: j,
                floating_penalty: k,
                content: l,
            },
        ) => a == g && b == h && c == i && d == j && e == k && f == l,
        (NodeRef::Whatsit(a), NodeRef::Whatsit(b)) => a == b,
        (NodeRef::MathOn(a), NodeRef::MathOn(b)) | (NodeRef::MathOff(a), NodeRef::MathOff(b)) => {
            a == b
        }
        (NodeRef::Direction(a), NodeRef::Direction(b)) => a == b,
        (NodeRef::MathNoad(a), NodeRef::MathNoad(b)) => a == b,
        (NodeRef::FractionNoad(a), NodeRef::FractionNoad(b)) => a == b,
        (NodeRef::MathStyle(a), NodeRef::MathStyle(b)) => a == b,
        (NodeRef::MathChoice(a), NodeRef::MathChoice(b)) => a == b,
        (NodeRef::MathList(a), NodeRef::MathList(b)) => a == b,
        (NodeRef::Nonscript, NodeRef::Nonscript) => true,
        (NodeRef::Adjust(a), NodeRef::Adjust(b)) => a == b,
        _ => false,
    }
}

/// Compares one node's allocation-independent physical shape. Child payloads
/// are compared by the graph walker, so only their compact coordinates are
/// erased here. Diagnostic origins and physical allocator metadata remain
/// exact, without rendering `Debug` output or allocating temporary strings.
pub(super) fn physical_shape_eq(left: &NodeRef<'_>, right: &NodeRef<'_>) -> bool {
    match (left, right) {
        (
            NodeRef::Char {
                font: a,
                ch: b,
                origin_root: c,
                ..
            },
            NodeRef::Char {
                font: d,
                ch: e,
                origin_root: f,
                ..
            },
        ) => a == d && b == e && c == f,
        (
            NodeRef::Lig {
                font: a,
                ch: b,
                orig: c,
                origin_roots: d,
                left_hit: e,
                right_hit: f,
                ..
            },
            NodeRef::Lig {
                font: g,
                ch: h,
                orig: i,
                origin_roots: j,
                left_hit: k,
                right_hit: l,
                ..
            },
        ) => a == g && b == h && c == i && d == j && e == k && f == l,
        (
            NodeRef::Glue {
                spec: a,
                kind: b,
                leader: c,
            },
            NodeRef::Glue {
                spec: d,
                kind: e,
                leader: f,
            },
        ) => {
            a == d
                && b == e
                && match (c, f) {
                    (Some(a), Some(b)) => leader_physical_shape_eq(a, b),
                    (None, None) => true,
                    _ => false,
                }
        }
        (NodeRef::HList(a), NodeRef::HList(b)) | (NodeRef::VList(a), NodeRef::VList(b)) => {
            box_physical_shape_eq(a, b)
        }
        (NodeRef::Unset(a), NodeRef::Unset(b)) => (*a).map_list(|_| ()) == (*b).map_list(|_| ()),
        (
            NodeRef::Disc {
                kind: a,
                physical_replace_count: b,
                ..
            },
            NodeRef::Disc {
                kind: c,
                physical_replace_count: d,
                ..
            },
        ) => a == c && b == d,
        (
            NodeRef::Ins {
                class: a,
                size: b,
                split_top_skip: c,
                split_max_depth: d,
                floating_penalty: e,
                ..
            },
            NodeRef::Ins {
                class: f,
                size: g,
                split_top_skip: h,
                split_max_depth: i,
                floating_penalty: j,
                ..
            },
        ) => a == f && b == g && c == h && d == i && e == j,
        (NodeRef::MathNoad(a), NodeRef::MathNoad(b)) => {
            noad_kind_physical_eq(&a.kind, &b.kind)
                && math_field_physical_shape_eq(&a.nucleus, &b.nucleus)
                && math_field_physical_shape_eq(&a.subscript, &b.subscript)
                && math_field_physical_shape_eq(&a.superscript, &b.superscript)
        }
        (NodeRef::FractionNoad(a), NodeRef::FractionNoad(b)) => {
            (*a).map_lists(|_| ()) == (*b).map_lists(|_| ())
        }
        (NodeRef::MathChoice(a), NodeRef::MathChoice(b)) => {
            (*a).map_lists(|_| ()) == (*b).map_lists(|_| ())
        }
        (NodeRef::MathList(a), NodeRef::MathList(b)) => {
            (*a).map_list(|_| ()) == (*b).map_list(|_| ())
        }
        (NodeRef::Adjust(a), NodeRef::Adjust(b)) => (*a).map_list(|_| ()) == (*b).map_list(|_| ()),
        _ => semantic_eq(left, right),
    }
}

fn box_physical_shape_eq(
    left: &crate::node::BoxNode<NodeListId>,
    right: &crate::node::BoxNode<NodeListId>,
) -> bool {
    (*left).map_lists(|_| ()) == (*right).map_lists(|_| ())
        && left.diagnostic_children.is_some() == right.diagnostic_children.is_some()
        && left.allocator_high_cell_overlap == right.allocator_high_cell_overlap
}

fn leader_physical_shape_eq(
    left: &crate::node::LeaderPayload<NodeListId>,
    right: &crate::node::LeaderPayload<NodeListId>,
) -> bool {
    match (left, right) {
        (crate::node::LeaderPayload::HList(a), crate::node::LeaderPayload::HList(b))
        | (crate::node::LeaderPayload::VList(a), crate::node::LeaderPayload::VList(b)) => {
            box_physical_shape_eq(a, b)
        }
        (
            crate::node::LeaderPayload::Rule {
                width: a,
                height: b,
                depth: c,
            },
            crate::node::LeaderPayload::Rule {
                width: d,
                height: e,
                depth: f,
            },
        ) => a == d && b == e && c == f,
        _ => false,
    }
}

fn math_char_physical_eq(left: &MathChar, right: &MathChar) -> bool {
    left.family == right.family && left.character == right.character && left.origin == right.origin
}

fn math_field_physical_shape_eq(
    left: &MathField<NodeListId>,
    right: &MathField<NodeListId>,
) -> bool {
    match (left, right) {
        (MathField::Empty, MathField::Empty)
        | (MathField::SubBox(_), MathField::SubBox(_))
        | (MathField::SubMlist(_), MathField::SubMlist(_)) => true,
        (MathField::MathChar(a), MathField::MathChar(b))
        | (MathField::MathTextChar(a), MathField::MathTextChar(b)) => math_char_physical_eq(a, b),
        _ => false,
    }
}

fn noad_kind_physical_eq(left: &NoadKind, right: &NoadKind) -> bool {
    match (left, right) {
        (NoadKind::Accent { accent: a }, NoadKind::Accent { accent: b }) => {
            math_char_physical_eq(a, b)
        }
        _ => left == right,
    }
}

fn content(visitor: &mut impl NodeSchemaVisitor, role: NodeHandleRole, handle: NodeHandle<'_>) {
    visitor.handle(NodeHandleEvent {
        role,
        policy: NodeHandlePolicy::Content,
        handle,
    });
}

fn child(
    visitor: &mut impl NodeSchemaVisitor,
    role: NodeChildRole,
    id: NodeListId,
    diagnostic: bool,
) {
    visitor.handle(NodeHandleEvent {
        role: NodeHandleRole::Child(role),
        policy: if diagnostic {
            NodeHandlePolicy::Diagnostic
        } else {
            NodeHandlePolicy::Child
        },
        handle: NodeHandle::NodeList(id),
    });
}

fn diagnostic_handle(
    visitor: &mut impl NodeSchemaVisitor,
    role: NodeHandleRole,
    handle: NodeHandle<'_>,
) {
    visitor.handle(NodeHandleEvent {
        role,
        policy: NodeHandlePolicy::Diagnostic,
        handle,
    });
}

fn visit_math_char(ch: &MathChar, visitor: &mut impl NodeSchemaVisitor) {
    diagnostic_handle(
        visitor,
        NodeHandleRole::MathOrigin,
        NodeHandle::Origin(ch.origin),
    );
}

fn visit_math_field(
    field: &MathField<NodeListId>,
    role: NodeChildRole,
    visitor: &mut impl NodeSchemaVisitor,
) {
    match field {
        MathField::MathChar(ch) | MathField::MathTextChar(ch) => visit_math_char(ch, visitor),
        MathField::SubBox(id) | MathField::SubMlist(id) => child(visitor, role, *id, false),
        MathField::Empty => {}
    }
}

fn visit_noad_kind(kind: &NoadKind, visitor: &mut impl NodeSchemaVisitor) {
    if let NoadKind::Accent { accent } = kind {
        visit_math_char(accent, visitor);
    }
}

fn visit_whatsit(value: &Whatsit, visitor: &mut impl NodeSchemaVisitor) {
    match value {
        Whatsit::DeferredWrite { tokens, .. }
        | Whatsit::DeferredSpecial { tokens, .. }
        | Whatsit::DeferredPdfLiteral { tokens, .. } => content(
            visitor,
            NodeHandleRole::Tokens,
            NodeHandle::TokenList(tokens.id()),
        ),
        Whatsit::PdfSnapY { glue } => content(
            visitor,
            NodeHandleRole::SnapGlue,
            NodeHandle::Glue(glue.id()),
        ),
        Whatsit::PdfDestination(node) => visit_identifier(&node.identifier, visitor),
        Whatsit::PdfThread(node) => {
            visit_identifier(&node.identifier, visitor);
            content(
                visitor,
                NodeHandleRole::Attributes,
                NodeHandle::TokenList(node.attributes.id()),
            );
        }
        Whatsit::OpenOut { .. }
        | Whatsit::CloseOut { .. }
        | Whatsit::Special { .. }
        | Whatsit::PdfReferenceObject { .. }
        | Whatsit::PdfAccessibility(_)
        | Whatsit::PdfAnnotation { .. }
        | Whatsit::PdfLinkStart { .. }
        | Whatsit::PdfLinkEnd { .. }
        | Whatsit::PdfRunningLink(_)
        | Whatsit::PdfLiteral { .. }
        | Whatsit::PdfSetMatrix { .. }
        | Whatsit::PdfSave
        | Whatsit::PdfRestore
        | Whatsit::PdfColorStack { .. }
        | Whatsit::PdfSavePos
        | Whatsit::PdfSnapRefPoint
        | Whatsit::PdfSnapYComp { .. }
        | Whatsit::PdfRefXForm { .. }
        | Whatsit::PdfRefXImage { .. }
        | Whatsit::PdfEndThread
        | Whatsit::Language { .. } => {}
    }
}

fn visit_identifier(value: &crate::PdfActionIdentifier, visitor: &mut impl NodeSchemaVisitor) {
    if let crate::PdfActionIdentifier::Name(tokens) | crate::PdfActionIdentifier::Raw(tokens) =
        value
    {
        content(
            visitor,
            NodeHandleRole::Identifier,
            NodeHandle::TokenList(tokens.id()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glue::Order;
    use crate::math::{
        FractionThickness, MathChoice, MathFraction, MathListNode, MathNoad, MathStyle, NoadClass,
    };
    use crate::node::{
        AdjustNode, BoxLr, BoxNode, BoxNodeFields, Direction, DiscKind, GlueKind, KernKind,
        MarginKernSide, Node, Sign, UnsetKind, UnsetNode, UnsetNodeFields,
    };
    use crate::scaled::{GlueSetRatio, Scaled};
    use crate::stores::Stores;

    #[derive(Default)]
    struct Snapshot {
        descriptor: Option<&'static NodeDescriptor>,
        handles: Vec<(NodeHandleRole, NodeHandlePolicy, String)>,
        children: Vec<(NodeHandleRole, NodeHandlePolicy, NodeListId)>,
    }

    impl NodeSchemaVisitor for Snapshot {
        fn descriptor(&mut self, descriptor: &'static NodeDescriptor) {
            self.descriptor = Some(descriptor);
        }

        fn handle(&mut self, event: NodeHandleEvent<'_>) {
            if let NodeHandle::NodeList(id) = event.handle {
                self.children.push((event.role, event.policy, id));
            }
            self.handles
                .push((event.role, event.policy, format!("{:?}", event.handle)));
        }
    }

    fn snapshot(node: &NodeRef<'_>) -> Snapshot {
        let mut snapshot = Snapshot::default();
        node.visit_schema(&mut snapshot);
        snapshot
    }

    #[test]
    fn owned_and_compact_views_have_exhaustively_equivalent_schema() {
        let empty = crate::node_arena::NodeListRef::empty();
        let mark_tokens = crate::token_store::testing_empty_token_list_ref();
        let write_tokens = crate::token_store::testing_empty_token_list_ref();
        let mut stores = Stores::new();
        let glue = stores.intern_glue_in_domain(crate::glue::GlueSpec::ZERO, None);
        let mut box_node = BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(1),
            height: Scaled::from_raw(2),
            depth: Scaled::from_raw(3),
            shift: Scaled::from_raw(4),
            box_lr: BoxLr::Reversed,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Stretching,
            glue_order: Order::Fil,
            children: empty.clone(),
        });
        box_node.diagnostic_children = Some(empty.clone());
        let unset = UnsetNode::new(UnsetNodeFields {
            kind: UnsetKind::HBox,
            width: Scaled::from_raw(5),
            height: Scaled::from_raw(6),
            depth: Scaled::from_raw(7),
            span_count: 2,
            stretch: Scaled::from_raw(8),
            stretch_order: Order::Fill,
            shrink: Scaled::from_raw(9),
            shrink_order: Order::Normal,
            children: empty.clone(),
        });
        let origin = crate::provenance::OriginRef::direct(OriginId::from_raw(17));
        let nodes = vec![
            Node::Char {
                font: crate::font::NULL_FONT,
                ch: 'a',
                origin: origin.clone(),
            },
            Node::Lig {
                font: crate::font::NULL_FONT,
                ch: 'b',
                orig: vec!['a'],
                origins: vec![origin],
                left_hit: true,
                right_hit: false,
            },
            Node::Kern {
                amount: Scaled::from_raw(10),
                kind: KernKind::Explicit,
            },
            Node::MarginKern {
                amount: Scaled::from_raw(11),
                side: MarginKernSide::Right,
                font: crate::font::NULL_FONT,
                ch: b'c',
            },
            Node::Glue {
                spec: glue.clone(),
                kind: GlueKind::Normal,
                leader: None,
            },
            Node::Penalty(-12),
            Node::Rule {
                width: Some(Scaled::from_raw(13)),
                height: None,
                depth: Some(Scaled::from_raw(14)),
            },
            Node::HList(box_node.clone()),
            Node::VList(box_node),
            Node::Unset(unset),
            Node::Disc {
                kind: DiscKind::ExplicitHyphen,
                pre: empty.clone(),
                post: empty.clone(),
                replace: empty.clone(),
                physical_replace_count: 1,
            },
            Node::Mark {
                class: 2,
                tokens: mark_tokens,
            },
            Node::Ins {
                class: 3,
                size: Scaled::from_raw(15),
                split_top_skip: glue,
                split_max_depth: Scaled::from_raw(16),
                floating_penalty: -17,
                content: empty.clone(),
            },
            Node::Whatsit(Whatsit::DeferredWrite {
                sink: crate::world::PrintSink::TerminalAndLog,
                tokens: write_tokens,
            }),
            Node::MathOn(Scaled::from_raw(18)),
            Node::MathOff(Scaled::from_raw(19)),
            Node::Direction(Direction::BeginR),
            Node::MathNoad(MathNoad::new(
                NoadKind::Normal(NoadClass::Ord),
                MathField::SubMlist(empty.clone()),
            )),
            Node::FractionNoad(MathFraction {
                numerator: empty.clone(),
                denominator: empty.clone(),
                thickness: FractionThickness::Default,
                left_delimiter: None,
                right_delimiter: Some(20),
            }),
            Node::MathStyle(MathStyle::Script),
            Node::MathChoice(MathChoice {
                display: empty.clone(),
                text: empty.clone(),
                script: empty.clone(),
                script_script: empty.clone(),
            }),
            Node::MathList(MathListNode {
                display: true,
                content: empty.clone(),
            }),
            Node::Nonscript,
            Node::Adjust(AdjustNode {
                content: empty.clone(),
                pre: true,
            }),
        ];
        let list = stores.freeze_node_list(&nodes);

        assert_eq!(nodes.len(), NodeKind::ALL.len());
        for (index, owned) in nodes.iter().enumerate() {
            let owned_ref = NodeRef::from(owned);
            let compact_ref = list_view(&list, index);
            assert_eq!(owned_ref, compact_ref);
            assert_eq!(
                owned,
                &compact_ref.to_owned_with(|_| crate::node_arena::NodeListRef::empty())
            );
            let owned_schema = snapshot(&owned_ref);
            let compact_schema = snapshot(&compact_ref);
            assert_eq!(owned_schema.descriptor, compact_schema.descriptor);
            assert_eq!(owned_schema.handles, compact_schema.handles);
            assert_eq!(
                owned_schema
                    .descriptor
                    .expect("visitor must report a descriptor")
                    .tag as usize,
                index
            );
            assert_eq!(owned.kind(), NodeKind::ALL[index]);
            assert_eq!(
                owned_schema
                    .children
                    .iter()
                    .filter(|child| child.1 == NodeHandlePolicy::Child)
                    .map(|child| child.2)
                    .collect::<Vec<_>>(),
                owned_ref.children().collect::<Vec<_>>()
            );
            assert_eq!(
                owned_schema
                    .children
                    .iter()
                    .map(|child| child.2)
                    .collect::<Vec<_>>(),
                owned_ref.physical_children().collect::<Vec<_>>()
            );
        }
    }

    fn list_view(list: &super::super::NodeListRef, index: usize) -> NodeRef<'_> {
        list.nodes().get(index).expect("schema fixture node")
    }

    #[test]
    fn child_order_origin_policy_and_semantic_field_policy_are_exact() {
        let empty = crate::node_arena::NodeListRef::empty();
        let node = Node::Disc {
            kind: DiscKind::Discretionary,
            pre: empty.clone(),
            post: empty.clone(),
            replace: empty,
            physical_replace_count: 9,
        };
        let schema = snapshot(&NodeRef::from(&node));
        assert_eq!(
            schema
                .handles
                .iter()
                .map(|event| event.0)
                .collect::<Vec<_>>(),
            [
                NodeHandleRole::Child(NodeChildRole::DiscPre),
                NodeHandleRole::Child(NodeChildRole::DiscPost),
                NodeHandleRole::Child(NodeChildRole::DiscReplace),
            ]
        );
        assert_eq!(
            schema
                .handles
                .iter()
                .map(|event| event.1)
                .collect::<Vec<_>>(),
            [NodeHandlePolicy::Child; 3]
        );
        assert_eq!(
            NodeKind::Disc.descriptor().fields.last(),
            Some(&NodeField {
                name: "physical_replace_count",
                policy: FieldPolicy::Diagnostic
            })
        );
        assert_eq!(
            NodeKind::Char.descriptor().fields.last(),
            Some(&NodeField {
                name: "origin",
                policy: FieldPolicy::Diagnostic
            })
        );
    }

    #[test]
    fn borrowed_hot_visit_uses_only_caller_storage() {
        struct FixedVisitor {
            descriptors: usize,
            handles: usize,
        }
        impl NodeSchemaVisitor for FixedVisitor {
            fn descriptor(&mut self, _: &'static NodeDescriptor) {
                self.descriptors += 1;
            }
            fn handle(&mut self, _: NodeHandleEvent<'_>) {
                self.handles += 1;
            }
        }
        let node = Node::Char {
            font: FontId::testing_new(9),
            ch: 'z',
            origin: crate::provenance::OriginRef::direct(OriginId::from_raw(23)),
        };
        let mut visitor = FixedVisitor {
            descriptors: 0,
            handles: 0,
        };
        NodeRef::from(&node).visit_schema(&mut visitor);
        assert_eq!((visitor.descriptors, visitor.handles), (1, 2));
    }
}
