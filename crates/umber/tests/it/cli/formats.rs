//! Format dump publication and failure atomicity at the CLI boundary.

use super::*;

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_publishes_a_dumped_format_from_the_resource_session() {
    let temp_dir = tempfile::tempdir().expect("create format output temp dir");
    let source = temp_dir.path().join("format.tex");
    let format = temp_dir.path().join("format.fmt");
    fs::write(&source, "\\catcode`@=11 \\dump\n").expect("write format source");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .args(["run", "--format-out"])
        .arg(&format)
        .arg(&source)
        .output()
        .expect("run format dump");

    assert!(
        output.status.success(),
        "format dump failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!fs::read(&format).expect("read dumped format").is_empty());
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains(&format!("Beginning to dump on file {}", format.display()))
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn initex_dump_survives_futurelet_reusing_a_control_sequence_that_means_space() {
    let temp_dir = tempfile::tempdir().expect("create format output temp dir");
    let source = temp_dir.path().join("futurelet-space.tex");
    let format = temp_dir.path().join("futurelet-space.fmt");
    fs::write(
        &source,
        concat!(
            "\\catcode`\\@=11\n",
            "\\long\\def\\@ifnextchar#1#2#3{\\let\\reserved@d=#1",
            "\\def\\reserved@a{#2}\\def\\reserved@b{#3}",
            "\\futurelet\\@let@token\\@ifnch}\n",
            "\\def\\@ifnch{\\ifx\\@let@token\\@sptoken",
            "\\let\\reserved@c\\@xifnch\\else",
            "\\ifx\\@let@token\\reserved@d\\let\\reserved@c\\reserved@a",
            "\\else\\let\\reserved@c\\reserved@b\\fi\\fi\\reserved@c}\n",
            "\\def\\:{\\let\\@sptoken= } \\: \n",
            "\\def\\:{\\@xifnch} \\expandafter\\def\\: ",
            "{\\futurelet\\@let@token\\@ifnch}\n",
            "\\def\\consume[#1]{\\def\\result{yes}}\n",
            "\\def\\bad{\\def\\result{no}}\n",
            "\\def\\probe{\\@ifnextchar[{\\consume}{\\bad}}\n",
            "\\probe\n  [1]\n",
            "\\def\\expected{yes}\n",
            "\\ifx\\result\\expected\\else\\errmessage{lookahead failed}\\fi\n",
            "\\dump\n",
        ),
    )
    .expect("write futurelet format source");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .args(["run", "--format-out"])
        .arg(&format)
        .arg(&source)
        .output()
        .expect("run futurelet format dump");

    assert!(
        output.status.success(),
        "futurelet format dump failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!fs::read(&format).expect("read dumped format").is_empty());
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_publishes_dump_to_default_tex82_name_before_announcing_it() {
    let temp_dir = tempfile::tempdir().expect("create format output temp dir");
    let source = temp_dir.path().join("plain.tex");
    fs::write(&source, "\\dump\n").expect("write format source");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .current_dir(temp_dir.path())
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .args(["run", "plain.tex"])
        .output()
        .expect("run default format dump");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !fs::read(temp_dir.path().join("plain.fmt"))
            .expect("read default dumped format")
            .is_empty()
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Beginning to dump on file plain.fmt")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn failed_format_publication_never_announces_success() {
    let temp_dir = tempfile::tempdir().expect("create format output temp dir");
    let source = temp_dir.path().join("plain.tex");
    fs::write(&source, "\\dump\n").expect("write format source");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .args(["run", "--format-out", "/proc/umber-unwritable.fmt"])
        .arg(&source)
        .output()
        .expect("run failed format publication");

    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Beginning to dump on file"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn format_output_rejects_a_successful_run_that_did_not_dump() {
    let temp_dir = tempfile::tempdir().expect("create format output temp dir");
    let source = temp_dir.path().join("format.tex");
    let format = temp_dir.path().join("format.fmt");
    fs::write(&source, "\\message{NO-DUMP}\\end\n").expect("write format source");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .args(["run", "--format-out"])
        .arg(&format)
        .arg(&source)
        .output()
        .expect("run missing format dump");

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "umber: --format-out requires the input to execute \\dump\n"
    );
    assert!(!format.exists());
}
