use tex_state::token::{Catcode, Token};

use super::HyphenationDataKind;
use crate::{CommandHostCapabilities, CommandState};

#[test]
fn pattern_scan_keeps_letters_and_interleaved_numeric_weights() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let tokens = "{a1bc3}"
            .chars()
            .map(|ch| Token::Char {
                ch,
                cat: match ch {
                    '{' => Catcode::BeginGroup,
                    '}' => Catcode::EndGroup,
                    'a'..='z' => Catcode::Letter,
                    _ => Catcode::Other,
                },
            })
            .collect::<Vec<_>>();
        crate::test_harness::push(&mut command, tokens);
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let scanned = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut diagnostic_effects,
        )
        .scan_hyphenation_data(HyphenationDataKind::Patterns)
        .expect("patterns");

        let pattern = scanned.patterns.first().expect("one pattern");
        assert_eq!(pattern.letters, ['a', 'b', 'c']);
        assert_eq!(pattern.values, [0, 1, 0, 3]);
    });
}
