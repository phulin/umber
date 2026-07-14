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
    let scanned = scan_glue(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        context(),
    )
    .expect("glue scan should succeed");
    let spec = stores.glue(scanned.id());
    let next = input
        .next_token(&mut tex_state::ExpansionContext::new(&mut stores))
        .expect("remaining token should lex");
    (spec, next)
}

fn install_glue_expressions(stores: &mut Universe) {
    for (name, primitive) in [
        ("glueexpr", UnexpandablePrimitive::GlueExpr),
        ("muexpr", UnexpandablePrimitive::MuExpr),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    let relax = stores.intern("relax");
    stores.set_meaning(relax, Meaning::Relax);
}

#[test]
fn glueexpr_preserves_precedence_orders_and_relax_termination() {
    let mut stores = Universe::new();
    install_glue_expressions(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\glueexpr(1pt plus 2fil minus 3pt)+4pt plus 5fill\\relax X",
    ));

    let scanned = scan_glue(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        context(),
    )
    .expect("glue expression scans");
    let spec = stores.glue(scanned.id());

    assert_eq!(spec.width.raw(), 5 * Scaled::UNITY);
    assert_eq!(spec.stretch.raw(), 5 * Scaled::UNITY);
    assert_eq!(spec.stretch_order, Order::Fill);
    assert_eq!(spec.shrink.raw(), 3 * Scaled::UNITY);
    assert_eq!(spec.shrink_order, Order::Normal);
    assert_eq!(
        input
            .next_token(&mut tex_state::ExpansionContext::new(&mut stores))
            .expect("terminator remains"),
        Some(char_token('X', Catcode::Letter))
    );
}

#[test]
fn muexpr_scales_every_component_with_etex_rounding() {
    let mut stores = Universe::new();
    install_glue_expressions(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\muexpr 2mu plus 3fil minus 1mu*3/2\\relax",
    ));

    let scanned = scan_muglue(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        context(),
    )
    .expect("mu expression scans");
    let spec = stores.glue(scanned.id());

    assert_eq!(spec.width.raw(), 3 * Scaled::UNITY);
    assert_eq!(spec.stretch.raw(), 294_912);
    assert_eq!(spec.stretch_order, Order::Fil);
    assert_eq!(spec.shrink.raw(), 98_304);
    assert_eq!(spec.shrink_order, Order::Normal);
}

#[test]
fn glueexpr_retains_component_order_when_scaling_to_zero() {
    let mut stores = Universe::new();
    install_glue_expressions(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\glueexpr 1pt plus 1fil*0\\relax"));

    let scanned = scan_glue(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        context(),
    )
    .expect("glue expression scans");
    let spec = stores.glue(scanned.id());

    assert_eq!(spec.width.raw(), 0);
    assert_eq!(spec.stretch.raw(), 0);
    assert_eq!(spec.stretch_order, Order::Fil);

    let mut input = InputStack::new(MemoryInput::new("\\glueexpr 0pt plus 0fil+0pt\\relax"));
    let scanned = scan_glue(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        context(),
    )
    .expect("glue expression scans");
    assert_eq!(stores.glue(scanned.id()).stretch_order, Order::Normal);
}

#[test]
fn glue_unit_conversions_preserve_all_components_and_orders() {
    let mut stores = Universe::new();
    for (name, primitive) in [
        ("gluetomu", UnexpandablePrimitive::GlueToMu),
        ("mutoglue", UnexpandablePrimitive::MuToGlue),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }

    let mut input = InputStack::new(MemoryInput::new("\\gluetomu 2pt plus 3fill minus 4fil"));
    let converted = scan_muglue(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        context(),
    )
    .expect("glue converts to mu");
    assert_eq!(
        stores.glue(converted.id()),
        GlueSpec {
            width: Scaled::from_raw(2 * Scaled::UNITY),
            stretch: Scaled::from_raw(3 * Scaled::UNITY),
            stretch_order: Order::Fill,
            shrink: Scaled::from_raw(4 * Scaled::UNITY),
            shrink_order: Order::Fil,
        }
    );

    let mut input = InputStack::new(MemoryInput::new("\\mutoglue 5mu plus 6fil minus 7mu"));
    let converted = scan_glue(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        context(),
    )
    .expect("mu converts to glue");
    let spec = stores.glue(converted.id());
    assert_eq!(spec.width.raw(), 5 * Scaled::UNITY);
    assert_eq!(spec.stretch.raw(), 6 * Scaled::UNITY);
    assert_eq!(spec.stretch_order, Order::Fil);
    assert_eq!(spec.shrink.raw(), 7 * Scaled::UNITY);
}

fn install_exprs(stores: &mut Universe) {
    for (name, primitive) in [
        ("glueexpr", UnexpandablePrimitive::GlueExpr),
        ("muexpr", UnexpandablePrimitive::MuExpr),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    let relax = stores.intern("relax");
    stores.set_meaning(relax, Meaning::Relax);
}

#[test]
fn glueexpr_matches_etex_order_dominance_and_combined_scaling() {
    let mut stores = Universe::new();
    install_exprs(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\glueexpr(1pt plus 2fil+3pt plus 4fil)*3/2\\relax",
    ));
    let scanned = scan_glue(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        context(),
    )
    .expect("glue expression");
    let spec = stores.glue(scanned.id());
    assert_eq!(spec.width.raw(), 6 * Scaled::UNITY);
    assert_eq!(spec.stretch.raw(), 9 * Scaled::UNITY);
    assert_eq!(spec.stretch_order, Order::Fil);

    let mut input = InputStack::new(MemoryInput::new(
        "\\muexpr1mu plus 2fil+3mu plus 4fill\\relax",
    ));
    let scanned = scan_muglue(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        context(),
    )
    .expect("muglue expression");
    let spec = stores.glue(scanned.id());
    assert_eq!(spec.width.raw(), 4 * Scaled::UNITY);
    assert_eq!(spec.stretch.raw(), 4 * Scaled::UNITY);
    assert_eq!(spec.stretch_order, Order::Fill);
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

    let scanned = scan_glue(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        context(),
    )
    .expect("skip should scan");

    assert_eq!(stores.glue(scanned.id()), stores.glue(id));
}

#[test]
fn scans_muglue_with_mu_units() {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new("1mu plus 2fil x"));

    let scanned = scan_muglue(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        context(),
    )
    .expect("muglue should scan");
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

    let scanned = scan_muglue(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        context(),
    )
    .expect("muskip should scan");

    assert_eq!(stores.glue(scanned.id()), stores.glue(id));
}

#[test]
fn scans_internal_muskip_widths_as_mu_components() {
    let mut stores = Universe::new();
    let muskip = stores.intern("muskip");
    stores.set_meaning(
        muskip,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Muskip),
    );
    let alias = stores.intern("alias");
    stores.set_meaning(alias, Meaning::MuskipRegister(3));
    let id = stores.intern_glue(GlueSpec {
        width: Scaled::from_raw(2 * Scaled::UNITY),
        ..GlueSpec::ZERO
    });
    stores.set_muskip(3, id);
    let mut input = InputStack::new(MemoryInput::new("5mu plus \\muskip3 minus .5\\alias"));

    let scanned = scan_muglue(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        context(),
    )
    .expect("muglue should scan");
    let spec = stores.glue(scanned.id());
    assert_eq!(spec.width.raw(), 5 * Scaled::UNITY);
    assert_eq!(spec.stretch.raw(), 2 * Scaled::UNITY);
    assert_eq!(spec.shrink.raw(), Scaled::UNITY);
}

#[test]
fn ordinary_glue_coerces_muskip_component_width_with_diagnostic() {
    let mut stores = Universe::new();
    let thin = stores.intern("thin");
    stores.set_meaning(thin, Meaning::MuskipRegister(3));
    let id = stores.intern_glue(GlueSpec {
        width: Scaled::from_raw(2 * Scaled::UNITY),
        ..GlueSpec::ZERO
    });
    stores.set_muskip(3, id);
    let mut input = InputStack::new(MemoryInput::new("1pt plus \\thin"));

    let scanned = scan_glue(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        context(),
    )
    .expect("glue should scan");
    assert_eq!(stores.glue(scanned.id()).stretch.raw(), 2 * Scaled::UNITY);
    assert_eq!(
        scanned.diagnostics().collect::<Vec<_>>(),
        vec![crate::scan_dimen::DimensionDiagnostic::IncompatibleGlueUnits]
    );
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
    let replacement = stores.intern_token_list(&[Token::Cs(penalty.symbol())]);
    stores.set_macro_meaning(
        nobreak,
        MacroMeaning::new(MeaningFlags::EMPTY, params, replacement),
    );
    let mut input = InputStack::new(MemoryInput::new("\\nobreak 10000"));

    let scanned = scan_glue(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        context(),
    )
    .expect("glue scan should recover");
    let replayed = input
        .next_traced_token(&mut tex_state::ExpansionContext::new(&mut stores))
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
    assert_eq!(replayed.token(), Some(Token::Cs(penalty.symbol())));
}
