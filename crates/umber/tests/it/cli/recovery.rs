//! Recoverable diagnostics, usage errors, and CLI context rendering.

use super::*;

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
         \\copy0 \\penalty-10000\n\
         \\end\n",
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
    fs::write(&source, "}\n\\end\n").expect("write diagnostic fixture");

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
    fs::write(&source, "\\undefined\n\\end\n").expect("write expansion diagnostic fixture");

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
    // tex.web §370 prints the message alone; §82's `show_context` display is
    // what names the offending control sequence, on its own `l.N` line.
    assert!(stdout.contains("! Undefined control sequence."), "{stdout}");
    assert!(stdout.contains("l.1 \\undefined"), "{stdout}");
    assert!(
        output.stderr.is_empty(),
        "recovered error must not reach stderr"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_show_publishes_canonical_error_context() {
    let temp_dir = tempfile::tempdir().expect("create show-context temp dir");
    let source = temp_dir.path().join("show.tex");
    fs::write(&source, "\\def\\foo{bar}\\show\\foo\\end\n").expect("write show fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .output()
        .expect("run umber show fixture");

    assert!(output.status.success(), "recovered show should succeed");
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    // TeX82 §§82, 310 publish the live input context after `show_eqtb`'s
    // diagnostic and before returning from the recoverable error.
    assert!(stdout.contains("> \\foo=macro:\n->bar."), "{stdout}");
    assert!(
        stdout.contains("l.1 \\def\\foo{bar}\\show\\foo"),
        "{stdout}"
    );
    assert!(
        output.stderr.is_empty(),
        "recovered diagnostic must stay on the terminal channel"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_recovers_from_extra_endgroup_in_macro() {
    let temp_dir = tempfile::tempdir().expect("create macro diagnostic temp dir");
    let source = temp_dir.path().join("macro.tex");
    fs::write(&source, "\\def\\a{\\endgroup}\\a\n\\end\n").expect("write macro diagnostic fixture");

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
