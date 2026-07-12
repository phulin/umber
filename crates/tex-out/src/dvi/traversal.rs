use tex_arith::Scaled;

use crate::{BoxNode, PageArtifact, PageEffect, PageNode};

use super::{
    DviError, DviWriter,
    glue::{add_scaled, adjusted_glue_width, sub_scaled},
    leaders,
    opcodes::{DOWN1, POP, PUSH, PUT_RULE, RIGHT1, SET_RULE, XXX1, XXX4},
};

// TeX82 map: this module ports `hlist_out`, `vlist_out`, `synch_h`,
// `synch_v`, `dvi_pop`, and their `Output ... node` fragments in `tex.web`.
// Child order, delayed coordinate synchronization, rule placement, recursive
// save/restore, movement pruning before pop, and push-pop cancellation are DVI
// semantics.  PageArtifact is Umber's detached representation, but traversal
// must treat its children in the same order and with the same dimensions.
// BoxNode::shift is the one sign boundary: Umber stores positive hlist shift
// upward, inverse to TeX's positive-down `shift_amount`; vlist shift remains
// positive rightward.  Thus hlist recursion subtracts shift and vlist
// recursion adds it.

impl<W: std::io::Write> DviWriter<W> {
    pub(super) fn hlist_out(
        &mut self,
        page: &PageArtifact,
        this_box: &BoxNode,
    ) -> Result<(), DviError> {
        let g_order = this_box.glue_order;
        let g_sign = this_box.glue_sign;
        self.enter_box();
        if self.cur_s > 0 {
            self.u8(PUSH);
        }
        let save_loc = self.bytes.len();
        let base_line = self.cur_v;
        let left_edge = self.cur_h;
        let mut cur_g = Scaled::from_raw(0);
        let mut cur_glue = Scaled::from_raw(0);

        for child in &this_box.children {
            match child {
                PageNode::Char { font_id, ch, width }
                | PageNode::Lig {
                    font_id, ch, width, ..
                } => {
                    self.synch_h()?;
                    self.synch_v()?;
                    self.change_font(*font_id)?;
                    self.set_char(*ch)?;
                    self.cur_h = add_scaled(self.cur_h, *width)?;
                    self.dvi_h = self.cur_h;
                }
                PageNode::HList(box_node) | PageNode::VList(box_node) => {
                    self.output_box_in_hlist(page, box_node, matches!(child, PageNode::VList(_)))?;
                }
                PageNode::Rule {
                    width,
                    height,
                    depth,
                } => {
                    let rule_ht = height.unwrap_or(this_box.height);
                    let rule_dp = depth.unwrap_or(this_box.depth);
                    let rule_wd = width.unwrap_or(Scaled::from_raw(0));
                    self.output_rule_in_hlist(rule_ht, rule_dp, rule_wd, base_line)?;
                    self.cur_h = add_scaled(self.cur_h, rule_wd)?;
                }
                PageNode::Glue { spec, kind, leader } => {
                    let rule_wd = adjusted_glue_width(
                        *spec,
                        g_sign,
                        g_order,
                        this_box.glue_set,
                        &mut cur_glue,
                        &mut cur_g,
                    )?;
                    self.move_right_or_output_leaders(leaders::HLeaderContext {
                        page,
                        this_box,
                        kind: *kind,
                        leader,
                        rule_wd,
                        left_edge,
                        base_line,
                    })?;
                }
                PageNode::Kern { amount, .. } => {
                    self.cur_h = add_scaled(self.cur_h, *amount)?;
                }
                PageNode::MathOn(width) | PageNode::MathOff(width) => {
                    self.cur_h = add_scaled(self.cur_h, *width)?;
                }
                PageNode::WhatsitAnchor { effect_index } => {
                    self.out_what(page, *effect_index)?;
                }
                PageNode::Penalty(_)
                | PageNode::Disc { .. }
                | PageNode::Mark { .. }
                | PageNode::Insert { .. }
                | PageNode::Adjust(_) => {}
            }
            self.cur_v = base_line;
        }

        self.prune_movements(save_loc);
        if self.cur_s > 0 {
            self.dvi_pop(save_loc);
        }
        self.cur_s -= 1;
        Ok(())
    }

    pub(super) fn vlist_out(
        &mut self,
        page: &PageArtifact,
        this_box: &BoxNode,
    ) -> Result<(), DviError> {
        let g_order = this_box.glue_order;
        let g_sign = this_box.glue_sign;
        self.enter_box();
        if self.cur_s > 0 {
            self.u8(PUSH);
        }
        let save_loc = self.bytes.len();
        let left_edge = self.cur_h;
        self.cur_v = sub_scaled(self.cur_v, this_box.height)?;
        let top_edge = self.cur_v;
        let mut cur_g = Scaled::from_raw(0);
        let mut cur_glue = Scaled::from_raw(0);

        for child in &this_box.children {
            match child {
                PageNode::HList(box_node) | PageNode::VList(box_node) => {
                    self.output_box_in_vlist(page, box_node, matches!(child, PageNode::VList(_)))?;
                    self.cur_h = left_edge;
                }
                PageNode::Rule {
                    width,
                    height,
                    depth,
                } => {
                    let rule_ht = add_scaled(
                        height.unwrap_or(Scaled::from_raw(0)),
                        depth.unwrap_or(Scaled::from_raw(0)),
                    )?;
                    let rule_wd = width.unwrap_or(this_box.width);
                    self.output_rule_in_vlist(rule_ht, rule_wd)?;
                }
                PageNode::Glue { spec, kind, leader } => {
                    let rule_ht = adjusted_glue_width(
                        *spec,
                        g_sign,
                        g_order,
                        this_box.glue_set,
                        &mut cur_glue,
                        &mut cur_g,
                    )?;
                    self.move_down_or_output_leaders(leaders::VLeaderContext {
                        page,
                        this_box,
                        kind: *kind,
                        leader,
                        rule_ht,
                        left_edge,
                        top_edge,
                    })?;
                }
                PageNode::Kern { amount, .. } => {
                    self.cur_v = add_scaled(self.cur_v, *amount)?;
                }
                PageNode::WhatsitAnchor { effect_index } => {
                    self.out_what(page, *effect_index)?;
                }
                PageNode::Char { .. }
                | PageNode::Lig { .. }
                | PageNode::Penalty(_)
                | PageNode::Disc { .. }
                | PageNode::Mark { .. }
                | PageNode::Insert { .. }
                | PageNode::MathOn(_)
                | PageNode::MathOff(_)
                | PageNode::Adjust(_) => {}
            }
        }

        self.prune_movements(save_loc);
        if self.cur_s > 0 {
            self.dvi_pop(save_loc);
        }
        self.cur_s -= 1;
        Ok(())
    }

    fn output_box_in_hlist(
        &mut self,
        page: &PageArtifact,
        box_node: &BoxNode,
        is_vlist: bool,
    ) -> Result<(), DviError> {
        if box_node.children.is_empty() {
            self.cur_h = add_scaled(self.cur_h, box_node.width)?;
            return Ok(());
        }
        let save_h = self.dvi_h;
        let save_v = self.dvi_v;
        let edge = self.cur_h;
        let base_line = self.cur_v;
        self.cur_v = add_scaled(base_line, box_node.shift)?;
        if is_vlist {
            self.vlist_out(page, box_node)?;
        } else {
            self.hlist_out(page, box_node)?;
        }
        self.dvi_h = save_h;
        self.dvi_v = save_v;
        self.cur_h = add_scaled(edge, box_node.width)?;
        self.cur_v = base_line;
        Ok(())
    }

    fn output_box_in_vlist(
        &mut self,
        page: &PageArtifact,
        box_node: &BoxNode,
        is_vlist: bool,
    ) -> Result<(), DviError> {
        if box_node.children.is_empty() {
            self.cur_v = add_scaled(add_scaled(self.cur_v, box_node.height)?, box_node.depth)?;
            return Ok(());
        }
        self.cur_v = add_scaled(self.cur_v, box_node.height)?;
        self.synch_v()?;
        let save_h = self.dvi_h;
        let save_v = self.dvi_v;
        let left_edge = self.cur_h;
        self.cur_h = add_scaled(left_edge, box_node.shift)?;
        if is_vlist {
            self.vlist_out(page, box_node)?;
        } else {
            self.hlist_out(page, box_node)?;
        }
        self.dvi_h = save_h;
        self.dvi_v = save_v;
        self.cur_v = add_scaled(save_v, box_node.depth)?;
        self.cur_h = left_edge;
        Ok(())
    }

    pub(super) fn output_rule_in_hlist(
        &mut self,
        rule_ht: Scaled,
        rule_dp: Scaled,
        rule_wd: Scaled,
        base_line: Scaled,
    ) -> Result<(), DviError> {
        let rule_ht = add_scaled(rule_ht, rule_dp)?;
        if rule_ht.raw() > 0 && rule_wd.raw() > 0 {
            self.synch_h()?;
            self.cur_v = add_scaled(base_line, rule_dp)?;
            self.synch_v()?;
            self.u8(SET_RULE);
            self.scaled(rule_ht);
            self.scaled(rule_wd);
            self.cur_v = base_line;
            self.dvi_h = add_scaled(self.dvi_h, rule_wd)?;
        }
        Ok(())
    }

    pub(super) fn output_rule_in_vlist(
        &mut self,
        rule_ht: Scaled,
        rule_wd: Scaled,
    ) -> Result<(), DviError> {
        self.cur_v = add_scaled(self.cur_v, rule_ht)?;
        if rule_ht.raw() > 0 && rule_wd.raw() > 0 {
            self.synch_h()?;
            self.synch_v()?;
            self.u8(PUT_RULE);
            self.scaled(rule_ht);
            self.scaled(rule_wd);
        }
        Ok(())
    }

    fn enter_box(&mut self) {
        self.cur_s += 1;
        if let Ok(depth) = u16::try_from(self.cur_s) {
            self.max_stack_depth = self.max_stack_depth.max(depth);
        }
    }

    pub(super) fn synch_h(&mut self) -> Result<(), DviError> {
        if self.cur_h != self.dvi_h {
            let movement = sub_scaled(self.cur_h, self.dvi_h)?;
            self.right_stack.movement(&mut self.bytes, movement, RIGHT1);
            self.dvi_h = self.cur_h;
        }
        Ok(())
    }

    pub(super) fn synch_v(&mut self) -> Result<(), DviError> {
        if self.cur_v != self.dvi_v {
            let movement = sub_scaled(self.cur_v, self.dvi_v)?;
            self.down_stack.movement(&mut self.bytes, movement, DOWN1);
            self.dvi_v = self.cur_v;
        }
        Ok(())
    }

    fn prune_movements(&mut self, save_loc: usize) {
        self.down_stack.prune_movements(save_loc);
        self.right_stack.prune_movements(save_loc);
    }

    fn dvi_pop(&mut self, save_loc: usize) {
        if save_loc == self.bytes.len() && !self.bytes.is_empty() {
            self.bytes.pop();
        } else {
            self.u8(POP);
        }
    }

    fn out_what(&mut self, page: &PageArtifact, effect_index: u32) -> Result<(), DviError> {
        let effect = page
            .effects
            .get(usize::try_from(effect_index).expect("u32 fits usize"))
            .ok_or(DviError::MissingEffect { effect_index })?;
        if let PageEffect::Special { payload, .. } = effect {
            self.special_out(payload)?;
        }
        Ok(())
    }

    fn special_out(&mut self, payload: &[u8]) -> Result<(), DviError> {
        self.synch_h()?;
        self.synch_v()?;
        if payload.len() < 256 {
            self.u8(XXX1);
            self.u8(payload.len() as u8);
        } else {
            let len = i32::try_from(payload.len())
                .map_err(|_| DviError::SpecialTooLong { len: payload.len() })?;
            self.u8(XXX4);
            self.i32(len);
        }
        self.raw(payload);
        Ok(())
    }
}
