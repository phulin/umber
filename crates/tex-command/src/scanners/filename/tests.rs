use tex_state::token::{Catcode, Token};

use crate::{CommandHostCapabilities, CommandState};

#[test]
fn filename_scan_stops_at_space_and_keeps_area_name_and_extension() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let tokens = "dir/job.tex "
            .chars()
            .map(|ch| Token::Char {
                ch,
                cat: if ch == ' ' {
                    Catcode::Space
                } else if ch.is_ascii_alphabetic() {
                    Catcode::Letter
                } else {
                    Catcode::Other
                },
            })
            .collect::<Vec<_>>();
        crate::test_harness::push(&mut command, tokens);
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
        let scalar_result = match processor.scan_file_name() {
            Ok(value) => value,
            Err(error) => panic!("scalar scan failed: {error:?}"),
        };
        let scanned = scalar_result;

        assert_eq!(scanned.packed(), "dir/job.tex");
        assert_eq!(scanned.components.area, "dir/");
        assert_eq!(scanned.components.name, "job");
        assert_eq!(scanned.components.extension, ".tex");
    });
}

#[test]
fn filename_character_aliases_preserve_the_backed_up_terminator_spelling() {
    #[derive(Default)]
    struct Observer;
    impl crate::CommandObserver for Observer {
        fn committed(&mut self, _: crate::CommandObservation) {}
    }
    for observed in [false, true] {
        crate::test_harness::with_universe(|universe| {
            let alias = universe.intern("name_character").expect("alias");
            universe
                .assign_meaning(
                    alias,
                    tex_state::MeaningWord::from_static(tex_state::meaning::Meaning::CharToken {
                        ch: 'o',
                        cat: Catcode::Other,
                    }),
                    tex_state::AssignmentScope::Global,
                )
                .expect("character meaning");
            let terminator = universe.intern("name_terminator").expect("terminator");
            universe
                .assign_meaning(
                    terminator,
                    tex_state::MeaningWord::from_static(tex_state::meaning::Meaning::Relax),
                    tex_state::AssignmentScope::Global,
                )
                .expect("terminator meaning");
            let other = |ch| Token::Char {
                ch,
                cat: Catcode::Other,
            };
            let mut command = CommandState::default();
            crate::test_harness::push(
                &mut command,
                [
                    other('j'),
                    Token::Cs(alias.symbol()),
                    other('b'),
                    other('.'),
                    other('t'),
                    other('e'),
                    other('x'),
                    Token::Cs(terminator.symbol()),
                    other('Z'),
                ],
            );
            let mut capabilities = CommandHostCapabilities::default();
            let mut fuel = crate::CommandFuelLedger::default();
            let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut context = universe.command_context().expect("context");
            let mut observer = Observer;
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
            assert_eq!(
                processor.scan_file_name().expect("filename").packed(),
                "job.tex"
            );
            assert_eq!(
                crate::test_harness::expect_expanded_command(&mut processor)
                    .spelling()
                    .semantic_token(),
                Token::Cs(terminator.symbol())
            );
            assert_eq!(
                crate::test_harness::expect_expanded_command(&mut processor)
                    .spelling()
                    .semantic_token(),
                other('Z')
            );
        });
    }
}
