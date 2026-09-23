//! Independent expansion and execution guards and fixture harvesting.

use super::*;

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_reports_independent_expansion_fuel_and_execution_step_caps() {
    let temp_dir = tempfile::tempdir().expect("create guard receipt temp dir");
    let source = temp_dir.path().join("main.tex");
    fs::write(&source, "\\end\n").expect("write guard receipt source");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .arg("run")
        .arg("--expansion-fuel")
        .arg("50000000")
        .arg("--execution-steps")
        .arg("100000000")
        .arg(&source)
        .output()
        .expect("run explicitly guarded fixture");

    assert!(
        output.status.success(),
        "explicitly guarded run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("RUN_GUARDS expansion_fuel_cap=50000000 execution_steps_cap=100000000"),
        "guard diagnostic must name both independent caps:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn step_cap_failure_reports_both_independent_guards() {
    let temp_dir = tempfile::tempdir().expect("create step guard temp dir");
    let source = temp_dir.path().join("main.tex");
    fs::write(&source, "\\shipout\\hbox{}\\shipout\\hbox{}\\end\n")
        .expect("write step guard source");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .arg("run")
        .arg("--expansion-fuel")
        .arg("50000000")
        .arg("--execution-steps")
        .arg("1")
        .arg(&source)
        .output()
        .expect("run step-limited fixture");

    assert!(!output.status.success(), "step-limited run must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("RUN_GUARDS expansion_fuel_cap=50000000 execution_steps_cap=1"),
        "guard diagnostic must name both independent caps:\n{stderr}"
    );
    assert!(
        stderr.contains("execution steps budget 1 exceeded at 2"),
        "terminal diagnostic must identify the committed-step cap:\n{stderr}"
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
    let cmr10 = test_support::repository_root()
        .join("crates/umber")
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
