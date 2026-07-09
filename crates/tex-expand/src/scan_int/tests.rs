use tex_lex::{InputStack, MemoryInput};
use tex_state::env::banks::IntParam;
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::provenance::{OriginRecord, SourceOrigin};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, Token};
use tex_state::{ExpansionState, Universe};

use crate::scan_int::{IntegerDiagnostic, ScanIntError, scan_int};

fn scan(input: &str) -> (i32, Option<IntegerDiagnostic>, Option<Token>) {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new(input));
    let scanned = scan_int(&mut input, &mut stores).expect("integer scan should succeed");
    let next = input
        .next_token(&mut stores)
        .expect("remaining token should lex");
    (scanned.value(), scanned.diagnostic(), next)
}

fn scan_with_stores(input_text: &str, stores: &mut impl ExpansionState) -> (i32, Option<Token>) {
    let mut input = InputStack::new(MemoryInput::new(input_text));
    let scanned = scan_int(&mut input, stores).expect("integer scan should succeed");
    let next = input
        .next_token(stores)
        .expect("remaining token should lex");
    (scanned.value(), next)
}

fn char_token(ch: char, cat: Catcode) -> Token {
    Token::Char { ch, cat }
}

#[test]
fn scans_repeated_signs_with_intervening_spaces() {
    let (value, diagnostic, next) = scan(" - + - 123x");

    assert_eq!(value, 123);
    assert_eq!(diagnostic, None);
    assert_eq!(next, Some(char_token('x', Catcode::Letter)));
}

#[test]
fn scans_decimal_octal_and_hex_constants() {
    assert_eq!(scan("123x").0, 123);
    assert_eq!(scan("'177x").0, 127);
    assert_eq!(scan("\"7F x").0, 127);
}

#[test]
fn scans_backtick_character_and_control_sequence_constants() {
    let (value, _diagnostic, next) = scan("`A x");
    assert_eq!(value, 65);
    assert_eq!(next, Some(char_token('x', Catcode::Letter)));

    let mut stores = Universe::new();
    let alpha = stores.intern("alpha");
    stores.set_meaning(alpha, Meaning::Relax);
    let (value, next) = scan_with_stores("`\\alpha x", &mut stores);
    assert_eq!(value, i32::from(b'a'));
    assert_eq!(next, Some(char_token('x', Catcode::Letter)));
}

#[test]
fn consumes_at_most_one_trailing_space() {
    let (value, _diagnostic, next) = scan("12  x");

    assert_eq!(value, 12);
    assert_eq!(next, Some(char_token('x', Catcode::Letter)));
}

#[test]
fn leaves_non_space_terminator_available() {
    let (value, _diagnostic, next) = scan("12+x");

    assert_eq!(value, 12);
    assert_eq!(next, Some(char_token('+', Catcode::Other)));
}

#[test]
fn scans_supported_internal_integers() {
    let mut stores = Universe::new();
    let count = stores.intern("count");
    let dimen = stores.intern("dimen");
    let endlinechar = stores.intern("endlinechar");
    stores.set_meaning(
        count,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Count),
    );
    stores.set_meaning(
        dimen,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dimen),
    );
    stores.set_meaning(
        endlinechar,
        Meaning::IntParam(IntParam::END_LINE_CHAR.raw()),
    );
    stores.set_count(12, -34);
    stores.set_dimen(3, Scaled::from_raw(65_536));
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);

    assert_eq!(scan_with_stores("\\count12 x", &mut stores).0, -34);
    assert_eq!(scan_with_stores("\\dimen3 x", &mut stores).0, 65_536);
    assert_eq!(scan_with_stores("\\endlinechar x", &mut stores).0, 13);
}

#[test]
fn scans_chardef_like_meanings() {
    let mut stores = Universe::new();
    let letter_a = stores.intern("a");
    stores.set_meaning(letter_a, Meaning::CharGiven('A'));

    let (value, next) = scan_with_stores("\\a x", &mut stores);

    assert_eq!(value, 65);
    assert_eq!(next, Some(char_token('x', Catcode::Letter)));
}

#[test]
fn scans_values_through_macro_expansion() {
    let mut stores = Universe::new();
    let number = stores.intern("number");
    let replacement = stores.intern_token_list(&[
        char_token('4', Catcode::Other),
        char_token('2', Catcode::Other),
    ]);
    let params = stores.intern_token_list(&[]);
    stores.set_macro_meaning(
        number,
        MacroMeaning::new(MeaningFlags::EMPTY, params, replacement),
    );

    assert_eq!(scan_with_stores("\\number x", &mut stores).0, 42);
}

#[test]
fn reports_number_too_big_and_caps_value() {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new("2147483648 x"));
    let scanned = scan_int(&mut input, &mut stores).expect("scan should cap overflow");

    assert_eq!(scanned.value(), i32::MAX);
    assert_eq!(scanned.diagnostic(), Some(IntegerDiagnostic::NumberTooBig));
    let diagnostic = scanned
        .diagnostic()
        .expect("overflow should emit diagnostic");
    assert_eq!(format!("{diagnostic}"), "Number too big");
}

#[test]
fn missing_number_recovers_zero_and_replays_offending_token() {
    let (value, diagnostic, next) = scan("x");

    assert_eq!(value, 0);
    assert_eq!(diagnostic, Some(IntegerDiagnostic::MissingNumber));
    assert_eq!(next, Some(char_token('x', Catcode::Letter)));
}

#[test]
fn missing_number_diagnostic_uses_offending_token_origin() {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new("x"));
    let scanned = scan_int(&mut input, &mut stores).expect("scan should recover");
    let replayed = input
        .next_traced_token(&mut stores)
        .expect("token should replay")
        .expect("offending token should be unread");

    assert_eq!(scanned.diagnostic(), Some(IntegerDiagnostic::MissingNumber));
    assert_eq!(scanned.diagnostic_origin(), Some(replayed.origin()));
    assert_eq!(
        stores.origin(replayed.origin()),
        OriginRecord::Source(SourceOrigin::new(tex_state::SourceId::new(0), 0, 1, 0))
    );
}

#[test]
fn relax_in_number_slot_recovers_zero_and_replays_token() {
    let mut stores = Universe::new();
    let relax = stores.intern("relax");
    stores.set_meaning(relax, Meaning::Relax);
    let mut input = InputStack::new(MemoryInput::new("\\relax"));
    let scanned = scan_int(&mut input, &mut stores).expect("relax should recover as missing");

    assert_eq!(scanned.value(), 0);
    assert_eq!(scanned.diagnostic(), Some(IntegerDiagnostic::MissingNumber));
    assert_eq!(
        input.next_token(&mut stores).expect("token should replay"),
        Some(Token::Cs(relax))
    );
}

#[test]
fn rejects_out_of_range_register_numbers() {
    let mut stores = Universe::new();
    let count = stores.intern("count");
    stores.set_meaning(
        count,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Count),
    );
    let mut input = InputStack::new(MemoryInput::new("\\count32768"));
    let err = scan_int(&mut input, &mut stores).expect_err("register should be rejected");

    assert!(matches!(err, ScanIntError::RegisterNumberOutOfRange(32768)));
}
