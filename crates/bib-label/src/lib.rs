//! Label, hash, visibility, and uniqueness stage boundary.

use bib_model::BibConfiguration;
use bib_unicode::UnicodeData;

#[derive(Clone, Copy, Debug)]
pub struct LabelContext<'a> {
    configuration: &'a BibConfiguration,
    unicode: &'a UnicodeData,
}

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
