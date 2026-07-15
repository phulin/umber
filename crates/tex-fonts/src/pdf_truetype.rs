//! Validated, PDF-ready TrueType font programs.

use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfTrueTypeProgramIdentity([u8; 32]);

impl PdfTrueTypeProgramIdentity {
    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Immutable SFNT bytes and descriptor metrics normalized to 1000 units/em.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfTrueTypeProgram {
    identity: PdfTrueTypeProgramIdentity,
    bytes: Vec<u8>,
    bbox: [i32; 4],
    ascent: i32,
    descent: i32,
    cap_height: i32,
    x_height: i32,
    italic_angle: i32,
    stem_v: i32,
    fixed_pitch: bool,
}

impl PdfTrueTypeProgram {
    pub fn parse(bytes: &[u8]) -> Result<Self, PdfTrueTypeProgramError> {
        let face =
            ttf_parser::Face::parse(bytes, 0).map_err(|_| PdfTrueTypeProgramError::InvalidSfnt)?;
        let em = i64::from(face.units_per_em());
        let scale = |value: i16| -> i32 {
            ((i64::from(value) * 1000 + if value >= 0 { em / 2 } else { -em / 2 }) / em) as i32
        };
        let bbox = face.global_bounding_box();
        let weight = i32::from(face.weight().to_number());
        Ok(Self {
            identity: PdfTrueTypeProgramIdentity(Sha256::digest(bytes).into()),
            bytes: bytes.to_vec(),
            bbox: [
                scale(bbox.x_min),
                scale(bbox.y_min),
                scale(bbox.x_max),
                scale(bbox.y_max),
            ],
            ascent: scale(face.ascender()),
            descent: scale(face.descender()),
            cap_height: face
                .capital_height()
                .map(scale)
                .unwrap_or_else(|| scale(face.ascender())),
            x_height: face.x_height().map(scale).unwrap_or(0),
            italic_angle: face.italic_angle().round() as i32,
            stem_v: 50 + weight.saturating_mul(3) / 40,
            fixed_pitch: face.is_monospaced(),
        })
    }

    pub fn from_woff2(bytes: &[u8]) -> Result<Self, PdfTrueTypeProgramError> {
        let mut source = bytes;
        let decoded = woff2_patched::convert_woff2_to_ttf(&mut source)
            .map_err(|_| PdfTrueTypeProgramError::InvalidWoff2)?;
        Self::parse(&decoded)
    }

    #[must_use]
    pub const fn identity(&self) -> PdfTrueTypeProgramIdentity {
        self.identity
    }
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    #[must_use]
    pub const fn bbox(&self) -> [i32; 4] {
        self.bbox
    }
    #[must_use]
    pub const fn ascent(&self) -> i32 {
        self.ascent
    }
    #[must_use]
    pub const fn descent(&self) -> i32 {
        self.descent
    }
    #[must_use]
    pub const fn cap_height(&self) -> i32 {
        self.cap_height
    }
    #[must_use]
    pub const fn x_height(&self) -> i32 {
        self.x_height
    }
    #[must_use]
    pub const fn italic_angle(&self) -> i32 {
        self.italic_angle
    }
    #[must_use]
    pub const fn stem_v(&self) -> i32 {
        self.stem_v
    }
    #[must_use]
    pub const fn fixed_pitch(&self) -> bool {
        self.fixed_pitch
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdfTrueTypeProgramError {
    InvalidSfnt,
    InvalidWoff2,
}

impl std::fmt::Display for PdfTrueTypeProgramError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSfnt => f.write_str("invalid TrueType SFNT font program"),
            Self::InvalidWoff2 => f.write_str("invalid WOFF2 font program"),
        }
    }
}

impl std::error::Error for PdfTrueTypeProgramError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_sfnt_bytes() {
        assert_eq!(
            PdfTrueTypeProgram::parse(b"not a font"),
            Err(PdfTrueTypeProgramError::InvalidSfnt)
        );
    }

    #[test]
    fn decodes_committed_woff2_to_pdf_ready_sfnt() {
        let bytes = include_bytes!("../../umber-wasm/assets/cmu-serif-500-roman.woff2");
        let program = PdfTrueTypeProgram::from_woff2(bytes).expect("committed WOFF2");
        assert!(program.bytes().starts_with(&[0, 1, 0, 0]));
        assert!(program.ascent() > 0);
        assert!(program.bbox()[2] > program.bbox()[0]);
    }
}
