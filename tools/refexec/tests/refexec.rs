#![allow(clippy::disallowed_methods)] // host tool, not engine code

use anyhow::Result;
use refexec::{DviComparison, compare_dvi_bytes};

#[test]
fn dvi_compare_normalizes_only_preamble_comment_payload() -> Result<()> {
    let mut left = vec![247, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 232, 3];
    left.extend_from_slice(b"abc");
    left.extend_from_slice(&[139, 140]);
    let mut right = vec![247, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 232, 3];
    right.extend_from_slice(b"xyz");
    right.extend_from_slice(&[139, 140]);

    assert_eq!(compare_dvi_bytes(&left, &right)?, DviComparison::Equal);

    right[18] = 141;
    let DviComparison::Different(diff) = compare_dvi_bytes(&left, &right)? else {
        panic!("body byte mismatch should be reported");
    };
    assert_eq!(diff.offset, 18);
    Ok(())
}

#[cfg(unix)]
#[test]
fn compatibility_cli_preserves_reference_run_contract() -> Result<()> {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    let root = tempfile::tempdir()?;
    let tex = root.path().join("case.tex");
    let extra = root.path().join("extra.dat");
    fs::write(&tex, "\\end\n")?;
    fs::write(&extra, "extra\n")?;
    let executable = root.path().join("fake-pdftex");
    fs::write(
        &executable,
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
    let mut permissions = fs::metadata(&executable)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&executable, permissions)?;

    let output = Command::new(env!("CARGO_BIN_EXE_refexec"))
        .env("UMBER_REF_TEX", &executable)
        .env_remove("SOURCE_DATE_EPOCH")
        .env_remove("FORCE_SOURCE_DATE")
        .arg(&tex)
        .arg("--dvi")
        .arg("--ini")
        .arg("--print-log")
        .arg("--extra-input")
        .arg(&extra)
        .output()?;
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout)?,
        "STDOUT 1783604160 1 staged\nLOG-BYTES\n"
    );
    assert!(output.stderr.is_empty());
    assert_eq!(fs::read(tex.with_extension("ref.dvi"))?, b"DVI-BYTES");
    Ok(())
}

#[cfg(unix)]
#[test]
fn compatibility_cli_maps_reference_failure_to_nonzero_status() -> Result<()> {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    let root = tempfile::tempdir()?;
    let tex = root.path().join("case.tex");
    fs::write(&tex, "\\end\n")?;
    let executable = root.path().join("fake-pdftex");
    fs::write(
        &executable,
        "#!/bin/sh\nset -eu\nlast=\nfor arg in \"$@\"; do last=\"$arg\"; done\nstem=${last%.tex}\nprintf 'LOG-BYTES\\n' > \"$stem.log\"\nexit 7\n",
    )?;
    let mut permissions = fs::metadata(&executable)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&executable, permissions)?;

    let output = Command::new(env!("CARGO_BIN_EXE_refexec"))
        .env("UMBER_REF_TEX", executable)
        .arg(tex)
        .output()?;
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    Ok(())
}
