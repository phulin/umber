use std::cell::Cell;

use tex_state::Universe;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam};
use tex_state::glue::GlueSpec;
use tex_state::ids::GlueId;
use tex_state::ids::{FontId, NodeListId};
use tex_state::math::MathListNode;
use tex_state::node::{BoxNode, BoxNodeFields, GlueKind, Node};
use tex_state::scaled::Scaled;
use tex_typeset::TypesetState;
use tex_typeset::math::MathLayoutReader;
use tex_typeset::math::{
    FrozenHList, MathBox, MathGlueKind, MathLayoutSink, MathNode, MathParamState, MathParams,
    MathTypesetState, Style, mlist_to_hlist_with_sink,
};

pub(crate) fn finish_math_list_node(
    stores: &mut Universe,
    list: MathListNode,
    insert_penalties: bool,
) -> Vec<Node> {
    finish_math_list_node_with_reads(stores, list, insert_penalties).0
}

pub(crate) fn finish_inline_math_list_node(
    stores: &mut Universe,
    list: MathListNode,
    insert_penalties: bool,
) -> (Vec<Node>, u64) {
    finish_math_list_node_with_reads(stores, list, insert_penalties)
}

fn finish_math_list_node_with_reads(
    stores: &mut Universe,
    list: MathListNode,
    insert_penalties: bool,
) -> (Vec<Node>, u64) {
    let mut sink = LoweredMathSink::new(stores);
    let params = MathParams::read(&sink);
    let style = if list.display {
        Style::DISPLAY
    } else {
        Style::TEXT
    };
    let hlist = convert_math_hlist_with_sink(
        &mut sink,
        list.content,
        style,
        insert_penalties && !list.display,
        &params,
    );
    let mut nodes = Vec::new();
    if !list.display {
        // AppG rule 22
        let surround = sink.stores.dimen_param(DimenParam::MATH_SURROUND);
        nodes.push(Node::MathOn(surround));
    }
    nodes.extend(hlist);
    if !list.display {
        // AppG rule 22
        let surround = sink.stores.dimen_param(DimenParam::MATH_SURROUND);
        nodes.push(Node::MathOff(surround));
    }
    (nodes, sink.family_mask.get())
}

pub(super) fn convert_math_hlist(
    stores: &mut Universe,
    input: NodeListId,
    style: Style,
    penalties: bool,
    params: &MathParams,
) -> Vec<Node> {
    let mut sink = LoweredMathSink::new(stores);
    convert_math_hlist_with_sink(&mut sink, input, style, penalties, params)
}

fn convert_math_hlist_with_sink(
    sink: &mut LoweredMathSink<'_>,
    input: NodeListId,
    style: Style,
    penalties: bool,
    params: &MathParams,
) -> Vec<Node> {
    let _layout = mlist_to_hlist_with_sink(sink, input, style, penalties, params);
    sink.take_root_nodes()
}

struct LoweredMathSink<'a> {
    stores: &'a mut Universe,
    root_nodes: Vec<Node>,
    glue_cache: Vec<(GlueSpec, GlueId)>,
    family_mask: Cell<u64>,
}

impl<'a> LoweredMathSink<'a> {
    fn new(stores: &'a mut Universe) -> Self {
        Self {
            stores,
            root_nodes: Vec::new(),
            glue_cache: Vec::with_capacity(8),
            family_mask: Cell::new(0),
        }
    }

    fn append_span(
        &mut self,
        list: FrozenHList,
        layout: &dyn MathLayoutReader,
        scratch: &mut Vec<Node>,
    ) {
        enum Task {
            Span(FrozenHList, usize),
            FinishBox {
                boxed: MathBox,
                vertical: bool,
                start: usize,
            },
        }

        let mut tasks = vec![Task::Span(list, 0)];
        while let Some(task) = tasks.pop() {
            let Task::Span(list, index) = task else {
                let Task::FinishBox {
                    boxed,
                    vertical,
                    start,
                } = task
                else {
                    unreachable!()
                };
                let children = self.stores.freeze_node_list(&scratch[start..]);
                scratch.truncate(start);
                let boxed_node = lower_math_box(&boxed, children);
                scratch.push(if vertical {
                    Node::VList(boxed_node)
                } else {
                    Node::HList(boxed_node)
                });
                continue;
            };
            let Some(node) = layout.math_nodes(list).get(index) else {
                continue;
            };
            tasks.push(Task::Span(list, index + 1));
            match node {
                MathNode::Sequence(child) => tasks.push(Task::Span(*child, 0)),
                MathNode::HList(boxed) | MathNode::VList(boxed) => {
                    let start = scratch.len();
                    tasks.push(Task::FinishBox {
                        boxed: boxed.clone(),
                        vertical: matches!(node, MathNode::VList(_)),
                        start,
                    });
                    tasks.push(Task::Span(boxed.list, 0));
                }
                MathNode::Char {
                    font, ch, origin, ..
                } => scratch.push(Node::Char {
                    font: *font,
                    ch: *ch,
                    origin: *origin,
                }),
                MathNode::Kern { amount, kind } => scratch.push(Node::Kern {
                    amount: *amount,
                    kind: *kind,
                }),
                MathNode::Glue { spec, kind, leader } => {
                    let id = if let Some((_, id)) =
                        self.glue_cache.iter().find(|(cached, _)| cached == spec)
                    {
                        *id
                    } else {
                        let id = self.stores.intern_glue(*spec);
                        self.glue_cache.push((*spec, id));
                        id
                    };
                    scratch.push(Node::Glue {
                        spec: id,
                        kind: lower_math_glue_kind(*kind),
                        leader: *leader,
                    });
                }
                MathNode::Penalty(penalty) => scratch.push(Node::Penalty(*penalty)),
                MathNode::Rule {
                    width,
                    height,
                    depth,
                } => scratch.push(Node::Rule {
                    width: *width,
                    height: *height,
                    depth: *depth,
                }),
                MathNode::Opaque(node) => scratch.push(node.as_ref().clone()),
            }
        }
    }

    fn take_root_nodes(&mut self) -> Vec<Node> {
        std::mem::take(&mut self.root_nodes)
    }
}

impl TypesetState for LoweredMathSink<'_> {
    fn nodes(&self, id: NodeListId) -> tex_state::node_arena::NodeList<'_> {
        self.stores.nodes(id)
    }

    fn glue(&self, id: GlueId) -> GlueSpec {
        self.stores.glue(id)
    }

    fn font_char_metrics(&self, font: FontId, code: u8) -> Option<tex_fonts::CharMetrics> {
        self.stores.font_char_metrics(font, code)
    }

    fn font_widths(&self, font: FontId) -> &[Scaled; 256] {
        self.stores.font_widths(font)
    }

    fn font_characters(&self, font: FontId) -> &[Option<tex_fonts::CharMetrics>] {
        self.stores.font_characters(font)
    }
}

impl MathTypesetState for LoweredMathSink<'_> {
    fn math_family_font(&self, size: tex_state::math::MathFontSize, family: u8) -> FontId {
        let index = u32::from(size.index()) * 16 + u32::from(family);
        self.family_mask
            .set(self.family_mask.get() | (1_u64 << index));
        self.stores.math_family_font(size, family)
    }

    fn font_parameter(&self, font: FontId, number: u16) -> Scaled {
        self.stores.font_parameter(font, u32::from(number))
    }

    fn font_next_larger(&self, font: FontId, code: u8) -> Option<u8> {
        self.stores.font_next_larger(font, code)
    }

    fn font_extensible_recipe(
        &self,
        font: FontId,
        code: u8,
    ) -> Option<tex_fonts::metrics::ExtensibleRecipe> {
        self.stores.extensible_recipe(font, code)
    }

    fn lig_kern_command(
        &self,
        font: FontId,
        left: tex_fonts::LigKernChar,
        right: tex_fonts::LigKernChar,
    ) -> Option<tex_fonts::LigKernCommand> {
        self.stores.lig_kern_command(font, left, right)
    }

    fn font_skew_char(&self, font: FontId) -> i32 {
        self.stores.font_skew_char(font)
    }

    fn math_metrics_source(&self, font: FontId) -> tex_fonts::MathMetricsSource<'_> {
        self.stores.font(font).math_metrics_source()
    }
}

impl MathParamState for LoweredMathSink<'_> {
    fn int_param(&self, param: IntParam) -> i32 {
        self.stores.int_param(param)
    }

    fn dimen_param(&self, param: DimenParam) -> Scaled {
        self.stores.dimen_param(param)
    }

    fn glue_param(&self, param: GlueParam) -> GlueId {
        self.stores.glue_param(param)
    }
}

impl MathLayoutSink for LoweredMathSink<'_> {
    fn finish_math_hlist(&mut self, list: FrozenHList, layout: &dyn MathLayoutReader) {
        let mut root = std::mem::take(&mut self.root_nodes);
        root.clear();
        root.reserve(list.node_count());
        self.append_span(list, layout, &mut root);
        self.root_nodes = root;
    }
}

pub(crate) fn finish_math_lists(
    stores: &mut Universe,
    nodes: &[Node],
    insert_penalties: bool,
) -> Vec<Node> {
    let mut out = Vec::with_capacity(nodes.len());
    for node in nodes {
        match node {
            Node::MathList(list) => {
                out.extend(finish_math_list_node(stores, *list, insert_penalties))
            }
            node => out.push(node.clone()),
        }
    }
    out
}

pub(crate) fn finish_math_lists_owned(
    stores: &mut Universe,
    nodes: Vec<Node>,
    insert_penalties: bool,
) -> Vec<Node> {
    if !nodes.iter().any(|node| matches!(node, Node::MathList(_))) {
        return nodes;
    }
    let mut out = Vec::with_capacity(nodes.len());
    for node in nodes {
        match node {
            Node::MathList(list) => {
                out.extend(finish_math_list_node(stores, list, insert_penalties));
            }
            node => out.push(node),
        }
    }
    out
}

fn lower_math_box(boxed: &MathBox, children: tex_state::ids::NodeListId) -> BoxNode {
    BoxNode::new(BoxNodeFields {
        width: boxed.width,
        height: boxed.height,
        depth: boxed.depth,
        shift: boxed.shift,
        display: boxed.display,
        glue_set: boxed.glue_set,
        glue_sign: boxed.glue_sign,
        glue_order: boxed.glue_order,
        children,
    })
}

fn lower_math_glue_kind(kind: MathGlueKind) -> GlueKind {
    match kind {
        MathGlueKind::NonScript => GlueKind::NonScript,
        MathGlueKind::MuSkip => GlueKind::MuSkip,
        MathGlueKind::ThinMuSkip => GlueKind::ThinMuSkip,
        MathGlueKind::MedMuSkip => GlueKind::MedMuSkip,
        MathGlueKind::ThickMuSkip => GlueKind::ThickMuSkip,
        MathGlueKind::Normal => GlueKind::Normal,
        other => other,
    }
}
