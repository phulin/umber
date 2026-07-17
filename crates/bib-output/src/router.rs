use std::fmt;

use bib_model::{GeneratedFile, OutputFormat, OutputRequest};

use crate::{
    BblOutputFailure, BblSerializer, BblXmlSerializer, BibLatexXmlSerializer, BibtexOptions,
    BibtexOutputFailure, BibtexSerializer, DotOptions, DotOutputFailure, DotSerializer,
    OutputContext, Serializer, XmlOutputFailure,
};

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
    Bbl,
    Bibtex,
    Xml,
    Dot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OutputFailure {
    Bbl(BblOutputFailure),
    Bibtex(BibtexOutputFailure),
    Xml(XmlOutputFailure),
    Dot(DotOutputFailure),
}

impl OutputFailure {
    #[must_use]
    pub const fn kind(&self) -> OutputFailureKind {
        match self {
            Self::Bbl(_) => OutputFailureKind::Bbl,
            Self::Bibtex(_) => OutputFailureKind::Bibtex,
            Self::Xml(_) => OutputFailureKind::Xml,
            Self::Dot(_) => OutputFailureKind::Dot,
        }
    }
}

impl fmt::Display for OutputFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bbl(error) => error.fmt(formatter),
            Self::Bibtex(error) => error.fmt(formatter),
            Self::Xml(error) => error.fmt(formatter),
            Self::Dot(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for OutputFailure {}

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
        match request.format() {
            OutputFormat::Bbl => BblSerializer
                .serialize(context, request)
                .map_err(OutputFailure::Bbl),
            OutputFormat::Bibtex => BibtexSerializer::new(self.options.bibtex.clone())
                .serialize(context, request)
                .map_err(OutputFailure::Bibtex),
            OutputFormat::BibLatexXml => BibLatexXmlSerializer
                .serialize(context, request)
                .map_err(OutputFailure::Xml),
            OutputFormat::BblXml => BblXmlSerializer
                .serialize(context, request)
                .map_err(OutputFailure::Xml),
            OutputFormat::Dot => DotSerializer::new(self.options.dot)
                .serialize(context, request)
                .map_err(OutputFailure::Dot),
        }
    }
}
