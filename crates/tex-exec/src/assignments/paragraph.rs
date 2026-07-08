use tex_lex::{InputSource, InputStack, TokenListReplayKind};
use tex_state::Universe;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::node::{GlueKind, Node};
use tex_state::scaled::Scaled;
use tex_typeset::{HpackParams, PackSpec, hpack};

use super::*;
use crate::mode::{IGNORE_DEPTH, ParagraphParams, ParagraphShape, ParagraphShapeLine};
use crate::vertical::append_node_to_current_list;
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
            nest.current_list_mut().push(Node::Glue {
                spec: parskip,
                kind: GlueKind::Normal,
            });
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
    let mut node = hpack(
        stores,
        empty,
        PackSpec::Exactly(stores.dimen_param(DimenParam::PAR_INDENT)),
        HpackParams::read(stores),
    )
    .node;
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
        kind: GlueKind::Normal,
    });
    let level = nest.pop()?;
    let list = stores.freeze_node_list(level.list().nodes());
    let line = hpack(stores, list, PackSpec::Natural, HpackParams::read(stores)).node;
    append_node_to_current_list(nest, stores, Node::HList(line))?;
    reset_after_par(nest, stores);
    Ok(())
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
