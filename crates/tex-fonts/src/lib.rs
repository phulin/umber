//! Font metric parsing and immutable font data.

pub mod metrics;
pub mod opentype;
pub mod pdf_encoding;
pub mod pdf_map;
pub mod pdf_pk;
pub mod pdf_truetype;
pub mod pdf_vf;
mod shaping;
pub mod tfm;
pub mod type1;

pub use metrics::{
    CharMetrics, CharTag as MetricCharTag, ExtensibleRecipe as MetricExtensibleRecipe,
    FONT_LAYOUT_POLICY_VERSION, FontConstruction, FontConstructionError, FontContentHash,
    FontLayoutPolicy, FontMappingFallbackPolicy, FontMetrics, FontMetricsSource,
    FontMetricsValidationError, FontSourceIdentity, LEGACY_ENCODING_MAP_VERSION, LegacyEncodingMap,
    LigKernChar, LigKernCommand, LigKernInstruction, LigKernIter, LigKernStep as MetricLigKernStep,
    LigatureCommand, LoadedFont, MAX_LIG_KERN_PROGRAM_LEN, MathKernCorner, MathMetricsSource,
    MathVariantDirection, OPENTYPE_FONTDIMEN_SYNTHESIS_VERSION, OpenTypeFontShaped,
    OpenTypeMathAssembly, OpenTypeMathAssemblyPart, OpenTypeMathConstruction, OpenTypeMathGlyph,
    OpenTypeMathMetrics, OpenTypeMathVariant, PdfFontResourceIdentity, RealizedFontIdentity,
    font_content_hash,
};
pub use opentype::{
    AcceptedFontContainers, CharacterMap, FONT_FEATURE_POLICY_VERSION, FeatureSetting,
    FontContainer, FontFeaturePolicy, FontInstanceContext, FontInstanceIdentity, FontLanguage,
    FontLimits, FontMetadata, FontObjectIdentity, FontParseError, FontProgramIdentity,
    FontPurposes, FontRequest, FontRequestKey, FontSelectionError, FontWireError,
    LegacyFontMapping, MathConstant, NamedVariationInstance, OpenTypeFont, OpenTypeMetrics,
    OpenTypeTag, ResolvedFont, ShapingTables, VariationAxis, VariationCoordinate,
    VariationInstance, VariationModel, VariationSelection, WritingDirection,
};
pub use pdf_encoding::{PdfEncoding, PdfEncodingError};
pub use pdf_map::{
    PdfFontMap, PdfFontMapDirective, PdfFontMapEntry, PdfFontMapError, PdfFontMapFile,
    PdfFontMapProgram,
};
pub use pdf_pk::{PdfPkFont, PdfPkFontError, PdfPkFontIdentity, PdfPkFontRequest, PdfPkGlyph};
pub use pdf_truetype::{
    PdfTrueTypeProgram, PdfTrueTypeProgramError, PdfTrueTypeProgramIdentity, PdfTrueTypeSubsetError,
};
pub use pdf_vf::{
    PDFTEX_VF_MAX_RECURSION, VfCharacterReference, VfCommand, VfLimits, VfLocalFont, VfPacket,
    VfPacketMetadata, VfParseError, VfProgram, VfProgramIdentity,
};
pub use shaping::{
    Script, ShapedGlyph, ShapedRun, ShapingMetadata, ShapingRequest, ShapingScratch,
    character_script, run_script, text_direction,
};
pub use tfm::{
    FontParameter, FontParameterKind, FontParameters, Header, ParseError, TfmFont, TfmTable,
};
pub use type1::{
    PdfType1Program, PdfType1ProgramError, PdfType1ProgramIdentity, PdfType1SubsetError,
    pdftex_subset_tag,
};

#[cfg(test)]
mod tests;
