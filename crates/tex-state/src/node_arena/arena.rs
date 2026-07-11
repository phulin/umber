use super::checked_len;
use super::copy::ChildPatch;
#[cfg(feature = "node-stats")]
use super::measurement::NodeMemoryColumn;
use super::storage::{NodeArenaMark, NodeStorage};
use super::view::NodeList;
use crate::identity::{HandleIdentity, IdentityAllocator};
use crate::ids::{ArenaRef, NodeListId};
use crate::node::Node;
use crate::survivor::SurvivorArena;

pub struct NodeListBuilder {
    buf: Vec<Node>,
}
impl NodeListBuilder {
    pub(crate) fn new() -> Self {
        Self { buf: Vec::new() }
    }
    pub fn push(&mut self, node: Node) {
        self.buf.push(node)
    }
    pub fn reserve(&mut self, additional: usize) {
        self.buf.reserve(additional)
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.buf.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
    #[must_use]
    pub(crate) fn as_slice(&self) -> &[Node] {
        &self.buf
    }
    pub fn clear(&mut self) {
        self.buf.clear()
    }
    pub(crate) fn finish(&mut self, arena: &mut NodeArena) -> NodeListId {
        let id = arena.append(&self.buf);
        self.buf.clear();
        id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EpochSpan {
    pub(crate) start: u32,
    pub(crate) len: u32,
}

#[derive(Debug)]
pub struct NodeArena {
    pub(super) storage: NodeStorage,
    identities: IdentityAllocator,
    spans: Vec<EpochSpan>,
}

impl Clone for NodeArena {
    fn clone(&self) -> Self {
        Self {
            storage: self.storage.clone(),
            identities: self.identities.fork(),
            spans: self.spans.clone(),
        }
    }
}

impl Default for NodeArena {
    fn default() -> Self {
        Self {
            storage: NodeStorage::default(),
            identities: IdentityAllocator::new(1),
            spans: vec![EpochSpan { start: 0, len: 0 }],
        }
    }
}
impl NodeArena {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn builder() -> NodeListBuilder {
        NodeListBuilder::new()
    }
    pub(crate) fn get<'a>(&'a self, id: NodeListId, survivors: &'a SurvivorArena) -> NodeList<'a> {
        match id.arena() {
            ArenaRef::Epoch => self.get_epoch(id),
            ArenaRef::Survivor(_) => survivors.get(id),
        }
    }
    pub(crate) fn get_epoch(&self, id: NodeListId) -> NodeList<'_> {
        let span = self
            .span(id)
            .unwrap_or_else(|| panic!("epoch node-list id is not live: {id:?}"));
        self.storage.view(span.start, span.len)
    }
    pub(crate) fn contains(&self, id: NodeListId) -> bool {
        matches!(id.arena(), ArenaRef::Epoch)
            && !id.is_format_reference()
            && self.identities.contains(id.epoch_identity())
    }
    pub(crate) fn watermark(&self) -> NodeArenaMark {
        NodeArenaMark {
            storage: self.storage.mark(),
            identities: self.identities.watermark(),
        }
    }
    pub(crate) fn truncate_to(&mut self, mark: NodeArenaMark) {
        self.identities
            .rollback(mark.identities)
            .expect("node identity rollback mark must name a retained ancestor");
        self.spans.truncate(mark.identities.len());
        self.storage.truncate(mark.storage)
    }
    #[cfg(feature = "node-stats")]
    pub(crate) fn memory_columns(&self) -> Vec<NodeMemoryColumn> {
        self.storage.memory_columns("epoch")
    }
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing_node_count(&self) -> usize {
        self.storage.len()
    }
    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    pub(crate) fn testing_all_nodes(&self) -> NodeList<'_> {
        self.storage.view(
            0,
            checked_len(self.storage.len(), "node arena exceeds u32 entries"),
        )
    }
    pub(crate) fn append(&mut self, nodes: &[Node]) -> NodeListId {
        if nodes.is_empty() {
            return NodeListId::new_epoch(HandleIdentity::builtin(0));
        }
        let start = checked_len(self.storage.len(), "node arena exceeds u32 entries");
        self.debug_assert_bottom_up(nodes, start);
        #[cfg(feature = "node-stats")]
        for n in nodes {
            crate::node::record_node_append(n);
        }
        let (start, len) = self.storage.append(nodes);
        self.mint_span(start, len)
    }
    pub(crate) fn append_compact_remapped(
        &mut self,
        source: NodeList<'_>,
        patches: &mut Vec<ChildPatch>,
        mut remap: impl FnMut(NodeListId) -> NodeListId,
    ) -> NodeListId {
        debug_assert!(
            patches.is_empty(),
            "epoch child-patch scratch must be clear"
        );
        let (start, len) = self.storage.append_compact(source, patches);
        for patch in patches.drain(..) {
            let patch = patch.remap(&mut remap);
            #[cfg(debug_assertions)]
            patch.for_each_child(|child| {
                let child = self
                    .span(child)
                    .expect("patched epoch child node-list id must be live");
                let end = child
                    .start
                    .checked_add(child.len)
                    .expect("child span overflow");
                debug_assert!(end <= start, "epoch child must end before its parent");
            });
            self.storage.apply_child_patch(patch);
        }
        if len == 0 {
            NodeListId::new_epoch(HandleIdentity::builtin(0))
        } else {
            self.mint_span(start, len)
        }
    }
    #[cfg(debug_assertions)]
    fn debug_assert_bottom_up(&self, nodes: &[Node], new_start: u32) {
        let mut children = Vec::new();
        for node in nodes {
            node.child_lists(&mut children)
        }
        for child in children {
            if let ArenaRef::Epoch = child.arena() {
                let child = self.span(child).expect("child node-list id is not live");
                let end = child
                    .start
                    .checked_add(child.len)
                    .expect("child span overflow");
                debug_assert!(
                    end <= new_start,
                    "child node-list span must be frozen below the parent span"
                );
                debug_assert!(
                    end as usize <= self.storage.len(),
                    "child node-list id is not live"
                );
            }
        }
    }
    #[cfg(not(debug_assertions))]
    fn debug_assert_bottom_up(&self, _: &[Node], _: u32) {}

    pub(crate) fn span(&self, id: NodeListId) -> Option<EpochSpan> {
        if !self.contains(id) {
            return None;
        }
        self.spans.get(id.epoch_identity().slot() as usize).copied()
    }

    fn mint_span(&mut self, start: u32, len: u32) -> NodeListId {
        let identity = self
            .identities
            .allocate()
            .expect("node-list identity capacity exhausted");
        assert_eq!(
            identity.slot() as usize,
            self.spans.len(),
            "node-list identity and span tables diverged"
        );
        self.spans.push(EpochSpan { start, len });
        NodeListId::new_epoch(identity)
    }

    #[cfg(test)]
    pub(crate) fn testing_subspan(&mut self, id: NodeListId, offset: u32, len: u32) -> NodeListId {
        let span = self.span(id).expect("source node-list id must be live");
        let start = span
            .start
            .checked_add(offset)
            .expect("subspan start overflow");
        let end = start.checked_add(len).expect("subspan end overflow");
        assert!(end <= span.start + span.len, "subspan exceeds source list");
        if len == 0 {
            NodeListId::new_epoch(HandleIdentity::builtin(0))
        } else {
            self.mint_span(start, len)
        }
    }
}
