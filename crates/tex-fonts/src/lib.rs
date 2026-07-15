//! Font metric parsing and immutable font data.

pub mod metrics;
pub mod opentype;
pub mod pdf_encoding;
pub mod pdf_map;
pub mod pdf_truetype;
pub mod tfm;
pub mod type1;

pub use metrics::{
    CharMetrics, CharTag as MetricCharTag, ExtensibleRecipe as MetricExtensibleRecipe,
    FontConstruction, FontConstructionError, FontContentHash, FontMetrics,
    FontMetricsValidationError, FontSourceIdentity, LigKernChar, LigKernCommand,
    LigKernInstruction, LigKernIter, LigKernStep as MetricLigKernStep, LigatureCommand, LoadedFont,
    MAX_LIG_KERN_PROGRAM_LEN, OpenTypeFontSelection, OpenTypeProgramSelection,
};
pub use opentype::{
    AcceptedFontContainers, CharacterMap, FeatureSetting, FontContainer, FontFeaturePolicy,
    FontInstanceIdentity, FontLimits, FontMetadata, FontObjectIdentity, FontParseError,
    FontProgramIdentity, FontPurposes, FontRequest, FontRequestKey, FontSelectionError,
    FontWireError, OpenTypeFont, OpenTypeMetrics, OpenTypeTag, ResolvedFont, ShapingTables,
    VariationCoordinate, VariationSelection, WritingDirection,
};
pub use pdf_encoding::{PdfEncoding, PdfEncodingError};
pub use pdf_map::{
    PdfFontMapDirective, PdfFontMapEntry, PdfFontMapError, PdfFontMapFile, PdfFontMapProgram,
};
pub use pdf_truetype::{PdfTrueTypeProgram, PdfTrueTypeProgramError, PdfTrueTypeProgramIdentity};
pub use tfm::{
    CharacterTag, ExtensibleRecipe, FontParameter, FontParameterKind, FontParameters, Header,
    LigKernAction, LigKernStep, Ligature, LigatureDeletes, ParseError, TfmFont, TfmTable,
};
pub use type1::{
    PdfType1Program, PdfType1ProgramError, PdfType1ProgramIdentity, PdfType1SubsetError,
    pdftex_subset_tag,
};

#[cfg(test)]
mod tests;
