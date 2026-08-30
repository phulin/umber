use std::cell::Cell;

use tex_state::CommandContext;
use tex_state::diagnostic::DiagnosticEffects;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam};
use tex_state::glue::GlueSpec;
use tex_state::ids::FontId;
use tex_state::math::MathListNode;
use tex_state::node::{BoxNode, BoxNodeFields, GlueKind, Node};
use tex_state::node_arena::PageListId;
use tex_state::page_node_arena::UniquePageList;
use tex_state::scaled::Scaled;
use tex_typeset::TypesetState;
use tex_typeset::math::{
    FrozenHList, MathBox, MathConversionEvent, MathGlueKind, MathLayout, MathNode, MathParamState,
    MathParams, MathTypesetState, Style, mlist_to_hlist,
};

#[cfg(test)]
mod tests;

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
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    list: MathListNode,
    insert_penalties: bool,
) -> UniquePageList {
    finish_math_list_node_with_reads(
        stores,
        diagnostic_effects,
        geometry,
        list,
        insert_penalties,
        None,
    )
    .0
}

/// Lowers a shipout-surviving math list directly into final reusable scratch
/// rows. Child coordinates point back to immutable page rows when a native
/// math node already owns such material; no node graph is rehomed.
pub(crate) fn finish_math_list_node_to_shipout_scratch<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    list: MathListNode,
    insert_penalties: bool,
) -> tex_state::ShipoutScratchListId {
    let mut sink = LoweredMathSink::new(stores, diagnostic_effects, geometry, None);
    let params = MathParams::read(&sink);
    let style = if list.display {
        Style::DISPLAY
    } else {
        Style::TEXT
    };
    let layout = mlist_to_hlist(
        &sink,
        list.content,
        style,
        insert_penalties && !list.display,
        &params,
    );
    sink.commit_math_events(&layout);
    let root = sink.stores.begin_shipout_scratch_list();
    if layout.recovered() {
        return root;
    }
    if !list.display {
        let surround = sink.stores.dimen_param(DimenParam::MATH_SURROUND);
        sink.stores
            .push_shipout_scratch_node(root, Node::MathOn(surround));
    }
    sink.append_span_to_shipout(layout.root(), &layout, root);
    if !list.display {
        let surround = sink.stores.dimen_param(DimenParam::MATH_SURROUND);
        sink.stores
            .push_shipout_scratch_node(root, Node::MathOff(surround));
    }
    root
}

pub(crate) fn finish_inline_math_list_node<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    list: MathListNode,
    insert_penalties: bool,
    error_context: MathConversionErrorContext,
) -> (PageListId, u64) {
    let (list, family_mask) = finish_math_list_node_with_reads(
        stores,
        diagnostic_effects,
        geometry,
        list,
        insert_penalties,
        Some(&error_context),
    );
    (stores.publish_unique_page_list(list), family_mask)
}

fn finish_math_list_node_with_reads<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    list: MathListNode,
    insert_penalties: bool,
    error_context: Option<&MathConversionErrorContext>,
) -> (UniquePageList, u64) {
    let mut sink = LoweredMathSink::new(stores, diagnostic_effects, geometry, error_context);
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
    let mut nodes = tex_state::page_node_arena::PageMaterialActiveListBuilder::vacant();
    sink.stores.open_page_active_list(&mut nodes);
    if !list.display {
        let surround = sink.stores.dimen_param(DimenParam::MATH_SURROUND);
        sink.stores
            .push_page_active_list(&mut nodes, Node::MathOn(surround));
    }
    let hlist = sink.stores.reclaim_unique_page_list(hlist);
    sink.stores
        .append_unique_page_active_list(&mut nodes, hlist);
    if !list.display {
        // AppG rule 22
        let surround = sink.stores.dimen_param(DimenParam::MATH_SURROUND);
        sink.stores
            .push_page_active_list(&mut nodes, Node::MathOff(surround));
    }
    (
        sink.stores.finalize_unique_page_active_list(&mut nodes),
        family_mask,
    )
}

#[allow(clippy::too_many_arguments)] // Math lowering keeps style, penalties, params, and error context independent.
pub(super) fn convert_math_hlist_with_error_context<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    input: PageListId,
    style: Style,
    penalties: bool,
    params: &MathParams,
    error_context: Option<&MathConversionErrorContext>,
) -> PageListId {
    let mut sink = LoweredMathSink::new(stores, diagnostic_effects, geometry, error_context);
    convert_math_hlist_with_sink(&mut sink, input, style, penalties, params)
}

fn convert_math_hlist_with_sink<G>(
    sink: &mut LoweredMathSink<'_, '_, G>,
    input: PageListId,
    style: Style,
    penalties: bool,
    params: &MathParams,
) -> PageListId {
    let transaction = mlist_to_hlist(&*sink, input, style, penalties, params);
    sink.commit_math_transaction(&transaction);
    sink.take_root_nodes()
}

struct LoweredMathSink<'a, 'ctx, G> {
    stores: &'a mut CommandContext<'ctx, G>,
    diagnostic_effects: &'a mut DiagnosticEffects,
    geometry: &'a mut dyn crate::geometry::PackGeometrySink,
    error_context: Option<&'a MathConversionErrorContext>,
    root_nodes: PageListId,
    glue_cache: Vec<(GlueSpec, GlueSpec)>,
    family_mask: Cell<u64>,
}

impl<'a, 'ctx, G> LoweredMathSink<'a, 'ctx, G> {
    fn new(
        stores: &'a mut CommandContext<'ctx, G>,
        diagnostic_effects: &'a mut DiagnosticEffects,
        geometry: &'a mut dyn crate::geometry::PackGeometrySink,
        error_context: Option<&'a MathConversionErrorContext>,
    ) -> Self {
        Self {
            stores,
            diagnostic_effects,
            geometry,
            error_context,
            root_nodes: PageListId::empty(),
            glue_cache: Vec::with_capacity(8),
            family_mask: Cell::new(0),
        }
    }

    fn lower_span(&mut self, list: FrozenHList, layout: &MathLayout) -> UniquePageList {
        if let Some(source) = self.whole_native_source(list, layout) {
            return self.stores.reclaim_unique_page_list(source);
        }
        let mut target = tex_state::page_node_arena::PageMaterialActiveListBuilder::vacant();
        self.stores.open_page_active_list(&mut target);
        for node in layout.nodes(list) {
            match node {
                MathNode::Sequence(child) => {
                    let result = self.stores.finalize_unique_page_active_list(&mut target);
                    let child = self.lower_span(*child, layout);
                    self.stores.open_page_active_list(&mut target);
                    self.stores
                        .append_unique_page_active_list(&mut target, result);
                    self.stores
                        .append_unique_page_active_list(&mut target, child);
                }
                MathNode::HList(boxed) | MathNode::VList(boxed) => {
                    let vertical = matches!(node, MathNode::VList(_));
                    if self.unchanged_native_box(boxed, vertical).is_some() {
                        let source = boxed.source.expect("unchanged source box has provenance");
                        self.stores.append_page_active_list_range(
                            &mut target,
                            source.list,
                            source.index as usize..source.index as usize + 1,
                        );
                        continue;
                    }
                    let result = self.stores.finalize_unique_page_active_list(&mut target);
                    let children = self.lower_span(boxed.list, layout);
                    let children = self.stores.publish_unique_page_list(children);
                    let boxed_node = lower_math_box(boxed, children);
                    self.stores.open_page_active_list(&mut target);
                    self.stores
                        .append_unique_page_active_list(&mut target, result);
                    self.stores.push_page_active_list(
                        &mut target,
                        if vertical {
                            Node::VList(boxed_node)
                        } else {
                            Node::HList(boxed_node)
                        },
                    );
                }
                MathNode::Char {
                    font, ch, origin, ..
                } => self.stores.push_page_active_list(
                    &mut target,
                    Node::Char {
                        font: *font,
                        ch: *ch,
                        origin: *origin,
                    },
                ),
                MathNode::Kern { amount, kind } => self.stores.push_page_active_list(
                    &mut target,
                    Node::Kern {
                        amount: *amount,
                        kind: *kind,
                    },
                ),
                MathNode::Glue { spec, kind, leader } => {
                    let value = if let Some((_, value)) =
                        self.glue_cache.iter().find(|(cached, _)| cached == spec)
                    {
                        *value
                    } else {
                        self.glue_cache.push((*spec, *spec));
                        *spec
                    };
                    self.stores.push_page_active_list(
                        &mut target,
                        Node::Glue {
                            spec: value,
                            kind: lower_math_glue_kind(*kind),
                            leader: *leader,
                        },
                    );
                }
                MathNode::Penalty(penalty) => self
                    .stores
                    .push_page_active_list(&mut target, Node::Penalty(*penalty)),
                MathNode::Rule {
                    width,
                    height,
                    depth,
                } => self.stores.push_page_active_list(
                    &mut target,
                    Node::Rule {
                        width: *width,
                        height: *height,
                        depth: *depth,
                    },
                ),
                MathNode::NativeSource { list, index, .. } => {
                    self.stores.append_page_active_list_range(
                        &mut target,
                        *list,
                        *index as usize..*index as usize + 1,
                    );
                }
            }
        }
        self.stores.finalize_unique_page_active_list(&mut target)
    }

    /// Returns the one source list represented by this span when math
    /// conversion left every node unchanged and retained the whole list in
    /// order. The caller owns the consumed math closure, so this is a direct
    /// ownership transfer rather than a shared-range append.
    fn whole_native_source(&self, list: FrozenHList, layout: &MathLayout) -> Option<PageListId> {
        let nodes = layout.nodes(list);
        let source_coordinate = |node: &MathNode| match node {
            MathNode::NativeSource { list, index, .. } => Some((*list, *index as usize)),
            MathNode::HList(boxed) => self.unchanged_native_box(boxed, false).and_then(|_| {
                boxed
                    .source
                    .map(|source| (source.list, source.index as usize))
            }),
            MathNode::VList(boxed) => self.unchanged_native_box(boxed, true).and_then(|_| {
                boxed
                    .source
                    .map(|source| (source.list, source.index as usize))
            }),
            _ => None,
        };
        let (source, first_index) = source_coordinate(nodes.first()?)?;
        if first_index != 0 || source.len() != nodes.len() {
            return None;
        }
        nodes
            .iter()
            .enumerate()
            .all(|(expected, node)| source_coordinate(node) == Some((source, expected)))
            .then_some(source)
    }

    fn append_span_to_shipout(
        &mut self,
        list: FrozenHList,
        layout: &MathLayout,
        target: tex_state::ShipoutScratchListId,
    ) {
        enum Task {
            Span {
                list: FrozenHList,
                index: usize,
                target: tex_state::ShipoutScratchListId,
            },
            FinishBox {
                boxed: MathBox,
                vertical: bool,
                target: tex_state::ShipoutScratchListId,
                children: tex_state::ShipoutScratchListId,
            },
        }

        let mut tasks = vec![Task::Span {
            list,
            index: 0,
            target,
        }];
        while let Some(task) = tasks.pop() {
            match task {
                Task::FinishBox {
                    boxed,
                    vertical,
                    target,
                    children,
                } => {
                    let boxed = lower_math_box(&boxed, children);
                    self.stores.push_shipout_scratch_node(
                        target,
                        if vertical {
                            Node::VList(boxed)
                        } else {
                            Node::HList(boxed)
                        },
                    );
                }
                Task::Span {
                    list,
                    index,
                    target,
                } => {
                    let Some(node) = layout.nodes(list).get(index) else {
                        continue;
                    };
                    tasks.push(Task::Span {
                        list,
                        index: index + 1,
                        target,
                    });
                    match node {
                        MathNode::Sequence(child) => tasks.push(Task::Span {
                            list: *child,
                            index: 0,
                            target,
                        }),
                        MathNode::HList(boxed) | MathNode::VList(boxed) => {
                            let vertical = matches!(node, MathNode::VList(_));
                            if let Some(source_box) = self.unchanged_native_box(boxed, vertical) {
                                let source_node = if vertical {
                                    Node::VList(source_box)
                                } else {
                                    Node::HList(source_box)
                                };
                                let source_node = source_node.map_lists(|child| {
                                    self.stores
                                        .copy_page_list_to_shipout_scratch(child)
                                        .expect("native math box children remain live")
                                });
                                self.stores.push_shipout_scratch_node(target, source_node);
                                continue;
                            }
                            let children = self.stores.begin_shipout_scratch_list();
                            tasks.push(Task::FinishBox {
                                boxed: *boxed,
                                vertical,
                                target,
                                children,
                            });
                            tasks.push(Task::Span {
                                list: boxed.list,
                                index: 0,
                                target: children,
                            });
                        }
                        MathNode::Char {
                            font, ch, origin, ..
                        } => self.stores.push_shipout_scratch_node(
                            target,
                            Node::Char {
                                font: *font,
                                ch: *ch,
                                origin: *origin,
                            },
                        ),
                        MathNode::Kern { amount, kind } => self.stores.push_shipout_scratch_node(
                            target,
                            Node::Kern {
                                amount: *amount,
                                kind: *kind,
                            },
                        ),
                        MathNode::Glue { spec, kind, leader } => {
                            let leader = leader.map(|leader| {
                                leader.map_lists(|child| {
                                    self.stores
                                        .copy_page_list_to_shipout_scratch(child)
                                        .expect("math leader children remain live")
                                })
                            });
                            self.stores.push_shipout_scratch_node(
                                target,
                                Node::Glue {
                                    spec: *spec,
                                    kind: lower_math_glue_kind(*kind),
                                    leader,
                                },
                            )
                        }
                        MathNode::Penalty(penalty) => self
                            .stores
                            .push_shipout_scratch_node(target, Node::Penalty(*penalty)),
                        MathNode::Rule {
                            width,
                            height,
                            depth,
                        } => self.stores.push_shipout_scratch_node(
                            target,
                            Node::Rule {
                                width: *width,
                                height: *height,
                                depth: *depth,
                            },
                        ),
                        MathNode::NativeSource { list, index, .. } => {
                            let node = self
                                .stores
                                .page_node_list(*list)
                                .expect("math native source belongs to the page arena")
                                .owned_node(*index as usize)
                                .expect("math native source index remains live")
                                .clone()
                                .map_lists(|child| {
                                    self.stores
                                        .copy_page_list_to_shipout_scratch(child)
                                        .expect("native math source children remain live")
                                });
                            self.stores.push_shipout_scratch_node(target, node);
                        }
                    }
                }
            }
        }
    }

    fn unchanged_native_box(&self, boxed: &MathBox, vertical: bool) -> Option<BoxNode> {
        boxed.source.and_then(|source| {
            if source.payload != boxed.list {
                return None;
            }
            let node = self
                .stores
                .page_node_list(source.list)
                .ok()
                .and_then(|nodes| nodes.owned_node(source.index as usize))?;
            match node {
                Node::HList(source_box)
                    if !vertical && lower_math_box(boxed, source_box.children) == *source_box =>
                {
                    Some(*source_box)
                }
                Node::VList(source_box)
                    if vertical && lower_math_box(boxed, source_box.children) == *source_box =>
                {
                    Some(*source_box)
                }
                _ => None,
            }
        })
    }

    fn take_root_nodes(&mut self) -> PageListId {
        std::mem::take(&mut self.root_nodes)
    }
}

impl<G> TypesetState for LoweredMathSink<'_, '_, G> {
    fn page_nodes(&self, list: PageListId) -> tex_state::node_arena::NodeCursor<'_> {
        self.stores
            .page_node_list(list)
            .expect("math list belongs to the admitted page arena")
            .nodes()
    }

    fn font_char_metrics(&self, font: FontId, code: u8) -> Option<tex_fonts::CharMetrics> {
        self.stores.font_character_metrics(font, char::from(code))
    }

    fn font_widths(&self, font: FontId) -> &[Scaled; 256] {
        self.stores.font_widths(font)
    }

    fn font_characters(&self, font: FontId) -> &[Option<tex_fonts::CharMetrics>] {
        self.stores.font_characters(font)
    }
}

impl<G> MathTypesetState for LoweredMathSink<'_, '_, G> {
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
        self.stores.font_extensible_recipe(font, code)
    }

    fn lig_kern_command(
        &self,
        font: FontId,
        left: tex_fonts::LigKernChar,
        right: tex_fonts::LigKernChar,
    ) -> Option<tex_fonts::LigKernCommand> {
        self.stores.font_lig_kern_command(font, left, right)
    }

    fn font_skew_char(&self, font: FontId) -> i32 {
        self.stores.font_skew_char(font)
    }

    fn math_metrics_source(&self, font: FontId) -> tex_fonts::MathMetricsSource<'_> {
        self.stores.font_math_metrics_source(font)
    }
}

impl<G> MathParamState for LoweredMathSink<'_, '_, G> {
    fn int_param(&self, param: IntParam) -> i32 {
        self.stores.int_param(param)
    }

    fn dimen_param(&self, param: DimenParam) -> Scaled {
        self.stores.dimen_param(param)
    }

    fn glue_param(&self, param: GlueParam) -> GlueSpec {
        self.stores
            .glue_param(param)
            .map_or(GlueSpec::ZERO, |id| self.stores.glue(id))
    }
}

impl<G> LoweredMathSink<'_, '_, G> {
    fn commit_math_events(&mut self, layout: &MathLayout) {
        for event in layout.conversion_events() {
            match *event {
                MathConversionEvent::MissingCharacter { font, character } => {
                    if self.stores.int_param(IntParam::TRACING_LOST_CHARS) > 0 {
                        let font_name = self.stores.font_name(font);
                        let mut diagnostic = self.stores.begin_diagnostic(self.diagnostic_effects);
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
                } => self.report_undefined_family(size, family, character),
            }
        }
        for packed in layout.pack_observations() {
            match packed.axis {
                tex_typeset::math::BoxAxis::Horizontal => {
                    self.geometry
                        .committed_hpack(packed.width, packed.height, packed.depth);
                }
                tex_typeset::math::BoxAxis::Vertical => {
                    self.geometry
                        .committed_vpack(packed.width, packed.height, packed.depth);
                }
            }
        }
    }

    fn report_undefined_family(
        &mut self,
        size: tex_state::math::MathFontSize,
        family: u8,
        character: char,
    ) {
        self.stores
            .publish_diagnostic_effects_before_synchronous_print(self.diagnostic_effects);
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

    fn commit_math_transaction(&mut self, layout: &MathLayout) {
        let list = layout.root();
        for event in layout.conversion_events() {
            match *event {
                MathConversionEvent::MissingCharacter { font, character } => {
                    if self.stores.int_param(IntParam::TRACING_LOST_CHARS) > 0 {
                        // TeX82 §581's `char_warning` prints the stored
                        // external name, not `\fontname`'s size-qualified
                        // rendering.
                        let font_name = self.stores.font_name(font);
                        let mut diagnostic = self.stores.begin_diagnostic(self.diagnostic_effects);
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
                    // TeX82 §§721 walks the converted noads in order.
                    // A preceding §581 missing-character warning has already
                    // completed before this synchronous error dialogue opens,
                    // so publish its detached print program at this exact
                    // event boundary.
                    self.stores
                        .publish_diagnostic_effects_before_synchronous_print(
                            self.diagnostic_effects,
                        );
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
            self.root_nodes = PageListId::empty();
            return;
        }
        for packed in layout.pack_observations() {
            match packed.axis {
                tex_typeset::math::BoxAxis::Horizontal => {
                    self.geometry
                        .committed_hpack(packed.width, packed.height, packed.depth);
                }
                tex_typeset::math::BoxAxis::Vertical => {
                    self.geometry
                        .committed_vpack(packed.width, packed.height, packed.depth);
                }
            }
        }
        let root = self.lower_span(list, layout);
        self.root_nodes = self.stores.publish_unique_page_list(root);
    }
}

pub(crate) fn finish_math_lists_owned<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    nodes: tex_state::node_arena::PageListId,
    insert_penalties: bool,
) -> tex_state::node_arena::PageListId {
    let source_len = nodes.len();
    let source = stores
        .admit_page_node_span(nodes)
        .expect("owned math-list source belongs to the page arena");
    let mut result = tex_state::page_node_arena::PageListSpan::empty();
    let mut copied_through = 0;
    if let Some(tail) = stores
        .page_node_span_tail_chunk(source)
        .expect("owned math-list source remains admitted")
    {
        finish_math_chunk_prefix(
            stores,
            diagnostic_effects,
            geometry,
            nodes,
            source,
            tail,
            insert_penalties,
            &mut result,
            &mut copied_through,
        );
    }
    if copied_through == 0 {
        return nodes;
    }
    if copied_through != source_len {
        // The suffix is the other edge of the consumed source block. Since
        // the prefix may already be linked into `result`, this is the one
        // unavoidable fragment copy for a replacement inside that block.
        let mut suffix = tex_state::page_node_arena::PageMaterialActiveListBuilder::vacant();
        stores.open_page_active_list(&mut suffix);
        stores.append_page_active_list_range(&mut suffix, nodes, copied_through..source_len);
        let suffix = stores.finalize_unique_page_active_list(&mut suffix);
        result = stores.append_unique_page_nodes(result, suffix);
    }
    result.list()
}

#[allow(clippy::too_many_arguments)]
fn finish_math_chunk_prefix<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    nodes: PageListId,
    source: tex_state::page_node_arena::PageListSpan,
    chunk: tex_state::page_node_arena::PageListChunkCursor,
    insert_penalties: bool,
    result: &mut tex_state::page_node_arena::PageListSpan,
    copied_through: &mut usize,
) {
    if let Some(previous) = stores
        .page_node_span_previous_chunk(source, &chunk)
        .expect("owned math-list source chunk remains live")
    {
        finish_math_chunk_prefix(
            stores,
            diagnostic_effects,
            geometry,
            nodes,
            source,
            previous,
            insert_penalties,
            result,
            copied_through,
        );
    }
    for offset in 0..chunk.len() {
        let index = chunk.logical_start() + offset;
        let list = {
            let (resolved, node) = stores
                .page_node_span_chunk_node(source, &chunk, offset)
                .expect("owned math-list source chunk remains live");
            debug_assert_eq!(resolved, index);
            match node {
                Node::MathList(list) => Some(*list),
                _ => None,
            }
        };
        let Some(list) = list else {
            continue;
        };
        if result.is_empty() && *copied_through == 0 && index != 0 {
            // The first retained prefix remains the immutable left root. Its
            // tail is never reopened; the generated math chain donates its
            // write-once head predecessor below.
            *result = stores.slice_page_node_span(source, 0..index);
        } else if *copied_through != index {
            // A second source fragment shares its original block topology
            // with the already-retained left fragment. It cannot donate a
            // second predecessor, so copy precisely this partial edge.
            let mut fragment = tex_state::page_node_arena::PageMaterialActiveListBuilder::vacant();
            stores.open_page_active_list(&mut fragment);
            stores.append_page_active_list_range(&mut fragment, nodes, *copied_through..index);
            let fragment = stores.finalize_unique_page_active_list(&mut fragment);
            *result = stores.append_unique_page_nodes(*result, fragment);
        }
        let lowered =
            finish_math_list_node(stores, diagnostic_effects, geometry, list, insert_penalties);
        *result = stores.append_unique_page_nodes(*result, lowered);
        *copied_through = index + 1;
    }
}

fn lower_math_box<List>(boxed: &MathBox, children: List) -> BoxNode<List> {
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
