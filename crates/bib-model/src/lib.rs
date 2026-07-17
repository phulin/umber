//! Typed immutable contracts shared by Umber's bibliography stages.
//!
//! Mutable construction is deliberately restricted to validating builders.
//! Frozen values retain every observable order explicitly and contain no host
//! handles or process-global state.

mod diagnostic;
mod document;
mod identifier;
mod options;
mod source;
mod value;

pub use bib_unicode::CompatibilityVersion;
pub use diagnostic::{
    BibDiagnostic, BibDiagnosticCode, BibSeverity, DiagnosticBuilder, DiagnosticError,
};
pub use document::{
    Annotation, BibConfiguration, BibConfigurationBuilder, BuildError, DataList, DataListItem,
    DataListKind, Entry, EntryBuilder, GeneratedFile, OutputFormat, OutputNewline, OutputRequest,
    ProcessedBibliography, ProcessedBibliographyBuilder, ProcessedSection, ProcessedSectionBuilder,
};
pub use identifier::{
    DataListId, EntryId, EntryType, FieldId, IdentifierError, OptionId, SectionId, TransformationId,
};
pub use options::{OptionLayer, OptionScope, OptionValue, ScopedOptions, ScopedOptionsBuilder};
pub use source::{BibSourceLocation, DerivedFrom, FieldProvenance, SourceSpan};
pub use umber_vfs::VirtualPath;
pub use value::{
    DateValue, Field, FieldMap, FieldValue, FieldValueStage, Literal, LiteralList, Name,
    NameAssignment, NameBuilder, NameList, NamePartKind, NamePartValue, Range, RangeEndpoint,
    RangeList, Uri, UriList, Verbatim,
};

/// Pinned semantic versions represented by this model revision.
pub const COMPATIBILITY_VERSION: CompatibilityVersion = CompatibilityVersion::BIBER_2_22_BETA;

#[cfg(test)]
mod tests;
