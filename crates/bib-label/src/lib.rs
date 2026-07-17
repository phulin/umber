//! Label, hash, visibility, and uniqueness stage boundary.

use bib_model::BibConfiguration;
use bib_unicode::UnicodeData;

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

#[derive(Clone, Copy, Debug)]
pub struct LabelContext<'a> {
    configuration: &'a BibConfiguration,
    unicode: &'a UnicodeData,
}

#[cfg(test)]
mod tests;

impl<'a> LabelContext<'a> {
    #[must_use]
    pub const fn new(configuration: &'a BibConfiguration, unicode: &'a UnicodeData) -> Self {
        Self {
            configuration,
            unicode,
        }
    }
    #[must_use]
    pub const fn configuration(self) -> &'a BibConfiguration {
        self.configuration
    }
    #[must_use]
    pub const fn unicode(self) -> &'a UnicodeData {
        self.unicode
    }
}
