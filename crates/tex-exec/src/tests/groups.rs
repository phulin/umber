use super::*;

#[test]
fn globaldefs_forces_and_suppresses_global_assignments() {
    let stores = run_grouping("{\\globaldefs=1 \\def\\a{A}\\globaldefs=-1 \\gdef\\b{B}}");
    let a = stores.symbol("a").expect("a");
    let b = stores.symbol("b").expect("b");
    assert!(matches!(stores.meaning(a), Meaning::Macro { .. }));
    assert_eq!(stores.meaning(b), Meaning::Undefined);
}

#[test]
fn brace_and_begingroup_groups_restore_local_assignments() {
    let stores = run_grouping("{\\count0=1\\global\\count1=2}\\begingroup\\count2=3\\endgroup");

    assert_eq!(stores.count(0), 0);
    assert_eq!(stores.count(1), 2);
    assert_eq!(stores.count(2), 0);
}

#[test]
fn box_builder_groups_restore_local_assignments() {
    let stores = run_grouping(
        "\\count0=1 \\dimen0=1pt \
         \\setbox0=\\hbox{{\\count0=9}\\count0=2\\dimen0=2pt\\global\\count1=3}\
         \\setbox1=\\vbox{\\count0=4\\dimen0=4pt\\global\\dimen1=5pt}\
         \\setbox2=\\vtop{\\count0=6\\dimen0=6pt\\global\\count2=7}",
    );

    assert_eq!(stores.count(0), 1);
    assert_eq!(stores.dimen(0).raw(), tex_state::scaled::Scaled::UNITY);
    assert_eq!(stores.count(1), 3);
    assert_eq!(stores.dimen(1).raw(), 5 * tex_state::scaled::Scaled::UNITY);
    assert_eq!(stores.count(2), 7);
}

#[test]
fn brace_aliases_delimit_box_builder_groups_by_meaning() {
    let stores = run_grouping(
        "\\let\\bgroup={\\let\\egroup=}\\count0=1 \\setbox0=\\vbox\\bgroup\\count0=2\\bgroup\\count0=3\\egroup\\egroup",
    );

    assert!(stores.box_reg(0).is_some());
    assert_eq!(stores.count(0), 1);
}

#[test]
fn aftergroup_replays_tokens_fifo_on_group_exit() {
    let stores =
        run_grouping("\\def\\A{\\count0=1}\\def\\B{\\count0=2}{\\aftergroup\\A\\aftergroup\\B}");

    assert_eq!(stores.count(0), 2);
}

#[test]
fn afterassignment_fires_before_aftergroup_tokens() {
    let stores = run_grouping(
        "\\def\\A{\\global\\count0=1}\\def\\B{\\global\\count0=2}{\\aftergroup\\B\\afterassignment\\A\\count1=7}",
    );

    assert_eq!(stores.count(0), 2);
    assert_eq!(stores.count(1), 0);
}

#[test]
fn afterassignment_slot_is_single_token_and_overwrites_previous() {
    let stores = run_grouping(
        "\\def\\A{\\count0=1}\\def\\B{\\count0=2}\\afterassignment\\A\\afterassignment\\B\\count1=7",
    );

    assert_eq!(stores.count(0), 2);
    assert_eq!(stores.count(1), 7);
}

#[test]
fn group_mismatch_errors_use_tex_primary_text() {
    let stores = run_grouping("}");
    assert!(support::terminal_effect_text(&stores).contains("Too many }'s"));

    let stores = run_grouping("\\begingroup}\\endgroup");
    assert!(support::terminal_effect_text(&stores).contains("Extra }, or forgotten \\endgroup."));

    let stores = run_grouping("\\endgroup");
    assert!(support::terminal_effect_text(&stores).contains("Extra \\endgroup"));

    let stores = run_grouping("{\\endgroup}");
    let terminal = support::terminal_effect_text(&stores);
    assert!(
        terminal.contains("Missing } inserted.") && terminal.contains("Extra \\endgroup."),
        "canonical mismatch diagnostic changed: {terminal}"
    );
}

fn run_grouping(source: &str) -> Universe {
    super::core::run_canonical_tex82(&format!("{source}\\end"))
}
