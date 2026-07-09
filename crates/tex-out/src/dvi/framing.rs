use tex_arith::Scaled;

use crate::{PageArtifact, PageNode};

use super::{
    DviError, DviWriter,
    extent::page_extent,
    opcodes::{BOP, DEN, EOP, ID_BYTE, NUM, PADDING, POST, POST_POST, PRE},
};

impl<'a> DviWriter<'a> {
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

    pub(super) fn page(&mut self, page: &'a PageArtifact) -> Result<(), DviError> {
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

        let mut defined_fonts = self.fonts.clone();
        defined_fonts.sort_by(|left, right| right.number.cmp(&left.number));
        for defined in defined_fonts {
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

    pub(super) fn current_pointer(&self) -> Result<i32, DviError> {
        i32::try_from(self.bytes.len()).map_err(|_| DviError::OffsetOverflow {
            offset: self.bytes.len(),
        })
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
