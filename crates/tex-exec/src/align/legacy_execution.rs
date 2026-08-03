use tex_expand::{get_alignment_x_or_protected_with_context, get_x_token_with_context};
#[cfg(test)]
#[path = "execution/tests.rs"]
mod tests;

use tex_lex::InputStack;
use tex_state::TokenListReplayKind;
use tex_state::env::banks::TokParam;
use tex_state::node::{GlueKind, Node};
use tex_state::token::{Token, TracedTokenWord};
use tex_state::{ExpansionContext, ExpansionState, InteractionMode, PrintSink, Universe};

use super::support::{
    align_kind, align_state, alignment_mode, cell_mode, is_alignment_tab, is_cr, is_crcr,
    is_end_group, is_noalign, is_omit, is_span, mutate_align_state, row_mode,
};
use super::{FinishedAlignment, append_finished_alignment};
use crate::assignments::flush_pending_hchars;
use crate::dispatch::{dispatch_delivered_token_with_context, insert_traced_tokens};
use crate::error_report::{back_tokens, report_input_error};
use crate::executor::sync_engine_state;
use crate::mode::{AlignState, AlignmentKind};
use crate::vertical::{
    append_node_to_vertical_list, append_vertical_contribution, build_page_if_outer_vertical,
};
use crate::{
    DispatchAction, ExecError, ExecutionStats, Mode, ModeNest, leave_group, push_traced_tokens,
};

/// TeX82 §370's `<Complain about an undefined macro>` help.
const UNDEFINED_CONTROL_SEQUENCE_HELP: &[&str] = &[
    "The control sequence at the end of the top line",
    "of your error message was never \\def'ed. If you have",
    "misspelled it (e.g., `\\hobx'), type `I' and the correct",
    "spelling (e.g., `I\\hbox'). Otherwise just continue,",
    "and I'll forget about whatever was undefined.",
];

/// TeX82 §792's `<If the preamble list has been traversed, check that the row
/// has ended>`, for a row that ran past the last preamble column.
fn report_extra_alignment_tab(input: &InputStack, stores: &mut Universe) -> Result<(), ExecError> {
    report_input_error(
        input,
        stores,
        "Extra alignment tab has been changed to \\cr",
        &[
            "You have given more \\span or & marks than there were",
            "in the preamble to the \\halign or \\valign now in progress.",
            "So I'll assume that you meant to type \\cr instead.",
        ],
    )?;
    Ok(())
}

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
        nest.push(alignment_mode(alignment_kind))?;
        if let Some(prev_depth) = enclosing_prev_depth {
            // TeX.web push_nest preserves aux, so an ordinary vertical-mode
            // alignment starts with the enclosing list's prev_depth too.
            nest.current_list_mutation().set_prev_depth(prev_depth);
        }
        let align_level = nest.depth() - 1;
        nest.current_list_mutation().set_align_state(state);
        // TeX82 keeps an entry align_group above the whole-alignment group.
        // fin_col replaces this level after every completed entry.
        stores.enter_group_with_kind(tex_state::GroupKind::Align);
        replay_everycr(input, stores);

        while let Some(first_token) = align_peek(align_level, nest, input, stores, execution)? {
            init_row(align_level, nest)?;
            // TeX82 §786's `cur_tail:=cur_head`: one holding list per row.
            let mut migrations = Vec::new();
            let suppress_redundant_cr = execute_row(
                align_level,
                first_token,
                &mut migrations,
                nest,
                input,
                stores,
                execution,
            )?;
            mutate_align_state(nest, align_level, |state| {
                state.set_suppress_redundant_cr(suppress_redundant_cr)
            })?;
            fin_row(align_level, migrations, nest, stores, execution)?;
            replay_everycr(input, stores);
        }

        let finished = finish_alignment_level(nest, stores, execution)?;
        append_finished_alignment(nest, stores, finished);
        build_page_if_outer_vertical(nest, stores)?;
        Ok(())
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
        nest.push(alignment_mode(alignment_kind))?;
        if let Some(prev_depth) = enclosing_prev_depth {
            // TeX.web init_align reaches through display math to recover the
            // enclosing vlist's prev_depth after push_nest preserves aux.
            nest.current_list_mutation().set_prev_depth(prev_depth);
        }
        let align_level = nest.depth() - 1;
        nest.current_list_mutation().set_align_state(state);
        // Match init_align's entry align_group for the display path too.
        stores.enter_group_with_kind(tex_state::GroupKind::Align);
        replay_everycr(input, stores);

        while let Some(first_token) = align_peek(align_level, nest, input, stores, execution)? {
            init_row(align_level, nest)?;
            // TeX82 §786's `cur_tail:=cur_head`: one holding list per row.
            let mut migrations = Vec::new();
            let suppress_redundant_cr = execute_row(
                align_level,
                first_token,
                &mut migrations,
                nest,
                input,
                stores,
                execution,
            )?;
            mutate_align_state(nest, align_level, |state| {
                state.set_suppress_redundant_cr(suppress_redundant_cr)
            })?;
            fin_row(align_level, migrations, nest, stores, execution)?;
            replay_everycr(input, stores);
        }

        finish_alignment_level(nest, stores, execution)
    }
}

fn finish_alignment_level(
    nest: &mut ModeNest,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<FinishedAlignment, ExecError> {
    let mut level =
        crate::assignments::commit_current_list(nest, stores, execution.command_fuel())?;
    let aux_prev_depth = level.list().prev_depth();
    let aux_space_factor = matches!(level.mode(), Mode::Horizontal | Mode::RestrictedHorizontal)
        .then(|| level.list().space_factor());
    let state = level
        .list_mutation()
        .take_align_state()
        .ok_or(ExecError::MissingToken {
            context: "alignment state",
        })?;
    let nodes = level.list_mutation().take_nodes();
    // TeX82 §800: `if nest[nest_ptr-1].mode_field=mmode then o:=display_indent
    // else o:=0`. The alignment level has just been popped, so the current mode
    // is the enclosing one §800 inspects.
    let offset = if nest.current_mode() == Mode::DisplayMath {
        stores.dimen_param(tex_state::env::banks::DimenParam::DISPLAY_INDENT)
    } else {
        tex_state::scaled::Scaled::from_raw(0)
    };
    let finished = super::widths::finish_alignment(&state, &nodes, offset, stores)?;
    Ok(FinishedAlignment {
        nodes: finished,
        aux_prev_depth,
        aux_space_factor,
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
        input.set_alignment_scanner_phase(tex_state::AlignmentScannerPhase::BetweenEntries);
        let Some(token) = next_non_space_protected(input, stores, execution)? else {
            stores.world_mut().write_text(
                PrintSink::TerminalAndLog,
                "\n! Missing } inserted while finishing alignment.\n",
            );
            leave_group(input, stores, tex_state::GroupKind::Align)?;
            leave_group(input, stores, tex_state::GroupKind::Align)?;
            return Ok(None);
        };
        let semantic = token.semantic_token();
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
            mutate_align_state(nest, align_level, |state| {
                state.set_suppress_redundant_cr(false)
            })?;
            continue;
        }
        // align_peek ignores \crcr between rows, but a bare \cr starts and
        // immediately terminates an empty row through the normal template
        // interception path.
        if is_crcr(stores, semantic) {
            continue;
        }
        mutate_align_state(nest, align_level, |state| {
            state.set_suppress_redundant_cr(false)
        })?;
        return Ok(Some(token));
    }
}

fn init_row(align_level: usize, nest: &mut ModeNest) -> Result<(), ExecError> {
    let kind = align_kind(nest, align_level)?;
    let first_tabskip = align_state(nest, align_level)?.tabskip_for_boundary(0);
    mutate_align_state(nest, align_level, AlignState::start_row)?;
    nest.push(row_mode(kind))?;
    if kind == AlignmentKind::HAlign {
        nest.current_list_mutation().set_space_factor(0);
    }
    nest.current_list_mutation().push(Node::Glue {
        spec: first_tabskip,
        kind: GlueKind::TabSkip,
        leader: None,
    });
    Ok(())
}

fn execute_row(
    align_level: usize,
    first_token: TracedTokenWord,
    migrations: &mut Vec<Node>,
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
            migrations,
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
        input.set_alignment_scanner_phase(tex_state::AlignmentScannerPhase::BetweenEntries);
        start_token = Some(next_non_space_protected(input, stores, execution)?.ok_or(
            ExecError::MissingToken {
                context: "alignment cell",
            },
        )?);
    }
}

fn fin_row(
    align_level: usize,
    migrations: Vec<Node>,
    nest: &mut ModeNest,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let kind = align_kind(nest, align_level)?;

    let mut row_level =
        crate::assignments::commit_current_list(nest, stores, execution.command_fuel())?;
    let nodes = row_level.list_mutation().take_nodes();
    let children = stores.freeze_node_list(&nodes);
    let row = super::packaging::make_unset_node(
        stores,
        children,
        super::packaging::row_unset_kind(kind),
        1,
        super::packaging::UnsetPackContext::Row,
    )?;
    if kind == AlignmentKind::HAlign {
        append_node_to_vertical_list(nest, stores, row)?;
        // §799 continues `if cur_head<>cur_tail then begin
        // link(tail):=link(cur_head); tail:=cur_tail end`: a plain splice
        // immediately after the row, with no interline glue of its own.
        for node in migrations {
            append_vertical_contribution(nest, stores, node);
        }
    } else {
        nest.current_list_mutation().push(row);
    }
    mutate_align_state(nest, align_level, AlignState::finish_row)?;
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
    migrations: &mut Vec<Node>,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<CellResult, ExecError> {
    let kind = align_kind(nest, align_level)?;
    nest.push(cell_mode(kind))?;
    super::init_span_aux(nest, stores);
    let mut column = start.column;
    let mut span_count = 1u16;
    let mut first_token = start.first_token;
    loop {
        let initial = first_token.take();
        let omit = initial
            .map(|token| token.semantic_token())
            .is_some_and(|token| is_omit(stores, token));
        mutate_align_state(nest, align_level, |state| {
            state.start_cell(column, span_count)
        })?;
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
        let escaped_endv = if !omit {
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
                )?
            } else {
                super::template::replay_template(
                    column_templates.u_template,
                    v_template,
                    nest,
                    input,
                    stores,
                    execution,
                )?
            }
        } else {
            input.begin_alignment_cell(None, v_template);
            None
        };
        mutate_align_state(nest, align_level, |state| {
            state.start_cell(column, span_count)
        })?;

        let terminator = if let Some(command) = escaped_endv {
            match do_template_driver_endv(command, input, stores)? {
                DoEndV::FinishCell => {
                    let terminator =
                        input
                            .finish_alignment_cell(stores)
                            .ok_or(ExecError::MissingToken {
                                context: "alignment cell terminator",
                            })?;
                    classify_cell_terminator(stores, terminator)?
                }
                DoEndV::Recovered => {
                    run_cell_body_until_terminator(align_level, nest, input, stores, execution)?
                }
                DoEndV::NotApplicable => {
                    return Err(ExecError::MissingToken {
                        context: "exhausted alignment v-template",
                    });
                }
            }
        } else {
            run_cell_body_until_terminator(align_level, nest, input, stores, execution)?
        };
        match terminator {
            CellTerminator::Span => {
                flush_pending_hchars(nest, stores, execution.command_fuel())?;
                let next_column = column.checked_add(1).ok_or(ExecError::ArithmeticOverflow)?;
                if align_state(nest, align_level)?
                    .column_for(next_column)
                    .is_none()
                {
                    report_extra_alignment_tab(input, stores)?;
                    package_cell(
                        (align_level, kind),
                        span_count,
                        next_column,
                        migrations,
                        nest,
                        stores,
                        execution.command_fuel(),
                    )?;
                    leave_group(input, stores, tex_state::GroupKind::Align)?;
                    stores.enter_group_with_kind(tex_state::GroupKind::Align);
                    mutate_align_state(nest, align_level, |state| state.finish_cell(next_column))?;
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
                input.set_alignment_scanner_phase(tex_state::AlignmentScannerPhase::BetweenEntries);
                first_token = next_non_space_protected(input, stores, execution)?;
            }
            CellTerminator::AlignmentTab | CellTerminator::Cr => {
                let next_column = column + 1;
                let extra_alignment_tab = matches!(terminator, CellTerminator::AlignmentTab)
                    && align_state(nest, align_level)?
                        .column_for(next_column)
                        .is_none();
                if extra_alignment_tab {
                    report_extra_alignment_tab(input, stores)?;
                }
                package_cell(
                    (align_level, kind),
                    span_count,
                    next_column,
                    migrations,
                    nest,
                    stores,
                    execution.command_fuel(),
                )?;
                leave_group(input, stores, tex_state::GroupKind::Align)?;
                // WEB fin_col immediately installs the next entry align_group,
                // including after a row-ending \cr for fin_align to remove.
                stores.enter_group_with_kind(tex_state::GroupKind::Align);
                mutate_align_state(nest, align_level, |state| state.finish_cell(next_column))?;
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
                    token.semantic_token(),
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
    alignment: (usize, AlignmentKind),
    span_count: u16,
    next_boundary: usize,
    migrations: &mut Vec<Node>,
    nest: &mut ModeNest,
    stores: &mut Universe,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    let (align_level, kind) = alignment;
    if kind == AlignmentKind::VAlign && nest.current_mode() == Mode::Horizontal {
        crate::assignments::end_paragraph_with_fuel(nest, stores, fuel)?;
    }

    let mut cell_level = crate::assignments::commit_current_list(nest, stores, fuel)?;
    let nodes = cell_level.list_mutation().take_nodes();
    let nodes = if kind == AlignmentKind::HAlign {
        // TeX82 §796 packs an `\halign` column with `adjust_tail:=cur_tail`,
        // so §651/§655 move its insertions, marks, and `\vadjust` contents
        // onto the row's holding list for §799 to append after the row. A
        // `\valign` column is `vpackage`d with `adjust_tail` null.
        let nodes = crate::math::finish_math_lists_owned(stores, nodes, false);
        let (retained, mut pre_migrated, migrated) =
            crate::assignments::split_hpack_migrations(stores, nodes);
        pre_migrated.extend(migrated);
        migrations.extend(pre_migrated);
        retained
    } else {
        nodes
    };
    let children = stores.freeze_node_list(&nodes);
    let cell = super::packaging::make_unset_node(
        stores,
        children,
        super::packaging::cell_unset_kind(kind),
        span_count,
        super::packaging::UnsetPackContext::Cell,
    )?;
    nest.current_list_mutation().push(cell);
    let tabskip = align_state(nest, align_level)?.tabskip_for_boundary(next_boundary);
    nest.current_list_mutation().push(Node::Glue {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DoEndV {
    NotApplicable,
    FinishCell,
    Recovered,
}

/// Applies the stack and group gates at the front of TeX82's `do_endv`.
///
/// The cell driver owns `fin_col`/`fin_row`, but aliases can reach ordinary
/// main control while an intervening group is still open. Keeping this gate
/// shared makes both paths validate the same exhausted v-template sentinel
/// before applying `off_save` and replaying end-v.
pub(crate) fn do_endv(
    command: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
) -> Result<DoEndV, ExecError> {
    if !command.semantic_token().is_frozen_endv()
        || !(input.has_exhausted_alignment_v_template(stores)
            || input.is_template_driver_endv(command)
            || (command
                .token()
                .is_some_and(tex_state::token::Token::is_frozen_endv)
                && input.has_terminating_alignment_cell()))
    {
        return Ok(DoEndV::NotApplicable);
    }
    if !input.has_active_alignment_cell() {
        input.retire_orphaned_alignment_v_templates();
        return Ok(DoEndV::Recovered);
    }
    if stores.innermost_group_kind() == Some(tex_state::GroupKind::Align) {
        return Ok(DoEndV::FinishCell);
    }
    crate::assignments::off_save_alignment(command, input, stores)?;
    Ok(DoEndV::Recovered)
}

/// Handles an end-v command that returns directly to the synchronous
/// u-template driver after nested lookahead has retired its source replay.
///
/// The driver and active cell together are the exact ownership proof that
/// ordinary main control obtains from `do_endv`'s input-stack walk. Group
/// recovery remains identical, and all commands outside this narrow return
/// path continue through the strict stack gate above.
fn do_template_driver_endv(
    command: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
) -> Result<DoEndV, ExecError> {
    if !command.semantic_token().is_frozen_endv() || !input.has_terminating_alignment_cell() {
        return Ok(DoEndV::NotApplicable);
    }
    let marked = input.mark_template_driver_endv(command);
    debug_assert!(marked, "terminating cell must accept its driver end-v");
    if stores.innermost_group_kind() == Some(tex_state::GroupKind::Align) {
        return Ok(DoEndV::FinishCell);
    }
    crate::assignments::off_save_alignment(command, input, stores)?;
    Ok(DoEndV::Recovered)
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
                if let Some(terminator) = input.finish_terminating_alignment_cell(stores) {
                    return classify_cell_terminator(stores, terminator);
                }
                return Err(ExecError::MissingToken {
                    context: "alignment cell",
                });
            }
            Err(error) if error.is_undefined_control_sequence() => {
                // §370 names the offending control sequence only through
                // §82's context display, never in the message text.
                report_input_error(
                    input,
                    stores,
                    "Undefined control sequence",
                    UNDEFINED_CONTROL_SEQUENCE_HELP,
                )?;
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        let semantic = token.semantic_token();
        if semantic.is_frozen_endv() {
            match do_endv(token, input, stores)? {
                DoEndV::Recovered => continue,
                DoEndV::FinishCell => {}
                DoEndV::NotApplicable => {
                    return Err(ExecError::MissingToken {
                        context: "exhausted alignment v-template",
                    });
                }
            }
            let terminator =
                input
                    .finish_alignment_cell(stores)
                    .ok_or(ExecError::MissingToken {
                        context: "alignment cell terminator",
                    })?;
            return classify_cell_terminator(stores, terminator);
        }
        stats.delivered_tokens += 1;
        if is_noalign(stores, semantic) {
            // TeX82 §1129's `no_align_error`.
            report_input_error(
                input,
                stores,
                "Misplaced \\noalign",
                &[
                    "I expect to see \\noalign only after the \\cr of",
                    "an alignment. Proceed, and I'll ignore this case.",
                ],
            )?;
            continue;
        }
        if is_omit(stores, semantic) {
            if stores.interaction_mode() == InteractionMode::ErrorStop {
                return Err(ExecError::MisplacedOmit);
            }
            // TeX82 §1129's `omit_error`.
            report_input_error(
                input,
                stores,
                "Misplaced \\omit",
                &[
                    "I expect to see \\omit only after tab marks or the \\cr of",
                    "an alignment. Proceed, and I'll ignore this case.",
                ],
            )?;
            continue;
        }
        if is_alignment_par(stores, semantic) && input.alignment_cell_below_base_depth() {
            // TeX.web §1091 hmode+par_end calls off_save when the
            // alignment brace level is negative. Backing up \par behind
            // the inserted right brace lets ordinary group dispatch
            // reach §1103's align_group recovery in the same order.
            recover_alignment_par_token(token, input, stores)?;
            continue;
        }
        if is_end_group(stores, semantic)
            && input.alignment_cell_below_base_depth()
            && stores.innermost_group_kind() == Some(tex_state::GroupKind::Align)
        {
            // TeX82 §1132's `align_group` case of `handle_right_brace` does
            // not unsave the align_group. It backs the brace up and reaches
            // `ins_error` with frozen \cr, which may itself need §1127's
            // missing-left-brace recovery before get_next can start v_j.
            let cr = stores.symbol("cr").ok_or(ExecError::MissingToken {
                context: "alignment recovery cr",
            })?;
            let cr = TracedTokenWord::pack(Token::Cs(cr.symbol()), token.origin());
            // §325's `back_input` for the delivered brace, whose alignment
            // depth accounting must be undone before it is read again, and a
            // separate `inserted` level for TeX's own repair token. Keeping
            // them apart is what makes §314 label them `<to be read again>`
            // and `<inserted text>` on their own context lines.
            back_tokens(input, stores, [token]);
            insert_traced_tokens(input, stores, [cr]);
            report_input_error(
                input,
                stores,
                "Missing \\cr inserted",
                &["I'm guessing that you meant to end an alignment here."],
            )?;
            continue;
        }
        if input.alignment_cell_below_base_depth()
            && (is_alignment_tab(stores, semantic)
                || is_span(stores, semantic)
                || is_cr(stores, semantic))
        {
            // TeX82 §1127's `align_error` with `align_state<0`: back up the
            // delimiter and put a left brace before it. Reading that inserted
            // brace brings the scanner level back to zero; the replayed
            // delimiter then starts v_j through the ordinary get_next
            // interception path.
            let left = Token::Char {
                ch: '{',
                cat: tex_state::token::Catcode::BeginGroup,
            };
            let origin = stores.inserted_origin(
                tex_state::provenance::InsertedOriginKind::ErrorRecovery,
                left,
                token.origin(),
            );
            back_tokens(input, stores, [token]);
            // The brace is TeX's own repair rather than a token get_next
            // already counted, so it is inserted without the brace-depth undo
            // `back_tokens` applies.
            insert_traced_tokens(input, stores, [TracedTokenWord::pack(left, origin)]);
            report_input_error(
                input,
                stores,
                "Missing { inserted",
                &[
                    "I've put in what seems to be necessary to fix",
                    "the current column of the current alignment.",
                    "Try to go on, since this might almost work.",
                ],
            )?;
            continue;
        }
        dispatch_and_drain(nest, token, input, stores, execution, &mut stats)?;
    }
}

fn classify_cell_terminator(
    stores: &mut Universe,
    terminator: TracedTokenWord,
) -> Result<CellTerminator, ExecError> {
    let semantic = terminator.semantic_token();
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
    EndV(TracedTokenWord),
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
    #[cfg(feature = "profiling")]
    super::record_template_token(token.semantic_token(), stores);
    if token.semantic_token().is_frozen_endv() {
        return Ok(TemplateStep::EndV(token));
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
        Err(error) if error.is_undefined_control_sequence() => {
            // §370 names the offending control sequence only through §82's
            // context display, never in the message text.
            report_input_error(
                input,
                stores,
                "Undefined control sequence",
                UNDEFINED_CONTROL_SEQUENCE_HELP,
            )?;
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    match action {
        DispatchAction::Continue => {
            crate::legacy_output::drain_pending_output(nest, input, stores, execution, stats)?;
            Ok(())
        }
        DispatchAction::Shipout(page) => {
            stats.prepared_dvi_pages.push(page);
            crate::legacy_output::drain_pending_output(nest, input, stores, execution, stats)?;
            Ok(())
        }
        DispatchAction::End => Ok(()),
        DispatchAction::NotConsumed => Err(ExecError::UnimplementedTypesetting {
            mode: nest.current_mode(),
            token: token.semantic_token(),
            origin: token.origin(),
            operation: "alignment cell",
        }),
    }
}

fn is_alignment_par(stores: &Universe, token: Token) -> bool {
    let Token::Cs(symbol) = token else {
        return false;
    };
    matches!(
        stores.meaning(symbol),
        tex_state::meaning::Meaning::UnexpandablePrimitive(
            tex_state::meaning::UnexpandablePrimitive::Par
        )
    )
}

fn recover_alignment_par_token(
    context: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    let closing = Token::Char {
        ch: '}',
        cat: tex_state::token::Catcode::EndGroup,
    };
    let origin = stores.inserted_origin(
        tex_state::provenance::InsertedOriginKind::ErrorRecovery,
        closing,
        context.origin(),
    );
    // TeX82 §1064's `off_save`: `back_input`, then §1065 builds the token
    // that matches the open group -- `}` for an align_group -- and `ins_list`
    // puts it above the backed-up one before `error` displays both levels.
    back_tokens(input, stores, [context]);
    insert_traced_tokens(input, stores, [TracedTokenWord::pack(closing, origin)]);
    report_input_error(
        input,
        stores,
        "Missing } inserted",
        &[
            "I've inserted something that you may have forgotten.",
            "(See the <inserted text> above.)",
            "With luck, this will get me unwedged. But if you",
            "really didn't forget anything, try typing `2' now; then",
            "my insertion and my current dilemma will both disappear.",
        ],
    )?;
    Ok(())
}
