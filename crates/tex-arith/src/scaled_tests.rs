use crate::{
    ArithmeticError, DimensionError, FontSizeSpec, GLUE_SET_RATIO_SCALE, GlueSetRatio,
    GlueSetRatioError, PhysicalUnit, Scaled, TfmConversionError, WideScaled, XOverN, XnOverD,
    font_units_to_scaled, half, mult_and_add, nx_plus_y, round_decimal_fraction, saturating_add,
    saturating_mul, saturating_sub, scale_true_dimension_parts, scaled_from_decimal_parts,
    text_accent_delta, tfm_design_size_from_fix_word, tfm_fix_word_to_scaled, tfm_font_size,
    tfm_slant_fix_word_to_scaled_ratio, x_over_n, xn_over_d,
};

#[test]
fn scaled_add_sub_neg_and_checked_variants() {
    let a = Scaled::from_raw(10);
    let b = Scaled::from_raw(3);

    assert_eq!((a + b).raw(), 13);
    assert_eq!((a - b).raw(), 7);
    assert_eq!((-b).raw(), -3);
    assert_eq!(Scaled::MIN.raw(), i32::MIN);
    assert_eq!(Scaled::MAX.raw(), i32::MAX);
    assert_eq!(Scaled::MAX_DIMEN.raw(), (1 << 30) - 1);

    assert_eq!(Scaled::MAX.checked_add(Scaled::from_raw(1)), None);
    assert_eq!(Scaled::from_raw(i32::MIN).checked_neg(), None);
}

#[test]
fn wide_scaled_preserves_prefix_information_past_i32() {
    let max = WideScaled::from_scaled(Scaled::MAX);
    let total = max
        .checked_add(WideScaled::from_scaled(Scaled::from_raw(123)))
        .expect("small wide addition fits i64");
    assert_eq!(total.raw(), i64::from(i32::MAX) + 123);
    assert_eq!(total.to_scaled(), None);
    assert_eq!(
        total
            .checked_sub(max)
            .expect("small wide subtraction fits i64")
            .to_scaled(),
        Some(Scaled::from_raw(123))
    );
}

#[test]
fn saturating_scaled_arithmetic_uses_widened_intermediates() {
    assert_eq!(
        saturating_add(Scaled::MAX, Scaled::from_raw(1)),
        Scaled::MAX
    );
    assert_eq!(
        saturating_sub(Scaled::MIN, Scaled::from_raw(1)),
        Scaled::MIN
    );
    assert_eq!(saturating_mul(2, Scaled::MAX), Scaled::MAX);
    assert_eq!(saturating_mul(-2, Scaled::MAX), Scaled::MIN);
    assert_eq!(
        saturating_add(Scaled::MAX_DIMEN, Scaled::from_raw(-1)),
        Scaled::from_raw(Scaled::MAX_DIMEN.raw() - 1)
    );
}

#[test]
fn text_accent_delta_matches_tex_rounding_for_signed_ties_and_products() {
    let zero = Scaled::from_raw(0);
    assert_eq!(
        text_accent_delta(
            Scaled::from_raw(10),
            Scaled::from_raw(1),
            zero,
            zero,
            zero,
            zero,
        ),
        Scaled::from_raw(5)
    );
    assert_eq!(
        text_accent_delta(
            Scaled::from_raw(1),
            Scaled::from_raw(10),
            zero,
            zero,
            zero,
            zero,
        ),
        Scaled::from_raw(-5)
    );
    assert_eq!(
        text_accent_delta(
            zero,
            zero,
            Scaled::from_raw(1),
            Scaled::from_raw(Scaled::UNITY / 2),
            zero,
            zero,
        ),
        Scaled::from_raw(1)
    );
    assert_eq!(
        text_accent_delta(
            zero,
            zero,
            Scaled::from_raw(1),
            Scaled::from_raw(-Scaled::UNITY / 2),
            zero,
            zero,
        ),
        Scaled::from_raw(-1)
    );
}

#[test]
fn true_dimension_scaling_handles_legal_wide_fraction_numerator() {
    // 4095 * 1000 leaves a large remainder modulo the maximum legal mag;
    // combining it with the largest fraction exceeds i32 before division.
    let (integer, fraction) =
        scale_true_dimension_parts(4095, Scaled::UNITY, 32_768).expect("legal scaling fits");
    assert_eq!((integer, fraction), (125, 0));

    assert_eq!(
        scale_true_dimension_parts(12, 34_567, 1000),
        Ok((12, 34_567))
    );
    assert_eq!(
        scale_true_dimension_parts(0, Scaled::UNITY, 1),
        Ok((1000, 0))
    );
}

#[test]
fn glue_set_ratio_preserves_exact_reduced_fraction() {
    assert_eq!(
        GlueSetRatio::from_scaled_ratio(Scaled::from_raw(1), Scaled::from_raw(2)),
        GlueSetRatio::from_raw(GLUE_SET_RATIO_SCALE / 2)
    );
    assert_eq!(
        GlueSetRatio::from_scaled_ratio(Scaled::from_raw(1), Scaled::from_raw(3)),
        GlueSetRatio::from_ratio_parts(1, 3)
    );
    assert_eq!(
        GlueSetRatio::from_scaled_ratio(Scaled::from_raw(i32::MAX), Scaled::from_raw(1)),
        GlueSetRatio::from_ratio_parts(i32::MAX, 1)
    );
}

#[test]
fn glue_set_ratio_deserialization_reconstructs_only_canonical_values() {
    fn wire(numerator: i32, denominator: i32) -> Vec<u8> {
        bincode::serialize(&(numerator, denominator)).expect("ratio wire serializes")
    }

    assert_eq!(
        bincode::deserialize::<GlueSetRatio>(&wire(6, 8)).expect("reducible ratio decodes"),
        GlueSetRatio::from_ratio_parts(3, 4)
    );
    assert_eq!(
        bincode::deserialize::<GlueSetRatio>(&wire(-6, 8)).expect("sign normalizes"),
        GlueSetRatio::from_ratio_parts(3, 4)
    );
    assert_eq!(
        bincode::deserialize::<GlueSetRatio>(&wire(0, 99)).expect("zero ratio decodes"),
        GlueSetRatio::ZERO
    );
    for malformed in [wire(1, 0), wire(1, -1), wire(i32::MIN, 1)] {
        assert!(bincode::deserialize::<GlueSetRatio>(&malformed).is_err());
    }
    assert_eq!(
        GlueSetRatio::try_from_ratio_parts(1, 0),
        Err(GlueSetRatioError::NonPositiveDenominator)
    );
    assert_eq!(
        GlueSetRatio::try_from_ratio_parts(i32::MIN, 1),
        Err(GlueSetRatioError::UnrepresentableNumerator)
    );
}

#[test]
fn canonical_glue_set_ratio_round_trips_with_identical_hash() {
    use std::hash::{Hash, Hasher};

    let ratio = GlueSetRatio::from_ratio_parts(37, 101);
    let bytes = bincode::serialize(&ratio).expect("canonical ratio serializes");
    let restored: GlueSetRatio = bincode::deserialize(&bytes).expect("canonical ratio decodes");
    assert_eq!(restored, ratio);
    assert_eq!(
        bincode::serialize(&restored).expect("restored ratio serializes"),
        bytes
    );

    let hash = |value: GlueSetRatio| {
        let mut hasher = ahash::AHasher::default();
        value.hash(&mut hasher);
        hasher.finish()
    };
    assert_eq!(hash(restored), hash(ratio));
}

#[test]
fn xn_over_d_matches_tex_remainder_and_overflow_rules() {
    assert_eq!(
        xn_over_d(Scaled::from_raw(1), 7_227, 100).expect("1in integer conversion fits"),
        XnOverD {
            quotient: Scaled::from_raw(72),
            remainder: 27,
        }
    );
    assert_eq!(
        xn_over_d(Scaled::from_raw(-1), 7_227, 100).expect("negative 1in integer conversion fits"),
        XnOverD {
            quotient: Scaled::from_raw(-72),
            remainder: -27,
        }
    );
    assert_eq!(
        xn_over_d(Scaled::MAX_DIMEN, Scaled::UNITY, 1),
        Err(DimensionError::TooLarge)
    );
}

#[test]
fn font_unit_scaling_rounds_half_away_from_zero_and_checks_bounds() {
    assert_eq!(
        font_units_to_scaled(1, Scaled::from_raw(5), 2),
        Ok(Scaled::from_raw(3))
    );
    assert_eq!(
        font_units_to_scaled(-1, Scaled::from_raw(5), 2),
        Ok(Scaled::from_raw(-3))
    );
    assert_eq!(
        font_units_to_scaled(1, Scaled::from_raw(4), 3),
        Ok(Scaled::from_raw(1))
    );
    assert_eq!(
        font_units_to_scaled(i32::MAX, Scaled::from_raw(i32::MAX), 1),
        Err(ArithmeticError::Overflow)
    );
    assert_eq!(
        font_units_to_scaled(1, Scaled::from_raw(1), 0),
        Err(ArithmeticError::DivisionByZero)
    );
}

#[test]
fn half_matches_tex_signed_odd_convention() {
    let cases = [
        (0, 0),
        (1, 1),
        (2, 1),
        (3, 2),
        (-1, 0),
        (-2, -1),
        (-3, -1),
        (-5, -2),
        (i32::MAX, 1_073_741_824),
        (i32::MIN, -1_073_741_824),
    ];
    for (input, expected) in cases {
        assert_eq!(half(input), expected, "half({input})");
    }
}

#[test]
fn x_over_n_matches_tex_sign_and_remainder_rules() {
    let cases = [
        (7, 3, 2, 1),
        (-7, 3, -2, -1),
        (7, -3, -2, 1),
        (-7, -3, 2, -1),
        (1, 2, 0, 1),
        (-1, 2, 0, -1),
        (Scaled::MAX_DIMEN.raw(), 1, Scaled::MAX_DIMEN.raw(), 0),
    ];
    for (x, n, quotient, remainder) in cases {
        assert_eq!(
            x_over_n(Scaled::from_raw(x), n).expect("division fits"),
            XOverN {
                quotient: Scaled::from_raw(quotient),
                remainder: Scaled::from_raw(remainder),
            },
            "x_over_n({x}, {n})"
        );
    }

    assert_eq!(
        x_over_n(Scaled::from_raw(123), 0),
        Err(ArithmeticError::DivisionByZero)
    );
}

#[test]
fn mult_and_add_and_nx_plus_y_match_tex_bounds() {
    assert_eq!(
        mult_and_add(
            0,
            Scaled::from_raw(i32::MAX),
            Scaled::from_raw(-7),
            Scaled::MAX_DIMEN,
        )
        .expect("n=0 returns y"),
        Scaled::from_raw(-7)
    );
    assert_eq!(
        nx_plus_y(-3, Scaled::from_raw(10), Scaled::from_raw(4)).expect("fits"),
        Scaled::from_raw(-26)
    );
    assert_eq!(
        nx_plus_y(
            2,
            Scaled::from_raw(Scaled::MAX_DIMEN.raw() / 2),
            Scaled::from_raw(1)
        )
        .expect("upper boundary fits"),
        Scaled::MAX_DIMEN
    );
    assert_eq!(
        nx_plus_y(
            2,
            Scaled::from_raw(Scaled::MAX_DIMEN.raw() / 2),
            Scaled::from_raw(2)
        ),
        Err(ArithmeticError::Overflow)
    );
    assert_eq!(
        nx_plus_y(
            2,
            Scaled::from_raw(-Scaled::MAX_DIMEN.raw() / 2),
            Scaled::from_raw(-1)
        )
        .expect("lower boundary fits"),
        Scaled::from_raw(-Scaled::MAX_DIMEN.raw())
    );
    assert_eq!(
        nx_plus_y(
            2,
            Scaled::from_raw(-(Scaled::MAX_DIMEN.raw() / 2) - 1),
            Scaled::from_raw(0)
        ),
        Err(ArithmeticError::Overflow)
    );
}

#[test]
fn decimal_fraction_rounding_matches_tex_edges() {
    assert_eq!(round_decimal_fraction(&[]), 0);
    assert_eq!(round_decimal_fraction(&[5]), Scaled::UNITY / 2);
    assert_eq!(round_decimal_fraction(&[9, 9, 9, 9, 9]), 65_535);
    assert_eq!(round_decimal_fraction(&[0, 0, 0, 0, 7, 6]), 5);
    assert_eq!(
        round_decimal_fraction(&[9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9]),
        Scaled::UNITY
    );
}

#[test]
fn physical_unit_table_matches_tex_web() {
    assert_eq!(PhysicalUnit::Sp.point_ratio(), (1, 65_536));
    assert_eq!(PhysicalUnit::Pt.point_ratio(), (1, 1));
    assert_eq!(PhysicalUnit::In.point_ratio(), (7_227, 100));
    assert_eq!(PhysicalUnit::Pc.point_ratio(), (12, 1));
    assert_eq!(PhysicalUnit::Cm.point_ratio(), (7_227, 254));
    assert_eq!(PhysicalUnit::Mm.point_ratio(), (7_227, 2_540));
    assert_eq!(PhysicalUnit::Bp.point_ratio(), (7_227, 7_200));
    assert_eq!(PhysicalUnit::Dd.point_ratio(), (1_238, 1_157));
    assert_eq!(PhysicalUnit::Cc.point_ratio(), (14_856, 1_157));
}

#[test]
fn converts_physical_unit_edge_values() {
    assert_eq!(
        scaled_from_decimal_parts(
            0,
            round_decimal_fraction(&[9, 9, 9, 9, 9]),
            PhysicalUnit::Pt
        )
        .expect("99999/100000pt fits")
        .raw(),
        65_535
    );
    assert_eq!(
        scaled_from_decimal_parts(
            16_383,
            round_decimal_fraction(&[9, 9, 9, 9, 8]),
            PhysicalUnit::Pt
        )
        .expect("1638399998/100000pt fits exactly at max_dimen"),
        Scaled::MAX_DIMEN
    );
    assert_eq!(
        scaled_from_decimal_parts(Scaled::MAX_DIMEN.raw(), 0, PhysicalUnit::Sp)
            .expect("max_dimen sp fits"),
        Scaled::MAX_DIMEN
    );
}

#[test]
fn converts_unit_fractions_with_tex_rounding() {
    assert_eq!(
        scaled_from_decimal_parts(1, 0, PhysicalUnit::In)
            .expect("1in fits")
            .raw(),
        4_736_286
    );
    assert_eq!(
        scaled_from_decimal_parts(1, 0, PhysicalUnit::Pc)
            .expect("1pc fits")
            .raw(),
        786_432
    );
    assert_eq!(
        scaled_from_decimal_parts(1, 0, PhysicalUnit::Cm)
            .expect("1cm fits")
            .raw(),
        1_864_679
    );
    assert_eq!(
        scaled_from_decimal_parts(1, 0, PhysicalUnit::Mm)
            .expect("1mm fits")
            .raw(),
        186_467
    );
    assert_eq!(
        scaled_from_decimal_parts(1, 0, PhysicalUnit::Bp)
            .expect("1bp fits")
            .raw(),
        65_781
    );
    assert_eq!(
        scaled_from_decimal_parts(1, 0, PhysicalUnit::Dd)
            .expect("1dd fits")
            .raw(),
        70_124
    );
    assert_eq!(
        scaled_from_decimal_parts(1, 0, PhysicalUnit::Cc)
            .expect("1cc fits")
            .raw(),
        841_489
    );
    assert_eq!(
        scaled_from_decimal_parts(1, round_decimal_fraction(&[5]), PhysicalUnit::Sp)
            .expect("fractional sp truncates to integer sp")
            .raw(),
        1
    );
}

#[test]
fn dimension_overflow_reports_tex_error_text() {
    let error = scaled_from_decimal_parts(16_384, 0, PhysicalUnit::Pt)
        .expect_err("16384pt exceeds max_dimen");
    assert_eq!(error, DimensionError::TooLarge);
    assert_eq!(error.to_string(), "Dimension too large");

    let error = scaled_from_decimal_parts(Scaled::MAX_DIMEN.raw() + 1, 0, PhysicalUnit::Sp)
        .expect_err("max_dimen plus 1sp exceeds max_dimen");
    assert_eq!(error.to_string(), "Dimension too large");
}

#[test]
fn tfm_design_size_and_font_size_rules_match_tex_web() {
    let design = tfm_design_size_from_fix_word([0x00, 0xa0, 0x00, 0x00]).expect("trip design size");
    assert_eq!(design.raw(), 10 * Scaled::UNITY);
    assert_eq!(
        tfm_font_size(design, FontSizeSpec::Design).expect("default font size is valid"),
        design
    );
    assert_eq!(
        tfm_font_size(
            design,
            FontSizeSpec::At(Scaled::from_raw(12 * Scaled::UNITY))
        )
        .expect("explicit at-size is valid"),
        Scaled::from_raw(12 * Scaled::UNITY)
    );
    assert_eq!(
        tfm_font_size(design, FontSizeSpec::Scale(1200)).expect("scaled factor is valid"),
        Scaled::from_raw(12 * Scaled::UNITY)
    );

    assert_eq!(
        tfm_design_size_from_fix_word([0x00, 0x09, 0xff, 0xff]),
        Err(TfmConversionError::InvalidDesignSize)
    );
    assert_eq!(
        tfm_design_size_from_fix_word([0x80, 0x00, 0x00, 0x00]),
        Err(TfmConversionError::InvalidDesignSize)
    );
    assert_eq!(
        tfm_font_size(design, FontSizeSpec::At(Scaled::from_raw(0))),
        Err(TfmConversionError::InvalidAtSize)
    );
    assert_eq!(
        tfm_font_size(design, FontSizeSpec::At(Scaled::from_raw(1 << 27))),
        Err(TfmConversionError::InvalidAtSize)
    );
    assert_eq!(
        tfm_font_size(design, FontSizeSpec::Scale(0)),
        Err(TfmConversionError::InvalidScale)
    );
    assert_eq!(
        tfm_font_size(Scaled::from_raw((1 << 27) - 1), FontSizeSpec::Scale(32_768)),
        Err(TfmConversionError::ArithmeticOverflow)
    );
}

#[test]
fn tfm_fix_word_conversion_matches_trip_tfm_tables() {
    let ten_pt = Scaled::from_raw(10 * Scaled::UNITY);
    let twelve_pt = Scaled::from_raw(12 * Scaled::UNITY);
    let cases = [
        ([0x00, 0x00, 0x00, 0x00], ten_pt, 0),
        ([0xff, 0xf8, 0x00, 0x00], ten_pt, -327_680),
        ([0x00, 0x01, 0x99, 0x9a], ten_pt, 65_536),
        ([0x00, 0x03, 0x33, 0x34], ten_pt, 131_072),
        ([0x00, 0x06, 0x66, 0x67], ten_pt, 262_144),
        ([0x00, 0x0b, 0x33, 0x34], ten_pt, 458_752),
        ([0xff, 0xfe, 0x66, 0x66], ten_pt, -65_537),
        ([0xff, 0xf0, 0x00, 0x00], ten_pt, -655_360),
        ([0xff, 0xff, 0xd7, 0x0a], ten_pt, -6_554),
        ([0x00, 0x08, 0x00, 0x00], ten_pt, 327_680),
        ([0x00, 0x08, 0x00, 0x00], twelve_pt, 393_216),
        ([0xff, 0xf8, 0x00, 0x00], twelve_pt, -393_216),
        ([0x00, 0x06, 0x66, 0x67], twelve_pt, 314_573),
        ([0xff, 0xfe, 0x66, 0x66], twelve_pt, -78_644),
        ([0x00, 0x0b, 0x33, 0x34], twelve_pt, 550_503),
    ];

    for (bytes, size, expected) in cases {
        assert_eq!(
            tfm_fix_word_to_scaled(bytes, size).expect("valid trip.tfm fix_word"),
            Scaled::from_raw(expected),
            "fix_word {bytes:02x?} at {}sp",
            size.raw()
        );
    }

    assert_eq!(
        tfm_fix_word_to_scaled([1, 0, 0, 0], ten_pt),
        Err(TfmConversionError::InvalidFixWord)
    );
}

#[test]
fn tfm_slant_fix_word_uses_tex_arithmetic_shift_semantics() {
    assert_eq!(
        tfm_slant_fix_word_to_scaled_ratio([0, 0, 0, 0x10]),
        Scaled::from_raw(1)
    );
    assert_eq!(
        tfm_slant_fix_word_to_scaled_ratio([0, 0, 0, 0x0f]),
        Scaled::from_raw(0)
    );
    assert_eq!(
        tfm_slant_fix_word_to_scaled_ratio([0xff, 0xff, 0xff, 0xff]),
        Scaled::from_raw(-1)
    );
    assert_eq!(
        tfm_slant_fix_word_to_scaled_ratio([0xff, 0xff, 0xff, 0xef]),
        Scaled::from_raw(-2)
    );
}
