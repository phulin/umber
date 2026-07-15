use tex_lex::{InputStack, TokenListReplayKind};
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::node::{BoxNode, Direction, GlueKind, Node};
use tex_state::scaled::Scaled;
use tex_state::{ParagraphShapeLine, PenaltyArrayKind, Universe};
use tex_typeset::PackSpec;
use tex_typeset::linebreak::{
    LineBreakParams, LineBreakResult, LineDimensions, LineMaterializer, LineShape, LineShapeEntry,
    ParagraphShape as TypesetParagraphShape, PostLineBreakParams, line_break_hyphenated,
    try_line_break_without_hyphenation,
};

use super::boxes::hpack_owned_with_overfull_rule;
use super::*;
use crate::mode::{IGNORE_DEPTH, ParagraphParams};
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
                normal_paragraph(nest, stores);
                build_page_if_outer_vertical(nest, stores)
            } else {
                end_paragraph(nest, stores)
            }
        }
        UnexpandablePrimitive::Indent => start_paragraph(nest, input, stores, true),
        UnexpandablePrimitive::NoIndent => start_paragraph(nest, input, stores, false),
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
            nest.current_list_mut().set_prev_depth(IGNORE_DEPTH);
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
        start_paragraph(nest, input, stores, true)?;
    }
    Ok(())
}

fn start_paragraph(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    indent: bool,
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
            let everypar = stores.tok_param(TokParam::EVERY_PAR);
            if !stores.tokens(everypar).is_empty() {
                input.push_token_list(everypar, TokenListReplayKind::EveryPar);
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
    )?;
    Ok(())
}

pub(crate) struct ParagraphBreakResult {
    pub(crate) last_line: Option<BoxNode>,
    pub(crate) active_directions: Vec<Direction>,
}

pub(crate) fn interrupt_paragraph_for_display(
    nest: &mut ModeNest,
    stores: &mut Universe,
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
    let line_params = line_break_params(stores, &params);
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
    let mut materializer = LineMaterializer::new(decisions.nodes, decisions.breaks, post_params);
    let mut line_nodes = Vec::new();
    let mut migrated = Vec::new();
    while let Some(mut broken) = materializer.materialize_next(stores, line_nodes) {
        line_count += 1;
        extract_migrating_material(stores, &mut broken.nodes, &mut migrated);
        let line = hpack_owned_with_overfull_rule(
            stores,
            &mut broken.nodes,
            PackSpec::Exactly(broken.dimensions.width),
        );
        let mut line = line;
        line.shift = broken.dimensions.indent;
        last_line = Some(line);
        append_node_to_current_list(nest, stores, Node::HList(line))?;
        for node in migrated.drain(..) {
            append_migrated_contribution(nest, stores, node);
        }
        if let Some(penalty) = broken.penalty_after {
            append_vertical_contribution(nest, stores, Node::Penalty(penalty));
        }
        line_nodes = broken.nodes;
    }
    nest.current_list_mut()
        .set_prev_graf(params.prev_graf.saturating_add(line_count));
    if reset_paragraph {
        reset_after_par(nest, stores);
    }
    build_page_if_outer_vertical(nest, stores)?;
    Ok(ParagraphBreakResult {
        last_line,
        active_directions,
    })
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
    if let Some(first) = try_line_break_without_hyphenation(stores, &hlist, &line_params) {
        first.with_nodes(hlist)
    } else {
        let hyphenated = super::hyphenation::hyphenated_hlist(stores, &hlist);
        line_break_hyphenated(stores, &hyphenated, &line_params).with_nodes(hyphenated)
    }
}

fn extract_migrating_material(stores: &Universe, nodes: &mut Vec<Node>, migrated: &mut Vec<Node>) {
    migrated.clear();
    for node in nodes.extract_if(.., |node| {
        matches!(node, Node::Mark { .. } | Node::Ins { .. } | Node::Adjust(_))
    }) {
        match node {
            Node::Mark { .. } | Node::Ins { .. } => migrated.push(node),
            Node::Adjust(list) => {
                migrated.extend(stores.nodes(list).into_iter().map(|node| node.to_owned()))
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
