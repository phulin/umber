use std::fmt;
use std::sync::Arc;

use bib_unicode::{CompatibilityVersion, LegacyEncoding};
use umber_vfs::VirtualPath;

use crate::{
    BibSourceLocation, DataListId, EntryId, EntryType, Field, FieldId, FieldMap, FieldProvenance,
    FieldValue, FieldValueStage, ScopedOptions, SectionId,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Annotation {
    field: Option<FieldId>,
    name: FieldId,
    value: String,
}

impl Annotation {
    pub fn new(name: FieldId, value: impl Into<String>) -> Result<Self, BuildError> {
        let value = value.into();
        if value.contains('\0') {
            return Err(BuildError::Invalid("annotation values cannot contain NUL"));
        }
        Ok(Self {
            field: None,
            name,
            value,
        })
    }

    pub fn for_field(
        field: FieldId,
        name: FieldId,
        value: impl Into<String>,
    ) -> Result<Self, BuildError> {
        let mut annotation = Self::new(name, value)?;
        annotation.field = Some(field);
        Ok(annotation)
    }

    #[must_use]
    pub const fn field(&self) -> Option<&FieldId> {
        self.field.as_ref()
    }

    #[must_use]
    pub const fn name(&self) -> &FieldId {
        &self.name
    }

    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
    id: EntryId,
    entry_type: EntryType,
    fields: FieldMap,
    options: ScopedOptions,
    annotations: Arc<[Annotation]>,
    source: BibSourceLocation,
}

impl Entry {
    #[must_use]
    pub const fn id(&self) -> &EntryId {
        &self.id
    }
    #[must_use]
    pub const fn entry_type(&self) -> &EntryType {
        &self.entry_type
    }
    #[must_use]
    pub const fn fields(&self) -> &FieldMap {
        &self.fields
    }
    #[must_use]
    pub const fn options(&self) -> &ScopedOptions {
        &self.options
    }
    pub fn annotations(&self) -> impl ExactSizeIterator<Item = &Annotation> {
        self.annotations.iter()
    }
    #[must_use]
    pub const fn source(&self) -> &BibSourceLocation {
        &self.source
    }
}

#[derive(Clone, Debug)]
pub struct EntryBuilder {
    id: EntryId,
    entry_type: EntryType,
    fields: Vec<Field>,
    options: ScopedOptions,
    annotations: Vec<Annotation>,
    source: BibSourceLocation,
}

impl EntryBuilder {
    #[must_use]
    pub fn new(id: EntryId, entry_type: EntryType, source: BibSourceLocation) -> Self {
        Self {
            id,
            entry_type,
            fields: Vec::new(),
            options: ScopedOptions::default(),
            annotations: Vec::new(),
            source,
        }
    }

    pub fn field(
        &mut self,
        id: FieldId,
        value: FieldValue,
        stage: FieldValueStage,
        provenance: FieldProvenance,
    ) -> Result<&mut Self, BuildError> {
        if self.fields.iter().any(|field| field.id() == &id) {
            return Err(BuildError::DuplicateField(id));
        }
        self.fields.push(Field::new(id, value, stage, provenance));
        Ok(self)
    }

    pub fn options(&mut self, options: ScopedOptions) -> &mut Self {
        self.options = options;
        self
    }

    pub fn annotation(&mut self, annotation: Annotation) -> Result<&mut Self, BuildError> {
        if self
            .annotations
            .iter()
            .any(|existing| existing.field == annotation.field && existing.name == annotation.name)
        {
            return Err(BuildError::DuplicateAnnotation(annotation.name));
        }
        self.annotations.push(annotation);
        Ok(self)
    }

    #[must_use]
    pub fn freeze(self) -> Entry {
        Entry {
            id: self.id,
            entry_type: self.entry_type,
            fields: FieldMap::from_fields(self.fields),
            options: self.options,
            annotations: self.annotations.into(),
            source: self.source,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataList {
    id: DataListId,
    kind: DataListKind,
    items: Arc<[DataListItem]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataListItem {
    entry: EntryId,
    context_fields: Arc<[Field]>,
}

impl DataListItem {
    #[must_use]
    pub const fn entry(&self) -> &EntryId {
        &self.entry
    }

    pub fn context_fields(&self) -> impl ExactSizeIterator<Item = &Field> {
        self.context_fields.iter()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DataListKind {
    #[default]
    Entry,
    List,
}

impl DataList {
    pub fn new(
        id: DataListId,
        entries: impl IntoIterator<Item = EntryId>,
    ) -> Result<Self, BuildError> {
        let entries = entries.into_iter().collect::<Vec<_>>();
        for (index, entry) in entries.iter().enumerate() {
            if entries[..index].contains(entry) {
                return Err(BuildError::DuplicateListEntry(entry.clone()));
            }
        }
        Ok(Self {
            id,
            kind: DataListKind::Entry,
            items: entries
                .into_iter()
                .map(|entry| DataListItem {
                    entry,
                    context_fields: Arc::from([]),
                })
                .collect(),
        })
    }

    #[must_use]
    pub const fn with_kind(mut self, kind: DataListKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn with_context_fields(
        mut self,
        entry: &EntryId,
        fields: impl IntoIterator<Item = Field>,
    ) -> Result<Self, BuildError> {
        let Some(item) = Arc::make_mut(&mut self.items)
            .iter_mut()
            .find(|item| &item.entry == entry)
        else {
            return Err(BuildError::UnknownListEntry(entry.clone()));
        };
        let fields = fields.into_iter().collect::<Vec<_>>();
        for (index, field) in fields.iter().enumerate() {
            if fields[..index]
                .iter()
                .any(|existing| existing.id() == field.id())
            {
                return Err(BuildError::DuplicateContextField(field.id().clone()));
            }
        }
        item.context_fields = fields.into();
        Ok(self)
    }

    #[must_use]
    pub const fn id(&self) -> &DataListId {
        &self.id
    }
    #[must_use]
    pub const fn kind(&self) -> DataListKind {
        self.kind
    }
    pub fn entries(&self) -> impl ExactSizeIterator<Item = &EntryId> {
        self.items.iter().map(DataListItem::entry)
    }
    pub fn items(&self) -> impl ExactSizeIterator<Item = &DataListItem> {
        self.items.iter()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessedSection {
    id: SectionId,
    entries: Arc<[Entry]>,
    lists: Arc<[DataList]>,
    aliases: Arc<[(EntryId, EntryId)]>,
    undefined_keys: Arc<[EntryId]>,
}

impl ProcessedSection {
    #[must_use]
    pub const fn id(&self) -> SectionId {
        self.id
    }
    pub fn entries(&self) -> impl ExactSizeIterator<Item = &Entry> {
        self.entries.iter()
    }
    pub fn lists(&self) -> impl ExactSizeIterator<Item = &DataList> {
        self.lists.iter()
    }
    pub fn aliases(&self) -> impl ExactSizeIterator<Item = (&EntryId, &EntryId)> {
        self.aliases.iter().map(|(alias, target)| (alias, target))
    }
    pub fn undefined_keys(&self) -> impl ExactSizeIterator<Item = &EntryId> {
        self.undefined_keys.iter()
    }
    #[must_use]
    pub fn entry(&self, id: &EntryId) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.id() == id)
    }
}

#[derive(Clone, Debug)]
pub struct ProcessedSectionBuilder {
    id: SectionId,
    entries: Vec<Entry>,
    lists: Vec<DataList>,
    aliases: Vec<(EntryId, EntryId)>,
    undefined_keys: Vec<EntryId>,
}

impl ProcessedSectionBuilder {
    #[must_use]
    pub fn new(id: SectionId) -> Self {
        Self {
            id,
            entries: Vec::new(),
            lists: Vec::new(),
            aliases: Vec::new(),
            undefined_keys: Vec::new(),
        }
    }

    pub fn entry(&mut self, entry: Entry) -> Result<&mut Self, BuildError> {
        if self
            .entries
            .iter()
            .any(|existing| existing.id() == entry.id())
        {
            return Err(BuildError::DuplicateEntry(entry.id().clone()));
        }
        self.entries.push(entry);
        Ok(self)
    }

    pub fn list(&mut self, list: DataList) -> Result<&mut Self, BuildError> {
        if self.lists.iter().any(|existing| existing.id() == list.id()) {
            return Err(BuildError::DuplicateList(list.id().clone()));
        }
        if let Some(missing) = list
            .entries()
            .find(|id| !self.entries.iter().any(|entry| entry.id() == *id))
        {
            return Err(BuildError::UnknownListEntry(missing.clone()));
        }
        self.lists.push(list);
        Ok(self)
    }

    pub fn alias(&mut self, alias: EntryId, target: EntryId) -> Result<&mut Self, BuildError> {
        if self.aliases.iter().any(|(existing, _)| existing == &alias) {
            return Err(BuildError::DuplicateAlias(alias));
        }
        if !self.entries.iter().any(|entry| entry.id() == &target) {
            return Err(BuildError::UnknownAliasTarget(target));
        }
        self.aliases.push((alias, target));
        Ok(self)
    }

    pub fn undefined_key(&mut self, key: EntryId) -> Result<&mut Self, BuildError> {
        if self.undefined_keys.contains(&key) {
            return Err(BuildError::DuplicateUndefinedKey(key));
        }
        self.undefined_keys.push(key);
        Ok(self)
    }

    #[must_use]
    pub fn freeze(self) -> ProcessedSection {
        ProcessedSection {
            id: self.id,
            entries: self.entries.into(),
            lists: self.lists.into(),
            aliases: self.aliases.into(),
            undefined_keys: self.undefined_keys.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibConfiguration {
    version: CompatibilityVersion,
    options: ScopedOptions,
}

impl BibConfiguration {
    #[must_use]
    pub const fn version(&self) -> CompatibilityVersion {
        self.version
    }
    #[must_use]
    pub const fn options(&self) -> &ScopedOptions {
        &self.options
    }
}

#[derive(Clone, Debug)]
pub struct BibConfigurationBuilder {
    version: CompatibilityVersion,
    options: ScopedOptions,
}

impl BibConfigurationBuilder {
    #[must_use]
    pub fn new(version: CompatibilityVersion) -> Self {
        Self {
            version,
            options: ScopedOptions::default(),
        }
    }
    pub fn options(&mut self, options: ScopedOptions) -> &mut Self {
        self.options = options;
        self
    }
    #[must_use]
    pub fn freeze(self) -> BibConfiguration {
        BibConfiguration {
            version: self.version,
            options: self.options,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessedBibliography {
    configuration: BibConfiguration,
    sections: Arc<[ProcessedSection]>,
}

impl ProcessedBibliography {
    #[must_use]
    pub const fn configuration(&self) -> &BibConfiguration {
        &self.configuration
    }
    pub fn sections(&self) -> impl ExactSizeIterator<Item = &ProcessedSection> {
        self.sections.iter()
    }
    #[must_use]
    pub fn section(&self, id: SectionId) -> Option<&ProcessedSection> {
        self.sections.iter().find(|section| section.id == id)
    }
}

#[derive(Clone, Debug)]
pub struct ProcessedBibliographyBuilder {
    configuration: BibConfiguration,
    sections: Vec<ProcessedSection>,
}

impl ProcessedBibliographyBuilder {
    #[must_use]
    pub fn new(configuration: BibConfiguration) -> Self {
        Self {
            configuration,
            sections: Vec::new(),
        }
    }
    pub fn section(&mut self, section: ProcessedSection) -> Result<&mut Self, BuildError> {
        if self
            .sections
            .iter()
            .any(|existing| existing.id == section.id)
        {
            return Err(BuildError::DuplicateSection(section.id));
        }
        self.sections.push(section);
        Ok(self)
    }
    #[must_use]
    pub fn freeze(self) -> ProcessedBibliography {
        ProcessedBibliography {
            configuration: self.configuration,
            sections: self.sections.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputFormat {
    Bbl,
    Bibtex,
    BibLatexXml,
    BblXml,
    Dot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputRequest {
    path: VirtualPath,
    format: OutputFormat,
    encoding: LegacyEncoding,
    newline: OutputNewline,
    max_bytes: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OutputNewline {
    #[default]
    Lf,
    CrLf,
}

impl OutputRequest {
    #[must_use]
    pub const fn new(path: VirtualPath, format: OutputFormat) -> Self {
        Self {
            path,
            format,
            encoding: LegacyEncoding::Utf8,
            newline: OutputNewline::Lf,
            max_bytes: 64 * 1024 * 1024,
        }
    }
    #[must_use]
    pub const fn with_encoding(mut self, encoding: LegacyEncoding) -> Self {
        self.encoding = encoding;
        self
    }
    #[must_use]
    pub const fn with_newline(mut self, newline: OutputNewline) -> Self {
        self.newline = newline;
        self
    }
    #[must_use]
    pub const fn with_max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = max_bytes;
        self
    }
    #[must_use]
    pub const fn path(&self) -> &VirtualPath {
        &self.path
    }
    #[must_use]
    pub const fn format(&self) -> OutputFormat {
        self.format
    }
    #[must_use]
    pub const fn encoding(&self) -> LegacyEncoding {
        self.encoding
    }
    #[must_use]
    pub const fn newline(&self) -> OutputNewline {
        self.newline
    }
    #[must_use]
    pub const fn max_bytes(&self) -> usize {
        self.max_bytes
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedFile {
    path: VirtualPath,
    bytes: Arc<[u8]>,
}

impl GeneratedFile {
    #[must_use]
    pub fn new(path: VirtualPath, bytes: impl Into<Arc<[u8]>>) -> Self {
        Self {
            path,
            bytes: bytes.into(),
        }
    }
    #[must_use]
    pub const fn path(&self) -> &VirtualPath {
        &self.path
    }
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BuildError {
    Invalid(&'static str),
    DuplicateField(FieldId),
    DuplicateAnnotation(FieldId),
    DuplicateEntry(EntryId),
    DuplicateListEntry(EntryId),
    UnknownListEntry(EntryId),
    DuplicateList(DataListId),
    DuplicateSection(SectionId),
    DuplicateAlias(EntryId),
    UnknownAliasTarget(EntryId),
    DuplicateUndefinedKey(EntryId),
    DuplicateContextField(FieldId),
}

impl fmt::Display for BuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for BuildError {}
