use tex_arith::Scaled;

use crate::{BoxNode, GlueKind, LeaderPayload, PageEffect};

use super::{
    DviError, DviWriter,
    glue::{add_scaled, sub_scaled},
};

// TeX82 map: `Move right or output leaders`, `Output leaders in an hlist`,
// and their vlist counterparts inside `hlist_out`/`vlist_out` in `tex.web`.
// The +10sp/-10sp compensation, inclusive edge test, aligned ceiling on the
// containing box's grid, centered remainder split, and expanded `(q + 1)`
// spacing with half the division error at each end are exact TeX arithmetic.
// Recursive leader output also follows TeX's synch/save/traverse/restore
// order.  As in traversal.rs, Umber's positive-up hlist shift accounts for
// the subtraction used for horizontal leader boxes; vlist shift adds right.

const LEADER_ROUNDING_COMPENSATION: Scaled = Scaled::from_raw(10);

pub(super) struct HLeaderContext<'a> {
    pub(super) effects: &'a [PageEffect],
    pub(super) this_box: &'a BoxNode,
    pub(super) kind: GlueKind,
    pub(super) leader: &'a Option<LeaderPayload>,
    pub(super) rule_wd: Scaled,
    pub(super) left_edge: Scaled,
    pub(super) base_line: Scaled,
}

pub(super) struct VLeaderContext<'a> {
    pub(super) effects: &'a [PageEffect],
    pub(super) this_box: &'a BoxNode,
    pub(super) kind: GlueKind,
    pub(super) leader: &'a Option<LeaderPayload>,
    pub(super) rule_ht: Scaled,
    pub(super) left_edge: Scaled,
    pub(super) top_edge: Scaled,
}

impl<W: std::io::Write> DviWriter<W> {
    pub(super) fn move_right_or_output_leaders(
        &mut self,
        context: HLeaderContext<'_>,
    ) -> Result<(), DviError> {
        let Some(leader_kind) = leader_kind(context.kind) else {
            self.cur_h = add_scaled(self.cur_h, context.rule_wd)?;
            return Ok(());
        };
        let Some(leader) = context.leader.as_ref() else {
            self.cur_h = add_scaled(self.cur_h, context.rule_wd)?;
            return Ok(());
        };

        match leader {
            LeaderPayload::Rule { height, depth, .. } => {
                let rule_ht = height.unwrap_or(context.this_box.height);
                let rule_dp = depth.unwrap_or(context.this_box.depth);
                self.output_rule_in_hlist(rule_ht, rule_dp, context.rule_wd, context.base_line)?;
                self.cur_h = add_scaled(self.cur_h, context.rule_wd)?;
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
                    while add_scaled(self.cur_h, leader_wd)?.raw() <= edge.raw() {
                        self.output_leader_box_in_hlist(
                            context.effects,
                            leader,
                            box_node,
                            leader_wd,
                            lx,
                            context.base_line,
                        )?;
                    }
                    self.cur_h = sub_scaled(edge, LEADER_ROUNDING_COMPENSATION)?;
                } else {
                    self.cur_h = add_scaled(self.cur_h, context.rule_wd)?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn move_down_or_output_leaders(
        &mut self,
        context: VLeaderContext<'_>,
    ) -> Result<(), DviError> {
        let Some(leader_kind) = leader_kind(context.kind) else {
            self.cur_v = add_scaled(self.cur_v, context.rule_ht)?;
            return Ok(());
        };
        let Some(leader) = context.leader.as_ref() else {
            self.cur_v = add_scaled(self.cur_v, context.rule_ht)?;
            return Ok(());
        };

        match leader {
            LeaderPayload::Rule { width, .. } => {
                let rule_wd = width.unwrap_or(context.this_box.width);
                self.output_rule_in_vlist(context.rule_ht, rule_wd)?;
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
                    while add_scaled(self.cur_v, leader_ht)?.raw() <= edge.raw() {
                        self.output_leader_box_in_vlist(
                            context.effects,
                            leader,
                            box_node,
                            leader_ht,
                            lx,
                            context.left_edge,
                        )?;
                    }
                    self.cur_v = sub_scaled(edge, LEADER_ROUNDING_COMPENSATION)?;
                } else {
                    self.cur_v = add_scaled(self.cur_v, context.rule_ht)?;
                }
            }
        }
        Ok(())
    }

    fn output_leader_box_in_hlist(
        &mut self,
        effects: &[PageEffect],
        leader: &LeaderPayload,
        box_node: &BoxNode,
        leader_wd: Scaled,
        lx: Scaled,
        base_line: Scaled,
    ) -> Result<(), DviError> {
        self.cur_v = add_scaled(base_line, box_node.shift)?;
        self.synch_v()?;
        let save_v = self.dvi_v;
        self.synch_h()?;
        let save_h = self.dvi_h;
        match leader {
            LeaderPayload::HList(_) => self.hlist_out(effects, box_node)?,
            LeaderPayload::VList(_) => self.vlist_out(effects, box_node)?,
            LeaderPayload::Rule { .. } => unreachable!("caller handles rule leaders"),
        }
        self.dvi_v = save_v;
        self.dvi_h = save_h;
        self.cur_v = base_line;
        self.cur_h = add_scaled(add_scaled(save_h, leader_wd)?, lx)?;
        Ok(())
    }

    fn output_leader_box_in_vlist(
        &mut self,
        effects: &[PageEffect],
        leader: &LeaderPayload,
        box_node: &BoxNode,
        leader_ht: Scaled,
        lx: Scaled,
        left_edge: Scaled,
    ) -> Result<(), DviError> {
        self.cur_h = add_scaled(left_edge, box_node.shift)?;
        self.synch_h()?;
        let save_h = self.dvi_h;
        self.cur_v = add_scaled(self.cur_v, box_node.height)?;
        self.synch_v()?;
        let save_v = self.dvi_v;
        match leader {
            LeaderPayload::HList(_) => self.hlist_out(effects, box_node)?,
            LeaderPayload::VList(_) => self.vlist_out(effects, box_node)?,
            LeaderPayload::Rule { .. } => unreachable!("caller handles rule leaders"),
        }
        self.dvi_v = save_v;
        self.dvi_h = save_h;
        self.cur_h = left_edge;
        self.cur_v = add_scaled(
            sub_scaled(save_v, box_node.height)?,
            add_scaled(leader_ht, lx)?,
        )?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LeaderKind {
    Aligned,
    Centered,
    Expanded,
}

fn leader_kind(kind: GlueKind) -> Option<LeaderKind> {
    match kind {
        GlueKind::Leaders => Some(LeaderKind::Aligned),
        GlueKind::Cleaders => Some(LeaderKind::Centered),
        GlueKind::Xleaders => Some(LeaderKind::Expanded),
        GlueKind::Normal
        | GlueKind::BaselineSkip
        | GlueKind::LineSkip
        | GlueKind::LeftSkip
        | GlueKind::RightSkip
        | GlueKind::ParFillSkip => None,
    }
}

fn leader_start(
    kind: LeaderKind,
    cur: Scaled,
    origin: Scaled,
    available: Scaled,
    leader_size: Scaled,
) -> Result<(Scaled, Scaled), DviError> {
    debug_assert!(available.raw() > 0);
    debug_assert!(leader_size.raw() > 0);
    match kind {
        LeaderKind::Aligned => {
            let diff = i64::from(cur.raw()) - i64::from(origin.raw());
            let quotient = diff / i64::from(leader_size.raw());
            let start = i64::from(origin.raw()) + i64::from(leader_size.raw()) * quotient;
            let mut start = scaled_from_i64(start)?;
            if start.raw() < cur.raw() {
                start = add_scaled(start, leader_size)?;
            }
            Ok((start, Scaled::from_raw(0)))
        }
        LeaderKind::Centered => {
            let remainder = available.raw() % leader_size.raw();
            Ok((
                add_scaled(cur, Scaled::from_raw(remainder / 2))?,
                Scaled::from_raw(0),
            ))
        }
        LeaderKind::Expanded => {
            let quotient = i64::from(available.raw() / leader_size.raw());
            let remainder = i64::from(available.raw() % leader_size.raw());
            let lx = remainder / (quotient + 1);
            let start_offset = (remainder - (quotient - 1) * lx) / 2;
            Ok((
                add_scaled(cur, scaled_from_i64(start_offset)?)?,
                scaled_from_i64(lx)?,
            ))
        }
    }
}

fn scaled_from_i64(value: i64) -> Result<Scaled, DviError> {
    i32::try_from(value)
        .map(Scaled::from_raw)
        .map_err(|_| DviError::PositionOverflow)
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
            leader_start(LeaderKind::Aligned, sp(23), sp(0), sp(40), sp(10))
                .expect("aligned positive leader start"),
            (sp(30), sp(0))
        );
        assert_eq!(
            leader_start(LeaderKind::Aligned, sp(-11), sp(0), sp(40), sp(10))
                .expect("aligned negative leader start below grid"),
            (sp(-10), sp(0))
        );
        assert_eq!(
            leader_start(LeaderKind::Aligned, sp(-9), sp(0), sp(40), sp(10))
                .expect("aligned negative leader start above grid"),
            (sp(0), sp(0))
        );
    }

    #[test]
    fn centered_leader_start_places_half_remainder_at_each_end() {
        assert_eq!(
            leader_start(LeaderKind::Centered, sp(20), sp(0), sp(37), sp(10))
                .expect("centered leader start"),
            (sp(23), sp(0))
        );
    }

    #[test]
    fn expanded_leader_start_matches_tex_web_integer_spacing() {
        assert_eq!(
            leader_start(LeaderKind::Expanded, sp(20), sp(0), sp(37), sp(10))
                .expect("expanded leader start"),
            (sp(22), sp(1))
        );
        assert_eq!(
            leader_start(LeaderKind::Expanded, sp(20), sp(0), sp(8), sp(10))
                .expect("expanded leader start shorter than payload"),
            (sp(28), sp(8))
        );
    }
}
