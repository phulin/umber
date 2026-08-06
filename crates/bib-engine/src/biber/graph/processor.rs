use std::collections::{BTreeMap, BTreeSet, VecDeque};

use bib_model::{
    BibDiagnostic, BibDiagnosticCode, BibSeverity, DerivedFrom, DiagnosticBuilder, EntryId, Field,
    FieldId, FieldProvenance, FieldValue, FieldValueStage, SectionId, TransformationId,
};

use super::maps::{MapAction, SourceMap, matches};
use super::validation::DataModel;
use crate::biber::DraftEntry;

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

#[derive(Clone)]
pub(crate) struct DraftSection {
    pub id: SectionId,
    pub entries: Vec<DraftEntry>,
    pub original_citekeys: Vec<EntryId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphError {
    DuplicateEntry(EntryId),
    DuplicateAlias(EntryId),
    InvalidMap(String),
    Limit(&'static str),
}

pub(crate) struct RelationshipPass {
    options: GraphOptions,
}

impl RelationshipPass {
    #[must_use]
    pub const fn new(options: GraphOptions) -> Self {
        Self { options }
    }

    pub fn process(
        &self,
        entries: Vec<DraftEntry>,
        input_aliases: Vec<(EntryId, EntryId)>,
        input_sections: Vec<SectionSpec>,
        maps: &[SourceMap],
        data_model: &DataModel,
    ) -> Result<(Vec<DraftSection>, Vec<BibDiagnostic>), GraphError> {
        if entries.len() > self.options.limits.max_entries {
            return Err(GraphError::Limit("entry limit exceeded"));
        }
        let (entries, mapped_aliases) = apply_maps(entries, maps, self.options.limits.max_entries)?;
        let mut index = BTreeMap::new();
        for (position, entry) in entries.iter().enumerate() {
            if index.insert(key(entry.id()), position).is_some() {
                return Err(GraphError::DuplicateEntry(entry.id().clone()));
            }
        }
        let mut aliases = BTreeMap::new();
        for (alias, target) in input_aliases.into_iter().chain(mapped_aliases) {
            if aliases.insert(key(&alias), target).is_some() {
                return Err(GraphError::DuplicateAlias(alias));
            }
        }
        let specs = if input_sections.is_empty() {
            vec![SectionSpec {
                id: SectionId::new(0),
                cited: Vec::new(),
                include_all: true,
                min_crossrefs: None,
            }]
        } else {
            input_sections
        };
        let mut diagnostics = Vec::new();
        let mut sections = Vec::with_capacity(specs.len());
        for spec in specs {
            sections.push(self.process_section(
                &entries,
                &index,
                &aliases,
                data_model,
                spec,
                &mut diagnostics,
            )?);
        }
        Ok((sections, diagnostics))
    }

    fn process_section(
        &self,
        entries: &[DraftEntry],
        index: &BTreeMap<String, usize>,
        aliases: &BTreeMap<String, EntryId>,
        model: &DataModel,
        spec: SectionSpec,
        diagnostics: &mut Vec<BibDiagnostic>,
    ) -> Result<DraftSection, GraphError> {
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
        Ok(DraftSection {
            id: spec.id,
            entries: output,
            original_citekeys,
        })
    }
}

type MappedEntries = (Vec<DraftEntry>, Vec<(EntryId, EntryId)>);

fn apply_maps(
    entries: Vec<DraftEntry>,
    maps: &[SourceMap],
    max_entries: usize,
) -> Result<MappedEntries, GraphError> {
    let mut output = Vec::new();
    let mut aliases = Vec::new();
    for entry in entries {
        let mut editable = entry;
        for map in maps {
            for step in &map.steps {
                if !matches(&editable, &step.matches) {
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
        let clones = editable.take_clones();
        output.push(editable);
        for id in clones {
            let clone = output.last().expect("entry was just inserted").clone_as(id);
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

impl DraftEntry {
    fn apply(
        &mut self,
        action: &MapAction,
        aliases: &mut Vec<(EntryId, EntryId)>,
    ) -> Result<(), GraphError> {
        match action {
            MapAction::Set(id, value) => {
                self.set_field(
                    id.clone(),
                    value.clone(),
                    FieldValueStage::Derived,
                    mapped_provenance(self.source().clone())?,
                );
            }
            MapAction::SetIfMissing(id, value) if self.field(id).is_none() => {
                self.set_field(
                    id.clone(),
                    value.clone(),
                    FieldValueStage::Derived,
                    mapped_provenance(self.source().clone())?,
                );
            }
            MapAction::SetIfMissing(_, _) => {}
            MapAction::Remove(id) => {
                self.remove_field(id);
            }
            MapAction::Rename(from, to) => {
                if let Some(old) = self.remove_field(from) {
                    self.push_field(Field::new(
                        to.clone(),
                        old.value().clone(),
                        FieldValueStage::Derived,
                        FieldProvenance::Transformed {
                            source: self.source().clone(),
                            transformation: transformation("sourcemap-rename")?,
                        },
                    ));
                }
            }
            MapAction::ChangeType(kind) => self.change_type(kind.clone()),
            MapAction::AddAlias(alias) => aliases.push((entry(alias)?, self.id().clone())),
            MapAction::CloneAs(id) => self.queue_clone(entry(id)?),
        }
        Ok(())
    }
}

struct Inheritance<'a> {
    entries: &'a [DraftEntry],
    index: &'a BTreeMap<String, usize>,
    aliases: &'a BTreeMap<String, EntryId>,
    inherit_xref: bool,
    limits: GraphLimits,
    memo: BTreeMap<usize, DraftEntry>,
    stack: Vec<usize>,
    diagnostics: &'a mut Vec<BibDiagnostic>,
}

impl Inheritance<'_> {
    fn resolve(&mut self, position: usize) -> Result<DraftEntry, GraphError> {
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
        let mut result = child.clone();
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
                for inherited in parent.fields() {
                    if is_relationship(inherited.id()) || result.field(inherited.id()).is_some() {
                        continue;
                    }
                    result.push_field(Field::new(
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
        self.memo.insert(position, result.clone());
        Ok(result)
    }
}
fn mapped_provenance(source: bib_model::BibSourceLocation) -> Result<FieldProvenance, GraphError> {
    Ok(FieldProvenance::Transformed {
        source,
        transformation: transformation("sourcemap-set")?,
    })
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
fn keys<'a>(entry: &'a DraftEntry, name: &str) -> Vec<&'a EntryId> {
    let Ok(id) = field(name) else {
        return Vec::new();
    };
    match entry.field(&id) {
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
fn provenance_source(field: &Field, parent: &DraftEntry) -> bib_model::BibSourceLocation {
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
