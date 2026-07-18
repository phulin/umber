use js_sys::{Array, Object, Reflect, Uint8Array};
use umber::{
    CompileAttemptResult, CompileDiagnostic, CompileError, LatexProjectAttempt,
    LatexProjectAttemptV2, LatexProjectError, LatexProjectOutput, LatexProjectOutputV2,
    MemoryRunOutput, ResourceRequest,
};
use wasm_bindgen::{JsCast, JsValue};

use crate::JsAttemptResult;
use crate::JsRenderedSourceResult;

pub(crate) fn attempt_result(result: CompileAttemptResult) -> Result<JsAttemptResult, JsValue> {
    let object = Object::new();
    match result {
        CompileAttemptResult::NeedResources(resources) => {
            set(&object, "kind", &JsValue::from_str("need-resources"))?;
            let required = resource_requests(resources.required)?;
            set(&object, "required", &required)?;
            let probes = resource_requests(resources.probes)?;
            set(&object, "probes", &probes)?;
            let hints = resource_requests(resources.prefetch_hints)?;
            set(&object, "prefetchHints", &hints)?;
        }
        CompileAttemptResult::Complete(output) => {
            set(&object, "kind", &JsValue::from_str("complete"))?;
            set(&object, "output", &compile_output(output)?)?;
        }
        CompileAttemptResult::Error(error) => {
            set(&object, "kind", &JsValue::from_str("error"))?;
            set(&object, "diagnostic", &diagnostic(error)?)?;
        }
    }
    Ok(object.unchecked_into())
}

pub(crate) fn project_attempt_result(
    result: LatexProjectAttempt,
) -> Result<JsAttemptResult, JsValue> {
    let object = Object::new();
    match result {
        LatexProjectAttempt::NeedResources(resources) => {
            set(&object, "kind", &JsValue::from_str("need-resources"))?;
            let required = resource_requests(resources.required)?;
            set(&object, "required", &required)?;
            let probes = resource_requests(resources.probes)?;
            set(&object, "probes", &probes)?;
            let hints = resource_requests(resources.prefetch_hints)?;
            set(&object, "prefetchHints", &hints)?;
        }
        LatexProjectAttempt::Complete(output) => {
            set(&object, "kind", &JsValue::from_str("complete"))?;
            set(&object, "output", &project_output(output)?)?;
        }
        LatexProjectAttempt::Error(error) => {
            set(&object, "kind", &JsValue::from_str("error"))?;
            set(&object, "diagnostic", &project_diagnostic(error)?)?;
        }
    }
    Ok(object.unchecked_into())
}

pub(crate) fn project_attempt_result_v2(
    result: LatexProjectAttemptV2,
) -> Result<JsAttemptResult, JsValue> {
    let object = Object::new();
    match result {
        LatexProjectAttemptV2::NeedResources(resources) => {
            set(&object, "kind", &JsValue::from_str("need-resources"))?;
            let required = resource_requests(resources.required)?;
            set(&object, "required", required.as_ref())?;
            let probes = resource_requests(resources.probes)?;
            set(&object, "probes", probes.as_ref())?;
            let hints = resource_requests(resources.prefetch_hints)?;
            set(&object, "prefetchHints", hints.as_ref())?;
        }
        LatexProjectAttemptV2::Complete(output) => {
            set(&object, "kind", &JsValue::from_str("complete"))?;
            set(&object, "output", &project_output_v2(*output)?)?;
        }
        LatexProjectAttemptV2::Error(error) => {
            set(&object, "kind", &JsValue::from_str("error"))?;
            set(&object, "diagnostic", &project_diagnostic(error)?)?;
        }
    }
    Ok(object.unchecked_into())
}

fn resource_requests(requests: Vec<ResourceRequest>) -> Result<Array, JsValue> {
    let result = Array::new();
    for request in requests {
        let object = Object::new();
        match request {
            ResourceRequest::File(request) => {
                set(&object, "type", &JsValue::from_str("file"))?;
                set(
                    &object,
                    "domain",
                    &JsValue::from_str(request.key().domain().wire_name()),
                )?;
                set(
                    &object,
                    "kind",
                    &JsValue::from_str(request.key().kind().wire_name()),
                )?;
                set(&object, "name", &JsValue::from_str(request.key().name()))?;
                set(
                    &object,
                    "originalName",
                    &JsValue::from_str(request.original_name()),
                )?;
            }
            ResourceRequest::Font(request) => {
                set(&object, "type", &JsValue::from_str("font"))?;
                set(
                    &object,
                    "logicalName",
                    &JsValue::from_str(request.key.logical_name()),
                )?;
                set(
                    &object,
                    "faceIndex",
                    &JsValue::from_f64(f64::from(request.key.face_index)),
                )?;
                let variations = Array::new();
                for coordinate in request.key.variation.coordinates() {
                    let value = Object::new();
                    set(
                        &value,
                        "tag",
                        &JsValue::from_str(&coordinate.tag.to_string()),
                    )?;
                    set(
                        &value,
                        "value",
                        &JsValue::from_f64(f64::from(coordinate.value)),
                    )?;
                    variations.push(&value);
                }
                set(&object, "variations", &variations)?;
                let features = Array::new();
                for setting in request.key.feature_policy.settings() {
                    let value = Object::new();
                    set(&value, "tag", &JsValue::from_str(&setting.tag.to_string()))?;
                    set(&value, "enabled", &JsValue::from_bool(setting.enabled))?;
                    features.push(&value);
                }
                set(&object, "features", &features)?;
                let accepted = Array::new();
                if request
                    .accepted_containers
                    .contains(umber::FontContainer::Woff2)
                {
                    accepted.push(&JsValue::from_str("woff2"));
                }
                set(&object, "acceptedContainers", &accepted)?;
            }
        }
        result.push(&object);
    }
    Ok(result)
}

fn compile_output(output: MemoryRunOutput) -> Result<JsValue, JsValue> {
    let object = Object::new();
    set(
        &object,
        "terminal",
        &JsValue::from_str(&String::from_utf8_lossy(&output.terminal)),
    )?;
    set(&object, "log", &typed_array(&output.log))?;
    set(&object, "dvi", &typed_array(&output.dvi))?;
    if let Some(html) = output.html {
        set(&object, "html", &typed_array(&html))?;
    }
    let html_assets = Array::new();
    for asset in output.html_assets {
        let file = Object::new();
        set(
            &file,
            "path",
            &JsValue::from_str(&asset.path.to_string_lossy()),
        )?;
        set(&file, "bytes", &typed_array(&asset.bytes))?;
        html_assets.push(&file);
    }
    set(&object, "htmlAssets", &html_assets)?;
    let files = Array::new();
    for output_file in output.files {
        let file = Object::new();
        set(
            &file,
            "path",
            &JsValue::from_str(&output_file.path.to_string_lossy()),
        )?;
        set(&file, "bytes", &typed_array(&output_file.bytes))?;
        files.push(&file);
    }
    set(&object, "files", &files)?;
    Ok(object.into())
}

fn project_output(output: LatexProjectOutput) -> Result<JsValue, JsValue> {
    let object = Object::new();
    set(
        &object,
        "revision",
        &JsValue::from_f64(output.revision.raw() as f64),
    )?;
    set(
        &object,
        "contentHash",
        &JsValue::from_str(&output.content_hash.hex()),
    )?;
    set(
        &object,
        "passes",
        &JsValue::from_f64(f64::from(output.passes)),
    )?;
    set(&object, "tex", &compile_output(output.tex)?)?;
    if let Some(bibliography) = output.bibliography {
        let bib = Object::new();
        let files = Array::new();
        for output_file in bibliography.files() {
            files.push(&output_file_value(
                output_file.path().as_str(),
                output_file.bytes(),
            )?);
        }
        set(&bib, "files", &files)?;
        let diagnostics = Array::new();
        for diagnostic in bibliography.diagnostics() {
            let value = Object::new();
            set(
                &value,
                "severity",
                &JsValue::from_str(match diagnostic.severity() {
                    bib_engine::BibSeverity::Info => "info",
                    bib_engine::BibSeverity::Warning => "warning",
                    bib_engine::BibSeverity::Error => "error",
                }),
            )?;
            set(
                &value,
                "code",
                &JsValue::from_str(diagnostic.code().as_str()),
            )?;
            set(&value, "message", &JsValue::from_str(diagnostic.message()))?;
            if let Some(source) = diagnostic.source() {
                set(&value, "path", &JsValue::from_str(source.path().as_str()))?;
                set(
                    &value,
                    "line",
                    &JsValue::from_f64(f64::from(source.span().line)),
                )?;
                set(
                    &value,
                    "column",
                    &JsValue::from_f64(f64::from(source.span().column)),
                )?;
            }
            diagnostics.push(&value);
        }
        set(&bib, "diagnostics", &diagnostics)?;
        let stats = bibliography.stats();
        let stats_value = Object::new();
        set(&stats_value, "sections", &usize_value(stats.sections()))?;
        set(&stats_value, "entries", &usize_value(stats.entries()))?;
        set(
            &stats_value,
            "generatedFiles",
            &usize_value(stats.generated_files()),
        )?;
        set(
            &stats_value,
            "generatedBytes",
            &usize_value(stats.generated_bytes()),
        )?;
        set(&bib, "stats", &stats_value)?;
        set(&object, "bibliography", &bib)?;
    }
    let generated = Array::new();
    for file in output.generated_files {
        generated.push(&output_file_value(
            &file.path.to_string_lossy(),
            &file.bytes,
        )?);
    }
    set(&object, "generatedFiles", &generated)?;
    Ok(object.into())
}

fn project_output_v2(output: LatexProjectOutputV2) -> Result<JsValue, JsValue> {
    let object = Object::new();
    set(
        &object,
        "revision",
        &JsValue::from_f64(output.revision.raw() as f64),
    )?;
    set(
        &object,
        "contentHash",
        &JsValue::from_str(&output.content_hash.hex()),
    )?;
    set(
        &object,
        "passes",
        &JsValue::from_f64(f64::from(output.passes)),
    )?;
    set(&object, "tex", &compile_output(output.tex)?)?;
    if let Some(bibliography) = output.bibliography {
        let bib = Object::new();
        set(
            &bib,
            "backend",
            &JsValue::from_str(match bibliography.backend() {
                bib_engine::BibliographyBackend::Biblatex => "biblatex",
                bib_engine::BibliographyBackend::Classic => "classic",
            }),
        )?;
        let files = Array::new();
        for file in bibliography.files() {
            files.push(&output_file_value(file.path().as_str(), file.bytes())?);
        }
        set(&bib, "files", &files)?;
        let diagnostics = Array::new();
        for diagnostic in bibliography.diagnostics() {
            let value = Object::new();
            set(
                &value,
                "code",
                &JsValue::from_str(match diagnostic.code() {
                    bib_engine::BibliographyDiagnosticCode::Biblatex(code) => code.as_str(),
                    bib_engine::BibliographyDiagnosticCode::Classic(code) => code.as_str(),
                }),
            )?;
            set(&value, "message", &JsValue::from_str(diagnostic.message()))?;
            diagnostics.push(&value);
        }
        set(&bib, "diagnostics", &diagnostics)?;
        set(&object, "bibliography", &bib)?;
    }
    let generated = Array::new();
    for file in output.generated_files {
        generated.push(&output_file_value(
            &file.path.to_string_lossy(),
            &file.bytes,
        )?);
    }
    set(&object, "generatedFiles", &generated)?;
    Ok(object.into())
}

fn output_file_value(path: &str, bytes: &[u8]) -> Result<JsValue, JsValue> {
    let file = Object::new();
    set(&file, "path", &JsValue::from_str(path))?;
    set(&file, "bytes", &typed_array(bytes))?;
    Ok(file.into())
}

fn project_diagnostic(error: LatexProjectError) -> Result<JsValue, JsValue> {
    let code = project_error_code(&error);
    let message = error.to_string();
    let object = Object::new();
    set(&object, "code", &JsValue::from_str(code))?;
    set(&object, "message", &JsValue::from_str(&message))?;
    if let LatexProjectError::Bibliography(failure) = error {
        let diagnostics = Array::new();
        for diagnostic in failure.diagnostics() {
            let value = Object::new();
            set(
                &value,
                "code",
                &JsValue::from_str(diagnostic.code().as_str()),
            )?;
            set(&value, "message", &JsValue::from_str(diagnostic.message()))?;
            diagnostics.push(&value);
        }
        set(&object, "bibliographyDiagnostics", &diagnostics)?;
    }
    Ok(object.into())
}

pub(crate) const fn project_error_code(error: &LatexProjectError) -> &'static str {
    match error {
        LatexProjectError::Compile(error) => compile_error_code(error),
        LatexProjectError::Bibliography(failure) => match failure.kind() {
            bib_engine::BibFailureKind::NoProgress => "no-progress",
            bib_engine::BibFailureKind::Limit => "limit",
            bib_engine::BibFailureKind::ResourceConflict => "conflicting-resource",
            _ => "bibliography",
        },
        LatexProjectError::BibliographyFacade(_) | LatexProjectError::BibliographyFatal { .. } => {
            "bibliography"
        }
        LatexProjectError::InvalidLimit { .. } => "invalid-options",
        LatexProjectError::PassLimit { .. } => "pass-limit",
        LatexProjectError::Oscillation { .. } => "oscillation",
        LatexProjectError::UnexpectedResource(_) => "unexpected-resource",
        LatexProjectError::ConflictingResource(_) => "conflicting-resource",
        LatexProjectError::Transaction(_) => "transaction",
        LatexProjectError::InvalidPatch(_) => "invalid-patch",
    }
}

fn diagnostic(error: CompileError) -> Result<JsValue, JsValue> {
    let code = compile_error_code(&error);
    let diagnostic = match error {
        CompileError::Diagnostic(diagnostic) => diagnostic,
        error => CompileDiagnostic {
            message: error.to_string(),
            file: None,
            line: None,
            column: None,
        },
    };
    let object = Object::new();
    set(&object, "code", &JsValue::from_str(code))?;
    set(&object, "message", &JsValue::from_str(&diagnostic.message))?;
    if let Some(file) = diagnostic.file {
        set(&object, "file", &JsValue::from_str(&file))?;
    }
    if let Some(line) = diagnostic.line {
        set(&object, "line", &usize_value(line))?;
    }
    if let Some(column) = diagnostic.column {
        set(&object, "column", &usize_value(column))?;
    }
    Ok(object.into())
}

pub(crate) const fn compile_error_code(error: &CompileError) -> &'static str {
    match error {
        CompileError::HardLimitExceeded { .. } | CompileError::LimitExceeded { .. } => "limit",
        CompileError::AttemptLimit { .. } => "attempt-limit",
        CompileError::NoProgress => "no-progress",
        CompileError::ConflictingResolvedBinding(_)
        | CompileError::DistributionPathCollision(_) => "conflicting-resource",
        CompileError::UnexpectedResourceResponse(_) => "unexpected-resource",
        CompileError::InvalidVirtualPath { .. }
        | CompileError::FileProvision(_)
        | CompileError::Font(_) => "invalid-resource",
        _ => "compile",
    }
}

fn typed_array(bytes: &[u8]) -> JsValue {
    Uint8Array::from(bytes).into()
}

pub(crate) fn reuse_metrics(metrics: Option<umber::ReuseMetrics>) -> Result<JsValue, JsValue> {
    let Some(metrics) = metrics else {
        return Ok(JsValue::UNDEFINED);
    };
    let object = Object::new();
    set(&object, "pagesReused", &usize_value(metrics.pages_reused))?;
    set(&object, "pagesRetyped", &usize_value(metrics.pages_retyped))?;
    set(
        &object,
        "reexecutedBytes",
        &usize_value(metrics.reexecuted_bytes),
    )?;
    set(
        &object,
        "reexecutedTokens",
        &usize_value(metrics.reexecuted_tokens),
    )?;
    set(
        &object,
        "reexecutedCommands",
        &usize_value(metrics.reexecuted_commands),
    )?;
    set(
        &object,
        "reexecutedParagraphs",
        &usize_value(metrics.reexecuted_paragraphs),
    )?;
    set(
        &object,
        "sameHistoryAttempts",
        &usize_value(metrics.same_history_attempts),
    )?;
    set(
        &object,
        "sameHistoryHashMismatches",
        &usize_value(metrics.same_history_hash_mismatches),
    )?;
    let stop = match metrics.same_history_stop {
        umber::SameHistoryStop::Matched => "matched",
        umber::SameHistoryStop::ScheduleDiverged => "schedule-diverged",
        umber::SameHistoryStop::HashesDiverged => "hashes-diverged",
        umber::SameHistoryStop::NoComparableBoundary => "no-comparable-boundary",
        umber::SameHistoryStop::NotAttempted => "not-attempted",
    };
    set(&object, "sameHistoryStop", &JsValue::from_str(stop))?;
    set(
        &object,
        "restartForkMicroseconds",
        &JsValue::from_f64(metrics.restart_fork_latency.as_micros() as f64),
    )?;
    set(
        &object,
        "reexecutionMicroseconds",
        &JsValue::from_f64(metrics.reexecution_latency.as_micros() as f64),
    )?;
    set(
        &object,
        "spliceMicroseconds",
        &JsValue::from_f64(metrics.splice_latency.as_micros() as f64),
    )?;
    Ok(object.into())
}

pub(crate) fn retention_metrics(
    metrics: Option<umber::RetentionMetrics>,
) -> Result<JsValue, JsValue> {
    let Some(metrics) = metrics else {
        return Ok(JsValue::UNDEFINED);
    };
    let object = Object::new();
    set(
        &object,
        "checkpointRootBytes",
        &usize_value(metrics.checkpoint_root_bytes),
    )?;
    set(
        &object,
        "diagnosticBytes",
        &usize_value(metrics.diagnostic_bytes),
    )?;
    set(&object, "outputBytes", &usize_value(metrics.output_bytes))?;
    set(
        &object,
        "resourceBytes",
        &usize_value(metrics.resource_bytes),
    )?;
    set(
        &object,
        "protectedOverageBytes",
        &usize_value(metrics.protected_overage_bytes),
    )?;
    Ok(object.into())
}

pub(crate) fn rendered_source_result(
    result: umber::RenderedSourceResult,
) -> Result<JsRenderedSourceResult, JsValue> {
    let object = Object::new();
    match result {
        umber::RenderedSourceResult::Current(location) => {
            set(&object, "kind", &JsValue::from_str("current"))?;
            set(&object, "path", &JsValue::from_str(&location.path))?;
            set(&object, "start", &JsValue::from_f64(location.start as f64))?;
            set(&object, "end", &JsValue::from_f64(location.end as f64))?;
            set(
                &object,
                "line",
                &JsValue::from_f64(f64::from(location.line)),
            )?;
            set(
                &object,
                "column",
                &JsValue::from_f64(f64::from(location.column)),
            )?;
        }
        umber::RenderedSourceResult::Deleted { minted_revision } => {
            set(&object, "kind", &JsValue::from_str("deleted"))?;
            set(
                &object,
                "mintedRevision",
                &JsValue::from_f64(minted_revision as f64),
            )?;
        }
        umber::RenderedSourceResult::StaleRevision { accepted } => {
            set(&object, "kind", &JsValue::from_str("stale-revision"))?;
            set(
                &object,
                "accepted",
                &JsValue::from_f64(accepted.raw() as f64),
            )?;
        }
        umber::RenderedSourceResult::OutputMismatch { accepted } => {
            set(&object, "kind", &JsValue::from_str("output-mismatch"))?;
            set(
                &object,
                "acceptedOutput",
                &JsValue::from_str(&accepted.to_string()),
            )?;
        }
    }
    Ok(object.unchecked_into())
}

fn usize_value(value: usize) -> JsValue {
    JsValue::from_f64(value as f64)
}

fn set(object: &Object, name: &str, value: &JsValue) -> Result<(), JsValue> {
    Reflect::set(object, &JsValue::from_str(name), value).map(|_| ())
}
