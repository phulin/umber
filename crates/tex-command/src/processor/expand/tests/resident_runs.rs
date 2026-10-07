use super::*;

#[test]
fn skipped_resident_run_completes_at_its_first_conditional_delimiter() {
    for source in [false, true] {
        crate::test_harness::with_universe(|universe| {
            let relax = install_static(universe, "relaxing", Meaning::Relax);
            let fi = install_static(
                universe,
                "fiish",
                Meaning::ExpandablePrimitive(tex_state::meaning::ExpandablePrimitive::Fi),
            );
            let mut command = CommandState::default();
            // TeX82 §494 discards every ordinary skipped command; only the
            // `fi_or_else` delimiter completes the consumer.
            if source {
                let text = "x\\relaxing\n".repeat(2_048) + "\\fiish%";
                let source_id = command
                    .register_source(crate::SourceRegistration::new(
                        crate::RegisteredSourceKind::Generated,
                        text.as_bytes(),
                    ))
                    .expect("source registration");
                command
                    .open_registered_source(source_id)
                    .expect("source input");
            } else {
                crate::test_harness::push(
                    &mut command,
                    (0..4_096)
                        .map(|index| {
                            if index % 2 == 0 {
                                Token::Char {
                                    ch: 'x',
                                    cat: Catcode::Letter,
                                }
                            } else {
                                relax
                            }
                        })
                        .chain([fi]),
                );
            }
            let mut capabilities = CommandHostCapabilities::default();
            let mut fuel = crate::CommandFuelLedger::new(4_097).expect("skip fuel");
            let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut effects,
            );
            let writes = crate::command::command_ownership_counters().resolved_writes;
            let stop = processor.consume_skipped_text().expect("completed skip");
            assert_eq!(
                stop.delimiter,
                crate::conditionals::ConditionalDelimiter::Fi
            );
            assert_eq!(stop.nested_conditions, 0);
            assert_eq!(
                crate::command::command_ownership_counters().resolved_writes,
                writes,
                "ordinary skip constructs no commands, source={source}"
            );
            drop(processor);
            assert_eq!(fuel.burned(), 4_097);
        });
    }
}

#[test]
fn skipped_resident_run_counts_nested_conditionals_and_braces_in_place() {
    crate::test_harness::with_universe(|universe| {
        use tex_state::meaning::ExpandablePrimitive;
        let if_true = install_static(
            universe,
            "iftrueish",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfTrue),
        );
        let else_ = install_static(
            universe,
            "elseish",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Else),
        );
        let fi = install_static(
            universe,
            "fiish",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Fi),
        );
        let letter = Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        };
        let open = Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        };
        let close = Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        };
        let mut command = CommandState::default();
        // TeX82 §494: the nested `\iftrue...\else...\fi` and the balanced
        // braces are skipped; only the outer `\fi` completes the consumer. §347 still
        // counts the skipped braces, which leaves `align_state` net +1 here.
        crate::test_harness::push(
            &mut command,
            [
                letter, open, if_true, letter, else_, letter, fi, close, open, fi,
            ],
        );
        let align_state = command.roots.alignment.align_state;
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(64).expect("skip fuel");
        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut effects,
        );
        let stop = processor.consume_skipped_text().expect("completed skip");
        assert_eq!(
            stop.delimiter,
            crate::conditionals::ConditionalDelimiter::Fi
        );
        assert_eq!(stop.nested_conditions, 0);
        drop(processor);
        assert_eq!(fuel.burned(), 10);
        assert_eq!(command.roots.alignment.align_state, align_state + 1);
    });
}

#[test]
fn skipped_resident_run_stops_before_the_over_budget_word() {
    for limit in 1..=5 {
        crate::test_harness::with_universe(|universe| {
            let mut command = CommandState::default();
            crate::test_harness::push(
                &mut command,
                (0..7).map(|_| Token::Char {
                    ch: 'x',
                    cat: Catcode::Letter,
                }),
            );
            let mut capabilities = CommandHostCapabilities::default();
            let mut fuel = crate::CommandFuelLedger::new(limit).expect("bounded fuel");
            let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut effects,
            );
            assert!(matches!(
                processor.consume_skipped_text(),
                Err(error) if matches!(error.kind, crate::CommandErrorKind::FuelExhausted(_))
            ));
            let row @ crate::input::InputLevel {
                storage:
                    crate::input::InputStorage::List(_)
                    | crate::input::InputStorage::StoredList(_)
                    | crate::input::InputStorage::MacroBody(_)
                    | crate::input::InputStorage::MacroArgument { .. },
                ..
            } = processor
                .command
                .roots
                .input
                .levels
                .rows
                .last()
                .expect("resident input")
            else {
                panic!("resident input expected")
            };
            assert_eq!(u64::from(row.frame.position()), limit);
            drop(processor);
            assert_eq!(fuel.burned(), limit);
        });
    }
}

#[test]
fn resident_character_stop_resumes_without_reselecting_each_word() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let letters = (0..4096).map(|_| Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        });
        crate::test_harness::push(
            &mut command,
            letters.chain([Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            }]),
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(4097).expect("run fuel");
        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut characters = String::new();
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut effects,
        );
        let writes = crate::command::command_ownership_counters().resolved_writes;
        assert_eq!(
            processor
                .consume_main_loop_lookahead(&mut crate::terminal_consumer::<
                    crate::CommandProcessor<'_, '_, _>,
                    _,
                >(
                    &mut |_, spelling, _, _code, _, exceptional| {
                        assert!(exceptional.is_none());
                        let Token::Char {
                            ch,
                            cat: Catcode::Letter,
                        } = spelling.semantic_token()
                        else {
                            panic!("native prefix stopped after its operation budget");
                        };
                        characters.push(ch);
                        Ok(if characters.len() == 17 {
                            std::ops::ControlFlow::Break(())
                        } else {
                            std::ops::ControlFlow::Continue(())
                        })
                    }
                ))
                .expect("native prefix"),
            crate::DeliveryStatus::Command
        );
        assert_eq!(characters.len(), 17);
        assert_eq!(processor.fuel.burned(), 17);
        let mut tail = None;
        assert_eq!(
            processor
                .consume_main_loop_lookahead(&mut crate::terminal_consumer::<
                    crate::CommandProcessor<'_, '_, _>,
                    _,
                >(
                    &mut |_, spelling, _, _code, _, exceptional| {
                        assert!(exceptional.is_none());
                        match spelling.semantic_token() {
                            Token::Char {
                                ch,
                                cat: Catcode::Letter,
                            } => {
                                characters.push(ch);
                                Ok(std::ops::ControlFlow::Continue(()))
                            }
                            token => {
                                tail = Some(token);
                                Ok(std::ops::ControlFlow::Break(()))
                            }
                        }
                    }
                ))
                .expect("native continuation"),
            crate::DeliveryStatus::Command
        );
        assert_eq!(characters, "x".repeat(4096));
        assert_eq!(
            tail,
            Some(Token::Char {
                ch: ' ',
                cat: Catcode::Space
            })
        );
        assert_eq!(
            crate::command::command_ownership_counters().resolved_writes,
            writes
        );
        drop(processor);
        assert_eq!(fuel.burned(), 4097);
    });
}

#[test]
fn resident_character_fuel_failure_preserves_exact_consumed_prefix() {
    for limit in 1..=5 {
        crate::test_harness::with_universe(|universe| {
            let mut command = CommandState::default();
            crate::test_harness::push(
                &mut command,
                "abcdefg".chars().map(|ch| Token::Char {
                    ch,
                    cat: Catcode::Letter,
                }),
            );
            let mut capabilities = CommandHostCapabilities::default();
            let mut fuel = crate::CommandFuelLedger::new(limit).expect("bounded fuel");
            let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut context = universe.command_context().expect("command context");
            let mut characters = String::new();
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut effects,
            );
            let writes = crate::command::command_ownership_counters().resolved_writes;
            assert!(
                processor
                    .consume_main_loop_lookahead(&mut crate::terminal_consumer::<
                        crate::CommandProcessor<'_, '_, _>,
                        _,
                    >(
                        &mut |_, spelling, _, _code, _, exceptional| {
                            assert!(exceptional.is_none());
                            let Token::Char {
                                ch,
                                cat: Catcode::Letter,
                            } = spelling.semantic_token()
                            else {
                                panic!("letter input");
                            };
                            characters.push(ch);
                            Ok(std::ops::ControlFlow::Continue(()))
                        }
                    ))
                    .is_err()
            );
            assert_eq!(characters, &"abcdefg"[..limit as usize]);
            assert_eq!(
                crate::command::command_ownership_counters().resolved_writes,
                writes
            );
            drop(processor);
            assert_eq!(fuel.burned(), limit);
        });
    }
}

#[test]
fn skipped_resident_run_crosses_an_exhausted_frame_without_scalar_delivery() {
    crate::test_harness::with_universe(|universe| {
        let fi = install_static(
            universe,
            "fiish",
            Meaning::ExpandablePrimitive(tex_state::meaning::ExpandablePrimitive::Fi),
        );
        let letter = Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        };
        let mut command = CommandState::default();
        // TeX82 §324 pops an exhausted token list inside §494's `get_next`
        // without delivering anything, so the skip continues into the
        // enclosing frame and only its `\fi` completes the consumer.
        crate::test_harness::push(&mut command, [letter, letter, fi]);
        crate::test_harness::push(&mut command, [letter, letter, letter]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(64).expect("skip fuel");
        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut effects,
        );
        let stop = processor.consume_skipped_text().expect("completed skip");
        assert_eq!(
            stop.delimiter,
            crate::conditionals::ConditionalDelimiter::Fi
        );
        assert_eq!(stop.nested_conditions, 0);
        drop(processor);
        assert_eq!(fuel.burned(), 6, "every skipped word is charged once");
        assert_eq!(
            command.roots.input.levels.rows.len(),
            1,
            "the exhausted frame is popped"
        );
    });
}
