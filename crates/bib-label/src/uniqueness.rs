use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UniquenessOptions {
    pub uniquename: bool,
    pub uniquelist: bool,
    pub uniquetitle: bool,
    pub uniquework: bool,
}

impl Default for UniquenessOptions {
    fn default() -> Self {
        Self {
            uniquename: true,
            uniquelist: true,
            uniquetitle: true,
            uniquework: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct UniquenessEntry<'a> {
    pub entry: &'a str,
    pub name_hashes: &'a [&'a str],
    pub visible_names: usize,
    pub title: Option<&'a str>,
    pub work: Option<&'a str>,
}

/// The resolved name-list boundary in which uniqueness is evaluated.
/// Keeping it in the input makes cite/bibliography/list contexts independent
/// and prevents one pass from leaking visibility decisions into another.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VisibleNameContext<'a> {
    pub list: &'a str,
    pub names: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NameDisambiguation {
    pub visible_names: usize,
    pub given_name_level: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UniqueState {
    pub names: BTreeMap<String, NameDisambiguation>,
    pub unique_title: BTreeSet<String>,
    pub unique_work: BTreeSet<String>,
    pub unique_primary_author: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct UniquenessProcessor;

impl UniquenessProcessor {
    #[must_use]
    pub fn process(entries: &[UniquenessEntry<'_>], options: UniquenessOptions) -> UniqueState {
        let mut state = UniqueState::default();
        let title_counts = counts(entries.iter().filter_map(|entry| entry.title));
        let work_counts = counts(entries.iter().filter_map(|entry| entry.work));
        let primary_counts = counts(
            entries
                .iter()
                .filter_map(|entry| entry.name_hashes.first().copied()),
        );
        for entry in entries {
            let complete = entry.name_hashes.len();
            let visible = if options.uniquelist {
                required_visible_names(entry, entries)
            } else {
                entry.visible_names.min(complete)
            };
            let given_name_level =
                usize::from(options.uniquename && shares_visible_prefix(entry, entries, visible));
            state.names.insert(
                entry.entry.to_owned(),
                NameDisambiguation {
                    visible_names: visible,
                    given_name_level,
                },
            );
            if options.uniquetitle && entry.title.is_some_and(|title| title_counts[title] == 1) {
                state.unique_title.insert(entry.entry.to_owned());
            }
            if options.uniquework && entry.work.is_some_and(|work| work_counts[work] == 1) {
                state.unique_work.insert(entry.entry.to_owned());
            }
            if entry
                .name_hashes
                .first()
                .is_some_and(|hash| primary_counts[*hash] == 1)
            {
                state.unique_primary_author.insert(entry.entry.to_owned());
            }
        }
        state
    }
}

fn counts<'a>(values: impl Iterator<Item = &'a str>) -> BTreeMap<&'a str, usize> {
    let mut counts = BTreeMap::new();
    for value in values {
        *counts.entry(value).or_default() += 1;
    }
    counts
}

fn required_visible_names(entry: &UniquenessEntry<'_>, entries: &[UniquenessEntry<'_>]) -> usize {
    let mut visible = entry.visible_names.min(entry.name_hashes.len());
    while visible < entry.name_hashes.len() && shares_visible_prefix(entry, entries, visible) {
        visible += 1;
    }
    visible
}

fn shares_visible_prefix(
    entry: &UniquenessEntry<'_>,
    entries: &[UniquenessEntry<'_>],
    visible: usize,
) -> bool {
    entries.iter().any(|other| {
        other.entry != entry.entry
            && other.name_hashes.get(..visible) == entry.name_hashes.get(..visible)
    })
}
