use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ExtraScope {
    Date,
    Title,
    TitleYear,
    Alpha,
}

#[derive(Clone, Debug)]
pub struct ExtraField<'a> {
    pub entry: &'a str,
    pub scope: ExtraScope,
    pub identity: Option<&'a str>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExtraValues(BTreeMap<(String, ExtraScope), usize>);

impl ExtraValues {
    #[must_use]
    pub fn get(&self, entry: &str, scope: ExtraScope) -> Option<usize> {
        self.0.get(&(entry.to_owned(), scope)).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, ExtraScope, usize)> {
        self.0
            .iter()
            .map(|((entry, scope), value)| (entry.as_str(), *scope, *value))
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ExtraFieldProcessor;

impl ExtraFieldProcessor {
    /// Assigns one-based values only to colliding identities. Input order is
    /// the observable data-list order and is preserved within each group.
    #[must_use]
    pub fn process(fields: &[ExtraField<'_>]) -> ExtraValues {
        let mut groups: BTreeMap<(ExtraScope, &str), Vec<&str>> = BTreeMap::new();
        for field in fields {
            if let Some(identity) = field.identity {
                groups
                    .entry((field.scope, identity))
                    .or_default()
                    .push(field.entry);
            }
        }
        let mut values = BTreeMap::new();
        for ((scope, _), entries) in groups {
            if entries.len() > 1 {
                for (index, entry) in entries.into_iter().enumerate() {
                    values.insert((entry.to_owned(), scope), index + 1);
                }
            }
        }
        ExtraValues(values)
    }
}
