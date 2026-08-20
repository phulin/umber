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

pub(crate) fn display_line_dimensions(nest: &ModeNest, stores: &Universe) -> LineDimensions {
    let params = ParagraphParams {
        left_skip: stores.glue_ref(stores.glue_param(GlueParam::LEFT_SKIP)),
        right_skip: stores.glue_ref(stores.glue_param(GlueParam::RIGHT_SKIP)),
        par_fill_skip: stores.glue_ref(stores.glue_param(GlueParam::PAR_FILL_SKIP)),
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

pub(crate) fn break_current_paragraph(
    nest: &mut ModeNest,
    stores: &mut Universe,
    widow_penalty_selector: tex_typeset::linebreak::WidowPenaltySelector,
    reset_paragraph: bool,
    error_context: String,
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
    let mut hlist =
        crate::math::finish_math_lists_owned(stores, level.list_mutation().take_nodes(), true);
    observe_paragraph_material_dependencies(stores, &hlist);
    let tracing = stores.int_param(IntParam::TRACING_PARAGRAPHS) > 0;
    normalize_paragraph_infinite_shrink(
        stores,
        &mut params,
        &mut hlist,
        tracing,
        Some(error_context.clone()),
    )?;
    let mut line_params = line_break_params(stores, &params);
    if line_params.pdf_adjust_spacing > 1 {
        line_params.expansion_steps =
            tex_typeset::linebreak::validate_paragraph_expansion(stores, &hlist)?;
    }
    let (mut decisions, trace, missing_hyphens) =
        break_hlist_with_trace(stores, hlist, line_params, fuel, tracing)?;
    stores.observe_line_break_memory_search(&decisions.memory);
    let break_memory = decisions.memory.clone();
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
        let spec = stores.intern_glue(spec);
        decisions.tape.replace_last_par_fill(spec);
    }
    let empty_list = tex_state::node_arena::NodeListRef::empty();
    let post_params = post_line_break_params(&params, widow_penalty_selector, empty_list.clone());
    let mut line_count = 0i32;
    let mut last_line = None;
    let total_lines = decisions.breaks.len();
    let pdf_line_dimensions = pdf_line_dimensions(stores);
    let protrudes_chars = stores.pdf_font_configuration().protrudes_chars();
    let adjusts_spacing = stores.pdf_font_configuration().adjusts_spacing();
    // §804: `pack_begin_line:=mode_line` for the whole of `post_line_break`,
    // restored to 0 when the paragraph's lines are packed. This is what makes
    // §663 say "in paragraph at lines A--B" instead of "detected at line B".
    let restore_pack_begin_line = stores.pack_begin_line();
    let paragraph_start_line = stores.pop_paragraph_start_line().unwrap_or(0);
    stores.set_pack_begin_line(paragraph_start_line);
    let mut materializer = LineMaterializer::new(decisions.tape, decisions.breaks, post_params);
    let mut line_nodes = Vec::new();
    let mut migrated = Vec::new();
    let mut pre_migrated = Vec::new();
    let mut retained_migrated = Vec::new();
    while let Some(mut broken) = materializer.materialize_next(stores, line_nodes) {
        crate::box_runtime::hmode::reshape_open_type_runs(stores, &mut broken.nodes);
        materialize_pdf_line(
            stores,
            &mut broken.nodes,
            broken.dimensions.width,
            adjusts_spacing,
            protrudes_chars,
        )?;
        extract_migrating_material(
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
            discretionary_diagnostics_differ(&diagnostic_nodes, &broken.nodes);
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
    stores.observe_line_break_memory_cleanup(&break_memory);
    stores.set_pack_begin_line(restore_pack_begin_line);
    nest.current_list_mutation().set_prev_graf(
        params
            .prev_graf
            .checked_add(line_count)
            .expect("TeX prev_graf overflow"),
    );
    if reset_paragraph {
        reset_after_par(nest, stores);
    }
    crate::vertical::build_page_if_outer_vertical_with_error_context(nest, stores, &error_context)?;
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
fn discretionary_diagnostics_differ(physical: &[Node], semantic: &[Node]) -> bool {
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
        if usize::from(*physical_replace_count) != replace.len()
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

fn materialize_pdf_line(
    stores: &mut Universe,
    nodes: &mut Vec<Node>,
    target: Scaled,
    adjusts_spacing: bool,
    protrudes_chars: bool,
) -> Result<(), ExecError> {
    if adjusts_spacing {
        apply_line_expansion(stores, nodes, target)?;
    }
    if protrudes_chars {
        tex_typeset::protrusion::insert_margin_kerns(stores, nodes);
    }
    Ok(())
}

/// TeX82 §825: paragraph glue may shrink only at normal order. Each
/// offending specification is copied and normalized, while recovery is
/// reported at most once for the whole paragraph.
fn normalize_paragraph_infinite_shrink(
    stores: &mut Universe,
    params: &mut ParagraphParams,
    nodes: &mut [Node],
    tracing: bool,
    mut error_context: Option<String>,
) -> Result<(), ExecError> {
    let mut reported = false;
    let mut normalize = |spec: &mut tex_state::glue::GlueSpecRef| -> Result<(), ExecError> {
        let mut glue = stores.glue_spec(*spec);
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
        *spec = stores.intern_glue(glue);
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

pub(crate) fn apply_line_expansion(
    stores: &mut Universe,
    nodes: &mut [Node],
    target: Scaled,
) -> Result<(), ExecError> {
    let line_ratio = tex_typeset::linebreak::plan_line_expansion(stores, nodes, target);
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
        if let Some(tex_fonts::LigKernCommand::Kern(amount)) = stores.lig_kern_command(
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

fn pdf_line_dimensions(stores: &mut Universe) -> PdfLineDimensions {
    for param in [
        DimenParam::PDF_IGNORED_DIMEN,
        DimenParam::PDF_FIRST_LINE_HEIGHT,
        DimenParam::PDF_LAST_LINE_DEPTH,
        DimenParam::PDF_EACH_LINE_HEIGHT,
        DimenParam::PDF_EACH_LINE_DEPTH,
    ] {
        stores.observe_semantic_dependency(tex_state::DependencyKey::Cell(
            tex_state::cell::CellId::new(
                tex_state::cell::BankTag::DimenParam,
                u32::from(param.raw()),
            ),
        ));
    }
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

fn break_hlist_with_trace(
    stores: &mut Universe,
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
        stores,
        tex_state::node_sequence::NodeSequence::mirrored(hlist),
        &line_params,
    );
    let (first, trace) = if tracing {
        try_tape_without_hyphenation_traced(stores, &tape, &line_params)
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
            stores,
            tex_state::node_sequence::NodeSequence::from_compacted_semantic(
                hyphenated,
                physical_nodes,
            ),
            &line_params,
        );
        let (plan, trace) = if tracing {
            break_hyphenated_tape_traced(stores, &tape, &line_params, trace)
        } else {
            (break_hyphenated_tape(stores, &tape, &line_params), trace)
        };
        Ok((
            tex_typeset::linebreak::plan_with_tape(plan, tape),
            trace,
            missing_hyphens,
        ))
    }
}

fn report_line_break_trace(
    stores: &mut Universe,
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
                    stores.font(warning.font).name().to_owned(),
                    warning.ch,
                )
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let mut diagnostic = stores.begin_diagnostic();
    let mut short_display = crate::pack_report::ShortDisplayRenderer::new();
    let mut next_warning = 0;
    for event in trace {
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
                short_display.reset();
                diagnostic.print_nl(match pass {
                    LineBreakPass::First => "@firstpass",
                    LineBreakPass::Second => "@secondpass",
                    LineBreakPass::Emergency => "@emergencypass",
                });
            }
            LineBreakTrace::Feasible {
                display,
                display_suffix,
                breakpoint,
                via,
                badness,
                penalty,
                demerits,
            } => {
                if !display.is_empty() {
                    let rendered =
                        short_display.render_nodes(diagnostic.state(), &nodes[display.clone()]);
                    diagnostic.print_nl("").print_rendered(&rendered);
                }
                if !display.is_empty()
                    && let Some(suffix) = display_suffix
                {
                    let rendered = short_display
                        .render_line_break_trace_suffix(diagnostic.state(), suffix.clone());
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

/// Looks up or computes the pure pretolerance line-breaking plan.
///
/// Callers retain ownership of the node list. The cache value contains only
/// stable positions, scalar demerits, and detached glue content.
pub fn cached_pretolerance_plan(
    stores: &mut Universe,
    hlist: &[Node],
    line_params: &LineBreakParams,
) -> Option<tex_typeset::linebreak::BreakPlan> {
    if !stores
        .with_pure_memo(|memo| memo.pretolerance_enabled())
        .unwrap_or(false)
    {
        if stores
            .with_pure_memo(|memo| memo.is_enabled())
            .unwrap_or(false)
        {
            stores.with_pure_memo(|memo| {
                memo.record_not_attempted(tex_state::PureMemoLayer::Pretolerance);
            });
        }
        return try_line_break_without_hyphenation(stores, hlist, line_params);
    }
    let validation_started = crate::timing::TelemetryTimer::start();
    let key = pretolerance_memo_key(stores, hlist, line_params);
    stores.with_pure_memo(|memo| {
        memo.record_timing(
            tex_state::PureMemoLayer::Pretolerance,
            tex_state::MemoTimingPhase::Validation,
            validation_started.elapsed(),
        );
    });
    match stores
        .with_pure_memo(|memo| memo.lookup_pretolerance(key))
        .flatten()
    {
        Some(plan) => plan,
        None => compute_and_cache_pretolerance(stores, key, hlist, line_params),
    }
}

const PRETOLERANCE_MEMO_DOMAIN: u32 = 1;
const PRETOLERANCE_PLAN_SCHEMA: u32 = 2;
const PRETOLERANCE_HASH_DOMAINS: [u64; 4] = [
    0x6c62_7072_6574_0001,
    0x6c62_7072_6574_0002,
    0x6c62_7072_6574_0003,
    0x6c62_7072_6574_0004,
];

fn compute_and_cache_pretolerance(
    stores: &mut Universe,
    key: PureMemoKey,
    hlist: &[Node],
    params: &LineBreakParams,
) -> Option<tex_typeset::linebreak::BreakPlan> {
    let plan = try_line_break_without_hyphenation(stores, hlist, params);
    stores.with_pure_memo(|memo| memo.insert_pretolerance(key, plan.clone()));
    plan
}

fn pretolerance_memo_key(
    stores: &Universe,
    hlist: &[Node],
    params: &LineBreakParams,
) -> PureMemoKey {
    let node_hashes =
        stores.engine_boundary_hashes(PRETOLERANCE_HASH_DOMAINS, |hash| hash.nodes(hlist));
    let mut bytes = Vec::with_capacity(256);
    bytes.extend_from_slice(&PRETOLERANCE_PLAN_SCHEMA.to_le_bytes());
    for hash in node_hashes {
        bytes.extend_from_slice(&hash.to_le_bytes());
    }
    encode_line_break_params(params, &mut bytes);
    PureMemoKey::new(
        PRETOLERANCE_MEMO_DOMAIN,
        node_hashes[0],
        ContentHash::from_bytes(&bytes),
    )
}

fn encode_line_break_params(params: &LineBreakParams, out: &mut Vec<u8>) {
    for value in [
        params.pretolerance,
        params.tolerance,
        params.line_penalty,
        params.hyphen_penalty,
        params.ex_hyphen_penalty,
        params.adj_demerits,
        params.double_hyphen_demerits,
        params.final_hyphen_demerits,
        params.emergency_stretch.raw(),
        params.looseness,
        params.last_line_fit,
        params.pdf_adjust_spacing,
        params.pdf_protrude_chars,
    ] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    match params.expansion_steps {
        Some((stretch, shrink)) => {
            out.push(1);
            out.extend_from_slice(&stretch.to_le_bytes());
            out.extend_from_slice(&shrink.to_le_bytes());
        }
        None => out.push(0),
    }
    encode_glue_spec(params.left_skip, out);
    encode_glue_spec(params.right_skip, out);
    encode_glue_spec(params.par_fill_skip, out);
    out.extend_from_slice(&params.shape.hsize.raw().to_le_bytes());
    out.extend_from_slice(&params.shape.hang_indent.raw().to_le_bytes());
    out.extend_from_slice(&params.shape.hang_after.to_le_bytes());
    out.extend_from_slice(&(params.shape.line_offset as u64).to_le_bytes());
    match &params.shape.parshape {
        Some(shape) => {
            out.push(1);
            out.extend_from_slice(&(shape.lines.len() as u64).to_le_bytes());
            for line in &shape.lines {
                out.extend_from_slice(&line.indent.raw().to_le_bytes());
                out.extend_from_slice(&line.width.raw().to_le_bytes());
            }
        }
        None => out.push(0),
    }
}

fn encode_glue_spec(spec: tex_state::glue::GlueSpec, out: &mut Vec<u8>) {
    out.extend_from_slice(&spec.width.raw().to_le_bytes());
    out.extend_from_slice(&spec.stretch.raw().to_le_bytes());
    out.push(spec.stretch_order as u8);
    out.extend_from_slice(&spec.shrink.raw().to_le_bytes());
    out.push(spec.shrink_order as u8);
}

fn extract_migrating_material(
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
                target.extend(adjust.content.to_vec());
                retained.push(Node::Adjust(adjust));
            }
            _ => unreachable!("extract predicate restricts migrating node kinds"),
        }
    }
}

fn observe_paragraph_material_dependencies(stores: &mut Universe, nodes: &[Node]) {
    let mut fonts = std::collections::BTreeSet::new();
    let mut characters = std::collections::BTreeSet::new();
    for node in nodes {
        match node {
            Node::Char { font, ch, .. } => {
                fonts.insert(*font);
                characters.insert(*ch);
            }
            Node::Lig { font, ch, orig, .. } => {
                fonts.insert(*font);
                characters.insert(*ch);
                characters.extend(orig.iter().copied());
            }
            _ => {}
        }
    }
    for font in fonts {
        for index in [2, 3, 4, 7] {
            stores.observe_semantic_dependency(tex_state::DependencyKey::Font {
                field: tex_state::DependencyFontField::Parameter,
                font: font.raw(),
                index,
            });
        }
    }
    for ch in characters {
        stores.observe_semantic_dependency(tex_state::DependencyKey::Code {
            table: tex_state::DependencyCodeTable::Sfcode,
            scalar: ch as u32,
        });
    }
}

fn snapshot_paragraph_params(nest: &ModeNest, stores: &mut Universe) -> ParagraphParams {
    use tex_state::cell::{BankTag, CellId};
    use tex_state::{DependencyEngineField, DependencyKey};
    for param in [
        IntParam::HANG_AFTER,
        IntParam::LOOSENESS,
        IntParam::PRETOLERANCE,
        IntParam::TOLERANCE,
        IntParam::LINE_PENALTY,
        IntParam::HYPHEN_PENALTY,
        IntParam::EX_HYPHEN_PENALTY,
        IntParam::ADJ_DEMERITS,
        IntParam::DOUBLE_HYPHEN_DEMERITS,
        IntParam::FINAL_HYPHEN_DEMERITS,
        IntParam::LAST_LINE_FIT,
        IntParam::INTERLINE_PENALTY,
        IntParam::CLUB_PENALTY,
        IntParam::WIDOW_PENALTY,
        IntParam::DISPLAY_WIDOW_PENALTY,
        IntParam::BROKEN_PENALTY,
    ] {
        stores.observe_semantic_dependency(DependencyKey::Cell(CellId::new(
            BankTag::IntParam,
            u32::from(param.raw()),
        )));
    }
    for param in [
        DimenParam::HANG_INDENT,
        DimenParam::EMERGENCY_STRETCH,
        DimenParam::H_SIZE,
    ] {
        stores.observe_semantic_dependency(DependencyKey::Cell(CellId::new(
            BankTag::DimenParam,
            u32::from(param.raw()),
        )));
    }
    for param in [
        GlueParam::LEFT_SKIP,
        GlueParam::RIGHT_SKIP,
        GlueParam::PAR_FILL_SKIP,
    ] {
        stores.observe_semantic_dependency(DependencyKey::Cell(CellId::new(
            BankTag::GlueParam,
            u32::from(param.raw()),
        )));
    }
    stores.observe_semantic_dependency(DependencyKey::Engine(DependencyEngineField::ParShape));
    stores.observe_semantic_dependency(DependencyKey::Engine(DependencyEngineField::PenaltyArrays));
    ParagraphParams {
        left_skip: stores.glue_ref(stores.glue_param(GlueParam::LEFT_SKIP)),
        right_skip: stores.glue_ref(stores.glue_param(GlueParam::RIGHT_SKIP)),
        par_fill_skip: stores.glue_ref(stores.glue_param(GlueParam::PAR_FILL_SKIP)),
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

fn line_break_params(stores: &Universe, params: &ParagraphParams) -> LineBreakParams {
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
        left_skip: stores.glue_spec(params.left_skip),
        right_skip: stores.glue_spec(params.right_skip),
        par_fill_skip: stores.glue_spec(params.par_fill_skip),
        shape: line_shape(params),
    }
}

fn post_line_break_params(
    params: &ParagraphParams,
    widow_penalty_selector: tex_typeset::linebreak::WidowPenaltySelector,
    empty_list: tex_state::node_arena::NodeListRef,
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

pub(crate) fn normal_paragraph(_nest: &mut ModeNest, stores: &mut Universe) {
    // e-TeX [47.1070] resets a non-null interline array through `eq_define`,
    // so [19.277]'s generic assignment hook traces the local write. The club
    // and widow arrays retain their scoped assignments (manual §3.4).
    let interline_penalties = stores.penalty_array(PenaltyArrayKind::InterLine);
    if !interline_penalties.is_empty() {
        stores.set_penalty_array(PenaltyArrayKind::InterLine, &[], false);
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
        stores.set_int_param(IntParam::LOOSENESS, 0);
    }
    if stores.dimen_param(DimenParam::HANG_INDENT).raw() != 0 {
        stores.set_dimen_param(DimenParam::HANG_INDENT, Scaled::from_raw(0));
    }
    if stores.int_param(IntParam::HANG_AFTER) != 1 {
        stores.set_int_param(IntParam::HANG_AFTER, 1);
    }
    stores.set_paragraph_shape(&[], false);
}

pub(crate) fn start_paragraph(
    nest: &mut ModeNest,
    stores: &mut Universe,
    indent: bool,
    error_context: &str,
) -> Result<(), ExecError> {
    match nest.current_mode() {
        crate::Mode::Vertical | crate::Mode::InternalVertical => {
            nest.set_enclosing_vertical_prev_graf(0);
            let parskip = stores.glue_param(GlueParam::PAR_SKIP);
            if nest.current_mode() == crate::Mode::Vertical || !nest.current_list().is_empty() {
                append_vertical_contribution(
                    nest,
                    stores,
                    Node::Glue {
                        spec: stores.glue_ref(parskip),
                        kind: GlueKind::ParSkip,
                        leader: None,
                    },
                );
                crate::vertical::build_page_if_outer_vertical_with_error_context(
                    nest,
                    stores,
                    error_context,
                )?;
            }
            nest.push_at_line(crate::Mode::Horizontal, stores.current_input_line())?;
            stores.push_paragraph_start_line(stores.current_input_line());
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
            token: tex_state::token::Token::Cs(stores.intern("par").symbol()),
            origin: tex_state::token::OriginId::UNKNOWN,
            operation: "canonical paragraph start",
        }),
    }
}

fn reset_after_par(nest: &mut ModeNest, stores: &mut Universe) {
    normal_paragraph(nest, stores);
}
use tex_state::env::banks::{DimenParam, GlueParam, IntParam};
use tex_state::font::PdfFontCode;
use tex_state::glue::Order;
use tex_state::node::{BoxNode, Direction, GlueKind, KernKind, Node};
use tex_state::scaled::Scaled;
use tex_state::{ContentHash, PenaltyArrayKind, PureMemoKey, Universe};
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
