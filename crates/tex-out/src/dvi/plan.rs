use crate::{
    ArtifactCodecLimits, BoxNode, FontResource, JobInfo, PageArtifact, PageEffect, PageNode,
    binary::V10PageDecoder,
};

use super::{
    DviError, DviWriter,
    extent::page_extent,
    fonts::{DefinedFont, FontKey},
    opcodes::{BOP, EOP},
    traversal::RootStreamState,
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
    root: BoxNode,
    state: Option<RootStreamState>,
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
        let state = writer.begin_root_stream(job.h_offset, job.v_offset, root, vertical)?;
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
            root: root.clone(),
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
        self.writer.add_page_fonts(&fonts[self.indexed_fonts..])?;
        self.indexed_fonts = fonts.len();
        self.writer.push_root_stream_child(
            effects,
            &self.root,
            self.state.as_mut().expect("unfinished page plan"),
            node,
        )
    }

    pub fn finish(mut self, fonts: &[FontResource]) -> Result<DviPagePlan, DviError> {
        // Recheck the final table so a caller cannot replace a resource after
        // the glyph event that first introduced it.
        self.writer.index_fonts(fonts)?;
        self.writer
            .finish_root_stream(self.state.take().expect("unfinished page plan"))?;
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

    /// Validates and compiles canonical v10 bytes without materializing the
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

        let mut writer = DviWriter::new(Vec::new());
        writer.font_definition_sites = Some(Vec::new());
        writer.index_page_fonts(&page)?;
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
        writer.ship_streamed_box(&page, &root, root_vertical, &mut decoder)?;
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

    pub(super) fn banner(&self) -> &str {
        &self.banner
    }

    pub(super) const fn mag(&self) -> i32 {
        self.mag
    }
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
