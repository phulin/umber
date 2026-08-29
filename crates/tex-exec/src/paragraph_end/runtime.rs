pub(crate) struct ParagraphBreakResult {
    pub(crate) last_line: Option<BoxNode>,
    pub(crate) active_directions: Vec<Direction>,
}

impl ParagraphBreakResult {
    pub(crate) const fn empty() -> Self {
        Self {
            last_line: None,
            active_directions: Vec::new(),
        }
    }
}

struct ArenaPostLineMaterializer {
    semantic: ArenaPostLineChannel,
    diagnostic: Option<ArenaPostLineChannel>,
    diagnostic_boundaries: Option<Vec<usize>>,
    actions: Vec<tex_typeset::linebreak::MaterializationAction>,
    breaks: Vec<tex_typeset::linebreak::BreakDecision>,
    line_no: usize,
    params: PostLineBreakParams,
    par_fill_override: Option<GlueSpec>,
    semantic_lineage_scratch: Vec<tex_state::node_sequence::DirectHighCellLineage>,
    diagnostic_lineage_scratch: Vec<tex_state::node_sequence::DirectHighCellLineage>,
}

struct ArenaPostLineChannel {
    source: tex_state::page_node_arena::PageListSpan,
    position: usize,
    lineages: Vec<tex_state::node_sequence::DirectHighCellLineages>,
    pending_post: tex_state::node_arena::PageListId,
    pending_post_lineages: Vec<tex_state::node_sequence::DirectHighCellLineage>,
    active_directions: Vec<Direction>,
}

struct ArenaBrokenLine {
    nodes: tex_state::node_arena::PageListId,
    diagnostic_nodes: Option<tex_state::node_arena::PageListId>,
    allocator_high_cell_overlap: u32,
    penalty_after: Option<i32>,
    dimensions: LineDimensions,
}

impl ArenaPostLineMaterializer {
    fn new<G>(
        stores: &CommandContext<'_, G>,
        tape: ParagraphTape<'static>,
        breaks: Vec<tex_typeset::linebreak::BreakDecision>,
        params: PostLineBreakParams,
    ) -> Self {
        let arena = tape
            .into_arena_materialization()
            .expect("production paragraph tape remains arena-backed");
        let semantic = stores
            .admit_page_node_span(arena.semantic)
            .expect("semantic paragraph crosses one live page-region boundary");
        let diagnostic = arena.diagnostic.map(|diagnostic| {
            stores
                .admit_page_node_span(diagnostic)
                .expect("diagnostic paragraph crosses one live page-region boundary")
        });
        Self {
            semantic: ArenaPostLineChannel::new(semantic, arena.semantic_high_cell_lineages),
            diagnostic: diagnostic
                .zip(arena.diagnostic_high_cell_lineages)
                .map(|(source, lineages)| ArenaPostLineChannel::new(source, lineages)),
            diagnostic_boundaries: arena.diagnostic_boundaries,
            actions: arena.actions,
            breaks,
            line_no: 0,
            params,
            par_fill_override: arena.par_fill_override,
            semantic_lineage_scratch: Vec::new(),
            diagnostic_lineage_scratch: Vec::new(),
        }
    }

    fn materialize_next<G>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
    ) -> Option<ArenaBrokenLine> {
        let decision = *self.breaks.get(self.line_no)?;
        let diagnostic_decision = tex_typeset::linebreak::BreakDecision {
            position: self
                .diagnostic_boundaries
                .as_ref()
                .map_or(decision.position, |boundaries| {
                    boundaries[decision.position]
                }),
            ..decision
        };
        let dimensions = self.params.shape.dimensions(self.line_no + 1);
        let nodes = self.semantic.materialize(
            stores,
            decision,
            &self.params,
            Some(&self.actions),
            self.par_fill_override,
            &mut self.semantic_lineage_scratch,
        );
        let diagnostic = self.diagnostic.as_mut().map(|diagnostic| {
            diagnostic.materialize(
                stores,
                diagnostic_decision,
                &self.params,
                None,
                self.par_fill_override,
                &mut self.diagnostic_lineage_scratch,
            )
        });
        let allocator_high_cell_overlap = diagnostic.map_or(0, |_| {
            tex_state::node_sequence::direct_high_cell_overlap(
                &self.semantic_lineage_scratch,
                &self.diagnostic_lineage_scratch,
            )
        });
        let penalty_after = tex_typeset::linebreak::line_penalty_after(
            self.line_no,
            &self.breaks,
            decision.hyphenated,
            &self.params,
        );
        self.line_no += 1;
        Some(ArenaBrokenLine {
            nodes,
            diagnostic_nodes: diagnostic,
            allocator_high_cell_overlap,
            penalty_after,
            dimensions,
        })
    }
}

impl ArenaPostLineChannel {
    fn new(
        source: tex_state::page_node_arena::PageListSpan,
        lineages: Vec<tex_state::node_sequence::DirectHighCellLineages>,
    ) -> Self {
        assert_eq!(source.len(), lineages.len());
        Self {
            source,
            position: 0,
            lineages,
            pending_post: tex_state::node_arena::PageListId::empty(),
            pending_post_lineages: Vec::new(),
            active_directions: Vec::new(),
        }
    }

    fn materialize<G>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
        decision: tex_typeset::linebreak::BreakDecision,
        params: &PostLineBreakParams,
        actions: Option<&[tex_typeset::linebreak::MaterializationAction]>,
        par_fill_override: Option<GlueSpec>,
        output_lineages: &mut Vec<tex_state::node_sequence::DirectHighCellLineage>,
    ) -> tex_state::node_arena::PageListId {
        let end = decision.position.min(self.source.len());
        let plain_source_run = params.left_skip == GlueSpec::ZERO
            && self.active_directions.is_empty()
            && self.pending_post.is_empty()
            && par_fill_override.is_none()
            && (self.position..end).all(|absolute| {
                actions
                    .and_then(|actions| actions.get(absolute))
                    .is_none_or(|action| {
                        *action == tex_typeset::linebreak::MaterializationAction::Copy
                    })
                    && matches!(
                        classify_post_line_node(stores, self.source, absolute),
                        PostLineNode::Other
                    )
                    && !matches!(
                        stores
                            .page_node_span(self.source)
                            .expect("paragraph source remains live")
                            .nodes()
                            .owned_node(absolute),
                        Some(Node::Direction(_))
                    )
            });
        if plain_source_run {
            output_lineages.clear();
            for absolute in self.position..end {
                output_lineages.extend(self.lineages[absolute].iter().cloned());
            }
            let retained = stores.slice_page_node_span(self.source, self.position..end);
            self.position = end;
            let suffix = stores.publish_unique_page_nodes(vec![Node::Glue {
                spec: params.right_skip,
                kind: GlueKind::RightSkip,
                leader: None,
            }]);
            let output = stores.append_unique_page_nodes(retained, suffix).list();
            while self.position < self.source.len()
                && stores
                    .page_node_span(self.source)
                    .expect("paragraph source remains live")
                    .nodes()
                    .owned_node(self.position)
                    .is_some_and(post_line_discardable)
            {
                self.position += 1;
            }
            return output;
        }
        let mut output = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
        stores.open_page_active_list(&mut output);
        output_lineages.clear();
        if params.left_skip != GlueSpec::ZERO {
            stores.push_page_active_list(
                &mut output,
                Node::Glue {
                    spec: params.left_skip,
                    kind: GlueKind::LeftSkip,
                    leader: None,
                },
            );
        }
        for direction in self.active_directions.iter().copied() {
            stores.push_page_active_list(&mut output, Node::Direction(direction));
        }
        if !self.pending_post.is_empty() {
            let pending_post = stores
                .admit_page_node_span(self.pending_post)
                .expect("pending discretionary post list remains live");
            append_direction_evidence(stores, pending_post, &mut self.active_directions);
            stores.append_page_active_span(&mut output, pending_post);
            output_lineages.append(&mut self.pending_post_lineages);
            self.pending_post = tex_state::node_arena::PageListId::empty();
        }

        while self.position < end {
            let absolute = self.position;
            let action = actions.and_then(|actions| actions.get(absolute)).copied();
            let node_action = classify_post_line_node(stores, self.source, absolute);
            self.position += 1;
            match node_action {
                PostLineNode::Discretionary { kind, pre, post, replace, physical_replace_count }
                    if decision.hyphenated && absolute + 1 == end =>
                {
                    stores.push_page_active_list(
                        &mut output,
                        Node::Disc {
                            kind,
                            pre: params.empty_list,
                            post: params.empty_list,
                            replace: params.empty_list,
                            physical_replace_count: 0,
                        },
                    );
                    let pre_span = stores
                        .admit_page_node_span(pre)
                        .expect("discretionary pre list remains live");
                    append_direction_evidence(stores, pre_span, &mut self.active_directions);
                    stores.append_page_active_span(&mut output, pre_span);
                    extend_frozen_lineages(
                        stores,
                        pre_span,
                        tex_state::node_sequence::FrozenListRole::Pre,
                        output_lineages,
                    );
                    self.pending_post = post;
                    self.pending_post_lineages.clear();
                    let post_span = stores
                        .admit_page_node_span(post)
                        .expect("discretionary post list remains live");
                    extend_frozen_lineages(
                        stores,
                        post_span,
                        tex_state::node_sequence::FrozenListRole::Post,
                        &mut self.pending_post_lineages,
                    );
                    let _ = (replace, physical_replace_count);
                }
                PostLineNode::Discretionary { replace, .. } => {
                    append_source_direction(stores, self.source, absolute, &mut self.active_directions);
                    stores.append_page_active_span_range(
                        &mut output,
                        self.source,
                        absolute..absolute + 1,
                    );
                    let replace_span = stores
                        .admit_page_node_span(replace)
                        .expect("discretionary replacement list remains live");
                    append_direction_evidence(stores, replace_span, &mut self.active_directions);
                    stores.append_page_active_span(&mut output, replace_span);
                    extend_frozen_lineages(
                        stores,
                        replace_span,
                        tex_state::node_sequence::FrozenListRole::Replace,
                        output_lineages,
                    );
                }
                PostLineNode::ParFillGlue if par_fill_override.is_some() => {
                    stores.push_page_active_list(
                        &mut output,
                        Node::Glue {
                            spec: par_fill_override.expect("matched override"),
                            kind: GlueKind::ParFillSkip,
                            leader: None,
                        },
                    );
                }
                PostLineNode::DiscardableGlue
                    if absolute + 1 == end
                        && end < self.source.len()
                        && action.is_none_or(|action| {
                            action == tex_typeset::linebreak::MaterializationAction::BreakDiscardable
                        }) => {}
                PostLineNode::MathOff
                    if absolute + 1 == end
                        && end < self.source.len()
                        && action.is_none_or(|action| {
                            action == tex_typeset::linebreak::MaterializationAction::BreakMath
                        }) =>
                {
                    stores.push_page_active_list(
                        &mut output,
                        Node::MathOff(Scaled::from_raw(0)),
                    );
                }
                _ => {
                    append_source_direction(stores, self.source, absolute, &mut self.active_directions);
                    stores.append_page_active_span_range(
                        &mut output,
                        self.source,
                        absolute..absolute + 1,
                    );
                    output_lineages.extend(self.lineages[absolute].iter().cloned());
                }
            }
        }
        for direction in self.active_directions.iter().rev().copied() {
            stores.push_page_active_list(
                &mut output,
                Node::Direction(matching_direction_end(direction)),
            );
        }
        stores.push_page_active_list(
            &mut output,
            Node::Glue {
                spec: params.right_skip,
                kind: GlueKind::RightSkip,
                leader: None,
            },
        );
        while self.position < self.source.len()
            && stores
                .page_node_span(self.source)
                .expect("paragraph source remains live")
                .nodes()
                .owned_node(self.position)
                .is_some_and(post_line_discardable)
        {
            self.position += 1;
        }
        stores.finalize_page_active_list(&mut output)
    }
}

enum PostLineNode {
    Discretionary {
        kind: tex_state::node::DiscKind,
        pre: tex_state::node_arena::PageListId,
        post: tex_state::node_arena::PageListId,
        replace: tex_state::node_arena::PageListId,
        physical_replace_count: u8,
    },
    ParFillGlue,
    DiscardableGlue,
    MathOff,
    Other,
}

fn classify_post_line_node<G>(
    stores: &CommandContext<'_, G>,
    source: tex_state::page_node_arena::PageListSpan,
    index: usize,
) -> PostLineNode {
    match stores
        .page_node_span(source)
        .expect("paragraph source remains live")
        .nodes()
        .owned_node(index)
        .expect("paragraph cursor remains in bounds")
    {
        Node::Disc {
            kind,
            pre,
            post,
            replace,
            physical_replace_count,
        } => PostLineNode::Discretionary {
            kind: *kind,
            pre: *pre,
            post: *post,
            replace: *replace,
            physical_replace_count: *physical_replace_count,
        },
        Node::Glue {
            kind: GlueKind::ParFillSkip,
            ..
        } => PostLineNode::ParFillGlue,
        Node::Glue { .. } => PostLineNode::DiscardableGlue,
        Node::MathOff(_) => PostLineNode::MathOff,
        _ => PostLineNode::Other,
    }
}

fn append_source_direction<G>(
    stores: &CommandContext<'_, G>,
    source: tex_state::page_node_arena::PageListSpan,
    index: usize,
    active: &mut Vec<Direction>,
) {
    if let Some(Node::Direction(direction)) = stores
        .page_node_span(source)
        .expect("paragraph source remains live")
        .nodes()
        .owned_node(index)
    {
        update_direction(*direction, active);
    }
}

fn append_direction_evidence<G>(
    stores: &CommandContext<'_, G>,
    source: tex_state::page_node_arena::PageListSpan,
    active: &mut Vec<Direction>,
) {
    for node in stores
        .page_node_span(source)
        .expect("paragraph branch remains live")
        .nodes()
        .iter()
    {
        if let Node::Direction(direction) = node {
            update_direction(*direction, active);
        }
    }
}

fn update_direction(direction: Direction, active: &mut Vec<Direction>) {
    match direction {
        Direction::BeginL | Direction::BeginR => active.push(direction),
        Direction::EndL if active.last() == Some(&Direction::BeginL) => {
            let _ = active.pop();
        }
        Direction::EndR if active.last() == Some(&Direction::BeginR) => {
            let _ = active.pop();
        }
        Direction::BeginM | Direction::EndM | Direction::EndL | Direction::EndR => {}
    }
}

const fn matching_direction_end(direction: Direction) -> Direction {
    match direction {
        Direction::BeginM => Direction::EndM,
        Direction::BeginL => Direction::EndL,
        Direction::BeginR => Direction::EndR,
        Direction::EndM | Direction::EndL | Direction::EndR => direction,
    }
}

fn extend_frozen_lineages<G>(
    stores: &CommandContext<'_, G>,
    span: tex_state::page_node_arena::PageListSpan,
    role: tex_state::node_sequence::FrozenListRole,
    output: &mut Vec<tex_state::node_sequence::DirectHighCellLineage>,
) {
    let nodes = stores
        .page_node_span(span)
        .expect("frozen discretionary branch remains live")
        .nodes();
    for (row, node) in nodes.iter().enumerate() {
        let count = match node {
            Node::Char { .. } => 1,
            Node::Lig { orig, .. } => orig.len(),
            _ => 0,
        };
        for unit in 0..count {
            output.push(tex_state::node_sequence::DirectHighCellLineage::Frozen {
                list: span.list(),
                row: u32::try_from(row).expect("frozen list exceeds u32 rows"),
                unit: u32::try_from(unit).expect("ligature source exceeds u32 cells"),
                role,
            });
        }
    }
}

fn post_line_discardable(node: &Node) -> bool {
    matches!(
        node,
        Node::Glue { .. }
            | Node::Kern {
                kind: KernKind::Explicit | KernKind::Mu,
                ..
            }
            | Node::Penalty(_)
            | Node::MathOn(_)
            | Node::MathOff(_)
    )
}

pub(crate) fn display_line_dimensions<G>(
    nest: &ModeNest,
    stores: &CommandContext<'_, G>,
) -> LineDimensions {
    let params = ParagraphParams {
        left_skip: glue_parameter_value(stores, GlueParam::LEFT_SKIP),
        right_skip: glue_parameter_value(stores, GlueParam::RIGHT_SKIP),
        par_fill_skip: glue_parameter_value(stores, GlueParam::PAR_FILL_SKIP),
        par_shape: stores.paragraph_shape(),
        prev_graf: nest.enclosing_vertical_prev_graf(),
        hang_indent: stores.dimen_param(DimenParam::HANG_INDENT),
        hang_after: stores.int_param(IntParam::HANG_AFTER),
        looseness: stores.int_param(IntParam::LOOSENESS),
        pretolerance: stores.int_param(IntParam::PRETOLERANCE),
        tolerance: stores.int_param(IntParam::TOLERANCE),
        line_penalty: stores.int_param(IntParam::LINE_PENALTY),
        hyphen_penalty: stores.int_param(IntParam::HYPHEN_PENALTY),
        ex_hyphen_penalty: stores.int_param(IntParam::EX_HYPHEN_PENALTY),
        adj_demerits: stores.int_param(IntParam::ADJ_DEMERITS),
        double_hyphen_demerits: stores.int_param(IntParam::DOUBLE_HYPHEN_DEMERITS),
        final_hyphen_demerits: stores.int_param(IntParam::FINAL_HYPHEN_DEMERITS),
        last_line_fit: stores.int_param(IntParam::LAST_LINE_FIT),
        emergency_stretch: stores.dimen_param(DimenParam::EMERGENCY_STRETCH),
        hsize: stores.dimen_param(DimenParam::H_SIZE),
        interline_penalty: stores.int_param(IntParam::INTERLINE_PENALTY),
        club_penalty: stores.int_param(IntParam::CLUB_PENALTY),
        widow_penalty: stores.int_param(IntParam::WIDOW_PENALTY),
        display_widow_penalty: stores.int_param(IntParam::DISPLAY_WIDOW_PENALTY),
        broken_penalty: stores.int_param(IntParam::BROKEN_PENALTY),
        interline_penalties: stores.penalty_array(PenaltyArrayKind::InterLine),
        club_penalties: stores.penalty_array(PenaltyArrayKind::Club),
        widow_penalties: stores.penalty_array(PenaltyArrayKind::Widow),
        display_widow_penalties: stores.penalty_array(PenaltyArrayKind::DisplayWidow),
    };
    line_shape(&params).dimensions(2)
}

#[allow(clippy::too_many_arguments)] // Paragraph finalization keeps policy and admitted execution services explicit.
pub(crate) fn break_current_paragraph<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    widow_penalty_selector: tex_typeset::linebreak::WidowPenaltySelector,
    reset_paragraph: bool,
    diagnostic_context: crate::pack_report::ExecutionDiagnosticContext,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    fuel: &mut tex_command::CommandFuel,
) -> Result<ParagraphBreakResult, ExecError> {
    flush_pending_hchars_with_fuel(nest, stores, diagnostic_effects, fuel)?;
    let active_directions = active_text_directions(nest.current_list().nodes(stores));
    let mut params = snapshot_paragraph_params(nest, stores);
    {
        let mut list = nest.current_list_mutation();
        if matches!(list.nodes(stores).last(), Some(Node::Glue { .. })) {
            let _ = list.pop_last_node(stores);
        }
    }
    nest.current_list_mutation()
        .push(stores, Node::Penalty(10_000));
    nest.current_list_mutation().push(
        stores,
        Node::Glue {
            spec: params.par_fill_skip,
            kind: GlueKind::ParFillSkip,
            leader: None,
        },
    );
    let mut level = commit_current_list(nest, stores, diagnostic_effects, fuel)?;
    let paragraph_diagnostic_context = diagnostic_context.with_pack_begin_line(level.entry_line());
    let hlist = crate::math::finish_math_lists_owned(
        stores,
        diagnostic_effects,
        geometry,
        level.list_mutation().take_nodes(),
        true,
    );
    let tracing = stores.int_param(IntParam::TRACING_PARAGRAPHS) > 0;
    let hlist = normalize_paragraph_infinite_shrink(
        stores,
        &mut params,
        hlist,
        tracing,
        &diagnostic_context,
        diagnostic_effects,
    )?;
    let mut line_params = line_break_params(stores, &params);
    if line_params.pdf_adjust_spacing > 1 {
        line_params.expansion_steps = tex_typeset::linebreak::validate_paragraph_expansion(
            &crate::typeset_context::TypesetContext::new(stores),
            stores
                .page_node_list(hlist)
                .expect("paragraph belongs to the live page arena")
                .nodes(),
        )?;
    }
    let (mut decisions, trace, missing_hyphens) = break_hlist_with_trace(
        stores,
        diagnostic_effects,
        hlist,
        line_params,
        fuel,
        tracing,
    )?;
    if tracing {
        report_line_break_trace(
            stores,
            diagnostic_effects,
            &decisions.tape,
            &trace,
            &missing_hyphens,
        );
    } else {
        for warning in missing_hyphens {
            crate::diagnostics::report_missing_character_warning(
                stores,
                diagnostic_effects,
                warning.font,
                warning.ch,
                false,
            );
        }
    }
    if let Some(spec) = decisions.last_line_fill {
        decisions.tape.replace_last_par_fill(spec);
    }
    let empty_list = tex_state::node_arena::PageListId::empty();
    let post_params = post_line_break_params(&params, widow_penalty_selector, empty_list);
    let mut line_count = 0i32;
    let mut last_line = None;
    let total_lines = decisions.breaks.len();
    let pdf_line_dimensions = pdf_line_dimensions(stores);
    let protrudes_chars = stores.pdf_font_configuration().protrudes_chars();
    let adjusts_spacing = stores.pdf_font_configuration().adjusts_spacing();
    // §804 labels every line packed by `post_line_break` with the horizontal
    // mode level's entry line. The value is detached before the packing loop,
    // so reporting never reaches into command or input state.
    let mut materializer =
        ArenaPostLineMaterializer::new(stores, decisions.tape, decisions.breaks, post_params);
    let mut packing_direction_scratch = Vec::new();
    let mut shaping_chars = Vec::new();
    let mut shaping_scratch = crate::box_runtime::hmode::OpenTypeShapingScratch::default();
    while let Some(mut broken) = materializer.materialize_next(stores) {
        broken.nodes = crate::box_runtime::hmode::reshape_open_type_runs_list(
            stores,
            broken.nodes,
            &mut shaping_chars,
            &mut shaping_scratch,
        );
        broken.nodes = materialize_pdf_line_list(
            stores,
            broken.nodes,
            broken.dimensions.width,
            adjusts_spacing,
            protrudes_chars,
        )?;
        let (line_nodes, pre_migrated, migrated) =
            extract_migrating_material_list(stores, broken.nodes);
        broken.nodes = line_nodes;
        // TeX82 §§174/879--882 keeps `replace_count` on an unchosen disc
        // while `short_display` examines the just-packed line, then the line
        // itself contains only the already materialized replacement nodes.
        // Retain the count in the diagnostic view and clear the immutable
        // production view so later packing and shipout cannot replay it.
        let diagnostic_nodes = broken
            .diagnostic_nodes
            .map(|nodes| filter_migrating_material_list(stores, nodes));
        let needs_physical_diagnostic = diagnostic_nodes.is_some_and(|diagnostic| {
            discretionary_diagnostics_differ_list(stores, diagnostic, broken.nodes)
        });
        let allocator_high_cell_overlap = if needs_physical_diagnostic {
            broken.allocator_high_cell_overlap
        } else {
            0
        };
        broken.nodes = clear_discretionary_replacements(stores, broken.nodes, empty_list);
        let line = crate::box_runtime::hpack_page_list_with_diagnostics(
            stores,
            diagnostic_effects,
            geometry,
            &paragraph_diagnostic_context,
            broken.nodes,
            if needs_physical_diagnostic {
                Some(diagnostic_nodes.expect("physical diagnostic was compared above"))
            } else {
                None
            },
            &mut packing_direction_scratch,
            allocator_high_cell_overlap,
            PackSpec::Exactly(broken.dimensions.width),
        );
        let mut line = line;
        line.shift = broken.dimensions.indent;
        pdf_line_dimensions.apply(&mut line, line_count as usize, total_lines);
        line_count = line_count
            .checked_add(1)
            .expect("paragraph line count exceeds i32");
        last_line = Some(line);
        append_migrated_contributions(nest, stores, pre_migrated);
        let line_node = Node::HList(line);
        append_node_to_current_list(nest, stores, diagnostic_effects, line_node, fuel)?;
        append_migrated_contributions(nest, stores, migrated);
        if let Some(penalty) = broken.penalty_after {
            let penalty = Node::Penalty(penalty);
            append_vertical_contribution(nest, stores, penalty);
        }
    }
    nest.current_list_mutation().set_prev_graf(
        params
            .prev_graf
            .checked_add(line_count)
            .expect("TeX prev_graf overflow"),
    );
    if reset_paragraph {
        reset_after_par(nest, stores, diagnostic_effects);
    }
    crate::vertical::build_page_if_outer_vertical_with_error_context(
        nest,
        stores,
        diagnostic_effects,
        &diagnostic_context.output_context,
    )?;
    Ok(ParagraphBreakResult {
        last_line,
        active_directions,
    })
}

fn discretionary_diagnostics_differ_list<G>(
    stores: &CommandContext<'_, G>,
    physical: tex_state::node_arena::PageListId,
    semantic: tex_state::node_arena::PageListId,
) -> bool {
    let mut semantic_index = 0;
    for physical_index in 0..physical.len() {
        let physical_disc = stores
            .page_node_list(physical)
            .expect("physical line remains live")
            .nodes()
            .owned_node(physical_index)
            .and_then(|node| match node {
                Node::Disc {
                    kind,
                    pre,
                    post,
                    replace,
                    physical_replace_count,
                } => Some((*kind, *pre, *post, *replace, *physical_replace_count)),
                _ => None,
            });
        let Some(physical_disc) = physical_disc else {
            continue;
        };
        let mut semantic_disc = None;
        while semantic_index < semantic.len() {
            semantic_disc = stores
                .page_node_list(semantic)
                .expect("semantic line remains live")
                .nodes()
                .owned_node(semantic_index)
                .and_then(|node| match node {
                    Node::Disc {
                        kind,
                        pre,
                        post,
                        replace,
                        physical_replace_count,
                    } => Some((*kind, *pre, *post, *replace, *physical_replace_count)),
                    _ => None,
                });
            semantic_index += 1;
            if semantic_disc.is_some() {
                break;
            }
        }
        if usize::from(physical_disc.4)
            != stores
                .page_node_list(physical_disc.3)
                .expect("discretionary replacement remains live")
                .len()
            || semantic_disc != Some(physical_disc)
        {
            return true;
        }
    }
    (semantic_index..semantic.len()).any(|index| {
        matches!(
            stores
                .page_node_list(semantic)
                .expect("semantic line remains live")
                .nodes()
                .owned_node(index),
            Some(Node::Disc { .. })
        )
    })
}

fn clear_discretionary_replacements<G>(
    stores: &mut CommandContext<'_, G>,
    source: tex_state::node_arena::PageListId,
    empty: tex_state::node_arena::PageListId,
) -> tex_state::node_arena::PageListId {
    let needs_replacement_clear = stores
        .page_node_list(source)
        .expect("semantic line remains live")
        .nodes()
        .iter()
        .any(|node| matches!(node, Node::Disc { replace, .. } if *replace != empty));
    if !needs_replacement_clear {
        return source;
    }
    let mut output = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut output);
    for index in 0..source.len() {
        let disc = stores
            .page_node_list(source)
            .expect("semantic line remains live")
            .nodes()
            .owned_node(index)
            .and_then(|node| match node {
                Node::Disc {
                    kind,
                    pre,
                    post,
                    replace,
                    physical_replace_count,
                } if *replace != empty => Some((*kind, *pre, *post, *physical_replace_count)),
                _ => None,
            });
        if let Some((kind, pre, post, physical_replace_count)) = disc {
            stores.push_page_active_list(
                &mut output,
                Node::Disc {
                    kind,
                    pre,
                    post,
                    replace: empty,
                    physical_replace_count,
                },
            );
        } else {
            stores.append_page_active_list_range(&mut output, source, index..index + 1);
        }
    }
    stores.finalize_page_active_list(&mut output)
}

fn filter_migrating_material_list<G>(
    stores: &mut CommandContext<'_, G>,
    source: tex_state::node_arena::PageListId,
) -> tex_state::node_arena::PageListId {
    let mut output = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut output);
    for index in 0..source.len() {
        let migrating = matches!(
            stores
                .page_node_list(source)
                .expect("physical line remains live")
                .nodes()
                .owned_node(index),
            Some(Node::Mark { .. } | Node::Ins { .. } | Node::Adjust(_))
        );
        if !migrating {
            stores.append_page_active_list_range(&mut output, source, index..index + 1);
        }
    }
    stores.finalize_page_active_list(&mut output)
}

fn extract_migrating_material_list<G>(
    stores: &mut CommandContext<'_, G>,
    source: tex_state::node_arena::PageListId,
) -> (
    tex_state::node_arena::PageListId,
    tex_state::node_arena::PageListId,
    tex_state::node_arena::PageListId,
) {
    let has_migrating_material = stores
        .page_node_list(source)
        .expect("line remains live")
        .nodes()
        .iter()
        .any(|node| matches!(node, Node::Mark { .. } | Node::Ins { .. } | Node::Adjust(_)));
    if !has_migrating_material {
        return (
            source,
            tex_state::node_arena::PageListId::empty(),
            tex_state::node_arena::PageListId::empty(),
        );
    }
    let mut retained = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut retained);
    for index in 0..source.len() {
        let migrating = matches!(
            stores
                .page_node_list(source)
                .expect("line remains live")
                .nodes()
                .owned_node(index),
            Some(Node::Mark { .. } | Node::Ins { .. } | Node::Adjust(_))
        );
        if !migrating {
            stores.append_page_active_list_range(&mut retained, source, index..index + 1);
        }
    }
    let retained = stores.finalize_page_active_list(&mut retained);

    let mut pre = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut pre);
    for index in 0..source.len() {
        let content = stores
            .page_node_list(source)
            .expect("line remains live")
            .nodes()
            .owned_node(index)
            .and_then(|node| match node {
                Node::Adjust(adjust) if adjust.pre => Some(adjust.content),
                _ => None,
            });
        if let Some(content) = content {
            stores.append_page_active_list(&mut pre, content);
        }
    }
    let pre = stores.finalize_page_active_list(&mut pre);

    let mut post = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut post);
    for index in 0..source.len() {
        enum PostMigration {
            Direct,
            Content(tex_state::node_arena::PageListId),
            None,
        }
        let migration = match stores
            .page_node_list(source)
            .expect("line remains live")
            .nodes()
            .owned_node(index)
        {
            Some(Node::Mark { .. } | Node::Ins { .. }) => PostMigration::Direct,
            Some(Node::Adjust(adjust)) if !adjust.pre => PostMigration::Content(adjust.content),
            _ => PostMigration::None,
        };
        match migration {
            PostMigration::Direct => {
                stores.append_page_active_list_range(&mut post, source, index..index + 1);
            }
            PostMigration::Content(content) => {
                stores.append_page_active_list(&mut post, content);
            }
            PostMigration::None => {}
        }
    }
    let post = stores.finalize_page_active_list(&mut post);
    (retained, pre, post)
}

fn append_migrated_contributions<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    nodes: tex_state::node_arena::PageListId,
) {
    if nodes.is_empty() {
        return;
    }
    if crate::vertical::is_outer_vertical(nest) {
        stores.append_page_contributions(nodes);
    } else {
        nest.current_list_mutation().append_list(stores, nodes);
    }
}

fn materialize_pdf_line_list<G>(
    stores: &mut CommandContext<'_, G>,
    mut nodes: tex_state::node_arena::PageListId,
    target: Scaled,
    adjusts_spacing: bool,
    protrudes_chars: bool,
) -> Result<tex_state::node_arena::PageListId, ExecError> {
    if adjusts_spacing {
        nodes = apply_line_expansion_list(stores, nodes, target)?;
    }
    if protrudes_chars {
        let plan = tex_typeset::protrusion::plan_margin_kerns(
            &crate::typeset_context::TypesetContext::new(stores),
            stores
                .page_node_list(nodes)
                .expect("finalized line belongs to the live page arena")
                .nodes(),
        );
        if plan.left.is_some() || plan.right.is_some() {
            let mut left = plan.left;
            let mut right = plan.right;
            let mut output = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
            stores.open_page_active_list(&mut output);
            for index in 0..=nodes.len() {
                if left.as_ref().is_some_and(|(at, _)| *at == index) {
                    let (_, node) = left.take().expect("matched left margin kern");
                    stores.push_page_active_list(&mut output, node);
                }
                if right.as_ref().is_some_and(|(at, _)| *at == index) {
                    let (_, node) = right.take().expect("matched right margin kern");
                    stores.push_page_active_list(&mut output, node);
                }
                if index < nodes.len() {
                    stores.append_page_active_list_range(&mut output, nodes, index..index + 1);
                }
            }
            nodes = stores.finalize_page_active_list(&mut output);
        }
    }
    Ok(nodes)
}

fn apply_line_expansion_list<G>(
    stores: &mut CommandContext<'_, G>,
    nodes: tex_state::node_arena::PageListId,
    target: Scaled,
) -> Result<tex_state::node_arena::PageListId, ExecError> {
    let line_ratio = tex_typeset::linebreak::plan_line_expansion_cursor(
        &crate::typeset_context::TypesetContext::new(stores),
        stores
            .page_node_list(nodes)
            .expect("line expansion source belongs to the live page arena")
            .nodes(),
        target,
    );
    if line_ratio == 0 {
        return Ok(nodes);
    }
    let mut output = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut output);
    for index in 0..nodes.len() {
        let replacement = expanded_line_node(stores, nodes, index, line_ratio)?;
        if let Some(replacement) = replacement {
            stores.push_page_active_list(&mut output, replacement);
        } else {
            stores.append_page_active_list_range(&mut output, nodes, index..index + 1);
        }
    }
    Ok(stores.finalize_page_active_list(&mut output))
}

fn expanded_line_node<G>(
    stores: &mut CommandContext<'_, G>,
    nodes: tex_state::node_arena::PageListId,
    index: usize,
    line_ratio: i32,
) -> Result<Option<Node>, ExecError> {
    let glyph = stores
        .page_node_list(nodes)
        .expect("line expansion source belongs to the live page arena")
        .nodes()
        .owned_node(index)
        .and_then(|node| match node {
            Node::Char { font, ch, .. } | Node::Lig { font, ch, .. } => Some((*font, *ch)),
            _ => None,
        });
    if let Some((font, ch)) = glyph {
        let code = u8::try_from(u32::from(ch)).ok();
        let Some(code) = code else { return Ok(None) };
        let Some(configured) = stores.font_expansion(font) else {
            return Ok(None);
        };
        let spec = tex_typeset::expansion::FontExpansionSpec::new(
            i32::from(configured.stretch),
            i32::from(configured.shrink),
            i32::from(configured.step),
            configured.auto_expand,
        )
        .expect("live font expansion settings are validated");
        let ratio = spec.discrete_ratio(
            line_ratio,
            stores.pdf_font_code(PdfFontCode::Ef, font, code),
        );
        let expanded = stores.try_expanded_font(font, ratio)?;
        if expanded == font {
            return Ok(None);
        }
        let replacement = match stores
            .page_node_list(nodes)
            .expect("expanded glyph source remains live")
            .nodes()
            .owned_node(index)
            .expect("expanded glyph index remains in bounds")
        {
            Node::Char { ch, origin, .. } => Node::Char {
                font: expanded,
                ch: *ch,
                origin: *origin,
            },
            Node::Lig {
                ch,
                orig,
                left_hit,
                right_hit,
                origins,
                ..
            } => Node::Lig {
                font: expanded,
                ch: *ch,
                orig: orig.clone(),
                left_hit: *left_hit,
                right_hit: *right_hit,
                origins: origins.clone(),
            },
            _ => unreachable!("glyph expansion plan targets a glyph"),
        };
        return Ok(Some(replacement));
    }
    if index == 0 || index + 1 >= nodes.len() {
        return Ok(None);
    }
    let is_font_kern = matches!(
        stores
            .page_node_list(nodes)
            .expect("line expansion source belongs to the live page arena")
            .nodes()
            .owned_node(index),
        Some(Node::Kern {
            kind: KernKind::Font,
            ..
        })
    );
    if !is_font_kern {
        return Ok(None);
    }
    let glyph_at = |index| {
        stores
            .page_node_list(nodes)
            .expect("line expansion source belongs to the live page arena")
            .nodes()
            .owned_node(index)
            .and_then(glyph_identity)
    };
    let (Some((left_font, left)), Some((right_font, right))) =
        (glyph_at(index - 1), glyph_at(index + 1))
    else {
        return Ok(None);
    };
    if left_font != right_font {
        return Ok(None);
    }
    Ok(
        match stores.font_lig_kern_command(
            left_font,
            tex_fonts::LigKernChar::Char(left),
            tex_fonts::LigKernChar::Char(right),
        ) {
            Some(tex_fonts::LigKernCommand::Kern(amount)) => Some(Node::Kern {
                amount,
                kind: KernKind::Font,
            }),
            _ => None,
        },
    )
}

/// TeX82 §825: paragraph glue may shrink only at normal order. Each
/// offending specification is copied and normalized, while recovery is
/// reported at most once for the whole paragraph.
fn normalize_paragraph_infinite_shrink<G>(
    stores: &mut CommandContext<'_, G>,
    params: &mut ParagraphParams,
    nodes: tex_state::node_arena::PageListId,
    tracing: bool,
    diagnostic_context: &crate::pack_report::ExecutionDiagnosticContext,
    diagnostic_effects: &mut DiagnosticEffects,
) -> Result<tex_state::node_arena::PageListId, ExecError> {
    let mut reported = false;
    normalize_paragraph_glue(
        stores,
        &mut params.left_skip,
        tracing,
        diagnostic_context,
        diagnostic_effects,
        &mut reported,
    )?;
    normalize_paragraph_glue(
        stores,
        &mut params.right_skip,
        tracing,
        diagnostic_context,
        diagnostic_effects,
        &mut reported,
    )?;
    let mut output = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut output);
    for index in 0..nodes.len() {
        let replacement = match stores
            .page_node_list(nodes)
            .expect("paragraph belongs to the live page arena")
            .nodes()
            .owned_node(index)
        {
            Some(Node::Glue { spec, kind, leader })
                if spec.shrink.raw() != 0 && spec.shrink_order != Order::Normal =>
            {
                Some((*spec, *kind, *leader))
            }
            _ => None,
        };
        if let Some((mut spec, kind, leader)) = replacement {
            normalize_paragraph_glue(
                stores,
                &mut spec,
                tracing,
                diagnostic_context,
                diagnostic_effects,
                &mut reported,
            )?;
            stores.push_page_active_list(&mut output, Node::Glue { spec, kind, leader });
        } else {
            stores.append_page_active_list_range(&mut output, nodes, index..index + 1);
        }
    }
    Ok(stores.finalize_page_active_list(&mut output))
}

fn normalize_paragraph_glue<G>(
    stores: &mut CommandContext<'_, G>,
    spec: &mut tex_state::glue::GlueSpec,
    tracing: bool,
    diagnostic_context: &crate::pack_report::ExecutionDiagnosticContext,
    diagnostic_effects: &mut DiagnosticEffects,
    reported: &mut bool,
) -> Result<(), ExecError> {
    if spec.shrink.raw() == 0 || spec.shrink_order == Order::Normal {
        return Ok(());
    }
    if !*reported {
        if tracing {
            stores.begin_diagnostic(diagnostic_effects).end(true);
            stores.publish_diagnostic_effects_before_synchronous_print(diagnostic_effects);
        }
        crate::diagnostics::report_paragraph_infinite_shrinkage(
            stores,
            diagnostic_effects,
            diagnostic_context,
        )?;
        *reported = true;
    }
    spec.shrink_order = Order::Normal;
    Ok(())
}

fn glyph_identity(node: &Node) -> Option<(tex_state::ids::FontId, u8)> {
    match node {
        Node::Char { font, ch, .. } | Node::Lig { font, ch, .. } => {
            u8::try_from(u32::from(*ch)).ok().map(|code| (*font, code))
        }
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct PdfLineDimensions {
    ignored: Scaled,
    first_height: Scaled,
    last_depth: Scaled,
    each_height: Scaled,
    each_depth: Scaled,
}

impl PdfLineDimensions {
    fn apply(self, line: &mut tex_state::node::BoxNode, index: usize, total: usize) {
        if self.each_height != self.ignored {
            line.height = self.each_height;
        }
        if self.each_depth != self.ignored {
            line.depth = self.each_depth;
        }
        if index == 0 && self.first_height != self.ignored {
            line.height = self.first_height;
        }
        if index + 1 == total && self.last_depth != self.ignored {
            line.depth = self.last_depth;
        }
    }
}

fn pdf_line_dimensions<G>(stores: &mut CommandContext<'_, G>) -> PdfLineDimensions {
    PdfLineDimensions {
        ignored: stores.dimen_param(DimenParam::PDF_IGNORED_DIMEN),
        first_height: stores.dimen_param(DimenParam::PDF_FIRST_LINE_HEIGHT),
        last_depth: stores.dimen_param(DimenParam::PDF_LAST_LINE_DEPTH),
        each_height: stores.dimen_param(DimenParam::PDF_EACH_LINE_HEIGHT),
        each_depth: stores.dimen_param(DimenParam::PDF_EACH_LINE_DEPTH),
    }
}

fn active_text_directions(nodes: tex_state::node_arena::NodeCursor<'_>) -> Vec<Direction> {
    let mut active = Vec::new();
    for node in nodes {
        match node {
            Node::Direction(direction @ (Direction::BeginL | Direction::BeginR)) => {
                active.push(*direction);
            }
            Node::Direction(Direction::EndL) if active.last() == Some(&Direction::BeginL) => {
                let _ = active.pop();
            }
            Node::Direction(Direction::EndR) if active.last() == Some(&Direction::BeginR) => {
                let _ = active.pop();
            }
            _ => {}
        }
    }
    active
}

fn break_hlist_with_trace<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    hlist: tex_state::node_arena::PageListId,
    line_params: LineBreakParams,
    fuel: &mut tex_command::CommandFuel,
    tracing: bool,
) -> Result<
    (
        LineBreakResult,
        Vec<LineBreakTrace>,
        Vec<super::hyphenation::MissingHyphenDiagnostic>,
    ),
    ExecError,
> {
    // TeX82 §815 skips the pretolerance pass when `pretolerance<0` and
    // enters the hyphenating second pass directly. §919 initializes the trie
    // at that boundary, even if Umber's pure non-hyphenating planner can
    // already find a layout for the same paragraph.
    if line_params.pretolerance < 0 {
        stores.close_hyphenation_patterns();
    }
    let tape = ParagraphTape::analyze_arena_id(
        &crate::typeset_context::TypesetContext::new(stores),
        hlist,
        &line_params,
    );
    let (first, trace) = if tracing {
        try_tape_without_hyphenation_traced(
            &crate::typeset_context::TypesetContext::new(stores),
            &tape,
            &line_params,
        )
    } else {
        (
            cached_pretolerance_plan(stores, &tape, &line_params),
            Vec::new(),
        )
    };
    if let Some(first) = first {
        Ok((
            tex_typeset::linebreak::plan_with_tape(first, tape),
            trace,
            Vec::new(),
        ))
    } else {
        drop(tape);
        let hyphenated = super::hyphenation::hyphenated_hlist_with_fuel(
            stores,
            diagnostic_effects,
            hlist,
            fuel,
        )?;
        let tape = ParagraphTape::analyze_arena_projection_ids(
            &crate::typeset_context::TypesetContext::new(stores),
            hyphenated.semantic,
            hyphenated.physical,
            Some(hyphenated.physical_boundaries),
            &line_params,
        );
        let (plan, trace) = if tracing {
            break_hyphenated_tape_traced(
                &crate::typeset_context::TypesetContext::new(stores),
                &tape,
                &line_params,
                trace,
            )
        } else {
            (
                break_hyphenated_tape(
                    &crate::typeset_context::TypesetContext::new(stores),
                    &tape,
                    &line_params,
                ),
                trace,
            )
        };
        Ok((
            tex_typeset::linebreak::plan_with_tape(plan, tape),
            trace,
            hyphenated.missing_hyphens,
        ))
    }
}

fn report_line_break_trace<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    tape: &ParagraphTape<'_>,
    trace: &[LineBreakTrace],
    missing_hyphens: &[super::hyphenation::MissingHyphenDiagnostic],
) {
    let missing_hyphens = if stores.int_param(IntParam::TRACING_LOST_CHARS) > 0 {
        missing_hyphens
            .iter()
            .map(|warning| {
                (
                    warning.node_index,
                    stores.font_name(warning.font),
                    warning.ch,
                )
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let rendered_trace = {
        let typeset = crate::typeset_context::TypesetContext::new(stores);
        let nodes = tape.nodes(&typeset);
        let mut short_display = crate::pack_report::ShortDisplayRenderer::new();
        trace
            .iter()
            .map(|event| match event {
                LineBreakTrace::Pass(_) => {
                    short_display.reset();
                    (None, None)
                }
                LineBreakTrace::Feasible {
                    display,
                    display_suffix,
                    ..
                } if !display.is_empty() => (
                    Some(short_display.render_node_range(stores, nodes, display.clone())),
                    display_suffix.as_ref().map(|suffix| {
                        short_display.render_line_break_trace_suffix(stores, *suffix)
                    }),
                ),
                _ => (None, None),
            })
            .collect::<Vec<_>>()
    };
    let mut diagnostic = stores.begin_diagnostic(diagnostic_effects);
    let mut next_warning = 0;
    for (event, (rendered_display, rendered_suffix)) in trace.iter().zip(rendered_trace) {
        if let LineBreakTrace::Feasible { display, .. } = event {
            while missing_hyphens
                .get(next_warning)
                .is_some_and(|(node_index, _, _)| *node_index < display.end)
            {
                let (_, font_name, ch) = &missing_hyphens[next_warning];
                diagnostic
                    .print_nl("Missing character: There is no ")
                    .print_ascii(*ch)
                    .print(" in font ")
                    .print(font_name)
                    .print_char('!');
                next_warning += 1;
            }
        }
        match event {
            LineBreakTrace::Pass(pass) => {
                // TeX82 §851 creates the initial active node once per pass
                // and resets `font_in_short_display` there. Feasible-break
                // fragments within the pass retain the selected font.
                diagnostic.print_nl(match pass {
                    LineBreakPass::First => "@firstpass",
                    LineBreakPass::Second => "@secondpass",
                    LineBreakPass::Emergency => "@emergencypass",
                });
            }
            LineBreakTrace::Feasible {
                breakpoint,
                via,
                badness,
                penalty,
                demerits,
                ..
            } => {
                if let Some(rendered) = rendered_display {
                    diagnostic.print_nl("").print_rendered(&rendered);
                }
                if let Some(rendered) = rendered_suffix {
                    diagnostic.print_rendered(&rendered);
                }
                diagnostic.print_nl("@");
                match breakpoint {
                    TraceBreakpoint::Glue => {}
                    TraceBreakpoint::Penalty => {
                        diagnostic.print_esc("penalty");
                    }
                    TraceBreakpoint::Discretionary => {
                        diagnostic.print_esc("discretionary");
                    }
                    TraceBreakpoint::Kern => {
                        diagnostic.print_esc("kern");
                    }
                    TraceBreakpoint::Math => {
                        diagnostic.print_esc("math");
                    }
                    TraceBreakpoint::Paragraph => {
                        diagnostic.print_esc("par");
                    }
                }
                diagnostic
                    .print(" via @@")
                    .print_int(*via as i32)
                    .print(" b=");
                if let Some(value) = badness {
                    diagnostic.print_int(*value);
                } else {
                    diagnostic.print_char('*');
                }
                diagnostic.print(" p=").print_int(*penalty).print(" d=");
                if let Some(value) = demerits {
                    diagnostic.print_int(*value);
                } else {
                    diagnostic.print_char('*');
                }
            }
            LineBreakTrace::Active {
                serial,
                line,
                fitness,
                hyphenated,
                total_demerits,
                last_line_fit,
                previous,
            } => {
                diagnostic
                    .print_nl("@@")
                    .print_int(*serial as i32)
                    .print(": line ")
                    .print_int(*line as i32)
                    .print_char('.')
                    .print_int(*fitness);
                if *hyphenated {
                    diagnostic.print_char('-');
                }
                diagnostic.print(" t=").print_int(*total_demerits);
                if let Some(last_line_fit) = last_line_fit {
                    // e-TeX change-file section 38.846 reports the additional
                    // active-node words whenever last-line fitting is active.
                    diagnostic
                        .print(" s=")
                        .print_scaled(last_line_fit.shortfall);
                    diagnostic
                        .print(if last_line_fit.terminal { " a=" } else { " g=" })
                        .print_scaled(last_line_fit.glue);
                }
                diagnostic.print(" -> @@").print_int(*previous as i32);
            }
        }
    }
    for (_, font_name, ch) in &missing_hyphens[next_warning..] {
        diagnostic
            .print_nl("Missing character: There is no ")
            .print_ascii(*ch)
            .print(" in font ")
            .print(font_name)
            .print_char('!');
    }
    diagnostic.end(true);
}

/// Computes the pure pretolerance line-breaking plan.
///
/// The public name is retained for callers whose outer execution service
/// memoizes this pure result. The admitted hot kernel itself owns no memo
/// runtime or generation-crossing cache values.
pub fn cached_pretolerance_plan<G>(
    stores: &mut CommandContext<'_, G>,
    tape: &ParagraphTape<'_>,
    line_params: &LineBreakParams,
) -> Option<tex_typeset::linebreak::BreakPlan> {
    tex_typeset::linebreak::try_tape_without_hyphenation(
        &crate::typeset_context::TypesetContext::new(stores),
        tape,
        line_params,
    )
}

fn snapshot_paragraph_params<G>(
    nest: &ModeNest,
    stores: &mut CommandContext<'_, G>,
) -> ParagraphParams {
    ParagraphParams {
        left_skip: glue_parameter_value(stores, GlueParam::LEFT_SKIP),
        right_skip: glue_parameter_value(stores, GlueParam::RIGHT_SKIP),
        par_fill_skip: glue_parameter_value(stores, GlueParam::PAR_FILL_SKIP),
        par_shape: stores.paragraph_shape(),
        prev_graf: nest.enclosing_vertical_prev_graf(),
        hang_indent: stores.dimen_param(DimenParam::HANG_INDENT),
        hang_after: stores.int_param(IntParam::HANG_AFTER),
        looseness: stores.int_param(IntParam::LOOSENESS),
        pretolerance: stores.int_param(IntParam::PRETOLERANCE),
        tolerance: stores.int_param(IntParam::TOLERANCE),
        line_penalty: stores.int_param(IntParam::LINE_PENALTY),
        hyphen_penalty: stores.int_param(IntParam::HYPHEN_PENALTY),
        ex_hyphen_penalty: stores.int_param(IntParam::EX_HYPHEN_PENALTY),
        adj_demerits: stores.int_param(IntParam::ADJ_DEMERITS),
        double_hyphen_demerits: stores.int_param(IntParam::DOUBLE_HYPHEN_DEMERITS),
        final_hyphen_demerits: stores.int_param(IntParam::FINAL_HYPHEN_DEMERITS),
        last_line_fit: stores.int_param(IntParam::LAST_LINE_FIT),
        emergency_stretch: stores.dimen_param(DimenParam::EMERGENCY_STRETCH),
        hsize: stores.dimen_param(DimenParam::H_SIZE),
        interline_penalty: stores.int_param(IntParam::INTERLINE_PENALTY),
        club_penalty: stores.int_param(IntParam::CLUB_PENALTY),
        widow_penalty: stores.int_param(IntParam::WIDOW_PENALTY),
        display_widow_penalty: stores.int_param(IntParam::DISPLAY_WIDOW_PENALTY),
        broken_penalty: stores.int_param(IntParam::BROKEN_PENALTY),
        interline_penalties: stores.penalty_array(PenaltyArrayKind::InterLine),
        club_penalties: stores.penalty_array(PenaltyArrayKind::Club),
        widow_penalties: stores.penalty_array(PenaltyArrayKind::Widow),
        display_widow_penalties: stores.penalty_array(PenaltyArrayKind::DisplayWidow),
    }
}

fn line_break_params<G>(
    stores: &CommandContext<'_, G>,
    params: &ParagraphParams,
) -> LineBreakParams {
    LineBreakParams {
        pretolerance: params.pretolerance,
        tolerance: params.tolerance,
        line_penalty: params.line_penalty,
        hyphen_penalty: params.hyphen_penalty,
        ex_hyphen_penalty: params.ex_hyphen_penalty,
        adj_demerits: params.adj_demerits,
        double_hyphen_demerits: params.double_hyphen_demerits,
        final_hyphen_demerits: params.final_hyphen_demerits,
        last_line_fit: params.last_line_fit,
        pdf_adjust_spacing: stores.int_param(IntParam::PDF_ADJUST_SPACING),
        expansion_steps: None,
        pdf_protrude_chars: stores.int_param(IntParam::PDF_PROTRUDE_CHARS),
        emergency_stretch: params.emergency_stretch,
        looseness: params.looseness,
        left_skip: params.left_skip,
        right_skip: params.right_skip,
        par_fill_skip: params.par_fill_skip,
        shape: line_shape(params),
    }
}

fn post_line_break_params(
    params: &ParagraphParams,
    widow_penalty_selector: tex_typeset::linebreak::WidowPenaltySelector,
    empty_list: tex_state::node_arena::PageListId,
) -> PostLineBreakParams {
    PostLineBreakParams {
        empty_list,
        left_skip: params.left_skip,
        right_skip: params.right_skip,
        interline_penalty: params.interline_penalty,
        club_penalty: params.club_penalty,
        widow_penalties: tex_typeset::linebreak::WidowPenalties {
            selector: widow_penalty_selector,
            ordinary: tex_typeset::linebreak::PenaltySequence {
                fallback: params.widow_penalty,
                values: params.widow_penalties.clone(),
            },
            display: tex_typeset::linebreak::PenaltySequence {
                fallback: params.display_widow_penalty,
                values: params.display_widow_penalties.clone(),
            },
        },
        broken_penalty: params.broken_penalty,
        prev_graf: params.prev_graf,
        interline_penalties: params.interline_penalties.clone(),
        club_penalties: params.club_penalties.clone(),
        shape: line_shape(params),
    }
}

fn line_shape(params: &ParagraphParams) -> LineShape {
    LineShape {
        hsize: params.hsize,
        parshape: (!params.par_shape.is_empty()).then(|| TypesetParagraphShape {
            lines: params
                .par_shape
                .iter()
                .map(|line| LineShapeEntry {
                    indent: line.indent,
                    width: line.width,
                })
                .collect(),
        }),
        hang_indent: params.hang_indent,
        hang_after: params.hang_after,
        line_offset: params.prev_graf.max(0) as usize,
    }
}

pub(crate) fn normal_paragraph<G>(
    _nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
) {
    // e-TeX [47.1070] resets a non-null interline array through `eq_define`,
    // so [19.277]'s generic assignment hook traces the local write. The club
    // and widow arrays retain their scoped assignments (manual §3.4).
    let interline_penalties = stores.penalty_array(PenaltyArrayKind::InterLine);
    if !interline_penalties.is_empty() {
        stores
            .assign_penalty_array(
                PenaltyArrayKind::InterLine,
                &[],
                tex_state::AssignmentScope::Local,
            )
            .expect("paragraph reset targets admitted state");
        crate::assignments::tracing::trace_penalty_array(
            stores,
            diagnostic_effects,
            PenaltyArrayKind::InterLine,
            false,
            &interline_penalties,
            &[],
        );
    }
    // TeX82 §1090 saves these eqtb entries in this exact order. Section 283
    // unwinds them in reverse, so `\parshape` is traced before `\hangafter`,
    // `\hangindent`, and `\looseness`.
    if stores.int_param(IntParam::LOOSENESS) != 0 {
        stores
            .assign_int_param(IntParam::LOOSENESS, 0, tex_state::AssignmentScope::Local)
            .expect("paragraph reset targets admitted state");
    }
    if stores.dimen_param(DimenParam::HANG_INDENT).raw() != 0 {
        stores
            .assign_dimen_param(
                DimenParam::HANG_INDENT,
                Scaled::from_raw(0),
                tex_state::AssignmentScope::Local,
            )
            .expect("paragraph reset targets admitted state");
    }
    if stores.int_param(IntParam::HANG_AFTER) != 1 {
        stores
            .assign_int_param(IntParam::HANG_AFTER, 1, tex_state::AssignmentScope::Local)
            .expect("paragraph reset targets admitted state");
    }
    stores
        .assign_paragraph_shape(&[], tex_state::AssignmentScope::Local)
        .expect("paragraph reset targets admitted state");
}

pub(crate) fn start_paragraph<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    indent: bool,
    diagnostic_context: &crate::pack_report::ExecutionDiagnosticContext,
) -> Result<(), ExecError> {
    match nest.current_mode() {
        crate::Mode::Vertical | crate::Mode::InternalVertical => {
            nest.set_enclosing_vertical_prev_graf(0);
            let parskip = glue_parameter_value(stores, GlueParam::PAR_SKIP);
            if nest.current_mode() == crate::Mode::Vertical || !nest.current_list().is_empty() {
                append_vertical_contribution(
                    nest,
                    stores,
                    Node::Glue {
                        spec: parskip,
                        kind: GlueKind::ParSkip,
                        leader: None,
                    },
                );
                crate::vertical::build_page_if_outer_vertical_with_error_context(
                    nest,
                    stores,
                    diagnostic_effects,
                    &diagnostic_context.output_context,
                )?;
            }
            nest.push_at_line(crate::Mode::Horizontal, diagnostic_context.current_line)?;
            let (language, left, right) = crate::box_runtime::hmode::current_hyphen_context(stores);
            nest.current_list_mutation()
                .set_hyphen_context(language, left, right);
            if indent {
                let mut fuel = tex_command::CommandFuelLedger::default();
                crate::box_runtime::indent_in_hmode(
                    nest,
                    stores,
                    diagnostic_effects,
                    true,
                    fuel.fuel_mut(),
                )?;
            }
            Ok(())
        }
        mode => Err(ExecError::UnimplementedTypesetting {
            mode,
            token: tex_state::token::Token::Cs(stores.intern_control_sequence("par")),
            origin: tex_state::token::OriginId::UNKNOWN,
            operation: "canonical paragraph start",
        }),
    }
}

fn reset_after_par<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
) {
    normal_paragraph(nest, stores, diagnostic_effects);
}

fn glue_parameter_value<G>(stores: &CommandContext<'_, G>, parameter: GlueParam) -> GlueSpec {
    stores
        .glue_param(parameter)
        .map_or(GlueSpec::ZERO, |id| stores.glue(id))
}

#[cfg(test)]
mod tests;
use tex_state::diagnostic::DiagnosticEffects;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam};
use tex_state::font::PdfFontCode;
use tex_state::glue::{GlueSpec, Order};
use tex_state::node::{BoxNode, Direction, GlueKind, KernKind, Node};
use tex_state::scaled::Scaled;
use tex_state::{CommandContext, PenaltyArrayKind};
use tex_typeset::PackSpec;
use tex_typeset::linebreak::{
    LineBreakParams, LineBreakPass, LineBreakResult, LineBreakTrace, LineDimensions, LineShape,
    LineShapeEntry, ParagraphShape as TypesetParagraphShape, ParagraphTape, PostLineBreakParams,
    TraceBreakpoint, break_hyphenated_tape, break_hyphenated_tape_traced,
    try_tape_without_hyphenation_traced,
};

use crate::box_runtime::{
    append_node_to_current_list, commit_current_list, flush_pending_hchars_with_fuel,
};
use crate::mode::ParagraphParams;
use crate::vertical::append_vertical_contribution;
use crate::{ExecError, ModeNest};
