#![allow(clippy::disallowed_methods)] // host tool, not engine code

use std::fs;

use anyhow::Result;
use refexec::{DviComparison, RefTex, RunOpts, compare_dvi_bytes};
use tempfile::tempdir;

#[test]
fn locate_finds_pdftex() -> Result<()> {
    RefTex::locate()?;
    Ok(())
}

#[test]
fn run_trivial_tex_and_capture_log() -> Result<()> {
    let temp_dir = tempdir()?;
    let tex_file = temp_dir.path().join("hello.tex");
    fs::write(&tex_file, r"\message{refexec-ok}\end")?;

    let output = RefTex::locate()?.run(&tex_file, &RunOpts::default())?;

    assert!(output.success);
    assert!(output.log.contains("refexec-ok"));
    Ok(())
}

#[test]
fn dvi_run_captures_dvi_preamble() -> Result<()> {
    let temp_dir = tempdir()?;
    let tex_file = temp_dir.path().join("page.tex");
    fs::write(&tex_file, r"\shipout\hbox{}\end")?;

    let output = RefTex::locate()?.run(
        &tex_file,
        &RunOpts {
            dvi: true,
            ..RunOpts::default()
        },
    )?;

    assert!(output.success);
    let dvi = output.dvi.expect("DVI output should be captured");
    assert_eq!(dvi.first(), Some(&247));
    Ok(())
}

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
