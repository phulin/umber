use std::fmt;
use std::sync::Arc;

use crate::{
    BibAttempt, BibDiagnostic, BibFailureKind, BibJob, BibOptionsBuilder, BibResult, BibSession,
    BibSeverity, BibtexOptions, DotInclude, DotOptions, FileProvisioner, LegacyEncoding,
    OutputFormat, OutputNewline, OutputOptions, OutputRequest, ResolvedFile, VfsSnapshot,
    VirtualPath, process_once,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BibExitStatus {
    Success,
    OperationalFailure,
    InvalidInvocation,
    /// Classic BibTeX completed with recoverable execution errors.
    ClassicExecutionError,
}

impl BibExitStatus {
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::OperationalFailure => 1,
            Self::InvalidInvocation | Self::ClassicExecutionError => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BibCommandMode {
    #[default]
    Process,
    Tool,
    ValidateControl,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibCommand {
    input: VirtualPath,
    job: BibJob,
    mode: BibCommandMode,
    log_enabled: bool,
}

impl BibCommand {
    pub fn parse(args: impl IntoIterator<Item = impl AsRef<str>>) -> Result<Self, BibCommandError> {
        let args = args
            .into_iter()
            .map(|arg| arg.as_ref().to_owned())
            .collect::<Vec<_>>();
        let mut input = None;
        let mut mode = BibCommandMode::Process;
        let mut configuration = None;
        let mut no_configuration = false;
        let mut output_file = None;
        let mut output_format = None;
        let mut output_encoding = LegacyEncoding::Utf8;
        let mut output_newline = OutputNewline::Lf;
        let mut output_alignment = false;
        let mut dot_include = DotInclude::default();
        let mut log_enabled = true;
        let mut index = 0;
        while index < args.len() {
            let argument = &args[index];
            let (name, inline) = argument
                .split_once('=')
                .map_or((argument.as_str(), None), |(name, value)| {
                    (name, Some(value))
                });
            match name {
                "--tool" => set_mode(&mut mode, BibCommandMode::Tool)?,
                "--validate-control" => set_mode(&mut mode, BibCommandMode::ValidateControl)?,
                "--nolog" => log_enabled = false,
                "--noconf" => no_configuration = true,
                "--configfile" | "--config-file" => {
                    configuration = Some(next_value(&args, &mut index, name, inline)?);
                }
                "--output-file" => {
                    output_file = Some(next_value(&args, &mut index, name, inline)?);
                }
                "--output-format" => {
                    output_format =
                        Some(parse_format(&next_value(&args, &mut index, name, inline)?)?);
                }
                "--output-encoding" => {
                    let value = next_value(&args, &mut index, name, inline)?;
                    output_encoding = LegacyEncoding::for_label(&value).map_err(|_| {
                        BibCommandError::new(format!("unknown output encoding `{value}`"))
                    })?;
                }
                "--output-newline" => {
                    output_newline = match next_value(&args, &mut index, name, inline)?.as_str() {
                        "lf" => OutputNewline::Lf,
                        "crlf" => OutputNewline::CrLf,
                        value => {
                            return Err(BibCommandError::new(format!(
                                "unknown output newline `{value}`"
                            )));
                        }
                    };
                }
                "--output-align" => output_alignment = true,
                "--dot-include" | "-dot-include" => {
                    dot_include = parse_dot_include(&next_value(&args, &mut index, name, inline)?)?;
                }
                flag if flag.starts_with('-') => {
                    return Err(BibCommandError::new(format!("unknown option `{flag}`")));
                }
                path => {
                    if inline.is_some() || input.is_some() {
                        return Err(BibCommandError::new("bib accepts exactly one input path"));
                    }
                    input = Some(path.to_owned());
                }
            }
            index += 1;
        }
        let input = input.ok_or_else(|| BibCommandError::new("missing bibliography input path"))?;
        let input = command_path(&input)
            .map_err(|error| BibCommandError::new(format!("invalid input path: {error}")))?;
        let format = output_format.unwrap_or(match mode {
            BibCommandMode::Tool => OutputFormat::Bibtex,
            BibCommandMode::Process | BibCommandMode::ValidateControl => OutputFormat::Bbl,
        });
        let output = output_file.unwrap_or_else(|| default_output_name(&input, mode, format));
        let output = command_path(&output)
            .map_err(|error| BibCommandError::new(format!("invalid output path: {error}")))?;
        if input == output {
            return Err(BibCommandError::new("input and output paths must differ"));
        }
        let mut options = BibOptionsBuilder::new();
        options.tool_mode(mode == BibCommandMode::Tool);
        options.output_options(
            OutputOptions::default()
                .with_bibtex(BibtexOptions::default().with_alignment(output_alignment))
                .with_dot(DotOptions::default().with_include(dot_include)),
        );
        if mode != BibCommandMode::ValidateControl {
            options
                .output(
                    OutputRequest::new(output, format)
                        .with_encoding(output_encoding)
                        .with_newline(output_newline),
                )
                .map_err(|error| BibCommandError::new(error.to_string()))?;
        }
        if let Some(path) = configuration.filter(|_| !no_configuration) {
            options.configuration(command_path(&path).map_err(|error| {
                BibCommandError::new(format!("invalid configuration path: {error}"))
            })?);
        }
        let control_path = if mode == BibCommandMode::Tool {
            VirtualPath::user(".umber/tool.bcf").expect("fixed tool control path is valid")
        } else {
            input.clone()
        };
        Ok(Self {
            input: input.clone(),
            job: BibJob::new(control_path, options.freeze()),
            mode,
            log_enabled,
        })
    }

    #[must_use]
    pub const fn input(&self) -> &VirtualPath {
        &self.input
    }

    #[must_use]
    pub const fn job(&self) -> &BibJob {
        &self.job
    }

    #[must_use]
    pub const fn mode(&self) -> BibCommandMode {
        self.mode
    }

    #[must_use]
    pub fn tool_control(&self) -> Option<Vec<u8>> {
        (self.mode == BibCommandMode::Tool).then(|| {
            let datasource = self.input.as_str();
            format!(
                r#"<bcf:controlfile version="3.11" bltxversion="3.21" xmlns:bcf="https://sourceforge.net/projects/biblatex">
  <bcf:bibdata section="0"><bcf:datasource type="file" datatype="bibtex">{datasource}</bcf:datasource></bcf:bibdata>
  <bcf:section number="0"><bcf:citekey>*</bcf:citekey></bcf:section>
</bcf:controlfile>"#,
            )
            .into_bytes()
        })
    }

    #[must_use]
    pub fn execute(&self, snapshot: &VfsSnapshot) -> BibCommandOutput {
        self.finish(process_once(&self.job, snapshot))
    }

    pub fn execute_provisioned(
        &self,
        files: &mut FileProvisioner,
        mut resolve: impl FnMut(&crate::FileRequest) -> Option<ResolvedFile>,
    ) -> BibCommandOutput {
        let mut session = BibSession::default();
        loop {
            let attempt = session.process(&self.job, &files.snapshot());
            let BibAttempt::NeedResources(resources) = attempt else {
                return self.finish(attempt);
            };
            files.expect(&resources);
            let mut responses = Vec::new();
            for request in &resources.required {
                let Some(response) = resolve(request) else {
                    return self.finish(BibAttempt::NeedResources(resources));
                };
                responses.push(response);
            }
            if let Err(error) = files.provision_batch(responses) {
                return self.failure(format!("ERROR - {error}\n").into_bytes());
            }
        }
    }

    fn finish(&self, attempt: BibAttempt) -> BibCommandOutput {
        match attempt {
            BibAttempt::Complete(result) => self.complete(result),
            BibAttempt::NeedResources(resources) => {
                let names = resources
                    .required
                    .iter()
                    .map(|request| request.original_name())
                    .collect::<Vec<_>>()
                    .join(", ");
                self.failure(
                    format!("ERROR - Missing required resource(s): {names}\n").into_bytes(),
                )
            }
            BibAttempt::Failed(failure) => {
                let terminal = render_diagnostics(failure.diagnostics());
                let status = if failure.kind() == BibFailureKind::InvalidInvocation {
                    BibExitStatus::InvalidInvocation
                } else {
                    BibExitStatus::OperationalFailure
                };
                BibCommandOutput::new(status, terminal.clone(), self.log(terminal), None)
            }
        }
    }

    fn complete(&self, result: BibResult) -> BibCommandOutput {
        let mut terminal = render_diagnostics(result.diagnostics());
        let stats = result.stats();
        terminal.extend_from_slice(
            format!(
                "INFO - Bibliography complete: {} section(s), {} entries, {} file(s)\n",
                stats.sections(),
                stats.entries(),
                stats.generated_files()
            )
            .as_bytes(),
        );
        BibCommandOutput::new(
            BibExitStatus::Success,
            terminal.clone(),
            self.log(terminal),
            Some(result),
        )
    }

    fn failure(&self, terminal: Vec<u8>) -> BibCommandOutput {
        BibCommandOutput::new(
            BibExitStatus::OperationalFailure,
            terminal.clone(),
            self.log(terminal),
            None,
        )
    }

    fn log(&self, terminal: Vec<u8>) -> Vec<u8> {
        if self.log_enabled {
            terminal
        } else {
            Vec::new()
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibCommandOutput {
    status: BibExitStatus,
    terminal: Arc<[u8]>,
    log: Arc<[u8]>,
    result: Option<BibResult>,
}

impl BibCommandOutput {
    fn new(
        status: BibExitStatus,
        terminal: Vec<u8>,
        log: Vec<u8>,
        result: Option<BibResult>,
    ) -> Self {
        Self {
            status,
            terminal: terminal.into(),
            log: log.into(),
            result,
        }
    }

    #[must_use]
    pub const fn status(&self) -> BibExitStatus {
        self.status
    }

    #[must_use]
    pub fn terminal(&self) -> &[u8] {
        &self.terminal
    }

    #[must_use]
    pub fn log(&self) -> &[u8] {
        &self.log
    }

    #[must_use]
    pub const fn result(&self) -> Option<&BibResult> {
        self.result.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibCommandError {
    message: Arc<str>,
}

impl BibCommandError {
    fn new(message: impl Into<Arc<str>>) -> Self {
        Self {
            message: message.into(),
        }
    }

    #[must_use]
    pub fn output(&self) -> BibCommandOutput {
        BibCommandOutput::new(
            BibExitStatus::InvalidInvocation,
            format!("ERROR - {}\n", self.message).into_bytes(),
            Vec::new(),
            None,
        )
    }
}

impl fmt::Display for BibCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for BibCommandError {}

fn set_mode(current: &mut BibCommandMode, next: BibCommandMode) -> Result<(), BibCommandError> {
    if *current != BibCommandMode::Process {
        return Err(BibCommandError::new("bib accepts only one mode option"));
    }
    *current = next;
    Ok(())
}

fn next_value(
    args: &[String],
    index: &mut usize,
    name: &str,
    inline: Option<&str>,
) -> Result<String, BibCommandError> {
    if let Some(value) = inline {
        if value.is_empty() {
            return Err(BibCommandError::new(format!("missing value for {name}")));
        }
        return Ok(value.to_owned());
    }
    *index += 1;
    args.get(*index)
        .filter(|value| !value.starts_with('-'))
        .cloned()
        .ok_or_else(|| BibCommandError::new(format!("missing value for {name}")))
}

fn parse_format(value: &str) -> Result<OutputFormat, BibCommandError> {
    match value.to_ascii_lowercase().replace('_', "-").as_str() {
        "bbl" => Ok(OutputFormat::Bbl),
        "bibtex" | "bib" => Ok(OutputFormat::Bibtex),
        "biblatexml" | "biblatex-xml" => Ok(OutputFormat::BibLatexXml),
        "bblxml" | "bbl-xml" => Ok(OutputFormat::BblXml),
        "dot" => Ok(OutputFormat::Dot),
        _ => Err(BibCommandError::new(format!(
            "unknown output format `{value}`"
        ))),
    }
}

fn parse_dot_include(value: &str) -> Result<DotInclude, BibCommandError> {
    let mut include = DotInclude {
        sections: false,
        fields: false,
        xdata: false,
        crossrefs: false,
        xrefs: false,
        related: false,
    };
    for item in value.split(',') {
        match item {
            "section" => include.sections = true,
            "field" => include.fields = true,
            "xdata" => include.xdata = true,
            "crossref" => include.crossrefs = true,
            "xref" => include.xrefs = true,
            "related" => include.related = true,
            _ => {
                return Err(BibCommandError::new(format!(
                    "unknown DOT inclusion `{item}`"
                )));
            }
        }
    }
    Ok(include)
}

fn default_output_name(input: &VirtualPath, mode: BibCommandMode, format: OutputFormat) -> String {
    let path = input
        .as_str()
        .strip_prefix("/job/")
        .unwrap_or(input.as_str());
    let (stem, _) = path.rsplit_once('.').unwrap_or((path, ""));
    let suffix = match format {
        OutputFormat::Bbl => "bbl",
        OutputFormat::Bibtex => "bib",
        OutputFormat::BibLatexXml => "bltxml",
        OutputFormat::BblXml => "bblxml",
        OutputFormat::Dot => "dot",
    };
    if mode == BibCommandMode::Tool {
        format!("{stem}_bibertool.{suffix}")
    } else {
        format!("{stem}.{suffix}")
    }
}

fn command_path(value: &str) -> Result<VirtualPath, umber_vfs::VirtualPathError> {
    if value.starts_with('/') && !value.starts_with("/job/") {
        let name = std::path::Path::new(value)
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| umber_vfs::VirtualPathError::new("path has no UTF-8 file name"))?;
        VirtualPath::user(name)
    } else {
        VirtualPath::user(value)
    }
}

fn render_diagnostics<'a>(diagnostics: impl Iterator<Item = &'a BibDiagnostic>) -> Vec<u8> {
    let mut rendered = Vec::new();
    for diagnostic in diagnostics {
        let severity = match diagnostic.severity() {
            BibSeverity::Info => "INFO",
            BibSeverity::Warning => "WARN",
            BibSeverity::Error => "ERROR",
        };
        rendered.extend_from_slice(format!("{severity} - {}\n", diagnostic.message()).as_bytes());
    }
    rendered
}

#[cfg(test)]
mod tests;
