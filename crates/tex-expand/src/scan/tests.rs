use super::{scan_toks, scan_toks_expanded};
use tex_lex::{InputStack, MemoryInput};
use tex_state::Universe;
use tex_state::meaning::MeaningFlags;
use tex_state::provenance::OriginRecord;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::NoopExpansionHooks;

fn scan(input: &str) -> (Universe, Vec<Token>, Vec<Token>) {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new(input));
    let context =
        TracedTokenWord::pack(Token::Cs(stores.intern("def").symbol()), OriginId::UNKNOWN);
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
fn expanded_definition_preserves_protected_macro_tokens() {
    let mut stores = Universe::new();
    let protected = stores.intern("protectedmacro");
    let empty = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('x', Catcode::Letter)]);
    stores.set_macro_meaning(
        protected,
        tex_state::macro_store::MacroMeaning::new(MeaningFlags::PROTECTED, empty, body),
    );
    let mut input = InputStack::new(MemoryInput::new("{\\protectedmacro}"));
    let context =
        TracedTokenWord::pack(Token::Cs(stores.intern("edef").symbol()), OriginId::UNKNOWN);

    let scanned = scan_toks_expanded(
        &mut input,
        &mut stores,
        MeaningFlags::EMPTY,
        context,
        &mut NoopExpansionHooks,
    )
    .expect("expanded definition scan");

    assert_eq!(
        stores.tokens(scanned.replacement_text()),
        &[Token::Cs(protected.symbol())]
    );
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
fn forbidden_outer_macro_closes_replacement_and_is_replayed() {
    let mut stores = Universe::new();
    let outer = stores.intern("outermacro");
    let empty = stores.intern_token_list(&[]);
    stores.set_macro_meaning(
        outer,
        tex_state::macro_store::MacroMeaning::new(MeaningFlags::OUTER, empty, empty),
    );
    let mut input = InputStack::new(MemoryInput::new("{\\outermacro trailing}"));
    let context =
        TracedTokenWord::pack(Token::Cs(stores.intern("def").symbol()), OriginId::UNKNOWN);

    let scanned = scan_toks(&mut input, &mut stores, MeaningFlags::EMPTY, context)
        .expect("outer token inserts a synthetic closing brace");
    assert!(stores.tokens(scanned.replacement_text()).is_empty());
    assert_eq!(
        input.next_token(&mut stores).expect("read replayed outer"),
        Some(Token::Cs(outer.symbol()))
    );
}

#[test]
fn freezes_parameter_and_replacement_origin_lists_from_source_tokens() {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new("#1{#1x}"));
    let context =
        TracedTokenWord::pack(Token::Cs(stores.intern("def").symbol()), OriginId::UNKNOWN);

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
    for (&origin, offset) in parameter_origins
        .iter()
        .chain(replacement_origins)
        .zip([1, 4, 5])
    {
        let OriginRecord::SourceSpan(span) = stores.origin(origin) else {
            panic!("ordinary source token must retain a logical source span");
        };
        assert_eq!(
            span.lo(),
            stores
                .source_position(tex_state::SourceId::new(0), offset)
                .expect("source position stays live")
        );
    }
}

#[test]
fn out_of_order_parameter_inserts_expected_and_replays_wrong_digit() {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new("#2{}"));

    let context =
        TracedTokenWord::pack(Token::Cs(stores.intern("def").symbol()), OriginId::UNKNOWN);
    let scanned = scan_toks(&mut input, &mut stores, MeaningFlags::EMPTY, context)
        .expect("scan should recover an out-of-order parameter");

    assert_eq!(
        stores.tokens(scanned.parameter_text()),
        &[Token::param(1), char_token('2', Catcode::Other)]
    );
    assert!(stores.tokens(scanned.replacement_text()).is_empty());
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
