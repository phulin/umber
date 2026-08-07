use tex_arith::Scaled;

use crate::{BoxNode, GlueKind, LeaderPayload};

use super::{DviBodyCompiler, DviError, glue::add_scaled};
use crate::geometry::{LEADER_ROUNDING_COMPENSATION, LeaderMode, leader_start};

// TeX82 map: `Move right or output leaders`, `Output leaders in an hlist`,
// and their vlist counterparts inside `hlist_out`/`vlist_out` in `tex.web`.
// The +10sp/-10sp compensation, inclusive edge test, aligned ceiling on the
// containing box's grid, centered remainder split, and expanded `(q + 1)`
// spacing with half the division error at each end are exact TeX arithmetic.
// Recursive leader output also follows TeX's synch/save/traverse/restore
// order.  As in traversal.rs, Umber's positive-up hlist shift accounts for
// the subtraction used for horizontal leader boxes; vlist shift adds right.

pub(super) struct HLeaderContext<'f, 'a> {
    pub(super) this_box: &'f BoxNode,
    pub(super) kind: GlueKind,
    pub(super) leader: &'a Option<LeaderPayload>,
    pub(super) rule_wd: Scaled,
    pub(super) left_edge: Scaled,
    pub(super) base_line: Scaled,
}

pub(super) struct VLeaderContext<'f, 'a> {
    pub(super) this_box: &'f BoxNode,
    pub(super) kind: GlueKind,
    pub(super) leader: &'a Option<LeaderPayload>,
    pub(super) rule_ht: Scaled,
    pub(super) left_edge: Scaled,
    pub(super) top_edge: Scaled,
}

#[derive(Clone, Copy)]
pub(super) struct HLeaderRepeat<'a> {
    pub(super) leader: &'a LeaderPayload,
    pub(super) box_node: &'a BoxNode,
    pub(super) edge: Scaled,
    pub(super) extra: Scaled,
    pub(super) base_line: Scaled,
}

#[derive(Clone, Copy)]
pub(super) struct VLeaderRepeat<'a> {
    pub(super) leader: &'a LeaderPayload,
    pub(super) box_node: &'a BoxNode,
    pub(super) edge: Scaled,
    pub(super) extra: Scaled,
    pub(super) left_edge: Scaled,
}

impl DviBodyCompiler {
    pub(super) fn move_right_or_output_leaders<'a>(
        &mut self,
        context: HLeaderContext<'_, 'a>,
    ) -> Result<Option<HLeaderRepeat<'a>>, DviError> {
        let Some(leader_kind) = LeaderMode::from_glue(context.kind) else {
            self.cur_h = add_scaled(self.cur_h, context.rule_wd)?;
            return Ok(None);
        };
        let Some(leader) = context.leader.as_ref() else {
            self.cur_h = add_scaled(self.cur_h, context.rule_wd)?;
            return Ok(None);
        };

        match leader {
            LeaderPayload::Rule { height, depth, .. } => {
                let rule_ht = height.unwrap_or(context.this_box.height);
                let rule_dp = depth.unwrap_or(context.this_box.depth);
                self.output_rule_in_hlist(rule_ht, rule_dp, context.rule_wd, context.base_line)?;
                self.cur_h = add_scaled(self.cur_h, context.rule_wd)?;
                Ok(None)
            }
            LeaderPayload::HList(box_node) | LeaderPayload::VList(box_node) => {
                let leader_wd = box_node.width;
                if leader_wd.raw() > 0 && context.rule_wd.raw() > 0 {
                    let leader_space = add_scaled(context.rule_wd, LEADER_ROUNDING_COMPENSATION)?;
                    let edge = add_scaled(self.cur_h, leader_space)?;
                    let (start, lx) = leader_start(
                        leader_kind,
                        self.cur_h,
                        context.left_edge,
                        leader_space,
                        leader_wd,
                    )?;
                    self.cur_h = start;
                    Ok(Some(HLeaderRepeat {
                        leader,
                        box_node,
                        edge,
                        extra: lx,
                        base_line: context.base_line,
                    }))
                } else {
                    self.cur_h = add_scaled(self.cur_h, context.rule_wd)?;
                    Ok(None)
                }
            }
        }
    }

    pub(super) fn move_down_or_output_leaders<'a>(
        &mut self,
        context: VLeaderContext<'_, 'a>,
    ) -> Result<Option<VLeaderRepeat<'a>>, DviError> {
        let Some(leader_kind) = LeaderMode::from_glue(context.kind) else {
            self.cur_v = add_scaled(self.cur_v, context.rule_ht)?;
            return Ok(None);
        };
        let Some(leader) = context.leader.as_ref() else {
            self.cur_v = add_scaled(self.cur_v, context.rule_ht)?;
            return Ok(None);
        };

        match leader {
            LeaderPayload::Rule { width, .. } => {
                let rule_wd = width.unwrap_or(context.this_box.width);
                self.output_rule_in_vlist(context.rule_ht, rule_wd)?;
                Ok(None)
            }
            LeaderPayload::HList(box_node) | LeaderPayload::VList(box_node) => {
                let leader_ht = add_scaled(box_node.height, box_node.depth)?;
                if leader_ht.raw() > 0 && context.rule_ht.raw() > 0 {
                    let leader_space = add_scaled(context.rule_ht, LEADER_ROUNDING_COMPENSATION)?;
                    let edge = add_scaled(self.cur_v, leader_space)?;
                    let (start, lx) = leader_start(
                        leader_kind,
                        self.cur_v,
                        context.top_edge,
                        leader_space,
                        leader_ht,
                    )?;
                    self.cur_v = start;
                    Ok(Some(VLeaderRepeat {
                        leader,
                        box_node,
                        edge,
                        extra: lx,
                        left_edge: context.left_edge,
                    }))
                } else {
                    self.cur_v = add_scaled(self.cur_v, context.rule_ht)?;
                    Ok(None)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sp(raw: i32) -> Scaled {
        Scaled::from_raw(raw)
    }

    #[test]
    fn aligned_leader_start_uses_first_grid_position_not_less_than_current() {
        assert_eq!(
            leader_start(LeaderMode::Aligned, sp(23), sp(0), sp(40), sp(10))
                .expect("aligned positive leader start"),
            (sp(30), sp(0))
        );
        assert_eq!(
            leader_start(LeaderMode::Aligned, sp(-11), sp(0), sp(40), sp(10))
                .expect("aligned negative leader start below grid"),
            (sp(-10), sp(0))
        );
        assert_eq!(
            leader_start(LeaderMode::Aligned, sp(-9), sp(0), sp(40), sp(10))
                .expect("aligned negative leader start above grid"),
            (sp(0), sp(0))
        );
    }

    #[test]
    fn centered_leader_start_places_half_remainder_at_each_end() {
        assert_eq!(
            leader_start(LeaderMode::Centered, sp(20), sp(0), sp(37), sp(10))
                .expect("centered leader start"),
            (sp(23), sp(0))
        );
    }

    #[test]
    fn expanded_leader_start_matches_tex_web_integer_spacing() {
        assert_eq!(
            leader_start(LeaderMode::Expanded, sp(20), sp(0), sp(37), sp(10))
                .expect("expanded leader start"),
            (sp(22), sp(1))
        );
        assert_eq!(
            leader_start(LeaderMode::Expanded, sp(20), sp(0), sp(8), sp(10))
                .expect("expanded leader start shorter than payload"),
            (sp(28), sp(8))
        );
    }
}
