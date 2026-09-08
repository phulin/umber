use super::*;

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
        let mut consumer = RecordingCharacterConsumer {
            stop_after: Some(17),
            ..Default::default()
        };
        let mut destination = None;
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut effects,
        );
        assert_eq!(
            processor
                .main_loop_source_step_into(&mut destination, &mut consumer)
                .expect("resident admission"),
            crate::DeliveryStatus::CharacterRun
        );
        assert!(destination.is_none());
        assert_eq!(consumer.characters.len(), 17);
        assert_eq!(
            processor
                .main_loop_source_step_into(&mut destination, &mut consumer)
                .expect("resident admission"),
            crate::DeliveryStatus::CharacterRunBoundary
        );
        assert_eq!(consumer.characters, "x".repeat(4096));
        assert!(matches!(
            destination.expect("space boundary").meaning(),
            tex_state::meaning::ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::Space,
                ..
            })
        ));
        drop(processor);
        assert_eq!(fuel.burned(), 4097);
        assert_eq!(
            command
                .roots
                .input
                .levels
                .cursor_mutations
                .typed_top_accesses,
            2,
            "one reader selection per admission episode"
        );
        assert_eq!(command.stored_token_advance_counters.packed_loads, 4097);
        assert_eq!(command.stored_token_advance_counters.command_writes, 1);
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
            let mut consumer = RecordingCharacterConsumer::default();
            let mut destination = None;
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut effects,
            );
            assert!(
                processor
                    .main_loop_source_step_into(&mut destination, &mut consumer)
                    .is_err()
            );
            assert_eq!(consumer.characters, &"abcdefg"[..limit as usize]);
            assert!(destination.is_none());
            drop(processor);
            assert_eq!(fuel.burned(), limit);
            // Scalar delivery advances input before attempting the charge.
            assert_eq!(
                command.stored_token_advance_counters.cursor_advances,
                limit + 1
            );
            assert_eq!(
                command
                    .roots
                    .input
                    .levels
                    .cursor_mutations
                    .typed_top_accesses,
                1
            );
        });
    }
}
