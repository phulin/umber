use bib_model::{Entry, EntryType, FieldId, FieldValue};

#[derive(Clone, Debug, Default)]
pub struct DataModel {
    pub rules: Vec<ValidationRule>,
}

#[derive(Clone, Debug)]
pub struct ValidationRule {
    pub entry_type: Option<EntryType>,
    pub constraint: DataConstraint,
}

#[derive(Clone, Debug)]
pub enum DataConstraint {
    Mandatory(FieldId),
    Conditional {
        if_present: FieldId,
        then_required: FieldId,
    },
    OneOf(Vec<FieldId>),
    MutuallyExclusive(Vec<FieldId>),
    AllowedType {
        field: FieldId,
        value_kind: &'static str,
    },
}

impl ValidationRule {
    pub(crate) fn violation(&self, entry: &Entry) -> Option<String> {
        if self
            .entry_type
            .as_ref()
            .is_some_and(|kind| kind != entry.entry_type())
        {
            return None;
        }
        let has = |field: &FieldId| entry.fields().get(field).is_some();
        match &self.constraint {
            DataConstraint::Mandatory(field) if !has(field) => {
                Some(format!("missing mandatory field `{field}`"))
            }
            DataConstraint::Conditional {
                if_present,
                then_required,
            } if has(if_present) && !has(then_required) => Some(format!(
                "field `{then_required}` is required when `{if_present}` is present"
            )),
            DataConstraint::OneOf(fields) if !fields.iter().any(has) => {
                Some(format!("one of [{}] is required", display_fields(fields)))
            }
            DataConstraint::MutuallyExclusive(fields)
                if fields.iter().filter(|field| has(field)).count() > 1 =>
            {
                Some(format!(
                    "fields [{}] are mutually exclusive",
                    display_fields(fields)
                ))
            }
            DataConstraint::AllowedType { field, value_kind } => entry
                .fields()
                .get(field)
                .filter(|value| kind(value) != *value_kind)
                .map(|_| format!("field `{field}` must have type `{value_kind}`")),
            _ => None,
        }
    }
}

fn display_fields(fields: &[FieldId]) -> String {
    fields
        .iter()
        .map(FieldId::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

fn kind(value: &FieldValue) -> &'static str {
    match value {
        FieldValue::Literal(_) => "literal",
        FieldValue::Verbatim(_) => "verbatim",
        FieldValue::Integer(_) => "integer",
        FieldValue::Boolean(_) => "boolean",
        FieldValue::NameList(_) => "name-list",
        FieldValue::LiteralList(_) => "literal-list",
        FieldValue::KeyList(_) => "key-list",
        FieldValue::UriList(_) => "uri-list",
        FieldValue::RangeList(_) => "range-list",
        FieldValue::Date(_) => "date",
    }
}
