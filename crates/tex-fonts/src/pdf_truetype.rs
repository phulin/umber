//! Validated, PDF-ready TrueType font programs.

use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

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
    postscript_name: Option<Vec<u8>>,
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
        let postscript_name = face
            .names()
            .into_iter()
            .find(|name| name.name_id == ttf_parser::name_id::POST_SCRIPT_NAME)
            .and_then(|name| {
                name.to_string()
                    .map(String::into_bytes)
                    .or_else(|| name.name.is_ascii().then(|| name.name.to_vec()))
            });
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
            postscript_name,
        })
    }

    pub fn from_woff2(bytes: &[u8]) -> Result<Self, PdfTrueTypeProgramError> {
        let mut source = bytes;
        let decoded = woff2_patched::convert_woff2_to_ttf(&mut source)
            .map_err(|_| PdfTrueTypeProgramError::InvalidWoff2)?;
        Self::parse(&decoded)
    }

    /// Builds a compact PDF-oriented SFNT containing the requested named
    /// glyphs and the composite-glyph closure computed by `subsetter`.
    pub fn subset(&self, glyph_names: &BTreeSet<Vec<u8>>) -> Result<Self, PdfTrueTypeSubsetError> {
        let face = ttf_parser::Face::parse(&self.bytes, 0)
            .map_err(|_| PdfTrueTypeSubsetError::InvalidSfnt)?;
        let mut remapper = subsetter::GlyphRemapper::new();
        for name in glyph_names {
            let glyph = (0..face.number_of_glyphs())
                .map(ttf_parser::GlyphId)
                .find(|glyph| {
                    face.glyph_name(*glyph)
                        .is_some_and(|value| value.as_bytes() == name)
                })
                .ok_or_else(|| PdfTrueTypeSubsetError::MissingGlyphName(name.clone()))?;
            remapper.remap(glyph.0);
        }
        let bytes = subsetter::subset(&self.bytes, 0, &remapper)
            .map_err(|_| PdfTrueTypeSubsetError::SubsetFailed)?;
        Self::parse(&bytes).map_err(|_| PdfTrueTypeSubsetError::InvalidSubset)
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
    #[must_use]
    pub fn postscript_name(&self) -> Option<&[u8]> {
        self.postscript_name.as_deref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfTrueTypeSubsetError {
    InvalidSfnt,
    MissingGlyphName(Vec<u8>),
    SubsetFailed,
    InvalidSubset,
}

impl std::fmt::Display for PdfTrueTypeSubsetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingGlyphName(name) => write!(
                f,
                "TrueType font has no glyph named {:?}",
                String::from_utf8_lossy(name)
            ),
            other => write!(f, "cannot subset TrueType font program: {other:?}"),
        }
    }
}

impl std::error::Error for PdfTrueTypeSubsetError {}

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
        assert_eq!(
            program.postscript_name(),
            Some(b"CMUSerif-Roman".as_slice())
        );
    }

    #[test]
    fn subsets_committed_truetype_to_named_glyph_closure() {
        let bytes = include_bytes!("../../umber-wasm/assets/cmu-serif-500-roman.woff2");
        let program = PdfTrueTypeProgram::from_woff2(bytes).expect("committed WOFF2");
        let names = [b"A".to_vec(), b"B".to_vec(), b"C".to_vec()]
            .into_iter()
            .collect();
        let subset = program.subset(&names).expect("named subset");
        assert!(subset.bytes().len() < program.bytes().len() / 4);
        let face = ttf_parser::Face::parse(subset.bytes(), 0).expect("subset SFNT parses");
        assert!(
            (0..face.number_of_glyphs())
                .map(ttf_parser::GlyphId)
                .any(|glyph| face.glyph_name(glyph) == Some("A"))
        );
        assert!(
            !(0..face.number_of_glyphs())
                .map(ttf_parser::GlyphId)
                .any(|glyph| face.glyph_name(glyph) == Some("D"))
        );
    }
}
