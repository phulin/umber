//! Direct borrowed traversal across owned and resident compact nodes.

use super::view::NodeView;
use crate::glue::GlueSpec;
use crate::node::Node;
use crate::page_node_arena::PageListId;

/// Width-bearing semantic facts read without expanding a compact node record.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HorizontalNode {
    Glyph {
        font: crate::ids::FontId,
        ch: char,
    },
    Kern {
        amount: crate::scaled::Scaled,
        kind: Option<crate::node::KernKind>,
    },
    Glue(GlueSpec),
    Rule(Option<crate::scaled::Scaled>),
    Box(crate::scaled::Scaled),
    Unset(crate::scaled::Scaled),
    Disc(PageListId),
    Image(crate::scaled::Scaled),
    Math(crate::scaled::Scaled),
    Ignored,
}

const _: () = assert!(core::mem::size_of::<HorizontalNode>() <= 48);

/// Small borrowed reference to either a native test node or one compact
/// resident page-material record.
///
/// Sequential algorithms should retain this reference and decode only their
/// demanded scalar or annex fields.
#[derive(Clone, Copy)]
pub struct DirectNodeView<'a> {
    source: DirectNodeSource<'a>,
}

#[derive(Clone, Copy)]
enum DirectNodeSource<'a> {
    Owned(&'a Node),
    Page(crate::page_node_arena::PageMaterialNodeRef<'a>),
}

impl<'a> DirectNodeView<'a> {
    fn owned(node: &'a Node) -> Self {
        Self {
            source: DirectNodeSource::Owned(node),
        }
    }

    fn page(node: crate::page_node_arena::PageMaterialNodeRef<'a>) -> Self {
        Self {
            source: DirectNodeSource::Page(node),
        }
    }

    #[must_use]
    pub fn kind(self) -> Option<crate::node::NodeKind> {
        match self.source {
            DirectNodeSource::Owned(node) => Some(node.kind()),
            DirectNodeSource::Page(node) => node.kind(),
        }
    }

    #[must_use]
    pub(crate) fn tex_memory_words(self, etex_node_sizes: bool) -> (usize, usize) {
        match self.source {
            DirectNodeSource::Owned(node) => NodeView::from(node).tex_memory_words(etex_node_sizes),
            DirectNodeSource::Page(node) => node.tex_memory_words(etex_node_sizes),
        }
    }

    #[must_use]
    pub(crate) fn retains_node_list(self) -> bool {
        match self.source {
            DirectNodeSource::Owned(node) => {
                let mut retains = false;
                NodeView::from(node).visit_semantic_node_lists(|list| retains |= !list.is_empty());
                retains
            }
            DirectNodeSource::Page(node) => node.retains_node_list(),
        }
    }

    #[must_use]
    pub fn character(self) -> Option<(crate::ids::FontId, char, crate::token::OriginId)> {
        match self.source {
            DirectNodeSource::Owned(Node::Char { font, ch, origin }) => Some((*font, *ch, *origin)),
            DirectNodeSource::Owned(_) => None,
            DirectNodeSource::Page(node) => node.character(),
        }
    }

    #[must_use]
    pub fn glyph(self) -> Option<(crate::ids::FontId, char)> {
        match self.source {
            DirectNodeSource::Owned(Node::Char { font, ch, .. } | Node::Lig { font, ch, .. }) => {
                Some((*font, *ch))
            }
            DirectNodeSource::Owned(_) => None,
            DirectNodeSource::Page(node) => node.glyph(),
        }
    }

    #[must_use]
    pub fn kern(self) -> Option<(crate::scaled::Scaled, crate::node::KernKind)> {
        match self.source {
            DirectNodeSource::Owned(Node::Kern { amount, kind }) => Some((*amount, *kind)),
            DirectNodeSource::Owned(_) => None,
            DirectNodeSource::Page(node) => node.kern(),
        }
    }

    #[must_use]
    pub fn penalty(self) -> Option<i32> {
        match self.source {
            DirectNodeSource::Owned(Node::Penalty(value)) => Some(*value),
            DirectNodeSource::Owned(_) => None,
            DirectNodeSource::Page(node) => node.penalty(),
        }
    }

    #[must_use]
    pub fn glue_kind(self) -> Option<crate::node::GlueKind> {
        match self.source {
            DirectNodeSource::Owned(Node::Glue { kind, .. }) => Some(*kind),
            DirectNodeSource::Owned(_) => None,
            DirectNodeSource::Page(node) => node.glue_spec_kind().map(|(_, kind)| kind),
        }
    }

    #[must_use]
    pub fn direction(self) -> Option<crate::node::Direction> {
        match self.source {
            DirectNodeSource::Owned(Node::Direction(direction)) => Some(*direction),
            DirectNodeSource::Owned(_) => None,
            DirectNodeSource::Page(node) => node.direction(),
        }
    }

    #[must_use]
    pub fn lineage_cell_count(self) -> usize {
        match self.source {
            DirectNodeSource::Owned(Node::Char { .. }) => 1,
            DirectNodeSource::Owned(Node::Lig { orig, .. }) => orig.len(),
            DirectNodeSource::Owned(_) => 0,
            DirectNodeSource::Page(node) => {
                if node.character().is_some() {
                    1
                } else {
                    let mut count = 0;
                    let _ = node.visit_ligature_source(|_, _| count += 1);
                    count
                }
            }
        }
    }

    #[must_use]
    pub fn discretionary(
        self,
    ) -> Option<(
        crate::node::DiscKind,
        PageListId,
        PageListId,
        PageListId,
        u8,
    )> {
        match self.source {
            DirectNodeSource::Owned(Node::Disc {
                kind,
                pre,
                post,
                replace,
                physical_replace_count,
            }) => Some((*kind, *pre, *post, *replace, *physical_replace_count)),
            DirectNodeSource::Owned(_) => None,
            DirectNodeSource::Page(node) => node.discretionary(),
        }
    }

    #[must_use]
    pub fn discretionary_break(self) -> Option<(crate::node::DiscKind, PageListId, PageListId)> {
        match self.source {
            DirectNodeSource::Owned(Node::Disc {
                kind, pre, post, ..
            }) => Some((*kind, *pre, *post)),
            DirectNodeSource::Owned(_) => None,
            DirectNodeSource::Page(node) => node.discretionary_break(),
        }
    }

    #[must_use]
    pub fn discretionary_replace(self) -> Option<PageListId> {
        match self.source {
            DirectNodeSource::Owned(Node::Disc { replace, .. }) => Some(*replace),
            DirectNodeSource::Owned(_) => None,
            DirectNodeSource::Page(node) => node.discretionary_replace(),
        }
    }

    #[must_use]
    pub fn horizontal(self) -> HorizontalNode {
        match self.source {
            DirectNodeSource::Owned(node) => match node {
                Node::Char { font, ch, .. } | Node::Lig { font, ch, .. } => HorizontalNode::Glyph {
                    font: *font,
                    ch: *ch,
                },
                Node::Kern { amount, kind } => HorizontalNode::Kern {
                    amount: *amount,
                    kind: Some(*kind),
                },
                Node::MarginKern { amount, .. } => HorizontalNode::Kern {
                    amount: *amount,
                    kind: None,
                },
                Node::Glue { spec, .. } => HorizontalNode::Glue(*spec),
                Node::Rule { width, .. } => HorizontalNode::Rule(*width),
                Node::HList(value) | Node::VList(value) => HorizontalNode::Box(value.width),
                Node::Unset(value) => HorizontalNode::Unset(value.width),
                Node::Disc { replace, .. } => HorizontalNode::Disc(*replace),
                Node::Whatsit(
                    crate::node::Whatsit::PdfRefXForm { width, .. }
                    | crate::node::Whatsit::PdfRefXImage { width, .. },
                ) => HorizontalNode::Image(*width),
                Node::MathOn(width) | Node::MathOff(width) => HorizontalNode::Math(*width),
                _ => HorizontalNode::Ignored,
            },
            DirectNodeSource::Page(node) => {
                if let Some((font, ch)) = node.glyph() {
                    HorizontalNode::Glyph { font, ch }
                } else if let Some((amount, kind)) = node.kern() {
                    HorizontalNode::Kern {
                        amount,
                        kind: Some(kind),
                    }
                } else if let Some(amount) = node.margin_kern_amount() {
                    HorizontalNode::Kern { amount, kind: None }
                } else if let Some((spec, _)) = node.glue_spec_kind() {
                    HorizontalNode::Glue(spec)
                } else if let Some(width) = node.rule_width() {
                    HorizontalNode::Rule(width)
                } else if let Some(width) = node.box_width() {
                    HorizontalNode::Box(width)
                } else if let Some(width) = node.unset_width() {
                    HorizontalNode::Unset(width)
                } else if let Some(replace) = node.discretionary_replace() {
                    HorizontalNode::Disc(replace)
                } else if let Some(width) = node.pdf_image_width() {
                    HorizontalNode::Image(width)
                } else if let Some((_, width)) = node.math_boundary() {
                    HorizontalNode::Math(width)
                } else {
                    HorizontalNode::Ignored
                }
            }
        }
    }
}

const _: () = assert!(core::mem::size_of::<DirectNodeView<'static>>() <= 24);

/// Unified borrowed view for operation buffers and direct page sequences.
///
/// The arena variant retains only a borrow and a compact direct root.
/// Genuinely positional compatibility consumers use [`Self::owned_node`].
/// Long compact consumers use [`Self::for_each_direct`] or
/// [`Self::try_for_each_direct_range`], which follow the sole predecessor
/// chain once without successor metadata or whole-node reconstruction.
/// Iterator adapters remain for algorithms that genuinely need pull-based
/// compatibility traversal and retain the admitted cursor within each packed
/// block.
#[derive(Clone, Copy)]
pub struct NodeCursor<'a> {
    source: NodeCursorSource<'a>,
}

/// Test-only observations which distinguish positional node probes from
/// linear predecessor-topology traversal.
#[cfg(any(test, feature = "testing"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NodeTraversalCounters {
    pub index_resolutions: u64,
    pub index_predecessor_steps: u64,
    pub forward_chunk_crossings: u64,
}

impl core::fmt::Debug for NodeCursor<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_list().entries(self.iter()).finish()
    }
}

impl PartialEq for NodeCursor<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

#[derive(Clone, Copy)]
enum NodeCursorSource<'a> {
    Slice(&'a [Node]),
    Fork(
        crate::fork_arena::ArenaListView<
            'a,
            crate::node_record::NodeRecord,
            crate::fork_arena::PageMaterialLane,
        >,
        crate::node_record::NodeAnnexView<'a>,
    ),
}

impl<'a> NodeCursor<'a> {
    #[must_use]
    pub const fn nodes(self) -> Self {
        self
    }
    #[must_use]
    pub const fn owned(nodes: &'a [Node]) -> Self {
        Self {
            source: NodeCursorSource::Slice(nodes),
        }
    }
    #[must_use]
    pub(crate) const fn fork_arena(
        view: crate::fork_arena::ArenaListView<
            'a,
            crate::node_record::NodeRecord,
            crate::fork_arena::PageMaterialLane,
        >,
        annex: crate::node_record::NodeAnnexView<'a>,
    ) -> Self {
        Self {
            source: NodeCursorSource::Fork(view, annex),
        }
    }
    #[must_use]
    pub const fn len(&self) -> usize {
        match self.source {
            NodeCursorSource::Slice(nodes) => nodes.len(),
            NodeCursorSource::Fork(view, _) => view.len(),
        }
    }
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[cfg(any(test, feature = "testing"))]
    #[doc(hidden)]
    pub fn testing_traversal_counters(&self) -> NodeTraversalCounters {
        let (index_resolutions, index_predecessor_steps, forward_chunk_crossings) =
            match self.source {
                NodeCursorSource::Slice(_) => (0, 0, 0),
                NodeCursorSource::Fork(view, _) => view.traversal_counters(),
            };
        NodeTraversalCounters {
            index_resolutions,
            index_predecessor_steps,
            forward_chunk_crossings,
        }
    }
    #[must_use]
    pub fn get(&self, index: usize) -> Option<NodeView<'a>> {
        match self.source {
            NodeCursorSource::Slice(nodes) => nodes.get(index).map(NodeView::from),
            NodeCursorSource::Fork(view, annex) => view
                .get(index)
                .and_then(|record| record.decode_owned(annex))
                .map(NodeView::from_owned),
        }
    }

    /// Borrows one node without expanding compact page material into the
    /// former whole-node value.
    #[must_use]
    pub fn get_direct(&self, index: usize) -> Option<DirectNodeView<'a>> {
        match self.source {
            NodeCursorSource::Slice(nodes) => nodes.get(index).map(DirectNodeView::owned),
            NodeCursorSource::Fork(view, annex) => view.get(index).map(|record| {
                DirectNodeView::page(crate::page_node_arena::PageMaterialNodeRef::new(
                    record, annex,
                ))
            }),
        }
    }

    /// Returns the backing-row address for exact retained-range tests.
    ///
    /// This is deliberately unavailable to production consumers: node reads
    /// must use [`NodeView`], while allocation/copy tests may still prove that
    /// a retained span names the same resident rows.
    #[doc(hidden)]
    #[must_use]
    pub fn testing_node_address(&self, index: usize) -> Option<*const Node> {
        match self.source {
            NodeCursorSource::Slice(nodes) => nodes.get(index).map(core::ptr::from_ref),
            NodeCursorSource::Fork(view, _) => view
                .get(index)
                .map(|record| core::ptr::from_ref(record).cast()),
        }
    }
    #[must_use]
    pub(crate) fn owned_node(&self, index: usize) -> Option<&'a Node> {
        match self.source {
            NodeCursorSource::Slice(nodes) => nodes.get(index),
            NodeCursorSource::Fork(_, _) => None,
        }
    }
    #[must_use]
    pub fn first(&self) -> Option<NodeView<'a>> {
        self.get(0)
    }
    #[must_use]
    pub fn last(&self) -> Option<NodeView<'a>> {
        self.len().checked_sub(1).and_then(|index| self.get(index))
    }
    #[must_use]
    pub fn char_codes(&self, index: usize) -> Option<CharCodes<'a>> {
        CharCodes::new(*self, index)
    }

    #[must_use]
    pub fn char_run(&self, index: usize) -> Option<CharRun<'a>> {
        CharRun::new(*self, index)
    }
    /// Builds the compatibility iterator used by mixed and reverse walks.
    ///
    /// Production forward scans should use [`Self::for_each`] or
    /// [`Self::try_for_each_range`] so arena-backed lists follow their packed
    /// predecessor chain exactly once.
    pub fn iter(&self) -> NodeCursorIter<'a> {
        self.iter_from(0)
    }

    /// Iterates from one logical node position. Arena-backed lists retain
    /// their admitted packed-block cursor rather than resolving every node by
    /// index.
    pub fn iter_from(&self, start: usize) -> NodeCursorIter<'a> {
        match self.source {
            NodeCursorSource::Slice(nodes) => NodeCursorIter::Slice(
                nodes
                    .get(start.min(nodes.len())..)
                    .unwrap_or_default()
                    .iter(),
            ),
            NodeCursorSource::Fork(view, annex) => {
                NodeCursorIter::Fork(view.iter_from(start), annex)
            }
        }
    }

    /// Canonically visits sequential nodes through authoritative storage.
    ///
    /// Arena-backed inputs use the direct chunk traversal rather than
    /// resolving each logical index independently. Slice-backed inputs retain
    /// their ordinary contiguous walk.
    pub fn for_each(&self, mut visit: impl FnMut(NodeView<'a>)) {
        match self.source {
            NodeCursorSource::Slice(nodes) => nodes.iter().map(NodeView::from).for_each(visit),
            NodeCursorSource::Fork(view, annex) => view.for_each(|record| {
                if let Some(node) = record.decode_owned(annex) {
                    visit(NodeView::from_owned(node));
                }
            }),
        }
    }

    /// Visits sequential nodes as small borrowed direct-record references.
    ///
    /// Compact page inputs never construct a complete [`Node`] or
    /// [`NodeView`]. Callers request only the scalar or annex fields consumed
    /// by their algorithm.
    pub fn for_each_direct(&self, mut visit: impl FnMut(DirectNodeView<'a>)) {
        match self.source {
            NodeCursorSource::Slice(nodes) => {
                nodes.iter().map(DirectNodeView::owned).for_each(visit)
            }
            NodeCursorSource::Fork(view, annex) => view.for_each(|record| {
                visit(DirectNodeView::page(
                    crate::page_node_arena::PageMaterialNodeRef::new(record, annex),
                ));
            }),
        }
    }

    /// Visits one logical range in forward order without materializing an
    /// iterator-side successor structure.
    ///
    /// Arena-backed inputs walk their sole predecessor topology once and use
    /// the Rust call stack as the temporary continuation. This is the natural
    /// zero-allocation boundary for long sequential scans.
    pub fn try_for_each_range<B>(
        &self,
        selected: core::ops::Range<usize>,
        mut visit: impl FnMut(usize, NodeView<'a>) -> core::ops::ControlFlow<B>,
    ) -> core::ops::ControlFlow<B> {
        assert!(
            selected.start <= selected.end && selected.end <= self.len(),
            "node traversal range must be in bounds"
        );
        match self.source {
            NodeCursorSource::Slice(nodes) => {
                for (offset, node) in nodes[selected.clone()].iter().enumerate() {
                    if let core::ops::ControlFlow::Break(value) =
                        visit(selected.start + offset, NodeView::from(node))
                    {
                        return core::ops::ControlFlow::Break(value);
                    }
                }
                core::ops::ControlFlow::Continue(())
            }
            NodeCursorSource::Fork(view, annex) => {
                view.try_for_each_range(selected, |index, record| {
                    record
                        .decode_owned(annex)
                        .map(NodeView::from_owned)
                        .map_or(core::ops::ControlFlow::Continue(()), |node| {
                            visit(index, node)
                        })
                })
            }
        }
    }

    /// Visits one logical range through borrowed direct-record references.
    pub fn try_for_each_direct_range<B>(
        &self,
        selected: core::ops::Range<usize>,
        mut visit: impl FnMut(usize, DirectNodeView<'a>) -> core::ops::ControlFlow<B>,
    ) -> core::ops::ControlFlow<B> {
        assert!(
            selected.start <= selected.end && selected.end <= self.len(),
            "node traversal range must be in bounds"
        );
        match self.source {
            NodeCursorSource::Slice(nodes) => {
                for (offset, node) in nodes[selected.clone()].iter().enumerate() {
                    if let core::ops::ControlFlow::Break(value) =
                        visit(selected.start + offset, DirectNodeView::owned(node))
                    {
                        return core::ops::ControlFlow::Break(value);
                    }
                }
                core::ops::ControlFlow::Continue(())
            }
            NodeCursorSource::Fork(view, annex) => {
                view.try_for_each_range(selected, |index, record| {
                    visit(
                        index,
                        DirectNodeView::page(crate::page_node_arena::PageMaterialNodeRef::new(
                            record, annex,
                        )),
                    )
                })
            }
        }
    }

    /// Visits every node in one logical range through the linear callback
    /// traversal boundary.
    pub fn for_each_range(
        &self,
        selected: core::ops::Range<usize>,
        mut visit: impl FnMut(usize, NodeView<'a>),
    ) {
        let _: core::ops::ControlFlow<core::convert::Infallible> =
            self.try_for_each_range(selected, |index, node| {
                visit(index, node);
                core::ops::ControlFlow::Continue(())
            });
    }
}

impl<'a> IntoIterator for NodeCursor<'a> {
    type Item = NodeView<'a>;
    type IntoIter = NodeCursorIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

pub enum NodeCursorIter<'a> {
    Slice(core::slice::Iter<'a, Node>),
    Fork(
        crate::fork_arena::ArenaListIter<
            'a,
            crate::node_record::NodeRecord,
            crate::fork_arena::PageMaterialLane,
        >,
        crate::node_record::NodeAnnexView<'a>,
    ),
}

impl NodeCursorIter<'_> {
    /// Explicitly materializes borrowed projections as owned enum values.
    ///
    /// This mirrors `Iterator::cloned` for callers which intentionally need
    /// mutation scratch or detached test evidence. Ordinary consumers should
    /// continue matching the `NodeView` items yielded by the iterator.
    pub fn cloned(self) -> impl DoubleEndedIterator<Item = Node> + ExactSizeIterator {
        self.map(|node| node.to_owned())
    }
}

impl<'a> Iterator for NodeCursorIter<'a> {
    type Item = NodeView<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Slice(nodes) => nodes.next().map(NodeView::from),
            Self::Fork(nodes, annex) => nodes
                .next()
                .and_then(|record| record.decode_owned(*annex))
                .map(NodeView::from_owned),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Slice(nodes) => nodes.size_hint(),
            Self::Fork(nodes, _) => nodes.size_hint(),
        }
    }
}

impl ExactSizeIterator for NodeCursorIter<'_> {}

impl<'a> DoubleEndedIterator for NodeCursorIter<'a> {
    fn next_back(&mut self) -> Option<Self::Item> {
        match self {
            Self::Slice(nodes) => nodes.next_back().map(NodeView::from),
            Self::Fork(nodes, annex) => nodes
                .next_back()
                .and_then(|record| record.decode_owned(*annex))
                .map(NodeView::from_owned),
        }
    }
}

/// Lazy same-font byte-character run.
pub struct CharCodes<'a> {
    nodes: NodeCursor<'a>,
    next: usize,
    font: crate::ids::FontId,
}

impl<'a> CharCodes<'a> {
    fn new(nodes: NodeCursor<'a>, index: usize) -> Option<Self> {
        let (font, ch, _) = nodes.get_direct(index)?.character()?;
        u8::try_from(ch as u32).ok()?;
        Some(Self {
            nodes,
            next: index,
            font,
        })
    }
    #[must_use]
    pub const fn font(&self) -> crate::ids::FontId {
        self.font
    }
}

impl Iterator for CharCodes<'_> {
    type Item = u8;
    fn next(&mut self) -> Option<Self::Item> {
        let (font, ch, _) = self.nodes.get_direct(self.next)?.character()?;
        if font != self.font {
            return None;
        }
        let code = u8::try_from(ch as u32).ok()?;
        self.next += 1;
        Some(code)
    }
}

/// Borrowed maximal run of page-arena byte characters with one font.
#[derive(Clone, Copy, Debug)]
pub struct CharRun<'a> {
    nodes: NodeCursor<'a>,
    start: usize,
    end: usize,
    font: crate::ids::FontId,
}

impl<'a> CharRun<'a> {
    fn new(nodes: NodeCursor<'a>, start: usize) -> Option<Self> {
        let Node::Char { font, ch, .. } = nodes.owned_node(start)? else {
            return None;
        };
        u8::try_from(*ch as u32).ok()?;
        let mut end = start + 1;
        while let Some(Node::Char {
            font: candidate,
            ch,
            ..
        }) = nodes.owned_node(end)
        {
            if candidate != font || u8::try_from(*ch as u32).is_err() {
                break;
            }
            end += 1;
        }
        Some(Self {
            nodes,
            start,
            end,
            font: *font,
        })
    }

    #[must_use]
    pub const fn font(self) -> crate::ids::FontId {
        self.font
    }

    #[must_use]
    pub const fn len(self) -> usize {
        self.end - self.start
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    pub fn codes(self) -> impl ExactSizeIterator<Item = u8> + 'a {
        (self.start..self.end).map(move |index| {
            let node = self
                .nodes
                .owned_node(index)
                .expect("character-run index remains live");
            let Node::Char { ch, .. } = node else {
                unreachable!("character-run bounds contain only characters")
            };
            u8::try_from(*ch as u32).expect("character-run bounds contain only byte characters")
        })
    }

    pub fn origins(self) -> impl ExactSizeIterator<Item = crate::token::OriginId> + 'a {
        (self.start..self.end).map(move |index| {
            let node = self
                .nodes
                .owned_node(index)
                .expect("character-run index remains live");
            let Node::Char { origin, .. } = node else {
                unreachable!("character-run bounds contain only characters")
            };
            *origin
        })
    }
}
