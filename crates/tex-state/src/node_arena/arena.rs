use super::checked_len;
#[cfg(feature = "profiling-stats")]
use super::measurement::NodeMemoryColumn;
use super::semantic::{NodeSemanticId, NodeSemanticIdBuilder};
use super::storage::{NodeArenaMark, NodeStorage, SidecarNeeds};
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
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn finish(&mut self, arena: &mut NodeArena) -> NodeListId {
        #[cfg(test)]
        {
            let id = arena.append(&self.buf);
            self.buf.clear();
            id
        }
        #[cfg(not(test))]
        {
            let _ = arena;
            panic!("node lists must be finished through Stores to compute semantic identity")
        }
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
    semantic_ids: Vec<NodeSemanticId>,
}

impl Clone for NodeArena {
    fn clone(&self) -> Self {
        Self {
            storage: self.storage.clone(),
            identities: self.identities.fork(),
            spans: self.spans.clone(),
            semantic_ids: self.semantic_ids.clone(),
        }
    }
}

impl Default for NodeArena {
    fn default() -> Self {
        Self {
            storage: NodeStorage::default(),
            identities: IdentityAllocator::new(1),
            spans: vec![EpochSpan { start: 0, len: 0 }],
            semantic_ids: vec![NodeSemanticIdBuilder::new().finish()],
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
        self.semantic_ids.truncate(mark.identities.len());
        self.storage.truncate(mark.storage)
    }
    #[cfg(feature = "profiling-stats")]
    pub(crate) fn memory_columns(&self) -> Vec<NodeMemoryColumn> {
        self.measurement_columns("epoch")
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
    // Used by tests and transitional format restoration, but the ordinary
    // freeze path carries a preflight plan from semantic validation.
    #[allow(dead_code)]
    pub(crate) fn append_with_semantic_id(
        &mut self,
        nodes: &[Node],
        semantic_id: NodeSemanticId,
    ) -> NodeListId {
        if nodes.is_empty() {
            debug_assert_eq!(semantic_id, self.semantic_ids[0]);
            return NodeListId::new_epoch(HandleIdentity::builtin(0));
        }
        let start = checked_len(self.storage.len(), "node arena exceeds u32 entries");
        self.debug_assert_bottom_up(nodes, start);
        #[cfg(feature = "profiling-stats")]
        for n in nodes {
            crate::node::record_node_append(n);
        }
        let (start, len) = self.storage.append(nodes);
        self.mint_span(start, len, semantic_id)
    }

    pub(crate) fn append_preflighted_with_semantic_id(
        &mut self,
        nodes: &[Node],
        semantic_id: NodeSemanticId,
        needs: SidecarNeeds,
    ) -> NodeListId {
        if nodes.is_empty() {
            debug_assert_eq!(semantic_id, self.semantic_ids[0]);
            return NodeListId::new_epoch(HandleIdentity::builtin(0));
        }
        let start = checked_len(self.storage.len(), "node arena exceeds u32 entries");
        self.debug_assert_bottom_up(nodes, start);
        #[cfg(feature = "profiling-stats")]
        for n in nodes {
            crate::node::record_node_append(n);
        }
        let (start, len) = self.storage.append_preflighted(nodes, needs);
        self.mint_span(start, len, semantic_id)
    }

    pub(crate) fn append_owned_preflighted_with_semantic_id(
        &mut self,
        nodes: &mut Vec<Node>,
        semantic_id: NodeSemanticId,
        needs: SidecarNeeds,
    ) -> NodeListId {
        if nodes.is_empty() {
            debug_assert_eq!(semantic_id, self.semantic_ids[0]);
            return NodeListId::new_epoch(HandleIdentity::builtin(0));
        }
        let start = checked_len(self.storage.len(), "node arena exceeds u32 entries");
        self.debug_assert_bottom_up(nodes, start);
        #[cfg(feature = "profiling-stats")]
        for n in nodes.iter() {
            crate::node::record_node_append(n);
        }
        let (start, len) = self.storage.append_owned_preflighted(nodes, needs);
        self.mint_span(start, len, semantic_id)
    }

    #[cfg(test)]
    pub(crate) fn append(&mut self, nodes: &[Node]) -> NodeListId {
        let semantic_id = if nodes.is_empty() {
            self.semantic_ids[0]
        } else {
            NodeSemanticId::testing(self.spans.len() as u64)
        };
        self.append_with_semantic_id(nodes, semantic_id)
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

    pub(crate) fn semantic_id(&self, id: NodeListId, survivors: &SurvivorArena) -> NodeSemanticId {
        match id.arena() {
            ArenaRef::Epoch => self.epoch_semantic_id(id),
            ArenaRef::Survivor(_) => survivors.semantic_id(id),
        }
    }

    pub(crate) fn epoch_semantic_id(&self, id: NodeListId) -> NodeSemanticId {
        assert!(self.contains(id), "epoch node-list id is not live: {id:?}");
        self.semantic_ids[id.epoch_identity().slot() as usize]
    }

    fn mint_span(&mut self, start: u32, len: u32, semantic_id: NodeSemanticId) -> NodeListId {
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
        self.semantic_ids.push(semantic_id);
        let id = NodeListId::new_epoch(identity);
        #[cfg(feature = "profiling-stats")]
        self.record_peak();
        id
    }

    #[cfg(feature = "profiling-stats")]
    pub(super) fn measurement_columns(&self, prefix: &str) -> Vec<NodeMemoryColumn> {
        let mut columns = self.storage.memory_columns(prefix);
        let (len, capacity, element_bytes) = self.identities.measurement_shape();
        columns.push(NodeMemoryColumn {
            name: format!("{prefix}.identity_tags"),
            len,
            capacity,
            element_bytes,
            logical_bytes: len * element_bytes,
            retained_payload_bytes: capacity * element_bytes,
        });
        let element_bytes = core::mem::size_of::<EpochSpan>();
        columns.push(NodeMemoryColumn {
            name: format!("{prefix}.spans"),
            len: self.spans.len(),
            capacity: self.spans.capacity(),
            element_bytes,
            logical_bytes: self.spans.len() * element_bytes,
            retained_payload_bytes: self.spans.capacity() * element_bytes,
        });
        let element_bytes = core::mem::size_of::<NodeSemanticId>();
        columns.push(NodeMemoryColumn {
            name: format!("{prefix}.semantic_ids"),
            len: self.semantic_ids.len(),
            capacity: self.semantic_ids.capacity(),
            element_bytes,
            logical_bytes: self.semantic_ids.len() * element_bytes,
            retained_payload_bytes: self.semantic_ids.capacity() * element_bytes,
        });
        columns
    }

    #[cfg(feature = "profiling-stats")]
    pub(super) fn measurement_payload_bytes(&self) -> (u64, u64) {
        let (mut logical, mut retained) = self.storage.payload_bytes();
        let (len, capacity, element_bytes) = self.identities.measurement_shape();
        logical += (len * element_bytes) as u64;
        retained += (capacity * element_bytes) as u64;
        let element_bytes = core::mem::size_of::<EpochSpan>();
        logical += (self.spans.len() * element_bytes) as u64;
        retained += (self.spans.capacity() * element_bytes) as u64;
        let element_bytes = core::mem::size_of::<NodeSemanticId>();
        logical += (self.semantic_ids.len() * element_bytes) as u64;
        retained += (self.semantic_ids.capacity() * element_bytes) as u64;
        (logical, retained)
    }

    pub(crate) fn retained_payload_bytes(&self) -> usize {
        let (_, capacity, element_bytes) = self.identities.measurement_shape();
        self.storage
            .retained_payload_bytes()
            .saturating_add(capacity.saturating_mul(element_bytes))
            .saturating_add(
                self.spans
                    .capacity()
                    .saturating_mul(core::mem::size_of::<EpochSpan>()),
            )
            .saturating_add(
                self.semantic_ids
                    .capacity()
                    .saturating_mul(core::mem::size_of::<NodeSemanticId>()),
            )
    }

    #[cfg(feature = "profiling-stats")]
    fn record_peak(&self) {
        super::measurement::record_peak_observation(self.measurement_payload_bytes(), || {
            self.measurement_columns("peak")
        });
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
            self.mint_span(start, len, NodeSemanticId::testing(self.spans.len() as u64))
        }
    }
}
