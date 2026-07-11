use super::checked_len;
use super::copy::ChildPatch;
#[cfg(feature = "node-stats")]
use super::measurement::NodeMemoryColumn;
use super::storage::{NodeArenaMark, NodeStorage};
use super::view::NodeList;
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

#[derive(Clone, Debug, Default)]
pub struct NodeArena {
    pub(super) storage: NodeStorage,
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
            ArenaRef::Epoch => self.storage.view(id.start(), id.len()),
            ArenaRef::Survivor(_) => survivors.get(id),
        }
    }
    pub(crate) fn get_epoch(&self, id: NodeListId) -> NodeList<'_> {
        assert!(matches!(id.arena(), ArenaRef::Epoch));
        self.storage.view(id.start(), id.len())
    }
    pub(crate) fn contains(&self, id: NodeListId) -> bool {
        matches!(id.arena(), ArenaRef::Epoch)
            && (id.start() as usize)
                .checked_add(id.len() as usize)
                .is_some_and(|e| e <= self.storage.len())
    }
    pub(crate) fn watermark(&self) -> NodeArenaMark {
        NodeArenaMark(self.storage.mark())
    }
    pub(crate) fn truncate_to(&mut self, mark: NodeArenaMark) {
        self.storage.truncate(mark.0)
    }
    #[cfg(feature = "node-stats")]
    pub(crate) fn memory_columns(&self) -> Vec<NodeMemoryColumn> {
        self.storage.memory_columns("epoch")
    }
    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    pub(crate) fn testing_node_count(&self) -> usize {
        self.storage.len()
    }
    pub(crate) fn append(&mut self, nodes: &[Node]) -> NodeListId {
        let start = checked_len(self.storage.len(), "node arena exceeds u32 entries");
        self.debug_assert_bottom_up(nodes, start);
        #[cfg(feature = "node-stats")]
        for n in nodes {
            crate::node::record_node_append(n);
        }
        let (start, len) = self.storage.append(nodes);
        NodeListId::new_epoch(start, len)
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
                let end = child
                    .start()
                    .checked_add(child.len())
                    .expect("child span overflow");
                debug_assert!(matches!(child.arena(), ArenaRef::Epoch));
                debug_assert!(end <= start, "epoch child must end before its parent");
            });
            self.storage.apply_child_patch(patch);
        }
        NodeListId::new_epoch(start, len)
    }
    #[cfg(debug_assertions)]
    fn debug_assert_bottom_up(&self, nodes: &[Node], new_start: u32) {
        let mut children = Vec::new();
        for node in nodes {
            node.child_lists(&mut children)
        }
        for child in children {
            if let ArenaRef::Epoch = child.arena() {
                let end = child
                    .start()
                    .checked_add(child.len())
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
}
