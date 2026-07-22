//! Validated OpenType resource contracts and immutable font programs.

mod contract;
mod math;
mod parse;
mod variation;

pub use contract::{
    AcceptedFontContainers, FONT_FEATURE_POLICY_VERSION, FeatureSetting, FontContainer,
    FontFeaturePolicy, FontInstanceContext, FontInstanceIdentity, FontLanguage, FontLimits,
    FontObjectIdentity, FontProgramIdentity, FontPurposes, FontRequest, FontRequestKey,
    FontSelectionError, FontWireError, OpenTypeTag, ResolvedFont, VariationCoordinate,
    VariationInstance, VariationSelection, WritingDirection,
};
pub use math::{
    MathAdjustment, MathConstant, MathConstants, MathGlyphAssembly, MathGlyphConstruction,
    MathGlyphInfo, MathGlyphPart, MathGlyphVariant, MathKern, MathKernInfo, MathTables, MathValue,
    MathVariants,
};
pub use parse::{
    CharacterMap, FontMetadata, FontParseError, OpenTypeFont, OpenTypeMetrics, ShapingTables,
};
pub use variation::{NamedVariationInstance, VariationAxis, VariationModel};

#[cfg(test)]
mod tests;
