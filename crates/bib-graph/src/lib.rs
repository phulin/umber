//! Deterministic cross-entry graph, transformation, and validation stage.

mod maps;
mod processor;
mod validation;

pub use maps::{MapAction, MapMatch, SourceMap, SourceMapStep};
pub use processor::{
    GraphContext, GraphError, GraphInput, GraphLimits, GraphOptions, GraphOutput, GraphProcessor,
    GraphSection, SectionSpec,
};
pub use validation::{DataConstraint, DataModel, ValidationRule};

#[cfg(test)]
mod tests;
