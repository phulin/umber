use tex_state::meaning::Meaning;
use tex_state::token::{Catcode, Token};

use crate::{CommandHostCapabilities, CommandState};

fn other(ch: char) -> Token {
    Token::Char {
        ch,
        cat: if ch == ' ' {
            Catcode::Space
        } else {
            Catcode::Other
        },
    }
}

#[test]
fn integer_scanner_preserves_signs_and_backs_up_the_nonspace_terminator() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [other('-'), other('4'), other('2'), other('X')],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor =
            crate::test_harness::processor(&mut command, universe, &mut capabilities);
        assert_eq!(processor.scan_integer().expect("integer").value, -42);
        assert_eq!(
            processor
                .get_x_token()
                .expect("terminator delivery")
                .expect("terminator")
                .meaning(),
            Meaning::CharToken {
                ch: 'X',
                cat: Catcode::Other,
            }
        );
    });
}

#[test]
fn optional_equals_consumes_spaces_but_leaves_the_following_operand() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [other(' '), other('='), other('7')]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor =
            crate::test_harness::processor(&mut command, universe, &mut capabilities);
        assert!(processor.scan_optional_equals().expect("equals").value);
        assert_eq!(processor.scan_integer().expect("operand").value, 7);
    });
}
