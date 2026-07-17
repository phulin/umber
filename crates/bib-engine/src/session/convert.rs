use std::collections::BTreeSet;

use bib_input::ClassicNameOptions;
use bib_label::{LabelEntry, select_labels};
use bib_model::{
    BibSourceLocation, Entry, EntryBuilder, EntryId, EntryType, FieldId, FieldProvenance,
    FieldValue, FieldValueStage, Literal, Range, RangeEndpoint, SourceSpan, TransformationId, Uri,
    Verbatim,
};
use umber_vfs::VirtualPath;

use super::{ProcessFailure, build_failure, invalid};

pub(super) fn convert_entry(
    raw: &bib_input::BibTexEntry,
    path: &VirtualPath,
) -> Result<Entry, ProcessFailure> {
    let source = source(path);
    let id = EntryId::new(raw.key()).map_err(|error| invalid(error.to_string()))?;
    let entry_type = EntryType::new(raw.entry_type().to_ascii_lowercase())
        .map_err(|error| invalid(error.to_string()))?;
    let mut builder = EntryBuilder::new(id, entry_type, source.clone());
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
        builder
            .field(
                field,
                value,
                FieldValueStage::Normalized,
                FieldProvenance::Datasource(source.clone()),
            )
            .map_err(build_failure)?;
        if name == "date" {
            add_date_parts(&mut builder, raw_field.value(), &source, &raw_names)?;
        }
    }
    Ok(builder.freeze())
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
    builder: &mut EntryBuilder,
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
        builder
            .field(
                FieldId::new(name).expect("fixed field id is valid"),
                FieldValue::Integer(value),
                FieldValueStage::Derived,
                FieldProvenance::Datasource(source.clone()),
            )
            .map_err(build_failure)?;
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

pub(super) fn add_label_sources(entry: Entry) -> Result<Entry, ProcessFailure> {
    let mut label = LabelEntry::default();
    for field in entry.fields().iter() {
        match field.value() {
            FieldValue::NameList(names) => {
                label.names.insert(field.id().as_str(), names);
            }
            FieldValue::Literal(value) => {
                label.fields.insert(field.id().as_str(), value.as_str());
            }
            _ => {}
        }
    }
    let selection = select_labels(
        &label,
        &["author", "editor", "translator"],
        &["labelyear", "year", "date"],
        &["labeltitle", "title", "maintitle"],
    );
    let mut builder = EntryBuilder::new(
        entry.id().clone(),
        entry.entry_type().clone(),
        entry.source().clone(),
    );
    for field in entry.fields().iter() {
        builder
            .field(
                field.id().clone(),
                field.value().clone(),
                field.stage(),
                field.provenance().clone(),
            )
            .map_err(build_failure)?;
    }
    let transformation = TransformationId::new("label-source").expect("fixed id is valid");
    for (name, value) in [
        ("labelnamesource", selection.name_source),
        ("labeldatesource", selection.date_source),
        ("labeltitlesource", selection.title_source),
    ] {
        if entry
            .fields()
            .get(&FieldId::new(name).expect("fixed field id is valid"))
            .is_none()
        {
            builder
                .field(
                    FieldId::new(name).expect("fixed field id is valid"),
                    FieldValue::Literal(Literal::new(value.unwrap_or_default())),
                    FieldValueStage::Computed,
                    FieldProvenance::Computed {
                        transformation: transformation.clone(),
                        inputs: Vec::new(),
                    },
                )
                .map_err(build_failure)?;
        }
    }
    Ok(builder.freeze())
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
