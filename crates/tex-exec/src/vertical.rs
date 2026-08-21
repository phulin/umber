use tex_state::CommandContext;
use tex_state::env::banks::{DimenParam, GlueParam};
use tex_state::glue::GlueSpec;
use tex_state::node::{GlueKind, Node};
use tex_state::node_arena::NodeRef;
use tex_state::scaled::Scaled;

use crate::mode::ignored_depth;
use crate::{ExecError, Mode, ModeNest};

pub(crate) fn append_node_to_current_list<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    node: Node,
) -> Result<(), ExecError> {
    if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
        append_node_to_vertical_list(nest, stores, node)
    } else {
        nest.current_list_mutation().push(node);
        Ok(())
    }
}

pub(crate) fn append_node_to_vertical_list<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    node: Node,
) -> Result<(), ExecError> {
    let Some((height, depth)) = vertical_baseline_dimensions(&node) else {
        append_vertical_contribution(nest, stores, node);
        return Ok(());
    };
    if let Some(prev_depth) = nest.current_list().prev_depth()
        && prev_depth.raw() > ignored_depth(stores).raw()
    {
        let baseline = stores.glue(stores.glue_param(GlueParam::BASELINE_SKIP));
        let requested = baseline
            .width
            .checked_sub(prev_depth)
            .and_then(|value| value.checked_sub(height))
            .ok_or(ExecError::ArithmeticOverflow)?;
        let (spec, kind) =
            if requested.raw() < stores.dimen_param(DimenParam::LINE_SKIP_LIMIT).raw() {
                (
                    stores.glue(stores.glue_param(GlueParam::LINE_SKIP)),
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
                spec,
                kind,
                leader: None,
            },
        );
    }
    append_vertical_contribution(nest, stores, node);
    nest.current_list_mutation().set_prev_depth(depth);
    Ok(())
}

pub(crate) fn append_migrated_contribution<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    node: Node,
) {
    append_vertical_contribution(nest, stores, node);
}

pub(crate) fn append_vertical_contribution<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    node: Node,
) {
    if is_outer_vertical(nest) {
        stores.append_page_contribution(node);
    } else {
        nest.current_list_mutation().push(node);
    }
}

pub(crate) fn build_page_if_outer_vertical_with_error_context<G>(
    nest: &ModeNest,
    stores: &mut CommandContext<'_, G>,
    error_context: &str,
) -> Result<(), ExecError> {
    if is_outer_vertical(nest) {
        crate::page_builder::build_page_with_error_context(stores, error_context)?;
    }
    Ok(())
}

pub(crate) fn is_outer_vertical(nest: &ModeNest) -> bool {
    nest.depth() == 1 && nest.current_mode() == Mode::Vertical
}

fn vertical_baseline_dimensions(node: &Node) -> Option<(Scaled, Scaled)> {
    NodeRef::from(node).vertical_dimensions()
}
