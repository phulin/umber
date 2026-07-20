use tex_expand::{get_alignment_x_or_protected_with_context, get_x_token_with_context};
use tex_lex::{InputStack, TokenListReplayKind};
use tex_state::env::banks::TokParam;
use tex_state::node::{GlueKind, Node};
use tex_state::token::{Token, TracedTokenWord};
use tex_state::{ExpansionContext, ExpansionState, InteractionMode, PrintSink, Universe};

use super::support::{
    align_kind, align_state, align_state_mut, alignment_mode, cell_mode, is_alignment_tab, is_cr,
    is_crcr, is_end_group, is_noalign, is_omit, is_span, row_mode, set_align_brace_depth,
};
use crate::assignments::flush_pending_hchars;
use crate::dispatch::{dispatch_delivered_token_with_context, insert_traced_tokens};
use crate::executor::sync_engine_state;
use crate::mode::{AlignState, AlignmentKind};
use crate::vertical::{
    append_node_to_vertical_list, append_vertical_contribution, build_page_if_outer_vertical,
};
use crate::{
    DispatchAction, ExecError, ExecutionStats, Mode, ModeNest, leave_group, push_traced_tokens,
};

pub(crate) fn execute_alignment(
    state: AlignState,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
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
        stores.enter_group_with_kind(tex_state::GroupKind::Align);
        replay_everycr(input, stores);

        while let Some(first_token) = align_peek(align_level, nest, input, stores, execution)? {
            init_row(align_level, nest)?;
            let suppress_redundant_cr =
                execute_row(align_level, first_token, nest, input, stores, execution)?;
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

pub(super) fn execute_alignment_to_nodes(
    state: AlignState,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<FinishedAlignment, ExecError> {
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
        stores.enter_group_with_kind(tex_state::GroupKind::Align);
        replay_everycr(input, stores);

        while let Some(first_token) = align_peek(align_level, nest, input, stores, execution)? {
            init_row(align_level, nest)?;
            let suppress_redundant_cr =
                execute_row(align_level, first_token, nest, input, stores, execution)?;
            align_state_mut(nest, align_level)?.set_suppress_redundant_cr(suppress_redundant_cr);
            fin_row(align_level, nest, stores)?;
            replay_everycr(input, stores);
        }

        finish_alignment_level(nest, stores)
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

fn replay_everycr(input: &mut InputStack, stores: &Universe) {
    let everycr = stores.tok_param(TokParam::EVERY_CR);
    if !stores.tokens(everycr).is_empty() {
        input.push_token_list(everycr, TokenListReplayKind::EveryCr);
    }
}

fn align_peek(
    align_level: usize,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<Option<TracedTokenWord>, ExecError> {
    loop {
        set_align_brace_depth(nest, align_level, 1_000_000);
        input.set_alignment_state(1_000_000);
        let Some(token) = next_non_space_protected(input, stores, execution)? else {
            stores.world_mut().write_text(
                PrintSink::TerminalAndLog,
                "\n! Missing } inserted while finishing alignment.\n",
            );
            leave_group(input, stores, tex_state::GroupKind::Align)?;
            leave_group(input, stores, tex_state::GroupKind::Align)?;
            return Ok(None);
        };
        let semantic = tex_expand::semantic_token(token);
        if is_noalign(stores, semantic) {
            super::noalign::execute_noalign(align_level, nest, input, stores, execution)?;
            continue;
        }
        if is_end_group(stores, semantic) {
            // fin_align unsaves the fresh entry level, then the level created
            // by scan_spec for the whole alignment.
            leave_group(input, stores, tex_state::GroupKind::Align)?;
            leave_group(input, stores, tex_state::GroupKind::Align)?;
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

fn execute_row(
    align_level: usize,
    first_token: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<bool, ExecError> {
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
            execution,
        )?;
        column = result.next_column;
        if result.ended_row {
            return Ok(result.extra_alignment_tab);
        }
        // TeX82 fin_col restores the sentinel before fetching the first token
        // of every following column, not only after a spanning column.
        input.set_alignment_state(1_000_000);
        start_token = Some(next_non_space_protected(input, stores, execution)?.ok_or(
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

fn execute_cell(
    align_level: usize,
    start: CellStart,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<CellResult, ExecError> {
    let kind = align_kind(nest, align_level)?;
    nest.push(cell_mode(kind));
    if kind == AlignmentKind::VAlign {
        nest.current_list_mut()
            .set_prev_depth(crate::mode::ignored_depth(stores));
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
                    execution,
                )?;
            } else {
                super::template::replay_template(
                    column_templates.u_template,
                    v_template,
                    nest,
                    input,
                    stores,
                    execution,
                )?;
            }
        } else {
            input.begin_alignment_cell(None, v_template, stores.execution_group_depth());
        }
        align_state_mut(nest, align_level)?.start_cell(column, span_count);

        let terminator =
            run_cell_body_until_terminator(align_level, nest, input, stores, execution)?;
        match terminator {
            CellTerminator::Span => {
                flush_pending_hchars(nest, stores)?;
                let next_column = column.checked_add(1).ok_or(ExecError::ArithmeticOverflow)?;
                if align_state(nest, align_level)?
                    .column_for(next_column)
                    .is_none()
                {
                    stores.world_mut().write_text(
                        PrintSink::TerminalAndLog,
                        "\n! Extra alignment tab has been changed to \\cr.\n",
                    );
                    package_cell(align_level, kind, span_count, next_column, nest, stores)?;
                    leave_group(input, stores, tex_state::GroupKind::Align)?;
                    stores.enter_group_with_kind(tex_state::GroupKind::Align);
                    align_state_mut(nest, align_level)?.finish_cell(next_column);
                    return Ok(CellResult {
                        next_column,
                        ended_row: true,
                        extra_alignment_tab: true,
                    });
                }
                column = next_column;
                span_count = span_count
                    .checked_add(1)
                    .ok_or(ExecError::ArithmeticOverflow)?;
                // TeX82 fin_col restores the sentinel before looking for the
                // first token of the next spanned column.
                input.set_alignment_state(1_000_000);
                first_token = next_non_space_protected(input, stores, execution)?;
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
                leave_group(input, stores, tex_state::GroupKind::Align)?;
                // WEB fin_col immediately installs the next entry align_group,
                // including after a row-ending \cr for fin_align to remove.
                stores.enter_group_with_kind(tex_state::GroupKind::Align);
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

fn next_non_space_protected(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<Option<TracedTokenWord>, ExecError> {
    loop {
        let token = {
            let mut expansion = ExpansionContext::new(stores);
            get_alignment_x_or_protected_with_context(input, &mut expansion, execution)?
        };
        match token {
            Some(token)
                if matches!(
                    tex_expand::semantic_token(token),
                    Token::Char {
                        cat: tex_state::token::Catcode::Space,
                        ..
                    }
                ) => {}
            token => return Ok(token),
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

fn run_cell_body_until_terminator(
    _align_level: usize,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<CellTerminator, ExecError> {
    let mut stats = ExecutionStats::default();
    loop {
        sync_engine_state(execution, nest, stores);
        let fetched = {
            let mut expansion = ExpansionContext::new(stores);
            get_x_token_with_context(input, &mut expansion, execution)
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
        if semantic.is_frozen_endv()
            && matches!(nest.current_mode(), Mode::Math | Mode::DisplayMath)
        {
            // TeX82 reaches `endv` through main_control. In math mode it must
            // first apply `off_save`, which inserts the delimiter needed to
            // close an intervening math or ordinary group and then retries
            // the inaccessible token in the alignment cell's base mode.
            crate::assignments::off_save_alignment(token, input, stores)?;
            continue;
        }
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
            stores.world_mut().write_text(
                PrintSink::TerminalAndLog,
                "\n! Misplaced \\noalign.\nI expect to see \\noalign only after the \\cr of an alignment.\n",
            );
            continue;
        }
        if is_omit(stores, semantic) {
            if stores.interaction_mode() == InteractionMode::ErrorStop {
                return Err(ExecError::MisplacedOmit);
            }
            stores.world_mut().write_text(
                PrintSink::TerminalAndLog,
                "\n! Misplaced \\omit.\nI expect to see \\omit only after the \\cr of an alignment.\n",
            );
            continue;
        }
        if is_alignment_par(stores, semantic) && input.alignment_cell_below_base_depth() {
            // TeX.web §1091 hmode+par_end calls off_save when the
            // alignment brace level is negative. Backing up \par behind
            // the inserted right brace lets ordinary group dispatch
            // reach §1103's align_group recovery in the same order.
            recover_alignment_par_token(token, input, stores);
            continue;
        }
        if is_end_group(stores, semantic)
            && input.alignment_cell_below_base_depth()
            && stores.innermost_group_kind() == Some(tex_state::GroupKind::Align)
        {
            // TeX.web §1103 does not unsave the align_group. It backs up the
            // brace and inserts frozen \cr, which may itself need §1102's
            // missing-left-brace recovery before get_next can start v_j.
            report_missing_cr_inserted(stores);
            let cr = stores.symbol("cr").ok_or(ExecError::MissingToken {
                context: "alignment recovery cr",
            })?;
            let cr = TracedTokenWord::pack(Token::Cs(cr.symbol()), token.origin());
            input.back_input_alignment_token(token);
            insert_traced_tokens(input, stores, [cr, token]);
            continue;
        }
        if input.alignment_cell_below_base_depth()
            && (is_alignment_tab(stores, semantic)
                || is_span(stores, semantic)
                || is_cr(stores, semantic))
        {
            // TeX.web §1102 align_error: back up the delimiter and put a
            // left brace before it. Reading that inserted brace brings the
            // scanner level back to zero; the replayed delimiter then starts
            // v_j through the ordinary get_next interception path.
            stores
                .world_mut()
                .write_text(PrintSink::TerminalAndLog, "\n! Missing { inserted.\n");
            let left = Token::Char {
                ch: '{',
                cat: tex_state::token::Catcode::BeginGroup,
            };
            let origin = stores.inserted_origin(
                tex_state::provenance::InsertedOriginKind::ErrorRecovery,
                left,
                token.origin(),
            );
            input.back_input_alignment_token(token);
            insert_traced_tokens(input, stores, [TracedTokenWord::pack(left, origin), token]);
            continue;
        }
        dispatch_and_drain(nest, token, input, stores, execution, &mut stats)?;
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
}

pub(super) fn run_one_main_control_token(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    stats: &mut ExecutionStats,
) -> Result<TemplateStep, ExecError> {
    sync_engine_state(execution, nest, stores);
    let fetched = {
        let mut expansion = ExpansionContext::new(stores);
        get_x_token_with_context(input, &mut expansion, execution)
    };
    let token = match fetched {
        Ok(Some(token)) => token,
        Ok(None) => {
            return Err(ExecError::MissingToken {
                context: "alignment template",
            });
        }
        Err(error) => return Err(error.into()),
    };
    stats.delivered_tokens += 1;
    if tex_expand::semantic_token(token).is_frozen_endv() {
        return Ok(TemplateStep::EndV);
    }
    dispatch_and_drain(nest, token, input, stores, execution, stats)?;
    Ok(TemplateStep::Continue)
}

pub(super) fn dispatch_and_drain(
    nest: &mut ModeNest,
    token: tex_state::token::TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    stats: &mut ExecutionStats,
) -> Result<(), ExecError> {
    let action = match dispatch_delivered_token_with_context(nest, token, input, stores, execution)
    {
        Ok(action) => action,
        Err(ExecError::Expand(tex_expand::ExpandError::UndefinedControlSequence {
            name, ..
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
            crate::output::drain_pending_output(nest, input, stores, execution, stats)?;
            Ok(())
        }
        DispatchAction::Shipout(page) => {
            stats.prepared_dvi_pages.push(page);
            crate::output::drain_pending_output(nest, input, stores, execution, stats)?;
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

fn recover_alignment_par_token(
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
) {
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
    input.back_input_alignment_token(context);
    insert_traced_tokens(
        input,
        stores,
        [TracedTokenWord::pack(closing, origin), context],
    );
}
