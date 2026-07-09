use crate::args::{MacroCallError, match_macro_call};
use tex_lex::{InputStack, MemoryInput, TokenListReplayKind};
use tex_state::TracedTokenList;
use tex_state::ids::OriginListId;
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::MeaningFlags;
use tex_state::token::{Catcode, OriginId, Token};
use tex_state::{ExpansionState, Universe};

fn char_token(ch: char, cat: Catcode) -> Token {
    Token::Char { ch, cat }
}

fn cs_token(stores: &mut impl ExpansionState, name: &str) -> Token {
    Token::Cs(stores.intern(name))
}

fn macro_meaning(
    stores: &mut impl ExpansionState,
    flags: MeaningFlags,
    params: &[Token],
) -> MacroMeaning {
    let params = stores.intern_token_list(params);
    let body = stores.intern_token_list(&[]);
    MacroMeaning::new(flags, params, body)
}

fn match_from_list(
    stores: &mut impl ExpansionState,
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

fn match_traced_from_list(
    stores: &mut impl ExpansionState,
    meaning: MacroMeaning,
    input_tokens: &[Token],
    input_origins: OriginListId,
) -> Result<Vec<TracedTokenList>, MacroCallError> {
    let call = cs_token(stores, "m");
    let input_list = stores.intern_token_list(input_tokens);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list_with_origins(input_list, input_origins, TokenListReplayKind::Inserted);
    let matched = match_macro_call(&mut input, stores, call, meaning)?;
    Ok((1..=matched.len())
        .map(|slot| matched.get_traced(slot as u8).expect("slot"))
        .collect())
}

fn source_origin(stores: &mut impl ExpansionState, line: u32, column: u32) -> OriginId {
    stores.source_origin(tex_state::SourceId::new(1), u64::from(column), line, column)
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
fn undelimited_argument_freezes_call_site_origins() {
    let mut stores = Universe::new();
    let meaning = macro_meaning(&mut stores, MeaningFlags::EMPTY, &[Token::param(1)]);
    let skipped_space = source_origin(&mut stores, 1, 1);
    let argument_origin = source_origin(&mut stores, 1, 2);
    let input_origins = stores.allocate_origin_list(&[skipped_space, argument_origin]);

    let args = match_traced_from_list(
        &mut stores,
        meaning,
        &[
            char_token(' ', Catcode::Space),
            char_token('x', Catcode::Letter),
        ],
        input_origins,
    )
    .expect("argument should match");

    assert_eq!(
        stores.tokens(args[0].token_list()),
        &[char_token('x', Catcode::Letter)]
    );
    assert_eq!(
        stores.origin_list(args[0].origin_list()),
        &[argument_origin]
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
fn delimited_argument_freezes_only_argument_origins() {
    let mut stores = Universe::new();
    let meaning = macro_meaning(
        &mut stores,
        MeaningFlags::EMPTY,
        &[Token::param(1), char_token(',', Catcode::Other)],
    );
    let argument_origin = source_origin(&mut stores, 2, 1);
    let delimiter_origin = source_origin(&mut stores, 2, 2);
    let input_origins = stores.allocate_origin_list(&[argument_origin, delimiter_origin]);

    let args = match_traced_from_list(
        &mut stores,
        meaning,
        &[
            char_token('x', Catcode::Letter),
            char_token(',', Catcode::Other),
        ],
        input_origins,
    )
    .expect("argument should match");

    assert_eq!(
        stores.tokens(args[0].token_list()),
        &[char_token('x', Catcode::Letter)]
    );
    assert_eq!(
        stores.origin_list(args[0].origin_list()),
        &[argument_origin]
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
fn delimited_argument_preserves_recovered_prefix_origins() {
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
    let recovered_origin = source_origin(&mut stores, 3, 1);
    let delimiter_a_origin = source_origin(&mut stores, 3, 2);
    let delimiter_b_origin = source_origin(&mut stores, 3, 3);
    let input_origins =
        stores.allocate_origin_list(&[recovered_origin, delimiter_a_origin, delimiter_b_origin]);

    let args = match_traced_from_list(
        &mut stores,
        meaning,
        &[
            char_token('a', Catcode::Letter),
            char_token('a', Catcode::Letter),
            char_token('b', Catcode::Letter),
        ],
        input_origins,
    )
    .expect("overlapping delimiter prefix should match");

    assert_eq!(
        stores.tokens(args[0].token_list()),
        &[char_token('a', Catcode::Letter)]
    );
    assert_eq!(
        stores.origin_list(args[0].origin_list()),
        &[recovered_origin]
    );
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
fn non_long_delimited_argument_allows_par_from_failed_delimiter_prefix() {
    let mut stores = Universe::new();
    let par = stores.intern("par");
    let bang = char_token('!', Catcode::Other);
    let question = char_token('?', Catcode::Other);
    let meaning = macro_meaning(
        &mut stores,
        MeaningFlags::EMPTY,
        &[Token::param(1), Token::Cs(par), bang],
    );

    let args = match_from_list(
        &mut stores,
        meaning,
        &[Token::Cs(par), question, Token::Cs(par), bang],
    )
    .expect("failed delimiter prefix should allow recovered par");

    assert_eq!(args, vec![vec![Token::Cs(par), question]]);
}

#[test]
fn non_long_delimited_argument_rejects_par_that_only_mismatches_delimiter() {
    let mut stores = Universe::new();
    let par = stores.intern("par");
    let meaning = macro_meaning(
        &mut stores,
        MeaningFlags::EMPTY,
        &[
            Token::param(1),
            char_token('a', Catcode::Letter),
            char_token('b', Catcode::Letter),
        ],
    );

    let err = match_from_list(
        &mut stores,
        meaning,
        &[char_token('a', Catcode::Letter), Token::Cs(par)],
    )
    .expect_err("mismatching par should still end a non-long argument");

    assert!(matches!(
        err,
        MacroCallError::ParagraphEndedBeforeComplete { ref macro_name } if macro_name == "\\m"
    ));
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
