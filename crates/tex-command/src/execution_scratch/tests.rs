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
            ArgumentRunAdmission::with_commands(&context, None, None, 1).completing_undelimited()
        };

        let mut arena = crate::input::ArgumentArena::<()>::default();
        let matching = arena.begin_match().expect("argument match");
        let cursor = || arena.begin_argument(&matching);

        // TeX82 §392: a sole ordinary token is the whole argument.
        let mut run = admission();
        let mut writer = cursor();
        assert!(run.admits(letter('x'), &mut writer));
        assert!(
            !run.admits(letter('y'), &mut writer),
            "the argument is already complete"
        );

        // §394: a group completes at its own closing brace, and interior
        // groups balance within it.
        let mut run = admission();
        let mut writer = cursor();
        for word in [open, open, letter('a'), close, letter('b'), close] {
            assert!(run.admits(word, &mut writer));
        }
        assert!(
            !run.admits(letter('c'), &mut writer),
            "the argument is already complete"
        );
        assert_eq!(run.brace_delta(), 0);

        // §393's skipped leading space and §395's stray closing brace stay
        // with the scalar matcher.
        assert!(!admission().admits(char_word(' ', Catcode::Space), &mut cursor()));
        assert!(!admission().admits(close, &mut cursor()));

        // An interior run resumes inside the group and still completes it.
        let mut writer = cursor();
        let mut run = admission();
        assert!(run.admits(open, &mut writer));
        let mut run = admission();
        assert!(run.admits(char_word(' ', Catcode::Space), &mut writer));
        assert!(run.admits(close, &mut writer));
        assert!(!run.admits(letter('d'), &mut writer));
    });
}
