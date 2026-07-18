use super::storage::{NodeStorage, NodeWord, decode_glue, decode_kern, decode_style};
use crate::ids::{ArenaRef, GlueId, NodeListId};
use crate::math::MathStyle;
use crate::node::{
    BoxNode, Direction, DiscKind, GlueKind, KernKind, Node, UnsetNode, UnsetNodeFields,
};
use crate::scaled::Scaled;
use crate::token::OriginId;

/// Per-mount diagnostic provenance for one immutable node-storage payload.
/// Semantic words and sidecars remain shared; only the nonsemantic origin
/// columns vary between restarted Universes.
#[derive(Clone, Debug)]
pub(crate) struct NodeOriginOverlay {
    word_origins: Vec<OriginId>,
    ligature_origins: Vec<Option<Vec<OriginId>>>,
}

/// A zero-allocation logical view of one compact arena node.
#[derive(Clone, Debug)]
pub enum NodeRef<'a> {
    Char {
        font: crate::ids::FontId,
        ch: char,
        origin: OriginId,
    },
    Lig {
        font: crate::ids::FontId,
        ch: char,
        orig: &'a [char],
        origins: &'a [OriginId],
    },
    Kern {
        amount: Scaled,
        kind: KernKind,
    },
    Glue {
        spec: GlueId,
        kind: GlueKind,
        leader: Option<&'a crate::node::LeaderPayload>,
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
        tokens: crate::ids::TokenListId,
    },
    Ins {
        class: u16,
        size: Scaled,
        split_top_skip: GlueId,
        split_max_depth: Scaled,
        floating_penalty: i32,
        content: NodeListId,
    },
    Whatsit(&'a crate::node::Whatsit),
    MathOn(Scaled),
    MathOff(Scaled),
    Direction(Direction),
    MathNoad(crate::math::MathNoad),
    FractionNoad(&'a crate::math::MathFraction),
    MathStyle(MathStyle),
    MathChoice(&'a crate::math::MathChoice),
    MathList(crate::math::MathListNode),
    Nonscript,
    Adjust(NodeListId),
}

impl PartialEq for NodeRef<'_> {
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
                    ..
                },
                Self::Lig {
                    font: right_font,
                    ch: right_ch,
                    orig: right_orig,
                    ..
                },
            ) => left_font == right_font && left_ch == right_ch && left_orig == right_orig,
            _ => self.to_owned() == other.to_owned(),
        }
    }
}

impl NodeRef<'_> {
    /// Materializes an owned node for builder/list-surgery output, never for storage.
    #[must_use]
    pub fn to_owned(&self) -> Node {
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
            } => Node::Lig {
                font: *font,
                ch: *ch,
                orig: orig.to_vec(),
                origins: origins.to_vec(),
            },
            Self::Kern { amount, kind } => Node::Kern {
                amount: *amount,
                kind: *kind,
            },
            Self::Glue { spec, kind, leader } => Node::Glue {
                spec: *spec,
                kind: *kind,
                leader: leader.cloned(),
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
            Self::HList(v) => Node::HList(*v),
            Self::VList(v) => Node::VList(*v),
            Self::Unset(v) => Node::Unset(*v),
            Self::Disc {
                kind,
                pre,
                post,
                replace,
            } => Node::Disc {
                kind: *kind,
                pre: *pre,
                post: *post,
                replace: *replace,
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
            Self::Whatsit(v) => Node::Whatsit((*v).clone()),
            Self::MathOn(v) => Node::MathOn(*v),
            Self::MathOff(v) => Node::MathOff(*v),
            Self::Direction(v) => Node::Direction(*v),
            Self::MathNoad(v) => Node::MathNoad(v.clone()),
            Self::FractionNoad(v) => Node::FractionNoad((*v).clone()),
            Self::MathStyle(v) => Node::MathStyle(*v),
            Self::MathChoice(v) => Node::MathChoice((*v).clone()),
            Self::MathList(v) => Node::MathList(*v),
            Self::Nonscript => Node::Nonscript,
            Self::Adjust(v) => Node::Adjust(*v),
        }
    }
}

/// An immutable compact node-list span.
#[derive(Clone, Copy)]
pub struct NodeList<'a> {
    pub(super) storage: &'a NodeStorage,
    pub(super) origins: Option<&'a NodeOriginOverlay>,
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
        (self.start + index < self.end)
            .then(|| self.storage.decode(self.start + index, self.origins))
    }
    #[must_use]
    pub fn first(self) -> Option<NodeRef<'a>> {
        self.get(0)
    }
    #[must_use]
    pub fn last(self) -> Option<NodeRef<'a>> {
        (!self.is_empty()).then(|| self.storage.decode(self.end - 1, self.origins))
    }
    pub fn iter(self) -> NodeIter<'a> {
        NodeIter {
            storage: self.storage,
            origins: self.origins,
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
            origins: self.origins.map_or_else(
                || &self.storage.origins[self.start + index..end],
                |origins| &origins.word_origins[self.start + index..end],
            ),
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
        self.iter().map(|node| node.to_owned()).collect()
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
    origins: Option<&'a NodeOriginOverlay>,
    next: usize,
    end: usize,
}
impl<'a> Iterator for NodeIter<'a> {
    type Item = NodeRef<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.end {
            None
        } else {
            let node = self.storage.decode(self.next, self.origins);
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
            Some(self.storage.decode(self.end, self.origins))
        }
    }
}
impl ExactSizeIterator for NodeIter<'_> {}

impl NodeStorage {
    fn decode<'a>(&'a self, index: usize, origins: Option<&'a NodeOriginOverlay>) -> NodeRef<'a> {
        let word = self.words[index];
        let payload = word.payload();
        let side = payload as usize;
        match word.tag() {
            0 => NodeRef::Char {
                font: crate::ids::FontId::new((payload >> 21) as u32),
                ch: char::from_u32((payload & 0x1f_ffff) as u32).expect("invalid stored scalar"),
                origin: origins.map_or(self.origins[index], |origins| origins.word_origins[index]),
            },
            1 => NodeRef::Lig {
                font: self.ligatures[side].0,
                ch: self.ligatures[side].1,
                orig: &self.ligatures[side].2,
                origins: origins
                    .and_then(|origins| origins.ligature_origins[side].as_deref())
                    .unwrap_or(&self.ligatures[side].3),
            },
            2 => NodeRef::Kern {
                amount: Scaled::from_raw(payload as u32 as i32),
                kind: decode_kern(((payload >> 32) & 7) as u8),
            },
            3 => NodeRef::Glue {
                spec: GlueId::new(payload as u32),
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
                    spec: *spec,
                    kind: *kind,
                    leader: Some(leader),
                }
            }
            14 => {
                let (kind, pre, post, replace) = self.discs[side];
                NodeRef::Disc {
                    kind,
                    pre,
                    post,
                    replace,
                }
            }
            15 => {
                let (class, tokens) = self.marks[side];
                NodeRef::Mark { class, tokens }
            }
            16 => NodeRef::Ins {
                class: self.insertions.class[side],
                size: self.insertions.size[side],
                split_top_skip: self.insertions.split_top_skip[side],
                split_max_depth: self.insertions.split_max_depth[side],
                floating_penalty: self.insertions.floating_penalty[side],
                content: self.insertions.content[side],
            },
            17 => NodeRef::Whatsit(&self.whatsits[side]),
            18 => NodeRef::MathNoad(crate::math::MathNoad {
                kind: self.noads.kind[side].clone(),
                nucleus: self.noads.nucleus[side].clone(),
                subscript: self.noads.subscript[side].clone(),
                superscript: self.noads.superscript[side].clone(),
            }),
            19 => NodeRef::FractionNoad(&self.fractions[side]),
            20 => NodeRef::MathChoice(&self.choices[side]),
            21 => NodeRef::MathList(self.math_lists[side]),
            22 => NodeRef::Adjust(self.adjusts[side]),
            _ => panic!("reserved node-word tag"),
        }
    }

    /// Builds a diagnostic-only provenance overlay for a survivor graph.
    /// Traversal follows the retained paragraph recipe's depth-first order;
    /// semantic words and sidecars remain untouched.
    pub(crate) fn paragraph_origin_overlay(
        &self,
        root: NodeListId,
        root_origins: &[OriginId],
        origin_slots: &[u32],
    ) -> Option<NodeOriginOverlay> {
        let ArenaRef::Survivor(root_id) = root.arena() else {
            return None;
        };
        let end = root.start().checked_add(root.len())? as usize;
        if end > self.words.len() {
            return None;
        }
        let mut overlay = NodeOriginOverlay {
            word_origins: self.origins.clone(),
            ligature_origins: vec![None; self.ligatures.len()],
        };
        let mut origin_slots = origin_slots.iter().copied();
        let origin_at = |ordinal: u32| {
            usize::try_from(ordinal)
                .ok()
                .and_then(|ordinal| root_origins.get(ordinal))
                .copied()
                .unwrap_or(OriginId::UNKNOWN)
        };
        let mut frames = vec![(root.start() as usize, end)];
        while let Some((next, frame_end)) = frames.last_mut() {
            if *next == *frame_end {
                frames.pop();
                continue;
            }
            let index = *next;
            *next += 1;
            let node = self.decode(index, None);
            let mut children = Vec::new();
            match node {
                NodeRef::Char { .. } => {
                    overlay.word_origins[index] =
                        origin_at(origin_slots.next().unwrap_or(u32::MAX));
                }
                NodeRef::Lig { orig, .. } => {
                    let side = self.words[index].payload() as usize;
                    overlay.ligature_origins[side] = Some(
                        (0..orig.len())
                            .map(|_| origin_at(origin_slots.next().unwrap_or(u32::MAX)))
                            .collect(),
                    );
                }
                NodeRef::HList(node) | NodeRef::VList(node) => children.push(node.children),
                NodeRef::Glue {
                    leader: Some(crate::node::LeaderPayload::HList(node)),
                    ..
                }
                | NodeRef::Glue {
                    leader: Some(crate::node::LeaderPayload::VList(node)),
                    ..
                } => children.push(node.children),
                NodeRef::Unset(node) => children.push(node.children),
                NodeRef::Disc {
                    pre, post, replace, ..
                } => children.extend([pre, post, replace]),
                NodeRef::Ins { content, .. } | NodeRef::Adjust(content) => {
                    children.push(content);
                }
                _ => {}
            }
            for child in children.into_iter().rev() {
                if child.arena() != ArenaRef::Survivor(root_id) {
                    return None;
                }
                let child_end = child.start().checked_add(child.len())? as usize;
                if child_end > self.words.len() {
                    return None;
                }
                frames.push((child.start() as usize, child_end));
            }
        }
        origin_slots.next().is_none().then_some(overlay)
    }
}
