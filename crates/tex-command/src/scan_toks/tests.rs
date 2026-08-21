use tex_state::token::{Catcode, Token};

use super::ScanToksMode;
use crate::{CommandHostCapabilities, CommandState};

fn token(ch: char, cat: Catcode) -> Token {
    Token::Char { ch, cat }
}

#[test]
fn balanced_collection_freezes_nested_tokens_in_the_attempt_arena() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
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
        let mut processor =
            crate::test_harness::processor(&mut command, universe, &mut capabilities);
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
        assert_eq!(
            processor
                .get_x_token()
                .expect("following delivery")
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
        let mut processor =
            crate::test_harness::processor(&mut command, universe, &mut capabilities);
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
