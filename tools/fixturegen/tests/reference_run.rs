#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use anyhow::Result;

fn fake_reference(root: &std::path::Path, body: &str) -> Result<std::path::PathBuf> {
    let executable = root.join("fake-pdftex");
    fs::write(&executable, body)?;
    let mut permissions = fs::metadata(&executable)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&executable, permissions)?;
    Ok(executable)
}

#[test]
fn direct_reference_run_preserves_staging_environment_and_requested_dvi() -> Result<()> {
    let root = tempfile::tempdir()?;
    let tex = root.path().join("case.tex");
    let extra = root.path().join("extra.dat");
    let dvi = root.path().join("staged.dvi");
    fs::write(&tex, "\\end\n")?;
    fs::write(&extra, "extra\n")?;
    let executable = fake_reference(
        root.path(),
        r#"#!/bin/sh
set -eu
last=
for arg in "$@"; do last="$arg"; done
stem=${last%.tex}
printf 'STDOUT %s %s %s\n' "$SOURCE_DATE_EPOCH" "$FORCE_SOURCE_DATE" "$(test -f extra.dat && printf staged)"
printf 'LOG-BYTES\n' > "$stem.log"
printf 'DVI-BYTES' > "$stem.dvi"
"#,
    )?;

    let output = Command::new(env!("CARGO_BIN_EXE_fixturegen"))
        .env("UMBER_REF_TEX", executable)
        .env_remove("SOURCE_DATE_EPOCH")
        .env_remove("FORCE_SOURCE_DATE")
        .arg("--reference-run")
        .arg(&tex)
        .arg("--dvi-output")
        .arg(&dvi)
        .args(["--ini", "--print-log", "--extra-input"])
        .arg(&extra)
        .output()?;
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(
        String::from_utf8(output.stdout)?,
        "STDOUT 1783604160 1 staged\nLOG-BYTES\n"
    );
    assert!(output.stderr.is_empty());
    assert_eq!(fs::read(dvi)?, b"DVI-BYTES");
    Ok(())
}

#[test]
fn unsuccessful_reference_run_exits_nonzero_without_publishing_dvi() -> Result<()> {
    let root = tempfile::tempdir()?;
    let tex = root.path().join("case.tex");
    fs::write(&tex, "\\end\n")?;
    let executable = fake_reference(
        root.path(),
        "#!/bin/sh\nset -eu\nlast=\nfor arg in \"$@\"; do last=\"$arg\"; done\nstem=${last%.tex}\nprintf 'LOG-BYTES\\n' > \"$stem.log\"\nexit 7\n",
    )?;

    let output = Command::new(env!("CARGO_BIN_EXE_fixturegen"))
        .env("UMBER_REF_TEX", executable)
        .arg("--reference-run")
        .arg(&tex)
        .output()?;
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr)?.contains("exited unsuccessfully"));
    Ok(())
}
