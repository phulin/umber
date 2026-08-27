use tex_state::env::AssignmentScope;
use tex_state::meaning::{MeaningFlags, MeaningWord};
use tex_state::token::{Catcode, Token, TokenWord};

use super::ScanToksMode;
use crate::{CommandHostCapabilities, CommandState, DeliveryStatus};

fn token(ch: char, cat: Catcode) -> Token {
    Token::Char { ch, cat }
}

#[test]
fn balanced_collection_freezes_nested_tokens_in_the_attempt_arena() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                token('{', Catcode::BeginGroup),
                token('a', Catcode::Letter),
                token('{', Catcode::BeginGroup),
                token('b', Catcode::Letter),
                token('}', Catcode::EndGroup),
                token('}', Catcode::EndGroup),
                token('X', Catcode::Letter),
            ],
        );
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
        let scanned = processor
            .scan_toks(ScanToksMode::General { expanded: false })
            .expect("balanced scan");
        assert_eq!(
            processor
                .command
                .attempt_token_words(scanned.replacement_text)
                .expect("attempt words")
                .iter()
                .map(|word| word.semantic_token())
                .collect::<Vec<_>>(),
            [
                token('a', Catcode::Letter),
                token('{', Catcode::BeginGroup),
                token('b', Catcode::Letter),
                token('}', Catcode::EndGroup),
            ]
        );
        let mut destination = None;
        assert_eq!(
            processor
                .get_x_token_into(&mut destination)
                .expect("following delivery"),
            DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .take()
                .expect("following token")
                .spelling()
                .semantic_token(),
            token('X', Catcode::Letter)
        );
    });
}

#[test]
fn expanded_collection_keeps_its_builder_live_across_nested_macro_retirement() {
    crate::test_harness::with_universe(|universe| {
        let replacement = token('a', Catcode::Letter);
        let definition = universe
            .allocate_definition(&[], &[TokenWord::pack(replacement)])
            .expect("macro definition");
        let symbol = universe.intern("scanmacro").expect("macro name");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
                AssignmentScope::Global,
            )
            .expect("macro meaning");
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                token('{', Catcode::BeginGroup),
                Token::Cs(symbol.symbol()),
                token('}', Catcode::EndGroup),
                token('X', Catcode::Letter),
            ],
        );
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

        let scanned = processor
            .scan_toks(ScanToksMode::General { expanded: true })
            .expect("expanded scan");
        assert_eq!(
            processor
                .command
                .attempt_token_words(scanned.replacement_text)
                .expect("replacement survives nested retirement")
                .iter()
                .map(|word| word.semantic_token())
                .collect::<Vec<_>>(),
            [replacement]
        );
        let mut destination = None;
        assert_eq!(
            processor
                .get_x_token_into(&mut destination)
                .expect("following delivery"),
            DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .take()
                .expect("following token")
                .spelling()
                .semantic_token(),
            token('X', Catcode::Letter)
        );
    });
}

#[test]
fn macro_definition_scan_keeps_parameter_and_replacement_lists_separate() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                token('#', Catcode::Parameter),
                token('1', Catcode::Other),
                token('{', Catcode::BeginGroup),
                token('#', Catcode::Parameter),
                token('1', Catcode::Other),
                token('}', Catcode::EndGroup),
            ],
        );
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
        let scanned = processor
            .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
            .expect("definition scan");
        assert!(!scanned.malformed_parameter);
        assert!(
            !processor
                .command
                .attempt_token_words(scanned.parameter_text)
                .expect("parameter words")
                .is_empty()
        );
        assert_eq!(
            processor
                .command
                .attempt_token_words(scanned.replacement_text)
                .expect("replacement words")
                .last()
                .map(|word| word.semantic_token()),
            Some(Token::Param(1))
        );
    });
}
