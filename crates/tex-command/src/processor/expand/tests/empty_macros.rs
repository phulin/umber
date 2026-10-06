//! Empty expansion residency, demanded tracing, retirement and fuel cuts.
use super::*;
use crate::{CommandError, DeliveryStatus};

#[test]
fn readiness_is_read_once_per_actual_physical_loan() {
    for source in [false, true] {
        crate::test_harness::with_universe(|universe| {
            let noop = universe.intern("noop").expect("name");
            universe
                .assign_meaning(
                    noop,
                    MeaningWord::from_static(Meaning::Relax),
                    AssignmentScope::Global,
                )
                .expect("terminal");
            let mut command = CommandState::default();
            if source {
                let text = format!("{}%", r"\noop".repeat(8));
                let source = command
                    .register_source(crate::SourceRegistration::new(
                        crate::RegisteredSourceKind::Generated,
                        text.as_bytes(),
                    ))
                    .expect("source");
                command.open_registered_source(source).expect("open");
            } else {
                crate::test_harness::push(&mut command, [Token::Cs(noop.symbol()); 8]);
            }
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
                ready: true,
                ..RelaxConsumer::default()
            };
            assert_eq!(
                processor
                    .consume_main_control(&mut consumer)
                    .expect("delivery"),
                DeliveryStatus::Command
            );
            assert_eq!(
                (consumer.completed, consumer.resident, consumer.full),
                (8, 8 - usize::from(source), usize::from(source)),
                "cold source-line acquisition invalidates the initial entry proof"
            );
            assert_eq!(
                consumer.readiness_reads.get(),
                1,
                "the physical loan must not re-query readiness per word"
            );
            drop(processor);
            assert_eq!(fuel.burned(), 8);
        });
    }
}

fn input<G>(
    command: &mut CommandState<G>,
    source: bool,
    entry: Option<Token>,
    empty: Token,
    noop: Option<Token>,
) {
    if source {
        let mut text = if entry.is_some() {
            r"\noop".to_owned()
        } else {
            String::new()
        };
        text.push_str(r"\empty\empty\empty\empty");
        if noop.is_some() {
            text.push_str(r"\noop");
        }
        text.push('%');
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                text.as_bytes(),
            ))
            .expect("source");
        command.open_registered_source(source).expect("open");
    } else {
        crate::test_harness::push(command, entry.into_iter().chain([empty; 4]).chain(noop));
    }
}

#[test]
fn empty_expansion_uses_the_existing_entry_proof_and_retires_at_its_real_boundary() {
    for source in [false, true] {
        for tracing in [false, true] {
            for terminal in [false, true] {
                for entered in [false, true] {
                    crate::test_harness::with_universe(|universe| {
                        let empty = universe.intern("empty").expect("name");
                        let noop = universe.intern("noop").expect("name");
                        let definition = universe.allocate_definition(&[], &[]).expect("publish");
                        universe
                            .assign_meaning(
                                empty,
                                definition.meaning(MeaningFlags::EMPTY),
                                AssignmentScope::Global,
                            )
                            .expect("macro");
                        universe
                            .assign_meaning(
                                noop,
                                MeaningWord::from_static(Meaning::Relax),
                                AssignmentScope::Global,
                            )
                            .expect("terminal");
                        if tracing {
                            universe
                                .assign_int_param(
                                    IntParam::TRACING_MACROS,
                                    1,
                                    AssignmentScope::Global,
                                )
                                .expect("trace");
                        }
                        let mut command = CommandState::default();
                        input(
                            &mut command,
                            source,
                            entered.then_some(Token::Cs(noop.symbol())),
                            Token::Cs(empty.symbol()),
                            terminal.then_some(Token::Cs(noop.symbol())),
                        );
                        let mut capabilities = CommandHostCapabilities::default();
                        let mut fuel = crate::CommandFuelLedger::new(5 + u64::from(entered))
                            .expect("four expansions and terminal/EOF");
                        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
                        let mut context = universe.command_context().expect("context");
                        let mut processor = crate::test_harness::processor(
                            &mut command,
                            &mut context,
                            &mut capabilities,
                            &mut fuel,
                            &mut effects,
                        );
                        let before = crate::macro_call::resident_empty_macro_count();
                        let mut consumer = RelaxConsumer {
                            stop: 1 + usize::from(entered),
                            ..RelaxConsumer::default()
                        };
                        assert!(
                            !consumer.ready,
                            "expansion must not require executor entry facts"
                        );
                        assert_eq!(
                            processor
                                .consume_main_control(&mut consumer)
                                .expect("delivery"),
                            if terminal {
                                crate::DeliveryStatus::Command
                            } else {
                                crate::DeliveryStatus::End
                            }
                        );
                        assert_eq!(
                            consumer.completed,
                            usize::from(terminal) + usize::from(entered),
                            "empty expansion is not a terminal operation"
                        );
                        assert_eq!(
                            consumer.full,
                            usize::from(entered) + usize::from(terminal && (!entered || tracing))
                        );
                        assert_eq!(
                            consumer.resident,
                            usize::from(entered && terminal && !tracing)
                        );
                        assert_eq!(
                            crate::macro_call::resident_empty_macro_count() - before,
                            if tracing || !entered {
                                0
                            } else if source || terminal {
                                4
                            } else {
                                3
                            },
                            "tracing and the last stored word require their actual activation boundary"
                        );
                        if !tracing {
                            assert_eq!(
                                processor.command.stack_usage().input_stack,
                                1,
                                "logical empty activation preserves input high-water accounting without a row"
                            );
                        }
                        drop(processor);
                        assert_eq!(fuel.burned(), 5 + u64::from(entered));
                    });
                }
            }
        }
    }
}

#[test]
fn empty_expansion_fuel_cuts_leave_the_next_real_word_unread() {
    for source in [false, true] {
        for limit in 1..=4_u64 {
            for entered in [false, true] {
                crate::test_harness::with_universe(|universe| {
                    let empty = universe.intern("empty").expect("name");
                    let noop = universe.intern("noop").expect("name");
                    let definition = universe.allocate_definition(&[], &[]).expect("publish");
                    universe
                        .assign_meaning(
                            empty,
                            definition.meaning(MeaningFlags::EMPTY),
                            AssignmentScope::Global,
                        )
                        .expect("macro");
                    universe
                        .assign_meaning(
                            noop,
                            MeaningWord::from_static(Meaning::Relax),
                            AssignmentScope::Global,
                        )
                        .expect("terminal");
                    let mut command = CommandState::default();
                    input(
                        &mut command,
                        source,
                        entered.then_some(Token::Cs(noop.symbol())),
                        Token::Cs(empty.symbol()),
                        Some(Token::Cs(noop.symbol())),
                    );
                    let mut capabilities = CommandHostCapabilities::default();
                    let mut fuel = crate::CommandFuelLedger::new(limit + u64::from(entered))
                        .expect("bounded fuel");
                    let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
                    let mut context = universe.command_context().expect("context");
                    let mut processor = crate::test_harness::processor(
                        &mut command,
                        &mut context,
                        &mut capabilities,
                        &mut fuel,
                        &mut effects,
                    );
                    let before = crate::macro_call::resident_empty_macro_count();
                    let mut consumer = RelaxConsumer {
                        stop: 1 + usize::from(entered),
                        ..RelaxConsumer::default()
                    };
                    assert!(processor.consume_main_control(&mut consumer).is_err());
                    assert_eq!(consumer.completed, usize::from(entered));
                    assert_eq!(
                        crate::macro_call::resident_empty_macro_count() - before,
                        if entered { limit } else { 0 }
                    );
                    drop(processor);
                    assert_eq!(fuel.burned(), limit + u64::from(entered));
                    let mut next_fuel = crate::CommandFuelLedger::new(1).expect("next real word");
                    let mut processor = crate::test_harness::processor(
                        &mut command,
                        &mut context,
                        &mut capabilities,
                        &mut next_fuel,
                        &mut effects,
                    );
                    assert_eq!(
                        processor
                            .get_token_spelling()
                            .expect("raw next")
                            .expect("word")
                            .semantic_token(),
                        Token::Cs(if limit == 4 {
                            noop.symbol()
                        } else {
                            empty.symbol()
                        })
                    );
                });
            }
        }
    }
}

struct ProtectedTerminal;

impl<G>
    crate::processor::expand::ExpandedReaderConsumer<
        crate::CommandProcessor<'_, '_, G>,
        G,
        crate::InitialCommandHostFacts,
    > for ProtectedTerminal
{
    const NATIVE_TERMINAL: bool = true;
    const RESIDENT_TERMINALS: bool = true;

    fn resident_ready(&self) -> bool {
        true
    }

    fn native_terminal(
        &mut self,
        _: &mut crate::CommandProcessor<'_, '_, G>,
        _: TracedTokenWord,
        meaning: &MeaningWord<G>,
        _: u64,
        _: Option<tex_state::interner::Symbol>,
    ) -> Result<ControlFlow<()>, CommandError> {
        assert_eq!(meaning.execution_key(), MeaningWord::<G>::EMPTY_MACRO_KEY);
        assert!(
            meaning
                .macro_flags()
                .expect("macro")
                .contains(MeaningFlags::PROTECTED)
        );
        Ok(ControlFlow::Break(()))
    }

    fn terminal(
        &mut self,
        _: &mut crate::CommandProcessor<'_, '_, G>,
        _: &mut Option<crate::CurrentCommand<G>>,
        _: crate::DeliveryStatus,
    ) -> Result<ControlFlow<()>, CommandError> {
        unreachable!("ordinary protected delivery retains its native meaning")
    }
}

#[test]
fn protected_empty_macro_is_delivered_before_resident_expansion() {
    for source in [false, true] {
        crate::test_harness::with_universe(|universe| {
            let empty = universe.intern("empty").expect("name");
            let noop = universe.intern("noop").expect("name");
            let definition = universe.allocate_definition(&[], &[]).expect("publish");
            universe
                .assign_meaning(
                    empty,
                    definition.meaning(MeaningFlags::PROTECTED),
                    AssignmentScope::Global,
                )
                .expect("protected macro");
            let mut command = CommandState::default();
            input(
                &mut command,
                source,
                None,
                Token::Cs(empty.symbol()),
                Some(Token::Cs(noop.symbol())),
            );
            let mut capabilities = CommandHostCapabilities::default();
            let mut fuel = crate::CommandFuelLedger::new(1).expect("protected word");
            let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut context = universe.command_context().expect("context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut effects,
            );
            let before = crate::macro_call::resident_empty_macro_count();
            assert_eq!(
                processor
                    .consume_expanded_with_mode::<false, true>(
                        &mut None,
                        None,
                        crate::processor::expand::ExpandedReaderRole::Scanner,
                        &mut ProtectedTerminal,
                    )
                    .expect("protected delivery"),
                crate::DeliveryStatus::Command
            );
            assert_eq!(crate::macro_call::resident_empty_macro_count(), before);
            drop(processor);
            assert_eq!(fuel.burned(), 1);
        });
    }
}
