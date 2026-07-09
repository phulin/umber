#![allow(clippy::disallowed_methods)] // host-side parity test files.

use std::fs;

use crate::{Executor, install_unexpandable_primitives};
use refexec::{RefTex, RunOpts};
use tempfile::tempdir;
use test_support::{
    assert_matches_fixture, live_reference_enabled, normalize, read_fixture,
    update_fixtures_enabled,
};
use tex_lex::{InputStack, MemoryInput};
use tex_state::Universe;
use tex_state::meaning::{ExpandablePrimitive, Meaning};

#[test]
fn grouping_after_tokens_match_pdftex_micro_suite() {
    let grouping = pdftex_reference_fixture(
        "grouping",
        r"{\count100=1\global\count101=2}\message{G:\the\count100,\the\count101}\end",
    );
    assert!(
        grouping.contains("success: true"),
        "pdftex grouping case failed:\n{}",
        grouping
    );
    assert!(
        grouping.contains("G:0,2"),
        "pdftex grouping output changed:\n{}",
        grouping
    );

    let after = pdftex_reference_fixture(
        "after",
        r"\def\A{\message{A}}\def\B{\message{B}}{\aftergroup\B\afterassignment\A\count1=7}\end",
    );
    assert!(
        after.contains("success: true"),
        "pdftex after-token case failed:\n{}",
        after
    );
    assert!(
        after.contains("A B"),
        "pdftex after-token ordering changed:\n{}",
        after
    );

    let too_many = pdftex_reference_fixture("too_many", "}\n\\end");
    assert!(too_many.contains("! Too many }'s."));
    let wrong_close = pdftex_reference_fixture("wrong_close", "\\begingroup}\n\\end");
    assert!(wrong_close.contains("! Extra }, or forgotten \\endgroup."));

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
    let illegal = pdftex_reference_fixture(
        "illegal_mag",
        r"\mag=40000\dimen0=1truept\showthe\dimen0\end",
    );
    assert!(illegal.contains("! Illegal magnification has been changed to 1000 (40000)."));

    let incompatible = pdftex_reference_fixture(
        "incompatible_mag",
        r"\mag=1200\dimen0=1truept\mag=2000\dimen1=1truept\showthe\dimen1\end",
    );
    assert!(incompatible.contains("! Incompatible magnification (2000);"));
    assert!(incompatible.contains("reverted to the magnification you used earlier"));
    assert!(incompatible.contains("> 0.83333pt."));

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
    let dimensions = pdftex_reference_fixture(
        "box_dimensions",
        r"\setbox0=\hbox to 10pt{}\wd0=12pt\ht0=3pt\dp0=2pt\message{B:\the\wd0,\the\ht0,\the\dp0}\end",
    );
    assert!(
        dimensions.contains("B:12.0pt,3.0pt,2.0pt"),
        "pdftex box dimension output changed:\n{}",
        dimensions
    );

    let movement = pdftex_reference_fixture(
        "box_movement",
        r"\setbox0=\hbox{}\setbox1=\copy0\box0\message{M:\ifvoid0 void\else full\fi,\ifvoid1 full\else void\fi}\end",
    );
    assert!(
        movement.contains("M:void,void"),
        "pdftex box movement output changed:\n{}",
        movement
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

fn pdftex_reference_fixture(stem: &str, input: &str) -> String {
    if update_fixtures_enabled() || live_reference_enabled() {
        let temp_dir = tempdir().expect("create reftex temp dir");
        let output = run_pdftex(temp_dir.path(), stem, input);
        let fixture = format_pdftex_reference(&output);
        assert_matches_fixture("tex_exec", stem, "ref", &fixture);
        fixture
    } else {
        read_fixture("tex_exec", stem, "ref")
    }
}

fn run_pdftex(dir: &std::path::Path, stem: &str, input: &str) -> refexec::RunOutput {
    let tex_file = dir.join(format!("{stem}.tex"));
    fs::write(&tex_file, input).expect("write reftex input");
    RefTex::locate()
        .expect("locate pdftex")
        .run(&tex_file, &RunOpts::default())
        .expect("run pdftex")
}

fn format_pdftex_reference(output: &refexec::RunOutput) -> String {
    format!(
        "success: {}\nstdout:\n{}log:\n{}",
        output.success,
        normalize_micro_reference_text(&output.stdout),
        normalize_micro_reference_text(&output.log)
    )
}

fn normalize_micro_reference_text(text: &str) -> String {
    let mut lines = Vec::new();
    for line in normalize::exec_log(text).lines() {
        let line = line.split_once(" [").map_or(line, |(message, _)| message);
        if line.starts_with("Output written on ")
            || line.starts_with("pdftex/")
            || line.starts_with("lic/")
            || line.starts_with("</")
        {
            continue;
        }
        lines.push(line.to_owned());
    }

    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
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
