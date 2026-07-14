//! Validated OpenType resource contracts and immutable font programs.

mod contract;
mod parse;

pub use contract::{
    AcceptedFontContainers, FeatureSetting, FontContainer, FontFeaturePolicy, FontInstanceIdentity,
    FontLimits, FontObjectIdentity, FontProgramIdentity, FontPurposes, FontRequest, FontRequestKey,
    FontSelectionError, FontWireError, OpenTypeTag, ResolvedFont, VariationCoordinate,
    VariationSelection, WritingDirection,
};
pub use parse::{
    CharacterMap, FontMetadata, FontParseError, OpenTypeFont, OpenTypeMetrics, ShapingTables,
};

#[cfg(test)]
mod tests;
