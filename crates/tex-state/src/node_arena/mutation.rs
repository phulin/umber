use super::storage::NodeStorage;
use crate::node::Node;

impl NodeStorage {
    pub(crate) fn replace_node(&mut self, index: usize, node: Node) {
        // Survivor remapping changes handles but not table shape. Replace the
        // corresponding sidecar row and word through the aggregate storage.
        let old = self.words[index];
        let side = old.payload() as usize;
        match old.tag() {
            9 | 10 => {
                if let Node::HList(v) | Node::VList(v) = node {
                    self.boxes.replace(side, v)
                } else {
                    unreachable!()
                }
            }
            11 => {
                if let Node::Unset(v) = node {
                    self.unsets.replace(side, v)
                } else {
                    unreachable!()
                }
            }
            13 => {
                if let Node::Glue {
                    spec,
                    kind,
                    leader: Some(v),
                } = node
                {
                    self.leaders[side] = (spec, kind, v)
                } else {
                    unreachable!()
                }
            }
            14 => {
                if let Node::Disc {
                    kind,
                    pre,
                    post,
                    replace,
                } = node
                {
                    self.discs[side] = (kind, pre, post, replace)
                } else {
                    unreachable!()
                }
            }
            16 => {
                if let Node::Ins {
                    class,
                    size,
                    split_top_skip,
                    split_max_depth,
                    floating_penalty,
                    content,
                } = node
                {
                    self.insertions.replace(
                        side,
                        (
                            class,
                            size,
                            split_top_skip,
                            split_max_depth,
                            floating_penalty,
                            content,
                        ),
                    )
                } else {
                    unreachable!()
                }
            }
            18 => {
                if let Node::MathNoad(v) = node {
                    self.noads.replace(side, v)
                } else {
                    unreachable!()
                }
            }
            19 => {
                if let Node::FractionNoad(v) = node {
                    self.fractions[side] = v
                } else {
                    unreachable!()
                }
            }
            20 => {
                if let Node::MathChoice(v) = node {
                    self.choices[side] = v
                } else {
                    unreachable!()
                }
            }
            21 => {
                if let Node::MathList(v) = node {
                    self.math_lists[side] = v
                } else {
                    unreachable!()
                }
            }
            22 => {
                if let Node::Adjust(v) = node {
                    self.adjusts[side] = v
                } else {
                    unreachable!()
                }
            }
            _ => {}
        }
    }
}
