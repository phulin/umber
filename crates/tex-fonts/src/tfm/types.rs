use std::path::PathBuf;

use tex_arith::{FontSizeSpec, Scaled};

use super::ParseError;
use crate::{FontContentHash, FontMetrics, LoadedFont};

/// A parsed TFM projected into the canonical runtime metric representation.
///
/// Raw table indices and lig/kern encodings exist only while the parser is
/// validating source references. A successful parse retains exactly the
/// metadata needed to construct a loaded font plus `FontMetrics`, the same
/// immutable record consumed by execution, typesetting, formats, and VF
/// lowering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TfmFont {
    pub header: Header,
    pub font_size: Scaled,
    pub parameters: FontParameters,
    metrics: FontMetrics,
    font_info_words: usize,
}

impl TfmFont {
    /// Parses a TFM byte slice using the design size stored in the file.
    pub fn parse(bytes: &[u8]) -> Result<Self, ParseError> {
        Self::parse_with_size(bytes, FontSizeSpec::Design)
    }

    /// Parses a TFM byte slice and scales metric dimensions for a TeX font size specification.
    pub fn parse_with_size(bytes: &[u8], size_spec: FontSizeSpec) -> Result<Self, ParseError> {
        super::parse::parse_tfm(bytes, size_spec)
    }

    /// Returns the canonical immutable runtime metrics produced by the parser.
    #[must_use]
    pub const fn metrics(&self) -> &FontMetrics {
        &self.metrics
    }

    /// Returns TeX82 §560's words copied into `font_info` by §565.
    #[must_use]
    pub const fn font_info_words(&self) -> usize {
        self.font_info_words
    }

    /// Constructs the one runtime font record for this parsed TFM.
    #[must_use]
    pub fn into_loaded_font(
        self,
        name: impl Into<String>,
        path: impl Into<PathBuf>,
        content_hash: FontContentHash,
    ) -> LoadedFont {
        let parameters = self
            .parameters
            .values
            .iter()
            .map(|parameter| parameter.value)
            .collect();
        LoadedFont::new(
            name,
            path,
            content_hash,
            self.header.checksum,
            self.header.design_size,
            self.font_size,
            parameters,
            self.metrics,
        )
        .with_font_info_words(self.font_info_words)
    }

    pub(super) fn from_parsed(
        header: Header,
        font_size: Scaled,
        parameters: FontParameters,
        metrics: FontMetrics,
        font_info_words: usize,
    ) -> Self {
        Self {
            header,
            font_size,
            parameters,
            metrics,
            font_info_words,
        }
    }
}

/// Header metadata stored before the metric tables.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Header {
    pub checksum: u32,
    pub design_size: Scaled,
    pub coding_scheme: Option<String>,
    pub family: Option<String>,
    pub seven_bit_safe: Option<bool>,
    pub face: Option<u8>,
    pub additional_words: Vec<[u8; 4]>,
}

/// Parsed `fontdimen` parameters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontParameters {
    pub values: Vec<FontParameter>,
}

impl FontParameters {
    #[must_use]
    pub fn get(&self, number: u16) -> Option<&FontParameter> {
        if number == 0 {
            return None;
        }
        self.values.get(usize::from(number - 1))
    }

    #[must_use]
    pub fn slant(&self) -> Option<Scaled> {
        self.get(1).map(|param| param.value)
    }

    #[must_use]
    pub fn math_parameters(&self) -> &[FontParameter] {
        if self.values.len() <= 7 {
            &[]
        } else {
            &self.values[7..]
        }
    }
}

/// One `fontdimen` value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FontParameter {
    pub number: u16,
    pub value: Scaled,
    pub kind: FontParameterKind,
}

/// Scaling rule used for a font parameter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FontParameterKind {
    /// `fontdimen1` is a dimensionless fix_word ratio; `Scaled::UNITY` represents 1.0.
    SlantRatio,
    /// All other parameters are font-size-scaled dimensions.
    Dimension,
}

/// Metric table names used in parse errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TfmTable {
    Width,
    Height,
    Depth,
    Italic,
    LigKern,
    Kern,
    Extensible,
    Param,
}
