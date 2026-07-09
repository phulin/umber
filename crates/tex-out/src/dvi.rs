use std::fmt;

use tex_arith::{GLUE_SET_RATIO_SCALE, Scaled};

use movement::MovementStack;

use crate::{
    BoxNode, FontResource, GlueOrder, GlueSetRatio, GlueSign, GlueSpec, PageArtifact, PageEffect,
    PageNode,
};

#[cfg(test)]
mod tests;

mod movement;

const ID_BYTE: u8 = 2;
const SET1: u8 = 128;
const SET_RULE: u8 = 132;
const PUT_RULE: u8 = 137;
const PRE: u8 = 247;
const POST: u8 = 248;
const POST_POST: u8 = 249;
const BOP: u8 = 139;
const EOP: u8 = 140;
const PUSH: u8 = 141;
const POP: u8 = 142;
const RIGHT1: u8 = 143;
const DOWN1: u8 = 157;
const FNT_NUM_0: u8 = 171;
const FNT1: u8 = 235;
const FNT2: u8 = 236;
const FNT3: u8 = 237;
const FNT4: u8 = 238;
const XXX1: u8 = 239;
const XXX4: u8 = 242;
const FNT_DEF1: u8 = 243;
const FNT_DEF2: u8 = 244;
const FNT_DEF3: u8 = 245;
const FNT_DEF4: u8 = 246;
const PADDING: u8 = 223;

const NUM: i32 = 25_400_000;
const DEN: i32 = 473_628_672;
const BILLION: i64 = 1_000_000_000;

/// DVI emission failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DviError {
    NoPages,
    EmptyFontName { font_id: u32 },
    FieldTooLong { field: &'static str, len: usize },
    MissingFont { font_id: u32 },
    MissingEffect { effect_index: u32 },
    CharacterOutOfRange { ch: u32 },
    InconsistentJobInfo,
    TooManyPages { pages: usize },
    SpecialTooLong { len: usize },
    OffsetOverflow { offset: usize },
    PositionOverflow,
}

impl fmt::Display for DviError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoPages => f.write_str("cannot write DVI without page artifacts"),
            Self::EmptyFontName { font_id } => {
                write!(f, "font resource {font_id} has an empty DVI font name")
            }
            Self::FieldTooLong { field, len } => {
                write!(f, "DVI {field} length {len} exceeds 255 bytes")
            }
            Self::MissingFont { font_id } => {
                write!(f, "page node references missing font resource {font_id}")
            }
            Self::MissingEffect { effect_index } => {
                write!(f, "page node references missing effect {effect_index}")
            }
            Self::CharacterOutOfRange { ch } => {
                write!(f, "DVI TeX82 character code {ch} is outside 0..=255")
            }
            Self::InconsistentJobInfo => {
                f.write_str("page artifacts disagree on job banner or magnification")
            }
            Self::TooManyPages { pages } => write!(f, "DVI page count {pages} exceeds 65535"),
            Self::SpecialTooLong { len } => {
                write!(
                    f,
                    "DVI special payload length {len} exceeds signed 32-bit range"
                )
            }
            Self::OffsetOverflow { offset } => {
                write!(
                    f,
                    "DVI byte offset {offset} exceeds signed 32-bit pointer range"
                )
            }
            Self::PositionOverflow => f.write_str("DVI page position arithmetic overflowed"),
        }
    }
}

impl std::error::Error for DviError {}

/// Writes a complete DVI file from committed page artifacts.
///
/// The writer is intentionally downstream-only: all DVI preamble data, page
/// counters, dimensions, and font resources come from the artifact stream.
pub fn write_dvi(pages: &[PageArtifact]) -> Result<Vec<u8>, DviError> {
    DviWriter::new(pages)?.finish()
}

struct DviWriter<'a> {
    pages: &'a [PageArtifact],
    bytes: Vec<u8>,
    fonts: Vec<DefinedFont<'a>>,
    previous_bop: i32,
    max_height_depth: i32,
    max_width: i32,
    max_stack_depth: u16,
    right_stack: MovementStack,
    down_stack: MovementStack,
    dvi_h: Scaled,
    dvi_v: Scaled,
    cur_h: Scaled,
    cur_v: Scaled,
    dvi_f: Option<u32>,
    cur_s: i32,
}

impl<'a> DviWriter<'a> {
    fn new(pages: &'a [PageArtifact]) -> Result<Self, DviError> {
        let Some(first) = pages.first() else {
            return Err(DviError::NoPages);
        };
        for page in pages {
            if page.job != first.job {
                return Err(DviError::InconsistentJobInfo);
            }
        }
        let page_count = u16::try_from(pages.len())
            .map_err(|_| DviError::TooManyPages { pages: pages.len() })?;
        let mut writer = Self {
            pages,
            bytes: Vec::new(),
            fonts: Vec::new(),
            previous_bop: -1,
            max_height_depth: 0,
            max_width: 0,
            max_stack_depth: 0,
            right_stack: MovementStack::default(),
            down_stack: MovementStack::default(),
            dvi_h: Scaled::from_raw(0),
            dvi_v: Scaled::from_raw(0),
            cur_h: Scaled::from_raw(0),
            cur_v: Scaled::from_raw(0),
            dvi_f: None,
            cur_s: -1,
        };
        writer.preamble(&first.job.banner, first.job.mag)?;
        debug_assert_eq!(page_count as usize, pages.len());
        Ok(writer)
    }

    fn finish(mut self) -> Result<Vec<u8>, DviError> {
        for page in self.pages {
            self.page(page)?;
        }
        self.postamble()?;
        Ok(self.bytes)
    }

    fn preamble(&mut self, banner: &str, mag: i32) -> Result<(), DviError> {
        let banner = limited_bytes("comment", banner)?;
        self.u8(PRE);
        self.u8(ID_BYTE);
        self.i32(NUM);
        self.i32(DEN);
        self.i32(mag);
        self.u8(banner.len() as u8);
        self.raw(banner);
        Ok(())
    }

    fn page(&mut self, page: &'a PageArtifact) -> Result<(), DviError> {
        self.reset_page_state();
        let bop_location = self.current_pointer()?;
        self.u8(BOP);
        for count in page.counts {
            self.i32(count);
        }
        self.i32(self.previous_bop);
        self.previous_bop = bop_location;

        let extent = page_extent(&page.root);
        self.max_height_depth = self.max_height_depth.max(extent.height_depth);
        self.max_width = self.max_width.max(extent.width);
        self.ship_box(page, &page.root)?;
        self.u8(EOP);
        Ok(())
    }

    fn reset_page_state(&mut self) {
        self.right_stack.clear();
        self.down_stack.clear();
        self.dvi_h = Scaled::from_raw(0);
        self.dvi_v = Scaled::from_raw(0);
        self.cur_h = Scaled::from_raw(0);
        self.cur_v = Scaled::from_raw(0);
        self.dvi_f = None;
        self.cur_s = -1;
    }

    fn ship_box(&mut self, page: &'a PageArtifact, node: &'a PageNode) -> Result<(), DviError> {
        match node {
            PageNode::HList(box_node) => {
                // tex.web ship_out: cur_v := height(p) + v_offset.
                self.cur_v = box_node.height;
                self.hlist_out(page, box_node)?;
            }
            PageNode::VList(box_node) => {
                // tex.web ship_out: cur_v := height(p) + v_offset.
                self.cur_v = box_node.height;
                self.vlist_out(page, box_node)?;
            }
            PageNode::Char { .. }
            | PageNode::Lig { .. }
            | PageNode::Kern { .. }
            | PageNode::Glue { .. }
            | PageNode::Penalty(_)
            | PageNode::Rule { .. }
            | PageNode::Unset
            | PageNode::Disc { .. }
            | PageNode::Mark { .. }
            | PageNode::Insert { .. }
            | PageNode::WhatsitAnchor { .. }
            | PageNode::MathOn(_)
            | PageNode::MathOff(_)
            | PageNode::Adjust(_) => {}
        }
        Ok(())
    }

    fn hlist_out(&mut self, page: &'a PageArtifact, this_box: &'a BoxNode) -> Result<(), DviError> {
        let g_order = this_box.glue_order;
        let g_sign = this_box.glue_sign;
        self.enter_box();
        if self.cur_s > 0 {
            self.u8(PUSH);
        }
        let save_loc = self.bytes.len();
        let base_line = self.cur_v;
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
                    self.change_font(page, *font_id)?;
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
                PageNode::Glue { spec, .. } => {
                    let rule_wd = adjusted_glue_width(
                        *spec,
                        g_sign,
                        g_order,
                        this_box.glue_set,
                        &mut cur_glue,
                        &mut cur_g,
                    )?;
                    self.cur_h = add_scaled(self.cur_h, rule_wd)?;
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
                | PageNode::Unset
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

    fn vlist_out(&mut self, page: &'a PageArtifact, this_box: &'a BoxNode) -> Result<(), DviError> {
        let g_order = this_box.glue_order;
        let g_sign = this_box.glue_sign;
        self.enter_box();
        if self.cur_s > 0 {
            self.u8(PUSH);
        }
        let save_loc = self.bytes.len();
        let left_edge = self.cur_h;
        self.cur_v = sub_scaled(self.cur_v, this_box.height)?;
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
                    self.cur_v = add_scaled(self.cur_v, rule_ht)?;
                    if rule_ht.raw() > 0 && rule_wd.raw() > 0 {
                        self.synch_h()?;
                        self.synch_v()?;
                        self.u8(PUT_RULE);
                        self.scaled(rule_ht);
                        self.scaled(rule_wd);
                    }
                }
                PageNode::Glue { spec, .. } => {
                    let rule_ht = adjusted_glue_width(
                        *spec,
                        g_sign,
                        g_order,
                        this_box.glue_set,
                        &mut cur_glue,
                        &mut cur_g,
                    )?;
                    self.cur_v = add_scaled(self.cur_v, rule_ht)?;
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
                | PageNode::Unset
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
        page: &'a PageArtifact,
        box_node: &'a BoxNode,
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
        self.cur_v = sub_scaled(base_line, box_node.shift)?;
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
        page: &'a PageArtifact,
        box_node: &'a BoxNode,
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

    fn output_rule_in_hlist(
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

    fn enter_box(&mut self) {
        self.cur_s += 1;
        if let Ok(depth) = u16::try_from(self.cur_s) {
            self.max_stack_depth = self.max_stack_depth.max(depth);
        }
    }

    fn synch_h(&mut self) -> Result<(), DviError> {
        if self.cur_h != self.dvi_h {
            let movement = sub_scaled(self.cur_h, self.dvi_h)?;
            self.right_stack.movement(&mut self.bytes, movement, RIGHT1);
            self.dvi_h = self.cur_h;
        }
        Ok(())
    }

    fn synch_v(&mut self) -> Result<(), DviError> {
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

    fn change_font(&mut self, page: &'a PageArtifact, font_id: u32) -> Result<(), DviError> {
        let font = page_font(page, font_id)?;
        let number = self.ensure_font_defined(font)?;
        if self.dvi_f == Some(number) {
            return Ok(());
        }
        match number {
            0..=63 => self.u8(FNT_NUM_0 + number as u8),
            64..=0xff => {
                self.u8(FNT1);
                self.u8(number as u8);
            }
            0x100..=0xffff => {
                self.u8(FNT2);
                self.u16(number as u16);
            }
            0x1_0000..=0xff_ffff => {
                self.u8(FNT3);
                self.u24(number);
            }
            _ => {
                self.u8(FNT4);
                self.u32(number);
            }
        }
        self.dvi_f = Some(number);
        Ok(())
    }

    fn set_char(&mut self, ch: u32) -> Result<(), DviError> {
        let ch = u8::try_from(ch).map_err(|_| DviError::CharacterOutOfRange { ch })?;
        if ch < SET1 {
            self.u8(ch);
        } else {
            self.u8(SET1);
            self.u8(ch);
        }
        Ok(())
    }

    fn out_what(&mut self, page: &'a PageArtifact, effect_index: u32) -> Result<(), DviError> {
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

    fn ensure_font_defined(&mut self, font: &'a FontResource) -> Result<u32, DviError> {
        let key = FontKey::from(font);
        if let Some(defined) = self.fonts.iter().find(|defined| defined.key == key) {
            return Ok(defined.number);
        }
        let number = u32::try_from(self.fonts.len()).expect("DVI font count exceeds u32");
        self.fnt_def(number, font)?;
        self.fonts.push(DefinedFont { number, key, font });
        Ok(number)
    }

    fn postamble(&mut self) -> Result<(), DviError> {
        let final_bop = self.previous_bop;
        let post_location = self.current_pointer()?;
        let mag = self.pages[0].job.mag;
        let total_pages = u16::try_from(self.pages.len()).map_err(|_| DviError::TooManyPages {
            pages: self.pages.len(),
        })?;

        self.u8(POST);
        self.i32(final_bop);
        self.i32(NUM);
        self.i32(DEN);
        self.i32(mag);
        self.i32(self.max_height_depth);
        self.i32(self.max_width);
        self.u16(self.max_stack_depth);
        self.u16(total_pages);

        for defined in self.fonts.clone() {
            self.fnt_def(defined.number, defined.font)?;
        }

        self.u8(POST_POST);
        self.i32(post_location);
        self.u8(ID_BYTE);
        for _ in 0..4 {
            self.u8(PADDING);
        }
        while !self.bytes.len().is_multiple_of(4) {
            self.u8(PADDING);
        }
        Ok(())
    }

    fn fnt_def(&mut self, number: u32, font: &FontResource) -> Result<(), DviError> {
        let name = limited_bytes("font name", &font.name)?;
        if name.is_empty() {
            return Err(DviError::EmptyFontName {
                font_id: font.font_id,
            });
        }
        match number {
            0..=0xff => {
                self.u8(FNT_DEF1);
                self.u8(number as u8);
            }
            0x100..=0xffff => {
                self.u8(FNT_DEF2);
                self.u16(number as u16);
            }
            0x1_0000..=0xff_ffff => {
                self.u8(FNT_DEF3);
                self.u24(number);
            }
            _ => {
                self.u8(FNT_DEF4);
                self.u32(number);
            }
        }
        self.u32(font.tfm_checksum);
        self.scaled(font.at_size);
        self.scaled(font.design_size);
        self.u8(0);
        self.u8(name.len() as u8);
        self.raw(name);
        Ok(())
    }

    fn current_pointer(&self) -> Result<i32, DviError> {
        i32::try_from(self.bytes.len()).map_err(|_| DviError::OffsetOverflow {
            offset: self.bytes.len(),
        })
    }

    fn raw(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn u24(&mut self, value: u32) {
        let bytes = value.to_be_bytes();
        self.bytes.extend_from_slice(&bytes[1..]);
    }

    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn i32(&mut self, value: i32) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn scaled(&mut self, value: Scaled) {
        self.i32(value.raw());
    }
}

#[derive(Clone, Debug)]
struct DefinedFont<'a> {
    number: u32,
    key: FontKey,
    font: &'a FontResource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FontKey {
    name: String,
    tfm_checksum: u32,
    design_size: Scaled,
    at_size: Scaled,
}

impl From<&FontResource> for FontKey {
    fn from(font: &FontResource) -> Self {
        Self {
            name: font.name.clone(),
            tfm_checksum: font.tfm_checksum,
            design_size: font.design_size,
            at_size: font.at_size,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PageExtent {
    height_depth: i32,
    width: i32,
}

fn page_extent(node: &PageNode) -> PageExtent {
    match node {
        PageNode::HList(box_node) | PageNode::VList(box_node) => box_extent(box_node),
        PageNode::Rule {
            width,
            height,
            depth,
        } => PageExtent {
            height_depth: optional_raw(*height) + optional_raw(*depth),
            width: optional_raw(*width),
        },
        PageNode::MathOn(width) | PageNode::MathOff(width) => PageExtent {
            height_depth: 0,
            width: width.raw(),
        },
        PageNode::Char { .. }
        | PageNode::Lig { .. }
        | PageNode::Kern { .. }
        | PageNode::Glue { .. }
        | PageNode::Penalty(_)
        | PageNode::Unset
        | PageNode::Disc { .. }
        | PageNode::Mark { .. }
        | PageNode::Insert { .. }
        | PageNode::WhatsitAnchor { .. }
        | PageNode::Adjust(_) => PageExtent::default(),
    }
}

fn box_extent(box_node: &BoxNode) -> PageExtent {
    PageExtent {
        height_depth: box_node.height.raw() + box_node.depth.raw(),
        width: box_node.width.raw(),
    }
}

fn optional_raw(value: Option<Scaled>) -> i32 {
    value.map_or(0, Scaled::raw)
}

fn adjusted_glue_width(
    spec: GlueSpec,
    g_sign: GlueSign,
    g_order: GlueOrder,
    glue_set: GlueSetRatio,
    cur_glue: &mut Scaled,
    cur_g: &mut Scaled,
) -> Result<Scaled, DviError> {
    // tex.web hlist_out/vlist_out: rule_wd/rule_ht := width(g) - cur_g,
    // then cur_g becomes round(glue_set(this_box) * cur_glue).
    let base = sub_scaled(spec.width, *cur_g)?;
    if g_sign != GlueSign::Normal {
        match g_sign {
            GlueSign::Stretching if spec.stretch_order == g_order => {
                *cur_glue = add_scaled(*cur_glue, spec.stretch)?;
                *cur_g = rounded_glue_set(glue_set, *cur_glue);
            }
            GlueSign::Shrinking if spec.shrink_order == g_order => {
                *cur_glue = sub_scaled(*cur_glue, spec.shrink)?;
                *cur_g = rounded_glue_set(glue_set, *cur_glue);
            }
            _ => {}
        }
    }
    add_scaled(base, *cur_g)
}

fn rounded_glue_set(glue_set: GlueSetRatio, cur_glue: Scaled) -> Scaled {
    let product = i128::from(glue_set.raw()) * i128::from(cur_glue.raw());
    let rounded = rounded_div(product, i128::from(GLUE_SET_RATIO_SCALE));
    let vetted = rounded.clamp(-i128::from(BILLION), i128::from(BILLION));
    Scaled::from_raw(i32::try_from(vetted).expect("vetted glue is in i32 range"))
}

fn rounded_div(value: i128, divisor: i128) -> i128 {
    debug_assert!(divisor > 0);
    if value >= 0 {
        (value + divisor / 2) / divisor
    } else {
        -((-value + divisor / 2) / divisor)
    }
}

fn add_scaled(left: Scaled, right: Scaled) -> Result<Scaled, DviError> {
    left.checked_add(right).ok_or(DviError::PositionOverflow)
}

fn sub_scaled(left: Scaled, right: Scaled) -> Result<Scaled, DviError> {
    left.checked_sub(right).ok_or(DviError::PositionOverflow)
}

fn page_font(page: &PageArtifact, font_id: u32) -> Result<&FontResource, DviError> {
    page.fonts
        .iter()
        .find(|font| font.font_id == font_id)
        .ok_or(DviError::MissingFont { font_id })
}

fn limited_bytes<'a>(field: &'static str, value: &'a str) -> Result<&'a [u8], DviError> {
    let bytes = value.as_bytes();
    if bytes.len() > u8::MAX as usize {
        return Err(DviError::FieldTooLong {
            field,
            len: bytes.len(),
        });
    }
    Ok(bytes)
}
