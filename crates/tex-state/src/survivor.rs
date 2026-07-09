//! Survivor arena for node lists that escape an epoch.
//!
//! Promotion copies an epoch-rooted node graph into one contiguous allocation
//! and rewrites child spans to be relative to the survivor root.

use crate::ids::{ArenaRef, NodeListId, SurvivorRootId};
use crate::math::MathField;
use crate::node::{LeaderPayload, Node};
use crate::node_arena::NodeArena;

/// Arena for promoted node-list roots.
#[derive(Clone, Debug)]
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
    pub(crate) fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }

    /// Promotes an epoch list into one survivor root with refcount 1.
    pub(crate) fn promote(&mut self, id: NodeListId, epoch: &NodeArena) -> NodeListId {
        assert!(
            matches!(id.arena(), ArenaRef::Epoch),
            "only epoch node lists are promoted"
        );

        let (nodes, start, len) = copy_list_iterative(id, epoch);
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

    /// Decrements the root refcount for a survivor list, freeing the root at zero.
    pub(crate) fn dec_ref(&mut self, id: NodeListId) {
        let ArenaRef::Survivor(root) = id.arena() else {
            panic!("only survivor node-list ids are refcounted");
        };
        let slot = self.root_mut(root);
        slot.refcount = slot
            .refcount
            .checked_sub(1)
            .expect("survivor root refcount underflow");
        if slot.refcount == 0 {
            self.slots[root.raw() as usize] = None;
            self.free.push(root);
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
        let index = root.raw() as usize;
        assert!(index < self.slots.len(), "survivor root is not live");
        self.slots[index]
            .as_mut()
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

fn copy_list_iterative(id: NodeListId, epoch: &NodeArena) -> (Vec<Node>, u32, u32) {
    let mut out = Vec::new();
    let (root_start, root_len) = append_list(id, epoch, &mut out);
    let mut pending: Vec<usize> = (root_start as usize..root_start as usize + root_len as usize)
        .rev()
        .collect();

    while let Some(index) = pending.pop() {
        remap_node_children(index, epoch, &mut out, &mut pending);
    }

    (out, root_start, root_len)
}

fn append_list(id: NodeListId, epoch: &NodeArena, out: &mut Vec<Node>) -> (u32, u32) {
    let start = u32_len(out.len(), "promoted node root exceeds u32 entries");
    out.extend_from_slice(epoch.get_epoch(id));
    let len = id.len();
    (start, len)
}

fn queue_children(start: u32, len: u32, pending: &mut Vec<usize>) {
    pending.extend((start as usize..start as usize + len as usize).rev());
}

fn remap_node_children(
    index: usize,
    epoch: &NodeArena,
    out: &mut Vec<Node>,
    pending: &mut Vec<usize>,
) {
    match out[index].clone() {
        Node::HList(mut box_node) => {
            let (start, len) = append_list(box_node.children, epoch, out);
            queue_children(start, len, pending);
            box_node.children = NodeListId::new_survivor(SurvivorRootId::new(0), start, len);
            out[index] = Node::HList(box_node);
        }
        Node::VList(mut box_node) => {
            let (start, len) = append_list(box_node.children, epoch, out);
            queue_children(start, len, pending);
            box_node.children = NodeListId::new_survivor(SurvivorRootId::new(0), start, len);
            out[index] = Node::VList(box_node);
        }
        Node::Disc {
            kind,
            pre,
            post,
            replace,
        } => {
            let (pre_start, pre_len) = append_list(pre, epoch, out);
            queue_children(pre_start, pre_len, pending);
            let (post_start, post_len) = append_list(post, epoch, out);
            queue_children(post_start, post_len, pending);
            let (replace_start, replace_len) = append_list(replace, epoch, out);
            queue_children(replace_start, replace_len, pending);
            out[index] = Node::Disc {
                kind,
                pre: NodeListId::new_survivor(SurvivorRootId::new(0), pre_start, pre_len),
                post: NodeListId::new_survivor(SurvivorRootId::new(0), post_start, post_len),
                replace: NodeListId::new_survivor(
                    SurvivorRootId::new(0),
                    replace_start,
                    replace_len,
                ),
            };
        }
        Node::Ins {
            class,
            size,
            split_top_skip,
            split_max_depth,
            floating_penalty,
            content,
        } => {
            let (start, len) = append_list(content, epoch, out);
            queue_children(start, len, pending);
            out[index] = Node::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content: NodeListId::new_survivor(SurvivorRootId::new(0), start, len),
            };
        }
        Node::Adjust(content) => {
            let (start, len) = append_list(content, epoch, out);
            queue_children(start, len, pending);
            out[index] = Node::Adjust(NodeListId::new_survivor(SurvivorRootId::new(0), start, len));
        }
        Node::MathNoad(mut noad) => {
            remap_math_field(&mut noad.nucleus, epoch, out, pending);
            remap_math_field(&mut noad.subscript, epoch, out, pending);
            remap_math_field(&mut noad.superscript, epoch, out, pending);
            out[index] = Node::MathNoad(noad);
        }
        Node::FractionNoad(mut fraction) => {
            fraction.numerator = remap_list(fraction.numerator, epoch, out, pending);
            fraction.denominator = remap_list(fraction.denominator, epoch, out, pending);
            out[index] = Node::FractionNoad(fraction);
        }
        Node::MathChoice(mut choice) => {
            choice.display = remap_list(choice.display, epoch, out, pending);
            choice.text = remap_list(choice.text, epoch, out, pending);
            choice.script = remap_list(choice.script, epoch, out, pending);
            choice.script_script = remap_list(choice.script_script, epoch, out, pending);
            out[index] = Node::MathChoice(choice);
        }
        Node::MathList(mut list) => {
            list.content = remap_list(list.content, epoch, out, pending);
            out[index] = Node::MathList(list);
        }
        Node::Glue {
            spec,
            kind,
            leader: Some(payload),
        } => {
            out[index] = Node::Glue {
                spec,
                kind,
                leader: Some(remap_leader_payload(payload, epoch, out, pending)),
            };
        }
        Node::Unset(mut unset) => {
            unset.children = remap_list(unset.children, epoch, out, pending);
            out[index] = Node::Unset(unset);
        }
        Node::Char { .. }
        | Node::Lig { .. }
        | Node::Kern { .. }
        | Node::Glue { .. }
        | Node::Penalty(_)
        | Node::Rule { .. }
        | Node::Mark { .. }
        | Node::Whatsit(_)
        | Node::MathOn(_)
        | Node::MathOff(_)
        | Node::MathStyle(_)
        | Node::Nonscript => {}
    }
}

fn rewrite_node_root_ids(node: &mut Node, root: SurvivorRootId) {
    match node {
        Node::HList(box_node) | Node::VList(box_node) => {
            box_node.children = with_root(box_node.children, root);
        }
        Node::Disc {
            pre, post, replace, ..
        } => {
            *pre = with_root(*pre, root);
            *post = with_root(*post, root);
            *replace = with_root(*replace, root);
        }
        Node::Ins { content, .. } | Node::Adjust(content) => {
            *content = with_root(*content, root);
        }
        Node::MathNoad(noad) => {
            rewrite_math_field_root(&mut noad.nucleus, root);
            rewrite_math_field_root(&mut noad.subscript, root);
            rewrite_math_field_root(&mut noad.superscript, root);
        }
        Node::FractionNoad(fraction) => {
            fraction.numerator = with_root(fraction.numerator, root);
            fraction.denominator = with_root(fraction.denominator, root);
        }
        Node::MathChoice(choice) => {
            choice.display = with_root(choice.display, root);
            choice.text = with_root(choice.text, root);
            choice.script = with_root(choice.script, root);
            choice.script_script = with_root(choice.script_script, root);
        }
        Node::MathList(list) => {
            list.content = with_root(list.content, root);
        }
        Node::Glue {
            leader: Some(payload),
            ..
        } => rewrite_leader_payload_root(payload, root),
        Node::Unset(unset) => {
            unset.children = with_root(unset.children, root);
        }
        Node::Char { .. }
        | Node::Lig { .. }
        | Node::Kern { .. }
        | Node::Glue { .. }
        | Node::Penalty(_)
        | Node::Rule { .. }
        | Node::Mark { .. }
        | Node::Whatsit(_)
        | Node::MathOn(_)
        | Node::MathOff(_)
        | Node::MathStyle(_)
        | Node::Nonscript => {}
    }
}

fn remap_list(
    id: NodeListId,
    epoch: &NodeArena,
    out: &mut Vec<Node>,
    pending: &mut Vec<usize>,
) -> NodeListId {
    let (start, len) = append_list(id, epoch, out);
    queue_children(start, len, pending);
    NodeListId::new_survivor(SurvivorRootId::new(0), start, len)
}

fn remap_math_field(
    field: &mut MathField,
    epoch: &NodeArena,
    out: &mut Vec<Node>,
    pending: &mut Vec<usize>,
) {
    if let MathField::SubBox(list) | MathField::SubMlist(list) = field {
        *list = remap_list(*list, epoch, out, pending);
    }
}

fn remap_leader_payload(
    payload: LeaderPayload,
    epoch: &NodeArena,
    out: &mut Vec<Node>,
    pending: &mut Vec<usize>,
) -> LeaderPayload {
    match payload {
        LeaderPayload::HList(mut box_node) => {
            box_node.children = remap_list(box_node.children, epoch, out, pending);
            LeaderPayload::HList(box_node)
        }
        LeaderPayload::VList(mut box_node) => {
            box_node.children = remap_list(box_node.children, epoch, out, pending);
            LeaderPayload::VList(box_node)
        }
        LeaderPayload::Rule {
            width,
            height,
            depth,
        } => LeaderPayload::Rule {
            width,
            height,
            depth,
        },
    }
}

fn rewrite_leader_payload_root(payload: &mut LeaderPayload, root: SurvivorRootId) {
    match payload {
        LeaderPayload::HList(box_node) | LeaderPayload::VList(box_node) => {
            box_node.children = with_root(box_node.children, root);
        }
        LeaderPayload::Rule { .. } => {}
    }
}

fn rewrite_math_field_root(field: &mut MathField, root: SurvivorRootId) {
    if let MathField::SubBox(list) | MathField::SubMlist(list) = field {
        *list = with_root(*list, root);
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
