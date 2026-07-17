use std::collections::BTreeMap;

use bib_model::NameList;

#[derive(Clone, Debug, Default)]
pub struct LabelEntry<'a> {
    pub names: BTreeMap<&'a str, &'a NameList>,
    pub fields: BTreeMap<&'a str, &'a str>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LabelSelection {
    pub name_source: Option<String>,
    pub date_source: Option<String>,
    pub title_source: Option<String>,
}

#[must_use]
pub fn select_labels(
    entry: &LabelEntry<'_>,
    name_candidates: &[&str],
    date_candidates: &[&str],
    title_candidates: &[&str],
) -> LabelSelection {
    LabelSelection {
        name_source: first_name(entry, name_candidates),
        date_source: first_field(entry, date_candidates),
        title_source: first_field(entry, title_candidates),
    }
}

fn first_name(entry: &LabelEntry<'_>, candidates: &[&str]) -> Option<String> {
    candidates.iter().find_map(|candidate| {
        entry
            .names
            .get(candidate)
            .filter(|names| !names.is_empty())
            .map(|_| (*candidate).to_owned())
    })
}

fn first_field(entry: &LabelEntry<'_>, candidates: &[&str]) -> Option<String> {
    candidates.iter().find_map(|candidate| {
        entry
            .fields
            .get(candidate)
            .filter(|value| !value.is_empty())
            .map(|_| (*candidate).to_owned())
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AlphaNameOptions {
    pub names: usize,
    pub name_chars: usize,
    pub final_name_chars: usize,
    pub others: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LabelAlphaComponent {
    Literal(String),
    Field { name: String, width: usize },
    Names(AlphaNameOptions),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LabelAlphaTemplate(pub Vec<LabelAlphaComponent>);

impl LabelAlphaTemplate {
    #[must_use]
    pub fn render(&self, entry: &LabelEntry<'_>, label_names: Option<&NameList>) -> String {
        let mut output = String::new();
        for component in &self.0 {
            match component {
                LabelAlphaComponent::Literal(value) => output.push_str(value),
                LabelAlphaComponent::Field { name, width } => {
                    if let Some(value) = entry.fields.get(name.as_str()) {
                        output.extend(value.chars().take(*width));
                    }
                }
                LabelAlphaComponent::Names(options) => {
                    if let Some(names) = label_names {
                        let taken = names.len().min(options.names);
                        for (index, name) in names.iter().take(taken).enumerate() {
                            let width = if index + 1 == taken {
                                options.final_name_chars
                            } else {
                                options.name_chars
                            };
                            if let Some(family) = name.family() {
                                output.extend(family.value().as_str().chars().take(width));
                            }
                        }
                        if names.has_others() || names.len() > taken {
                            output.push_str(options.others);
                        }
                    }
                }
            }
        }
        output
    }
}
