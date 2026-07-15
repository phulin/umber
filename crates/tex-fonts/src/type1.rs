//! Detached Type-1 PFB decoding for PDF embedding.

use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PdfType1ProgramIdentity([u8; 32]);

impl PdfType1ProgramIdentity {
    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

/// PDF-ready Type-1 bytes with the PFB segment framing removed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfType1Program {
    identity: PdfType1ProgramIdentity,
    bytes: Vec<u8>,
    length1: u32,
    length2: u32,
    length3: u32,
}

impl PdfType1Program {
    pub fn from_pfb(bytes: &[u8]) -> Result<Self, PdfType1ProgramError> {
        let mut cursor = 0usize;
        let mut decoded = Vec::new();
        let mut lengths = [0u32; 3];
        let mut segment = 0usize;
        while cursor < bytes.len() {
            if bytes.get(cursor..cursor + 2) == Some(&[0x80, 0x03]) {
                cursor += 2;
                break;
            }
            if bytes.get(cursor) != Some(&0x80) {
                return Err(PdfType1ProgramError::BadSegmentMarker);
            }
            let kind = *bytes
                .get(cursor + 1)
                .ok_or(PdfType1ProgramError::TruncatedSegmentHeader)?;
            if segment >= 3 || kind != if segment == 1 { 2 } else { 1 } {
                return Err(PdfType1ProgramError::UnexpectedSegmentKind(kind));
            }
            let length_bytes: [u8; 4] = bytes
                .get(cursor + 2..cursor + 6)
                .ok_or(PdfType1ProgramError::TruncatedSegmentHeader)?
                .try_into()
                .expect("four-byte slice");
            let length = u32::from_le_bytes(length_bytes);
            let end = (cursor + 6)
                .checked_add(length as usize)
                .ok_or(PdfType1ProgramError::SegmentTooLarge)?;
            let data = bytes
                .get(cursor + 6..end)
                .ok_or(PdfType1ProgramError::TruncatedSegmentData)?;
            decoded.extend_from_slice(data);
            lengths[segment] = length;
            segment += 1;
            cursor = end;
        }
        if segment < 2 || cursor != bytes.len() {
            return Err(PdfType1ProgramError::MissingEndMarker);
        }
        let identity = PdfType1ProgramIdentity(Sha256::digest(&decoded).into());
        Ok(Self {
            identity,
            bytes: decoded,
            length1: lengths[0],
            length2: lengths[1],
            length3: lengths[2],
        })
    }

    /// Builds a deterministic PDF-ready subset containing only the named
    /// CharStrings (plus `.notdef`). Subroutines are retained because their
    /// transitive calls are encoded inside encrypted Type-1 programs; removing
    /// CharStrings still produces a genuine, renderable subset without host
    /// PostScript execution.
    pub fn subset(
        &self,
        glyph_names: &BTreeSet<Vec<u8>>,
        subset_font_name: &[u8],
    ) -> Result<Self, PdfType1SubsetError> {
        let clear_end = usize::try_from(self.length1).map_err(|_| PdfType1SubsetError::Overflow)?;
        let encrypted_end = clear_end
            .checked_add(usize::try_from(self.length2).map_err(|_| PdfType1SubsetError::Overflow)?)
            .ok_or(PdfType1SubsetError::Overflow)?;
        let clear = self
            .bytes
            .get(..clear_end)
            .ok_or(PdfType1SubsetError::InvalidSegments)?;
        let encrypted = self
            .bytes
            .get(clear_end..encrypted_end)
            .ok_or(PdfType1SubsetError::InvalidSegments)?;
        let trailer = self
            .bytes
            .get(encrypted_end..)
            .ok_or(PdfType1SubsetError::InvalidSegments)?;

        let clear = replace_font_name(clear, subset_font_name)?;
        let decrypted = eexec_crypt(encrypted, false);
        let subset_plaintext = subset_charstrings(&decrypted, glyph_names)?;
        let encrypted = eexec_crypt(&subset_plaintext, true);
        let mut bytes = Vec::with_capacity(clear.len() + encrypted.len() + trailer.len());
        bytes.extend_from_slice(&clear);
        bytes.extend_from_slice(&encrypted);
        bytes.extend_from_slice(trailer);
        let length1 = u32::try_from(clear.len()).map_err(|_| PdfType1SubsetError::Overflow)?;
        let length2 = u32::try_from(encrypted.len()).map_err(|_| PdfType1SubsetError::Overflow)?;
        let length3 = u32::try_from(trailer.len()).map_err(|_| PdfType1SubsetError::Overflow)?;
        Ok(Self {
            identity: PdfType1ProgramIdentity(Sha256::digest(&bytes).into()),
            bytes,
            length1,
            length2,
            length3,
        })
    }

    /// Resolves a code through a cleartext built-in Type-1 encoding array.
    #[must_use]
    pub fn builtin_glyph_name(&self, code: u8) -> Option<Vec<u8>> {
        let cleartext = self.bytes.get(..self.length1 as usize)?;
        let needle = format!("dup {code} /").into_bytes();
        let start = cleartext
            .windows(needle.len())
            .position(|window| window == needle)?
            + needle.len();
        let end = cleartext[start..]
            .iter()
            .position(|byte| byte.is_ascii_whitespace())?
            + start;
        (end > start).then(|| cleartext[start..end].to_vec())
    }

    #[must_use]
    pub const fn identity(&self) -> PdfType1ProgramIdentity {
        self.identity
    }
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    #[must_use]
    pub const fn lengths(&self) -> [u32; 3] {
        [self.length1, self.length2, self.length3]
    }

    /// Reads the cleartext `/FontBBox` without interpreting PostScript.
    #[must_use]
    pub fn font_bbox(&self) -> Option<[i32; 4]> {
        let cleartext = self.bytes.get(..self.length1 as usize)?;
        let marker = b"/FontBBox";
        let start = cleartext
            .windows(marker.len())
            .position(|window| window == marker)?
            + marker.len();
        let mut values = [0; 4];
        let mut count = 0usize;
        for token in cleartext[start..]
            .split(|byte| byte.is_ascii_whitespace() || matches!(byte, b'{' | b'}' | b'[' | b']'))
            .filter(|token| !token.is_empty())
        {
            if count == 4 {
                break;
            }
            let text = std::str::from_utf8(token).ok()?;
            match text.parse::<i32>() {
                Ok(value) => {
                    values[count] = value;
                    count += 1;
                }
                Err(_) if count == 0 => continue,
                Err(_) => return None,
            }
        }
        (count == 4).then_some(values)
    }

    /// Reads `/StdVW`, the Type-1 vertical stem width used by PDF descriptors.
    #[must_use]
    pub fn stem_v(&self) -> Option<i32> {
        self.cleartext_integer(b"/StdVW")
    }

    #[must_use]
    pub fn italic_angle(&self) -> Option<i32> {
        self.cleartext_integer(b"/ItalicAngle")
    }

    #[must_use]
    pub fn is_fixed_pitch(&self) -> bool {
        self.cleartext_value(b"/isFixedPitch")
            .is_some_and(|value| value == b"true")
    }

    fn cleartext_integer(&self, marker: &[u8]) -> Option<i32> {
        let value = self.cleartext_value(marker)?;
        std::str::from_utf8(value).ok()?.parse().ok()
    }

    fn cleartext_value(&self, marker: &[u8]) -> Option<&[u8]> {
        let cleartext = self.bytes.get(..self.length1 as usize)?;
        let start = cleartext
            .windows(marker.len())
            .position(|window| window == marker)?
            + marker.len();
        cleartext[start..]
            .split(|byte| byte.is_ascii_whitespace() || matches!(byte, b'[' | b']' | b'{' | b'}'))
            .find(|token| !token.is_empty())
    }
}

/// Reproduces pdfTeX's deterministic six-letter subset tag for a sorted glyph
/// set and PostScript font name. Collision handling is performed by the PDF
/// document assembler, so this function represents round zero.
#[must_use]
pub fn pdftex_subset_tag(glyph_names: &BTreeSet<Vec<u8>>, font_name: &[u8]) -> [u8; 6] {
    let mut digest = md5::Md5::new();
    for glyph in glyph_names {
        digest.update(glyph);
        digest.update(b" ");
    }
    digest.update(font_name);
    digest.update(0i32.to_ne_bytes());
    let digest = digest.finalize();
    let mut rolling = digest[..13]
        .iter()
        .map(|value| i32::from(*value))
        .sum::<i32>();
    let mut tag = [0; 6];
    for index in 0..6 {
        if index > 0 {
            rolling = rolling - i32::from(digest[index - 1]) + i32::from(digest[(index + 12) % 16]);
        }
        tag[index] = (rolling % 26) as u8 + b'A';
    }
    tag
}

fn replace_font_name(cleartext: &[u8], name: &[u8]) -> Result<Vec<u8>, PdfType1SubsetError> {
    let marker = b"/FontName";
    let marker_start = cleartext
        .windows(marker.len())
        .position(|window| window == marker)
        .ok_or(PdfType1SubsetError::MissingFontName)?;
    let slash = cleartext[marker_start + marker.len()..]
        .iter()
        .position(|byte| *byte == b'/')
        .map(|offset| marker_start + marker.len() + offset)
        .ok_or(PdfType1SubsetError::MissingFontName)?;
    let end = cleartext[slash + 1..]
        .iter()
        .position(|byte| byte.is_ascii_whitespace())
        .map(|offset| slash + 1 + offset)
        .ok_or(PdfType1SubsetError::MissingFontName)?;
    let mut replaced = Vec::with_capacity(cleartext.len() + name.len());
    replaced.extend_from_slice(&cleartext[..slash + 1]);
    replaced.extend_from_slice(name);
    replaced.extend_from_slice(&cleartext[end..]);
    Ok(replaced)
}

fn eexec_crypt(bytes: &[u8], encrypt: bool) -> Vec<u8> {
    let mut state = 55_665u16;
    bytes
        .iter()
        .map(|byte| {
            let output = byte ^ (state >> 8) as u8;
            let cipher = if encrypt { output } else { *byte };
            state = (u32::from(cipher) + u32::from(state))
                .wrapping_mul(52_845)
                .wrapping_add(22_719) as u16;
            output
        })
        .collect()
}

fn subset_charstrings(
    plaintext: &[u8],
    glyph_names: &BTreeSet<Vec<u8>>,
) -> Result<Vec<u8>, PdfType1SubsetError> {
    let marker = b"/CharStrings";
    let marker_start = plaintext
        .windows(marker.len())
        .position(|window| window == marker)
        .ok_or(PdfType1SubsetError::MissingCharStrings)?;
    let mut cursor = marker_start + marker.len();
    skip_space(plaintext, &mut cursor);
    let count_start = cursor;
    let _declared = parse_decimal(plaintext, &mut cursor)?;
    let count_end = cursor;
    let begin = plaintext[cursor..]
        .windows(b"begin".len())
        .position(|window| window == b"begin")
        .map(|offset| cursor + offset + b"begin".len())
        .ok_or(PdfType1SubsetError::MissingCharStrings)?;
    cursor = begin;
    let mut entries = Vec::new();
    loop {
        let entry_start = cursor;
        skip_space(plaintext, &mut cursor);
        if plaintext.get(cursor) != Some(&b'/') {
            cursor = entry_start;
            break;
        }
        cursor += 1;
        let name_start = cursor;
        while plaintext
            .get(cursor)
            .is_some_and(|byte| !byte.is_ascii_whitespace())
        {
            cursor += 1;
        }
        let name = plaintext
            .get(name_start..cursor)
            .ok_or(PdfType1SubsetError::MalformedCharStrings)?;
        skip_space(plaintext, &mut cursor);
        let length = parse_decimal(plaintext, &mut cursor)?;
        skip_space(plaintext, &mut cursor);
        while plaintext
            .get(cursor)
            .is_some_and(|byte| !byte.is_ascii_whitespace())
        {
            cursor += 1;
        }
        if !plaintext.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            return Err(PdfType1SubsetError::MalformedCharStrings);
        }
        cursor += 1;
        cursor = cursor
            .checked_add(length)
            .ok_or(PdfType1SubsetError::Overflow)?;
        if cursor > plaintext.len() {
            return Err(PdfType1SubsetError::MalformedCharStrings);
        }
        while plaintext.get(cursor).is_some_and(|byte| *byte != b'\n') {
            cursor += 1;
        }
        if plaintext.get(cursor) == Some(&b'\n') {
            cursor += 1;
        }
        entries.push((entry_start, cursor, name.to_vec()));
    }
    if entries.is_empty() {
        return Err(PdfType1SubsetError::MalformedCharStrings);
    }
    let kept = entries
        .iter()
        .filter(|(_, _, name)| name == b".notdef" || glyph_names.contains(name))
        .collect::<Vec<_>>();
    if kept.len() == 1 && !glyph_names.is_empty() {
        return Err(PdfType1SubsetError::MissingRequestedGlyphs);
    }
    let mut subset = Vec::with_capacity(plaintext.len());
    subset.extend_from_slice(&plaintext[..count_start]);
    subset.extend_from_slice(kept.len().to_string().as_bytes());
    subset.extend_from_slice(&plaintext[count_end..begin]);
    for (start, end, _) in kept {
        subset.extend_from_slice(&plaintext[*start..*end]);
    }
    subset.extend_from_slice(&plaintext[cursor..]);
    Ok(subset)
}

fn skip_space(bytes: &[u8], cursor: &mut usize) {
    while bytes.get(*cursor).is_some_and(u8::is_ascii_whitespace) {
        *cursor += 1;
    }
}

fn parse_decimal(bytes: &[u8], cursor: &mut usize) -> Result<usize, PdfType1SubsetError> {
    let start = *cursor;
    while bytes.get(*cursor).is_some_and(u8::is_ascii_digit) {
        *cursor += 1;
    }
    if *cursor == start {
        return Err(PdfType1SubsetError::MalformedCharStrings);
    }
    std::str::from_utf8(&bytes[start..*cursor])
        .ok()
        .and_then(|text| text.parse().ok())
        .ok_or(PdfType1SubsetError::MalformedCharStrings)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdfType1SubsetError {
    InvalidSegments,
    MissingFontName,
    MissingCharStrings,
    MalformedCharStrings,
    MissingRequestedGlyphs,
    Overflow,
}

impl std::fmt::Display for PdfType1SubsetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cannot subset Type-1 font program: {self:?}")
    }
}

impl std::error::Error for PdfType1SubsetError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdfType1ProgramError {
    BadSegmentMarker,
    TruncatedSegmentHeader,
    UnexpectedSegmentKind(u8),
    SegmentTooLarge,
    TruncatedSegmentData,
    MissingEndMarker,
}

impl std::fmt::Display for PdfType1ProgramError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid Type-1 PFB program: {self:?}")
    }
}
impl std::error::Error for PdfType1ProgramError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_pfb_framing_and_records_pdf_segment_lengths() {
        let parts: &[&[u8]] = &[
            &[0x80, 1, 3, 0, 0, 0],
            b"abc",
            &[0x80, 2, 2, 0, 0, 0],
            b"de",
            &[0x80, 1, 1, 0, 0, 0],
            b"f",
            &[0x80, 3],
        ];
        let pfb = parts.concat();
        let program = PdfType1Program::from_pfb(&pfb).expect("valid synthetic PFB");
        assert_eq!(program.bytes(), b"abcdef");
        assert_eq!(program.lengths(), [3, 2, 1]);
        assert_ne!(program.identity().bytes(), [0; 32]);
    }

    #[test]
    fn reads_cleartext_font_bbox_without_postscript_execution() {
        let header = b"%!PS /FontBBox {-40 -250 1009 750 }readonly def /StdVW [69] def /ItalicAngle 0 def /isFixedPitch true def\n";
        let mut pfb = vec![0x80, 1];
        pfb.extend_from_slice(&(header.len() as u32).to_le_bytes());
        pfb.extend_from_slice(header);
        pfb.extend_from_slice(&[0x80, 2, 1, 0, 0, 0, 0, 0x80, 3]);
        let program = PdfType1Program::from_pfb(&pfb).expect("valid synthetic PFB");
        assert_eq!(program.font_bbox(), Some([-40, -250, 1009, 750]));
        assert_eq!(program.stem_v(), Some(69));
        assert_eq!(program.italic_angle(), Some(0));
        assert!(program.is_fixed_pitch());
    }

    #[test]
    fn subsets_committed_cmr_charstrings_and_matches_pdftex_tag() {
        let pfb = include_bytes!("../../../tests/corpus/pdf/embedded_type1.pfb");
        let program = PdfType1Program::from_pfb(pfb).expect("committed PFB");
        let glyphs = [b"A".to_vec(), b"B".to_vec(), b"C".to_vec()]
            .into_iter()
            .collect::<BTreeSet<_>>();
        let tag = pdftex_subset_tag(&glyphs, b"CMR10");
        assert_eq!(&tag, b"QBBONQ");
        let subset_name = [tag.as_slice(), b"+CMR10"].concat();
        let subset = program
            .subset(&glyphs, &subset_name)
            .expect("committed CMR subsets");
        assert!(subset.bytes().len() < program.bytes().len());
        assert!(
            subset
                .bytes()
                .windows(b"/FontName /QBBONQ+CMR10".len())
                .any(|window| window == b"/FontName /QBBONQ+CMR10")
        );
        let decrypted = eexec_crypt(
            &subset.bytes()[subset.length1 as usize..(subset.length1 + subset.length2) as usize],
            false,
        );
        for glyph in [b"/.notdef ".as_slice(), b"/A ", b"/B ", b"/C "] {
            assert!(
                decrypted.windows(glyph.len()).any(|window| window == glyph),
                "missing {}",
                String::from_utf8_lossy(glyph)
            );
        }
        assert!(!decrypted.windows(3).any(|window| window == b"/D "));
    }
}
