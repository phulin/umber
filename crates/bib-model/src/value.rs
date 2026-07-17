use std::sync::Arc;

use crate::{FieldId, FieldProvenance};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Literal(String);

impl Literal {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Verbatim(String);

impl Verbatim {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamePartValue {
    value: Literal,
    initials: Arc<[String]>,
    outer_braces_stripped: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NamePartKind {
    Family,
    Given,
    Prefix,
    Suffix,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NameAssignment {
    key: Arc<str>,
    value: Arc<str>,
}

impl NameAssignment {
    #[must_use]
    pub fn new(key: impl Into<Arc<str>>, value: impl Into<Arc<str>>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }

    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

impl NamePartValue {
    #[must_use]
    pub fn new(
        value: Literal,
        initials: impl IntoIterator<Item = String>,
        outer_braces_stripped: bool,
    ) -> Self {
        Self {
            value,
            initials: initials.into_iter().collect(),
            outer_braces_stripped,
        }
    }

    #[must_use]
    pub const fn value(&self) -> &Literal {
        &self.value
    }

    pub fn initials(&self) -> impl ExactSizeIterator<Item = &str> {
        self.initials.iter().map(String::as_str)
    }

    #[must_use]
    pub const fn outer_braces_stripped(&self) -> bool {
        self.outer_braces_stripped
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Name {
    family: Option<NamePartValue>,
    given: Option<NamePartValue>,
    prefix: Option<NamePartValue>,
    suffix: Option<NamePartValue>,
    source: Option<Arc<str>>,
    assignments: Arc<[NameAssignment]>,
    hash_id: Option<Arc<str>>,
    use_prefix: Option<bool>,
    sorting_name_key_template: Option<Arc<str>>,
}

impl Name {
    #[must_use]
    pub const fn family(&self) -> Option<&NamePartValue> {
        self.family.as_ref()
    }
    #[must_use]
    pub const fn given(&self) -> Option<&NamePartValue> {
        self.given.as_ref()
    }
    #[must_use]
    pub const fn prefix(&self) -> Option<&NamePartValue> {
        self.prefix.as_ref()
    }
    #[must_use]
    pub const fn suffix(&self) -> Option<&NamePartValue> {
        self.suffix.as_ref()
    }
    #[must_use]
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }
    pub fn assignments(&self) -> impl ExactSizeIterator<Item = &NameAssignment> {
        self.assignments.iter()
    }
    #[must_use]
    pub fn hash_id(&self) -> Option<&str> {
        self.hash_id.as_deref()
    }
    #[must_use]
    pub const fn use_prefix(&self) -> Option<bool> {
        self.use_prefix
    }
    #[must_use]
    pub fn sorting_name_key_template(&self) -> Option<&str> {
        self.sorting_name_key_template.as_deref()
    }

    #[must_use]
    pub fn to_bibtex(&self) -> String {
        let mut value = String::new();
        if let Some(prefix) = &self.prefix {
            push_part(&mut value, prefix);
            value.push(' ');
        }
        if let Some(family) = &self.family {
            push_part(&mut value, family);
        }
        if let Some(suffix) = &self.suffix {
            value.push_str(", ");
            push_part(&mut value, suffix);
        }
        if let Some(given) = &self.given {
            value.push_str(", ");
            push_part(&mut value, given);
        }
        value
    }
}

fn push_part(output: &mut String, part: &NamePartValue) {
    if part.outer_braces_stripped {
        output.push('{');
    }
    output.push_str(&part.value.as_str().replace('~', " "));
    if part.outer_braces_stripped {
        output.push('}');
    }
}

#[derive(Clone, Debug, Default)]
pub struct NameBuilder {
    family: Option<NamePartValue>,
    given: Option<NamePartValue>,
    prefix: Option<NamePartValue>,
    suffix: Option<NamePartValue>,
    source: Option<Arc<str>>,
    assignments: Vec<NameAssignment>,
    hash_id: Option<Arc<str>>,
    use_prefix: Option<bool>,
    sorting_name_key_template: Option<Arc<str>>,
}

impl NameBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    pub fn family(&mut self, value: Literal) -> &mut Self {
        self.family = Some(NamePartValue::new(value, [], false));
        self
    }
    pub fn given(&mut self, value: Literal) -> &mut Self {
        self.given = Some(NamePartValue::new(value, [], false));
        self
    }
    pub fn prefix(&mut self, value: Literal) -> &mut Self {
        self.prefix = Some(NamePartValue::new(value, [], false));
        self
    }
    pub fn suffix(&mut self, value: Literal) -> &mut Self {
        self.suffix = Some(NamePartValue::new(value, [], false));
        self
    }
    pub fn family_part(&mut self, value: NamePartValue) -> &mut Self {
        self.family = Some(value);
        self
    }
    pub fn given_part(&mut self, value: NamePartValue) -> &mut Self {
        self.given = Some(value);
        self
    }
    pub fn prefix_part(&mut self, value: NamePartValue) -> &mut Self {
        self.prefix = Some(value);
        self
    }
    pub fn suffix_part(&mut self, value: NamePartValue) -> &mut Self {
        self.suffix = Some(value);
        self
    }
    pub fn source(&mut self, value: impl Into<Arc<str>>) -> &mut Self {
        self.source = Some(value.into());
        self
    }
    pub fn assignment(&mut self, value: NameAssignment) -> &mut Self {
        self.assignments.push(value);
        self
    }
    pub fn hash_id(&mut self, value: impl Into<Arc<str>>) -> &mut Self {
        self.hash_id = Some(value.into());
        self
    }
    pub fn use_prefix(&mut self, value: bool) -> &mut Self {
        self.use_prefix = Some(value);
        self
    }
    pub fn sorting_name_key_template(&mut self, value: impl Into<Arc<str>>) -> &mut Self {
        self.sorting_name_key_template = Some(value.into());
        self
    }
    pub fn freeze(self) -> Result<Name, &'static str> {
        if self.family.is_none()
            && self.given.is_none()
            && self.prefix.is_none()
            && self.suffix.is_none()
        {
            return Err("a name must contain at least one part");
        }
        Ok(Name {
            family: self.family,
            given: self.given,
            prefix: self.prefix,
            suffix: self.suffix,
            source: self.source,
            assignments: self.assignments.into(),
            hash_id: self.hash_id,
            use_prefix: self.use_prefix,
            sorting_name_key_template: self.sorting_name_key_template,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Uri(String);

impl Uri {
    pub fn new(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        if value.is_empty() || value.chars().any(char::is_control) {
            return Err("URI values must be nonempty and contain no control characters");
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RangeEndpoint {
    Integer(i64),
    Literal(Literal),
    Open,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Range {
    start: RangeEndpoint,
    end: RangeEndpoint,
}

impl Range {
    #[must_use]
    pub const fn new(start: RangeEndpoint, end: RangeEndpoint) -> Self {
        Self { start, end }
    }
    #[must_use]
    pub const fn start(&self) -> &RangeEndpoint {
        &self.start
    }
    #[must_use]
    pub const fn end(&self) -> &RangeEndpoint {
        &self.end
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DateValue {
    year: i32,
    month: Option<u8>,
    day: Option<u8>,
    uncertain: bool,
    approximate: bool,
}

impl DateValue {
    pub fn new(year: i32, month: Option<u8>, day: Option<u8>) -> Result<Self, &'static str> {
        if !month.is_none_or(|value| (1..=12).contains(&value)) {
            return Err("month must be in 1..=12");
        }
        if day.is_some() && month.is_none() {
            return Err("a day requires a month");
        }
        if !day.is_none_or(|value| (1..=31).contains(&value)) {
            return Err("day must be in 1..=31");
        }
        Ok(Self {
            year,
            month,
            day,
            uncertain: false,
            approximate: false,
        })
    }

    #[must_use]
    pub const fn year(&self) -> i32 {
        self.year
    }
    #[must_use]
    pub const fn month(&self) -> Option<u8> {
        self.month
    }
    #[must_use]
    pub const fn day(&self) -> Option<u8> {
        self.day
    }
    #[must_use]
    pub const fn is_uncertain(&self) -> bool {
        self.uncertain
    }
    #[must_use]
    pub const fn is_approximate(&self) -> bool {
        self.approximate
    }
    #[must_use]
    pub const fn with_uncertain(mut self, uncertain: bool) -> Self {
        self.uncertain = uncertain;
        self
    }
    #[must_use]
    pub const fn with_approximate(mut self, approximate: bool) -> Self {
        self.approximate = approximate;
        self
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NameList {
    names: Arc<[Name]>,
    has_others: bool,
}

impl NameList {
    #[must_use]
    pub fn new(names: impl IntoIterator<Item = Name>, has_others: bool) -> Self {
        Self {
            names: names.into_iter().collect(),
            has_others,
        }
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &Name> {
        self.names.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.names.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    #[must_use]
    pub const fn has_others(&self) -> bool {
        self.has_others
    }
}
pub type LiteralList = Vec<Literal>;
pub type UriList = Vec<Uri>;
pub type RangeList = Vec<Range>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FieldValue {
    Literal(Literal),
    Verbatim(Verbatim),
    Integer(i64),
    NameList(NameList),
    LiteralList(LiteralList),
    KeyList(Vec<crate::EntryId>),
    UriList(UriList),
    RangeList(RangeList),
    Date(DateValue),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FieldValueStage {
    RawDecoded,
    Normalized,
    Derived,
    Computed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Field {
    id: FieldId,
    value: FieldValue,
    stage: FieldValueStage,
    provenance: FieldProvenance,
}

impl Field {
    #[must_use]
    pub const fn new(
        id: FieldId,
        value: FieldValue,
        stage: FieldValueStage,
        provenance: FieldProvenance,
    ) -> Self {
        Self {
            id,
            value,
            stage,
            provenance,
        }
    }

    #[must_use]
    pub const fn id(&self) -> &FieldId {
        &self.id
    }

    #[must_use]
    pub const fn value(&self) -> &FieldValue {
        &self.value
    }

    #[must_use]
    pub const fn stage(&self) -> FieldValueStage {
        self.stage
    }

    #[must_use]
    pub const fn provenance(&self) -> &FieldProvenance {
        &self.provenance
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FieldMap(Arc<[Field]>);

impl FieldMap {
    pub(crate) fn from_fields(fields: Vec<Field>) -> Self {
        Self(fields.into())
    }

    #[must_use]
    pub fn get(&self, id: &FieldId) -> Option<&FieldValue> {
        self.0
            .iter()
            .find(|field| field.id() == id)
            .map(Field::value)
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &Field> {
        self.0.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
