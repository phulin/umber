//! Validated OpenType resource contracts and immutable font programs.

mod contract;
mod math;
mod parse;

pub use contract::{
    AcceptedFontContainers, FeatureSetting, FontContainer, FontFeaturePolicy, FontInstanceIdentity,
    FontLimits, FontObjectIdentity, FontProgramIdentity, FontPurposes, FontRequest, FontRequestKey,
    FontSelectionError, FontWireError, OpenTypeTag, ResolvedFont, VariationCoordinate,
    VariationSelection, WritingDirection,
};
pub use math::{
    MathAdjustment, MathConstant, MathConstants, MathGlyphAssembly, MathGlyphConstruction,
    MathGlyphInfo, MathGlyphPart, MathGlyphVariant, MathKern, MathKernInfo, MathTables, MathValue,
    MathVariants,
};
pub use parse::{
    CharacterMap, FontMetadata, FontParseError, OpenTypeFont, OpenTypeMetrics, ShapingTables,
};

#[cfg(test)]
mod tests;
