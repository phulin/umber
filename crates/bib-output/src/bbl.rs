use std::fmt;
use std::fmt::Write as _;
use std::sync::Arc;

use bib_model::{
    BibDiagnostic, BibDiagnosticCode, BibSeverity, DataListKind, DateValue, Entry, Field,
    FieldValue, GeneratedFile, Name, OutputFormat, OutputNewline, OutputRequest, Range,
    RangeEndpoint,
};
use bib_unicode::{EncodingError, compatibility_hash, encode_legacy, normalise_nfc};

use crate::{OutputContext, Serializer};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BblOutputFailureKind {
    WrongFormat,
    IncompatibleVersion,
    MalformedValue,
    Unrepresentable,
    Limit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BblOutputFailure {
    kind: BblOutputFailureKind,
    diagnostics: Arc<[BibDiagnostic]>,
}

impl BblOutputFailure {
    #[must_use]
    pub const fn kind(&self) -> BblOutputFailureKind {
        self.kind
    }

    pub fn diagnostics(&self) -> impl ExactSizeIterator<Item = &BibDiagnostic> {
        self.diagnostics.iter()
    }
}

impl fmt::Display for BblOutputFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = self
            .diagnostics
            .first()
            .map_or("BBL output failed", BibDiagnostic::message);
        formatter.write_str(message)
    }
}

impl std::error::Error for BblOutputFailure {}

#[derive(Clone, Copy, Debug, Default)]
pub struct BblSerializer;

impl Serializer for BblSerializer {
    type Error = BblOutputFailure;

    fn serialize(
        &self,
        context: OutputContext<'_>,
        request: &OutputRequest,
    ) -> Result<GeneratedFile, Self::Error> {
        if request.format() != OutputFormat::Bbl {
            return Err(failure(
                BblOutputFailureKind::WrongFormat,
                "BIB_OUTPUT_FORMAT",
                "the BBL serializer requires a BBL output request",
            ));
        }
        let compatibility = context.document().configuration().version();
        if compatibility != context.unicode().compatibility() || compatibility.bbl_schema != "3.3" {
            return Err(failure(
                BblOutputFailureKind::IncompatibleVersion,
                "BIB_OUTPUT_VERSION",
                "the processed document and Unicode tables must use BBL schema 3.3",
            ));
        }

        let mut writer = BoundedWriter::new(request.max_bytes());
        writer.push(HEADER)?;
        for section in context.document().sections() {
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
                    write_entry(&mut writer, entry, item.context_fields())?;
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

        let text = match request.newline() {
            OutputNewline::Lf => writer.finish(),
            OutputNewline::CrLf => writer.finish().replace('\n', "\r\n"),
        };
        let bytes = encode_legacy(&text, request.encoding()).map_err(|error| match error {
            EncodingError::UnmappableCharacter => failure(
                BblOutputFailureKind::Unrepresentable,
                "BIB_OUTPUT_ENCODING",
                "BBL output contains a character unavailable in the requested encoding",
            ),
            EncodingError::UnknownLabel | EncodingError::MalformedInput => failure(
                BblOutputFailureKind::MalformedValue,
                "BIB_OUTPUT_ENCODING",
                "the requested BBL encoding is invalid",
            ),
        })?;
        if bytes.len() > request.max_bytes() {
            return Err(limit_failure(request.max_bytes()));
        }
        Ok(GeneratedFile::new(request.path().clone(), bytes))
    }
}

fn write_entry<'a>(
    writer: &mut BoundedWriter,
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

fn write_field(writer: &mut BoundedWriter, field: &Field) -> Result<(), BblOutputFailure> {
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
    writer: &mut BoundedWriter,
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

fn write_name(writer: &mut BoundedWriter, name: &Name) -> Result<(), BblOutputFailure> {
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
    writer: &mut BoundedWriter,
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
    writer: &mut BoundedWriter,
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
    let code = BibDiagnosticCode::new(code).expect("static output diagnostic code is valid");
    let diagnostic = bib_model::DiagnosticBuilder::new(code, BibSeverity::Error, message)
        .expect("output diagnostic message is valid")
        .freeze();
    BblOutputFailure {
        kind,
        diagnostics: Arc::from([diagnostic]),
    }
}

fn limit_failure(limit: usize) -> BblOutputFailure {
    failure(
        BblOutputFailureKind::Limit,
        "BIB_OUTPUT_LIMIT",
        &format!("BBL output exceeds the configured {limit}-byte limit"),
    )
}

struct BoundedWriter {
    value: String,
    output_limit: usize,
    work_limit: usize,
}

impl BoundedWriter {
    fn new(output_limit: usize) -> Self {
        Self {
            value: String::new(),
            output_limit,
            // A legacy encoding can represent a multi-byte UTF-8 scalar in
            // one byte, while CRLF can expand generated line endings. Keep
            // construction bounded, then enforce the exact encoded size.
            work_limit: output_limit.saturating_mul(4),
        }
    }

    fn push(&mut self, value: &str) -> Result<(), BblOutputFailure> {
        let length = self
            .value
            .len()
            .checked_add(value.len())
            .ok_or_else(|| limit_failure(self.output_limit))?;
        if length > self.work_limit {
            return Err(limit_failure(self.output_limit));
        }
        self.value.push_str(value);
        Ok(())
    }

    fn line(&mut self, value: &str) -> Result<(), BblOutputFailure> {
        self.push(value)?;
        self.push("\n")
    }

    fn finish(self) -> String {
        self.value
    }
}
