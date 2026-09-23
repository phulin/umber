//! Borrowed projections and cursors over resident node records.

use super::PageListId;
use crate::glue::GlueSpec;
use crate::node::{Node, NodeTokenList};

/// Zero-allocation logical projection of one immutable node.
#[derive(Clone, Debug, PartialEq)]
pub enum NodeView<'a, List = PageListId, Glue = GlueSpec, Tokens = NodeTokenList> {
    Char {
        font: crate::ids::FontId,
        ch: char,
        origin: crate::token::OriginId,
    },
    Lig {
        font: crate::ids::FontId,
        ch: char,
        orig: std::borrow::Cow<'a, [char]>,
        origins: std::borrow::Cow<'a, [crate::token::OriginId]>,
        left_hit: bool,
        right_hit: bool,
    },
    Kern {
        amount: crate::scaled::Scaled,
        kind: crate::node::KernKind,
    },
    MarginKern {
        amount: crate::scaled::Scaled,
        side: crate::node::MarginKernSide,
        font: crate::ids::FontId,
        ch: u8,
    },
    Glue {
        spec: Glue,
        kind: crate::node::GlueKind,
        leader: Option<crate::node::LeaderPayload<List>>,
    },
    Penalty(i32),
    Rule {
        width: Option<crate::scaled::Scaled>,
        height: Option<crate::scaled::Scaled>,
        depth: Option<crate::scaled::Scaled>,
    },
    HList(crate::node::BoxNode<List>),
    VList(crate::node::BoxNode<List>),
    Unset(crate::node::UnsetNode<List>),
    Disc {
        kind: crate::node::DiscKind,
        pre: List,
        post: List,
        replace: List,
        physical_replace_count: u8,
    },
    Mark {
        class: u16,
        tokens: Tokens,
    },
    Ins {
        class: u16,
        size: crate::scaled::Scaled,
        split_top_skip: Glue,
        split_max_depth: crate::scaled::Scaled,
        floating_penalty: i32,
        content: List,
    },
    Whatsit(crate::node::Whatsit<Glue, Tokens>),
    MathOn(crate::scaled::Scaled),
    MathOff(crate::scaled::Scaled),
    Direction(crate::node::Direction),
    MathNoad(crate::math::MathNoad<List>),
    FractionNoad(crate::math::MathFraction<List>),
    MathStyle(crate::math::MathStyle),
    MathChoice(crate::math::MathChoice<List>),
    MathList(crate::math::MathListNode<List>),
    Nonscript,
    Adjust(crate::node::AdjustNode<List>),
}

/// Transitional source alias. Production consumers use [`NodeView`]; the
/// alias remains only so out-of-tree callers can migrate without naming the
/// resident enum.
#[deprecated(note = "use NodeView")]
pub type NodeRef<'a, List = PageListId, Glue = GlueSpec, Tokens = NodeTokenList> =
    NodeView<'a, List, Glue, Tokens>;

impl<'a, List: Copy, Glue: Copy, Tokens: Clone> From<&'a Node<List, Glue, Tokens>>
    for NodeView<'a, List, Glue, Tokens>
{
    fn from(node: &'a Node<List, Glue, Tokens>) -> Self {
        match node {
            Node::Char { font, ch, origin } => Self::Char {
                font: *font,
                ch: *ch,
                origin: *origin,
            },
            Node::Lig {
                font,
                ch,
                orig,
                left_hit,
                right_hit,
                origins,
            } => Self::Lig {
                font: *font,
                ch: *ch,
                orig: std::borrow::Cow::Borrowed(orig),
                origins: std::borrow::Cow::Borrowed(origins),
                left_hit: *left_hit,
                right_hit: *right_hit,
            },
            Node::Kern { amount, kind } => Self::Kern {
                amount: *amount,
                kind: *kind,
            },
            Node::MarginKern {
                amount,
                side,
                font,
                ch,
            } => Self::MarginKern {
                amount: *amount,
                side: *side,
                font: *font,
                ch: *ch,
            },
            Node::Glue { spec, kind, leader } => Self::Glue {
                spec: *spec,
                kind: *kind,
                leader: *leader,
            },
            Node::Penalty(value) => Self::Penalty(*value),
            Node::Rule {
                width,
                height,
                depth,
            } => Self::Rule {
                width: *width,
                height: *height,
                depth: *depth,
            },
            Node::HList(value) => Self::HList(*value),
            Node::VList(value) => Self::VList(*value),
            Node::Unset(value) => Self::Unset(*value),
            Node::Disc {
                kind,
                pre,
                post,
                replace,
                physical_replace_count,
            } => Self::Disc {
                kind: *kind,
                pre: *pre,
                post: *post,
                replace: *replace,
                physical_replace_count: *physical_replace_count,
            },
            Node::Mark { class, tokens } => Self::Mark {
                class: *class,
                tokens: tokens.clone(),
            },
            Node::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => Self::Ins {
                class: *class,
                size: *size,
                split_top_skip: *split_top_skip,
                split_max_depth: *split_max_depth,
                floating_penalty: *floating_penalty,
                content: *content,
            },
            Node::Whatsit(value) => Self::Whatsit(value.clone()),
            Node::MathOn(value) => Self::MathOn(*value),
            Node::MathOff(value) => Self::MathOff(*value),
            Node::Direction(value) => Self::Direction(*value),
            Node::MathNoad(value) => Self::MathNoad(value.clone()),
            Node::FractionNoad(value) => Self::FractionNoad(*value),
            Node::MathStyle(value) => Self::MathStyle(*value),
            Node::MathChoice(value) => Self::MathChoice(*value),
            Node::MathList(value) => Self::MathList(*value),
            Node::Nonscript => Self::Nonscript,
            Node::Adjust(value) => Self::Adjust(*value),
        }
    }
}

impl NodeView<'static> {
    pub(crate) fn from_owned(node: Node) -> Self {
        match node {
            Node::Char { font, ch, origin } => Self::Char { font, ch, origin },
            Node::Lig {
                font,
                ch,
                orig,
                origins,
                left_hit,
                right_hit,
            } => Self::Lig {
                font,
                ch,
                orig: std::borrow::Cow::Owned(orig),
                origins: std::borrow::Cow::Owned(origins),
                left_hit,
                right_hit,
            },
            Node::Kern { amount, kind } => Self::Kern { amount, kind },
            Node::MarginKern {
                amount,
                side,
                font,
                ch,
            } => Self::MarginKern {
                amount,
                side,
                font,
                ch,
            },
            Node::Glue { spec, kind, leader } => Self::Glue { spec, kind, leader },
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
            Node::HList(value) => Self::HList(value),
            Node::VList(value) => Self::VList(value),
            Node::Unset(value) => Self::Unset(value),
            Node::Disc {
                kind,
                pre,
                post,
                replace,
                physical_replace_count,
            } => Self::Disc {
                kind,
                pre,
                post,
                replace,
                physical_replace_count,
            },
            Node::Mark { class, tokens } => Self::Mark { class, tokens },
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
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            },
            Node::Whatsit(value) => Self::Whatsit(value),
            Node::MathOn(value) => Self::MathOn(value),
            Node::MathOff(value) => Self::MathOff(value),
            Node::Direction(value) => Self::Direction(value),
            Node::MathNoad(value) => Self::MathNoad(value),
            Node::FractionNoad(value) => Self::FractionNoad(value),
            Node::MathStyle(value) => Self::MathStyle(value),
            Node::MathChoice(value) => Self::MathChoice(value),
            Node::MathList(value) => Self::MathList(value),
            Node::Nonscript => Self::Nonscript,
            Node::Adjust(value) => Self::Adjust(value),
        }
    }
}

/// Dimension-bearing projection shared by page-arena and operation buffers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PackedNode<'a> {
    Glyph {
        font: crate::ids::FontId,
        ch: char,
    },
    Kern {
        amount: crate::scaled::Scaled,
        kind: Option<crate::node::KernKind>,
    },
    Glue {
        spec: GlueSpec,
        leader: Option<&'a crate::node::LeaderPayload<PageListId>>,
    },
    Rule {
        width: Option<crate::scaled::Scaled>,
        height: Option<crate::scaled::Scaled>,
        depth: Option<crate::scaled::Scaled>,
    },
    Box(crate::node::BoxNode<PageListId>),
    Unset(crate::node::UnsetNode<PageListId>),
    Disc(PageListId),
    Image {
        width: crate::scaled::Scaled,
        height: crate::scaled::Scaled,
        depth: crate::scaled::Scaled,
    },
    Math(crate::scaled::Scaled),
    Ignored,
}

impl NodeView<'_> {
    pub(crate) fn tex_memory_words(&self, etex_node_sizes: bool) -> (usize, usize) {
        let synctex_extra = usize::from(etex_node_sizes) * 2;
        let variable = match self {
            Self::Char { .. } => return (0, 1),
            Self::Lig { orig, .. } => return (2, orig.len()),
            Self::HList(_) | Self::VList(_) | Self::Unset(_) => 7 + synctex_extra,
            Self::Rule { .. } => 4 + synctex_extra,
            Self::Ins { .. } => 5,
            Self::MathNoad(noad) => match noad.kind {
                crate::math::NoadKind::Radical { .. } | crate::math::NoadKind::Accent { .. } => 5,
                _ => 4,
            },
            Self::FractionNoad(_) => 6,
            Self::MathStyle(_) | Self::MathChoice(_) | Self::MarginKern { .. } => 3,
            Self::Kern { .. }
            | Self::Glue { .. }
            | Self::Penalty(_)
            | Self::MathOn(_)
            | Self::MathOff(_)
            | Self::Nonscript => 2 + synctex_extra,
            Self::Direction(_) if etex_node_sizes => 2 + synctex_extra,
            Self::Disc { .. }
            | Self::Mark { .. }
            | Self::Whatsit(_)
            | Self::Direction(_)
            | Self::MathList(_)
            | Self::Adjust(_) => 2,
        };
        (variable, 0)
    }

    pub fn visit_semantic_node_lists(&self, mut visit: impl FnMut(&PageListId)) {
        fn field(field: &crate::math::MathField<PageListId>, visit: &mut impl FnMut(&PageListId)) {
            if let crate::math::MathField::SubBox(list) | crate::math::MathField::SubMlist(list) =
                field
            {
                visit(list);
            }
        }
        match self {
            Self::HList(node) | Self::VList(node) => visit(&node.children),
            Self::Unset(node) => visit(&node.children),
            Self::Glue {
                leader:
                    Some(
                        crate::node::LeaderPayload::HList(node)
                        | crate::node::LeaderPayload::VList(node),
                    ),
                ..
            } => visit(&node.children),
            Self::Disc {
                pre, post, replace, ..
            } => {
                visit(pre);
                visit(post);
                visit(replace);
            }
            Self::Ins { content, .. } => visit(content),
            Self::MathNoad(noad) => {
                field(&noad.nucleus, &mut visit);
                field(&noad.subscript, &mut visit);
                field(&noad.superscript, &mut visit);
            }
            Self::FractionNoad(fraction) => {
                visit(&fraction.numerator);
                visit(&fraction.denominator);
            }
            Self::MathChoice(choice) => {
                visit(&choice.display);
                visit(&choice.text);
                visit(&choice.script);
                visit(&choice.script_script);
            }
            Self::MathList(list) => visit(&list.content),
            Self::Adjust(adjustment) => visit(&adjustment.content),
            _ => {}
        }
    }

    #[must_use]
    pub const fn kind(&self) -> crate::node::NodeKind {
        use crate::node::NodeKind;
        match self {
            Self::Char { .. } => NodeKind::Char,
            Self::Lig { .. } => NodeKind::Lig,
            Self::Kern { .. } => NodeKind::Kern,
            Self::MarginKern { .. } => NodeKind::MarginKern,
            Self::Glue { .. } => NodeKind::Glue,
            Self::Penalty(_) => NodeKind::Penalty,
            Self::Rule { .. } => NodeKind::Rule,
            Self::HList(_) => NodeKind::HList,
            Self::VList(_) => NodeKind::VList,
            Self::Unset(_) => NodeKind::Unset,
            Self::Disc { .. } => NodeKind::Disc,
            Self::Mark { .. } => NodeKind::Mark,
            Self::Ins { .. } => NodeKind::Ins,
            Self::Whatsit(_) => NodeKind::Whatsit,
            Self::MathOn(_) => NodeKind::MathOn,
            Self::MathOff(_) => NodeKind::MathOff,
            Self::Direction(_) => NodeKind::Direction,
            Self::MathNoad(_) => NodeKind::MathNoad,
            Self::FractionNoad(_) => NodeKind::FractionNoad,
            Self::MathStyle(_) => NodeKind::MathStyle,
            Self::MathChoice(_) => NodeKind::MathChoice,
            Self::MathList(_) => NodeKind::MathList,
            Self::Nonscript => NodeKind::Nonscript,
            Self::Adjust(_) => NodeKind::Adjust,
        }
    }

    #[must_use]
    pub const fn etex_type(&self) -> i32 {
        self.kind().etex_type()
    }

    #[must_use]
    pub fn packed(&self) -> PackedNode<'_> {
        match self {
            Self::Char { font, ch, .. } | Self::Lig { font, ch, .. } => PackedNode::Glyph {
                font: *font,
                ch: *ch,
            },
            Self::Kern { amount, kind } => PackedNode::Kern {
                amount: *amount,
                kind: Some(*kind),
            },
            Self::MarginKern { amount, .. } => PackedNode::Kern {
                amount: *amount,
                kind: None,
            },
            Self::Glue { spec, leader, .. } => PackedNode::Glue {
                spec: *spec,
                leader: leader.as_ref(),
            },
            Self::Rule {
                width,
                height,
                depth,
            } => PackedNode::Rule {
                width: *width,
                height: *height,
                depth: *depth,
            },
            Self::HList(value) | Self::VList(value) => PackedNode::Box(*value),
            Self::Unset(value) => PackedNode::Unset(*value),
            Self::Disc { replace, .. } => PackedNode::Disc(*replace),
            Self::Whatsit(
                crate::node::Whatsit::PdfRefXForm {
                    width,
                    height,
                    depth,
                    ..
                }
                | crate::node::Whatsit::PdfRefXImage {
                    width,
                    height,
                    depth,
                    ..
                },
            ) => PackedNode::Image {
                width: *width,
                height: *height,
                depth: *depth,
            },
            Self::MathOn(value) | Self::MathOff(value) => PackedNode::Math(*value),
            _ => PackedNode::Ignored,
        }
    }

    #[must_use]
    pub fn vertical_dimensions(&self) -> Option<(crate::scaled::Scaled, crate::scaled::Scaled)> {
        match self.packed() {
            PackedNode::Box(node) => Some((node.height, node.depth)),
            PackedNode::Unset(node) => Some((node.height, node.depth)),
            PackedNode::Rule { height, depth, .. } => Some((
                height.unwrap_or(crate::scaled::Scaled::from_raw(0)),
                depth.unwrap_or(crate::scaled::Scaled::from_raw(0)),
            )),
            _ => None,
        }
    }

    #[must_use]
    pub fn box_node(&self) -> Option<crate::node::BoxNode<PageListId>> {
        match self.packed() {
            PackedNode::Box(node) => Some(node),
            _ => None,
        }
    }

    #[must_use]
    pub fn to_owned_with(&self, _resolve: impl FnMut(PageListId) -> PageListId) -> Node {
        match self {
            Self::Char { font, ch, origin } => Node::Char {
                font: *font,
                ch: *ch,
                origin: *origin,
            },
            Self::Lig {
                font,
                ch,
                orig,
                origins,
                left_hit,
                right_hit,
            } => Node::Lig {
                font: *font,
                ch: *ch,
                orig: orig.to_vec(),
                left_hit: *left_hit,
                right_hit: *right_hit,
                origins: origins.to_vec(),
            },
            Self::Kern { amount, kind } => Node::Kern {
                amount: *amount,
                kind: *kind,
            },
            Self::MarginKern {
                amount,
                side,
                font,
                ch,
            } => Node::MarginKern {
                amount: *amount,
                side: *side,
                font: *font,
                ch: *ch,
            },
            Self::Glue { spec, kind, leader } => Node::Glue {
                spec: *spec,
                kind: *kind,
                leader: *leader,
            },
            Self::Penalty(value) => Node::Penalty(*value),
            Self::Rule {
                width,
                height,
                depth,
            } => Node::Rule {
                width: *width,
                height: *height,
                depth: *depth,
            },
            Self::HList(value) => Node::HList(*value),
            Self::VList(value) => Node::VList(*value),
            Self::Unset(value) => Node::Unset(*value),
            Self::Disc {
                kind,
                pre,
                post,
                replace,
                physical_replace_count,
            } => Node::Disc {
                kind: *kind,
                pre: *pre,
                post: *post,
                replace: *replace,
                physical_replace_count: *physical_replace_count,
            },
            Self::Mark { class, tokens } => Node::Mark {
                class: *class,
                tokens: *tokens,
            },
            Self::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => Node::Ins {
                class: *class,
                size: *size,
                split_top_skip: *split_top_skip,
                split_max_depth: *split_max_depth,
                floating_penalty: *floating_penalty,
                content: *content,
            },
            Self::Whatsit(value) => Node::Whatsit((*value).clone()),
            Self::MathOn(value) => Node::MathOn(*value),
            Self::MathOff(value) => Node::MathOff(*value),
            Self::Direction(value) => Node::Direction(*value),
            Self::MathNoad(value) => Node::MathNoad(value.clone()),
            Self::FractionNoad(value) => Node::FractionNoad(*value),
            Self::MathStyle(value) => Node::MathStyle(*value),
            Self::MathChoice(value) => Node::MathChoice(*value),
            Self::MathList(value) => Node::MathList(*value),
            Self::Nonscript => Node::Nonscript,
            Self::Adjust(value) => Node::Adjust(*value),
        }
    }
}
