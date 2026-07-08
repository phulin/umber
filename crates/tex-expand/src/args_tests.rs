use crate::args::{MacroCallError, match_macro_call};
use tex_lex::{InputStack, MemoryInput, TokenListReplayKind};
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::MeaningFlags;
use tex_state::token::{Catcode, Token};
use tex_state::{ExpansionState, InputOpenState, Universe};

fn char_token(ch: char, cat: Catcode) -> Token {
    Token::Char { ch, cat }
}

fn cs_token(stores: &mut (impl ExpansionState + InputOpenState), name: &str) -> Token {
    Token::Cs(stores.intern(name))
}

fn macro_meaning(
    stores: &mut (impl ExpansionState + InputOpenState),
    flags: MeaningFlags,
    params: &[Token],
) -> MacroMeaning {
    let params = stores.intern_token_list(params);
    let body = stores.intern_token_list(&[]);
    MacroMeaning::new(flags, params, body)
}

fn match_from_list(
    stores: &mut (impl ExpansionState + InputOpenState),
    meaning: MacroMeaning,
    input_tokens: &[Token],
) -> Result<Vec<Vec<Token>>, MacroCallError> {
    let call = cs_token(stores, "m");
    let input_list = stores.intern_token_list(input_tokens);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(input_list, TokenListReplayKind::Inserted);
    let matched = match_macro_call(&mut input, stores, call, meaning)?;
    Ok((1..=matched.len())
        .map(|slot| {
            stores
                .tokens(matched.get(slot as u8).expect("slot"))
                .to_vec()
        })
        .collect())
}

#[test]
fn matches_undelimited_single_token_argument_after_optional_spaces() {
    let mut stores = Universe::new();
    let meaning = macro_meaning(&mut stores, MeaningFlags::EMPTY, &[Token::param(1)]);

    let args = match_from_list(
        &mut stores,
        meaning,
        &[
            char_token(' ', Catcode::Space),
            char_token('x', Catcode::Letter),
        ],
    )
    .expect("argument should match");

    assert_eq!(args, vec![vec![char_token('x', Catcode::Letter)]]);
}

#[test]
fn matches_undelimited_balanced_group_without_outer_braces() {
    let mut stores = Universe::new();
    let meaning = macro_meaning(&mut stores, MeaningFlags::EMPTY, &[Token::param(1)]);

    let args = match_from_list(
        &mut stores,
        meaning,
        &[
            char_token('{', Catcode::BeginGroup),
            char_token('a', Catcode::Letter),
            char_token('{', Catcode::BeginGroup),
            char_token('b', Catcode::Letter),
            char_token('}', Catcode::EndGroup),
            char_token('}', Catcode::EndGroup),
        ],
    )
    .expect("argument should match");

    assert_eq!(
        args,
        vec![vec![
            char_token('a', Catcode::Letter),
            char_token('{', Catcode::BeginGroup),
            char_token('b', Catcode::Letter),
            char_token('}', Catcode::EndGroup),
        ]]
    );
}

#[test]
fn matches_delimited_argument_runs() {
    let mut stores = Universe::new();
    let meaning = macro_meaning(
        &mut stores,
        MeaningFlags::EMPTY,
        &[
            Token::param(1),
            char_token('a', Catcode::Letter),
            char_token('b', Catcode::Letter),
            Token::param(2),
            char_token('!', Catcode::Other),
        ],
    );

    let args = match_from_list(
        &mut stores,
        meaning,
        &[
            char_token('x', Catcode::Letter),
            char_token('a', Catcode::Letter),
            char_token('b', Catcode::Letter),
            char_token('y', Catcode::Letter),
            char_token('!', Catcode::Other),
        ],
    )
    .expect("arguments should match");

    assert_eq!(
        args,
        vec![
            vec![char_token('x', Catcode::Letter)],
            vec![char_token('y', Catcode::Letter)]
        ]
    );
}

#[test]
fn delimited_argument_matching_handles_overlapping_prefixes() {
    let mut stores = Universe::new();
    let meaning = macro_meaning(
        &mut stores,
        MeaningFlags::EMPTY,
        &[
            Token::param(1),
            char_token('a', Catcode::Letter),
            char_token('b', Catcode::Letter),
        ],
    );

    let args = match_from_list(
        &mut stores,
        meaning,
        &[
            char_token('a', Catcode::Letter),
            char_token('a', Catcode::Letter),
            char_token('b', Catcode::Letter),
        ],
    )
    .expect("overlapping delimiter prefix should match");

    assert_eq!(args, vec![vec![char_token('a', Catcode::Letter)]]);
}

#[test]
fn delimiter_inside_nested_braces_does_not_end_argument() {
    let mut stores = Universe::new();
    let meaning = macro_meaning(
        &mut stores,
        MeaningFlags::EMPTY,
        &[Token::param(1), char_token(',', Catcode::Other)],
    );

    let args = match_from_list(
        &mut stores,
        meaning,
        &[
            char_token('{', Catcode::BeginGroup),
            char_token(',', Catcode::Other),
            char_token('}', Catcode::EndGroup),
            char_token(',', Catcode::Other),
        ],
    )
    .expect("argument should match");

    assert_eq!(args, vec![vec![char_token(',', Catcode::Other)]]);
}

#[test]
fn delimited_argument_strips_one_outer_balanced_group() {
    let mut stores = Universe::new();
    let meaning = macro_meaning(
        &mut stores,
        MeaningFlags::EMPTY,
        &[Token::param(1), char_token('x', Catcode::Letter)],
    );

    let args = match_from_list(
        &mut stores,
        meaning,
        &[
            char_token('{', Catcode::BeginGroup),
            char_token('{', Catcode::BeginGroup),
            char_token('a', Catcode::Letter),
            char_token('}', Catcode::EndGroup),
            char_token('}', Catcode::EndGroup),
            char_token('x', Catcode::Letter),
        ],
    )
    .expect("argument should match");

    assert_eq!(
        args,
        vec![vec![
            char_token('{', Catcode::BeginGroup),
            char_token('a', Catcode::Letter),
            char_token('}', Catcode::EndGroup),
        ]]
    );
}

#[test]
fn leading_parameter_text_mismatch_reports_tex_message() {
    let mut stores = Universe::new();
    let meaning = macro_meaning(
        &mut stores,
        MeaningFlags::EMPTY,
        &[char_token('a', Catcode::Letter), Token::param(1)],
    );

    let err = match_from_list(&mut stores, meaning, &[char_token('b', Catcode::Letter)])
        .expect_err("call should not match");

    assert!(matches!(
        err,
        MacroCallError::DoesNotMatchDefinition { ref macro_name } if macro_name == "\\m"
    ));
    assert_eq!(err.to_string(), "Use of \\m doesn't match its definition");
}

#[test]
fn non_long_macro_rejects_paragraph_token_in_argument() {
    let mut stores = Universe::new();
    let par = stores.intern("par");
    let meaning = macro_meaning(&mut stores, MeaningFlags::EMPTY, &[Token::param(1)]);

    let err = match_from_list(&mut stores, meaning, &[Token::Cs(par)])
        .expect_err("non-long macro should reject par");

    assert!(matches!(
        err,
        MacroCallError::ParagraphEndedBeforeComplete { ref macro_name } if macro_name == "\\m"
    ));
    assert_eq!(err.to_string(), "Paragraph ended before \\m was complete");
}

#[test]
fn long_macro_accepts_paragraph_token_in_argument() {
    let mut stores = Universe::new();
    let par = stores.intern("par");
    let meaning = macro_meaning(&mut stores, MeaningFlags::LONG, &[Token::param(1)]);

    let args = match_from_list(&mut stores, meaning, &[Token::Cs(par)])
        .expect("long macro should accept par");

    assert_eq!(args, vec![vec![Token::Cs(par)]]);
}

#[test]
fn rejects_outer_control_sequence_while_scanning_argument() {
    let mut stores = Universe::new();
    let outer = stores.intern("outer");
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[]);
    stores.set_macro_meaning(outer, MacroMeaning::new(MeaningFlags::OUTER, params, body));
    let meaning = macro_meaning(&mut stores, MeaningFlags::EMPTY, &[Token::param(1)]);

    let err = match_from_list(&mut stores, meaning, &[Token::Cs(outer)])
        .expect_err("outer token should be rejected");

    assert!(matches!(
        err,
        MacroCallError::ForbiddenOuterToken { ref macro_name } if macro_name == "\\m"
    ));
    assert_eq!(
        err.to_string(),
        "Forbidden control sequence found while scanning use of \\m"
    );
}
