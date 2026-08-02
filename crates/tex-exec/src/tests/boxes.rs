use super::*;
use tex_command::{CommandProfile, RegisteredSourceKind, SourceRegistration};

fn run(source: &str) -> Universe {
    super::core::run_canonical_tex82(&format!(r"{source}\end"))
}

fn run_loaded_format(source: &str, stores: &mut Universe) {
    let mut control = CanonicalMainControl::with_profile(CommandProfile::TEX82);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            format!(r"{source}\end").into_bytes(),
        ))
        .expect("register format-loaded box-hook source");
    for _ in 0..1024 {
        if control.step(stores).expect("format-loaded box-hook step") == MainControlStep::End {
            return;
        }
    }
    panic!("format-loaded box-hook source did not terminate");
}

#[test]
fn pending_character_box_commit_preserves_nonempty_geometry() {
    let stores = run(r"\setbox0=\hbox{\vrule height7pt A}");
    assert_eq!(
        stores
            .box_dimension(0, tex_state::BoxDimension::Height)
            .expect("committed box has a height")
            .raw(),
        7 * tex_state::scaled::Scaled::UNITY
    );
}

#[test]
fn every_box_hooks_match_tex82_reference_observation() {
    let reference = test_support::read_fixture("tex_exec", "every_box_hooks", "ref");
    assert!(
        reference.contains("H:3,10.0pt;V:2"),
        "reference every-box timing changed:\n{reference}"
    );
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
    run_loaded_format(r"\setbox0=\hbox{}\setbox1=\vbox{}", &mut stores);

    assert_eq!(stores.count(4), 1);
    assert_eq!(stores.count(5), 1);
}

#[test]
fn every_hbox_diagnostic_reports_its_replay_context() {
    let stores = run(r"\nonstopmode\everyhbox{\errmessage{hook failure}}\hbox{}");
    let terminal = support::terminal_effect_text(&stores);
    assert!(
        terminal.contains("hook failure"),
        "hook diagnostic missing: {terminal}"
    );
    assert!(
        terminal.contains("<everyhbox>"),
        "hook replay context missing: {terminal}"
    );
}

#[test]
fn every_box_hook_execution_converges_after_rollback() {
    let mut stores = run(r"\everyhbox{\global\advance\count6 by1}
           \everyvbox{\global\advance\count7 by1}");
    let checkpoint = stores.snapshot();
    let source = r"\setbox0=\hbox{\vbox{}}";

    run_loaded_format(source, &mut stores);
    let expected = stores.snapshot().state_hash();
    assert_eq!((stores.count(6), stores.count(7)), (1, 1));

    stores.rollback(&checkpoint);
    run_loaded_format(source, &mut stores);
    assert_eq!(stores.snapshot().state_hash(), expected);
}
