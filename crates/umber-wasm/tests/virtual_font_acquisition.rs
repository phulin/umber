#![cfg(target_arch = "wasm32")]

use js_sys::{Array, Object, Reflect, Uint8Array};
use umber_wasm::{CompilerSession, JsSessionOptions};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::wasm_bindgen_test;

const CMR10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
const CMSY10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmsy10.tfm");

#[wasm_bindgen_test]
fn pdf_virtual_font_closure_crosses_typed_bounded_retries() {
    let options = Object::new();
    set(&options, "mainPath", &JsValue::from_str("main.tex"));
    set(&options, "engine", &JsValue::from_str("pdftex"));
    let mut session =
        CompilerSession::new(options.unchecked_ref::<JsSessionOptions>()).expect("PDF session");
    session
        .add_user_file(
            "main.tex",
            &bytes(b"\\pdfoutput=1 \\font\\root=cmr10\\relax \\root \\shipout\\hbox{A}\\end"),
        )
        .expect("main source");

    let root_tfm = request(&mut session, "required", "tfm");
    assert_eq!(string_field(&root_tfm, "name"), "cmr10.tfm");
    provide(&mut session, &root_tfm, "/texlive/cmr10.tfm", CMR10);

    let root_vf = request(&mut session, "probes", "vf");
    assert_eq!(string_field(&root_vf, "domain"), "tex");
    assert_eq!(string_field(&root_vf, "name"), "cmr10.vf");
    assert_eq!(string_field(&root_vf, "originalName"), "cmr10.vf");
    provide(
        &mut session,
        &root_vf,
        "/texlive/cmr10.vf",
        &minimal_vf_with_local(b"cmsy10"),
    );

    let local_tfm = request(&mut session, "required", "tfm");
    assert_eq!(string_field(&local_tfm, "name"), "cmsy10.tfm");
    assert_eq!(string_field(&local_tfm, "originalName"), "cmsy10.tfm");
    provide(&mut session, &local_tfm, "/texlive/cmsy10.tfm", CMSY10);

    let local_vf = request(&mut session, "probes", "vf");
    assert_eq!(string_field(&local_vf, "name"), "cmsy10.vf");
    provide_unavailable(&mut session, &local_vf);

    let map = request(&mut session, "required", "font-map");
    assert_eq!(string_field(&map, "name"), "pdftex.map");
    provide(
        &mut session,
        &map,
        "/texlive/pdftex.map",
        b"cmsy10 FixturePS <[fixture.enc <fixture.pfb\n",
    );

    let missing = session.advance().expect("encoding and program request");
    assert_eq!(string_field(missing.as_ref(), "kind"), "need-resources");
    let required = Array::from(&field(missing.as_ref(), "required"));
    assert_eq!(required.length(), 2);
    let encoding = find_kind(&required, "font-encoding");
    let program = find_kind(&required, "font-program");
    provide(
        &mut session,
        &encoding,
        "/texlive/fixture.enc",
        &fixture_encoding(),
    );
    provide(
        &mut session,
        &program,
        "/texlive/fixture.pfb",
        &fixture_pfb(),
    );

    let complete = session.advance().expect("completed bounded closure");
    assert_eq!(string_field(complete.as_ref(), "kind"), "complete");
    assert!(session.attempts().expect("attempt count") <= 7);
}

fn request(session: &mut CompilerSession, collection: &str, kind: &str) -> JsValue {
    let attempt = session.advance().expect("resource request");
    assert_eq!(string_field(attempt.as_ref(), "kind"), "need-resources");
    find_kind(&Array::from(&field(attempt.as_ref(), collection)), kind)
}

fn find_kind(requests: &Array, kind: &str) -> JsValue {
    (0..requests.length())
        .map(|index| requests.get(index))
        .find(|request| string_field(request, "kind") == kind)
        .unwrap_or_else(|| panic!("missing {kind} request"))
}

fn provide(session: &mut CompilerSession, request: &JsValue, path: &str, contents: &[u8]) {
    let response = Object::assign(&Object::new(), request.unchecked_ref());
    set(&response, "type", &JsValue::from_str("file"));
    set(&response, "virtualPath", &JsValue::from_str(path));
    set(&response, "bytes", bytes(contents).as_ref());
    session
        .provide_resources(&Array::of1(&response))
        .expect("provide resolved file");
}

fn provide_unavailable(session: &mut CompilerSession, request: &JsValue) {
    let unavailable = Object::new();
    for name in ["domain", "kind", "name"] {
        set(&unavailable, name, &field(request, name));
    }
    set(&unavailable, "type", &JsValue::from_str("file-unavailable"));
    session
        .provide_resources(&Array::of1(&unavailable))
        .expect("provide authoritative absence");
}

fn minimal_vf_with_local(name: &[u8]) -> Vec<u8> {
    let mut value = vec![247, 202, 0];
    value.extend_from_slice(&0_u32.to_be_bytes());
    value.extend_from_slice(&(10_i32 << 20).to_be_bytes());
    value.extend_from_slice(&[243, 0]);
    value.extend_from_slice(&0_u32.to_be_bytes());
    value.extend_from_slice(&(1_i32 << 20).to_be_bytes());
    value.extend_from_slice(&(10_i32 << 20).to_be_bytes());
    value.push(0);
    value.push(u8::try_from(name.len()).expect("short fixture font name"));
    value.extend_from_slice(name);
    value.push(248);
    while !value.len().is_multiple_of(4) {
        value.push(248);
    }
    value
}

fn fixture_encoding() -> Vec<u8> {
    let mut value = b"/FixtureEncoding [".to_vec();
    for _ in 0..256 {
        value.extend_from_slice(b" /.notdef");
    }
    value.extend_from_slice(b" ] def\n");
    value
}

fn fixture_pfb() -> Vec<u8> {
    let mut value = vec![0x80, 0x01];
    value.extend_from_slice(&1_u32.to_le_bytes());
    value.push(b'a');
    value.extend_from_slice(&[0x80, 0x02]);
    value.extend_from_slice(&1_u32.to_le_bytes());
    value.push(0);
    value.extend_from_slice(&[0x80, 0x03]);
    value
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
