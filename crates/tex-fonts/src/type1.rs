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
}
