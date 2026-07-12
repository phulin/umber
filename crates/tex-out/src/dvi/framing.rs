use std::cmp::Reverse;

use tex_arith::Scaled;

use crate::{PageArtifact, PageNode};

use super::{
    DviError, DviWriter,
    extent::page_extent,
    opcodes::{BOP, DEN, EOP, ID_BYTE, NUM, PADDING, POST, POST_POST, PRE},
};

// TeX82 map: `Initialize variables as ship_out begins`, `Ship box p out`,
// and `Finish the DVI file` in `tex.web`.  Preamble conversion fields,
// bop/count/backpointer before traversal, eop after traversal, postamble
// pointer/conversion/mag/maxima/stack/page fields, descending used-font
// definitions, post_post pointer/id, and at least four 223 bytes through a
// four-byte boundary retain TeX's ordering.  Umber's streaming writer and
// detached per-page font index are policy; they must not alter those bytes or
// the previous-bop chain.

impl<W: std::io::Write> DviWriter<W> {
    pub(super) fn preamble(&mut self, banner: &str, mag: i32) -> Result<(), DviError> {
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

    pub(super) fn page(&mut self, page: &PageArtifact) -> Result<(), DviError> {
        self.index_page_fonts(page)?;
        self.reset_page_state();
        let bop_location = self.current_pointer()?;
        self.u8(BOP);
        for count in page.counts {
            self.i32(count);
        }
        self.i32(self.previous_bop);
        self.previous_bop = bop_location;

        let extent = page_extent(&page.root);
        let height_depth = extent
            .height_depth
            .checked_add(page.job.v_offset.raw())
            .ok_or(DviError::PositionOverflow)?;
        let width = extent
            .width
            .checked_add(page.job.h_offset.raw())
            .ok_or(DviError::PositionOverflow)?;
        self.max_height_depth = self.max_height_depth.max(height_depth);
        self.max_width = self.max_width.max(width);
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

    fn ship_box(&mut self, page: &PageArtifact, node: &PageNode) -> Result<(), DviError> {
        // tex.web `Initialize variables as ship_out begins` and `Ship box p out`:
        // the page reference point includes both dimension parameters before
        // hlist_out/vlist_out performs its normal traversal.
        self.cur_h = page.job.h_offset;
        match node {
            PageNode::HList(box_node) => {
                // tex.web ship_out: cur_v := height(p) + v_offset.
                self.cur_v = box_node
                    .height
                    .checked_add(page.job.v_offset)
                    .ok_or(DviError::PositionOverflow)?;
                self.hlist_out(page, box_node)?;
            }
            PageNode::VList(box_node) => {
                // tex.web ship_out: cur_v := height(p) + v_offset.
                self.cur_v = box_node
                    .height
                    .checked_add(page.job.v_offset)
                    .ok_or(DviError::PositionOverflow)?;
                self.vlist_out(page, box_node)?;
            }
            PageNode::Char { .. }
            | PageNode::Lig { .. }
            | PageNode::Kern { .. }
            | PageNode::Glue { .. }
            | PageNode::Penalty(_)
            | PageNode::Rule { .. }
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

    pub(super) fn postamble(&mut self) -> Result<(), DviError> {
        let final_bop = self.previous_bop;
        let post_location = self.current_pointer()?;
        let mag = self.job_mag.expect("postamble requires one page");
        let total_pages = self.page_count;

        self.u8(POST);
        self.i32(final_bop);
        self.i32(NUM);
        self.i32(DEN);
        self.i32(mag);
        self.i32(self.max_height_depth);
        self.i32(self.max_width);
        self.u16(self.max_stack_depth);
        self.u16(total_pages);

        let mut defined_fonts: Vec<_> = self.fonts.values().cloned().collect();
        defined_fonts.sort_by_key(|defined| Reverse(defined.number));
        for defined in defined_fonts {
            self.fnt_def(defined.number, &defined.font)?;
        }

        self.u8(POST_POST);
        self.i32(post_location);
        self.u8(ID_BYTE);
        for _ in 0..4 {
            self.u8(PADDING);
        }
        while !self.current_offset()?.is_multiple_of(4) {
            self.u8(PADDING);
        }
        Ok(())
    }

    pub(super) fn current_pointer(&self) -> Result<i32, DviError> {
        let offset = self.current_offset()?;
        i32::try_from(offset).map_err(|_| DviError::OffsetOverflow { offset })
    }

    fn current_offset(&self) -> Result<usize, DviError> {
        self.committed_offset
            .checked_add(self.bytes.len())
            .ok_or(DviError::OffsetOverflow { offset: usize::MAX })
    }

    pub(super) fn raw(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    pub(super) fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub(super) fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    pub(super) fn u24(&mut self, value: u32) {
        let bytes = value.to_be_bytes();
        self.bytes.extend_from_slice(&bytes[1..]);
    }

    pub(super) fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    pub(super) fn i32(&mut self, value: i32) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    pub(super) fn scaled(&mut self, value: Scaled) {
        self.i32(value.raw());
    }
}

pub(super) fn limited_bytes<'a>(field: &'static str, value: &'a str) -> Result<&'a [u8], DviError> {
    let bytes = value.as_bytes();
    if bytes.len() > u8::MAX as usize {
        return Err(DviError::FieldTooLong {
            field,
            len: bytes.len(),
        });
    }
    Ok(bytes)
}
