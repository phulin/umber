//! Bounded, host-neutral PK bitmap font decoding.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

const PK_PRE: u8 = 247;
const PK_ID: u8 = 89;
const PK_POST: u8 = 245;
const PK_NO_OP: u8 = 246;
const MAX_PK_BYTES: usize = 16 * 1024 * 1024;
const MAX_GLYPHS: usize = 65_536;
const MAX_GLYPH_DIMENSION: u32 = 16_384;
const MAX_DECODED_BITMAP_BYTES: usize = 64 * 1024 * 1024;

/// Complete host-neutral identity of one requested PK bitmap program.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PdfPkFontRequest {
    tex_name: Vec<u8>,
    dpi: u32,
    mode: Vec<u8>,
}

impl PdfPkFontRequest {
    #[must_use]
    pub fn new(tex_name: Vec<u8>, dpi: u32, mode: Vec<u8>) -> Self {
        Self {
            tex_name,
            dpi,
            mode,
        }
    }

    #[must_use]
    pub fn tex_name(&self) -> &[u8] {
        &self.tex_name
    }

    #[must_use]
    pub const fn dpi(&self) -> u32 {
        self.dpi
    }

    #[must_use]
    pub fn mode(&self) -> &[u8] {
        &self.mode
    }

    #[must_use]
    pub fn logical_name(&self) -> Vec<u8> {
        let mut name = self.tex_name.clone();
        name.push(b'.');
        name.extend_from_slice(self.dpi.to_string().as_bytes());
        name.extend_from_slice(b"pk");
        name
    }
}

/// Stable identity of the exact acquired PK program.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfPkFontIdentity([u8; 32]);

impl PdfPkFontIdentity {
    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

/// One decoded PK glyph. `bitmap` is a row-byte-aligned, most-significant-bit
/// first mask whose set bits are black pixels.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfPkGlyph {
    pub code: u32,
    pub tfm_width: u32,
    pub horizontal_escapement: i32,
    pub width: u32,
    pub height: u32,
    pub x_offset: i32,
    pub y_offset: i32,
    pub bitmap: Vec<u8>,
}

/// An immutable decoded PK bitmap program.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfPkFont {
    identity: PdfPkFontIdentity,
    design_size: u32,
    checksum: u32,
    horizontal_pixels_per_point: u32,
    vertical_pixels_per_point: u32,
    glyphs: BTreeMap<u32, PdfPkGlyph>,
}

impl PdfPkFont {
    pub fn parse(bytes: &[u8]) -> Result<Self, PdfPkFontError> {
        if bytes.len() > MAX_PK_BYTES {
            return Err(PdfPkFontError::InputTooLarge);
        }
        let identity = PdfPkFontIdentity(Sha256::digest(bytes).into());
        let mut cursor = Cursor::new(bytes);
        if cursor.u8()? != PK_PRE || cursor.u8()? != PK_ID {
            return Err(PdfPkFontError::InvalidPreamble);
        }
        let comment_length = usize::from(cursor.u8()?);
        cursor.skip(comment_length)?;
        let design_size = cursor.u32()?;
        let checksum = cursor.u32()?;
        let horizontal_pixels_per_point = cursor.u32()?;
        let vertical_pixels_per_point = cursor.u32()?;
        let mut glyphs = BTreeMap::new();
        let mut decoded_bytes = 0usize;
        let mut saw_post = false;

        while !cursor.is_empty() {
            let flag = cursor.u8()?;
            match flag {
                0..=239 => {
                    if glyphs.len() == MAX_GLYPHS {
                        return Err(PdfPkFontError::TooManyGlyphs);
                    }
                    let glyph = parse_glyph(flag, &mut cursor)?;
                    decoded_bytes = decoded_bytes
                        .checked_add(glyph.bitmap.len())
                        .filter(|total| *total <= MAX_DECODED_BITMAP_BYTES)
                        .ok_or(PdfPkFontError::DecodedBitmapLimit)?;
                    if glyphs.insert(glyph.code, glyph).is_some() {
                        return Err(PdfPkFontError::DuplicateGlyph);
                    }
                }
                240..=243 => {
                    let length = match flag {
                        240 => u32::from(cursor.u8()?),
                        241 => u32::from(cursor.u16()?),
                        242 => cursor.u24()?,
                        243 => cursor.u32()?,
                        _ => unreachable!(),
                    };
                    cursor.skip(usize::try_from(length).map_err(|_| PdfPkFontError::Truncated)?)?;
                }
                244 => cursor.skip(4)?,
                PK_POST => {
                    saw_post = true;
                    break;
                }
                PK_NO_OP => {}
                _ => return Err(PdfPkFontError::InvalidCommand(flag)),
            }
        }
        if !saw_post {
            return Err(PdfPkFontError::MissingPostamble);
        }
        while !cursor.is_empty() {
            if cursor.u8()? != PK_NO_OP {
                return Err(PdfPkFontError::TrailingData);
            }
        }
        Ok(Self {
            identity,
            design_size,
            checksum,
            horizontal_pixels_per_point,
            vertical_pixels_per_point,
            glyphs,
        })
    }

    #[must_use]
    pub const fn identity(&self) -> PdfPkFontIdentity {
        self.identity
    }

    #[must_use]
    pub const fn design_size(&self) -> u32 {
        self.design_size
    }

    #[must_use]
    pub const fn checksum(&self) -> u32 {
        self.checksum
    }

    #[must_use]
    pub const fn horizontal_pixels_per_point(&self) -> u32 {
        self.horizontal_pixels_per_point
    }

    #[must_use]
    pub const fn vertical_pixels_per_point(&self) -> u32 {
        self.vertical_pixels_per_point
    }

    #[must_use]
    pub fn glyph(&self, code: u32) -> Option<&PdfPkGlyph> {
        self.glyphs.get(&code)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfPkFontError {
    InputTooLarge,
    InvalidPreamble,
    Truncated,
    MissingPostamble,
    TrailingData,
    InvalidCommand(u8),
    PacketLength,
    InvalidDimensions,
    DecodedBitmapLimit,
    TooManyGlyphs,
    DuplicateGlyph,
    InvalidPackedNumber,
    RasterOverflow,
    RasterUnderflow,
    InvalidRepeatCount,
}

impl std::fmt::Display for PdfPkFontError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid PK bitmap font: {self:?}")
    }
}

impl std::error::Error for PdfPkFontError {}

fn parse_glyph(flag: u8, cursor: &mut Cursor<'_>) -> Result<PdfPkGlyph, PdfPkFontError> {
    let dyn_f = flag >> 4;
    let first_black = flag & 8 != 0;
    let format = flag & 7;
    let (packet_length, length_bytes) = match format {
        0..=3 => (u32::from(format) * 256 + u32::from(cursor.u8()?), 1usize),
        4..=6 => (u32::from(format - 4) * 65_536 + u32::from(cursor.u16()?), 2),
        7 => (cursor.u32()?, 4),
        _ => unreachable!(),
    };
    let packet_length = usize::try_from(packet_length)
        .ok()
        .and_then(|length| length.checked_add(length_bytes))
        .ok_or(PdfPkFontError::PacketLength)?;
    let packet = cursor.take(packet_length)?;
    let mut packet = Cursor::new(packet);
    let (code, tfm_width, horizontal_escapement, width, height, x_offset, y_offset) = if format <= 3
    {
        (
            u32::from(packet.u8()?),
            packet.u24()?,
            i32::from(packet.u8()?),
            u32::from(packet.u8()?),
            u32::from(packet.u8()?),
            i32::from(packet.i8()?),
            i32::from(packet.i8()?),
        )
    } else if format <= 6 {
        (
            u32::from(packet.u8()?),
            packet.u24()?,
            i32::from(packet.u16()?),
            u32::from(packet.u16()?),
            u32::from(packet.u16()?),
            i32::from(packet.i16()?),
            i32::from(packet.i16()?),
        )
    } else {
        (
            packet.u32()?,
            packet.u32()?,
            packet.i32()?,
            {
                packet.i32()?;
                packet.u32()?
            },
            packet.u32()?,
            packet.i32()?,
            packet.i32()?,
        )
    };
    if width > MAX_GLYPH_DIMENSION || height > MAX_GLYPH_DIMENSION {
        return Err(PdfPkFontError::InvalidDimensions);
    }
    let row_bytes =
        usize::try_from(width.div_ceil(8)).map_err(|_| PdfPkFontError::InvalidDimensions)?;
    let bitmap_len = row_bytes
        .checked_mul(usize::try_from(height).map_err(|_| PdfPkFontError::InvalidDimensions)?)
        .filter(|length| *length <= MAX_DECODED_BITMAP_BYTES)
        .ok_or(PdfPkFontError::DecodedBitmapLimit)?;
    let bitmap = if dyn_f == 14 {
        decode_raw(packet.remaining(), width, height, bitmap_len)?
    } else {
        decode_packed(
            packet.remaining(),
            dyn_f,
            first_black,
            width,
            height,
            bitmap_len,
        )?
    };
    Ok(PdfPkGlyph {
        code,
        tfm_width,
        horizontal_escapement,
        width,
        height,
        x_offset,
        y_offset,
        bitmap,
    })
}

fn decode_raw(
    raster: &[u8],
    width: u32,
    height: u32,
    bitmap_len: usize,
) -> Result<Vec<u8>, PdfPkFontError> {
    let bit_count = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .ok_or(PdfPkFontError::DecodedBitmapLimit)?;
    if raster
        .len()
        .checked_mul(8)
        .is_none_or(|bits| bits < bit_count)
    {
        return Err(PdfPkFontError::RasterUnderflow);
    }
    let row_bytes =
        usize::try_from(width.div_ceil(8)).map_err(|_| PdfPkFontError::InvalidDimensions)?;
    let mut bitmap = vec![0; bitmap_len];
    for bit in 0..bit_count {
        if raster[bit / 8] & (0x80 >> (bit % 8)) != 0 {
            let row =
                bit / usize::try_from(width).map_err(|_| PdfPkFontError::InvalidDimensions)?;
            let column =
                bit % usize::try_from(width).map_err(|_| PdfPkFontError::InvalidDimensions)?;
            bitmap[row * row_bytes + column / 8] |= 0x80 >> (column % 8);
        }
    }
    Ok(bitmap)
}

fn decode_packed(
    raster: &[u8],
    dyn_f: u8,
    first_black: bool,
    width: u32,
    height: u32,
    bitmap_len: usize,
) -> Result<Vec<u8>, PdfPkFontError> {
    let width = usize::try_from(width).map_err(|_| PdfPkFontError::InvalidDimensions)?;
    let height = usize::try_from(height).map_err(|_| PdfPkFontError::InvalidDimensions)?;
    let row_bytes = width.div_ceil(8);
    let mut nybbles = Nybbles::new(raster);
    let mut bitmap = vec![0; bitmap_len];
    let mut row = 0usize;
    let mut column = 0usize;
    let mut black = first_black;
    while row < height {
        let count = nybbles.packed_number(dyn_f)?;
        let mut count = usize::try_from(count).map_err(|_| PdfPkFontError::RasterOverflow)?;
        while count != 0 {
            if row == height {
                return Err(PdfPkFontError::RasterOverflow);
            }
            let take = count.min(width - column);
            if black {
                for x in column..column + take {
                    bitmap[row * row_bytes + x / 8] |= 0x80 >> (x % 8);
                }
            }
            column += take;
            count -= take;
            if column == width {
                let repeats = usize::try_from(nybbles.take_repeat_count())
                    .map_err(|_| PdfPkFontError::InvalidRepeatCount)?;
                if row.checked_add(repeats).is_none_or(|last| last >= height) {
                    return Err(PdfPkFontError::InvalidRepeatCount);
                }
                for repeated in 1..=repeats {
                    let source = row * row_bytes;
                    let destination = (row + repeated) * row_bytes;
                    let (before, after) = bitmap.split_at_mut(destination);
                    after[..row_bytes].copy_from_slice(&before[source..source + row_bytes]);
                }
                row += repeats + 1;
                column = 0;
            }
        }
        black = !black;
    }
    if column != 0 {
        return Err(PdfPkFontError::RasterUnderflow);
    }
    Ok(bitmap)
}

struct Nybbles<'a> {
    bytes: &'a [u8],
    index: usize,
    repeat_count: u32,
}

impl<'a> Nybbles<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            index: 0,
            repeat_count: 0,
        }
    }

    fn next(&mut self) -> Result<u8, PdfPkFontError> {
        let byte = *self
            .bytes
            .get(self.index / 2)
            .ok_or(PdfPkFontError::RasterUnderflow)?;
        let value = if self.index.is_multiple_of(2) {
            byte >> 4
        } else {
            byte & 0x0f
        };
        self.index += 1;
        Ok(value)
    }

    fn packed_number(&mut self, dyn_f: u8) -> Result<u32, PdfPkFontError> {
        let first = self.next()?;
        match first {
            0 => {
                let mut zeros = 1u32;
                let mut digit = self.next()?;
                while digit == 0 {
                    zeros = zeros
                        .checked_add(1)
                        .ok_or(PdfPkFontError::InvalidPackedNumber)?;
                    digit = self.next()?;
                }
                let mut value = u32::from(digit);
                for _ in 0..zeros {
                    value = value
                        .checked_mul(16)
                        .and_then(|value| value.checked_add(u32::from(self.next().ok()?)))
                        .ok_or(PdfPkFontError::InvalidPackedNumber)?;
                }
                value
                    .checked_add(u32::from(13 - dyn_f) * 16 + u32::from(dyn_f))
                    .and_then(|value| value.checked_sub(15))
                    .ok_or(PdfPkFontError::InvalidPackedNumber)
            }
            value if value <= dyn_f => Ok(u32::from(value)),
            14 => {
                if self.repeat_count != 0 {
                    return Err(PdfPkFontError::InvalidRepeatCount);
                }
                self.repeat_count = self.packed_number(dyn_f)?;
                self.packed_number(dyn_f)
            }
            15 => {
                if self.repeat_count != 0 {
                    return Err(PdfPkFontError::InvalidRepeatCount);
                }
                self.repeat_count = 1;
                self.packed_number(dyn_f)
            }
            value => Ok((u32::from(value) - u32::from(dyn_f) - 1) * 16
                + u32::from(self.next()?)
                + u32::from(dyn_f)
                + 1),
        }
    }

    fn take_repeat_count(&mut self) -> u32 {
        std::mem::take(&mut self.repeat_count)
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, index: 0 }
    }
    fn is_empty(&self) -> bool {
        self.index == self.bytes.len()
    }
    fn remaining(&self) -> &'a [u8] {
        &self.bytes[self.index..]
    }
    fn take(&mut self, length: usize) -> Result<&'a [u8], PdfPkFontError> {
        let end = self
            .index
            .checked_add(length)
            .ok_or(PdfPkFontError::Truncated)?;
        let bytes = self
            .bytes
            .get(self.index..end)
            .ok_or(PdfPkFontError::Truncated)?;
        self.index = end;
        Ok(bytes)
    }
    fn skip(&mut self, length: usize) -> Result<(), PdfPkFontError> {
        self.take(length).map(|_| ())
    }
    fn u8(&mut self) -> Result<u8, PdfPkFontError> {
        Ok(self.take(1)?[0])
    }
    fn i8(&mut self) -> Result<i8, PdfPkFontError> {
        Ok(self.u8()? as i8)
    }
    fn u16(&mut self) -> Result<u16, PdfPkFontError> {
        Ok(u16::from_be_bytes(
            self.take(2)?.try_into().expect("length checked"),
        ))
    }
    fn i16(&mut self) -> Result<i16, PdfPkFontError> {
        Ok(i16::from_be_bytes(
            self.take(2)?.try_into().expect("length checked"),
        ))
    }
    fn u24(&mut self) -> Result<u32, PdfPkFontError> {
        let b = self.take(3)?;
        Ok(u32::from_be_bytes([0, b[0], b[1], b[2]]))
    }
    fn u32(&mut self) -> Result<u32, PdfPkFontError> {
        Ok(u32::from_be_bytes(
            self.take(4)?.try_into().expect("length checked"),
        ))
    }
    fn i32(&mut self) -> Result<i32, PdfPkFontError> {
        Ok(i32::from_be_bytes(
            self.take(4)?.try_into().expect("length checked"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_raw_bitmap_packet_into_row_aligned_mask() {
        let mut bytes = vec![PK_PRE, PK_ID, 0];
        bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        bytes.extend_from_slice(&[0xe0, 9, 65, 0, 0, 0, 3, 3, 2, 0, 1, 0b101_01000]);
        bytes.push(PK_POST);
        let font = PdfPkFont::parse(&bytes).expect("synthetic PK parses");
        let glyph = font.glyph(65).expect("glyph present");
        assert_eq!(glyph.bitmap, [0b1010_0000, 0b0100_0000]);
    }

    #[test]
    fn rejects_truncated_and_oversized_rasters() {
        assert_eq!(
            PdfPkFont::parse(&[PK_PRE, PK_ID]),
            Err(PdfPkFontError::Truncated)
        );
        assert_eq!(
            decode_raw(&[], 1, 1, 1),
            Err(PdfPkFontError::RasterUnderflow)
        );
    }

    #[test]
    fn decodes_the_committed_real_pk_font() {
        let font_300 = PdfPkFont::parse(include_bytes!("../../../tests/corpus/pdf/cmr10.300pk"))
            .expect("committed 300 DPI PK font parses");
        let glyph = font_300.glyph(65).expect("300 DPI A glyph");
        assert_eq!((glyph.width, glyph.height), (28, 29));
        assert_eq!((glyph.x_offset, glyph.y_offset), (-1, 28));
        let font_600 = PdfPkFont::parse(include_bytes!("../../../tests/corpus/pdf/cmr10.600pk"))
            .expect("committed 600 DPI PK font parses");
        let glyph = font_600.glyph(65).expect("600 DPI A glyph");
        assert_eq!((glyph.width, glyph.height), (55, 60));
        assert_eq!((glyph.x_offset, glyph.y_offset), (-3, 59));
    }
}
