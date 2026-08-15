//! Direct ownership for immutable compact node-list graphs.
//!
//! A [`NodeListRef`] is the lifetime authority. Compact coordinates are
//! projections validated against its payload and cannot recover a dropped
//! graph. The optional candidate index below stores only bounded weak entries.

use super::{ChildPatch, NodeList, NodeSemanticId, NodeStorage, SidecarNeeds, checked_len};
use crate::ids::{ArenaRef, NodeListId, NodePayloadId};
use crate::node::Node;
use ahash::{AHashMap, AHashSet};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock, Weak};

const NODE_PAYLOAD_ROOT_MAX: u32 = (1 << 20) - 2;
const WEAK_INDEX_LIMIT: usize = 64;
static NEXT_NODE_PAYLOAD_ROOT: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OwnedSemanticSpan {
    pub(crate) start: u32,
    pub(crate) len: u32,
    pub(crate) semantic_id: NodeSemanticId,
}

/// One immutable, self-contained compact graph.
#[derive(Debug)]
pub(crate) struct NodeListPayload {
    root: NodePayloadId,
    pub(crate) storage: NodeStorage,
    semantic_spans: Box<[OwnedSemanticSpan]>,
    logical_bytes: usize,
    retained_bytes: usize,
}

/// A strong owner of one exact span in an immutable compact graph.
#[derive(Clone)]
pub struct NodeListRef {
    id: NodeListId,
    payload: Arc<NodeListPayload>,
}

impl PartialEq for NodeListRef {
    fn eq(&self, other: &Self) -> bool {
        (self.id == other.id && self.shares_payload(other)) || self.exact_semantic_eq(other)
    }
}

impl Eq for NodeListRef {}

impl core::hash::Hash for NodeListRef {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        core::hash::Hash::hash(&self.semantic_id(), state);
    }
}

impl core::fmt::Debug for NodeListRef {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("NodeListRef")
            .field("semantic_fingerprint", &self.semantic_fingerprint())
            .field("node_count", &self.len())
            .finish_non_exhaustive()
    }
}

impl NodeListRef {
    #[cfg(test)]
    pub(crate) fn testing_with_id(id: NodeListId) -> Self {
        assert!(
            id.is_empty(),
            "synthetic Env owner must be a zero-length projection"
        );
        let ArenaRef::Owned(root) = id.arena() else {
            panic!("synthetic Env owner must use an owned projection");
        };
        Self::from_payload(
            id,
            NodeListPayload::new(root, NodeStorage::default(), Vec::new()),
            NodeSemanticId::empty(),
        )
    }

    /// Returns the explicitly owned canonical empty list.
    #[must_use]
    pub fn empty() -> Self {
        static EMPTY: OnceLock<NodeListRef> = OnceLock::new();
        EMPTY
            .get_or_init(|| {
                let root = allocate_node_payload_root()
                    .expect("node-list payload coordinate space exhausted");
                let semantic_id = NodeSemanticId::empty();
                let id = NodeListId::new_owned(root, 0, 0);
                Self::from_payload(
                    id,
                    NodeListPayload::new(root, NodeStorage::default(), Vec::new()),
                    semantic_id,
                )
            })
            .clone()
    }

    pub(crate) fn from_payload(
        id: NodeListId,
        payload: NodeListPayload,
        expected_semantic_id: NodeSemanticId,
    ) -> Self {
        let owner = Self {
            id,
            payload: Arc::new(payload),
        };
        assert_eq!(
            owner.semantic_id(),
            expected_semantic_id,
            "node-list owner semantic projection mismatch"
        );
        owner
    }

    pub(crate) fn from_shared(id: NodeListId, payload: Arc<NodeListPayload>) -> Self {
        let owner = Self { id, payload };
        let _ = owner.semantic_id();
        owner
    }

    #[must_use]
    pub fn nodes(&self) -> NodeList<'_> {
        self.payload.storage.view(self.id.start(), self.id.len())
    }

    /// Materializes this owned list while resolving every direct child through
    /// the same immutable payload.
    #[must_use]
    pub fn to_vec(&self) -> Vec<Node> {
        self.nodes()
            .iter()
            .map(|node| {
                node.to_owned_with(|child| {
                    self.resolve(child)
                        .expect("owned node child is outside its enclosing payload")
                })
            })
            .collect()
    }

    /// Materializes one node while retaining structural ownership of all of
    /// its nested lists.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<Node> {
        self.nodes().get(index).map(|node| {
            node.to_owned_with(|child| {
                self.resolve(child)
                    .expect("owned node child is outside its enclosing payload")
            })
        })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.id.len() as usize
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.id.len() == 0
    }

    /// Returns the allocation-independent semantic fingerprint.
    #[must_use]
    pub fn semantic_fingerprint(&self) -> u64 {
        self.semantic_id().value()
    }

    /// Reports the logical compact bytes in the complete owned graph.
    #[must_use]
    pub fn logical_payload_bytes(&self) -> usize {
        self.payload.logical_bytes
    }

    /// Reports allocator-retained bytes in the complete owned graph.
    #[must_use]
    pub fn retained_payload_bytes(&self) -> usize {
        self.payload.retained_bytes
    }

    /// Resolves a compact child coordinate for the duration of this owner
    /// borrow. The coordinate cannot upgrade a dead payload or extend its
    /// lifetime.
    #[must_use]
    pub fn child_nodes(&self, child: NodeListId) -> Option<NodeList<'_>> {
        if child.is_empty() {
            return Some(self.payload.storage.view(0, 0));
        }
        let ArenaRef::Owned(root) = child.arena() else {
            return None;
        };
        (root == self.payload.root && self.payload.semantic_span(child).is_some())
            .then(|| self.payload.storage.view(child.start(), child.len()))
    }

    pub(crate) fn id(&self) -> NodeListId {
        self.id
    }

    pub(crate) fn semantic_id(&self) -> NodeSemanticId {
        if self.id.len() == 0 {
            return NodeSemanticId::empty();
        }
        self.payload
            .semantic_span(self.id)
            .expect("node-list owner span is not part of its payload")
            .semantic_id
    }

    /// Resolves an exact child span using only this structural owner.
    pub fn resolve(&self, child: NodeListId) -> Option<Self> {
        self.child_nodes(child)?;
        if child.len() == 0 {
            return Some(Self::empty());
        }
        Some(Self::from_shared(child, Arc::clone(&self.payload)))
    }

    pub(crate) fn downgrade(&self) -> NodeListWeak {
        NodeListWeak {
            id: self.id,
            semantic_id: self.semantic_id(),
            payload: Arc::downgrade(&self.payload),
        }
    }

    #[cfg(test)]
    pub(crate) fn strong_count(&self) -> usize {
        Arc::strong_count(&self.payload)
    }

    pub(crate) fn shares_payload(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.payload, &other.payload)
    }

    pub(crate) fn freeze_builder(
        mut nodes: Vec<Node>,
        children: Vec<NodeListRef>,
        semantic_id: NodeSemanticId,
        needs: SidecarNeeds,
    ) -> Self {
        if nodes.is_empty() {
            assert_eq!(semantic_id, NodeSemanticId::empty());
            return Self::empty();
        }

        let root =
            allocate_node_payload_root().expect("node-list payload coordinate space exhausted");
        let mut source = NodeStorage::default();
        let (_, len) = source.append_owned_preflighted(&mut nodes, needs);
        let mut storage = NodeStorage::default();
        let mut pending = Vec::new();
        let (start, copied_len) = storage.append_compact(source.view(0, len), &mut pending);
        assert_eq!((start, copied_len), (0, len));

        let root_id = NodeListId::new_owned(root, 0, len);
        let mut copier = DirectGraphCopy::new(root, storage, children);
        copier.semantic_spans.push(OwnedSemanticSpan {
            start: 0,
            len,
            semantic_id,
        });
        copier.pending = pending;
        copier.finish(root_id, semantic_id)
    }

    fn exact_semantic_eq(&self, other: &Self) -> bool {
        if self.semantic_id() != other.semantic_id() || self.len() != other.len() {
            return false;
        }
        let mut pending = vec![(self.clone(), other.clone())];
        let mut compared = AHashSet::new();
        while let Some((left, right)) = pending.pop() {
            if !compared.insert((left.id(), right.id())) {
                continue;
            }
            if left.len() != right.len() {
                return false;
            }
            for (left_node, right_node) in left.nodes().iter().zip(right.nodes().iter()) {
                if normalized_node(left_node.clone()) != normalized_node(right_node.clone()) {
                    return false;
                }
                let left_children = left_node.physical_children().collect::<Vec<_>>();
                let right_children = right_node.physical_children().collect::<Vec<_>>();
                if left_children.len() != right_children.len() {
                    return false;
                }
                for (left_child, right_child) in left_children.into_iter().zip(right_children) {
                    let Some(left_child) = left.resolve(left_child) else {
                        return false;
                    };
                    let Some(right_child) = right.resolve(right_child) else {
                        return false;
                    };
                    pending.push((left_child, right_child));
                }
            }
        }
        true
    }
}

impl NodeListPayload {
    pub(crate) fn new(
        root: NodePayloadId,
        storage: NodeStorage,
        mut semantic_spans: Vec<OwnedSemanticSpan>,
    ) -> Self {
        semantic_spans.sort_unstable_by_key(|span| (span.start, span.len));
        for duplicate in semantic_spans.windows(2) {
            if (duplicate[0].start, duplicate[0].len) == (duplicate[1].start, duplicate[1].len) {
                assert_eq!(
                    duplicate[0].semantic_id, duplicate[1].semantic_id,
                    "one owned node-list span has conflicting semantic identities"
                );
            }
        }
        semantic_spans.dedup_by_key(|span| (span.start, span.len));
        for span in &semantic_spans {
            let end = span
                .start
                .checked_add(span.len)
                .expect("owned node-list span overflow");
            assert!(
                end as usize <= storage.len(),
                "owned node-list span exceeds payload"
            );
        }
        let (storage_logical, storage_retained) = storage.payload_bytes();
        let span_logical = semantic_spans
            .len()
            .saturating_mul(core::mem::size_of::<OwnedSemanticSpan>());
        let span_retained = span_logical;
        Self {
            root,
            storage,
            semantic_spans: semantic_spans.into_boxed_slice(),
            logical_bytes: usize::try_from(storage_logical)
                .expect("node-list logical bytes exceed usize")
                .saturating_add(span_logical),
            retained_bytes: usize::try_from(storage_retained)
                .expect("node-list retained bytes exceed usize")
                .saturating_add(span_retained),
        }
    }

    fn semantic_span(&self, id: NodeListId) -> Option<OwnedSemanticSpan> {
        let ArenaRef::Owned(root) = id.arena() else {
            return None;
        };
        if root != self.root {
            return None;
        }
        let key = (id.start(), id.len());
        let index = self
            .semantic_spans
            .binary_search_by_key(&key, |span| (span.start, span.len))
            .ok()?;
        Some(self.semantic_spans[index])
    }

    #[cfg(test)]
    pub(crate) fn spans(&self) -> &[OwnedSemanticSpan] {
        &self.semantic_spans
    }

    pub(crate) fn spans_mut(&mut self) -> &mut [OwnedSemanticSpan] {
        &mut self.semantic_spans
    }
}

/// A non-owning coordinate projection. Upgrade succeeds only while a typed
/// owner still keeps the exact payload alive.
#[derive(Clone, Debug)]
pub(crate) struct NodeListWeak {
    id: NodeListId,
    semantic_id: NodeSemanticId,
    payload: Weak<NodeListPayload>,
}

impl NodeListWeak {
    pub(crate) fn upgrade(&self) -> Option<NodeListRef> {
        let payload = self.payload.upgrade()?;
        let owner = NodeListRef::from_shared(self.id, payload);
        (owner.semantic_id() == self.semantic_id).then_some(owner)
    }
}

/// Optional bounded, weak candidate acceleration for exact frozen graphs.
#[derive(Debug)]
pub(crate) struct NodeListWeakIndex {
    entries: VecDeque<NodeListWeak>,
}

impl NodeListWeakIndex {
    pub(crate) fn new() -> Self {
        Self {
            entries: VecDeque::new(),
        }
    }

    pub(crate) fn intern(&mut self, candidate: NodeListRef) -> NodeListRef {
        self.entries
            .retain(|entry| entry.payload.strong_count() != 0);
        if let Some(existing) =
            self.entries
                .iter()
                .filter_map(NodeListWeak::upgrade)
                .find(|value| {
                    value.semantic_fingerprint() == candidate.semantic_fingerprint()
                        && value.exact_semantic_eq(&candidate)
                })
        {
            return existing;
        }
        if self.entries.len() == WEAK_INDEX_LIMIT {
            self.entries.pop_front();
        }
        self.entries.push_back(candidate.downgrade());
        candidate
    }

    #[cfg(test)]
    pub(crate) fn shape(&self) -> (usize, usize) {
        (self.entries.len(), self.entries.capacity())
    }
}

struct DirectGraphCopy {
    root: NodePayloadId,
    storage: NodeStorage,
    sources: AHashMap<NodePayloadId, NodeListRef>,
    remapped: AHashMap<NodeListId, NodeListId>,
    pending: Vec<ChildPatch>,
    semantic_spans: Vec<OwnedSemanticSpan>,
}

impl DirectGraphCopy {
    fn new(root: NodePayloadId, storage: NodeStorage, children: Vec<NodeListRef>) -> Self {
        let mut sources = AHashMap::new();
        for child in children {
            if child.is_empty() {
                continue;
            }
            let ArenaRef::Owned(source_root) = child.id().arena() else {
                unreachable!("direct node-list owners use private compact payload coordinates")
            };
            if let Some(existing) = sources.insert(source_root, child.clone()) {
                assert!(
                    existing.shares_payload(&child),
                    "one compact root cannot name two immutable payloads"
                );
            }
        }
        Self {
            root,
            storage,
            sources,
            remapped: AHashMap::new(),
            pending: Vec::new(),
            semantic_spans: Vec::new(),
        }
    }

    fn finish(mut self, root_id: NodeListId, semantic_id: NodeSemanticId) -> NodeListRef {
        while let Some(patch) = self.pending.pop() {
            let patch = patch.remap(|child| self.copy_child(child));
            self.storage.apply_child_patch(patch);
        }
        let payload = NodeListPayload::new(self.root, self.storage, self.semantic_spans);
        NodeListRef::from_payload(root_id, payload, semantic_id)
    }

    fn copy_child(&mut self, source_id: NodeListId) -> NodeListId {
        if source_id.len() == 0 {
            let empty = NodeListRef::empty();
            assert_eq!(
                source_id,
                empty.id(),
                "direct builder empty child is not the canonical owner projection"
            );
            return empty.id();
        }
        if let Some(&remapped) = self.remapped.get(&source_id) {
            return remapped;
        }
        let ArenaRef::Owned(source_root) = source_id.arena() else {
            panic!("direct builder child coordinate is not structurally owned")
        };
        let source = self
            .sources
            .get(&source_root)
            .and_then(|owner| owner.resolve(source_id))
            .unwrap_or_else(|| panic!("direct builder child coordinate is stale or unowned"));

        let start = checked_len(self.storage.len(), "owned node graph exceeds u32 entries");
        let len = checked_len(source.len(), "owned child node list exceeds u32 entries");
        let remapped = NodeListId::new_owned(self.root, start, len);
        self.remapped.insert(source_id, remapped);
        self.semantic_spans.push(OwnedSemanticSpan {
            start,
            len,
            semantic_id: source.semantic_id(),
        });
        let appended = self
            .storage
            .append_compact(source.nodes(), &mut self.pending);
        assert_eq!(appended, (start, len));
        remapped
    }
}

/// Allocation-independent exact-node collision guard. Child coordinates are
/// compared recursively by [`NodeListRef::exact_semantic_eq`], so the local
/// projection retains only field order and empty/nonempty shape.
fn normalized_node(node: super::NodeRef<'_>) -> String {
    let root = NodePayloadId::new(0);
    let mut node = node.to_compact_owned();
    let mut child = 0;
    node.visit_lists_mut(|id| {
        *id = if id.is_empty() {
            NodeListId::new_owned(root, 0, 0)
        } else {
            child += 1;
            NodeListId::new_owned(root, child, 1)
        };
    });
    format!("{node:?}")
}

pub(crate) fn allocate_node_payload_root() -> Option<NodePayloadId> {
    NEXT_NODE_PAYLOAD_ROOT
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
            (next <= NODE_PAYLOAD_ROOT_MAX).then_some(next + 1)
        })
        .ok()
        .map(NodePayloadId::new)
}

#[cfg(test)]
mod tests;
