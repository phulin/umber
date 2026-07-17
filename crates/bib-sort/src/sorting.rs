use std::cmp::Ordering;
use std::fmt;

use bib_model::{
    DataList, DataListId, Entry, EntryId, FieldId, FieldValue, Literal, Name, OptionId,
    OptionValue, ProcessedSection,
};
use bib_unicode::{CollationData, compatibility_hash};

const DEFAULT_MAX_ENTRIES: usize = 1_000_000;
const DEFAULT_MAX_COMPONENTS: usize = 64;
const DEFAULT_MAX_KEY_CHARS: usize = 1_048_576;
const NAME_SEPARATOR: char = '\u{10fffd}';

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntryDisposition {
    Normal,
    SkipBibliography,
    SkipLabels,
    DataOnly,
}

impl EntryDisposition {
    #[must_use]
    pub fn from_entry(entry: &Entry) -> Self {
        if option_is_true(entry, "dataonly") {
            Self::DataOnly
        } else if option_is_true(entry, "skipbib") {
            Self::SkipBibliography
        } else if option_is_true(entry, "skiplab") {
            Self::SkipLabels
        } else {
            Self::Normal
        }
    }

    #[must_use]
    pub const fn appears_in_list(self) -> bool {
        !matches!(self, Self::SkipBibliography | Self::DataOnly)
    }

    #[must_use]
    pub const fn computes_labels(self) -> bool {
        !matches!(self, Self::SkipLabels | Self::DataOnly)
    }
}

fn option_is_true(entry: &Entry, name: &str) -> bool {
    OptionId::new(name).ok().is_some_and(|id| {
        matches!(
            entry.options().resolve(&id),
            Some(OptionValue::Boolean(true))
        )
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataListFilter {
    All,
    EntryType(String),
    HasField(FieldId),
    FieldEquals(FieldId, String),
    EntryIds(Vec<EntryId>),
    Not(Box<Self>),
    And(Vec<Self>),
    Or(Vec<Self>),
}

impl DataListFilter {
    #[must_use]
    pub fn matches(&self, entry: &Entry) -> bool {
        match self {
            Self::All => true,
            Self::EntryType(value) => entry.entry_type().as_str() == value,
            Self::HasField(field) => entry.fields().get(field).is_some(),
            Self::FieldEquals(field, value) => entry
                .fields()
                .get(field)
                .and_then(field_text)
                .is_some_and(|actual| actual == *value),
            Self::EntryIds(ids) => ids.contains(entry.id()),
            Self::Not(filter) => !filter.matches(entry),
            Self::And(filters) => filters.iter().all(|filter| filter.matches(entry)),
            Self::Or(filters) => filters.iter().any(|filter| filter.matches(entry)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DataListLimits {
    pub maximum_entries: usize,
    pub maximum_sort_components: usize,
    pub maximum_key_chars: usize,
}

impl Default for DataListLimits {
    fn default() -> Self {
        Self {
            maximum_entries: DEFAULT_MAX_ENTRIES,
            maximum_sort_components: DEFAULT_MAX_COMPONENTS,
            maximum_key_chars: DEFAULT_MAX_KEY_CHARS,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Locale {
    Root,
    Swedish,
    Spanish,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaseOrder {
    Folded,
    UpperFirst,
    LowerFirst,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MissingOrder {
    First,
    Last,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PadDirection {
    Left,
    Right,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SortField {
    Field(FieldId),
    EntryId,
    EntryType,
    CiteOrder,
    Constant(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SortOptions {
    pub direction: SortDirection,
    pub missing: MissingOrder,
    pub final_value: bool,
    pub numeric: bool,
    pub pad_width: Option<usize>,
    pub pad_direction: PadDirection,
    pub pad_char: char,
    pub substring: Option<(usize, usize)>,
    pub locale: Locale,
    pub case_order: CaseOrder,
}

impl Default for SortOptions {
    fn default() -> Self {
        Self {
            direction: SortDirection::Ascending,
            missing: MissingOrder::Last,
            final_value: false,
            numeric: false,
            pad_width: None,
            pad_direction: PadDirection::Left,
            pad_char: '0',
            substring: None,
            locale: Locale::Root,
            case_order: CaseOrder::Folded,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SortComponent {
    pub field: SortField,
    pub options: SortOptions,
}

impl SortComponent {
    #[must_use]
    pub fn ascending(field: SortField) -> Self {
        Self {
            field,
            options: SortOptions::default(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SortTemplate {
    components: Vec<SortComponent>,
}

impl SortTemplate {
    pub fn new(components: impl IntoIterator<Item = SortComponent>) -> Result<Self, SortError> {
        let components = components.into_iter().collect::<Vec<_>>();
        if components.is_empty() {
            return Err(SortError::EmptyTemplate);
        }
        Ok(Self { components })
    }

    pub fn components(&self) -> impl ExactSizeIterator<Item = &SortComponent> {
        self.components.iter()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SortedEntry {
    pub id: EntryId,
    pub keys: Vec<Option<String>>,
}

pub struct DataListBuilder<'a> {
    section: &'a ProcessedSection,
    id: DataListId,
    filter: DataListFilter,
    template: SortTemplate,
    limits: DataListLimits,
    include_skipped: bool,
}

impl<'a> DataListBuilder<'a> {
    #[must_use]
    pub fn new(section: &'a ProcessedSection, id: DataListId, template: SortTemplate) -> Self {
        Self {
            section,
            id,
            filter: DataListFilter::All,
            template,
            limits: DataListLimits::default(),
            include_skipped: false,
        }
    }

    #[must_use]
    pub fn filter(mut self, filter: DataListFilter) -> Self {
        self.filter = filter;
        self
    }

    #[must_use]
    pub fn limits(mut self, limits: DataListLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Includes `skipbib` and `dataonly` entries for internal data lists.
    #[must_use]
    pub fn include_skipped(mut self, include: bool) -> Self {
        self.include_skipped = include;
        self
    }

    pub fn build(self) -> Result<DataList, SortError> {
        let sorted = self.sorted_entries()?;
        DataList::new(self.id, sorted.into_iter().map(|entry| entry.id))
            .map_err(|_| SortError::DuplicateEntry)
    }

    pub fn sorted_entries(&self) -> Result<Vec<SortedEntry>, SortError> {
        if self.template.components.len() > self.limits.maximum_sort_components {
            return Err(SortError::TooManyComponents);
        }
        let entries = self
            .section
            .entries()
            .filter(|entry| {
                self.filter.matches(entry)
                    && (self.include_skipped
                        || EntryDisposition::from_entry(entry).appears_in_list())
            })
            .collect::<Vec<_>>();
        if entries.len() > self.limits.maximum_entries {
            return Err(SortError::TooManyEntries);
        }
        let mut keyed = entries
            .into_iter()
            .enumerate()
            .map(|(source_order, entry)| {
                let keys = self
                    .template
                    .components
                    .iter()
                    .map(|component| component_value(entry, component, source_order))
                    .collect::<Vec<_>>();
                let chars: usize = keys.iter().flatten().map(|key| key.chars().count()).sum();
                if chars > self.limits.maximum_key_chars {
                    return Err(SortError::KeyTooLong);
                }
                Ok((source_order, entry, keys))
            })
            .collect::<Result<Vec<_>, _>>()?;
        keyed.sort_by(|left, right| {
            for (index, component) in self.template.components.iter().enumerate() {
                let order = compare_component(&left.2[index], &right.2[index], &component.options);
                if order != Ordering::Equal {
                    return order;
                }
                // A present final value terminates the template. This is used
                // for sentinel/fallback values whose equality must not expose
                // later missing fields; source order remains the stable tie.
                if component.options.final_value
                    && left.2[index].is_some()
                    && right.2[index].is_some()
                {
                    return left.0.cmp(&right.0);
                }
            }
            left.0.cmp(&right.0)
        });
        Ok(keyed
            .into_iter()
            .map(|(_, entry, keys)| SortedEntry {
                id: entry.id().clone(),
                keys,
            })
            .collect())
    }
}

fn component_value(entry: &Entry, component: &SortComponent, cite_order: usize) -> Option<String> {
    let value = match &component.field {
        SortField::Field(field) => entry.fields().get(field).and_then(field_text),
        SortField::EntryId => Some(entry.id().as_str().to_owned()),
        SortField::EntryType => Some(entry.entry_type().as_str().to_owned()),
        SortField::CiteOrder => Some(cite_order.to_string()),
        SortField::Constant(value) => Some(value.clone()),
    }?;
    let value = if let Some((offset, length)) = component.options.substring {
        value.chars().skip(offset).take(length).collect()
    } else {
        value
    };
    Some(if let Some(width) = component.options.pad_width {
        pad(
            &value,
            width,
            component.options.pad_direction,
            component.options.pad_char,
        )
    } else {
        value
    })
}

fn field_text(value: &FieldValue) -> Option<String> {
    match value {
        FieldValue::Literal(value) => Some(value.as_str().to_owned()),
        FieldValue::Verbatim(value) => Some(value.as_str().to_owned()),
        FieldValue::Integer(value) => Some(value.to_string()),
        FieldValue::LiteralList(values) => Some(
            values
                .iter()
                .map(Literal::as_str)
                .collect::<Vec<_>>()
                .join(&NAME_SEPARATOR.to_string()),
        ),
        FieldValue::NameList(names) => Some(
            names
                .iter()
                .map(|name| name_sort_key(name, &NameKeyTemplate::default()))
                .collect::<Vec<_>>()
                .join(&NAME_SEPARATOR.to_string()),
        ),
        _ => None,
    }
}

fn compare_component(
    left: &Option<String>,
    right: &Option<String>,
    options: &SortOptions,
) -> Ordering {
    let order = match (left, right) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => match options.missing {
            MissingOrder::First => Ordering::Less,
            MissingOrder::Last => Ordering::Greater,
        },
        (Some(_), None) => match options.missing {
            MissingOrder::First => Ordering::Greater,
            MissingOrder::Last => Ordering::Less,
        },
        (Some(left), Some(right)) => {
            if options.numeric {
                match (left.parse::<i128>(), right.parse::<i128>()) {
                    (Ok(left), Ok(right)) => left.cmp(&right),
                    _ => collation_key(left, options).cmp(&collation_key(right, options)),
                }
            } else {
                collation_key(left, options).cmp(&collation_key(right, options))
            }
        }
    };
    if matches!(options.direction, SortDirection::Descending) {
        order.reverse()
    } else {
        order
    }
}

fn collation_key(value: &str, options: &SortOptions) -> (Vec<u32>, Vec<u8>) {
    let root = CollationData.root_key(value);
    let mut primary = root.weights().to_vec();
    if !matches!(options.locale, Locale::Root) {
        primary = locale_weights(value, options.locale);
    }
    let case = value
        .chars()
        .map(|character| match options.case_order {
            CaseOrder::Folded => 0,
            CaseOrder::UpperFirst => u8::from(character.is_lowercase()),
            CaseOrder::LowerFirst => u8::from(character.is_uppercase()),
        })
        .collect();
    (primary, case)
}

fn locale_weights(value: &str, locale: Locale) -> Vec<u32> {
    let mut weights = Vec::new();
    for character in value.chars().flat_map(char::to_lowercase) {
        match (locale, character) {
            (Locale::Swedish, 'å') => weights.push(('z' as u32) + 1),
            (Locale::Swedish, 'ä') => weights.push(('z' as u32) + 2),
            (Locale::Swedish, 'ö') => weights.push(('z' as u32) + 3),
            (Locale::Spanish, 'ñ') => {
                weights.extend(['n' as u32, u32::MAX / 2]);
            }
            _ => weights.extend(CollationData.root_key(&character.to_string()).weights()),
        }
    }
    weights
}

fn pad(value: &str, width: usize, direction: PadDirection, pad_char: char) -> String {
    let missing = width.saturating_sub(value.chars().count());
    let padding = std::iter::repeat_n(pad_char, missing).collect::<String>();
    match direction {
        PadDirection::Left => format!("{padding}{value}"),
        PadDirection::Right => format!("{value}{padding}"),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NameKeyPart {
    Prefix,
    Family,
    Given,
    Suffix,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NameKeyTemplate {
    pub parts: Vec<NameKeyPart>,
    pub use_prefix: bool,
    pub initials_only: bool,
}

impl Default for NameKeyTemplate {
    fn default() -> Self {
        Self {
            parts: vec![
                NameKeyPart::Prefix,
                NameKeyPart::Family,
                NameKeyPart::Given,
                NameKeyPart::Suffix,
            ],
            use_prefix: true,
            initials_only: false,
        }
    }
}

#[must_use]
pub fn name_sort_key(name: &Name, template: &NameKeyTemplate) -> String {
    template
        .parts
        .iter()
        .filter_map(|part| {
            let value = match part {
                NameKeyPart::Prefix if !template.use_prefix || name.use_prefix() == Some(false) => {
                    return None;
                }
                NameKeyPart::Prefix => name.prefix(),
                NameKeyPart::Family => name.family(),
                NameKeyPart::Given => name.given(),
                NameKeyPart::Suffix => name.suffix(),
            }?;
            if template.initials_only {
                let initials = value.initials().collect::<String>();
                Some(if initials.is_empty() {
                    value.value().as_str().chars().next().into_iter().collect()
                } else {
                    initials
                })
            } else {
                Some(value.value().as_str().to_owned())
            }
        })
        .collect::<Vec<_>>()
        .join(&NAME_SEPARATOR.to_string())
}

#[must_use]
pub fn list_initial(value: &str) -> String {
    value
        .chars()
        .next()
        .map(char::to_uppercase)
        .into_iter()
        .flatten()
        .collect()
}

#[must_use]
pub fn list_initial_hash(value: &str) -> String {
    compatibility_hash(&list_initial(value))
}

#[must_use]
pub fn limit_literal_list(
    values: &[Literal],
    maximum: usize,
    minimum: usize,
) -> (Vec<Literal>, bool) {
    if values.len() <= maximum {
        (values.to_vec(), false)
    } else {
        (values[..values.len().min(minimum)].to_vec(), true)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SortError {
    EmptyTemplate,
    TooManyEntries,
    TooManyComponents,
    KeyTooLong,
    DuplicateEntry,
}

impl fmt::Display for SortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyTemplate => "sort template must have at least one component",
            Self::TooManyEntries => "data list exceeds the configured entry limit",
            Self::TooManyComponents => "sort template exceeds the configured component limit",
            Self::KeyTooLong => "sort key exceeds the configured character limit",
            Self::DuplicateEntry => "data list contains a duplicate entry",
        })
    }
}

impl std::error::Error for SortError {}

#[cfg(test)]
mod tests;
