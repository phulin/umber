//! Survivor arena for node lists that escape an epoch.
//!
//! Promotion copies an epoch-rooted node graph into one contiguous allocation
//! and rewrites child spans to be relative to the survivor root.

use crate::ids::{ArenaRef, NodeListId, SurvivorRootId};
use crate::node::Node;
use crate::node_arena::NodeArena;

/// Arena for promoted node-list roots.
#[derive(Clone, Debug, Default)]
pub struct SurvivorArena {
    slots: Vec<Option<SurvivorRoot>>,
    free: Vec<SurvivorRootId>,
}

#[derive(Clone, Debug)]
struct SurvivorRoot {
    nodes: Box<[Node]>,
    refcount: u32,
}

impl SurvivorArena {
    /// Creates an empty survivor arena.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Promotes an epoch list into one survivor root with refcount 1.
    pub(crate) fn promote(&mut self, id: NodeListId, epoch: &NodeArena) -> NodeListId {
        assert!(
            matches!(id.arena(), ArenaRef::Epoch),
            "only epoch node lists are promoted"
        );

        let mut nodes = Vec::new();
        let (start, len) = copy_list(id, epoch, &mut nodes);
        let root = self.allocate_root(nodes.into_boxed_slice());
        self.rewrite_root_ids(root);
        let promoted = NodeListId::new_survivor(root, start, len);
        self.debug_assert_no_epoch_ids(promoted);
        promoted
    }

    /// Reads a live survivor span.
    #[must_use]
    pub(crate) fn get(&self, id: NodeListId) -> &[Node] {
        let ArenaRef::Survivor(root) = id.arena() else {
            panic!("survivor arena can only read survivor node-list ids");
        };
        let root = self.root(root);
        let start = id.start() as usize;
        let end = start + id.len() as usize;
        assert!(end <= root.nodes.len(), "survivor node-list id is not live");
        &root.nodes[start..end]
    }

    /// Increments the root refcount for a survivor list.
    pub(crate) fn inc_ref(&mut self, id: NodeListId) {
        let ArenaRef::Survivor(root) = id.arena() else {
            panic!("only survivor node-list ids are refcounted");
        };
        let root = self.root_mut(root);
        root.refcount = root
            .refcount
            .checked_add(1)
            .expect("survivor root refcount overflow");
    }

    /// Decrements the root refcount and frees the slot at zero.
    pub(crate) fn dec_ref(&mut self, id: NodeListId) {
        let ArenaRef::Survivor(root_id) = id.arena() else {
            panic!("only survivor node-list ids are refcounted");
        };
        let root = self.root_mut(root_id);
        assert!(root.refcount > 0, "survivor root refcount underflow");
        root.refcount -= 1;
        if root.refcount == 0 {
            self.slots[root_id.raw() as usize] = None;
            self.free.push(root_id);
        }
    }

    /// Returns whether a survivor list names a live root and span.
    #[must_use]
    pub(crate) fn contains(&self, id: NodeListId) -> bool {
        let ArenaRef::Survivor(root) = id.arena() else {
            return false;
        };
        let Some(Some(slot)) = self.slots.get(root.raw() as usize) else {
            return false;
        };
        (id.start() as usize)
            .checked_add(id.len() as usize)
            .is_some_and(|end| end <= slot.nodes.len())
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_live_slot_count(&self) -> usize {
        self.slots.iter().filter(|slot| slot.is_some()).count()
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_refcount(&self, id: NodeListId) -> u32 {
        let ArenaRef::Survivor(root) = id.arena() else {
            panic!("expected survivor id");
        };
        self.root(root).refcount
    }

    fn allocate_root(&mut self, nodes: Box<[Node]>) -> SurvivorRootId {
        let slot = SurvivorRoot { nodes, refcount: 1 };
        if let Some(root) = self.free.pop() {
            self.slots[root.raw() as usize] = Some(slot);
            root
        } else {
            let raw = u32_len(self.slots.len(), "survivor arena exceeds u32 roots");
            self.slots.push(Some(slot));
            SurvivorRootId::new(raw)
        }
    }

    fn root(&self, root: SurvivorRootId) -> &SurvivorRoot {
        self.slots
            .get(root.raw() as usize)
            .and_then(Option::as_ref)
            .expect("survivor root is not live")
    }

    fn root_mut(&mut self, root: SurvivorRootId) -> &mut SurvivorRoot {
        self.slots
            .get_mut(root.raw() as usize)
            .and_then(Option::as_mut)
            .expect("survivor root is not live")
    }

    fn rewrite_root_ids(&mut self, root: SurvivorRootId) {
        for node in &mut self.root_mut(root).nodes {
            rewrite_node_root_ids(node, root);
        }
    }

    #[cfg(debug_assertions)]
    fn debug_assert_no_epoch_ids(&self, id: NodeListId) {
        for node in self.get(id) {
            debug_assert_no_epoch_ids_in_node(node);
        }
    }

    #[cfg(not(debug_assertions))]
    fn debug_assert_no_epoch_ids(&self, _id: NodeListId) {}
}

fn copy_list(id: NodeListId, epoch: &NodeArena, out: &mut Vec<Node>) -> (u32, u32) {
    let start = u32_len(out.len(), "promoted node root exceeds u32 entries");
    out.extend_from_slice(epoch.get_epoch(id));
    let len = id.len();
    for offset in 0..len as usize {
        remap_node_children(start as usize + offset, epoch, out);
    }
    (start, len)
}

fn remap_node_children(index: usize, epoch: &NodeArena, out: &mut Vec<Node>) {
    match out[index].clone() {
        Node::HList(mut box_node) => {
            let (start, len) = copy_list(box_node.children, epoch, out);
            box_node.children = NodeListId::new_survivor(SurvivorRootId::new(0), start, len);
            out[index] = Node::HList(box_node);
        }
        Node::VList(mut box_node) => {
            let (start, len) = copy_list(box_node.children, epoch, out);
            box_node.children = NodeListId::new_survivor(SurvivorRootId::new(0), start, len);
            out[index] = Node::VList(box_node);
        }
        Node::Disc { pre, post, replace } => {
            let (pre_start, pre_len) = copy_list(pre, epoch, out);
            let (post_start, post_len) = copy_list(post, epoch, out);
            let (replace_start, replace_len) = copy_list(replace, epoch, out);
            out[index] = Node::Disc {
                pre: NodeListId::new_survivor(SurvivorRootId::new(0), pre_start, pre_len),
                post: NodeListId::new_survivor(SurvivorRootId::new(0), post_start, post_len),
                replace: NodeListId::new_survivor(
                    SurvivorRootId::new(0),
                    replace_start,
                    replace_len,
                ),
            };
        }
        Node::Ins { class, content } => {
            let (start, len) = copy_list(content, epoch, out);
            out[index] = Node::Ins {
                class,
                content: NodeListId::new_survivor(SurvivorRootId::new(0), start, len),
            };
        }
        Node::Adjust(content) => {
            let (start, len) = copy_list(content, epoch, out);
            out[index] = Node::Adjust(NodeListId::new_survivor(SurvivorRootId::new(0), start, len));
        }
        Node::Char { .. }
        | Node::Lig { .. }
        | Node::Kern { .. }
        | Node::Glue { .. }
        | Node::Penalty(_)
        | Node::Rule { .. }
        | Node::Unset
        | Node::Mark { .. }
        | Node::Whatsit(_)
        | Node::MathOn
        | Node::MathOff => {}
    }
}

fn rewrite_node_root_ids(node: &mut Node, root: SurvivorRootId) {
    match node {
        Node::HList(box_node) | Node::VList(box_node) => {
            box_node.children = with_root(box_node.children, root);
        }
        Node::Disc { pre, post, replace } => {
            *pre = with_root(*pre, root);
            *post = with_root(*post, root);
            *replace = with_root(*replace, root);
        }
        Node::Ins { content, .. } | Node::Adjust(content) => {
            *content = with_root(*content, root);
        }
        Node::Char { .. }
        | Node::Lig { .. }
        | Node::Kern { .. }
        | Node::Glue { .. }
        | Node::Penalty(_)
        | Node::Rule { .. }
        | Node::Unset
        | Node::Mark { .. }
        | Node::Whatsit(_)
        | Node::MathOn
        | Node::MathOff => {}
    }
}

fn with_root(id: NodeListId, root: SurvivorRootId) -> NodeListId {
    let ArenaRef::Survivor(_) = id.arena() else {
        panic!("promoted child should already be survivor-rooted");
    };
    NodeListId::new_survivor(root, id.start(), id.len())
}

#[cfg(debug_assertions)]
fn debug_assert_no_epoch_ids_in_node(node: &Node) {
    let mut children = Vec::new();
    node.child_lists(&mut children);
    for child in children {
        debug_assert!(
            matches!(child.arena(), ArenaRef::Survivor(_)),
            "promoted survivor root contains epoch node-list id"
        );
    }
}

fn u32_len(value: usize, message: &str) -> u32 {
    match u32::try_from(value) {
        Ok(value) => value,
        Err(_) => panic!("{message}"),
    }
}
