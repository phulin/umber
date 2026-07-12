use tex_expand::{ExpansionHooks, ReadRecorder, get_x_token_with_recorder_and_hooks};
use tex_lex::{AlignmentTerminator, InputSource, InputStack, TokenListReplayKind};
use tex_state::env::banks::TokParam;
use tex_state::node::{GlueKind, Node};
use tex_state::token::{Token, TracedTokenWord};
use tex_state::{ExpansionContext, ExpansionState, PrintSink, Universe};

use super::support::{
    align_kind, align_state, align_state_mut, alignment_mode, cell_mode, is_alignment_tab, is_cr,
    is_crcr, is_end_group, is_noalign, is_omit, is_span, row_mode, set_align_brace_depth,
};
use crate::assignments::{flush_pending_hchars, next_non_space_traced_x};
use crate::dispatch::dispatch_delivered_token_with_recorder;
use crate::executor::sync_engine_state;
use crate::mode::{AlignState, AlignmentKind};
use crate::vertical::{
    append_node_to_vertical_list, append_vertical_contribution, build_page_if_outer_vertical,
};
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
    {
        let alignment_kind = state.kind();
        let enclosing_prev_depth = nest.current_list().prev_depth();
        nest.push(alignment_mode(alignment_kind));
        if let Some(prev_depth) = enclosing_prev_depth {
            // TeX.web push_nest preserves aux, so an ordinary vertical-mode
            // alignment starts with the enclosing list's prev_depth too.
            nest.current_list_mut().set_prev_depth(prev_depth);
        }
        let align_level = nest.depth() - 1;
        nest.current_list_mut().set_align_state(state);
        // TeX82 keeps an entry align_group above the whole-alignment group.
        // fin_col replaces this level after every completed entry.
        stores.enter_group_with_kind(tex_state::GroupKind::Simple);
        replay_everycr(input, stores);

        while let Some(first_token) = align_peek(align_level, nest, input, stores, recorder, hooks)?
        {
            init_row(align_level, nest)?;
            let suppress_redundant_cr = execute_row(
                align_level,
                first_token,
                nest,
                input,
                stores,
                recorder,
                hooks,
            )?;
            align_state_mut(nest, align_level)?.set_suppress_redundant_cr(suppress_redundant_cr);
            fin_row(align_level, nest, stores)?;
            replay_everycr(input, stores);
        }

        let finished = finish_alignment_level(nest, stores)?;
        append_finished_alignment(nest, stores, finished);
        build_page_if_outer_vertical(nest, stores)?;
        Ok(())
    }
}

pub(crate) struct FinishedAlignment {
    pub(crate) nodes: Vec<Node>,
    pub(crate) aux_prev_depth: Option<tex_state::scaled::Scaled>,
}

pub(crate) fn append_finished_alignment(
    nest: &mut ModeNest,
    stores: &mut Universe,
    finished: FinishedAlignment,
) {
    if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical)
        && let Some(prev_depth) = finished.aux_prev_depth
    {
        // TeX.web fin_align restores the alignment level's aux wholesale
        // before splicing nodes whose dimensions may have been transformed.
        nest.current_list_mut().set_prev_depth(prev_depth);
    }
    for node in finished.nodes {
        if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
            append_vertical_contribution(nest, stores, node);
        } else {
            nest.current_list_mut().push(node);
        }
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
    {
        let alignment_kind = state.kind();
        let enclosing_prev_depth = nest.current_list().prev_depth();
        nest.push(alignment_mode(alignment_kind));
        if let Some(prev_depth) = enclosing_prev_depth {
            // TeX.web init_align reaches through display math to recover the
            // enclosing vlist's prev_depth after push_nest preserves aux.
            nest.current_list_mut().set_prev_depth(prev_depth);
        }
        let align_level = nest.depth() - 1;
        nest.current_list_mut().set_align_state(state);
        // Match init_align's entry align_group for the display path too.
        stores.enter_group_with_kind(tex_state::GroupKind::Simple);
        replay_everycr(input, stores);

        while let Some(first_token) = align_peek(align_level, nest, input, stores, recorder, hooks)?
        {
            init_row(align_level, nest)?;
            let suppress_redundant_cr = execute_row(
                align_level,
                first_token,
                nest,
                input,
                stores,
                recorder,
                hooks,
            )?;
            align_state_mut(nest, align_level)?.set_suppress_redundant_cr(suppress_redundant_cr);
            fin_row(align_level, nest, stores)?;
            replay_everycr(input, stores);
        }

        Ok(finish_alignment_level(nest, stores)?.nodes)
    }
}

fn finish_alignment_level(
    nest: &mut ModeNest,
    stores: &mut Universe,
) -> Result<FinishedAlignment, ExecError> {
    let mut level = nest.pop()?;
    let aux_prev_depth = level.list().prev_depth();
    let state = level
        .list_mut()
        .take_align_state()
        .ok_or(ExecError::MissingToken {
            context: "alignment state",
        })?;
    let nodes = level.list().nodes().to_vec();
    let finished = super::widths::finish_alignment(&state, &nodes, stores)?;
    Ok(FinishedAlignment {
        nodes: finished,
        aux_prev_depth,
    })
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
            stores.world_mut().write_text(
                PrintSink::TerminalAndLog,
                "\n! Missing } inserted while finishing alignment.\n",
            );
            leave_group(input, stores, tex_state::GroupKind::Simple)?;
            leave_group(input, stores, tex_state::GroupKind::Simple)?;
            return Ok(None);
        };
        set_align_brace_depth(nest, align_level, 0);
        let semantic = tex_expand::semantic_token(token);
        if is_noalign(stores, semantic) {
            super::noalign::execute_noalign(align_level, nest, input, stores, recorder, hooks)?;
            continue;
        }
        if is_end_group(stores, semantic) {
            // fin_align unsaves the fresh entry level, then the level created
            // by scan_spec for the whole alignment.
            leave_group(input, stores, tex_state::GroupKind::Simple)?;
            leave_group(input, stores, tex_state::GroupKind::Simple)?;
            return Ok(None);
        }
        // WEB changes an extra alignment tab to a row-ending \cr. A source
        // \cr immediately following that recovery is the redundant terminator
        // of the same malformed row, not the start of another empty row.
        if align_state(nest, align_level)?.suppress_redundant_cr() && is_cr(stores, semantic) {
            align_state_mut(nest, align_level)?.set_suppress_redundant_cr(false);
            continue;
        }
        // align_peek ignores \crcr between rows, but a bare \cr starts and
        // immediately terminates an empty row through the normal template
        // interception path.
        if is_crcr(stores, semantic) {
            continue;
        }
        align_state_mut(nest, align_level)?.set_suppress_redundant_cr(false);
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
) -> Result<bool, ExecError>
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
            return Ok(result.extra_alignment_tab);
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
    if kind == AlignmentKind::HAlign {
        append_node_to_vertical_list(nest, stores, row)?;
    } else {
        nest.current_list_mut().push(row);
    }
    align_state_mut(nest, align_level)?.finish_row();
    Ok(())
}

struct CellResult {
    next_column: usize,
    ended_row: bool,
    extra_alignment_tab: bool,
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
        let v_template = if omit {
            stores.intern_token_list(&[stores.frozen_end_template_token()])
        } else {
            column_templates.v_template
        };
        if !omit {
            if let Some(token) = initial {
                push_traced_tokens(input, stores, [token]);
            }
            if span_count > 1 {
                super::template::expand_spanned_column_template_at_span_time(
                    column_templates.u_template,
                    v_template,
                    nest,
                    input,
                    stores,
                    recorder,
                    hooks,
                )?;
            } else {
                super::template::replay_template(
                    column_templates.u_template,
                    v_template,
                    nest,
                    input,
                    stores,
                    recorder,
                    hooks,
                )?;
            }
        } else {
            input.begin_alignment_cell(None, v_template, stores.execution_group_depth());
        }
        align_state_mut(nest, align_level)?.start_cell(column, span_count);

        let terminator =
            run_cell_body_until_terminator(align_level, nest, input, stores, recorder, hooks)?;
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
                let extra_alignment_tab = matches!(terminator, CellTerminator::AlignmentTab)
                    && align_state(nest, align_level)?
                        .column_for(next_column)
                        .is_none();
                if extra_alignment_tab {
                    stores.world_mut().write_text(
                        PrintSink::TerminalAndLog,
                        "\n! Extra alignment tab has been changed to \\cr.\n",
                    );
                }
                package_cell(align_level, kind, span_count, next_column, nest, stores)?;
                leave_group(input, stores, tex_state::GroupKind::Simple)?;
                // WEB fin_col immediately installs the next entry align_group,
                // including after a row-ending \cr for fin_align to remove.
                stores.enter_group_with_kind(tex_state::GroupKind::Simple);
                align_state_mut(nest, align_level)?.finish_cell(next_column);
                return Ok(CellResult {
                    next_column,
                    ended_row: matches!(terminator, CellTerminator::Cr) || extra_alignment_tab,
                    extra_alignment_tab,
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
    _align_level: usize,
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
        let fetched = {
            let mut expansion = ExpansionContext::new(stores);
            get_x_token_with_recorder_and_hooks(input, &mut expansion, recorder, hooks)
        };
        let token = match fetched {
            Ok(Some(token)) => token,
            Ok(None) => {
                if let Some(terminator) = input.finish_terminating_alignment_cell() {
                    return classify_cell_terminator(stores, terminator);
                }
                return Err(ExecError::MissingToken {
                    context: "alignment cell",
                });
            }
            Err(tex_expand::ExpandError::UndefinedControlSequence { name, .. }) => {
                stores.world_mut().write_text(
                    PrintSink::TerminalAndLog,
                    &format!("\n! Undefined control sequence \\{name}.\n"),
                );
                continue;
            }
            Err(tex_expand::ExpandError::ForbiddenOuterTokenInAlignment { context }) => {
                recover_outer_alignment_token(context, input, stores);
                continue;
            }
            Err(tex_expand::ExpandError::Captured { error, .. })
                if matches!(
                    error.as_ref(),
                    tex_expand::ExpandError::ForbiddenOuterTokenInAlignment { .. }
                ) =>
            {
                let tex_expand::ExpandError::ForbiddenOuterTokenInAlignment { context } = *error
                else {
                    unreachable!("guard restricts captured expansion error")
                };
                recover_outer_alignment_token(context, input, stores);
                continue;
            }
            Err(tex_expand::ExpandError::Captured { error, .. })
                if matches!(
                    error.as_ref(),
                    tex_expand::ExpandError::UndefinedControlSequence { .. }
                ) =>
            {
                let tex_expand::ExpandError::UndefinedControlSequence { name, .. } = *error else {
                    unreachable!("guard restricts captured expansion error")
                };
                stores.world_mut().write_text(
                    PrintSink::TerminalAndLog,
                    &format!("\n! Undefined control sequence \\{name}.\n"),
                );
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        let semantic = tex_expand::semantic_token(token);
        if semantic.is_frozen_endv() {
            let terminator = input
                .finish_alignment_cell()
                .ok_or(ExecError::MissingToken {
                    context: "alignment cell terminator",
                })?;
            return classify_cell_terminator(stores, terminator);
        }
        stats.delivered_tokens += 1;
        if is_noalign(stores, semantic) {
            return Err(ExecError::MisplacedNoAlign);
        }
        if is_omit(stores, semantic) {
            return Err(ExecError::MisplacedOmit);
        }
        if is_alignment_par(stores, semantic) {
            if input.alignment_cell_at_base_depth() {
                recover_outer_alignment_token(token, input, stores);
                continue;
            }
            if input.alignment_cell_below_base_depth() {
                report_missing_cr_inserted(stores);
                let cr = stores.symbol("cr").ok_or(ExecError::MissingToken {
                    context: "alignment recovery cr",
                })?;
                let cr = TracedTokenWord::pack(Token::Cs(cr.symbol()), token.origin());
                push_traced_tokens(input, stores, [token]);
                input.reset_alignment_cell_to_base_depth();
                assert!(input.intercept_alignment_token(
                    cr,
                    tex_lex::AlignmentTokenDelivery::Other,
                    Some(AlignmentTerminator::Cr),
                    stores.execution_group_depth(),
                ));
                continue;
            }
        }
        if is_end_group(stores, semantic) && input.alignment_cell_below_base_depth() {
            report_missing_cr_inserted(stores);
            let cr = stores.symbol("cr").ok_or(ExecError::MissingToken {
                context: "alignment recovery cr",
            })?;
            let cr = TracedTokenWord::pack(Token::Cs(cr.symbol()), token.origin());
            push_traced_tokens(input, stores, [token]);
            input.reset_alignment_cell_to_base_depth();
            assert!(input.intercept_alignment_token(
                cr,
                tex_lex::AlignmentTokenDelivery::Other,
                Some(AlignmentTerminator::Cr),
                stores.execution_group_depth(),
            ));
            continue;
        }
        dispatch_and_drain(nest, token, input, stores, recorder, hooks, &mut stats)?;
    }
}

fn classify_cell_terminator(
    stores: &mut Universe,
    terminator: TracedTokenWord,
) -> Result<CellTerminator, ExecError> {
    let semantic = tex_expand::semantic_token(terminator);
    if is_alignment_tab(stores, semantic) {
        return Ok(CellTerminator::AlignmentTab);
    }
    if is_cr(stores, semantic) {
        return Ok(CellTerminator::Cr);
    }
    if is_span(stores, semantic) {
        return Ok(CellTerminator::Span);
    }
    Err(ExecError::MissingToken {
        context: "alignment cell terminator",
    })
}

pub(super) enum TemplateStep {
    Continue,
    EndV,
    DeferredOuterRecovery,
}

pub(super) fn run_one_main_control_token<S, R, H>(
    nest: &mut ModeNest,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
    stats: &mut ExecutionStats,
) -> Result<TemplateStep, ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    sync_engine_state::<S, _>(hooks, nest, stores);
    let fetched = {
        let mut expansion = ExpansionContext::new(stores);
        get_x_token_with_recorder_and_hooks(input, &mut expansion, recorder, hooks)
    };
    let token = match fetched {
        Ok(Some(token)) => token,
        Ok(None) => {
            return Err(ExecError::MissingToken {
                context: "alignment template",
            });
        }
        Err(tex_expand::ExpandError::Captured { error, .. })
            if matches!(
                error.as_ref(),
                tex_expand::ExpandError::ForbiddenOuterTokenInAlignment { .. }
            ) =>
        {
            let tex_expand::ExpandError::ForbiddenOuterTokenInAlignment { context } = *error else {
                unreachable!("guard restricts captured expansion error")
            };
            recover_outer_alignment_token(context, input, stores);
            return Ok(TemplateStep::DeferredOuterRecovery);
        }
        Err(error) => return Err(error.into()),
    };
    stats.delivered_tokens += 1;
    if tex_expand::semantic_token(token).is_frozen_endv() {
        return Ok(TemplateStep::EndV);
    }
    dispatch_and_drain(nest, token, input, stores, recorder, hooks, stats)?;
    Ok(TemplateStep::Continue)
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
    let action =
        match dispatch_delivered_token_with_recorder(nest, token, input, stores, recorder, hooks) {
            Ok(action) => action,
            Err(ExecError::Expand(tex_expand::ExpandError::UndefinedControlSequence {
                name,
                ..
            })) => {
                stores.world_mut().write_text(
                    PrintSink::TerminalAndLog,
                    &format!("\n! Undefined control sequence \\{name}.\n"),
                );
                return Ok(());
            }
            Err(ExecError::Expand(tex_expand::ExpandError::Captured { error, .. }))
                if matches!(
                    error.as_ref(),
                    tex_expand::ExpandError::UndefinedControlSequence { .. }
                ) =>
            {
                let tex_expand::ExpandError::UndefinedControlSequence { name, .. } = *error else {
                    unreachable!("guard restricts captured expansion error")
                };
                stores.world_mut().write_text(
                    PrintSink::TerminalAndLog,
                    &format!("\n! Undefined control sequence \\{name}.\n"),
                );
                return Ok(());
            }
            Err(error) => return Err(error),
        };
    match action {
        DispatchAction::Continue => {
            crate::output::drain_pending_output(nest, input, stores, recorder, hooks, stats)?;
            Ok(())
        }
        DispatchAction::Shipout(artifact) => {
            let _ = artifact;
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

fn report_missing_cr_inserted(stores: &mut Universe) {
    stores
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, "\n! Missing \\cr inserted.\n");
}

fn is_alignment_par(stores: &Universe, token: Token) -> bool {
    let Token::Cs(symbol) = token else {
        return false;
    };
    matches!(
        stores.meaning(symbol),
        tex_state::meaning::Meaning::UnexpandablePrimitive(
            tex_state::meaning::UnexpandablePrimitive::Par
                | tex_state::meaning::UnexpandablePrimitive::EndGraf
        )
    )
}

fn recover_outer_alignment_token<S>(
    context: TracedTokenWord,
    input: &mut InputStack<S>,
    stores: &mut Universe,
) where
    S: InputSource,
{
    stores.world_mut().write_text(
        PrintSink::TerminalAndLog,
        "\n! Missing } inserted.\nI've inserted something that you may have forgotten.\n",
    );
    let closing = Token::Char {
        ch: '}',
        cat: tex_state::token::Catcode::EndGroup,
    };
    let origin = stores.inserted_origin(
        tex_state::provenance::InsertedOriginKind::ErrorRecovery,
        closing,
        context.origin(),
    );
    push_traced_tokens(
        input,
        stores,
        [TracedTokenWord::pack(closing, origin), context],
    );
}
