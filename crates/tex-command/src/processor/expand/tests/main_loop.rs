//! Actual native lookahead and source-suffix ownership, fuel, and rollback.

use super::*;
use std::ops::ControlFlow;

#[test]
fn main_source_suffix_yields_lexical_boundaries_to_the_native_reader() {
    for (source, prefix, boundary, space_a) in [
        (
            "xab c%",
            "ab",
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            false,
        ),
        (
            "Aλ%",
            "",
            Token::Char {
                ch: 'λ',
                cat: Catcode::Other,
            },
            false,
        ),
        (
            "A^^61%",
            "",
            Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            },
            false,
        ),
        (
            "Aab%",
            "",
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            true,
        ),
    ] {
        crate::test_harness::with_universe(|universe| {
            universe
                .assign_code(
                    tex_state::env::CodeTableKind::Catcode,
                    '^',
                    i64::from(Catcode::Superscript as u8),
                    AssignmentScope::Global,
                )
                .expect("superscript catcode");
            let mut command =
                CommandState::new(CommandProfile::unicode_extended(CommandDialect::Tex82));
            let source_id = command
                .register_source(crate::SourceRegistration::new(
                    crate::RegisteredSourceKind::Generated,
                    source.as_bytes(),
                ))
                .expect("source registration");
            command
                .open_registered_source(source_id)
                .expect("source input");
            let mut capabilities = CommandHostCapabilities::default();
            let mut fuel = crate::CommandFuelLedger::new(8).expect("source fuel");
            let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut context = universe.command_context().expect("context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut effects,
            );
            crate::test_harness::expect_raw_command(&mut processor);
            if space_a {
                processor
                    .with_execution_owners(|context, _, _, _, _, _, _, _, _| {
                        context.assign_code(
                            tex_state::env::CodeTableKind::Catcode,
                            'a',
                            i64::from(Catcode::Space as u8),
                            AssignmentScope::Global,
                        )
                    })
                    .expect("live catcode change");
            }
            processor
                .command
                .profile_reset_input_source_context_counters();
            let writes = crate::command::command_ownership_counters().resolved_writes;
            let mut characters = RecordingCharacterConsumer::default();
            let count = processor
                .consume_main_source_characters(usize::MAX, &mut characters)
                .expect("source suffix");
            assert_eq!(count as usize, prefix.len(), "source={source}");
            assert_eq!(characters.characters, prefix, "source={source}");
            if !prefix.is_empty() {
                assert_eq!(
                    processor.command.profile_input_source_context_counters(),
                    (0, 0, 0, 1)
                );
                assert_ne!(characters.origins[0], characters.origins[1]);
            }
            let mut tail = None;
            assert_eq!(
                processor
                    .consume_main_loop_lookahead(&mut crate::terminal_consumer::<
                        crate::CommandProcessor<'_, '_, _>,
                        _,
                    >(
                        &mut |_, spelling, meaning, _code, _, exceptional| {
                            assert!(exceptional.is_none());
                            tail = Some((spelling.semantic_token(), meaning.character()));
                            Ok(ControlFlow::Break(()))
                        }
                    ))
                    .expect("native lexical boundary"),
                crate::DeliveryStatus::Command
            );
            assert_eq!(
                tail,
                Some((
                    boundary,
                    match boundary {
                        Token::Char { ch, cat } => Some((ch, cat)),
                        _ => unreachable!(),
                    }
                )),
                "source={source}"
            );
            assert_eq!(
                crate::command::command_ownership_counters().resolved_writes,
                writes
            );
            drop(processor);
            assert_eq!(fuel.burned(), 2 + prefix.len() as u64, "source={source}");
        });
    }
}

#[test]
fn source_scalar_fallback_and_operation_limit_advance_one_cursor() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"xabcdef%"[..],
            ))
            .expect("source");
        command
            .open_registered_source(source)
            .expect("source input");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(7).expect("fuel");
        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut effects,
        );
        crate::test_harness::expect_raw_command(&mut processor);
        let writes = crate::command::command_ownership_counters().resolved_writes;
        let mut characters = RecordingCharacterConsumer::default();
        assert_eq!(
            processor
                .consume_main_source_characters(2, &mut characters)
                .expect("bounded suffix"),
            2
        );
        assert_eq!(characters.characters, "ab");
        characters.fallback_borrowed = true;
        for expected in ["abc", "abcd"] {
            assert_eq!(
                processor
                    .consume_main_source_characters(2, &mut characters)
                    .expect("scalar fallback"),
                1
            );
            assert_eq!(characters.characters, expected);
        }
        characters.fallback_borrowed = false;
        assert_eq!(
            processor
                .consume_main_source_characters(2, &mut characters)
                .expect("remaining suffix"),
            2
        );
        assert_eq!(characters.characters, "abcdef");
        assert!(characters.origins.windows(2).all(|pair| pair[0] != pair[1]));
        assert_eq!(
            crate::command::command_ownership_counters().resolved_writes,
            writes
        );
        drop(processor);
        assert_eq!(fuel.burned(), 7);
    });
}

#[test]
fn native_macro_body_charges_each_word_once() {
    crate::test_harness::with_universe(|universe| {
        let body_tokens = [
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'b',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: 'c',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
        ];
        let words: Vec<_> = body_tokens.iter().copied().map(TokenWord::pack).collect();
        let definition = universe
            .allocate_definition(&[], &words)
            .expect("macro body definition");
        let definition_publication = definition;
        let definition = definition_publication.reference();
        let macro_name = universe.intern("runbody").expect("macro name").symbol();
        let body = universe
            .command_context()
            .expect("macro body context")
            .admit_macro_body(definition)
            .expect("resident macro body");
        let mut command = CommandState::default();
        command.push_macro_activation(macro_name, body, None, OriginId::UNKNOWN);

        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(4).expect("macro body character-run fuel");
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let writes = crate::command::command_ownership_counters().resolved_writes;
        let mut delivered = Vec::new();
        assert_eq!(
            processor
                .consume_main_loop_lookahead(&mut crate::terminal_consumer::<
                    crate::CommandProcessor<'_, '_, _>,
                    _,
                >(
                    &mut |_, spelling, _, _code, _, exceptional| {
                        assert!(exceptional.is_none());
                        delivered.push(spelling);
                        Ok(if delivered.len() == 4 {
                            ControlFlow::Break(())
                        } else {
                            ControlFlow::Continue(())
                        })
                    }
                ))
                .expect("native lookahead"),
            crate::DeliveryStatus::Command
        );
        assert_eq!(
            delivered
                .iter()
                .map(|word| word.semantic_token())
                .collect::<Vec<_>>(),
            body_tokens
        );
        assert_eq!(
            crate::command::command_ownership_counters().resolved_writes,
            writes
        );
        assert!(
            delivered
                .iter()
                .all(|word| word.origin() == OriginId::UNKNOWN)
        );
        drop(processor);
        assert_eq!(fuel.burned(), 4);
    });
}

#[test]
fn native_macro_argument_charges_each_word_once() {
    crate::test_harness::with_universe(|universe| {
        let argument_tokens = [
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'b',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: 'c',
                cat: Catcode::Letter,
            },
        ];
        let traced = argument_tokens.map(|token| TracedTokenWord::pack(token, OriginId::UNKNOWN));
        let mut command = CommandState::default();
        let mut matching = command
            .input
            .levels
            .arguments
            .begin_match()
            .expect("macro match");
        let mut writer = command.input.levels.arguments.begin_argument(&matching);
        for word in traced {
            command
                .input
                .levels
                .arguments
                .append_argument_word(&mut writer, word)
                .expect("macro argument word");
        }
        command
            .input
            .levels
            .arguments
            .publish_argument(&mut matching, writer)
            .expect("macro argument range");
        let macro_name = universe.intern("runargument").expect("macro name").symbol();
        let body_definition = universe
            .allocate_definition(
                &[TokenWord::pack(Token::Param(1))],
                &[TokenWord::pack(Token::Param(1))],
            )
            .expect("macro body definition");
        let body_definition_publication = body_definition;
        let body_definition = body_definition_publication.reference();
        let body = universe
            .command_context()
            .expect("macro body context")
            .admit_macro_body(body_definition)
            .expect("resident macro body");
        command.push_macro_activation(macro_name, body, Some(matching), OriginId::UNKNOWN);

        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(3).expect("macro argument fuel");
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let writes = crate::command::command_ownership_counters().resolved_writes;
        let mut delivered = Vec::new();
        assert_eq!(
            processor
                .consume_main_loop_lookahead(&mut crate::terminal_consumer::<
                    crate::CommandProcessor<'_, '_, _>,
                    _,
                >(
                    &mut |_, spelling, _, _code, _, exceptional| {
                        assert!(exceptional.is_none());
                        delivered.push(spelling);
                        Ok(if delivered.len() == 3 {
                            ControlFlow::Break(())
                        } else {
                            ControlFlow::Continue(())
                        })
                    }
                ))
                .expect("native lookahead"),
            crate::DeliveryStatus::Command
        );
        assert_eq!(
            delivered
                .iter()
                .map(|word| word.semantic_token())
                .collect::<Vec<_>>(),
            argument_tokens
        );
        assert_eq!(
            crate::command::command_ownership_counters().resolved_writes,
            writes
        );
        assert_eq!(delivered, traced);
        drop(processor);
        assert_eq!(fuel.burned(), 3);
    });
}

#[test]
fn native_lookahead_restore_keeps_fuel_monotonic() {
    crate::test_harness::with_universe(|universe| {
        let tokens = [
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'b',
                cat: Catcode::Other,
            },
        ];
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, tokens);
        let summary = command
            .publish_summary(universe)
            .expect("character-run summary");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(4).expect("rollback character-run fuel");
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();

        for expected_burned in [2, 4] {
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            let writes = crate::command::command_ownership_counters().resolved_writes;
            let mut delivered = Vec::new();
            assert_eq!(
                processor
                    .consume_main_loop_lookahead(&mut crate::terminal_consumer::<
                        crate::CommandProcessor<'_, '_, _>,
                        _,
                    >(
                        &mut |_, spelling, _, _code, _, exceptional| {
                            assert!(exceptional.is_none());
                            delivered.push(spelling);
                            Ok(if delivered.len() == 2 {
                                ControlFlow::Break(())
                            } else {
                                ControlFlow::Continue(())
                            })
                        }
                    ))
                    .expect("native lookahead"),
                crate::DeliveryStatus::Command
            );
            assert_eq!(
                delivered
                    .iter()
                    .map(|word| word.semantic_token())
                    .collect::<Vec<_>>(),
                tokens
            );
            assert_eq!(
                crate::command::command_ownership_counters().resolved_writes,
                writes
            );
            drop(processor);
            assert_eq!(fuel.burned(), expected_burned);
            if expected_burned == 2 {
                drop(context);
                command
                    .restore_summary(&summary, universe)
                    .expect("restore actual input");
            }
        }
    });
}

#[test]
fn noexpand_suppresses_exactly_one_expandable_delivery() {
    crate::test_harness::with_universe(|universe| {
        let noexpand = install_static(
            universe,
            "noexpand",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand),
        );
        let replacement = Token::Char {
            ch: 'B',
            cat: Catcode::Letter,
        };
        let definition = universe
            .allocate_definition(&[], &[TokenWord::pack(replacement)])
            .expect("definition");
        let definition_publication = definition;
        let symbol = universe.intern("m").expect("macro name");
        universe
            .assign_meaning(
                symbol,
                definition_publication.meaning(MeaningFlags::EMPTY),
                AssignmentScope::Global,
            )
            .expect("macro meaning");
        let macro_token = Token::Cs(symbol.symbol());
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [noexpand, macro_token, macro_token]);
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
        let persistent_mode = processor.command.delivery_mode;

        let suppressed = crate::test_harness::expect_expanded_command(&mut processor);
        assert_eq!(suppressed.spelling().semantic_token(), macro_token);
        assert_eq!(suppressed.meaning(), Meaning::Relax);
        assert_eq!(processor.command.delivery_mode, persistent_mode);
        assert_eq!(
            crate::test_harness::expect_expanded_command(&mut processor)
                .spelling()
                .semantic_token(),
            replacement
        );
        assert_eq!(processor.command.delivery_mode, persistent_mode);
    });
}

#[test]
fn execution_character_boundaries_expand_without_constructing_commands() {
    // TeX82 §§380/389/1038: expand the boundary, then consume replacement
    // characters through the same input owner. The final relax is the terminal.
    for source in [
        r"a\m c\relax",
        r"a\empty c\relax",
        r"a\expandafter\m\empty c\relax",
    ] {
        crate::test_harness::with_universe(|universe| {
            let m = universe.intern("m").expect("macro symbol");
            let publication = universe
                .allocate_definition(
                    &[],
                    &[TokenWord::pack(Token::Char {
                        ch: 'b',
                        cat: Catcode::Letter,
                    })],
                )
                .expect("macro body");
            universe
                .assign_meaning(
                    m,
                    publication.meaning(MeaningFlags::EMPTY),
                    AssignmentScope::Global,
                )
                .expect("macro meaning");
            let empty = universe.intern("empty").expect("empty macro symbol");
            let empty_publication = universe
                .allocate_definition(&[], &[])
                .expect("empty macro body");
            universe
                .assign_meaning(
                    empty,
                    empty_publication.meaning(MeaningFlags::EMPTY),
                    AssignmentScope::Global,
                )
                .expect("empty macro meaning");
            for (name, meaning) in [
                ("relax", Meaning::Relax),
                (
                    "expandafter",
                    Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter),
                ),
            ] {
                let symbol = universe.intern(name).expect("primitive symbol");
                universe
                    .assign_meaning(
                        symbol,
                        MeaningWord::scalar(meaning.encode()),
                        AssignmentScope::Global,
                    )
                    .expect("primitive meaning");
            }
            let relax = universe.intern("relax").expect("terminal symbol").symbol();
            let mut command = CommandState::default();
            let id = command
                .register_source(crate::SourceRegistration::new(
                    crate::RegisteredSourceKind::Generated,
                    source.as_bytes(),
                ))
                .expect("source");
            command.open_registered_source(id).expect("source input");
            let mut capabilities = CommandHostCapabilities::default();
            let mut fuel = crate::CommandFuelLedger::new(32).expect("fuel");
            let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut context = universe.command_context().expect("context");
            let before = crate::command::command_ownership_counters();
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut effects,
            );
            let mut characters = String::new();
            let mut terminal = None;
            assert_eq!(
                processor
                    .consume_main_loop_lookahead(&mut crate::terminal_consumer::<
                        crate::CommandProcessor<'_, '_, _>,
                        _,
                    >(
                        &mut |_, spelling, _, _code, _, exceptional| {
                            assert!(exceptional.is_none(), "source={source}");
                            match spelling.semantic_token() {
                                Token::Char {
                                    ch,
                                    cat: Catcode::Letter | Catcode::Other,
                                } => {
                                    characters.push(ch);
                                    Ok(ControlFlow::Continue(()))
                                }
                                Token::Cs(symbol) if symbol == relax => {
                                    terminal = Some(spelling.token_word());
                                    Ok(ControlFlow::Break(()))
                                }
                                token => {
                                    panic!("unexpected native terminal {token:?}, source={source}")
                                }
                            }
                        }
                    ))
                    .expect("native expansion"),
                crate::DeliveryStatus::Command
            );
            assert_eq!(processor.command.transient.active_expansion_depth, 0);
            assert_eq!(terminal, Some(TokenWord::pack(Token::Cs(relax))));
            assert_eq!(
                characters,
                if source.contains("\\empty") && !source.contains("expandafter") {
                    "ac"
                } else {
                    "abc"
                }
            );
            drop(processor);
            assert_eq!(
                crate::command::command_ownership_counters().resolved_writes,
                before.resolved_writes,
                "source={source}"
            );
            assert_eq!(
                fuel.burned(),
                if source.contains("expandafter") {
                    8
                } else if source.contains("empty") {
                    4
                } else {
                    5
                },
                "source={source}"
            );
        });
    }
}
