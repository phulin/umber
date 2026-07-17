use bib_model::{Literal, Name, NameAssignment, NameBuilder, NamePartValue};
use bib_unicode::{normalise_nfc, remove_outer};

use crate::names::{initials, join_name_parts, split_words};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExtendedNameLimits {
    pub max_record_bytes: usize,
    pub max_fields: usize,
    pub max_field_bytes: usize,
    pub max_nesting: usize,
    pub max_work: usize,
    pub max_diagnostics: usize,
}

impl Default for ExtendedNameLimits {
    fn default() -> Self {
        Self {
            max_record_bytes: 1024 * 1024,
            max_fields: 256,
            max_field_bytes: 1024 * 1024,
            max_nesting: 256,
            max_work: 8 * 1024 * 1024,
            max_diagnostics: 100,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExtendedNameOptions<'a> {
    pub separator: &'a str,
    /// Input-key aliases as `(alias, canonical_key)` pairs.
    pub aliases: &'a [(&'a str, &'a str)],
    pub limits: ExtendedNameLimits,
}

impl Default for ExtendedNameOptions<'static> {
    fn default() -> Self {
        Self {
            separator: "=",
            aliases: &[],
            limits: ExtendedNameLimits::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExtendedNameDiagnosticKind {
    EmptyRecord,
    MalformedField,
    UnknownField,
    InvalidAttribute,
    UnbalancedBraces,
    Limit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtendedNameDiagnostic {
    pub kind: ExtendedNameDiagnosticKind,
    pub offset: usize,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtendedNameParse {
    pub name: Option<Name>,
    pub diagnostics: Vec<ExtendedNameDiagnostic>,
}

pub fn parse_extended_name(source: &str, options: ExtendedNameOptions<'_>) -> ExtendedNameParse {
    let mut parser = ExtendedParser::new(source, options);
    parser.parse()
}

struct ExtendedParser<'a> {
    source: &'a str,
    options: ExtendedNameOptions<'a>,
    diagnostics: Vec<ExtendedNameDiagnostic>,
    work: usize,
}

impl<'a> ExtendedParser<'a> {
    fn new(source: &'a str, options: ExtendedNameOptions<'a>) -> Self {
        Self {
            source,
            options,
            diagnostics: Vec::new(),
            work: 0,
        }
    }

    fn parse(&mut self) -> ExtendedNameParse {
        if self.source.trim().is_empty() {
            self.push(
                ExtendedNameDiagnosticKind::EmptyRecord,
                0,
                "extended name is empty",
            );
            return self.finish(None);
        }
        if self.options.separator.is_empty()
            || self.source.len() > self.options.limits.max_record_bytes
            || self.source.len() > self.options.limits.max_work
        {
            self.push(
                ExtendedNameDiagnosticKind::Limit,
                0,
                "extended name record exceeds its configured limit",
            );
            return self.finish(None);
        }

        let fields = match split_csv(self.source, self.options.limits.max_fields) {
            Ok(fields) => fields,
            Err((offset, message)) => {
                self.push(
                    if message.contains("limit") {
                        ExtendedNameDiagnosticKind::Limit
                    } else {
                        ExtendedNameDiagnosticKind::MalformedField
                    },
                    offset,
                    message,
                );
                return self.finish(None);
            }
        };
        let mut builder = NameBuilder::new();
        builder.source(self.source.trim().to_owned());
        let mut explicit_initials: [Option<Vec<String>>; 4] = Default::default();
        let mut part_values: [Option<(String, bool)>; 4] = Default::default();

        for (offset, field) in fields {
            self.work = self.work.saturating_add(field.len());
            if self.work > self.options.limits.max_work {
                self.push(
                    ExtendedNameDiagnosticKind::Limit,
                    offset,
                    "extended name work limit exceeded",
                );
                break;
            }
            if field.len() > self.options.limits.max_field_bytes {
                self.push(
                    ExtendedNameDiagnosticKind::Limit,
                    offset,
                    "extended name field exceeds byte limit",
                );
                continue;
            }
            let Some(separator) = field.find(self.options.separator) else {
                self.push(
                    ExtendedNameDiagnosticKind::MalformedField,
                    offset,
                    "extended name field has no key/value separator",
                );
                continue;
            };
            let raw_key = field[..separator].trim();
            let raw_value = field[separator + self.options.separator.len()..].trim();
            if raw_key.is_empty() || raw_value.is_empty() {
                self.push(
                    ExtendedNameDiagnosticKind::MalformedField,
                    offset,
                    "extended name field has an empty key or value",
                );
                continue;
            }
            if let Err((relative, message)) =
                validate_braces(raw_value, self.options.limits.max_nesting)
            {
                self.push(
                    if message.contains("limit") {
                        ExtendedNameDiagnosticKind::Limit
                    } else {
                        ExtendedNameDiagnosticKind::UnbalancedBraces
                    },
                    offset + separator + self.options.separator.len() + relative,
                    message,
                );
                continue;
            }

            let folded = raw_key.to_ascii_lowercase();
            let canonical = self
                .options
                .aliases
                .iter()
                .find_map(|(alias, canonical)| {
                    folded.eq_ignore_ascii_case(alias).then_some(*canonical)
                })
                .unwrap_or(&folded)
                .to_ascii_lowercase();
            let normalized_value = normalise_nfc(raw_value);
            builder.assignment(NameAssignment::new(
                canonical.clone(),
                normalized_value.clone(),
            ));

            let (base, initial_override) = canonical
                .strip_suffix("-i")
                .map_or((canonical.as_str(), false), |base| (base, true));
            if let Some(index) = part_index(base) {
                if initial_override {
                    explicit_initials[index] = Some(split_explicit_initials(&normalized_value));
                } else {
                    let (stripped, value) = remove_outer(&normalized_value);
                    let value = if stripped {
                        value
                    } else {
                        join_name_parts(&split_words(&value))
                    };
                    part_values[index] = Some((normalise_nfc(&value), stripped));
                }
                continue;
            }

            match canonical.as_str() {
                "id" => {
                    let (_, value) = remove_outer(&normalized_value);
                    builder.hash_id(normalise_nfc(&value));
                }
                "useprefix" => match parse_bool(&normalized_value) {
                    Some(value) => {
                        builder.use_prefix(value);
                    }
                    None => self.push(
                        ExtendedNameDiagnosticKind::InvalidAttribute,
                        offset,
                        "useprefix must be a boolean",
                    ),
                },
                "sortingnamekeytemplatename" | "nametemplates" => {
                    let (_, value) = remove_outer(&normalized_value);
                    builder.sorting_name_key_template(normalise_nfc(&value));
                }
                _ => self.push(
                    ExtendedNameDiagnosticKind::UnknownField,
                    offset,
                    format!("unknown extended name field '{raw_key}'"),
                ),
            }
        }

        for (index, value) in part_values.into_iter().enumerate() {
            let Some((value, stripped)) = value else {
                continue;
            };
            let generated = initials(&value, stripped);
            let part_value = NamePartValue::new(
                Literal::new(value),
                explicit_initials[index].take().unwrap_or(generated),
                stripped,
            );
            match index {
                0 => builder.family_part(part_value),
                1 => builder.given_part(part_value),
                2 => builder.prefix_part(part_value),
                3 => builder.suffix_part(part_value),
                _ => unreachable!(),
            };
        }

        let name = builder.freeze().ok();
        if name.is_none() {
            self.push(
                ExtendedNameDiagnosticKind::EmptyRecord,
                0,
                "extended name contains no name parts",
            );
        }
        self.finish(name)
    }

    fn finish(&mut self, name: Option<Name>) -> ExtendedNameParse {
        ExtendedNameParse {
            name,
            diagnostics: std::mem::take(&mut self.diagnostics),
        }
    }

    fn push(
        &mut self,
        kind: ExtendedNameDiagnosticKind,
        offset: usize,
        message: impl Into<String>,
    ) {
        if self.diagnostics.len() < self.options.limits.max_diagnostics {
            self.diagnostics.push(ExtendedNameDiagnostic {
                kind,
                offset,
                message: message.into(),
            });
        }
    }
}

fn part_index(value: &str) -> Option<usize> {
    match value {
        "family" => Some(0),
        "given" => Some(1),
        "prefix" => Some(2),
        "suffix" => Some(3),
        _ => None,
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn split_explicit_initials(value: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut group = String::new();
    let mut depth = 0usize;
    for character in value.chars() {
        match character {
            '{' => {
                if depth > 0 {
                    group.push(character);
                }
                depth += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    if !group.is_empty() {
                        result.push(std::mem::take(&mut group));
                    }
                } else {
                    group.push(character);
                }
            }
            _ if depth > 0 => group.push(character),
            _ if unicode_normalization::char::is_combining_mark(character) => {
                if let Some(previous) = result.last_mut() {
                    previous.push(character);
                }
            }
            _ => result.push(character.to_string()),
        }
    }
    result
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect()
}

fn split_csv(
    value: &str,
    max_fields: usize,
) -> Result<Vec<(usize, String)>, (usize, &'static str)> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut start = 0usize;
    let mut quoted = false;
    let mut quote_closed = false;
    let mut chars = value.char_indices().peekable();
    while let Some((offset, character)) = chars.next() {
        if character == '"' {
            if quoted && chars.peek().is_some_and(|(_, next)| *next == '"') {
                current.push('"');
                chars.next();
            } else if quoted {
                quoted = false;
                quote_closed = true;
            } else if current.trim().is_empty() && !quote_closed {
                current.clear();
                quoted = true;
            } else {
                return Err((offset, "misplaced quote in extended name field"));
            }
        } else if character == ',' && !quoted {
            fields.push((start, current.trim().to_owned()));
            if fields.len() > max_fields {
                return Err((offset, "extended name field count limit exceeded"));
            }
            current.clear();
            start = offset + 1;
            quote_closed = false;
        } else if quote_closed && !character.is_whitespace() {
            return Err((offset, "characters follow a closing CSV quote"));
        } else {
            current.push(character);
        }
    }
    if quoted {
        return Err((value.len(), "unclosed quote in extended name record"));
    }
    fields.push((start, current.trim().to_owned()));
    if fields.len() > max_fields {
        return Err((value.len(), "extended name field count limit exceeded"));
    }
    Ok(fields)
}

fn validate_braces(value: &str, max_nesting: usize) -> Result<(), (usize, &'static str)> {
    let mut depth = 0usize;
    for (offset, character) in value.char_indices() {
        match character {
            '{' => {
                depth += 1;
                if depth > max_nesting {
                    return Err((offset, "extended name brace nesting limit exceeded"));
                }
            }
            '}' if depth == 0 => return Err((offset, "unmatched closing brace in extended name")),
            '}' => depth -= 1,
            _ => {}
        }
    }
    (depth == 0)
        .then_some(())
        .ok_or((value.len(), "unclosed brace in extended name"))
}

#[cfg(test)]
mod tests;
