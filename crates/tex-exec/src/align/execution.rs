//! Source-free canonical alignment completion and list contribution.

use tex_state::CommandContext;

use crate::{Mode, ModeNest};

pub(crate) struct FinishedAlignment {
    pub(crate) nodes: tex_state::node_arena::PageListId,
    pub(crate) aux_prev_depth: Option<tex_state::scaled::Scaled>,
    pub(crate) aux_space_factor: Option<i32>,
}

pub(crate) fn append_finished_alignment<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    finished: FinishedAlignment,
) {
    if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical)
        && let Some(prev_depth) = finished.aux_prev_depth
    {
        nest.current_list_mutation().set_prev_depth(prev_depth);
    }
    if matches!(
        nest.current_mode(),
        Mode::Horizontal | Mode::RestrictedHorizontal
    ) && let Some(space_factor) = finished.aux_space_factor
    {
        nest.current_list_mutation().set_space_factor(space_factor);
    }
    if crate::vertical::is_outer_vertical(nest) {
        stores.append_page_contributions(finished.nodes);
    } else {
        nest.current_list_mutation()
            .append_list(stores, finished.nodes);
    }
}
