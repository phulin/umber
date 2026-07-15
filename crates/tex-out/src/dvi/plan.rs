use crate::{
    ArtifactCodecLimits, BoxNode, FontResource, JobInfo, LeaderPayload, PageArtifact, PageEffect,
    PageNode,
    binary::{V10NodeListReader, V10PageDecoder, V10StreamLeader, V10StreamNode},
};

use super::{
    DviError, DviWriter,
    extent::page_extent,
    fonts::{DefinedFont, FontKey},
    opcodes::{BOP, EOP},
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
    max_height_depth: i32,
    max_width: i32,
    max_stack_depth: u16,
}

/// Incremental fresh-page compiler driven by the same detached node events as
/// canonical artifact encoding.
pub struct DviPagePlanBuilder {
    writer: DviWriter<Vec<u8>>,
    job: JobInfo,
    counts: [i32; 10],
    state: Option<DirectStreamState>,
    max_height_depth: i32,
    max_width: i32,
    indexed_fonts: usize,
}

impl DviPagePlanBuilder {
    pub fn new(
        job: JobInfo,
        counts: [i32; 10],
        root: &BoxNode,
        vertical: bool,
    ) -> Result<Self, DviError> {
        let mut writer = DviWriter::new(Vec::new());
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

    pub fn add_fonts(&mut self, fonts: &[FontResource]) -> Result<(), DviError> {
        self.sync_fonts(fonts)
    }

    fn push_owned_node(&mut self, node: &PageNode, effects: &[PageEffect]) -> Result<(), DviError> {
        match node {
            PageNode::Char { font_id, ch, width }
            | PageNode::Lig {
                font_id, ch, width, ..
            } => self.char(*font_id, *ch, *width),
            PageNode::Kern { amount, .. } => self.kern(*amount),
            PageNode::Glue {
                spec, leader: None, ..
            } => self.glue(*spec),
            PageNode::Glue {
                leader: Some(_), ..
            } => self.writer.direct_owned_leader(
                self.state.as_mut().expect("unfinished page plan"),
                effects,
                node,
            ),
            PageNode::Penalty(_)
            | PageNode::Disc { .. }
            | PageNode::Mark { .. }
            | PageNode::Insert { .. }
            | PageNode::Adjust(_) => Ok(()),
            PageNode::Rule {
                width,
                height,
                depth,
            } => self.rule(*width, *height, *depth),
            PageNode::HList(box_node) | PageNode::VList(box_node) => {
                let entered = self.begin_box(
                    box_node,
                    matches!(node, PageNode::VList(_)),
                    box_node.children.is_empty(),
                )?;
                if entered {
                    for child in &box_node.children {
                        self.push_owned_node(child, effects)?;
                    }
                    self.end_box()?;
                }
                Ok(())
            }
            PageNode::WhatsitAnchor { effect_index } => self.whatsit(*effect_index, effects),
            PageNode::MathOn(width) | PageNode::MathOff(width) => self.math(*width),
        }
    }

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
    /// Fresh shipout may materialize only this localized payload; ordinary
    /// boxes and leaves use the scalar methods above.
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
        Ok(DviPagePlan {
            banner: self.job.banner,
            mag: self.job.mag,
            counts: self.counts,
            fonts: fonts.to_vec(),
            body,
            font_definition_sites,
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

impl DviPagePlan {
    /// Compiles all page-local traversal decisions into final DVI body bytes.
    pub fn compile(page: &PageArtifact) -> Result<Self, DviError> {
        let mut writer = DviWriter::new(Vec::new());
        writer.font_definition_sites = Some(Vec::new());
        writer.index_page_fonts(page)?;
        writer.reset_page_state();

        let extent = page_extent(&page.root);
        let max_height_depth = extent
            .height_depth
            .checked_add(page.job.v_offset.raw())
            .ok_or(DviError::PositionOverflow)?;
        let max_width = extent
            .width
            .checked_add(page.job.h_offset.raw())
            .ok_or(DviError::PositionOverflow)?;
        writer.ship_box(page, &page.root)?;
        let body = std::mem::take(&mut writer.bytes);
        let font_definition_sites = writer
            .font_definition_sites
            .take()
            .expect("page-plan compiler enables font relocation recording");

        Ok(Self {
            banner: page.job.banner.clone(),
            mag: page.job.mag,
            counts: page.counts,
            fonts: page.fonts.clone(),
            body,
            font_definition_sites,
            max_height_depth,
            max_width,
            max_stack_depth: writer.max_stack_depth,
        })
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
        builder.add_fonts(&page.fonts)?;
        let mut children = decoder.stream_children();
        feed_v10_list(&mut builder, &mut children, &page.effects)?;
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
    nodes: &mut V10NodeListReader<'_, '_>,
    effects: &[PageEffect],
) -> Result<(), DviError> {
    while let Some(node) = nodes.next()? {
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
                builder.leader(
                    &PageNode::Glue {
                        spec,
                        kind,
                        leader: Some(leader),
                    },
                    effects,
                )?;
            }
            V10StreamNode::Rule {
                width,
                height,
                depth,
            } => builder.rule(width, height, depth)?,
            V10StreamNode::Box {
                vertical,
                fields,
                mut children,
            } => {
                let entered = builder.begin_box(&fields, vertical, children.is_empty())?;
                if entered {
                    feed_v10_list(builder, &mut children, effects)?;
                    builder.end_box()?;
                }
            }
            V10StreamNode::WhatsitAnchor(effect_index) => {
                builder.whatsit(effect_index, effects)?;
            }
            V10StreamNode::Math(amount) => builder.math(amount)?,
            V10StreamNode::Ignored => {}
        }
    }
    Ok(())
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
            let children = children.with_reader(materialize_v10_list)?;
            let box_node = BoxNode { children, ..fields };
            Ok(if vertical {
                LeaderPayload::VList(box_node)
            } else {
                LeaderPayload::HList(box_node)
            })
        }
    }
}

fn materialize_v10_list(nodes: &mut V10NodeListReader<'_, '_>) -> Result<Vec<PageNode>, DviError> {
    let mut materialized = Vec::new();
    while let Some(node) = nodes.next()? {
        let node = match node {
            V10StreamNode::Char { font_id, ch, width } => {
                Some(PageNode::Char { font_id, ch, width })
            }
            V10StreamNode::Kern(amount) => Some(PageNode::Kern {
                amount,
                kind: crate::KernKind::Explicit,
            }),
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
                mut children,
            } => {
                let children = materialize_v10_list(&mut children)?;
                let box_node = BoxNode { children, ..fields };
                Some(if vertical {
                    PageNode::VList(box_node)
                } else {
                    PageNode::HList(box_node)
                })
            }
            V10StreamNode::WhatsitAnchor(effect_index) => {
                Some(PageNode::WhatsitAnchor { effect_index })
            }
            V10StreamNode::Math(amount) => Some(PageNode::MathOn(amount)),
            V10StreamNode::Ignored => None,
        };
        if let Some(node) = node {
            materialized.push(node);
        }
    }
    Ok(materialized)
}

impl<W: std::io::Write> DviWriter<W> {
    pub(super) fn page_plan(&mut self, plan: &DviPagePlan) -> Result<(), DviError> {
        self.index_fonts(&plan.fonts)?;
        let bop_location = self.current_pointer()?;
        self.u8(BOP);
        for count in plan.counts {
            self.i32(count);
        }
        self.i32(self.previous_bop);
        self.previous_bop = bop_location;

        self.max_height_depth = self.max_height_depth.max(plan.max_height_depth);
        self.max_width = self.max_width.max(plan.max_width);
        self.max_stack_depth = self.max_stack_depth.max(plan.max_stack_depth);

        let mut cursor = 0usize;
        for site in &plan.font_definition_sites {
            debug_assert!(cursor <= site.start && site.start <= site.end);
            debug_assert!(site.end <= plan.body.len());
            self.raw(&plan.body[cursor..site.start]);
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
        }
        self.raw(&plan.body[cursor..]);
        self.u8(EOP);
        Ok(())
    }
}
