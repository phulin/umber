use super::{
    MacroScanDiagnostic, ScanToksError, scan_general_text_expanded_with_expanded_open, scan_toks,
    scan_toks_expanded, scan_toks_expanded_with_driver,
};
use tex_lex::{InputStack, MemoryInput, TokenListReplayKind};
use tex_state::Universe;
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::provenance::OriginRecord;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::ExpansionContext;

fn scan(input: &str) -> (Universe, Vec<Token>, Vec<Token>) {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new(input));
    let context =
        TracedTokenWord::pack(Token::Cs(stores.intern("def").symbol()), OriginId::UNKNOWN);
    let scanned = scan_toks(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        MeaningFlags::EMPTY,
        context,
    )
    .expect("scan should succeed");
    let params = stores.tokens(scanned.parameter_text()).to_vec();
    let replacement = stores.tokens(scanned.replacement_text()).to_vec();
    (stores, params, replacement)
}

fn char_token(ch: char, cat: Catcode) -> Token {
    Token::Char { ch, cat }
}

fn install_passthrough_macro(stores: &mut Universe, name: &str) {
    let parameter = stores.intern_token_list(&[Token::param(1)]);
    let replacement = stores.intern_token_list(&[Token::param(1)]);
    let symbol = stores.intern(name);
    stores.set_macro_meaning(
        symbol,
        MacroMeaning::new(MeaningFlags::EMPTY, parameter, replacement),
    );
}

#[test]
fn expanded_preserves_group_pairs_around_nested_unexpanded_text() {
    let mut stores = Universe::new();
    crate::install_etex_expandable_primitives(&mut stores);
    crate::install_latex_expandable_primitives(&mut stores);
    let target = stores.intern("target");
    let context = TracedTokenWord::pack(Token::Cs(target.symbol()), OriginId::UNKNOWN);
    let mut input = InputStack::new(MemoryInput::new(
        "{\\unexpanded{\\target}{\\unexpanded{X}{Y}}}",
    ));

    let expanded = scan_general_text_expanded_with_expanded_open(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        &mut ExpansionContext::new("texput"),
        &mut crate::DriverExpansionMode,
        context,
    )
    .expect("expanded text should preserve nested grouping");

    assert_eq!(
        stores.tokens(expanded.token_list()),
        &[
            Token::Cs(target.symbol()),
            char_token('{', Catcode::BeginGroup),
            char_token('X', Catcode::Letter),
            char_token('{', Catcode::BeginGroup),
            char_token('Y', Catcode::Letter),
            char_token('}', Catcode::EndGroup),
            char_token('}', Catcode::EndGroup),
        ]
    );
}

#[test]
fn expanded_preserves_unexpanded_replay_expanded_once_by_expandafter() {
    let mut stores = Universe::new();
    crate::install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    crate::install_latex_expandable_primitives(&mut stores);
    let target = stores.intern("target");
    let context = TracedTokenWord::pack(Token::Cs(target.symbol()), OriginId::UNKNOWN);
    let mut input = InputStack::new(MemoryInput::new("{\\expandafter A\\unexpanded{\\target}}"));

    let expanded = scan_general_text_expanded_with_expanded_open(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        &mut ExpansionContext::new("texput"),
        &mut crate::DriverExpansionMode,
        context,
    )
    .expect("expandafter should preserve unexpanded replay during collection");

    assert_eq!(
        stores.tokens(expanded.token_list()),
        &[char_token('A', Catcode::Letter), Token::Cs(target.symbol()),]
    );
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
        &mut tex_state::ExpansionContext::new(&mut stores),
        MeaningFlags::EMPTY,
        context,
        &mut ExpansionContext::new("texput"),
    )
    .expect("expanded definition scan");

    assert_eq!(
        stores.tokens(scanned.replacement_text()),
        &[Token::Cs(protected.symbol())]
    );
}

#[test]
fn expanded_definition_records_and_discards_undefined_control_sequences() {
    let mut stores = Universe::new();
    let missing = stores.intern("missing");
    let mut input = InputStack::new(MemoryInput::new("{a\\missing b}"));
    let context =
        TracedTokenWord::pack(Token::Cs(stores.intern("edef").symbol()), OriginId::UNKNOWN);

    let scanned = scan_toks_expanded_with_driver(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        MeaningFlags::EMPTY,
        context,
        &mut ExpansionContext::new("texput"),
    )
    .expect("undefined expansion is recoverable inside an expanded definition");

    assert_eq!(
        stores.tokens(scanned.replacement_text()),
        &[
            char_token('a', Catcode::Letter),
            char_token('b', Catcode::Letter),
        ]
    );
    assert!(matches!(
        scanned.diagnostics(),
        [MacroScanDiagnostic::UndefinedControlSequence { name, .. }] if name == "missing"
    ));
    assert_eq!(stores.meaning(missing), Meaning::Undefined);
}

#[test]
fn expanded_definition_expandafter_forces_only_its_protected_target() {
    // e-TeX manual section 3.1: protected macros resist `\edef`, but an
    // explicit `\expandafter` still expands its target by one step.
    let mut stores = Universe::new();
    crate::install_expandable_primitives(&mut stores);
    let first = stores.intern("first");
    let second = stores.intern("second");
    let empty = stores.intern_token_list(&[]);
    let second_body = stores.intern_token_list(&[Token::Cs(second.symbol())]);
    stores.set_macro_meaning(
        first,
        tex_state::macro_store::MacroMeaning::new(MeaningFlags::PROTECTED, empty, second_body),
    );
    stores.set_macro_meaning(
        second,
        tex_state::macro_store::MacroMeaning::new(MeaningFlags::PROTECTED, empty, empty),
    );
    let mut input = InputStack::new(MemoryInput::new("{\\expandafter\\first\\first}"));
    let context =
        TracedTokenWord::pack(Token::Cs(stores.intern("edef").symbol()), OriginId::UNKNOWN);

    let scanned = scan_toks_expanded(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        MeaningFlags::EMPTY,
        context,
        &mut ExpansionContext::new("texput"),
    )
    .expect("expanded definition scan");

    assert_eq!(
        stores.tokens(scanned.replacement_text()),
        &[Token::Cs(first.symbol()), Token::Cs(second.symbol())]
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

    let scanned = scan_toks(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        MeaningFlags::EMPTY,
        context,
    )
    .expect("outer token inserts a synthetic closing brace");
    assert!(stores.tokens(scanned.replacement_text()).is_empty());
    assert_eq!(
        input
            .next_token(&mut tex_state::ExpansionContext::new(&mut stores))
            .expect("read replayed outer"),
        Some(Token::Cs(outer.symbol()))
    );
}

#[test]
fn forbidden_outer_macro_closes_expanded_replacement_before_expansion() {
    let mut stores = Universe::new();
    let outer = stores.intern("outermacro");
    let empty = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('x', Catcode::Letter)]);
    stores.set_macro_meaning(
        outer,
        tex_state::macro_store::MacroMeaning::new(MeaningFlags::OUTER, empty, body),
    );
    let mut input = InputStack::new(MemoryInput::new("{\\outermacro trailing}"));
    let context =
        TracedTokenWord::pack(Token::Cs(stores.intern("xdef").symbol()), OriginId::UNKNOWN);

    let scanned = scan_toks_expanded_with_driver(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        MeaningFlags::EMPTY,
        context,
        &mut ExpansionContext::new("texput"),
    )
    .expect("outer token inserts a synthetic closing brace");

    assert!(stores.tokens(scanned.replacement_text()).is_empty());
    assert_eq!(
        input
            .next_token(&mut tex_state::ExpansionContext::new(&mut stores))
            .expect("read replayed outer"),
        Some(Token::Cs(outer.symbol()))
    );
}

#[test]
fn noexpand_suppresses_outer_validation_in_expanded_replacement() {
    let mut stores = Universe::new();
    crate::install_expandable_primitives(&mut stores);
    let outer = stores.intern("outermacro");
    let empty = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('x', Catcode::Letter)]);
    stores.set_macro_meaning(
        outer,
        tex_state::macro_store::MacroMeaning::new(MeaningFlags::OUTER, empty, body),
    );
    let mut input = InputStack::new(MemoryInput::new("{\\noexpand\\outermacro}"));
    let context =
        TracedTokenWord::pack(Token::Cs(stores.intern("xdef").symbol()), OriginId::UNKNOWN);

    let scanned = scan_toks_expanded_with_driver(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        MeaningFlags::EMPTY,
        context,
        &mut ExpansionContext::new("texput"),
    )
    .expect("noexpand should hide the outer command code from get_next");

    assert_eq!(
        stores.tokens(scanned.replacement_text()),
        &[Token::Cs(outer.symbol())]
    );
}

#[test]
fn ordinary_expanded_replacement_avoids_back_input() {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new("{abcdefghijklmnopqrstuvwxyz}"));
    let context =
        TracedTokenWord::pack(Token::Cs(stores.intern("xdef").symbol()), OriginId::UNKNOWN);
    crate::reset_back_input_call_count();

    let scanned = scan_toks_expanded_with_driver(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        MeaningFlags::EMPTY,
        context,
        &mut ExpansionContext::new("texput"),
    )
    .expect("expanded definition scan");

    assert_eq!(stores.tokens(scanned.replacement_text()).len(), 26);
    assert_eq!(crate::back_input_call_count(), 0);
}

#[test]
fn expanded_definition_interprets_parameter_references_from_macro_argument_replay() {
    let mut stores = Universe::new();
    install_passthrough_macro(&mut stores, "passthrough");
    let mut input = InputStack::new(MemoryInput::new(
        "#1#2#3#4#5#6#7#8#9{\\passthrough{#1#2#3#4#5#6#7#8#9}}",
    ));
    let context =
        TracedTokenWord::pack(Token::Cs(stores.intern("xdef").symbol()), OriginId::UNKNOWN);

    let scanned = scan_toks_expanded_with_driver(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        MeaningFlags::EMPTY,
        context,
        &mut ExpansionContext::new("texput"),
    )
    .expect("macro-argument replay should retain parameter semantics");

    assert_eq!(
        stores.tokens(scanned.replacement_text()),
        &(1_u8..=9).map(Token::param).collect::<Vec<_>>()
    );
}

#[test]
fn expanded_definition_interprets_doubled_parameter_from_macro_argument_replay() {
    let mut stores = Universe::new();
    install_passthrough_macro(&mut stores, "passthrough");
    let mut input = InputStack::new(MemoryInput::new("{\\passthrough{##}}"));
    let context =
        TracedTokenWord::pack(Token::Cs(stores.intern("xdef").symbol()), OriginId::UNKNOWN);

    let scanned = scan_toks_expanded_with_driver(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        MeaningFlags::EMPTY,
        context,
        &mut ExpansionContext::new("texput"),
    )
    .expect("doubled parameter should survive macro-argument replay");

    assert_eq!(
        stores.tokens(scanned.replacement_text()),
        &[char_token('#', Catcode::Parameter)]
    );
}

#[test]
fn expanded_definition_copies_unexpanded_parameter_character_verbatim() {
    let mut stores = Universe::new();
    crate::install_etex_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("{\\unexpanded{#1}}"));
    let context =
        TracedTokenWord::pack(Token::Cs(stores.intern("xdef").symbol()), OriginId::UNKNOWN);

    let scanned = scan_toks_expanded_with_driver(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        MeaningFlags::EMPTY,
        context,
        &mut ExpansionContext::new("texput"),
    )
    .expect("unexpanded parameter character is copied literally");

    assert_eq!(
        stores.tokens(scanned.replacement_text()),
        &[
            char_token('#', Catcode::Parameter),
            char_token('1', Catcode::Other),
        ]
    );
}

#[test]
fn expanded_definition_does_not_expand_the_token_register_contents() {
    let mut stores = Universe::new();
    let the = stores.intern("the");
    let toks = stores.intern("toks");
    stores.set_meaning(the, Meaning::ExpandablePrimitive(ExpandablePrimitive::The));
    stores.set_meaning(
        toks,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Toks),
    );
    let macro_cs = stores.intern("macro");
    let empty = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('x', Catcode::Letter)]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, empty, body),
    );
    let contents = stores.intern_token_list(&[Token::Cs(macro_cs.symbol())]);
    stores.set_toks(4, contents);
    let mut input = InputStack::new(MemoryInput::new("{\\the\\toks4}"));
    let context =
        TracedTokenWord::pack(Token::Cs(stores.intern("xdef").symbol()), OriginId::UNKNOWN);

    let scanned = scan_toks_expanded_with_driver(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        MeaningFlags::EMPTY,
        context,
        &mut ExpansionContext::new("texput"),
    )
    .expect("the token-register contents should be copied without expansion");

    assert_eq!(
        stores.tokens(scanned.replacement_text()),
        &[Token::Cs(macro_cs.symbol())]
    );
}

#[test]
fn expanded_definition_copies_parameter_tokens_from_token_register() {
    let mut stores = Universe::new();
    let the = stores.intern("the");
    let toks = stores.intern("toks");
    stores.set_meaning(the, Meaning::ExpandablePrimitive(ExpandablePrimitive::The));
    stores.set_meaning(
        toks,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Toks),
    );
    let contents = stores.intern_token_list(&[Token::param(1)]);
    stores.set_toks(4, contents);
    let mut input = InputStack::new(MemoryInput::new("{\\the\\toks4}"));
    let context =
        TracedTokenWord::pack(Token::Cs(stores.intern("xdef").symbol()), OriginId::UNKNOWN);

    let scanned = scan_toks_expanded_with_driver(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        MeaningFlags::EMPTY,
        context,
        &mut ExpansionContext::new("texput"),
    )
    .expect("parameter tokens from token registers should be copied verbatim");

    assert_eq!(
        stores.tokens(scanned.replacement_text()),
        &[Token::param(1)]
    );
}

#[test]
fn definition_accepts_internal_parameter_after_parameter_marker() {
    let mut stores = Universe::new();
    let replay = stores.intern_token_list(&[
        char_token('{', Catcode::BeginGroup),
        char_token('#', Catcode::Parameter),
        Token::param(1),
        char_token('}', Catcode::EndGroup),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(replay, TokenListReplayKind::Inserted);
    let context =
        TracedTokenWord::pack(Token::Cs(stores.intern("def").symbol()), OriginId::UNKNOWN);

    let scanned = scan_toks(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        MeaningFlags::EMPTY,
        context,
    )
    .expect("an internal parameter token is a valid parameter follower");

    assert_eq!(
        stores.tokens(scanned.replacement_text()),
        &[Token::param(1)]
    );
}

#[test]
fn expanded_definition_reexpands_nested_unexpanded_output() {
    let mut stores = Universe::new();
    crate::install_etex_expandable_primitives(&mut stores);
    crate::install_latex_expandable_primitives(&mut stores);
    let macro_cs = stores.intern("macro");
    let empty = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('x', Catcode::Letter)]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, empty, body),
    );
    let mut input = InputStack::new(MemoryInput::new("{\\expanded{\\unexpanded{\\macro}}}"));
    let context =
        TracedTokenWord::pack(Token::Cs(stores.intern("xdef").symbol()), OriginId::UNKNOWN);

    let scanned = scan_toks_expanded_with_driver(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        MeaningFlags::EMPTY,
        context,
        &mut ExpansionContext::new("texput"),
    )
    .expect("nested unexpanded output should re-enter the enclosing expansion");

    assert_eq!(
        stores.tokens(scanned.replacement_text()),
        &[char_token('x', Catcode::Letter)]
    );
}

#[test]
fn unexpanded_provenance_does_not_suppress_later_macro_replay() {
    let mut stores = Universe::new();
    crate::install_etex_expandable_primitives(&mut stores);
    let macro_cs = stores.intern("macro");
    let empty = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('x', Catcode::Letter)]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, empty, body),
    );
    let context =
        TracedTokenWord::pack(Token::Cs(stores.intern("xdef").symbol()), OriginId::UNKNOWN);
    let mut first_input = InputStack::new(MemoryInput::new("{\\unexpanded{\\macro}}"));
    let first = scan_toks_expanded_with_driver(
        &mut first_input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        MeaningFlags::EMPTY,
        context,
        &mut ExpansionContext::new("texput"),
    )
    .expect("first expanded definition");
    let holder = stores.intern("holder");
    stores.set_macro_meaning_with_provenance(holder, first.meaning(), first.provenance());

    let mut second_input = InputStack::new(MemoryInput::new("{\\holder}"));
    let second = scan_toks_expanded_with_driver(
        &mut second_input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        MeaningFlags::EMPTY,
        context,
        &mut ExpansionContext::new("texput"),
    )
    .expect("later replay expands normally");

    assert_eq!(
        stores.tokens(second.replacement_text()),
        &[char_token('x', Catcode::Letter)]
    );
}

#[test]
fn expanded_definition_tracks_braces_returned_by_nested_expanded() {
    let mut stores = Universe::new();
    crate::install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    crate::install_latex_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("{\\expanded{{\\iffalse}}}\\fi X}}}"));
    let context =
        TracedTokenWord::pack(Token::Cs(stores.intern("xdef").symbol()), OriginId::UNKNOWN);

    let scanned = scan_toks_expanded_with_driver(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        MeaningFlags::EMPTY,
        context,
        &mut ExpansionContext::new("texput"),
    )
    .expect("nested expanded braces should extend the outer definition scan");

    assert_eq!(
        stores.tokens(scanned.replacement_text()),
        &[
            char_token('{', Catcode::BeginGroup),
            char_token('X', Catcode::Letter),
            char_token('}', Catcode::EndGroup),
        ]
    );
}

#[test]
fn expanded_definition_rejects_invalid_parameter_from_macro_argument_replay() {
    let mut stores = Universe::new();
    install_passthrough_macro(&mut stores, "passthrough");
    let mut input = InputStack::new(MemoryInput::new("{\\passthrough{#x}}"));
    let context =
        TracedTokenWord::pack(Token::Cs(stores.intern("xdef").symbol()), OriginId::UNKNOWN);

    let error = scan_toks_expanded_with_driver(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        MeaningFlags::EMPTY,
        context,
        &mut ExpansionContext::new("texput"),
    )
    .expect_err("invalid parameter follower must not be swallowed by a literal span");

    let ScanToksError::InvalidParameterTokenInReplacementText { context } = error else {
        panic!("unexpected error: {error}");
    };
    assert_eq!(
        context.token(),
        Some(char_token('x', Catcode::Letter)),
        "the diagnostic must retain the offending replayed token"
    );
    assert_ne!(context.origin(), OriginId::UNKNOWN);
}

#[test]
fn freezes_parameter_and_replacement_origin_lists_from_source_tokens() {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new("#1{#1x}"));
    let context =
        TracedTokenWord::pack(Token::Cs(stores.intern("def").symbol()), OriginId::UNKNOWN);

    let scanned = scan_toks(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        MeaningFlags::EMPTY,
        context,
    )
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
    let scanned = scan_toks(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        MeaningFlags::EMPTY,
        context,
    )
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
    assert_eq!(
        replacement,
        vec![Token::param(1), char_token('{', Catcode::BeginGroup)]
    );
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
