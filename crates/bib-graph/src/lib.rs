//! Cross-entry graph and validation stage boundary.

use bib_model::BibConfiguration;
use bib_unicode::UnicodeData;

#[derive(Clone, Copy, Debug)]
pub struct GraphContext<'a> {
    configuration: &'a BibConfiguration,
    unicode: &'a UnicodeData,
}

impl<'a> GraphContext<'a> {
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
