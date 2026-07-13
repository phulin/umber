use tex_arith::Scaled;

use crate::{BoxNode, PageArtifact, PageEffect, PageNode, binary::V10PageDecoder};

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

pub(super) enum RootStreamState {
    H {
        save_loc: usize,
        base_line: Scaled,
        left_edge: Scaled,
        cur_g: Scaled,
        cur_glue: Scaled,
    },
    V {
        save_loc: usize,
        left_edge: Scaled,
        top_edge: Scaled,
        cur_g: Scaled,
        cur_glue: Scaled,
    },
}

impl<W: std::io::Write> DviWriter<W> {
    pub(super) fn begin_root_stream(
        &mut self,
        h_offset: Scaled,
        v_offset: Scaled,
        root: &BoxNode,
        vertical: bool,
    ) -> Result<RootStreamState, DviError> {
        self.cur_h = h_offset;
        self.cur_v = root
            .height
            .checked_add(v_offset)
            .ok_or(DviError::PositionOverflow)?;
        self.enter_box();
        if self.cur_s > 0 {
            self.u8(PUSH);
        }
        let save_loc = self.bytes.len();
        if vertical {
            let left_edge = self.cur_h;
            self.cur_v = sub_scaled(self.cur_v, root.height)?;
            Ok(RootStreamState::V {
                save_loc,
                left_edge,
                top_edge: self.cur_v,
                cur_g: Scaled::from_raw(0),
                cur_glue: Scaled::from_raw(0),
            })
        } else {
            Ok(RootStreamState::H {
                save_loc,
                base_line: self.cur_v,
                left_edge: self.cur_h,
                cur_g: Scaled::from_raw(0),
                cur_glue: Scaled::from_raw(0),
            })
        }
    }

    pub(super) fn push_root_stream_child(
        &mut self,
        effects: &[PageEffect],
        root: &BoxNode,
        state: &mut RootStreamState,
        child: &PageNode,
    ) -> Result<(), DviError> {
        match state {
            RootStreamState::H {
                base_line,
                left_edge,
                cur_g,
                cur_glue,
                ..
            } => {
                self.output_hlist_child(
                    effects, root, child, *base_line, *left_edge, cur_g, cur_glue,
                )?;
                self.cur_v = *base_line;
            }
            RootStreamState::V {
                left_edge,
                top_edge,
                cur_g,
                cur_glue,
                ..
            } => self
                .output_vlist_child(effects, root, child, *left_edge, *top_edge, cur_g, cur_glue)?,
        }
        Ok(())
    }

    pub(super) fn finish_root_stream(&mut self, state: RootStreamState) -> Result<(), DviError> {
        let save_loc = match state {
            RootStreamState::H { save_loc, .. } | RootStreamState::V { save_loc, .. } => save_loc,
        };
        self.prune_movements(save_loc);
        if self.cur_s > 0 {
            self.dvi_pop(save_loc);
        }
        self.cur_s -= 1;
        Ok(())
    }

    pub(super) fn ship_streamed_box(
        &mut self,
        page: &PageArtifact,
        root: &BoxNode,
        vertical: bool,
        decoder: &mut V10PageDecoder<'_>,
    ) -> Result<(), DviError> {
        let mut state =
            self.begin_root_stream(page.job.h_offset, page.job.v_offset, root, vertical)?;
        while let Some(child) = decoder.next_child()? {
            self.push_root_stream_child(&page.effects, root, &mut state, &child)?;
        }
        self.finish_root_stream(state)
    }

    #[allow(clippy::too_many_arguments)] // Explicit TeX hlist traversal registers.
    fn output_hlist_child(
        &mut self,
        effects: &[PageEffect],
        this_box: &BoxNode,
        child: &PageNode,
        base_line: Scaled,
        left_edge: Scaled,
        cur_g: &mut Scaled,
        cur_glue: &mut Scaled,
    ) -> Result<(), DviError> {
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
                self.output_box_in_hlist(effects, box_node, matches!(child, PageNode::VList(_)))?;
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
                    this_box.glue_sign,
                    this_box.glue_order,
                    this_box.glue_set,
                    cur_glue,
                    cur_g,
                )?;
                self.move_right_or_output_leaders(leaders::HLeaderContext {
                    effects,
                    this_box,
                    kind: *kind,
                    leader,
                    rule_wd,
                    left_edge,
                    base_line,
                })?;
            }
            PageNode::Kern { amount, .. } => self.cur_h = add_scaled(self.cur_h, *amount)?,
            PageNode::MathOn(width) | PageNode::MathOff(width) => {
                self.cur_h = add_scaled(self.cur_h, *width)?;
            }
            PageNode::WhatsitAnchor { effect_index } => {
                self.out_what(effects, *effect_index)?;
            }
            PageNode::Penalty(_)
            | PageNode::Disc { .. }
            | PageNode::Mark { .. }
            | PageNode::Insert { .. }
            | PageNode::Adjust(_) => {}
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)] // Explicit TeX vlist traversal registers.
    fn output_vlist_child(
        &mut self,
        effects: &[PageEffect],
        this_box: &BoxNode,
        child: &PageNode,
        left_edge: Scaled,
        top_edge: Scaled,
        cur_g: &mut Scaled,
        cur_glue: &mut Scaled,
    ) -> Result<(), DviError> {
        match child {
            PageNode::HList(box_node) | PageNode::VList(box_node) => {
                self.output_box_in_vlist(effects, box_node, matches!(child, PageNode::VList(_)))?;
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
                self.output_rule_in_vlist(rule_ht, width.unwrap_or(this_box.width))?;
            }
            PageNode::Glue { spec, kind, leader } => {
                let rule_ht = adjusted_glue_width(
                    *spec,
                    this_box.glue_sign,
                    this_box.glue_order,
                    this_box.glue_set,
                    cur_glue,
                    cur_g,
                )?;
                self.move_down_or_output_leaders(leaders::VLeaderContext {
                    effects,
                    this_box,
                    kind: *kind,
                    leader,
                    rule_ht,
                    left_edge,
                    top_edge,
                })?;
            }
            PageNode::Kern { amount, .. } => self.cur_v = add_scaled(self.cur_v, *amount)?,
            PageNode::WhatsitAnchor { effect_index } => {
                self.out_what(effects, *effect_index)?;
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
        Ok(())
    }

    pub(super) fn hlist_out(
        &mut self,
        effects: &[PageEffect],
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
                    self.output_box_in_hlist(
                        effects,
                        box_node,
                        matches!(child, PageNode::VList(_)),
                    )?;
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
                        effects,
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
                    self.out_what(effects, *effect_index)?;
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
        effects: &[PageEffect],
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
                    self.output_box_in_vlist(
                        effects,
                        box_node,
                        matches!(child, PageNode::VList(_)),
                    )?;
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
                        effects,
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
                    self.out_what(effects, *effect_index)?;
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
        effects: &[PageEffect],
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
            self.vlist_out(effects, box_node)?;
        } else {
            self.hlist_out(effects, box_node)?;
        }
        self.dvi_h = save_h;
        self.dvi_v = save_v;
        self.cur_h = add_scaled(edge, box_node.width)?;
        self.cur_v = base_line;
        Ok(())
    }

    fn output_box_in_vlist(
        &mut self,
        effects: &[PageEffect],
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
            self.vlist_out(effects, box_node)?;
        } else {
            self.hlist_out(effects, box_node)?;
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

    fn out_what(&mut self, effects: &[PageEffect], effect_index: u32) -> Result<(), DviError> {
        let effect = effects
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
