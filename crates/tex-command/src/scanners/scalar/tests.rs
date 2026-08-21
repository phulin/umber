use tex_state::glue::{GlueSpec, Order};
use tex_state::meaning::Meaning;
use tex_state::scaled::Scaled;
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

fn letter(ch: char) -> Token {
    Token::Char {
        ch,
        cat: Catcode::Letter,
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
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut processor = crate::test_harness::processor(
            &mut command,
            universe,
            &mut capabilities,
            &mut diagnostic_effects,
        );
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
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut processor = crate::test_harness::processor(
            &mut command,
            universe,
            &mut capabilities,
            &mut diagnostic_effects,
        );
        assert!(processor.scan_optional_equals().expect("equals").value);
        assert_eq!(processor.scan_integer().expect("operand").value, 7);
    });
}

#[test]
fn failed_keyword_replays_the_matched_prefix_before_the_offender() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [letter('e'), letter('x'), other('!')]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut processor = crate::test_harness::processor(
            &mut command,
            universe,
            &mut capabilities,
            &mut diagnostic_effects,
        );

        assert!(!processor.scan_keyword("em").expect("keyword").value);
        let replayed = (0..3)
            .map(|_| {
                processor
                    .get_x_token()
                    .expect("replayed delivery")
                    .expect("replayed token")
                    .spelling()
                    .semantic_token()
            })
            .collect::<Vec<_>>();
        assert_eq!(replayed, [letter('e'), letter('x'), other('!')]);
    });
}

#[test]
fn dimension_scanner_preserves_fractional_points_and_following_input() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            [
                other('1'),
                other('.'),
                other('5'),
                letter('p'),
                letter('t'),
                other('X'),
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut processor = crate::test_harness::processor(
            &mut command,
            universe,
            &mut capabilities,
            &mut diagnostic_effects,
        );

        assert_eq!(
            processor.scan_dimension().expect("dimension").value,
            Scaled::from_raw(Scaled::UNITY + Scaled::UNITY / 2)
        );
        assert_eq!(
            processor
                .get_x_token()
                .expect("terminator")
                .expect("terminator token")
                .spelling()
                .semantic_token(),
            other('X')
        );
    });
}

#[test]
fn glue_scanner_preserves_width_stretch_shrink_and_orders() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            "1pt plus 2fil minus 3pt!".chars().map(|ch| {
                if ch.is_ascii_alphabetic() {
                    letter(ch)
                } else {
                    other(ch)
                }
            }),
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut processor = crate::test_harness::processor(
            &mut command,
            universe,
            &mut capabilities,
            &mut diagnostic_effects,
        );

        assert_eq!(
            processor.scan_glue(false).expect("glue").value,
            GlueSpec {
                width: Scaled::from_raw(Scaled::UNITY),
                stretch: Scaled::from_raw(2 * Scaled::UNITY),
                stretch_order: Order::Fil,
                shrink: Scaled::from_raw(3 * Scaled::UNITY),
                shrink_order: Order::Normal,
            }
        );
        assert_eq!(
            processor
                .get_x_token()
                .expect("terminator")
                .expect("terminator token")
                .spelling()
                .semantic_token(),
            other('!')
        );
    });
}
