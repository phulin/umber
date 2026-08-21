use std::cell::Cell;

use tex_state::Universe;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam};
use tex_state::glue::GlueSpec;
use tex_state::ids::FontId;
use tex_state::math::MathListNode;
use tex_state::node::{BoxNode, BoxNodeFields, GlueKind, Node};
use tex_state::node_arena::PageListId;
use tex_state::scaled::Scaled;
use tex_typeset::TypesetState;
use tex_typeset::math::{
    FrozenHList, MathBox, MathConversionEvent, MathGlueKind, MathLayout, MathNode, MathParamState,
    MathParams, MathTypesetState, Style, mlist_to_hlist,
};

/// Detached TeX82 §82 input context for an error raised while converting
/// an already-complete math list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MathConversionErrorContext(String);

impl MathConversionErrorContext {
    pub(crate) fn new(rendered: String) -> Self {
        Self(rendered)
    }
}

pub(crate) fn finish_math_list_node<G>(
    stores: &mut Universe<G>,
    list: MathListNode,
    insert_penalties: bool,
) -> Vec<Node> {
    finish_math_list_node_with_reads(stores, list, insert_penalties, None).0
}

pub(crate) fn finish_inline_math_list_node<G>(
    stores: &mut Universe<G>,
    list: MathListNode,
    insert_penalties: bool,
    error_context: MathConversionErrorContext,
) -> (Vec<Node>, u64) {
    finish_math_list_node_with_reads(stores, list, insert_penalties, Some(&error_context))
}

fn finish_math_list_node_with_reads<G>(
    stores: &mut Universe<G>,
    list: MathListNode,
    insert_penalties: bool,
    error_context: Option<&MathConversionErrorContext>,
) -> (Vec<Node>, u64) {
    let mut sink = LoweredMathSink::new(stores, error_context);
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
    let family_mask = sink.family_mask.get();
    for index in 0..48 {
        if family_mask & (1_u64 << index) != 0 {
            sink.stores
                .observe_semantic_dependency(tex_state::DependencyKey::Cell(
                    tex_state::cell::CellId::new(tex_state::cell::BankTag::MathFamilyFont, index),
                ));
        }
    }
    let mut nodes = Vec::new();
    if !list.display {
        // AppG rule 22
        sink.stores
            .observe_semantic_dependency(tex_state::DependencyKey::Cell(
                tex_state::cell::CellId::new(
                    tex_state::cell::BankTag::DimenParam,
                    u32::from(DimenParam::MATH_SURROUND.raw()),
                ),
            ));
        let surround = sink.stores.dimen_param(DimenParam::MATH_SURROUND);
        nodes.push(Node::MathOn(surround));
    }
    nodes.extend(hlist);
    if !list.display {
        // AppG rule 22
        let surround = sink.stores.dimen_param(DimenParam::MATH_SURROUND);
        nodes.push(Node::MathOff(surround));
    }
    (nodes, family_mask)
}

pub(super) fn convert_math_hlist_with_error_context<G>(
    stores: &mut Universe<G>,
    input: PageListId,
    style: Style,
    penalties: bool,
    params: &MathParams,
    error_context: Option<&MathConversionErrorContext>,
) -> Vec<Node> {
    let mut sink = LoweredMathSink::new(stores, error_context);
    convert_math_hlist_with_sink(&mut sink, input, style, penalties, params)
}

fn convert_math_hlist_with_sink<G>(
    sink: &mut LoweredMathSink<'_, G>,
    input: PageListId,
    style: Style,
    penalties: bool,
    params: &MathParams,
) -> Vec<Node> {
    let transaction = mlist_to_hlist(&*sink, input, style, penalties, params);
    sink.commit_math_transaction(&transaction);
    sink.take_root_nodes()
}

struct LoweredMathSink<'a, G> {
    stores: &'a mut Universe<G>,
    error_context: Option<&'a MathConversionErrorContext>,
    root_nodes: Vec<Node>,
    glue_cache: Vec<(GlueSpec, GlueSpec)>,
    family_mask: Cell<u64>,
}

impl<'a, G> LoweredMathSink<'a, G> {
    fn new(
        stores: &'a mut Universe<G>,
        error_context: Option<&'a MathConversionErrorContext>,
    ) -> Self {
        Self {
            stores,
            error_context,
            root_nodes: Vec::new(),
            glue_cache: Vec::with_capacity(8),
            family_mask: Cell::new(0),
        }
    }

    fn append_span(&mut self, list: FrozenHList, layout: &MathLayout, scratch: &mut Vec<Node>) {
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
                let children = self.stores.publish_page_nodes(&scratch[start..]);
                scratch.truncate(start);
                let boxed_node = lower_math_box(&boxed, children);
                scratch.push(if vertical {
                    Node::VList(boxed_node)
                } else {
                    Node::HList(boxed_node)
                });
                continue;
            };
            let Some(node) = layout.nodes(list).get(index) else {
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
                    origin: origin.clone(),
                }),
                MathNode::Kern { amount, kind } => scratch.push(Node::Kern {
                    amount: *amount,
                    kind: *kind,
                }),
                MathNode::Glue { spec, kind, leader } => {
                    let value = if let Some((_, value)) =
                        self.glue_cache.iter().find(|(cached, _)| cached == spec)
                    {
                        *value
                    } else {
                        self.glue_cache.push((*spec, *spec));
                        *spec
                    };
                    scratch.push(Node::Glue {
                        spec: value,
                        kind: lower_math_glue_kind(*kind),
                        leader: leader.clone(),
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
                MathNode::Native(node) => scratch.push(node.as_ref().clone()),
            }
        }
    }

    fn take_root_nodes(&mut self) -> Vec<Node> {
        std::mem::take(&mut self.root_nodes)
    }
}

impl<G> TypesetState for LoweredMathSink<'_, G> {
    fn page_nodes(&self, list: PageListId) -> &[Node] {
        self.stores
            .page_node_list(list)
            .expect("math list belongs to the admitted page arena")
            .nodes()
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

impl<G> MathTypesetState for LoweredMathSink<'_, G> {
    fn math_family_font(&self, size: tex_state::math::MathFontSize, family: u8) -> FontId {
        let index = u32::from(size.index()) * 16 + u32::from(family);
        self.family_mask
            .set(self.family_mask.get() | (1_u64 << index));
        self.stores.math_family_font(size, family)
    }

    fn font_parameter(&self, font: FontId, number: u16) -> Scaled {
        self.stores.classic_math_parameter(font, number)
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

impl<G> MathParamState for LoweredMathSink<'_, G> {
    fn int_param(&self, param: IntParam) -> i32 {
        self.stores.int_param(param)
    }

    fn dimen_param(&self, param: DimenParam) -> Scaled {
        self.stores.dimen_param(param)
    }

    fn glue_param(&self, param: GlueParam) -> GlueSpec {
        self.stores.glue_param(param)
    }
}

impl<G> LoweredMathSink<'_, G> {
    fn commit_math_transaction(&mut self, layout: &MathLayout) {
        let list = layout.root();
        for event in layout.conversion_events() {
            match *event {
                MathConversionEvent::MissingCharacter { font, character } => {
                    if self.stores.int_param(IntParam::TRACING_LOST_CHARS) > 0 {
                        // TeX82 §581's `char_warning` prints the stored
                        // external name, not `\fontname`'s size-qualified
                        // rendering.
                        let font_name = self.stores.font(font).name().to_owned();
                        let mut diagnostic = self.stores.begin_diagnostic();
                        diagnostic
                            .print_nl("Missing character: There is no ")
                            .print(&character.to_string())
                            .print(" in font ")
                            .print(&font_name)
                            .print_char('!');
                        diagnostic.end(false);
                    }
                }
                MathConversionEvent::UndefinedFamily {
                    size,
                    family,
                    character,
                } => {
                    let size = match size {
                        tex_state::math::MathFontSize::Text => "\\textfont",
                        tex_state::math::MathFontSize::Script => "\\scriptfont",
                        tex_state::math::MathFontSize::ScriptScript => "\\scriptscriptfont",
                    };
                    let mut report = self.stores.print_err("");
                    report
                        .print(size)
                        .print_char(' ')
                        .print_int(i32::from(family))
                        .print(" is undefined (character ")
                        // TeX82 §721 calls `print_ASCII(c)`: byte-domain
                        // characters use TeX's one-character-string spelling,
                        // while Umber's Unicode extension remains lossless.
                        .print_ascii(character)
                        .print_char(')');
                    report.help(&[
                        "Somewhere in the math formula just ended, you used the",
                        "stated character from an undefined font family. For example,",
                        "plain TeX doesn't allow \\it or \\sl in subscripts. Proceed,",
                        "and I'll try to forget that I needed that character.",
                    ]);
                    if let Some(context) = self.error_context {
                        report.context(context.0.clone());
                    }
                    let _ = report.error();
                }
            }
        }
        if layout.recovered() {
            self.root_nodes.clear();
            return;
        }
        for packed in layout.pack_observations() {
            self.stores.record_geometry_observation(match packed.axis {
                tex_typeset::math::BoxAxis::Horizontal => GeometryObservation::Hpack {
                    width_sp: i64::from(packed.width.raw()),
                    height_sp: i64::from(packed.height.raw()),
                    depth_sp: i64::from(packed.depth.raw()),
                    line: self.stores.current_input_line().max(0) as u32,
                    source: self.stores.current_input_source(),
                },
                tex_typeset::math::BoxAxis::Vertical => GeometryObservation::Vpack {
                    width_sp: i64::from(packed.width.raw()),
                    height_sp: i64::from(packed.height.raw()),
                    depth_sp: i64::from(packed.depth.raw()),
                    line: self.stores.current_input_line().max(0) as u32,
                    source: self.stores.current_input_source(),
                },
            });
        }
        let mut root = std::mem::take(&mut self.root_nodes);
        root.clear();
        root.reserve(list.node_count());
        self.append_span(list, layout, &mut root);
        self.root_nodes = root;
    }
}

pub(crate) fn finish_math_lists_owned<G>(
    stores: &mut Universe<G>,
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

fn lower_math_box(boxed: &MathBox, children: PageListId) -> BoxNode {
    BoxNode::new(BoxNodeFields {
        width: boxed.width,
        height: boxed.height,
        depth: boxed.depth,
        shift: boxed.shift,
        box_lr: if boxed.display {
            tex_state::node::BoxLr::DList
        } else {
            tex_state::node::BoxLr::Normal
        },
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
