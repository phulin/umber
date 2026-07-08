#![allow(clippy::disallowed_methods)] // host-side parity test files.

use std::fs;

use refexec::{RefTex, RunOpts};
use tempfile::tempdir;
use tex_exec::{Executor, install_unexpandable_primitives};
use tex_lex::{InputStack, MemoryInput};
use tex_state::stores::Stores;

#[test]
fn grouping_after_tokens_match_pdftex_micro_suite() {
    let temp_dir = tempdir().expect("create reftex temp dir");

    let grouping = run_pdftex(
        temp_dir.path(),
        "grouping",
        r"{\count100=1\global\count101=2}\message{G:\the\count100,\the\count101}\end",
    );
    assert!(
        grouping.success,
        "pdftex grouping case failed:\n{}",
        grouping.stdout
    );
    assert!(
        grouping.stdout.contains("G:0,2"),
        "pdftex grouping output changed:\n{}",
        grouping.stdout
    );

    let after = run_pdftex(
        temp_dir.path(),
        "after",
        r"\def\A{\message{A}}\def\B{\message{B}}{\aftergroup\B\afterassignment\A\count1=7}\end",
    );
    assert!(
        after.success,
        "pdftex after-token case failed:\n{}",
        after.stdout
    );
    assert!(
        after.stdout.contains("A B"),
        "pdftex after-token ordering changed:\n{}",
        after.stdout
    );

    let too_many = run_pdftex(temp_dir.path(), "too_many", "}\n\\end");
    assert!(too_many.log.contains("! Too many }'s."));
    let wrong_close = run_pdftex(temp_dir.path(), "wrong_close", "\\begingroup}\n\\end");
    assert!(
        wrong_close
            .log
            .contains("! Extra }, or forgotten \\endgroup.")
    );

    let stores = run_umber_exec(
        r"{\count100=1\global\count101=2}\def\A{\global\count0=1}\def\B{\global\count0=2}{\aftergroup\B\afterassignment\A\count102=7}",
    );
    assert_eq!(stores.count(0), 2);
    assert_eq!(stores.count(100), 0);
    assert_eq!(stores.count(101), 2);
    assert_eq!(stores.count(102), 0);
}

fn run_pdftex(dir: &std::path::Path, stem: &str, input: &str) -> refexec::RunOutput {
    let tex_file = dir.join(format!("{stem}.tex"));
    fs::write(&tex_file, input).expect("write reftex input");
    RefTex::locate()
        .expect("locate pdftex")
        .run(&tex_file, &RunOpts::default())
        .expect("run pdftex")
}

fn run_umber_exec(input: &str) -> Stores {
    let mut stores = Stores::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(input));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect("umber execution succeeds");
    stores
}
