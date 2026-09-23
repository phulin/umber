//! Character-run and source-step ownership, fuel, and rollback.

use super::*;

#[test]
fn main_loop_character_run_resolves_only_its_non_character_tail() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [
                Token::Char {
                    ch: 'A',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: 'b',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: ' ',
                    cat: Catcode::Space,
                },
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(3).expect("character-run fuel");
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let ownership_before = crate::command::command_ownership_counters();
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let mut destination = None;
        let mut consumer = RecordingCharacterConsumer::default();
        assert_eq!(
            processor
                .main_loop_source_step_into(&mut destination, &mut consumer)
                .expect("borrowed character run"),
            crate::DeliveryStatus::CharacterRunBoundary
        );
        assert_eq!(consumer.characters, "Ab");
        assert!(matches!(
            destination.as_ref().expect("tail command").meaning(),
            tex_state::meaning::ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::Space,
                ..
            })
        ));
        drop(processor);
        assert_eq!(
            fuel.burned(),
            3,
            "two run characters and the consumed tail each cost one charge"
        );
        let ownership_after = crate::command::command_ownership_counters();
        assert_eq!(
            ownership_after.resolved_writes - ownership_before.resolved_writes,
            1,
            "the borrowed characters never become CurrentCommand values"
        );
    });
}

#[test]
fn main_loop_character_run_lexes_a_resident_source_prefix_once() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"xab c"[..],
            ))
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(4).expect("source character-run fuel");
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        let first = processor
            .get_next()
            .expect("source line acquisition")
            .expect("first source token");
        assert_eq!(
            first.spelling().semantic_token(),
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            }
        );
        processor
            .command
            .profile_reset_input_cursor_mutation_counters();
        processor
            .command
            .profile_reset_input_source_context_counters();
        let ownership_before = crate::command::command_ownership_counters();
        let mut destination = None;
        let mut consumer = RecordingCharacterConsumer::default();
        assert_eq!(
            processor
                .main_loop_source_step_into(&mut destination, &mut consumer)
                .expect("source character run"),
            crate::DeliveryStatus::CharacterRun
        );
        assert_eq!(consumer.characters, "ab");
        assert_eq!(consumer.origins.len(), 2);
        assert_ne!(consumer.origins[0], consumer.origins[1]);
        assert!(destination.is_none());
        assert_eq!(
            processor.command.profile_resident_input_branch_counters(),
            (1, 1, 0, 0)
        );
        assert_eq!(
            processor.command.profile_input_source_context_counters(),
            (0, 0, 0, 1)
        );
        assert_eq!(
            processor.fuel.burned(),
            3,
            "the deferred space tail remains uncharged until its owner fetches it"
        );
        let ownership_after = crate::command::command_ownership_counters();
        assert_eq!(
            ownership_after.resolved_writes - ownership_before.resolved_writes,
            0
        );

        assert_eq!(
            processor
                .main_loop_source_step_into(&mut destination, &mut consumer)
                .expect("scalar source boundary"),
            crate::DeliveryStatus::CharacterRunBoundary
        );
        assert!(matches!(
            destination.as_ref().expect("space boundary").meaning(),
            tex_state::meaning::ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::Space,
                ..
            })
        ));
        drop(processor);
        assert_eq!(
            fuel.burned(),
            4,
            "the initial source character plus the run and tail each cost one charge"
        );
    });
}

#[test]
fn main_loop_source_step_settles_zero_prefix_without_reopening_source() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"xab "[..],
            ))
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(3).expect("source character-run fuel");
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        processor
            .get_next()
            .expect("source line acquisition")
            .expect("first source token");
        processor
            .command
            .profile_reset_input_source_context_counters();
        let mut consumer = RecordingCharacterConsumer {
            fallback_borrowed: true,
            ..RecordingCharacterConsumer::default()
        };
        let mut destination = None;
        assert_eq!(
            processor
                .main_loop_source_step_into(&mut destination, &mut consumer)
                .expect("source metric boundary"),
            crate::DeliveryStatus::CharacterRun
        );
        assert_eq!(consumer.characters, "a");
        assert_eq!(
            processor.command.profile_input_source_context_counters(),
            (0, 0, 0, 1)
        );
        drop(processor);
        assert_eq!(fuel.burned(), 2);
        assert!(destination.is_none());
    });
}

#[test]
fn main_loop_source_step_sends_utf8_boundary_to_scalar_tokenizer() {
    crate::test_harness::with_universe(|universe| {
        let mut command =
            CommandState::new(CommandProfile::unicode_extended(CommandDialect::Tex82));
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                "Aλ".as_bytes(),
            ))
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(3).expect("source character-run fuel");
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        assert_eq!(
            processor
                .get_next()
                .expect("source line acquisition")
                .expect("first source token")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter,
            }
        );
        let mut destination = None;
        let mut consumer = RecordingCharacterConsumer::default();
        assert_eq!(
            processor
                .main_loop_source_step_into(&mut destination, &mut consumer)
                .expect("UTF-8 scalar boundary"),
            crate::DeliveryStatus::CharacterRunBoundary
        );
        assert!(consumer.characters.is_empty());
        assert_eq!(
            destination
                .as_ref()
                .expect("decoded scalar command")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: 'λ',
                cat: Catcode::Other,
            }
        );
    });
}

#[test]
fn main_loop_source_step_sends_superscript_boundary_to_scalar_tokenizer() {
    crate::test_harness::with_universe(|universe| {
        universe
            .assign_code(
                tex_state::env::CodeTableKind::Catcode,
                '^',
                i64::from(Catcode::Superscript as u8),
                AssignmentScope::Global,
            )
            .expect("superscript catcode");
        let mut command = CommandState::default();
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"A^^41"[..],
            ))
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(5).expect("source character-run fuel");
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        assert_eq!(
            processor
                .get_next()
                .expect("source line acquisition")
                .expect("first source token")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter,
            }
        );
        let mut destination = None;
        let mut consumer = RecordingCharacterConsumer::default();
        assert_eq!(
            processor
                .main_loop_source_step_into(&mut destination, &mut consumer)
                .expect("superscript scalar boundary"),
            crate::DeliveryStatus::CharacterRunBoundary
        );
        assert!(consumer.characters.is_empty());
        assert_eq!(
            destination
                .as_ref()
                .expect("reduced scalar command")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter,
            }
        );
    });
}

#[test]
fn main_loop_source_step_uses_live_catcode_at_run_boundary() {
    crate::test_harness::with_universe(|universe| {
        universe
            .assign_code(
                tex_state::env::CodeTableKind::Catcode,
                'b',
                i64::from(Catcode::Space as u8),
                AssignmentScope::Global,
            )
            .expect("space catcode");
        let mut command = CommandState::default();
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"Ab"[..],
            ))
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::new(2).expect("source character-run fuel");
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        assert_eq!(
            processor
                .get_next()
                .expect("source line acquisition")
                .expect("first source token")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter,
            }
        );
        let mut destination = None;
        let mut consumer = RecordingCharacterConsumer::default();
        assert_eq!(
            processor
                .main_loop_source_step_into(&mut destination, &mut consumer)
                .expect("catcode scalar boundary"),
            crate::DeliveryStatus::CharacterRunBoundary
        );
        assert!(consumer.characters.is_empty());
        assert_eq!(
            destination
                .as_ref()
                .expect("space-catcode command")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            }
        );
    });
}

#[test]
fn main_loop_character_run_charges_resident_macro_body_once_per_character() {
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
        let macro_name = universe.intern("runbody").expect("macro name").symbol();
        let body = universe
            .command_context()
            .expect("macro body context")
            .admit_macro_body(definition)
            .expect("resident macro body")
            .2;
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
        let mut destination = None;
        let mut consumer = RecordingCharacterConsumer::default();
        assert_eq!(
            processor
                .main_loop_source_step_into(&mut destination, &mut consumer)
                .expect("macro body character run"),
            crate::DeliveryStatus::CharacterRunBoundary
        );
        assert_eq!(consumer.characters, "Abc");
        assert!(matches!(
            destination.as_ref().expect("macro body tail").meaning(),
            tex_state::meaning::ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::Space,
                ..
            })
        ));
        drop(processor);
        assert_eq!(fuel.burned(), 4);
    });
}

#[test]
fn main_loop_character_run_charges_macro_argument_chars_once() {
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
        let matching = command.scratch.begin_macro_match().expect("macro match");
        let mut writer = command
            .scratch
            .begin_argument_writer(&matching)
            .expect("macro argument writer");
        for word in traced {
            command
                .scratch
                .append_argument_token(
                    &mut writer,
                    crate::token_collector::ClassifiedToken::from_word(word, None),
                    true,
                )
                .expect("macro argument word");
        }
        command
            .scratch
            .publish_argument(writer)
            .expect("macro argument range");
        let argument_set = command
            .scratch
            .commit_macro_match(matching)
            .expect("macro argument set");
        let macro_name = universe.intern("runargument").expect("macro name").symbol();
        let body_definition = universe
            .allocate_definition(
                &[TokenWord::pack(Token::Param(1))],
                &[TokenWord::pack(Token::Param(1))],
            )
            .expect("macro body definition");
        let body = universe
            .command_context()
            .expect("macro body context")
            .admit_macro_body(body_definition)
            .expect("resident macro body")
            .2;
        command.push_macro_activation(macro_name, body, Some(argument_set), OriginId::UNKNOWN);

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
        let mut destination = None;
        let mut consumer = RecordingCharacterConsumer::default();
        assert_eq!(
            processor
                .main_loop_source_step_into(&mut destination, &mut consumer)
                .expect("macro argument character run"),
            crate::DeliveryStatus::CharacterRun
        );
        assert_eq!(consumer.characters, "Abc");
        assert!(destination.is_none());
        drop(processor);
        assert_eq!(fuel.burned(), 3);
    });
}

#[test]
fn main_loop_character_run_rollback_keeps_fuel_monotonic() {
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
        let snapshot = command.snapshot(universe).expect("character-run snapshot");
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
            let mut destination = None;
            let mut consumer = RecordingCharacterConsumer::default();
            assert_eq!(
                processor
                    .main_loop_source_step_into(&mut destination, &mut consumer)
                    .expect("character run retry"),
                crate::DeliveryStatus::CharacterRun
            );
            assert_eq!(consumer.characters, "Ab");
            drop(processor);
            assert_eq!(fuel.burned(), expected_burned);

            if expected_burned == 2 {
                drop(context);
                command
                    .rollback(&snapshot, universe)
                    .expect("rollback restores the character row");
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
        let symbol = universe.intern("m").expect("macro name");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
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

        let suppressed = processor
            .get_x_token()
            .expect("suppressed delivery")
            .expect("suppressed command");
        assert_eq!(suppressed.spelling().semantic_token(), macro_token);
        assert_eq!(suppressed.meaning(), Meaning::Relax);
        assert_eq!(processor.command.delivery_mode, persistent_mode);
        assert_eq!(
            processor
                .get_x_token()
                .expect("second delivery")
                .expect("replacement")
                .spelling()
                .semantic_token(),
            replacement
        );
        assert_eq!(processor.command.delivery_mode, persistent_mode);
    });
}
