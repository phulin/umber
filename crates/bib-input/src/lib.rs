//! Control, configuration, and datasource input-stage boundary.

mod biblatexml;
mod bibtex;
mod config;
mod control;
mod extended_names;
mod names;
mod xml;

pub use biblatexml::{
    BIBLATEX_XML_NAMESPACE, BibLatexXmlData, BibLatexXmlEntry, BibLatexXmlError, NamePart,
    XmlAnnotation, XmlFieldValue, XmlListItem, XmlName, parse_biblatexml, parse_biblatexml_bytes,
    validate_biblatexml_bytes,
};
pub use bibtex::{
    BibTexCache, BibTexDiagnostic, BibTexDiagnosticKind, BibTexEntry, BibTexField, BibTexLimits,
    BibTexOptions, BibTexPreamble, BibTexSource, RawBibClassicSource, RawBibComment,
    RawBibControlSequence, RawBibDatabase, RawBibEntry, RawBibField, RawBibIdentifier,
    RawBibLocation, RawBibPreamble, RawBibRecord, RawBibRecovery, RawBibStringMacro, RawBibText,
    RawBibValue, RawBibValuePart, RawName, parse_bibtex, parse_bibtex_bytes,
    parse_raw_bibtex_bytes,
};
pub use config::{
    ConfigError, ConfigValue, ConfigurationFile, ConfigurationLayer, ResolvedConfiguration,
    parse_config, parse_config_bytes, validate_config_bytes,
};
pub use control::{
    CONTROL_NAMESPACE, CONTROL_VERSION, ControlError, ControlFile, ControlOptionSet,
    ControlOptionValue, ControlSection, DataModel, DataModelField, OptionComponent,
    StructuredValue, Template, TemplateElement, parse_control, parse_control_bytes,
    validate_control_bytes,
};
pub use extended_names::{
    ExtendedNameDiagnostic, ExtendedNameDiagnosticKind, ExtendedNameLimits, ExtendedNameOptions,
    ExtendedNameParse, parse_extended_name,
};
pub use names::{
    ClassicNameDiagnostic, ClassicNameDiagnosticKind, ClassicNameLimits, ClassicNameOptions,
    ClassicNameParse, NameHashScope, classic_name_hash, parse_classic_name,
    parse_classic_name_list,
};
pub use xml::{XmlError, XmlLimits};

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

#[cfg(test)]
mod tests;
