use std::collections::BTreeSet;
use std::fmt;
use std::fmt::Write as _;
use std::sync::Arc;

use bib_model::{
    BibDiagnostic, BibDiagnosticCode, BibSeverity, DateValue, DiagnosticBuilder, Entry, Field,
    FieldValue, GeneratedFile, OutputFormat, OutputNewline, OutputRequest, Range, RangeEndpoint,
};
use bib_unicode::{EncodingError, RecodeSet, TexRecoder, encode_legacy};

use crate::{OutputContext, Serializer};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BibtexCase {
    Preserve,
    Lower,
    #[default]
    Upper,
}

impl BibtexCase {
    fn apply(self, value: &str) -> String {
        match self {
            Self::Preserve => value.to_owned(),
            Self::Lower => value.to_ascii_lowercase(),
            Self::Upper => value.to_ascii_uppercase(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibtexMacro {
    name: Arc<str>,
    value: Arc<str>,
    fields: Arc<[Arc<str>]>,
}

impl BibtexMacro {
    #[must_use]
    pub fn new(name: impl Into<Arc<str>>, value: impl Into<Arc<str>>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            fields: Arc::from([]),
        }
    }

    #[must_use]
    pub fn with_fields(mut self, fields: impl IntoIterator<Item = impl Into<Arc<str>>>) -> Self {
        self.fields = fields.into_iter().map(Into::into).collect();
        self
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    fn applies_to(&self, field: &Field) -> bool {
        field_text(field.value()).is_some_and(|value| self.value.as_ref() == value)
            && (self.fields.is_empty()
                || self
                    .fields
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(field.id().as_str())))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibtexOptions {
    align_fields: bool,
    entry_case: BibtexCase,
    field_case: BibtexCase,
    escape: RecodeSet,
    macros: Arc<[BibtexMacro]>,
    comments: Arc<[Arc<str>]>,
}

impl Default for BibtexOptions {
    fn default() -> Self {
        Self {
            align_fields: false,
            entry_case: BibtexCase::Upper,
            field_case: BibtexCase::Upper,
            escape: RecodeSet::Null,
            macros: Arc::from([]),
            comments: Arc::from([]),
        }
    }
}

impl BibtexOptions {
    #[must_use]
    pub const fn with_alignment(mut self, enabled: bool) -> Self {
        self.align_fields = enabled;
        self
    }

    #[must_use]
    pub const fn with_entry_case(mut self, value: BibtexCase) -> Self {
        self.entry_case = value;
        self
    }

    #[must_use]
    pub const fn with_field_case(mut self, value: BibtexCase) -> Self {
        self.field_case = value;
        self
    }

    #[must_use]
    pub const fn with_escape(mut self, value: RecodeSet) -> Self {
        self.escape = value;
        self
    }

    #[must_use]
    pub fn with_macros(mut self, values: impl IntoIterator<Item = BibtexMacro>) -> Self {
        self.macros = values.into_iter().collect();
        self
    }

    #[must_use]
    pub fn with_comments(mut self, values: impl IntoIterator<Item = impl Into<Arc<str>>>) -> Self {
        self.comments = values.into_iter().map(Into::into).collect();
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BibtexOutputFailureKind {
    WrongFormat,
    IncompatibleVersion,
    MalformedValue,
    Unrepresentable,
    Limit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibtexOutputFailure {
    kind: BibtexOutputFailureKind,
    diagnostics: Arc<[BibDiagnostic]>,
}

impl BibtexOutputFailure {
    #[must_use]
    pub const fn kind(&self) -> BibtexOutputFailureKind {
        self.kind
    }

    pub fn diagnostics(&self) -> impl ExactSizeIterator<Item = &BibDiagnostic> {
        self.diagnostics.iter()
    }
}

impl fmt::Display for BibtexOutputFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = self
            .diagnostics
            .first()
            .map_or("BibTeX output failed", BibDiagnostic::message);
        formatter.write_str(message)
    }
}

impl std::error::Error for BibtexOutputFailure {}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BibtexSerializer {
    options: BibtexOptions,
}

impl BibtexSerializer {
    #[must_use]
    pub fn new(options: BibtexOptions) -> Self {
        Self { options }
    }

    #[must_use]
    pub const fn options(&self) -> &BibtexOptions {
        &self.options
    }

    pub fn serialize_entry(&self, entry: &Entry) -> Result<String, BibtexOutputFailure> {
        let mut output = String::new();
        self.write_entry(&mut output, entry)?;
        Ok(output)
    }

    fn write_entry(&self, output: &mut String, entry: &Entry) -> Result<(), BibtexOutputFailure> {
        validate_identifier(entry.id().as_str(), "entry identifier")?;
        validate_identifier(entry.entry_type().as_str(), "entry type")?;
        writeln!(
            output,
            "@{}{{{},",
            self.options.entry_case.apply(entry.entry_type().as_str()),
            entry.id()
        )
        .expect("writing a String cannot fail");

        let annotations = entry.annotations().collect::<Vec<_>>();
        let mut emitted_annotations = vec![false; annotations.len()];
        let mut rendered = Vec::with_capacity(entry.fields().len() + annotations.len());
        for field in entry.fields().iter() {
            let name = self.options.field_case.apply(field.id().as_str());
            validate_identifier(&name, "field name")?;
            let value = self.render_field(field)?;
            rendered.push((name, value));
            for (index, annotation) in annotations.iter().enumerate() {
                if annotation.field() == Some(field.id()) {
                    rendered.push(self.render_annotation(annotation)?);
                    emitted_annotations[index] = true;
                }
            }
        }
        for (index, annotation) in annotations.iter().enumerate() {
            if !emitted_annotations[index] {
                rendered.push(self.render_annotation(annotation)?);
            }
        }
        let width = if self.options.align_fields {
            rendered
                .iter()
                .map(|(name, _)| name.len())
                .max()
                .unwrap_or(0)
        } else {
            0
        };
        for (name, value) in rendered {
            if width == 0 {
                writeln!(output, "  {name} = {value},")
            } else {
                writeln!(output, "  {name:<width$} = {value},")
            }
            .expect("writing a String cannot fail");
        }
        output.push_str("}\n\n");
        Ok(())
    }

    fn render_field(&self, field: &Field) -> Result<String, BibtexOutputFailure> {
        if let Some(value) = self
            .options
            .macros
            .iter()
            .find(|value| value.applies_to(field))
        {
            return Ok(value.name().to_owned());
        }
        let value = match field.value() {
            FieldValue::Literal(value) => value.as_str().to_owned(),
            FieldValue::Verbatim(value) => value.as_str().to_owned(),
            FieldValue::Integer(value) => value.to_string(),
            FieldValue::Boolean(value) => value.to_string(),
            FieldValue::NameList(value) => value
                .iter()
                .map(bib_model::Name::to_bibtex)
                .chain(value.has_others().then(|| "others".to_owned()))
                .collect::<Vec<_>>()
                .join(" and "),
            FieldValue::LiteralList(value) => value
                .iter()
                .map(|item| item.as_str())
                .collect::<Vec<_>>()
                .join(" and "),
            FieldValue::KeyList(value) => value
                .iter()
                .map(|item| item.as_str())
                .collect::<Vec<_>>()
                .join(","),
            FieldValue::UriList(value) => value
                .iter()
                .map(|item| item.as_str())
                .collect::<Vec<_>>()
                .join(" and "),
            FieldValue::RangeList(value) => value
                .iter()
                .map(format_range)
                .collect::<Vec<_>>()
                .join(" and "),
            FieldValue::Date(value) => format_date(value),
        };
        validate_value(&value, "field value")?;
        Ok(format!("{{{value}}}"))
    }

    fn render_annotation(
        &self,
        annotation: &bib_model::Annotation,
    ) -> Result<(String, String), BibtexOutputFailure> {
        let name = annotation.field().map_or_else(
            || self.options.field_case.apply(annotation.name().as_str()),
            |field| {
                format!(
                    "{}+an:{}",
                    self.options.field_case.apply(field.as_str()),
                    annotation.name()
                )
            },
        );
        validate_identifier(&name, "annotation name")?;
        validate_value(annotation.value(), "annotation value")?;
        Ok((name, format!("{{{}}}", annotation.value())))
    }
}

impl Serializer for BibtexSerializer {
    type Error = BibtexOutputFailure;

    fn serialize(
        &self,
        context: OutputContext<'_>,
        request: &OutputRequest,
    ) -> Result<GeneratedFile, Self::Error> {
        if request.format() != OutputFormat::Bibtex {
            return Err(failure(
                BibtexOutputFailureKind::WrongFormat,
                "BIB_BIBTEX_FORMAT",
                "the BibTeX serializer requires a BibTeX output request",
            ));
        }
        if context.document().configuration().version() != context.unicode().compatibility() {
            return Err(failure(
                BibtexOutputFailureKind::IncompatibleVersion,
                "BIB_BIBTEX_VERSION",
                "the processed document and Unicode tables are incompatible",
            ));
        }

        let mut text = String::new();
        for comment in self.options.comments.iter() {
            validate_comment(comment)?;
            writeln!(text, "% {comment}").expect("writing a String cannot fail");
        }
        if !self.options.comments.is_empty() {
            text.push('\n');
        }
        check_work_limit(&text, request.max_bytes())?;
        for value in self.options.macros.iter() {
            validate_identifier(value.name(), "macro name")?;
            validate_value(value.value(), "macro value")?;
            writeln!(
                text,
                "@STRING{{{} = \"{}\"}}\n",
                self.options.field_case.apply(value.name()),
                value.value()
            )
            .expect("writing a String cannot fail");
            check_work_limit(&text, request.max_bytes())?;
        }

        let mut emitted = BTreeSet::new();
        for section in context.document().sections() {
            if section.lists().len() == 0 {
                for entry in section.entries() {
                    if emitted.insert((section.id().get(), entry.id().as_str().to_owned())) {
                        self.write_entry(&mut text, entry)?;
                        check_work_limit(&text, request.max_bytes())?;
                    }
                }
                continue;
            }
            for list in section.lists() {
                for id in list.entries() {
                    if !emitted.insert((section.id().get(), id.as_str().to_owned())) {
                        continue;
                    }
                    let entry = section.entry(id).ok_or_else(|| {
                        failure(
                            BibtexOutputFailureKind::MalformedValue,
                            "BIB_BIBTEX_UNKNOWN_ENTRY",
                            &format!("data list references unknown entry `{id}`"),
                        )
                    })?;
                    self.write_entry(&mut text, entry)?;
                    check_work_limit(&text, request.max_bytes())?;
                }
            }
        }

        let text = TexRecoder::new(RecodeSet::Null, self.options.escape).encode(&text);
        let text = match request.newline() {
            OutputNewline::Lf => text,
            OutputNewline::CrLf => text.replace('\n', "\r\n"),
        };
        let bytes = encode_legacy(&text, request.encoding()).map_err(|error| match error {
            EncodingError::UnmappableCharacter => failure(
                BibtexOutputFailureKind::Unrepresentable,
                "BIB_BIBTEX_ENCODING",
                "BibTeX output contains a character unavailable in the requested encoding",
            ),
            EncodingError::UnknownLabel | EncodingError::MalformedInput => failure(
                BibtexOutputFailureKind::MalformedValue,
                "BIB_BIBTEX_ENCODING",
                "the requested BibTeX encoding is invalid",
            ),
        })?;
        if bytes.len() > request.max_bytes() {
            return Err(failure(
                BibtexOutputFailureKind::Limit,
                "BIB_BIBTEX_LIMIT",
                &format!(
                    "BibTeX output exceeds the {} byte limit",
                    request.max_bytes()
                ),
            ));
        }
        Ok(GeneratedFile::new(request.path().clone(), bytes))
    }
}

fn field_text(value: &FieldValue) -> Option<&str> {
    match value {
        FieldValue::Literal(value) => Some(value.as_str()),
        FieldValue::Verbatim(value) => Some(value.as_str()),
        _ => None,
    }
}

fn check_work_limit(value: &str, output_limit: usize) -> Result<(), BibtexOutputFailure> {
    let work_limit = output_limit.saturating_mul(32);
    if value.len() > work_limit {
        return Err(failure(
            BibtexOutputFailureKind::Limit,
            "BIB_BIBTEX_LIMIT",
            &format!("BibTeX output exceeds the {output_limit} byte limit"),
        ));
    }
    Ok(())
}

fn format_range(value: &Range) -> String {
    format!(
        "{}--{}",
        format_endpoint(value.start()),
        format_endpoint(value.end())
    )
}

fn format_endpoint(value: &RangeEndpoint) -> String {
    match value {
        RangeEndpoint::Integer(value) => value.to_string(),
        RangeEndpoint::Literal(value) => value.as_str().to_owned(),
        RangeEndpoint::Open => String::new(),
    }
}

fn format_date(value: &DateValue) -> String {
    let mut output = format!("{:04}", value.year());
    if let Some(month) = value.month() {
        write!(output, "-{month:02}").expect("writing a String cannot fail");
    }
    if let Some(day) = value.day() {
        write!(output, "-{day:02}").expect("writing a String cannot fail");
    }
    if value.is_uncertain() {
        output.push('?');
    }
    if value.is_approximate() {
        output.push('~');
    }
    output
}

fn validate_identifier(value: &str, what: &str) -> Result<(), BibtexOutputFailure> {
    if value.is_empty()
        || value
            .chars()
            .any(|character| character.is_control() || matches!(character, '{' | '}' | ',' | '='))
    {
        return Err(failure(
            BibtexOutputFailureKind::MalformedValue,
            "BIB_BIBTEX_IDENTIFIER",
            &format!("{what} cannot be represented safely in BibTeX"),
        ));
    }
    Ok(())
}

fn validate_value(value: &str, what: &str) -> Result<(), BibtexOutputFailure> {
    let mut depth = 0usize;
    let mut escaped = false;
    for character in value.chars() {
        if character == '\0' || (character.is_control() && !matches!(character, '\n' | '\t')) {
            return Err(malformed_value(what));
        }
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '{' {
            depth = depth.checked_add(1).ok_or_else(|| malformed_value(what))?;
        } else if character == '}' {
            depth = depth.checked_sub(1).ok_or_else(|| malformed_value(what))?;
        }
    }
    if depth != 0 {
        return Err(malformed_value(what));
    }
    Ok(())
}

fn validate_comment(value: &str) -> Result<(), BibtexOutputFailure> {
    if value.contains(['\n', '\r', '\0']) {
        return Err(malformed_value("comment"));
    }
    Ok(())
}

fn malformed_value(what: &str) -> BibtexOutputFailure {
    failure(
        BibtexOutputFailureKind::MalformedValue,
        "BIB_BIBTEX_VALUE",
        &format!("{what} is malformed"),
    )
}

fn failure(kind: BibtexOutputFailureKind, code: &str, message: &str) -> BibtexOutputFailure {
    BibtexOutputFailure {
        kind,
        diagnostics: Arc::from([DiagnosticBuilder::new(
            BibDiagnosticCode::new(code).expect("static output diagnostic code is valid"),
            BibSeverity::Error,
            message,
        )
        .expect("output diagnostic message is valid")
        .freeze()]),
    }
}
