//! Survivor arena for node lists that escape an epoch.
//!
//! Promotion copies an epoch-rooted node graph into one contiguous allocation
//! and rewrites child spans to be relative to the survivor root.

use crate::ids::{ArenaRef, NodeListId, SurvivorRootId};
use crate::math::MathField;
use crate::node::{LeaderPayload, Node};
use crate::node_arena::NodeArena;
use std::collections::HashMap;

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

        let (nodes, start, len) = copy_list_iterative(id, epoch, self);
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

fn copy_list_iterative(
    id: NodeListId,
    epoch: &NodeArena,
    survivor: &SurvivorArena,
) -> (Vec<Node>, u32, u32) {
    let mut copy = PromotionCopy::new(epoch, survivor);
    let root = copy.copy_list(id);

    while let Some(index) = copy.pending.pop() {
        remap_node_children(index, &mut copy);
    }

    (copy.out, root.start(), root.len())
}

/// Copies a mixed epoch/survivor DAG into one canonical survivor allocation.
///
/// The source arena is selected from the opaque handle's ownership tag inside
/// the arena implementation. Exact list handles are memoized before their
/// children are traversed, so shared children are copied and remapped once.
struct PromotionCopy<'a> {
    epoch: &'a NodeArena,
    survivor: &'a SurvivorArena,
    out: Vec<Node>,
    remapped: HashMap<NodeListId, NodeListId>,
    pending: Vec<usize>,
}

impl<'a> PromotionCopy<'a> {
    fn new(epoch: &'a NodeArena, survivor: &'a SurvivorArena) -> Self {
        Self {
            epoch,
            survivor,
            out: Vec::new(),
            remapped: HashMap::new(),
            pending: Vec::new(),
        }
    }

    fn copy_list(&mut self, id: NodeListId) -> NodeListId {
        if let Some(remapped) = self.remapped.get(&id) {
            return *remapped;
        }

        let nodes = match id.arena() {
            ArenaRef::Epoch => self.epoch.get_epoch(id),
            ArenaRef::Survivor(_) => self.survivor.get(id),
        }
        .to_vec();
        let start = u32_len(self.out.len(), "promoted node root exceeds u32 entries");
        let len = id.len();
        self.out.extend(nodes);
        let remapped = NodeListId::new_survivor(SurvivorRootId::new(0), start, len);
        self.remapped.insert(id, remapped);
        self.pending
            .extend((start as usize..start as usize + len as usize).rev());
        remapped
    }
}

fn remap_node_children(index: usize, copy: &mut PromotionCopy<'_>) {
    match copy.out[index].clone() {
        Node::HList(mut box_node) => {
            box_node.children = copy.copy_list(box_node.children);
            copy.out[index] = Node::HList(box_node);
        }
        Node::VList(mut box_node) => {
            box_node.children = copy.copy_list(box_node.children);
            copy.out[index] = Node::VList(box_node);
        }
        Node::Disc {
            kind,
            pre,
            post,
            replace,
        } => {
            copy.out[index] = Node::Disc {
                kind,
                pre: copy.copy_list(pre),
                post: copy.copy_list(post),
                replace: copy.copy_list(replace),
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
            copy.out[index] = Node::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content: copy.copy_list(content),
            };
        }
        Node::Adjust(content) => {
            copy.out[index] = Node::Adjust(copy.copy_list(content));
        }
        Node::MathNoad(mut noad) => {
            remap_math_field(&mut noad.nucleus, copy);
            remap_math_field(&mut noad.subscript, copy);
            remap_math_field(&mut noad.superscript, copy);
            copy.out[index] = Node::MathNoad(noad);
        }
        Node::FractionNoad(mut fraction) => {
            fraction.numerator = copy.copy_list(fraction.numerator);
            fraction.denominator = copy.copy_list(fraction.denominator);
            copy.out[index] = Node::FractionNoad(fraction);
        }
        Node::MathChoice(mut choice) => {
            choice.display = copy.copy_list(choice.display);
            choice.text = copy.copy_list(choice.text);
            choice.script = copy.copy_list(choice.script);
            choice.script_script = copy.copy_list(choice.script_script);
            copy.out[index] = Node::MathChoice(choice);
        }
        Node::MathList(mut list) => {
            list.content = copy.copy_list(list.content);
            copy.out[index] = Node::MathList(list);
        }
        Node::Glue {
            spec,
            kind,
            leader: Some(payload),
        } => {
            copy.out[index] = Node::Glue {
                spec,
                kind,
                leader: Some(remap_leader_payload(payload, copy)),
            };
        }
        Node::Unset(mut unset) => {
            unset.children = copy.copy_list(unset.children);
            copy.out[index] = Node::Unset(unset);
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

fn remap_math_field(field: &mut MathField, copy: &mut PromotionCopy<'_>) {
    if let MathField::SubBox(list) | MathField::SubMlist(list) = field {
        *list = copy.copy_list(*list);
    }
}

fn remap_leader_payload(payload: LeaderPayload, copy: &mut PromotionCopy<'_>) -> LeaderPayload {
    match payload {
        LeaderPayload::HList(mut box_node) => {
            box_node.children = copy.copy_list(box_node.children);
            LeaderPayload::HList(box_node)
        }
        LeaderPayload::VList(mut box_node) => {
            box_node.children = copy.copy_list(box_node.children);
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
