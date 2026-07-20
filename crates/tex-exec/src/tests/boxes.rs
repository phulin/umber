use super::*;
use tex_state::provenance::{InsertedOriginKind, OriginRecord};

fn run(source: &str) -> Universe {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    Executor::new()
        .run(&mut InputStack::new(MemoryInput::new(source)), &mut stores)
        .expect("box hook source executes");
    stores
}

#[test]
fn every_box_hooks_cover_empty_nested_vtop_and_implicit_groups() {
    let stores = run(r"\everyhbox{}\setbox0=\hbox{}
           \everyhbox{\global\advance\count0 by1}
           \everyvbox{\global\advance\count1 by1}
           \let\bgroup={\let\egroup=}
           \setbox0=\hbox\bgroup\hbox{}\egroup
           \setbox1=\vbox\bgroup\vtop{}\vbox{}\egroup");

    assert_eq!(stores.count(0), 2, "both nested hboxes execute the hook");
    assert_eq!(
        stores.count(1),
        3,
        "vbox, vtop, and nested vbox execute the vertical hook"
    );
}

#[test]
fn every_box_hooks_run_after_spec_and_afterassignment_but_before_body() {
    let stores = run(r"\dimen0=10pt
           \def\after{\global\count0=1}
           \everyhbox{\global\count0=2\global\dimen0=20pt}
           \afterassignment\after
           \setbox0=\hbox to\dimen0{\global\count1=\count0}");

    assert_eq!(stores.count(1), 2, "hook follows the afterassignment token");
    assert_eq!(
        stores
            .box_dimension(0, tex_state::BoxDimension::Width)
            .expect("box width")
            .raw(),
        10 * tex_state::scaled::Scaled::UNITY,
        "the pack specification is scanned before the hook"
    );
}

#[test]
fn every_box_hook_assignments_obey_local_and_global_scope() {
    let stores = run(r"\everyhbox{\global\advance\count0 by1}
           {\everyhbox{\global\advance\count1 by1}\setbox0=\hbox{}}
           \setbox0=\hbox{}
           {\global\everyvbox{\global\advance\count2 by1}}
           \setbox1=\vbox{}");

    assert_eq!(stores.count(0), 1, "outer hook is restored after the group");
    assert_eq!(
        stores.count(1),
        1,
        "local replacement executes in its group"
    );
    assert_eq!(
        stores.count(2),
        1,
        "global hook assignment survives its group"
    );
}

#[test]
fn every_box_hooks_survive_format_round_trip() {
    let initex = run(r"\everyhbox{\global\advance\count4 by1}
           \everyvbox{\global\advance\count5 by1}");
    let format = initex.dump_format().expect("box-hook format dumps");
    let mut stores =
        Universe::from_format(tex_state::World::memory(), &format).expect("box-hook format loads");
    tex_expand::register_expandable_primitives(&mut stores);
    register_unexpandable_primitives(&mut stores);

    Executor::new()
        .run(
            &mut InputStack::new(MemoryInput::new(r"\setbox0=\hbox{}\setbox1=\vbox{}")),
            &mut stores,
        )
        .expect("format-loaded hooks execute");

    assert_eq!(stores.count(4), 1);
    assert_eq!(stores.count(5), 1);
}

#[test]
fn every_hbox_replay_has_specific_token_list_provenance() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let error = Executor::new()
        .run(
            &mut InputStack::new(MemoryInput::new(r"\everyhbox{\input}\hbox{}")),
            &mut stores,
        )
        .expect_err("input from everyhbox lacks a file name");
    let origin = error.primary_origin().expect("hook replay origin");
    let OriginRecord::Inserted(inserted) = stores.origin(origin) else {
        panic!("hook token should have inserted provenance");
    };
    assert!(matches!(
        inserted.kind(),
        InsertedOriginKind::TokenListReplay(_)
    ));
    assert_eq!(
        format!("{:?}", inserted.kind()),
        "TokenListReplay(EveryHBox)"
    );
}

#[test]
fn every_box_hook_execution_converges_after_rollback() {
    let mut stores = run(r"\everyhbox{\global\advance\count6 by1}
           \everyvbox{\global\advance\count7 by1}");
    let checkpoint = stores.snapshot();
    let source = r"\setbox0=\hbox{\vbox{}}";

    Executor::new()
        .run(&mut InputStack::new(MemoryInput::new(source)), &mut stores)
        .expect("first hook execution");
    let expected = stores.snapshot().state_hash();
    assert_eq!((stores.count(6), stores.count(7)), (1, 1));

    stores.rollback(&checkpoint);
    Executor::new()
        .run(&mut InputStack::new(MemoryInput::new(source)), &mut stores)
        .expect("replayed hook execution");
    assert_eq!(stores.snapshot().state_hash(), expected);
}
