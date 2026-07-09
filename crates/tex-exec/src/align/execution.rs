use tex_expand::{ExpansionHooks, ReadRecorder, get_x_token_with_recorder_and_hooks};
use tex_lex::{InputSource, InputStack, TokenListReplayKind};
use tex_state::env::banks::TokParam;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::node::{GlueKind, Node, UnsetKind, UnsetNode, UnsetNodeFields};
use tex_state::token::{Catcode, Token};
use tex_state::{ExpansionContext, PrintSink, Universe};
use tex_typeset::measure_unset;

use crate::assignments::{flush_pending_hchars, next_non_space_x};
use crate::dispatch::dispatch_delivered_token_with_recorder;
use crate::executor::sync_engine_state;
use crate::mode::{AlignState, AlignmentKind};
use crate::vertical::{append_vertical_contribution, build_page_if_outer_vertical};
use crate::{DispatchAction, ExecError, ExecutionStats, Mode, ModeNest, leave_group, push_tokens};

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
    let alignment_kind = state.kind();
    nest.push(alignment_mode(alignment_kind));
    let align_level = nest.depth() - 1;
    nest.current_list_mut().set_align_state(state);
    replay_everycr(input, stores);

    while let Some(first_token) = align_peek(align_level, nest, input, stores, recorder, hooks)? {
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
    let alignment_kind = state.kind();
    nest.push(alignment_mode(alignment_kind));
    let align_level = nest.depth() - 1;
    nest.current_list_mut().set_align_state(state);
    replay_everycr(input, stores);

    while let Some(first_token) = align_peek(align_level, nest, input, stores, recorder, hooks)? {
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
) -> Result<Option<Token>, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    loop {
        set_align_brace_depth(nest, align_level, 1_000_000);
        let Some(token) = next_non_space_x(input, stores, hooks)? else {
            return Err(ExecError::MissingToken {
                context: "alignment row",
            });
        };
        set_align_brace_depth(nest, align_level, 0);
        if is_noalign(stores, token) {
            execute_noalign(align_level, nest, input, stores, recorder, hooks)?;
            continue;
        }
        if is_end_group(stores, token) {
            leave_group(input, stores, tex_state::GroupKind::Simple)?;
            return Ok(None);
        }
        if is_cr(stores, token) {
            continue;
        }
        return Ok(Some(token));
    }
}

fn execute_noalign<S, R, H>(
    align_level: usize,
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
    let opener = next_non_space_x(input, stores, hooks)?.ok_or(ExecError::MissingToken {
        context: "\\noalign group",
    })?;
    if !is_begin_group(stores, opener) {
        report_missing_left_brace_inserted(stores);
        push_tokens(input, stores, [opener]);
    }
    stores.enter_group_with_kind(tex_state::GroupKind::Simple);
    nest.push(Mode::InternalVertical);
    scan_noalign_group(nest, input, stores, recorder, hooks)?;
    let level = nest.pop()?;
    let nodes = level.list().nodes().to_vec();
    leave_group(input, stores, tex_state::GroupKind::Simple)?;
    let align_list = nest.list_mut(align_level).ok_or(ExecError::MissingToken {
        context: "alignment state",
    })?;
    align_list.append(nodes);
    Ok(())
}

fn scan_noalign_group<S, R, H>(
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
    let mut stats = ExecutionStats::default();
    let mut brace_depth = 1usize;
    loop {
        sync_engine_state::<S, _>(hooks, nest, stores);
        let token = {
            let mut expansion = ExpansionContext::new(stores);
            get_x_token_with_recorder_and_hooks(input, &mut expansion, recorder, hooks)?
        }
        .ok_or(ExecError::MissingToken {
            context: "\\noalign closing brace",
        })?;
        if is_begin_group(stores, token) {
            brace_depth += 1;
        }
        if is_end_group(stores, token) {
            brace_depth -= 1;
            if brace_depth == 0 {
                flush_pending_hchars(nest, stores)?;
                return Ok(());
            }
        }
        dispatch_and_drain(nest, token, input, stores, recorder, hooks, &mut stats)?;
    }
}

fn init_row(align_level: usize, nest: &mut ModeNest) -> Result<(), ExecError> {
    let kind = align_kind(nest, align_level)?;
    let first_tabskip = align_state(nest, align_level)?.tabskip_for_boundary(0);
    align_state_mut(nest, align_level)?.start_row();
    nest.push(row_mode(kind));
    nest.current_list_mut().push(Node::Glue {
        spec: first_tabskip,
        kind: GlueKind::TabSkip,
    });
    Ok(())
}

fn execute_row<S, R, H>(
    align_level: usize,
    first_token: Token,
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
        start_token = Some(next_non_space_x(input, stores, hooks)?.ok_or(
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
    let row = make_unset_node(stores, children, row_unset_kind(kind), 1);
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
    first_token: Option<Token>,
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
    let mut column = start.column;
    let mut span_count = 1u16;
    let mut first_token = start.first_token;
    loop {
        let initial = first_token.take();
        let omit = initial.is_some_and(|token| is_omit(stores, token));
        align_state_mut(nest, align_level)?.start_cell(column, span_count);
        let column_templates = align_state(nest, align_level)?
            .column_for(column)
            .copied()
            .ok_or(ExecError::MissingToken {
                context: "alignment template",
            })?;
        if !omit {
            if let Some(token) = initial {
                push_tokens(input, stores, [token]);
            }
            if span_count > 1 {
                expand_spanned_column_template_at_span_time(
                    column_templates.u_template,
                    align_level,
                    nest,
                    input,
                    stores,
                    recorder,
                    hooks,
                )?;
            } else {
                replay_template(
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
            replay_template(
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
                first_token = next_non_space_x(input, stores, hooks)?;
            }
            CellTerminator::AlignmentTab | CellTerminator::Cr => {
                let next_column = column + 1;
                package_cell(align_level, kind, span_count, next_column, nest, stores)?;
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
    flush_pending_hchars(nest, stores)?;
    let cell_level = nest.pop()?;
    let nodes = if kind == AlignmentKind::HAlign {
        crate::math::finish_math_lists(stores, cell_level.list().nodes(), false)
    } else {
        cell_level.list().nodes().to_vec()
    };
    let children = stores.freeze_node_list(&nodes);
    let cell = make_unset_node(stores, children, cell_unset_kind(kind), span_count);
    nest.current_list_mut().push(cell);
    let tabskip = align_state(nest, align_level)?.tabskip_for_boundary(next_boundary);
    nest.current_list_mut().push(Node::Glue {
        spec: tabskip,
        kind: GlueKind::TabSkip,
    });
    Ok(())
}

fn replay_template<S, R, H>(
    template: tex_state::ids::TokenListId,
    sentinel: Option<Token>,
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
    if stores.tokens(template).is_empty() {
        return Ok(());
    }
    input.push_token_list(template, TokenListReplayKind::Inserted);
    let mut stats = ExecutionStats::default();
    loop {
        if template_finished(input, stores, template, sentinel) {
            return Ok(());
        }
        run_one_main_control_token(nest, input, stores, recorder, hooks, &mut stats)?;
    }
}

fn expand_spanned_column_template_at_span_time<S, R, H>(
    template: tex_state::ids::TokenListId,
    align_level: usize,
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
    // Architecture §7 makes alignment the only impure kernel: span-time
    // template expansion is the single explicit gullet interleave while the
    // mutable alignment state on the mode nest is live.
    let _ = align_level;
    replay_template(template, None, nest, input, stores, recorder, hooks)
}

fn template_finished<S>(
    input: &mut InputStack<S>,
    stores: &Universe,
    template: tex_state::ids::TokenListId,
    sentinel: Option<Token>,
) -> bool {
    let Some((frame, replay_kind, index)) = input.current_token_list_frame() else {
        return true;
    };
    if frame != template || replay_kind != TokenListReplayKind::Inserted {
        return false;
    }
    let tokens = stores.tokens(template);
    if sentinel.is_some_and(|token| tokens.get(index).is_some_and(|&next| next == token)) {
        return input.pop_current_token_list_frame(template, TokenListReplayKind::Inserted);
    }
    if index >= tokens.len() {
        return input.pop_current_token_list_frame(template, TokenListReplayKind::Inserted);
    }
    false
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
        stats.delivered_tokens += 1;
        if align_state(nest, align_level)?.brace_depth() == 0 {
            if is_alignment_tab(stores, token) {
                return Ok(CellTerminator::AlignmentTab);
            }
            if is_cr(stores, token) {
                return Ok(CellTerminator::Cr);
            }
            if is_span(stores, token) {
                return Ok(CellTerminator::Span);
            }
            if is_noalign(stores, token) {
                return Err(ExecError::MisplacedNoAlign);
            }
            if is_omit(stores, token) {
                return Err(ExecError::MisplacedOmit);
            }
            if is_end_group(stores, token) {
                report_missing_cr_inserted(stores);
                push_tokens(input, stores, [token]);
                return Ok(CellTerminator::Cr);
            }
        }
        update_cell_brace_depth(align_level, nest, stores, token)?;
        dispatch_and_drain(nest, token, input, stores, recorder, hooks, &mut stats)?;
    }
}

fn run_one_main_control_token<S, R, H>(
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

fn dispatch_and_drain<S, R, H>(
    nest: &mut ModeNest,
    token: Token,
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
            token,
            operation: "alignment cell",
        }),
    }
}

fn update_cell_brace_depth(
    align_level: usize,
    nest: &mut ModeNest,
    stores: &Universe,
    token: Token,
) -> Result<(), ExecError> {
    if is_begin_group(stores, token) {
        align_state_mut(nest, align_level)?.increment_brace_depth();
    } else if is_end_group(stores, token) {
        align_state_mut(nest, align_level)?.decrement_brace_depth();
    }
    Ok(())
}

fn make_unset_node(
    stores: &Universe,
    children: tex_state::ids::NodeListId,
    kind: UnsetKind,
    span_count: u16,
) -> Node {
    let metrics = measure_unset(stores, children, kind);
    Node::Unset(UnsetNode::new(UnsetNodeFields {
        kind,
        width: metrics.width,
        height: metrics.height,
        depth: metrics.depth,
        span_count,
        stretch: metrics.stretch,
        stretch_order: metrics.stretch_order,
        shrink: metrics.shrink,
        shrink_order: metrics.shrink_order,
        children,
    }))
}

fn report_missing_cr_inserted(stores: &mut Universe) {
    stores
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, "\n! Missing \\cr inserted.\n");
}

fn report_missing_left_brace_inserted(stores: &mut Universe) {
    stores
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, "\n! Missing { inserted.\n");
}

fn align_state(nest: &ModeNest, align_level: usize) -> Result<&AlignState, ExecError> {
    nest.list(align_level)
        .and_then(crate::mode::ModeList::align_state)
        .ok_or(ExecError::MissingToken {
            context: "alignment state",
        })
}

fn align_state_mut(nest: &mut ModeNest, align_level: usize) -> Result<&mut AlignState, ExecError> {
    nest.list_mut(align_level)
        .and_then(crate::mode::ModeList::align_state_mut)
        .ok_or(ExecError::MissingToken {
            context: "alignment state",
        })
}

fn set_align_brace_depth(nest: &mut ModeNest, align_level: usize, value: i32) {
    if let Some(state) = nest
        .list_mut(align_level)
        .and_then(crate::mode::ModeList::align_state_mut)
    {
        state.set_brace_depth(value);
    }
}

fn align_kind(nest: &ModeNest, align_level: usize) -> Result<AlignmentKind, ExecError> {
    Ok(align_state(nest, align_level)?.kind())
}

fn alignment_mode(kind: AlignmentKind) -> Mode {
    match kind {
        AlignmentKind::HAlign => Mode::InternalVertical,
        AlignmentKind::VAlign => Mode::RestrictedHorizontal,
    }
}

fn row_mode(kind: AlignmentKind) -> Mode {
    match kind {
        AlignmentKind::HAlign => Mode::RestrictedHorizontal,
        AlignmentKind::VAlign => Mode::InternalVertical,
    }
}

fn cell_mode(kind: AlignmentKind) -> Mode {
    row_mode(kind)
}

fn cell_unset_kind(kind: AlignmentKind) -> UnsetKind {
    match kind {
        AlignmentKind::HAlign => UnsetKind::HBox,
        AlignmentKind::VAlign => UnsetKind::VBox,
    }
}

fn row_unset_kind(kind: AlignmentKind) -> UnsetKind {
    cell_unset_kind(kind)
}

fn is_alignment_tab(stores: &Universe, token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::AlignmentTab,
            ..
        }
    ) || matches!(
        token_meaning(stores, token),
        Some(Meaning::CharToken {
            cat: Catcode::AlignmentTab,
            ..
        })
    )
}

fn is_begin_group(stores: &Universe, token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::BeginGroup,
            ..
        }
    ) || matches!(
        token_meaning(stores, token),
        Some(Meaning::CharToken {
            cat: Catcode::BeginGroup,
            ..
        })
    )
}

fn is_end_group(stores: &Universe, token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::EndGroup,
            ..
        }
    ) || matches!(
        token_meaning(stores, token),
        Some(Meaning::CharToken {
            cat: Catcode::EndGroup,
            ..
        })
    )
}

fn is_noalign(stores: &Universe, token: Token) -> bool {
    primitive_token(stores, token) == Some(UnexpandablePrimitive::NoAlign)
}

fn is_omit(stores: &Universe, token: Token) -> bool {
    primitive_token(stores, token) == Some(UnexpandablePrimitive::Omit)
}

fn is_cr(stores: &Universe, token: Token) -> bool {
    matches!(
        primitive_token(stores, token),
        Some(UnexpandablePrimitive::Cr | UnexpandablePrimitive::CrCr)
    )
}

fn is_span(stores: &Universe, token: Token) -> bool {
    primitive_token(stores, token) == Some(UnexpandablePrimitive::Span)
}

fn primitive_token(stores: &Universe, token: Token) -> Option<UnexpandablePrimitive> {
    match token_meaning(stores, token) {
        Some(Meaning::UnexpandablePrimitive(primitive)) => Some(primitive),
        _ => None,
    }
}

fn token_meaning(stores: &Universe, token: Token) -> Option<Meaning> {
    let Token::Cs(symbol) = token else {
        return None;
    };
    Some(stores.meaning(symbol))
}
