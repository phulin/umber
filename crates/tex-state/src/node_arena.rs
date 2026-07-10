//! Epoch arena storage for immutable node lists.
//!
//! Node-list watermarks are crate-private so rollback remains coupled to the
//! aggregate `Universe` boundary.

use crate::ids::{ArenaRef, NodeListId};
use crate::node::Node;
use crate::survivor::SurvivorArena;

/// A rollback watermark for the epoch node arena.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct NodeArenaMark {
    nodes: u32,
}

/// An owned scratch buffer for building a node list before freezing it.
#[derive(Clone, Debug)]
pub struct NodeListBuilder {
    buf: Vec<Node>,
}

impl NodeListBuilder {
    /// Creates an empty reusable node-list builder.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Appends one node to the unfinished list.
    pub fn push(&mut self, node: Node) {
        self.buf.push(node);
    }

    /// Returns the number of nodes currently buffered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Returns whether the builder currently holds no nodes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Returns the unfinished list content.
    #[must_use]
    pub(crate) fn as_slice(&self) -> &[Node] {
        &self.buf
    }

    /// Clears the unfinished list without freezing it.
    pub fn clear(&mut self) {
        self.buf.clear();
    }

    /// Freezes the current node list into `arena` and clears this builder.
    pub(crate) fn finish(&mut self, arena: &mut NodeArena) -> NodeListId {
        let id = arena.append(&self.buf);
        self.buf.clear();
        id
    }
}

/// Per-epoch bump arena for frozen node lists.
#[derive(Clone, Debug)]
pub struct NodeArena {
    nodes: Vec<Node>,
}

impl NodeArena {
    /// Creates an empty epoch node arena.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    /// Creates a fresh owned scratch builder.
    #[must_use]
    pub(crate) fn builder() -> NodeListBuilder {
        NodeListBuilder::new()
    }

    /// Reads a live frozen epoch node list.
    #[must_use]
    pub(crate) fn get<'a>(&'a self, id: NodeListId, survivors: &'a SurvivorArena) -> &'a [Node] {
        match id.arena() {
            ArenaRef::Epoch => {
                let start = id.start() as usize;
                let end = start + id.len() as usize;
                assert!(end <= self.nodes.len(), "node-list id is not live");
                &self.nodes[start..end]
            }
            ArenaRef::Survivor(_) => survivors.get(id),
        }
    }

    /// Reads a live frozen epoch node list.
    #[must_use]
    pub(crate) fn get_epoch(&self, id: NodeListId) -> &[Node] {
        assert!(
            matches!(id.arena(), ArenaRef::Epoch),
            "expected epoch node-list id"
        );
        let start = id.start() as usize;
        let end = start + id.len() as usize;
        assert!(end <= self.nodes.len(), "node-list id is not live");
        &self.nodes[start..end]
    }

    /// Returns whether `id` names a currently-live epoch span in this arena.
    #[must_use]
    pub(crate) fn contains(&self, id: NodeListId) -> bool {
        match id.arena() {
            ArenaRef::Epoch => (id.start() as usize)
                .checked_add(id.len() as usize)
                .is_some_and(|end| end <= self.nodes.len()),
            ArenaRef::Survivor(_) => false,
        }
    }

    /// Takes a rollback watermark for aggregate snapshots.
    #[must_use]
    pub(crate) fn watermark(&self) -> NodeArenaMark {
        NodeArenaMark {
            nodes: u32_len(self.nodes.len(), "node arena exceeds u32 entries"),
        }
    }

    /// Truncates to a previously-taken aggregate snapshot watermark.
    pub(crate) fn truncate_to(&mut self, mark: NodeArenaMark) {
        let nodes = mark.nodes as usize;
        assert!(
            nodes <= self.nodes.len(),
            "node-arena mark has too many nodes"
        );
        self.nodes.truncate(nodes);
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    #[must_use]
    pub(crate) fn testing_node_count(&self) -> usize {
        self.nodes.len()
    }

    pub(crate) fn append(&mut self, nodes: &[Node]) -> NodeListId {
        let start = u32_len(self.nodes.len(), "node arena exceeds u32 entries");
        let len = u32_len(nodes.len(), "node list exceeds u32 entries");
        let id = NodeListId::new_epoch(start, len);
        self.debug_assert_bottom_up(nodes, start);
        #[cfg(feature = "node-stats")]
        for node in nodes {
            crate::node::record_node_append(node);
        }
        self.nodes.extend_from_slice(nodes);
        id
    }

    #[cfg(debug_assertions)]
    fn debug_assert_bottom_up(&self, nodes: &[Node], new_start: u32) {
        let mut children = Vec::new();
        for node in nodes {
            node.child_lists(&mut children);
        }

        for child in children {
            match child.arena() {
                ArenaRef::Epoch => {
                    let child_end = child
                        .start()
                        .checked_add(child.len())
                        .expect("child node-list span overflows u32");
                    debug_assert!(
                        child_end <= new_start,
                        "child node-list span must be frozen below the parent span"
                    );
                    debug_assert!(
                        (child_end as usize) <= self.nodes.len(),
                        "child node-list id is not live in this epoch arena"
                    );
                }
                ArenaRef::Survivor(_) => {}
            }
        }
    }

    #[cfg(not(debug_assertions))]
    fn debug_assert_bottom_up(&self, _nodes: &[Node], _new_start: u32) {}
}

fn u32_len(value: usize, message: &str) -> u32 {
    match u32::try_from(value) {
        Ok(value) => value,
        Err(_) => panic!("{message}"),
    }
}

#[cfg(test)]
mod tests;
