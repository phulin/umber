use bib_unicode::{LegacyEncoding, RecodeSet};
use umber_vfs::{VfsSnapshot, VirtualPath};

mod raw;

pub use raw::{
    RawBibClassicSource, RawBibComment, RawBibControlSequence, RawBibDatabase, RawBibEntry,
    RawBibField, RawBibIdentifier, RawBibLocation, RawBibPreamble, RawBibRecord, RawBibRecovery,
    RawBibStringMacro, RawBibText, RawBibValue, RawBibValuePart, parse_raw_bibtex_bytes,
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BibTexLimits {
    pub max_bytes: usize,
    pub max_entries: usize,
    pub max_fields_per_entry: usize,
    pub max_macros: usize,
    pub max_value_bytes: usize,
    pub max_nesting: usize,
    pub max_work: usize,
    pub max_diagnostics: usize,
}

impl Default for BibTexLimits {
    fn default() -> Self {
        Self {
            max_bytes: 16 * 1024 * 1024,
            max_entries: 100_000,
            max_fields_per_entry: 1_000,
            max_macros: 10_000,
            max_value_bytes: 1024 * 1024,
            max_nesting: 256,
            max_work: 64 * 1024 * 1024,
            max_diagnostics: 1_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BibTexOptions {
    pub encoding: LegacyEncoding,
    pub decode: RecodeSet,
    pub limits: BibTexLimits,
}

impl Default for BibTexOptions {
    fn default() -> Self {
        Self {
            encoding: LegacyEncoding::Utf8,
            decode: RecodeSet::Base,
            limits: BibTexLimits::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BibTexDiagnosticKind {
    Encoding,
    Syntax,
    UndefinedMacro,
    DuplicateEntry,
    CaseCollision,
    DuplicateField,
    Limit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibTexDiagnostic {
    pub kind: BibTexDiagnosticKind,
    pub offset: usize,
    pub message: String,
}

pub fn parse_bibtex(
    snapshot: &VfsSnapshot,
    path: &VirtualPath,
    options: BibTexOptions,
) -> Result<RawBibDatabase, BibTexDiagnostic> {
    let file = snapshot
        .get(path)
        .map_err(|error| {
            diagnostic(
                BibTexDiagnosticKind::Syntax,
                0,
                format!("cannot read `{path}`: {error}"),
            )
        })?
        .ok_or_else(|| {
            diagnostic(
                BibTexDiagnosticKind::Syntax,
                0,
                format!("datasource `{path}` is missing"),
            )
        })?;
    Ok(parse_bibtex_bytes(file.bytes(), options))
}

#[must_use]
pub fn parse_bibtex_bytes(bytes: &[u8], options: BibTexOptions) -> RawBibDatabase {
    parse_raw_bibtex_bytes(bytes, options)
}

pub(super) fn diagnostic(
    kind: BibTexDiagnosticKind,
    offset: usize,
    message: impl Into<String>,
) -> BibTexDiagnostic {
    BibTexDiagnostic {
        kind,
        offset,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests;
