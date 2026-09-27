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
    // `\count1=1` settles a unit inside the brace group. The lane then
    // closes that already-open group, which it must settle in place, and
    // merges a balanced semi-simple group into the unit that `\input`
    // suspends and discards.
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
