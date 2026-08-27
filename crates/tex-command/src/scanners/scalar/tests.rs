use tex_state::glue::{GlueSpec, Order};
use tex_state::meaning::Meaning;
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, Token};

use crate::{CommandHostCapabilities, CommandState, processor::DeliveryStatus};

#[test]
fn keyword_replay_keeps_scalar_continuations_compact() {
    let prefix = std::mem::size_of::<super::MatchedKeywordPrefix<()>>();
    let pending = std::mem::size_of::<super::PendingScalarFrame<()>>();
    assert_eq!(prefix, 736);
    assert_eq!(pending, 792);
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
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut diagnostic_effects,
        );
        assert_eq!(processor.scan_integer().expect("integer").value, -42);
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
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut diagnostic_effects,
        );
        assert!(processor.scan_optional_equals().expect("equals").value);
        assert_eq!(processor.scan_integer().expect("operand").value, 7);
    });
}

#[test]
fn failed_keyword_replays_the_matched_prefix_before_the_offender() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [letter('e'), letter('x'), other('!')]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
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
        const SCANS: usize = 257;
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            (0..SCANS).flat_map(|_| "dimension".chars().map(letter)),
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut diagnostic_effects,
        );
        let crate::RetainedScalarScan::Complete(warm) =
            processor.scan_keyword_retained("dimension")
        else {
            panic!("preloaded keyword scan must complete synchronously")
        };
        assert!(warm.value);

        let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
        let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        {
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            for _ in 1..SCANS {
                let crate::RetainedScalarScan::Complete(scanned) =
                    processor.scan_keyword_retained("dimension")
                else {
                    panic!("preloaded keyword scan must complete synchronously")
                };
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
        const SCANS: usize = 257;
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            (0..SCANS).flat_map(|_| "dimensiox".chars().map(letter)),
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut diagnostic_effects,
        );
        let run = |processor: &mut crate::CommandProcessor<'_, '_, _>| {
            let crate::RetainedScalarScan::Complete(scanned) =
                processor.scan_keyword_retained("dimension")
            else {
                panic!("preloaded keyword scan must complete synchronously")
            };
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
        run(&mut processor);

        let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
        let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        {
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            for _ in 1..SCANS {
                run(&mut processor);
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
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut diagnostic_effects,
        );

        assert_eq!(
            processor.scan_dimension().expect("dimension").value,
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
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut diagnostic_effects,
        );

        assert_eq!(
            processor.scan_glue(false).expect("glue").value,
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
