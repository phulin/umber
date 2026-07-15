use std::collections::BTreeMap;
use tex_arith::Scaled;

use crate::dvi::glue::adjusted_glue_width;
use crate::{BoxNode, GlueKind, KernKind, LeaderPayload, PageArtifact, PageEffect, PageNode};

use super::{
    BoxKind, PositionedBox, PositionedBoxEnd, PositionedError, PositionedEvent, PositionedLimits,
    PositionedPage, PositionedPdfAccessibility, PositionedPdfAnnotation, PositionedRule,
    PositionedPdfGraphics, PositionedSourceRef, PositionedSpecial, PositionedTextRun, TextUnit,
};

const LEADER_ROUNDING_COMPENSATION: Scaled = Scaled::from_raw(10);

pub(super) fn lower(
    page: &PageArtifact,
    page_index: u32,
    limits: PositionedLimits,
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
    let right = add(page.job.h_offset, root.width)?;
    let bottom = add(page.job.v_offset, height)?;
    let mut out = Lowerer {
        effects: &page.effects,
        events: Vec::new(),
        limits,
        cur_h: page.job.h_offset,
        cur_v: add(root.height, page.job.v_offset)?,
        current_font_id: None,
        node_ordinals: index_nodes(&page.root),
        next_box_id: 0,
        box_stack: Vec::new(),
        pdf_save_positions: Vec::new(),
        diagnostics: Vec::new(),
    };
    match kind {
        BoxKind::Horizontal => out.hlist(root, 1)?,
        BoxKind::Vertical => out.vlist(root, 1)?,
    }
    if !out.pdf_save_positions.is_empty() {
        return Err(PositionedError::UnmatchedPdfSaves {
            count: out.pdf_save_positions.len(),
        });
    }
    Ok(PositionedPage {
        page_index,
        width: Scaled::from_raw(right.raw().max(0)),
        height: Scaled::from_raw(bottom.raw().max(0)),
        mag: page.job.mag,
        counts: page.counts,
        fonts: page.fonts.clone(),
        events: out.events,
        diagnostics: out.diagnostics,
    })
}

struct Lowerer<'a> {
    effects: &'a [PageEffect],
    events: Vec<PositionedEvent>,
    limits: PositionedLimits,
    cur_h: Scaled,
    cur_v: Scaled,
    current_font_id: Option<u32>,
    node_ordinals: BTreeMap<usize, u32>,
    next_box_id: u32,
    box_stack: Vec<u32>,
    pdf_save_positions: Vec<(Scaled, Scaled)>,
    diagnostics: Vec<String>,
}

impl Lowerer<'_> {
    fn node_ordinal(&self, node: &PageNode) -> u32 {
        self.node_ordinals[&(node as *const PageNode as usize)]
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
        self.check_depth(depth)?;
        let base_line = self.cur_v;
        let left_edge = self.cur_h;
        let box_id = self.box_event(BoxKind::Horizontal, this_box, left_edge, base_line, depth)?;
        let mut cur_g = Scaled::from_raw(0);
        let mut cur_glue = Scaled::from_raw(0);
        let mut run = RunBuilder::default();

        for child in &this_box.children {
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
                            physical_code: Some(
                                u8::try_from(*ch).map_err(|_| {
                                    PositionedError::CharacterOutOfRange { ch: *ch }
                                })?,
                            ),
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
                                    .then(|| u8::try_from(*ch))
                                    .transpose()
                                    .map_err(|_| PositionedError::CharacterOutOfRange {
                                        ch: *ch,
                                    })?,
                                source: PositionedSourceRef {
                                    node_ordinal,
                                    source_index: u16::try_from(source_index).map_err(|_| {
                                        PositionedError::TextRunTooLong {
                                            limit: self.limits.max_run_units,
                                        }
                                    })?,
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
                PageNode::Glue { spec, kind, leader } => {
                    let width = glue_width(this_box, *spec, &mut cur_glue, &mut cur_g)?;
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
                        self.hleaders(this_box, *kind, leader, width, left_edge, base_line, depth)?;
                    }
                }
                PageNode::HList(box_node) | PageNode::VList(box_node) => {
                    run.flush(self)?;
                    self.box_in_hlist(box_node, matches!(child, PageNode::VList(_)), depth + 1)?;
                }
                PageNode::Rule {
                    width,
                    height,
                    depth: rule_depth,
                } => {
                    run.flush(self)?;
                    let rule_height = height.unwrap_or(this_box.height);
                    let rule_depth = rule_depth.unwrap_or(this_box.depth);
                    let rule_width = width.unwrap_or(Scaled::from_raw(0));
                    self.rule_h(rule_height, rule_depth, rule_width, base_line)?;
                    self.cur_h = add(self.cur_h, rule_width)?;
                }
                PageNode::MathOn(width) | PageNode::MathOff(width) => {
                    run.flush(self)?;
                    self.cur_h = add(self.cur_h, *width)?;
                }
                PageNode::WhatsitAnchor { effect_index } => {
                    run.flush(self)?;
                    self.special(*effect_index, depth)?;
                }
                PageNode::Penalty(_)
                | PageNode::Disc { .. }
                | PageNode::Mark { .. }
                | PageNode::Insert { .. }
                | PageNode::Adjust(_) => {
                    run.flush(self)?;
                }
            }
            self.cur_v = base_line;
        }
        run.flush(self)?;
        self.end_box(box_id, depth)
    }

    fn vlist(&mut self, this_box: &BoxNode, depth: usize) -> Result<(), PositionedError> {
        self.check_depth(depth)?;
        let baseline = self.cur_v;
        let left_edge = self.cur_h;
        let box_id = self.box_event(BoxKind::Vertical, this_box, left_edge, baseline, depth)?;
        self.cur_v = sub(self.cur_v, this_box.height)?;
        let top_edge = self.cur_v;
        let mut cur_g = Scaled::from_raw(0);
        let mut cur_glue = Scaled::from_raw(0);

        for child in &this_box.children {
            match child {
                PageNode::HList(box_node) | PageNode::VList(box_node) => {
                    self.box_in_vlist(box_node, matches!(child, PageNode::VList(_)), depth + 1)?;
                    self.cur_h = left_edge;
                }
                PageNode::Rule {
                    width,
                    height,
                    depth,
                } => {
                    let rule_height = add(
                        height.unwrap_or(Scaled::from_raw(0)),
                        depth.unwrap_or(Scaled::from_raw(0)),
                    )?;
                    self.rule_v(rule_height, width.unwrap_or(this_box.width))?;
                }
                PageNode::Glue { spec, kind, leader } => {
                    let height = glue_width(this_box, *spec, &mut cur_glue, &mut cur_g)?;
                    self.vleaders(this_box, *kind, leader, height, left_edge, top_edge, depth)?;
                }
                PageNode::Kern { amount, .. } => self.cur_v = add(self.cur_v, *amount)?,
                PageNode::WhatsitAnchor { effect_index } => self.special(*effect_index, depth)?,
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
        self.end_box(box_id, depth)
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

    fn box_in_hlist(
        &mut self,
        node: &BoxNode,
        vertical: bool,
        depth: usize,
    ) -> Result<(), PositionedError> {
        if node.children.is_empty() {
            self.cur_h = add(self.cur_h, node.width)?;
            return Ok(());
        }
        let edge = self.cur_h;
        let baseline = self.cur_v;
        self.cur_v = add(baseline, node.shift)?;
        if vertical {
            self.vlist(node, depth)?
        } else {
            self.hlist(node, depth)?
        }
        self.cur_h = add(edge, node.width)?;
        self.cur_v = baseline;
        Ok(())
    }

    fn box_in_vlist(
        &mut self,
        node: &BoxNode,
        vertical: bool,
        depth: usize,
    ) -> Result<(), PositionedError> {
        if node.children.is_empty() {
            self.cur_v = add(add(self.cur_v, node.height)?, node.depth)?;
            return Ok(());
        }
        self.cur_v = add(self.cur_v, node.height)?;
        let baseline = self.cur_v;
        let left = self.cur_h;
        self.cur_h = add(left, node.shift)?;
        if vertical {
            self.vlist(node, depth)?
        } else {
            self.hlist(node, depth)?
        }
        self.cur_v = add(baseline, node.depth)?;
        self.cur_h = left;
        Ok(())
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

    fn special(&mut self, effect_index: u32, depth: usize) -> Result<(), PositionedError> {
        let effect = self
            .effects
            .get(effect_index as usize)
            .ok_or(PositionedError::MissingEffect { effect_index })?;
        match effect {
            PageEffect::Special { class, payload } => {
                self.push(PositionedEvent::Special(PositionedSpecial {
                    x: self.cur_h,
                    y: self.cur_v,
                    class: class.clone(),
                    payload: payload.clone(),
                }))?;
            }
            PageEffect::PdfAccessibility(control) => {
                self.push(PositionedEvent::PdfAccessibility(
                    PositionedPdfAccessibility {
                        x: self.cur_h,
                        y: self.cur_v,
                        control: *control,
                    },
                ))?;
            }
            PageEffect::PdfAnnotation(marker) => {
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
            }
            PageEffect::PdfLiteral { .. }
            | PageEffect::PdfSetMatrix { .. }
            | PageEffect::PdfSave
            | PageEffect::PdfRestore => {
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
                            ));
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
            PageEffect::OpenOut { .. } | PageEffect::CloseOut { .. } | PageEffect::Write { .. } => {
            }
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
        let code = u8::try_from(character.source_code).map_err(|_| {
            PositionedError::CharacterOutOfRange {
                ch: character.source_code,
            }
        })?;
        debug_assert!(self.font_id.is_none_or(|current| current == font_id));
        if self.font_id.is_none() {
            self.font_id = Some(font_id);
            self.x = Some(x);
            self.baseline = Some(baseline);
        }
        self.resolve_pending_space(limits)?;
        self.add_unit(
            TextUnit::Code(code),
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

fn index_nodes(root: &PageNode) -> BTreeMap<usize, u32> {
    let mut result = BTreeMap::new();
    let mut stack = vec![root];
    let mut ordinal = 0_u32;
    while let Some(node) = stack.pop() {
        result.insert(node as *const PageNode as usize, ordinal);
        ordinal = ordinal
            .checked_add(1)
            .expect("validated artifact node count fits u32");
        match node {
            PageNode::HList(node) | PageNode::VList(node) => {
                stack.extend(node.children.iter().rev());
            }
            PageNode::Glue {
                leader: Some(LeaderPayload::HList(node) | LeaderPayload::VList(node)),
                ..
            } => stack.extend(node.children.iter().rev()),
            PageNode::Disc {
                pre, post, replace, ..
            } => {
                stack.extend(replace.iter().rev());
                stack.extend(post.iter().rev());
                stack.extend(pre.iter().rev());
            }
            PageNode::Insert { content, .. } | PageNode::Adjust(content) => {
                stack.extend(content.iter().rev());
            }
            _ => {}
        }
    }
    result
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

#[derive(Clone, Copy)]
enum LeaderMode {
    Aligned,
    Centered,
    Expanded,
}

impl LeaderMode {
    fn from_glue(kind: GlueKind) -> Option<Self> {
        match kind {
            GlueKind::Leaders => Some(Self::Aligned),
            GlueKind::Cleaders => Some(Self::Centered),
            GlueKind::Xleaders => Some(Self::Expanded),
            _ => None,
        }
    }
}

fn leader_start(
    kind: LeaderMode,
    cur: Scaled,
    origin: Scaled,
    available: Scaled,
    size: Scaled,
) -> Result<(Scaled, Scaled), PositionedError> {
    match kind {
        LeaderMode::Aligned => {
            let diff = i64::from(cur.raw()) - i64::from(origin.raw());
            let quotient = diff / i64::from(size.raw());
            let mut start = scaled(i64::from(origin.raw()) + i64::from(size.raw()) * quotient)?;
            if start.raw() < cur.raw() {
                start = add(start, size)?;
            }
            Ok((start, Scaled::from_raw(0)))
        }
        LeaderMode::Centered => Ok((
            add(cur, Scaled::from_raw(available.raw() % size.raw() / 2))?,
            Scaled::from_raw(0),
        )),
        LeaderMode::Expanded => {
            let q = i64::from(available.raw() / size.raw());
            let r = i64::from(available.raw() % size.raw());
            let extra = r / (q + 1);
            Ok((
                add(cur, scaled((r - (q - 1) * extra) / 2)?)?,
                scaled(extra)?,
            ))
        }
    }
}

fn add(left: Scaled, right: Scaled) -> Result<Scaled, PositionedError> {
    left.checked_add(right)
        .ok_or(PositionedError::PositionOverflow)
}

fn sub(left: Scaled, right: Scaled) -> Result<Scaled, PositionedError> {
    left.checked_sub(right)
        .ok_or(PositionedError::PositionOverflow)
}

fn scaled(value: i64) -> Result<Scaled, PositionedError> {
    i32::try_from(value)
        .map(Scaled::from_raw)
        .map_err(|_| PositionedError::PositionOverflow)
}
