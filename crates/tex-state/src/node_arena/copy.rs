//! Private compact-to-compact copying and typed child patches.

use super::storage::{NodeStorage, NodeWord, SidecarNeeds};
use super::view::NodeList;
use super::{checked_len, preflight_capacity};
use crate::ids::NodeListId;
use crate::math::MathField;

/// One destination sidecar row whose child handles still name the source
/// graph. The row is never published until every handle has been remapped.
#[derive(Clone, Debug)]
pub(crate) enum ChildPatch {
    Box {
        row: usize,
        child: NodeListId,
    },
    Unset {
        row: usize,
        child: NodeListId,
    },
    Leader {
        row: usize,
        child: NodeListId,
    },
    Disc {
        row: usize,
        children: [NodeListId; 3],
    },
    Insertion {
        row: usize,
        child: NodeListId,
    },
    Noad {
        row: usize,
        children: [Option<NodeListId>; 3],
    },
    Fraction {
        row: usize,
        children: [NodeListId; 2],
    },
    Choice {
        row: usize,
        children: [NodeListId; 4],
    },
    MathList {
        row: usize,
        child: NodeListId,
    },
    Adjust {
        row: usize,
        child: NodeListId,
    },
}

impl ChildPatch {
    pub(crate) fn remap(mut self, mut map: impl FnMut(NodeListId) -> NodeListId) -> Self {
        match &mut self {
            Self::Box { child, .. }
            | Self::Unset { child, .. }
            | Self::Leader { child, .. }
            | Self::Insertion { child, .. }
            | Self::MathList { child, .. }
            | Self::Adjust { child, .. } => *child = map(*child),
            Self::Disc { children, .. } => children.iter_mut().for_each(|id| *id = map(*id)),
            Self::Noad { children, .. } => {
                for child in children.iter_mut().flatten() {
                    *child = map(*child);
                }
            }
            Self::Fraction { children, .. } => {
                children.iter_mut().for_each(|id| *id = map(*id));
            }
            Self::Choice { children, .. } => {
                children.iter_mut().for_each(|id| *id = map(*id));
            }
        }
        self
    }
}

impl NodeStorage {
    /// Appends one source span without decoding it to owned `Node` values.
    /// Child-bearing rows are copied shallowly and recorded for later patching.
    pub(crate) fn append_compact(
        &mut self,
        source: NodeList<'_>,
        pending: &mut Vec<ChildPatch>,
    ) -> (u32, u32) {
        #[cfg(feature = "node-stats")]
        let capacity_before = self.capacity_signature();
        #[cfg(feature = "node-stats")]
        let retained_before = self.retained_payload_bytes();

        let start = checked_len(self.words.len(), "node arena exceeds u32 entries");
        let len = checked_len(source.len(), "node list exceeds u32 entries");
        preflight_capacity(start, len, "node arena span overflows u32");

        let source_words = &source.storage.words[source.start..source.end];
        let mut needs = SidecarNeeds::default();
        for word in source_words {
            count_sidecar(word.tag(), &mut needs);
        }
        for (have, add) in self.sidecar_lengths().into_iter().zip(needs.as_array()) {
            preflight_capacity(have, add, "node sidecar exceeds u32 entries");
        }
        self.words.reserve(source_words.len());
        self.reserve_sidecars(needs);

        for &word in source_words {
            let side = word.payload() as usize;
            let copied = match word.tag() {
                0..=8 => word,
                9 | 10 => {
                    let row = self.boxes.copy_row(&source.storage.boxes, side);
                    pending.push(ChildPatch::Box {
                        row: row as usize,
                        child: source.storage.boxes.children[side],
                    });
                    NodeWord::sidecar(word.tag(), row)
                }
                11 => {
                    let row = self.unsets.copy_row(&source.storage.unsets, side);
                    pending.push(ChildPatch::Unset {
                        row: row as usize,
                        child: source.storage.unsets.children[side],
                    });
                    NodeWord::sidecar(11, row)
                }
                12 => copy_vec_row(12, &mut self.rules, &source.storage.rules, side),
                13 => {
                    let row = copy_vec_row(13, &mut self.leaders, &source.storage.leaders, side);
                    if let Some(child) = leader_child(&self.leaders[row.payload() as usize].2) {
                        pending.push(ChildPatch::Leader {
                            row: row.payload() as usize,
                            child,
                        });
                    }
                    row
                }
                14 => {
                    let row = copy_vec_row(14, &mut self.discs, &source.storage.discs, side);
                    let index = row.payload() as usize;
                    let (_, pre, post, replace) = self.discs[index];
                    pending.push(ChildPatch::Disc {
                        row: index,
                        children: [pre, post, replace],
                    });
                    row
                }
                15 => copy_vec_row(15, &mut self.marks, &source.storage.marks, side),
                16 => {
                    let row = self.insertions.copy_row(&source.storage.insertions, side);
                    pending.push(ChildPatch::Insertion {
                        row: row as usize,
                        child: source.storage.insertions.content[side],
                    });
                    NodeWord::sidecar(16, row)
                }
                17 => copy_vec_row(17, &mut self.whatsits, &source.storage.whatsits, side),
                18 => {
                    let row = self.noads.copy_row(&source.storage.noads, side);
                    let index = row as usize;
                    let children = [
                        math_field_child(&self.noads.nucleus[index]),
                        math_field_child(&self.noads.subscript[index]),
                        math_field_child(&self.noads.superscript[index]),
                    ];
                    if children.iter().any(Option::is_some) {
                        pending.push(ChildPatch::Noad {
                            row: index,
                            children,
                        });
                    }
                    NodeWord::sidecar(18, row)
                }
                19 => {
                    let row =
                        copy_vec_row(19, &mut self.fractions, &source.storage.fractions, side);
                    let index = row.payload() as usize;
                    let fraction = &self.fractions[index];
                    pending.push(ChildPatch::Fraction {
                        row: index,
                        children: [fraction.numerator, fraction.denominator],
                    });
                    row
                }
                20 => {
                    let row = copy_vec_row(20, &mut self.choices, &source.storage.choices, side);
                    let index = row.payload() as usize;
                    let choice = &self.choices[index];
                    pending.push(ChildPatch::Choice {
                        row: index,
                        children: [
                            choice.display,
                            choice.text,
                            choice.script,
                            choice.script_script,
                        ],
                    });
                    row
                }
                21 => {
                    let row =
                        copy_vec_row(21, &mut self.math_lists, &source.storage.math_lists, side);
                    let index = row.payload() as usize;
                    pending.push(ChildPatch::MathList {
                        row: index,
                        child: self.math_lists[index].content,
                    });
                    row
                }
                22 => {
                    let row = copy_vec_row(22, &mut self.adjusts, &source.storage.adjusts, side);
                    let index = row.payload() as usize;
                    pending.push(ChildPatch::Adjust {
                        row: index,
                        child: self.adjusts[index],
                    });
                    row
                }
                _ => panic!("reserved node-word tag"),
            };
            self.words.push(copied);
        }

        #[cfg(feature = "node-stats")]
        {
            let capacity_after = self.capacity_signature();
            let growth_events = capacity_before
                .iter()
                .zip(capacity_after)
                .filter(|(before, after)| **before != *after)
                .count();
            crate::measurement::record_node_append(
                source_words.len(),
                needs.as_array(),
                growth_events,
                self.retained_payload_bytes()
                    .saturating_sub(retained_before),
            );
            self.record_peak();
        }
        (start, len)
    }
}

fn copy_vec_row<T: Clone>(
    tag: u8,
    destination: &mut Vec<T>,
    source: &[T],
    index: usize,
) -> NodeWord {
    let row = checked_len(destination.len(), "node sidecar exceeds u32 entries");
    destination.push(source[index].clone());
    NodeWord::sidecar(tag, row)
}

fn math_field_child(field: &MathField) -> Option<NodeListId> {
    match field {
        MathField::SubBox(id) | MathField::SubMlist(id) => Some(*id),
        MathField::Empty | MathField::MathChar(_) | MathField::MathTextChar(_) => None,
    }
}

fn leader_child(payload: &crate::node::LeaderPayload) -> Option<NodeListId> {
    match payload {
        crate::node::LeaderPayload::HList(value) | crate::node::LeaderPayload::VList(value) => {
            Some(value.children)
        }
        crate::node::LeaderPayload::Rule { .. } => None,
    }
}

fn count_sidecar(tag: u8, needs: &mut SidecarNeeds) {
    let target = match tag {
        0..=8 => None,
        9 | 10 => Some(&mut needs.boxes),
        11 => Some(&mut needs.unsets),
        12 => Some(&mut needs.rules),
        13 => Some(&mut needs.leaders),
        14 => Some(&mut needs.discs),
        15 => Some(&mut needs.marks),
        16 => Some(&mut needs.insertions),
        17 => Some(&mut needs.whatsits),
        18 => Some(&mut needs.noads),
        19 => Some(&mut needs.fractions),
        20 => Some(&mut needs.choices),
        21 => Some(&mut needs.math_lists),
        22 => Some(&mut needs.adjusts),
        _ => panic!("reserved node-word tag"),
    };
    if let Some(target) = target {
        *target = target.checked_add(1).expect("sidecar count overflow");
    }
}
