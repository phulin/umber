use tex_lex::{InputSource, InputStack, TokenListReplayKind};
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::node::{BoxNode, GlueKind, Node};
use tex_state::scaled::Scaled;
use tex_state::{ParagraphShapeLine, Universe};
use tex_typeset::PackSpec;
use tex_typeset::linebreak::{
    HyphenationHook, LineBreakParams, LineDimensions, LineShape, LineShapeEntry,
    ParagraphShape as TypesetParagraphShape, PostLineBreakParams, line_break, post_line_break,
};

use super::boxes::hpack_with_overfull_rule;
use super::*;
use crate::mode::{IGNORE_DEPTH, ParagraphParams};
use crate::vertical::{
    append_migrated_contribution, append_node_to_current_list, append_vertical_contribution,
    build_page_if_outer_vertical,
};
use crate::{ExecError, Mode, ModeNest};

pub(super) fn execute_paragraph_command<S, H>(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    global: bool,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    match primitive {
        UnexpandablePrimitive::Par | UnexpandablePrimitive::EndGraf => end_paragraph(nest, stores),
        UnexpandablePrimitive::Indent => start_paragraph(nest, input, stores, true),
        UnexpandablePrimitive::NoIndent => start_paragraph(nest, input, stores, false),
        UnexpandablePrimitive::ParShape => assign_parshape(input, stores, hooks, context, global),
        UnexpandablePrimitive::PrevDepth => assign_prevdepth(nest, input, stores, hooks, context),
        UnexpandablePrimitive::PrevGraf => assign_prevgraf(nest, input, stores, hooks, context),
        UnexpandablePrimitive::NoInterlineSkip => {
            nest.current_list_mut().set_prev_depth(IGNORE_DEPTH);
            Ok(())
        }
        _ => unreachable!("caller restricts paragraph commands"),
    }
}

pub(crate) fn ensure_horizontal_for_character<S>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
        start_paragraph(nest, input, stores, true)?;
    }
    Ok(())
}

fn start_paragraph<S>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    indent: bool,
) -> Result<(), ExecError>
where
    S: InputSource,
{
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
    if !matches!(nest.current_mode(), Mode::Horizontal) {
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
    let _ = break_current_paragraph(nest, stores, final_widow_penalty, true)?;
    Ok(())
}

pub(crate) struct ParagraphBreakResult {
    pub(crate) last_line: Option<BoxNode>,
}

pub(crate) fn interrupt_paragraph_for_display(
    nest: &mut ModeNest,
    stores: &mut Universe,
) -> Result<ParagraphBreakResult, ExecError> {
    flush_pending_hchars(nest, stores)?;
    if nest.current_list().is_empty() {
        let _ = nest.pop()?;
        return Ok(ParagraphBreakResult { last_line: None });
    }
    let final_widow_penalty = stores.int_param(IntParam::DISPLAY_WIDOW_PENALTY);
    break_current_paragraph(nest, stores, final_widow_penalty, false)
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
        emergency_stretch: stores.dimen_param(DimenParam::EMERGENCY_STRETCH),
        hsize: stores.dimen_param(DimenParam::H_SIZE),
        interline_penalty: stores.int_param(IntParam::INTERLINE_PENALTY),
        club_penalty: stores.int_param(IntParam::CLUB_PENALTY),
        widow_penalty: stores.int_param(IntParam::WIDOW_PENALTY),
        broken_penalty: stores.int_param(IntParam::BROKEN_PENALTY),
    };
    line_shape(&params).dimensions(2)
}

fn break_current_paragraph(
    nest: &mut ModeNest,
    stores: &mut Universe,
    final_widow_penalty: i32,
    reset_paragraph: bool,
) -> Result<ParagraphBreakResult, ExecError> {
    flush_pending_hchars(nest, stores)?;
    let params = snapshot_paragraph_params(nest, stores);
    remove_final_glue(nest.current_list_mut());
    nest.current_list_mut().push(Node::Penalty(10_000));
    nest.current_list_mut().push(Node::Glue {
        spec: params.par_fill_skip,
        kind: GlueKind::ParFillSkip,
        leader: None,
    });
    let level = nest.pop()?;
    let hlist = crate::math::finish_math_lists(stores, level.list().nodes(), true);
    let line_params = line_break_params(stores, &params);
    let hyphenated = super::hyphenation::hyphenated_hlist(stores, &hlist);
    let mut hook = ExecHyphenationHook { hyphenated };
    let decisions = line_break(stores, &hlist, line_params, &mut hook);
    let post_params = post_line_break_params(&params, final_widow_penalty);
    let mut line_count = 0i32;
    let mut last_line = None;
    for mut broken in post_line_break(stores, &decisions.nodes, &decisions.breaks, post_params) {
        line_count += 1;
        let migrated = extract_migrating_material(stores, &mut broken.nodes);
        let list = stores.freeze_node_list(&broken.nodes);
        let line =
            hpack_with_overfull_rule(stores, list, PackSpec::Exactly(broken.dimensions.width));
        let mut line = line;
        line.shift = broken.dimensions.indent;
        last_line = Some(line);
        append_node_to_current_list(nest, stores, Node::HList(line))?;
        for node in migrated {
            append_migrated_contribution(nest, stores, node);
        }
        if let Some(penalty) = broken.penalty_after {
            append_vertical_contribution(nest, stores, Node::Penalty(penalty));
        }
    }
    nest.current_list_mut()
        .set_prev_graf(params.prev_graf.saturating_add(line_count));
    if reset_paragraph {
        reset_after_par(nest, stores);
    }
    build_page_if_outer_vertical(nest, stores)?;
    Ok(ParagraphBreakResult { last_line })
}

fn extract_migrating_material(stores: &Universe, nodes: &mut Vec<Node>) -> Vec<Node> {
    let mut retained = Vec::with_capacity(nodes.len());
    let mut migrated = Vec::new();
    for node in nodes.drain(..) {
        match node {
            Node::Mark { .. } | Node::Ins { .. } => migrated.push(node),
            Node::Adjust(list) => {
                migrated.extend(stores.nodes(list).into_iter().map(|node| node.to_owned()))
            }
            node => retained.push(node),
        }
    }
    *nodes = retained;
    migrated
}

struct ExecHyphenationHook {
    hyphenated: Vec<Node>,
}

impl HyphenationHook<Universe> for ExecHyphenationHook {
    fn hyphenate(&mut self, _nodes: &[Node]) -> Vec<Node> {
        self.hyphenated.clone()
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
        emergency_stretch: stores.dimen_param(DimenParam::EMERGENCY_STRETCH),
        hsize: stores.dimen_param(DimenParam::H_SIZE),
        interline_penalty: stores.int_param(IntParam::INTERLINE_PENALTY),
        club_penalty: stores.int_param(IntParam::CLUB_PENALTY),
        widow_penalty: stores.int_param(IntParam::WIDOW_PENALTY),
        broken_penalty: stores.int_param(IntParam::BROKEN_PENALTY),
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
        emergency_stretch: params.emergency_stretch,
        looseness: params.looseness,
        left_skip: stores.glue(params.left_skip),
        right_skip: stores.glue(params.right_skip),
        shape: line_shape(params),
    }
}

fn post_line_break_params(
    params: &ParagraphParams,
    final_widow_penalty: i32,
) -> PostLineBreakParams {
    PostLineBreakParams {
        left_skip: params.left_skip,
        right_skip: params.right_skip,
        interline_penalty: params.interline_penalty,
        club_penalty: params.club_penalty,
        widow_penalty: final_widow_penalty,
        broken_penalty: params.broken_penalty,
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

fn assign_parshape<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: TracedTokenWord,
    global: bool,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    skip_optional_equals_x(input, stores, hooks)?;
    let count = scan_i32(input, stores, hooks, context)?.max(0) as usize;
    let mut lines = Vec::with_capacity(count);
    for _ in 0..count {
        lines.push(ParagraphShapeLine {
            indent: scan_scaled(input, stores, hooks, context)?,
            width: scan_scaled(input, stores, hooks, context)?,
        });
    }
    stores.set_paragraph_shape(&lines, global);
    Ok(())
}

fn assign_prevdepth<S, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: TracedTokenWord,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    skip_optional_equals_x(input, stores, hooks)?;
    let depth = scan_scaled(input, stores, hooks, context)?;
    nest.current_list_mut().set_prev_depth(depth);
    Ok(())
}

fn assign_prevgraf<S, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: TracedTokenWord,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    skip_optional_equals_x(input, stores, hooks)?;
    let lines = scan_i32(input, stores, hooks, context)?;
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
