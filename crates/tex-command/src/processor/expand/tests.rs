use tex_state::env::AssignmentScope;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags, MeaningWord};
use tex_state::token::{Catcode, Token, TokenWord};

use crate::{CommandHostCapabilities, CommandProfile, CommandState};

fn install_static<G>(universe: &mut tex_state::Universe<G>, name: &str, meaning: Meaning) -> Token {
    let symbol = universe.intern(name).expect("intern primitive");
    universe
        .assign_meaning(
            symbol,
            MeaningWord::from_static(meaning),
            AssignmentScope::Global,
        )
        .expect("install primitive");
    Token::Cs(symbol.symbol())
}

#[test]
fn parameterless_macro_expands_from_a_generation_typed_definition() {
    crate::test_harness::with_universe(|universe| {
        let replacement = Token::Char {
            ch: 'A',
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
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(&mut command, [Token::Cs(symbol.symbol())]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut processor = crate::test_harness::processor(
            &mut command,
            universe,
            &mut capabilities,
            &mut diagnostic_effects,
        );

        let expanded = processor
            .get_x_token()
            .expect("macro expansion")
            .expect("replacement command");
        assert_eq!(expanded.spelling().semantic_token(), replacement);
        assert_eq!(
            expanded.meaning(),
            Meaning::CharToken {
                ch: 'A',
                cat: Catcode::Letter,
            }
        );
        assert!(processor.get_x_token().expect("end").is_none());
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
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut processor = crate::test_harness::processor(
            &mut command,
            universe,
            &mut capabilities,
            &mut diagnostic_effects,
        );

        let suppressed = processor
            .get_x_token()
            .expect("suppressed delivery")
            .expect("suppressed command");
        assert_eq!(suppressed.spelling().semantic_token(), macro_token);
        assert_eq!(suppressed.meaning(), Meaning::Relax);
        assert_eq!(
            processor
                .get_x_token()
                .expect("second delivery")
                .expect("replacement")
                .spelling()
                .semantic_token(),
            replacement
        );
    });
}

#[test]
fn protected_replay_delivery_writes_the_terminal_macro_into_its_caller_slot() {
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
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut processor = crate::test_harness::processor(
            &mut command,
            universe,
            &mut capabilities,
            &mut diagnostic_effects,
        );

        let mut destination = None;
        assert_eq!(
            processor
                .get_x_or_protected_with_replay_completion_into(&mut destination)
                .expect("protected delivery"),
            super::DeliveryStatus::Command
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
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut processor = crate::test_harness::processor(
            &mut command,
            universe,
            &mut capabilities,
            &mut diagnostic_effects,
        );

        let expanded = processor
            .get_x_token()
            .expect("csname expansion")
            .expect("named control sequence");
        assert_eq!(
            expanded.spelling().semantic_token(),
            Token::Cs(latent.symbol())
        );
        assert_eq!(expanded.meaning(), Meaning::Relax);
        assert!(processor.get_x_token().expect("end").is_none());
    });
}
