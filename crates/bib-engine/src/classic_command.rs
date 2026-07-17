//! In-process `bibtex`-compatible command adapter.

use std::fmt;
use std::sync::Arc;

use crate::{
    BibExitStatus, BibliographyAttempt, BibliographyDocument, BibliographyHistory,
    BibliographyResult, BibliographySession, BibliographySourceLocation, ClassicBibJob,
    ClassicBibOptions, FileProvisioner, ResolvedFile, VfsSnapshot, VirtualPath,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassicBibCommand {
    aux: VirtualPath,
    job: ClassicBibJob,
}

impl ClassicBibCommand {
    pub fn parse(
        args: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Result<Self, ClassicBibCommandError> {
        let args = args
            .into_iter()
            .map(|arg| arg.as_ref().to_owned())
            .collect::<Vec<_>>();
        if args.len() != 1 || args[0].starts_with('-') {
            return Err(ClassicBibCommandError::new(
                "bibtex accepts exactly one AUX job name",
            ));
        }
        let input = command_path(&args[0])?;
        let aux = with_extension(&input, "aux");
        Ok(Self {
            job: ClassicBibJob::new(aux.clone(), ClassicBibOptions::default()),
            aux,
        })
    }

    #[must_use]
    pub const fn aux_path(&self) -> &VirtualPath {
        &self.aux
    }

    #[must_use]
    pub const fn job(&self) -> &ClassicBibJob {
        &self.job
    }

    #[must_use]
    pub fn execute(&self, snapshot: &VfsSnapshot) -> ClassicBibCommandOutput {
        let mut session = BibliographySession::classic();
        self.finish(session.process(&crate::BibliographyJob::Classic(self.job.clone()), snapshot))
    }

    pub fn execute_provisioned(
        &self,
        files: &mut FileProvisioner,
        mut resolve: impl FnMut(&crate::FileRequest) -> Option<ResolvedFile>,
    ) -> ClassicBibCommandOutput {
        let mut session = BibliographySession::classic();
        loop {
            match session.process(
                &crate::BibliographyJob::Classic(self.job.clone()),
                &files.snapshot(),
            ) {
                BibliographyAttempt::NeedResources(resources) => {
                    files.expect(&resources);
                    let mut responses = Vec::new();
                    for request in &resources.required {
                        let Some(response) = resolve(request) else {
                            return ClassicBibCommandOutput::failure(format!(
                                "I couldn't open database file {}\n",
                                request.original_name()
                            ));
                        };
                        responses.push(response);
                    }
                    if let Err(error) = files.provision_batch(responses) {
                        return ClassicBibCommandOutput::failure(format!("{error}\n"));
                    }
                }
                attempt => return self.finish(attempt),
            }
        }
    }

    fn finish(&self, attempt: BibliographyAttempt) -> ClassicBibCommandOutput {
        match attempt {
            BibliographyAttempt::Finished(result) => {
                let status = match result.history() {
                    BibliographyHistory::Spotless | BibliographyHistory::Warning => {
                        BibExitStatus::Success
                    }
                    BibliographyHistory::Error | BibliographyHistory::Fatal => {
                        BibExitStatus::OperationalFailure
                    }
                };
                let terminal = render_terminal(&result);
                ClassicBibCommandOutput::new(status, terminal, Some(result))
            }
            BibliographyAttempt::NeedResources(resources) => {
                ClassicBibCommandOutput::failure(format!(
                    "I couldn't open database file {}\n",
                    resources
                        .required
                        .first()
                        .map_or("<unknown>", crate::FileRequest::original_name)
                ))
            }
            BibliographyAttempt::Failed(failure) => {
                ClassicBibCommandOutput::failure(format!("BibTeX failed: {failure:?}\n"))
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassicBibCommandOutput {
    status: BibExitStatus,
    terminal: Arc<[u8]>,
    result: Option<BibliographyResult>,
}

impl ClassicBibCommandOutput {
    fn new(status: BibExitStatus, terminal: Vec<u8>, result: Option<BibliographyResult>) -> Self {
        Self {
            status,
            terminal: terminal.into(),
            result,
        }
    }

    fn failure(message: String) -> Self {
        Self::new(
            BibExitStatus::OperationalFailure,
            message.into_bytes(),
            None,
        )
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
    pub const fn result(&self) -> Option<&BibliographyResult> {
        self.result.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassicBibCommandError {
    message: Arc<str>,
}

impl ClassicBibCommandError {
    fn new(message: impl Into<Arc<str>>) -> Self {
        Self {
            message: message.into(),
        }
    }

    #[must_use]
    pub fn output(&self) -> ClassicBibCommandOutput {
        ClassicBibCommandOutput::new(
            BibExitStatus::InvalidInvocation,
            format!("{self}\n").into_bytes(),
            None,
        )
    }
}

impl fmt::Display for ClassicBibCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ClassicBibCommandError {}

fn command_path(value: &str) -> Result<VirtualPath, ClassicBibCommandError> {
    let name = if value.starts_with('/') && !value.starts_with("/job/") {
        std::path::Path::new(value)
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| ClassicBibCommandError::new("path has no UTF-8 file name"))?
    } else {
        value
    };
    VirtualPath::user(name).map_err(|error| ClassicBibCommandError::new(error.to_string()))
}

fn with_extension(path: &VirtualPath, extension: &str) -> VirtualPath {
    let raw = path
        .as_str()
        .strip_prefix("/job/")
        .expect("command paths are user paths");
    if raw
        .rsplit('/')
        .next()
        .is_some_and(|name| name.ends_with(&format!(".{extension}")))
    {
        return path.clone();
    }
    VirtualPath::user(&format!("{raw}.{extension}")).expect("command extension is valid")
}

fn render_terminal(result: &BibliographyResult) -> Vec<u8> {
    let mut terminal = String::from("This is BibTeX, Version 0.99d (TeX Live 2025)\n");
    let BibliographyDocument::Classic(document) = result.document() else {
        return terminal.into_bytes();
    };
    if let Some(aux) = document.aux_files().next() {
        terminal.push_str("The top-level auxiliary file: ");
        terminal.push_str(file_name(aux));
        terminal.push('\n');
    }
    if let Some(style) = document.style() {
        terminal.push_str("The style file: ");
        terminal.push_str(style);
        if !style.ends_with(".bst") {
            terminal.push_str(".bst");
        }
        terminal.push('\n');
    }
    for (index, database) in document.databases().enumerate() {
        terminal.push_str(&format!("Database file #{}: {database}", index + 1));
        if !database.ends_with(".bib") {
            terminal.push_str(".bib");
        }
        terminal.push('\n');
    }
    for diagnostic in result.diagnostics() {
        let source = match diagnostic.source() {
            Some(BibliographySourceLocation::Classic(source)) => Some(source),
            _ => None,
        };
        crate::classic_execution::render_warning(&mut terminal, diagnostic.message(), source);
    }
    if result.history() == BibliographyHistory::Fatal {
        terminal.push_str("(Fatal error)\n");
    } else {
        crate::classic_execution::render_history(&mut terminal, result.diagnostics().len());
    }
    terminal.into_bytes()
}

fn file_name(path: &VirtualPath) -> &str {
    path.as_str().rsplit('/').next().unwrap_or(path.as_str())
}

#[cfg(test)]
mod tests;
