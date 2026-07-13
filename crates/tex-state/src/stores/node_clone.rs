//! Iterative child-first cloning from mixed node ownership into the epoch.

use super::Stores;
use crate::ids::{ArenaRef, NodeListId};
use crate::math::MathField;
use crate::node::{LeaderPayload, Node};
use crate::node_arena::ChildPatch;
use std::collections::HashMap;

type CloneMap = HashMap<NodeListId, CloneState, ahash::RandomState>;
const RETAINED_SCRATCH_LIMIT: usize = 4_096;

#[derive(Clone, Copy, Debug)]
enum CloneState {
    Visiting,
    Mapped(NodeListId),
}

#[derive(Clone, Copy, Debug)]
enum CloneTask {
    Enter(NodeListId),
    Exit(NodeListId),
}

#[derive(Debug, Default)]
pub(super) struct EpochCloneScratch {
    states: CloneMap,
    tasks: Vec<CloneTask>,
    children: Vec<NodeListId>,
    patches: Vec<ChildPatch>,
}

impl Stores {
    pub(crate) fn clone_node_list_to_epoch(&mut self, root: NodeListId) -> NodeListId {
        let mut scratch = core::mem::take(&mut self.epoch_clone_scratch);
        debug_assert!(scratch.states.is_empty());
        debug_assert!(scratch.tasks.is_empty());
        debug_assert!(scratch.children.is_empty());
        debug_assert!(scratch.patches.is_empty());
        if matches!(root.arena(), ArenaRef::Survivor(_)) {
            let graph_words = self
                .survivors
                .root_storage_len(root)
                .min(RETAINED_SCRATCH_LIMIT);
            scratch.states.reserve(graph_words);
            scratch.tasks.reserve(graph_words.saturating_mul(2));
            scratch.children.reserve(4);
        }
        scratch.tasks.push(CloneTask::Enter(root));

        while let Some(task) = scratch.tasks.pop() {
            match task {
                CloneTask::Enter(id) => match scratch.states.get(&id) {
                    Some(CloneState::Mapped(_)) => continue,
                    Some(CloneState::Visiting) => panic!("node-list graph contains a cycle"),
                    None => {
                        scratch.states.insert(id, CloneState::Visiting);
                        scratch.children.clear();
                        self.nodes(id).child_lists(&mut scratch.children);
                        scratch.tasks.push(CloneTask::Exit(id));
                        for &child in scratch.children.iter().rev() {
                            scratch.tasks.push(CloneTask::Enter(child));
                        }
                        #[cfg(feature = "node-stats")]
                        crate::measurement::record_epoch_clone(
                            self.nodes(id).len(),
                            matches!(id.arena(), ArenaRef::Epoch).then_some(self.nodes(id).len()),
                        );
                    }
                },
                CloneTask::Exit(id) => {
                    assert!(matches!(
                        scratch.states.get(&id),
                        Some(CloneState::Visiting)
                    ));
                    let mapped = match id.arena() {
                        ArenaRef::Survivor(_) => {
                            let source = self.survivors.get(id);
                            let states = &scratch.states;
                            self.nodes.append_compact_remapped(
                                source,
                                &mut scratch.patches,
                                |child| mapped_child(states, child),
                            )
                        }
                        ArenaRef::Epoch => {
                            // Epoch self-copy cannot hold a borrowed span across
                            // destination growth. Keep this cold mixed-source
                            // case iterative and list-local; survivor roots use
                            // the compact path above without owned nodes.
                            let mut nodes = self.nodes.get_epoch(id).to_vec();
                            for node in &mut nodes {
                                remap_owned_node(node, &scratch.states);
                            }
                            self.nodes.append(&nodes)
                        }
                    };
                    scratch.states.insert(id, CloneState::Mapped(mapped));
                }
            }
        }

        let result = mapped_child(&scratch.states, root);
        scratch.states.clear();
        scratch.tasks.clear();
        scratch.children.clear();
        scratch.patches.clear();
        if scratch.states.capacity() > RETAINED_SCRATCH_LIMIT {
            scratch.states.shrink_to(RETAINED_SCRATCH_LIMIT);
        }
        if scratch.tasks.capacity() > RETAINED_SCRATCH_LIMIT * 2 {
            scratch.tasks.shrink_to(RETAINED_SCRATCH_LIMIT * 2);
        }
        self.epoch_clone_scratch = scratch;
        result
    }
}

fn mapped_child(states: &CloneMap, id: NodeListId) -> NodeListId {
    match states.get(&id) {
        Some(CloneState::Mapped(id)) => *id,
        Some(CloneState::Visiting) => panic!("node-list child is still being visited"),
        None => panic!("node-list child was not scheduled"),
    }
}

fn remap_owned_node(node: &mut Node, states: &CloneMap) {
    let mut remap = |id| mapped_child(states, id);
    match node {
        Node::HList(value) | Node::VList(value) => value.children = remap(value.children),
        Node::Unset(value) => value.children = remap(value.children),
        Node::Disc {
            pre, post, replace, ..
        } => {
            *pre = remap(*pre);
            *post = remap(*post);
            *replace = remap(*replace);
        }
        Node::Ins { content, .. } | Node::Adjust(content) => *content = remap(*content),
        Node::MathNoad(noad) => {
            remap_math_field(&mut noad.nucleus, &mut remap);
            remap_math_field(&mut noad.subscript, &mut remap);
            remap_math_field(&mut noad.superscript, &mut remap);
        }
        Node::FractionNoad(value) => {
            value.numerator = remap(value.numerator);
            value.denominator = remap(value.denominator);
        }
        Node::MathChoice(value) => {
            value.display = remap(value.display);
            value.text = remap(value.text);
            value.script = remap(value.script);
            value.script_script = remap(value.script_script);
        }
        Node::MathList(value) => value.content = remap(value.content),
        Node::Glue {
            leader: Some(LeaderPayload::HList(value) | LeaderPayload::VList(value)),
            ..
        } => value.children = remap(value.children),
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
        | Node::Direction(_)
        | Node::MathStyle(_)
        | Node::Nonscript => {}
    }
}

fn remap_math_field(field: &mut MathField, remap: &mut impl FnMut(NodeListId) -> NodeListId) {
    if let MathField::SubBox(id) | MathField::SubMlist(id) = field {
        *id = remap(*id);
    }
}
