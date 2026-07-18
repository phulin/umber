use tex_lex::{InputStack, TokenListReplayKind};
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::font::PdfFontCode;
use tex_state::node::{BoxNode, Direction, GlueKind, KernKind, Node};
use tex_state::scaled::Scaled;
use tex_state::{ContentHash, ParagraphShapeLine, PenaltyArrayKind, PureMemoKey, Universe};
use tex_typeset::PackSpec;
use tex_typeset::linebreak::{
    LineBreakParams, LineBreakResult, LineDimensions, LineMaterializer, LineShape, LineShapeEntry,
    ParagraphShape as TypesetParagraphShape, PostLineBreakParams, line_break_hyphenated,
    try_line_break_without_hyphenation,
};

use super::boxes::hpack_owned_with_overfull_rule;
use super::*;
use crate::mode::{ParagraphParams, ignored_depth};
use crate::vertical::{
    append_migrated_contribution, append_node_to_current_list, append_vertical_contribution,
    build_page_if_outer_vertical,
};
use crate::{ExecError, Mode, ModeNest};

pub(super) fn execute_paragraph_command(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    global: bool,
) -> Result<(), ExecError> {
    match primitive {
        UnexpandablePrimitive::Par | UnexpandablePrimitive::EndGraf => {
            if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
                // TeX82's `vmode + par_end` branch calls `normal_paragraph`
                // even though there is no horizontal list to finish. LaTeX
                // relies on this to clear a list's one-line `\parshape`
                // before starting nested verbatim paragraphs.
                execution.pending_paragraph_memo = None;
                normal_paragraph(nest, stores);
                build_page_if_outer_vertical(nest, stores)
            } else {
                end_paragraph_with_memo(nest, input, stores, execution)
            }
        }
        UnexpandablePrimitive::Indent => start_paragraph(nest, input, stores, true, true),
        UnexpandablePrimitive::NoIndent => start_paragraph(nest, input, stores, false, true),
        UnexpandablePrimitive::QuitVMode => {
            if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
                start_paragraph(nest, input, stores, true, true)
            } else {
                Ok(())
            }
        }
        UnexpandablePrimitive::ParShape => {
            assign_parshape(input, stores, execution, context, global)
        }
        primitive @ (UnexpandablePrimitive::InterLinePenalties
        | UnexpandablePrimitive::ClubPenalties
        | UnexpandablePrimitive::WidowPenalties
        | UnexpandablePrimitive::DisplayWidowPenalties) => {
            assign_penalty_array(primitive, input, stores, execution, context, global)
        }
        UnexpandablePrimitive::PrevDepth => {
            assign_prevdepth(nest, input, stores, execution, context)
        }
        UnexpandablePrimitive::PrevGraf => assign_prevgraf(nest, input, stores, execution, context),
        UnexpandablePrimitive::NoInterlineSkip => {
            nest.current_list_mut()
                .set_prev_depth(ignored_depth(stores));
            Ok(())
        }
        _ => unreachable!("caller restricts paragraph commands"),
    }
}

pub(crate) fn ensure_horizontal_for_character(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
        start_paragraph(nest, input, stores, true, true)?;
    }
    Ok(())
}

fn start_paragraph(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    indent: bool,
    replay_everypar: bool,
) -> Result<(), ExecError> {
    match nest.current_mode() {
        Mode::Vertical | Mode::InternalVertical => {
            // TeX82 new_graf starts every fresh paragraph at line zero. The
            // enclosing prev_graf is only a continuation offset while a
            // paragraph is interrupted by display math.
            nest.set_enclosing_vertical_prev_graf(0);
            let parskip = stores.glue_param(GlueParam::PAR_SKIP);
            if nest.current_mode() == Mode::Vertical || !nest.current_list().is_empty() {
                append_vertical_contribution(
                    nest,
                    stores,
                    Node::Glue {
                        spec: parskip,
                        kind: GlueKind::Normal,
                        leader: None,
                    },
                );
                build_page_if_outer_vertical(nest, stores)?;
            }
            nest.push(Mode::Horizontal);
            if indent {
                append_indent_box(nest, stores)?;
            }
            if replay_everypar {
                let everypar = stores.tok_param(TokParam::EVERY_PAR);
                if !stores.tokens(everypar).is_empty() {
                    input.push_token_list(everypar, TokenListReplayKind::EveryPar);
                }
            }
            Ok(())
        }
        Mode::Horizontal | Mode::RestrictedHorizontal => {
            if indent {
                append_indent_box(nest, stores)?;
            }
            Ok(())
        }
        mode => Err(ExecError::UnimplementedTypesetting {
            mode,
            token: tex_state::token::Token::Cs(stores.intern("par").symbol()),
            origin: OriginId::UNKNOWN,
            operation: "paragraph start",
        }),
    }
}

fn append_indent_box(nest: &mut ModeNest, stores: &mut Universe) -> Result<(), ExecError> {
    nest.current_list_mut().push(make_indent_box(stores));
    Ok(())
}

pub(crate) fn make_indent_box(stores: &mut Universe) -> Node {
    let empty = stores.freeze_node_list(&[]);
    let par_indent = stores.dimen_param(DimenParam::PAR_INDENT);
    let mut node = hpack_with_overfull_rule(stores, empty, PackSpec::Exactly(par_indent));
    node.height = Scaled::from_raw(0);
    node.depth = Scaled::from_raw(0);
    Node::HList(node)
}

pub(crate) fn end_paragraph(nest: &mut ModeNest, stores: &mut Universe) -> Result<(), ExecError> {
    if nest.current_mode() != Mode::Horizontal {
        return Ok(());
    }
    flush_pending_hchars(nest, stores)?;
    if nest.current_list().is_empty() {
        let _ = nest.pop()?;
        normal_paragraph(nest, stores);
        build_page_if_outer_vertical(nest, stores)?;
        return Ok(());
    }
    let final_widow_penalty = stores.int_param(IntParam::WIDOW_PENALTY);
    let final_widow_penalties = stores.penalty_array(PenaltyArrayKind::Widow);
    let _ = break_current_paragraph(
        nest,
        stores,
        final_widow_penalty,
        final_widow_penalties,
        true,
        None,
    )?;
    Ok(())
}

fn end_paragraph_with_memo(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    if nest.current_mode() == Mode::Horizontal {
        flush_pending_hchars(nest, stores)?;
        crate::paragraph_memo::publish_prepared_hlist(
            input,
            stores,
            execution,
            nest.current_list().nodes(),
            nest.enclosing_vertical_prev_graf(),
            crate::executor::ParagraphContinuation::End,
        );
    } else {
        execution.pending_paragraph_memo = None;
    }
    if nest.current_mode() != Mode::Horizontal {
        execution.pending_paragraph_memo = None;
        return end_paragraph(nest, stores);
    }
    if nest.current_list().is_empty() {
        execution.pending_paragraph_memo = None;
        return end_paragraph(nest, stores);
    }
    let final_widow_penalty = stores.int_param(IntParam::WIDOW_PENALTY);
    let final_widow_penalties = stores.penalty_array(PenaltyArrayKind::Widow);
    let _ = break_current_paragraph(
        nest,
        stores,
        final_widow_penalty,
        final_widow_penalties,
        true,
        Some(execution),
    )?;
    Ok(())
}

pub(crate) fn install_reused_paragraph_hlist(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    nodes: Vec<Node>,
    finished: Option<(Vec<Node>, i32, i32)>,
    continuation: crate::executor::ParagraphContinuation,
) -> Result<Option<BoxNode>, ExecError> {
    // The retained hlist already includes the recorded `everypar` execution;
    // scheduling it again would leave its tokens after the consumed paragraph.
    // Finished retained lines already contain the recorded indent box and
    // `\everypar` material. Enter horizontal mode only to reproduce the
    // paragraph's vertical-side effects; constructing either input again
    // would be immediately discarded below.
    start_paragraph(nest, input, stores, finished.is_none(), false)?;
    let _ = nest.current_list_mut().take_nodes();
    nest.current_list_mut().append(nodes);
    let Some((finished, line_count, last_badness)) = finished else {
        let final_widow_penalty = stores.int_param(IntParam::WIDOW_PENALTY);
        let final_widow_penalties = stores.penalty_array(PenaltyArrayKind::Widow);
        let _ = break_current_paragraph(
            nest,
            stores,
            final_widow_penalty,
            final_widow_penalties,
            true,
            Some(execution),
        )?;
        return Ok(None);
    };
    let last_line = finished.iter().rev().find_map(|node| match node {
        Node::HList(line) => Some(*line),
        _ => None,
    });
    let _ = nest.pop()?;
    for node in finished {
        match node {
            Node::Adjust(list) => {
                let adjusted = stores.nodes(list).to_vec();
                for node in adjusted {
                    append_migrated_contribution(nest, stores, node);
                }
            }
            node @ (Node::Mark { .. } | Node::Ins { .. }) => {
                append_migrated_contribution(nest, stores, node);
            }
            node => append_node_to_current_list(nest, stores, node)?,
        }
    }
    let prev_graf = nest.enclosing_vertical_prev_graf();
    nest.current_list_mut()
        .set_prev_graf(prev_graf.saturating_add(line_count));
    stores.set_last_badness(last_badness);
    if continuation == crate::executor::ParagraphContinuation::End {
        reset_after_par(nest, stores);
    }
    build_page_if_outer_vertical(nest, stores)?;
    Ok(last_line)
}

pub(crate) struct ParagraphBreakResult {
    pub(crate) last_line: Option<BoxNode>,
    pub(crate) active_directions: Vec<Direction>,
}

pub(crate) fn interrupt_paragraph_for_display(
    nest: &mut ModeNest,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<ParagraphBreakResult, ExecError> {
    flush_pending_hchars(nest, stores)?;
    if nest.current_list().is_empty() {
        let _ = nest.pop()?;
        return Ok(ParagraphBreakResult {
            last_line: None,
            active_directions: Vec::new(),
        });
    }
    let final_widow_penalty = stores.int_param(IntParam::DISPLAY_WIDOW_PENALTY);
    let final_widow_penalties = stores.penalty_array(PenaltyArrayKind::DisplayWidow);
    break_current_paragraph(
        nest,
        stores,
        final_widow_penalty,
        final_widow_penalties,
        false,
        Some(execution),
    )
}

pub(crate) fn display_line_dimensions(nest: &ModeNest, stores: &Universe) -> LineDimensions {
    let params = ParagraphParams {
        left_skip: stores.glue_param(GlueParam::LEFT_SKIP),
        right_skip: stores.glue_param(GlueParam::RIGHT_SKIP),
        par_fill_skip: stores.glue_param(GlueParam::PAR_FILL_SKIP),
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
        broken_penalty: stores.int_param(IntParam::BROKEN_PENALTY),
        interline_penalties: stores.penalty_array(PenaltyArrayKind::InterLine),
        club_penalties: stores.penalty_array(PenaltyArrayKind::Club),
        widow_penalties: stores.penalty_array(PenaltyArrayKind::Widow),
        display_widow_penalties: stores.penalty_array(PenaltyArrayKind::DisplayWidow),
    };
    line_shape(&params).dimensions(2)
}

fn break_current_paragraph(
    nest: &mut ModeNest,
    stores: &mut Universe,
    final_widow_penalty: i32,
    final_widow_penalties: Vec<i32>,
    reset_paragraph: bool,
    mut memo: Option<&mut crate::ExecutionContext<'_>>,
) -> Result<ParagraphBreakResult, ExecError> {
    flush_pending_hchars(nest, stores)?;
    let active_directions = active_text_directions(nest.current_list().nodes());
    let params = snapshot_paragraph_params(nest, stores);
    remove_final_glue(nest.current_list_mut());
    nest.current_list_mut().push(Node::Penalty(10_000));
    nest.current_list_mut().push(Node::Glue {
        spec: params.par_fill_skip,
        kind: GlueKind::ParFillSkip,
        leader: None,
    });
    let mut level = nest.pop()?;
    let hlist = crate::math::finish_math_lists_owned(stores, level.list_mut().take_nodes(), true);
    let mut line_params = line_break_params(stores, &params);
    if line_params.pdf_adjust_spacing > 1 {
        line_params.expansion_steps =
            tex_typeset::linebreak::validate_paragraph_expansion(stores, &hlist)?;
    }
    let mut decisions = break_hlist(stores, hlist, line_params);
    if let Some(spec) = decisions.last_line_fill {
        let spec = stores.intern_glue(spec);
        if let Some(Node::Glue { spec: par_fill, .. }) =
            decisions.nodes.iter_mut().rev().find(|node| {
                matches!(
                    node,
                    Node::Glue {
                        kind: GlueKind::ParFillSkip,
                        ..
                    }
                )
            })
        {
            *par_fill = spec;
        }
    }
    let empty_list = stores.freeze_node_list(&[]);
    let post_params = post_line_break_params(
        &params,
        final_widow_penalty,
        final_widow_penalties,
        empty_list,
    );
    let mut line_count = 0i32;
    let mut last_line = None;
    let total_lines = decisions.breaks.len();
    let pdf_line_dimensions = pdf_line_dimensions(stores);
    let protrudes_chars = stores.pdf_font_configuration().protrudes_chars();
    let adjusts_spacing = stores.pdf_font_configuration().adjusts_spacing();
    let mut materializer = LineMaterializer::new(decisions.nodes, decisions.breaks, post_params);
    let mut line_nodes = Vec::new();
    let mut migrated = Vec::new();
    let mut retained_migrated = Vec::new();
    let mut finished_nodes = Vec::new();
    while let Some(mut broken) = materializer.materialize_next(stores, line_nodes) {
        super::hmode::reshape_open_type_runs(stores, &mut broken.nodes);
        if adjusts_spacing {
            apply_line_expansion(stores, &mut broken.nodes, broken.dimensions.width)?;
        }
        if protrudes_chars {
            tex_typeset::protrusion::insert_margin_kerns(stores, &mut broken.nodes);
        }
        extract_migrating_material(
            stores,
            &mut broken.nodes,
            &mut migrated,
            &mut retained_migrated,
        );
        let line = hpack_owned_with_overfull_rule(
            stores,
            &mut broken.nodes,
            PackSpec::Exactly(broken.dimensions.width),
        );
        let mut line = line;
        line.shift = broken.dimensions.indent;
        pdf_line_dimensions.apply(&mut line, line_count as usize, total_lines);
        line_count = line_count
            .checked_add(1)
            .expect("paragraph line count exceeds i32");
        last_line = Some(line);
        let line_node = Node::HList(line);
        finished_nodes.push(line_node.clone());
        append_node_to_current_list(nest, stores, line_node)?;
        for node in migrated.drain(..) {
            append_migrated_contribution(nest, stores, node);
        }
        finished_nodes.append(&mut retained_migrated);
        if let Some(penalty) = broken.penalty_after {
            let penalty = Node::Penalty(penalty);
            finished_nodes.push(penalty.clone());
            append_vertical_contribution(nest, stores, penalty);
        }
        line_nodes = broken.nodes;
    }
    nest.current_list_mut().set_prev_graf(
        params
            .prev_graf
            .checked_add(line_count)
            .expect("TeX prev_graf overflow"),
    );
    if let Some(execution) = memo.take() {
        crate::paragraph_memo::publish_finished_lines(
            stores,
            execution,
            &finished_nodes,
            line_count,
            &active_directions,
        );
    }
    if reset_paragraph {
        reset_after_par(nest, stores);
    }
    build_page_if_outer_vertical(nest, stores)?;
    Ok(ParagraphBreakResult {
        last_line,
        active_directions,
    })
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

fn pdf_line_dimensions(stores: &Universe) -> PdfLineDimensions {
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

pub(crate) fn break_hlist(
    stores: &mut Universe,
    hlist: Vec<Node>,
    line_params: LineBreakParams,
) -> LineBreakResult {
    let first = cached_pretolerance_plan(stores, &hlist, &line_params);
    if let Some(first) = first {
        tex_typeset::linebreak::plan_with_nodes(first, hlist)
    } else {
        let mut hyphenated = super::hyphenation::hyphenated_hlist(stores, &hlist);
        super::hmode::reshape_open_type_runs(stores, &mut hyphenated);
        tex_typeset::linebreak::plan_with_nodes(
            line_break_hyphenated(stores, &hyphenated, &line_params),
            hyphenated,
        )
    }
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
    if !stores.pretolerance_memo_enabled() {
        if stores.pure_memo_enabled() {
            stores.record_pure_memo_not_attempted(tex_state::PureMemoLayer::Pretolerance);
        }
        return try_line_break_without_hyphenation(stores, hlist, line_params);
    }
    #[allow(clippy::disallowed_methods)]
    let validation_started = std::time::Instant::now();
    let key = pretolerance_memo_key(stores, hlist, line_params);
    stores.record_pure_memo_timing(
        tex_state::PureMemoLayer::Pretolerance,
        tex_state::MemoTimingPhase::Validation,
        validation_started.elapsed(),
    );
    match stores.lookup_pure_pretolerance(key) {
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
    stores.insert_pure_pretolerance(key, plan.clone());
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

#[cfg(test)]
pub(crate) fn test_pretolerance_memo_key(
    stores: &Universe,
    hlist: &[Node],
    params: &LineBreakParams,
) -> PureMemoKey {
    pretolerance_memo_key(stores, hlist, params)
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
    stores: &Universe,
    nodes: &mut Vec<Node>,
    migrated: &mut Vec<Node>,
    retained: &mut Vec<Node>,
) {
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
            Node::Adjust(list) => {
                migrated.extend(stores.nodes(list).into_iter().map(|node| node.to_owned()));
                retained.push(Node::Adjust(list));
            }
            _ => unreachable!("extract predicate restricts migrating node kinds"),
        }
    }
}

fn snapshot_paragraph_params(nest: &ModeNest, stores: &Universe) -> ParagraphParams {
    ParagraphParams {
        left_skip: stores.glue_param(GlueParam::LEFT_SKIP),
        right_skip: stores.glue_param(GlueParam::RIGHT_SKIP),
        par_fill_skip: stores.glue_param(GlueParam::PAR_FILL_SKIP),
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
        left_skip: stores.glue(params.left_skip),
        right_skip: stores.glue(params.right_skip),
        par_fill_skip: stores.glue(params.par_fill_skip),
        shape: line_shape(params),
    }
}

fn post_line_break_params(
    params: &ParagraphParams,
    final_widow_penalty: i32,
    final_widow_penalties: Vec<i32>,
    empty_list: tex_state::ids::NodeListId,
) -> PostLineBreakParams {
    PostLineBreakParams {
        empty_list,
        left_skip: params.left_skip,
        right_skip: params.right_skip,
        interline_penalty: params.interline_penalty,
        club_penalty: params.club_penalty,
        widow_penalty: final_widow_penalty,
        broken_penalty: params.broken_penalty,
        prev_graf: params.prev_graf,
        interline_penalties: params.interline_penalties.clone(),
        club_penalties: params.club_penalties.clone(),
        widow_penalties: final_widow_penalties,
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
    stores.set_paragraph_shape(&[], false);
    // e-TeX resets only the interline array at every normal paragraph; the
    // club and widow arrays retain their scoped assignments (manual §3.4).
    stores.set_penalty_array(PenaltyArrayKind::InterLine, &[], false);
    if stores.int_param(IntParam::LOOSENESS) != 0 {
        stores.set_int_param(IntParam::LOOSENESS, 0);
    }
    if stores.dimen_param(DimenParam::HANG_INDENT).raw() != 0 {
        stores.set_dimen_param(DimenParam::HANG_INDENT, Scaled::from_raw(0));
    }
    if stores.int_param(IntParam::HANG_AFTER) != 1 {
        stores.set_int_param(IntParam::HANG_AFTER, 1);
    }
}

fn reset_after_par(nest: &mut ModeNest, stores: &mut Universe) {
    normal_paragraph(nest, stores);
}

fn remove_final_glue(list: &mut crate::ModeList) {
    if matches!(list.nodes().last(), Some(Node::Glue { .. })) {
        let _ = list.pop_last_node();
    }
}

fn assign_parshape(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
    global: bool,
) -> Result<(), ExecError> {
    skip_optional_equals_x(input, stores, execution)?;
    let count = scan_i32(input, stores, execution, context)?.max(0) as usize;
    let mut lines = Vec::with_capacity(count);
    for _ in 0..count {
        lines.push(ParagraphShapeLine {
            indent: scan_scaled(input, stores, execution, context)?,
            width: scan_scaled(input, stores, execution, context)?,
        });
    }
    stores.set_paragraph_shape(&lines, global);
    Ok(())
}

fn assign_penalty_array(
    primitive: UnexpandablePrimitive,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
    global: bool,
) -> Result<(), ExecError> {
    skip_optional_equals_x(input, stores, execution)?;
    let count = scan_i32(input, stores, execution, context)?;
    let kind = match primitive {
        UnexpandablePrimitive::InterLinePenalties => PenaltyArrayKind::InterLine,
        UnexpandablePrimitive::ClubPenalties => PenaltyArrayKind::Club,
        UnexpandablePrimitive::WidowPenalties => PenaltyArrayKind::Widow,
        UnexpandablePrimitive::DisplayWidowPenalties => PenaltyArrayKind::DisplayWidow,
        _ => unreachable!("caller restricts primitive"),
    };
    if count <= 0 {
        stores.set_penalty_array(kind, &[], global);
        return Ok(());
    }
    let count = count as usize;
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| ExecError::ArithmeticOverflow)?;
    for _ in 0..count {
        values.push(scan_i32(input, stores, execution, context)?);
    }
    stores.set_penalty_array(kind, &values, global);
    Ok(())
}

fn assign_prevdepth(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<(), ExecError> {
    skip_optional_equals_x(input, stores, execution)?;
    let depth = scan_scaled(input, stores, execution, context)?;
    nest.current_list_mut().set_prev_depth(depth);
    Ok(())
}

fn assign_prevgraf(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<(), ExecError> {
    skip_optional_equals_x(input, stores, execution)?;
    let lines = scan_i32(input, stores, execution, context)?;
    if lines < 0 {
        // TeX.web §1247 reports the invalid value and leaves the enclosing
        // vertical list's prev_graf unchanged.
        stores.world_mut().write_text(
            tex_state::PrintSink::TerminalAndLog,
            &format!("\n! Bad \\prevgraf ({lines}).\nI allow only nonnegative values here.\n"),
        );
        return Ok(());
    }
    nest.set_enclosing_vertical_prev_graf(lines);
    Ok(())
}
