//! TeX.web page-builder accounting for outer vertical contributions.

use tex_command::FatalError;
use tex_state::diagnostic::Diagnostic;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::node::{GlueKind, Node};
use tex_state::node_arena::NodeRef;
use tex_state::page::{
    AWFUL_BAD, DEPLORABLE, EJECT_PENALTY, INF_PENALTY, PageContents, PageDimension, PageInsertion,
    PageInsertionStatus,
};
use tex_state::scaled::{Scaled, nx_plus_y, x_over_n};
use tex_state::{
    ContentHash, MemoTimingPhase, MemoValueLimits, PureMemoKey, PureMemoLayer, PurePageEntry,
    Universe,
};
use tex_typeset::{INF_BAD, VerticalBreakError, badness, vert_break};

use crate::{ExecError, diagnostics};

const PAGE_EPISODE_DOMAIN: u32 = 3;
const PAGE_EPISODE_SCHEMA: u32 = 1;
const PAGE_ENV_HASH_DOMAIN: u64 = 0x7061_6765_656e_7601;

#[cfg(test)]
pub(crate) fn build_page(stores: &mut Universe) -> Result<(), ExecError> {
    build_page_impl(stores, None)
}

/// Runs TeX82's page builder with the live §82 input display of the command
/// whose contribution triggered it.
pub(crate) fn build_page_with_error_context(
    stores: &mut Universe,
    error_context: &str,
) -> Result<(), ExecError> {
    build_page_impl(stores, Some(error_context))
}

/// The shared implementation. `None` is reserved for input-free continuations
/// that have no borrowed command cursor and therefore use the last published
/// Universe summary.
fn build_page_impl(stores: &mut Universe, error_context: Option<&str>) -> Result<(), ExecError> {
    if stores.page_fire_up().is_some() {
        return Ok(());
    }
    if stores.page_contributions().is_empty() {
        return build_page_cold(stores, error_context);
    }
    if !stores
        .with_pure_memo(|memo| memo.page_episodes_enabled())
        .unwrap_or(false)
    {
        if stores
            .with_pure_memo(|memo| memo.is_enabled())
            .unwrap_or(false)
        {
            stores.with_pure_memo(|memo| memo.record_not_attempted(PureMemoLayer::Page));
        }
        return build_page_cold(stores, error_context);
    }
    let validation_started = crate::timing::TelemetryTimer::start();
    let key = page_episode_key(stores);
    stores.with_pure_memo(|memo| {
        memo.record_timing(
            PureMemoLayer::Page,
            MemoTimingPhase::Validation,
            validation_started.elapsed(),
        );
    });
    let input_origins = stores.page_memo_origins().ok();
    if let Some(entry) = stores
        .with_pure_memo(|memo| memo.lookup_page(key))
        .flatten()
    {
        let import_started = crate::timing::TelemetryTimer::start();
        let imported_bytes = entry.transition.retained_bytes();
        let replay_origins = input_origins.as_ref().map(|input_origins| {
            entry
                .origin_ordinals
                .iter()
                .map(|ordinal| {
                    usize::try_from(*ordinal)
                        .ok()
                        .and_then(|ordinal| input_origins.get(ordinal))
                        .cloned()
                        .unwrap_or_else(tex_state::provenance::OriginRef::unknown)
                })
                .collect::<Vec<_>>()
        });
        let imported = replay_origins.as_ref().is_some_and(|origins| {
            stores
                .import_page_memo_transition(&entry.transition, MemoValueLimits::default(), origins)
                .is_ok()
        });
        stores.with_pure_memo(|memo| {
            memo.record_timing(
                PureMemoLayer::Page,
                MemoTimingPhase::Import,
                import_started.elapsed(),
            );
        });
        if imported {
            stores.with_pure_memo(|memo| {
                memo.record_page_hit(entry.contributions, imported_bytes);
            });
            return Ok(());
        }
        stores.with_pure_memo(|memo| {
            memo.record_page_import_failure();
            memo.reject(key);
        });
    }
    let contributions = stores.page_contributions().len();
    let effect_start = stores.world().effect_records().len();
    build_page_cold(stores, error_context)?;
    if stores.world().effect_records().len() == effect_start
        && let Some(input_origins) = input_origins
        && let Ok((transition, output_origins)) = stores.detach_page_memo_transition()
    {
        let origin_ordinals = output_origins
            .iter()
            .map(|origin| {
                input_origins
                    .iter()
                    .position(|candidate| candidate == origin)
                    .and_then(|index| u32::try_from(index).ok())
                    .unwrap_or(u32::MAX)
            })
            .collect();
        stores.with_pure_memo(|memo| {
            memo.insert_page(
                key,
                PurePageEntry {
                    transition,
                    contributions,
                    origin_ordinals,
                },
            );
        });
    }
    Ok(())
}

fn page_episode_key(stores: &Universe) -> PureMemoKey {
    let page = stores.page_memo_fingerprint();
    let mut classes: Vec<u16> = stores
        .page_contributions()
        .iter()
        .filter_map(|node| match node {
            Node::Ins { class, .. } => Some(*class),
            _ => None,
        })
        .collect();
    classes.sort_unstable();
    classes.dedup();
    let environment = stores.engine_boundary_hash(PAGE_ENV_HASH_DOMAIN, |hash| {
        hash.i32(stores.dimen_param(DimenParam::V_SIZE).raw());
        hash.i32(stores.dimen_param(DimenParam::MAX_DEPTH).raw());
        hash.glue(stores.glue_param(GlueParam::TOP_SKIP));
        hash.i32(stores.int_param(IntParam::SAVING_V_DISCARDS));
        // A traced episode prints; reuse must not silently swallow its text.
        hash.i32(stores.int_param(IntParam::TRACING_PAGES));
        hash.i32(stores.int_param(IntParam::TRACING_ONLINE));
        for class in &classes {
            hash.u16(*class);
            hash.i32(stores.count(*class));
            hash.i32(stores.dimen(*class).raw());
            hash.glue(stores.skip(*class));
            match stores.box_reg_ref(*class) {
                Some(list) => {
                    hash.u32(1);
                    hash.node_list_ref(&list);
                }
                None => hash.u32(0),
            }
        }
    });
    let mut bytes = Vec::with_capacity(24 + classes.len() * 2);
    bytes.extend_from_slice(&PAGE_EPISODE_SCHEMA.to_le_bytes());
    bytes.extend_from_slice(&page.to_le_bytes());
    bytes.extend_from_slice(&environment.to_le_bytes());
    for class in classes {
        bytes.extend_from_slice(&class.to_le_bytes());
    }
    PureMemoKey::new(
        PAGE_EPISODE_DOMAIN,
        page ^ environment,
        ContentHash::from_bytes(&bytes),
    )
}

fn build_page_cold(stores: &mut Universe, error_context: Option<&str>) -> Result<(), ExecError> {
    if stores.page_fire_up().is_some() {
        return Ok(());
    }

    while let Some(node) = stores.page_contribution_front().cloned() {
        if !matches!(
            node,
            Node::HList(_)
                | Node::VList(_)
                | Node::Rule { .. }
                | Node::Glue { .. }
                | Node::Kern { .. }
                | Node::Penalty(_)
                | Node::Ins { .. }
                | Node::Whatsit(_)
                | Node::Mark { .. }
        ) {
            return Err(ExecError::Fatal(FatalError::confusion("page")));
        }
        stores.update_page_last_from_node(&node);
        match node {
            Node::HList(_) | Node::VList(_) | Node::Rule { .. } => {
                if !stores.page_contents().has_box() {
                    initialize_page_with_topskip(stores, &node)?;
                    continue;
                }
                prepare_box_or_rule(stores, &node)?;
                contribute_front(stores)?;
            }
            Node::Glue { ref spec, .. } => {
                if !stores.page_contents().has_box() {
                    discard_front(stores);
                } else if stores.current_page_tail().is_some_and(precedes_break) {
                    check_break(stores, 0)?;
                    if stores.page_fire_up().is_some() {
                        return Ok(());
                    }
                    let node = update_glue_or_kern(stores, &node, error_context)?;
                    contribute_front_as(stores, node)?;
                } else {
                    let _ = spec;
                    let node = update_glue_or_kern(stores, &node, error_context)?;
                    contribute_front_as(stores, node)?;
                }
            }
            Node::Kern { .. } => {
                if !stores.page_contents().has_box() {
                    discard_front(stores);
                } else if stores.page_contribution_second().is_none() {
                    return Ok(());
                } else if matches!(stores.page_contribution_second(), Some(Node::Glue { .. })) {
                    check_break(stores, 0)?;
                    if stores.page_fire_up().is_some() {
                        return Ok(());
                    }
                    let node = update_glue_or_kern(stores, &node, error_context)?;
                    contribute_front_as(stores, node)?;
                } else {
                    let node = update_glue_or_kern(stores, &node, error_context)?;
                    contribute_front_as(stores, node)?;
                }
            }
            Node::Penalty(penalty) => {
                if !stores.page_contents().has_box() {
                    discard_front(stores);
                } else {
                    check_break(stores, penalty)?;
                    if stores.page_fire_up().is_some() {
                        return Ok(());
                    }
                    contribute_front(stores)?;
                }
            }
            Node::Ins { .. } => {
                if stores.page_contents() == PageContents::Empty {
                    freeze_page_specs(stores, PageContents::InsertsOnly);
                }
                let node = prepare_insertion(stores, &node, error_context)?.unwrap_or(node);
                contribute_front_as(stores, node)?;
            }
            Node::Whatsit(_)
                if !stores.page_contents().has_box()
                    && crate::splitting::is_page_top_discardable(&node) =>
            {
                discard_front(stores);
            }
            Node::Whatsit(_) | Node::Mark { .. } => {
                contribute_front(stores)?;
            }
            _ => return Err(ExecError::Fatal(FatalError::confusion("page"))),
        }
    }
    Ok(())
}

fn prepare_insertion(
    stores: &mut Universe,
    node: &Node,
    error_context: Option<&str>,
) -> Result<Option<Node>, ExecError> {
    let Node::Ins {
        class,
        size,
        split_max_depth,
        floating_penalty,
        content,
        ..
    } = node
    else {
        return Ok(None);
    };

    let mut insertion = match stores.page_insertion(*class) {
        Some(insertion) => insertion,
        None => create_page_insertion(stores, *class, error_context)?,
    };
    let mut replacement = None;

    match insertion.status() {
        PageInsertionStatus::SplitUp { .. } => {
            add_insert_penalty(stores, *floating_penalty);
        }
        PageInsertionStatus::Inserting => {
            let current_index = stores.current_page_len();
            insertion.set_last_ins_index(Some(current_index));
            let delta = insertion_delta(stores)?;
            let scaled_size = scaled_insertion_size(*size, stores.count(*class))?;
            if ((scaled_size.raw() <= 0) || scaled_size <= delta)
                && add(insertion.height(), *size)? <= stores.dimen(*class)
            {
                let goal = sub(stores.page_dimension(PageDimension::Goal), scaled_size)?;
                stores.set_page_dimension(PageDimension::Goal, goal);
                insertion.set_height(add(insertion.height(), *size)?);
            } else {
                replacement = split_page_insertion(
                    stores,
                    &mut insertion,
                    current_index,
                    node,
                    content.clone(),
                    *split_max_depth,
                    error_context,
                )?;
            }
        }
    }

    stores.upsert_page_insertion(insertion);
    Ok(replacement)
}

fn create_page_insertion(
    stores: &mut Universe,
    class: u16,
    error_context: Option<&str>,
) -> Result<PageInsertion, ExecError> {
    let existing_height = insertion_box_size(stores, class, error_context)?;
    let insertion = PageInsertion::new(class, existing_height);
    let scaled_height = scaled_insertion_size(existing_height, stores.count(class))?;
    let skip = stores.glue(stores.skip(class));
    let goal = sub(stores.page_dimension(PageDimension::Goal), scaled_height)?;
    let goal = sub(goal, skip.width)?;
    stores.set_page_dimension(PageDimension::Goal, goal);
    add_glue_stretch(stores, skip)?;
    let shrink = add(stores.page_dimension(PageDimension::Shrink), skip.shrink)?;
    stores.set_page_dimension(PageDimension::Shrink, shrink);
    if skip.shrink_order != Order::Normal && skip.shrink.raw() != 0 {
        diagnostics::report_insertion_skip_infinite_shrinkage(stores, class, error_context)?;
    }
    Ok(insertion)
}

fn insertion_box_size(
    stores: &mut Universe,
    class: u16,
    error_context: Option<&str>,
) -> Result<Scaled, ExecError> {
    let Some(list) = ensure_insertion_vbox(stores, class, error_context)? else {
        return Ok(Scaled::from_raw(0));
    };
    let Some(node) = list.nodes().first() else {
        return Ok(Scaled::from_raw(0));
    };
    match node {
        tex_state::node_arena::NodeRef::VList(box_node) => add(box_node.height, box_node.depth),
        _ => Ok(Scaled::from_raw(0)),
    }
}

/// TeX82 §993's `ensure_vbox`, used both when a class first reaches the page
/// and when §1018 prepares insertion queues during `fire_up`.
pub(crate) fn ensure_insertion_vbox(
    stores: &mut Universe,
    class: u16,
    error_context: Option<&str>,
) -> Result<Option<tex_state::node_arena::NodeListRef>, ExecError> {
    let Some(list) = stores.box_reg_ref(class) else {
        return Ok(None);
    };
    if !matches!(
        list.nodes().first(),
        Some(tex_state::node_arena::NodeRef::HList(_))
    ) {
        return Ok(Some(list));
    }

    // Production page building is synchronous with the contributing command,
    // whose dispatcher supplies §82's live display. Only the explicit
    // source-free test seam falls back to the published summary.
    let context = error_context.map_or_else(
        || crate::diagnostics::show_context(stores, stores.input_summary()),
        str::to_owned,
    );
    crate::error_report::report_error(
        stores,
        "Insertions can only be added to a vbox",
        &[
            "Tut tut: You're trying to \\insert into a",
            "\\box register that now contains an \\hbox.",
            "Proceed, and I'll discard its present contents.",
        ],
        context,
    )?;
    // TeX82 §993's `box_error` continues after `error`: it enters a diagnostic
    // scope, identifies the rejected box, and applies `show_box` before
    // flushing the register. `show_box` starts with §182's structural newline.
    let text = crate::node_dump::dump_node_list_ref(
        stores,
        &list,
        crate::node_dump::DumpConfig::read(stores),
    );
    let mut diagnostic = stores.begin_diagnostic();
    diagnostic
        .print_nl("The following box has been deleted:")
        .print_ln()
        .print_rendered(&text);
    diagnostic.end(true);
    // TeX82 §993 flushes the list and then assigns `box(n):=null` directly;
    // it does not call §275's `eq_define` or create a local save-stack entry.
    // Preserve the register's existing eq level while voiding its value.
    stores.clear_box_reg_same_level(class);
    Ok(None)
}

fn insertion_delta(stores: &Universe) -> Result<Scaled, ExecError> {
    let delta = sub(
        stores.page_dimension(PageDimension::Goal),
        stores.page_dimension(PageDimension::Total),
    )?;
    let delta = sub(delta, stores.page_dimension(PageDimension::Depth))?;
    add(delta, stores.page_dimension(PageDimension::Shrink))
}

fn split_page_insertion(
    stores: &mut Universe,
    insertion: &mut PageInsertion,
    current_index: usize,
    node: &Node,
    content: tex_state::node_arena::NodeListRef,
    split_max_depth: Scaled,
    error_context: Option<&str>,
) -> Result<Option<Node>, ExecError> {
    let class = insertion.class();
    let count = stores.count(class);
    let mut capacity = if count <= 0 {
        Scaled::MAX_DIMEN
    } else {
        let available = sub(
            sub(
                stores.page_dimension(PageDimension::Goal),
                stores.page_dimension(PageDimension::Total),
            )?,
            stores.page_dimension(PageDimension::Depth),
        )?;
        inverse_scaled_insertion_capacity(available, count)?
    };
    let remaining_cap = sub(stores.dimen(class), insertion.height())?;
    if capacity > remaining_cap {
        capacity = remaining_cap;
    }

    let mut content_nodes = content.to_vec();
    let split = vert_break(stores, &content_nodes, capacity, split_max_depth)
        .map_err(vertical_break_error)?;
    if stores.int_param(IntParam::TRACING_PAGES) > 0 {
        trace_insertion_split(stores, class, capacity, &split, &content_nodes);
    }
    let replacement = normalize_insert_content_shrink(
        stores,
        node,
        &mut content_nodes,
        &split.infinite_shrink_glue,
        error_context,
    )?;
    insertion.set_height(add(insertion.height(), split.best_height_plus_depth)?);
    let scaled_best = scaled_insertion_size(split.best_height_plus_depth, count)?;
    let goal = sub(stores.page_dimension(PageDimension::Goal), scaled_best)?;
    stores.set_page_dimension(PageDimension::Goal, goal);
    insertion.set_status(PageInsertionStatus::SplitUp {
        broken_ins_index: current_index,
        broken_at: split.break_index,
    });

    match split.break_index {
        None => add_insert_penalty(stores, EJECT_PENALTY),
        Some(index) => {
            if let Some(Node::Penalty(penalty)) = content_nodes.get(index) {
                add_insert_penalty(stores, *penalty);
            }
        }
    }
    Ok(replacement)
}

/// TeX82 §1012's insertion `vert_break` trace.
fn trace_insertion_split(
    stores: &mut Universe,
    class: u16,
    capacity: Scaled,
    split: &tex_typeset::VerticalBreak,
    content: &[Node],
) {
    let penalty = split.break_index.map_or(EJECT_PENALTY, |index| {
        content.get(index).map_or(0, |node| match node {
            Node::Penalty(value) => *value,
            _ => 0,
        })
    });
    let mut diagnostic = stores.begin_diagnostic();
    diagnostic.print_nl("% split").print_int(i32::from(class));
    diagnostic.print(" to ").print_scaled(capacity);
    diagnostic
        .print_char(',')
        .print_scaled(split.best_height_plus_depth);
    diagnostic.print(" p=").print_int(penalty);
    diagnostic.end(false);
}

fn add_insert_penalty(stores: &mut Universe, penalty: i32) {
    let value = stores
        .insert_penalties()
        .checked_add(penalty)
        .expect("page insertion penalty total overflow");
    stores.set_page_integer(tex_state::page::PageInteger::InsertPenalties, value);
}

pub(crate) fn scaled_insertion_size(size: Scaled, count: i32) -> Result<Scaled, ExecError> {
    if count == 1000 {
        return Ok(size);
    }
    let quotient = x_over_n(size, 1000)
        .map_err(|_| ExecError::ArithmeticOverflow)?
        .quotient;
    nx_plus_y(count, quotient, Scaled::from_raw(0)).map_err(|_| ExecError::ArithmeticOverflow)
}

fn inverse_scaled_insertion_capacity(size: Scaled, count: i32) -> Result<Scaled, ExecError> {
    if count == 1000 {
        return Ok(size);
    }
    let quotient = x_over_n(size, count)
        .map_err(|_| ExecError::ArithmeticOverflow)?
        .quotient;
    nx_plus_y(1000, quotient, Scaled::from_raw(0)).map_err(|_| ExecError::ArithmeticOverflow)
}

fn initialize_page_with_topskip(stores: &mut Universe, node: &Node) -> Result<(), ExecError> {
    if stores.page_contents() == PageContents::Empty {
        freeze_page_specs(stores, PageContents::BoxThere);
    } else {
        stores.set_page_contents(PageContents::BoxThere);
    }
    let top_skip = stores.glue(stores.glue_param(GlueParam::TOP_SKIP));
    let adjusted = GlueSpec {
        width: top_skip
            .width
            .checked_sub(vertical_height(node))
            .filter(|width| width.raw() > 0)
            .unwrap_or_else(|| Scaled::from_raw(0)),
        stretch: top_skip.stretch,
        stretch_order: top_skip.stretch_order,
        shrink: top_skip.shrink,
        shrink_order: top_skip.shrink_order,
    };
    let spec = stores.intern_glue(adjusted);
    stores.prepend_page_contribution(Node::Glue {
        spec,
        kind: GlueKind::TopSkip,
        leader: None,
    });
    Ok(())
}

fn prepare_box_or_rule(stores: &mut Universe, node: &Node) -> Result<(), ExecError> {
    let total = add(
        stores.page_dimension(PageDimension::Total),
        stores.page_dimension(PageDimension::Depth),
    )?;
    let total = add(total, vertical_height(node))?;
    stores.set_page_dimension(PageDimension::Total, total);
    stores.set_page_dimension(PageDimension::Depth, vertical_depth(node));
    Ok(())
}

fn update_glue_or_kern(
    stores: &mut Universe,
    node: &Node,
    error_context: Option<&str>,
) -> Result<Node, ExecError> {
    let mut replacement = None;
    let width = match node {
        Node::Kern { amount, .. } => *amount,
        Node::Glue { spec, kind, leader } => {
            let spec = stores.glue_spec(*spec);
            let spec = finite_page_shrink(stores, spec, error_context)?;
            let finite_id = stores.intern_glue(spec);
            replacement = Some(Node::Glue {
                spec: finite_id,
                kind: *kind,
                leader: leader.clone(),
            });
            add_glue_stretch(stores, spec)?;
            let shrink = add(stores.page_dimension(PageDimension::Shrink), spec.shrink)?;
            stores.set_page_dimension(PageDimension::Shrink, shrink);
            spec.width
        }
        _ => return Ok(node.clone()),
    };
    let total = add(
        stores.page_dimension(PageDimension::Total),
        stores.page_dimension(PageDimension::Depth),
    )?;
    let total = add(total, width)?;
    stores.set_page_dimension(PageDimension::Total, total);
    stores.set_page_dimension(PageDimension::Depth, Scaled::from_raw(0));
    Ok(replacement.unwrap_or_else(|| node.clone()))
}

fn finite_page_shrink(
    stores: &mut Universe,
    mut spec: GlueSpec,
    error_context: Option<&str>,
) -> Result<GlueSpec, ExecError> {
    if spec.shrink_order != Order::Normal && spec.shrink.raw() != 0 {
        diagnostics::report_page_infinite_shrinkage(stores, error_context)?;
        spec.shrink_order = Order::Normal;
    }
    Ok(spec)
}

fn normalize_insert_content_shrink(
    stores: &mut Universe,
    insert_node: &Node,
    content_nodes: &mut [Node],
    indices: &[usize],
    error_context: Option<&str>,
) -> Result<Option<Node>, ExecError> {
    if indices.is_empty() {
        return Ok(None);
    }

    let mut changed = false;
    for &index in indices {
        let Some(Node::Glue { spec, kind, leader }) = content_nodes.get(index) else {
            continue;
        };
        let mut finite = stores.glue_spec(*spec);
        if finite.shrink_order == Order::Normal || finite.shrink.raw() == 0 {
            continue;
        }
        diagnostics::report_split_infinite_shrinkage(stores, error_context)?;
        finite.shrink_order = Order::Normal;
        content_nodes[index] = Node::Glue {
            spec: stores.intern_glue(finite),
            kind: *kind,
            leader: leader.clone(),
        };
        changed = true;
    }

    if !changed {
        return Ok(None);
    }
    let Node::Ins {
        class,
        size,
        split_top_skip,
        split_max_depth,
        floating_penalty,
        ..
    } = insert_node
    else {
        return Ok(None);
    };
    let content = stores.freeze_node_list(content_nodes);
    Ok(Some(Node::Ins {
        class: *class,
        size: *size,
        split_top_skip: split_top_skip.clone(),
        split_max_depth: *split_max_depth,
        floating_penalty: *floating_penalty,
        content,
    }))
}

fn add_glue_stretch(stores: &mut Universe, spec: GlueSpec) -> Result<(), ExecError> {
    let dimension = match spec.stretch_order {
        Order::Normal => PageDimension::Stretch,
        Order::Fil => PageDimension::FilStretch,
        Order::Fill => PageDimension::FillStretch,
        Order::Filll => PageDimension::FilllStretch,
    };
    let value = add(stores.page_dimension(dimension), spec.stretch)?;
    stores.set_page_dimension(dimension, value);
    Ok(())
}

/// tex.web §987's `freeze_page_specs`, including the `\tracingpages` report
/// its `stat` block prints.
fn freeze_page_specs(stores: &mut Universe, contents: PageContents) {
    stores.freeze_page_specs(contents);
    if stores.int_param(IntParam::TRACING_PAGES) > 0 {
        let mut diagnostic = stores.begin_diagnostic();
        diagnostic.print_nl("%% goal height=");
        let goal = diagnostic.state().page_dimension(PageDimension::Goal);
        diagnostic.print_scaled(goal);
        diagnostic.print(", max depth=");
        let max_depth = diagnostic.state().page_max_depth();
        diagnostic.print_scaled(max_depth);
        diagnostic.end(false);
    }
}

/// tex.web §1006's "Display the page break cost", reached from §1005 once the
/// badness `b`, penalty `pi`, and cost `c` of a candidate breakpoint are known
/// and before `least_page_cost` is updated, so the trailing `#` marks the
/// champion the breakpoint is about to become.
fn trace_page_break_cost(stores: &mut Universe, badness: i32, penalty: i32, cost: i32) {
    let least_page_cost = stores.least_page_cost();
    let mut diagnostic = stores.begin_diagnostic();
    diagnostic.print_nl("%");
    diagnostic.print(" t=");
    print_page_totals(&mut diagnostic);
    diagnostic.print(" g=");
    let goal = diagnostic.state().page_dimension(PageDimension::Goal);
    diagnostic.print_scaled(goal);
    diagnostic.print(" b=");
    print_cost(&mut diagnostic, badness);
    diagnostic.print(" p=");
    diagnostic.print_int(penalty);
    diagnostic.print(" c=");
    print_cost(&mut diagnostic, cost);
    if cost <= least_page_cost {
        diagnostic.print_char('#');
    }
    diagnostic.end(false);
}

/// tex.web §985's `print_totals`.
fn print_page_totals(diagnostic: &mut Diagnostic<'_>) {
    let total = diagnostic.state().page_dimension(PageDimension::Total);
    diagnostic.print_scaled(total);
    for (dimension, unit) in [
        (PageDimension::Stretch, ""),
        (PageDimension::FilStretch, "fil"),
        (PageDimension::FillStretch, "fill"),
        (PageDimension::FilllStretch, "filll"),
    ] {
        let stretch = diagnostic.state().page_dimension(dimension);
        if stretch.raw() != 0 {
            diagnostic.print(" plus ");
            diagnostic.print_scaled(stretch);
            diagnostic.print(unit);
        }
    }
    let shrink = diagnostic.state().page_dimension(PageDimension::Shrink);
    if shrink.raw() != 0 {
        diagnostic.print(" minus ");
        diagnostic.print_scaled(shrink);
    }
}

/// tex.web §1006 prints `awful_bad` as `*` rather than as its numeric value.
fn print_cost(diagnostic: &mut Diagnostic<'_>, value: i32) {
    if value == AWFUL_BAD {
        diagnostic.print_char('*');
    } else {
        diagnostic.print_int(value);
    }
}

fn check_break(stores: &mut Universe, penalty: i32) -> Result<(), ExecError> {
    if penalty >= INF_PENALTY {
        return Ok(());
    }
    let badness = page_badness(stores)?;
    let mut cost = if badness < AWFUL_BAD {
        if penalty <= EJECT_PENALTY {
            penalty
        } else if badness < INF_BAD {
            badness
                .checked_add(penalty)
                .and_then(|value| value.checked_add(stores.insert_penalties()))
                .ok_or(ExecError::ArithmeticOverflow)?
        } else {
            DEPLORABLE
        }
    } else {
        badness
    };
    if stores.insert_penalties() >= INF_PENALTY {
        cost = AWFUL_BAD;
    }
    if stores.int_param(IntParam::TRACING_PAGES) > 0 {
        trace_page_break_cost(stores, badness, penalty, cost);
    }

    let break_index = stores.current_page_len();
    if cost <= stores.least_page_cost() {
        stores.record_best_page_break(
            break_index,
            stores.page_dimension(PageDimension::Goal),
            cost,
        );
    }
    if cost == AWFUL_BAD || penalty <= EJECT_PENALTY {
        stores.record_page_fire_up(break_index);
    }
    Ok(())
}

fn page_badness(stores: &Universe) -> Result<i32, ExecError> {
    let total = stores.page_dimension(PageDimension::Total);
    let goal = stores.page_dimension(PageDimension::Goal);
    if total < goal {
        if stores.page_dimension(PageDimension::FilStretch).raw() != 0
            || stores.page_dimension(PageDimension::FillStretch).raw() != 0
            || stores.page_dimension(PageDimension::FilllStretch).raw() != 0
        {
            Ok(0)
        } else {
            Ok(badness(
                sub(goal, total)?,
                stores.page_dimension(PageDimension::Stretch),
            ))
        }
    } else {
        let excess = sub(total, goal)?;
        if excess > stores.page_dimension(PageDimension::Shrink) {
            Ok(AWFUL_BAD)
        } else {
            Ok(badness(
                excess,
                stores.page_dimension(PageDimension::Shrink),
            ))
        }
    }
}

fn contribute_front(stores: &mut Universe) -> Result<(), ExecError> {
    ensure_max_depth(stores)?;
    if let Some(node) = stores.pop_page_contribution_front() {
        stores.push_current_page_node(node);
    }
    Ok(())
}

fn contribute_front_as(stores: &mut Universe, node: Node) -> Result<(), ExecError> {
    ensure_max_depth(stores)?;
    if stores.pop_page_contribution_front().is_some() {
        stores.update_page_last_from_node(&node);
        stores.push_current_page_node(node);
    }
    Ok(())
}

fn discard_front(stores: &mut Universe) {
    if let Some(node) = stores.pop_page_contribution_front()
        && stores.int_param(tex_state::env::banks::IntParam::SAVING_V_DISCARDS) > 0
    {
        stores.push_page_discard(node);
    }
}

fn ensure_max_depth(stores: &mut Universe) -> Result<(), ExecError> {
    let depth = stores.page_dimension(PageDimension::Depth);
    let max_depth = stores.page_max_depth();
    if depth > max_depth {
        let excess = sub(depth, max_depth)?;
        let total = add(stores.page_dimension(PageDimension::Total), excess)?;
        stores.set_page_dimension(PageDimension::Total, total);
        stores.set_page_dimension(PageDimension::Depth, max_depth);
    }
    Ok(())
}

fn precedes_break(node: &Node) -> bool {
    !matches!(
        node,
        Node::Glue { .. }
            | Node::Kern { .. }
            | Node::Penalty(_)
            | Node::MathOn(_)
            | Node::MathOff(_)
    )
}

fn vertical_height(node: &Node) -> Scaled {
    NodeRef::from(node)
        .vertical_dimensions()
        .map_or(Scaled::from_raw(0), |(height, _)| height)
}

fn vertical_depth(node: &Node) -> Scaled {
    NodeRef::from(node)
        .vertical_dimensions()
        .map_or(Scaled::from_raw(0), |(_, depth)| depth)
}

fn add(lhs: Scaled, rhs: Scaled) -> Result<Scaled, ExecError> {
    lhs.checked_add(rhs).ok_or(ExecError::ArithmeticOverflow)
}

fn sub(lhs: Scaled, rhs: Scaled) -> Result<Scaled, ExecError> {
    lhs.checked_sub(rhs).ok_or(ExecError::ArithmeticOverflow)
}

fn vertical_break_error(error: VerticalBreakError) -> ExecError {
    match error {
        VerticalBreakError::ArithmeticOverflow => ExecError::ArithmeticOverflow,
    }
}

#[cfg(test)]
mod tests;
