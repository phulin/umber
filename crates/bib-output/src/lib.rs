//! Detached deterministic bibliography serializer boundary.

mod bbl;
mod bibtex;
mod xml;

use bib_model::{GeneratedFile, OutputRequest, ProcessedBibliography};
use bib_unicode::UnicodeData;

pub use bbl::{BblOutputFailure, BblOutputFailureKind, BblSerializer};
pub use bibtex::{
    BibtexCase, BibtexMacro, BibtexOptions, BibtexOutputFailure, BibtexOutputFailureKind,
    BibtexSerializer,
};
pub use xml::{
    BBL_XML_NAMESPACE, BIBLATEX_XML_NAMESPACE, BblXmlSerializer, BibLatexXmlSerializer,
    XmlOutputFailure, XmlOutputFailureKind, XmlSchemaKind, generate_xml_schema,
};

#[derive(Clone, Copy, Debug)]
pub struct OutputContext<'a> {
    document: &'a ProcessedBibliography,
    unicode: &'a UnicodeData,
}

impl<'a> OutputContext<'a> {
    #[must_use]
    pub const fn new(document: &'a ProcessedBibliography, unicode: &'a UnicodeData) -> Self {
        Self { document, unicode }
    }
    #[must_use]
    pub const fn document(self) -> &'a ProcessedBibliography {
        self.document
    }
    #[must_use]
    pub const fn unicode(self) -> &'a UnicodeData {
        self.unicode
    }
}

/// Implemented serializers are pure functions over a frozen document.
pub trait Serializer {
    type Error;

    fn serialize(
        &self,
        context: OutputContext<'_>,
        request: &OutputRequest,
    ) -> Result<GeneratedFile, Self::Error>;
}

#[cfg(test)]
mod tests;
