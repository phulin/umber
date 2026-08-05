use std::collections::BTreeSet;

use bib_input::ClassicNameOptions;
use bib_model::{
    BibSourceLocation, EntryId, EntryType, FieldId, FieldProvenance, FieldValue, FieldValueStage,
    Literal, Range, RangeEndpoint, SourceSpan, Uri, Verbatim,
};
use umber_vfs::VirtualPath;

use super::{ProcessFailure, invalid};
use crate::biber::EntryEditor;

pub(super) fn convert_entry(
    raw: &bib_input::BibTexEntry,
    path: &VirtualPath,
) -> Result<EntryEditor, ProcessFailure> {
    let source = source(path);
    let id = EntryId::new(raw.key()).map_err(|error| invalid(error.to_string()))?;
    let entry_type = EntryType::new(raw.entry_type().to_ascii_lowercase())
        .map_err(|error| invalid(error.to_string()))?;
    let mut editor = EntryEditor::new(id, entry_type, source.clone());
    let raw_names = raw
        .fields()
        .iter()
        .map(|field| field.name().to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let mut names = BTreeSet::new();
    for raw_field in raw.fields() {
        let name = raw_field.name().to_ascii_lowercase();
        if !names.insert(name.clone()) {
            continue;
        }
        let field = FieldId::new(name.clone()).map_err(|error| invalid(error.to_string()))?;
        let value = typed_field(&name, raw_field.value())?;
        editor.set_field(
            field,
            value,
            FieldValueStage::Normalized,
            FieldProvenance::Datasource(source.clone()),
        );
        if name == "date" {
            add_date_parts(&mut editor, raw_field.value(), &source, &raw_names)?;
        }
    }
    Ok(editor)
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
    editor: &mut EntryEditor,
    value: &str,
    source: &BibSourceLocation,
    existing: &BTreeSet<String>,
) -> Result<(), ProcessFailure> {
    let mut parts = value.split('-');
    for (name, part) in [
        ("year", parts.next()),
        ("month", parts.next()),
        ("day", parts.next()),
    ] {
        if existing.contains(name) {
            continue;
        }
        let Some(Ok(value)) = part.map(str::parse::<i64>) else {
            continue;
        };
        editor.set_field(
            FieldId::new(name).expect("fixed field id is valid"),
            FieldValue::Integer(value),
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
