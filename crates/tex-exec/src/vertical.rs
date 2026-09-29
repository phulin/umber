use tex_state::CommandContext;
use tex_state::diagnostic::DiagnosticEffects;
use tex_state::env::banks::{DimenParam, GlueParam};
use tex_state::glue::GlueSpec;
use tex_state::node::{GlueKind, Node};
use tex_state::node_view::NodeView;
use tex_state::scaled::Scaled;

use crate::{ExecError, Mode, ModeNest};

pub(crate) fn append_node_to_current_list<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    node: Node,
) -> Result<(), ExecError> {
    if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
        append_node_to_vertical_list(nest, stores, node)
    } else {
        nest.current_list_mutation().push(stores, node);
        Ok(())
    }
}

pub(crate) fn append_node_to_vertical_list<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    node: Node,
) -> Result<(), ExecError> {
    append_vertical_baseline_glue(nest, stores, &node)?;
    append_vertical_node_after_baseline(nest, stores, node);
    Ok(())
}

/// TeX82 §679's glue between consecutive boxes. A newly constructed box
/// publishes this page-owned node before sealing its transferable wrapper.
pub(crate) fn append_vertical_baseline_glue<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    node: &Node,
) -> Result<(), ExecError> {
    let Some((height, _)) = vertical_baseline_dimensions(node) else {
        return Ok(());
    };
    let ignored_depth = if stores.primitive_resolved("pdfignoreddimen").is_some() {
        stores.dimen_param(DimenParam::PDF_IGNORED_DIMEN)
    } else {
        crate::mode::IGNORE_DEPTH
    };
    let prev_depth = nest.current_list().prev_depth();
    if let Some(prev_depth) = prev_depth
        && prev_depth.raw() > ignored_depth.raw()
    {
        let baseline = stores
            .glue_param(GlueParam::BASELINE_SKIP)
            .map_or(GlueSpec::ZERO, |id| stores.glue(id));
        let requested = baseline
            .width
            .checked_sub(prev_depth)
            .and_then(|value| value.checked_sub(height))
            .ok_or(ExecError::ArithmeticOverflow)?;
        let (spec, kind) =
            if requested.raw() < stores.dimen_param(DimenParam::LINE_SKIP_LIMIT).raw() {
                (
                    stores
                        .glue_param(GlueParam::LINE_SKIP)
                        .map_or(GlueSpec::ZERO, |id| stores.glue(id)),
                    GlueKind::LineSkip,
                )
            } else {
                (
                    GlueSpec {
                        width: requested,
                        stretch: baseline.stretch,
                        stretch_order: baseline.stretch_order,
                        shrink: baseline.shrink,
                        shrink_order: baseline.shrink_order,
                    },
                    GlueKind::BaselineSkip,
                )
            };
        append_vertical_contribution(
            nest,
            stores,
            Node::Glue {
                origin: if kind == GlueKind::LineSkip {
                    tex_state::node::GlueSpecOrigin::from_trapped_parameter(spec)
                } else {
                    tex_state::node::GlueSpecOrigin::Owned
                },
                spec,
                kind,
                leader: None,
            },
        );
    }
    Ok(())
}

pub(crate) fn append_vertical_node_after_baseline<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    node: Node,
) {
    let depth = vertical_baseline_dimensions(&node).map(|(_, depth)| depth);
    append_vertical_contribution(nest, stores, node);
    if let Some(depth) = depth {
        nest.current_list_mutation().set_prev_depth(depth);
    }
}

pub(crate) fn append_vertical_contribution<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    node: Node,
) {
    if is_outer_vertical(nest) {
        stores.append_page_contribution(node);
    } else {
        nest.current_list_mutation().push(stores, node);
    }
}

pub(crate) fn build_page_if_outer_vertical_with_error_context<G>(
    nest: &ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    error_context: &str,
) -> Result<(), ExecError> {
    if is_outer_vertical(nest) {
        crate::page_builder::build_page_with_error_context(
            stores,
            diagnostic_effects,
            error_context,
        )?;
    }
    Ok(())
}

pub(crate) fn build_page_if_outer_vertical_with_diagnostic_context<G>(
    nest: &ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    diagnostic_context: &crate::diagnostics::ExecutionDiagnosticContext<'_, G>,
) -> Result<(), ExecError> {
    if is_outer_vertical(nest) {
        crate::page_builder::build_page_with_diagnostic_context(
            stores,
            diagnostic_effects,
            diagnostic_context,
        )?;
    }
    Ok(())
}

pub(crate) fn build_page_if_outer_vertical<G>(
    nest: &ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    command: &tex_command::CommandState<G>,
) -> Result<(), ExecError> {
    if is_outer_vertical(nest) {
        crate::page_builder::build_page(stores, diagnostic_effects, command)?;
    }
    Ok(())
}

pub(crate) fn is_outer_vertical(nest: &ModeNest) -> bool {
    nest.depth() == 1 && nest.current_mode() == Mode::Vertical
}

fn vertical_baseline_dimensions(node: &Node) -> Option<(Scaled, Scaled)> {
    NodeView::from(node).vertical_dimensions()
}
