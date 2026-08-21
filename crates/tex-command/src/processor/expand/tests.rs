use tex_state::env::AssignmentScope;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags, MeaningWord};
use tex_state::token::{Catcode, Token, TokenWord};

use crate::{CommandHostCapabilities, CommandState};

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
