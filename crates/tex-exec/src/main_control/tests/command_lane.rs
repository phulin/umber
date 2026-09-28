//! Command-lane equivalence: each job runs once in unobserved production
//! episodes, where the lane settles its commands in place, and once observed,
//! where the lane is ineligible and the admitted run dispatches every
//! command. Both runs must print the same terminal text, including every
//! report that ends the lane.

use super::*;

/// Drives production-sized episodes, whose operation limit admits the lane.
fn run_episodes_to_end<G>(control: &mut MainControl<G>, stores: &mut Universe<G>) {
    for _ in 0..TEST_STEP_LIMIT {
        match control
            .advance_episode(stores)
            .unwrap_or_else(|error| panic!("program executes: {error:?}"))
        {
            StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput) => return,
            StepResult::Progress(MainControlStep::Continue) => {}
            StepResult::Suspended(need) => panic!("unexpected resource suspension: {need:?}"),
        }
    }
    panic!("lane test job exceeded the bounded {TEST_STEP_LIMIT}-step driver");
}

/// Runs `source` unobserved and observed and returns both terminal texts.
fn lane_and_generic_terminals(source: &[u8], etex: bool) -> (String, String) {
    let run = |observed: bool| {
        let mut terminal = String::new();
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = if etex {
                etex_initex(stores)
            } else {
                MainControl::tex82_initex(stores)
            };
            register_source(&mut control, source);
            if observed {
                run_to_end_observed(&mut control, stores, &mut ObservationRecorder::default());
            } else {
                run_episodes_to_end(&mut control, stores);
            }
            terminal = terminal_text(stores);
        });
        terminal
    };
    (run(false), run(true))
}

/// Runs `source` unobserved and observed until it fails, and returns both
/// errors.
fn lane_and_generic_error(source: &[u8]) -> (String, String) {
    let run = |observed: bool| {
        let mut rendered = String::new();
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            register_source(&mut control, source);
            let mut observer = ObservationRecorder::default();
            for _ in 0..TEST_STEP_LIMIT {
                let step = if observed {
                    control.advance_with_observer(stores, &mut observer)
                } else {
                    control.advance_episode(stores)
                };
                match step {
                    Err(error) => {
                        rendered = format!("{error:?}");
                        return;
                    }
                    Ok(StepResult::Progress(MainControlStep::Continue)) => {}
                    Ok(step) => panic!("job ended without failing: {step:?}"),
                }
            }
            panic!("failing job exceeded the bounded {TEST_STEP_LIMIT}-step driver");
        });
        rendered
    };
    (run(false), run(true))
}

fn assert_lane_matches_generic(source: &[u8], etex: bool, expected: &[&str]) {
    let (lane, generic) = lane_and_generic_terminals(source, etex);
    assert_eq!(lane, generic, "lane and generic terminals differ");
    for text in expected {
        assert!(lane.contains(text), "missing {text:?} in {lane}");
    }
}

#[test]
fn lane_assignments_fire_afterassignment_through_the_same_processor() {
    assert_lane_matches_generic(
        br"\def\x{\message{X}}\afterassignment\x\let\a\relax\message{A:\meaning\a}\afterassignment\x\def\b{b}\message{B:\meaning\b}\afterassignment\x\catcode`\Z=11 \message{C:\the\catcode`\Z}\end",
        false,
        &["X A:\\relax X B:macro:->b X C:11"],
    );
}

#[test]
fn lane_groups_restore_locals_and_replay_aftergroup_tokens() {
    assert_lane_matches_generic(
        br"\def\y{\message{Y:\meaning\a}}\let\a\undefined{\let\a\relax\aftergroup\y}\begingroup\def\a{z}\aftergroup\y\endgroup\message{Z:\meaning\a}\end",
        false,
        &["Y:undefined Y:undefined Z:undefined"],
    );
}

#[test]
fn lane_category_codes_retokenize_following_source() {
    assert_lane_matches_generic(
        br"\catcode`\Z=13 \def Z{\message{active}}\let\q\relax Z\catcode`\Z=11 \message{\meaning Z}\end",
        false,
        &["active", "the letter Z"],
    );
}

#[test]
fn lane_hands_invalid_category_codes_to_the_admitted_run() {
    assert_lane_matches_generic(
        br"\let\a\relax\catcode`\Z=16 \let\b\relax\message{\the\catcode`\Z}\end",
        false,
        &["Invalid code (16)", "\n0"],
    );
}

#[test]
fn lane_assignment_traces_end_the_lane_in_order() {
    assert_lane_matches_generic(
        br"\tracingonline=1 \let\a\relax\def\b{}\tracingassigns=1 \tracingrestores=1 \let\a\relax\def\b{c}{\let\c\relax}\tracingassigns=0 \let\d\relax\end",
        true,
        &["{changing \\b =macro:->}", "{into \\b =macro:->c}", "{restoring \\c"],
    );
}

#[test]
fn lane_group_end_restoring_command_tracing_leaves_the_lane() {
    assert_lane_matches_generic(
        br"\tracingonline=1 \tracingcommands=2 {\tracingcommands=0 \let\a\relax}\relax\let\b\relax\tracingcommands=0 \end",
        false,
        &["{\\relax}", "{\\let}"],
    );
}

#[test]
fn lane_group_traces_and_cross_file_warnings_match_the_admitted_run() {
    assert_lane_matches_generic(
        br"\tracingonline=1 \tracinggroups=1 \tracingnesting=2 \let\a\relax{\begingroup\let\b\relax\endgroup}\let\c\relax\end",
        true,
        &["{entering simple group (level 1)", "{leaving semi simple group (level 2)"],
    );
}

#[test]
fn lane_futurelet_sees_the_following_token() {
    assert_lane_matches_generic(
        br"\def\p{\message{\meaning\n}}\futurelet\n\p\relax\futurelet\n\p{}\end",
        false,
        &["\\relax", "begin-group character {"],
    );
}

#[test]
fn lane_units_discard_through_a_resource_suspension() {
    // Every command the lane settles, including its group transitions,
    // settles its own unit, so `\input` suspends and discards only its own.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"{\count1=1 \let\a\relax}\begingroup\def\b{x}\catcode`\!=11 \endgroup\let\c\relax\input child\end",
        );
        let step = control.advance_episode(stores).expect("batch suspends");
        assert!(
            matches!(
                step,
                StepResult::Suspended(ResourceNeed::Input { ref name, .. }) if name == "child.tex"
            ),
            "unexpected batch step: {step:?}"
        );
        assert_eq!(control.advance_telemetry().rollbacks, 1);
    });
}

#[test]
fn lane_group_closes_update_the_hand_off_dispatch_group() {
    // `\count1=1` opens an admitted iteration inside the simple group. The
    // lane closes that group, and the admitted run must dispatch the
    // following `}` against the box group now innermost.
    assert_lane_matches_generic(
        br"\setbox0\hbox{\kern1pt{\count1=1 \let\a\relax}}\message{W:\the\wd0}\end",
        false,
        &["W:1.0pt"],
    );
}

#[test]
fn lane_folds_global_into_its_assignments() {
    assert_lane_matches_generic(
        br"{\global\let\a\relax\global \relax\global\def\b{b}\global\catcode`\Z=11 \global\global\futurelet\c\relax}\message{A:\meaning\a B:\meaning\b C:\meaning\c Z:\the\catcode`\Z}\end",
        false,
        &["A:\\relaxB:macro:->bC:end-group character }Z:11"],
    );
}

#[test]
fn lane_global_respects_negative_globaldefs() {
    assert_lane_matches_generic(
        br"\globaldefs=-1 {\global\let\a\relax\global\def\b{b}}\message{A:\meaning\a B:\meaning\b}\end",
        false,
        &["A:undefinedB:undefined"],
    );
}

#[test]
fn lane_hands_prefixed_commands_it_does_not_own_to_the_admitted_run() {
    // `\count` and `\advance` dispatch inline with the consumed prefix;
    // `\long` makes the command a prefix barrier, which continues §1211 from
    // the resident command.
    assert_lane_matches_generic(
        br"{\let\q\relax\global\count1=5 \global\advance\count1 by 2 \global\long\def\b#1{#1}}\message{C:\the\count1 B:\meaning\b}\end",
        false,
        &["C:7B:\\long macro:#1->#1"],
    );
}

#[test]
fn lane_reports_prefixes_on_non_prefixed_commands() {
    assert_lane_matches_generic(
        br"\let\q\relax\global\relax\message{after}\global{\let\a\relax}\global\hbox{}\end",
        false,
        &[
            "You can't use a prefix with `\\message'",
            "You can't use a prefix with `begin-group character {'",
        ],
    );
}

#[test]
fn lane_reports_a_prefix_at_the_end_of_input() {
    let (lane, generic) = lane_and_generic_error(br"\let\q\relax\global");
    assert_eq!(lane, generic);
}

#[test]
fn lane_settles_register_and_parameter_assignments() {
    assert_lane_matches_generic(
        br"\countdef\c=3 \dimendef\d=4 {\count1=5 \c=-7\relax\dimen2=1.5pt \d=2pt\global\count5=9 \global\dimen6=3pt \tolerance=321 \global\hsize=10pt}\message{A:\the\count1,\the\c,\the\dimen2,\the\d,\the\count5,\the\dimen6,\the\tolerance,\the\hsize}\end",
        false,
        &["A:0,0,0.0pt,0.0pt,9,3.0pt,10000,10.0pt"],
    );
}

#[test]
fn lane_settles_register_arithmetic() {
    assert_lane_matches_generic(
        br"\count1=5 \dimen1=2pt \skip1=1pt plus 1fil {\advance\count1 by 3 \multiply\count1 2 \global\divide\count1 by 4 \advance\dimen1 1pt \global\multiply\dimen1 3 \advance\skip1 by 2pt \global\advance\tolerance -5 }\message{A:\the\count1,\the\dimen1,\the\skip1,\the\tolerance}\end",
        false,
        &["A:4,9.0pt,1.0pt plus 1.0fil,9995"],
    );
}

#[test]
fn lane_hands_arithmetic_reports_to_the_admitted_run() {
    assert_lane_matches_generic(
        br"\count1=2 \multiply\count1 by 2147483647 \let\a\relax\advance\a by 1 \divide\count1 by 0 \count2=x \message{A:\the\count1,\the\count2}\end",
        false,
        &["Arithmetic overflow", "You can't use `\\relax' after \\advance", "Missing number", "A:2,0"],
    );
}

#[test]
fn lane_scalar_assignments_fire_afterassignment_and_traces() {
    assert_lane_matches_generic(
        br"\def\x{\message{X}}\afterassignment\x\count1=4 \afterassignment\x\advance\count1 by 1 \message{A:\the\count1}\tracingonline=1 \tracingassigns=1 \count1=6 \dimen0=1pt \tracingassigns=0 \end",
        true,
        &["X X A:5", "{changing \\count1=5}", "{into \\count1=6}", "{into \\dimen0=1.0pt}"],
    );
}
