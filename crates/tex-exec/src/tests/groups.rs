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
fn box_builder_groups_restore_local_assignments() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\count0=1 \\dimen0=1pt \
         \\setbox0=\\hbox{{\\count0=9}\\count0=2\\dimen0=2pt\\global\\count1=3}\
         \\setbox1=\\vbox{\\count0=4\\dimen0=4pt\\global\\dimen1=5pt}\
         \\setbox2=\\vtop{\\count0=6\\dimen0=6pt\\global\\count2=7}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("box builder groups execute");

    assert_eq!(stores.count(0), 1);
    assert_eq!(stores.dimen(0).raw(), tex_state::scaled::Scaled::UNITY);
    assert_eq!(stores.count(1), 3);
    assert_eq!(stores.dimen(1).raw(), 5 * tex_state::scaled::Scaled::UNITY);
    assert_eq!(stores.count(2), 7);
}

#[test]
fn brace_aliases_delimit_box_builder_groups_by_meaning() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\let\\bgroup={\\let\\egroup=}\\count0=1 \\setbox0=\\vbox\\bgroup\\count0=2\\bgroup\\count0=3\\egroup\\egroup",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("brace aliases delimit nested box groups");

    assert!(stores.box_reg(0).is_some());
    assert_eq!(stores.count(0), 1);
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

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("extra right brace is reported and ignored");
    assert!(support::terminal_effect_text(&stores).contains("Too many }'s"));

    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    stores.enter_group_with_kind(tex_state::GroupKind::SemiSimple);
    let mut input = InputStack::new(MemoryInput::new("}"));
    let token = tex_expand::get_x_token(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect("right brace tokenizes")
    .expect("right brace exists");
    let err = crate::dispatch::dispatch_delivered_token(
        &mut ModeNest::new(),
        token,
        &mut input,
        &mut stores,
        &mut crate::ExecutionContext::new("texput"),
    )
    .expect_err("direct dispatch exposes the recoverable mismatch");
    assert_eq!(err.to_string(), "Extra }, or forgotten \\endgroup.");
    assert_ne!(err.primary_origin(), Some(OriginId::UNKNOWN));

    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\endgroup"));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect("extra endgroup is reported and ignored");
    assert!(support::terminal_effect_text(&stores).contains("Extra \\endgroup"));

    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    stores.enter_group_with_kind(tex_state::GroupKind::Simple);
    let mut input = InputStack::new(MemoryInput::new("\\endgroup"));
    let token = tex_expand::get_x_token(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect("endgroup tokenizes")
    .expect("endgroup exists");
    let err = crate::dispatch::dispatch_delivered_token(
        &mut ModeNest::new(),
        token,
        &mut input,
        &mut stores,
        &mut crate::ExecutionContext::new("texput"),
    )
    .expect_err("direct dispatch exposes the recoverable mismatch");
    assert_eq!(err.to_string(), "\\endgroup ended a group started by {");
    assert_ne!(err.primary_origin(), Some(OriginId::UNKNOWN));
}
