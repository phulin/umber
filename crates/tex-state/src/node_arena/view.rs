use super::NodeListRef;
use super::storage::{NodeStorage, NodeWord, decode_glue, decode_kern, decode_style};
use crate::glue::GlueSpecRef;
use crate::ids::{GlueId, NodeListId};
use crate::math::MathStyle;
use crate::node::{
    BoxNode, Direction, DiscKind, GlueKind, KernKind, LeaderPayload, MarginKernSide, Node,
    NodeKind, UnsetNode, UnsetNodeFields, Whatsit,
};
use crate::provenance::OriginRef;
use crate::scaled::Scaled;
use crate::token::OriginId;
use crate::token_store::TokenListRef;

/// A zero-allocation logical view of one compact arena node.
#[derive(Clone, Debug)]
pub enum NodeRef<'a> {
    Char {
        font: crate::ids::FontId,
        ch: char,
        origin: OriginId,
        origin_root: &'a OriginRef,
    },
    Lig {
        font: crate::ids::FontId,
        ch: char,
        orig: &'a [char],
        origins: &'a [OriginId],
        origin_roots: &'a [OriginRef],
        left_hit: bool,
        right_hit: bool,
    },
    Kern {
        amount: Scaled,
        kind: KernKind,
    },
    MarginKern {
        amount: Scaled,
        side: MarginKernSide,
        font: crate::ids::FontId,
        ch: u8,
    },
    Glue {
        spec: &'a GlueSpecRef,
        kind: GlueKind,
        leader: Option<crate::node::LeaderPayload<NodeListId>>,
    },
    Penalty(i32),
    Rule {
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
    },
    HList(BoxNode<NodeListId>),
    VList(BoxNode<NodeListId>),
    Unset(UnsetNode<NodeListId>),
    Disc {
        kind: DiscKind,
        pre: NodeListId,
        post: NodeListId,
        replace: NodeListId,
        physical_replace_count: u8,
    },
    Mark {
        class: u16,
        tokens: &'a TokenListRef,
    },
    Ins {
        class: u16,
        size: Scaled,
        split_top_skip: &'a GlueSpecRef,
        split_max_depth: Scaled,
        floating_penalty: i32,
        content: NodeListId,
    },
    Whatsit(&'a crate::node::Whatsit),
    MathOn(Scaled),
    MathOff(Scaled),
    Direction(Direction),
    MathNoad(crate::math::MathNoad<NodeListId>),
    FractionNoad(crate::math::MathFraction<NodeListId>),
    MathStyle(MathStyle),
    MathChoice(crate::math::MathChoice<NodeListId>),
    MathList(crate::math::MathListNode<NodeListId>),
    Nonscript,
    Adjust(crate::node::AdjustNode<NodeListId>),
}

/// Dimension-bearing projection shared by owned and compact nodes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PackedNode<'a> {
    Glyph {
        font: crate::ids::FontId,
        ch: char,
    },
    Kern {
        amount: Scaled,
        kind: Option<KernKind>,
    },
    Glue {
        spec: GlueId,
        leader: Option<&'a LeaderPayload<NodeListId>>,
    },
    Rule {
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
    },
    Box(BoxNode<NodeListId>),
    Unset(UnsetNode<NodeListId>),
    Disc(NodeListId),
    Image {
        width: Scaled,
        height: Scaled,
        depth: Scaled,
    },
    Math(Scaled),
    Ignored,
}

impl<'a> From<&'a Node> for NodeRef<'a> {
    fn from(node: &'a Node) -> Self {
        match node {
            Node::Char { font, ch, origin } => Self::Char {
                font: *font,
                ch: *ch,
                origin: origin.id(),
                origin_root: origin,
            },
            Node::Lig {
                font,
                ch,
                orig,
                origins,
                left_hit,
                right_hit,
            } => Self::Lig {
                font: *font,
                ch: *ch,
                orig,
                origins: &[],
                origin_roots: origins,
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
                spec,
                kind: *kind,
                leader: leader
                    .clone()
                    .map(|value| value.map_lists(|list| list.id())),
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
            Node::HList(value) => Self::HList(value.clone().map_lists(|list| list.id())),
            Node::VList(value) => Self::VList(value.clone().map_lists(|list| list.id())),
            Node::Unset(value) => Self::Unset(value.clone().map_list(|list| list.id())),
            Node::Disc {
                kind,
                pre,
                post,
                replace,
                physical_replace_count,
            } => Self::Disc {
                kind: *kind,
                pre: pre.id(),
                post: post.id(),
                replace: replace.id(),
                physical_replace_count: *physical_replace_count,
            },
            Node::Mark { class, tokens } => Self::Mark {
                class: *class,
                tokens,
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
                split_top_skip,
                split_max_depth: *split_max_depth,
                floating_penalty: *floating_penalty,
                content: content.id(),
            },
            Node::Whatsit(value) => Self::Whatsit(value),
            Node::MathOn(value) => Self::MathOn(*value),
            Node::MathOff(value) => Self::MathOff(*value),
            Node::Direction(value) => Self::Direction(*value),
            Node::MathNoad(value) => Self::MathNoad(value.clone().map_lists(|list| list.id())),
            Node::FractionNoad(value) => {
                Self::FractionNoad(value.clone().map_lists(|list| list.id()))
            }
            Node::MathStyle(value) => Self::MathStyle(*value),
            Node::MathChoice(value) => Self::MathChoice(value.clone().map_lists(|list| list.id())),
            Node::MathList(value) => Self::MathList(value.clone().map_list(|list| list.id())),
            Node::Nonscript => Self::Nonscript,
            Node::Adjust(value) => Self::Adjust(value.clone().map_list(|list| list.id())),
        }
    }
}

impl PartialEq for NodeRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        super::schema::semantic_eq(self, other)
    }
}

impl NodeRef<'_> {
    #[must_use]
    pub const fn kind(&self) -> NodeKind {
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

    /// e-TeX `\lastnodetype` code independent of the node's storage source.
    #[must_use]
    pub const fn etex_type(&self) -> i32 {
        self.kind().etex_type()
    }

    /// Materializes an owned node for builder/list-surgery output, never for storage.
    #[must_use]
    pub fn to_owned_with(&self, mut resolve: impl FnMut(NodeListId) -> NodeListRef) -> Node {
        match self {
            Self::Char {
                font,
                ch,
                origin_root,
                ..
            } => Node::Char {
                font: *font,
                ch: *ch,
                origin: (*origin_root).clone(),
            },
            Self::Lig {
                font,
                ch,
                orig,
                origin_roots,
                left_hit,
                right_hit,
                ..
            } => Node::Lig {
                font: *font,
                ch: *ch,
                orig: orig.to_vec(),
                origins: origin_roots.to_vec(),
                left_hit: *left_hit,
                right_hit: *right_hit,
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
                spec: *(*spec),
                kind: *kind,
                leader: (*leader).map(|value| value.map_lists(&mut resolve)),
            },
            Self::Penalty(v) => Node::Penalty(*v),
            Self::Rule {
                width,
                height,
                depth,
            } => Node::Rule {
                width: *width,
                height: *height,
                depth: *depth,
            },
            Self::HList(v) => Node::HList((*v).map_lists(&mut resolve)),
            Self::VList(v) => Node::VList((*v).map_lists(&mut resolve)),
            Self::Unset(v) => Node::Unset((*v).map_list(&mut resolve)),
            Self::Disc {
                kind,
                pre,
                post,
                replace,
                physical_replace_count,
            } => Node::Disc {
                kind: *kind,
                pre: resolve(*pre),
                post: resolve(*post),
                replace: resolve(*replace),
                physical_replace_count: *physical_replace_count,
            },
            Self::Mark { class, tokens } => Node::Mark {
                class: *class,
                tokens: *(*tokens),
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
                split_top_skip: *(*split_top_skip),
                split_max_depth: *split_max_depth,
                floating_penalty: *floating_penalty,
                content: resolve(*content),
            },
            Self::Whatsit(v) => Node::Whatsit((*v).clone()),
            Self::MathOn(v) => Node::MathOn(*v),
            Self::MathOff(v) => Node::MathOff(*v),
            Self::Direction(v) => Node::Direction(*v),
            Self::MathNoad(v) => Node::MathNoad(v.clone().map_lists(&mut resolve)),
            Self::FractionNoad(v) => Node::FractionNoad((*v).map_lists(&mut resolve)),
            Self::MathStyle(v) => Node::MathStyle(*v),
            Self::MathChoice(v) => Node::MathChoice((*v).map_lists(&mut resolve)),
            Self::MathList(v) => Node::MathList((*v).map_list(&mut resolve)),
            Self::Nonscript => Node::Nonscript,
            Self::Adjust(v) => Node::Adjust((*v).map_list(resolve)),
        }
    }

    /// Projects the fields used by packing and line-width algorithms.
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
                spec: spec.id(),
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
                Whatsit::PdfRefXForm {
                    width,
                    height,
                    depth,
                    ..
                }
                | Whatsit::PdfRefXImage {
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
            Self::Penalty(_)
            | Self::Mark { .. }
            | Self::Ins { .. }
            | Self::Whatsit(_)
            | Self::Direction(_)
            | Self::MathNoad(_)
            | Self::FractionNoad(_)
            | Self::MathStyle(_)
            | Self::MathChoice(_)
            | Self::MathList(_)
            | Self::Nonscript
            | Self::Adjust(_) => PackedNode::Ignored,
        }
    }

    #[must_use]
    pub fn box_node(&self) -> Option<BoxNode<NodeListId>> {
        match self.packed() {
            PackedNode::Box(node) => Some(node),
            _ => None,
        }
    }

    #[must_use]
    pub fn vertical_dimensions(&self) -> Option<(Scaled, Scaled)> {
        match self.packed() {
            PackedNode::Box(node) => Some((node.height, node.depth)),
            PackedNode::Unset(node) => Some((node.height, node.depth)),
            PackedNode::Rule { height, depth, .. } => Some((
                height.unwrap_or(Scaled::from_raw(0)),
                depth.unwrap_or(Scaled::from_raw(0)),
            )),
            _ => None,
        }
    }

    /// Child lists in canonical semantic traversal order.
    #[must_use]
    pub fn children(&self) -> impl DoubleEndedIterator<Item = NodeListId> {
        self.child_array(false).into_iter().flatten()
    }

    /// Child lists in physical diagnostic traversal order.
    #[must_use]
    pub fn physical_children(&self) -> impl DoubleEndedIterator<Item = NodeListId> {
        self.child_array(true).into_iter().flatten()
    }

    fn child_array(&self, physical: bool) -> [Option<NodeListId>; 6] {
        let mut children = [None; 6];
        match self {
            Self::HList(node) | Self::VList(node) => {
                children[0] = Some(node.children);
                if physical {
                    children[1] = node.diagnostic_children;
                }
            }
            Self::Glue {
                leader: Some(LeaderPayload::HList(node) | LeaderPayload::VList(node)),
                ..
            } => {
                children[0] = Some(node.children);
                if physical {
                    children[1] = node.diagnostic_children;
                }
            }
            Self::Unset(node) => children[0] = Some(node.children),
            Self::Disc {
                pre, post, replace, ..
            } => {
                children[0] = Some(*pre);
                children[1] = Some(*post);
                children[2] = Some(*replace);
            }
            Self::Ins { content, .. } => children[0] = Some(*content),
            Self::MathNoad(node) => {
                children[0] = math_field_child(&node.nucleus);
                children[1] = math_field_child(&node.subscript);
                children[2] = math_field_child(&node.superscript);
            }
            Self::FractionNoad(node) => {
                children[0] = Some(node.numerator);
                children[1] = Some(node.denominator);
            }
            Self::MathChoice(node) => {
                children[0] = Some(node.display);
                children[1] = Some(node.text);
                children[2] = Some(node.script);
                children[3] = Some(node.script_script);
            }
            Self::MathList(node) => children[0] = Some(node.content),
            Self::Adjust(node) => children[0] = Some(node.content),
            _ => {}
        }
        children
    }
}

fn math_field_child(field: &crate::math::MathField<NodeListId>) -> Option<NodeListId> {
    match field {
        crate::math::MathField::SubBox(child) | crate::math::MathField::SubMlist(child) => {
            Some(*child)
        }
        crate::math::MathField::Empty
        | crate::math::MathField::MathChar(_)
        | crate::math::MathField::MathTextChar(_) => None,
    }
}

/// An immutable compact node-list span.
#[derive(Clone, Copy)]
pub struct NodeList<'a> {
    pub(super) storage: &'a NodeStorage,
    pub(super) start: usize,
    pub(super) end: usize,
}

impl core::fmt::Debug for NodeList<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}
impl<const N: usize> PartialEq<&[Node; N]> for NodeList<'_> {
    fn eq(&self, rhs: &&[Node; N]) -> bool {
        self.to_vec().as_slice() == *rhs
    }
}
impl PartialEq<&[Node]> for NodeList<'_> {
    fn eq(&self, rhs: &&[Node]) -> bool {
        self.to_vec().as_slice() == *rhs
    }
}
impl PartialEq<Vec<Node>> for NodeList<'_> {
    fn eq(&self, rhs: &Vec<Node>) -> bool {
        self.to_vec() == *rhs
    }
}

impl<'a> NodeList<'a> {
    #[must_use]
    pub const fn len(self) -> usize {
        self.end - self.start
    }
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }
    #[must_use]
    pub fn get(self, index: usize) -> Option<NodeRef<'a>> {
        (self.start + index < self.end).then(|| self.storage.decode(self.start + index))
    }
    #[must_use]
    pub fn first(self) -> Option<NodeRef<'a>> {
        self.get(0)
    }
    #[must_use]
    pub fn last(self) -> Option<NodeRef<'a>> {
        (!self.is_empty()).then(|| self.storage.decode(self.end - 1))
    }
    pub fn iter(self) -> NodeIter<'a> {
        NodeIter {
            storage: self.storage,
            next: self.start,
            end: self.end,
        }
    }
    /// Reports whether this list contains a TeX--XeT direction marker without
    /// decoding the node sidecars. Shipout uses this cheap tag scan to avoid a
    /// second decoded traversal for the overwhelmingly common direction-free
    /// list.
    #[must_use]
    pub fn contains_direction(self) -> bool {
        self.storage.words[self.start..self.end]
            .iter()
            .any(|word| word.tag() == 23)
    }
    /// Reports whether shipout must decode this list during its mutable
    /// normalization phase. Inline leaves are already canonical; only nested
    /// lists, executable whatsits, math nodes, direction markers, and node
    /// kinds rejected by shipout require inspection.
    #[must_use]
    pub fn requires_shipout_normalization(self) -> bool {
        self.storage.words[self.start..self.end]
            .iter()
            .any(|word| !shipout_normalization_inert_tag(word.tag()))
    }
    /// Reports whether one node requires inspection during shipout
    /// normalization, using only its compact tag.
    #[must_use]
    pub fn node_requires_shipout_normalization(self, index: usize) -> Option<bool> {
        if index >= self.len() {
            return None;
        }
        Some(!shipout_normalization_inert_tag(
            self.storage.words[self.start + index].tag(),
        ))
    }
    /// Returns the maximal same-font run of inline byte-character words at
    /// `index`. Ligatures and every non-character word deliberately terminate
    /// a run so callers retain their ordinary semantic handling.
    #[must_use]
    pub fn char_run(self, index: usize) -> Option<CharRun<'a>> {
        if index >= self.len() {
            return None;
        }
        let first = *self.storage.words.get(self.start + index)?;
        if first.tag() != 0 {
            return None;
        }
        let font = crate::ids::FontId::new((first.payload() >> 21) as u32);
        let mut end = self.start + index + 1;
        while end < self.end {
            let word = self.storage.words[end];
            if word.tag() != 0 || (word.payload() >> 21) as u32 != font.raw() {
                break;
            }
            // TFM widths are defined only for the byte character domain.
            if word.payload() & 0x1f_ffff > u8::MAX as u64 {
                break;
            }
            end += 1;
        }
        if first.payload() & 0x1f_ffff > u8::MAX as u64 {
            return None;
        }
        Some(CharRun {
            words: &self.storage.words[self.start + index..end],
            origins: &self.storage.origins[self.start + index..end],
            origin_roots: &self.storage.origin_roots[self.start + index..end],
            font,
        })
    }

    /// Creates a lazy, single-pass iterator over the same-font byte-character
    /// run beginning at `index`.
    #[must_use]
    pub fn char_codes(self, index: usize) -> Option<CharCodes<'a>> {
        if index >= self.len() {
            return None;
        }
        let first = self.storage.words[self.start + index];
        let payload = first.payload();
        if first.tag() != 0 || payload & 0x1f_ffff > u8::MAX as u64 {
            return None;
        }
        Some(CharCodes {
            words: &self.storage.words[self.start + index..self.end],
            next: 0,
            font: crate::ids::FontId::new((payload >> 21) as u32),
        })
    }
    #[must_use]
    pub fn to_vec(self) -> Vec<Node> {
        self.iter()
            .map(|node| {
                node.to_owned_with(|_| {
                    panic!("materializing a nested compact list requires its structural owner")
                })
            })
            .collect()
    }
    /// Test/debug-only decoded view for legacy structural assertions.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    #[doc(hidden)]
    pub fn testing_decoded(self) -> &'static [Node] {
        Box::leak(self.to_vec().into_boxed_slice())
    }
}

pub(super) const fn shipout_normalization_inert_tag(tag: u8) -> bool {
    matches!(tag, 0..=6 | 12 | 15)
}

/// Lazy byte codes from one contiguous same-font inline character run.
pub struct CharCodes<'a> {
    words: &'a [NodeWord],
    next: usize,
    font: crate::ids::FontId,
}

impl CharCodes<'_> {
    #[must_use]
    pub const fn font(&self) -> crate::ids::FontId {
        self.font
    }
}

impl Iterator for CharCodes<'_> {
    type Item = u8;
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let word = *self.words.get(self.next)?;
        let payload = word.payload();
        if word.tag() != 0
            || (payload >> 21) as u32 != self.font.raw()
            || payload & 0x1f_ffff > u8::MAX as u64
        {
            return None;
        }
        self.next += 1;
        Some(payload as u8)
    }
}

/// Opaque zero-allocation view of a contiguous same-font byte-character run.
#[derive(Clone, Copy, Debug)]
pub struct CharRun<'a> {
    words: &'a [NodeWord],
    origins: &'a [OriginId],
    origin_roots: &'a [Option<OriginRef>],
    font: crate::ids::FontId,
}

impl<'a> CharRun<'a> {
    #[must_use]
    pub const fn font(self) -> crate::ids::FontId {
        self.font
    }
    #[must_use]
    pub const fn len(self) -> usize {
        self.words.len()
    }
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.words.is_empty()
    }
    pub fn codes(self) -> impl ExactSizeIterator<Item = u8> + 'a {
        self.words.iter().map(|word| word.payload() as u8)
    }
    pub fn origins(self) -> impl ExactSizeIterator<Item = OriginId> + 'a {
        self.origins.iter().copied()
    }
    pub fn origin_roots(self) -> impl ExactSizeIterator<Item = &'a OriginRef> + 'a {
        self.origin_roots
            .iter()
            .map(|root| root.as_ref().expect("character origin root is missing"))
    }
}

impl<'a> IntoIterator for NodeList<'a> {
    type Item = NodeRef<'a>;
    type IntoIter = NodeIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

pub struct NodeIter<'a> {
    storage: &'a NodeStorage,
    next: usize,
    end: usize,
}
impl<'a> Iterator for NodeIter<'a> {
    type Item = NodeRef<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.end {
            None
        } else {
            let node = self.storage.decode(self.next);
            self.next += 1;
            Some(node)
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.end - self.next;
        (n, Some(n))
    }
}
impl<'a> DoubleEndedIterator for NodeIter<'a> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.next == self.end {
            None
        } else {
            self.end -= 1;
            Some(self.storage.decode(self.end))
        }
    }
}
impl ExactSizeIterator for NodeIter<'_> {}

/// A source-independent cursor over owned or detached compact nodes.
///
/// Structural ownership remains in `NodeListRef`; the cursor only normalizes
/// immutable logical access.
#[derive(Clone, Copy)]
pub struct NodeCursor<'a> {
    source: NodeCursorSource<'a>,
}

#[derive(Clone, Copy)]
enum NodeCursorSource<'a> {
    Owned(&'a [Node]),
    Compact(NodeList<'a>),
}

impl<'a> NodeCursor<'a> {
    #[must_use]
    pub fn owned(nodes: &'a [Node]) -> Self {
        Self {
            source: NodeCursorSource::Owned(nodes),
        }
    }

    #[must_use]
    pub fn compact(nodes: NodeList<'a>) -> Self {
        Self {
            source: NodeCursorSource::Compact(nodes),
        }
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.source_len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn get(&self, index: usize) -> Option<NodeRef<'a>> {
        (index < self.source_len()).then(|| match self.source {
            NodeCursorSource::Owned(nodes) => NodeRef::from(&nodes[index]),
            NodeCursorSource::Compact(nodes) => nodes
                .get(index)
                .expect("cursor index belongs to compact list"),
        })
    }

    /// Returns the decoded source node when this cursor borrows an owned list.
    #[must_use]
    pub fn owned_node(&self, index: usize) -> Option<&'a Node> {
        match self.source {
            NodeCursorSource::Owned(nodes) => nodes.get(index),
            NodeCursorSource::Compact(_) => None,
        }
    }

    const fn source_len(&self) -> usize {
        match self.source {
            NodeCursorSource::Owned(nodes) => nodes.len(),
            NodeCursorSource::Compact(nodes) => nodes.len(),
        }
    }

    /// Fast same-font byte-character scan when the source has compact words.
    #[must_use]
    pub fn char_codes(&self, index: usize) -> Option<CharCodes<'a>> {
        match self.source {
            NodeCursorSource::Owned(_) => None,
            NodeCursorSource::Compact(nodes) => nodes.char_codes(index),
        }
    }
}

impl NodeStorage {
    fn decode(&self, index: usize) -> NodeRef<'_> {
        let word = self.words[index];
        let payload = word.payload();
        let side = payload as usize;
        match word.tag() {
            0 => NodeRef::Char {
                font: crate::ids::FontId::new((payload >> 21) as u32),
                ch: char::from_u32((payload & 0x1f_ffff) as u32).expect("invalid stored scalar"),
                origin: self.origins[index],
                origin_root: self.origin_roots[index]
                    .as_ref()
                    .expect("character origin root is missing"),
            },
            1 => NodeRef::Lig {
                font: self.ligatures[side].font,
                ch: self.ligatures[side].ch,
                orig: &self.ligatures[side].orig,
                origins: &self.ligatures[side].origins,
                origin_roots: &self.ligatures[side].origin_roots,
                left_hit: self.ligatures[side].left_hit,
                right_hit: self.ligatures[side].right_hit,
            },
            2 => NodeRef::Kern {
                amount: Scaled::from_raw(payload as u32 as i32),
                kind: decode_kern(((payload >> 32) & 7) as u8),
            },
            24 => NodeRef::MarginKern {
                amount: Scaled::from_raw(payload as u32 as i32),
                side: if ((payload >> 32) & 1) == 0 {
                    MarginKernSide::Left
                } else {
                    MarginKernSide::Right
                },
                font: crate::ids::FontId::new(((payload >> 33) & 0x7fff) as u32),
                ch: (payload >> 48) as u8,
            },
            3 => NodeRef::Glue {
                spec: self.glue_roots[index]
                    .as_ref()
                    .expect("ordinary glue word has strong root"),
                kind: decode_glue(((payload >> 32) & 0x3f) as u8),
                leader: None,
            },
            4 => NodeRef::Penalty(payload as u32 as i32),
            5 => NodeRef::MathOn(Scaled::from_raw(payload as u32 as i32)),
            6 => NodeRef::MathOff(Scaled::from_raw(payload as u32 as i32)),
            23 => NodeRef::Direction(match payload {
                0 => Direction::BeginL,
                1 => Direction::EndL,
                2 => Direction::BeginR,
                3 => Direction::EndR,
                4 => Direction::BeginM,
                5 => Direction::EndM,
                _ => unreachable!("stored direction code is valid"),
            }),
            7 => NodeRef::MathStyle(decode_style(payload as u8)),
            8 => NodeRef::Nonscript,
            9 | 10 => {
                let b = self.boxes.rows[side];
                if word.tag() == 9 {
                    NodeRef::HList(b)
                } else {
                    NodeRef::VList(b)
                }
            }
            11 => NodeRef::Unset(UnsetNode::new(UnsetNodeFields {
                kind: self.unsets.kind[side],
                width: self.unsets.width[side],
                height: self.unsets.height[side],
                depth: self.unsets.depth[side],
                span_count: self.unsets.span_count[side],
                stretch: self.unsets.stretch[side],
                stretch_order: self.unsets.stretch_order[side],
                shrink: self.unsets.shrink[side],
                shrink_order: self.unsets.shrink_order[side],
                children: self.unsets.children[side],
            })),
            12 => {
                let (width, height, depth) = self.rules[side];
                NodeRef::Rule {
                    width,
                    height,
                    depth,
                }
            }
            13 => {
                let (spec, kind, leader) = &self.leaders[side];
                NodeRef::Glue {
                    spec,
                    kind: *kind,
                    leader: Some(*leader),
                }
            }
            14 => {
                let (kind, pre, post, replace, physical_replace_count) = self.discs[side];
                NodeRef::Disc {
                    kind,
                    pre,
                    post,
                    replace,
                    physical_replace_count,
                }
            }
            15 => {
                let (class, tokens) = &self.marks[side];
                NodeRef::Mark {
                    class: *class,
                    tokens,
                }
            }
            16 => NodeRef::Ins {
                class: self.insertions.class[side],
                size: self.insertions.size[side],
                split_top_skip: &self.insertions.split_top_skip[side],
                split_max_depth: self.insertions.split_max_depth[side],
                floating_penalty: self.insertions.floating_penalty[side],
                content: self.insertions.content[side],
            },
            17 => NodeRef::Whatsit(&self.whatsits[side]),
            18 => NodeRef::MathNoad(crate::math::MathNoad {
                kind: self.noads.kind[side].clone(),
                nucleus: self.noads.nucleus[side],
                subscript: self.noads.subscript[side],
                superscript: self.noads.superscript[side],
            }),
            19 => NodeRef::FractionNoad(self.fractions[side]),
            20 => NodeRef::MathChoice(self.choices[side]),
            21 => NodeRef::MathList(self.math_lists[side]),
            22 => NodeRef::Adjust(self.adjusts[side]),
            _ => panic!("reserved node-word tag"),
        }
    }
}
