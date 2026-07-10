use tex_lex::{InputStack, MemoryInput};
use tex_state::Universe;
use tex_state::glue::{GlueSpec, Order};
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::scan_glue::{scan_glue, scan_muglue};

fn char_token(ch: char, cat: Catcode) -> Token {
    Token::Char { ch, cat }
}

fn context() -> TracedTokenWord {
    TracedTokenWord::pack(
        Token::Char {
            ch: '=',
            cat: Catcode::Other,
        },
        OriginId::UNKNOWN,
    )
}

fn scan(input_text: &str) -> (GlueSpec, Option<Token>) {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new(input_text));
    let scanned = scan_glue(&mut input, &mut stores, context()).expect("glue scan should succeed");
    let spec = stores.glue(scanned.id());
    let next = input
        .next_token(&mut stores)
        .expect("remaining token should lex");
    (spec, next)
}

#[test]
fn scans_width_plus_and_minus_components() {
    let (spec, next) = scan("1pt plus 2pt minus .5pt x");

    assert_eq!(spec.width.raw(), 65_536);
    assert_eq!(spec.stretch.raw(), 131_072);
    assert_eq!(spec.stretch_order, Order::Normal);
    assert_eq!(spec.shrink.raw(), 32_768);
    assert_eq!(spec.shrink_order, Order::Normal);
    assert_eq!(next, Some(char_token('x', Catcode::Letter)));
}

#[test]
fn scans_infinite_orders_case_insensitively() {
    let (spec, _next) = scan("0pt PlUs 1fil minus 2FiLlL x");

    assert_eq!(spec.stretch.raw(), 65_536);
    assert_eq!(spec.stretch_order, Order::Fil);
    assert_eq!(spec.shrink.raw(), 131_072);
    assert_eq!(spec.shrink_order, Order::Filll);
}

#[test]
fn keeps_mixed_component_orders_independent() {
    let (spec, _next) = scan("0pt plus 3fill minus 4fil x");

    assert_eq!(spec.stretch.raw(), 196_608);
    assert_eq!(spec.stretch_order, Order::Fill);
    assert_eq!(spec.shrink.raw(), 262_144);
    assert_eq!(spec.shrink_order, Order::Fil);
}

#[test]
fn restores_partially_matched_component_keyword_tokens() {
    let (spec, next) = scan("1pt plux 2pt");

    assert_eq!(spec.width.raw(), 65_536);
    assert_eq!(spec.stretch.raw(), 0);
    assert_eq!(next, Some(char_token('p', Catcode::Letter)));
}

#[test]
fn omitted_components_stay_zero() {
    let (spec, next) = scan("3pt x");

    assert_eq!(spec.width.raw(), 196_608);
    assert_eq!(spec.stretch.raw(), 0);
    assert_eq!(spec.shrink.raw(), 0);
    assert_eq!(next, Some(char_token('x', Catcode::Letter)));
}

#[test]
fn scans_internal_skip_values() {
    let mut stores = Universe::new();
    let skip = stores.intern("skip");
    stores.set_meaning(
        skip,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Skip),
    );
    let id = stores.intern_glue(GlueSpec {
        width: Scaled::from_raw(10),
        stretch: Scaled::from_raw(20),
        stretch_order: Order::Fill,
        shrink: Scaled::from_raw(30),
        shrink_order: Order::Fil,
    });
    stores.set_skip(7, id);
    let mut input = InputStack::new(MemoryInput::new("\\skip7 x"));

    let scanned = scan_glue(&mut input, &mut stores, context()).expect("skip should scan");

    assert_eq!(stores.glue(scanned.id()), stores.glue(id));
}

#[test]
fn scans_muglue_with_mu_units() {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new("1mu plus 2fil x"));

    let scanned = scan_muglue(&mut input, &mut stores, context()).expect("muglue should scan");
    let spec = stores.glue(scanned.id());

    assert_eq!(spec.width.raw(), 65_536);
    assert_eq!(spec.stretch.raw(), 131_072);
    assert_eq!(spec.stretch_order, Order::Fil);
}

#[test]
fn scans_internal_muskip_values() {
    let mut stores = Universe::new();
    let muskip = stores.intern("muskip");
    stores.set_meaning(
        muskip,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Muskip),
    );
    let id = stores.intern_glue(GlueSpec {
        width: Scaled::from_raw(10),
        stretch: Scaled::from_raw(20),
        stretch_order: Order::Fill,
        shrink: Scaled::from_raw(30),
        shrink_order: Order::Fil,
    });
    stores.set_muskip(7, id);
    let mut input = InputStack::new(MemoryInput::new("\\muskip7 x"));

    let scanned = scan_muglue(&mut input, &mut stores, context()).expect("muskip should scan");

    assert_eq!(stores.glue(scanned.id()), stores.glue(id));
}

#[test]
fn macro_expanding_to_penalty_recovers_zero_glue_and_replays_command() {
    let mut stores = Universe::new();
    let penalty = stores.intern("penalty");
    stores.set_meaning(
        penalty,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Penalty),
    );
    let nobreak = stores.intern("nobreak");
    let params = stores.intern_token_list(&[]);
    let replacement = stores.intern_token_list(&[Token::Cs(penalty)]);
    stores.set_macro_meaning(
        nobreak,
        MacroMeaning::new(MeaningFlags::EMPTY, params, replacement),
    );
    let mut input = InputStack::new(MemoryInput::new("\\nobreak 10000"));

    let scanned = scan_glue(&mut input, &mut stores, context()).expect("glue scan should recover");
    let replayed = input
        .next_traced_token(&mut stores)
        .expect("token should replay")
        .expect("penalty should remain for execution");

    assert_eq!(stores.glue(scanned.id()).width, Scaled::from_raw(0));
    assert_eq!(
        scanned.diagnostics().collect::<Vec<_>>(),
        vec![
            crate::scan_dimen::DimensionDiagnostic::MissingNumber,
            crate::scan_dimen::DimensionDiagnostic::IllegalUnit {
                inserted: crate::scan_dimen::InsertedUnit::Pt,
            },
        ]
    );
    let diagnostic_records = scanned.diagnostic_records().collect::<Vec<_>>();
    assert_eq!(diagnostic_records[0].1, replayed.origin());
    assert_eq!(diagnostic_records[1].1, replayed.origin());
    assert_eq!(replayed.token(), Some(Token::Cs(penalty)));
}
