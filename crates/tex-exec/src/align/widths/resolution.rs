use tex_state::CommandContext;
#[cfg(test)]
mod tests;

use tex_state::node::Node;
use tex_typeset::alignment::{
    AlignmentPlanError, AlignmentWidthRequirement, plan_alignment_widths,
};

use crate::ExecError;
use crate::mode::{AlignState, AlignmentKind};

use super::{ResolvedWidths, unset_axis_size};

pub(super) fn resolve_widths<G>(
    state: &AlignState,
    rows: tex_state::node_arena::NodeCursor<'_>,
    stores: &CommandContext<'_, G>,
) -> Result<ResolvedWidths, ExecError> {
    let requirements = collect_width_requirements(state.kind(), rows, stores)?;
    let column_count = requirements
        .iter()
        .map(|requirement| requirement.first_column + requirement.span)
        .max()
        .unwrap_or(state.columns().len())
        .max(state.columns().len());
    let mut tabskips = initial_tabskips(state, column_count);
    let tabskip_widths = tabskips.iter().map(|root| root.width).collect::<Vec<_>>();
    let plan = plan_alignment_widths(state.columns().len(), &tabskip_widths, requirements)
        .map_err(map_plan_error)?;
    for boundary in plan.zero_tabskip_boundaries {
        tabskips[boundary] = tex_state::glue::GlueSpec::ZERO;
    }

    Ok(ResolvedWidths {
        columns: plan.columns,
        tabskips,
    })
}

fn initial_tabskips(state: &AlignState, columns: usize) -> Vec<tex_state::glue::GlueSpec> {
    (0..=columns)
        .map(|boundary| *state.tabskip_for_boundary(boundary))
        .collect()
}

fn collect_width_requirements<G>(
    kind: AlignmentKind,
    rows: tex_state::node_arena::NodeCursor<'_>,
    stores: &CommandContext<'_, G>,
) -> Result<Vec<AlignmentWidthRequirement>, ExecError> {
    let mut requirements = Vec::new();
    for node in rows.iter() {
        let Node::Unset(row) = node else {
            continue;
        };
        let mut column = 0usize;
        for child in stores
            .page_node_list(row.children)
            .expect("alignment row belongs to the live page arena")
            .iter()
        {
            let Node::Unset(cell) = child else {
                continue;
            };
            let span = usize::from(cell.span_count) + 1;
            requirements.push(AlignmentWidthRequirement {
                first_column: column,
                span,
                width: unset_axis_size(kind, cell)?,
            });
            column += span;
        }
    }
    Ok(requirements)
}

fn map_plan_error(error: AlignmentPlanError) -> ExecError {
    match error {
        AlignmentPlanError::ArithmeticOverflow => ExecError::ArithmeticOverflow,
        AlignmentPlanError::MissingTabskipBoundary(boundary) => {
            panic!("alignment extraction omitted tabskip boundary {boundary}")
        }
    }
}
