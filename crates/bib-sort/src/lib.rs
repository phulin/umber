//! Data-list construction and stable sorting stage boundary.

use bib_model::BibConfiguration;
use bib_unicode::UnicodeData;

mod name_lists;
mod sorting;

pub use name_lists::{
    NameListLimitError, NameListLimits, NameListVisibility, NameVisibility, NameVisibilityOptions,
};
pub use sorting::{
    CaseOrder, DataListBuilder, DataListFilter, DataListLimits, EntryDisposition, Locale,
    MissingOrder, NameKeyPart, NameKeyTemplate, PadDirection, SortComponent, SortDirection,
    SortError, SortField, SortOptions, SortTemplate, SortedEntry, limit_literal_list, list_initial,
    list_initial_hash, name_sort_key,
};

#[derive(Clone, Copy, Debug)]
pub struct SortContext<'a> {
    configuration: &'a BibConfiguration,
    unicode: &'a UnicodeData,
}

impl<'a> SortContext<'a> {
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
