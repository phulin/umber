use super::checked_len;
use crate::ids::{GlueId, NodeListId};
use crate::scaled::Scaled;

#[derive(Clone, Debug, Default)]
pub(super) struct BoxTable {
    pub(super) rows: Vec<crate::node::BoxNode>,
}

impl BoxTable {
    pub(super) fn len(&self) -> usize {
        self.rows.len()
    }
    pub(super) fn reserve(&mut self, additional: usize) {
        self.rows.reserve(additional);
    }
    pub(super) fn push(&mut self, value: crate::node::BoxNode) -> u32 {
        let index = checked_len(self.len(), "box sidecar exceeds u32 entries");
        self.rows.push(value);
        index
    }
    pub(super) fn copy_row(&mut self, source: &Self, index: usize) -> u32 {
        self.push(source.rows[index])
    }
    pub(super) fn truncate(&mut self, len: usize) {
        self.rows.truncate(len);
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct UnsetTable {
    pub(super) kind: Vec<crate::node::UnsetKind>,
    pub(super) width: Vec<Scaled>,
    pub(super) height: Vec<Scaled>,
    pub(super) depth: Vec<Scaled>,
    pub(super) span_count: Vec<u16>,
    pub(super) stretch: Vec<Scaled>,
    pub(super) stretch_order: Vec<crate::glue::Order>,
    pub(super) shrink: Vec<Scaled>,
    pub(super) shrink_order: Vec<crate::glue::Order>,
    pub(super) children: Vec<NodeListId>,
}
impl UnsetTable {
    pub(super) fn len(&self) -> usize {
        self.kind.len()
    }
    pub(super) fn reserve(&mut self, additional: usize) {
        self.kind.reserve(additional);
        self.width.reserve(additional);
        self.height.reserve(additional);
        self.depth.reserve(additional);
        self.span_count.reserve(additional);
        self.stretch.reserve(additional);
        self.stretch_order.reserve(additional);
        self.shrink.reserve(additional);
        self.shrink_order.reserve(additional);
        self.children.reserve(additional);
    }
    pub(super) fn push(&mut self, v: crate::node::UnsetNode) -> u32 {
        let i = checked_len(self.len(), "unset sidecar exceeds u32 entries");
        self.kind.push(v.kind);
        self.width.push(v.width);
        self.height.push(v.height);
        self.depth.push(v.depth);
        self.span_count.push(v.span_count);
        self.stretch.push(v.stretch);
        self.stretch_order.push(v.stretch_order);
        self.shrink.push(v.shrink);
        self.shrink_order.push(v.shrink_order);
        self.children.push(v.children);
        i
    }
    pub(super) fn copy_row(&mut self, source: &Self, index: usize) -> u32 {
        self.push(crate::node::UnsetNode::new(crate::node::UnsetNodeFields {
            kind: source.kind[index],
            width: source.width[index],
            height: source.height[index],
            depth: source.depth[index],
            span_count: source.span_count[index],
            stretch: source.stretch[index],
            stretch_order: source.stretch_order[index],
            shrink: source.shrink[index],
            shrink_order: source.shrink_order[index],
            children: source.children[index],
        }))
    }
    pub(super) fn truncate(&mut self, n: usize) {
        self.kind.truncate(n);
        self.width.truncate(n);
        self.height.truncate(n);
        self.depth.truncate(n);
        self.span_count.truncate(n);
        self.stretch.truncate(n);
        self.stretch_order.truncate(n);
        self.shrink.truncate(n);
        self.shrink_order.truncate(n);
        self.children.truncate(n)
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct InsertionTable {
    pub(super) class: Vec<u16>,
    pub(super) size: Vec<Scaled>,
    pub(super) split_top_skip: Vec<GlueId>,
    pub(super) split_max_depth: Vec<Scaled>,
    pub(super) floating_penalty: Vec<i32>,
    pub(super) content: Vec<NodeListId>,
}
impl InsertionTable {
    pub(super) fn len(&self) -> usize {
        self.class.len()
    }
    pub(super) fn reserve(&mut self, additional: usize) {
        self.class.reserve(additional);
        self.size.reserve(additional);
        self.split_top_skip.reserve(additional);
        self.split_max_depth.reserve(additional);
        self.floating_penalty.reserve(additional);
        self.content.reserve(additional);
    }
    pub(super) fn push(&mut self, v: (u16, Scaled, GlueId, Scaled, i32, NodeListId)) -> u32 {
        let i = checked_len(self.len(), "insertion sidecar exceeds u32 entries");
        self.class.push(v.0);
        self.size.push(v.1);
        self.split_top_skip.push(v.2);
        self.split_max_depth.push(v.3);
        self.floating_penalty.push(v.4);
        self.content.push(v.5);
        i
    }
    pub(super) fn copy_row(&mut self, source: &Self, index: usize) -> u32 {
        self.push((
            source.class[index],
            source.size[index],
            source.split_top_skip[index],
            source.split_max_depth[index],
            source.floating_penalty[index],
            source.content[index],
        ))
    }
    pub(super) fn truncate(&mut self, n: usize) {
        self.class.truncate(n);
        self.size.truncate(n);
        self.split_top_skip.truncate(n);
        self.split_max_depth.truncate(n);
        self.floating_penalty.truncate(n);
        self.content.truncate(n)
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct NoadTable {
    pub(super) kind: Vec<crate::math::NoadKind>,
    pub(super) nucleus: Vec<crate::math::MathField>,
    pub(super) subscript: Vec<crate::math::MathField>,
    pub(super) superscript: Vec<crate::math::MathField>,
}
impl NoadTable {
    pub(super) fn len(&self) -> usize {
        self.kind.len()
    }
    pub(super) fn reserve(&mut self, additional: usize) {
        self.kind.reserve(additional);
        self.nucleus.reserve(additional);
        self.subscript.reserve(additional);
        self.superscript.reserve(additional);
    }
    pub(super) fn push(&mut self, v: crate::math::MathNoad) -> u32 {
        let i = checked_len(self.len(), "noad sidecar exceeds u32 entries");
        self.kind.push(v.kind);
        self.nucleus.push(v.nucleus);
        self.subscript.push(v.subscript);
        self.superscript.push(v.superscript);
        i
    }
    pub(super) fn copy_row(&mut self, source: &Self, index: usize) -> u32 {
        self.push(crate::math::MathNoad {
            kind: source.kind[index].clone(),
            nucleus: source.nucleus[index].clone(),
            subscript: source.subscript[index].clone(),
            superscript: source.superscript[index].clone(),
        })
    }
    pub(super) fn truncate(&mut self, n: usize) {
        self.kind.truncate(n);
        self.nucleus.truncate(n);
        self.subscript.truncate(n);
        self.superscript.truncate(n)
    }
}
