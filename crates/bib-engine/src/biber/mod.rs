//! Engine-private Biber semantic worker.

use bib_model::{
    Annotation, BibConfiguration, BibDiagnostic, BibSourceLocation, DataList, DataListId, Entry,
    EntryBuilder, EntryId, EntryType, Field, FieldId, FieldProvenance, FieldValue, FieldValueStage,
    ProcessedBibliography, ProcessedBibliographyBuilder, ProcessedSectionBuilder, ScopedOptions,
};

pub(super) mod graph;
pub(super) mod label;
pub(super) mod sort;

use graph::{DataModel, DraftSection, GraphOptions, RelationshipPass, SectionSpec};
use label::select_labels;

const DEFAULT_LIST: &str = "nty/global//global/global/global";

pub(super) struct BiberWorker {
    draft: BiberDraft,
}

/// The single engine-owned mutation root from datasource lowering through publication.
pub(super) struct BiberDraft {
    configuration: BibConfiguration,
    entries: Vec<DraftEntry>,
    section_specs: Vec<SectionSpec>,
    sections: Vec<DraftSection>,
    diagnostics: Vec<BibDiagnostic>,
}

pub(super) enum WorkerError {
    Relationship(String),
    Build(String),
}

impl WorkerError {
    pub(super) const fn code(&self) -> &'static str {
        match self {
            Self::Relationship(_) => "GRAPH",
            Self::Build(_) => "MODEL_BUILD",
        }
    }

    pub(super) fn message(self) -> String {
        match self {
            Self::Relationship(message) | Self::Build(message) => message,
        }
    }

    pub(super) const fn is_build(&self) -> bool {
        matches!(self, Self::Build(_))
    }
}

impl BiberWorker {
    pub(super) const fn new(draft: BiberDraft) -> Self {
        Self { draft }
    }

    pub(super) fn process(
        mut self,
    ) -> Result<(ProcessedBibliography, Vec<BibDiagnostic>), WorkerError> {
        self.draft.resolve_relationships()?;
        for section in &mut self.draft.sections {
            for entry in &mut section.entries {
                add_label_sources(entry);
            }
        }
        self.draft.freeze()
    }
}

impl BiberDraft {
    pub(super) fn new(
        configuration: BibConfiguration,
        entries: Vec<DraftEntry>,
        section_specs: Vec<SectionSpec>,
        diagnostics: Vec<BibDiagnostic>,
    ) -> Self {
        Self {
            configuration,
            entries,
            section_specs,
            sections: Vec::new(),
            diagnostics,
        }
    }

    fn resolve_relationships(&mut self) -> Result<(), WorkerError> {
        // Configuration-owned sourcemaps and data-model semantics remain
        // deliberately inactive until their existing option paths implement
        // them; this refactor must not change compatibility behavior.
        let (sections, diagnostics) = RelationshipPass::new(GraphOptions::default())
            .process(
                std::mem::take(&mut self.entries),
                Vec::new(),
                std::mem::take(&mut self.section_specs),
                &[],
                &DataModel::default(),
            )
            .map_err(|error| WorkerError::Relationship(format!("{error:?}")))?;
        self.sections = sections;
        self.diagnostics.extend(diagnostics);
        Ok(())
    }

    fn freeze(self) -> Result<(ProcessedBibliography, Vec<BibDiagnostic>), WorkerError> {
        let mut document = ProcessedBibliographyBuilder::new(self.configuration);
        for section in self.sections {
            let list = DataList::new(
                DataListId::new(DEFAULT_LIST).expect("fixed list id is valid"),
                section.entries.iter().map(|entry| entry.id().clone()),
            )
            .map_err(|error| WorkerError::Build(error.to_string()))?;
            let mut builder = ProcessedSectionBuilder::new(section.id);
            for entry in section.entries {
                builder
                    .entry(entry.freeze().map_err(WorkerError::Build)?)
                    .map_err(|error| WorkerError::Build(error.to_string()))?;
            }
            builder
                .list(list)
                .map_err(|error| WorkerError::Build(error.to_string()))?;
            document
                .section(builder.freeze())
                .map_err(|error| WorkerError::Build(error.to_string()))?;
        }
        Ok((document.freeze(), self.diagnostics))
    }
}

fn add_label_sources(entry: &mut DraftEntry) {
    let mut label = label::LabelEntry::default();
    for field in entry.fields() {
        match field.value() {
            FieldValue::NameList(names) => {
                label.names.insert(field.id().as_str(), names);
            }
            FieldValue::Literal(value) => {
                label.fields.insert(field.id().as_str(), value.as_str());
            }
            _ => {}
        }
    }
    let selection = select_labels(
        &label,
        &["author", "editor", "translator"],
        &["labelyear", "year", "date"],
        &["labeltitle", "title", "maintitle"],
    );
    let transformation =
        bib_model::TransformationId::new("label-source").expect("fixed transformation id is valid");
    for (name, value) in [
        ("labelnamesource", selection.name_source),
        ("labeldatesource", selection.date_source),
        ("labeltitlesource", selection.title_source),
    ] {
        let id = FieldId::new(name).expect("fixed field id is valid");
        if entry.field(&id).is_none() {
            entry.set_field(
                id,
                FieldValue::Literal(bib_model::Literal::new(value.unwrap_or_default())),
                FieldValueStage::Computed,
                FieldProvenance::Computed {
                    transformation: transformation.clone(),
                    inputs: Vec::new(),
                },
            );
        }
    }
}

/// The sole mutable typed representation used by the Biber semantic pipeline.
#[derive(Clone)]
pub(super) struct DraftEntry {
    id: EntryId,
    kind: EntryType,
    fields: Vec<Field>,
    options: ScopedOptions,
    annotations: Vec<Annotation>,
    source: BibSourceLocation,
    clones: Vec<EntryId>,
}

impl DraftEntry {
    pub(super) fn new(id: EntryId, kind: EntryType, source: BibSourceLocation) -> Self {
        Self {
            id,
            kind,
            fields: Vec::new(),
            options: ScopedOptions::default(),
            annotations: Vec::new(),
            source,
            clones: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(super) fn from_entry(entry: &Entry) -> Self {
        Self {
            id: entry.id().clone(),
            kind: entry.entry_type().clone(),
            fields: entry.fields().iter().cloned().collect(),
            options: entry.options().clone(),
            annotations: entry.annotations().cloned().collect(),
            source: entry.source().clone(),
            clones: Vec::new(),
        }
    }

    pub(super) fn id(&self) -> &EntryId {
        &self.id
    }

    pub(super) fn entry_type(&self) -> &EntryType {
        &self.kind
    }

    pub(super) fn fields(&self) -> &[Field] {
        &self.fields
    }

    pub(super) fn field(&self, id: &FieldId) -> Option<&FieldValue> {
        self.fields
            .iter()
            .find(|field| field.id() == id)
            .map(Field::value)
    }

    pub(super) fn source(&self) -> &BibSourceLocation {
        &self.source
    }

    pub(super) fn set_field(
        &mut self,
        id: FieldId,
        value: FieldValue,
        stage: FieldValueStage,
        provenance: FieldProvenance,
    ) {
        self.fields.retain(|field| field.id() != &id);
        self.fields.push(Field::new(id, value, stage, provenance));
    }

    pub(super) fn push_field(&mut self, field: Field) {
        self.fields.push(field);
    }

    pub(super) fn remove_field(&mut self, id: &FieldId) -> Option<Field> {
        self.fields
            .iter()
            .position(|field| field.id() == id)
            .map(|position| self.fields.remove(position))
    }

    pub(super) fn change_type(&mut self, kind: EntryType) {
        self.kind = kind;
    }

    pub(super) fn clone_as(&self, id: EntryId) -> Self {
        let mut clone = self.clone();
        clone.id = id;
        clone.clones.clear();
        clone
    }

    pub(super) fn queue_clone(&mut self, id: EntryId) {
        self.clones.push(id);
    }

    pub(super) fn take_clones(&mut self) -> Vec<EntryId> {
        std::mem::take(&mut self.clones)
    }

    pub(super) fn freeze(self) -> Result<Entry, String> {
        let mut builder = EntryBuilder::new(self.id, self.kind, self.source);
        builder.options(self.options);
        for field in self.fields {
            builder
                .field(
                    field.id().clone(),
                    field.value().clone(),
                    field.stage(),
                    field.provenance().clone(),
                )
                .map_err(|error| error.to_string())?;
        }
        for annotation in self.annotations {
            builder
                .annotation(annotation)
                .map_err(|error| error.to_string())?;
        }
        Ok(builder.freeze())
    }
}
