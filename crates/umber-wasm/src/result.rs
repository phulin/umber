use js_sys::{Array, Object, Reflect, Uint8Array};
use umber::{
    CompileAttemptResult, CompileDiagnostic, CompileError, EditorSessionStatus,
    EditorStabilizationAttempt, LatexProjectAttempt, LatexProjectError, LatexProjectOutput,
    MemoryRunOutput, OutputCapability, ResourceRequest, TexFixedPointError, TexFixedPointOutput,
};
use wasm_bindgen::{JsCast, JsValue};

use crate::JsAcceptedInputObservationLedger;
use crate::JsAttemptResult;
use crate::JsEditorAttemptResult;
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
            set(&object, "output", &project_output(*output)?)?;
        }
        LatexProjectAttempt::Error(error) => {
            set(&object, "kind", &JsValue::from_str("error"))?;
            set(&object, "diagnostic", &project_diagnostic(error)?)?;
        }
    }
    Ok(object.unchecked_into())
}

pub(crate) fn editor_advance_result(
    result: CompileAttemptResult,
    status: Option<EditorSessionStatus>,
    output: Option<&TexFixedPointOutput>,
) -> Result<JsEditorAttemptResult, JsValue> {
    let object = Object::new();
    match result {
        CompileAttemptResult::NeedResources(resources) => {
            set(&object, "kind", &JsValue::from_str("need-resources"))?;
            set(&object, "phase", &JsValue::from_str("advance"))?;
            set_resource_batch(&object, resources)?;
        }
        CompileAttemptResult::Complete(_) => {
            let output = output.expect("completed editor advance retains display output");
            match status.expect("completed editor advance has status") {
                EditorSessionStatus::Provisional {
                    revision,
                    stabilization_required,
                } => {
                    set(&object, "kind", &JsValue::from_str("provisional"))?;
                    set_editor_status_fields(&object, revision, stabilization_required, None)?;
                }
                EditorSessionStatus::Stable {
                    revision,
                    passes,
                    stabilization_required,
                } => {
                    set(&object, "kind", &JsValue::from_str("stable"))?;
                    set_editor_status_fields(
                        &object,
                        revision,
                        stabilization_required,
                        Some(("passes", passes)),
                    )?;
                }
                EditorSessionStatus::Stabilizing { .. } => {
                    unreachable!("advance cannot complete while stabilization is active")
                }
            }
            set(&object, "output", &tex_fixed_point_output(output.clone())?)?;
        }
        CompileAttemptResult::Error(error) => {
            set(&object, "kind", &JsValue::from_str("error"))?;
            set(&object, "phase", &JsValue::from_str("advance"))?;
            set(&object, "diagnostic", &diagnostic(error)?)?;
        }
    }
    Ok(object.unchecked_into())
}

pub(crate) fn editor_stabilization_result(
    result: EditorStabilizationAttempt,
    status: Option<EditorSessionStatus>,
) -> Result<JsEditorAttemptResult, JsValue> {
    let object = Object::new();
    match result {
        EditorStabilizationAttempt::NeedResources(resources) => {
            set(&object, "kind", &JsValue::from_str("need-resources"))?;
            set(&object, "phase", &JsValue::from_str("stabilization"))?;
            set_resource_batch(&object, resources)?;
            if let Some(EditorSessionStatus::Stabilizing {
                revision,
                completed_passes,
                stabilization_required,
            }) = status
            {
                let value = Object::new();
                set(&value, "kind", &JsValue::from_str("stabilizing"))?;
                set_editor_status_fields(
                    &value,
                    revision,
                    stabilization_required,
                    Some(("completedPasses", completed_passes)),
                )?;
                set(&object, "status", &value)?;
            }
        }
        EditorStabilizationAttempt::Complete(output) => {
            set(&object, "kind", &JsValue::from_str("stable"))?;
            set_editor_status_fields(
                &object,
                output.revision,
                false,
                Some(("passes", output.passes)),
            )?;
            set(&object, "output", &tex_fixed_point_output(*output)?)?;
        }
        EditorStabilizationAttempt::Error(error) => {
            set(&object, "kind", &JsValue::from_str("error"))?;
            set(&object, "phase", &JsValue::from_str("stabilization"))?;
            set(&object, "diagnostic", &tex_fixed_point_diagnostic(error)?)?;
        }
    }
    Ok(object.unchecked_into())
}

pub(crate) fn editor_status(status: Option<EditorSessionStatus>) -> Result<JsValue, JsValue> {
    let Some(status) = status else {
        return Ok(JsValue::UNDEFINED);
    };
    let object = Object::new();
    match status {
        EditorSessionStatus::Provisional {
            revision,
            stabilization_required,
        } => {
            set(&object, "kind", &JsValue::from_str("provisional"))?;
            set_editor_status_fields(&object, revision, stabilization_required, None)?;
        }
        EditorSessionStatus::Stabilizing {
            revision,
            completed_passes,
            stabilization_required,
        } => {
            set(&object, "kind", &JsValue::from_str("stabilizing"))?;
            set_editor_status_fields(
                &object,
                revision,
                stabilization_required,
                Some(("completedPasses", completed_passes)),
            )?;
        }
        EditorSessionStatus::Stable {
            revision,
            passes,
            stabilization_required,
        } => {
            set(&object, "kind", &JsValue::from_str("stable"))?;
            set_editor_status_fields(
                &object,
                revision,
                stabilization_required,
                Some(("passes", passes)),
            )?;
        }
    }
    Ok(object.into())
}

fn set_resource_batch(object: &Object, resources: umber::NeedResources) -> Result<(), JsValue> {
    let required = resource_requests(resources.required)?;
    set(object, "required", &required)?;
    let probes = resource_requests(resources.probes)?;
    set(object, "probes", &probes)?;
    let hints = resource_requests(resources.prefetch_hints)?;
    set(object, "prefetchHints", &hints)
}

fn set_editor_status_fields(
    object: &Object,
    revision: umber::RevisionId,
    stabilization_required: bool,
    pass_field: Option<(&str, u32)>,
) -> Result<(), JsValue> {
    set(
        object,
        "revision",
        &JsValue::from_f64(revision.raw() as f64),
    )?;
    set(
        object,
        "stabilizationRequired",
        &JsValue::from_bool(stabilization_required),
    )?;
    if let Some((name, value)) = pass_field {
        set(object, name, &JsValue::from_f64(f64::from(value)))?;
    }
    Ok(())
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
                match request.key.variation.instance() {
                    umber::VariationInstance::Default => {
                        set(&object, "variationInstance", &JsValue::from_str("default"))?;
                    }
                    umber::VariationInstance::Coordinates => {
                        set(
                            &object,
                            "variationInstance",
                            &JsValue::from_str("coordinates"),
                        )?;
                    }
                    umber::VariationInstance::Named(name_id) => {
                        let instance = Object::new();
                        set(
                            &instance,
                            "namedNameId",
                            &JsValue::from_f64(f64::from(name_id)),
                        )?;
                        set(&object, "variationInstance", &instance)?;
                    }
                }
                let features = Array::new();
                for setting in request.key.feature_policy.settings() {
                    let value = Object::new();
                    set(&value, "tag", &JsValue::from_str(&setting.tag.to_string()))?;
                    set(
                        &value,
                        "value",
                        &JsValue::from_f64(f64::from(setting.value)),
                    )?;
                    features.push(&value);
                }
                set(&object, "features", &features)?;
                set(
                    &object,
                    "direction",
                    &JsValue::from_str(match request.key.direction {
                        umber::WritingDirection::LeftToRight => "ltr",
                        umber::WritingDirection::RightToLeft => "rtl",
                    }),
                )?;
                if let Some(script) = request.key.script {
                    set(&object, "script", &JsValue::from_str(&script.to_string()))?;
                }
                if let Some(language) = &request.key.language {
                    set(&object, "language", &JsValue::from_str(language.as_str()))?;
                }
                let accepted = Array::new();
                if request
                    .accepted_containers
                    .contains(umber::FontContainer::Woff2)
                {
                    accepted.push(&JsValue::from_str("woff2"));
                }
                set(&object, "acceptedContainers", &accepted)?;
            }
            ResourceRequest::PkFont(request) => {
                set(&object, "type", &JsValue::from_str("pk-font"))?;
                set(&object, "texName", &typed_array(request.tex_name()))?;
                set(&object, "dpi", &JsValue::from_f64(f64::from(request.dpi())))?;
                set(&object, "mode", &typed_array(request.mode()))?;
            }
        }
        result.push(&object);
    }
    Ok(result)
}

fn compile_output(output: MemoryRunOutput) -> Result<JsValue, JsValue> {
    let object = Object::new();
    let outputs = Array::new();
    for capability in output.outputs.iter() {
        outputs.push(&JsValue::from_str(match capability {
            OutputCapability::Dvi => "dvi",
            OutputCapability::Pdf => "pdf",
            OutputCapability::Html => "html",
        }));
    }
    set(&object, "outputs", &outputs)?;
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

pub(crate) fn tex_fixed_point_output(output: TexFixedPointOutput) -> Result<JsValue, JsValue> {
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
    if let LatexProjectError::Compile(CompileError::Diagnostic(diagnostic)) = &error {
        set_diagnostic_location(&object, diagnostic.location.as_ref())?;
    }
    if let LatexProjectError::Bibliography(bib_engine::BibliographyFailure::Biblatex(failure)) =
        &error
    {
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
        LatexProjectError::Bibliography(failure) => match failure {
            bib_engine::BibliographyFailure::Biblatex(failure) => match failure.kind() {
                bib_engine::BibFailureKind::NoProgress => "no-progress",
                bib_engine::BibFailureKind::Limit => "limit",
                bib_engine::BibFailureKind::ResourceConflict => "conflicting-resource",
                _ => "bibliography",
            },
            bib_engine::BibliographyFailure::Classic(bib_engine::ClassicBibFailure::NoProgress) => {
                "no-progress"
            }
            bib_engine::BibliographyFailure::Classic(bib_engine::ClassicBibFailure::Limit) => {
                "limit"
            }
            bib_engine::BibliographyFailure::Classic(
                bib_engine::ClassicBibFailure::ResourceConflict,
            ) => "conflicting-resource",
            _ => "bibliography",
        },
        LatexProjectError::BibliographyFatal { .. } => "bibliography",
        LatexProjectError::InvalidLimit { .. } => "invalid-options",
        LatexProjectError::PassLimit { .. } => "pass-limit",
        LatexProjectError::Oscillation { .. } => "oscillation",
        LatexProjectError::UnexpectedResource(_) => "unexpected-resource",
        LatexProjectError::ConflictingResource(_) => "conflicting-resource",
        LatexProjectError::Transaction(_) => "transaction",
        LatexProjectError::InvalidPatch(_) => "invalid-patch",
    }
}

fn tex_fixed_point_diagnostic(error: TexFixedPointError) -> Result<JsValue, JsValue> {
    if let TexFixedPointError::Compile(error) = error {
        return diagnostic(error);
    }
    let object = Object::new();
    set(
        &object,
        "code",
        &JsValue::from_str(tex_fixed_point_error_code(&error)),
    )?;
    set(&object, "message", &JsValue::from_str(&error.to_string()))?;
    Ok(object.into())
}

pub(crate) const fn tex_fixed_point_error_code(error: &TexFixedPointError) -> &'static str {
    match error {
        TexFixedPointError::Compile(error) => compile_error_code(error),
        TexFixedPointError::InvalidLimit { .. } => "invalid-options",
        TexFixedPointError::PassLimit { .. } => "pass-limit",
        TexFixedPointError::Oscillation { .. } => "oscillation",
        TexFixedPointError::Transaction(_) => "transaction",
        TexFixedPointError::InvalidPatch(_) => "invalid-patch",
        TexFixedPointError::UnexpectedResource(_) => "unexpected-resource",
        TexFixedPointError::ConflictingResource(_) => "conflicting-resource",
    }
}

fn diagnostic(error: CompileError) -> Result<JsValue, JsValue> {
    let code = compile_error_code(&error);
    let diagnostic = match error {
        CompileError::Diagnostic(diagnostic) => diagnostic,
        error => CompileDiagnostic {
            message: error.to_string(),
            location: None,
        },
    };
    let object = Object::new();
    set(&object, "code", &JsValue::from_str(code))?;
    set(&object, "message", &JsValue::from_str(&diagnostic.message))?;
    set_diagnostic_location(&object, diagnostic.location.as_ref())?;
    Ok(object.into())
}

fn set_diagnostic_location(
    diagnostic: &Object,
    location: Option<&umber::CompileSourceLocation>,
) -> Result<(), JsValue> {
    let Some(location) = location else {
        return Ok(());
    };
    let value = Object::new();
    set(&value, "file", &JsValue::from_str(&location.file))?;
    set(
        &value,
        "byteStart",
        &JsValue::from_f64(location.byte_start as f64),
    )?;
    set(
        &value,
        "byteEnd",
        &JsValue::from_f64(location.byte_end as f64),
    )?;
    set(&value, "line", &JsValue::from_f64(f64::from(location.line)))?;
    set(
        &value,
        "column",
        &JsValue::from_f64(f64::from(location.column)),
    )?;
    set(diagnostic, "location", &value)
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

pub(crate) fn accepted_input_observations(
    ledger: Option<&umber::AcceptedInputObservationLedger>,
) -> Result<Option<JsAcceptedInputObservationLedger>, JsValue> {
    let Some(ledger) = ledger else {
        return Ok(None);
    };
    let object = Object::new();
    set(
        &object,
        "schemaVersion",
        &JsValue::from_f64(f64::from(ledger.schema_version())),
    )?;
    set(
        &object,
        "revision",
        &JsValue::from_f64(ledger.revision().raw() as f64),
    )?;
    let observations = Array::new();
    for observation in ledger.observations() {
        let value = Object::new();
        set(
            &value,
            "path",
            &JsValue::from_str(observation.path().as_str()),
        )?;
        set(
            &value,
            "namespace",
            &JsValue::from_str(match observation.namespace() {
                umber::InputObservationNamespace::Authored => "authored",
                umber::InputObservationNamespace::Generated => "generated",
                umber::InputObservationNamespace::Distribution => "distribution",
            }),
        )?;
        let outcome = Object::new();
        match observation.outcome() {
            umber::InputObservationOutcome::Present(hash) => {
                set(&outcome, "kind", &JsValue::from_str("present"))?;
                set(&outcome, "contentHash", &JsValue::from_str(&hash.hex()))?;
            }
            umber::InputObservationOutcome::Missing => {
                set(&outcome, "kind", &JsValue::from_str("missing"))?;
            }
        }
        set(&value, "outcome", &outcome)?;
        set(
            &value,
            "access",
            &JsValue::from_str(match observation.access() {
                umber::InputDependencyAccess::RequiredRead => "required-read",
                umber::InputDependencyAccess::AuthoritativeProbe => "authoritative-probe",
            }),
        )?;
        set(
            &value,
            "resourceKind",
            &JsValue::from_str(observation.resource_kind().wire_name()),
        )?;
        set(
            &value,
            "phase",
            &JsValue::from_str(match observation.phase() {
                umber::InputObservationPhase::Tex => "tex",
                umber::InputObservationPhase::BibliographyDetection => "bibliography-detection",
                umber::InputObservationPhase::Bibliography => "bibliography",
            }),
        )?;
        set(
            &value,
            "revision",
            &JsValue::from_f64(observation.revision().raw() as f64),
        )?;
        if let Some(pass) = observation.project_pass() {
            set(&value, "projectPass", &JsValue::from_f64(f64::from(pass)))?;
        }
        if let Some(source) = observation.requesting_source() {
            set(
                &value,
                "requestingSource",
                &JsValue::from_str(source.as_str()),
            )?;
        }
        set(
            &value,
            "owner",
            &JsValue::from_str(match observation.owner() {
                umber::InputObservationOwner::TexEngine => "tex-engine",
                umber::InputObservationOwner::BibliographyDetector => "bibliography-detector",
                umber::InputObservationOwner::Biblatex => "biblatex",
                umber::InputObservationOwner::ClassicBibtex => "classic-bibtex",
            }),
        )?;
        observations.push(&value);
    }
    set(&object, "observations", &observations)?;
    Ok(Some(object.unchecked_into()))
}

fn set(object: &Object, name: &str, value: &JsValue) -> Result<(), JsValue> {
    Reflect::set(object, &JsValue::from_str(name), value).map(|_| ())
}
