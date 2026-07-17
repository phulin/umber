use bib_model::{Entry, EntryType, FieldId, FieldValue};

/// A declared sourcemap. Maps and their steps are evaluated in declaration order.
#[derive(Clone, Debug, Default)]
pub struct SourceMap {
    pub steps: Vec<SourceMapStep>,
}

#[derive(Clone, Debug)]
pub struct SourceMapStep {
    pub matches: Vec<MapMatch>,
    pub actions: Vec<MapAction>,
    pub final_step: bool,
}

#[derive(Clone, Debug)]
pub enum MapMatch {
    EntryType(EntryType),
    FieldExists(FieldId),
    FieldEquals(FieldId, String),
}

#[derive(Clone, Debug)]
pub enum MapAction {
    Set(FieldId, FieldValue),
    SetIfMissing(FieldId, FieldValue),
    Remove(FieldId),
    Rename(FieldId, FieldId),
    ChangeType(EntryType),
    AddAlias(String),
    CloneAs(String),
}

pub(crate) fn matches(entry: &Entry, predicates: &[MapMatch]) -> bool {
    predicates.iter().all(|predicate| match predicate {
        MapMatch::EntryType(kind) => entry.entry_type() == kind,
        MapMatch::FieldExists(field) => entry.fields().get(field).is_some(),
        MapMatch::FieldEquals(field, expected) => entry
            .fields()
            .get(field)
            .and_then(text)
            .is_some_and(|actual| actual == expected),
    })
}

pub(crate) fn text(value: &FieldValue) -> Option<&str> {
    match value {
        FieldValue::Literal(value) => Some(value.as_str()),
        FieldValue::Verbatim(value) => Some(value.as_str()),
        _ => None,
    }
}
