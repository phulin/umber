use tex_state::env::AssignmentScope;
use tex_state::meaning::{Meaning, MeaningWord, UnexpandablePrimitive};
use tex_state::token::{Catcode, Token};

use crate::{CommandHostCapabilities, CommandProfile, CommandState};

fn other(ch: char) -> Token {
    Token::Char {
        ch,
        cat: Catcode::Other,
    }
}

#[test]
fn numexpr_honors_precedence_and_leaves_its_relax_terminator_consumed() {
    crate::test_harness::with_universe(|universe| {
        let numexpr = universe.intern("numexpr").expect("numexpr");
        universe
            .assign_meaning(
                numexpr,
                MeaningWord::from_static(Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::NumExpr,
                )),
                AssignmentScope::Global,
            )
            .expect("numexpr meaning");
        let relax = universe.intern("relax").expect("relax");
        universe
            .assign_meaning(
                relax,
                MeaningWord::from_static(Meaning::Relax),
                AssignmentScope::Global,
            )
            .expect("relax meaning");
        let mut command = CommandState::new(CommandProfile::ETEX26);
        crate::test_harness::push(
            &mut command,
            [
                Token::Cs(numexpr.symbol()),
                other('2'),
                other('+'),
                other('3'),
                other('*'),
                other('4'),
                Token::Cs(relax.symbol()),
                other('X'),
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor =
            crate::test_harness::processor(&mut command, universe, &mut capabilities);
        assert_eq!(processor.scan_integer().expect("expression").value, 14);
        assert_eq!(
            processor
                .get_x_token()
                .expect("following")
                .expect("following token")
                .spelling()
                .semantic_token(),
            other('X')
        );
    });
}
