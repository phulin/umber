use std::{fs, process::Command};

use test_support::{assert_matches_fixture, corpus_cases, dvi, normalize, read_binary_fixture};
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
fn expand_dump_execution_error_renders_primary_source_context() {
    let temp_dir = tempfile::tempdir().expect("create execution diagnostic temp dir");
    let source = temp_dir.path().join("prefix.tex");
    fs::write(&source, "\\global X\n").expect("write diagnostic fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("expand-dump")
        .arg(&source)
        .output()
        .expect("run umber expand-dump execution diagnostic fixture");

    assert!(!output.status.success(), "prefix expand-dump should fail");
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf-8");
    assert!(stderr.contains("You can't use a prefix"));
    assert!(stderr.contains("prefix.tex:1:9"));
    assert!(stderr.contains("  1 | \\global X"));
    assert!(stderr.contains("|         ^"));
    assert!(!stderr.contains("unknown origin"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary fixture setup and command execution.
fn run_diagnostic_after_tfm_load_keeps_tex_source_path() {
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

    assert!(!output.status.success(), "invalid prefix use should fail");
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf-8");
    assert!(stderr.contains("You can't use a prefix"), "{stderr}");
    assert!(stderr.contains("child.tex:1:9"), "{stderr}");
    assert!(stderr.contains("  1 | \\global X"), "{stderr}");
    assert!(!stderr.contains("cmr10.tfm:1:9"), "{stderr}");
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side corpus discovery and command execution.
fn run_exec_corpus_matches_committed_diagnostics() {
    run_corpus_matches_committed_log_fixtures("exec", false);
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side corpus discovery and command execution.
fn run_typeset_corpus_matches_committed_box_dumps() {
    run_corpus_matches_committed_log_fixtures("typeset", true);
}

#[allow(clippy::disallowed_methods)] // host-side corpus discovery and command execution.
fn run_corpus_matches_committed_log_fixtures(area: &str, show_fixtures: bool) {
    for case in corpus_cases(area) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_umber"));
        command.env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH);
        command.arg("run");
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
fn run_reports_deadcycles_overflow_primary_text() {
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

    assert!(!output.status.success(), "deadcycles run should fail");
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf-8");
    assert!(stderr.contains("Output loop---1 consecutive dead cycles"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_error_renders_primary_source_context() {
    let temp_dir = tempfile::tempdir().expect("create diagnostic temp dir");
    let source = temp_dir.path().join("brace.tex");
    fs::write(&source, "}\n").expect("write diagnostic fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .output()
        .expect("run umber diagnostic fixture");

    assert!(!output.status.success(), "brace run should fail");
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf-8");
    assert!(stderr.contains("Too many }'s."));
    assert!(stderr.contains("brace.tex:1:1"));
    assert!(stderr.contains("  1 | }"));
    assert!(stderr.contains("    | ^"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_expansion_error_renders_primary_source_context() {
    let temp_dir = tempfile::tempdir().expect("create expansion diagnostic temp dir");
    let source = temp_dir.path().join("undefined.tex");
    fs::write(&source, "\\undefined\n").expect("write expansion diagnostic fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .output()
        .expect("run umber expansion diagnostic fixture");

    assert!(!output.status.success(), "undefined run should fail");
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf-8");
    assert!(stderr.contains("Undefined control sequence \\undefined"));
    assert!(stderr.contains("undefined.tex:1:1"));
    assert!(stderr.contains("  1 | \\undefined"));
    assert!(stderr.contains("    | ^"));
    assert!(!stderr.contains("unknown origin"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_macro_error_renders_bounded_expansion_trace() {
    let temp_dir = tempfile::tempdir().expect("create macro diagnostic temp dir");
    let source = temp_dir.path().join("macro.tex");
    fs::write(&source, "\\def\\a{\\endgroup}\\a\n").expect("write macro diagnostic fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .output()
        .expect("run umber macro diagnostic fixture");

    assert!(!output.status.success(), "macro run should fail");
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf-8");
    assert!(stderr.contains("Extra \\endgroup."));
    assert!(stderr.contains("macro.tex:1:8"));
    assert!(stderr.contains("expansion trace:"));
    assert!(stderr.contains("invoked at"));
    assert!(stderr.contains("defined at"));
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
        "umber: run accepts one input path with optional --show-fixtures, --plain-format, and --dvi <path>\n"
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
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_plain_format_bootstrap_defines_corpus_plain_macros() {
    let temp_dir = tempfile::tempdir().expect("create plain bootstrap temp dir");
    copy_plain_bootstrap_test_tfms(temp_dir.path());
    let source = temp_dir.path().join("plain_bootstrap.tex");
    fs::write(
        &source,
        "\\centerline{A}\n\
         \\newif\\ifamrfonts\n\
         \\amrfontsfalse\n\
         \\ifamrfonts\\message{bad}\\else\\message{ok}\\fi\n\
         \\end\n",
    )
    .expect("write plain bootstrap fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .current_dir(temp_dir.path())
        .arg("run")
        .arg("plain_bootstrap.tex")
        .arg("--plain-format")
        .arg("--show-fixtures")
        .output()
        .expect("run umber plain bootstrap smoke");

    assert!(
        output.status.success(),
        "plain bootstrap run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("ok"));
    assert!(!stdout.contains("bad"));
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

fn copy_plain_bootstrap_test_tfms(dir: &std::path::Path) {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    for name in ["cmr10", "cmbx10", "cmsl10", "cmtt10", "cmti10"] {
        fs::copy(&source, dir.join(format!("{name}.tfm"))).expect("copy test TFM");
    }
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

fn lexer_fixture(case: &str) -> (Lexer<WorldInput>, Universe) {
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

fn push_remaining_tokens(
    actual: &mut String,
    lexer: &mut Lexer<WorldInput>,
    stores: &mut Universe,
) {
    while let Some(token) = lexer
        .next_token(stores)
        .expect("dynamic lexer fixture should succeed")
    {
        push_token(actual, token, stores);
    }
}

fn push_next_token(actual: &mut String, lexer: &mut Lexer<WorldInput>, stores: &mut Universe) {
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
        Token::Frozen(tex_state::token::FrozenToken::EndTemplate) => {
            "frozen:endtemplate".to_owned()
        }
        Token::Frozen(tex_state::token::FrozenToken::EndV) => "frozen:endv".to_owned(),
    };
    actual.push_str(&line);
    actual.push('\n');
}
