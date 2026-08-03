use tex_lex::InputStack;
use tex_state::TokenListReplayKind;
use tex_state::env::banks::{DimenParam, GlueParam, TokParam};
use tex_state::glue::Order;
use tex_state::math::{MathField, MathNoad, NoadClass, NoadKind};
use tex_state::node::{BoxNode, BoxNodeFields, GlueKind, Node, Sign};
use tex_state::scaled::{GlueSetRatio, Scaled};
use tex_state::{ParagraphShapeLine, PenaltyArrayKind, Universe};

use super::*;
use crate::canonical_box_runtime::append_node_to_current_list;
use crate::canonical_paragraph_end::{
    ParagraphBreakResult, break_current_paragraph, normal_paragraph,
};
use crate::legacy_paragraph_memo::ParagraphMemoConsumer;
use crate::vertical::{
    append_migrated_contribution, append_vertical_contribution, build_page_if_outer_vertical,
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
        UnexpandablePrimitive::Par => {
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
        UnexpandablePrimitive::Indent => {
            start_paragraph(nest, input, stores, true, true, execution.command_fuel())
        }
        UnexpandablePrimitive::NoIndent => {
            start_paragraph(nest, input, stores, false, true, execution.command_fuel())
        }
        UnexpandablePrimitive::QuitVMode => {
            if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
                start_paragraph(nest, input, stores, true, true, execution.command_fuel())
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
        _ => unreachable!("caller restricts paragraph commands"),
    }
}

pub(crate) fn ensure_horizontal_for_character(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
        start_paragraph(nest, input, stores, true, true, fuel)?;
    }
    Ok(())
}

/// Performs TeX82 §1091 `new_graf` after command control has retained the
/// triggering command and scheduled `\everypar`.  Canonical main control
/// owns that scheduling; this helper deliberately accepts no input source.
///
/// `new_graf` is reachable only from §1090's `vmode+...` cases, so this
/// helper is only ever entered from (internal) vertical mode.  §1092 sends
/// `hmode+start_par` and `mmode+start_par` to `indent_in_hmode` instead,
/// which starts no paragraph and therefore pushes no `\everypar`.
pub(crate) fn start_canonical_paragraph(
    nest: &mut ModeNest,
    stores: &mut Universe,
    indent: bool,
) -> Result<(), ExecError> {
    match nest.current_mode() {
        Mode::Vertical | Mode::InternalVertical => {
            nest.set_enclosing_vertical_prev_graf(0);
            let parskip = stores.glue_param(GlueParam::PAR_SKIP);
            if nest.current_mode() == Mode::Vertical || !nest.current_list().is_empty() {
                append_vertical_contribution(
                    nest,
                    stores,
                    Node::Glue {
                        spec: parskip,
                        // §1091's `new_param_glue(par_skip_code)` retains the
                        // parameter subtype that §182 displays.
                        kind: GlueKind::ParSkip,
                        leader: None,
                    },
                );
                build_page_if_outer_vertical(nest, stores)?;
            }
            nest.push_at_line(Mode::Horizontal, stores.current_input_line())?;
            // §1091's `mode_line:=line`, which §804 reports this paragraph's
            // over/underfull lines from.
            stores.push_paragraph_start_line(stores.current_input_line());
            if indent {
                append_indent_box(nest, stores)?;
            }
            Ok(())
        }
        mode => Err(ExecError::UnimplementedTypesetting {
            mode,
            token: tex_state::token::Token::Cs(stores.intern("par").symbol()),
            origin: OriginId::UNKNOWN,
            operation: "canonical paragraph start",
        }),
    }
}

/// TeX82 §1093 `indent_in_hmode`, which §1092 selects for both
/// `hmode+start_par` and `mmode+start_par`.
///
/// Neither `\indent` nor `\noindent` begins a paragraph once one is already
/// under way, so neither reaches §1091 `new_graf` and neither pushes
/// `\everypar`.  `\noindent` (`cur_chr=0`) contributes nothing at all.
/// `\indent` appends a `\parindent`-wide null box: in (restricted)
/// horizontal mode directly, resetting the space factor to 1000, and in math
/// mode wrapped in an ordinary noad whose nucleus is a `sub_box`.
pub(crate) fn indent_in_hmode(
    nest: &mut ModeNest,
    stores: &mut Universe,
    indent: bool,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    if !indent {
        return Ok(());
    }
    match nest.current_mode() {
        Mode::Math | Mode::DisplayMath => {
            let box_node = make_indent_box(stores);
            let list = stores.freeze_node_list(&[box_node]);
            nest.current_list_mutation()
                .push(Node::MathNoad(MathNoad::new(
                    NoadKind::Normal(NoadClass::Ord),
                    MathField::SubBox(list),
                )));
            Ok(())
        }
        _ => {
            flush_pending_hchars(nest, stores, fuel)?;
            nest.current_list_mutation().set_space_factor(1000);
            append_indent_box(nest, stores)
        }
    }
}

fn start_paragraph(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    indent: bool,
    replay_everypar: bool,
    fuel: &mut tex_command::CommandFuel,
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
                        // §1091's `new_param_glue(par_skip_code)` retains the
                        // parameter subtype that §182 displays.
                        kind: GlueKind::ParSkip,
                        leader: None,
                    },
                );
                build_page_if_outer_vertical(nest, stores)?;
            }
            nest.push_at_line(Mode::Horizontal, stores.current_input_line())?;
            // §1091's `mode_line:=line`.
            stores.push_paragraph_start_line(stores.current_input_line());
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
        // §1092 `hmode+start_par`: `indent_in_hmode`, not `new_graf`.
        Mode::Horizontal | Mode::RestrictedHorizontal => {
            indent_in_hmode(nest, stores, indent, fuel)
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
    nest.current_list_mutation().push(make_indent_box(stores));
    Ok(())
}

pub(crate) fn make_indent_box(stores: &mut Universe) -> Node {
    // TeX82 §1090 uses `new_null_box`, then assigns only its width. This is
    // an ordinary zeroed box constructor, not §649 `hpack`; in particular it
    // must not publish a packing geometry transition.
    Node::HList(BoxNode::new(BoxNodeFields {
        width: stores.dimen_param(DimenParam::PAR_INDENT),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        box_lr: tex_state::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: stores.freeze_node_list(&[]),
    }))
}

pub(crate) fn end_paragraph_with_fuel(
    nest: &mut ModeNest,
    stores: &mut Universe,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    if nest.current_mode() != Mode::Horizontal {
        return Ok(());
    }
    flush_pending_hchars_with_fuel(nest, stores, fuel)?;
    stores.begin_paragraph_break_dependency_region();
    if nest.current_list().is_empty() {
        let _ = crate::assignments::commit_current_list(nest, stores, fuel)?;
        normal_paragraph(nest, stores);
        build_page_if_outer_vertical(nest, stores)?;
        return Ok(());
    }
    let _ = break_current_paragraph(
        nest,
        stores,
        tex_typeset::linebreak::WidowPenaltySelector::Ordinary,
        true,
        None,
        fuel,
    )?;
    Ok(())
}

fn end_paragraph_with_memo(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    flush_pending_hchars_with_fuel(nest, stores, execution.command_fuel())?;
    if nest.current_mode() != Mode::Horizontal || nest.current_list().is_empty() {
        {
            let mut memo = crate::legacy_paragraph_memo::ExecutorParagraphMemoConsumer::new(
                input,
                execution,
                crate::executor::ParagraphContinuation::End,
            );
            memo.abandon();
        }
        return end_paragraph_with_fuel(nest, stores, execution.command_fuel());
    }
    {
        let mut memo = crate::legacy_paragraph_memo::ExecutorParagraphMemoConsumer::new(
            input,
            execution,
            crate::executor::ParagraphContinuation::End,
        );
        memo.prepare_hlist(
            stores,
            nest.current_list().nodes(),
            nest.enclosing_vertical_prev_graf(),
            crate::executor::ParagraphContinuation::End,
        );
    }
    let result = break_current_paragraph(
        nest,
        stores,
        tex_typeset::linebreak::WidowPenaltySelector::Ordinary,
        true,
        None,
        execution.command_fuel(),
    )?;
    let mut memo = crate::legacy_paragraph_memo::PendingParagraphMemoConsumer::new(execution);
    memo.publish_finished_lines(
        stores,
        &result.finished_nodes,
        result.line_count,
        &result.active_directions,
    );
    Ok(())
}

pub(crate) fn start_reused_paragraph(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    // The retained hlist already includes the recorded `everypar` execution;
    // scheduling it again would leave its tokens after the consumed paragraph.
    // Finished retained lines already contain the recorded indent box and
    // `\everypar` material, so reproduce only the vertical-side transition.
    start_paragraph(nest, input, stores, false, false, fuel)
}

pub(crate) fn install_reused_paragraph_hlist_after_start(
    nest: &mut ModeNest,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    nodes: Vec<Node>,
    finished: Option<(Vec<Node>, i32, i32)>,
    continuation: crate::executor::ParagraphContinuation,
) -> Result<Option<BoxNode>, ExecError> {
    debug_assert_eq!(nest.current_mode(), Mode::Horizontal);
    let _ = nest.current_list_mutation().take_nodes();
    nest.current_list_mutation().append(nodes);
    let Some((finished, line_count, last_badness)) = finished else {
        let result = break_current_paragraph(
            nest,
            stores,
            tex_typeset::linebreak::WidowPenaltySelector::Ordinary,
            true,
            None,
            execution.command_fuel(),
        )?;
        let mut memo = crate::legacy_paragraph_memo::PendingParagraphMemoConsumer::new(execution);
        memo.publish_finished_lines(
            stores,
            &result.finished_nodes,
            result.line_count,
            &result.active_directions,
        );
        return Ok(None);
    };
    let last_line = finished.iter().rev().find_map(|node| match node {
        Node::HList(line) => Some(*line),
        _ => None,
    });
    let _ = crate::assignments::commit_current_list(nest, stores, execution.command_fuel())?;
    for node in finished {
        match node {
            Node::Adjust(adjust) => {
                let adjusted = stores.nodes(adjust.content).to_vec();
                for node in adjusted {
                    append_migrated_contribution(nest, stores, node);
                }
            }
            node @ (Node::Mark { .. } | Node::Ins { .. }) => {
                append_migrated_contribution(nest, stores, node);
            }
            node => append_node_to_current_list(nest, stores, node, execution.command_fuel())?,
        }
    }
    let prev_graf = nest.enclosing_vertical_prev_graf();
    nest.current_list_mutation()
        .set_prev_graf(prev_graf.saturating_add(line_count));
    stores.set_last_badness(last_badness);
    if continuation == crate::executor::ParagraphContinuation::End {
        normal_paragraph(nest, stores);
    }
    build_page_if_outer_vertical(nest, stores)?;
    Ok(last_line)
}

pub(crate) fn interrupt_paragraph_for_display(
    nest: &mut ModeNest,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<ParagraphBreakResult, ExecError> {
    flush_pending_hchars(nest, stores, execution.command_fuel())?;
    if nest.current_list().is_empty() {
        let _ = commit_current_list(nest, stores, execution.command_fuel())?;
        return Ok(ParagraphBreakResult::empty());
    }
    let result = break_current_paragraph(
        nest,
        stores,
        tex_typeset::linebreak::WidowPenaltySelector::DisplayInterrupted,
        false,
        None,
        execution.command_fuel(),
    )?;
    let mut memo = crate::legacy_paragraph_memo::PendingParagraphMemoConsumer::new(execution);
    memo.publish_finished_lines(
        stores,
        &result.finished_nodes,
        result.line_count,
        &result.active_directions,
    );
    Ok(result)
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
    nest.current_list_mutation().set_prev_depth(depth);
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
        // TeX.web §1244's `alter_prev_graf` reports the invalid value through
        // §91's `int_error`, which parenthesizes it before §82's `error`
        // closes the message, and leaves the enclosing list's prev_graf alone.
        crate::error_report::report_input_error(
            input,
            stores,
            &format!("Bad \\prevgraf ({lines})"),
            &["I allow only nonnegative values here."],
        )?;
        return Ok(());
    }
    nest.set_enclosing_vertical_prev_graf(lines);
    Ok(())
}
