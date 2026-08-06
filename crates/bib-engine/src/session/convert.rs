use std::collections::{BTreeMap, BTreeSet};

use bib_input::{
    BibTexDiagnostic, BibTexDiagnosticKind, ClassicNameOptions, RawBibDatabase, RawBibRecord,
    RawBibValue, RawBibValuePart,
};
use bib_model::{
    BibSourceLocation, EntryId, EntryType, FieldId, FieldProvenance, FieldValue, FieldValueStage,
    Literal, Range, RangeEndpoint, SourceSpan, Uri, Verbatim,
};
use bib_unicode::{RecodeSet, TexRecoder};
use umber_vfs::VirtualPath;

use super::{ProcessFailure, invalid};
use crate::biber::DraftEntry;

pub(super) fn lower_database(
    raw: &RawBibDatabase,
    path: &VirtualPath,
    seen_entries: &mut BTreeSet<String>,
) -> Result<(Vec<DraftEntry>, Vec<BibTexDiagnostic>), ProcessFailure> {
    let source = source(path);
    let recoder = TexRecoder::new(raw.options().decode, RecodeSet::Null);
    let mut macros = month_macros();
    let mut diagnostics = raw.diagnostics().to_vec();
    let mut entries = Vec::new();
    for record in raw.records() {
        match record {
            RawBibRecord::String(definition) => {
                let name = recoder.decode(definition.name().folded());
                let value = expand(definition.value(), &recoder, &macros, &mut diagnostics);
                macros.insert(name, value);
            }
            RawBibRecord::Preamble(preamble) => {
                let _ = expand(preamble.value(), &recoder, &macros, &mut diagnostics);
            }
            RawBibRecord::Entry(raw_entry) => {
                let key = recoder.decode(raw_entry.key().source());
                if !seen_entries.insert(key.to_ascii_lowercase()) {
                    continue;
                }
                let id = EntryId::new(key).map_err(|error| invalid(error.to_string()))?;
                let entry_type = EntryType::new(recoder.decode(raw_entry.entry_type().folded()))
                    .map_err(|error| invalid(error.to_string()))?;
                let mut draft = DraftEntry::new(id, entry_type, source.clone());
                let mut names = BTreeSet::new();
                let mut expanded_fields = Vec::new();
                for raw_field in raw_entry.fields() {
                    let name = recoder.decode(raw_field.name().folded());
                    if !names.insert(name.clone()) {
                        continue;
                    }
                    let value = expand(raw_field.value(), &recoder, &macros, &mut diagnostics);
                    expanded_fields.push((name, value));
                }
                let existing = expanded_fields
                    .iter()
                    .map(|(name, _)| name.clone())
                    .collect::<BTreeSet<_>>();
                let date = expanded_fields
                    .iter()
                    .find(|(name, _)| name == "date")
                    .map(|(_, value)| value.clone());
                for (name, value) in expanded_fields {
                    draft.set_field(
                        FieldId::new(name.clone()).map_err(|error| invalid(error.to_string()))?,
                        typed_field(&name, &value)?,
                        FieldValueStage::Normalized,
                        FieldProvenance::Datasource(source.clone()),
                    );
                }
                if let Some(date) = date {
                    add_date_parts(&mut draft, &date, &source, &existing)?;
                }
                entries.push(draft);
            }
            RawBibRecord::Comment(_) | RawBibRecord::Recovery(_) => {}
        }
    }
    Ok((entries, diagnostics))
}

fn expand(
    value: &RawBibValue,
    recoder: &TexRecoder,
    macros: &BTreeMap<String, String>,
    diagnostics: &mut Vec<BibTexDiagnostic>,
) -> String {
    let mut result = String::new();
    for part in value.parts() {
        match part {
            RawBibValuePart::Braced(text)
            | RawBibValuePart::Quoted(text)
            | RawBibValuePart::Number(text) => result.push_str(&recoder.decode(text.source())),
            RawBibValuePart::Macro(name) => {
                let name = recoder.decode(name.folded());
                if let Some(value) = macros.get(&name) {
                    result.push_str(value);
                } else {
                    diagnostics.push(BibTexDiagnostic {
                        kind: BibTexDiagnosticKind::UndefinedMacro,
                        offset: part.location().byte_start(),
                        message: format!("undefined string macro `{name}`"),
                    });
                    result.push_str(&name);
                }
            }
        }
    }
    result
}

fn month_macros() -> BTreeMap<String, String> {
    [
        ("jan", "1"),
        ("feb", "2"),
        ("mar", "3"),
        ("apr", "4"),
        ("may", "5"),
        ("jun", "6"),
        ("jul", "7"),
        ("aug", "8"),
        ("sep", "9"),
        ("oct", "10"),
        ("nov", "11"),
        ("dec", "12"),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value.to_owned()))
    .collect()
}

fn typed_field(name: &str, value: &str) -> Result<FieldValue, ProcessFailure> {
    let trimmed = value.trim();
    if matches!(
        name,
        "author" | "bookauthor" | "commentator" | "editor" | "nameholder" | "translator"
    ) {
        let parsed = bib_input::parse_classic_name_list(trimmed, ClassicNameOptions::default());
        return Ok(FieldValue::NameList(parsed.names));
    }
    if matches!(name, "url" | "urls") {
        let values = trimmed
            .split(" and ")
            .map(|value| Uri::new(value.trim()).map_err(|error| invalid(error.to_owned())))
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(FieldValue::UriList(values));
    }
    if matches!(name, "pages" | "pagetotal") {
        return Ok(FieldValue::RangeList(parse_ranges(trimmed)));
    }
    if matches!(name, "doi" | "eprint" | "file") {
        return Ok(FieldValue::Verbatim(Verbatim::new(trimmed)));
    }
    if matches!(name, "keywords" | "location" | "publisher") {
        return Ok(FieldValue::LiteralList(
            trimmed
                .split(',')
                .map(|part| Literal::new(part.trim()))
                .collect(),
        ));
    }
    if matches!(name, "crossref" | "xref" | "xdata" | "related" | "entryset") {
        let keys = trimmed
            .split(',')
            .map(|key| EntryId::new(key.trim()).map_err(|error| invalid(error.to_string())))
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(FieldValue::KeyList(keys));
    }
    if let Ok(integer) = trimmed.parse::<i64>() {
        return Ok(FieldValue::Integer(integer));
    }
    Ok(FieldValue::Literal(Literal::new(trimmed)))
}

fn add_date_parts(
    editor: &mut DraftEntry,
    value: &str,
    source: &BibSourceLocation,
    existing: &BTreeSet<String>,
) -> Result<(), ProcessFailure> {
    let mut parts = value.split('-');
    let year = parts.next().filter(|value| value.len() == 4);
    let month = parts
        .next()
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| (1..=12).contains(value));
    for (name, value) in [
        ("year", year.and_then(|value| value.parse().ok())),
        ("month", month),
    ] {
        if existing.contains(name) || value.is_none() {
            continue;
        }
        editor.set_field(
            FieldId::new(name).expect("fixed field id is valid"),
            FieldValue::Integer(value.expect("checked above")),
            FieldValueStage::Derived,
            FieldProvenance::Datasource(source.clone()),
        );
    }
    Ok(())
}

fn parse_ranges(value: &str) -> Vec<Range> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (start, end) = if let Some((start, end)) = part.split_once("---") {
                (start, Some(end))
            } else if let Some((start, end)) = part.split_once("--") {
                (start, Some(end))
            } else if let Some((start, end)) = part.split_once('-') {
                (start, Some(end))
            } else {
                (part, None)
            };
            Range::new(endpoint(start), end.map_or(RangeEndpoint::Open, endpoint))
        })
        .collect()
}

fn endpoint(value: &str) -> RangeEndpoint {
    let value = value.trim().trim_matches('{').trim_matches('}').trim();
    value.parse::<i64>().map_or_else(
        |_| RangeEndpoint::Literal(Literal::new(value)),
        RangeEndpoint::Integer,
    )
}

fn source(path: &VirtualPath) -> BibSourceLocation {
    BibSourceLocation::new(
        path.clone(),
        SourceSpan {
            byte_start: 0,
            byte_end: 0,
            line: 1,
            column: 1,
        },
    )
    .expect("fixed source span is valid")
}
