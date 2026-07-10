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
fn prefix_expands_macros_before_selecting_the_assignment() {
    let prefixed_macro = reference_fixture("prefixed_macro");
    assert!(
        prefixed_macro.contains("P:7"),
        "reference prefixed-macro output changed:\n{}",
        prefixed_macro
    );

    let stores =
        run_umber_exec(r"\def\setglobal{\count0=7}{\global\relax\setglobal}\count1=\count0");
    assert_eq!(stores.count(0), 7);
    assert_eq!(stores.count(1), 7);
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

#[test]
fn last_box_cases_match_reference_micro_suite() {
    let last_box = reference_fixture("last_box");
    assert!(
        last_box.contains("L:0.0pt,7.0pt;0.0pt,8.0pt;void;3.0pt,0.0pt;11.0pt;12.0pt;void,void"),
        "reference last-box behavior changed:\n{last_box}"
    );
    assert!(last_box.contains("usually can't take things from the current page"));
    assert!(last_box.contains("You can't use `\\lastbox' in math mode"));

    let stores = run_umber_exec_with_box_expandables(include_str!(
        "../../../../tests/corpus/tex_exec/last_box.tex"
    ));
    assert_eq!(
        stores
            .box_dimension(1, tex_state::BoxDimension::Width)
            .expect("horizontal lastbox")
            .raw(),
        7 * tex_state::scaled::Scaled::UNITY
    );
    assert_eq!(
        stores
            .box_dimension(3, tex_state::BoxDimension::Width)
            .expect("internal vertical lastbox")
            .raw(),
        8 * tex_state::scaled::Scaled::UNITY
    );
    assert!(stores.box_reg(5).is_none(), "a non-box tail blocks lastbox");
    assert_eq!(
        stores
            .box_dimension(6, tex_state::BoxDimension::Width)
            .expect("local lastbox assignment restores")
            .raw(),
        3 * tex_state::scaled::Scaled::UNITY
    );
    assert_eq!(
        stores
            .box_dimension(8, tex_state::BoxDimension::Width)
            .expect("global lastbox assignment persists")
            .raw(),
        11 * tex_state::scaled::Scaled::UNITY
    );
    assert_eq!(
        stores
            .box_dimension(11, tex_state::BoxDimension::Width)
            .expect("outer vertical unboxed tail remains available")
            .raw(),
        12 * tex_state::scaled::Scaled::UNITY
    );
    assert!(stores.box_reg(9).is_none());
    assert!(stores.box_reg(10).is_none());
}

#[test]
fn named_parameters_match_reference_as_internal_dimensions() {
    let parameters = reference_fixture("internal_dimension_params");
    assert!(
        parameters.contains("D:11.0pt,7.0pt"),
        "reference internal-dimension output changed:\n{}",
        parameters
    );

    let stores = run_umber_exec(
        r"\hsize=11pt\splittopskip=7pt plus 2fil minus 1pt\setbox0=\vbox to\hsize{}\setbox1=\vbox to\splittopskip{}",
    );
    assert_eq!(
        stores
            .box_dimension(0, tex_state::BoxDimension::Height)
            .expect("dimension parameter should size the box")
            .raw(),
        11 * tex_state::scaled::Scaled::UNITY
    );
    assert_eq!(
        stores
            .box_dimension(1, tex_state::BoxDimension::Height)
            .expect("glue parameter width should size the box")
            .raw(),
        7 * tex_state::scaled::Scaled::UNITY
    );
}

#[test]
fn hskip_replays_unexpandable_penalty_after_numeric_recovery() {
    let reference = reference_fixture("hskip_penalty_recovery");
    assert!(
        reference.contains("! Missing number, treated as zero.")
            && reference.contains("! Illegal unit of measure (pt inserted).")
            && reference.contains("R:recovered"),
        "reference hskip recovery changed:\n{reference}"
    );

    let source = include_str!("../../../../tests/corpus/tex_exec/hskip_penalty_recovery.tex");
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(source));
    let checkpoint = stores.snapshot();

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("hskip recovery should leave penalty for execution");
    let first_hash = stores.snapshot().state_hash();
    let box0 = stores.box_reg(0).expect("recovery hbox should be assigned");
    let [tex_state::node::Node::HList(hbox)] = stores.nodes(box0).testing_decoded() else {
        panic!("register 0 should contain the recovery hbox");
    };
    let nodes = stores.nodes(hbox.children).testing_decoded();
    assert!(matches!(
        nodes,
        [tex_state::node::Node::Glue { spec, .. }, tex_state::node::Node::Penalty(10_000)]
            if stores.glue(*spec).width.raw() == 0
    ));

    stores.rollback(&checkpoint);
    let mut replay = InputStack::new(MemoryInput::new(source));
    Executor::new()
        .run(&mut replay, &mut stores)
        .expect("replayed hskip recovery should succeed");
    assert_eq!(stores.snapshot().state_hash(), first_hash);
}

#[test]
fn vertical_mode_hskip_runs_everypar_before_scanning_glue() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let source = r"\everypar{\def\skipamount{2.5in}}\hskip\skipamount\dimen0=\lastskip\par\end";
    let mut input = InputStack::new(MemoryInput::new(source));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("vertical-mode hskip should replay after everypar");

    assert_eq!(stores.dimen(0).raw(), 11_840_716);
}

#[test]
fn insert_group_delimiters_match_reference_micro_suite() {
    let insertion = reference_fixture("insert_brace_aliases");
    assert!(
        insertion.contains("I:1,3"),
        "reference insertion grouping changed:\n{}",
        insertion
    );

    let stores = run_umber_exec(
        r"\let\bgroup={\let\egroup=}\count0=1\splittopskip=1pt\insert7\bgroup\count0=2\global\count1=3\splittopskip=9pt\hrule height4pt\egroup",
    );
    assert_eq!(stores.count(0), 1);
    assert_eq!(stores.count(1), 3);
    assert_eq!(
        stores
            .glue(stores.glue_param(tex_state::env::banks::GlueParam::SPLIT_TOP_SKIP))
            .width,
        tex_state::scaled::Scaled::from_raw(tex_state::scaled::Scaled::UNITY)
    );
    let insertion_split_top_skip = stores
        .current_page_nodes()
        .iter()
        .find_map(|node| match node {
            tex_state::node::Node::Ins {
                class: 7,
                split_top_skip,
                ..
            } => Some(stores.glue(*split_top_skip).width),
            _ => None,
        })
        .expect("insertion node");
    assert_eq!(
        insertion_split_top_skip,
        tex_state::scaled::Scaled::from_raw(9 * tex_state::scaled::Scaled::UNITY)
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
