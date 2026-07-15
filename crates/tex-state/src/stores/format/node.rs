use super::{FormatListKey, StoreFormatError};
use crate::glue::Order;
use crate::ids::{FontId, GlueId, NodeListId, SurvivorRootId, TokenListId};
use crate::math::{
    FractionThickness, MathChoice, MathField, MathFraction, MathListNode, MathNoad, MathStyle,
    NoadKind,
};
use crate::node::{
    BoxNode, DiscKind, GlueKind, KernKind, LeaderPayload, Node, PdfAccessibilityControl, Sign,
    PdfLiteralMode, UnsetKind, UnsetNode, Whatsit,
};
use crate::scaled::{GlueSetRatio, Scaled};
use crate::stores::Stores;
use crate::world::{PrintSink, StreamSlot};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

type SurvivorRoots = BTreeMap<SurvivorRootId, u32>;
type NodeIds = BTreeMap<FormatListKey, NodeListId>;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) enum FormatNode {
    Char {
        font: u32,
        ch: char,
    },
    Lig {
        font: u32,
        ch: char,
        orig: Vec<char>,
    },
    Kern {
        amount: Scaled,
        kind: KernKind,
    },
    Glue {
        spec: u32,
        kind: GlueKind,
        leader: Option<FormatLeaderPayload>,
    },
    Penalty(i32),
    Rule {
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
    },
    HList(FormatBoxNode),
    VList(FormatBoxNode),
    Unset(FormatUnsetNode),
    Disc {
        kind: DiscKind,
        pre: FormatListKey,
        post: FormatListKey,
        replace: FormatListKey,
    },
    Mark {
        class: u16,
        tokens: u32,
    },
    Ins {
        class: u16,
        size: Scaled,
        split_top_skip: u32,
        split_max_depth: Scaled,
        floating_penalty: i32,
        content: FormatListKey,
    },
    Whatsit(FormatWhatsit),
    MathOn(Scaled),
    MathOff(Scaled),
    Direction(crate::node::Direction),
    MathNoad(FormatMathNoad),
    FractionNoad(FormatMathFraction),
    MathStyle(MathStyle),
    MathChoice(FormatMathChoice),
    MathList(FormatMathListNode),
    Nonscript,
    Adjust(FormatListKey),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) enum FormatLeaderPayload {
    HList(FormatBoxNode),
    VList(FormatBoxNode),
    Rule {
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(super) struct FormatBoxNode {
    width: Scaled,
    height: Scaled,
    depth: Scaled,
    shift: Scaled,
    display: bool,
    glue_set: GlueSetRatio,
    glue_sign: Sign,
    glue_order: Order,
    children: FormatListKey,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(super) struct FormatUnsetNode {
    kind: UnsetKind,
    width: Scaled,
    height: Scaled,
    depth: Scaled,
    span_count: u16,
    stretch: Scaled,
    stretch_order: Order,
    shrink: Scaled,
    shrink_order: Order,
    children: FormatListKey,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) enum FormatWhatsit {
    OpenOut {
        slot: StreamSlot,
        path: String,
    },
    CloseOut {
        slot: StreamSlot,
    },
    DeferredWrite {
        sink: PrintSink,
        tokens: u32,
    },
    Special {
        class: String,
        payload: Vec<u8>,
    },
    PdfLiteral {
        mode: u8,
        payload: Vec<u8>,
    },
    DeferredPdfLiteral {
        mode: u8,
        tokens: u32,
    },
    PdfSetMatrix {
        payload: Vec<u8>,
    },
    PdfSave,
    PdfRestore,
    PdfColorStack {
        id: u32,
        action: u8,
        payload: Vec<u8>,
    },
    Language {
        language: u8,
        left_hyphen_min: u8,
        right_hyphen_min: u8,
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
}

#[derive(Clone, Debug, Deserialize, Serialize)]
enum FormatMathField {
    Empty,
    MathChar(crate::math::MathChar),
    MathTextChar(crate::math::MathChar),
    SubBox(FormatListKey),
    SubMlist(FormatListKey),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct FormatMathNoad {
    kind: NoadKind,
    nucleus: FormatMathField,
    subscript: FormatMathField,
    superscript: FormatMathField,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct FormatMathFraction {
    numerator: FormatListKey,
    denominator: FormatListKey,
    thickness: FractionThickness,
    left_delimiter: Option<u32>,
    right_delimiter: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct FormatMathChoice {
    display: FormatListKey,
    text: FormatListKey,
    script: FormatListKey,
    script_script: FormatListKey,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(super) struct FormatMathListNode {
    display: bool,
    content: FormatListKey,
}

impl FormatNode {
    pub(super) fn capture(stores: &Stores, node: Node, roots: &mut SurvivorRoots) -> Self {
        match node {
            Node::Char { font, ch, .. } => Self::Char {
                font: font.raw(),
                ch,
            },
            Node::Lig { font, ch, orig, .. } => Self::Lig {
                font: font.raw(),
                ch,
                orig,
            },
            Node::Kern { amount, kind } => Self::Kern { amount, kind },
            Node::Glue { spec, kind, leader } => Self::Glue {
                spec: spec.raw(),
                kind,
                leader: leader.map(|leader| FormatLeaderPayload::capture(stores, leader, roots)),
            },
            Node::Penalty(value) => Self::Penalty(value),
            Node::Rule {
                width,
                height,
                depth,
            } => Self::Rule {
                width,
                height,
                depth,
            },
            Node::HList(box_node) => Self::HList(FormatBoxNode::capture(stores, box_node, roots)),
            Node::VList(box_node) => Self::VList(FormatBoxNode::capture(stores, box_node, roots)),
            Node::Unset(unset) => Self::Unset(FormatUnsetNode::capture(stores, unset, roots)),
            Node::Disc {
                kind,
                pre,
                post,
                replace,
            } => Self::Disc {
                kind,
                pre: key(stores, pre, roots),
                post: key(stores, post, roots),
                replace: key(stores, replace, roots),
            },
            Node::Mark { class, tokens } => Self::Mark {
                class,
                tokens: tokens.raw(),
            },
            Node::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => Self::Ins {
                class,
                size,
                split_top_skip: split_top_skip.raw(),
                split_max_depth,
                floating_penalty,
                content: key(stores, content, roots),
            },
            Node::Whatsit(whatsit) => Self::Whatsit(FormatWhatsit::capture(whatsit)),
            Node::MathOn(value) => Self::MathOn(value),
            Node::MathOff(value) => Self::MathOff(value),
            Node::Direction(direction) => Self::Direction(direction),
            Node::MathNoad(noad) => Self::MathNoad(FormatMathNoad::capture(stores, noad, roots)),
            Node::FractionNoad(fraction) => {
                Self::FractionNoad(FormatMathFraction::capture(stores, fraction, roots))
            }
            Node::MathStyle(style) => Self::MathStyle(style),
            Node::MathChoice(choice) => {
                Self::MathChoice(FormatMathChoice::capture(stores, choice, roots))
            }
            Node::MathList(list) => {
                Self::MathList(FormatMathListNode::capture(stores, list, roots))
            }
            Node::Nonscript => Self::Nonscript,
            Node::Adjust(content) => Self::Adjust(key(stores, content, roots)),
        }
    }

    pub(super) fn restore(self, stores: &Stores, ids: &NodeIds) -> Result<Node, StoreFormatError> {
        Ok(match self {
            Self::Char { font, ch } => Node::Char {
                font: font_id(stores, font)?,
                ch,
                origin: crate::token::OriginId::UNKNOWN,
            },
            Self::Lig { font, ch, orig } => Node::Lig {
                font: font_id(stores, font)?,
                ch,
                origins: vec![crate::token::OriginId::UNKNOWN; orig.len()],
                orig,
            },
            Self::Kern { amount, kind } => Node::Kern { amount, kind },
            Self::Glue { spec, kind, leader } => Node::Glue {
                spec: glue_id(stores, spec)?,
                kind,
                leader: leader.map(|leader| leader.restore(ids)).transpose()?,
            },
            Self::Penalty(value) => Node::Penalty(value),
            Self::Rule {
                width,
                height,
                depth,
            } => Node::Rule {
                width,
                height,
                depth,
            },
            Self::HList(box_node) => Node::HList(box_node.restore(ids)?),
            Self::VList(box_node) => Node::VList(box_node.restore(ids)?),
            Self::Unset(unset) => Node::Unset(unset.restore(ids)?),
            Self::Disc {
                kind,
                pre,
                post,
                replace,
            } => Node::Disc {
                kind,
                pre: list_id(ids, pre)?,
                post: list_id(ids, post)?,
                replace: list_id(ids, replace)?,
            },
            Self::Mark { class, tokens } => Node::Mark {
                class,
                tokens: token_list_id(stores, tokens)?,
            },
            Self::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => Node::Ins {
                class,
                size,
                split_top_skip: glue_id(stores, split_top_skip)?,
                split_max_depth,
                floating_penalty,
                content: list_id(ids, content)?,
            },
            Self::Whatsit(whatsit) => Node::Whatsit(whatsit.restore(stores)?),
            Self::MathOn(value) => Node::MathOn(value),
            Self::MathOff(value) => Node::MathOff(value),
            Self::Direction(direction) => Node::Direction(direction),
            Self::MathNoad(noad) => Node::MathNoad(noad.restore(ids)?),
            Self::FractionNoad(fraction) => Node::FractionNoad(fraction.restore(ids)?),
            Self::MathStyle(style) => Node::MathStyle(style),
            Self::MathChoice(choice) => Node::MathChoice(choice.restore(ids)?),
            Self::MathList(list) => Node::MathList(list.restore(ids)?),
            Self::Nonscript => Node::Nonscript,
            Self::Adjust(content) => Node::Adjust(list_id(ids, content)?),
        })
    }
}

impl FormatBoxNode {
    fn capture(stores: &Stores, node: BoxNode, roots: &mut SurvivorRoots) -> Self {
        Self {
            width: node.width,
            height: node.height,
            depth: node.depth,
            shift: node.shift,
            display: node.display,
            glue_set: node.glue_set,
            glue_sign: node.glue_sign,
            glue_order: node.glue_order,
            children: key(stores, node.children, roots),
        }
    }

    fn restore(self, ids: &NodeIds) -> Result<BoxNode, StoreFormatError> {
        Ok(BoxNode {
            width: self.width,
            height: self.height,
            depth: self.depth,
            shift: self.shift,
            display: self.display,
            glue_set: self.glue_set,
            glue_sign: self.glue_sign,
            glue_order: self.glue_order,
            children: list_id(ids, self.children)?,
        })
    }
}

impl FormatLeaderPayload {
    fn capture(stores: &Stores, leader: LeaderPayload, roots: &mut SurvivorRoots) -> Self {
        match leader {
            LeaderPayload::HList(node) => Self::HList(FormatBoxNode::capture(stores, node, roots)),
            LeaderPayload::VList(node) => Self::VList(FormatBoxNode::capture(stores, node, roots)),
            LeaderPayload::Rule {
                width,
                height,
                depth,
            } => Self::Rule {
                width,
                height,
                depth,
            },
        }
    }

    fn restore(self, ids: &NodeIds) -> Result<LeaderPayload, StoreFormatError> {
        Ok(match self {
            Self::HList(node) => LeaderPayload::HList(node.restore(ids)?),
            Self::VList(node) => LeaderPayload::VList(node.restore(ids)?),
            Self::Rule {
                width,
                height,
                depth,
            } => LeaderPayload::Rule {
                width,
                height,
                depth,
            },
        })
    }
}

impl FormatUnsetNode {
    fn capture(stores: &Stores, node: UnsetNode, roots: &mut SurvivorRoots) -> Self {
        Self {
            kind: node.kind,
            width: node.width,
            height: node.height,
            depth: node.depth,
            span_count: node.span_count,
            stretch: node.stretch,
            stretch_order: node.stretch_order,
            shrink: node.shrink,
            shrink_order: node.shrink_order,
            children: key(stores, node.children, roots),
        }
    }

    fn restore(self, ids: &NodeIds) -> Result<UnsetNode, StoreFormatError> {
        Ok(UnsetNode {
            kind: self.kind,
            width: self.width,
            height: self.height,
            depth: self.depth,
            span_count: self.span_count,
            stretch: self.stretch,
            stretch_order: self.stretch_order,
            shrink: self.shrink,
            shrink_order: self.shrink_order,
            children: list_id(ids, self.children)?,
        })
    }
}

impl FormatWhatsit {
    fn capture(whatsit: Whatsit) -> Self {
        match whatsit {
            Whatsit::OpenOut { slot, path } => Self::OpenOut { slot, path },
            Whatsit::CloseOut { slot } => Self::CloseOut { slot },
            Whatsit::DeferredWrite { sink, tokens } => Self::DeferredWrite {
                sink,
                tokens: tokens.raw(),
            },
            Whatsit::Special { class, payload } => Self::Special { class, payload },
            Whatsit::PdfLiteral { mode, payload } => Self::PdfLiteral {
                mode: mode as u8,
                payload,
            },
            Whatsit::DeferredPdfLiteral { mode, tokens } => Self::DeferredPdfLiteral {
                mode: mode as u8,
                tokens: tokens.raw(),
            },
            Whatsit::PdfSetMatrix { payload } => Self::PdfSetMatrix { payload },
            Whatsit::PdfSave => Self::PdfSave,
            Whatsit::PdfRestore => Self::PdfRestore,
            Whatsit::PdfColorStack { id, action } => {
                let (action, payload) = match action {
                    crate::PdfColorStackAction::Set(payload) => (0, payload),
                    crate::PdfColorStackAction::Push(payload) => (1, payload),
                    crate::PdfColorStackAction::Pop => (2, Vec::new()),
                    crate::PdfColorStackAction::Current => (3, Vec::new()),
                };
                Self::PdfColorStack {
                    id,
                    action,
                    payload,
                }
            }
            Whatsit::Language {
                language,
                left_hyphen_min,
                right_hyphen_min,
            } => Self::Language {
                language,
                left_hyphen_min,
                right_hyphen_min,
            },
            Whatsit::PdfReferenceObject { object } => Self::PdfReferenceObject { object },
            Whatsit::PdfAccessibility(control) => Self::PdfAccessibility(control),
            Whatsit::PdfAnnotation { object } => Self::PdfAnnotation { object },
            Whatsit::PdfLinkStart { object } => Self::PdfLinkStart { object },
            Whatsit::PdfLinkEnd { object } => Self::PdfLinkEnd { object },
            Whatsit::PdfRunningLink(enabled) => Self::PdfRunningLink(enabled),
        }
    }

    fn restore(self, stores: &Stores) -> Result<Whatsit, StoreFormatError> {
        Ok(match self {
            Self::OpenOut { slot, path } => Whatsit::OpenOut { slot, path },
            Self::CloseOut { slot } => Whatsit::CloseOut { slot },
            Self::DeferredWrite { sink, tokens } => Whatsit::DeferredWrite {
                sink,
                tokens: token_list_id(stores, tokens)?,
            },
            Self::Special { class, payload } => Whatsit::Special { class, payload },
            Self::PdfLiteral { mode, payload } => Whatsit::PdfLiteral {
                mode: pdf_literal_mode(mode)?,
                payload,
            },
            Self::DeferredPdfLiteral { mode, tokens } => Whatsit::DeferredPdfLiteral {
                mode: pdf_literal_mode(mode)?,
                tokens: token_list_id(stores, tokens)?,
            },
            Self::PdfSetMatrix { payload } => Whatsit::PdfSetMatrix { payload },
            Self::PdfSave => Whatsit::PdfSave,
            Self::PdfRestore => Whatsit::PdfRestore,
            Self::PdfColorStack {
                id,
                action,
                payload,
            } => Whatsit::PdfColorStack {
                id,
                action: match action {
                    0 => crate::PdfColorStackAction::Set(payload),
                    1 => crate::PdfColorStackAction::Push(payload),
                    2 => crate::PdfColorStackAction::Pop,
                    3 => crate::PdfColorStackAction::Current,
                    _ => return Err(StoreFormatError::Invalid("PDF color stack action")),
                },
            },
            Self::Language {
                language,
                left_hyphen_min,
                right_hyphen_min,
            } => Whatsit::Language {
                language,
                left_hyphen_min,
                right_hyphen_min,
            },
            Self::PdfReferenceObject { object } => Whatsit::PdfReferenceObject { object },
            Self::PdfAccessibility(control) => Whatsit::PdfAccessibility(control),
            Self::PdfAnnotation { object } => Whatsit::PdfAnnotation { object },
            Self::PdfLinkStart { object } => Whatsit::PdfLinkStart { object },
            Self::PdfLinkEnd { object } => Whatsit::PdfLinkEnd { object },
            Self::PdfRunningLink(enabled) => Whatsit::PdfRunningLink(enabled),
        })
    }
}

fn pdf_literal_mode(mode: u8) -> Result<PdfLiteralMode, StoreFormatError> {
    match mode {
        0 => Ok(PdfLiteralMode::Origin),
        1 => Ok(PdfLiteralMode::Page),
        2 => Ok(PdfLiteralMode::Direct),
        _ => Err(StoreFormatError::Invalid("PDF literal mode")),
    }
}

impl FormatMathField {
    fn capture(stores: &Stores, field: MathField, roots: &mut SurvivorRoots) -> Self {
        match field {
            MathField::Empty => Self::Empty,
            MathField::MathChar(value) => Self::MathChar(value),
            MathField::MathTextChar(value) => Self::MathTextChar(value),
            MathField::SubBox(id) => Self::SubBox(key(stores, id, roots)),
            MathField::SubMlist(id) => Self::SubMlist(key(stores, id, roots)),
        }
    }

    fn restore(self, ids: &NodeIds) -> Result<MathField, StoreFormatError> {
        Ok(match self {
            Self::Empty => MathField::Empty,
            Self::MathChar(value) => MathField::MathChar(value),
            Self::MathTextChar(value) => MathField::MathTextChar(value),
            Self::SubBox(key) => MathField::SubBox(list_id(ids, key)?),
            Self::SubMlist(key) => MathField::SubMlist(list_id(ids, key)?),
        })
    }
}

impl FormatMathNoad {
    fn capture(stores: &Stores, noad: MathNoad, roots: &mut SurvivorRoots) -> Self {
        Self {
            kind: noad.kind,
            nucleus: FormatMathField::capture(stores, noad.nucleus, roots),
            subscript: FormatMathField::capture(stores, noad.subscript, roots),
            superscript: FormatMathField::capture(stores, noad.superscript, roots),
        }
    }

    fn restore(self, ids: &NodeIds) -> Result<MathNoad, StoreFormatError> {
        Ok(MathNoad {
            kind: self.kind,
            nucleus: self.nucleus.restore(ids)?,
            subscript: self.subscript.restore(ids)?,
            superscript: self.superscript.restore(ids)?,
        })
    }
}

impl FormatMathFraction {
    fn capture(stores: &Stores, value: MathFraction, roots: &mut SurvivorRoots) -> Self {
        Self {
            numerator: key(stores, value.numerator, roots),
            denominator: key(stores, value.denominator, roots),
            thickness: value.thickness,
            left_delimiter: value.left_delimiter,
            right_delimiter: value.right_delimiter,
        }
    }

    fn restore(self, ids: &NodeIds) -> Result<MathFraction, StoreFormatError> {
        Ok(MathFraction {
            numerator: list_id(ids, self.numerator)?,
            denominator: list_id(ids, self.denominator)?,
            thickness: self.thickness,
            left_delimiter: self.left_delimiter,
            right_delimiter: self.right_delimiter,
        })
    }
}

impl FormatMathChoice {
    fn capture(stores: &Stores, value: MathChoice, roots: &mut SurvivorRoots) -> Self {
        Self {
            display: key(stores, value.display, roots),
            text: key(stores, value.text, roots),
            script: key(stores, value.script, roots),
            script_script: key(stores, value.script_script, roots),
        }
    }

    fn restore(self, ids: &NodeIds) -> Result<MathChoice, StoreFormatError> {
        Ok(MathChoice {
            display: list_id(ids, self.display)?,
            text: list_id(ids, self.text)?,
            script: list_id(ids, self.script)?,
            script_script: list_id(ids, self.script_script)?,
        })
    }
}

impl FormatMathListNode {
    fn capture(stores: &Stores, value: MathListNode, roots: &mut SurvivorRoots) -> Self {
        Self {
            display: value.display,
            content: key(stores, value.content, roots),
        }
    }

    fn restore(self, ids: &NodeIds) -> Result<MathListNode, StoreFormatError> {
        Ok(MathListNode {
            display: self.display,
            content: list_id(ids, self.content)?,
        })
    }
}

fn key(stores: &Stores, id: NodeListId, roots: &mut SurvivorRoots) -> FormatListKey {
    FormatListKey::capture(stores, id, roots)
}

fn list_id(ids: &NodeIds, key: FormatListKey) -> Result<NodeListId, StoreFormatError> {
    ids.get(&key)
        .copied()
        .ok_or(StoreFormatError::Invalid("node child precedes dependency"))
}

fn font_id(stores: &Stores, raw: u32) -> Result<FontId, StoreFormatError> {
    stores
        .fonts
        .resolve_stored(FontId::new(raw))
        .ok_or(StoreFormatError::Invalid("node font reference"))
}

fn glue_id(stores: &Stores, raw: u32) -> Result<GlueId, StoreFormatError> {
    stores
        .glue
        .resolve_stored(GlueId::new(raw))
        .ok_or(StoreFormatError::Invalid("node glue reference"))
}

fn token_list_id(stores: &Stores, raw: u32) -> Result<TokenListId, StoreFormatError> {
    stores
        .tokens
        .resolve_stored(TokenListId::new(raw))
        .ok_or(StoreFormatError::Invalid("node token-list reference"))
}
