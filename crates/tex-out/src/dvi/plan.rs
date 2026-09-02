use crate::{
    ArtifactCodecLimits, BoxNode, FontResource, JobInfo, LeaderPayload, PageArtifact, PageEffect,
    PageNode,
    binary::{V10NodeListReader, V10NodeListSlice, V10PageDecoder, V10StreamLeader, V10StreamNode},
};

use super::{
    DVI_BUFFER_SIZE, DviBodyCompiler, DviError, DviFileWriter,
    fonts::{DefinedFont, FontKey},
    opcodes::{BOP, EOP, POP, PUSH},
    traversal::DirectStreamState,
};

/// Detached page-local DVI body compiled before shipout publication.
///
/// Job framing, page backpointers, and cross-page font-definition suppression
/// remain the final assembler's responsibility. The plan owns all of its data
/// and contains no live engine or store handles.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DviPagePlan {
    banner: String,
    mag: i32,
    counts: [i32; 10],
    fonts: Vec<FontResource>,
    body: Vec<u8>,
    font_definition_sites: Vec<FontDefinitionSite>,
    dvi_pop_save_locs: Vec<usize>,
    dvi_pop_sites: Vec<DviPopSite>,
    max_height_depth: i32,
    max_width: i32,
    max_stack_depth: u16,
}

/// Shared detached page compiler used by the owned and canonical-byte
/// adapters.
pub struct DviPagePlanBuilder {
    writer: DviBodyCompiler,
    job: JobInfo,
    counts: [i32; 10],
    state: Option<DirectStreamState>,
    max_height_depth: i32,
    max_width: i32,
    indexed_fonts: usize,
}

/// Operation-local DVI sidecar emitted alongside canonical artifact bytes.
///
/// The active builder consumes the same scalar node events as the artifact
/// encoder and owns no page-node representation. Box leaders require subtree
/// replay by definition; those uncommon pages fall back to the canonical
/// streaming-byte compiler rather than retaining a second node authority.
pub struct DviPagePlanCoEmitter {
    builder: Option<DviPagePlanBuilder>,
    replay_required: bool,
}

impl DviPagePlanCoEmitter {
    #[must_use]
    pub const fn disabled() -> Self {
        Self {
            builder: None,
            replay_required: false,
        }
    }

    pub fn new(
        job: JobInfo,
        counts: [i32; 10],
        root: &BoxNode,
        vertical: bool,
        effects: &[PageEffect],
        enabled: bool,
    ) -> Result<Self, DviError> {
        let builder = if enabled {
            let mut builder = DviPagePlanBuilder::new(job, counts, root, vertical)?;
            builder.set_snap_reference(effects);
            Some(builder)
        } else {
            None
        };
        Ok(Self {
            builder,
            replay_required: false,
        })
    }

    #[inline]
    pub fn add_fonts(&mut self, fonts: &[FontResource]) -> Result<(), DviError> {
        self.builder
            .as_mut()
            .map_or(Ok(()), |builder| builder.add_fonts(fonts))
    }

    #[inline]
    pub fn char(
        &mut self,
        font_id: u32,
        ch: u32,
        width: tex_arith::Scaled,
    ) -> Result<(), DviError> {
        self.builder
            .as_mut()
            .map_or(Ok(()), |builder| builder.char(font_id, ch, width))
    }

    #[inline]
    pub fn kern(&mut self, amount: tex_arith::Scaled) -> Result<(), DviError> {
        self.builder
            .as_mut()
            .map_or(Ok(()), |builder| builder.kern(amount))
    }

    pub fn math(&mut self, amount: tex_arith::Scaled) -> Result<(), DviError> {
        self.builder
            .as_mut()
            .map_or(Ok(()), |builder| builder.math(amount))
    }

    pub fn rule(
        &mut self,
        width: Option<tex_arith::Scaled>,
        height: Option<tex_arith::Scaled>,
        depth: Option<tex_arith::Scaled>,
    ) -> Result<(), DviError> {
        self.builder
            .as_mut()
            .map_or(Ok(()), |builder| builder.rule(width, height, depth))
    }

    pub fn glue(&mut self, spec: crate::GlueSpec) -> Result<(), DviError> {
        self.builder
            .as_mut()
            .map_or(Ok(()), |builder| builder.glue(spec))
    }

    pub fn begin_box(
        &mut self,
        fields: &BoxNode,
        vertical: bool,
        empty: bool,
    ) -> Result<(), DviError> {
        if let Some(builder) = &mut self.builder {
            let entered = builder.begin_box(fields, vertical, empty)?;
            debug_assert_eq!(entered, !empty);
        }
        Ok(())
    }

    pub fn end_box(&mut self) -> Result<(), DviError> {
        self.builder
            .as_mut()
            .map_or(Ok(()), DviPagePlanBuilder::end_box)
    }

    /// Marks a subtree-replaying leader for canonical streaming compilation.
    /// No semantic node storage is retained while switching adapters.
    pub fn leader_requires_replay(&mut self) {
        if self.builder.take().is_some() {
            self.replay_required = true;
        }
    }

    pub fn whatsit(&mut self, effect_index: u32, effects: &[PageEffect]) -> Result<(), DviError> {
        self.builder
            .as_mut()
            .map_or(Ok(()), |builder| builder.whatsit(effect_index, effects))
    }

    pub fn finish(
        self,
        fonts: &[FontResource],
        artifact_bytes: &[u8],
    ) -> Result<Option<DviPagePlan>, DviError> {
        if self.replay_required {
            DviPagePlan::compile_v10(artifact_bytes).map(Some)
        } else {
            self.builder
                .map(|builder| builder.finish(fonts))
                .transpose()
        }
    }
}

impl DviPagePlanBuilder {
    pub fn new(
        job: JobInfo,
        counts: [i32; 10],
        root: &BoxNode,
        vertical: bool,
    ) -> Result<Self, DviError> {
        let mut writer = DviBodyCompiler::new();
        writer.font_definition_sites = Some(Vec::new());
        writer.reset_page_state();
        let state = writer.begin_direct_stream(job.h_offset, job.v_offset, root, vertical)?;
        let max_height_depth = root
            .height
            .raw()
            .checked_add(root.depth.raw())
            .and_then(|extent| extent.checked_add(job.v_offset.raw()))
            .ok_or(DviError::PositionOverflow)?;
        let max_width = root
            .width
            .raw()
            .checked_add(job.h_offset.raw())
            .ok_or(DviError::PositionOverflow)?;
        Ok(Self {
            writer,
            job,
            counts,
            state: Some(state),
            max_height_depth,
            max_width,
            indexed_fonts: 0,
        })
    }

    pub fn push_node(
        &mut self,
        node: &PageNode,
        fonts: &[FontResource],
        effects: &[PageEffect],
    ) -> Result<(), DviError> {
        self.sync_fonts(fonts)?;
        self.push_owned_node(node, effects)
    }

    fn sync_fonts(&mut self, fonts: &[FontResource]) -> Result<(), DviError> {
        self.writer.add_page_fonts(&fonts[self.indexed_fonts..])?;
        self.indexed_fonts = fonts.len();
        Ok(())
    }

    #[inline]
    pub fn add_fonts(&mut self, fonts: &[FontResource]) -> Result<(), DviError> {
        self.sync_fonts(fonts)
    }

    fn set_snap_reference(&mut self, effects: &[PageEffect]) {
        self.writer.snap_reference = crate::snapping::initial_reference(effects);
    }

    fn push_owned_node(&mut self, node: &PageNode, effects: &[PageEffect]) -> Result<(), DviError> {
        self.writer.direct_owned_node(
            self.state.as_mut().expect("unfinished page plan"),
            node,
            effects,
        )
    }

    fn push_owned_list(
        &mut self,
        nodes: &[PageNode],
        effects: &[PageEffect],
    ) -> Result<(), DviError> {
        self.writer.direct_owned_list(
            self.state.as_mut().expect("unfinished page plan"),
            nodes,
            effects,
        )
    }

    #[inline]
    pub fn char(
        &mut self,
        font_id: u32,
        ch: u32,
        width: tex_arith::Scaled,
    ) -> Result<(), DviError> {
        self.writer.direct_char(
            self.state.as_mut().expect("unfinished page plan"),
            font_id,
            ch,
            width,
        )
    }

    #[inline]
    pub fn kern(&mut self, amount: tex_arith::Scaled) -> Result<(), DviError> {
        self.writer
            .direct_kern(self.state.as_ref().expect("unfinished page plan"), amount)
    }

    pub fn math(&mut self, amount: tex_arith::Scaled) -> Result<(), DviError> {
        self.writer
            .direct_math(self.state.as_ref().expect("unfinished page plan"), amount)
    }

    pub fn rule(
        &mut self,
        width: Option<tex_arith::Scaled>,
        height: Option<tex_arith::Scaled>,
        depth: Option<tex_arith::Scaled>,
    ) -> Result<(), DviError> {
        self.writer.direct_rule(
            self.state.as_ref().expect("unfinished page plan"),
            width,
            height,
            depth,
        )
    }

    pub fn glue(&mut self, spec: crate::GlueSpec) -> Result<(), DviError> {
        self.writer
            .direct_glue(self.state.as_mut().expect("unfinished page plan"), spec)
    }

    pub fn begin_box(
        &mut self,
        fields: &BoxNode,
        vertical: bool,
        empty: bool,
    ) -> Result<bool, DviError> {
        self.writer.direct_begin_box(
            self.state.as_mut().expect("unfinished page plan"),
            fields,
            vertical,
            empty,
        )
    }

    pub fn end_box(&mut self) -> Result<(), DviError> {
        self.writer
            .direct_end_box(self.state.as_mut().expect("unfinished page plan"))
    }

    /// Emits the one node kind whose DVI semantics require subtree replay.
    /// Canonical-byte compilation materializes only this localized payload;
    /// ordinary boxes and leaves use the scalar methods above.
    pub fn leader(&mut self, node: &PageNode, effects: &[PageEffect]) -> Result<(), DviError> {
        debug_assert!(matches!(
            node,
            PageNode::Glue {
                leader: Some(_),
                ..
            }
        ));
        self.writer.direct_owned_leader(
            self.state.as_mut().expect("unfinished page plan"),
            effects,
            node,
        )
    }

    pub fn whatsit(&mut self, effect_index: u32, effects: &[PageEffect]) -> Result<(), DviError> {
        self.writer.direct_whatsit(
            self.state.as_ref().expect("unfinished page plan"),
            effects,
            effect_index,
        )
    }

    pub fn finish(mut self, fonts: &[FontResource]) -> Result<DviPagePlan, DviError> {
        // Recheck the final table so a caller cannot replace a resource after
        // the glyph event that first introduced it.
        self.writer.index_fonts(fonts)?;
        self.writer
            .finish_direct_stream(self.state.take().expect("unfinished page plan"))?;
        let body = std::mem::take(&mut self.writer.bytes);
        let font_definition_sites = self
            .writer
            .font_definition_sites
            .take()
            .expect("page-plan compiler enables font relocation recording");
        let mut dvi_pop_sites = std::mem::take(&mut self.writer.dvi_pop_sites);
        dvi_pop_sites.sort_unstable_by_key(|site| site.pop_offset);
        let dvi_pop_save_locs = std::mem::take(&mut self.writer.dvi_pop_save_locs);
        debug_assert_eq!(dvi_pop_save_locs.len(), dvi_pop_sites.len());
        Ok(DviPagePlan {
            banner: self.job.banner,
            mag: self.job.mag,
            counts: self.counts,
            fonts: fonts.to_vec(),
            body,
            font_definition_sites,
            dvi_pop_save_locs,
            dvi_pop_sites,
            max_height_depth: self.max_height_depth,
            max_width: self.max_width,
            max_stack_depth: self.writer.max_stack_depth,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FontDefinitionSite {
    pub(super) font_id: u32,
    pub(super) start: usize,
    pub(super) end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DviPopSite {
    pub(super) pop_offset: usize,
}

impl DviPagePlan {
    /// Compiles all page-local traversal decisions into final DVI body bytes.
    pub fn compile(page: &PageArtifact) -> Result<Self, DviError> {
        let (vertical, root) = match &page.root {
            PageNode::HList(root) => (false, root),
            PageNode::VList(root) => (true, root),
            _ => unreachable!("validated page root is a box"),
        };
        let mut builder = DviPagePlanBuilder::new(page.job.clone(), page.counts, root, vertical)?;
        builder.writer.snap_reference = crate::snapping::initial_reference(&page.effects);
        builder.add_fonts(&page.fonts)?;
        builder.push_owned_list(&root.children, &page.effects)?;
        builder.finish(&page.fonts)
    }

    /// Validates and compiles canonical artifact bytes without materializing the
    /// complete recursive page tree.
    pub fn compile_v10(bytes: &[u8]) -> Result<Self, DviError> {
        let mut decoder = V10PageDecoder::new(bytes, ArtifactCodecLimits::default())?;
        let page = decoder.page.clone();
        let (root_vertical, root) = match &page.root {
            PageNode::HList(root) => (false, root.clone()),
            PageNode::VList(root) => (true, root.clone()),
            _ => unreachable!("stream decoder accepts only box roots"),
        };
        debug_assert_eq!(root_vertical, decoder.root_vertical);

        let mut builder =
            DviPagePlanBuilder::new(page.job.clone(), page.counts, &root, root_vertical)?;
        builder.writer.snap_reference = crate::snapping::initial_reference(&page.effects);
        builder.add_fonts(&page.fonts)?;
        let children = decoder.stream_children();
        children.validate_all()?;
        feed_v10_list(&mut builder, children, &page.effects)?;
        builder.finish(&page.fonts)
    }

    pub(super) fn banner(&self) -> &str {
        &self.banner
    }

    pub(super) const fn mag(&self) -> i32 {
        self.mag
    }
}

fn feed_v10_list(
    builder: &mut DviPagePlanBuilder,
    nodes: V10NodeListSlice<'_, '_>,
    effects: &[PageEffect],
) -> Result<(), DviError> {
    enum Frame<'r, 'a> {
        List(V10NodeListReader<'r, 'a>),
        EndBox,
    }

    let mut frames = vec![Frame::List(nodes.reader())];
    while let Some(frame) = frames.pop() {
        let Frame::List(mut nodes) = frame else {
            builder.end_box()?;
            continue;
        };
        let Some(node) = nodes.next(false)? else {
            continue;
        };
        frames.push(Frame::List(nodes));
        match node {
            V10StreamNode::Char { font_id, ch, width } => builder.char(font_id, ch, width)?,
            V10StreamNode::Kern(amount) => builder.kern(amount)?,
            V10StreamNode::Glue {
                spec,
                leader: V10StreamLeader::None,
                ..
            } => builder.glue(spec)?,
            V10StreamNode::Glue { spec, kind, leader } => {
                let leader = materialize_v10_leader(leader)?;
                let node = PageNode::Glue {
                    spec,
                    kind,
                    leader: Some(leader),
                };
                let result = builder.leader(&node, effects);
                drop_page_node_iterative(node);
                result?;
            }
            V10StreamNode::Rule {
                width,
                height,
                depth,
            } => builder.rule(width, height, depth)?,
            V10StreamNode::Box {
                vertical,
                fields,
                children,
            } => {
                let entered = builder.begin_box(&fields, vertical, children.reader().is_empty())?;
                if entered {
                    frames.push(Frame::EndBox);
                    frames.push(Frame::List(children.reader()));
                }
            }
            V10StreamNode::WhatsitAnchor(effect_index) => {
                builder.whatsit(effect_index, effects)?;
            }
            V10StreamNode::Math(amount) => builder.math(amount)?,
            V10StreamNode::Ignored(_) => {}
        }
    }
    Ok(())
}

fn drop_page_node_iterative(root: PageNode) {
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        match node {
            PageNode::HList(box_node) | PageNode::VList(box_node) => {
                pending.extend(box_node.children);
            }
            PageNode::Glue {
                leader: Some(LeaderPayload::HList(box_node) | LeaderPayload::VList(box_node)),
                ..
            } => pending.extend(box_node.children),
            PageNode::Disc {
                pre, post, replace, ..
            } => {
                pending.extend(pre);
                pending.extend(post);
                pending.extend(replace);
            }
            PageNode::Insert { content, .. } | PageNode::Adjust(content) => {
                pending.extend(content);
            }
            PageNode::Char { .. }
            | PageNode::Lig { .. }
            | PageNode::Kern { .. }
            | PageNode::MarginKern { .. }
            | PageNode::Glue { .. }
            | PageNode::Penalty(_)
            | PageNode::Rule { .. }
            | PageNode::Mark { .. }
            | PageNode::WhatsitAnchor { .. }
            | PageNode::MathOn(_)
            | PageNode::MathOff(_) => {}
        }
    }
}

fn materialize_v10_leader(leader: V10StreamLeader<'_, '_>) -> Result<LeaderPayload, DviError> {
    match leader {
        V10StreamLeader::None => unreachable!("caller handles absent leaders"),
        V10StreamLeader::Rule {
            width,
            height,
            depth,
        } => Ok(LeaderPayload::Rule {
            width,
            height,
            depth,
        }),
        V10StreamLeader::Box {
            vertical,
            fields,
            children,
        } => {
            let children = materialize_v10_list(children)?;
            let box_node = BoxNode { children, ..fields };
            Ok(if vertical {
                LeaderPayload::VList(box_node)
            } else {
                LeaderPayload::HList(box_node)
            })
        }
    }
}

fn materialize_v10_list(nodes: V10NodeListSlice<'_, '_>) -> Result<Vec<PageNode>, DviError> {
    enum Frame<'r, 'a> {
        List {
            reader: V10NodeListReader<'r, 'a>,
            nodes: Vec<PageNode>,
        },
        Box {
            vertical: bool,
            fields: BoxNode,
        },
        Leader {
            spec: crate::GlueSpec,
            kind: crate::GlueKind,
            vertical: bool,
            fields: BoxNode,
        },
    }

    let mut frames = vec![Frame::List {
        reader: nodes.reader(),
        nodes: Vec::new(),
    }];
    let mut completed = None;
    loop {
        let frame = frames.pop().expect("materialization root frame");
        match frame {
            Frame::Box { vertical, fields } => {
                let children = completed.take().expect("completed box children");
                let box_node = BoxNode { children, ..fields };
                let node = if vertical {
                    PageNode::VList(box_node)
                } else {
                    PageNode::HList(box_node)
                };
                let Some(Frame::List { nodes, .. }) = frames.last_mut() else {
                    unreachable!("box continuation follows a list")
                };
                nodes.push(node);
            }
            Frame::Leader {
                spec,
                kind,
                vertical,
                fields,
            } => {
                let children = completed.take().expect("completed leader children");
                let box_node = BoxNode { children, ..fields };
                let leader = if vertical {
                    LeaderPayload::VList(box_node)
                } else {
                    LeaderPayload::HList(box_node)
                };
                let Some(Frame::List { nodes, .. }) = frames.last_mut() else {
                    unreachable!("leader continuation follows a list")
                };
                nodes.push(PageNode::Glue {
                    spec,
                    kind,
                    leader: Some(leader),
                });
            }
            Frame::List {
                mut reader,
                mut nodes,
            } => {
                let Some(node) = reader.next(false)? else {
                    completed = Some(nodes);
                    if frames.is_empty() {
                        return Ok(completed.expect("completed root list"));
                    }
                    continue;
                };
                let node = match node {
                    V10StreamNode::Char { font_id, ch, width } => {
                        Some(PageNode::Char { font_id, ch, width })
                    }
                    V10StreamNode::Kern(amount) => Some(PageNode::Kern {
                        amount,
                        kind: crate::KernKind::Explicit,
                    }),
                    V10StreamNode::Glue {
                        spec,
                        kind,
                        leader:
                            V10StreamLeader::Box {
                                vertical,
                                fields,
                                children,
                            },
                    } => {
                        frames.push(Frame::List { reader, nodes });
                        frames.push(Frame::Leader {
                            spec,
                            kind,
                            vertical,
                            fields,
                        });
                        frames.push(Frame::List {
                            reader: children.reader(),
                            nodes: Vec::new(),
                        });
                        continue;
                    }
                    V10StreamNode::Glue { spec, kind, leader } => Some(PageNode::Glue {
                        spec,
                        kind,
                        leader: match leader {
                            V10StreamLeader::None => None,
                            leader => Some(materialize_v10_leader(leader)?),
                        },
                    }),
                    V10StreamNode::Rule {
                        width,
                        height,
                        depth,
                    } => Some(PageNode::Rule {
                        width,
                        height,
                        depth,
                    }),
                    V10StreamNode::Box {
                        vertical,
                        fields,
                        children,
                    } => {
                        frames.push(Frame::List { reader, nodes });
                        frames.push(Frame::Box { vertical, fields });
                        frames.push(Frame::List {
                            reader: children.reader(),
                            nodes: Vec::new(),
                        });
                        continue;
                    }
                    V10StreamNode::WhatsitAnchor(effect_index) => {
                        Some(PageNode::WhatsitAnchor { effect_index })
                    }
                    V10StreamNode::Math(amount) => Some(PageNode::MathOn(amount)),
                    V10StreamNode::Ignored(_) => None,
                };
                if let Some(node) = node {
                    nodes.push(node);
                }
                frames.push(Frame::List { reader, nodes });
            }
        }
    }
}

impl<W: std::io::Write> DviFileWriter<W> {
    pub(super) fn page_plan(&mut self, plan: &DviPagePlan) -> Result<(), DviError> {
        self.index_fonts(&plan.fonts)?;
        let bop_location = self.current_pointer()?;
        self.u8(BOP);
        for count in plan.counts {
            self.i32(count);
        }
        let previous_bop = self.previous_bop;
        self.i32(previous_bop);
        self.previous_bop = bop_location;

        self.max_height_depth = self.max_height_depth.max(plan.max_height_depth);
        self.max_width = self.max_width.max(plan.max_width);
        self.max_stack_depth = self.max_stack_depth.max(plan.max_stack_depth);

        self.write_plan_body(plan)?;
        self.u8(EOP);
        Ok(())
    }

    fn write_plan_body(&mut self, plan: &DviPagePlan) -> Result<(), DviError> {
        self.dvi_pop_save_offsets.clear();
        let mut save_index = 0usize;
        let mut pop_index = 0usize;
        let mut font_index = 0usize;
        let mut cursor = 0usize;

        while cursor < plan.body.len() {
            while plan
                .dvi_pop_save_locs
                .get(save_index)
                .is_some_and(|&offset| offset == cursor)
            {
                let current = self.current_offset()?;
                self.dvi_pop_save_offsets.push(current);
                save_index += 1;
            }

            if let Some(site) = plan.font_definition_sites.get(font_index)
                && site.start == cursor
            {
                debug_assert!(site.start <= site.end && site.end <= plan.body.len());
                debug_assert!(
                    plan.dvi_pop_save_locs
                        .get(save_index)
                        .is_none_or(|&offset| offset >= site.end)
                );
                debug_assert!(
                    plan.dvi_pop_sites
                        .get(pop_index)
                        .is_none_or(|pop| pop.pop_offset >= site.end)
                );
                let font =
                    self.page_fonts
                        .get(&site.font_id)
                        .cloned()
                        .ok_or(DviError::MissingFont {
                            font_id: site.font_id,
                        })?;
                let key = FontKey::from(&font);
                if !self.fonts.contains_key(&key) {
                    self.raw(&plan.body[site.start..site.end]);
                    self.fonts.insert(
                        key.clone(),
                        DefinedFont {
                            number: font.font_id,
                            font,
                        },
                    );
                }
                cursor = site.end;
                font_index += 1;
                continue;
            }

            if let Some(site) = plan.dvi_pop_sites.get(pop_index)
                && site.pop_offset == cursor
            {
                debug_assert_eq!(plan.body[cursor], POP);
                let current = self.current_offset()?;
                let saved = self
                    .dvi_pop_save_offsets
                    .pop()
                    .expect("every planned DVI pop has a matching push");
                if saved == current && !current.is_multiple_of(DVI_BUFFER_SIZE) {
                    debug_assert_eq!(self.bytes.last(), Some(&PUSH));
                    self.bytes.pop();
                } else {
                    self.u8(POP);
                }
                cursor += 1;
                pop_index += 1;
                continue;
            }

            let next_save = plan
                .dvi_pop_save_locs
                .get(save_index)
                .copied()
                .unwrap_or(plan.body.len());
            let next_pop = plan
                .dvi_pop_sites
                .get(pop_index)
                .map_or(plan.body.len(), |site| site.pop_offset);
            let next_font = plan
                .font_definition_sites
                .get(font_index)
                .map_or(plan.body.len(), |site| site.start);
            let end = next_save.min(next_pop).min(next_font).min(plan.body.len());
            debug_assert!(cursor < end);
            self.raw(&plan.body[cursor..end]);
            cursor = end;
        }
        debug_assert_eq!(save_index, plan.dvi_pop_save_locs.len());
        debug_assert_eq!(pop_index, plan.dvi_pop_sites.len());
        debug_assert_eq!(font_index, plan.font_definition_sites.len());
        debug_assert!(self.dvi_pop_save_offsets.is_empty());
        Ok(())
    }
}
