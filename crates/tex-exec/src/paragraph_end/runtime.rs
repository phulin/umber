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

pub(crate) fn break_current_paragraph<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    widow_penalty_selector: tex_typeset::linebreak::WidowPenaltySelector,
    reset_paragraph: bool,
    diagnostic_context: crate::pack_report::ExecutionDiagnosticContext,
    fuel: &mut tex_command::CommandFuel,
) -> Result<ParagraphBreakResult, ExecError> {
    flush_pending_hchars_with_fuel(nest, stores, fuel)?;
    let active_directions = active_text_directions(nest.current_list().nodes());
    let mut params = snapshot_paragraph_params(nest, stores);
    {
        let mut list = nest.current_list_mutation();
        if matches!(list.nodes().last(), Some(Node::Glue { .. })) {
            let _ = list.pop_last_node();
        }
    }
    nest.current_list_mutation().push(Node::Penalty(10_000));
    nest.current_list_mutation().push(Node::Glue {
        spec: params.par_fill_skip,
        kind: GlueKind::ParFillSkip,
        leader: None,
    });
    let mut level = commit_current_list(nest, stores, fuel)?;
    let paragraph_diagnostic_context = diagnostic_context.with_pack_begin_line(level.entry_line());
    let mut hlist =
        crate::math::finish_math_lists_owned(stores, level.list_mutation().take_nodes(), true);
    let tracing = stores.int_param(IntParam::TRACING_PARAGRAPHS) > 0;
    normalize_paragraph_infinite_shrink(
        stores,
        &mut params,
        &mut hlist,
        tracing,
        Some(diagnostic_context.output_context.clone()),
    )?;
    let mut line_params = line_break_params(stores, &params);
    if line_params.pdf_adjust_spacing > 1 {
        line_params.expansion_steps = tex_typeset::linebreak::validate_paragraph_expansion(
            &crate::typeset_context::TypesetContext::new(stores),
            &hlist,
        )?;
    }
    let (mut decisions, trace, missing_hyphens) =
        break_hlist_with_trace(stores, hlist, line_params, fuel, tracing)?;
    if tracing {
        report_line_break_trace(stores, decisions.tape.nodes(), &trace, &missing_hyphens);
    } else {
        for warning in missing_hyphens {
            crate::diagnostics::report_missing_character_warning(
                stores,
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
    let post_params = post_line_break_params(&params, widow_penalty_selector, empty_list.clone());
    let mut line_count = 0i32;
    let mut last_line = None;
    let total_lines = decisions.breaks.len();
    let pdf_line_dimensions = pdf_line_dimensions(stores);
    let protrudes_chars = stores.pdf_font_configuration().protrudes_chars();
    let adjusts_spacing = stores.pdf_font_configuration().adjusts_spacing();
    // §804 labels every line packed by `post_line_break` with the horizontal
    // mode level's entry line. The value is detached before the packing loop,
    // so reporting never reaches into command or input state.
    let mut materializer = LineMaterializer::new(decisions.tape, decisions.breaks, post_params);
    let mut line_nodes = Vec::new();
    let mut migrated = Vec::new();
    let mut pre_migrated = Vec::new();
    let mut retained_migrated = Vec::new();
    while let Some(mut broken) = materializer.materialize_next(
        &crate::typeset_context::TypesetContext::new(stores),
        line_nodes,
    ) {
        crate::box_runtime::hmode::reshape_open_type_runs(stores, &mut broken.nodes);
        materialize_pdf_line(
            stores,
            &mut broken.nodes,
            broken.dimensions.width,
            adjusts_spacing,
            protrudes_chars,
        )?;
        extract_migrating_material(
            stores,
            &mut broken.nodes,
            &mut pre_migrated,
            &mut migrated,
            &mut retained_migrated,
        );
        // TeX82 §§174/879--882 keeps `replace_count` on an unchosen disc
        // while `short_display` examines the just-packed line, then the line
        // itself contains only the already materialized replacement nodes.
        // Retain the count in the diagnostic view and clear the immutable
        // production view so later packing and shipout cannot replay it.
        let mut diagnostic_nodes = broken
            .physical_nodes
            .into_iter()
            .filter(|node| !matches!(node, Node::Mark { .. } | Node::Ins { .. } | Node::Adjust(_)))
            .collect::<Vec<_>>();
        let needs_physical_diagnostic =
            discretionary_diagnostics_differ(stores, &diagnostic_nodes, &broken.nodes);
        let allocator_high_cell_overlap = if needs_physical_diagnostic {
            tex_state::node_sequence::direct_high_cell_overlap(
                &broken.high_cell_lineages,
                &broken.physical_high_cell_lineages,
            )
        } else {
            0
        };
        for node in &mut broken.nodes {
            if let Node::Disc { replace, .. } = node {
                *replace = empty_list.clone();
            }
        }
        let line = hpack_owned_with_overfull_rule(
            stores,
            &paragraph_diagnostic_context,
            &mut broken.nodes,
            needs_physical_diagnostic.then_some(&mut diagnostic_nodes),
            allocator_high_cell_overlap,
            PackSpec::Exactly(broken.dimensions.width),
        );
        let mut line = line;
        line.shift = broken.dimensions.indent;
        pdf_line_dimensions.apply(&mut line, line_count as usize, total_lines);
        line_count = line_count
            .checked_add(1)
            .expect("paragraph line count exceeds i32");
        last_line = Some(line.clone());
        for node in pre_migrated.drain(..) {
            append_migrated_contribution(nest, stores, node);
        }
        let line_node = Node::HList(line);
        append_node_to_current_list(nest, stores, line_node, fuel)?;
        for node in migrated.drain(..) {
            append_migrated_contribution(nest, stores, node);
        }
        retained_migrated.clear();
        if let Some(penalty) = broken.penalty_after {
            let penalty = Node::Penalty(penalty);
            append_vertical_contribution(nest, stores, penalty);
        }
        line_nodes = broken.nodes;
    }
    nest.current_list_mutation().set_prev_graf(
        params
            .prev_graf
            .checked_add(line_count)
            .expect("TeX prev_graf overflow"),
    );
    if reset_paragraph {
        reset_after_par(nest, stores);
    }
    crate::vertical::build_page_if_outer_vertical_with_error_context(
        nest,
        stores,
        &diagnostic_context.output_context,
    )?;
    Ok(ParagraphBreakResult {
        last_line,
        active_directions,
    })
}

/// Whether §663 needs the paragraph's TeX-physical discretionary projection.
///
/// Equal replacement counts do not imply equal diagnostics: ligature
/// reconstitution can change pre/post branches while leaving the count
/// unchanged. Compare the ordered discretionary records explicitly, while
/// retaining the count-vs-side-list guard for flattened physical topology.
fn discretionary_diagnostics_differ<G>(
    stores: &CommandContext<'_, G>,
    physical: &[Node],
    semantic: &[Node],
) -> bool {
    let mut semantic = semantic.iter().filter_map(|node| match node {
        Node::Disc {
            kind,
            pre,
            post,
            replace,
            physical_replace_count,
        } => Some((
            *kind,
            pre.clone(),
            post.clone(),
            replace.clone(),
            *physical_replace_count,
        )),
        _ => None,
    });
    for node in physical {
        let Node::Disc {
            kind,
            pre,
            post,
            replace,
            physical_replace_count,
        } = node
        else {
            continue;
        };
        if usize::from(*physical_replace_count)
            != stores
                .page_node_list(replace.clone())
                .expect("discretionary replacement belongs to the live page arena")
                .len()
            || semantic.next()
                != Some((
                    *kind,
                    pre.clone(),
                    post.clone(),
                    replace.clone(),
                    *physical_replace_count,
                ))
        {
            return true;
        }
    }
    semantic.next().is_some()
}

fn materialize_pdf_line<G>(
    stores: &mut CommandContext<'_, G>,
    nodes: &mut Vec<Node>,
    target: Scaled,
    adjusts_spacing: bool,
    protrudes_chars: bool,
) -> Result<(), ExecError> {
    if adjusts_spacing {
        apply_line_expansion(stores, nodes, target)?;
    }
    if protrudes_chars {
        tex_typeset::protrusion::insert_margin_kerns(
            &crate::typeset_context::TypesetContext::new(stores),
            nodes,
        );
    }
    Ok(())
}

/// TeX82 §825: paragraph glue may shrink only at normal order. Each
/// offending specification is copied and normalized, while recovery is
/// reported at most once for the whole paragraph.
fn normalize_paragraph_infinite_shrink<G>(
    stores: &mut CommandContext<'_, G>,
    params: &mut ParagraphParams,
    nodes: &mut [Node],
    tracing: bool,
    mut error_context: Option<String>,
) -> Result<(), ExecError> {
    let mut reported = false;
    let mut normalize = |spec: &mut tex_state::glue::GlueSpec| -> Result<(), ExecError> {
        let mut glue = *spec;
        if glue.shrink.raw() == 0 || glue.shrink_order == Order::Normal {
            return Ok(());
        }
        if !reported {
            if tracing {
                // TeX82 §825 temporarily closes the active paragraph
                // diagnostic with `end_diagnostic(true)` before `print_err`.
                // Umber materializes the detached trace later, but must keep
                // this print-channel boundary at the recovery point.
                stores.begin_diagnostic().end(true);
            }
            crate::diagnostics::report_paragraph_infinite_shrinkage(
                stores,
                error_context
                    .take()
                    .expect("paragraph completion owns its live error context"),
            )?;
            reported = true;
        }
        glue.shrink_order = Order::Normal;
        *spec = glue;
        Ok(())
    };

    normalize(&mut params.left_skip)?;
    normalize(&mut params.right_skip)?;
    for node in nodes {
        if let Node::Glue { spec, .. } = node {
            normalize(spec)?;
        }
    }
    Ok(())
}

pub(crate) fn apply_line_expansion<G>(
    stores: &mut CommandContext<'_, G>,
    nodes: &mut [Node],
    target: Scaled,
) -> Result<(), ExecError> {
    let line_ratio = tex_typeset::linebreak::plan_line_expansion(
        &crate::typeset_context::TypesetContext::new(stores),
        nodes,
        target,
    );
    if line_ratio == 0 {
        return Ok(());
    }
    for node in nodes.iter_mut() {
        let Some((font, code)) = glyph_identity(node) else {
            continue;
        };
        let Some(configured) = stores.font_expansion(font) else {
            continue;
        };
        let spec = tex_typeset::expansion::FontExpansionSpec::new(
            i32::from(configured.stretch),
            i32::from(configured.shrink),
            i32::from(configured.step),
            configured.auto_expand,
        )
        .expect("live font expansion settings are validated");
        let efcode = stores.pdf_font_code(PdfFontCode::Ef, font, code);
        let ratio = spec.discrete_ratio(line_ratio, efcode);
        let expanded = stores.try_expanded_font(font, ratio)?;
        match node {
            Node::Char { font, .. } | Node::Lig { font, .. } => *font = expanded,
            _ => unreachable!("glyph identity restricts expansion substitution"),
        }
    }
    let Some(interior_end) = nodes.len().checked_sub(1) else {
        return Ok(());
    };
    for index in 1..interior_end {
        if !matches!(
            nodes[index],
            Node::Kern {
                kind: KernKind::Font,
                ..
            }
        ) {
            continue;
        }
        let (Some((left_font, left)), Some((right_font, right))) = (
            glyph_identity(&nodes[index - 1]),
            glyph_identity(&nodes[index + 1]),
        ) else {
            continue;
        };
        if left_font != right_font {
            continue;
        }
        if let Some(tex_fonts::LigKernCommand::Kern(amount)) = stores.font_lig_kern_command(
            left_font,
            tex_fonts::LigKernChar::Char(left),
            tex_fonts::LigKernChar::Char(right),
        ) && let Node::Kern { amount: kern, .. } = &mut nodes[index]
        {
            *kern = amount;
        }
    }
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

fn active_text_directions(nodes: &[Node]) -> Vec<Direction> {
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
    hlist: Vec<Node>,
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
    let tape = ParagraphTape::analyze(
        &crate::typeset_context::TypesetContext::new(stores),
        tex_state::node_sequence::NodeSequence::mirrored(hlist),
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
            cached_pretolerance_plan(stores, tape.nodes(), &line_params),
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
        let hlist = tape.into_semantic_nodes();
        let (sequence, missing_hyphens) =
            super::hyphenation::hyphenated_hlist_sequence_with_fuel(stores, hlist, fuel)?;
        let (mut hyphenated, physical_nodes) = sequence.take();
        crate::box_runtime::hmode::reshape_open_type_runs(stores, &mut hyphenated);
        let tape = ParagraphTape::analyze(
            &crate::typeset_context::TypesetContext::new(stores),
            tex_state::node_sequence::NodeSequence::from_compacted_semantic(
                hyphenated,
                physical_nodes,
            ),
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
            missing_hyphens,
        ))
    }
}

fn report_line_break_trace<G>(
    stores: &mut CommandContext<'_, G>,
    nodes: &[Node],
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
    let mut short_display = crate::pack_report::ShortDisplayRenderer::new();
    let rendered_trace = trace
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
                Some(short_display.render_nodes(stores, &nodes[display.clone()])),
                display_suffix.as_ref().map(|suffix| {
                    short_display.render_line_break_trace_suffix(stores, suffix.clone())
                }),
            ),
            _ => (None, None),
        })
        .collect::<Vec<_>>();
    let mut diagnostic = stores.begin_diagnostic();
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
    hlist: &[Node],
    line_params: &LineBreakParams,
) -> Option<tex_typeset::linebreak::BreakPlan> {
    try_line_break_without_hyphenation(
        &crate::typeset_context::TypesetContext::new(stores),
        hlist,
        line_params,
    )
}

fn extract_migrating_material<G>(
    stores: &CommandContext<'_, G>,
    nodes: &mut Vec<Node>,
    pre_migrated: &mut Vec<Node>,
    migrated: &mut Vec<Node>,
    retained: &mut Vec<Node>,
) {
    pre_migrated.clear();
    migrated.clear();
    retained.clear();
    for node in nodes.extract_if(.., |node| {
        matches!(node, Node::Mark { .. } | Node::Ins { .. } | Node::Adjust(_))
    }) {
        match node {
            node @ (Node::Mark { .. } | Node::Ins { .. }) => {
                migrated.push(node.clone());
                retained.push(node);
            }
            Node::Adjust(adjust) => {
                let target = if adjust.pre {
                    &mut *pre_migrated
                } else {
                    &mut *migrated
                };
                target.extend(
                    stores
                        .page_node_list(adjust.content)
                        .expect("adjustment content belongs to the live page arena")
                        .nodes()
                        .iter()
                        .cloned(),
                );
                retained.push(Node::Adjust(adjust));
            }
            _ => unreachable!("extract predicate restricts migrating node kinds"),
        }
    }
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

pub(crate) fn normal_paragraph<G>(_nest: &mut ModeNest, stores: &mut CommandContext<'_, G>) {
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
                    &diagnostic_context.output_context,
                )?;
            }
            nest.push_at_line(crate::Mode::Horizontal, diagnostic_context.current_line)?;
            let (language, left, right) = crate::box_runtime::hmode::current_hyphen_context(stores);
            nest.current_list_mutation()
                .set_hyphen_context(language, left, right);
            if indent {
                let mut fuel = tex_command::CommandFuelLedger::default();
                crate::box_runtime::indent_in_hmode(nest, stores, true, fuel.fuel_mut())?;
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

fn reset_after_par<G>(nest: &mut ModeNest, stores: &mut CommandContext<'_, G>) {
    normal_paragraph(nest, stores);
}

fn glue_parameter_value<G>(stores: &CommandContext<'_, G>, parameter: GlueParam) -> GlueSpec {
    stores
        .glue_param(parameter)
        .map_or(GlueSpec::ZERO, |id| stores.glue(id))
}
use tex_state::env::banks::{DimenParam, GlueParam, IntParam};
use tex_state::font::PdfFontCode;
use tex_state::glue::{GlueSpec, Order};
use tex_state::node::{BoxNode, Direction, GlueKind, KernKind, Node};
use tex_state::scaled::Scaled;
use tex_state::{CommandContext, PenaltyArrayKind};
use tex_typeset::PackSpec;
use tex_typeset::linebreak::{
    LineBreakParams, LineBreakPass, LineBreakResult, LineBreakTrace, LineDimensions,
    LineMaterializer, LineShape, LineShapeEntry, ParagraphShape as TypesetParagraphShape,
    ParagraphTape, PostLineBreakParams, TraceBreakpoint, break_hyphenated_tape,
    break_hyphenated_tape_traced, try_line_break_without_hyphenation,
    try_tape_without_hyphenation_traced,
};

use crate::box_runtime::{
    append_node_to_current_list, commit_current_list, flush_pending_hchars_with_fuel,
    hpack_owned_with_overfull_rule,
};
use crate::mode::ParagraphParams;
use crate::vertical::{append_migrated_contribution, append_vertical_contribution};
use crate::{ExecError, ModeNest};
