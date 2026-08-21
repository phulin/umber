use tex_state::CommandContext;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::node::{BoxNode, GlueKind, KernKind, Node, Sign};
use tex_state::scaled::Scaled;
use tex_typeset::PackSpec;
use tex_typeset::math::{MathParams, Style};

use crate::box_runtime::{hpack_with_overfull_rule, split_hpack_migrations};
use crate::mode::EqNoSide;
use crate::packing_params::{hpack as hpack_nodes, hpack_params};
use crate::vertical::{append_node_to_vertical_list, append_vertical_contribution};
use crate::{ExecError, Mode, ModeNest};

fn scaled_add(left: Scaled, right: Scaled) -> Scaled {
    left.checked_add(right)
        .expect("display-math scaled addition overflow")
}

fn scaled_sub(left: Scaled, right: Scaled) -> Scaled {
    left.checked_sub(right)
        .expect("display-math scaled subtraction overflow")
}

fn scaled_mul(factor: i32, value: Scaled) -> Scaled {
    let product = i64::from(factor) * i64::from(value.raw());
    Scaled::from_raw(i32::try_from(product).expect("display-math scaled multiplication overflow"))
}

fn glue_parameter_value<G>(stores: &CommandContext<'_, G>, parameter: GlueParam) -> GlueSpec {
    stores
        .glue_param(parameter)
        .map_or(GlueSpec::ZERO, |id| stores.glue(id))
}

use super::lower::{MathConversionErrorContext, convert_math_hlist_with_error_context};

mod prototype;
#[cfg(test)]
mod tests;

pub(crate) use prototype::display_line_prototype;
use prototype::package_directed_display_line;

pub(crate) struct FinishedEqNo {
    pub side: EqNoSide,
    pub boxed: BoxNode,
}

pub(crate) fn finish_eq_no<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    diagnostic_context: &crate::pack_report::ExecutionDiagnosticContext,
    side: EqNoSide,
    content: tex_state::node_arena::PageListId,
    error_context: Option<&MathConversionErrorContext>,
) -> FinishedEqNo {
    let params = MathParams::read(&crate::typeset_context::TypesetContext::new(stores));
    let nodes = convert_math_hlist_with_error_context(
        stores,
        content,
        Style::TEXT,
        false,
        &params,
        error_context,
    );
    let list = stores.publish_page_nodes(nodes);
    let mut boxed = hpack_nodes(
        stores,
        diagnostic_effects,
        diagnostic_context,
        list,
        PackSpec::Natural,
        hpack_params(stores),
    )
    .node;
    boxed.box_lr = tex_state::node::BoxLr::DList;
    FinishedEqNo { side, boxed }
}

pub(crate) fn finish_display_math<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    diagnostic_context: &crate::pack_report::ExecutionDiagnosticContext,
    content: tex_state::node_arena::PageListId,
    eq_no: Option<FinishedEqNo>,
    prototype: Option<BoxNode>,
    error_context: Option<&MathConversionErrorContext>,
) -> Result<(), ExecError> {
    let (display_content, mut eq_box, left_eq_no) = match eq_no {
        Some(eq_no) => (content, Some(eq_no.boxed), eq_no.side == EqNoSide::Left),
        None => (content, None, false),
    };
    // AppG rule 22
    let params = MathParams::read(&crate::typeset_context::TypesetContext::new(stores));
    let display_nodes = convert_math_hlist_with_error_context(
        stores,
        display_content,
        Style::DISPLAY,
        false,
        &params,
        error_context,
    );
    // TeX82 §1196 sets `adjust_tail:=adjust_head` before the display's
    // `hpack`. Consequently §§651/655 remove insertions, marks, and
    // adjustments before §663's `short_display` examines an overfull
    // formula. Keep the migrated material beside the display instead of
    // leaving zero-dimensional wrappers inside its packed hlist.
    let (display_nodes, pre_migrated, migrated) = split_hpack_migrations(stores, display_nodes);
    let shrink = hlist_shrink(stores, &display_nodes);
    let display_starts_with_glue = display_nodes
        .first()
        .is_some_and(|node| matches!(node, Node::Glue { .. }));
    let display_list = stores.publish_page_nodes(display_nodes);
    let mut display_box = hpack_nodes(
        stores,
        diagnostic_effects,
        diagnostic_context,
        display_list.clone(),
        PackSpec::Natural,
        hpack_params(stores),
    )
    .node;
    display_box.box_lr = tex_state::node::BoxLr::DList;
    let natural_display_width = display_box.width;

    // TeX.web after_math variables: w=display width, z=line width, s=line indent,
    // e=eqno width, q=eqno width plus math quad, d=center displacement.
    // The display parameters are ordinary scoped assignments while the display
    // is being scanned, so read their finish-time values and use the interrupt
    // record only to restore the enclosing state afterward.
    let z = stores.dimen_param(DimenParam::DISPLAY_WIDTH);
    let s = stores.dimen_param(DimenParam::DISPLAY_INDENT);
    let pre_display_size = stores.dimen_param(DimenParam::PRE_DISPLAY_SIZE);
    let mut w = natural_display_width;
    let mut e = eq_box
        .as_ref()
        .map_or(Scaled::from_raw(0), |boxed| boxed.width);
    let q = if eq_box.is_some() {
        e + params.text.symbols.math_quad
    } else {
        Scaled::from_raw(0)
    };

    if scaled_add(w, q) > z {
        if e.raw() != 0 && display_can_shrink_with_eqno(w, q, z, shrink) {
            display_box = hpack_nodes(
                stores,
                diagnostic_effects,
                diagnostic_context,
                display_list,
                PackSpec::Exactly(z - q),
                hpack_params(stores),
            )
            .node;
            display_box.box_lr = tex_state::node::BoxLr::DList;
        } else {
            e = Scaled::from_raw(0);
            if w > z {
                display_box = hpack_with_overfull_rule(
                    stores,
                    diagnostic_effects,
                    diagnostic_context,
                    display_list,
                    PackSpec::Exactly(z),
                );
                display_box.box_lr = tex_state::node::BoxLr::DList;
            }
        }
        w = display_box.width;
    }

    let mut d = Scaled::from_raw(tex_half(scaled_sub(z, w).raw()));
    if e.raw() > 0 && d < scaled_mul(2, e) {
        d = Scaled::from_raw(tex_half(scaled_sub(scaled_sub(z, w), e).raw()));
        if display_starts_with_glue {
            d = Scaled::from_raw(0);
        }
    }

    append_vertical_contribution(
        nest,
        stores,
        Node::Penalty(stores.int_param(IntParam::PRE_DISPLAY_PENALTY)),
    );
    let mut above = GlueParam::ABOVE_DISPLAY_SKIP;
    let mut below = Some(GlueParam::BELOW_DISPLAY_SKIP);
    if scaled_add(d, s) > pre_display_size && !left_eq_no {
        above = GlueParam::ABOVE_DISPLAY_SHORT_SKIP;
        below = Some(GlueParam::BELOW_DISPLAY_SHORT_SKIP);
    }

    if left_eq_no && e.raw() == 0 {
        if let Some(mut boxed) = eq_box.take() {
            boxed.shift = s;
            append_node_to_vertical_list(nest, stores, Node::HList(boxed))?;
            append_vertical_contribution(nest, stores, Node::Penalty(10_000));
        }
    } else {
        let spec = glue_parameter_value(stores, above);
        append_vertical_contribution(
            nest,
            stores,
            Node::Glue {
                spec,
                kind: above_display_glue_kind(above),
                leader: None,
            },
        );
    }

    let mut display_line = display_box;
    if e.raw() != 0
        && let Some(eq_box) = eq_box.take()
    {
        let kern = Node::Kern {
            amount: scaled_sub(scaled_sub(scaled_sub(z, w), e), d),
            kind: KernKind::Font,
        };
        let children = if left_eq_no {
            d = Scaled::from_raw(0);
            vec![Node::HList(eq_box), kern, Node::HList(display_line)]
        } else {
            vec![Node::HList(display_line), kern, Node::HList(eq_box)]
        };
        let list = stores.publish_page_nodes(children);
        display_line = hpack_nodes(
            stores,
            diagnostic_effects,
            diagnostic_context,
            list,
            PackSpec::Natural,
            hpack_params(stores),
        )
        .node;
    }
    let pre_display_direction = stores.int_param(IntParam::PRE_DISPLAY_DIRECTION);
    if pre_display_direction == 0 {
        display_line.shift = s + d;
    } else {
        display_line = package_directed_display_line(
            stores,
            diagnostic_effects,
            diagnostic_context,
            display_line,
            prototype,
            d,
            s,
            z,
            pre_display_direction,
        );
    }
    for node in pre_migrated {
        append_vertical_contribution(nest, stores, node);
    }
    append_node_to_vertical_list(nest, stores, Node::HList(display_line))?;

    if let Some(mut boxed) = eq_box
        && e.raw() == 0
        && !left_eq_no
    {
        append_vertical_contribution(nest, stores, Node::Penalty(10_000));
        boxed.shift = s + z - boxed.width;
        append_node_to_vertical_list(nest, stores, Node::HList(boxed))?;
        below = None;
    }

    // TeX82 §1196 keeps `adjust_tail` live until the display and any
    // separately stacked equation number have both reached the vertical
    // list. Only then does it splice the migrated insertion/mark/adjustment
    // tail ahead of the post-display penalty. Appending it immediately after
    // the formula exposes adjustment penalties as page-break candidates
    // before a non-fitting equation-number line contributes its height.
    for node in migrated {
        append_vertical_contribution(nest, stores, node);
    }

    append_vertical_contribution(
        nest,
        stores,
        Node::Penalty(stores.int_param(IntParam::POST_DISPLAY_PENALTY)),
    );
    if let Some(below) = below {
        let spec = glue_parameter_value(stores, below);
        append_vertical_contribution(
            nest,
            stores,
            Node::Glue {
                spec,
                kind: below_display_glue_kind(below),
                leader: None,
            },
        );
    }

    Ok(())
}

pub(crate) fn finish_display_alignment<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    finished: crate::align::FinishedAlignment,
) -> Result<(), ExecError> {
    append_vertical_contribution(
        nest,
        stores,
        Node::Penalty(stores.int_param(IntParam::PRE_DISPLAY_PENALTY)),
    );

    let above = GlueParam::ABOVE_DISPLAY_SKIP;
    let spec = glue_parameter_value(stores, above);
    append_vertical_contribution(
        nest,
        stores,
        Node::Glue {
            spec,
            kind: above_display_glue_kind(above),
            leader: None,
        },
    );

    // TeX82 §1206 splices §812's already-finished `(p,q)` list directly.
    // `append_vertical_contribution` is that direct tail/page-contribution
    // router; unlike `append_node_to_vertical_list`, it inserts no baseline
    // glue around the rows §799 already separated.
    for node in finished.nodes {
        append_vertical_contribution(nest, stores, display_alignment_node(node));
    }
    if let Some(prev_depth) = finished.aux_prev_depth {
        nest.current_list_mutation().set_prev_depth(prev_depth);
    }

    append_vertical_contribution(
        nest,
        stores,
        Node::Penalty(stores.int_param(IntParam::POST_DISPLAY_PENALTY)),
    );
    let spec = glue_parameter_value(stores, GlueParam::BELOW_DISPLAY_SKIP);
    append_vertical_contribution(
        nest,
        stores,
        Node::Glue {
            spec,
            kind: GlueKind::BelowDisplaySkip,
            leader: None,
        },
    );

    Ok(())
}

/// Marks a finished display alignment's boxes as display material.
///
/// The `\displayindent` shift is *not* applied here: TeX82 §800 decides it
/// once as `o` and §806/§807 apply it while the unset boxes and running rules
/// are being set, which is the only place a rule -- a node with no
/// `shift_amount` field -- can receive it at all.
fn display_alignment_node(mut node: Node) -> Node {
    if let Node::HList(box_node) | Node::VList(box_node) = &mut node {
        box_node.box_lr = tex_state::node::BoxLr::DList;
    }
    node
}

fn above_display_glue_kind(param: GlueParam) -> GlueKind {
    if param == GlueParam::ABOVE_DISPLAY_SHORT_SKIP {
        GlueKind::AboveDisplayShortSkip
    } else {
        GlueKind::AboveDisplaySkip
    }
}

fn below_display_glue_kind(param: GlueParam) -> GlueKind {
    if param == GlueParam::BELOW_DISPLAY_SHORT_SKIP {
        GlueKind::BelowDisplayShortSkip
    } else {
        GlueKind::BelowDisplaySkip
    }
}

const fn tex_half(x: i32) -> i32 {
    if x % 2 != 0 && x > 0 {
        x / 2 + 1
    } else {
        x / 2
    }
}

pub(crate) fn build_page_after_display_resume<G>(
    nest: &ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    error_context: &str,
) -> Result<(), ExecError> {
    // tex.web §1200's closing `if nest_ptr=1 then build_page`. `nest_ptr` is
    // the number of levels pushed above the outermost vertical list, so the
    // test is satisfied by exactly the horizontal level `resume_after_display`
    // has just pushed directly above outer vertical mode -- Umber's
    // `depth()==2` with that level current. `build_page_if_outer_vertical`
    // alone can never fire here, because the current mode is always the
    // freshly pushed horizontal one; using it would miss the just-appended
    // display penalties and defer a forced break until unrelated later
    // material.
    if nest.depth() == 2 && nest.current_mode() == Mode::Horizontal {
        crate::page_builder::build_page_with_error_context(
            stores,
            diagnostic_effects,
            error_context,
        )
    } else {
        crate::vertical::build_page_if_outer_vertical_with_error_context(
            nest,
            stores,
            diagnostic_effects,
            error_context,
        )
    }
}

fn display_can_shrink_with_eqno(w: Scaled, q: Scaled, z: Scaled, shrink: ShrinkTotals) -> bool {
    scaled_add(scaled_sub(w, shrink.normal), q) <= z
        || shrink.fil.raw() != 0
        || shrink.fill.raw() != 0
        || shrink.filll.raw() != 0
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ShrinkTotals {
    normal: Scaled,
    fil: Scaled,
    fill: Scaled,
    filll: Scaled,
}

fn hlist_shrink<G>(stores: &CommandContext<'_, G>, nodes: &[Node]) -> ShrinkTotals {
    let mut totals = [Scaled::from_raw(0); 4];
    for node in nodes {
        if let Node::Glue { spec, .. } = node {
            let glue = spec;
            totals[glue.shrink_order as usize] = totals[glue.shrink_order as usize] + glue.shrink;
        }
    }
    ShrinkTotals {
        normal: totals[Order::Normal as usize],
        fil: totals[Order::Fil as usize],
        fill: totals[Order::Fill as usize],
        filll: totals[Order::Filll as usize],
    }
}

pub(crate) fn pre_display_size<G>(stores: &CommandContext<'_, G>, line: &BoxNode) -> Scaled {
    let quad = stores.font_parameter(stores.current_font(), 6);
    let mut v = line.shift + quad + quad;
    let mut w = Scaled::from_raw(-Scaled::MAX_DIMEN.raw());
    for node in stores
        .page_node_list(line.children)
        .expect("display line belongs to the live page arena")
        .iter()
    {
        let (d, visible, glue_depends_on_set) = pre_display_node_width(stores, line, node);
        if glue_depends_on_set {
            v = Scaled::MAX_DIMEN;
        }
        if v < Scaled::MAX_DIMEN {
            v = v + d;
        }
        if visible {
            if v < Scaled::MAX_DIMEN {
                w = v;
            } else {
                return Scaled::MAX_DIMEN;
            }
        }
    }
    w
}

fn pre_display_node_width<G>(
    stores: &CommandContext<'_, G>,
    line: &BoxNode,
    node: tex_state::node_arena::NodeRef<'_>,
) -> (Scaled, bool, bool) {
    match node {
        tex_state::node_arena::NodeRef::Char { font, ch, .. }
        | tex_state::node_arena::NodeRef::Lig { font, ch, .. } => {
            let width = u8::try_from(ch as u32)
                .ok()
                .and_then(|code| stores.font_char_metrics(font, code))
                .map_or(Scaled::from_raw(0), |metrics| metrics.width);
            (width, true, false)
        }
        tex_state::node_arena::NodeRef::HList(boxed)
        | tex_state::node_arena::NodeRef::VList(boxed) => (boxed.width, true, false),
        tex_state::node_arena::NodeRef::Rule { width, .. } => {
            (width.unwrap_or(Scaled::from_raw(0)), true, false)
        }
        tex_state::node_arena::NodeRef::Kern { amount, .. }
        | tex_state::node_arena::NodeRef::MathOn(amount)
        | tex_state::node_arena::NodeRef::MathOff(amount) => (amount, false, false),
        tex_state::node_arena::NodeRef::Glue { spec, .. } => {
            let glue = spec;
            let depends = match line.glue_sign {
                Sign::Stretching => {
                    line.glue_order == glue.stretch_order && glue.stretch.raw() != 0
                }
                Sign::Shrinking => line.glue_order == glue.shrink_order && glue.shrink.raw() != 0,
                Sign::Normal => false,
            };
            (glue.width, false, depends)
        }
        _ => (Scaled::from_raw(0), false, false),
    }
}
