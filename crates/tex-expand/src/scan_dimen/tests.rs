use tex_lex::{InputStack, MemoryInput};
use tex_state::Universe;
use tex_state::env::banks::{DimenParam, GlueParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::provenance::{OriginRecord, SourceOrigin};
use tex_state::scaled::{PhysicalUnit, Scaled, round_decimal_fraction, scaled_from_decimal_parts};
use tex_state::token::{Catcode, Token};

use crate::scan_dimen::{
    DimensionDiagnostic, InsertedUnit, ScanDimenOptions, scan_dimen, scan_dimen_with_options,
};

fn scan(input_text: &str) -> (i32, Option<DimensionDiagnostic>, Option<Token>) {
    let mut stores = Universe::new();
    scan_with_stores(input_text, &mut stores)
}

fn scan_with_stores(
    input_text: &str,
    stores: &mut Universe,
) -> (i32, Option<DimensionDiagnostic>, Option<Token>) {
    let mut input = InputStack::new(MemoryInput::new(input_text));
    let scanned = scan_dimen(&mut input, stores).expect("dimension scan should succeed");
    let next = input
        .next_token(stores)
        .expect("remaining token should lex");
    (scanned.value().raw(), scanned.diagnostic(), next)
}

fn scan_coerced(input_text: &str) -> (i32, Option<DimensionDiagnostic>, Option<Token>) {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new(input_text));
    let scanned = scan_dimen_with_options(
        &mut input,
        &mut stores,
        ScanDimenOptions::with_integer_to_sp_coercion(),
    )
    .expect("dimension scan should succeed");
    let next = input
        .next_token(&mut stores)
        .expect("remaining token should lex");
    (scanned.value().raw(), scanned.diagnostic(), next)
}

fn char_token(ch: char, cat: Catcode) -> Token {
    Token::Char { ch, cat }
}

#[test]
fn scans_fractional_decimal_constants_with_dot_and_comma() {
    assert_eq!(scan("1.5pt x").0, 98_304);
    assert_eq!(scan("1,25pt x").0, 81_920);
    assert_eq!(scan(".5pt x").0, 32_768);
    assert_eq!(scan("-.5pt x").0, -32_768);
}

#[test]
fn scans_all_physical_units() {
    for (unit, text) in [
        (PhysicalUnit::Pt, "1pt x"),
        (PhysicalUnit::Pc, "1pc x"),
        (PhysicalUnit::In, "1in x"),
        (PhysicalUnit::Bp, "1bp x"),
        (PhysicalUnit::Cm, "1cm x"),
        (PhysicalUnit::Mm, "1mm x"),
        (PhysicalUnit::Dd, "1dd x"),
        (PhysicalUnit::Cc, "1cc x"),
        (PhysicalUnit::Sp, "1sp x"),
    ] {
        let expected = scaled_from_decimal_parts(1, 0, unit)
            .expect("unit conversion should fit")
            .raw();
        assert_eq!(scan(text).0, expected);
    }
}

#[test]
fn scans_true_units_at_default_magnification_without_rescaling() {
    assert_eq!(scan("1truept x").0, 65_536);
    assert_eq!(scan("1 true in x").0, 4_736_286);
}

#[test]
fn true_units_use_current_mag_before_physical_unit_conversion() {
    let mut stores = Universe::new();
    stores.set_mag(2000);

    assert_eq!(scan_with_stores("1truept x", &mut stores).0, 32_768);
    assert_eq!(scan_with_stores("1truein x", &mut stores).0, 2_368_143);
    assert_eq!(scan_with_stores("1pt x", &mut stores).0, 65_536);
}

#[test]
fn true_unit_scaling_folds_xn_over_d_remainder_into_fraction() {
    let mut stores = Universe::new();
    stores.set_mag(1200);

    assert_eq!(scan_with_stores("1.5truept x", &mut stores).0, 81_920);
    assert_eq!(scan_with_stores("1truesp x", &mut stores).0, 0);
}

#[test]
fn true_units_prepare_and_freeze_magnification() {
    let mut stores = Universe::new();
    stores.set_mag(1200);

    let (value, diagnostic, _next) = scan_with_stores("1truept x", &mut stores);
    assert_eq!(value, 54_613);
    assert_eq!(diagnostic, None);
    assert_eq!(stores.prepared_mag(), Some(1200));

    stores.set_mag(2000);
    let (value, diagnostic, _next) = scan_with_stores("1truept x", &mut stores);
    assert_eq!(value, 54_613);
    assert_eq!(stores.mag(), 1200);
    assert_eq!(
        diagnostic,
        Some(DimensionDiagnostic::IncompatibleMagnification {
            attempted: 2000,
            retained: 1200
        })
    );
    assert_eq!(
        diagnostic.expect("magnification diagnostic").to_string(),
        "Incompatible magnification (2000); the previous value will be retained"
    );
}

#[test]
fn true_units_report_and_coerce_illegal_magnification() {
    let mut stores = Universe::new();
    stores.set_mag(40_000);

    let (value, diagnostic, _next) = scan_with_stores("1truept x", &mut stores);

    assert_eq!(value, 65_536);
    assert_eq!(stores.mag(), 1000);
    assert_eq!(stores.prepared_mag(), Some(1000));
    assert_eq!(
        diagnostic,
        Some(DimensionDiagnostic::IllegalMagnification { attempted: 40_000 })
    );
    assert_eq!(
        diagnostic.expect("magnification diagnostic").to_string(),
        "Illegal magnification has been changed to 1000"
    );
}

#[test]
fn supports_integer_to_sp_coercion_when_requested() {
    let (value, diagnostic, next) = scan_coerced("123 x");

    assert_eq!(value, 123);
    assert_eq!(diagnostic, None);
    assert_eq!(next, Some(char_token('x', Catcode::Letter)));
}

#[test]
fn bare_integer_without_unit_recovers_with_pt() {
    let (value, diagnostic, next) = scan("123 x");

    assert_eq!(value, 123 * Scaled::UNITY);
    assert_eq!(
        diagnostic,
        Some(DimensionDiagnostic::IllegalUnit {
            inserted: InsertedUnit::Pt
        })
    );
    assert_eq!(next, Some(char_token('x', Catcode::Letter)));
}

#[test]
fn scans_supported_internal_dimensions() {
    let mut stores = Universe::new();
    let dimen = stores.intern("dimen");
    stores.set_meaning(
        dimen,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dimen),
    );
    stores.set_dimen(3, Scaled::from_raw(42_000));

    let (value, diagnostic, next) = scan_with_stores("\\dimen3 x", &mut stores);

    assert_eq!(value, 42_000);
    assert_eq!(diagnostic, None);
    assert_eq!(next, Some(char_token('x', Catcode::Letter)));
}

#[test]
fn scans_named_dimension_parameter() {
    let mut stores = Universe::new();
    let hsize = stores.intern("hsize");
    stores.set_meaning(hsize, Meaning::DimenParam(DimenParam::H_SIZE.raw()));
    stores.set_dimen_param(DimenParam::H_SIZE, Scaled::from_raw(42_000));

    let (value, diagnostic, next) = scan_with_stores("\\hsize x", &mut stores);

    assert_eq!(value, 42_000);
    assert_eq!(diagnostic, None);
    assert_eq!(next, Some(char_token('x', Catcode::Letter)));
}

#[test]
fn coerces_named_glue_parameter_width_to_internal_dimension() {
    let mut stores = Universe::new();
    let split_top_skip = stores.intern("splittopskip");
    stores.set_meaning(
        split_top_skip,
        Meaning::GlueParam(GlueParam::SPLIT_TOP_SKIP.raw()),
    );
    let glue = stores.intern_glue(GlueSpec {
        width: Scaled::from_raw(42_000),
        stretch: Scaled::from_raw(7_000),
        stretch_order: Order::Fil,
        shrink: Scaled::from_raw(3_000),
        shrink_order: Order::Normal,
    });
    stores.set_glue_param(GlueParam::SPLIT_TOP_SKIP, glue);

    let (value, diagnostic, next) = scan_with_stores("\\splittopskip x", &mut stores);

    assert_eq!(value, 42_000);
    assert_eq!(diagnostic, None);
    assert_eq!(next, Some(char_token('x', Catcode::Letter)));
}

#[test]
fn coerces_named_skip_register_width_to_internal_dimension() {
    let mut stores = Universe::new();
    let named_skip = stores.intern("namedskip");
    stores.set_meaning(named_skip, Meaning::SkipRegister(42));
    let glue = stores.intern_glue(GlueSpec {
        width: Scaled::from_raw(42_000),
        stretch: Scaled::from_raw(7_000),
        stretch_order: Order::Fil,
        shrink: Scaled::from_raw(3_000),
        shrink_order: Order::Fill,
    });
    stores.set_skip(42, glue);

    let (value, diagnostic, next) = scan_with_stores("-\\namedskip x", &mut stores);

    assert_eq!(value, -42_000);
    assert_eq!(diagnostic, None);
    assert_eq!(next, Some(char_token('x', Catcode::Letter)));
}

#[test]
fn coerces_primitive_skip_register_width_to_internal_dimension() {
    let mut stores = Universe::new();
    let skip = stores.intern("skip");
    stores.set_meaning(
        skip,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Skip),
    );
    let glue = stores.intern_glue(GlueSpec {
        width: Scaled::from_raw(42_000),
        stretch: Scaled::from_raw(7_000),
        stretch_order: Order::Fil,
        shrink: Scaled::from_raw(3_000),
        shrink_order: Order::Normal,
    });
    stores.set_skip(42, glue);

    let (value, diagnostic, next) = scan_with_stores("\\skip42 x", &mut stores);

    assert_eq!(value, 42_000);
    assert_eq!(diagnostic, None);
    assert_eq!(next, Some(char_token('x', Catcode::Letter)));
}

#[test]
fn rejects_muglue_register_as_an_incompatible_dimension() {
    let mut stores = Universe::new();
    let named_muskip = stores.intern("namedmuskip");
    stores.set_meaning(named_muskip, Meaning::MuskipRegister(42));
    let mut input = InputStack::new(MemoryInput::new("\\namedmuskip"));

    let error = scan_dimen(&mut input, &mut stores).expect_err("muglue is not a dimension");

    assert!(matches!(
        error,
        super::ScanDimenError::IncompatibleGlueUnits
    ));
}

#[test]
fn decimal_factor_multiplies_dimension_register_unit_with_tex_rounding() {
    let mut stores = Universe::new();
    let p_unit = stores.intern("punit");
    stores.set_meaning(p_unit, Meaning::DimenRegister(3));
    stores.set_dimen(3, Scaled::from_raw(65_537));

    let (value, diagnostic, next) = scan_with_stores("8.5\\punit x", &mut stores);

    assert_eq!(value, 557_064);
    assert_eq!(diagnostic, None);
    assert_eq!(next, Some(char_token('x', Catcode::Letter)));
}

#[test]
fn decimal_factor_multiplies_primitive_dimension_register_unit() {
    let mut stores = Universe::new();
    let dimen = stores.intern("dimen");
    stores.set_meaning(
        dimen,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dimen),
    );
    stores.set_dimen(3, Scaled::from_raw(42_001));

    let (value, diagnostic, next) = scan_with_stores("8.5\\dimen3 x", &mut stores);

    assert_eq!(value, 357_008);
    assert_eq!(diagnostic, None);
    assert_eq!(next, Some(char_token('x', Catcode::Letter)));
}

#[test]
fn scans_integer_like_internal_values_with_units() {
    let mut stores = Universe::new();
    let count = stores.intern("count");
    stores.set_meaning(
        count,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Count),
    );
    stores.set_count(4, 2);

    assert_eq!(scan_with_stores("\\count4pt x", &mut stores).0, 131_072);
}

#[test]
fn scans_hex_integer_constants_with_units() {
    assert_eq!(scan("\"7Fpt x").0, 127 * Scaled::UNITY);
}

#[test]
fn restores_partially_matched_true_keyword_tokens() {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new("1truxpt"));
    let scanned = scan_dimen(&mut input, &mut stores).expect("bad true keyword recovers");

    assert_eq!(scanned.value().raw(), Scaled::UNITY);
    assert_eq!(
        scanned.diagnostic(),
        Some(DimensionDiagnostic::IllegalUnit {
            inserted: InsertedUnit::Pt
        })
    );
    assert_eq!(
        input.next_token(&mut stores).expect("token should replay"),
        Some(char_token('t', Catcode::Letter))
    );
    assert_eq!(
        input.next_token(&mut stores).expect("token should replay"),
        Some(char_token('r', Catcode::Letter))
    );
    assert_eq!(
        input.next_token(&mut stores).expect("token should replay"),
        Some(char_token('u', Catcode::Letter))
    );
    assert_eq!(
        input.next_token(&mut stores).expect("token should replay"),
        Some(char_token('x', Catcode::Letter))
    );
}

#[test]
fn partially_matched_keyword_pushback_preserves_source_origins() {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new("1truxpt"));
    let scanned = scan_dimen(&mut input, &mut stores).expect("bad true keyword recovers");

    assert_eq!(
        scanned.diagnostic(),
        Some(DimensionDiagnostic::IllegalUnit {
            inserted: InsertedUnit::Pt
        })
    );
    let replayed = input
        .next_traced_token(&mut stores)
        .expect("token should replay")
        .expect("partial keyword should be unread");
    assert_eq!(replayed.token(), Some(char_token('t', Catcode::Letter)));
    assert_eq!(
        stores.origin(replayed.origin()),
        OriginRecord::Source(SourceOrigin::new(tex_state::SourceId::new(0), 1, 1, 1))
    );
}

#[test]
fn missing_number_recovers_zero_then_inserted_pt() {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new("x"));
    let scanned = scan_dimen(&mut input, &mut stores).expect("missing dimension recovers");

    assert_eq!(scanned.value().raw(), 0);
    assert_eq!(
        scanned.diagnostics().collect::<Vec<_>>(),
        vec![
            DimensionDiagnostic::MissingNumber,
            DimensionDiagnostic::IllegalUnit {
                inserted: InsertedUnit::Pt
            },
        ]
    );
    assert_eq!(
        input.next_token(&mut stores).expect("token should replay"),
        Some(char_token('x', Catcode::Letter))
    );
}

#[test]
fn eof_missing_dimension_uses_current_input_origin() {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new(""));
    let scanned = scan_dimen(&mut input, &mut stores).expect("missing dimension recovers");
    let origins = scanned.diagnostic_origins().collect::<Vec<_>>();

    assert_eq!(
        scanned.diagnostics().collect::<Vec<_>>(),
        vec![DimensionDiagnostic::MissingNumber]
    );
    assert_eq!(origins.len(), 1);
    assert!(matches!(stores.origin(origins[0]), OriginRecord::Source(_)));
}

#[test]
fn font_relative_units_scan_as_nullfont_zero_by_default() {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new("1em x"));
    let em = scan_dimen(&mut input, &mut stores).expect("em scans");
    assert_eq!(em.value().raw(), 0);

    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new("1ex x"));
    let ex = scan_dimen(&mut input, &mut stores).expect("ex scans");
    assert_eq!(ex.value().raw(), 0);
}

#[test]
fn reports_dimension_too_large_and_caps_value() {
    let (value, diagnostic, _next) = scan("16384pt x");

    assert_eq!(value, Scaled::MAX_DIMEN.raw());
    assert_eq!(diagnostic, Some(DimensionDiagnostic::TooLarge));
    assert_eq!(
        diagnostic.expect("overflow diagnostic").to_string(),
        "Dimension too large"
    );
}

#[test]
fn scans_values_through_macro_expansion() {
    let mut stores = Universe::new();
    let number = stores.intern("number");
    let replacement = stores.intern_token_list(&[
        char_token('1', Catcode::Other),
        char_token('.', Catcode::Other),
        char_token('5', Catcode::Other),
        char_token('p', Catcode::Letter),
        char_token('t', Catcode::Letter),
    ]);
    let params = stores.intern_token_list(&[]);
    stores.set_macro_meaning(
        number,
        MacroMeaning::new(MeaningFlags::EMPTY, params, replacement),
    );

    assert_eq!(scan_with_stores("\\number x", &mut stores).0, 98_304);
}

#[test]
fn fractional_sp_truncates_to_integer_scaled_points() {
    let expected = scaled_from_decimal_parts(1, round_decimal_fraction(&[5]), PhysicalUnit::Sp)
        .expect("fractional sp conversion fits")
        .raw();

    assert_eq!(scan("1.5sp x").0, expected);
}
