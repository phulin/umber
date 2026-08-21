use tex_state::token::{Catcode, Token};

use super::WriteStreamSelector;
use crate::{CommandHostCapabilities, CommandState};

fn other(ch: char) -> Token {
    Token::Char {
        ch,
        cat: Catcode::Other,
    }
}

fn scan_write_stream(tokens: impl IntoIterator<Item = Token>) -> WriteStreamSelector {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, tokens);
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        crate::test_harness::processor(
            &mut command,
            universe,
            &mut capabilities,
            &mut diagnostic_effects,
        )
        .scan_write_stream()
        .expect("write stream")
    })
}

#[test]
fn write_stream_scan_keeps_texs_two_out_of_range_classes_distinct() {
    assert_eq!(
        scan_write_stream([other('-'), other('1')]),
        WriteStreamSelector::Negative
    );
    assert_eq!(
        scan_write_stream([other('1'), other('6')]),
        WriteStreamSelector::AboveRange
    );
    assert_eq!(
        scan_write_stream([other('7')]),
        WriteStreamSelector::Stream(7)
    );
}

#[test]
fn normalized_write_stream_numbers_match_texs_reserved_slots() {
    assert_eq!(WriteStreamSelector::Negative.normalized_number(), 17);
    assert_eq!(WriteStreamSelector::AboveRange.normalized_number(), 16);
    assert_eq!(WriteStreamSelector::Stream(4).normalized_number(), 4);
}
