use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use bib_unicode::{LegacyEncoding, RecodeSet, TexRecoder};
use umber_vfs::{FileContentId, VfsSnapshot, VirtualPath};

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawName {
    value: String,
}

impl RawName {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibTexField {
    name: String,
    value: String,
    names: Option<Vec<RawName>>,
}

impl BibTexField {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
    #[must_use]
    pub fn raw_names(&self) -> Option<&[RawName]> {
        self.names.as_deref()
    }
    #[must_use]
    pub fn classic_names(
        &self,
        options: crate::ClassicNameOptions<'_>,
    ) -> Option<crate::ClassicNameParse> {
        self.names
            .as_ref()
            .map(|_| crate::parse_classic_name_list(&self.value, options))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibTexEntry {
    key: String,
    entry_type: String,
    fields: Vec<BibTexField>,
}

impl BibTexEntry {
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }
    #[must_use]
    pub fn entry_type(&self) -> &str {
        &self.entry_type
    }
    #[must_use]
    pub fn fields(&self) -> &[BibTexField] {
        &self.fields
    }
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&BibTexField> {
        self.fields
            .iter()
            .find(|field| field.name.eq_ignore_ascii_case(name))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibTexPreamble(String);

impl BibTexPreamble {
    #[must_use]
    pub fn value(&self) -> &str {
        &self.0
    }
}

/// Eager Biber-facing adapter derived from [`RawBibDatabase`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibTexSource {
    entries: Vec<BibTexEntry>,
    preambles: Vec<BibTexPreamble>,
    macros: BTreeMap<String, String>,
    diagnostics: Vec<BibTexDiagnostic>,
}

impl BibTexSource {
    #[must_use]
    pub fn from_raw(raw: &RawBibDatabase) -> Self {
        BiberAdapter::new(raw).convert()
    }
    #[must_use]
    pub fn entries(&self) -> &[BibTexEntry] {
        &self.entries
    }
    #[must_use]
    pub fn preambles(&self) -> &[BibTexPreamble] {
        &self.preambles
    }
    #[must_use]
    pub fn macros(&self) -> &BTreeMap<String, String> {
        &self.macros
    }
    #[must_use]
    pub fn diagnostics(&self) -> &[BibTexDiagnostic] {
        &self.diagnostics
    }
    #[must_use]
    pub fn entry(&self, key: &str) -> Option<&BibTexEntry> {
        self.entries
            .iter()
            .find(|entry| entry.key.eq_ignore_ascii_case(key))
    }
}

struct BiberAdapter<'a> {
    raw: &'a RawBibDatabase,
    source: BibTexSource,
    keys: BTreeMap<String, String>,
}

impl<'a> BiberAdapter<'a> {
    fn new(raw: &'a RawBibDatabase) -> Self {
        Self {
            raw,
            source: BibTexSource {
                entries: Vec::new(),
                preambles: Vec::new(),
                macros: month_macros(),
                diagnostics: raw.diagnostics().to_vec(),
            },
            keys: BTreeMap::new(),
        }
    }

    fn convert(mut self) -> BibTexSource {
        for record in self.raw.records() {
            match record {
                RawBibRecord::String(mac) => {
                    let name = self.recode(mac.name().folded());
                    let value = self.expand(mac.value());
                    self.source.macros.insert(name, value);
                }
                RawBibRecord::Preamble(preamble) => {
                    let value = self.expand(preamble.value());
                    self.source.preambles.push(BibTexPreamble(value));
                }
                RawBibRecord::Entry(raw) => self.entry(raw),
                RawBibRecord::Comment(_) | RawBibRecord::Recovery(_) => {}
            }
        }
        self.source
    }

    fn entry(&mut self, raw: &RawBibEntry) {
        let key = self.recode(raw.key().source());
        let folded = key.to_ascii_lowercase();
        if self.keys.contains_key(&folded) {
            return;
        }
        self.keys.insert(folded, key.clone());
        let mut fields = Vec::new();
        let mut seen_fields = BTreeSet::new();
        for raw_field in raw.fields() {
            let name = self.recode(raw_field.name().folded());
            let value = self.expand(raw_field.value());
            if !seen_fields.insert(name.clone()) {
                continue;
            }
            let names = is_name_field(&name).then(|| split_names(&value));
            fields.push(BibTexField { name, value, names });
        }
        add_date_parts(&mut fields);
        self.source.entries.push(BibTexEntry {
            key,
            entry_type: self.recode(raw.entry_type().folded()),
            fields,
        });
    }

    fn expand(&mut self, value: &RawBibValue) -> String {
        let mut result = String::new();
        for part in value.parts() {
            match part {
                RawBibValuePart::Braced(text)
                | RawBibValuePart::Quoted(text)
                | RawBibValuePart::Number(text) => result.push_str(&self.recode(text.source())),
                RawBibValuePart::Macro(name) => {
                    let name = self.recode(name.folded());
                    if let Some(value) = self.source.macros.get(&name) {
                        result.push_str(value);
                    } else {
                        self.source.diagnostics.push(diagnostic(
                            BibTexDiagnosticKind::UndefinedMacro,
                            name_location(part),
                            format!("undefined string macro `{name}`"),
                        ));
                        result.push_str(&name);
                    }
                }
            }
        }
        result
    }

    fn recode(&self, value: &str) -> String {
        TexRecoder::new(self.raw.options().decode, RecodeSet::Null).decode(value)
    }
}

fn name_location(part: &RawBibValuePart) -> usize {
    part.location().byte_start()
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct CacheKey {
    content: FileContentId,
    encoding: u8,
    decode: u8,
    limits: BibTexLimits,
}

#[derive(Debug, Default)]
pub struct BibTexCache {
    values: HashMap<CacheKey, Arc<BibTexSource>>,
}

impl BibTexCache {
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn parse(
        &mut self,
        snapshot: &VfsSnapshot,
        path: &VirtualPath,
        options: BibTexOptions,
    ) -> Result<Arc<BibTexSource>, BibTexDiagnostic> {
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
        let key = CacheKey::new(file.content_id(), options);
        if let Some(source) = self.values.get(&key) {
            return Ok(Arc::clone(source));
        }
        let source = Arc::new(parse_bibtex_bytes(file.bytes(), options));
        self.values.insert(key, Arc::clone(&source));
        Ok(source)
    }
}

impl CacheKey {
    fn new(content: FileContentId, options: BibTexOptions) -> Self {
        Self {
            content,
            encoding: match options.encoding {
                LegacyEncoding::Utf8 => 0,
                LegacyEncoding::Latin1 => 1,
                LegacyEncoding::Latin2 => 2,
                LegacyEncoding::Latin3 => 3,
                LegacyEncoding::MacRoman => 4,
            },
            decode: match options.decode {
                RecodeSet::Null => 0,
                RecodeSet::Base => 1,
                RecodeSet::Full => 2,
            },
            limits: options.limits,
        }
    }
}

pub fn parse_bibtex(
    snapshot: &VfsSnapshot,
    path: &VirtualPath,
    options: BibTexOptions,
) -> Result<BibTexSource, BibTexDiagnostic> {
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
pub fn parse_bibtex_bytes(bytes: &[u8], options: BibTexOptions) -> BibTexSource {
    BibTexSource::from_raw(&parse_raw_bibtex_bytes(bytes, options))
}

fn month_macros() -> BTreeMap<String, String> {
    [
        ("jan", "1"),
        ("feb", "2"),
        ("mar", "3"),
        ("apr", "4"),
        ("may", "5"),
        ("jun", "6"),
        ("jul", "7"),
        ("aug", "8"),
        ("sep", "9"),
        ("oct", "10"),
        ("nov", "11"),
        ("dec", "12"),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value.to_owned()))
    .collect()
}

fn is_name_field(name: &str) -> bool {
    matches!(
        name,
        "author"
            | "editor"
            | "translator"
            | "commentator"
            | "annotator"
            | "introduction"
            | "foreword"
            | "afterword"
            | "bookauthor"
            | "holder"
            | "namea"
            | "nameb"
            | "namec"
    )
}

fn split_names(value: &str) -> Vec<RawName> {
    let bytes = value.as_bytes();
    let mut start = 0usize;
    let mut at = 0usize;
    let mut braces = 0usize;
    let mut names = Vec::new();
    while at < bytes.len() {
        match bytes[at] {
            b'{' => braces += 1,
            b'}' if braces > 0 => braces -= 1,
            b'a' | b'A'
                if braces == 0
                    && value[at..]
                        .get(..3)
                        .is_some_and(|part| part.eq_ignore_ascii_case("and")) =>
            {
                let before = at == 0 || bytes[at - 1].is_ascii_whitespace();
                let after = at + 3 == bytes.len() || bytes[at + 3].is_ascii_whitespace();
                if before && after {
                    let name = value[start..at].trim();
                    if !name.is_empty() {
                        names.push(RawName {
                            value: name.to_owned(),
                        });
                    }
                    at += 3;
                    start = at;
                    continue;
                }
            }
            _ => {}
        }
        at += 1;
    }
    let name = value[start..].trim();
    if !name.is_empty() {
        names.push(RawName {
            value: name.to_owned(),
        });
    }
    names
}

fn add_date_parts(fields: &mut Vec<BibTexField>) {
    let Some(date) = fields
        .iter()
        .find(|field| field.name == "date")
        .map(|field| field.value.clone())
    else {
        return;
    };
    let mut parts = date.split('-');
    let year = parts.next().filter(|value| value.len() == 4);
    let month = parts
        .next()
        .and_then(|value| value.parse::<u8>().ok())
        .filter(|value| (1..=12).contains(value));
    if let Some(year) = year
        && !fields.iter().any(|field| field.name == "year")
    {
        fields.push(BibTexField {
            name: "year".into(),
            value: year.into(),
            names: None,
        });
    }
    if let Some(month) = month
        && !fields.iter().any(|field| field.name == "month")
    {
        fields.push(BibTexField {
            name: "month".into(),
            value: month.to_string(),
            names: None,
        });
    }
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
