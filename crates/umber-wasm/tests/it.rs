#![cfg(target_arch = "wasm32")]

use js_sys::{Array, Object, Reflect, Uint8Array};
use umber_wasm::{
    CompilerSession, JsFileRequestKey, JsSessionOptions, JsSourcePatch, format_schema_version,
    package_version,
};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn typed_attempts_preserve_binary_inputs_and_clear_cached_allocations() {
    let mut session = session("/job/main.tex");
    session
        .add_user_file("main.tex", &bytes(b"\\input remote \\end"))
        .expect("add main source");

    let missing = session.compile_attempt().expect("missing attempt");
    assert_eq!(string_field(missing.as_ref(), "kind"), "need-resources");
    let files = Array::from(&field(missing.as_ref(), "required"));
    assert_eq!(files.length(), 1);
    let request = files.get(0);
    assert_eq!(string_field(&request, "domain"), "tex");
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
    assert_eq!(string_field(second.as_ref(), "kind"), "need-resources");
    let second_files = Array::from(&field(second.as_ref(), "required"));
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
fn pdftex_engine_option_reports_the_pinned_identity() {
    let session_options = options("main.tex");
    set(&session_options, "engine", &JsValue::from_str("pdftex"));
    let mut session = CompilerSession::new(session_options.unchecked_ref::<JsSessionOptions>())
        .expect("pdfTeX session");
    session
        .add_user_file(
            "main.tex",
            &bytes(b"\\message{engine=\\the\\pdftexversion.\\pdftexrevision}\\end"),
        )
        .expect("add identity source");
    let complete = session
        .compile_attempt()
        .expect("complete identity attempt");
    assert_eq!(string_field(complete.as_ref(), "kind"), "complete");
    let terminal = field(&field(complete.as_ref(), "output"), "terminal")
        .as_string()
        .expect("terminal text");
    assert!(terminal.contains("engine=140.27"));

    let invalid = options("main.tex");
    set(&invalid, "engine", &JsValue::from_str("pdfelatex"));
    assert!(CompilerSession::new(invalid.unchecked_ref::<JsSessionOptions>()).is_err());
}

#[wasm_bindgen_test]
fn pdftex_return_value_reports_invalid_object_recovery() {
    let session_options = options("main.tex");
    set(&session_options, "engine", &JsValue::from_str("pdftex"));
    let mut session = CompilerSession::new(session_options.unchecked_ref::<JsSessionOptions>())
        .expect("pdfTeX session");
    session
        .add_user_file(
            "main.tex",
            &bytes(
                b"\\pdfoutput=1\\message{r0=\\the\\pdfretval}\\pdfobj useobjnum 99{}\\message{r1=\\the\\pdfretval}\\end",
            ),
        )
        .expect("add return-value source");
    let complete = session
        .compile_attempt()
        .expect("complete return-value attempt");
    assert_eq!(string_field(complete.as_ref(), "kind"), "complete");
    let terminal = field(&field(complete.as_ref(), "output"), "terminal")
        .as_string()
        .expect("terminal text");
    assert!(terminal.contains("r0=0"), "{terminal}");
    assert!(
        terminal.contains("invalid object number being ignored"),
        "{terminal}"
    );
    assert!(terminal.contains("r1=-1"), "{terminal}");
}

#[wasm_bindgen_test]
fn pdftex_ximage_enquiries_survive_binary_resource_retry() {
    let session_options = options("main.tex");
    set(&session_options, "engine", &JsValue::from_str("pdftex"));
    let mut session = CompilerSession::new(session_options.unchecked_ref::<JsSessionOptions>())
        .expect("pdfTeX session");
    session
        .add_user_file(
            "main.tex",
            &bytes(
                b"\\pdfoutput=1 \\message{initial=\\the\\pdflastximagepages/\\the\\pdflastximagecolordepth} \\pdfximage{figure.png} \\message{image=\\the\\pdflastximagepages/\\the\\pdflastximagecolordepth} \\pdfrefximage\\pdflastximage \\message{reuse=\\the\\pdflastximagepages/\\the\\pdflastximagecolordepth} \\end",
            ),
        )
        .expect("add ximage source");

    let missing = session.compile_attempt().expect("image request");
    assert_eq!(string_field(missing.as_ref(), "kind"), "need-resources");
    let required = Array::from(&field(missing.as_ref(), "required"));
    assert_eq!(required.length(), 1);
    let request = required.get(0);
    assert_eq!(string_field(&request, "kind"), "image");
    assert_eq!(string_field(&request, "name"), "figure.png");

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(&13_u32.to_be_bytes());
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&40_u32.to_be_bytes());
    png.extend_from_slice(&20_u32.to_be_bytes());
    png.extend_from_slice(&[8, 2, 0, 0, 0]);
    session
        .provide_resolved_file(
            request.unchecked_ref::<JsFileRequestKey>(),
            "/texlive/figure.png",
            &bytes(&png),
        )
        .expect("provide PNG");

    let complete = session.compile_attempt().expect("complete retry");
    assert_eq!(string_field(complete.as_ref(), "kind"), "complete");
    let terminal = field(&field(complete.as_ref(), "output"), "terminal")
        .as_string()
        .expect("terminal text");
    assert!(terminal.contains("initial=0/0"), "{terminal}");
    assert!(terminal.contains("image=1/8"), "{terminal}");
    assert!(terminal.contains("reuse=1/8"), "{terminal}");
}

#[wasm_bindgen_test]
async fn generated_html_projects_exact_geometry_at_firefox_zoom_levels() {
    let session_options = options("main.tex");
    set(&session_options, "html", Object::new().as_ref());
    let mut session = CompilerSession::new(session_options.unchecked_ref::<JsSessionOptions>())
        .expect("HTML session");
    let tfm = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    session
        .add_user_file("cmr10.tfm", &bytes(tfm))
        .expect("add TFM");
    let source = b"\\font\\tenrm=cmr10\\relax\\shipout\\hbox{\\kern-2pt\\vrule width3pt height4pt depth1pt\\tenrm AV office}\\end";
    session
        .add_user_file("main.tex", &bytes(source))
        .expect("add source");
    let missing = session.advance().expect("font request");
    assert_eq!(string_field(missing.as_ref(), "kind"), "need-resources");
    let required = Array::from(&field(missing.as_ref(), "required"));
    assert_eq!(required.length(), 1);
    let request: Object = required.get(0).unchecked_into();
    let response = Object::assign(&Object::new(), &request);
    set(&response, "container", &JsValue::from_str("woff2"));
    set(
        &response,
        "bytes",
        bytes(include_bytes!("../assets/cmu-serif-500-roman.woff2")).as_ref(),
    );
    set(
        &response,
        "provenance",
        &JsValue::from_str("test CM Unicode fixture under the SIL OFL"),
    );
    let responses = Array::of1(&response);
    session
        .provide_resources(&responses)
        .expect("provide retained WOFF2 once");
    let complete = session.advance().expect("HTML compile");
    if string_field(complete.as_ref(), "kind") != "complete" {
        let diagnostic = field(complete.as_ref(), "diagnostic");
        panic!("{}", string_field(&diagnostic, "message"));
    }
    let output = field(complete.as_ref(), "output");
    let html = field(&output, "html");
    assert!(html.is_instance_of::<Uint8Array>());
    let html_text = String::from_utf8(Uint8Array::new(&html).to_vec()).expect("HTML UTF-8");
    let event = rendered_text_event(&html_text, b'A');
    let output_id = rendered_output_id(&html_text);
    let retention_before = session.retention_metrics().expect("accepted retention");
    assert!(
        field(&retention_before, "resourceBytes")
            .as_f64()
            .expect("numeric resource bytes")
            > 0.0
    );
    let diagnostic_before = field(&retention_before, "diagnosticBytes")
        .as_f64()
        .expect("numeric diagnostic bytes");
    let location = session
        .rendered_source_location(1, event, Some(0), output_id.clone(), 1)
        .expect("source query")
        .expect("mapped source");
    let retention_after = session.retention_metrics().expect("live retention");
    let diagnostic_after = field(&retention_after, "diagnosticBytes")
        .as_f64()
        .expect("numeric diagnostic bytes");
    assert!(diagnostic_after > diagnostic_before);
    assert!(
        session
            .rendered_source_location(1, event, Some(2), output_id.clone(), 1)
            .expect("space query")
            .is_none()
    );
    let source_start = source
        .windows(2)
        .position(|window| window == b"AV")
        .expect("rendered A");
    assert_eq!(string_field(location.as_ref(), "kind"), "current");
    assert_eq!(string_field(location.as_ref(), "path"), "/job/main.tex");
    assert_eq!(
        field(location.as_ref(), "start").as_f64(),
        Some(source_start as f64)
    );
    assert_eq!(
        field(location.as_ref(), "end").as_f64(),
        Some((source_start + 1) as f64)
    );
    let stale = session
        .rendered_source_location(1, event, Some(0), output_id.clone(), 0)
        .expect("stale query")
        .expect("typed stale result");
    assert_eq!(string_field(stale.as_ref(), "kind"), "stale-revision");
    assert_eq!(field(stale.as_ref(), "accepted").as_f64(), Some(1.0));

    let other_options = options("main.tex");
    set(&other_options, "html", Object::new().as_ref());
    let mut other = CompilerSession::new(other_options.unchecked_ref::<JsSessionOptions>())
        .expect("second HTML session");
    other
        .add_user_file("cmr10.tfm", &bytes(tfm))
        .expect("second TFM");
    other
        .add_user_file(
            "main.tex",
            &bytes(b"\\font\\tenrm=cmr10\\relax\\shipout\\hbox{\\tenrm BBB}\\end"),
        )
        .expect("second source");
    provide_requested_html_font(&mut other);
    let other_complete = other.advance().expect("second HTML compile");
    let other_output = field(other_complete.as_ref(), "output");
    let other_html = String::from_utf8(Uint8Array::new(&field(&other_output, "html")).to_vec())
        .expect("second HTML UTF-8");
    assert_ne!(rendered_output_id(&other_html), output_id);
    let mismatch = other
        .rendered_source_location(1, event, Some(0), output_id.clone(), 1)
        .expect("cross-session query")
        .expect("typed mismatch");
    assert_eq!(string_field(mismatch.as_ref(), "kind"), "output-mismatch");
    let function = js_sys::Function::new_with_args(
        "bytes",
        r#"
          const iframe = document.createElement('iframe');
          iframe.style.cssText = 'border:0;width:900px;height:500px';
          return new Promise((resolve, reject) => {
            iframe.addEventListener('load', () => {
              try {
                const doc = iframe.contentDocument;
                const page = doc.querySelector('.umber-page');
                const mag = Number(page.dataset.umberMag);
                const px = raw => Number(raw) * mag * 48 / (65536 * 5 * 7227);
                const close = (a, b) => Math.abs(a - b) <= 1 / 30 + 1e-6;
                let ok = doc.documentElement.outerHTML.includes('umber-html/1');
                for (const zoom of [1, 1.25, 2]) {
                  page.style.zoom = String(zoom);
                  const pageRect = page.getBoundingClientRect();
                  const rule = page.querySelector('.umber-rule');
                  const ruleRect = rule.getBoundingClientRect();
                  const run = page.querySelector('.umber-run');
                  const baseline = run.querySelector('.umber-baseline').getBoundingClientRect();
                  ok = ok && Number(rule.dataset.umberXSp) < 0
                    && close(pageRect.width, px(page.dataset.umberWidthSp) * zoom)
                    && close(ruleRect.left - pageRect.left, px(rule.dataset.umberXSp) * zoom)
                    && close(ruleRect.top - pageRect.top, px(rule.dataset.umberYSp) * zoom)
                    && close(ruleRect.width, px(rule.dataset.umberWidthSp) * zoom)
                    && close(ruleRect.height, px(rule.dataset.umberHeightSp) * zoom)
                    && close(baseline.left - pageRect.left, px(run.dataset.umberXSp) * zoom)
                    && close(baseline.top - pageRect.top, px(run.dataset.umberBaselineSp) * zoom);
                }
                iframe.remove();
                resolve(ok);
              } catch (error) {
                reject(error);
              }
            }, {once:true});
            iframe.srcdoc = new TextDecoder('utf-8', {fatal:true}).decode(bytes);
            document.body.append(iframe);
          });
        "#,
    );
    let promise = function
        .call1(&JsValue::NULL, &html)
        .expect("start generated HTML measurement")
        .dyn_into::<js_sys::Promise>()
        .expect("measurement promise");
    let passed = JsFuture::from(promise)
        .await
        .expect("measure generated HTML");
    assert_eq!(passed.as_bool(), Some(true));
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

    let limited_options = options("main.tex");
    let limits = Object::new();
    set(&limits, "userFiles", &JsValue::from_f64(1.0));
    set(&limited_options, "limits", limits.as_ref());
    let mut limited = CompilerSession::new(limited_options.unchecked_ref()).expect("limited");
    limited
        .add_user_file("main.tex", &bytes(b"\\end"))
        .expect("first user file");
    assert!(limited.add_user_file("extra.tex", &bytes(b"")).is_err());
}

#[wasm_bindgen_test]
fn resource_batches_use_rust_validation_and_retry_state() {
    let mut stalled_session = session("main.tex");
    stalled_session
        .add_user_file("main.tex", &bytes(b"\\input remote \\end"))
        .expect("main source");
    let missing = stalled_session.advance().expect("resource request");
    assert_eq!(string_field(missing.as_ref(), "kind"), "need-resources");

    stalled_session
        .provide_resources(&Array::new())
        .expect("empty partial batch reaches Rust");
    let stalled = stalled_session.advance().expect("typed no-progress result");
    assert_eq!(string_field(stalled.as_ref(), "kind"), "error");
    assert_eq!(
        string_field(&field(stalled.as_ref(), "diagnostic"), "code"),
        "no-progress"
    );

    let mut fresh = session("main.tex");
    fresh
        .add_user_file("main.tex", &bytes(b"\\input remote \\end"))
        .expect("main source");
    let missing = fresh.advance().expect("resource request");
    let request: Object = Array::from(&field(missing.as_ref(), "required"))
        .get(0)
        .unchecked_into();
    let first = file_response(&request, "/texlive/remote.tex", b"first");
    let conflict = file_response(&request, "/texlive/remote.tex", b"second");
    let batch = Array::of2(&first, &conflict);
    let error = fresh
        .provide_resources(&batch)
        .expect_err("conflicting batch must fail atomically");
    assert_eq!(string_field(&error, "code"), "conflicting-resource");
    assert_eq!(fresh.resolved_file_count().expect("count"), 0);

    let invalid = file_response(&request, "/texlive/../escape.tex", b"first");
    let error = fresh
        .provide_resources(&Array::of1(&invalid))
        .expect_err("invalid path");
    assert_eq!(string_field(&error, "code"), "invalid-resource");
    assert_eq!(fresh.resolved_file_count().expect("count"), 0);

    let malformed = Object::new();
    set(&malformed, "type", &JsValue::from_str("unknown"));
    let error = fresh
        .provide_resources(&Array::of1(&malformed))
        .expect_err("invalid response representation");
    assert_eq!(string_field(&error, "code"), "invalid-resource");
    assert_eq!(fresh.resolved_file_count().expect("count"), 0);

    fresh
        .provide_resources(&Array::of1(&first))
        .expect("valid response");
    fresh
        .provide_resources(&Array::of1(&first))
        .expect("exact duplicate is idempotent");
    assert_eq!(fresh.resolved_file_count().expect("count"), 1);

    let limited_options = options("main.tex");
    let limits = Object::new();
    set(&limits, "oneFileBytes", &JsValue::from_f64(1.0));
    set(&limited_options, "limits", limits.as_ref());
    let mut limited = CompilerSession::new(limited_options.unchecked_ref()).expect("limited");
    limited
        .add_user_file("main.tex", &bytes(b""))
        .expect("empty main source");
    let oversized = file_response(&request, "/texlive/remote.tex", b"xx");
    let error = limited
        .provide_resources(&Array::of1(&oversized))
        .expect_err("oversized response");
    assert_eq!(string_field(&error, "code"), "limit");
    assert_eq!(limited.resolved_file_count().expect("count"), 0);
}

#[wasm_bindgen_test]
fn unavailable_file_response_crosses_the_wire_and_counts_as_progress() {
    let mut session = session("main.tex");
    session
        .add_user_file("main.tex", &bytes(b"\\input absent \\end"))
        .expect("main source");
    let missing = session.advance().expect("resource request");
    let request = Array::from(&field(missing.as_ref(), "required")).get(0);
    let unavailable = Object::new();
    for field_name in ["domain", "kind", "name"] {
        set(&unavailable, field_name, &field(&request, field_name));
    }
    set(&unavailable, "type", &JsValue::from_str("file-unavailable"));
    session
        .provide_resources(&Array::of1(&unavailable))
        .expect("negative response");
    session
        .provide_resources(&Array::of1(&unavailable))
        .expect("duplicate negative response");
    let result = session.advance().expect("retry after negative response");
    assert_eq!(string_field(result.as_ref(), "kind"), "error");
    assert_ne!(
        string_field(&field(result.as_ref(), "diagnostic"), "code"),
        "no-progress"
    );
}

#[wasm_bindgen_test]
fn committed_plain_format_loads_and_rejects_incompatible_bytes() {
    assert_eq!(package_version(), env!("CARGO_PKG_VERSION"));
    assert_eq!(format_schema_version(), 8);
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

    for incompatible in [7_u32, 9] {
        let mut wrong_schema = format.to_vec();
        wrong_schema[8..12].copy_from_slice(&incompatible.to_le_bytes());
        assert_format_error(
            &wrong_schema,
            &format!("unsupported Umber format version {incompatible}"),
        );
    }

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

#[wasm_bindgen_test]
fn persistent_session_applies_revision_checked_patches() {
    let source = b"\\shipout\\vbox{\\hrule height 1pt}\\end";
    let mut session = session("main.tex");
    session
        .add_user_file("main.tex", &bytes(source))
        .expect("source");
    let initial = session.advance().expect("initial revision");
    assert_eq!(string_field(initial.as_ref(), "kind"), "complete");
    assert_eq!(session.revision().expect("revision"), Some(1));
    let hash = session
        .accepted_content_hash()
        .expect("hash getter")
        .expect("accepted hash");
    let start = source
        .windows(3)
        .position(|window| window == b"1pt")
        .expect("height");
    let patch = source_patch(2, 1, &hash, start, start + 1, "2");
    session
        .apply_patch(patch.unchecked_ref::<JsSourcePatch>())
        .expect("patch");
    let edited = session.advance().expect("edited revision");
    assert_eq!(string_field(edited.as_ref(), "kind"), "complete");
    assert_eq!(session.revision().expect("revision"), Some(2));

    let stale = source_patch(3, 1, &hash, start, start + 1, "3");
    assert!(
        session
            .apply_patch(stale.unchecked_ref::<JsSourcePatch>())
            .is_err()
    );
}

#[wasm_bindgen_test]
fn rendered_queries_track_length_changes_before_a_reused_page() {
    let original =
        "\\font\\tenrm=cmr10\\relax\\tenrm %a\n\\shipout\\hbox{\\char65}\\shipout\\hbox{B}\\end";
    let options = options("main.tex");
    set(&options, "html", Object::new().as_ref());
    let mut session =
        CompilerSession::new(options.unchecked_ref::<JsSessionOptions>()).expect("HTML session");
    session
        .add_user_file(
            "cmr10.tfm",
            &bytes(include_bytes!(
                "../../tex-fonts/tests/fixtures/cm/cmr10.tfm"
            )),
        )
        .expect("add TFM");
    session
        .add_user_file("main.tex", &bytes(original.as_bytes()))
        .expect("add source");
    provide_requested_html_font(&mut session);
    let initial = session.advance().expect("initial compile");
    assert_eq!(string_field(initial.as_ref(), "kind"), "complete");

    let comment = original.find("%a").expect("comment") + 1;
    let first_hash = session
        .accepted_content_hash()
        .expect("content hash")
        .expect("accepted revision");
    let first_patch = source_patch(2, 1, &first_hash, comment, comment + 1, "b");
    session
        .apply_patch(first_patch.unchecked_ref::<JsSourcePatch>())
        .expect("comment patch");
    let second = session.advance().expect("second revision");
    assert_eq!(string_field(second.as_ref(), "kind"), "complete");

    let mut revision_two = original.to_owned();
    revision_two.replace_range(comment..comment + 1, "b");
    let insert_at = revision_two.find('\n').expect("comment newline");
    let inserted = " longer";
    let second_hash = session
        .accepted_content_hash()
        .expect("content hash")
        .expect("accepted revision");
    let second_patch = source_patch(3, 2, &second_hash, insert_at, insert_at, inserted);
    session
        .apply_patch(second_patch.unchecked_ref::<JsSourcePatch>())
        .expect("length-changing patch");
    let third = session.advance().expect("third revision");
    assert_eq!(string_field(third.as_ref(), "kind"), "complete");
    let reuse = session.reuse_metrics().expect("reuse metrics");
    assert!(field(&reuse, "pagesReused").as_f64().unwrap_or_default() > 0.0);
    let third_output = field(third.as_ref(), "output");
    let third_html = String::from_utf8(Uint8Array::new(&field(&third_output, "html")).to_vec())
        .expect("third HTML");
    let b_event = rendered_text_event(&third_html, b'B');
    let output_id = rendered_output_id(&third_html);
    let mut revision_three = revision_two;
    revision_three.insert_str(insert_at, inserted);
    let b_offset = revision_three.find("{B}").expect("B box") + 1;
    let current = session
        .rendered_source_location(2, b_event, Some(0), output_id.clone(), 3)
        .expect("current query")
        .expect("current result");
    assert_eq!(string_field(current.as_ref(), "kind"), "current");
    assert_eq!(
        field(current.as_ref(), "start").as_f64(),
        Some(b_offset as f64)
    );

    let line_start = revision_three
        .find("\\shipout\\hbox{\\char65}")
        .expect("char line");
    let line_end = revision_three[line_start..]
        .find("\\shipout\\hbox{B}")
        .map(|offset| line_start + offset)
        .expect("second shipout");
    let replacement = &revision_three[line_start..line_end];
    let third_hash = session
        .accepted_content_hash()
        .expect("content hash")
        .expect("accepted revision");
    let remint = source_patch(4, 3, &third_hash, line_start, line_end, replacement);
    session
        .apply_patch(remint.unchecked_ref::<JsSourcePatch>())
        .expect("equivalent remint patch");
    let fourth = session.advance().expect("fourth revision");
    assert_eq!(string_field(fourth.as_ref(), "kind"), "complete");
    let deleted = session
        .rendered_source_location(2, b_event, Some(0), output_id, 4)
        .expect("deleted query")
        .expect("deleted result");
    assert_eq!(string_field(deleted.as_ref(), "kind"), "deleted");
    assert_eq!(
        field(deleted.as_ref(), "mintedRevision").as_f64(),
        Some(1.0)
    );
}

fn session(main_path: &str) -> CompilerSession {
    let options = options(main_path);
    CompilerSession::new(options.unchecked_ref::<JsSessionOptions>()).expect("construct session")
}

fn file_response(request: &Object, path: &str, contents: &[u8]) -> Object {
    let response = Object::assign(&Object::new(), request);
    set(&response, "virtualPath", &JsValue::from_str(path));
    set(&response, "bytes", bytes(contents).as_ref());
    response
}

fn provide_requested_html_font(session: &mut CompilerSession) {
    let missing = session.advance().expect("font request");
    assert_eq!(string_field(missing.as_ref(), "kind"), "need-resources");
    let required = Array::from(&field(missing.as_ref(), "required"));
    assert_eq!(required.length(), 1);
    let request: Object = required.get(0).unchecked_into();
    let response = Object::assign(&Object::new(), &request);
    set(&response, "container", &JsValue::from_str("woff2"));
    set(
        &response,
        "bytes",
        bytes(include_bytes!("../assets/cmu-serif-500-roman.woff2")).as_ref(),
    );
    set(
        &response,
        "provenance",
        &JsValue::from_str("test CM fixture"),
    );
    session
        .provide_resources(&Array::of1(&response))
        .expect("provide HTML font");
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

fn source_patch(
    next_revision: u32,
    base_revision: u32,
    expected_hash: &str,
    start: usize,
    end: usize,
    replacement: &str,
) -> Object {
    let patch = Object::new();
    set(
        &patch,
        "nextRevision",
        &JsValue::from_f64(f64::from(next_revision)),
    );
    set(
        &patch,
        "baseRevision",
        &JsValue::from_f64(f64::from(base_revision)),
    );
    set(&patch, "expectedHash", &JsValue::from_str(expected_hash));
    set(&patch, "start", &JsValue::from_f64(start as f64));
    set(&patch, "end", &JsValue::from_f64(end as f64));
    set(&patch, "replacement", &JsValue::from_str(replacement));
    patch
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

fn rendered_text_event(html: &str, code: u8) -> u32 {
    let marker = format!("data-umber-codes=\"0x{code:02x}");
    let codes = html.find(&marker).expect("text run");
    let event_prefix = "data-umber-event=\"";
    let event_start = html[..codes]
        .rfind(event_prefix)
        .map(|start| start + event_prefix.len())
        .expect("text event id");
    let event_end = html[event_start..]
        .find('"')
        .map(|end| event_start + end)
        .expect("event id end");
    html[event_start..event_end]
        .parse::<u32>()
        .expect("numeric event id")
}

fn rendered_output_id(html: &str) -> String {
    let prefix = "data-umber-output=\"";
    let start = html
        .find(prefix)
        .map(|start| start + prefix.len())
        .expect("rendered output id");
    let end = html[start..]
        .find('"')
        .map(|end| start + end)
        .expect("rendered output id end");
    html[start..end].to_owned()
}

fn set(object: &Object, name: &str, value: &JsValue) {
    assert!(Reflect::set(object, &JsValue::from_str(name), value).expect("set field"));
}
