use tex_state::CommandContext;
use tex_state::diagnostic::DiagnosticEffects;
use tex_state::math::{MathField, MathNoad, NoadClass, NoadKind};
use tex_state::meaning::UnexpandablePrimitive;
use tex_state::node::{KernKind, Node};
use tex_state::scaled::Scaled;

use crate::vertical::{append_vertical_contribution, is_outer_vertical};

use super::append_node_to_current_list;
use crate::{ExecError, Mode, ModeNest};

use crate::box_runtime::first_box_node;
use crate::box_runtime::hmode::flush_pending_hchars;

pub(crate) fn execute_scanned_unbox_with_error_context<G>(
    primitive: UnexpandablePrimitive,
    index: u16,
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
    error_context: &str,
) -> Result<(), ExecError> {
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

fn execute_scanned_unbox_impl<G>(
    primitive: UnexpandablePrimitive,
    index: u16,
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
    error_context: &str,
) -> Result<(), ExecError> {
    let destructive = matches!(
        primitive,
        UnexpandablePrimitive::UnHBox | UnexpandablePrimitive::UnVBox
    );
    let Some(register) = stores.copy_box_to_page(index) else {
        return Ok(());
    };
    // TeX82 §1110 first returns for a void register, then refuses every
    // nonvoid box in math mode before testing its horizontal/vertical kind.
    // In particular, a matching hbox still cannot be opened in an mlist.
    if matches!(nest.current_mode(), Mode::Math | Mode::DisplayMath) {
        report_incompatible_unbox(stores, diagnostic_effects, error_context)?;
        return Ok(());
    }
    let Some(node) = first_box_node(stores, Some(register)) else {
        report_incompatible_unbox(stores, diagnostic_effects, error_context)?;
        return Ok(());
    };
    if !unbox_kind_matches(primitive, &node) {
        report_incompatible_unbox(stores, diagnostic_effects, error_context)?;
        return Ok(());
    }
    let children = match node {
        Node::HList(node) | Node::VList(node) => node.children,
        _ => unreachable!(),
    };
    if destructive {
        // The durable register closure was copied into page-lifetime storage
        // above. Clearing the dense register cell now changes only its TeX
        // equivalent at the existing level; the copied children remain owned
        // by the current page arena while they are spliced into the mode list.
        stores.clear_box_preserving_level(index);
    }
    append_unboxed(nest, stores, diagnostic_effects, Some(children), fuel)
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
    if is_outer_vertical(nest) {
        stores.append_page_contributions(nodes);
    } else {
        nest.current_list_mutation().append_list(stores, nodes);
    }
    Ok(())
}

pub(crate) fn execute_delete_last<G>(
    primitive: UnexpandablePrimitive,
    error_context: String,
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    flush_pending_hchars(nest, stores, diagnostic_effects, fuel)?;
    if is_outer_vertical(nest) {
        execute_delete_last_outer_vertical(primitive, &error_context, stores, diagnostic_effects)?;
        return Ok(());
    }
    let current_list = nest.current_list();
    let Some(tail) = crate::effective_tail::EffectiveTail::find(current_list.nodes(stores).iter())
    else {
        return Ok(());
    };
    let matches_target = matches!(
        (primitive, tail.node()),
        (UnexpandablePrimitive::UnSkip, Node::Glue { .. })
            | (UnexpandablePrimitive::UnPenalty, Node::Penalty(_))
            | (UnexpandablePrimitive::UnKern, Node::Kern { .. })
    );
    let range = tail.removal_range();
    drop(current_list);
    if matches_target {
        let _ = nest
            .current_list_mutation()
            .remove_node_range(stores, range);
    }
    Ok(())
}

fn execute_delete_last_outer_vertical<G>(
    primitive: UnexpandablePrimitive,
    error_context: &str,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
) -> Result<(), ExecError> {
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
            report_cannot_delete_from_page(primitive, error_context, stores, diagnostic_effects)?;
        }
        return Ok(());
    };
    let matches_target = matches!(
        (primitive, tail.node()),
        (UnexpandablePrimitive::UnSkip, Node::Glue { .. })
            | (UnexpandablePrimitive::UnPenalty, Node::Penalty(_))
            | (UnexpandablePrimitive::UnKern, Node::Kern { .. })
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
pub(crate) fn append_box_node_to_current_list<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    mut node: Node,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    let (pre_migrated, migrated) =
        if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
            extract_box_migrations(stores, &mut node)
        } else {
            (
                tex_state::node_arena::PageListId::empty(),
                tex_state::node_arena::PageListId::empty(),
            )
        };
    let node = if matches!(nest.current_mode(), Mode::Math | Mode::DisplayMath) {
        let nucleus = stores.publish_page_nodes(vec![node]);
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::SubBox(nucleus),
        ))
    } else {
        node
    };
    append_migration_list(nest, stores, pre_migrated);
    append_node_to_current_list(nest, stores, diagnostic_effects, node, fuel)?;
    append_migration_list(nest, stores, migrated);
    if matches!(
        nest.current_mode(),
        Mode::Horizontal | Mode::RestrictedHorizontal
    ) {
        nest.current_list_mutation().set_space_factor(1000);
    }
    Ok(())
}

fn extract_box_migrations<G>(
    stores: &mut CommandContext<'_, G>,
    node: &mut Node,
) -> (
    tex_state::node_arena::PageListId,
    tex_state::node_arena::PageListId,
) {
    let Node::HList(boxed) = node else {
        return (
            tex_state::node_arena::PageListId::empty(),
            tex_state::node_arena::PageListId::empty(),
        );
    };
    let children = boxed.children;
    let (retained, pre_migrated, migrated) = split_hpack_migrations(stores, children);
    if !pre_migrated.is_empty() || !migrated.is_empty() {
        boxed.children = retained;
    }
    (pre_migrated, migrated)
}

fn append_migration_list<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    nodes: tex_state::node_arena::PageListId,
) {
    if nodes.is_empty() {
        return;
    }
    if is_outer_vertical(nest) {
        stores.append_page_contributions(nodes);
    } else {
        nest.current_list_mutation().append_list(stores, nodes);
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
    nodes: tex_state::node_arena::PageListId,
) -> (
    tex_state::node_arena::PageListId,
    tex_state::node_arena::PageListId,
    tex_state::node_arena::PageListId,
) {
    let mut retained = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    let mut pre_migrated = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    let mut migrated = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut retained);
    stores.open_page_active_list(&mut pre_migrated);
    stores.open_page_active_list(&mut migrated);
    for index in 0..nodes.len() {
        let disposition = match stores
            .page_node_list(nodes)
            .expect("hpack source belongs to the live page arena")
            .nodes()
            .owned_node(index)
            .expect("hpack source index remains in range")
        {
            Node::Mark { .. } | Node::Ins { .. } => (2, None),
            Node::Adjust(adjust) => (usize::from(!adjust.pre) + 1, Some(adjust.content)),
            _ => (0, None),
        };
        let target = match disposition.0 {
            0 => &mut retained,
            1 => &mut pre_migrated,
            _ => &mut migrated,
        };
        if let Some(content) = disposition.1 {
            stores.append_page_active_list(target, content);
        } else {
            stores.append_page_active_list_range(target, nodes, index..index + 1);
        }
    }
    (
        stores.finalize_page_active_list(&mut retained),
        stores.finalize_page_active_list(&mut pre_migrated),
        stores.finalize_page_active_list(&mut migrated),
    )
}

fn append_unboxed<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    source: Option<tex_state::node_arena::PageListId>,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    let Some(children) = source else {
        return Ok(());
    };
    flush_pending_hchars(nest, stores, diagnostic_effects, fuel)?;
    // pdfTeX's margin-kern nodes are line-breaking annotations owned by the
    // containing packed line. Copying the box preserves them, but either
    // unboxing primitive removes them while splicing the remaining children;
    // the frozen source list itself must remain immutable for `\unhcopy`.
    let mut retained = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut retained);
    for index in 0..children.len() {
        let remove = stores
            .page_node_list(children)
            .expect("unboxed children belong to the live page arena")
            .nodes()
            .get(index)
            .is_some_and(|node| {
                matches!(
                    node,
                    tex_state::node_arena::NodeRef::MarginKern { .. }
                        | tex_state::node_arena::NodeRef::Kern {
                            kind: KernKind::LeftMargin | KernKind::RightMargin,
                            ..
                        }
                )
            });
        if !remove {
            stores.append_page_active_list_range(&mut retained, children, index..index + 1);
        }
    }
    let retained = stores.finalize_page_active_list(&mut retained);
    if is_outer_vertical(nest) {
        stores.append_page_contributions(retained);
    } else {
        nest.current_list_mutation().append_list(stores, retained);
    }
    Ok(())
}

fn unbox_kind_matches(primitive: UnexpandablePrimitive, node: &Node) -> bool {
    matches!(
        (primitive, node),
        (
            UnexpandablePrimitive::UnHBox | UnexpandablePrimitive::UnHCopy,
            Node::HList(_)
        ) | (
            UnexpandablePrimitive::UnVBox | UnexpandablePrimitive::UnVCopy,
            Node::VList(_)
        )
    )
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
