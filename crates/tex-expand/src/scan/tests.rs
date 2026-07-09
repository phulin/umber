use super::{ScanToksError, scan_toks};
use tex_lex::{InputStack, MemoryInput};
use tex_state::Universe;
use tex_state::meaning::MeaningFlags;
use tex_state::provenance::{OriginRecord, SourceOrigin};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

fn scan(input: &str) -> (Universe, Vec<Token>, Vec<Token>) {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new(input));
    let context = TracedTokenWord::pack(Token::Cs(stores.intern("def")), OriginId::UNKNOWN);
    let scanned = scan_toks(&mut input, &mut stores, MeaningFlags::EMPTY, context)
        .expect("scan should succeed");
    let params = stores.tokens(scanned.parameter_text()).to_vec();
    let replacement = stores.tokens(scanned.replacement_text()).to_vec();
    (stores, params, replacement)
}

fn char_token(ch: char, cat: Catcode) -> Token {
    Token::Char { ch, cat }
}

#[test]
fn scans_delimited_and_undelimited_parameters() {
    let (_stores, params, replacement) = scan("#1a#2{#2#1}");

    assert_eq!(
        params,
        vec![
            Token::param(1),
            char_token('a', Catcode::Letter),
            Token::param(2),
        ]
    );
    assert_eq!(replacement, vec![Token::param(2), Token::param(1)]);
}

#[test]
fn scans_all_nine_parameters_in_order() {
    let (_stores, params, replacement) = scan("#1#2#3#4#5#6#7#8#9{#9#1}");

    assert_eq!(params, (1_u8..=9).map(Token::param).collect::<Vec<_>>());
    assert_eq!(replacement, vec![Token::param(9), Token::param(1)]);
}

#[test]
fn freezes_parameter_and_replacement_origin_lists_from_source_tokens() {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new("#1{#1x}"));
    let context = TracedTokenWord::pack(Token::Cs(stores.intern("def")), OriginId::UNKNOWN);

    let scanned = scan_toks(&mut input, &mut stores, MeaningFlags::EMPTY, context)
        .expect("scan should succeed");
    let provenance = scanned.provenance();
    let parameter_origins = stores.origin_list(provenance.parameter_origins());
    let replacement_origins = stores.origin_list(provenance.replacement_origins());

    assert_eq!(stores.tokens(scanned.parameter_text()), &[Token::param(1)]);
    assert_eq!(
        stores.tokens(scanned.replacement_text()),
        &[Token::param(1), char_token('x', Catcode::Letter)]
    );
    assert_eq!(parameter_origins.len(), 1);
    assert_eq!(replacement_origins.len(), 2);
    assert_eq!(
        stores.origin(parameter_origins[0]),
        OriginRecord::Source(SourceOrigin::new(tex_state::SourceId::new(0), 1, 1, 1))
    );
    assert_eq!(
        stores.origin(replacement_origins[0]),
        OriginRecord::Source(SourceOrigin::new(tex_state::SourceId::new(0), 4, 1, 4))
    );
    assert_eq!(
        stores.origin(replacement_origins[1]),
        OriginRecord::Source(SourceOrigin::new(tex_state::SourceId::new(0), 5, 1, 5))
    );
}

#[test]
fn rejects_out_of_order_parameter_numbers() {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new("#2{}"));

    let context = TracedTokenWord::pack(Token::Cs(stores.intern("def")), OriginId::UNKNOWN);
    let err = scan_toks(&mut input, &mut stores, MeaningFlags::EMPTY, context)
        .expect_err("scan should reject out-of-order parameter");

    assert!(matches!(
        err,
        ScanToksError::ParameterNumberOutOfOrder {
            expected: 1,
            found: 2,
            ..
        }
    ));
}

#[test]
fn scans_trailing_hash_brace_parameter_text() {
    let (_stores, params, replacement) = scan("#1#{#1}");

    assert_eq!(
        params,
        vec![Token::param(1), char_token('{', Catcode::BeginGroup)]
    );
    assert_eq!(replacement, vec![Token::param(1)]);
}

#[test]
fn captures_nested_braces_in_replacement_text() {
    let (_stores, params, replacement) = scan("{a{b}c}");

    assert!(params.is_empty());
    assert_eq!(
        replacement,
        vec![
            char_token('a', Catcode::Letter),
            char_token('{', Catcode::BeginGroup),
            char_token('b', Catcode::Letter),
            char_token('}', Catcode::EndGroup),
            char_token('c', Catcode::Letter),
        ]
    );
}

#[test]
fn scans_doubled_hash_as_literal_parameter_character_in_body() {
    let (_stores, params, replacement) = scan("{##}");

    assert!(params.is_empty());
    assert_eq!(replacement, vec![char_token('#', Catcode::Parameter)]);
}
