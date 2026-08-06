use std::fmt;
use std::sync::Arc;

use bib_model::{
    BibDiagnostic, BibDiagnosticCode, BibSeverity, DiagnosticBuilder, GeneratedFile, OutputFormat,
    OutputNewline, OutputRequest, ProcessedBibliography,
};
use bib_unicode::{EncodingError, UnicodeData, encode_legacy};

use crate::{BibtexOptions, DotOptions, OutputContext, bbl, bibtex, dot, xml};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OutputOptions {
    bibtex: BibtexOptions,
    dot: DotOptions,
}

impl OutputOptions {
    #[must_use]
    pub fn with_bibtex(mut self, options: BibtexOptions) -> Self {
        self.bibtex = options;
        self
    }

    #[must_use]
    pub const fn with_dot(mut self, options: DotOptions) -> Self {
        self.dot = options;
        self
    }

    #[must_use]
    pub const fn bibtex(&self) -> &BibtexOptions {
        &self.bibtex
    }

    #[must_use]
    pub const fn dot(&self) -> DotOptions {
        self.dot
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputFailureKind {
    WrongFormat,
    IncompatibleVersion,
    MalformedValue,
    Unrepresentable,
    Limit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputFailure {
    format: OutputFormat,
    kind: OutputFailureKind,
    diagnostics: Arc<[BibDiagnostic]>,
}

impl OutputFailure {
    #[must_use]
    pub const fn format(&self) -> OutputFormat {
        self.format
    }

    #[must_use]
    pub const fn kind(&self) -> OutputFailureKind {
        self.kind
    }

    pub fn diagnostics(&self) -> impl ExactSizeIterator<Item = &BibDiagnostic> {
        self.diagnostics.iter()
    }
}

impl fmt::Display for OutputFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            self.diagnostics
                .first()
                .map_or("bibliography output failed", BibDiagnostic::message),
        )
    }
}

impl std::error::Error for OutputFailure {}

/// One closed serialization plan over a frozen document projection.
#[derive(Clone, Debug)]
pub struct OutputPlan<'a> {
    document: &'a ProcessedBibliography,
    unicode: &'a UnicodeData,
    request: &'a OutputRequest,
    options: &'a OutputOptions,
}

impl<'a> OutputPlan<'a> {
    fn new(
        context: OutputContext<'a>,
        request: &'a OutputRequest,
        options: &'a OutputOptions,
    ) -> Result<Self, OutputFailure> {
        let format = request.format();
        let compatibility = context.document().configuration().version();
        if compatibility != context.unicode().compatibility()
            || (format == OutputFormat::Bbl && compatibility.bbl_schema != "3.3")
        {
            return Err(version_failure(format));
        }

        Ok(Self {
            document: context.document(),
            unicode: context.unicode(),
            request,
            options,
        })
    }

    #[must_use]
    pub const fn document(&self) -> &'a ProcessedBibliography {
        self.document
    }

    #[must_use]
    pub const fn unicode(&self) -> &'a UnicodeData {
        self.unicode
    }

    #[must_use]
    pub const fn request(&self) -> &'a OutputRequest {
        self.request
    }

    #[must_use]
    pub const fn options(&self) -> &'a OutputOptions {
        self.options
    }

    pub(crate) fn sections(
        &self,
    ) -> impl ExactSizeIterator<Item = &'a bib_model::ProcessedSection> {
        self.document.sections()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OutputRouter {
    options: OutputOptions,
}

impl OutputRouter {
    #[must_use]
    pub fn new(options: OutputOptions) -> Self {
        Self { options }
    }

    #[must_use]
    pub const fn options(&self) -> &OutputOptions {
        &self.options
    }

    pub fn serialize(
        &self,
        context: OutputContext<'_>,
        request: &OutputRequest,
    ) -> Result<GeneratedFile, OutputFailure> {
        let plan = OutputPlan::new(context, request, &self.options)?;
        let mut sink = OutputSink::new(request);
        match request.format() {
            OutputFormat::Bbl => bbl::render(&plan, &mut sink),
            OutputFormat::Bibtex => bibtex::render(&plan, &mut sink),
            OutputFormat::BibLatexXml => xml::render_biblatex(&plan, &mut sink),
            OutputFormat::BblXml => xml::render_bbl(&plan, &mut sink),
            OutputFormat::Dot => dot::render(&plan, &mut sink),
        }?;
        sink.finish()
    }

    pub(crate) fn serialize_as(
        &self,
        expected: OutputFormat,
        context: OutputContext<'_>,
        request: &OutputRequest,
    ) -> Result<GeneratedFile, OutputFailure> {
        if request.format() != expected {
            return Err(wrong_format(expected));
        }
        self.serialize(context, request)
    }
}

pub(crate) fn failure(
    format: OutputFormat,
    kind: OutputFailureKind,
    code: &str,
    message: impl Into<String>,
) -> OutputFailure {
    OutputFailure {
        format,
        kind,
        diagnostics: Arc::from([DiagnosticBuilder::new(
            BibDiagnosticCode::new(code).expect("static output diagnostic code"),
            BibSeverity::Error,
            message,
        )
        .expect("output diagnostic message is valid")
        .freeze()]),
    }
}

pub(crate) struct OutputSink<'a> {
    request: &'a OutputRequest,
    text: String,
}

impl<'a> OutputSink<'a> {
    pub(crate) fn new(request: &'a OutputRequest) -> Self {
        Self {
            request,
            text: String::new(),
        }
    }

    pub(crate) fn push(&mut self, value: &str) -> Result<(), OutputFailure> {
        let multiplier = match self.request.format() {
            OutputFormat::Bibtex => 32,
            OutputFormat::Bbl | OutputFormat::BibLatexXml | OutputFormat::BblXml => 4,
            OutputFormat::Dot => 1,
        };
        let work_limit = self.request.max_bytes().saturating_mul(multiplier);
        let length = self
            .text
            .len()
            .checked_add(value.len())
            .ok_or_else(|| output_limit_failure(self.request))?;
        if length > work_limit {
            return Err(output_limit_failure(self.request));
        }
        self.text.push_str(value);
        Ok(())
    }

    pub(crate) fn line(&mut self, value: &str) -> Result<(), OutputFailure> {
        self.push(value)?;
        self.push("\n")
    }

    pub(crate) fn indented_line(
        &mut self,
        indent: usize,
        value: &str,
    ) -> Result<(), OutputFailure> {
        for _ in 0..indent {
            self.push("  ")?;
        }
        self.line(value)
    }

    pub(crate) fn finish(self) -> Result<GeneratedFile, OutputFailure> {
        let request = self.request;
        let mut text = self.text;
        if request.newline() == OutputNewline::CrLf {
            text = text.replace('\n', "\r\n");
        }
        let format = request.format();
        let bytes = encode_legacy(&text, request.encoding()).map_err(|error| match error {
            EncodingError::UnmappableCharacter => failure(
                format,
                OutputFailureKind::Unrepresentable,
                encoding_code(format),
                format!(
                    "{} output contains a character unavailable in the requested encoding",
                    format_label(format)
                ),
            ),
            EncodingError::UnknownLabel | EncodingError::MalformedInput => failure(
                format,
                OutputFailureKind::MalformedValue,
                encoding_code(format),
                format!("the requested {} encoding is invalid", format_label(format)),
            ),
        })?;
        if bytes.len() > request.max_bytes() {
            return Err(failure(
                format,
                OutputFailureKind::Limit,
                limit_code(format),
                format!(
                    "{} output exceeds the {} byte limit",
                    format_label(format),
                    request.max_bytes()
                ),
            ));
        }
        Ok(GeneratedFile::new(request.path().clone(), bytes))
    }
}

fn output_limit_failure(request: &OutputRequest) -> OutputFailure {
    let format = request.format();
    failure(
        format,
        OutputFailureKind::Limit,
        limit_code(format),
        format!(
            "{} output exceeds the {} byte limit",
            format_label(format),
            request.max_bytes()
        ),
    )
}

fn wrong_format(format: OutputFormat) -> OutputFailure {
    failure(
        format,
        OutputFailureKind::WrongFormat,
        match format {
            OutputFormat::Bbl => "BIB_OUTPUT_FORMAT",
            OutputFormat::Bibtex => "BIB_BIBTEX_FORMAT",
            OutputFormat::BibLatexXml | OutputFormat::BblXml => "BIB_XML_FORMAT",
            OutputFormat::Dot => "BIB_DOT_FORMAT",
        },
        format!(
            "the {} serializer requires a {} output request",
            format_label(format),
            format_label(format)
        ),
    )
}

fn version_failure(format: OutputFormat) -> OutputFailure {
    failure(
        format,
        OutputFailureKind::IncompatibleVersion,
        match format {
            OutputFormat::Bbl => "BIB_OUTPUT_VERSION",
            OutputFormat::Bibtex => "BIB_BIBTEX_VERSION",
            OutputFormat::BibLatexXml | OutputFormat::BblXml => "BIB_XML_VERSION",
            OutputFormat::Dot => "BIB_DOT_VERSION",
        },
        match format {
            OutputFormat::Bbl => {
                "the processed document and Unicode tables must use BBL schema 3.3"
            }
            OutputFormat::Bibtex => "the processed document and Unicode tables are incompatible",
            OutputFormat::BibLatexXml | OutputFormat::BblXml => {
                "the processed document and Unicode tables are incompatible"
            }
            OutputFormat::Dot => "the processed document and Unicode tables are incompatible",
        },
    )
}

const fn encoding_code(format: OutputFormat) -> &'static str {
    match format {
        OutputFormat::Bbl => "BIB_OUTPUT_ENCODING",
        OutputFormat::Bibtex => "BIB_BIBTEX_ENCODING",
        OutputFormat::BibLatexXml | OutputFormat::BblXml => "BIB_XML_ENCODING",
        OutputFormat::Dot => "BIB_DOT_ENCODING",
    }
}

const fn limit_code(format: OutputFormat) -> &'static str {
    match format {
        OutputFormat::Bbl => "BIB_OUTPUT_LIMIT",
        OutputFormat::Bibtex => "BIB_BIBTEX_LIMIT",
        OutputFormat::BibLatexXml | OutputFormat::BblXml => "BIB_XML_LIMIT",
        OutputFormat::Dot => "BIB_DOT_LIMIT",
    }
}

const fn format_label(format: OutputFormat) -> &'static str {
    match format {
        OutputFormat::Bbl => "BBL",
        OutputFormat::Bibtex => "BibTeX",
        OutputFormat::BibLatexXml => "BibLaTeXML",
        OutputFormat::BblXml => "BBLXML",
        OutputFormat::Dot => "DOT",
    }
}
