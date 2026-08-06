use std::fmt::Write as _;

use bib_model::{
    DataListKind, DateValue, Entry, Field, FieldValue, GeneratedFile, Name, OutputFormat,
    OutputRequest, Range, RangeEndpoint,
};
use bib_unicode::{compatibility_hash, normalise_nfc};

use crate::{
    BblOutputFailure, BblOutputFailureKind, OutputContext, OutputPlan, OutputRouter,
    router::{OutputSink, failure as output_failure},
};

const HEADER: &str = concat!(
    "% $ biblatex auxiliary file $\n",
    "% $ biblatex bbl format version 3.3 $\n",
    "% Do not modify the above lines!\n",
    "%\n",
    "% This is an auxiliary file used by the 'biblatex' package.\n",
    "% This file may safely be deleted. It will be recreated by\n",
    "% biber as required.\n",
    "%\n",
    "\\begingroup\n",
    "\\makeatletter\n",
    "\\@ifundefined{ver@biblatex.sty}\n",
    "  {\\@latex@error\n",
    "     {Missing 'biblatex' package}\n",
    "     {The bibliography requires the 'biblatex' package.}\n",
    "      \\aftergroup\\endinput}\n",
    "  {}\n",
    "\\endgroup\n\n\n",
);

#[derive(Clone, Copy, Debug, Default)]
pub struct BblSerializer;

impl BblSerializer {
    pub fn serialize(
        &self,
        context: OutputContext<'_>,
        request: &OutputRequest,
    ) -> Result<GeneratedFile, BblOutputFailure> {
        OutputRouter::default().serialize_as(OutputFormat::Bbl, context, request)
    }
}

pub(crate) fn render(
    plan: &OutputPlan<'_>,
    writer: &mut OutputSink<'_>,
) -> Result<(), BblOutputFailure> {
    writer.push(HEADER)?;
    for section in plan.sections() {
        writer.line(&format!("\\refsection{{{}}}", section.id()))?;
        for list in section.lists() {
            let kind = match list.kind() {
                DataListKind::Entry => "entry",
                DataListKind::List => "list",
            };
            validate_argument(list.id().as_str(), "data-list identifier")?;
            writer.line(&format!("  \\datalist[{kind}]{{{}}}", list.id()))?;
            for item in list.items() {
                let entry_id = item.entry();
                let entry = section.entry(entry_id).ok_or_else(|| {
                    failure(
                        BblOutputFailureKind::MalformedValue,
                        "BIB_OUTPUT_UNKNOWN_ENTRY",
                        &format!("data list references unknown entry `{entry_id}`"),
                    )
                })?;
                write_entry(writer, entry, item.context_fields())?;
            }
            writer.line("  \\enddatalist")?;
        }
        for (alias, target) in section.aliases() {
            validate_argument(alias.as_str(), "entry alias")?;
            validate_argument(target.as_str(), "entry identifier")?;
            writer.line(&format!("  \\keyalias{{{alias}}}{{{target}}}"))?;
        }
        for key in section.undefined_keys() {
            validate_argument(key.as_str(), "undefined entry identifier")?;
            writer.line(&format!("  \\missing{{{key}}}"))?;
        }
        writer.line("\\endrefsection")?;
    }
    writer.line("\\endinput")?;
    writer.push("\n")?;

    Ok(())
}

fn write_entry<'a>(
    writer: &mut OutputSink<'_>,
    entry: &Entry,
    context_fields: impl ExactSizeIterator<Item = &'a Field>,
) -> Result<(), BblOutputFailure> {
    validate_argument(entry.id().as_str(), "entry identifier")?;
    writer.line(&format!(
        "    \\entry{{{}}}{{{}}}{{}}{{}}",
        entry.id(),
        entry.entry_type()
    ))?;
    let context_fields = context_fields.collect::<Vec<_>>();
    for field in entry.fields().iter() {
        if let Some(context) = context_fields
            .iter()
            .find(|context| context.id() == field.id())
        {
            write_field(writer, context)?;
        } else {
            write_field(writer, field)?;
        }
    }
    for field in context_fields {
        if entry.fields().get(field.id()).is_none() {
            write_field(writer, field)?;
        }
    }
    for annotation in entry.annotations() {
        validate_text(annotation.value(), "annotation value")?;
        writer.line(&format!(
            "      \\annotation{{{}}}{{{}}}",
            annotation.name(),
            annotation.value()
        ))?;
    }
    writer.line("    \\endentry")
}

fn write_field(writer: &mut OutputSink<'_>, field: &Field) -> Result<(), BblOutputFailure> {
    let id = field.id().as_str();
    match field.value() {
        FieldValue::Literal(value) => {
            validate_text(value.as_str(), "literal field")?;
            let command = if is_string_field(id) {
                "strng"
            } else {
                "field"
            };
            writer.line(&format!("      \\{command}{{{id}}}{{{}}}", value.as_str()))
        }
        FieldValue::Verbatim(value) => {
            validate_text(value.as_str(), "verbatim field")?;
            writer.line(&format!(
                "      \\verb{{{id}}}\n      \\verb {}\n      \\endverb",
                value.as_str()
            ))
        }
        FieldValue::Integer(value) => writer.line(&format!("      \\field{{{id}}}{{{value}}}")),
        FieldValue::Boolean(value) => writer.line(&format!(
            "      \\{}{{{id}}}",
            if *value { "true" } else { "false" }
        )),
        FieldValue::NameList(names) => write_names(writer, id, names),
        FieldValue::LiteralList(values) => {
            let values = values
                .iter()
                .map(|value| value.as_str())
                .collect::<Vec<_>>();
            write_list(writer, id, &values)
        }
        FieldValue::KeyList(values) => {
            let values = values
                .iter()
                .map(|value| value.as_str())
                .collect::<Vec<_>>();
            write_list(writer, id, &values)
        }
        FieldValue::UriList(values) => {
            let values = values
                .iter()
                .map(|value| value.as_str())
                .collect::<Vec<_>>();
            write_list(writer, id, &values)
        }
        FieldValue::RangeList(values) => write_ranges(writer, id, values),
        FieldValue::Date(value) => {
            writer.line(&format!("      \\field{{{id}}}{{{}}}", format_date(value)))
        }
    }
}

fn write_names(
    writer: &mut OutputSink<'_>,
    id: &str,
    names: &bib_model::NameList,
) -> Result<(), BblOutputFailure> {
    writer.line(&format!("      \\name{{{id}}}{{{}}}{{}}{{%", names.len()))?;
    for name in names.iter() {
        write_name(writer, name)?;
    }
    if names.has_others() {
        writer.line("        {{}%")?;
    }
    writer.line("      }")
}

fn write_name(writer: &mut OutputSink<'_>, name: &Name) -> Result<(), BblOutputFailure> {
    let hash = name
        .hash_id()
        .map_or_else(|| name_hash(name), compatibility_hash);
    let mut attributes = vec![
        "un=0".to_owned(),
        "uniquepart=base".to_owned(),
        format!("hash={hash}"),
    ];
    attributes.extend(
        name.assignments()
            .map(|assignment| format!("{}={}", assignment.key(), assignment.value())),
    );
    if let Some(value) = name.use_prefix() {
        attributes.push(format!("useprefix={}", usize::from(value)));
    }
    writer.line(&format!("        {{{{{}}}{{%", attributes.join(",")))?;
    let parts = [
        ("family", name.family()),
        ("given", name.given()),
        ("prefix", name.prefix()),
        ("suffix", name.suffix()),
    ];
    let mut properties = Vec::new();
    for (kind, part) in parts {
        let Some(part) = part else { continue };
        let value = name_text(part.value().as_str());
        validate_text(&value, "name part")?;
        properties.push(format!("{kind}={{{value}}}"));
        let initials = part
            .initials()
            .map(|initial| format!("{}\\bibinitperiod", initial.trim_end_matches('.')))
            .collect::<Vec<_>>()
            .join("\\bibinitdelim ");
        if !initials.is_empty() {
            properties.push(format!("{kind}i={{{initials}}}"));
        }
        if kind == "given" {
            properties.push("givenun=0".to_owned());
        }
    }
    let property_count = properties.len();
    for (index, property) in properties.into_iter().enumerate() {
        writer.line(&format!(
            "           {property}{}",
            if index + 1 == property_count {
                "}}%"
            } else {
                ","
            }
        ))?;
    }
    Ok(())
}

fn write_list(
    writer: &mut OutputSink<'_>,
    id: &str,
    values: &[&str],
) -> Result<(), BblOutputFailure> {
    for value in values {
        validate_text(value, "list item")?;
    }
    writer.line(&format!("      \\list{{{id}}}{{{}}}{{%", values.len()))?;
    for value in values {
        writer.line(&format!("        {{{value}}}%"))?;
    }
    writer.line("      }")
}

fn write_ranges(
    writer: &mut OutputSink<'_>,
    id: &str,
    values: &[Range],
) -> Result<(), BblOutputFailure> {
    writer.line(&format!("      \\range{{{id}}}{{{}}}{{%", values.len()))?;
    for value in values {
        writer.line(&format!(
            "        \\range{{{}}}{{{}}}%",
            range_endpoint(value.start()),
            range_endpoint(value.end())
        ))?;
    }
    writer.line("      }")
}

fn range_endpoint(value: &RangeEndpoint) -> String {
    match value {
        RangeEndpoint::Integer(value) => value.to_string(),
        RangeEndpoint::Literal(value) => value.as_str().to_owned(),
        RangeEndpoint::Open => String::new(),
    }
}

fn format_date(value: &DateValue) -> String {
    let mut output = format!("{:04}", value.year());
    if let Some(month) = value.month() {
        write!(output, "-{month:02}").expect("writing to String cannot fail");
    }
    if let Some(day) = value.day() {
        write!(output, "-{day:02}").expect("writing to String cannot fail");
    }
    if value.is_uncertain() {
        output.push('?');
    }
    if value.is_approximate() {
        output.push('~');
    }
    output
}

fn is_string_field(id: &str) -> bool {
    matches!(id, "namehash" | "fullhash" | "fullhashraw" | "bibnamehash")
        || id.ends_with("namehash")
        || id.ends_with("fullhash")
        || id.ends_with("fullhashraw")
}

fn name_hash(name: &Name) -> String {
    let mut value = String::new();
    for part in [name.prefix(), name.family(), name.suffix(), name.given()]
        .into_iter()
        .flatten()
    {
        value.push_str(part.value().as_str());
    }
    compatibility_hash(&normalise_nfc(&value))
}

fn name_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("\\bibnamedelima ")
}

fn validate_argument(value: &str, kind: &str) -> Result<(), BblOutputFailure> {
    validate_text(value, kind)?;
    if value.contains(['{', '}']) {
        return Err(failure(
            BblOutputFailureKind::MalformedValue,
            "BIB_OUTPUT_ARGUMENT",
            &format!("{kind} contains a structural brace"),
        ));
    }
    Ok(())
}

fn validate_text(value: &str, kind: &str) -> Result<(), BblOutputFailure> {
    if value.contains('\0') {
        return Err(failure(
            BblOutputFailureKind::MalformedValue,
            "BIB_OUTPUT_VALUE",
            &format!("{kind} contains NUL"),
        ));
    }
    Ok(())
}

fn failure(kind: BblOutputFailureKind, code: &str, message: &str) -> BblOutputFailure {
    output_failure(OutputFormat::Bbl, kind, code, message)
}
