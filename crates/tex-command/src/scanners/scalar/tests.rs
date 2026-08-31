use tex_state::glue::{GlueSpec, Order};
use tex_state::meaning::Meaning;
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, Token};

use crate::{
    CommandHostCapabilities, CommandState, ScalarScanFrame, ScalarScanStatus,
    processor::DeliveryStatus,
};

#[test]
fn keyword_replay_keeps_scalar_continuations_compact() {
    let prefix = std::mem::size_of::<super::MatchedKeywordPrefix<()>>();
    let pending = std::mem::size_of::<super::PendingScalarFrame<()>>();
    assert_eq!(prefix, 120);
    assert_eq!(pending, 208);
}

#[test]
fn scalar_call_frame_separates_compact_status_value_and_error() {
    assert_eq!(std::mem::size_of::<super::ScalarCallStatus>(), 1);
    assert!(
        std::mem::size_of::<super::ScalarCallFrame<super::ScannedScalar<i32>>>()
            <= std::mem::size_of::<Option<crate::CommandError>>()
                + std::mem::size_of::<Option<super::ScannedScalar<i32>>>()
                + std::mem::align_of::<crate::CommandError>()
    );

    let mut call = super::ScalarCallFrame::default();
    call.put_complete(super::ScannedScalar {
        value: 17,
        recovery: super::ScalarRecovery::None,
        provenance: super::ScalarProvenance {
            primary: tex_state::token::OriginId::UNKNOWN,
        },
    });
    assert!(call.error.is_none());
    assert_eq!(call.take_complete().value, 17);
}

fn other(ch: char) -> Token {
    Token::Char {
        ch,
        cat: if ch == ' ' {
            Catcode::Space
        } else {
            Catcode::Other
        },
    }
}

fn letter(ch: char) -> Token {
    Token::Char {
        ch,
        cat: Catcode::Letter,
    }
}

#[test]
fn integer_scanner_preserves_signs_and_backs_up_the_nonspace_terminator() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [other('-'), other('4'), other('2'), other('X')],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let mut scalar = ScalarScanFrame::default();
        assert_eq!(
            processor.scan_integer_into(&mut scalar),
            ScalarScanStatus::Complete
        );
        let integer = scalar.take_integer();
        assert_eq!(integer.value, -42);
        let mut terminator = None;
        assert_eq!(
            processor
                .get_x_token_into(&mut terminator)
                .expect("terminator delivery"),
            DeliveryStatus::Command
        );
        assert_eq!(
            terminator.expect("terminator").meaning(),
            Meaning::CharToken {
                ch: 'X',
                cat: Catcode::Other,
            }
        );
    });
}

#[test]
fn optional_equals_consumes_spaces_but_leaves_the_following_operand() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [other(' '), other('='), other('7')]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let mut scalar = ScalarScanFrame::default();
        assert_eq!(
            processor.scan_optional_equals_into(&mut scalar),
            ScalarScanStatus::Complete
        );
        assert!(scalar.take_boolean().value);
        assert_eq!(
            processor.scan_integer_into(&mut scalar),
            ScalarScanStatus::Complete
        );
        let integer = scalar.take_integer();
        assert_eq!(integer.value, 7);
    });
}

#[test]
fn failed_keyword_replays_the_matched_prefix_before_the_offender() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [letter('e'), letter('x'), other('!')]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        assert!(!processor.scan_keyword("em").expect("keyword").value);
        let replayed = (0..3)
            .map(|_| {
                let mut replayed = None;
                assert_eq!(
                    processor
                        .get_x_token_into(&mut replayed)
                        .expect("replayed delivery"),
                    DeliveryStatus::Command
                );
                replayed
                    .expect("replayed token")
                    .spelling()
                    .semantic_token()
            })
            .collect::<Vec<_>>();
        assert_eq!(replayed, [letter('e'), letter('x'), other('!')]);
    });
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_keyword_success_path_allocates_zero_heap() {
    crate::test_harness::with_universe(|universe| {
        const SCANS: usize = 4_097;
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            (0..SCANS).flat_map(|_| "dimension".chars().map(letter)),
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let mut scalar = ScalarScanFrame::default();
        assert_eq!(
            processor.scan_keyword_into("dimension", &mut scalar),
            ScalarScanStatus::Complete
        );
        let warm = scalar.take_boolean();
        assert!(warm.value);

        let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
        let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        {
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            for _ in 1..SCANS {
                assert_eq!(
                    processor.scan_keyword_into("dimension", &mut scalar),
                    ScalarScanStatus::Complete
                );
                let scanned = scalar.take_boolean();
                assert!(scanned.value);
            }
        }
        let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        assert_eq!(after.calls - before.calls, 0);
        assert_eq!(after.requested_bytes - before.requested_bytes, 0);
    });
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_keyword_failed_prefix_path_allocates_zero_heap() {
    crate::test_harness::with_universe(|universe| {
        const SCANS: usize = 4_097;
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            (0..SCANS).flat_map(|_| "dimensiox".chars().map(letter)),
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let mut scalar = ScalarScanFrame::default();
        let run = |processor: &mut crate::CommandProcessor<'_, '_, _>,
                   scalar: &mut ScalarScanFrame| {
            assert_eq!(
                processor.scan_keyword_into("dimension", scalar),
                ScalarScanStatus::Complete
            );
            let scanned = scalar.take_boolean();
            assert!(!scanned.value);
            for _ in 0..9 {
                let mut replayed = None;
                assert_eq!(
                    processor
                        .get_x_token_into(&mut replayed)
                        .expect("replayed delivery"),
                    DeliveryStatus::Command
                );
                replayed.expect("replayed token");
            }
        };
        run(&mut processor, &mut scalar);

        let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
        let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        {
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            for _ in 1..SCANS {
                run(&mut processor, &mut scalar);
            }
        }
        let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        assert_eq!(after.calls - before.calls, 0);
        assert_eq!(after.requested_bytes - before.requested_bytes, 0);
    });
}

#[test]
fn dimension_scanner_preserves_fractional_points_and_following_input() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [
                other('1'),
                other('.'),
                other('5'),
                letter('p'),
                letter('t'),
                other('X'),
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        let mut scalar = ScalarScanFrame::default();
        assert_eq!(
            processor.scan_dimension_into(&mut scalar),
            ScalarScanStatus::Complete
        );
        assert_eq!(
            scalar.take_dimension().value,
            Scaled::from_raw(Scaled::UNITY + Scaled::UNITY / 2)
        );
        let mut terminator = None;
        assert_eq!(
            processor
                .get_x_token_into(&mut terminator)
                .expect("terminator"),
            DeliveryStatus::Command
        );
        assert_eq!(
            terminator
                .expect("terminator token")
                .spelling()
                .semantic_token(),
            other('X')
        );
    });
}

#[test]
fn glue_scanner_preserves_width_stretch_shrink_and_orders() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            "1pt plus 2fil minus 3pt!".chars().map(|ch| {
                if ch.is_ascii_alphabetic() {
                    letter(ch)
                } else {
                    other(ch)
                }
            }),
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        let mut scalar = ScalarScanFrame::default();
        assert_eq!(
            processor.scan_glue_into(false, &mut scalar),
            ScalarScanStatus::Complete
        );
        assert_eq!(
            scalar.take_glue().value,
            GlueSpec {
                width: Scaled::from_raw(Scaled::UNITY),
                stretch: Scaled::from_raw(2 * Scaled::UNITY),
                stretch_order: Order::Fil,
                shrink: Scaled::from_raw(3 * Scaled::UNITY),
                shrink_order: Order::Normal,
            }
        );
        let mut terminator = None;
        assert_eq!(
            processor
                .get_x_token_into(&mut terminator)
                .expect("terminator"),
            DeliveryStatus::Command
        );
        assert_eq!(
            terminator
                .expect("terminator token")
                .spelling()
                .semantic_token(),
            other('!')
        );
    });
}
