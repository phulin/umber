//! Classic BibTeX `READ` preparation.
//!
//! This is intentionally separate from the Biber conversion path.  The raw
//! syntax layer is scanned in datasource order so that classic macro lifetime,
//! duplicate handling, citation selection, and crossref inheritance remain
//! observable to the later BST VM.

use std::collections::{BTreeMap, BTreeSet};
use std::mem;
use std::sync::Arc;

use bib_input::{RawBibDatabase, RawBibEntry, RawBibRecord, RawBibValue, RawBibValuePart};
use umber_vfs::{FileContentId, VirtualPath};

use super::{ClassicStringPool, CompiledStyle, SymbolKind};
use crate::{ClassicControl, ClassicDatabaseOptions, ClassicSourceLocation};

/// One raw datasource with its immutable VFS identity.
#[derive(Clone, Copy, Debug)]
pub struct ClassicDatabaseSource<'a> {
    path: &'a VirtualPath,
    content_id: FileContentId,
    database: &'a RawBibDatabase,
}

impl<'a> ClassicDatabaseSource<'a> {
    #[must_use]
    pub const fn new(
        path: &'a VirtualPath,
        content_id: FileContentId,
        database: &'a RawBibDatabase,
    ) -> Self {
        Self {
            path,
            content_id,
            database,
        }
    }

    pub(super) fn identity(&self) -> (VirtualPath, FileContentId, bib_input::BibTexOptions) {
        (self.path.clone(), self.content_id, self.database.options())
    }
}

/// A diagnostic emitted while preparing classic VM-visible entry state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassicDatabaseDiagnostic {
    kind: ClassicDatabaseDiagnosticKind,
    message: String,
    source: Option<ClassicSourceLocation>,
}

impl ClassicDatabaseDiagnostic {
    #[must_use]
    #[cfg(test)]
    pub const fn kind(&self) -> ClassicDatabaseDiagnosticKind {
        self.kind
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    #[must_use]
    pub const fn source(&self) -> Option<&ClassicSourceLocation> {
        self.source.as_ref()
    }
}

/// Stable classes of classic `READ` diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClassicDatabaseDiagnosticKind {
    DuplicateEntry,
    DuplicateField,
    UndefinedMacro,
    MissingCitation,
    MissingCrossref,
    CrossrefCycle,
    UndefinedEntryType,
    Limit,
}

/// A selected entry projected to the compiled style's `ENTRY` fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassicDatabaseEntry {
    key: String,
    entry_type: String,
    fields: Vec<Option<String>>,
    crossref: Option<String>,
    source: ClassicSourceLocation,
}

impl ClassicDatabaseEntry {
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    #[must_use]
    pub fn entry_type(&self) -> &str {
        &self.entry_type
    }

    /// Returns `None` for a missing field; an empty string remains present.
    #[must_use]
    pub fn field(&self, index: u32) -> Option<&str> {
        self.fields.get(index as usize).and_then(Option::as_deref)
    }

    #[must_use]
    pub fn crossref(&self) -> Option<&str> {
        self.crossref.as_deref()
    }

    #[must_use]
    pub const fn source(&self) -> &ClassicSourceLocation {
        &self.source
    }
}

/// Immutable input to the later classic style VM.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ClassicDatabase {
    entries: Arc<[ClassicDatabaseEntry]>,
    preambles: Arc<[String]>,
    diagnostics: Arc<[ClassicDatabaseDiagnostic]>,
    pool_trace: Arc<[String]>,
}

impl ClassicDatabase {
    pub fn entries(&self) -> impl ExactSizeIterator<Item = &ClassicDatabaseEntry> {
        self.entries.iter()
    }

    /// The value visible through the future `preamble$` builtin.
    #[must_use]
    pub fn preamble(&self) -> String {
        self.preambles.concat()
    }

    pub fn diagnostics(&self) -> impl ExactSizeIterator<Item = &ClassicDatabaseDiagnostic> {
        self.diagnostics.iter()
    }

    /// Replays the raw database values that Web2C retains in its string pool
    /// while processing `READ`. The trace is independent of the prepared
    /// database cache and must be applied only to a single job pool.
    pub(crate) fn apply_pool_trace(&self, pool: &mut ClassicStringPool) {
        for value in self.pool_trace.iter() {
            let _ = pool.intern(value);
        }
    }

    /// Conservatively charges every allocation retained by this immutable
    /// prepared view. The raw parser input is deliberately excluded: a
    /// prepared database owns only its projected entry state and diagnostics.
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        mem::size_of::<Self>()
            .saturating_add(
                self.entries
                    .iter()
                    .map(ClassicDatabaseEntry::retained_bytes)
                    .sum::<usize>(),
            )
            .saturating_add(
                self.preambles
                    .iter()
                    .map(|value| mem::size_of::<String>().saturating_add(value.capacity()))
                    .sum::<usize>(),
            )
            .saturating_add(
                self.diagnostics
                    .iter()
                    .map(ClassicDatabaseDiagnostic::retained_bytes)
                    .sum::<usize>(),
            )
            .saturating_add(
                self.pool_trace
                    .iter()
                    .map(|value| mem::size_of::<String>().saturating_add(value.capacity()))
                    .sum::<usize>(),
            )
    }
}

impl ClassicDatabaseEntry {
    pub(super) fn retained_bytes(&self) -> usize {
        mem::size_of::<Self>()
            .saturating_add(self.key.capacity())
            .saturating_add(self.entry_type.capacity())
            .saturating_add(self.crossref.as_ref().map_or(0, String::capacity))
            .saturating_add(
                self.fields
                    .iter()
                    .map(|value| {
                        mem::size_of::<Option<String>>()
                            + value.as_ref().map_or(0, String::capacity)
                    })
                    .sum::<usize>(),
            )
            .saturating_add(self.source.path().as_str().len())
    }
}

impl ClassicDatabaseDiagnostic {
    fn retained_bytes(&self) -> usize {
        mem::size_of::<Self>()
            .saturating_add(self.message.capacity())
            .saturating_add(
                self.source
                    .as_ref()
                    .map_or(0, |source| source.path().as_str().len()),
            )
    }
}

/// Prepares one classic database without caching.
#[must_use]
pub fn prepare_classic_database(
    control: &ClassicControl,
    style: &CompiledStyle,
    sources: &[ClassicDatabaseSource<'_>],
    options: &ClassicDatabaseOptions,
) -> ClassicDatabase {
    let mut reader = Reader::new(style, options);
    for source in sources {
        reader.scan(source);
    }
    reader.select(control.citations());
    reader.finish()
}

#[derive(Clone)]
struct RawEntry {
    key: String,
    entry_type: String,
    fields: Vec<Option<String>>,
    crossref: Option<String>,
    source: ClassicSourceLocation,
}

struct Reader<'a> {
    style: &'a CompiledStyle,
    options: &'a ClassicDatabaseOptions,
    macros: BTreeMap<String, String>,
    entries: BTreeMap<String, RawEntry>,
    source_order: Vec<String>,
    preambles: Vec<String>,
    pool_trace: Vec<String>,
    all_entries: bool,
    diagnostics: Vec<ClassicDatabaseDiagnostic>,
    work: usize,
    work_exhausted: bool,
}

impl<'a> Reader<'a> {
    fn new(style: &'a CompiledStyle, options: &'a ClassicDatabaseOptions) -> Self {
        let macros = style
            .declarations()
            .symbols()
            .iter()
            .filter_map(|symbol| match symbol.kind() {
                SymbolKind::StringMacro(id) => style
                    .declarations()
                    .strings()
                    .get(id.0 as usize)
                    .map(|value| (symbol.name().to_owned(), value.clone())),
                _ => None,
            })
            .collect();
        Self {
            style,
            options,
            macros,
            entries: BTreeMap::new(),
            source_order: Vec::new(),
            preambles: Vec::new(),
            pool_trace: Vec::new(),
            all_entries: false,
            diagnostics: Vec::new(),
            work: 0,
            work_exhausted: false,
        }
    }

    fn scan(&mut self, source: &ClassicDatabaseSource<'_>) {
        for record in source.database.classic().records() {
            if !self.charge() {
                break;
            }
            match record {
                RawBibRecord::String(mac) => {
                    let name = mac.name().folded();
                    if !self.macros.contains_key(name)
                        && self.macros.len() >= self.options.limits().macros
                    {
                        self.diagnostic(
                            ClassicDatabaseDiagnosticKind::Limit,
                            "classic READ macro limit exceeded",
                        );
                    } else {
                        let value = self.expand(mac.value());
                        self.pool_trace.push(name.to_owned());
                        self.pool_trace.push(value.clone());
                        self.macros.insert(name.to_owned(), value);
                    }
                }
                RawBibRecord::Preamble(preamble) => {
                    let value = self.expand(preamble.value());
                    if self.preambles.iter().map(String::len).sum::<usize>() + value.len()
                        > self.options.limits().preamble_bytes
                    {
                        self.diagnostic(
                            ClassicDatabaseDiagnosticKind::Limit,
                            "preamble byte limit exceeded",
                        );
                    } else {
                        self.pool_trace.push(value.clone());
                        self.preambles.push(value);
                    }
                }
                RawBibRecord::Entry(entry) => self.entry(source.path, entry),
                RawBibRecord::Comment(_) | RawBibRecord::Recovery(_) => {}
            }
        }
    }

    fn entry(&mut self, path: &VirtualPath, entry: &RawBibEntry) {
        if !self.charge() {
            return;
        }
        if self.entries.len() >= self.options.limits().entries {
            self.diagnostic(
                ClassicDatabaseDiagnosticKind::Limit,
                "classic READ entry limit exceeded",
            );
            return;
        }
        let folded = entry.key().folded().to_owned();
        if self.entries.contains_key(&folded) {
            self.diagnostic(
                ClassicDatabaseDiagnosticKind::DuplicateEntry,
                format!("duplicate classic entry `{}`", entry.key().source()),
            );
            return;
        }
        let mut fields = vec![None; self.style.declarations().entry_fields().len()];
        let mut seen_fields = BTreeSet::new();
        let mut crossref = None;
        for field in entry
            .fields()
            .iter()
            .take(self.options.limits().fields_per_entry)
        {
            let name = field.name().folded().to_owned();
            if !seen_fields.insert(name.clone()) {
                self.diagnostic(
                    ClassicDatabaseDiagnosticKind::DuplicateField,
                    format!("duplicate field `{name}` in `{}`", entry.key().source()),
                );
                continue;
            }
            let value = self.expand(field.value());
            if name == "crossref" {
                crossref = Some(value.clone());
            }
            if let Some(SymbolKind::EntryField(index)) = self
                .style
                .declarations()
                .lookup(&name)
                .and_then(|symbol| self.style.declarations().symbol(symbol))
                .map(|declaration| declaration.kind())
            {
                fields[*index as usize] = Some(value);
            }
        }
        if entry.fields().len() > self.options.limits().fields_per_entry {
            self.diagnostic(
                ClassicDatabaseDiagnosticKind::Limit,
                "classic READ field limit exceeded",
            );
        }
        self.source_order.push(folded.clone());
        self.entries.insert(
            folded,
            RawEntry {
                key: entry.key().source().to_owned(),
                entry_type: entry.entry_type().folded().to_owned(),
                fields,
                crossref,
                source: ClassicSourceLocation::new(
                    path.clone(),
                    entry.location().byte_start() as u64,
                    u32::try_from(entry.location().line()).ok(),
                ),
            },
        );
    }

    fn expand(&mut self, value: &RawBibValue) -> String {
        let mut output = String::new();
        for part in value.parts() {
            if !self.charge() {
                break;
            }
            match part {
                RawBibValuePart::Braced(text)
                | RawBibValuePart::Quoted(text)
                | RawBibValuePart::Number(text) => output.push_str(text.source()),
                RawBibValuePart::Macro(name) => match self.macros.get(name.folded()) {
                    Some(value) => output.push_str(value),
                    None => {
                        self.diagnostic(
                            ClassicDatabaseDiagnosticKind::UndefinedMacro,
                            format!("undefined string macro `{}`", name.source()),
                        );
                    }
                },
            }
            if output.len() > self.options.limits().value_bytes {
                self.diagnostic(
                    ClassicDatabaseDiagnosticKind::Limit,
                    "classic READ value byte limit exceeded",
                );
                output.truncate(self.options.limits().value_bytes);
                break;
            }
        }
        normalize_classic_value(&output)
    }

    fn select<'b>(&mut self, citations: impl Iterator<Item = &'b str>) {
        let citations = citations.collect::<Vec<_>>();
        let wildcard = citations.contains(&"*");
        self.all_entries = wildcard;
        let mut selected = Vec::new();
        let mut seen = BTreeSet::new();
        for citation in citations {
            if !self.charge() {
                break;
            }
            if citation == "*" {
                for key in self.source_order.clone() {
                    if !self.charge() {
                        break;
                    }
                    if seen.insert(key.clone()) {
                        selected.push(key);
                    }
                }
                continue;
            }
            let folded = citation.to_ascii_lowercase();
            if self.entries.contains_key(&folded) {
                if seen.insert(folded.clone()) {
                    selected.push(folded);
                }
            } else {
                self.diagnostic(
                    ClassicDatabaseDiagnosticKind::MissingCitation,
                    format!("citation `{citation}` was not found"),
                );
            }
        }
        if wildcard && selected.is_empty() {
            return;
        }
        let mut refs = BTreeMap::<String, usize>::new();
        for key in &selected {
            if !self.charge() {
                break;
            }
            if let Some(parent) = self
                .entries
                .get(key)
                .and_then(|entry| entry.crossref.clone())
            {
                *refs.entry(parent.to_ascii_lowercase()).or_default() += 1;
            }
        }
        for key in self.source_order.clone() {
            if !self.charge() {
                break;
            }
            if refs
                .get(&key)
                .copied()
                .is_some_and(|count| count >= self.options.min_crossrefs())
                && seen.insert(key.clone())
            {
                selected.push(key);
            }
        }
        self.source_order = selected;
    }

    fn finish(mut self) -> ClassicDatabase {
        let selected_raw_entries = self
            .source_order
            .iter()
            .filter_map(|key| self.entries.get(key))
            .collect::<Vec<_>>();
        for entry in &selected_raw_entries {
            if self.all_entries {
                // Whole-database inclusion discovers each database key while
                // reading records; ordinary citations already own their keys
                // before READ starts.
                self.pool_trace.push(entry.key.clone());
            }
            for value in entry.fields.iter().flatten() {
                self.pool_trace.push(value.clone());
            }
            if let Some(value) = &entry.crossref {
                self.pool_trace.push(value.clone());
            }
        }
        let entries = self
            .source_order
            .clone()
            .into_iter()
            .filter_map(|key| self.visible_entry(&key, &mut BTreeSet::new(), 0))
            .collect::<Vec<_>>();
        for entry in &entries {
            if self
                .style
                .declarations()
                .lookup(entry.entry_type())
                .is_none()
            {
                self.diagnostic_at(
                    ClassicDatabaseDiagnosticKind::UndefinedEntryType,
                    format!(
                        "entry type for \"{}\" isn't style-file defined",
                        entry.key()
                    ),
                    entry.source().clone(),
                );
            }
        }
        ClassicDatabase {
            entries: entries.into(),
            preambles: self.preambles.into(),
            diagnostics: self.diagnostics.into(),
            pool_trace: self.pool_trace.into(),
        }
    }

    fn visible_entry(
        &mut self,
        key: &str,
        visiting: &mut BTreeSet<String>,
        depth: usize,
    ) -> Option<ClassicDatabaseEntry> {
        if !self.charge() {
            return None;
        }
        if depth > self.options.limits().crossref_depth {
            self.diagnostic(
                ClassicDatabaseDiagnosticKind::Limit,
                "crossref depth limit exceeded",
            );
            return None;
        }
        let entry = self.entries.get(key)?.clone();
        if !visiting.insert(key.to_owned()) {
            self.diagnostic(
                ClassicDatabaseDiagnosticKind::CrossrefCycle,
                format!("crossref cycle at `{}`", entry.key),
            );
            return Some(project(&entry, None, entry.crossref.clone()));
        }
        let parent = entry
            .crossref
            .as_ref()
            .map(|value| value.to_ascii_lowercase());
        let inherited = parent.as_ref().and_then(|parent| {
            if self.entries.contains_key(parent) {
                self.visible_entry(parent, visiting, depth + 1)
            } else {
                self.diagnostic(
                    ClassicDatabaseDiagnosticKind::MissingCrossref,
                    format!(
                        "crossref `{}` for `{}` was not found",
                        entry.crossref.as_deref().unwrap_or_default(),
                        entry.key
                    ),
                );
                None
            }
        });
        let crossref = parent
            .as_ref()
            .and_then(|parent| self.entries.get(parent))
            .map(|parent| parent.key.clone())
            .or_else(|| entry.crossref.clone());
        visiting.remove(key);
        Some(project(&entry, inherited.as_ref(), crossref))
    }

    fn charge(&mut self) -> bool {
        if self.work_exhausted {
            return false;
        }
        self.work = self.work.saturating_add(1);
        if self.work > self.options.limits().work {
            self.work_exhausted = true;
            self.diagnostic(
                ClassicDatabaseDiagnosticKind::Limit,
                "classic READ work limit exceeded",
            );
            return false;
        }
        true
    }

    fn diagnostic(&mut self, kind: ClassicDatabaseDiagnosticKind, message: impl Into<String>) {
        if self.diagnostics.len() < self.options.limits().diagnostics {
            self.diagnostics.push(ClassicDatabaseDiagnostic {
                kind,
                message: message.into(),
                source: None,
            });
        }
    }

    fn diagnostic_at(
        &mut self,
        kind: ClassicDatabaseDiagnosticKind,
        message: impl Into<String>,
        source: ClassicSourceLocation,
    ) {
        if self.diagnostics.len() < self.options.limits().diagnostics {
            self.diagnostics.push(ClassicDatabaseDiagnostic {
                kind,
                message: message.into(),
                source: Some(source),
            });
        }
    }
}

fn normalize_classic_value(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut pending_space = false;
    for character in value.chars() {
        if character.is_ascii_whitespace() {
            pending_space = true;
        } else {
            if pending_space {
                normalized.push(' ');
                pending_space = false;
            }
            normalized.push(character);
        }
    }
    if pending_space {
        normalized.push(' ');
    }
    normalized
}

fn project(
    entry: &RawEntry,
    inherited: Option<&ClassicDatabaseEntry>,
    crossref: Option<String>,
) -> ClassicDatabaseEntry {
    let fields = entry
        .fields
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value.clone().or_else(|| {
                inherited.and_then(|parent| parent.field(index as u32).map(str::to_owned))
            })
        })
        .collect();
    ClassicDatabaseEntry {
        key: entry.key.clone(),
        entry_type: entry.entry_type.clone(),
        fields,
        crossref,
        source: entry.source.clone(),
    }
}

#[cfg(test)]
#[path = "read/tests.rs"]
mod tests;
