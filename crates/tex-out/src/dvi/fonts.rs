use tex_arith::Scaled;

use crate::{ContentHash, FontResource, PageArtifact};

use super::{
    DviError, DviWriter,
    framing::limited_bytes,
    opcodes::{FNT_DEF1, FNT_DEF2, FNT_DEF3, FNT_DEF4, FNT_NUM_0, FNT1, FNT2, FNT3, FNT4, SET1},
};

// TeX82 map: `dvi_font_def`, `Output the font name`, and `Change font dvi_f
// to f` in `tex.web`.  A font definition must precede its first selection;
// checksum, scaled size, design size, area length, name length, then name are
// emitted in that order.  framing.rs mirrors TeX's final descending
// `font_ptr` walk when it repeats used definitions in the postamble.
//
// Umber policy: detached artifacts carry stable u32 resource numbers and no
// separate TeX font-area string, so the area length is zero; the shortest DVI
// fnt/fnt_def width is selected instead of TeX82's at-most-256-font shortcut.
// Cross-page identity checks prevent one DVI number from changing meaning.

impl<W: std::io::Write> DviWriter<W> {
    pub(super) fn index_page_fonts(&mut self, page: &PageArtifact) -> Result<(), DviError> {
        self.index_fonts(&page.fonts)
    }

    pub(super) fn index_fonts(&mut self, fonts: &[FontResource]) -> Result<(), DviError> {
        self.page_fonts.clear();
        self.add_page_fonts(fonts)
    }

    pub(super) fn add_page_fonts(&mut self, fonts: &[FontResource]) -> Result<(), DviError> {
        for font in fonts {
            let key = FontKey::from(font);
            if self
                .fonts_by_number
                .insert(font.font_id, key.clone())
                .is_some_and(|existing| existing != key)
            {
                return Err(DviError::InconsistentFontResource {
                    font_id: font.font_id,
                });
            }
            self.page_fonts.insert(font.font_id, font.clone());
        }
        Ok(())
    }

    pub(super) fn change_font(&mut self, font_id: u32) -> Result<(), DviError> {
        // Glyph runs overwhelmingly repeat the selected font.  The DVI font
        // number is the artifact's stable `font_id`, so this check is both
        // sufficient and much cheaper than cloning and re-keying the page
        // resource for every character in the run.
        if self.dvi_f == Some(font_id) {
            return Ok(());
        }
        let font = self
            .page_fonts
            .get(&font_id)
            .cloned()
            .ok_or(DviError::MissingFont { font_id })?;
        let number = self.ensure_font_defined(&font)?;
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

    fn ensure_font_defined(&mut self, font: &FontResource) -> Result<u32, DviError> {
        let key = FontKey::from(font);
        if let Some(defined) = self.fonts.get(&key) {
            return Ok(defined.number);
        }
        let number = font.font_id;
        let definition_start = self.bytes.len();
        self.fnt_def(number, font)?;
        if let Some(sites) = &mut self.font_definition_sites {
            sites.push(super::plan::FontDefinitionSite {
                font_id: number,
                start: definition_start,
                end: self.bytes.len(),
            });
        }
        self.fonts.insert(
            key.clone(),
            DefinedFont {
                number,
                font: font.clone(),
            },
        );
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
pub(super) struct DefinedFont {
    pub(super) number: u32,
    pub(super) font: FontResource,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct FontKey {
    font_id: u32,
    name: String,
    tfm_content_hash: ContentHash,
    tfm_checksum: u32,
    design_size: Scaled,
    at_size: Scaled,
}

impl From<&FontResource> for FontKey {
    fn from(font: &FontResource) -> Self {
        Self {
            font_id: font.font_id,
            name: font.name.clone(),
            tfm_content_hash: font.tfm_content_hash,
            tfm_checksum: font.tfm_checksum,
            design_size: font.design_size,
            at_size: font.at_size,
        }
    }
}
