use std::{fs, process::Command};

use sha2::{Digest, Sha256};
use test_support::{
    CorpusCase, assert_matches_fixture, corpus_cases, dvi, normalize, read_binary_fixture,
};
use tex_lex::{Lexer, WorldInput};
use tex_state::env::banks::IntParam;
use tex_state::token::{Catcode, Token};
use tex_state::{Universe, World};

const PINNED_SOURCE_DATE_EPOCH: &str = "1783604160";

#[test]
fn exits_successfully() {
    let status = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .status()
        .expect("failed to run umber binary");

    assert!(status.success());
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn pdftex_rule_page_is_published_only_to_an_explicit_distinct_pdf_path() {
    let temp_dir = tempfile::tempdir().expect("create PDF output temp dir");
    let source = temp_dir.path().join("rule.tex");
    let pdf = temp_dir.path().join("rule.pdf");
    let dvi = temp_dir.path().join("rule.dvi");
    fs::write(
        &source,
        "\\pdfoutput=1\\pdfcompresslevel=0\\shipout\\vbox{\\hrule width10pt height5pt}\\end\n",
    )
    .expect("write PDF rule fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--pdftex")
        .arg("--pdf")
        .arg(&pdf)
        .arg("--dvi")
        .arg(&dvi)
        .arg(&source)
        .output()
        .expect("run pdfTeX PDF fixture");

    assert!(
        output.status.success(),
        "pdfTeX run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let pdf_bytes = fs::read(&pdf).expect("read published PDF");
    assert!(pdf_bytes.starts_with(b"%PDF-1.4"));
    assert!(pdf_bytes.ends_with(b"%%EOF"));
    assert!(fs::metadata(&dvi).expect("published DVI").len() > 0);

    let rejected = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--pdf")
        .arg(temp_dir.path().join("wrong-mode.pdf"))
        .arg(&source)
        .output()
        .expect("reject PDF without pdfTeX mode");
    assert!(!rejected.status.success());
    assert_eq!(
        String::from_utf8(rejected.stderr).expect("stderr is utf-8"),
        "umber: --pdf requires --pdftex or --pdflatex\n"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn pdflatex_mode_composes_latex_compatibility_with_pdf_output() {
    let temp_dir = tempfile::tempdir().expect("create pdfLaTeX output temp dir");
    let source = temp_dir.path().join("composed.tex");
    let pdf = temp_dir.path().join("composed.pdf");
    fs::write(
        &source,
        "\\catcode123=1\\catcode125=2\\pdfoutput=1\\ifnum\\strcmp{same}{same}=0\\shipout\\vbox{\\hrule width10pt height5pt}\\fi\\end\n",
    )
    .expect("write composed pdfLaTeX fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--pdflatex")
        .arg("--pdf")
        .arg(&pdf)
        .arg(&source)
        .output()
        .expect("run composed pdfLaTeX fixture");

    assert!(
        output.status.success(),
        "pdfLaTeX run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let pdf_bytes = fs::read(&pdf).expect("read composed pdfLaTeX PDF");
    assert!(pdf_bytes.starts_with(b"%PDF-1.4"));
    assert!(pdf_bytes.ends_with(b"%%EOF"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn pdfdraftmode_does_not_replace_the_requested_pdf_output() {
    let temp_dir = tempfile::tempdir().expect("create draft-mode output temp dir");
    let source = temp_dir.path().join("draft.tex");
    let pdf = temp_dir.path().join("draft.pdf");
    fs::write(
        &source,
        "\\pdfoutput=1\\pdfdraftmode=1\\shipout\\vbox{\\hrule width10pt height5pt}\\end\n",
    )
    .expect("write draft-mode fixture");
    fs::write(&pdf, b"existing output\n").expect("seed existing PDF path");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--pdftex")
        .arg("--pdf")
        .arg(&pdf)
        .arg(&source)
        .output()
        .expect("run draft-mode fixture");

    assert!(
        output.status.success(),
        "draft-mode run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr is utf-8"),
        "pdfTeX warning: \\pdfdraftmode enabled, not changing output pdf\n"
    );
    assert_eq!(
        fs::read(&pdf).expect("read unchanged output"),
        b"existing output\n"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn pdf_lowering_failure_does_not_publish_any_driver_output() {
    let temp_dir = tempfile::tempdir().expect("create PDF failure temp dir");
    let source = temp_dir.path().join("text.tex");
    let pdf = temp_dir.path().join("text.pdf");
    let dvi = temp_dir.path().join("text.dvi");
    fs::write(
        &source,
        "\\pdfoutput=1\\shipout\\vbox{\\special{unsupported}}\\end\n",
    )
    .expect("write unsupported PDF special fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--pdftex")
        .arg("--pdf")
        .arg(&pdf)
        .arg("--dvi")
        .arg(&dvi)
        .arg(&source)
        .output()
        .expect("run unsupported PDF special fixture");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("PDF output does not support special")
    );
    assert!(!pdf.exists(), "failed PDF finalization published a file");
    assert!(
        !dvi.exists(),
        "failed PDF finalization published peer output"
    );
}

#[cfg(feature = "profiling-stats")]
#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary file and command execution.
fn profiling_stats_are_reported_only_when_requested() {
    let temp_dir = tempfile::tempdir().expect("create profiling stats temp dir");
    let source = temp_dir.path().join("stats.tex");
    fs::write(&source, "\\end\n").expect("write profiling stats fixture");

    let quiet = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .output()
        .expect("run instrumented umber without reporting");
    assert!(quiet.status.success());
    assert!(quiet.stderr.is_empty());

    let reported = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--profiling-stats")
        .arg(&source)
        .output()
        .expect("run instrumented umber with reporting");
    assert!(reported.status.success());
    let stderr = String::from_utf8(reported.stderr).expect("stderr is utf-8");
    assert!(stderr.contains("EXPANSION_STATS "));
    assert!(stderr.contains("NODE_MEMORY_TOTAL "));
    assert!(stderr.contains("ALLOC_NODE_APPEND "));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side fixture discovery and expected-output reads.
fn lex_dump_prints_stable_token_format_for_corpus() {
    for case in corpus_cases("lexer") {
        let output = Command::new(env!("CARGO_BIN_EXE_umber"))
            .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
            .arg("lex-dump")
            .arg(case.source_path())
            .output()
            .expect("run umber lex-dump");

        assert!(
            output.status.success(),
            "lex-dump failed for {}:\n{}",
            case.source_path().display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let actual = String::from_utf8(output.stdout).expect("lex-dump output is utf-8");
        assert_matches_fixture("lexer", case.name(), "tokens", &actual);
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side fixture discovery and expected-output reads.
fn expand_dump_prints_stable_token_format_for_corpus() {
    for case in corpus_cases("expand") {
        let output = Command::new(env!("CARGO_BIN_EXE_umber"))
            .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
            .arg("expand-dump")
            .arg(case.source_path())
            .output()
            .expect("run umber expand-dump");

        assert!(
            output.status.success(),
            "expand-dump failed for {}:\n{}",
            case.source_path().display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let actual = String::from_utf8(output.stdout).expect("expand-dump output is utf-8");
        assert_matches_fixture("expand", case.name(), "tokens", &actual);
    }
}

#[test]
fn expand_dump_usage_errors_follow_lex_dump_shape() {
    let missing = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("expand-dump")
        .output()
        .expect("run umber expand-dump without path");
    assert!(!missing.status.success());
    assert_eq!(
        String::from_utf8(missing.stderr).expect("stderr is utf-8"),
        "umber: missing input path for expand-dump\n"
    );

    let extra = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("expand-dump")
        .arg("one.tex")
        .arg("two.tex")
        .output()
        .expect("run umber expand-dump with extra path");
    assert!(!extra.status.success());
    assert_eq!(
        String::from_utf8(extra.stderr).expect("stderr is utf-8"),
        "umber: expand-dump accepts exactly one input path\n"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn expand_dump_expansion_error_renders_primary_source_context() {
    let temp_dir = tempfile::tempdir().expect("create diagnostic temp dir");
    let source = temp_dir.path().join("undefined.tex");
    fs::write(&source, "\\undefined\n").expect("write diagnostic fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("expand-dump")
        .arg(&source)
        .output()
        .expect("run umber expand-dump diagnostic fixture");

    assert!(
        !output.status.success(),
        "undefined expand-dump should fail"
    );
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf-8");
    assert!(stderr.contains("Undefined control sequence \\undefined"));
    assert!(stderr.contains("undefined.tex:1:1"));
    assert!(stderr.contains("  1 | \\undefined"));
    assert!(stderr.contains("    | ^"));
    assert!(!stderr.contains("unknown origin"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn expand_dump_macro_error_renders_bounded_expansion_trace() {
    let temp_dir = tempfile::tempdir().expect("create macro diagnostic temp dir");
    let source = temp_dir.path().join("macro.tex");
    fs::write(&source, "\\def\\a{\\undefined X}\\a\n").expect("write diagnostic fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("expand-dump")
        .arg(&source)
        .output()
        .expect("run umber expand-dump macro diagnostic fixture");

    assert!(!output.status.success(), "macro expand-dump should fail");
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf-8");
    assert!(stderr.contains("Undefined control sequence \\undefined"));
    assert!(stderr.contains("macro.tex:1:8"));
    assert!(stderr.contains("expansion trace:"));
    assert!(stderr.contains("invoked at"));
    assert!(stderr.contains("defined at"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn expand_dump_recovered_execution_error_exits_successfully() {
    let temp_dir = tempfile::tempdir().expect("create execution diagnostic temp dir");
    let source = temp_dir.path().join("prefix.tex");
    fs::write(&source, "\\global X\n").expect("write diagnostic fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("expand-dump")
        .arg(&source)
        .output()
        .expect("run umber expand-dump execution diagnostic fixture");

    assert!(
        output.status.success(),
        "recovered prefix error should succeed"
    );
    assert!(
        output.stderr.is_empty(),
        "recovered error must not reach stderr"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary fixture setup and command execution.
fn run_recovered_diagnostic_after_tfm_load_exits_successfully() {
    let temp_dir = tempfile::tempdir().expect("create font provenance temp dir");
    let source = temp_dir.path().join("after-font.tex");
    let child = temp_dir.path().join("child.tex");
    let tfm = temp_dir.path().join("cmr10.tfm");
    fs::write(&source, "\\font\\f=cmr10 \\relax\n\\input child\n").expect("write main fixture");
    fs::write(&child, "\\global X\n").expect("write diagnostic fixture");
    fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../tex-fonts/tests/fixtures/cm/cmr10.tfm"
        ),
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
fn run_exec_corpus_matches_committed_diagnostics() {
    run_corpus_matches_committed_log_fixtures("exec", false, &[]);
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
fn run_typeset_corpus_matches_committed_box_dumps() {
    run_corpus_matches_committed_log_fixtures("typeset", true, &[]);
}

#[allow(clippy::disallowed_methods)] // host-side corpus discovery and command execution.
fn run_corpus_matches_committed_log_fixtures(
    area: &str,
    show_fixtures: bool,
    ignored_cases: &[&str],
) {
    for case in corpus_cases(area) {
        if !ignored_cases.contains(&case.name()) {
            assert_log_case_matches_committed_fixture(area, &case, show_fixtures, false);
        }
    }
}

#[allow(clippy::disallowed_methods)] // host-side command execution and expected-output reads.
fn assert_log_case_matches_committed_fixture(
    area: &str,
    case: &CorpusCase,
    show_fixtures: bool,
    etex: bool,
) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_umber"));
    command.env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH);
    if etex {
        command.current_dir(
            case.source_path()
                .parent()
                .expect("corpus source has a parent directory"),
        );
    }
    command.arg("run");
    if etex {
        command.arg("--etex");
    }
    if show_fixtures {
        command.arg("--show-fixtures");
    }
    let output = command
        .arg(case.source_path())
        .output()
        .expect("run umber run");
    assert!(
        output.status.success(),
        "umber run failed for {}:\n{}",
        case.source_path().display(),
        String::from_utf8_lossy(&output.stderr)
    );
    let actual_stdout = String::from_utf8(output.stdout).expect("umber run output is utf-8");
    let actual = if show_fixtures {
        normalize::box_dump(&actual_stdout)
    } else {
        normalize::exec_log(&actual_stdout)
    };
    assert_matches_fixture(area, case.name(), "log", &actual);
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_dvi_corpus_matches_committed_dvi() {
    assert_dvi_area_matches_committed_fixture("dvi");
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_page_corpus_matches_committed_dvi() {
    assert_dvi_area_matches_committed_fixture("page");
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

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_leaders_corpus_matches_committed_dvi() {
    assert_dvi_area_matches_committed_fixture("leaders");
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary bundle and command execution.
fn run_html_and_dvi_share_one_run_and_publish_deterministically() {
    let setup = dvi::DviCaseSetup::new("dvi", "boxes_rules");
    let font_dir = setup.run_dir().join("web-fonts");
    fs::create_dir(&font_dir).expect("create web-font bundle");
    install_test_web_font(&font_dir, &setup.run_dir().join("cmr10.tfm"), "cmr10");

    let invoke = |dvi: &str, html: &str, assets: &str| {
        Command::new(env!("CARGO_BIN_EXE_umber"))
            .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
            .current_dir(setup.run_dir())
            .args([
                "run",
                setup.source_file_name(),
                "--dvi",
                dvi,
                "--html",
                html,
                "--html-font-dir",
                "web-fonts",
                "--html-assets",
                assets,
            ])
            .output()
            .expect("run simultaneous DVI and HTML")
    };
    for (dvi, html, assets) in [
        ("first.dvi", "first.html", "assets"),
        ("second.dvi", "second.html", "assets"),
    ] {
        let output = invoke(dvi, html, assets);
        assert!(
            output.status.success(),
            "simultaneous output failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let expected = read_binary_fixture("dvi", "boxes_rules", "dvi");
    for name in ["first.dvi", "second.dvi"] {
        let actual = fs::read(setup.run_dir().join(name)).expect("read DVI output");
        dvi::assert_dvi_matches(&expected, &actual, name);
    }
    let first = fs::read(setup.run_dir().join("first.html")).expect("read first HTML");
    let second = fs::read(setup.run_dir().join("second.html")).expect("read second HTML");
    assert_eq!(first, second);
    let html = String::from_utf8(first).expect("HTML is UTF-8");
    assert!(html.contains("data-umber-baseline-sp="));
    assert!(html.contains("assets/"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side focused corpus and temporary font bundles.
fn focused_html_corpora_pass_the_dvi_coordinate_oracle() {
    for area in ["dvi", "page", "math", "align", "leaders"] {
        for case in corpus_cases(area) {
            let setup = dvi::DviCaseSetup::new(area, case.name());
            let font_dir = setup.run_dir().join("web-fonts");
            fs::create_dir(&font_dir).expect("create web-font bundle");
            for tfm in setup
                .extra_inputs()
                .iter()
                .filter(|path| path.extension().is_some_and(|ext| ext == "tfm"))
            {
                let name = tfm
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .expect("TFM name is UTF-8");
                install_test_web_font(&font_dir, tfm, name);
            }
            let output = Command::new(env!("CARGO_BIN_EXE_umber"))
                .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
                .current_dir(setup.run_dir())
                .args([
                    "run",
                    setup.source_file_name(),
                    "--dvi",
                    "actual.dvi",
                    "--html",
                    "actual.html",
                    "--html-font-dir",
                    "web-fonts",
                    "--html-assets",
                    "assets",
                ])
                .output()
                .expect("run focused HTML coordinate case");
            assert!(
                output.status.success(),
                "HTML coordinate oracle failed for {area}/{}:\n{}",
                case.name(),
                String::from_utf8_lossy(&output.stderr)
            );
            let actual = fs::read(setup.actual_dvi_path()).expect("read DVI output");
            let expected = read_binary_fixture(area, case.name(), "dvi");
            dvi::assert_dvi_matches(&expected, &actual, &format!("{area}/{}", case.name()));
        }
    }
}

#[allow(clippy::disallowed_methods)] // host-side temporary web-font bundle.
fn install_test_web_font(directory: &std::path::Path, tfm: &std::path::Path, name: &str) {
    let tfm = fs::read(tfm).expect("read TFM fixture");
    let woff2 = include_bytes!("../../../umber-wasm/assets/cmu-serif-500-roman.woff2");
    let woff_digest: [u8; 32] = Sha256::digest(woff2).into();
    fs::write(directory.join(format!("{name}.woff2")), woff2).expect("write WOFF2");
    fs::write(
        directory.join(format!("{name}.woff2.sha256")),
        hex(&woff_digest),
    )
    .expect("write WOFF2 digest");
    fs::write(
        directory.join(format!("{name}.tfm-hash")),
        tex_out::ContentHash::from_bytes(&tfm).hex(),
    )
    .expect("write TFM identity");
    fs::write(
        directory.join(format!("{name}.license")),
        "Computer Modern Unicode 0.7.0; SIL Open Font License 1.1",
    )
    .expect("write license");
    let mapping = (0u16..=255)
        .map(|code| {
            let mapped = match code {
                0 => "Γ".to_owned(),
                1 => "Δ".to_owned(),
                2 => "Θ".to_owned(),
                3 => "Λ".to_owned(),
                4 => "Ξ".to_owned(),
                5 => "Π".to_owned(),
                6 => "Σ".to_owned(),
                7 => "Υ".to_owned(),
                8 => "Φ".to_owned(),
                9 => "Ψ".to_owned(),
                10 => "Ω".to_owned(),
                16 => "ı".to_owned(),
                17 => "ȷ".to_owned(),
                18 => "`".to_owned(),
                19 => "´".to_owned(),
                20 => "ˇ".to_owned(),
                21 => "˘".to_owned(),
                22 => "¯".to_owned(),
                23 => "˚".to_owned(),
                24 => "¸".to_owned(),
                25 => "ß".to_owned(),
                26 => "æ".to_owned(),
                27 => "œ".to_owned(),
                28 => "ø".to_owned(),
                29 => "Æ".to_owned(),
                30 => "Œ".to_owned(),
                31 => "Ø".to_owned(),
                45 => "‐".to_owned(),
                32..=126 => char::from_u32(u32::from(code)).expect("ASCII").to_string(),
                127 => "¨".to_owned(),
                // This corpus gate compares exact coordinates, not glyph artwork.
                // The single bundled Roman face stands in for math faces here, so
                // retain a cmap-backed placeholder for codes outside its OT1 map.
                _ => "A".to_owned(),
            };
            format!("{code:02x}\t{mapped}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(directory.join(format!("{name}.map")), mapping).expect("write encoding map");
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_initializes_clock_parameters_from_source_date_epoch() {
    let temp_dir = tempfile::tempdir().expect("create clock temp dir");
    let source = temp_dir.path().join("clock.tex");
    fs::write(
        &source,
        "\\message{clock=\\the\\time/\\the\\day/\\the\\month/\\the\\year}\\end\n",
    )
    .expect("write clock fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--show-fixtures")
        .arg(&source)
        .output()
        .expect("run umber clock fixture");

    assert!(
        output.status.success(),
        "clock run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("clock=816/9/7/2026"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn latex_creationdate_uses_the_source_date_epoch_job_clock() {
    let temp_dir = tempfile::tempdir().expect("create creation-date temp dir");
    let source = temp_dir.path().join("creationdate.tex");
    fs::write(
        &source,
        "\\catcode123=1 \\catcode125=2 \\message{created=\\creationdate}\\end\n",
    )
    .expect("write creation-date fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--latex")
        .arg("--show-fixtures")
        .arg(&source)
        .output()
        .expect("run Umber LaTeX creation-date fixture");

    assert!(
        output.status.success(),
        "creation-date run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("created=D:20260709133600Z"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn pdftex_mode_reports_the_pinned_engine_identity() {
    let temp_dir = tempfile::tempdir().expect("create pdfTeX identity temp dir");
    let source = temp_dir.path().join("identity.tex");
    fs::write(
        &source,
        "\\message{engine=\\the\\pdftexversion\\pdftexrevision}\\end\n",
    )
    .expect("write pdfTeX identity fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .arg("run")
        .arg("--pdftex")
        .arg("--show-fixtures")
        .arg(&source)
        .output()
        .expect("run Umber pdfTeX identity fixture");

    assert!(
        output.status.success(),
        "pdfTeX run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stdout)
            .expect("stdout is utf-8")
            .contains("engine=140.27")
    );
}

#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn assert_dvi_area_matches_committed_fixture(area: &str) {
    for case in corpus_cases(area) {
        assert_dvi_case_matches_committed_fixture(area, case.name());
    }
}

#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn assert_dvi_case_matches_committed_fixture(area: &str, case: &str) {
    let setup = dvi::DviCaseSetup::new(area, case);

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .current_dir(setup.run_dir())
        .arg("run")
        .arg(setup.source_file_name())
        .arg("--dvi")
        .arg(setup.actual_dvi_file_name())
        .output()
        .expect("run umber DVI smoke");
    assert!(
        output.status.success(),
        "umber DVI smoke failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let actual = fs::read(setup.actual_dvi_path()).expect("read umber DVI");
    let expected = read_binary_fixture(area, case, "dvi");
    dvi::assert_dvi_matches(&expected, &actual, &format!("{area}/{case}"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_recovers_from_deadcycles_overflow() {
    let temp_dir = tempfile::tempdir().expect("create deadcycles temp dir");
    let source = temp_dir.path().join("deadcycles.tex");
    fs::write(
        &source,
        "\\maxdeadcycles=1 \\output={\\setbox1=\\box255}\n\
         \\topskip=0pt \\setbox0=\\hbox{}\n\
         \\copy0 \\penalty-10000\n\
         \\copy0 \\penalty-10000\n",
    )
    .expect("write deadcycles fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .output()
        .expect("run umber deadcycles fixture");

    assert!(
        output.status.success(),
        "recovered deadcycles overflow should succeed"
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("Output loop---1 consecutive dead cycles"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_recovers_from_extra_right_brace() {
    let temp_dir = tempfile::tempdir().expect("create diagnostic temp dir");
    let source = temp_dir.path().join("brace.tex");
    fs::write(&source, "}\n").expect("write diagnostic fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .output()
        .expect("run umber diagnostic fixture");

    assert!(
        output.status.success(),
        "recovered extra brace should succeed"
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("Too many }'s."));
    assert!(
        output.stderr.is_empty(),
        "recovered error must not reach stderr"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_recovers_from_undefined_control_sequence() {
    let temp_dir = tempfile::tempdir().expect("create expansion diagnostic temp dir");
    let source = temp_dir.path().join("undefined.tex");
    fs::write(&source, "\\undefined\n").expect("write expansion diagnostic fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .output()
        .expect("run umber expansion diagnostic fixture");

    assert!(
        output.status.success(),
        "recovered undefined control sequence should succeed"
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("Undefined control sequence \\undefined"));
    assert!(
        output.stderr.is_empty(),
        "recovered error must not reach stderr"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_recovers_from_extra_endgroup_in_macro() {
    let temp_dir = tempfile::tempdir().expect("create macro diagnostic temp dir");
    let source = temp_dir.path().join("macro.tex");
    fs::write(&source, "\\def\\a{\\endgroup}\\a\n").expect("write macro diagnostic fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .output()
        .expect("run umber macro diagnostic fixture");

    assert!(
        output.status.success(),
        "recovered extra endgroup should succeed"
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("Extra \\endgroup."));
    assert!(
        output.stderr.is_empty(),
        "recovered error must not reach stderr"
    );
}

#[test]
fn run_usage_errors_follow_existing_shape() {
    let missing = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .output()
        .expect("run umber run without path");
    assert!(!missing.status.success());
    assert_eq!(
        String::from_utf8(missing.stderr).expect("stderr is utf-8"),
        "umber: missing input path for run\n"
    );

    let extra = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("one.tex")
        .arg("two.tex")
        .output()
        .expect("run umber run with extra path");
    assert!(!extra.status.success());
    assert_eq!(
        String::from_utf8(extra.stderr).expect("stderr is utf-8"),
        "umber: run accepts one input path with optional --show-fixtures and --dvi <path>\n"
    );

    let removed_plain_format = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("one.tex")
        .arg("--plain-format")
        .output()
        .expect("run umber run with removed --plain-format flag");
    assert!(!removed_plain_format.status.success());
    assert_eq!(
        String::from_utf8(removed_plain_format.stderr).expect("stderr is utf-8"),
        "umber: run accepts one input path with optional --show-fixtures and --dvi <path>\n"
    );

    let missing_show_fixtures = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--show-fixtures")
        .output()
        .expect("run umber run with show-fixtures but without path");
    assert!(!missing_show_fixtures.status.success());
    assert_eq!(
        String::from_utf8(missing_show_fixtures.stderr).expect("stderr is utf-8"),
        "umber: missing input path for run\n"
    );

    let missing_dvi_path = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("one.tex")
        .arg("--dvi")
        .output()
        .expect("run umber run with --dvi but without output path");
    assert!(!missing_dvi_path.status.success());
    assert_eq!(
        String::from_utf8(missing_dvi_path.stderr).expect("stderr is utf-8"),
        "umber: missing output path for --dvi\n"
    );

    let conflicting_outputs = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .args([
            "run",
            "one.tex",
            "--dvi",
            "same.out",
            "--format-out",
            "same.out",
        ])
        .output()
        .expect("run umber with conflicting output paths");
    assert!(!conflicting_outputs.status.success());
    assert_eq!(
        String::from_utf8(conflicting_outputs.stderr).expect("stderr is utf-8"),
        "umber: --dvi and --format-out must use different output paths\n"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_resolves_area_less_input_through_texinputs_and_advances() {
    let temp_dir = tempfile::tempdir().expect("create TeX input search temp dir");
    let job_dir = temp_dir.path().join("plain/base");
    let search_dir = temp_dir.path().join("generic/hyphen");
    fs::create_dir_all(&job_dir).expect("create principal input directory");
    fs::create_dir_all(&search_dir).expect("create TeX input search directory");
    let source = job_dir.join("plain.tex");
    fs::write(&source, "\\input hyphen \\message{after-hyphen}\\end\n")
        .expect("write principal input");
    fs::write(search_dir.join("hyphen.tex"), "\\message{loaded-hyphen}\n")
        .expect("write searched input");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .env("TEXINPUTS", &search_dir)
        .arg("run")
        .arg(&source)
        .arg("--show-fixtures")
        .output()
        .expect("run input search smoke");

    assert!(
        output.status.success(),
        "input search run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("loaded-hyphen"));
    assert!(stdout.contains("after-hyphen"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_writes_a_sorted_deduplicated_input_record_receipt() {
    let temp_dir = tempfile::tempdir().expect("create input receipt temp dir");
    let source = temp_dir.path().join("main.tex");
    let helper = temp_dir.path().join("helper.tex");
    let receipt = temp_dir.path().join("inputs.tsv");
    let source_bytes = b"\\input helper \\input helper \\end\n";
    let helper_bytes = b"\\relax\n";
    fs::write(&source, source_bytes).expect("write principal input");
    fs::write(&helper, helper_bytes).expect("write included input");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .arg("--input-records-out")
        .arg(&receipt)
        .output()
        .expect("run input receipt smoke");

    assert!(
        output.status.success(),
        "input receipt run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected = format!(
        "{}\t{}\n{}\t{}\n",
        helper_bytes.len(),
        helper.display(),
        source_bytes.len(),
        source.display()
    );
    assert_eq!(
        fs::read_to_string(receipt).expect("read input receipt"),
        expected
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_resolves_quoted_openin_through_texinputs() {
    let temp_dir = tempfile::tempdir().expect("create TeX stream search temp dir");
    let job_dir = temp_dir.path().join("latex/base");
    let search_dir = temp_dir.path().join("latex/l3kernel");
    fs::create_dir_all(&job_dir).expect("create principal input directory");
    fs::create_dir_all(&search_dir).expect("create TeX stream search directory");
    let source = job_dir.join("stream-search.tex");
    fs::write(
        &source,
        "\\openin1=\"probe.tex\" \\ifeof1 \\errmessage{missing-probe}\\else \\message{found-probe}\\fi \\end\n",
    )
    .expect("write stream search input");
    fs::write(search_dir.join("probe.tex"), "found\n").expect("write searched stream");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .env("TEXINPUTS", &search_dir)
        .arg("run")
        .arg(&source)
        .arg("--show-fixtures")
        .output()
        .expect("run stream search smoke");

    assert!(
        output.status.success(),
        "stream search run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("found-probe"));
    assert!(!stdout.contains("missing-probe"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_resolves_area_less_tfm_through_texfonts_and_advances() {
    let temp_dir = tempfile::tempdir().expect("create TeX font search temp dir");
    let job_dir = temp_dir.path().join("plain/base");
    let font_dir = temp_dir.path().join("fonts/tfm/public/cm");
    fs::create_dir_all(&job_dir).expect("create principal input directory");
    fs::create_dir_all(&font_dir).expect("create TeX font search directory");
    let source = job_dir.join("font-search.tex");
    fs::write(
        &source,
        "\\font\\tenrm=cmr10 \\relax \\message{after-font}\\end\n",
    )
    .expect("write font search input");
    let cmr10 = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    fs::copy(cmr10, font_dir.join("cmr10.tfm")).expect("copy searched TFM");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .env("TEXFONTS", &font_dir)
        .arg("run")
        .arg(&source)
        .arg("--show-fixtures")
        .output()
        .expect("run font search smoke");

    assert!(
        output.status.success(),
        "font search run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("after-font"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side fixture command execution and file checks.
fn run_show_fixtures_harvests_without_committing_immediate_stream_effects() {
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let normal_dir = temp_dir.path().join("normal");
    let fixture_dir = temp_dir.path().join("fixture");
    fs::create_dir_all(&normal_dir).expect("create normal dir");
    fs::create_dir_all(&fixture_dir).expect("create fixture dir");
    let input = temp_dir.path().join("stream_effect.tex");
    fs::write(
        &input,
        "\\immediate\\openout0=side-effect.txt\n\
         \\immediate\\write0{immediate-effect}\n\
         \\immediate\\closeout0\n\\end\n",
    )
    .expect("write input");

    let normal = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .current_dir(&normal_dir)
        .arg("run")
        .arg(&input)
        .output()
        .expect("run ordinary umber run");
    assert!(
        normal.status.success(),
        "ordinary run failed:\n{}",
        String::from_utf8_lossy(&normal.stderr)
    );
    assert!(
        normal_dir.join("side-effect.txt").exists(),
        "ordinary run should commit immediate stream effects at final commit"
    );

    let fixture = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .current_dir(&fixture_dir)
        .arg("run")
        .arg("--show-fixtures")
        .arg(&input)
        .output()
        .expect("run umber fixture harvest");
    assert!(
        fixture.status.success(),
        "fixture run failed:\n{}",
        String::from_utf8_lossy(&fixture.stderr)
    );
    assert!(
        !fixture_dir.join("side-effect.txt").exists(),
        "--show-fixtures must not run the final commit for pending immediate effects"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side corpus files, not engine I/O.
fn lexer_dynamic_corpus_covers_mutable_input_state() {
    assert_matches_fixture(
        "lexer_dynamic",
        "catcode_mutation",
        "tokens",
        &lex_catcode_mutation_fixture(),
    );
    assert_matches_fixture(
        "lexer_dynamic",
        "endlinechar_mutation",
        "tokens",
        &lex_endlinechar_mutation_fixture(),
    );
    assert_matches_fixture(
        "lexer_dynamic",
        "ignored_character",
        "tokens",
        &lex_ignored_character_fixture(),
    );
    assert_matches_fixture(
        "lexer_dynamic",
        "invalid_character",
        "tokens",
        &lex_invalid_character_fixture(),
    );
}

fn lex_catcode_mutation_fixture() -> String {
    let (mut lexer, mut stores) = lexer_fixture("catcode_mutation");
    let mut actual = String::new();

    push_next_token(&mut actual, &mut lexer, &mut stores);
    stores.set_catcode('@', Catcode::Letter);
    push_remaining_tokens(&mut actual, &mut lexer, &mut stores);

    actual
}

fn lex_endlinechar_mutation_fixture() -> String {
    let (mut lexer, mut stores) = lexer_fixture("endlinechar_mutation");
    stores.set_int_param(IntParam::END_LINE_CHAR, b'!' as i32);
    let mut actual = String::new();

    push_next_token(&mut actual, &mut lexer, &mut stores);
    push_next_token(&mut actual, &mut lexer, &mut stores);
    stores.set_int_param(IntParam::END_LINE_CHAR, b'?' as i32);
    push_next_token(&mut actual, &mut lexer, &mut stores);
    push_next_token(&mut actual, &mut lexer, &mut stores);
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    push_remaining_tokens(&mut actual, &mut lexer, &mut stores);

    actual
}

fn lex_ignored_character_fixture() -> String {
    let (mut lexer, mut stores) = lexer_fixture("ignored_character");
    stores.set_catcode('!', Catcode::Ignored);
    let mut actual = String::new();

    push_remaining_tokens(&mut actual, &mut lexer, &mut stores);

    actual
}

fn lex_invalid_character_fixture() -> String {
    let (mut lexer, mut stores) = lexer_fixture("invalid_character");
    stores.set_catcode('?', Catcode::Invalid);
    let mut actual = String::new();

    loop {
        match lexer.next_token(&mut stores) {
            Ok(Some(token)) => push_token(&mut actual, token, &stores),
            Ok(None) => break,
            Err(err) => {
                actual.push_str(&format!("error:{err}\n"));
                break;
            }
        }
    }

    actual
}

fn lexer_fixture(case: &str) -> (Lexer, Universe) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/corpus/lexer_dynamic")
        .join(format!("{case}.tex"));
    let mut stores = Universe::with_world(World::real());
    let content = stores
        .world_mut()
        .read_file(&path)
        .expect("open dynamic lexer fixture");
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    (Lexer::new(WorldInput::from_content(content)), stores)
}

fn push_remaining_tokens(actual: &mut String, lexer: &mut Lexer, stores: &mut Universe) {
    while let Some(token) = lexer
        .next_token(stores)
        .expect("dynamic lexer fixture should succeed")
    {
        push_token(actual, token, stores);
    }
}

fn push_next_token(actual: &mut String, lexer: &mut Lexer, stores: &mut Universe) {
    let token = lexer
        .next_token(stores)
        .expect("dynamic lexer fixture should succeed")
        .expect("dynamic lexer fixture ended early");
    push_token(actual, token, stores);
}

fn push_token(actual: &mut String, token: Token, stores: &Universe) {
    let line = match token {
        Token::Char { ch, cat } => format!("char:{}:{}", ch as u32, cat as u8),
        Token::Cs(symbol) => format!("cs:{}", stores.resolve(symbol)),
        Token::Param(slot) => format!("param:{slot}"),
        token if token.is_frozen_end_template() => "frozen:endtemplate".to_owned(),
        token if token.is_frozen_endv() => "frozen:endv".to_owned(),
        Token::Frozen(_) => unreachable!("invalid frozen token payload"),
    };
    actual.push_str(&line);
    actual.push('\n');
}
