use js_sys::{Array, Object, Reflect, Uint8Array};
use umber::{
    CompileAttemptResult, CompileDiagnostic, CompileError, MemoryRunOutput, ResourceRequest,
};
use wasm_bindgen::{JsCast, JsValue};

use crate::JsAttemptResult;

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
                let kind = match request.key().kind() {
                    umber::FileKind::TexInput => "tex",
                    umber::FileKind::Tfm => "tfm",
                };
                set(&object, "kind", &JsValue::from_str(kind))?;
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

fn typed_array(bytes: &[u8]) -> JsValue {
    Uint8Array::from(bytes).into()
}

fn usize_value(value: usize) -> JsValue {
    JsValue::from_f64(value as f64)
}

fn set(object: &Object, name: &str, value: &JsValue) -> Result<(), JsValue> {
    Reflect::set(object, &JsValue::from_str(name), value).map(|_| ())
}
