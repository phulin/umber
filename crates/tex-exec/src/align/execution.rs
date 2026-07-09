use tex_expand::{ExpansionHooks, ReadRecorder, get_x_token_with_recorder_and_hooks};
use tex_lex::{InputSource, InputStack, TokenListReplayKind};
use tex_state::env::banks::TokParam;
use tex_state::node::{GlueKind, Node};
use tex_state::token::{Token, TracedTokenWord};
use tex_state::{ExpansionContext, PrintSink, Universe};

use super::support::{
    align_kind, align_state, align_state_mut, alignment_mode, cell_mode, is_alignment_tab,
    is_begin_group, is_cr, is_end_group, is_noalign, is_omit, is_span, row_mode,
    set_align_brace_depth,
};
use crate::assignments::{flush_pending_hchars, next_non_space_traced_x};
use crate::dispatch::dispatch_delivered_token_with_recorder;
use crate::executor::sync_engine_state;
use crate::mode::{AlignState, AlignmentKind};
use crate::vertical::{append_vertical_contribution, build_page_if_outer_vertical};
use crate::{
    DispatchAction, ExecError, ExecutionStats, Mode, ModeNest, leave_group, push_traced_tokens,
};

pub(crate) fn execute_alignment<S, R, H>(
    state: AlignState,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    stores.with_hash_only_checkpoints(|stores| {
        let alignment_kind = state.kind();
        nest.push(alignment_mode(alignment_kind));
        let align_level = nest.depth() - 1;
        nest.current_list_mut().set_align_state(state);
        replay_everycr(input, stores);

        while let Some(first_token) = align_peek(align_level, nest, input, stores, recorder, hooks)?
        {
            init_row(align_level, nest)?;
            execute_row(
                align_level,
                first_token,
                nest,
                input,
                stores,
                recorder,
                hooks,
            )?;
            fin_row(align_level, nest, stores)?;
            replay_everycr(input, stores);
        }

        let finished = finish_alignment_level(nest, stores)?;
        for node in finished {
            append_finished_alignment_node(nest, stores, node);
        }
        build_page_if_outer_vertical(nest, stores)?;
        Ok(())
    })
}

fn append_finished_alignment_node(nest: &mut ModeNest, stores: &mut Universe, node: Node) {
    if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
        update_prev_depth_for_finished_alignment_node(nest, &node);
        append_vertical_contribution(nest, stores, node);
    } else {
        nest.current_list_mut().push(node);
    }
}

fn update_prev_depth_for_finished_alignment_node(nest: &mut ModeNest, node: &Node) {
    match node {
        Node::HList(box_node) | Node::VList(box_node) => {
            nest.current_list_mut().set_prev_depth(box_node.depth);
        }
        Node::Rule { .. } => nest
            .current_list_mut()
            .set_prev_depth(crate::mode::IGNORE_DEPTH),
        _ => {}
    }
}

pub(super) fn execute_alignment_to_nodes<S, R, H>(
    state: AlignState,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<Vec<Node>, ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    stores.with_hash_only_checkpoints(|stores| {
        let alignment_kind = state.kind();
        nest.push(alignment_mode(alignment_kind));
        let align_level = nest.depth() - 1;
        nest.current_list_mut().set_align_state(state);
        replay_everycr(input, stores);

        while let Some(first_token) = align_peek(align_level, nest, input, stores, recorder, hooks)?
        {
            init_row(align_level, nest)?;
            execute_row(
                align_level,
                first_token,
                nest,
                input,
                stores,
                recorder,
                hooks,
            )?;
            fin_row(align_level, nest, stores)?;
            replay_everycr(input, stores);
        }

        finish_alignment_level(nest, stores)
    })
}

fn finish_alignment_level(
    nest: &mut ModeNest,
    stores: &mut Universe,
) -> Result<Vec<Node>, ExecError> {
    let mut level = nest.pop()?;
    let state = level
        .list_mut()
        .take_align_state()
        .ok_or(ExecError::MissingToken {
            context: "alignment state",
        })?;
    let nodes = level.list().nodes().to_vec();
    let finished = super::widths::finish_alignment(&state, &nodes, stores)?;
    Ok(finished)
}

fn replay_everycr<S>(input: &mut InputStack<S>, stores: &Universe) {
    let everycr = stores.tok_param(TokParam::EVERY_CR);
    if !stores.tokens(everycr).is_empty() {
        input.push_token_list(everycr, TokenListReplayKind::EveryCr);
    }
}

fn align_peek<S, H>(
    align_level: usize,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut impl ReadRecorder,
    hooks: &mut H,
) -> Result<Option<TracedTokenWord>, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    loop {
        set_align_brace_depth(nest, align_level, 1_000_000);
        let Some(token) = next_non_space_traced_x(input, stores, hooks)? else {
            return Err(ExecError::MissingToken {
                context: "alignment row",
            });
        };
        set_align_brace_depth(nest, align_level, 0);
        let semantic = tex_expand::semantic_token(token);
        if is_noalign(stores, semantic) {
            super::noalign::execute_noalign(align_level, nest, input, stores, recorder, hooks)?;
            continue;
        }
        if is_end_group(stores, semantic) {
            leave_group(input, stores, tex_state::GroupKind::Simple)?;
            return Ok(None);
        }
        if is_cr(stores, semantic) {
            continue;
        }
        return Ok(Some(token));
    }
}

fn init_row(align_level: usize, nest: &mut ModeNest) -> Result<(), ExecError> {
    let kind = align_kind(nest, align_level)?;
    let first_tabskip = align_state(nest, align_level)?.tabskip_for_boundary(0);
    align_state_mut(nest, align_level)?.start_row();
    nest.push(row_mode(kind));
    if kind == AlignmentKind::HAlign {
        nest.current_list_mut().set_space_factor(0);
    }
    nest.current_list_mut().push(Node::Glue {
        spec: first_tabskip,
        kind: GlueKind::TabSkip,
        leader: None,
    });
    Ok(())
}

fn execute_row<S, R, H>(
    align_level: usize,
    first_token: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let mut start_token = Some(first_token);
    let mut column = 0usize;
    loop {
        let result = execute_cell(
            align_level,
            CellStart {
                column,
                first_token: start_token.take(),
            },
            nest,
            input,
            stores,
            recorder,
            hooks,
        )?;
        column = result.next_column;
        if result.ended_row {
            return Ok(());
        }
        start_token = Some(next_non_space_traced_x(input, stores, hooks)?.ok_or(
            ExecError::MissingToken {
                context: "alignment cell",
            },
        )?);
    }
}

fn fin_row(
    align_level: usize,
    nest: &mut ModeNest,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    let kind = align_kind(nest, align_level)?;
    flush_pending_hchars(nest, stores)?;
    let row_level = nest.pop()?;
    let nodes = row_level.list().nodes().to_vec();
    let children = stores.freeze_node_list(&nodes);
    let row = super::packaging::make_unset_node(
        stores,
        children,
        super::packaging::row_unset_kind(kind),
        1,
    );
    if kind == AlignmentKind::HAlign
        && let Node::Unset(unset) = &row
    {
        nest.current_list_mut().set_prev_depth(unset.depth);
    }
    nest.current_list_mut().push(row);
    align_state_mut(nest, align_level)?.finish_row();
    Ok(())
}

struct CellResult {
    next_column: usize,
    ended_row: bool,
}

struct CellStart {
    column: usize,
    first_token: Option<TracedTokenWord>,
}

fn execute_cell<S, R, H>(
    align_level: usize,
    start: CellStart,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<CellResult, ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let kind = align_kind(nest, align_level)?;
    stores.enter_group_with_kind(tex_state::GroupKind::Simple);
    nest.push(cell_mode(kind));
    if kind == AlignmentKind::VAlign {
        nest.current_list_mut()
            .set_prev_depth(crate::mode::IGNORE_DEPTH);
    }
    let mut column = start.column;
    let mut span_count = 1u16;
    let mut first_token = start.first_token;
    loop {
        let initial = first_token.take();
        let omit = initial
            .map(tex_expand::semantic_token)
            .is_some_and(|token| is_omit(stores, token));
        align_state_mut(nest, align_level)?.start_cell(column, span_count);
        let column_templates = align_state(nest, align_level)?
            .column_for(column)
            .copied()
            .ok_or(ExecError::MissingToken {
                context: "alignment template",
            })?;
        if !omit {
            if let Some(token) = initial {
                push_traced_tokens(input, stores, [token]);
            }
            if span_count > 1 {
                super::template::expand_spanned_column_template_at_span_time(
                    column_templates.u_template,
                    align_level,
                    nest,
                    input,
                    stores,
                    recorder,
                    hooks,
                )?;
            } else {
                super::template::replay_template(
                    column_templates.u_template,
                    None,
                    nest,
                    input,
                    stores,
                    recorder,
                    hooks,
                )?;
            }
        }
        align_state_mut(nest, align_level)?.start_cell(column, span_count);

        let terminator =
            run_cell_body_until_terminator(align_level, nest, input, stores, recorder, hooks)?;
        if !omit {
            let end_template = align_state(nest, align_level)?.end_template();
            super::template::replay_template(
                column_templates.v_template,
                Some(end_template),
                nest,
                input,
                stores,
                recorder,
                hooks,
            )?;
        }
        match terminator {
            CellTerminator::Span => {
                flush_pending_hchars(nest, stores)?;
                column = column.checked_add(1).ok_or(ExecError::ArithmeticOverflow)?;
                span_count = span_count
                    .checked_add(1)
                    .ok_or(ExecError::ArithmeticOverflow)?;
                first_token = next_non_space_traced_x(input, stores, hooks)?;
            }
            CellTerminator::AlignmentTab | CellTerminator::Cr => {
                let next_column = column + 1;
                package_cell(align_level, kind, span_count, next_column, nest, stores)?;
                leave_group(input, stores, tex_state::GroupKind::Simple)?;
                align_state_mut(nest, align_level)?.finish_cell(next_column);
                return Ok(CellResult {
                    next_column,
                    ended_row: matches!(terminator, CellTerminator::Cr),
                });
            }
        }
    }
}

fn package_cell(
    align_level: usize,
    kind: AlignmentKind,
    span_count: u16,
    next_boundary: usize,
    nest: &mut ModeNest,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    if kind == AlignmentKind::VAlign && nest.current_mode() == Mode::Horizontal {
        crate::assignments::end_paragraph(nest, stores)?;
    }
    flush_pending_hchars(nest, stores)?;
    let cell_level = nest.pop()?;
    let nodes = if kind == AlignmentKind::HAlign {
        crate::math::finish_math_lists(stores, cell_level.list().nodes(), false)
    } else {
        cell_level.list().nodes().to_vec()
    };
    let children = stores.freeze_node_list(&nodes);
    let cell = super::packaging::make_unset_node(
        stores,
        children,
        super::packaging::cell_unset_kind(kind),
        span_count,
    );
    nest.current_list_mut().push(cell);
    let tabskip = align_state(nest, align_level)?.tabskip_for_boundary(next_boundary);
    nest.current_list_mut().push(Node::Glue {
        spec: tabskip,
        kind: GlueKind::TabSkip,
        leader: None,
    });
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CellTerminator {
    AlignmentTab,
    Cr,
    Span,
}

fn run_cell_body_until_terminator<S, R, H>(
    align_level: usize,
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<CellTerminator, ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let mut stats = ExecutionStats::default();
    loop {
        sync_engine_state::<S, _>(hooks, nest, stores);
        let token = {
            let mut expansion = ExpansionContext::new(stores);
            get_x_token_with_recorder_and_hooks(input, &mut expansion, recorder, hooks)?
        }
        .ok_or(ExecError::MissingToken {
            context: "alignment cell",
        })?;
        let semantic = tex_expand::semantic_token(token);
        stats.delivered_tokens += 1;
        if align_state(nest, align_level)?.brace_depth() == 0 {
            if is_alignment_tab(stores, semantic) {
                return Ok(CellTerminator::AlignmentTab);
            }
            if is_cr(stores, semantic) {
                return Ok(CellTerminator::Cr);
            }
            if is_span(stores, semantic) {
                return Ok(CellTerminator::Span);
            }
            if is_noalign(stores, semantic) {
                return Err(ExecError::MisplacedNoAlign);
            }
            if is_omit(stores, semantic) {
                return Err(ExecError::MisplacedOmit);
            }
            if is_end_group(stores, semantic) {
                report_missing_cr_inserted(stores);
                push_traced_tokens(input, stores, [token]);
                return Ok(CellTerminator::Cr);
            }
        }
        update_persistent_cell_brace_depth(align_level, nest, stores, semantic)?;
        dispatch_and_drain(nest, token, input, stores, recorder, hooks, &mut stats)?;
    }
}

pub(super) fn run_one_main_control_token<S, R, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
    stats: &mut ExecutionStats,
) -> Result<(), ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    sync_engine_state::<S, _>(hooks, nest, stores);
    let token = {
        let mut expansion = ExpansionContext::new(stores);
        get_x_token_with_recorder_and_hooks(input, &mut expansion, recorder, hooks)?
    }
    .ok_or(ExecError::MissingToken {
        context: "alignment template",
    })?;
    stats.delivered_tokens += 1;
    dispatch_and_drain(nest, token, input, stores, recorder, hooks, stats)
}

pub(super) fn dispatch_and_drain<S, R, H>(
    nest: &mut ModeNest,
    token: tex_state::token::TracedTokenWord,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
    stats: &mut ExecutionStats,
) -> Result<(), ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    match dispatch_delivered_token_with_recorder(nest, token, input, stores, recorder, hooks)? {
        DispatchAction::Continue => {
            crate::output::drain_pending_output(nest, input, stores, recorder, hooks, stats)?;
            Ok(())
        }
        DispatchAction::Shipout(artifact) => {
            stats.shipped_artifacts.push(artifact);
            crate::output::drain_pending_output(nest, input, stores, recorder, hooks, stats)?;
            Ok(())
        }
        DispatchAction::End => Ok(()),
        DispatchAction::NotConsumed => Err(ExecError::UnimplementedTypesetting {
            mode: nest.current_mode(),
            token: tex_expand::semantic_token(token),
            origin: token.origin(),
            operation: "alignment cell",
        }),
    }
}

fn update_persistent_cell_brace_depth(
    align_level: usize,
    nest: &mut ModeNest,
    stores: &Universe,
    token: Token,
) -> Result<(), ExecError> {
    // TeX82 updates align_state in get_next for both braces of a math group.
    // Our math dispatcher scans that whole group synchronously after receiving
    // its opening brace, so neither boundary persists across cell-loop pulls.
    if matches!(nest.current_mode(), Mode::Math | Mode::DisplayMath)
        && matches!(
            token,
            Token::Char {
                cat: tex_state::token::Catcode::BeginGroup,
                ..
            }
        )
    {
        return Ok(());
    }
    if is_begin_group(stores, token) {
        align_state_mut(nest, align_level)?.increment_brace_depth();
    } else if is_end_group(stores, token) {
        align_state_mut(nest, align_level)?.decrement_brace_depth();
    }
    Ok(())
}

fn report_missing_cr_inserted(stores: &mut Universe) {
    stores
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, "\n! Missing \\cr inserted.\n");
}
