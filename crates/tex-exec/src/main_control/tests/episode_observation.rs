//! Direct command episodes, observation publication, and dispatch boundaries.

use super::*;

#[test]
fn save_stack_high_water_samples_each_checked_push_without_hardcoded_job_totals() {
    // TeX82 §§273/275--276 and §645: the high-water mark is the depth
    // immediately before a checked push, not the completed live depth. These
    // cases separate one/two-word restores, global no-save assignment,
    // command-owned aftergroup ordering, and executor-owned box specs.
    for (source, expected) in [
        (br"{}\end".as_slice(), 0),
        (br"{\global\count0=1}\end", 0),
        (br"{\count0=1}\end", 1),
        (br"{\def\fresh{}\count0=1}\end", 2),
        (br"{\count0=1\count1=1}\end", 3),
        (br"{\count0=1\aftergroup\relax\count1=1}\end", 4),
        (br"\setbox0=\hbox{}\end", 3),
    ] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            register_source(&mut control, source);
            run_to_end(&mut control, stores);
            assert_eq!(control.max_save_stack, expected, "source: {source:?}");
        });
    }
}
#[test]
fn ordinary_font_selection_keeps_its_expanded_delivery() {
    // Negative control: only §1270's already-settled handoff suppresses a
    // duplicate observation. An ordinary §1030 `big_switch` font command is
    // still one raw plus one expanded delivery.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\nullfont\end");
        let mut observations = ObservationRecorder::default();
        run_to_end_observed(&mut control, stores, &mut observations);

        let deliveries: Vec<_> = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Command(record)
                    if record.spelling == ObservedToken::ControlSequence("nullfont".into())
                        && record.command == "set_font" =>
                {
                    Some(record.boundary)
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            deliveries,
            [
                CommandDeliveryBoundary::Raw,
                CommandDeliveryBoundary::Expanded,
            ]
        );
    });
}
#[test]
fn etex_unexpanded_input_survives_the_first_main_control_operation() {
    // e-TeX change file §27.465 implements `\unexpanded` as `the_toks` plus
    // `ins_list`. The inserted list remains input after its first
    // unexpandable command reaches main control, so later tokens must not
    // borrow the attempt arena retired with that command operation.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"\count255=0 \unexpanded{\relax\global\advance\count255 by1}\end",
        );

        run_to_end_observed(&mut control, stores, &mut ObservationRecorder::default());

        assert_eq!(
            stores.count(255).expect("count register"),
            1,
            "terminal: {}",
            terminal_text(stores)
        );
    });
}
#[test]
fn production_batch_commits_ordinary_prefix_before_terminal_transaction() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\count0=11 \count1=22 \end");

        assert_eq!(
            control.advance_episode(stores).expect("batch completes"),
            StepResult::Progress(ReplayStep::End)
        );
        assert_eq!(stores.count(0).expect("count register"), 11);
        assert_eq!(stores.count(1).expect("count register"), 22);
        assert_eq!(control.advance_telemetry().attempts, 2);
        assert_eq!(control.advance_telemetry().commits, 2);
    });
}
#[cfg(feature = "profiling")]
#[test]
fn one_and_4096_ordinary_episodes_construct_zero_operation_frames() {
    let (one_allocations, one_transitions, one_copies, one_overlapping_moves) =
        ordinary_command_episode_evidence(1);
    let (many_allocations, many_transitions, many_copies, many_overlapping_moves) =
        ordinary_command_episode_evidence(4_096);

    assert_eq!(one_allocations.calls, 0);
    assert_eq!(one_allocations.requested_bytes, 0);
    assert_eq!(many_allocations.calls, 0);
    assert_eq!(many_allocations.requested_bytes, 0);
    assert_eq!(one_transitions, 4);
    assert_eq!(many_transitions, 16_384);
    assert_eq!(one_copies, 0);
    assert_eq!(many_copies, 0);
    assert_eq!(one_overlapping_moves, 0);
    assert_eq!(many_overlapping_moves, 0);
}
#[cfg(feature = "profiling")]
#[test]
fn one_and_4096_cold_scan_cycles_are_allocation_free_and_stationary() {
    let (one_allocations, one_transitions, one_address_changes, one_moves, one_checksum) =
        resident_cold_scan_evidence(1);
    let (many_allocations, many_transitions, many_address_changes, many_moves, many_checksum) =
        resident_cold_scan_evidence(4_096);

    assert_eq!(one_allocations.calls, 0);
    assert_eq!(one_allocations.requested_bytes, 0);
    assert_eq!(many_allocations.calls, 0);
    assert_eq!(many_allocations.requested_bytes, 0);
    assert_eq!(one_transitions, 2);
    assert_eq!(many_transitions, 8_192);
    assert_eq!(one_address_changes, 0);
    assert_eq!(many_address_changes, 0);
    assert_eq!(one_moves, 0);
    assert_eq!(many_moves, 0);
    assert_ne!(one_checksum, many_checksum);
}
#[test]
fn production_batch_returns_after_a_world_effect() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\message{effect}\count0=11 \end");

        assert_eq!(
            control.advance_episode(stores).expect("effect commits"),
            StepResult::Progress(ReplayStep::Continue)
        );
        assert_eq!(
            stores.count(0).expect("count register"),
            0,
            "later input remains for the next host step"
        );
        assert_eq!(control.advance_telemetry().attempts, 1);
    });
}
#[test]
fn main_control_dispatch_matrix_consumes_each_command_once() {
    const MODES: [Mode; 6] = [
        Mode::Vertical,
        Mode::InternalVertical,
        Mode::Horizontal,
        Mode::RestrictedHorizontal,
        Mode::Math,
        Mode::DisplayMath,
    ];

    for mode in MODES {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            if mode != Mode::Vertical {
                control.modes.push(mode).expect("test mode push");
            }
            register_source(&mut control, br"\count0=17\count1=29");

            let mut observations = ObservationRecorder::default();
            assert_eq!(
                control
                    .step_with_observer(stores, &mut observations)
                    .expect("mode-independent assignment dispatches"),
                MainControlStep::Continue,
                "mode {mode:?}"
            );
            assert_eq!(
                stores.count(0).expect("count register"),
                17,
                "mode {mode:?}"
            );
            assert_eq!(stores.count(1).expect("count register"), 0, "mode {mode:?}");
            assert_eq!(control.current_mode(), mode);
            assert_eq!(
                observations
                    .0
                    .iter()
                    .filter(|observation| matches!(observation, CommandObservation::Mutation(_)))
                    .count(),
                1,
                "one main-control mutation committed in mode {mode:?}: {:?}",
                observations.0
            );
            assert!(observations.0.iter().any(|observation| matches!(
                observation,
                CommandObservation::Mutation(mutation)
                    if mutation.key == ObservationValue::Name("count:0".into())
                        && mutation.value == ObservationValue::Integer(17)
            )));

            observations.0.clear();
            assert_eq!(
                control
                    .step_with_observer(stores, &mut observations)
                    .expect("following command remains available"),
                MainControlStep::Continue,
                "mode {mode:?}"
            );
            assert_eq!(
                stores.count(1).expect("count register"),
                29,
                "mode {mode:?}"
            );
            assert_eq!(
                observations
                    .0
                    .iter()
                    .filter(|observation| matches!(observation, CommandObservation::Mutation(_)))
                    .count(),
                1,
                "the following command commits exactly once in mode {mode:?}"
            );
            assert!(observations.0.iter().any(|observation| matches!(
                observation,
                CommandObservation::Mutation(mutation)
                    if mutation.key == ObservationValue::Name("count:1".into())
                        && mutation.value == ObservationValue::Integer(29)
            )));
        });
    }
}
