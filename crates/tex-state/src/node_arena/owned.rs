//! Direct ownership for immutable compact node-list graphs.
//!
//! A [`NodeListRef`] is the lifetime authority. Compact coordinates are
//! projections validated against its payload and cannot recover a dropped
//! graph. The optional candidate index below stores only bounded weak entries.

use super::builder::CompactBuilderNode;
use super::{NodeList, NodeSemanticId, NodeStorage, SidecarNeeds};
use crate::ids::{ArenaRef, NodeListId, NodePayloadId};
use crate::node::Node;
use ahash::AHashSet;
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

/// One immutable compact payload and its direct structural child owners.
#[derive(Debug)]
pub(crate) struct NodeListPayload {
    root: NodePayloadId,
    pub(crate) storage: NodeStorage,
    semantic_spans: Box<[OwnedSemanticSpan]>,
    children: Box<[NodeListRef]>,
    runtime_value_roots: Option<crate::hot_core::arena::store::RuntimeValueRootSet>,
    logical_bytes: usize,
    retained_bytes: usize,
}

/// A strong owner of one exact span in an immutable compact graph.
#[derive(Clone)]
pub struct NodeListRef {
    id: NodeListId,
    // `None` exists only while `Drop` moves the payload into its iterative
    // release worklist. Every observable owner retains `Some`.
    payload: Option<Arc<NodeListPayload>>,
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
            NodeListPayload::new(root, NodeStorage::default(), Vec::new(), Vec::new(), None),
            NodeSemanticId::empty(),
        )
    }

    /// Returns the explicitly owned canonical empty list.
    #[must_use]
    pub fn empty() -> Self {
        let payload = empty_payload();
        Self {
            id: NodeListId::new_owned(payload.root, 0, 0),
            payload: Some(Arc::clone(payload)),
        }
    }

    pub(crate) fn from_payload(
        id: NodeListId,
        payload: NodeListPayload,
        expected_semantic_id: NodeSemanticId,
    ) -> Self {
        let owner = Self {
            id,
            payload: Some(Arc::new(payload)),
        };
        assert_eq!(
            owner.semantic_id(),
            expected_semantic_id,
            "node-list owner semantic projection mismatch"
        );
        owner
    }

    pub(crate) fn from_shared(id: NodeListId, payload: Arc<NodeListPayload>) -> Self {
        let owner = Self {
            id,
            payload: Some(payload),
        };
        let _ = owner.semantic_id();
        owner
    }

    #[must_use]
    pub fn nodes(&self) -> NodeList<'_> {
        self.payload().storage.view(self.id.start(), self.id.len())
    }

    /// Materializes this owned list while resolving every direct child through
    /// this payload's structural owner set.
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

    /// Reports the logical compact bytes owned directly by this payload.
    #[must_use]
    pub fn logical_payload_bytes(&self) -> usize {
        self.payload().logical_bytes
    }

    /// Reports allocator-retained bytes owned directly by this payload.
    #[must_use]
    pub fn retained_payload_bytes(&self) -> usize {
        self.payload().retained_bytes.saturating_add(
            self.payload()
                .runtime_value_roots()
                .map_or(0, |roots| roots.retained_owner_bytes()),
        )
    }

    /// Resolves a compact child coordinate for the duration of this owner
    /// borrow. The coordinate cannot upgrade a dead payload or extend its
    /// lifetime.
    #[must_use]
    pub fn child_nodes(&self, child: NodeListId) -> Option<NodeList<'_>> {
        if child.is_empty() {
            return Some(self.payload().storage.view(0, 0));
        }
        let ArenaRef::Owned(root) = child.arena() else {
            return None;
        };
        if root == self.payload().root && self.payload().semantic_span(child).is_some() {
            return Some(self.payload().storage.view(child.start(), child.len()));
        }
        self.payload().child_owner(root)?.child_nodes(child)
    }

    pub(crate) fn id(&self) -> NodeListId {
        self.id
    }

    pub(crate) fn semantic_id(&self) -> NodeSemanticId {
        if self.id.is_empty() {
            return NodeSemanticId::empty();
        }
        self.payload()
            .semantic_span(self.id)
            .expect("node-list owner span is not part of its payload")
            .semantic_id
    }

    /// Resolves an exact child span using only this structural owner.
    pub fn resolve(&self, child: NodeListId) -> Option<Self> {
        if child.is_empty() {
            return Some(Self::empty());
        }
        let ArenaRef::Owned(root) = child.arena() else {
            return None;
        };
        if root == self.payload().root {
            self.child_nodes(child)?;
            return Some(Self::from_shared(child, Arc::clone(self.payload())));
        }
        self.payload().child_owner(root)?.resolve(child)
    }

    pub(crate) fn downgrade(&self) -> NodeListWeak {
        NodeListWeak {
            id: self.id,
            semantic_id: self.semantic_id(),
            payload: Arc::downgrade(self.payload()),
        }
    }

    #[cfg(test)]
    pub(crate) fn strong_count(&self) -> usize {
        Arc::strong_count(self.payload())
    }

    pub(crate) fn shares_payload(&self, other: &Self) -> bool {
        Arc::ptr_eq(self.payload(), other.payload())
    }

    fn payload(&self) -> &Arc<NodeListPayload> {
        self.payload
            .as_ref()
            .expect("live node-list owner must retain its payload")
    }

    #[cfg(test)]
    pub(crate) fn runtime_value_roots(
        &self,
    ) -> Option<&crate::hot_core::arena::store::RuntimeValueRootSet> {
        self.payload().runtime_value_roots()
    }

    pub(crate) fn freeze_builder(
        mut nodes: Vec<Node>,
        children: Vec<NodeListRef>,
        runtime_value_roots: Option<crate::hot_core::arena::store::RuntimeValueRootSet>,
        semantic_id: NodeSemanticId,
        needs: SidecarNeeds,
    ) -> Self {
        if nodes.is_empty() {
            assert_eq!(semantic_id, NodeSemanticId::empty());
            return Self::empty();
        }

        let root =
            allocate_node_payload_root().expect("node-list payload coordinate space exhausted");
        let mut storage = NodeStorage::default();
        let (start, len) = storage.append_owned_preflighted(&mut nodes, needs);
        assert_eq!(start, 0);

        let root_id = NodeListId::new_owned(root, 0, len);
        let payload = NodeListPayload::new(
            root,
            storage,
            vec![OwnedSemanticSpan {
                start: 0,
                len,
                semantic_id,
            }],
            children,
            runtime_value_roots,
        );
        Self::from_payload(root_id, payload, semantic_id)
    }

    pub(crate) fn freeze_compact_builder(
        rows: Vec<CompactBuilderNode>,
        semantic_id: NodeSemanticId,
    ) -> Self {
        if rows.is_empty() {
            assert_eq!(semantic_id, NodeSemanticId::empty());
            return Self::empty();
        }

        let root =
            allocate_node_payload_root().expect("node-list payload coordinate space exhausted");
        let mut storage = NodeStorage::default();
        let (start, len) = storage.append_compact_builder(rows);
        assert_eq!(start, 0);

        let root_id = NodeListId::new_owned(root, 0, len);
        let payload = NodeListPayload::new(
            root,
            storage,
            vec![OwnedSemanticSpan {
                start: 0,
                len,
                semantic_id,
            }],
            Vec::new(),
            None,
        );
        Self::from_payload(root_id, payload, semantic_id)
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
                if !super::schema::physical_shape_eq(&left_node, &right_node) {
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

impl Drop for NodeListRef {
    fn drop(&mut self) {
        let Some(payload) = self.payload.take() else {
            return;
        };
        let Ok(mut payload) = Arc::try_unwrap(payload) else {
            return;
        };
        let mut pending = Vec::new();

        loop {
            // Disarm the child owners before dropping the payload itself, then
            // release their Arcs from this explicit worklist. Otherwise each
            // final child would recursively enter Rust's field destructor.
            for child in &mut payload.children {
                pending.push(
                    child
                        .payload
                        .take()
                        .expect("owned child must retain its payload"),
                );
            }
            drop(payload);

            loop {
                let Some(next) = pending.pop() else {
                    return;
                };
                if let Ok(next) = Arc::try_unwrap(next) {
                    payload = next;
                    break;
                }
            }
        }
    }
}

fn empty_payload() -> &'static Arc<NodeListPayload> {
    static EMPTY: OnceLock<Arc<NodeListPayload>> = OnceLock::new();
    EMPTY.get_or_init(|| {
        let root =
            allocate_node_payload_root().expect("node-list payload coordinate space exhausted");
        Arc::new(NodeListPayload::new(
            root,
            NodeStorage::default(),
            Vec::new(),
            Vec::new(),
            None,
        ))
    })
}

impl NodeListPayload {
    pub(crate) fn new(
        root: NodePayloadId,
        storage: NodeStorage,
        mut semantic_spans: Vec<OwnedSemanticSpan>,
        mut children: Vec<NodeListRef>,
        runtime_value_roots: Option<crate::hot_core::arena::store::RuntimeValueRootSet>,
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
        children.retain(|child| !child.is_empty());
        children.sort_unstable_by_key(|child| child.payload().root);
        children.dedup_by(|left, right| {
            if left.payload().root != right.payload().root {
                return false;
            }
            assert!(
                left.shares_payload(right),
                "one child payload coordinate must identify one immutable allocation"
            );
            true
        });
        let (storage_logical, storage_retained) = storage.payload_bytes();
        let span_logical = semantic_spans
            .len()
            .saturating_mul(core::mem::size_of::<OwnedSemanticSpan>());
        let span_retained = span_logical;
        let child_bytes = children
            .len()
            .saturating_mul(core::mem::size_of::<NodeListRef>());
        Self {
            root,
            storage,
            semantic_spans: semantic_spans.into_boxed_slice(),
            children: children.into_boxed_slice(),
            runtime_value_roots,
            logical_bytes: usize::try_from(storage_logical)
                .expect("node-list logical bytes exceed usize")
                .saturating_add(span_logical)
                .saturating_add(child_bytes),
            retained_bytes: usize::try_from(storage_retained)
                .expect("node-list retained bytes exceed usize")
                .saturating_add(span_retained)
                .saturating_add(child_bytes),
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

    fn child_owner(&self, root: NodePayloadId) -> Option<&NodeListRef> {
        self.children
            .binary_search_by_key(&root, |child| child.payload().root)
            .ok()
            .map(|index| &self.children[index])
    }

    pub(crate) fn runtime_value_roots(
        &self,
    ) -> Option<&crate::hot_core::arena::store::RuntimeValueRootSet> {
        self.runtime_value_roots.as_ref()
    }

    #[cfg(test)]
    fn child_roots(&self) -> impl ExactSizeIterator<Item = NodePayloadId> + '_ {
        self.children.iter().map(|child| child.payload().root)
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

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn shape(&self) -> (usize, usize) {
        (self.entries.len(), self.entries.capacity())
    }
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
