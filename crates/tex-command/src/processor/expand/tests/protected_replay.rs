//! Noexpand, protected replay, and terminal classification.

use super::*;

#[test]
fn protected_replay_delivery_writes_the_terminal_macro_into_its_caller_slot() {
    for supplied in [false, true] {
        crate::test_harness::with_universe(|universe| {
            let replacement = Token::Char {
                ch: 'P',
                cat: Catcode::Letter,
            };
            let definition = universe
                .allocate_definition(&[], &[TokenWord::pack(replacement)])
                .expect("definition");
            let symbol = universe.intern("protected").expect("macro name");
            universe
                .assign_meaning(
                    symbol,
                    MeaningWord::macro_definition(MeaningFlags::PROTECTED, definition),
                    AssignmentScope::Global,
                )
                .expect("macro meaning");
            let macro_token = Token::Cs(symbol.symbol());
            let mut command = CommandState::new(CommandProfile::ETEX26);
            crate::test_harness::push(&mut command, [macro_token]);
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

            let mut destination = None;
            if supplied {
                assert_eq!(
                    processor
                        .raw_next(&mut destination)
                        .expect("supplied raw command"),
                    super::super::DeliveryStatus::Command
                );
            }
            assert_eq!(
                processor
                    .get_x_or_protected_with_replay_completion_into(&mut destination)
                    .expect("protected delivery"),
                super::super::DeliveryStatus::Command
            );
            let delivered = destination.expect("caller destination");
            assert_eq!(delivered.spelling().semantic_token(), macro_token);
            assert!(matches!(
                delivered.meaning(),
                tex_state::meaning::ResolvedMeaning::Macro { flags, .. }
                    if flags.contains(MeaningFlags::PROTECTED)
            ));
        });
    }
}

#[test]
fn protected_delivery_restarts_terminal_classification_after_macro_expansion() {
    crate::test_harness::with_universe(|universe| {
        let replacement = Token::Char {
            ch: 'P',
            cat: Catcode::Letter,
        };
        let protected_definition = universe
            .allocate_definition(&[], &[TokenWord::pack(replacement)])
            .expect("protected definition");
        let protected_symbol = universe.intern("protected-nested").expect("protected name");
        universe
            .assign_meaning(
                protected_symbol,
                MeaningWord::macro_definition(MeaningFlags::PROTECTED, protected_definition),
                AssignmentScope::Global,
            )
            .expect("protected meaning");
        let protected_token = Token::Cs(protected_symbol.symbol());

        let outer_definition = universe
            .allocate_definition(&[], &[TokenWord::pack(protected_token)])
            .expect("outer definition");
        let outer_symbol = universe.intern("outer-nested").expect("outer name");
        universe
            .assign_meaning(
                outer_symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, outer_definition),
                AssignmentScope::Global,
            )
            .expect("outer meaning");
        let outer_token = Token::Cs(outer_symbol.symbol());

        let mut command = CommandState::new(CommandProfile::ETEX26);
        crate::test_harness::push(&mut command, [outer_token]);
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

        let mut destination = None;
        assert_eq!(
            processor
                .get_x_or_protected_with_replay_completion_into(&mut destination)
                .expect("nested protected delivery"),
            super::super::DeliveryStatus::Command
        );
        let delivered = destination.expect("nested protected destination");
        assert_eq!(delivered.spelling().semantic_token(), protected_token);
        assert!(matches!(
            delivered.meaning(),
            tex_state::meaning::ResolvedMeaning::Macro { flags, .. }
                if flags.contains(MeaningFlags::PROTECTED)
        ));
    });
}

#[test]
fn protected_delivery_does_not_observe_macro_expanded_relax_as_terminal() {
    crate::test_harness::with_universe(|universe| {
        let relax = install_static(universe, "relax-protected", Meaning::Relax);
        let definition = universe
            .allocate_definition(&[], &[TokenWord::pack(relax)])
            .expect("relax definition");
        let symbol = universe.intern("relax-wrapper").expect("wrapper name");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
                AssignmentScope::Global,
            )
            .expect("wrapper meaning");
        let wrapper = Token::Cs(symbol.symbol());

        let mut command = CommandState::new(CommandProfile::ETEX26);
        crate::test_harness::push(&mut command, [wrapper]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut observer = RecordingObserver::default();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        )
        .with_observer(&mut observer);

        let mut destination = None;
        assert_eq!(
            processor
                .get_x_or_protected_with_replay_completion_into(&mut destination)
                .expect("macro-wrapped relax delivery"),
            super::super::DeliveryStatus::Command
        );
        let delivered = destination.expect("relax destination");
        assert_eq!(delivered.spelling().semantic_token(), relax);
        assert_eq!(delivered.meaning(), Meaning::Relax);
        drop(processor);

        let expanded = observer
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Command(record)
                    if record.boundary == CommandDeliveryBoundary::Expanded =>
                {
                    Some(record)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            expanded.is_empty(),
            "protected delivery observed terminal expansion: {expanded:?}"
        );
    });
}

#[test]
fn csname_relaxes_an_already_interned_undefined_name() {
    crate::test_harness::with_universe(|universe| {
        let csname = install_static(
            universe,
            "csname",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::CsName),
        );
        let endcsname = install_static(
            universe,
            "endcsname",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName),
        );
        let latent = universe.intern("latent").expect("pre-intern name");
        let mut input = vec![csname];
        input.extend("latent".chars().map(|ch| Token::Char {
            ch,
            cat: Catcode::Letter,
        }));
        input.push(endcsname);
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, input);
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

        let expanded = crate::test_harness::expect_expanded_command(&mut processor);
        assert_eq!(
            expanded.spelling().semantic_token(),
            Token::Cs(latent.symbol())
        );
        assert_eq!(expanded.meaning(), Meaning::Relax);
        {
            let mut destination = None;
            assert_eq!(
                processor.get_x_token_into(&mut destination).expect("end"),
                crate::DeliveryStatus::End
            );
            assert!(destination.is_none());
        };
    });
}

#[test]
fn pdf_insert_height_queries_live_state_and_distinguishes_missing_from_zero() {
    crate::test_harness::with_universe(|universe| {
        let pdf_insert_height = install_static(
            universe,
            "pdfinsertht",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfInsertHeight),
        );
        let class = [Token::Char {
            ch: '7',
            cat: Catcode::Other,
        }];

        let mut missing = CommandState::new(CommandProfile::PDFTEX14029);
        crate::test_harness::push(
            &mut missing,
            std::iter::once(pdf_insert_height).chain(class),
        );
        assert_eq!(collect_expanded_characters(universe, &mut missing), "0pt");

        universe
            .command_context()
            .expect("command context")
            .upsert_page_insertion(tex_state::page::PageInsertion::new(
                7,
                tex_state::scaled::Scaled::from_raw(0),
            ));
        let mut present = CommandState::new(CommandProfile::PDFTEX14029);
        crate::test_harness::push(
            &mut present,
            std::iter::once(pdf_insert_height).chain(class),
        );
        assert_eq!(collect_expanded_characters(universe, &mut present), "0.0pt");
    });
}
