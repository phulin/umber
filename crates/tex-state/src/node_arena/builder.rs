use super::NodeListRef;
use crate::ids::FontId;
use crate::node::{KernKind, Node};
use crate::provenance::OriginRef;
use crate::scaled::Scaled;

/// Compact rows used by the builder while a character/kern-only episode is
/// live. These are mutable construction data, not a second immutable node
/// store: the rows move directly into the canonical `NodeStorage` at freeze.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(transparent)]
pub(crate) struct CompactBuilderNode(u64);

const COMPACT_KERN_TAG: u64 = 1 << 63;
const COMPACT_KIND_SHIFT: u32 = 32;

#[derive(Clone, Debug, PartialEq)]
enum BuilderStorage {
    Owned(Vec<Node>),
    Compact(Vec<CompactBuilderNode>),
}

/// The sole mutable builder for native node material.
///
/// Child ownership, semantic identity, and compact storage sidecars are
/// deliberately absent while a semantic episode is live. The aggregate
/// freeze boundary derives them once from the mutable rows before publishing an
/// immutable [`NodeListRef`].
#[derive(Clone, Debug, PartialEq)]
pub struct NodeListBuilder {
    storage: BuilderStorage,
}

impl Default for NodeListBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeListBuilder {
    pub(crate) fn new() -> Self {
        Self {
            storage: BuilderStorage::Owned(Vec::new()),
        }
    }

    pub(crate) fn new_compact() -> Self {
        Self {
            storage: BuilderStorage::Compact(Vec::new()),
        }
    }

    pub fn push(&mut self, node: Node) {
        self.as_mut_vec().push(node)
    }

    /// Appends a character whose provenance is known to be the canonical
    /// unknown origin, without constructing the much larger owned `Node` row.
    #[inline]
    pub fn push_unknown_character(&mut self, font: FontId, ch: char) {
        match &mut self.storage {
            BuilderStorage::Compact(rows) => {
                rows.push(CompactBuilderNode::character(font, ch));
            }
            BuilderStorage::Owned(nodes) => nodes.push(Node::Char {
                font,
                ch,
                origin: OriginRef::unknown(),
            }),
        }
    }

    #[inline]
    pub fn push_kern(&mut self, amount: Scaled, kind: KernKind) {
        match &mut self.storage {
            BuilderStorage::Compact(rows) => {
                rows.push(CompactBuilderNode::kern(amount, kind));
            }
            BuilderStorage::Owned(nodes) => nodes.push(Node::Kern { amount, kind }),
        }
    }

    /// Computes the width of a character/kern-only builder without
    /// materializing owned nodes. `None` reports scaled-dimension overflow.
    #[must_use]
    pub fn compact_width(&self, character_width: Scaled) -> Option<Scaled> {
        let BuilderStorage::Compact(rows) = &self.storage else {
            panic!("compact width requested from a general node builder")
        };
        rows.iter()
            .try_fold(0_i32, |width, row| {
                let contribution = row
                    .as_kern()
                    .map_or(character_width.raw(), |(amount, _)| amount.raw());
                width.checked_add(contribution)
            })
            .map(Scaled::from_raw)
    }

    pub fn reserve(&mut self, additional: usize) {
        match &mut self.storage {
            BuilderStorage::Owned(nodes) => nodes.reserve(additional),
            BuilderStorage::Compact(rows) => rows.reserve(additional),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        match &self.storage {
            BuilderStorage::Owned(nodes) => nodes.len(),
            BuilderStorage::Compact(rows) => rows.len(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn as_slice(&self) -> &[Node] {
        match &self.storage {
            BuilderStorage::Owned(nodes) => nodes,
            BuilderStorage::Compact(rows) if rows.is_empty() => &[],
            BuilderStorage::Compact(_) => {
                panic!("compact node-builder rows require typed access before freeze")
            }
        }
    }

    pub(crate) fn as_mut_vec(&mut self) -> &mut Vec<Node> {
        if let BuilderStorage::Compact(rows) = &mut self.storage {
            let nodes = rows.drain(..).map(CompactBuilderNode::into_owned).collect();
            self.storage = BuilderStorage::Owned(nodes);
        }
        let BuilderStorage::Owned(nodes) = &mut self.storage else {
            unreachable!()
        };
        nodes
    }

    pub(crate) fn truncate(&mut self, len: usize) {
        match &mut self.storage {
            BuilderStorage::Owned(nodes) => nodes.truncate(len),
            BuilderStorage::Compact(rows) => rows.truncate(len),
        }
    }

    pub(crate) fn into_nodes(self) -> Vec<Node> {
        match self.storage {
            BuilderStorage::Owned(nodes) => nodes,
            BuilderStorage::Compact(rows) => rows
                .into_iter()
                .map(CompactBuilderNode::into_owned)
                .collect(),
        }
    }

    pub fn clear(&mut self) {
        match &mut self.storage {
            BuilderStorage::Owned(nodes) => nodes.clear(),
            BuilderStorage::Compact(rows) => rows.clear(),
        }
    }

    pub(crate) fn direct_children(&self) -> Vec<NodeListRef> {
        let mut direct_children = Vec::new();
        let BuilderStorage::Owned(nodes) = &self.storage else {
            return direct_children;
        };
        for node in nodes {
            node.visit_node_lists(|child| {
                if !direct_children.iter().any(|existing: &NodeListRef| {
                    existing.id() == child.id() && existing.shares_payload(child)
                }) {
                    direct_children.push(child.clone());
                }
            });
        }
        direct_children
    }

    pub(crate) fn compact_rows(&self) -> Option<&[CompactBuilderNode]> {
        match &self.storage {
            BuilderStorage::Owned(_) => None,
            BuilderStorage::Compact(rows) => Some(rows),
        }
    }

    pub(crate) fn into_compact_rows(self) -> Result<Vec<CompactBuilderNode>, Vec<Node>> {
        match self.storage {
            BuilderStorage::Owned(nodes) => Err(nodes),
            BuilderStorage::Compact(rows) => Ok(rows),
        }
    }
}

impl CompactBuilderNode {
    fn character(font: FontId, ch: char) -> Self {
        debug_assert!(font.raw() < (1 << 31));
        Self((u64::from(font.raw()) << 21) | u64::from(ch as u32))
    }

    fn kern(amount: Scaled, kind: KernKind) -> Self {
        Self(
            COMPACT_KERN_TAG
                | (u64::from(kern_code(kind)) << COMPACT_KIND_SHIFT)
                | u64::from(amount.raw() as u32),
        )
    }

    pub(crate) fn as_character(self) -> Option<(FontId, char)> {
        if self.0 & COMPACT_KERN_TAG != 0 {
            return None;
        }
        Some((
            FontId::new((self.0 >> 21) as u32),
            char::from_u32((self.0 & 0x1f_ffff) as u32)
                .expect("compact builder character is valid"),
        ))
    }

    pub(crate) fn as_kern(self) -> Option<(Scaled, KernKind)> {
        if self.0 & COMPACT_KERN_TAG == 0 {
            return None;
        }
        Some((
            Scaled::from_raw(self.0 as u32 as i32),
            decode_kern_code(((self.0 >> COMPACT_KIND_SHIFT) & 7) as u8),
        ))
    }

    fn into_owned(self) -> Node {
        if let Some((font, ch)) = self.as_character() {
            Node::Char {
                font,
                ch,
                origin: OriginRef::unknown(),
            }
        } else {
            let (amount, kind) = self.as_kern().expect("compact builder row has a node tag");
            Node::Kern { amount, kind }
        }
    }
}

fn kern_code(kind: KernKind) -> u8 {
    match kind {
        KernKind::Explicit => 0,
        KernKind::Font => 1,
        KernKind::Accent => 2,
        KernKind::Mu => 3,
        KernKind::LeftMargin => 4,
        KernKind::RightMargin => 5,
        KernKind::Auto => 6,
    }
}

fn decode_kern_code(code: u8) -> KernKind {
    match code {
        0 => KernKind::Explicit,
        1 => KernKind::Font,
        2 => KernKind::Accent,
        3 => KernKind::Mu,
        4 => KernKind::LeftMargin,
        5 => KernKind::RightMargin,
        6 => KernKind::Auto,
        _ => unreachable!("compact builder kern code is valid"),
    }
}
