//! Font metric parsing and immutable font data.

pub mod metrics;
pub mod tfm;

pub use metrics::{
    CharMetrics, CharTag as MetricCharTag, ExtensibleRecipe as MetricExtensibleRecipe,
    FontContentHash, FontMetrics, FontMetricsValidationError, LigKernChar, LigKernCommand,
    LigKernInstruction, LigKernIter, LigKernStep as MetricLigKernStep, LigatureCommand, LoadedFont,
    MAX_LIG_KERN_PROGRAM_LEN,
};
pub use tfm::{
    CharacterTag, ExtensibleRecipe, FontParameter, FontParameterKind, FontParameters, Header,
    LigKernAction, LigKernStep, Ligature, LigatureDeletes, ParseError, TfmFont, TfmTable,
};

#[cfg(test)]
mod tests;
