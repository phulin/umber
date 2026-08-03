//! Retired `Executor` page-output and output-routine input fronts.

use tex_expand::get_x_or_protected_with_context;
use tex_lex::{InputStack, MemoryInput};
use tex_state::env::banks::{IntParam, TokParam};
use tex_state::page::{PageFireUp, PageInteger};
use tex_state::{GroupKind, TokenListReplayKind, Universe};

use crate::canonical_page_output::{
    append_end_job_contributions, job_is_quiescent, prepare_box255, prepend_output_heldover,
    report_box255_not_emptied, report_output_loop, take_box255_node,
};
use crate::executor::{MainControlExit, run_main_control_until};
use crate::legacy_assignments::shipout_node;
use crate::mode::ignored_depth;
use crate::page_builder::build_page;
use crate::{ExecError, ExecutionStats, Mode, ModeNest, leave_group, push_traced_tokens};

pub(crate) fn expand_shipout_write(
    stores: &mut Universe,
    expansion: &mut tex_expand::ExpansionContext<'_>,
    tokens: tex_state::ids::TokenListId,
) -> Result<crate::canonical_shipout::ExpandedWrite, ExecError> {
    let mut input = InputStack::empty();
    input.push_token_list(tokens, TokenListReplayKind::Inserted);
    let mut text = String::new();
    expansion.with_expanded_token_list(|expansion| -> Result<(), ExecError> {
        while let Some(token) = get_x_or_protected_with_context(
            &mut input,
            &mut tex_state::ExpansionContext::new(stores),
            expansion,
        )?
        .map(|token| token.semantic_token())
        {
            tex_state::token_show::append_token_string_text(stores, token, &mut text);
        }
        Ok(())
    })?;
    let mut text = crate::diagnostics::print_text_with_newlinechar(stores, &text);
    text.push('\n');
    Ok(crate::canonical_shipout::ExpandedWrite(text))
}

pub(crate) fn expand_shipout_text(
    stores: &mut Universe,
    expansion: &mut tex_expand::ExpansionContext<'_>,
    kind: crate::canonical_shipout::ReplayTextKind,
    tokens: tex_state::ids::TokenListId,
) -> Result<crate::canonical_shipout::ExpandedReplayText, ExecError> {
    let mut input = match kind {
        crate::canonical_shipout::ReplayTextKind::Special => InputStack::empty(),
        crate::canonical_shipout::ReplayTextKind::PdfLiteral => {
            InputStack::new(MemoryInput::new(""))
        }
    };
    input.push_token_list(tokens, TokenListReplayKind::Inserted);
    let mut text = String::new();
    let mut collect = |expansion: &mut tex_expand::ExpansionContext<'_>| -> Result<(), ExecError> {
        while let Some(token) = get_x_or_protected_with_context(
            &mut input,
            &mut tex_state::ExpansionContext::new(stores),
            expansion,
        )?
        .map(|token| token.semantic_token())
        {
            match kind {
                crate::canonical_shipout::ReplayTextKind::Special => {
                    tex_state::token_show::append_token_string_text(stores, token, &mut text);
                }
                crate::canonical_shipout::ReplayTextKind::PdfLiteral => {
                    crate::diagnostics::append_token_show_text(stores, token, &mut text);
                }
            }
        }
        Ok(())
    };
    match kind {
        crate::canonical_shipout::ReplayTextKind::Special => {
            expansion.with_expanded_token_list(&mut collect)?;
        }
        crate::canonical_shipout::ReplayTextKind::PdfLiteral => collect(expansion)?,
    }
    Ok(crate::canonical_shipout::ExpandedReplayText(
        text.into_bytes(),
    ))
}

pub(crate) fn drain_pending_output(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    stats: &mut ExecutionStats,
) -> Result<(), ExecError> {
    while let Some(fire_up) = stores.page_fire_up() {
        fire_up_page(nest, input, stores, execution, stats, fire_up)?;
    }
    Ok(())
}

pub(crate) fn finish_end(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    stats: &mut ExecutionStats,
) -> Result<(), ExecError> {
    while !job_is_quiescent(stores) {
        append_end_job_contributions(stores);
        build_page(stores)?;
        drain_pending_output(nest, input, stores, execution, stats)?;
    }
    Ok(())
}

fn fire_up_page(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    stats: &mut ExecutionStats,
    fire_up: PageFireUp,
) -> Result<(), ExecError> {
    prepare_box255(stores, fire_up, None)?;
    let output = stores.tok_param(TokParam::OUTPUT);
    if stores.tokens(output).is_empty() {
        prepend_output_heldover(stores, Vec::new(), true);
        let node = take_box255_node(stores)?;
        let _artifact = shipout_node(node, input, stores, execution)?;
        stores.clear_page_discards();
        build_page(stores)?;
        return Ok(());
    }

    let dead_cycles = stores.page_integer(PageInteger::DeadCycles);
    let max_dead_cycles = stores.int_param(IntParam::MAX_DEAD_CYCLES);
    if dead_cycles >= max_dead_cycles {
        let context = crate::diagnostics::show_context(stores, stores.input_summary());
        report_output_loop(stores, dead_cycles, context)?;
        prepend_output_heldover(stores, Vec::new(), true);
        let node = take_box255_node(stores)?;
        let _artifact = shipout_node(node, input, stores, execution)?;
        stores.clear_page_discards();
        build_page(stores)?;
        return Ok(());
    }
    stores.record_output_routine_execution();
    stores.set_page_integer(
        PageInteger::DeadCycles,
        dead_cycles
            .checked_add(1)
            .expect("dead-cycle count is bounded by max_dead_cycles"),
    );
    run_output_routine(nest, input, stores, execution, stats, output)?;
    stores.clear_page_discards();
    build_page(stores)?;
    Ok(())
}

fn run_output_routine(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    stats: &mut ExecutionStats,
    output: tex_state::ids::TokenListId,
) -> Result<(), ExecError> {
    execution.mark_paragraph_barrier(tex_state::ParagraphBarrierReason::NestedOutputRoutine);
    let mut transaction = crate::transaction::ExecutionTransaction::begin(nest, stores);
    let mut replay = None;
    let result = {
        let (nest, stores) = transaction.parts();
        run_output_routine_inner(nest, input, stores, execution, stats, output, &mut replay)
    };
    if result.is_ok() {
        transaction.commit();
    } else if let Some(replay) = replay {
        let _ = input.abort_token_list_replay(replay);
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn run_output_routine_inner(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    stats: &mut ExecutionStats,
    output: tex_state::ids::TokenListId,
    replay: &mut Option<tex_state::TokenListReplayMarker>,
) -> Result<(), ExecError> {
    stores.set_output_routine_active(true);
    stores.enter_group_with_kind(GroupKind::Output);
    nest.push(Mode::InternalVertical)?;
    nest.current_list_mutation()
        .set_prev_depth(ignored_depth(stores));
    crate::legacy_assignments::normal_paragraph(nest, stores);
    let output_replay = input.push_token_list(output, TokenListReplayKind::OutputRoutine);
    *replay = Some(output_replay);

    match run_main_control_until(nest, input, stores, execution, stats, |input, stores| {
        pop_finished_output_frame(input, stores, output_replay)
    })? {
        MainControlExit::Stopped => {}
        MainControlExit::EndOfInput => {
            if !input.finish_exhausted_token_list_replay(output_replay, stores) {
                return Err(ExecError::MissingToken {
                    context: "output routine",
                });
            }
        }
        MainControlExit::End { token } => push_traced_tokens(input, stores, [token]),
        MainControlExit::NotConsumed { token } => {
            return Err(ExecError::UnimplementedTypesetting {
                mode: nest.current_mode(),
                token: token.semantic_token(),
                origin: token.origin(),
                operation: "output routine",
            });
        }
    }

    let output_level =
        crate::legacy_assignments::commit_current_list(nest, stores, execution.command_fuel())?;
    leave_group(input, stores, GroupKind::Output)?;
    stores.set_output_routine_active(false);
    if let Some(box255) = stores.box_reg(255) {
        stores.clear_box_reg_same_level(255);
        let context = crate::diagnostics::show_context(stores, stores.input_summary());
        report_box255_not_emptied(stores, box255, context)?;
    }
    prepend_output_heldover(stores, output_level.list().nodes().to_vec(), false);
    Ok(())
}

fn pop_finished_output_frame(
    input: &mut InputStack,
    stores: &Universe,
    output_replay: tex_state::TokenListReplayMarker,
) -> bool {
    input.finish_exhausted_token_list_replay(output_replay, stores)
}
