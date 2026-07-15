use js_sys::{Array, Object, Reflect, Uint8Array};
use umber::{
    CompileAttemptResult, CompileDiagnostic, CompileError, MemoryRunOutput, ResourceRequest,
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
