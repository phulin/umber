#![cfg(target_arch = "wasm32")]

use js_sys::{Array, Object, Reflect, Uint8Array};
use umber_wasm::{
    CompilerSession, JsFileRequestKey, JsSessionOptions, format_schema_version, package_version,
};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn typed_attempts_preserve_binary_inputs_and_clear_cached_allocations() {
    let mut session = session("/job/main.tex");
    session
        .add_user_file("main.tex", &bytes(b"\\input remote \\end"))
        .expect("add main source");

    let missing = session.compile_attempt().expect("missing attempt");
    assert_eq!(string_field(missing.as_ref(), "kind"), "need-files");
    let files = Array::from(&field(missing.as_ref(), "files"));
    assert_eq!(files.length(), 1);
    let request = files.get(0);
    assert_eq!(string_field(&request, "kind"), "tex");
    assert_eq!(string_field(&request, "name"), "remote.tex");
    assert_eq!(string_field(&request, "originalName"), "remote");

    let remote = b"%\0\n\\input second ";
    session
        .provide_resolved_file(
            request.unchecked_ref::<JsFileRequestKey>(),
            "/texlive/tex/remote.tex",
            &bytes(remote),
        )
        .expect("provide binary remote input");
    let second = session.compile_attempt().expect("second missing attempt");
    assert_eq!(string_field(second.as_ref(), "kind"), "need-files");
    let second_files = Array::from(&field(second.as_ref(), "files"));
    let second_request = second_files.get(0);
    assert_eq!(string_field(&second_request, "name"), "second.tex");
    let second_bytes = b"%\0\n";
    session
        .provide_resolved_file(
            second_request.unchecked_ref(),
            "/texlive/tex/second.tex",
            &bytes(second_bytes),
        )
        .expect("provide second binary input");
    let complete = session.compile_attempt().expect("complete retry");
    assert_eq!(string_field(complete.as_ref(), "kind"), "complete");
    assert_eq!(
        session.cached_file_bytes().expect("cache bytes"),
        remote.len() + second_bytes.len()
    );
    assert_eq!(session.resolved_file_count().expect("file count"), 2);
    session
        .clear_distribution_cache()
        .expect("clear distribution cache");
    assert_eq!(session.cached_file_bytes().expect("cleared bytes"), 0);
    assert_eq!(session.resolved_file_count().expect("cleared count"), 0);
}

#[wasm_bindgen_test]
fn complete_output_uses_strings_and_uint8arrays() {
    let mut session = session("main.tex");
    session
        .add_user_file("main.tex", &bytes(b"\\shipout\\hbox{}\\end"))
        .expect("add main source");
    let complete = session.compile_attempt().expect("complete attempt");
    assert_eq!(string_field(complete.as_ref(), "kind"), "complete");
    let output = field(complete.as_ref(), "output");
    assert!(field(&output, "terminal").as_string().is_some());
    let log = field(&output, "log");
    let dvi = field(&output, "dvi");
    assert!(log.is_instance_of::<Uint8Array>());
    assert!(dvi.is_instance_of::<Uint8Array>());
    let dvi = Uint8Array::new(&dvi).to_vec();
    assert!(!dvi.is_empty());
    assert!(dvi.contains(&0), "DVI embedded zero bytes must survive");
    assert!(Array::is_array(&field(&output, "files")));
}

#[wasm_bindgen_test]
fn errors_are_typed_and_invalid_boundary_values_throw() {
    let mut missing_main = session("main.tex");
    let result = missing_main.compile_attempt().expect("error result");
    assert_eq!(string_field(result.as_ref(), "kind"), "error");
    assert!(
        string_field(&field(result.as_ref(), "diagnostic"), "message").contains("was not provided")
    );

    let invalid = options("../escape.tex");
    assert!(CompilerSession::new(invalid.unchecked_ref()).is_err());

    let request = Object::new();
    set(&request, "kind", &JsValue::from_str("other"));
    set(&request, "name", &JsValue::from_str("x.tex"));
    assert!(
        missing_main
            .provide_resolved_file(request.unchecked_ref(), "/texlive/x.tex", &bytes(b"x"),)
            .is_err()
    );
}

#[wasm_bindgen_test]
fn committed_plain_format_loads_and_rejects_incompatible_bytes() {
    assert_eq!(package_version(), env!("CARGO_PKG_VERSION"));
    assert_eq!(format_schema_version(), 4);
    let format = include_bytes!("../assets/plain.fmt");
    let mut plain = session_with_format("main.tex", format);
    plain
        .add_user_file("main.tex", &bytes(b"\\shipout\\hbox{}\\end"))
        .expect("add plain source");
    assert_eq!(
        string_field(
            plain.compile_attempt().expect("plain attempt").as_ref(),
            "kind",
        ),
        "complete",
    );

    let native_tex = b"\\catcode`\\{=1 \\catcode`\\}=2 \\endinput";
    assert_format_error(native_tex, "not an Umber format file");

    let mut wrong_schema = format.to_vec();
    wrong_schema[8..12].copy_from_slice(&5_u32.to_le_bytes());
    assert_format_error(&wrong_schema, "unsupported Umber format version 5");

    let mut corrupt = format.to_vec();
    let last = corrupt.last_mut().expect("format payload");
    *last ^= 1;
    assert_format_error(&corrupt, "Umber format checksum mismatch");
}

#[wasm_bindgen_test]
fn explicit_disposal_releases_session_and_rejects_later_calls() {
    let mut session = session("main.tex");
    assert!(!session.disposed());
    session.dispose();
    assert!(session.disposed());
    assert!(session.compile_attempt().is_err());
    assert!(session.attempts().is_err());
}

fn session(main_path: &str) -> CompilerSession {
    let options = options(main_path);
    CompilerSession::new(options.unchecked_ref::<JsSessionOptions>()).expect("construct session")
}

fn session_with_format(main_path: &str, format: &[u8]) -> CompilerSession {
    let options = options(main_path);
    set(&options, "format", bytes(format).as_ref());
    CompilerSession::new(options.unchecked_ref::<JsSessionOptions>()).expect("construct session")
}

fn assert_format_error(format: &[u8], expected: &str) {
    let mut session = session_with_format("main.tex", format);
    session
        .add_user_file("main.tex", &bytes(b"\\end"))
        .expect("add main source");
    let attempt = session.compile_attempt().expect("format error attempt");
    assert_eq!(string_field(attempt.as_ref(), "kind"), "error");
    let diagnostic = field(attempt.as_ref(), "diagnostic");
    assert!(
        string_field(&diagnostic, "message").contains(expected),
        "expected format diagnostic containing {expected}",
    );
}

fn options(main_path: &str) -> Object {
    let options = Object::new();
    set(&options, "mainPath", &JsValue::from_str(main_path));
    options
}

fn bytes(value: &[u8]) -> Uint8Array {
    Uint8Array::from(value)
}

fn field(object: &JsValue, name: &str) -> JsValue {
    Reflect::get(object, &JsValue::from_str(name)).expect("read field")
}

fn string_field(object: &JsValue, name: &str) -> String {
    field(object, name).as_string().expect("string field")
}

fn set(object: &Object, name: &str, value: &JsValue) {
    assert!(Reflect::set(object, &JsValue::from_str(name), value).expect("set field"));
}
