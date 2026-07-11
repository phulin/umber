use super::checked_len;
use crate::ids::{GlueId, NodeListId};
use crate::scaled::Scaled;

#[derive(Clone, Debug, Default)]
pub(super) struct BoxTable {
    pub(super) width: Vec<Scaled>,
    pub(super) height: Vec<Scaled>,
    pub(super) depth: Vec<Scaled>,
    pub(super) shift: Vec<Scaled>,
    pub(super) display: Vec<bool>,
    pub(super) glue_set: Vec<crate::scaled::GlueSetRatio>,
    pub(super) glue_sign: Vec<crate::node::Sign>,
    pub(super) glue_order: Vec<crate::glue::Order>,
    pub(super) children: Vec<NodeListId>,
}

impl BoxTable {
    pub(super) fn len(&self) -> usize {
        self.width.len()
    }
    pub(super) fn reserve(&mut self, additional: usize) {
        self.width.reserve(additional);
        self.height.reserve(additional);
        self.depth.reserve(additional);
        self.shift.reserve(additional);
        self.display.reserve(additional);
        self.glue_set.reserve(additional);
        self.glue_sign.reserve(additional);
        self.glue_order.reserve(additional);
        self.children.reserve(additional);
    }
    pub(super) fn push(&mut self, value: crate::node::BoxNode) -> u32 {
        let index = checked_len(self.len(), "box sidecar exceeds u32 entries");
        self.width.push(value.width);
        self.height.push(value.height);
        self.depth.push(value.depth);
        self.shift.push(value.shift);
        self.display.push(value.display);
        self.glue_set.push(value.glue_set);
        self.glue_sign.push(value.glue_sign);
        self.glue_order.push(value.glue_order);
        self.children.push(value.children);
        index
    }
    pub(super) fn replace(&mut self, index: usize, value: crate::node::BoxNode) {
        self.width[index] = value.width;
        self.height[index] = value.height;
        self.depth[index] = value.depth;
        self.shift[index] = value.shift;
        self.display[index] = value.display;
        self.glue_set[index] = value.glue_set;
        self.glue_sign[index] = value.glue_sign;
        self.glue_order[index] = value.glue_order;
        self.children[index] = value.children;
    }
    pub(super) fn truncate(&mut self, len: usize) {
        self.width.truncate(len);
        self.height.truncate(len);
        self.depth.truncate(len);
        self.shift.truncate(len);
        self.display.truncate(len);
        self.glue_set.truncate(len);
        self.glue_sign.truncate(len);
        self.glue_order.truncate(len);
        self.children.truncate(len);
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
    pub(super) fn replace(&mut self, i: usize, v: crate::node::UnsetNode) {
        self.kind[i] = v.kind;
        self.width[i] = v.width;
        self.height[i] = v.height;
        self.depth[i] = v.depth;
        self.span_count[i] = v.span_count;
        self.stretch[i] = v.stretch;
        self.stretch_order[i] = v.stretch_order;
        self.shrink[i] = v.shrink;
        self.shrink_order[i] = v.shrink_order;
        self.children[i] = v.children
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
    pub(super) fn replace(&mut self, i: usize, v: (u16, Scaled, GlueId, Scaled, i32, NodeListId)) {
        self.class[i] = v.0;
        self.size[i] = v.1;
        self.split_top_skip[i] = v.2;
        self.split_max_depth[i] = v.3;
        self.floating_penalty[i] = v.4;
        self.content[i] = v.5
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
    pub(super) fn replace(&mut self, i: usize, v: crate::math::MathNoad) {
        self.kind[i] = v.kind;
        self.nucleus[i] = v.nucleus;
        self.subscript[i] = v.subscript;
        self.superscript[i] = v.superscript
    }
    pub(super) fn truncate(&mut self, n: usize) {
        self.kind.truncate(n);
        self.nucleus.truncate(n);
        self.subscript.truncate(n);
        self.superscript.truncate(n)
    }
}
