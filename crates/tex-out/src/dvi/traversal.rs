use tex_arith::Scaled;

use crate::{BoxNode, PageEffect, PageNode};

use super::{
    DviError, DviWriter,
    coordinates::DviCoordinateEvent,
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

/// Explicit traversal state for direct page emission.
///
/// Unlike `RootStreamState`, this stack represents every live box.  Fresh
/// shipout can therefore feed scalar nodes straight from the engine arena
/// without constructing `PageNode` children or recursively entering the DVI
/// walker.
pub(super) struct DirectStreamState {
    frames: Vec<DirectFrame>,
}

struct DirectFrame {
    fields: BoxNode,
    save_loc: usize,
    axis: DirectAxis,
    continuation: DirectContinuation,
}

#[derive(Clone, Copy)]
enum DirectAxis {
    H {
        base_line: Scaled,
        left_edge: Scaled,
        cur_g: Scaled,
        cur_glue: Scaled,
    },
    V {
        left_edge: Scaled,
        top_edge: Scaled,
        cur_g: Scaled,
        cur_glue: Scaled,
    },
}

#[derive(Clone, Copy)]
enum DirectContinuation {
    Root,
    H {
        save_h: Scaled,
        save_v: Scaled,
        edge: Scaled,
        base_line: Scaled,
        width: Scaled,
    },
    V {
        save_h: Scaled,
        save_v: Scaled,
        left_edge: Scaled,
        depth: Scaled,
    },
}

impl<W: std::io::Write> DviWriter<W> {
    pub(super) fn begin_direct_stream(
        &mut self,
        h_offset: Scaled,
        v_offset: Scaled,
        root: &BoxNode,
        vertical: bool,
    ) -> Result<DirectStreamState, DviError> {
        self.cur_h = h_offset;
        self.cur_v = root
            .height
            .checked_add(v_offset)
            .ok_or(DviError::PositionOverflow)?;
        let mut state = DirectStreamState { frames: Vec::new() };
        self.enter_direct_frame(&mut state, root, vertical, DirectContinuation::Root)?;
        Ok(state)
    }

    fn enter_direct_frame(
        &mut self,
        state: &mut DirectStreamState,
        fields: &BoxNode,
        vertical: bool,
        continuation: DirectContinuation,
    ) -> Result<(), DviError> {
        self.enter_box();
        if self.cur_s > 0 {
            self.u8(PUSH);
        }
        let save_loc = self.bytes.len();
        let axis = if vertical {
            let left_edge = self.cur_h;
            self.cur_v = sub_scaled(self.cur_v, fields.height)?;
            DirectAxis::V {
                left_edge,
                top_edge: self.cur_v,
                cur_g: Scaled::from_raw(0),
                cur_glue: Scaled::from_raw(0),
            }
        } else {
            DirectAxis::H {
                base_line: self.cur_v,
                left_edge: self.cur_h,
                cur_g: Scaled::from_raw(0),
                cur_glue: Scaled::from_raw(0),
            }
        };
        state.frames.push(DirectFrame {
            fields: fields.clone(),
            save_loc,
            axis,
            continuation,
        });
        Ok(())
    }

    pub(super) fn direct_begin_box(
        &mut self,
        state: &mut DirectStreamState,
        fields: &BoxNode,
        vertical: bool,
        empty: bool,
    ) -> Result<bool, DviError> {
        let parent = state.frames.last().expect("direct stream has a root frame");
        if empty {
            match parent.axis {
                DirectAxis::H { .. } => self.cur_h = add_scaled(self.cur_h, fields.width)?,
                DirectAxis::V { .. } => {
                    self.cur_v = add_scaled(add_scaled(self.cur_v, fields.height)?, fields.depth)?;
                }
            }
            return Ok(false);
        }

        let continuation = match parent.axis {
            DirectAxis::H { base_line, .. } => {
                let continuation = DirectContinuation::H {
                    save_h: self.dvi_h,
                    save_v: self.dvi_v,
                    edge: self.cur_h,
                    base_line,
                    width: fields.width,
                };
                self.cur_v = add_scaled(base_line, fields.shift)?;
                continuation
            }
            DirectAxis::V { left_edge, .. } => {
                self.cur_v = add_scaled(self.cur_v, fields.height)?;
                self.synch_v()?;
                let continuation = DirectContinuation::V {
                    save_h: self.dvi_h,
                    save_v: self.dvi_v,
                    left_edge,
                    depth: fields.depth,
                };
                self.cur_h = add_scaled(left_edge, fields.shift)?;
                continuation
            }
        };
        self.enter_direct_frame(state, fields, vertical, continuation)?;
        Ok(true)
    }

    pub(super) fn direct_end_box(&mut self, state: &mut DirectStreamState) -> Result<(), DviError> {
        let frame = state.frames.pop().expect("direct stream box is balanced");
        self.prune_movements(frame.save_loc);
        if self.cur_s > 0 {
            self.dvi_pop(frame.save_loc);
        }
        self.cur_s -= 1;
        match frame.continuation {
            DirectContinuation::Root => {}
            DirectContinuation::H {
                save_h,
                save_v,
                edge,
                base_line,
                width,
            } => {
                self.dvi_h = save_h;
                self.dvi_v = save_v;
                self.cur_h = add_scaled(edge, width)?;
                self.cur_v = base_line;
            }
            DirectContinuation::V {
                save_h,
                save_v,
                left_edge,
                depth,
            } => {
                self.dvi_h = save_h;
                self.dvi_v = save_v;
                self.cur_v = add_scaled(save_v, depth)?;
                self.cur_h = left_edge;
            }
        }
        Ok(())
    }

    pub(super) fn finish_direct_stream(
        &mut self,
        mut state: DirectStreamState,
    ) -> Result<(), DviError> {
        if state.frames.len() != 1 {
            return Err(DviError::Artifact {
                message: "unbalanced direct page box events".to_owned(),
            });
        }
        self.direct_end_box(&mut state)
    }

    pub(super) fn direct_char(
        &mut self,
        state: &mut DirectStreamState,
        font_id: u32,
        ch: u32,
        width: Scaled,
    ) -> Result<(), DviError> {
        let frame = state.frames.last().expect("direct stream has a root frame");
        if let DirectAxis::H { base_line, .. } = frame.axis {
            self.synch_h()?;
            self.synch_v()?;
            self.change_font(font_id)?;
            self.set_char(ch)?;
            self.cur_h = add_scaled(self.cur_h, width)?;
            self.dvi_h = self.cur_h;
            self.cur_v = base_line;
        }
        Ok(())
    }

    pub(super) fn direct_kern(
        &mut self,
        state: &DirectStreamState,
        amount: Scaled,
    ) -> Result<(), DviError> {
        match state
            .frames
            .last()
            .expect("direct stream has a root frame")
            .axis
        {
            DirectAxis::H { .. } => self.cur_h = add_scaled(self.cur_h, amount)?,
            DirectAxis::V { .. } => self.cur_v = add_scaled(self.cur_v, amount)?,
        }
        Ok(())
    }

    pub(super) fn direct_math(
        &mut self,
        state: &DirectStreamState,
        amount: Scaled,
    ) -> Result<(), DviError> {
        if matches!(
            state
                .frames
                .last()
                .expect("direct stream has a root frame")
                .axis,
            DirectAxis::H { .. }
        ) {
            self.cur_h = add_scaled(self.cur_h, amount)?;
        }
        Ok(())
    }

    pub(super) fn direct_rule(
        &mut self,
        state: &DirectStreamState,
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
    ) -> Result<(), DviError> {
        let frame = state.frames.last().expect("direct stream has a root frame");
        match frame.axis {
            DirectAxis::H { base_line, .. } => {
                let rule_ht = height.unwrap_or(frame.fields.height);
                let rule_dp = depth.unwrap_or(frame.fields.depth);
                let rule_wd = width.unwrap_or(Scaled::from_raw(0));
                self.output_rule_in_hlist(rule_ht, rule_dp, rule_wd, base_line)?;
                self.cur_h = add_scaled(self.cur_h, rule_wd)?;
                self.cur_v = base_line;
            }
            DirectAxis::V { .. } => {
                let rule_ht = add_scaled(
                    height.unwrap_or(Scaled::from_raw(0)),
                    depth.unwrap_or(Scaled::from_raw(0)),
                )?;
                self.output_rule_in_vlist(rule_ht, width.unwrap_or(frame.fields.width))?;
            }
        }
        Ok(())
    }

    pub(super) fn direct_glue(
        &mut self,
        state: &mut DirectStreamState,
        spec: crate::GlueSpec,
    ) -> Result<(), DviError> {
        let frame = state
            .frames
            .last_mut()
            .expect("direct stream has a root frame");
        match &mut frame.axis {
            DirectAxis::H {
                base_line,
                cur_g,
                cur_glue,
                ..
            } => {
                let width = adjusted_glue_width(
                    spec,
                    frame.fields.glue_sign,
                    frame.fields.glue_order,
                    frame.fields.glue_set,
                    cur_glue,
                    cur_g,
                )?;
                self.cur_h = add_scaled(self.cur_h, width)?;
                self.cur_v = *base_line;
            }
            DirectAxis::V {
                cur_g, cur_glue, ..
            } => {
                let height = adjusted_glue_width(
                    spec,
                    frame.fields.glue_sign,
                    frame.fields.glue_order,
                    frame.fields.glue_set,
                    cur_glue,
                    cur_g,
                )?;
                self.cur_v = add_scaled(self.cur_v, height)?;
            }
        }
        Ok(())
    }

    pub(super) fn direct_owned_leader(
        &mut self,
        state: &mut DirectStreamState,
        effects: &[PageEffect],
        node: &PageNode,
    ) -> Result<(), DviError> {
        let frame = state
            .frames
            .last_mut()
            .expect("direct stream has a root frame");
        match &mut frame.axis {
            DirectAxis::H {
                base_line,
                left_edge,
                cur_g,
                cur_glue,
            } => {
                self.output_hlist_child(
                    effects,
                    &frame.fields,
                    node,
                    *base_line,
                    *left_edge,
                    cur_g,
                    cur_glue,
                )?;
                self.cur_v = *base_line;
            }
            DirectAxis::V {
                left_edge,
                top_edge,
                cur_g,
                cur_glue,
            } => {
                self.output_vlist_child(
                    effects,
                    &frame.fields,
                    node,
                    *left_edge,
                    *top_edge,
                    cur_g,
                    cur_glue,
                )?;
            }
        }
        Ok(())
    }

    pub(super) fn direct_whatsit(
        &mut self,
        state: &DirectStreamState,
        effects: &[PageEffect],
        effect_index: u32,
    ) -> Result<(), DviError> {
        self.out_what(effects, effect_index)?;
        if let DirectAxis::H { base_line, .. } = state
            .frames
            .last()
            .expect("direct stream has a root frame")
            .axis
        {
            self.cur_v = base_line;
        }
        Ok(())
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
        self.trace_box(false, this_box)?;
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
                PageNode::Char { font_id, ch, width } => {
                    self.trace_glyph(*font_id, &[*ch])?;
                    self.synch_h()?;
                    self.synch_v()?;
                    self.change_font(*font_id)?;
                    self.set_char(*ch)?;
                    self.cur_h = add_scaled(self.cur_h, *width)?;
                    self.dvi_h = self.cur_h;
                }
                PageNode::Lig {
                    font_id,
                    ch,
                    left,
                    right,
                    width,
                } => {
                    self.trace_glyph(*font_id, &[*left, *right])?;
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
        self.trace_box(true, this_box)?;
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
        let total = add_scaled(rule_ht, rule_dp)?;
        if total.raw() > 0 && rule_wd.raw() > 0 {
            if let Some(trace) = &mut self.coordinate_trace {
                trace.push(DviCoordinateEvent::Rule {
                    x: self.cur_h,
                    y: sub_scaled(base_line, rule_ht)?,
                    width: rule_wd,
                    height: total,
                });
            }
            self.synch_h()?;
            self.cur_v = add_scaled(base_line, rule_dp)?;
            self.synch_v()?;
            self.u8(SET_RULE);
            self.scaled(total);
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
        let top = self.cur_v;
        self.cur_v = add_scaled(self.cur_v, rule_ht)?;
        if rule_ht.raw() > 0 && rule_wd.raw() > 0 {
            if let Some(trace) = &mut self.coordinate_trace {
                trace.push(DviCoordinateEvent::Rule {
                    x: self.cur_h,
                    y: top,
                    width: rule_wd,
                    height: rule_ht,
                });
            }
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
            if let Some(trace) = &mut self.coordinate_trace {
                trace.push(DviCoordinateEvent::Special {
                    x: self.cur_h,
                    y: self.cur_v,
                    payload: payload.clone(),
                });
            }
            self.special_out(payload)?;
        }
        Ok(())
    }

    fn trace_box(&mut self, vertical: bool, node: &BoxNode) -> Result<(), DviError> {
        if let Some(trace) = &mut self.coordinate_trace {
            trace.push(DviCoordinateEvent::Box {
                vertical,
                x: self.cur_h,
                y: sub_scaled(self.cur_v, node.height)?,
                width: node.width,
                height: add_scaled(node.height, node.depth)?,
                baseline: self.cur_v,
            });
        }
        Ok(())
    }

    fn trace_glyph(&mut self, font_id: u32, source_codes: &[u32]) -> Result<(), DviError> {
        if let Some(trace) = &mut self.coordinate_trace {
            let source_codes = source_codes
                .iter()
                .map(|code| {
                    u8::try_from(*code).map_err(|_| DviError::CharacterOutOfRange { ch: *code })
                })
                .collect::<Result<Vec<_>, _>>()?;
            trace.push(DviCoordinateEvent::Glyph {
                x: self.cur_h,
                baseline: self.cur_v,
                font_id,
                source_codes,
            });
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
