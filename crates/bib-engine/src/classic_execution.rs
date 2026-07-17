//! Classic style execution after AUX control and resource discovery.
//!
//! This module is deliberately the seam between host-neutral resource lookup
//! and the pure compiler/database/VM workers.  It retains only immutable
//! compilation and prepared-database caches; a cache hit never changes the
//! detached artifacts or diagnostics returned to a caller.

use std::sync::Arc;

use bib_bst::{CompilationCache, CompileLimits};
use bib_input::parse_raw_bibtex_bytes;
use umber_vfs::{FileKind, FileRequest, FileRequestBatch, VfsSnapshot, VirtualPath};

use crate::{
    BibliographyAttempt, BibliographyDiagnostic, BibliographyDiagnosticCode, BibliographyDocument,
    BibliographyHistory, BibliographyResult, BibliographyStats, ClassicBibJob, ClassicControl,
    ClassicDatabaseCache, ClassicDatabaseDiagnostic, ClassicDiagnosticCode, ClassicVmDiagnostic,
    ClassicVmDiagnosticKind, ClassicVmLimits, GeneratedFile, execute_classic_style,
};

#[derive(Clone, Debug)]
pub(crate) struct ClassicExecutionSession {
    styles: CompilationCache,
    databases: ClassicDatabaseCache,
}

impl Default for ClassicExecutionSession {
    fn default() -> Self {
        Self::new()
    }
}

impl ClassicExecutionSession {
    pub(crate) fn new() -> Self {
        let limits = CompileLimits::default();
        Self {
            styles: CompilationCache::new(32, limits.retained_cache_bytes),
            databases: ClassicDatabaseCache::new(32),
        }
    }

    pub(crate) fn process(
        &mut self,
        job: &ClassicBibJob,
        control: ClassicControl,
        snapshot: &VfsSnapshot,
        control_session: &mut crate::classic::ClassicControlSession,
    ) -> BibliographyAttempt {
        let Some(style_name) = control.style() else {
            return self.finished_empty(control);
        };
        let style_request = FileRequest::new(
            crate::classic::request_key(
                FileKind::BibStyle,
                &crate::classic::default_extension(style_name, "bst"),
            ),
            style_name,
        );
        let mut requests = vec![style_request.clone()];
        requests.extend(control.databases().map(|name| {
            FileRequest::new(
                crate::classic::request_key(
                    FileKind::ClassicBibData,
                    &crate::classic::default_extension(name, "bib"),
                ),
                name,
            )
        }));
        let mut found = Vec::with_capacity(requests.len());
        let mut missing = Vec::new();
        for request in &requests {
            match crate::classic::locate(snapshot, None, request.key()) {
                Ok(Some(file)) => found.push(file),
                Ok(None) => missing.push(request.clone()),
                Err(error) => return BibliographyAttempt::Failed(error.into_failure()),
            }
        }
        if !missing.is_empty() {
            return control_session.need(job, FileRequestBatch::new(missing, []));
        }
        let style_file = found[0];
        let compile = self
            .styles
            .compile(style_file.bytes(), CompileLimits::default());
        let mut diagnostics = compile
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic_message("BST_COMPILE", diagnostic.message().to_owned()))
            .collect::<Vec<_>>();
        let Some(style) = compile.program() else {
            return self.fatal(control, diagnostics, Vec::new(), Vec::new());
        };
        let sources = found[1..]
            .iter()
            .map(|file| {
                let raw = parse_raw_bibtex_bytes(
                    file.bytes(),
                    job.options().database_options().bibtex_options(),
                );
                (file, raw)
            })
            .collect::<Vec<_>>();
        let database_sources = sources
            .iter()
            .map(|(file, raw)| {
                crate::ClassicDatabaseSource::new(file.path(), file.content_id(), raw)
            })
            .collect::<Vec<_>>();
        let database = self.databases.prepare(
            &control,
            style,
            &database_sources,
            job.options().database_options(),
        );
        diagnostics.extend(database.diagnostics().map(database_diagnostic));
        let vm = execute_classic_style(style, &database, ClassicVmLimits::default());
        diagnostics.extend(vm.diagnostics().iter().map(vm_diagnostic));
        let bbl_path = output_path(job.aux_path(), "bbl");
        let blg_path = output_path(job.aux_path(), "blg");
        let bbl = vm.partial_bbl().as_bytes().to_vec();
        let blg = render_log(&control, &database, &vm);
        if vm.is_fatal() {
            return self.fatal(
                control,
                diagnostics,
                vec![GeneratedFile::new(bbl_path, Arc::<[u8]>::from(bbl))],
                vec![GeneratedFile::new(blg_path, Arc::<[u8]>::from(blg))],
            );
        }
        let history = if diagnostics.is_empty() {
            BibliographyHistory::Spotless
        } else {
            BibliographyHistory::Warning
        };
        BibliographyAttempt::Finished(
            BibliographyResult::new(
                history,
                BibliographyDocument::Classic(Arc::new(crate::ClassicBibliography::from_control(
                    &control,
                ))),
                [
                    GeneratedFile::new(bbl_path, Arc::<[u8]>::from(bbl)),
                    GeneratedFile::new(blg_path, Arc::<[u8]>::from(blg)),
                ],
                [],
                diagnostics,
                BibliographyStats::Classic(Default::default()),
            )
            .expect("classic execution artifacts have distinct names"),
        )
    }

    fn finished_empty(&self, control: ClassicControl) -> BibliographyAttempt {
        BibliographyAttempt::Finished(
            BibliographyResult::new(
                BibliographyHistory::Spotless,
                BibliographyDocument::Classic(Arc::new(crate::ClassicBibliography::from_control(
                    &control,
                ))),
                [],
                [],
                [],
                BibliographyStats::Classic(Default::default()),
            )
            .expect("empty classic result is valid"),
        )
    }

    fn fatal(
        &self,
        control: ClassicControl,
        diagnostics: Vec<BibliographyDiagnostic>,
        bbl: Vec<GeneratedFile>,
        blg: Vec<GeneratedFile>,
    ) -> BibliographyAttempt {
        BibliographyAttempt::Finished(
            BibliographyResult::new(
                BibliographyHistory::Fatal,
                BibliographyDocument::Classic(Arc::new(crate::ClassicBibliography::from_control(
                    &control,
                ))),
                [],
                bbl.into_iter().chain(blg).collect::<Vec<_>>(),
                diagnostics,
                BibliographyStats::Classic(Default::default()),
            )
            .expect("fatal classic artifacts are detached"),
        )
    }
}

fn output_path(aux: &VirtualPath, extension: &str) -> VirtualPath {
    let raw = aux
        .as_str()
        .strip_prefix("/job/")
        .expect("classic AUX is a user path");
    let stem = raw.rsplit_once('.').map_or(raw, |(stem, _)| stem);
    VirtualPath::user(&format!("{stem}.{extension}")).expect("AUX-derived output is valid")
}

fn diagnostic_message(code: &str, message: String) -> BibliographyDiagnostic {
    BibliographyDiagnostic::new(
        crate::BibSeverity::Warning,
        BibliographyDiagnosticCode::Classic(
            ClassicDiagnosticCode::new(code).expect("fixed classic diagnostic code is valid"),
        ),
        message,
        None,
    )
}

fn database_diagnostic(diagnostic: &ClassicDatabaseDiagnostic) -> BibliographyDiagnostic {
    diagnostic_message("CLASSIC_READ", diagnostic.message().to_owned())
}

fn vm_diagnostic(diagnostic: &ClassicVmDiagnostic) -> BibliographyDiagnostic {
    let code = match diagnostic.kind() {
        ClassicVmDiagnosticKind::Underflow => "CLASSIC_VM_UNDERFLOW",
        ClassicVmDiagnosticKind::WrongType => "CLASSIC_VM_TYPE",
        ClassicVmDiagnosticKind::NoCurrentEntry => "CLASSIC_VM_ENTRY",
        ClassicVmDiagnosticKind::InvalidFunction => "CLASSIC_VM_FUNCTION",
        ClassicVmDiagnosticKind::Limit => "CLASSIC_VM_LIMIT",
        ClassicVmDiagnosticKind::Arithmetic => "CLASSIC_VM_ARITHMETIC",
    };
    diagnostic_message(code, diagnostic.message().to_owned())
}

fn render_log(
    control: &ClassicControl,
    database: &crate::ClassicDatabase,
    vm: &crate::ClassicVmResult,
) -> Vec<u8> {
    let mut log = String::from("This is Umber classic BibTeX compatibility mode\n");
    if let Some(style) = control.style() {
        log.push_str("The style file: ");
        log.push_str(style);
        log.push_str(".bst\n");
    }
    for (index, database_name) in control.databases().enumerate() {
        log.push_str(&format!(
            "Database file #{}: {}.bib\n",
            index + 1,
            database_name
        ));
    }
    for diagnostic in database.diagnostics() {
        log.push_str("Warning--");
        log.push_str(diagnostic.message());
        log.push('\n');
    }
    log.push_str(vm.partial_blg());
    log.into_bytes()
}
