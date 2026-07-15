use tex_lex::{InputStack, MemoryInput};
use tex_state::Universe;
use tex_state::env::banks::{DimenParam, GlueParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::macro_store::{MacroDefinitionProvenance, MacroMeaning};
use tex_state::meaning::{Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::node::{BoxNode, BoxNodeFields, Node, Sign};
use tex_state::provenance::OriginRecord;
use tex_state::scaled::{
    GlueSetRatio, PhysicalUnit, Scaled, round_decimal_fraction, scaled_from_decimal_parts,
};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::scan_dimen::{
    DimensionDiagnostic, InsertedUnit, ScanDimenOptions, scan_dimen, scan_dimen_with_options,
    scan_dimen_with_options_and_context,
};
use crate::{ExpansionContext, ReadBank, ReadDependency, ReadSetRecorder};

fn scan(input_text: &str) -> (i32, Option<DimensionDiagnostic>, Option<Token>) {
    let mut stores = Universe::new();
    scan_with_stores(
        input_text,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
}

fn scan_with_stores(
    input_text: &str,
    stores: &mut tex_state::ExpansionContext<'_>,
) -> (i32, Option<DimensionDiagnostic>, Option<Token>) {
    let mut input = InputStack::new(MemoryInput::new(input_text));
    let scanned = scan_dimen(&mut input, stores, context()).expect("dimension scan should succeed");
    let next = input
        .next_token(stores)
        .expect("remaining token should lex");
    (scanned.value().raw(), scanned.diagnostic(), next)
}

#[test]
fn scans_fontdimen_as_an_internal_dimension() {
    let mut stores = Universe::new();
    let fontdimen = stores.intern("fontdimen");
    stores.set_meaning(
        fontdimen,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::FontDimen),
    );
    let font = stores.intern("testfont");
    stores.set_meaning(font, Meaning::Font(tex_state::font::NULL_FONT));
    stores
        .set_font_dimen(
            tex_state::font::NULL_FONT,
            8,
            Scaled::from_raw(12_345),
            true,
        )
        .expect("font parameter should be writable");

    let (value, diagnostic, next) = scan_with_stores(
        "\\fontdimen8\\testfont!",
        &mut tex_state::ExpansionContext::new(&mut stores),
    );

    assert_eq!(value, 12_345);
    assert_eq!(diagnostic, None);
    assert_eq!(
        next,
        Some(Token::Char {
            ch: '!',
            cat: Catcode::Other
        })
    );
}

#[test]
fn negative_integer_scales_an_internal_dimension_unit() {
    let scanned = super::convert_font_relative_unit(-3, 0, Scaled::from_raw(7))
        .expect("internal dimension conversion");

    assert_eq!(scanned.value().raw(), -21);
}

fn scan_coerced(input_text: &str) -> (i32, Option<DimensionDiagnostic>, Option<Token>) {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new(input_text));
    let scanned = scan_dimen_with_options(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        ScanDimenOptions::with_integer_to_sp_coercion(),
        context(),
    )
    .expect("dimension scan should succeed");
    let next = input
        .next_token(&mut tex_state::ExpansionContext::new(&mut stores))
        .expect("remaining token should lex");
    (scanned.value().raw(), scanned.diagnostic(), next)
}

#[test]
fn dimension_scanner_records_typed_value_and_magnification_dependencies() {
    let mut stores = Universe::new();
    let dimen = stores.intern("measured");
    stores.set_meaning(dimen, Meaning::DimenRegister(3));
    stores.set_dimen(3, Scaled::from_raw(77));
    let mut input = InputStack::new(MemoryInput::new("\\measured"));
    let mut reads = ReadSetRecorder::default();
    let mut expansion = ExpansionContext::new("texput").recording(&mut reads);

    let scanned = scan_dimen_with_options_and_context(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        &mut expansion,
        ScanDimenOptions::STANDARD,
        context(),
    )
    .expect("internal dimension scan");

    assert_eq!(scanned.value(), Scaled::from_raw(77));
    let reads = reads.dependencies().collect::<Vec<_>>();
    assert!(reads.contains(&ReadDependency::Meaning(dimen.symbol().raw())));
    assert!(reads.contains(&ReadDependency::Cell {
        bank: ReadBank::Dimen,
        index: 3,
    }));
    assert!(reads.contains(&ReadDependency::Cell {
        bank: ReadBank::Magnification,
        index: 0,
    }));
}

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

fn install_dimexpr(stores: &mut Universe) {
    let dimexpr = stores.intern("dimexpr");
    stores.set_meaning(
        dimexpr,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::DimExpr),
    );
    let relax = stores.intern("relax");
    stores.set_meaning(relax, Meaning::Relax);
}

#[test]
fn dimexpr_obeys_precedence_parentheses_and_relax_termination() {
    let mut stores = Universe::new();
    install_dimexpr(&mut stores);

    let (value, diagnostic, next) = scan_with_stores(
        "\\dimexpr 1pt+2pt*3\\relax X",
        &mut tex_state::ExpansionContext::new(&mut stores),
    );
    assert_eq!(value, 7 * Scaled::UNITY);
    assert_eq!(diagnostic, None);
    assert_eq!(next, Some(char_token('X', Catcode::Letter)));

    let (value, _, _) = scan_with_stores(
        "\\dimexpr (1pt+2pt)*3\\relax",
        &mut tex_state::ExpansionContext::new(&mut stores),
    );
    assert_eq!(value, 9 * Scaled::UNITY);
}

#[test]
fn dimexpr_uses_etex_rounded_division_and_combined_scaling() {
    let mut stores = Universe::new();
    install_dimexpr(&mut stores);

    assert_eq!(
        scan_with_stores(
            "\\dimexpr 1pt/3\\relax",
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .0,
        21_845
    );
    assert_eq!(
        scan_with_stores(
            "\\dimexpr 1pt*10/3\\relax",
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .0,
        218_453
    );
}

#[test]
fn dimexpr_recovers_to_zero_after_intermediate_overflow() {
    let mut stores = Universe::new();
    install_dimexpr(&mut stores);

    let (value, diagnostic, _) = scan_with_stores(
        "\\dimexpr 16383pt+1pt\\relax",
        &mut tex_state::ExpansionContext::new(&mut stores),
    );
    assert_eq!(value, 0);
    assert_eq!(diagnostic, Some(DimensionDiagnostic::TooLarge));
}

#[test]
fn glue_component_enquiries_return_raw_scaled_dimensions() {
    let mut stores = Universe::new();
    for (name, primitive) in [
        ("gluestretch", UnexpandablePrimitive::GlueStretch),
        ("glueshrink", UnexpandablePrimitive::GlueShrink),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }

    assert_eq!(
        scan_with_stores(
            "\\gluestretch 0pt plus 2fill",
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .0,
        2 * Scaled::UNITY
    );
    assert_eq!(
        scan_with_stores(
            "\\glueshrink 0pt minus 1.5fil",
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .0,
        98_304
    );
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

    assert_eq!(
        scan_with_stores(
            "1truept x",
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .0,
        32_768
    );
    assert_eq!(
        scan_with_stores(
            "1truein x",
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .0,
        2_368_143
    );
    assert_eq!(
        scan_with_stores("1pt x", &mut tex_state::ExpansionContext::new(&mut stores)).0,
        65_536
    );
}

#[test]
fn true_physical_units_match_tex_fixed_point_order_at_mag_two_thousand() {
    let cases = [
        ("12.truept x", 393_216),
        ("12.truein x", 28_417_720),
        ("12.truepc x", 4_718_592),
        ("12.truecm x", 11_188_078),
        ("12.truemm x", 1_118_807),
        ("12.truebp x", 394_690),
        ("12.truedd x", 420_744),
        ("12.truecc x", 5_048_934),
    ];

    for (source, expected) in cases {
        let mut stores = Universe::new();
        stores.set_mag(2000);
        assert_eq!(
            scan_with_stores(source, &mut tex_state::ExpansionContext::new(&mut stores)).0,
            expected,
            "{source}"
        );
    }

    let mut stores = Universe::new();
    stores.set_mag(2000);
    assert_eq!(
        scan_with_stores(
            "-12.truedd x",
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .0,
        -420_744
    );
}

#[test]
fn true_unit_scaling_folds_xn_over_d_remainder_into_fraction() {
    let mut stores = Universe::new();
    stores.set_mag(1200);

    assert_eq!(
        scan_with_stores(
            "1.5truept x",
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .0,
        81_920
    );
    assert_eq!(
        scan_with_stores(
            "1truesp x",
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .0,
        0
    );
}

#[test]
fn true_unit_scaling_widens_legal_maximum_mag_fraction_arithmetic() {
    let mut stores = Universe::new();
    stores.set_mag(32_768);

    // TeX rounds the 17 retained nines to one whole fractional unit. Combined
    // with xn_over_d's remainder, the pre-division numerator exceeds i32.
    assert_eq!(
        scan_with_stores(
            "4095.99999999999999999truept x",
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .0,
        125 * Scaled::UNITY
    );
}

#[test]
fn true_units_prepare_and_freeze_magnification() {
    let mut stores = Universe::new();
    stores.set_mag(1200);

    let (value, diagnostic, _next) = scan_with_stores(
        "1truept x",
        &mut tex_state::ExpansionContext::new(&mut stores),
    );
    assert_eq!(value, 54_613);
    assert_eq!(diagnostic, None);
    assert_eq!(stores.prepared_mag(), Some(1200));

    stores.set_mag(2000);
    let (value, diagnostic, _next) = scan_with_stores(
        "1truept x",
        &mut tex_state::ExpansionContext::new(&mut stores),
    );
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

    let (value, diagnostic, _next) = scan_with_stores(
        "1truept x",
        &mut tex_state::ExpansionContext::new(&mut stores),
    );

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

    let (value, diagnostic, next) = scan_with_stores(
        "\\dimen3 x",
        &mut tex_state::ExpansionContext::new(&mut stores),
    );

    assert_eq!(value, 42_000);
    assert_eq!(diagnostic, None);
    assert_eq!(next, Some(char_token('x', Catcode::Letter)));
}

#[test]
fn scales_box_dimensions_used_as_internal_units() {
    let mut stores = Universe::new();
    let dp = stores.intern("dp");
    stores.set_meaning(
        dp,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dp),
    );
    let empty = stores.freeze_node_list(&[]);
    let boxed = stores.freeze_node_list(&[Node::VList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(-559_403),
        shift: Scaled::from_raw(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: empty,
    }))]);
    stores.set_box_reg(3, boxed);

    let (value, diagnostic, next) = scan_with_stores(
        "2\\dp3 x",
        &mut tex_state::ExpansionContext::new(&mut stores),
    );

    assert_eq!(value, -1_118_806);
    assert_eq!(diagnostic, None);
    assert_eq!(next, Some(char_token('x', Catcode::Letter)));
}

#[test]
fn scans_named_dimension_parameter() {
    let mut stores = Universe::new();
    let hsize = stores.intern("hsize");
    stores.set_meaning(hsize, Meaning::DimenParam(DimenParam::H_SIZE.raw()));
    stores.set_dimen_param(DimenParam::H_SIZE, Scaled::from_raw(42_000));

    let (value, diagnostic, next) = scan_with_stores(
        "\\hsize x",
        &mut tex_state::ExpansionContext::new(&mut stores),
    );

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

    let (value, diagnostic, next) = scan_with_stores(
        "\\splittopskip x",
        &mut tex_state::ExpansionContext::new(&mut stores),
    );

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

    let (value, diagnostic, next) = scan_with_stores(
        "-\\namedskip x",
        &mut tex_state::ExpansionContext::new(&mut stores),
    );

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

    let (value, diagnostic, next) = scan_with_stores(
        "\\skip42 x",
        &mut tex_state::ExpansionContext::new(&mut stores),
    );

    assert_eq!(value, 42_000);
    assert_eq!(diagnostic, None);
    assert_eq!(next, Some(char_token('x', Catcode::Letter)));
}

#[test]
fn coerces_muglue_register_width_with_incompatible_units_diagnostic() {
    let mut stores = Universe::new();
    let named_muskip = stores.intern("namedmuskip");
    stores.set_meaning(named_muskip, Meaning::MuskipRegister(42));
    let glue = stores.intern_glue(GlueSpec {
        width: Scaled::from_raw(42_000),
        ..GlueSpec::ZERO
    });
    stores.set_muskip(42, glue);
    let mut input = InputStack::new(MemoryInput::new("\\namedmuskip"));

    let scanned = scan_dimen(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        context(),
    )
    .expect("TeX assumes 1mu=1pt for mixed glue units");

    assert_eq!(scanned.value().raw(), 42_000);
    assert_eq!(
        scanned.diagnostic(),
        Some(super::DimensionDiagnostic::IncompatibleGlueUnits)
    );
}

#[test]
fn decimal_factor_multiplies_dimension_register_unit_with_tex_rounding() {
    let mut stores = Universe::new();
    let p_unit = stores.intern("punit");
    stores.set_meaning(p_unit, Meaning::DimenRegister(3));
    stores.set_dimen(3, Scaled::from_raw(65_537));

    let (value, diagnostic, next) = scan_with_stores(
        "8.5\\punit x",
        &mut tex_state::ExpansionContext::new(&mut stores),
    );

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

    let (value, diagnostic, next) = scan_with_stores(
        "8.5\\dimen3 x",
        &mut tex_state::ExpansionContext::new(&mut stores),
    );

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

    assert_eq!(
        scan_with_stores(
            "\\count4pt x",
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .0,
        131_072
    );
}

#[test]
fn decimal_factor_accepts_internal_integer_as_scaled_point_unit() {
    let mut stores = Universe::new();
    let mag = stores.intern("mag");
    stores.set_meaning(
        mag,
        Meaning::IntParam(tex_state::env::banks::IntParam::MAG.raw()),
    );
    stores.set_mag(2000);

    assert_eq!(
        scan_with_stores(
            ".5\\mag x",
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .0,
        1000
    );
}

#[test]
fn scans_hex_integer_constants_with_units() {
    assert_eq!(scan("\"7Fpt x").0, 127 * Scaled::UNITY);
}

#[test]
fn restores_partially_matched_true_keyword_tokens() {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new("1truxpt"));
    let scanned = scan_dimen(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        context(),
    )
    .expect("bad true keyword recovers");

    assert_eq!(scanned.value().raw(), Scaled::UNITY);
    assert_eq!(
        scanned.diagnostic(),
        Some(DimensionDiagnostic::IllegalUnit {
            inserted: InsertedUnit::Pt
        })
    );
    assert_eq!(
        input
            .next_token(&mut tex_state::ExpansionContext::new(&mut stores))
            .expect("token should replay"),
        Some(char_token('t', Catcode::Letter))
    );
    assert_eq!(
        input
            .next_token(&mut tex_state::ExpansionContext::new(&mut stores))
            .expect("token should replay"),
        Some(char_token('r', Catcode::Letter))
    );
    assert_eq!(
        input
            .next_token(&mut tex_state::ExpansionContext::new(&mut stores))
            .expect("token should replay"),
        Some(char_token('u', Catcode::Letter))
    );
    assert_eq!(
        input
            .next_token(&mut tex_state::ExpansionContext::new(&mut stores))
            .expect("token should replay"),
        Some(char_token('x', Catcode::Letter))
    );
}

#[test]
fn partially_matched_keyword_pushback_preserves_source_origins() {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new("1truxpt"));
    let scanned = scan_dimen(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        context(),
    )
    .expect("bad true keyword recovers");

    assert_eq!(
        scanned.diagnostic(),
        Some(DimensionDiagnostic::IllegalUnit {
            inserted: InsertedUnit::Pt
        })
    );
    let replayed = input
        .next_traced_token(&mut tex_state::ExpansionContext::new(&mut stores))
        .expect("token should replay")
        .expect("partial keyword should be unread");
    assert_eq!(replayed.token(), Some(char_token('t', Catcode::Letter)));
    let OriginRecord::SourceSpan(span) = stores.origin(replayed.origin()) else {
        panic!("ordinary source token must retain a logical source span");
    };
    assert_eq!(
        span.lo(),
        stores
            .source_position(tex_state::SourceId::new(0), 1)
            .expect("source position stays live")
    );
}

#[test]
fn missing_number_recovers_zero_then_inserted_pt() {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new("x"));
    let scanned = scan_dimen(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        context(),
    )
    .expect("missing dimension recovers");

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
        input
            .next_token(&mut tex_state::ExpansionContext::new(&mut stores))
            .expect("token should replay"),
        Some(char_token('x', Catcode::Letter))
    );
}

#[test]
fn expanded_command_recovery_keeps_replay_frontier_origin() {
    let mut stores = Universe::new();
    let penalty = stores.intern("penalty");
    stores.set_meaning(
        penalty,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Penalty),
    );
    let nobreak = stores.intern("nobreak");
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[Token::Cs(penalty.symbol())]);
    let definition_origin = stores.source_origin(tex_state::SourceId::new(7), 10, 2, 1);
    let body_origin = stores.source_origin(tex_state::SourceId::new(7), 21, 2, 12);
    let body_origins = stores.allocate_origin_list(&[body_origin]);
    stores.set_macro_meaning_with_provenance(
        nobreak,
        MacroMeaning::new(MeaningFlags::EMPTY, params, body),
        MacroDefinitionProvenance::new(
            definition_origin,
            tex_state::ids::OriginListId::EMPTY,
            body_origins,
        ),
    );

    let baseline = stores.snapshot();
    let baseline_stats = stores.provenance_stats();
    let mut input = InputStack::new(MemoryInput::new("\\nobreak"));
    let scanned = scan_dimen(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        context(),
    )
    .expect("missing dimension recovers");

    assert_eq!(
        scanned.diagnostics().collect::<Vec<_>>(),
        vec![
            DimensionDiagnostic::MissingNumber,
            DimensionDiagnostic::IllegalUnit {
                inserted: InsertedUnit::Pt,
            },
        ]
    );
    assert_eq!(
        scanned.diagnostic_origins().collect::<Vec<_>>(),
        vec![body_origin, body_origin]
    );
    let replayed = input
        .next_traced_token(&mut tex_state::ExpansionContext::new(&mut stores))
        .expect("replay should succeed")
        .expect("rejected command should remain at the replay frontier");
    assert_eq!(replayed.token(), Some(Token::Cs(penalty.symbol())));
    assert_eq!(replayed.origin(), body_origin);

    verify_shadow(&stores);
    let after = stores.snapshot();
    assert_eq!(
        after.state_hash(),
        baseline.state_hash(),
        "diagnostic provenance and scanner pushback must be hash-neutral"
    );
    stores.rollback(&baseline);
    assert_eq!(stores.provenance_stats(), baseline_stats);
    verify_shadow(&stores);
}

#[test]
fn eof_missing_dimension_uses_caller_context_origin() {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new(""));
    let origin = stores.source_origin(tex_state::SourceId::new(7), 12, 3, 4);
    let caller = TracedTokenWord::pack(
        Token::Char {
            ch: '=',
            cat: Catcode::Other,
        },
        origin,
    );
    let scanned = scan_dimen(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        caller,
    )
    .expect("missing dimension recovers");
    let origins = scanned.diagnostic_origins().collect::<Vec<_>>();

    assert_eq!(
        scanned.diagnostics().collect::<Vec<_>>(),
        vec![DimensionDiagnostic::MissingNumber]
    );
    assert_eq!(origins.len(), 1);
    assert_eq!(origins, vec![origin]);
}

#[cfg(feature = "shadow")]
fn verify_shadow(stores: &Universe) {
    stores.verify_shadow();
}

#[cfg(not(feature = "shadow"))]
fn verify_shadow(_: &Universe) {}

#[test]
fn font_relative_units_scan_as_nullfont_zero_by_default() {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new("1em x"));
    let em = scan_dimen(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        context(),
    )
    .expect("em scans");
    assert_eq!(em.value().raw(), 0);

    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new("1ex x"));
    let ex = scan_dimen(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        context(),
    )
    .expect("ex scans");
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

    assert_eq!(
        scan_with_stores(
            "\\number x",
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .0,
        98_304
    );
}

#[test]
fn fractional_sp_truncates_to_integer_scaled_points() {
    let expected = scaled_from_decimal_parts(1, round_decimal_fraction(&[5]), PhysicalUnit::Sp)
        .expect("fractional sp conversion fits")
        .raw();

    assert_eq!(scan("1.5sp x").0, expected);
}

#[test]
fn dimexpr_matches_etex_precedence_parentheses_and_rounding() {
    let mut stores = Universe::new();
    for (name, meaning) in [
        (
            "dimexpr",
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::DimExpr),
        ),
        ("relax", Meaning::Relax),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, meaning);
    }

    assert_eq!(
        scan_with_stores(
            "\\dimexpr1pt+2pt*3\\relax",
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .0,
        7 * 65_536
    );
    assert_eq!(
        scan_with_stores(
            "\\dimexpr(1pt+2pt)*3\\relax",
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .0,
        9 * 65_536
    );
    assert_eq!(
        scan_with_stores(
            "\\dimexpr5sp/2\\relax",
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .0,
        3
    );
    assert_eq!(
        scan_with_stores(
            "\\dimexpr-5sp/2\\relax",
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .0,
        -3
    );
}

#[test]
fn dimexpr_coerces_a_primitive_skip_width_used_as_a_numeric_factor() {
    let mut stores = Universe::new();
    for (name, meaning) in [
        (
            "dimexpr",
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::DimExpr),
        ),
        (
            "skip",
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Skip),
        ),
        ("relax", Meaning::Relax),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, meaning);
    }
    let glue = stores.intern_glue(GlueSpec {
        width: Scaled::from_raw(3 * 65_536),
        ..GlueSpec::ZERO
    });
    stores.set_skip(44, glue);

    assert_eq!(
        scan_with_stores(
            "\\dimexpr1sp*\\skip44\\relax",
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .0,
        3 * 65_536
    );
}
