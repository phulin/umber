use tex_lex::{InputSource, InputStack, TokenListReplayKind};
use tex_state::Universe;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::glue::GlueSpec;
use tex_state::node::{GlueKind, Node};
use tex_state::scaled::Scaled;
use tex_typeset::PackSpec;
use tex_typeset::linebreak::{
    HyphenationHook, LineBreakParams, LineShape, LineShapeEntry,
    ParagraphShape as TypesetParagraphShape, PostLineBreakParams, line_break, post_line_break,
};

use super::boxes::hpack_with_overfull_rule;
use super::*;
use crate::mode::{IGNORE_DEPTH, ParagraphParams, ParagraphShape, ParagraphShapeLine};
use crate::vertical::{append_migrated_contribution, append_node_to_current_list};
use crate::{ExecError, Mode, ModeNest};

pub(super) fn execute_paragraph_command<S, H>(
    primitive: UnexpandablePrimitive,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    match primitive {
        UnexpandablePrimitive::Par | UnexpandablePrimitive::EndGraf => end_paragraph(nest, stores),
        UnexpandablePrimitive::Indent => start_paragraph(nest, input, stores, true),
        UnexpandablePrimitive::NoIndent => start_paragraph(nest, input, stores, false),
        UnexpandablePrimitive::ParShape => assign_parshape(nest, input, stores, hooks),
        UnexpandablePrimitive::PrevDepth => assign_prevdepth(nest, input, stores, hooks),
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
            let par_shape = nest.current_list().par_shape().cloned();
            let parskip = stores.glue_param(GlueParam::PAR_SKIP);
            if stores.glue(parskip) != GlueSpec::ZERO {
                nest.current_list_mut().push(Node::Glue {
                    spec: parskip,
                    kind: GlueKind::Normal,
                });
            }
            nest.push(Mode::Horizontal);
            if let Some(shape) = par_shape {
                nest.current_list_mut().set_par_shape(shape);
            }
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
            token: tex_state::token::Token::Cs(stores.intern("par")),
            operation: "paragraph start",
        }),
    }
}

fn append_indent_box(nest: &mut ModeNest, stores: &mut Universe) -> Result<(), ExecError> {
    let empty = stores.freeze_node_list(&[]);
    let par_indent = stores.dimen_param(DimenParam::PAR_INDENT);
    let mut node = hpack_with_overfull_rule(stores, empty, PackSpec::Exactly(par_indent));
    node.height = Scaled::from_raw(0);
    node.depth = Scaled::from_raw(0);
    nest.current_list_mut().push(Node::HList(node));
    Ok(())
}

fn end_paragraph(nest: &mut ModeNest, stores: &mut Universe) -> Result<(), ExecError> {
    if !matches!(nest.current_mode(), Mode::Horizontal) {
        return Ok(());
    }
    flush_pending_hchars(nest, stores)?;
    let params = snapshot_paragraph_params(nest, stores);
    trim_trailing_glue(nest.current_list_mut());
    nest.current_list_mut().push(Node::Penalty(10_000));
    nest.current_list_mut().push(Node::Glue {
        spec: params.par_fill_skip,
        kind: GlueKind::ParFillSkip,
    });
    let level = nest.pop()?;
    let hlist = level.list().nodes();
    let line_params = line_break_params(&params);
    let hyphenated = super::hyphenation::hyphenated_hlist(stores, hlist);
    let mut hook = ExecHyphenationHook { hyphenated };
    let decisions = line_break(stores, hlist, line_params, &mut hook);
    let post_params = post_line_break_params(&params);
    for broken in post_line_break(stores, &decisions.nodes, &decisions.breaks, post_params) {
        let list = stores.freeze_node_list(&broken.nodes);
        let line =
            hpack_with_overfull_rule(stores, list, PackSpec::Exactly(broken.dimensions.width));
        let mut line = line;
        line.shift = broken.dimensions.indent;
        append_node_to_current_list(nest, stores, Node::HList(line))?;
        for node in broken.migrated {
            append_migrated_contribution(nest, node);
        }
        if let Some(penalty) = broken.penalty_after {
            nest.current_list_mut().push(Node::Penalty(penalty));
        }
    }
    reset_after_par(nest, stores);
    Ok(())
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
        par_shape: nest.current_list().par_shape().cloned(),
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

fn line_break_params(params: &ParagraphParams) -> LineBreakParams {
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
        shape: line_shape(params),
    }
}

fn post_line_break_params(params: &ParagraphParams) -> PostLineBreakParams {
    PostLineBreakParams {
        left_skip: params.left_skip,
        right_skip: params.right_skip,
        interline_penalty: params.interline_penalty,
        club_penalty: params.club_penalty,
        widow_penalty: params.widow_penalty,
        broken_penalty: params.broken_penalty,
        shape: line_shape(params),
    }
}

fn line_shape(params: &ParagraphParams) -> LineShape {
    LineShape {
        hsize: params.hsize,
        parshape: params
            .par_shape
            .as_ref()
            .map(|shape| TypesetParagraphShape {
                lines: shape
                    .lines()
                    .iter()
                    .map(|line| LineShapeEntry {
                        indent: line.indent,
                        width: line.width,
                    })
                    .collect(),
            }),
        hang_indent: params.hang_indent,
        hang_after: params.hang_after,
    }
}

fn reset_after_par(nest: &mut ModeNest, stores: &mut Universe) {
    nest.current_list_mut().reset_par_shape();
    stores.set_dimen_param(DimenParam::HANG_INDENT, Scaled::from_raw(0));
    stores.set_int_param(IntParam::HANG_AFTER, 1);
}

fn trim_trailing_glue(list: &mut crate::ModeList) {
    while matches!(list.nodes().last(), Some(Node::Glue { .. })) {
        let _ = list.pop_last_node();
    }
}

fn assign_parshape<S, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    skip_optional_equals_x(input, stores, hooks)?;
    let count = scan_i32(input, stores, hooks)?.max(0) as usize;
    let mut lines = Vec::with_capacity(count);
    for _ in 0..count {
        lines.push(ParagraphShapeLine {
            indent: scan_scaled(input, stores, hooks)?,
            width: scan_scaled(input, stores, hooks)?,
        });
    }
    nest.current_list_mut()
        .set_par_shape(ParagraphShape::new(lines));
    Ok(())
}

fn assign_prevdepth<S, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    skip_optional_equals_x(input, stores, hooks)?;
    let depth = scan_scaled(input, stores, hooks)?;
    nest.current_list_mut().set_prev_depth(depth);
    Ok(())
}
