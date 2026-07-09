use tex_arith::Scaled;

use crate::{FontResource, PageArtifact};

use super::{
    DviError, DviWriter,
    framing::limited_bytes,
    opcodes::{FNT_DEF1, FNT_DEF2, FNT_DEF3, FNT_DEF4, FNT_NUM_0, FNT1, FNT2, FNT3, FNT4, SET1},
};

impl<'a> DviWriter<'a> {
    pub(super) fn change_font(
        &mut self,
        page: &'a PageArtifact,
        font_id: u32,
    ) -> Result<(), DviError> {
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

    pub(super) fn set_char(&mut self, ch: u32) -> Result<(), DviError> {
        let ch = u8::try_from(ch).map_err(|_| DviError::CharacterOutOfRange { ch })?;
        if ch < SET1 {
            self.u8(ch);
        } else {
            self.u8(SET1);
            self.u8(ch);
        }
        Ok(())
    }

    fn ensure_font_defined(&mut self, font: &'a FontResource) -> Result<u32, DviError> {
        let key = FontKey::from(font);
        if let Some(defined) = self.fonts.iter().find(|defined| defined.key == key) {
            return Ok(defined.number);
        }
        let number = font.font_id;
        self.fnt_def(number, font)?;
        self.fonts.push(DefinedFont { number, key, font });
        Ok(number)
    }

    pub(super) fn fnt_def(&mut self, number: u32, font: &FontResource) -> Result<(), DviError> {
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
}

#[derive(Clone, Debug)]
pub(super) struct DefinedFont<'a> {
    pub(super) number: u32,
    key: FontKey,
    pub(super) font: &'a FontResource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FontKey {
    font_id: u32,
    name: String,
    tfm_checksum: u32,
    design_size: Scaled,
    at_size: Scaled,
}

impl From<&FontResource> for FontKey {
    fn from(font: &FontResource) -> Self {
        Self {
            font_id: font.font_id,
            name: font.name.clone(),
            tfm_checksum: font.tfm_checksum,
            design_size: font.design_size,
            at_size: font.at_size,
        }
    }
}

fn page_font(page: &PageArtifact, font_id: u32) -> Result<&FontResource, DviError> {
    page.fonts
        .iter()
        .find(|font| font.font_id == font_id)
        .ok_or(DviError::MissingFont { font_id })
}
