//! Committed terminal, log, and DVI corpus comparisons through the CLI.

use super::*;

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary fixture setup and command execution.
fn run_recovered_diagnostic_after_tfm_load_exits_successfully() {
    let temp_dir = tempfile::tempdir().expect("create font provenance temp dir");
    let source = temp_dir.path().join("after-font.tex");
    let child = temp_dir.path().join("child.tex");
    let tfm = temp_dir.path().join("cmr10.tfm");
    fs::write(&source, "\\font\\f=cmr10 \\relax\n\\input child\n\\end\n")
        .expect("write main fixture");
    fs::write(&child, "\\global X\n").expect("write diagnostic fixture");
    fs::copy(
        test_support::repository_root().join("crates/tex-fonts/tests/fixtures/cm/cmr10.tfm"),
        &tfm,
    )
    .expect("copy TFM fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .output()
        .expect("run font provenance fixture");

    assert!(
        output.status.success(),
        "recovered prefix error should succeed"
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("You can't use a prefix"), "{stdout}");
    assert!(
        output.stderr.is_empty(),
        "recovered error must not reach stderr"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side corpus discovery and command execution.
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn run_exec_corpus_matches_committed_diagnostics() {
    run_corpus_matches_committed_terminal_fixtures(
        "exec",
        false,
        &["hmode_material_primitives"], // umber2-johp.757
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side corpus discovery and command execution.
fn run_etex_exec_corpus_matches_committed_diagnostics() {
    for case in corpus_cases("etex_exec") {
        assert_log_case_matches_committed_fixture("etex_exec", &case, false, true);
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side corpus discovery and command execution.
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn run_typeset_corpus_matches_committed_box_dumps() {
    run_corpus_matches_committed_terminal_fixtures(
        "typeset",
        true,
        &[
            "alignment_showlists_unset", // umber2-johp.758
            "material_primitives",       // umber2-johp.757
        ],
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_math_corpus_matches_committed_dvi() {
    assert_dvi_area_matches_committed_fixture("math");
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_align_corpus_matches_committed_dvi() {
    assert_dvi_area_matches_committed_fixture("align");
}
