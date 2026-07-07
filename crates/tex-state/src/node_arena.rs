//! Epoch arena storage for immutable node lists.
//!
//! Node-list watermarks are crate-private so rollback remains coupled to the
//! aggregate `Stores`/future `Universe` boundary.

use crate::ids::{ArenaRef, NodeListId};
use crate::node::Node;
use crate::survivor::SurvivorArena;

/// A rollback watermark for the epoch node arena.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NodeArenaMark {
    nodes: u32,
}

/// An owned scratch buffer for building a node list before freezing it.
#[derive(Clone, Debug, Default)]
pub struct NodeListBuilder {
    buf: Vec<Node>,
}

impl NodeListBuilder {
    /// Creates an empty reusable node-list builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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

    /// Clears the unfinished list without freezing it.
    pub fn clear(&mut self) {
        self.buf.clear();
    }

    /// Freezes the current node list into `arena` and clears this builder.
    pub fn finish(&mut self, arena: &mut NodeArena) -> NodeListId {
        let id = arena.append(&self.buf);
        self.buf.clear();
        id
    }
}

/// Per-epoch bump arena for frozen node lists.
#[derive(Clone, Debug, Default)]
pub struct NodeArena {
    nodes: Vec<Node>,
}

impl NodeArena {
    /// Creates an empty epoch node arena.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a fresh owned scratch builder.
    #[must_use]
    pub fn builder() -> NodeListBuilder {
        NodeListBuilder::new()
    }

    /// Reads a live frozen epoch node list.
    #[must_use]
    pub fn get<'a>(&'a self, id: NodeListId, survivors: &'a SurvivorArena) -> &'a [Node] {
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
        self.debug_assert_bottom_up(nodes, start);
        self.nodes.extend_from_slice(nodes);
        NodeListId::new_epoch(start, len)
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
mod tests {
    use super::{NodeArena, NodeListBuilder};
    use crate::glue::Order;
    use crate::ids::{FontId, NodeListId};
    use crate::node::{BoxNode, BoxNodeFields, Node, Sign};
    use crate::scaled::Scaled;

    #[test]
    fn nested_lists_build_bottom_up_and_read_back() {
        let mut arena = NodeArena::new();
        let survivors = crate::survivor::SurvivorArena::new();

        let mut inner = NodeListBuilder::new();
        inner.push(Node::Char {
            font: FontId::testing_new(1),
            ch: 'x',
        });
        let inner_id = inner.finish(&mut arena);

        let mut middle = NodeListBuilder::new();
        middle.push(Node::HList(BoxNode::new(BoxNodeFields {
            width: scaled(10),
            height: scaled(7),
            depth: scaled(3),
            shift: scaled(1),
            glue_set: -0.0,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: inner_id,
        })));
        let middle_id = middle.finish(&mut arena);

        let mut outer = NodeListBuilder::new();
        outer.push(Node::VList(BoxNode::new(BoxNodeFields {
            width: scaled(20),
            height: scaled(9),
            depth: scaled(4),
            shift: scaled(0),
            glue_set: 1.5,
            glue_sign: Sign::Stretching,
            glue_order: Order::Fil,
            children: middle_id,
        })));
        let outer_id = outer.finish(&mut arena);

        assert_eq!(
            arena.get(inner_id, &survivors),
            &[Node::Char {
                font: FontId::testing_new(1),
                ch: 'x'
            }]
        );
        let [Node::HList(middle_box)] = arena.get(middle_id, &survivors) else {
            panic!("middle list should contain one hlist")
        };
        assert_eq!(middle_box.children, inner_id);
        assert_eq!(middle_box.glue_set.to_bits(), 0.0_f64.to_bits());
        let [Node::VList(outer_box)] = arena.get(outer_id, &survivors) else {
            panic!("outer list should contain one vlist")
        };
        assert_eq!(outer_box.children, middle_id);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "child node-list span must be frozen below the parent span")]
    fn bottom_up_debug_assert_fires_on_hand_constructed_violation() {
        let mut arena = NodeArena::new();
        let future_id = NodeListId::testing_epoch(0, 1);

        let mut builder = NodeListBuilder::new();
        builder.push(Node::Adjust(future_id));

        let _ = builder.finish(&mut arena);
    }

    #[test]
    fn watermark_truncation_drops_exactly_the_suffix() {
        let mut arena = NodeArena::new();
        let survivors = crate::survivor::SurvivorArena::new();
        let kept = one_char(&mut arena, 'a');
        let mark = arena.watermark();
        let dropped = one_char(&mut arena, 'b');

        assert_eq!(arena.get(dropped, &survivors).len(), 1);
        arena.truncate_to(mark);

        assert_eq!(arena.get(kept, &survivors).len(), 1);
        assert!(!arena.contains(dropped));
        let replacement = one_char(&mut arena, 'c');
        assert_eq!(replacement.start(), dropped.start());
        assert_eq!(
            arena.get(replacement, &survivors)[0],
            Node::Char {
                font: FontId::testing_new(1),
                ch: 'c',
            }
        );
    }

    #[test]
    fn builder_reuse_after_finish_leaves_buffer_empty() {
        let mut arena = NodeArena::new();
        let survivors = crate::survivor::SurvivorArena::new();
        let mut builder = NodeListBuilder::new();

        builder.push(Node::MathOn);
        let first = builder.finish(&mut arena);
        assert!(builder.is_empty());

        builder.push(Node::MathOff);
        let second = builder.finish(&mut arena);

        assert_eq!(arena.get(first, &survivors), &[Node::MathOn]);
        assert_eq!(arena.get(second, &survivors), &[Node::MathOff]);
        assert!(builder.is_empty());
    }

    fn one_char(arena: &mut NodeArena, ch: char) -> NodeListId {
        let mut builder = NodeListBuilder::new();
        builder.push(Node::Char {
            font: FontId::testing_new(1),
            ch,
        });
        builder.finish(arena)
    }

    fn scaled(raw: i32) -> Scaled {
        Scaled::from_raw(raw)
    }
}
