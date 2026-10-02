use tex_state::glue::{GlueSpec, Order};
use tex_state::meaning::Meaning;
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, Token};

use crate::{CommandHostCapabilities, CommandState, processor::DeliveryStatus};

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
        let scalar_result = match processor.scan_integer_retained() {
            crate::RetainedScalarScan::Complete(value) => value,
            crate::RetainedScalarScan::Failed(error) => panic!("scalar scan failed: {error:?}"),
        };
        let integer = scalar_result;
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
fn integer_scanner_preserves_the_frozen_conditional_recovery_terminator() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [other('4'), other('2'), Token::frozen_relax(), other('X')],
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
        let scalar_result = match processor.scan_integer_retained() {
            crate::RetainedScalarScan::Complete(value) => value,
            crate::RetainedScalarScan::Failed(error) => panic!("scalar scan failed: {error:?}"),
        };
        assert_eq!(scalar_result.value, 42);
        // TeX82 §§379 and 444: the inserted sentinel is still the next
        // command after scanning the integer; only a spacer is absorbed.
        for expected in [Token::frozen_relax(), other('X')] {
            let mut next = None;
            assert_eq!(
                processor.get_x_token_into(&mut next).expect("next token"),
                DeliveryStatus::Command
            );
            assert_eq!(
                next.expect("terminator delivery initializes destination")
                    .spelling()
                    .semantic_token(),
                expected
            );
        }
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
        let scalar_result = match processor.scan_optional_equals_retained() {
            crate::RetainedScalarScan::Complete(value) => value,
            crate::RetainedScalarScan::Failed(error) => panic!("scalar scan failed: {error:?}"),
        };
        assert!(scalar_result.value);
        let scalar_result = match processor.scan_integer_retained() {
            crate::RetainedScalarScan::Complete(value) => value,
            crate::RetainedScalarScan::Failed(error) => panic!("scalar scan failed: {error:?}"),
        };
        let integer = scalar_result;
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
        let scalar_result = match processor.scan_keyword_retained("dimension") {
            crate::RetainedScalarScan::Complete(value) => value,
            crate::RetainedScalarScan::Failed(error) => panic!("scalar scan failed: {error:?}"),
        };
        let warm = scalar_result;
        assert!(warm.value);

        let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
        let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        {
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            for _ in 1..SCANS {
                let scalar_result = match processor.scan_keyword_retained("dimension") {
                    crate::RetainedScalarScan::Complete(value) => value,
                    crate::RetainedScalarScan::Failed(error) => {
                        panic!("scalar scan failed: {error:?}")
                    }
                };
                let scanned = scalar_result;
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
        let run = |processor: &mut crate::CommandProcessor<'_, '_, _>| {
            let scalar_result = match processor.scan_keyword_retained("dimension") {
                crate::RetainedScalarScan::Complete(value) => value,
                crate::RetainedScalarScan::Failed(error) => panic!("scalar scan failed: {error:?}"),
            };
            let scanned = scalar_result;
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
        let scalar_result = match processor.scan_dimension_retained() {
            crate::RetainedScalarScan::Complete(value) => value,
            crate::RetainedScalarScan::Failed(error) => panic!("scalar scan failed: {error:?}"),
        };
        assert_eq!(
            scalar_result.value,
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
        let scalar_result = match processor.scan_glue_retained(false) {
            crate::RetainedScalarScan::Complete(value) => value,
            crate::RetainedScalarScan::Failed(error) => panic!("scalar scan failed: {error:?}"),
        };
        assert_eq!(
            scalar_result.value,
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

#[test]
fn numeric_heads_and_operands_need_no_unobserved_command_records() {
    struct Observer;
    impl crate::CommandObserver for Observer {
        fn committed(&mut self, _: crate::CommandObservation) {}
    }
    for observed in [false, true] {
        for signs in [0, 4096] {
            for kind in 0..6 {
                crate::test_harness::with_universe(|universe| {
                    let mut input = vec![other('-'); signs];
                    let expected = match kind {
                        0 => {
                            input.extend([other('4'), other('2')]);
                            42
                        }
                        1 => {
                            input.extend([
                                other('1'),
                                other('.'),
                                other('2'),
                                other('5'),
                                letter('p'),
                                letter('t'),
                            ]);
                            81920
                        }
                        2 => {
                            input.extend([
                                other('1'),
                                letter('p'),
                                letter('t'),
                                other(' '),
                                letter('p'),
                                letter('l'),
                                letter('u'),
                                letter('s'),
                                other(' '),
                                other('2'),
                                letter('p'),
                                letter('t'),
                            ]);
                            65536
                        }
                        3 => {
                            let name = universe.intern("native-internal-value").expect("name");
                            universe
                                .assign_meaning(
                                    name,
                                    tex_state::meaning::MeaningWord::from_static(
                                        Meaning::CharGiven('*'),
                                    ),
                                    tex_state::env::AssignmentScope::Global,
                                )
                                .expect("internal meaning");
                            input.push(Token::Cs(name.symbol()));
                            42
                        }
                        4 => {
                            input.extend([
                                other('`'),
                                Token::Char {
                                    ch: '}',
                                    cat: Catcode::EndGroup,
                                },
                            ]);
                            125
                        }
                        _ => {
                            input.extend([
                                other('1'),
                                letter('p'),
                                letter('t'),
                                other(' '),
                                letter('p'),
                                letter('l'),
                                letter('u'),
                                letter('s'),
                                other(' '),
                                other('2'),
                                letter('f'),
                                letter('i'),
                                letter('l'),
                            ]);
                            65536
                        }
                    };
                    input.push(other('X'));
                    let mut command = CommandState::default();
                    crate::test_harness::push(&mut command, input);
                    let mut capabilities = CommandHostCapabilities::default();
                    let mut fuel = crate::CommandFuelLedger::default();
                    let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
                    let mut observer = Observer;
                    let mut context = universe.command_context().expect("context");
                    let processor = crate::test_harness::processor(
                        &mut command,
                        &mut context,
                        &mut capabilities,
                        &mut fuel,
                        &mut effects,
                    );
                    let mut processor = if observed {
                        processor.with_observer(&mut observer)
                    } else {
                        processor
                    };
                    let alignment_before = processor.command.alignment.align_state;
                    let before = crate::command::command_ownership_counters();
                    let value = match kind {
                        1 => processor
                            .scan_dimension_retained()
                            .into_result()
                            .expect("dimension")
                            .value
                            .raw(),
                        2 | 5 => {
                            let glue = processor
                                .scan_glue_retained(false)
                                .into_result()
                                .expect("glue")
                                .value;
                            assert_eq!(glue.stretch.raw(), 131072);
                            assert_eq!(
                                glue.stretch_order,
                                if kind == 5 { Order::Fil } else { Order::Normal }
                            );
                            glue.width.raw()
                        }
                        _ => {
                            processor
                                .scan_integer_retained()
                                .into_result()
                                .expect("integer")
                                .value
                        }
                    };
                    assert_eq!(value, expected);
                    assert_eq!(
                        processor.command.alignment.align_state, alignment_before,
                        "alphabetic brace constants cancel their raw brace accounting"
                    );
                    let after = crate::command::command_ownership_counters();
                    if observed {
                        assert!(after.resolved_writes > before.resolved_writes);
                    } else {
                        assert_eq!(
                            after.resolved_writes, before.resolved_writes,
                            "numeric signs, introducers, internal values, units and backups consume native facts"
                        );
                    }
                    let next = crate::test_harness::expect_expanded_command(&mut processor);
                    assert_eq!(next.spelling().semantic_token(), other('X'));
                    assert_eq!(processor.command.transient.active_expansion_depth, 0);
                });
            }
        }
    }
}

#[test]
fn internal_expansion_and_font_selection_consume_native_operands() {
    crate::test_harness::with_universe(|universe| {
        let integer = universe.intern("native-internal").expect("name");
        let the = universe.intern("the").expect("name");
        let font = universe.intern("native-font").expect("name");
        for (name, meaning) in [
            (integer, Meaning::CharGiven('*')),
            (
                the,
                Meaning::ExpandablePrimitive(tex_state::meaning::ExpandablePrimitive::The),
            ),
            (font, Meaning::Font(tex_state::font::NULL_FONT)),
        ] {
            universe
                .assign_meaning(
                    name,
                    tex_state::meaning::MeaningWord::from_static(meaning),
                    tex_state::env::AssignmentScope::Global,
                )
                .expect("meaning");
        }
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [
                Token::Cs(the.symbol()),
                Token::Cs(integer.symbol()),
                Token::Cs(font.symbol()),
                other('X'),
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut effects,
        );
        let before = crate::command::command_ownership_counters();
        let mut text = String::new();
        processor
            .consume_expanded_words(&mut |processor, spelling, meaning, _, exceptional| {
                if let Some((ch, _)) = super::native_character(meaning) {
                    text.push(ch);
                    Ok(std::ops::ControlFlow::Continue(()))
                } else {
                    processor.back_consumed_word(spelling, exceptional)?;
                    Ok(std::ops::ControlFlow::Break(()))
                }
            })
            .expect("internal conversion");
        assert_eq!(text, "42");
        assert_eq!(
            processor
                .scan_font_selector_retained()
                .into_result()
                .expect("font"),
            tex_state::font::NULL_FONT
        );
        let after = crate::command::command_ownership_counters();
        assert_eq!(after.resolved_writes, before.resolved_writes);
        assert_eq!(
            crate::test_harness::expect_expanded_command(&mut processor)
                .spelling()
                .semantic_token(),
            other('X')
        );
    });
}
