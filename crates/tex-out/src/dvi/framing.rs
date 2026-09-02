use tex_arith::Scaled;

use super::{
    DviBodyCompiler, DviError, DviFileWriter,
    opcodes::{DEN, ID_BYTE, NUM, PADDING, POST, POST_POST, PRE},
};

// TeX82 map: `Initialize variables as ship_out begins`, `Ship box p out`,
// and `Finish the DVI file` in `tex.web`.  Preamble conversion fields,
// bop/count/backpointer before traversal, eop after traversal, postamble
// pointer/conversion/mag/maxima/stack/page fields, descending used-font
// definitions, post_post pointer/id, and at least four 223 bytes through a
// four-byte boundary retain TeX's ordering.  Umber's streaming writer and
// detached per-page font index are policy; they must not alter those bytes or
// the previous-bop chain.

impl<W: std::io::Write> DviFileWriter<W> {
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

    pub(super) fn current_pointer(&self) -> Result<i32, DviError> {
        let offset = self.current_offset()?;
        i32::try_from(offset).map_err(|_| DviError::OffsetOverflow { offset })
    }

    pub(super) fn current_offset(&self) -> Result<usize, DviError> {
        self.committed_offset
            .checked_add(self.bytes.len())
            .ok_or(DviError::OffsetOverflow { offset: usize::MAX })
    }

    pub(super) fn postamble(&mut self) -> Result<(), DviError> {
        use std::cmp::Reverse;

        let final_bop = self.previous_bop;
        let post_location = self.current_pointer()?;
        let mag = self.job_mag.expect("postamble requires one page");
        let total_pages = self.page_count;
        let max_height_depth = self.max_height_depth;
        let max_width = self.max_width;
        let max_stack_depth = self.max_stack_depth;

        self.u8(POST);
        self.i32(final_bop);
        self.i32(NUM);
        self.i32(DEN);
        self.i32(mag);
        self.i32(max_height_depth);
        self.i32(max_width);
        self.u16(max_stack_depth);
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
}

impl DviBodyCompiler {
    pub(super) fn reset_page_state(&mut self) {
        self.right_stack.clear();
        self.down_stack.clear();
        self.dvi_h = Scaled::from_raw(0);
        self.dvi_v = Scaled::from_raw(0);
        self.cur_h = Scaled::from_raw(0);
        self.cur_v = Scaled::from_raw(0);
        self.dvi_f = None;
        self.cur_s = -1;
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
