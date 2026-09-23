//! Input identity, engine identity, and reproducible job clock behavior.

use super::*;

#[test]
#[allow(clippy::disallowed_methods)] // host-side command execution.
fn removed_html_font_directory_names_the_typed_replacement() {
    // Any small committed source will do: this asserts an argument-parsing
    // rejection that never reaches the engine. `dvi`/`page` were retired
    // into the minifixture corpus, so it names a surviving area.
    let setup = dvi::DviCaseSetup::new("math", "accents");
    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .current_dir(setup.run_dir())
        .args([
            "run",
            setup.source_file_name(),
            "--html",
            "actual.html",
            "--html-font-dir",
            "web-fonts",
        ])
        .output()
        .expect("run removed HTML font option");
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("--html-font-dir was removed"));
    assert!(error.contains("typed resource resolver API"));
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
        "\\message{engine=\\the\\pdftexversion.\\pdftexrevision}\\end\n",
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
