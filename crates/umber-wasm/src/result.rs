use js_sys::{Array, Object, Reflect, Uint8Array};
use umber::{CompileAttemptResult, CompileDiagnostic, CompileError, MemoryRunOutput};
use wasm_bindgen::{JsCast, JsValue};

use crate::JsAttemptResult;

pub(crate) fn attempt_result(result: CompileAttemptResult) -> Result<JsAttemptResult, JsValue> {
    let object = Object::new();
    match result {
        CompileAttemptResult::NeedFiles(requests) => {
            set(&object, "kind", &JsValue::from_str("need-files"))?;
            let files = Array::new();
            for request in requests {
                let file = Object::new();
                let kind = match request.key().kind() {
                    umber::FileKind::TexInput => "tex",
                    umber::FileKind::Tfm => "tfm",
                };
                set(&file, "kind", &JsValue::from_str(kind))?;
                set(&file, "name", &JsValue::from_str(request.key().name()))?;
                set(
                    &file,
                    "originalName",
                    &JsValue::from_str(request.original_name()),
                )?;
                files.push(&file);
            }
            set(&object, "files", &files)?;
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

fn compile_output(output: MemoryRunOutput) -> Result<JsValue, JsValue> {
    let object = Object::new();
    set(
        &object,
        "terminal",
        &JsValue::from_str(&String::from_utf8_lossy(&output.terminal)),
    )?;
    set(&object, "log", &typed_array(&output.log))?;
    set(&object, "dvi", &typed_array(&output.dvi))?;
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
