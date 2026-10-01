use super::*;
use tex_state::token::{Catcode, Token, TokenWord};

#[test]
fn undelimited_admission_completes_on_its_sole_token_or_closing_brace() {
    crate::test_harness::with_universe(|universe| {
        let context = universe.command_context().expect("command context");
        let char_word = |ch, cat| TokenWord::pack(Token::Char { ch, cat });
        let letter = |ch| char_word(ch, Catcode::Letter);
        let open = char_word('{', Catcode::BeginGroup);
        let close = char_word('}', Catcode::EndGroup);
        let admission = || {
            ArgumentRunAdmission::with_commands(&context, None, None, 0, 1).completing_undelimited()
        };

        // TeX82 §392: a sole ordinary token is the whole argument.
        let mut run = admission();
        assert!(run.admits(letter('x')));
        assert!(!run.admits(letter('y')), "the argument is already complete");

        // §394: a group completes at its own closing brace, and interior
        // groups balance within it.
        let mut run = admission();
        for word in [open, open, letter('a'), close, letter('b'), close] {
            assert!(run.admits(word));
        }
        assert!(!run.admits(letter('c')), "the argument is already complete");
        assert_eq!(run.brace_delta(), 0);

        // §393's skipped leading space and §395's stray closing brace stay
        // with the scalar matcher.
        assert!(!admission().admits(char_word(' ', Catcode::Space)));
        assert!(!admission().admits(close));

        // An interior run resumes inside the group and still completes it.
        let mut run = ArgumentRunAdmission::with_commands(&context, None, None, 1, 1)
            .completing_undelimited();
        assert!(run.admits(char_word(' ', Catcode::Space)));
        assert!(run.admits(close));
        assert!(!run.admits(letter('d')));
    });
}
