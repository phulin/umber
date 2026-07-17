//! Classic style execution after AUX control and resource discovery.
//!
//! This module is deliberately the seam between host-neutral resource lookup
//! and the pure compiler/database/VM workers.  It retains only immutable
//! compilation and prepared-database caches; a cache hit never changes the
//! detached artifacts or diagnostics returned to a caller.

use std::sync::Arc;

use bib_bst::{ClassicStringPool, CompilationCache, CompileLimits, CompiledStyle, Instruction};
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
            .map(|diagnostic| {
                diagnostic_message(
                    "BST_COMPILE",
                    crate::BibSeverity::Warning,
                    diagnostic.message().to_owned(),
                    None,
                )
            })
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
        let bbl = wrap_bbl(vm.partial_bbl());
        let blg = render_log(&control, style, &database, &vm);
        if vm.is_fatal() {
            return self.fatal(
                control,
                diagnostics,
                vec![GeneratedFile::new(bbl_path, Arc::<[u8]>::from(bbl))],
                vec![GeneratedFile::new(blg_path, Arc::<[u8]>::from(blg))],
            );
        }
        let history = if diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity() == crate::BibSeverity::Error)
        {
            BibliographyHistory::Error
        } else if diagnostics.is_empty() {
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

fn wrap_bbl(raw: &str) -> Vec<u8> {
    const MIN_PRINT_LINE: usize = 3;
    const MAX_PRINT_LINE: usize = 79;

    let mut output = String::with_capacity(raw.len());
    for raw_line in raw.split_inclusive('\n') {
        let mut line = raw_line.strip_suffix('\n').unwrap_or(raw_line).to_owned();
        loop {
            line = line.trim_end_matches(classic_output_whitespace).to_owned();
            if line.is_empty() {
                if raw_line.ends_with('\n') {
                    output.push('\n');
                }
                break;
            }
            if line.len() <= MAX_PRINT_LINE {
                output.push_str(&line);
                if raw_line.ends_with('\n') {
                    output.push('\n');
                }
                break;
            }
            let bytes = line.as_bytes();
            let backward_break = (MIN_PRINT_LINE..=MAX_PRINT_LINE)
                .rev()
                .find(|&at| classic_output_whitespace(bytes[at] as char));
            let break_at = backward_break.or_else(|| {
                (MAX_PRINT_LINE + 1..bytes.len())
                    .find(|&at| classic_output_whitespace(bytes[at] as char))
            });
            let Some(mut break_at) = break_at else {
                output.push_str(&line);
                if raw_line.ends_with('\n') {
                    output.push('\n');
                }
                break;
            };
            while break_at + 1 < bytes.len()
                && classic_output_whitespace(bytes[break_at + 1] as char)
            {
                break_at += 1;
            }
            output.push_str(line[..break_at].trim_end_matches(classic_output_whitespace));
            output.push('\n');
            line = format!("  {}", &line[break_at + 1..]);
        }
    }
    output.into_bytes()
}

fn classic_output_whitespace(character: char) -> bool {
    matches!(character, ' ' | '\t' | '\r' | '\u{000c}')
}

fn output_path(aux: &VirtualPath, extension: &str) -> VirtualPath {
    let raw = aux
        .as_str()
        .strip_prefix("/job/")
        .expect("classic AUX is a user path");
    let stem = raw.rsplit_once('.').map_or(raw, |(stem, _)| stem);
    VirtualPath::user(&format!("{stem}.{extension}")).expect("AUX-derived output is valid")
}

fn diagnostic_message(
    code: &str,
    severity: crate::BibSeverity,
    message: String,
    source: Option<crate::BibliographySourceLocation>,
) -> BibliographyDiagnostic {
    BibliographyDiagnostic::new(
        severity,
        BibliographyDiagnosticCode::Classic(
            ClassicDiagnosticCode::new(code).expect("fixed classic diagnostic code is valid"),
        ),
        message,
        source,
    )
}

fn database_diagnostic(diagnostic: &ClassicDatabaseDiagnostic) -> BibliographyDiagnostic {
    diagnostic_message(
        "CLASSIC_READ",
        crate::BibSeverity::Warning,
        diagnostic.message().to_owned(),
        diagnostic
            .source()
            .cloned()
            .map(crate::BibliographySourceLocation::Classic),
    )
}

fn vm_diagnostic(diagnostic: &ClassicVmDiagnostic) -> BibliographyDiagnostic {
    let code = match diagnostic.kind() {
        ClassicVmDiagnosticKind::Warning => "CLASSIC_VM_WARNING",
        ClassicVmDiagnosticKind::Underflow => "CLASSIC_VM_UNDERFLOW",
        ClassicVmDiagnosticKind::WrongType => "CLASSIC_VM_TYPE",
        ClassicVmDiagnosticKind::NoCurrentEntry => "CLASSIC_VM_ENTRY",
        ClassicVmDiagnosticKind::InvalidFunction => "CLASSIC_VM_FUNCTION",
        ClassicVmDiagnosticKind::Limit => "CLASSIC_VM_LIMIT",
        ClassicVmDiagnosticKind::Arithmetic => "CLASSIC_VM_ARITHMETIC",
    };
    let severity = if diagnostic.kind() == ClassicVmDiagnosticKind::Underflow {
        crate::BibSeverity::Error
    } else {
        crate::BibSeverity::Warning
    };
    diagnostic_message(code, severity, diagnostic.message().to_owned(), None)
}

fn render_log(
    control: &ClassicControl,
    style: &CompiledStyle,
    database: &crate::ClassicDatabase,
    vm: &crate::ClassicVmResult,
) -> Vec<u8> {
    let mut log = String::from("This is BibTeX, Version 0.99d (TeX Live 2025)\n");
    log.push_str("Capacity: max_strings=200000, hash_size=200000, hash_prime=170003\n");
    render_control_header(&mut log, control);
    for event in style.web2c_reallocations() {
        log.push_str(&format!(
            "Reallocated {} (elt_size={}) to {} items from {}.\n",
            event.array(),
            event.element_size(),
            event.new_capacity(),
            event.old_capacity(),
        ));
    }
    for diagnostic in database.diagnostics() {
        render_warning(&mut log, diagnostic.message(), diagnostic.source());
    }
    for event in vm.log_events() {
        match event {
            crate::classic_vm::ClassicVmLogEvent::Stack(value) => {
                log.push_str(value);
                log.push('\n');
            }
            crate::classic_vm::ClassicVmLogEvent::Diagnostic(diagnostic) => {
                render_vm_diagnostic(&mut log, diagnostic, control)
            }
        }
    }
    let entries = database.entries().len();
    let wiz_locations = wiz_defined_locations(style);
    let (strings, characters) = classic_string_usage(control, style, database);
    log.push_str(&format!(
        "You've used {entries} entr{},\n",
        if entries == 1 { "y" } else { "ies" }
    ));
    log.push_str(&format!(
        "            {wiz_locations} wiz_defined-function locations,\n"
    ));
    log.push_str(&format!(
        "            {strings} strings with {characters} characters,\n"
    ));
    let calls = vm.builtin_calls().iter().sum::<usize>();
    log.push_str(&format!(
        "and the built_in function-call counts, {calls} in all, are:\n"
    ));
    for ((_, name), calls) in crate::classic_vm::CLASSIC_BUILTINS
        .iter()
        .zip(vm.builtin_calls())
    {
        log.push_str(&format!("{name} -- {calls}\n"));
    }
    let errors = vm
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.kind() == ClassicVmDiagnosticKind::Underflow)
        .count();
    if errors == 0 {
        render_history(
            &mut log,
            database.diagnostics().len() + vm.diagnostics().len(),
        );
    } else {
        render_error_history(&mut log, errors);
    }
    log.into_bytes()
}

fn render_vm_diagnostic(
    log: &mut String,
    diagnostic: &ClassicVmDiagnostic,
    control: &ClassicControl,
) {
    if diagnostic.kind() != ClassicVmDiagnosticKind::Underflow {
        render_warning(log, diagnostic.message(), None);
        return;
    }
    log.push_str("You can't pop an empty literal stack");
    if let Some(entry) = diagnostic.entry() {
        log.push_str(" for entry ");
        log.push_str(entry);
    }
    log.push('\n');
    if let (Some(line), Some(style)) = (
        diagnostic.source().map(|source| source.line()),
        control.style(),
    ) {
        log.push_str(&format!("while executing---line {line} of file {style}"));
        if !style.ends_with(".bst") {
            log.push_str(".bst");
        }
        log.push('\n');
    }
}

pub(crate) fn render_control_header(log: &mut String, control: &ClassicControl) {
    if let Some(aux) = control.aux_files().next() {
        log.push_str("The top-level auxiliary file: ");
        log.push_str(file_name(aux));
        log.push('\n');
    }
    if let Some(style) = control.style() {
        log.push_str("The style file: ");
        log.push_str(style);
        if !style.ends_with(".bst") {
            log.push_str(".bst");
        }
        log.push('\n');
    }
    for (index, database_name) in control.databases().enumerate() {
        log.push_str(&format!("Database file #{}: {database_name}", index + 1));
        if !database_name.ends_with(".bib") {
            log.push_str(".bib");
        }
        log.push('\n');
    }
}

pub(crate) fn render_warning(
    output: &mut String,
    message: &str,
    source: Option<&crate::ClassicSourceLocation>,
) {
    output.push_str("Warning--");
    output.push_str(message);
    output.push('\n');
    if let Some(source) = source
        && let Some(line) = source.line()
    {
        output.push_str(&format!(
            "--line {line} of file {}\n",
            file_name(source.path())
        ));
    }
}

pub(crate) fn render_history(output: &mut String, warnings: usize) {
    match warnings {
        0 => {}
        1 => output.push_str("(There was 1 warning)\n"),
        count => output.push_str(&format!("(There were {count} warnings)\n")),
    }
}

pub(crate) fn render_error_history(output: &mut String, errors: usize) {
    match errors {
        0 => {}
        1 => output.push_str("(There was 1 error message)\n"),
        count => output.push_str(&format!("(There were {count} error messages)\n")),
    }
}

fn file_name(path: &VirtualPath) -> &str {
    path.as_str().rsplit('/').next().unwrap_or(path.as_str())
}

fn wiz_defined_locations(style: &CompiledStyle) -> usize {
    style
        .functions()
        .iter()
        .filter(|function| {
            !function.name().starts_with("<builtin:") && !function.name().starts_with("<read:")
        })
        .map(|function| {
            1 + function
                .instructions()
                .iter()
                .map(|instruction| {
                    usize::from(matches!(
                        instruction,
                        Instruction::PushFunction(_) | Instruction::Assign(_)
                    )) + 1
                })
                .sum::<usize>()
        })
        .sum()
}

fn classic_string_usage(
    control: &ClassicControl,
    style: &CompiledStyle,
    database: &crate::ClassicDatabase,
) -> (usize, usize) {
    let mut pool = ClassicStringPool::web2c();
    for (index, aux) in control.aux_files().enumerate() {
        let name = file_name(aux);
        if index == 0 {
            // The command-line job name and its opened top-level AUX file
            // have separate hash ilks, but retain distinct pool strings.
            let base = name.strip_suffix(".aux").unwrap_or(name);
            let _ = pool.intern(base);
        }
        let _ = pool.intern(name);
    }
    if let Some(style_name) = control.style() {
        let _ = pool.intern(style_name);
    }
    for database_name in control.databases() {
        let _ = pool.intern(database_name);
    }
    for citation in control.citations() {
        // `*` switches BibTeX to whole-database inclusion; it is not itself
        // looked up. READ inserts the encountered database keys instead.
        if citation != "*" {
            let _ = pool.intern(citation);
        }
    }
    style.apply_pool_trace(&mut pool);
    database.apply_pool_trace(&mut pool);
    let usage = pool.usage();
    (usage.strings(), usage.characters())
}
