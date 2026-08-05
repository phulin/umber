//! Label, hash, visibility, and uniqueness stage boundary.
#![allow(dead_code, unused_imports)]

mod extras;
mod hashes;
mod labels;
mod uniqueness;

pub use extras::{ExtraField, ExtraFieldProcessor, ExtraScope, ExtraValues};
pub use hashes::{NameHashes, hash_name, hash_name_list};
pub use labels::{
    AlphaNameOptions, LabelAlphaComponent, LabelAlphaTemplate, LabelEntry, LabelSelection,
    select_labels,
};
pub use uniqueness::{
    NameDisambiguation, UniqueState, UniquenessEntry, UniquenessOptions, UniquenessProcessor,
    VisibleNameContext,
};

#[cfg(test)]
mod tests;
