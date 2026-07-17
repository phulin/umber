//! Control, configuration, and datasource input-stage boundary.

use bib_model::BibConfiguration;
use bib_unicode::UnicodeData;
use umber_vfs::VfsSnapshot;

/// All ambient state available to an input parser.
#[derive(Clone, Copy, Debug)]
pub struct InputContext<'a> {
    snapshot: &'a VfsSnapshot,
    configuration: &'a BibConfiguration,
    unicode: &'a UnicodeData,
}

impl<'a> InputContext<'a> {
    #[must_use]
    pub const fn new(
        snapshot: &'a VfsSnapshot,
        configuration: &'a BibConfiguration,
        unicode: &'a UnicodeData,
    ) -> Self {
        Self {
            snapshot,
            configuration,
            unicode,
        }
    }

    #[must_use]
    pub const fn snapshot(self) -> &'a VfsSnapshot {
        self.snapshot
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
