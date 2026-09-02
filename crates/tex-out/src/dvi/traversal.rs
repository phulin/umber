use tex_arith::Scaled;

use crate::geometry::predict_snap_correction;
use crate::{BoxNode, PageEffect, PageNode};

use super::{
    DviBodyCompiler, DviError,
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
    HLeader {
        save_h: Scaled,
        save_v: Scaled,
        base_line: Scaled,
        width: Scaled,
        extra: Scaled,
    },
    VLeader {
        save_h: Scaled,
        save_v: Scaled,
        left_edge: Scaled,
        height: Scaled,
        depth: Scaled,
        extra: Scaled,
    },
}

impl DviBodyCompiler {
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
        if self.cur_s > 0 {
            self.dvi_pop_save_locs.push(save_loc);
        }
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
            fields: BoxNode {
                width: fields.width,
                height: fields.height,
                depth: fields.depth,
                shift: fields.shift,
                glue_set: fields.glue_set,
                glue_sign: fields.glue_sign,
                glue_order: fields.glue_order,
                children: Vec::new(),
            },
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
            DirectContinuation::HLeader {
                save_h,
                save_v,
                base_line,
                width,
                extra,
            } => {
                self.dvi_h = save_h;
                self.dvi_v = save_v;
                self.cur_h = add_scaled(add_scaled(save_h, width)?, extra)?;
                self.cur_v = base_line;
            }
            DirectContinuation::VLeader {
                save_h,
                save_v,
                left_edge,
                height,
                depth,
                extra,
            } => {
                self.dvi_h = save_h;
                self.dvi_v = save_v;
                self.cur_h = left_edge;
                self.cur_v = add_scaled(
                    sub_scaled(save_v, height)?,
                    add_scaled(add_scaled(height, depth)?, extra)?,
                )?;
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

    #[inline]
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

    #[inline]
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
        self.direct_owned_list(state, std::slice::from_ref(node), effects)
    }

    pub(super) fn direct_owned_node(
        &mut self,
        state: &mut DirectStreamState,
        node: &PageNode,
        effects: &[PageEffect],
    ) -> Result<(), DviError> {
        match node {
            PageNode::Char { font_id, ch, width } => self.direct_char(state, *font_id, *ch, *width),
            PageNode::Lig {
                font_id, ch, width, ..
            } => self.direct_char(state, *font_id, *ch, *width),
            PageNode::Kern { amount, .. } | PageNode::MarginKern { amount, .. } => {
                self.direct_kern(state, *amount)
            }
            PageNode::Glue {
                leader: None, spec, ..
            } => self.direct_glue(state, *spec),
            PageNode::Glue {
                leader: Some(_), ..
            } => self.direct_owned_leader(state, effects, node),
            PageNode::Penalty(_)
            | PageNode::Disc { .. }
            | PageNode::Mark { .. }
            | PageNode::Insert { .. }
            | PageNode::Adjust(_) => Ok(()),
            PageNode::Rule {
                width,
                height,
                depth,
            } => self.direct_rule(state, *width, *height, *depth),
            PageNode::HList(box_node) | PageNode::VList(box_node) => {
                let _ = box_node;
                self.direct_owned_list(state, std::slice::from_ref(node), effects)
            }
            PageNode::WhatsitAnchor { effect_index } => {
                self.direct_whatsit(state, effects, *effect_index)
            }
            PageNode::MathOn(width) | PageNode::MathOff(width) => self.direct_math(state, *width),
        }
    }

    pub(super) fn direct_owned_list(
        &mut self,
        state: &mut DirectStreamState,
        nodes: &[PageNode],
        effects: &[PageEffect],
    ) -> Result<(), DviError> {
        enum Action<'a> {
            Node {
                node: &'a PageNode,
                following: &'a [PageNode],
            },
            EndBox,
            HLeader(leaders::HLeaderRepeat<'a>),
            VLeader(leaders::VLeaderRepeat<'a>),
        }

        fn schedule<'a>(pending: &mut Vec<Action<'a>>, nodes: &'a [PageNode]) {
            pending.extend(
                nodes
                    .iter()
                    .enumerate()
                    .rev()
                    .map(|(index, node)| Action::Node {
                        node,
                        following: &nodes[index + 1..],
                    }),
            );
        }

        let mut pending = Vec::new();
        schedule(&mut pending, nodes);
        while let Some(action) = pending.pop() {
            let (node, following) = match action {
                Action::EndBox => {
                    self.direct_end_box(state)?;
                    continue;
                }
                Action::HLeader(repeat) => {
                    let width = repeat.box_node.width;
                    if add_scaled(self.cur_h, width)?.raw() > repeat.edge.raw() {
                        self.cur_h =
                            sub_scaled(repeat.edge, crate::geometry::LEADER_ROUNDING_COMPENSATION)?;
                        continue;
                    }
                    self.cur_v = add_scaled(repeat.base_line, repeat.box_node.shift)?;
                    self.synch_v()?;
                    let save_v = self.dvi_v;
                    self.synch_h()?;
                    let save_h = self.dvi_h;
                    self.enter_direct_frame(
                        state,
                        repeat.box_node,
                        matches!(repeat.leader, crate::LeaderPayload::VList(_)),
                        DirectContinuation::HLeader {
                            save_h,
                            save_v,
                            base_line: repeat.base_line,
                            width,
                            extra: repeat.extra,
                        },
                    )?;
                    pending.push(Action::HLeader(repeat));
                    pending.push(Action::EndBox);
                    schedule(&mut pending, &repeat.box_node.children);
                    continue;
                }
                Action::VLeader(repeat) => {
                    let height = repeat.box_node.height;
                    let depth = repeat.box_node.depth;
                    let size = add_scaled(height, depth)?;
                    if add_scaled(self.cur_v, size)?.raw() > repeat.edge.raw() {
                        self.cur_v =
                            sub_scaled(repeat.edge, crate::geometry::LEADER_ROUNDING_COMPENSATION)?;
                        continue;
                    }
                    self.cur_h = add_scaled(repeat.left_edge, repeat.box_node.shift)?;
                    self.synch_h()?;
                    let save_h = self.dvi_h;
                    self.cur_v = add_scaled(self.cur_v, height)?;
                    self.synch_v()?;
                    let save_v = self.dvi_v;
                    self.enter_direct_frame(
                        state,
                        repeat.box_node,
                        matches!(repeat.leader, crate::LeaderPayload::VList(_)),
                        DirectContinuation::VLeader {
                            save_h,
                            save_v,
                            left_edge: repeat.left_edge,
                            height,
                            depth,
                            extra: repeat.extra,
                        },
                    )?;
                    pending.push(Action::VLeader(repeat));
                    pending.push(Action::EndBox);
                    schedule(&mut pending, &repeat.box_node.children);
                    continue;
                }
                Action::Node { node, following } => (node, following),
            };
            if let PageNode::WhatsitAnchor { effect_index } = node {
                let frame = state.frames.last().expect("direct stream has a root frame");
                if let DirectAxis::V {
                    cur_g, cur_glue, ..
                } = frame.axis
                {
                    self.out_what_v(
                        effects,
                        *effect_index,
                        following,
                        &frame.fields,
                        cur_g,
                        cur_glue,
                    )?;
                    continue;
                }
            }
            match node {
                PageNode::HList(box_node) | PageNode::VList(box_node) => {
                    let entered = self.direct_begin_box(
                        state,
                        box_node,
                        matches!(node, PageNode::VList(_)),
                        box_node.children.is_empty(),
                    )?;
                    if entered {
                        pending.push(Action::EndBox);
                        schedule(&mut pending, &box_node.children);
                    }
                }
                PageNode::Glue { spec, kind, leader } if leader.is_some() => {
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
                            let width = adjusted_glue_width(
                                *spec,
                                frame.fields.glue_sign,
                                frame.fields.glue_order,
                                frame.fields.glue_set,
                                cur_glue,
                                cur_g,
                            )?;
                            if let Some(repeat) =
                                self.move_right_or_output_leaders(leaders::HLeaderContext {
                                    this_box: &frame.fields,
                                    kind: *kind,
                                    leader,
                                    rule_wd: width,
                                    left_edge: *left_edge,
                                    base_line: *base_line,
                                })?
                            {
                                pending.push(Action::HLeader(repeat));
                            }
                            self.cur_v = *base_line;
                        }
                        DirectAxis::V {
                            left_edge,
                            top_edge,
                            cur_g,
                            cur_glue,
                        } => {
                            let height = adjusted_glue_width(
                                *spec,
                                frame.fields.glue_sign,
                                frame.fields.glue_order,
                                frame.fields.glue_set,
                                cur_glue,
                                cur_g,
                            )?;
                            if let Some(repeat) =
                                self.move_down_or_output_leaders(leaders::VLeaderContext {
                                    this_box: &frame.fields,
                                    kind: *kind,
                                    leader,
                                    rule_ht: height,
                                    left_edge: *left_edge,
                                    top_edge: *top_edge,
                                })?
                            {
                                pending.push(Action::VLeader(repeat));
                            }
                        }
                    }
                }
                _ => self.direct_owned_node(state, node, effects)?,
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
    pub(super) fn output_rule_in_hlist(
        &mut self,
        rule_ht: Scaled,
        rule_dp: Scaled,
        rule_wd: Scaled,
        base_line: Scaled,
    ) -> Result<(), DviError> {
        let total = add_scaled(rule_ht, rule_dp)?;
        if total.raw() > 0 && rule_wd.raw() > 0 {
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

    fn dvi_pop(&mut self, _save_loc: usize) {
        // Record the unoptimized command stream. The final file assembler
        // owns the absolute DVI address needed by pdfTeX WEB §628's
        // `dvi_ptr > 0` condition and performs the cancellation there.
        self.dvi_pop_sites.push(super::plan::DviPopSite {
            pop_offset: self.bytes.len(),
        });
        self.u8(POP);
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

    fn out_what_v(
        &mut self,
        effects: &[PageEffect],
        effect_index: u32,
        following: &[PageNode],
        this_box: &BoxNode,
        cur_g: Scaled,
        cur_glue: Scaled,
    ) -> Result<(), DviError> {
        let effect = effects
            .get(effect_index as usize)
            .ok_or(DviError::MissingEffect { effect_index })?;
        match effect {
            PageEffect::PdfSnapRefPoint => self.snap_reference = (self.cur_h, self.cur_v),
            PageEffect::PdfSnapY { spec } => {
                if let Some(delta) =
                    crate::snapping::correction(self.cur_v, self.snap_reference.1, *spec)
                {
                    self.cur_v = add_scaled(self.cur_v, delta)?;
                }
            }
            PageEffect::PdfSnapYComp { ratio } => {
                if let Some(delta) = predict_snap_correction(
                    following,
                    effects,
                    this_box,
                    self.cur_v,
                    self.snap_reference,
                    cur_g,
                    cur_glue,
                )? {
                    self.cur_v =
                        add_scaled(self.cur_v, crate::snapping::compensate(delta, *ratio))?;
                }
            }
            _ => self.out_what(effects, effect_index)?,
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
