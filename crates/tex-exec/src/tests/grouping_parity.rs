#![allow(clippy::disallowed_methods)] // host-side parity test files.

use crate::{Executor, install_unexpandable_primitives};
use test_support::read_fixture;
use tex_lex::{InputStack, MemoryInput};
use tex_state::Universe;
use tex_state::meaning::{ExpandablePrimitive, Meaning};

#[test]
fn grouping_after_tokens_match_reference_micro_suite() {
    let grouping = reference_fixture("grouping");
    assert!(
        grouping.contains("success: true"),
        "reference grouping case failed:\n{}",
        grouping
    );
    assert!(
        grouping.contains("G:0,2"),
        "reference grouping output changed:\n{}",
        grouping
    );

    let after = reference_fixture("after");
    assert!(
        after.contains("success: true"),
        "reference after-token case failed:\n{}",
        after
    );
    assert!(
        after.contains("A B"),
        "reference after-token ordering changed:\n{}",
        after
    );

    let too_many = reference_fixture("too_many");
    assert!(too_many.contains("! Too many }'s."));
    let wrong_close = reference_fixture("wrong_close");
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
fn prepare_mag_cases_match_reference_micro_suite() {
    let illegal = reference_fixture("illegal_mag");
    assert!(illegal.contains("! Illegal magnification has been changed to 1000 (40000)."));

    let incompatible = reference_fixture("incompatible_mag");
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
fn box_register_cases_match_reference_micro_suite() {
    let brace_aliases = reference_fixture("box_brace_aliases");
    assert!(
        brace_aliases.contains("B:7.0pt"),
        "reference brace-alias box output changed:\n{}",
        brace_aliases
    );

    let dimensions = reference_fixture("box_dimensions");
    assert!(
        dimensions.contains("B:12.0pt,3.0pt,2.0pt"),
        "reference box dimension output changed:\n{}",
        dimensions
    );

    let movement = reference_fixture("box_movement");
    assert!(
        movement.contains("M:void,void"),
        "reference box movement output changed:\n{}",
        movement
    );
    let uncopy_badness = reference_fixture("box_uncopy_badness");
    assert!(
        uncopy_badness.contains("B:10000"),
        "reference badness output changed:\n{}",
        uncopy_badness
    );
    assert!(
        uncopy_badness.contains("H:kept") && uncopy_badness.contains("V:kept"),
        "reference uncopy register behavior changed:\n{}",
        uncopy_badness
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

    stores = run_umber_exec_with_box_expandables(
        r"\setbox0=\hbox to 10pt{\hskip0pt plus1pt}\count0=\badness\setbox1=\hbox{\unhcopy0}",
    );
    assert_eq!(stores.count(0), 10_000);
    assert!(stores.box_reg(0).is_some());

    stores =
        run_umber_exec(r"\let\bgroup={\let\egroup=}\setbox0=\vbox\bgroup\hrule height7pt\egroup");
    assert_eq!(
        stores
            .box_dimension(0, tex_state::BoxDimension::Height)
            .expect("aliased box delimiters should produce a vbox")
            .raw(),
        7 * 65_536
    );
}

fn reference_fixture(stem: &str) -> String {
    read_fixture("tex_exec", stem, "ref")
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
