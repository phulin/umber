//! Input-preserving native terminal admission, exact fuel and interruption.
#[path = "empty_macros.rs"]
mod empty_macros;

use super::*;
use std::ops::ControlFlow;

#[derive(Default)]
struct RelaxConsumer {
    stop: usize,
    completed: usize,
    full: usize,
    resident: usize,
    ready: bool,
    readiness_reads: core::cell::Cell<usize>,
}

impl<G>
    crate::NativeWordConsumer<crate::CommandProcessor<'_, '_, G>, G, crate::InitialCommandHostFacts>
    for RelaxConsumer
{
    const RESIDENT_TERMINALS: bool = true;
    fn resident_ready(&self) -> bool {
        self.readiness_reads.set(self.readiness_reads.get() + 1);
        self.ready
    }

    #[allow(clippy::too_many_arguments)] // Disjoint input-preserving execution owners and spelling.
    fn resident_terminal(
        &mut self,
        _: &mut tex_state::CommandContext<'_, G>,
        _fuel: &mut crate::CommandFuel,
        _effects: &mut tex_state::diagnostic::DiagnosticEffects,
        _: &mut crate::InitialCommandHostFacts,
        _: u32,
        _macro_origin: Option<tex_state::token::OriginId>,
        _: TracedTokenWord,
        code: u64,
        _operand: u64,
    ) -> ControlFlow<(), bool> {
        assert_eq!(
            code,
            Meaning::runtime_word_control_code(Meaning::Relax.encode()),
            "expanding meanings must not be offered to a terminal consumer"
        );
        self.resident += 1;
        self.completed += 1;
        if self.completed == self.stop {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(true)
        }
    }

    fn input_boundary(&mut self) {
        self.ready = false;
    }

    fn consume(
        &mut self,
        _: &mut crate::CommandProcessor<'_, '_, G>,
        _: TracedTokenWord,
        meaning: &MeaningWord<G>,
        code: u64,
        _: Option<tex_state::interner::Symbol>,
        exceptional: Option<&crate::CurrentCommand<G>>,
    ) -> Result<ControlFlow<()>, crate::CommandError> {
        assert!(exceptional.is_none());
        assert_eq!(
            meaning.execution_key(),
            Meaning::runtime_word_control_code(Meaning::Relax.encode())
        );
        assert_eq!(code, meaning.execution_key());
        self.full += 1;
        self.completed += 1;
        self.ready = true;
        self.readiness_reads.set(0);
        Ok(if self.completed == self.stop {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        })
    }
}

#[test]
fn pure_macro_input_changes_preserve_entry_facts_and_exact_fuel() {
    // Ownership/accounting contract, not an oracle event-count golden:
    // simple activation and Pop retirement cannot change execution-entry
    // facts. Tracing must invalidate them before its actual contextual path.
    for source in [false, true] {
        for tracing in [false, true] {
            for matching in [false, true] {
                let cuts: &[(usize, u64, usize)] = if matching {
                    &[
                        (1, 1, 1),
                        (2, 4, 2),
                        (3, 6, 3),
                        (4, 8, 4),
                        (5, 9, 4),
                        (6, 10, 4),
                        (7, 11, 4),
                        (8, 13, 5),
                        (9, 14, 5),
                    ]
                } else {
                    &[
                        (1, 1, 1),
                        (2, 3, 2),
                        (3, 5, 3),
                        (4, 6, 3),
                        (5, 7, 3),
                        (6, 9, 4),
                        (7, 10, 4),
                    ]
                };
                for (stop, burned, traced_entries) in cuts.iter().copied() {
                    crate::test_harness::with_universe(|universe| {
                        let noop = install_static(universe, "noop", Meaning::Relax);
                        let mut define = |name, parameters: &[TokenWord], body: &[Token]| {
                            let words: Vec<_> = body.iter().copied().map(TokenWord::pack).collect();
                            let definition = universe
                                .allocate_definition(parameters, &words)
                                .expect("replacement definition");
                            let name = universe.intern(name).expect("macro name");
                            universe
                                .assign_meaning(
                                    name,
                                    definition.meaning(MeaningFlags::EMPTY),
                                    AssignmentScope::Global,
                                )
                                .expect("installed definition");
                            Token::Cs(name.symbol())
                        };
                        let inner = define("inner", &[], &[noop, noop]);
                        let outer = define("outer", &[], &[noop, inner, noop]);
                        let empty = define("empty", &[], &[]);
                        let pass = define(
                            "pass",
                            &[TokenWord::pack(Token::Param(1))],
                            &[noop, Token::Param(1), noop],
                        );
                        if tracing {
                            universe
                                .assign_int_param(
                                    tex_state::env::banks::IntParam::TRACING_MACROS,
                                    1,
                                    AssignmentScope::Global,
                                )
                                .expect("trace parameter");
                        }
                        let mut command = CommandState::default();
                        if source {
                            let source = command
                                .register_source(crate::SourceRegistration::new(
                                    crate::RegisteredSourceKind::Generated,
                                    if matching {
                                        br"\noop\pass\outer\empty\noop\noop%".as_slice()
                                    } else {
                                        br"\noop\outer\empty\noop\noop%".as_slice()
                                    },
                                ))
                                .expect("source registration");
                            command
                                .open_registered_source(source)
                                .expect("source input");
                        } else {
                            let words = if matching {
                                vec![noop, pass, outer, empty, noop, noop]
                            } else {
                                vec![noop, outer, empty, noop, noop]
                            };
                            crate::test_harness::push(&mut command, words);
                        }
                        let mut capabilities = CommandHostCapabilities::default();
                        let mut fuel = crate::CommandFuelLedger::new(burned).expect("exact fuel");
                        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
                        let mut context = universe.command_context().expect("context");
                        let mut processor = crate::test_harness::processor(
                            &mut command,
                            &mut context,
                            &mut capabilities,
                            &mut fuel,
                            &mut effects,
                        );
                        let mut consumer = RelaxConsumer {
                            stop,
                            ..RelaxConsumer::default()
                        };
                        assert_eq!(
                            processor
                                .consume_main_control(&mut consumer)
                                .expect("budget cut"),
                            crate::DeliveryStatus::Command,
                        );
                        assert_eq!(consumer.completed, stop);
                        let full = if tracing {
                            traced_entries
                        } else if matching && stop > 1 {
                            2
                        } else {
                            1
                        };
                        assert_eq!(
                            consumer.full, full,
                            "source={source}, tracing={tracing}, matching={matching}, stop={stop}"
                        );
                        assert_eq!(consumer.resident, stop - full);
                        drop(processor);
                        assert_eq!(fuel.burned(), burned);
                    });
                }
            }
        }
    }
}

#[test]
fn expandafter_enters_the_expander_before_terminal_admission() {
    // TeX82 §368: expand the second operand before replaying the first.
    // Ownership assertion: the resident terminal cannot execute an expander.
    for source in [false, true] {
        crate::test_harness::with_universe(|universe| {
            let noop = install_static(universe, "noop", Meaning::Relax);
            let expand = install_static(
                universe,
                "expandafter",
                Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter),
            );
            let definition = universe.allocate_definition(&[], &[]).expect("empty body");
            let empty = universe.intern("empty").expect("empty name");
            universe
                .assign_meaning(
                    empty,
                    definition.meaning(MeaningFlags::EMPTY),
                    AssignmentScope::Global,
                )
                .expect("empty macro");
            let mut command = CommandState::default();
            if source {
                let source = command
                    .register_source(crate::SourceRegistration::new(
                        crate::RegisteredSourceKind::Generated,
                        br"\noop\expandafter\noop\empty\noop%",
                    ))
                    .expect("source registration");
                command
                    .open_registered_source(source)
                    .expect("source input");
            } else {
                crate::test_harness::push(
                    &mut command,
                    [noop, expand, noop, Token::Cs(empty.symbol()), noop],
                );
            }
            let mut capabilities = CommandHostCapabilities::default();
            let mut fuel = crate::CommandFuelLedger::new(6).expect("exact fuel");
            let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut context = universe.command_context().expect("context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut effects,
            );
            let mut consumer = RelaxConsumer {
                stop: 3,
                ..RelaxConsumer::default()
            };
            assert_eq!(
                processor
                    .consume_main_control(&mut consumer)
                    .expect("expandafter"),
                crate::DeliveryStatus::Command
            );
            assert_eq!(consumer.completed, 3);
            assert_eq!(
                consumer.full, 2,
                "primitive expansion requires actual full entry"
            );
            assert_eq!(consumer.resident, 1);
            drop(processor);
            assert_eq!(fuel.burned(), 6);
        });
    }
}

#[test]
fn native_resident_noops_keep_the_span_until_the_exact_budget_cut() {
    for source in [false, true] {
        for stop in 1..=8 {
            crate::test_harness::with_universe(|universe| {
                let relax = install_static(universe, "noop", Meaning::Relax);
                let mut command = CommandState::default();
                if source {
                    let text = "\\noop".repeat(8) + "%";
                    let source = command
                        .register_source(crate::SourceRegistration::new(
                            crate::RegisteredSourceKind::Generated,
                            text.as_bytes(),
                        ))
                        .expect("source registration");
                    command
                        .open_registered_source(source)
                        .expect("source input");
                } else {
                    crate::test_harness::push(&mut command, [relax; 8]);
                }
                let mut capabilities = CommandHostCapabilities::default();
                let mut fuel = crate::CommandFuelLedger::new(stop as u64).expect("exact fuel");
                let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
                let mut context = universe.command_context().expect("context");
                let mut processor = crate::test_harness::processor(
                    &mut command,
                    &mut context,
                    &mut capabilities,
                    &mut fuel,
                    &mut effects,
                );
                let mut consumer = RelaxConsumer {
                    stop,
                    ..RelaxConsumer::default()
                };
                assert_eq!(
                    processor
                        .consume_main_control(&mut consumer)
                        .expect("budget cut"),
                    crate::DeliveryStatus::Command
                );
                assert_eq!(consumer.completed, stop);
                assert_eq!(
                    consumer.full, 1,
                    "full access occurs only before the clean span"
                );
                assert_eq!(consumer.resident, stop - 1);
                assert_eq!(
                    processor
                        .command
                        .roots
                        .input
                        .levels
                        .rows
                        .last()
                        .expect("actual frame")
                        .frame
                        .position() as usize,
                    stop
                );
                drop(processor);
                assert_eq!(fuel.burned(), stop as u64);
            });
        }
    }
}

#[test]
fn resident_noop_fuel_exhaustion_leaves_the_next_word_unread() {
    for source in [false, true] {
        for limit in 1..8 {
            crate::test_harness::with_universe(|universe| {
                let relax = install_static(universe, "noop", Meaning::Relax);
                let mut command = CommandState::default();
                if source {
                    let text = "\\noop".repeat(8) + "%";
                    let source = command
                        .register_source(crate::SourceRegistration::new(
                            crate::RegisteredSourceKind::Generated,
                            text.as_bytes(),
                        ))
                        .expect("source registration");
                    command
                        .open_registered_source(source)
                        .expect("source input");
                } else {
                    crate::test_harness::push(&mut command, [relax; 8]);
                }
                let mut capabilities = CommandHostCapabilities::default();
                let mut fuel = crate::CommandFuelLedger::new(limit as u64).expect("bounded fuel");
                let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
                let mut context = universe.command_context().expect("context");
                let mut processor = crate::test_harness::processor(
                    &mut command,
                    &mut context,
                    &mut capabilities,
                    &mut fuel,
                    &mut effects,
                );
                let mut consumer = RelaxConsumer {
                    stop: 8,
                    ..RelaxConsumer::default()
                };
                assert!(processor.consume_main_control(&mut consumer).is_err());
                assert_eq!(consumer.completed, limit);
                assert_eq!(
                    processor
                        .command
                        .roots
                        .input
                        .levels
                        .rows
                        .last()
                        .expect("actual frame")
                        .frame
                        .position() as usize,
                    limit
                );
                drop(processor);
                assert_eq!(fuel.burned(), limit as u64);
            });
        }
    }
}

#[test]
fn ordinary_frame_retirement_preserves_the_clean_continuation() {
    crate::test_harness::with_universe(|universe| {
        let relax = install_static(universe, "noop", Meaning::Relax);
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [relax; 4]);
        crate::test_harness::push(&mut command, [relax; 4]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(8).expect("exact fuel");
        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut effects,
        );
        let mut consumer = RelaxConsumer {
            stop: 8,
            ..RelaxConsumer::default()
        };
        assert_eq!(
            processor
                .consume_main_control(&mut consumer)
                .expect("retired frame"),
            crate::DeliveryStatus::Command
        );
        assert_eq!(
            consumer.full, 1,
            "ordinary token retirement does not change execution-entry facts"
        );
        assert_eq!(consumer.resident, 7);
        assert_eq!(processor.command.roots.input.levels.rows.len(), 1);
        drop(processor);
        assert_eq!(fuel.burned(), 8);
    });
}

#[test]
fn terminal_selectors_follow_expansion_and_live_redefinition() {
    for observed in [false, true] {
        crate::test_harness::with_universe(|universe| {
            let mutable = universe.intern("mutable").expect("mutable symbol");
            universe
                .assign_meaning(
                    mutable,
                    MeaningWord::from_static(Meaning::Relax),
                    AssignmentScope::Global,
                )
                .expect("initial meaning");
            let letter = Meaning::CharToken {
                ch: 'Q',
                cat: Catcode::Letter,
            };
            let replacement = Token::Char {
                ch: 'Q',
                cat: Catcode::Letter,
            };
            let definition = universe
                .allocate_definition(&[], &[TokenWord::pack(replacement)])
                .expect("definition");
            let name = universe.intern("macro").expect("macro symbol");
            universe
                .assign_meaning(
                    name,
                    definition.meaning(MeaningFlags::EMPTY),
                    AssignmentScope::Global,
                )
                .expect("macro meaning");
            let count_meaning =
                Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Count);
            let count = install_static(universe, "count", count_meaning);
            let end_name_meaning = Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName);
            let end_name = install_static(universe, "endcsname", end_name_meaning);
            let mut command = CommandState::default();
            crate::test_harness::push(
                &mut command,
                [
                    Token::Cs(name.symbol()),
                    Token::Cs(mutable.symbol()),
                    Token::Cs(mutable.symbol()),
                    count,
                    end_name,
                ],
            );
            let mut capabilities = CommandHostCapabilities::default();
            let mut fuel = crate::CommandFuelLedger::new(6).expect("exact fuel");
            let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut observer = RecordingObserver::default();
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
            let expected = [
                letter,
                Meaning::Relax,
                Meaning::CharGiven('A'),
                count_meaning,
                end_name_meaning,
            ];
            let mut delivered = 0;
            processor
                .consume_main_control(&mut crate::terminal_consumer::<
                    crate::CommandProcessor<'_, '_, _>,
                    _,
                >(
                    &mut |processor, _, meaning, code, _, exceptional| {
                        assert_eq!(
                            meaning.resolve(),
                            tex_state::meaning::ResolvedMeaning::Static(expected[delivered])
                        );
                        assert_eq!(
                            code,
                            Meaning::runtime_word_control_code(expected[delivered].encode())
                        );
                        assert_eq!(exceptional.is_some(), observed);
                        if delivered == 1 {
                            processor
                                .state
                                .assign_meaning(
                                    mutable.symbol(),
                                    MeaningWord::from_static(Meaning::CharGiven('A')),
                                    AssignmentScope::Global,
                                )
                                .expect("redefine next terminal");
                        }
                        delivered += 1;
                        Ok(if delivered == expected.len() {
                            ControlFlow::Break(())
                        } else {
                            ControlFlow::Continue(())
                        })
                    },
                ))
                .expect("native terminal delivery");
            assert_eq!(delivered, expected.len());
            drop(processor);
            assert_eq!(fuel.burned(), 6);
        });
    }
}
