//! Detached deterministic bibliography serializer boundary.

mod bbl;
mod bibtex;
mod dot;
mod router;
mod xml;

use bib_model::ProcessedBibliography;
use bib_unicode::UnicodeData;

pub use bbl::BblSerializer;
pub use bibtex::{BibtexCase, BibtexMacro, BibtexOptions, BibtexSerializer};
pub use dot::{DotInclude, DotOptions, DotSerializer};
pub use router::{OutputFailure, OutputFailureKind, OutputOptions, OutputPlan, OutputRouter};
pub use xml::{
    BBL_XML_NAMESPACE, BIBLATEX_XML_NAMESPACE, BblXmlSerializer, BibLatexXmlSerializer,
    XmlSchemaKind, generate_xml_schema,
};

pub type BblOutputFailure = OutputFailure;
pub type BblOutputFailureKind = OutputFailureKind;
pub type BibtexOutputFailure = OutputFailure;
pub type BibtexOutputFailureKind = OutputFailureKind;
pub type DotOutputFailure = OutputFailure;
pub type DotOutputFailureKind = OutputFailureKind;
pub type XmlOutputFailure = OutputFailure;
pub type XmlOutputFailureKind = OutputFailureKind;

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

#[cfg(test)]
mod tests;
