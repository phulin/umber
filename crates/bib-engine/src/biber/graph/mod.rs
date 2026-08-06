//! Indexed relationship, inheritance, sourcemap, and validation pass.
#![allow(dead_code, unused_imports)]

mod maps;
mod processor;
mod validation;

pub(super) use maps::{MapAction, MapMatch, SourceMap, SourceMapStep};
pub(crate) use processor::{
    DraftSection, GraphError, GraphLimits, GraphOptions, RelationshipPass, SectionSpec,
};
pub(super) use validation::{DataConstraint, DataModel, ValidationRule};

#[cfg(test)]
mod tests;
