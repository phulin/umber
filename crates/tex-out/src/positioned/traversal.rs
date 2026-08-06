use tex_arith::Scaled;

use crate::dvi::glue::adjusted_glue_width;
use crate::geometry::{
    LEADER_ROUNDING_COMPENSATION, LeaderMode, NodeOrdinals, leader_start, predict_snap_correction,
};
use crate::{BoxNode, GlueKind, KernKind, LeaderPayload, PageArtifact, PageEffect, PageNode};

use super::{
    BoxKind, PositionedBox, PositionedBoxEnd, PositionedError, PositionedEvent, PositionedLimits,
    PositionedPage, PositionedPdfAccessibility, PositionedPdfAnnotation, PositionedPdfDestination,
    PositionedPdfGraphics, PositionedPdfThread, PositionedRule, PositionedSourceRef,
    PositionedSpecial, PositionedTextRun, TextUnit,
};

pub(super) fn lower(
    page: &PageArtifact,
    page_index: u32,
    limits: PositionedLimits,
    require_balanced_pdf_saves: bool,
) -> Result<PositionedPage, PositionedError> {
    if page.job.mag <= 0 {
        return Err(PositionedError::InvalidMagnification { mag: page.job.mag });
    }
    let (root, kind) = match &page.root {
        PageNode::HList(root) => (root, BoxKind::Horizontal),
        PageNode::VList(root) => (root, BoxKind::Vertical),
        _ => unreachable!("validated artifact root is a box"),
    };
    let height = add(root.height, root.depth)?;
    let content_origin_x = add(page.job.page_origin_x, page.job.h_offset)?;
    let content_origin_y = add(page.job.page_origin_y, page.job.v_offset)?;
    let content_right = add(content_origin_x, root.width)?;
    let content_bottom = add(content_origin_y, height)?;
    let right = if page.job.page_width.raw() > 0 {
        page.job.page_width
    } else {
        Scaled::from_raw(add(content_right, content_origin_x)?.raw().max(0))
    };
    let bottom = if page.job.page_height.raw() > 0 {
        page.job.page_height
    } else {
        Scaled::from_raw(add(content_bottom, content_origin_y)?.raw().max(0))
    };
    let mut out = Lowerer {
        effects: &page.effects,
        events: Vec::new(),
        limits,
        cur_h: page.job.h_offset,
        cur_v: add(root.height, page.job.v_offset)?,
        current_font_id: None,
        node_ordinals: NodeOrdinals::new(&page.root),
        next_box_id: 0,
        box_stack: Vec::new(),
        pdf_save_positions: Vec::new(),
        diagnostics: Vec::new(),
        last_saved_position: None,
        snap_reference: crate::snapping::initial_reference(&page.effects),
    };
    match kind {
        BoxKind::Horizontal => out.hlist(root, 1)?,
        BoxKind::Vertical => out.vlist(root, 1)?,
    }
    if require_balanced_pdf_saves && !out.pdf_save_positions.is_empty() {
        return Err(PositionedError::UnmatchedPdfSaves {
            count: out.pdf_save_positions.len(),
        });
    }
    if out
        .events
        .len()
        .checked_add(page.math_events.len())
        .is_none_or(|count| count > limits.max_events)
    {
        return Err(PositionedError::TooManyEvents {
            limit: limits.max_events,
        });
    }
    Ok(PositionedPage {
        page_index,
        width: right,
        height: bottom,
        page_origin_x: page.job.page_origin_x,
        page_origin_y: page.job.page_origin_y,
        mag: page.job.mag,
        counts: page.counts,
        fonts: page.fonts.clone(),
        events: out.events,
        math_events: page.math_events.clone(),
        diagnostics: out.diagnostics,
        last_saved_position: out.last_saved_position,
        snap_reference: out.snap_reference,
    })
}

struct Lowerer<'a> {
    effects: &'a [PageEffect],
    events: Vec<PositionedEvent>,
    limits: PositionedLimits,
    cur_h: Scaled,
    cur_v: Scaled,
    current_font_id: Option<u32>,
    node_ordinals: NodeOrdinals,
    next_box_id: u32,
    box_stack: Vec<u32>,
    pdf_save_positions: Vec<(Scaled, Scaled)>,
    diagnostics: Vec<String>,
    last_saved_position: Option<(Scaled, Scaled)>,
    snap_reference: (Scaled, Scaled),
}

struct PositionedFrame<'a> {
    node: &'a BoxNode,
    depth: usize,
    box_id: u32,
    index: usize,
    axis: PositionedAxis,
    continuation: PositionedContinuation,
}

enum PositionedAxis {
    Horizontal {
        base_line: Scaled,
        left_edge: Scaled,
        cur_g: Scaled,
        cur_glue: Scaled,
        run: RunBuilder,
    },
    Vertical {
        left_edge: Scaled,
        top_edge: Scaled,
        cur_g: Scaled,
        cur_glue: Scaled,
    },
}

#[derive(Clone, Copy)]
enum PositionedContinuation {
    Root,
    Horizontal {
        edge: Scaled,
        base_line: Scaled,
        width: Scaled,
    },
    Vertical {
        baseline: Scaled,
        left_edge: Scaled,
        depth: Scaled,
    },
}

impl Lowerer<'_> {
    fn node_ordinal(&self, node: &PageNode) -> u32 {
        self.node_ordinals.get(node)
    }

    fn push(&mut self, event: PositionedEvent) -> Result<(), PositionedError> {
        if self.events.len() >= self.limits.max_events {
            return Err(PositionedError::TooManyEvents {
                limit: self.limits.max_events,
            });
        }
        self.events.push(event);
        Ok(())
    }

    fn check_depth(&self, depth: usize) -> Result<(), PositionedError> {
        if depth > self.limits.max_depth {
            Err(PositionedError::NestingTooDeep {
                limit: self.limits.max_depth,
            })
        } else {
            Ok(())
        }
    }

    fn hlist(&mut self, this_box: &BoxNode, depth: usize) -> Result<(), PositionedError> {
        self.walk_box(this_box, false, depth)
    }

    fn vlist(&mut self, this_box: &BoxNode, depth: usize) -> Result<(), PositionedError> {
        self.walk_box(this_box, true, depth)
    }

    fn walk_box(
        &mut self,
        root: &BoxNode,
        vertical: bool,
        depth: usize,
    ) -> Result<(), PositionedError> {
        let mut frames = Vec::new();
        self.enter_frame(
            &mut frames,
            root,
            vertical,
            depth,
            PositionedContinuation::Root,
        )?;
        while let Some(mut frame) = frames.pop() {
            if frame.index == frame.node.children.len() {
                if let PositionedAxis::Horizontal { run, .. } = &mut frame.axis {
                    run.flush(self)?;
                }
                self.end_box(frame.box_id, frame.depth)?;
                self.restore_after_frame(frame.continuation)?;
                continue;
            }

            let index = frame.index;
            frame.index += 1;
            let child = &frame.node.children[index];
            let mut child_frame = None;
            match &mut frame.axis {
                PositionedAxis::Horizontal {
                    base_line,
                    left_edge,
                    cur_g,
                    cur_glue,
                    run,
                } => {
                    let base_line = *base_line;
                    let left_edge = *left_edge;
                    let node_ordinal = self.node_ordinal(child);
                    match child {
                        PageNode::Char { font_id, ch, width } => {
                            if run.font_id.is_some_and(|current| current != *font_id) {
                                run.resolve_pending_space(self.limits)?;
                                run.flush(self)?;
                            }
                            run.character(
                                *font_id,
                                CharacterUnit {
                                    source_code: *ch,
                                    physical_code: u8::try_from(*ch).ok(),
                                    source: PositionedSourceRef {
                                        node_ordinal,
                                        source_index: 0,
                                    },
                                },
                                self.cur_h,
                                base_line,
                                self.limits,
                            )?;
                            self.current_font_id = Some(*font_id);
                            self.cur_h = add(self.cur_h, *width)?;
                        }
                        PageNode::Lig {
                            font_id,
                            ch,
                            source,
                            width,
                            ..
                        } => {
                            if run.font_id.is_some_and(|current| current != *font_id) {
                                run.resolve_pending_space(self.limits)?;
                                run.flush(self)?;
                            }
                            for (source_index, code) in source.iter().enumerate() {
                                run.character(
                                    *font_id,
                                    CharacterUnit {
                                        source_code: *code,
                                        physical_code: (source_index == 0)
                                            .then(|| u8::try_from(*ch).ok())
                                            .flatten(),
                                        source: PositionedSourceRef {
                                            node_ordinal,
                                            source_index: u16::try_from(source_index).map_err(
                                                |_| PositionedError::TextRunTooLong {
                                                    limit: self.limits.max_run_units,
                                                },
                                            )?,
                                        },
                                    },
                                    self.cur_h,
                                    base_line,
                                    self.limits,
                                )?;
                            }
                            self.current_font_id = Some(*font_id);
                            self.cur_h = add(self.cur_h, *width)?;
                        }
                        PageNode::Kern { amount, kind } => {
                            if !matches!(kind, KernKind::Font | KernKind::Auto) {
                                run.flush(self)?;
                            }
                            self.cur_h = add(self.cur_h, *amount)?;
                        }
                        PageNode::MarginKern { amount, .. } => {
                            run.flush(self)?;
                            self.cur_h = add(self.cur_h, *amount)?;
                        }
                        PageNode::Glue { spec, kind, leader } => {
                            let width = glue_width(frame.node, *spec, cur_glue, cur_g)?;
                            if leader.is_none()
                                && !matches!(
                                    kind,
                                    GlueKind::Leaders | GlueKind::Cleaders | GlueKind::Xleaders
                                )
                            {
                                run.pending_space(self.current_font_id, self.cur_h, base_line);
                                self.cur_h = add(self.cur_h, width)?;
                            } else {
                                run.flush(self)?;
                                self.hleaders(
                                    frame.node,
                                    *kind,
                                    leader,
                                    width,
                                    left_edge,
                                    base_line,
                                    frame.depth,
                                )?;
                            }
                        }
                        PageNode::HList(node) | PageNode::VList(node) => {
                            run.flush(self)?;
                            if node.children.is_empty() {
                                self.cur_h = add(self.cur_h, node.width)?;
                            } else {
                                let edge = self.cur_h;
                                self.cur_v = add(base_line, node.shift)?;
                                child_frame = Some((
                                    node,
                                    matches!(child, PageNode::VList(_)),
                                    PositionedContinuation::Horizontal {
                                        edge,
                                        base_line,
                                        width: node.width,
                                    },
                                ));
                            }
                        }
                        PageNode::Rule {
                            width,
                            height,
                            depth,
                        } => {
                            run.flush(self)?;
                            let height = height.unwrap_or(frame.node.height);
                            let depth = depth.unwrap_or(frame.node.depth);
                            let width = width.unwrap_or(Scaled::from_raw(0));
                            self.rule_h(height, depth, width, base_line)?;
                            self.cur_h = add(self.cur_h, width)?;
                        }
                        PageNode::MathOn(width) | PageNode::MathOff(width) => {
                            run.flush(self)?;
                            self.cur_h = add(self.cur_h, *width)?;
                        }
                        PageNode::WhatsitAnchor { effect_index } => {
                            run.flush(self)?;
                            self.special_h(*effect_index, frame.depth)?;
                        }
                        PageNode::Penalty(_)
                        | PageNode::Disc { .. }
                        | PageNode::Mark { .. }
                        | PageNode::Insert { .. }
                        | PageNode::Adjust(_) => run.flush(self)?,
                    }
                    if child_frame.is_none() {
                        self.cur_v = base_line;
                    }
                }
                PositionedAxis::Vertical {
                    left_edge,
                    top_edge,
                    cur_g,
                    cur_glue,
                } => {
                    let left_edge = *left_edge;
                    match child {
                        PageNode::HList(node) | PageNode::VList(node) => {
                            if node.children.is_empty() {
                                self.cur_v = add(add(self.cur_v, node.height)?, node.depth)?;
                            } else {
                                self.cur_v = add(self.cur_v, node.height)?;
                                let baseline = self.cur_v;
                                self.cur_h = add(left_edge, node.shift)?;
                                child_frame = Some((
                                    node,
                                    matches!(child, PageNode::VList(_)),
                                    PositionedContinuation::Vertical {
                                        baseline,
                                        left_edge,
                                        depth: node.depth,
                                    },
                                ));
                            }
                        }
                        PageNode::Rule {
                            width,
                            height,
                            depth,
                        } => {
                            let height = add(
                                height.unwrap_or(Scaled::from_raw(0)),
                                depth.unwrap_or(Scaled::from_raw(0)),
                            )?;
                            self.rule_v(height, width.unwrap_or(frame.node.width))?;
                        }
                        PageNode::Glue { spec, kind, leader } => {
                            let height = glue_width(frame.node, *spec, cur_glue, cur_g)?;
                            self.vleaders(
                                frame.node,
                                *kind,
                                leader,
                                height,
                                left_edge,
                                *top_edge,
                                frame.depth,
                            )?;
                        }
                        PageNode::Kern { amount, .. } | PageNode::MarginKern { amount, .. } => {
                            self.cur_v = add(self.cur_v, *amount)?;
                        }
                        PageNode::WhatsitAnchor { effect_index } => self.special_v(
                            *effect_index,
                            &frame.node.children[index + 1..],
                            frame.node,
                            *cur_g,
                            *cur_glue,
                            frame.depth,
                        )?,
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
            }
            let child_depth = frame.depth + 1;
            frames.push(frame);
            if let Some((node, vertical, continuation)) = child_frame {
                self.enter_frame(&mut frames, node, vertical, child_depth, continuation)?;
            }
        }
        Ok(())
    }

    fn enter_frame<'a>(
        &mut self,
        frames: &mut Vec<PositionedFrame<'a>>,
        node: &'a BoxNode,
        vertical: bool,
        depth: usize,
        continuation: PositionedContinuation,
    ) -> Result<(), PositionedError> {
        self.check_depth(depth)?;
        let baseline = self.cur_v;
        let left_edge = self.cur_h;
        let kind = if vertical {
            BoxKind::Vertical
        } else {
            BoxKind::Horizontal
        };
        let box_id = self.box_event(kind, node, left_edge, baseline, depth)?;
        let axis = if vertical {
            self.cur_v = sub(self.cur_v, node.height)?;
            PositionedAxis::Vertical {
                left_edge,
                top_edge: self.cur_v,
                cur_g: Scaled::from_raw(0),
                cur_glue: Scaled::from_raw(0),
            }
        } else {
            PositionedAxis::Horizontal {
                base_line: baseline,
                left_edge,
                cur_g: Scaled::from_raw(0),
                cur_glue: Scaled::from_raw(0),
                run: RunBuilder::default(),
            }
        };
        frames.push(PositionedFrame {
            node,
            depth,
            box_id,
            index: 0,
            axis,
            continuation,
        });
        Ok(())
    }

    fn restore_after_frame(
        &mut self,
        continuation: PositionedContinuation,
    ) -> Result<(), PositionedError> {
        match continuation {
            PositionedContinuation::Root => {}
            PositionedContinuation::Horizontal {
                edge,
                base_line,
                width,
            } => {
                self.cur_h = add(edge, width)?;
                self.cur_v = base_line;
            }
            PositionedContinuation::Vertical {
                baseline,
                left_edge,
                depth,
            } => {
                self.cur_v = add(baseline, depth)?;
                self.cur_h = left_edge;
            }
        }
        Ok(())
    }

    fn box_event(
        &mut self,
        kind: BoxKind,
        node: &BoxNode,
        x: Scaled,
        baseline: Scaled,
        depth: usize,
    ) -> Result<u32, PositionedError> {
        let id = self.next_box_id;
        self.next_box_id =
            self.next_box_id
                .checked_add(1)
                .ok_or(PositionedError::TooManyEvents {
                    limit: self.limits.max_events,
                })?;
        let depth = u32::try_from(depth).map_err(|_| PositionedError::NestingTooDeep {
            limit: self.limits.max_depth,
        })?;
        self.push(PositionedEvent::Box(PositionedBox {
            id,
            depth,
            kind,
            x,
            y: sub(baseline, node.height)?,
            width: node.width,
            height: add(node.height, node.depth)?,
            baseline,
        }))?;
        self.box_stack.push(id);
        Ok(id)
    }

    fn end_box(&mut self, id: u32, depth: usize) -> Result<(), PositionedError> {
        debug_assert_eq!(self.box_stack.pop(), Some(id));
        self.push(PositionedEvent::BoxEnd(PositionedBoxEnd {
            id,
            depth: u32::try_from(depth).map_err(|_| PositionedError::NestingTooDeep {
                limit: self.limits.max_depth,
            })?,
        }))
    }

    fn rule_h(
        &mut self,
        height: Scaled,
        depth: Scaled,
        width: Scaled,
        baseline: Scaled,
    ) -> Result<(), PositionedError> {
        let total = add(height, depth)?;
        if total.raw() > 0 && width.raw() > 0 {
            self.push(PositionedEvent::Rule(PositionedRule {
                x: self.cur_h,
                y: sub(baseline, height)?,
                width,
                height: total,
            }))?;
        }
        Ok(())
    }

    fn rule_v(&mut self, height: Scaled, width: Scaled) -> Result<(), PositionedError> {
        let top = self.cur_v;
        self.cur_v = add(self.cur_v, height)?;
        if height.raw() > 0 && width.raw() > 0 {
            self.push(PositionedEvent::Rule(PositionedRule {
                x: self.cur_h,
                y: top,
                width,
                height,
            }))?;
        }
        Ok(())
    }

    fn special_h(&mut self, effect_index: u32, depth: usize) -> Result<(), PositionedError> {
        self.special_position(effect_index, false, &[], None, depth)
    }

    fn special_v(
        &mut self,
        effect_index: u32,
        following: &[PageNode],
        this_box: &BoxNode,
        cur_g: Scaled,
        cur_glue: Scaled,
        depth: usize,
    ) -> Result<(), PositionedError> {
        self.special_position(
            effect_index,
            true,
            following,
            Some((this_box, cur_g, cur_glue)),
            depth,
        )
    }

    fn special_position(
        &mut self,
        effect_index: u32,
        vertical: bool,
        following: &[PageNode],
        glue_state: Option<(&BoxNode, Scaled, Scaled)>,
        depth: usize,
    ) -> Result<(), PositionedError> {
        let effect = self
            .effects
            .get(effect_index as usize)
            .ok_or(PositionedError::MissingEffect { effect_index })?;
        if let PageEffect::PdfRefXForm {
            width,
            height,
            depth,
            ..
        }
        | PageEffect::PdfRefXImage {
            width,
            height,
            depth,
            ..
        } = effect
        {
            if vertical {
                self.cur_v = add(self.cur_v, *height)?;
            }
            self.push(PositionedEvent::PdfGraphics(PositionedPdfGraphics {
                x: self.cur_h,
                y: self.cur_v,
                effect: effect.clone(),
            }))?;
            if vertical {
                self.cur_v = add(self.cur_v, *depth)?;
            } else {
                self.cur_h = add(self.cur_h, *width)?;
            }
        } else if matches!(effect, PageEffect::PdfSavePosition) {
            self.last_saved_position = Some((self.cur_h, self.cur_v));
        } else if matches!(effect, PageEffect::PdfSnapRefPoint) {
            self.snap_reference = (self.cur_h, self.cur_v);
        } else if let PageEffect::PdfSnapY { spec } = effect {
            if vertical
                && let Some(delta) =
                    crate::snapping::correction(self.cur_v, self.snap_reference.1, *spec)
            {
                self.cur_v = add(self.cur_v, delta)?;
            }
        } else if let PageEffect::PdfSnapYComp { ratio } = effect {
            if vertical
                && let Some((this_box, cur_g, cur_glue)) = glue_state
                && let Some(delta) = predict_snap_correction(
                    following,
                    self.effects,
                    this_box,
                    self.cur_v,
                    self.snap_reference,
                    cur_g,
                    cur_glue,
                )?
            {
                self.cur_v = add(self.cur_v, crate::snapping::compensate(delta, *ratio))?;
            }
        } else if let PageEffect::PdfAccessibility(control) = effect {
            self.push(PositionedEvent::PdfAccessibility(
                PositionedPdfAccessibility {
                    x: self.cur_h,
                    y: self.cur_v,
                    control: *control,
                },
            ))?;
        } else if let PageEffect::PdfAnnotation(marker) = effect {
            self.push(PositionedEvent::PdfAnnotation(PositionedPdfAnnotation {
                x: self.cur_h,
                y: self.cur_v,
                containing_box: *self
                    .box_stack
                    .last()
                    .expect("positioned effects are nested in a box"),
                depth: u32::try_from(depth).map_err(|_| PositionedError::NestingTooDeep {
                    limit: self.limits.max_depth,
                })?,
                marker: *marker,
            }))?;
        } else if let PageEffect::PdfDestination(marker) = effect {
            self.push(PositionedEvent::PdfDestination(PositionedPdfDestination {
                x: self.cur_h,
                y: self.cur_v,
                containing_box: *self
                    .box_stack
                    .last()
                    .expect("positioned effects are nested in a box"),
                marker: marker.clone(),
            }))?;
        } else if let PageEffect::PdfThread(marker) | PageEffect::PdfStartThread(marker) = effect {
            self.push(PositionedEvent::PdfThread(PositionedPdfThread {
                x: self.cur_h,
                y: self.cur_v,
                containing_box: *self
                    .box_stack
                    .last()
                    .expect("positioned effects are nested in a box"),
                running: matches!(effect, PageEffect::PdfStartThread(_)),
                marker: marker.clone(),
            }))?;
        } else if matches!(effect, PageEffect::PdfEndThread) {
            self.push(PositionedEvent::PdfEndThread {
                x: self.cur_h,
                y: self.cur_v,
            })?;
        } else if let PageEffect::Special { class, payload } = effect {
            self.push(PositionedEvent::Special(PositionedSpecial {
                x: self.cur_h,
                y: self.cur_v,
                class: class.clone(),
                payload: payload.clone(),
            }))?;
        } else if matches!(
            effect,
            PageEffect::PdfLiteral { .. }
                | PageEffect::PdfColorStack {
                    page_start: false,
                    ..
                }
                | PageEffect::PdfSetMatrix { .. }
                | PageEffect::PdfSave
                | PageEffect::PdfRestore
        ) {
            match effect {
                PageEffect::PdfSave => self.pdf_save_positions.push((self.cur_h, self.cur_v)),
                PageEffect::PdfRestore => match self.pdf_save_positions.pop() {
                    None => self
                        .diagnostics
                        .push("\\pdfrestore: missing \\pdfsave".to_owned()),
                    Some((x, y)) if x != self.cur_h || y != self.cur_v => {
                        self.diagnostics.push(format!(
                            "Misplaced \\pdfrestore by ({}sp, {}sp)",
                            i64::from(self.cur_h.raw()) - i64::from(x.raw()),
                            i64::from(self.cur_v.raw()) - i64::from(y.raw())
                        ))
                    }
                    Some(_) => {}
                },
                _ => {}
            }
            self.push(PositionedEvent::PdfGraphics(PositionedPdfGraphics {
                x: self.cur_h,
                y: self.cur_v,
                effect: effect.clone(),
            }))?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)] // Mirrors TeX's explicit leader registers.
    fn hleaders(
        &mut self,
        this_box: &BoxNode,
        kind: GlueKind,
        leader: &Option<LeaderPayload>,
        available: Scaled,
        left_edge: Scaled,
        baseline: Scaled,
        depth: usize,
    ) -> Result<(), PositionedError> {
        let Some(kind) = LeaderMode::from_glue(kind) else {
            self.cur_h = add(self.cur_h, available)?;
            return Ok(());
        };
        let Some(leader) = leader else {
            self.cur_h = add(self.cur_h, available)?;
            return Ok(());
        };
        match leader {
            LeaderPayload::Rule { height, depth, .. } => {
                self.rule_h(
                    height.unwrap_or(this_box.height),
                    depth.unwrap_or(this_box.depth),
                    available,
                    baseline,
                )?;
                self.cur_h = add(self.cur_h, available)?;
            }
            LeaderPayload::HList(node) | LeaderPayload::VList(node) => {
                if node.width.raw() <= 0 || available.raw() <= 0 {
                    self.cur_h = add(self.cur_h, available)?;
                    return Ok(());
                }
                let space = add(available, LEADER_ROUNDING_COMPENSATION)?;
                let edge = add(self.cur_h, space)?;
                let (start, extra) = leader_start(kind, self.cur_h, left_edge, space, node.width)?;
                self.cur_h = start;
                while add(self.cur_h, node.width)?.raw() <= edge.raw() {
                    let save_h = self.cur_h;
                    let save_v = self.cur_v;
                    self.cur_v = add(baseline, node.shift)?;
                    if matches!(leader, LeaderPayload::VList(_)) {
                        self.vlist(node, depth + 1)?;
                    } else {
                        self.hlist(node, depth + 1)?;
                    }
                    self.cur_h = add(add(save_h, node.width)?, extra)?;
                    self.cur_v = save_v;
                }
                self.cur_h = sub(edge, LEADER_ROUNDING_COMPENSATION)?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)] // Mirrors TeX's explicit leader registers.
    fn vleaders(
        &mut self,
        this_box: &BoxNode,
        kind: GlueKind,
        leader: &Option<LeaderPayload>,
        available: Scaled,
        left_edge: Scaled,
        top_edge: Scaled,
        depth: usize,
    ) -> Result<(), PositionedError> {
        let Some(kind) = LeaderMode::from_glue(kind) else {
            self.cur_v = add(self.cur_v, available)?;
            return Ok(());
        };
        let Some(leader) = leader else {
            self.cur_v = add(self.cur_v, available)?;
            return Ok(());
        };
        match leader {
            LeaderPayload::Rule { width, .. } => {
                self.rule_v(available, width.unwrap_or(this_box.width))?;
            }
            LeaderPayload::HList(node) | LeaderPayload::VList(node) => {
                let size = add(node.height, node.depth)?;
                if size.raw() <= 0 || available.raw() <= 0 {
                    self.cur_v = add(self.cur_v, available)?;
                    return Ok(());
                }
                let space = add(available, LEADER_ROUNDING_COMPENSATION)?;
                let edge = add(self.cur_v, space)?;
                let (start, extra) = leader_start(kind, self.cur_v, top_edge, space, size)?;
                self.cur_v = start;
                while add(self.cur_v, size)?.raw() <= edge.raw() {
                    let start_v = self.cur_v;
                    let save_h = self.cur_h;
                    self.cur_h = add(left_edge, node.shift)?;
                    self.cur_v = add(start_v, node.height)?;
                    if matches!(leader, LeaderPayload::VList(_)) {
                        self.vlist(node, depth + 1)?;
                    } else {
                        self.hlist(node, depth + 1)?;
                    }
                    self.cur_h = save_h;
                    self.cur_v = add(add(start_v, size)?, extra)?;
                }
                self.cur_v = sub(edge, LEADER_ROUNDING_COMPENSATION)?;
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct RunBuilder {
    font_id: Option<u32>,
    x: Option<Scaled>,
    baseline: Option<Scaled>,
    units: Vec<TextUnit>,
    positions: Vec<Scaled>,
    physical_codes: Vec<Option<u8>>,
    sources: Vec<Option<PositionedSourceRef>>,
    pending_space: Option<Scaled>,
}

struct CharacterUnit {
    source_code: u32,
    physical_code: Option<u8>,
    source: PositionedSourceRef,
}

impl RunBuilder {
    fn character(
        &mut self,
        font_id: u32,
        character: CharacterUnit,
        x: Scaled,
        baseline: Scaled,
        limits: PositionedLimits,
    ) -> Result<(), PositionedError> {
        debug_assert!(self.font_id.is_none_or(|current| current == font_id));
        if self.font_id.is_none() {
            self.font_id = Some(font_id);
            self.x = Some(x);
            self.baseline = Some(baseline);
        }
        self.resolve_pending_space(limits)?;
        self.add_unit(
            TextUnit::Code(character.source_code),
            x,
            character.physical_code,
            Some(character.source),
            limits,
        )
    }

    fn add_unit(
        &mut self,
        unit: TextUnit,
        position: Scaled,
        physical_code: Option<u8>,
        source: Option<PositionedSourceRef>,
        limits: PositionedLimits,
    ) -> Result<(), PositionedError> {
        if self.units.len() >= limits.max_run_units {
            return Err(PositionedError::TextRunTooLong {
                limit: limits.max_run_units,
            });
        }
        self.units.push(unit);
        self.positions.push(position);
        self.physical_codes.push(physical_code);
        self.sources.push(source);
        Ok(())
    }

    fn pending_space(&mut self, font_id: Option<u32>, position: Scaled, baseline: Scaled) {
        if let Some(font_id) = font_id {
            if self.font_id.is_none() {
                self.font_id = Some(font_id);
                self.x = Some(position);
                self.baseline = Some(baseline);
            }
            self.pending_space.get_or_insert(position);
        }
    }

    fn resolve_pending_space(&mut self, limits: PositionedLimits) -> Result<(), PositionedError> {
        if let Some(position) = self.pending_space.take() {
            self.add_unit(TextUnit::Space, position, None, None, limits)?;
        }
        Ok(())
    }

    fn flush(&mut self, lowerer: &mut Lowerer<'_>) -> Result<(), PositionedError> {
        if let (Some(font_id), Some(x), Some(baseline)) =
            (self.font_id.take(), self.x.take(), self.baseline.take())
        {
            let units = std::mem::take(&mut self.units);
            let positions = std::mem::take(&mut self.positions);
            let physical_codes = std::mem::take(&mut self.physical_codes);
            let sources = std::mem::take(&mut self.sources);
            self.pending_space = None;
            lowerer.push(PositionedEvent::TextRun(PositionedTextRun {
                x,
                baseline,
                font_id,
                units,
                positions,
                physical_codes,
                sources,
            }))?;
        }
        Ok(())
    }
}

fn glue_width(
    node: &BoxNode,
    spec: crate::GlueSpec,
    cur_glue: &mut Scaled,
    cur_g: &mut Scaled,
) -> Result<Scaled, PositionedError> {
    adjusted_glue_width(
        spec,
        node.glue_sign,
        node.glue_order,
        node.glue_set,
        cur_glue,
        cur_g,
    )
    .map_err(|_| PositionedError::PositionOverflow)
}

fn add(left: Scaled, right: Scaled) -> Result<Scaled, PositionedError> {
    left.checked_add(right)
        .ok_or(PositionedError::PositionOverflow)
}

fn sub(left: Scaled, right: Scaled) -> Result<Scaled, PositionedError> {
    left.checked_sub(right)
        .ok_or(PositionedError::PositionOverflow)
}
