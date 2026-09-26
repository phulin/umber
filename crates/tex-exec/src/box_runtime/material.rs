use tex_state::CommandContext;
use tex_state::diagnostic::DiagnosticEffects;
use tex_state::math::{MathField, MathNoad, NoadClass, NoadKind};
use tex_state::meaning::UnexpandablePrimitive;
use tex_state::node::{KernKind, Node};
use tex_state::node_region::{PageBoxSegment, PageClosureBuildMark};
use tex_state::scaled::Scaled;

use crate::vertical::is_outer_vertical;

use super::append_node_to_current_list;
use crate::{ExecError, Mode, ModeNest};

use crate::box_runtime::hmode::flush_pending_hchars;

pub(crate) fn execute_scanned_unbox_with_error_context<G, F>(
    primitive: UnexpandablePrimitive,
    index: u16,
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
    error_context: F,
) -> Result<(), ExecError>
where
    F: FnOnce(&CommandContext<'_, G>) -> Result<String, ExecError>,
{
    execute_scanned_unbox_impl(
        primitive,
        index,
        nest,
        stores,
        diagnostic_effects,
        fuel,
        error_context,
    )
}

fn execute_scanned_unbox_impl<G, F>(
    primitive: UnexpandablePrimitive,
    index: u16,
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
    error_context: F,
) -> Result<(), ExecError>
where
    F: FnOnce(&CommandContext<'_, G>) -> Result<String, ExecError>,
{
    let destructive = matches!(
        primitive,
        UnexpandablePrimitive::UnHBox | UnexpandablePrimitive::UnVBox
    );
    if stores.box_register(index).is_none() {
        return Ok(());
    }
    // TeX82 §1110 first returns for a void register, then refuses every
    // nonvoid box in math mode before testing its horizontal/vertical kind.
    // In particular, a matching hbox still cannot be opened in an mlist.
    if matches!(nest.current_mode(), Mode::Math | Mode::DisplayMath) {
        let error_context = error_context(stores)?;
        report_incompatible_unbox(stores, diagnostic_effects, &error_context)?;
        return Ok(());
    }
    let expected_kind = match primitive {
        UnexpandablePrimitive::UnHBox | UnexpandablePrimitive::UnHCopy => {
            tex_state::CommandBoxKind::Horizontal
        }
        UnexpandablePrimitive::UnVBox | UnexpandablePrimitive::UnVCopy => {
            tex_state::CommandBoxKind::Vertical
        }
        _ => unreachable!("caller restricts unbox primitives"),
    };
    if stores.box_kind(index) != Some(expected_kind) {
        let error_context = error_context(stores)?;
        report_incompatible_unbox(stores, diagnostic_effects, &error_context)?;
        return Ok(());
    }
    let register = if destructive {
        stores.take_box_to_page(index)
    } else {
        stores.copy_box_to_page(index)
    }
    .expect("admitted box register remains live");
    let children = stores.consumed_box_children(register);
    append_unboxed(nest, stores, diagnostic_effects, children, fuel)
}

/// Splices one of e-TeX 2.6 `etex.ch` [45.999]'s saved vertical-discard
/// lists into the current list.
///
/// The primitive shares TeX82's `un_vbox` command code, but its modifier is
/// above `copy_code`, so `unpackage` takes this operand-free branch before
/// scanning a register and clears the saved-list pointer as it detaches it.
pub(crate) fn execute_scanned_saved_vertical_discards<G>(
    primitive: UnexpandablePrimitive,
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    let nodes = match primitive {
        UnexpandablePrimitive::PageDiscards => stores.take_page_discards(),
        UnexpandablePrimitive::SplitDiscards => stores.take_split_discards(),
        _ => unreachable!("caller restricts saved vertical-discard primitives"),
    };
    flush_pending_hchars(nest, stores, diagnostic_effects, fuel)?;
    let nodes = stores.reclaim_unique_page_list(nodes);
    if is_outer_vertical(nest) {
        stores.append_unique_page_contributions(nodes);
    } else {
        nest.current_list_mutation()
            .append_unique_list(stores, nodes);
    }
    Ok(())
}

pub(crate) fn execute_delete_last<G, F>(
    primitive: UnexpandablePrimitive,
    error_context: F,
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError>
where
    F: FnOnce(&CommandContext<'_, G>) -> Result<String, ExecError>,
{
    flush_pending_hchars(nest, stores, diagnostic_effects, fuel)?;
    if is_outer_vertical(nest) {
        execute_delete_last_outer_vertical(primitive, error_context, stores, diagnostic_effects)?;
        return Ok(());
    }
    let current_list = nest.current_list();
    let Some(tail) = crate::effective_tail::EffectiveTail::find(current_list.nodes(stores).iter())
    else {
        return Ok(());
    };
    let matches_target = matches!(
        (primitive, tail.node()),
        (
            UnexpandablePrimitive::UnSkip,
            tex_state::NodeView::Glue { .. }
        ) | (
            UnexpandablePrimitive::UnPenalty,
            tex_state::NodeView::Penalty(_)
        ) | (
            UnexpandablePrimitive::UnKern,
            tex_state::NodeView::Kern { .. }
        )
    );
    let range = tail.removal_range();
    let _ = current_list;
    if matches_target {
        let _ = nest
            .current_list_mutation()
            .remove_node_range(stores, range);
    }
    Ok(())
}

fn execute_delete_last_outer_vertical<G, F>(
    primitive: UnexpandablePrimitive,
    error_context: F,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
) -> Result<(), ExecError>
where
    F: FnOnce(&CommandContext<'_, G>) -> Result<String, ExecError>,
{
    let Some(tail) = crate::effective_tail::EffectiveTail::find(stores.page_contributions().iter())
    else {
        // TeX82 §1105: `(mode=vmode)and(tail=head)` -- the contribution list
        // is empty because `build_page` has already swept every prior item
        // onto the current page, whose cost accounting can no longer be
        // undone. Nothing is ever structurally removed in this branch: only
        // the diagnostic differs. `\unpenalty`/`\unkern` always apologize
        // ("Sorry...I usually can't take things from the current page.").
        // `\unskip` apologizes only when the page builder's own `last_glue`
        // memo (§996) shows the most recently placed page item really was
        // glue; otherwise it is `\unskip` "following non-glue" and silently
        // succeeds, matching the one case tex.web exempts from the apology.
        if primitive != UnexpandablePrimitive::UnSkip || stores.page_has_last_glue() {
            let error_context = error_context(stores)?;
            report_cannot_delete_from_page(primitive, &error_context, stores, diagnostic_effects)?;
        }
        return Ok(());
    };
    let matches_target = matches!(
        (primitive, tail.node()),
        (
            UnexpandablePrimitive::UnSkip,
            tex_state::NodeView::Glue { .. }
        ) | (
            UnexpandablePrimitive::UnPenalty,
            tex_state::NodeView::Penalty(_)
        ) | (
            UnexpandablePrimitive::UnKern,
            tex_state::NodeView::Kern { .. }
        )
    );
    if matches_target {
        let range = tail.removal_range();
        let _ = stores.remove_page_contribution_range(range);
    }
    Ok(())
}

/// TeX82 §1105's recoverable
/// `@<Apologize for inability to do the operation now...@>` error.
///
/// This must not escape as an `ExecError`: tex.web calls `error` and resumes
/// main control with the following token. The final help line is selected by
/// the requested node type.
fn report_cannot_delete_from_page<G>(
    primitive: UnexpandablePrimitive,
    error_context: &str,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
) -> Result<(), ExecError> {
    let command = match primitive {
        UnexpandablePrimitive::UnSkip => "unskip",
        UnexpandablePrimitive::UnKern => "unkern",
        UnexpandablePrimitive::UnPenalty => "unpenalty",
        _ => unreachable!("caller restricts delete_last primitives"),
    };
    let last_help = match primitive {
        UnexpandablePrimitive::UnSkip => "Try `I\\vskip-\\lastskip' instead.",
        UnexpandablePrimitive::UnKern => "Try `I\\kern-\\lastkern' instead.",
        UnexpandablePrimitive::UnPenalty => "Perhaps you can make the output routine do it.",
        _ => unreachable!("caller restricts delete_last primitives"),
    };
    let mut report = stores.print_err("You can't use `");
    report
        .print_esc(command)
        .print("' in vertical mode")
        .help(&[
            "Sorry...I usually can't take things from the current page.",
            last_help,
        ])
        .context(error_context.to_owned());
    report.error().defer_recovery(diagnostic_effects)?;
    Ok(())
}

/// TeX82 §1076, `<Append box |cur_box| to the current list, shifted by
/// |box_context|>`, the branch §1075's `box_end` takes for every
/// non-register, non-`\shipout`, non-leader box: `\hbox`/`\vbox`/`\vtop`,
/// `\vsplit`, `\box`/`\copy`, `\lastbox`, and the `\raise`/`\lower`/
/// `\moveleft`/`\moveright` shifts of those.
///
/// The module has three mode branches, not two:
///
/// ```text
/// if abs(mode)=vmode then begin append_to_vlist(cur_box); ... end
/// else begin if abs(mode)=hmode then space_factor:=1000
///   else begin p:=new_noad; math_type(nucleus(p)):=sub_box;
///     info(nucleus(p)):=cur_box; cur_box:=p;
///     end;
///   link(tail):=cur_box; tail:=cur_box;
///   end
/// ```
///
/// In math mode the box is never linked into the mlist directly: it becomes
/// the `sub_box` nucleus of a fresh ordinary noad. That wrapper is what makes
/// the box visible to §727's `check_dimensions`, which updates `max_h`/`max_d`
/// only from noads -- §726 sends a bare `hlist_node`/`vlist_node` straight to
/// `done_with_node`. §762's `make_left_right` derives its `\left`/`\right`
/// delimiter target from exactly those `max_h`/`max_d`, so an unwrapped box
/// silently shrank the target and §706's `var_delimiter` returned the
/// smallest variant instead of the size the box calls for.
pub(crate) struct PreparedBoxAppend {
    node: Option<Node>,
    pre_migrated: tex_state::page_node_arena::PageListId,
    migrated: tex_state::page_node_arena::PageListId,
    obsolete_input_positions: Vec<usize>,
    migrated_projection_segment: Option<PageBoxSegment>,
}

/// Materializes hpack's three list projections before a caller seals the
/// final wrapper chunk. A split can copy partial boundary records; those
/// copies must remain in the body interval, not in the wrapper's chunk.
pub(crate) fn prepare_box_append<G>(
    mode: Mode,
    stores: &mut CommandContext<'_, G>,
    mut node: Node,
) -> PreparedBoxAppend {
    let (pre_migrated, migrated, obsolete_input_positions, migrated_projection_segment) =
        if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
            extract_box_migrations(stores, &mut node)
        } else {
            (
                tex_state::page_node_arena::PageListId::empty(),
                tex_state::page_node_arena::PageListId::empty(),
                Vec::new(),
                None,
            )
        };
    let node = if matches!(mode, Mode::Math | Mode::DisplayMath) {
        let nucleus = stores.publish_page_nodes(vec![node]);
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::SubBox(nucleus),
        ))
    } else {
        node
    };
    PreparedBoxAppend {
        node: Some(node),
        pre_migrated,
        migrated,
        obsolete_input_positions,
        migrated_projection_segment,
    }
}

pub(crate) fn append_prepared_box_pre_migrations<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    prepared: &mut PreparedBoxAppend,
) {
    append_migration_list(nest, stores, std::mem::take(&mut prepared.pre_migrated));
}

pub(crate) fn append_prepared_box_baseline_glue<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    prepared: &PreparedBoxAppend,
) -> Result<(), ExecError> {
    crate::vertical::append_vertical_baseline_glue(
        nest,
        stores,
        prepared
            .node
            .as_ref()
            .expect("prepared wrapper remains live"),
    )
}

pub(crate) fn append_prepared_box_wrapper<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    prepared: &mut PreparedBoxAppend,
    baseline_prepared: bool,
    fuel: &mut tex_command::CommandFuel,
) -> Result<Option<tex_state::page_node_arena::PageListId>, ExecError> {
    let node = prepared
        .node
        .take()
        .expect("prepared box wrapper is appended once");
    if baseline_prepared {
        crate::vertical::append_vertical_node_after_baseline(nest, stores, node);
    } else {
        append_node_to_current_list(nest, stores, diagnostic_effects, node, fuel)?;
    }
    // Pre-adjustments precede this wrapper, whereas insertions and ordinary
    // adjustments follow it. Capture the wrapper before appending that
    // trailing material: the list tail afterward need not be the box.
    let wrapper = (!is_outer_vertical(nest))
        .then(|| nest.current_list().last_node_root(stores))
        .flatten();
    Ok(wrapper)
}

pub(crate) fn append_prepared_box_post_migrations<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    prepared: PreparedBoxAppend,
) {
    append_migration_list(nest, stores, prepared.migrated);
    if matches!(
        nest.current_mode(),
        Mode::Horizontal | Mode::RestrictedHorizontal
    ) {
        nest.current_list_mutation().set_space_factor(1000);
    }
}

pub(crate) fn append_box_node_to_current_list<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    node: Node,
    fuel: &mut tex_command::CommandFuel,
) -> Result<Option<tex_state::page_node_arena::PageListId>, ExecError> {
    let mut prepared = prepare_box_append(nest.current_mode(), stores, node);
    append_prepared_box_pre_migrations(nest, stores, &mut prepared);
    let wrapper =
        append_prepared_box_wrapper(nest, stores, diagnostic_effects, &mut prepared, false, fuel)?;
    append_prepared_box_post_migrations(nest, stores, prepared);
    Ok(wrapper)
}

/// Publishes a uniquely constructed box and its page-owned splices as
/// separate sealed intervals. The wrapper is last in the stamped envelope;
/// earlier baseline glue and pre-adjustments stay with the parent list.
pub(crate) fn append_box_node_with_segment<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
    node: Node,
    start: PageClosureBuildMark,
    mut exclusions: Vec<PageBoxSegment>,
) -> Result<(), ExecError> {
    let mut prepared = prepare_box_append(nest.current_mode(), stores, node);
    let vertical = matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical);
    if let Some(segment) = prepared.migrated_projection_segment
        && !segment.is_empty()
    {
        exclusions.push(segment);
    }
    let pre_append_start = vertical.then(|| stores.begin_page_node_region());
    append_prepared_box_pre_migrations(nest, stores, &mut prepared);
    if vertical {
        append_prepared_box_baseline_glue(nest, stores, &prepared)?;
    }
    if let Some(mark) = pre_append_start {
        let segment = stores.close_page_box_segment(mark);
        if !segment.is_empty() {
            exclusions.push(segment);
        }
    }
    let migration_key =
        stores.publish_page_box_migration_segments(&exclusions, &prepared.obsolete_input_positions);
    stores.rotate_page_box_wrapper_tail();
    let root = append_prepared_box_wrapper(
        nest,
        stores,
        diagnostic_effects,
        &mut prepared,
        vertical,
        fuel,
    )?
    .expect("constructed box appended one original wrapper");
    let stamped = stores.stamp_page_box_segment(&start, root, migration_key);
    let sealed = stores.close_page_box_segment(start);
    assert_eq!(stamped, sealed, "wrapper stamp matches sealed bounds");
    append_prepared_box_post_migrations(nest, stores, prepared);
    Ok(())
}

fn extract_box_migrations<G>(
    stores: &mut CommandContext<'_, G>,
    node: &mut Node,
) -> (
    tex_state::page_node_arena::PageListId,
    tex_state::page_node_arena::PageListId,
    Vec<usize>,
    Option<PageBoxSegment>,
) {
    let Node::HList(boxed) = node else {
        return (
            tex_state::page_node_arena::PageListId::empty(),
            tex_state::page_node_arena::PageListId::empty(),
            Vec::new(),
            None,
        );
    };
    let children = boxed.children;
    let migrates = stores
        .page_node_list(children)
        .expect("hpack source belongs to the live page arena")
        .nodes()
        .iter()
        .any(|node| {
            matches!(
                node,
                tex_state::node_view::NodeView::Mark { .. }
                    | tex_state::node_view::NodeView::Ins { .. }
                    | tex_state::node_view::NodeView::Adjust(_)
            )
        });
    let obsolete_input_positions = if migrates {
        let positions = stores
            .page_list_chunk_positions(children)
            .expect("consumed hpack input list has exact chunk positions");
        stores.rotate_page_box_wrapper_tail();
        positions
    } else {
        Vec::new()
    };
    let (retained, pre_migrated, migrated, projection) = split_hpack_migrations(stores, children);
    if !pre_migrated.is_empty() || !migrated.is_empty() {
        boxed.children = retained;
    }
    (
        pre_migrated,
        migrated,
        obsolete_input_positions,
        Some(projection),
    )
}

fn append_migration_list<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    nodes: tex_state::page_node_arena::PageListId,
) {
    if nodes.is_empty() {
        return;
    }
    let nodes = stores.reclaim_unique_page_list(nodes);
    if is_outer_vertical(nest) {
        stores.append_unique_page_contributions(nodes);
    } else {
        nest.current_list_mutation()
            .append_unique_list(stores, nodes);
    }
}

/// Performs TeX82 §647's `adjust_tail` split for one horizontal list.
///
/// §651 sends every `ins_node`, `mark_node`, and `adjust_node` of a list being
/// `hpack`ed to §655, which moves an insertion or a mark node itself onto the
/// adjustment list but splices only an adjustment's *contents*
/// (`link(adjust_tail):=adjust_ptr(p)`) and frees the `\vadjust` node. Every
/// caller that packs a horizontal list with `adjust_tail` non-null -- §1076's
/// `\hbox` contribution to a vertical list, §796's alignment column -- performs
/// exactly this split, and differs only in where the migrated material lands.
pub(crate) fn split_hpack_migrations<G>(
    stores: &mut CommandContext<'_, G>,
    nodes: tex_state::page_node_arena::PageListId,
) -> (
    tex_state::page_node_arena::PageListId,
    tex_state::page_node_arena::PageListId,
    tex_state::page_node_arena::PageListId,
    PageBoxSegment,
) {
    fn select<G>(
        stores: &mut CommandContext<'_, G>,
        nodes: tex_state::page_node_arena::PageListId,
        selected_class: usize,
    ) -> tex_state::page_node_arena::PageListId {
        let mut output = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
        stores.open_page_active_list(&mut output);
        for index in 0..nodes.len() {
            let (class, adjustment) = match stores
                .page_node_list(nodes)
                .expect("hpack source belongs to the live page arena")
                .nodes()
                .get(index)
                .expect("hpack source index remains in range")
            {
                tex_state::node_view::NodeView::Mark { .. }
                | tex_state::node_view::NodeView::Ins { .. } => (2, None),
                tex_state::node_view::NodeView::Adjust(adjust) => {
                    (usize::from(!adjust.pre) + 1, Some(adjust.content))
                }
                _ => (0, None),
            };
            if class != selected_class {
                continue;
            }
            if let Some(content) = adjustment {
                stores.append_page_active_list(&mut output, content);
            } else {
                stores.append_page_active_list_range(&mut output, nodes, index..index + 1);
            }
        }
        stores.finalize_page_active_list(&mut output)
    }

    // A ForkArena lane admits exactly one persistent builder. Build the three
    // disjoint zero-copy projections sequentially so no partial operation
    // coordinates overlap.
    let retained = select(stores, nodes, 0);
    let migration_projection_start = stores.begin_page_node_region();
    let pre_migrated = select(stores, nodes, 1);
    let migrated = select(stores, nodes, 2);
    let projection = stores.close_page_box_segment(migration_projection_start);
    (retained, pre_migrated, migrated, projection)
}

fn append_unboxed<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    source: tex_state::page_node_arena::ConsumedBoxChildren,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    let children = source.list();
    flush_pending_hchars(nest, stores, diagnostic_effects, fuel)?;
    // pdfTeX's margin-kern nodes are line-breaking annotations owned by the
    // containing packed line. Copying the box preserves them, but either
    // unboxing primitive removes them while splicing the remaining children;
    // the frozen source list itself must remain immutable for `\unhcopy`.
    let has_margin_kern = stores
        .page_node_list(children)
        .expect("unboxed children belong to the live page arena")
        .nodes()
        .iter()
        .any(|node| {
            matches!(
                node,
                tex_state::node_view::NodeView::MarginKern { .. }
                    | tex_state::node_view::NodeView::Kern {
                        kind: KernKind::LeftMargin | KernKind::RightMargin,
                        ..
                    }
            )
        });
    if !has_margin_kern
        && let Some(retained) = stores
            .reclaim_unlinked_page_list(children)
            .expect("unboxed children belong to the admitted page owner")
    {
        if is_outer_vertical(nest) {
            stores.append_unique_page_contributions(retained);
        } else {
            nest.current_list_mutation()
                .append_unique_list(stores, retained);
        }
        return Ok(());
    }
    let retained = stores.project_consumed_box_children(source, has_margin_kern);
    if is_outer_vertical(nest) {
        stores.append_unique_page_contributions(retained);
    } else {
        nest.current_list_mutation()
            .append_unique_list(stores, retained);
    }
    Ok(())
}

/// TeX.web §1110's `unpackage` refusal, which leaves the register alone.
///
/// The completed register scan owns the live §82 context for this command.
fn report_incompatible_unbox<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    error_context: &str,
) -> Result<(), ExecError> {
    crate::error_report::report_error(
        stores,
        diagnostic_effects,
        "Incompatible list can't be unboxed",
        &[
            "Sorry, Pandora. (You sneaky devil.)",
            "I refuse to unbox an \\hbox in vertical mode or vice versa.",
            "And I can't open any boxes in math mode.",
        ],
        error_context.to_owned(),
    )?;
    Ok(())
}

pub(crate) fn apply_box_shift_delta(node: &mut Node, delta: Scaled) -> Result<(), ExecError> {
    let box_node = match node {
        Node::HList(box_node) | Node::VList(box_node) => box_node,
        _ => return Err(ExecError::MissingToken { context: "box" }),
    };
    box_node.shift = box_node
        .shift
        .checked_add(delta)
        .ok_or(ExecError::ArithmeticOverflow)?;
    Ok(())
}
