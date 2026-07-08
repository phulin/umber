#![allow(clippy::disallowed_methods)] // host-side parity test files.

use std::fs;

use crate::{Executor, install_unexpandable_primitives};
use refexec::{RefTex, RunOpts};
use tempfile::tempdir;
use tex_lex::{InputStack, MemoryInput};
use tex_state::Universe;
use tex_state::meaning::{ExpandablePrimitive, Meaning};

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

#[test]
fn prepare_mag_cases_match_pdftex_micro_suite() {
    let temp_dir = tempdir().expect("create reftex temp dir");

    let illegal = run_pdftex(
        temp_dir.path(),
        "illegal_mag",
        r"\mag=40000\dimen0=1truept\showthe\dimen0\end",
    );
    assert!(
        illegal
            .log
            .contains("! Illegal magnification has been changed to 1000 (40000).")
    );

    let incompatible = run_pdftex(
        temp_dir.path(),
        "incompatible_mag",
        r"\mag=1200\dimen0=1truept\mag=2000\dimen1=1truept\showthe\dimen1\end",
    );
    assert!(
        incompatible
            .log
            .contains("! Incompatible magnification (2000);")
    );
    assert!(
        incompatible
            .log
            .contains("the previous value will be retained")
    );
    assert!(incompatible.log.contains("> 0.83333pt."));

    let stores = run_umber_exec(r"\mag=1200\dimen0=1truept\mag=2000\dimen1=1truept");
    assert_eq!(stores.mag(), 1200);
    assert_eq!(stores.prepared_mag(), Some(1200));
    assert_eq!(stores.dimen(0).raw(), 54_613);
    assert_eq!(stores.dimen(1).raw(), 54_613);

    let stores = run_umber_exec(r"\mag=40000\dimen0=1truept");
    assert_eq!(stores.mag(), 1000);
    assert_eq!(stores.prepared_mag(), Some(1000));
    assert_eq!(stores.dimen(0).raw(), 65_536);
}

#[test]
fn box_register_cases_match_pdftex_micro_suite() {
    let temp_dir = tempdir().expect("create reftex temp dir");

    let dimensions = run_pdftex(
        temp_dir.path(),
        "box_dimensions",
        r"\setbox0=\hbox to 10pt{}\wd0=12pt\ht0=3pt\dp0=2pt\message{B:\the\wd0,\the\ht0,\the\dp0}\end",
    );
    assert!(
        dimensions.stdout.contains("B:12.0pt,3.0pt,2.0pt"),
        "pdftex box dimension output changed:\n{}",
        dimensions.stdout
    );

    let movement = run_pdftex(
        temp_dir.path(),
        "box_movement",
        r"\setbox0=\hbox{}\setbox1=\copy0\box0\message{M:\ifvoid0 void\else full\fi,\ifvoid1 full\else void\fi}\end",
    );
    assert!(
        movement.stdout.contains("M:void,void"),
        "pdftex box movement output changed:\n{}",
        movement.stdout
    );

    let mut stores =
        run_umber_exec_with_box_expandables(r"\setbox0=\hbox to 10pt{}\wd0=12pt\ht0=3pt\dp0=2pt");
    assert_eq!(
        stores
            .box_dimension(0, tex_state::BoxDimension::Width)
            .expect("box width should be readable")
            .raw(),
        12 * 65_536
    );
    assert_eq!(
        stores
            .box_dimension(0, tex_state::BoxDimension::Height)
            .expect("box height should be readable")
            .raw(),
        3 * 65_536
    );
    assert_eq!(
        stores
            .box_dimension(0, tex_state::BoxDimension::Depth)
            .expect("box depth should be readable")
            .raw(),
        2 * 65_536
    );

    stores = run_umber_exec_with_box_expandables(r"\setbox0=\hbox{}\setbox1=\copy0\box0");
    assert!(stores.box_reg(0).is_none());
    assert!(stores.box_reg(1).is_some());
}

fn run_pdftex(dir: &std::path::Path, stem: &str, input: &str) -> refexec::RunOutput {
    let tex_file = dir.join(format!("{stem}.tex"));
    fs::write(&tex_file, input).expect("write reftex input");
    RefTex::locate()
        .expect("locate pdftex")
        .run(&tex_file, &RunOpts::default())
        .expect("run pdftex")
}

fn run_umber_exec(input: &str) -> Universe {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(input));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect("umber execution succeeds");
    stores
}

fn run_umber_exec_with_box_expandables(input: &str) -> Universe {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    for (name, primitive) in [
        ("the", ExpandablePrimitive::The),
        ("ifvoid", ExpandablePrimitive::IfVoid),
        ("else", ExpandablePrimitive::Else),
        ("fi", ExpandablePrimitive::Fi),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::ExpandablePrimitive(primitive));
    }
    let mut input = InputStack::new(MemoryInput::new(input));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect("umber execution succeeds");
    stores
}
