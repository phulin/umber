//! Font metric parsing and immutable font data.

pub mod metrics;
pub mod opentype;
pub mod pdf_encoding;
pub mod pdf_map;
pub mod pdf_pk;
pub mod pdf_truetype;
pub mod pdf_vf;
pub mod tfm;
pub mod type1;

pub use metrics::{
    CharMetrics, CharTag as MetricCharTag, ExtensibleRecipe as MetricExtensibleRecipe,
    FONT_LAYOUT_POLICY_VERSION, FontConstruction, FontConstructionError, FontContentHash,
    FontLayoutPolicy, FontMappingFallbackPolicy, FontMetrics, FontMetricsSource,
    FontMetricsValidationError, FontSourceIdentity, LEGACY_ENCODING_MAP_VERSION, LegacyEncodingMap,
    LigKernChar, LigKernCommand, LigKernInstruction, LigKernIter, LigKernStep as MetricLigKernStep,
    LigatureCommand, LoadedFont, MAX_LIG_KERN_PROGRAM_LEN, MathKernCorner, MathMetricsSource,
    MathVariantDirection, OPENTYPE_FONTDIMEN_SYNTHESIS_VERSION, OpenTypeFontSelection,
    OpenTypeFontShaped, OpenTypeMathAssembly, OpenTypeMathAssemblyPart, OpenTypeMathConstruction,
    OpenTypeMathGlyph, OpenTypeMathMetrics, OpenTypeMathVariant, OpenTypeProgramSelection,
    ShapingFont,
};
pub use opentype::{
    AcceptedFontContainers, CharacterMap, FONT_FEATURE_POLICY_VERSION, FeatureSetting,
    FontContainer, FontFeaturePolicy, FontInstanceContext, FontInstanceIdentity, FontLanguage,
    FontLimits, FontMetadata, FontObjectIdentity, FontParseError, FontProgramIdentity,
    FontPurposes, FontRequest, FontRequestKey, FontSelectionError, FontWireError,
    LegacyFontMapping, MathAdjustment, MathConstant, MathConstants, MathGlyphAssembly,
    MathGlyphConstruction, MathGlyphInfo, MathGlyphPart, MathGlyphVariant, MathKern, MathKernInfo,
    MathTables, MathValue, MathVariants, NamedVariationInstance, OpenTypeFont, OpenTypeMetrics,
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
