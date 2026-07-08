use crate::scaled::{
    ArithmeticError, DimensionError, FontSizeSpec, PhysicalUnit, Scaled, TfmConversionError,
    XOverN, XnOverD, half, mult_and_add, nx_plus_y, round_decimal_fraction,
    scaled_from_decimal_parts, tfm_design_size_from_fix_word, tfm_fix_word_to_scaled,
    tfm_font_size, x_over_n, xn_over_d,
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
