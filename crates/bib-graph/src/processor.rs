use std::collections::{BTreeMap, BTreeSet, VecDeque};

use bib_model::{
    BibConfiguration, BibDiagnostic, BibDiagnosticCode, BibSeverity, DerivedFrom,
    DiagnosticBuilder, Entry, EntryBuilder, EntryId, EntryType, Field, FieldId, FieldProvenance,
    FieldValue, FieldValueStage, SectionId, TransformationId,
};
use bib_unicode::UnicodeData;

use crate::maps::{MapAction, SourceMap, matches};
use crate::validation::DataModel;

#[derive(Clone, Copy, Debug)]
pub struct GraphContext<'a> {
    configuration: &'a BibConfiguration,
    unicode: &'a UnicodeData,
}

impl<'a> GraphContext<'a> {
    #[must_use]
    pub const fn new(configuration: &'a BibConfiguration, unicode: &'a UnicodeData) -> Self {
        Self {
            configuration,
            unicode,
        }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphLimits {
    pub max_entries: usize,
    pub max_edges: usize,
    pub max_inheritance_depth: usize,
    pub max_diagnostics: usize,
}

impl Default for GraphLimits {
    fn default() -> Self {
        Self {
            max_entries: 100_000,
            max_edges: 1_000_000,
            max_inheritance_depth: 256,
            max_diagnostics: 1_000,
        }
    }
}

#[derive(Clone, Debug)]
pub struct GraphOptions {
    pub min_crossrefs: usize,
    pub include_related: bool,
    pub inherit_xref: bool,
    pub limits: GraphLimits,
}

impl Default for GraphOptions {
    fn default() -> Self {
        Self {
            min_crossrefs: 2,
            include_related: true,
            inherit_xref: true,
            limits: GraphLimits::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SectionSpec {
    pub id: SectionId,
    pub cited: Vec<EntryId>,
    pub include_all: bool,
    pub min_crossrefs: Option<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct GraphInput {
    pub entries: Vec<Entry>,
    pub aliases: Vec<(EntryId, EntryId)>,
    pub sections: Vec<SectionSpec>,
    pub maps: Vec<SourceMap>,
    pub data_model: DataModel,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphSection {
    pub id: SectionId,
    pub entries: Vec<Entry>,
    pub original_citekeys: Vec<EntryId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphOutput {
    pub sections: Vec<GraphSection>,
    pub diagnostics: Vec<BibDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphError {
    DuplicateEntry(EntryId),
    DuplicateAlias(EntryId),
    InvalidMap(String),
    Limit(&'static str),
}

pub struct GraphProcessor<'a> {
    #[allow(dead_code)]
    context: GraphContext<'a>,
    options: GraphOptions,
}

impl<'a> GraphProcessor<'a> {
    #[must_use]
    pub const fn new(context: GraphContext<'a>, options: GraphOptions) -> Self {
        Self { context, options }
    }

    pub fn process(&self, input: GraphInput) -> Result<GraphOutput, GraphError> {
        if input.entries.len() > self.options.limits.max_entries {
            return Err(GraphError::Limit("entry limit exceeded"));
        }
        let (entries, mapped_aliases) =
            apply_maps(input.entries, &input.maps, self.options.limits.max_entries)?;
        let mut index = BTreeMap::new();
        for (position, entry) in entries.iter().enumerate() {
            if index.insert(key(entry.id()), position).is_some() {
                return Err(GraphError::DuplicateEntry(entry.id().clone()));
            }
        }
        let mut aliases = BTreeMap::new();
        for (alias, target) in input.aliases.into_iter().chain(mapped_aliases) {
            if aliases.insert(key(&alias), target).is_some() {
                return Err(GraphError::DuplicateAlias(alias));
            }
        }
        let specs = if input.sections.is_empty() {
            vec![SectionSpec {
                id: SectionId::new(0),
                cited: Vec::new(),
                include_all: true,
                min_crossrefs: None,
            }]
        } else {
            input.sections
        };
        let mut diagnostics = Vec::new();
        let mut sections = Vec::with_capacity(specs.len());
        for spec in specs {
            sections.push(self.process_section(
                &entries,
                &index,
                &aliases,
                &input.data_model,
                spec,
                &mut diagnostics,
            )?);
        }
        Ok(GraphOutput {
            sections,
            diagnostics,
        })
    }

    fn process_section(
        &self,
        entries: &[Entry],
        index: &BTreeMap<String, usize>,
        aliases: &BTreeMap<String, EntryId>,
        model: &DataModel,
        spec: SectionSpec,
        diagnostics: &mut Vec<BibDiagnostic>,
    ) -> Result<GraphSection, GraphError> {
        let original_citekeys = spec.cited.clone();
        let mut selected = BTreeSet::new();
        let mut queue = VecDeque::new();
        if spec.include_all {
            queue.extend(0..entries.len());
        }
        for cited in &spec.cited {
            if let Some(position) = resolve(cited, index, aliases) {
                queue.push_back(position);
            } else {
                push_diagnostic(
                    diagnostics,
                    self.options.limits,
                    "MISSING_ENTRY",
                    BibSeverity::Warning,
                    format!("citekey `{cited}` was not found"),
                    Some(cited),
                    None,
                )?;
            }
        }
        let mut edges = 0usize;
        while let Some(position) = queue.pop_front() {
            if !selected.insert(position) {
                continue;
            }
            let entry = &entries[position];
            for field_name in ["entryset", "related"] {
                if field_name == "related" && !self.options.include_related {
                    continue;
                }
                for dependent in keys(entry, field_name) {
                    edges = edges
                        .checked_add(1)
                        .ok_or(GraphError::Limit("edge limit exceeded"))?;
                    if edges > self.options.limits.max_edges {
                        return Err(GraphError::Limit("edge limit exceeded"));
                    }
                    if let Some(child) = resolve(dependent, index, aliases) {
                        queue.push_back(child);
                    } else {
                        push_diagnostic(
                            diagnostics,
                            self.options.limits,
                            "MISSING_DEPENDENT",
                            BibSeverity::Warning,
                            format!("entry `{}` references missing `{dependent}`", entry.id()),
                            Some(entry.id()),
                            field(field_name).ok().as_ref(),
                        )?;
                    }
                }
            }
        }
        let min_crossrefs = spec.min_crossrefs.unwrap_or(self.options.min_crossrefs);
        let mut counts = BTreeMap::<usize, usize>::new();
        for &position in &selected {
            for field_name in ["crossref", "xref"] {
                for parent in keys(&entries[position], field_name) {
                    edges += 1;
                    if edges > self.options.limits.max_edges {
                        return Err(GraphError::Limit("edge limit exceeded"));
                    }
                    if let Some(parent) = resolve(parent, index, aliases) {
                        *counts.entry(parent).or_default() += 1;
                    }
                }
            }
        }
        for (parent, count) in counts {
            if count >= min_crossrefs {
                selected.insert(parent);
            }
        }

        let mut inheritance = Inheritance {
            entries,
            index,
            aliases,
            inherit_xref: self.options.inherit_xref,
            limits: self.options.limits,
            memo: BTreeMap::new(),
            stack: Vec::new(),
            diagnostics,
        };
        let mut output = Vec::new();
        for position in 0..entries.len() {
            if selected.contains(&position) {
                let inherited = inheritance.resolve(position)?;
                for rule in &model.rules {
                    if let Some(message) = rule.violation(&inherited) {
                        push_diagnostic(
                            inheritance.diagnostics,
                            self.options.limits,
                            "DATA_MODEL",
                            BibSeverity::Warning,
                            message,
                            Some(inherited.id()),
                            None,
                        )?;
                    }
                }
                output.push(inherited);
            }
        }
        Ok(GraphSection {
            id: spec.id,
            entries: output,
            original_citekeys,
        })
    }
}

type MappedEntries = (Vec<Entry>, Vec<(EntryId, EntryId)>);

fn apply_maps(
    entries: Vec<Entry>,
    maps: &[SourceMap],
    max_entries: usize,
) -> Result<MappedEntries, GraphError> {
    let mut output = Vec::new();
    let mut aliases = Vec::new();
    for entry in entries {
        let mut editable = Editable::from(entry);
        for map in maps {
            for step in &map.steps {
                let snapshot = editable.freeze()?;
                if !matches(&snapshot, &step.matches) {
                    continue;
                }
                for action in &step.actions {
                    editable.apply(action, &mut aliases)?;
                }
                if step.final_step {
                    break;
                }
            }
        }
        let clones = std::mem::take(&mut editable.clones);
        output.push(editable.freeze()?);
        for id in clones {
            let mut clone = output.last().expect("entry was just inserted").clone();
            clone = rebuild(
                &clone,
                id,
                clone.entry_type().clone(),
                clone.fields().iter().cloned().collect(),
            )?;
            output.push(clone);
        }
        if output.len() > max_entries {
            return Err(GraphError::Limit(
                "entry limit exceeded after sourcemap cloning",
            ));
        }
    }
    Ok((output, aliases))
}

#[derive(Clone)]
struct Editable {
    id: EntryId,
    kind: EntryType,
    fields: Vec<Field>,
    options: bib_model::ScopedOptions,
    annotations: Vec<bib_model::Annotation>,
    source: bib_model::BibSourceLocation,
    clones: Vec<EntryId>,
}

impl Editable {
    fn from(entry: Entry) -> Self {
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
    fn freeze(&self) -> Result<Entry, GraphError> {
        rebuild_parts(
            &self.id,
            &self.kind,
            &self.fields,
            &self.options,
            &self.annotations,
            &self.source,
        )
    }
    fn apply(
        &mut self,
        action: &MapAction,
        aliases: &mut Vec<(EntryId, EntryId)>,
    ) -> Result<(), GraphError> {
        match action {
            MapAction::Set(id, value) => {
                self.fields.retain(|field| field.id() != id);
                self.fields.push(mapped_field(
                    id.clone(),
                    value.clone(),
                    self.source.clone(),
                )?);
            }
            MapAction::SetIfMissing(id, value)
                if !self.fields.iter().any(|field| field.id() == id) =>
            {
                self.fields.push(mapped_field(
                    id.clone(),
                    value.clone(),
                    self.source.clone(),
                )?)
            }
            MapAction::SetIfMissing(_, _) => {}
            MapAction::Remove(id) => self.fields.retain(|field| field.id() != id),
            MapAction::Rename(from, to) => {
                if let Some(position) = self.fields.iter().position(|field| field.id() == from) {
                    let old = self.fields.remove(position);
                    self.fields.push(Field::new(
                        to.clone(),
                        old.value().clone(),
                        FieldValueStage::Derived,
                        FieldProvenance::Transformed {
                            source: self.source.clone(),
                            transformation: transformation("sourcemap-rename")?,
                        },
                    ));
                }
            }
            MapAction::ChangeType(kind) => self.kind = kind.clone(),
            MapAction::AddAlias(alias) => aliases.push((entry(alias)?, self.id.clone())),
            MapAction::CloneAs(id) => self.clones.push(entry(id)?),
        }
        Ok(())
    }
}

struct Inheritance<'a> {
    entries: &'a [Entry],
    index: &'a BTreeMap<String, usize>,
    aliases: &'a BTreeMap<String, EntryId>,
    inherit_xref: bool,
    limits: GraphLimits,
    memo: BTreeMap<usize, Entry>,
    stack: Vec<usize>,
    diagnostics: &'a mut Vec<BibDiagnostic>,
}

impl Inheritance<'_> {
    fn resolve(&mut self, position: usize) -> Result<Entry, GraphError> {
        if let Some(entry) = self.memo.get(&position) {
            return Ok(entry.clone());
        }
        if self.stack.len() >= self.limits.max_inheritance_depth {
            return Err(GraphError::Limit("inheritance depth limit exceeded"));
        }
        if let Some(cycle_start) = self
            .stack
            .iter()
            .position(|candidate| *candidate == position)
        {
            let cycle = self.stack[cycle_start..]
                .iter()
                .map(|p| self.entries[*p].id().as_str())
                .chain(std::iter::once(self.entries[position].id().as_str()))
                .collect::<Vec<_>>()
                .join(" -> ");
            push_diagnostic(
                self.diagnostics,
                self.limits,
                "CIRCULAR_INHERITANCE",
                BibSeverity::Error,
                format!("circular inheritance: {cycle}"),
                Some(self.entries[position].id()),
                None,
            )?;
            return Ok(self.entries[position].clone());
        }
        self.stack.push(position);
        let child = self.entries[position].clone();
        let mut fields: Vec<Field> = child.fields().iter().cloned().collect();
        let relationship_order: &[&str] = if self.inherit_xref {
            &["xdata", "crossref", "xref"]
        } else {
            &["xdata", "crossref"]
        };
        for relationship in relationship_order {
            for parent_id in keys(&child, relationship) {
                let Some(parent_position) = resolve(parent_id, self.index, self.aliases) else {
                    push_diagnostic(
                        self.diagnostics,
                        self.limits,
                        "MISSING_PARENT",
                        BibSeverity::Warning,
                        format!("entry `{}` inherits from missing `{parent_id}`", child.id()),
                        Some(child.id()),
                        field(relationship).ok().as_ref(),
                    )?;
                    continue;
                };
                let parent = self.resolve(parent_position)?;
                for inherited in parent.fields().iter() {
                    if is_relationship(inherited.id())
                        || fields.iter().any(|own| own.id() == inherited.id())
                    {
                        continue;
                    }
                    fields.push(Field::new(
                        inherited.id().clone(),
                        inherited.value().clone(),
                        FieldValueStage::Derived,
                        FieldProvenance::Inherited {
                            source: provenance_source(inherited, &parent),
                            parent: DerivedFrom::new(parent.id().clone(), inherited.id().clone()),
                        },
                    ));
                }
            }
        }
        self.stack.pop();
        let result = rebuild(
            &child,
            child.id().clone(),
            child.entry_type().clone(),
            fields,
        )?;
        self.memo.insert(position, result.clone());
        Ok(result)
    }
}

fn rebuild(
    original: &Entry,
    id: EntryId,
    kind: EntryType,
    fields: Vec<Field>,
) -> Result<Entry, GraphError> {
    let annotations = original.annotations().cloned().collect::<Vec<_>>();
    rebuild_parts(
        &id,
        &kind,
        &fields,
        original.options(),
        &annotations,
        original.source(),
    )
}
fn rebuild_parts(
    id: &EntryId,
    kind: &EntryType,
    fields: &[Field],
    options: &bib_model::ScopedOptions,
    annotations: &[bib_model::Annotation],
    source: &bib_model::BibSourceLocation,
) -> Result<Entry, GraphError> {
    let mut builder = EntryBuilder::new(id.clone(), kind.clone(), source.clone());
    builder.options(options.clone());
    for field in fields {
        builder
            .field(
                field.id().clone(),
                field.value().clone(),
                field.stage(),
                field.provenance().clone(),
            )
            .map_err(|error| GraphError::InvalidMap(error.to_string()))?;
    }
    for annotation in annotations {
        builder
            .annotation(annotation.clone())
            .map_err(|error| GraphError::InvalidMap(error.to_string()))?;
    }
    Ok(builder.freeze())
}

fn mapped_field(
    id: FieldId,
    value: FieldValue,
    source: bib_model::BibSourceLocation,
) -> Result<Field, GraphError> {
    Ok(Field::new(
        id,
        value,
        FieldValueStage::Derived,
        FieldProvenance::Transformed {
            source,
            transformation: transformation("sourcemap-set")?,
        },
    ))
}
fn transformation(value: &str) -> Result<TransformationId, GraphError> {
    TransformationId::new(value).map_err(|error| GraphError::InvalidMap(error.to_string()))
}
fn entry(value: &str) -> Result<EntryId, GraphError> {
    EntryId::new(value).map_err(|error| GraphError::InvalidMap(error.to_string()))
}
fn field(value: &str) -> Result<FieldId, GraphError> {
    FieldId::new(value).map_err(|error| GraphError::InvalidMap(error.to_string()))
}
fn key(id: &EntryId) -> String {
    id.as_str().to_lowercase()
}
fn resolve(
    id: &EntryId,
    index: &BTreeMap<String, usize>,
    aliases: &BTreeMap<String, EntryId>,
) -> Option<usize> {
    let normalized = key(id);
    index.get(&normalized).copied().or_else(|| {
        aliases
            .get(&normalized)
            .and_then(|target| index.get(&key(target)).copied())
    })
}
fn keys<'a>(entry: &'a Entry, name: &str) -> Vec<&'a EntryId> {
    let Ok(id) = field(name) else {
        return Vec::new();
    };
    match entry.fields().get(&id) {
        Some(FieldValue::KeyList(keys)) => keys.iter().collect(),
        _ => Vec::new(),
    }
}
fn is_relationship(id: &FieldId) -> bool {
    matches!(
        id.as_str(),
        "xdata" | "crossref" | "xref" | "related" | "entryset"
    )
}
fn provenance_source(field: &Field, parent: &Entry) -> bib_model::BibSourceLocation {
    match field.provenance() {
        FieldProvenance::Datasource(source)
        | FieldProvenance::Transformed { source, .. }
        | FieldProvenance::Inherited { source, .. } => source.clone(),
        FieldProvenance::Computed { .. } => parent.source().clone(),
    }
}

fn push_diagnostic(
    diagnostics: &mut Vec<BibDiagnostic>,
    limits: GraphLimits,
    code: &str,
    severity: BibSeverity,
    message: String,
    entry: Option<&EntryId>,
    field: Option<&FieldId>,
) -> Result<(), GraphError> {
    if diagnostics.len() >= limits.max_diagnostics {
        return Err(GraphError::Limit("diagnostic limit exceeded"));
    }
    let mut builder = DiagnosticBuilder::new(
        BibDiagnosticCode::new(code).expect("static graph diagnostic code"),
        severity,
        message,
    )
    .expect("nonempty graph diagnostic");
    if let Some(entry) = entry {
        builder.entry(entry.clone());
    }
    if let Some(field) = field {
        builder.field(field.clone());
    }
    diagnostics.push(builder.freeze());
    Ok(())
}
