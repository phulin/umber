use super::*;

#[test]
fn globaldefs_forces_and_suppresses_global_assignments() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    stores.enter_group();
    let mut input = InputStack::new(MemoryInput::new(
        "\\globaldefs=1 \\def\\a{A}\\globaldefs=-1 \\gdef\\b{B}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("globaldefs assignments execute");
    let a = stores.symbol("a").expect("a");
    let b = stores.symbol("b").expect("b");
    assert!(matches!(stores.meaning(a), Meaning::Macro { .. }));
    assert!(matches!(stores.meaning(b), Meaning::Macro { .. }));

    let _ = stores.leave_group();
    assert!(matches!(stores.meaning(a), Meaning::Macro { .. }));
    assert_eq!(stores.meaning(b), Meaning::Undefined);
}

#[test]
fn brace_and_begingroup_groups_restore_local_assignments() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "{\\count0=1\\global\\count1=2}\\begingroup\\count2=3\\endgroup",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("grouping primitives execute");

    assert_eq!(stores.count(0), 0);
    assert_eq!(stores.count(1), 2);
    assert_eq!(stores.count(2), 0);
}

#[test]
fn aftergroup_replays_tokens_fifo_on_group_exit() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\def\\A{\\count0=1}\\def\\B{\\count0=2}{\\aftergroup\\A\\aftergroup\\B}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("aftergroup executes");

    assert_eq!(stores.count(0), 2);
}

#[test]
fn afterassignment_fires_before_aftergroup_tokens() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\def\\A{\\global\\count0=1}\\def\\B{\\global\\count0=2}{\\aftergroup\\B\\afterassignment\\A\\count1=7}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("afterassignment and aftergroup execute");

    assert_eq!(stores.count(0), 2);
    assert_eq!(stores.count(1), 0);
}

#[test]
fn afterassignment_slot_is_single_token_and_overwrites_previous() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\def\\A{\\count0=1}\\def\\B{\\count0=2}\\afterassignment\\A\\afterassignment\\B\\count1=7",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("afterassignment executes");

    assert_eq!(stores.count(0), 2);
    assert_eq!(stores.count(1), 7);
}

#[test]
fn group_mismatch_errors_use_tex_primary_text() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("}"));

    let err = Executor::new()
        .run(&mut input, &mut stores)
        .expect_err("extra right brace is an error");
    assert_eq!(err.to_string(), "Too many }'s.");

    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\begingroup}"));
    let err = Executor::new()
        .run(&mut input, &mut stores)
        .expect_err("right brace cannot close begingroup");
    assert_eq!(err.to_string(), "Extra }, or forgotten \\endgroup.");

    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\endgroup"));
    let err = Executor::new()
        .run(&mut input, &mut stores)
        .expect_err("extra endgroup is an error");
    assert_eq!(err.to_string(), "Extra \\endgroup.");

    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("{\\endgroup"));
    let err = Executor::new()
        .run(&mut input, &mut stores)
        .expect_err("endgroup cannot close brace group");
    assert_eq!(err.to_string(), "\\endgroup ended a group started by {");
}
