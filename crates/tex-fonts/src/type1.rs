//! Detached Type-1 PFB decoding for PDF embedding.

use sha2::{Digest, Sha256};

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
}
