//! TeX scaled-point arithmetic substrate.

use core::fmt;
use core::ops::{Add, Neg, Sub};

const XN_OVER_D_RADIX: i32 = 32_768;

/// A TeX scaled-point value.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Scaled(i32);

impl Scaled {
    /// The number of scaled points in one TeX point.
    pub const UNITY: i32 = 65_536;

    /// TeX's largest legal dimension, `2^30 - 1` scaled points.
    pub const MAX_DIMEN: Self = Self((1 << 30) - 1);

    /// The smallest representable scaled value for the M1 substrate.
    pub const MIN: Self = Self(i32::MIN);

    /// The largest representable scaled value for the M1 substrate.
    pub const MAX: Self = Self(i32::MAX);

    /// Creates a scaled value from its raw representation.
    #[must_use]
    pub const fn from_raw(raw: i32) -> Self {
        Self(raw)
    }

    /// Returns the raw scaled-point representation.
    #[must_use]
    pub const fn raw(self) -> i32 {
        self.0
    }

    /// Checked addition.
    #[must_use]
    pub const fn checked_add(self, rhs: Self) -> Option<Self> {
        match self.0.checked_add(rhs.0) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Checked subtraction.
    #[must_use]
    pub const fn checked_sub(self, rhs: Self) -> Option<Self> {
        match self.0.checked_sub(rhs.0) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Checked negation.
    #[must_use]
    pub const fn checked_neg(self) -> Option<Self> {
        match self.0.checked_neg() {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Returns this value if it is in TeX's legal dimension range.
    pub const fn check_dimension(self) -> Result<Self, DimensionError> {
        if self.0 >= (1 << 30) || self.0 <= -(1 << 30) {
            Err(DimensionError::TooLarge)
        } else {
            Ok(self)
        }
    }
}

/// Errors produced by TeX scaled arithmetic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DimensionError {
    /// The arithmetic exceeded TeX's legal dimension range.
    TooLarge,
}

impl fmt::Display for DimensionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => f.write_str("Dimension too large"),
        }
    }
}

impl std::error::Error for DimensionError {}

/// The physical units accepted by TeX's dimension scanner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhysicalUnit {
    /// Scaled points.
    Sp,
    /// Printer's points.
    Pt,
    /// Inches.
    In,
    /// Picas.
    Pc,
    /// Centimeters.
    Cm,
    /// Millimeters.
    Mm,
    /// Big points.
    Bp,
    /// Didot points.
    Dd,
    /// Ciceros.
    Cc,
}

impl PhysicalUnit {
    /// Returns TeX's exact conversion ratio from this unit to points.
    #[must_use]
    pub const fn point_ratio(self) -> (i32, i32) {
        match self {
            Self::Sp => (1, Scaled::UNITY),
            Self::Pt => (1, 1),
            Self::In => (7_227, 100),
            Self::Pc => (12, 1),
            Self::Cm => (7_227, 254),
            Self::Mm => (7_227, 2_540),
            Self::Bp => (7_227, 7_200),
            Self::Dd => (1_238, 1_157),
            Self::Cc => (14_856, 1_157),
        }
    }
}

/// Result of TeX's `xn_over_d` scaled multiplication.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct XnOverD {
    /// The quotient `x * n / d`, rounded toward zero.
    pub quotient: Scaled,
    /// The signed remainder TeX stores after the division.
    pub remainder: i32,
}

/// Computes TeX's `xn_over_d(x, n, d)` routine.
///
/// `n` and `d` must be nonnegative 16-bit conversion factors, with `d > 0`.
/// The result preserves TeX's 1.5-precision arithmetic and overflow test.
pub fn xn_over_d(x: Scaled, n: i32, d: i32) -> Result<XnOverD, DimensionError> {
    assert!(
        (0..=Scaled::UNITY).contains(&n),
        "numerator out of TeX range"
    );
    assert!(
        (1..=Scaled::UNITY).contains(&d),
        "denominator out of TeX range"
    );

    let positive = x.raw() >= 0;
    let x_abs = i64::from(x.raw()).abs();
    let n = i64::from(n);
    let d = i64::from(d);
    let radix = i64::from(XN_OVER_D_RADIX);

    let t = (x_abs % radix) * n;
    let u = (x_abs / radix) * n + (t / radix);
    let v = (u % d) * radix + (t % radix);
    if u / d >= radix {
        return Err(DimensionError::TooLarge);
    }

    let quotient = radix * (u / d) + (v / d);
    let remainder = v % d;
    let quotient = i32::try_from(if positive { quotient } else { -quotient })
        .expect("xn_over_d quotient fits i32 after TeX overflow check");
    let remainder = i32::try_from(if positive { remainder } else { -remainder })
        .expect("xn_over_d remainder fits i32");

    Ok(XnOverD {
        quotient: Scaled::from_raw(quotient),
        remainder,
    })
}

/// Converts decimal fraction digits to TeX's correctly rounded scaled fraction.
///
/// TeX keeps only the first 17 decimal digits; later digits cannot affect the
/// rounded scaled-point fraction.
#[must_use]
pub fn round_decimal_fraction(digits: &[u8]) -> i32 {
    let mut a = 0;
    for &digit in digits.iter().take(17).rev() {
        assert!(digit <= 9, "decimal fraction digit out of range");
        a = (a + i32::from(digit) * 2 * Scaled::UNITY) / 10;
    }
    (a + 1) / 2
}

/// Converts a nonnegative scanned decimal value and physical unit to sp.
///
/// `integer` is the part before the decimal point. `fraction` is the result of
/// [`round_decimal_fraction`] for the digits after the decimal point. This is
/// the arithmetic-only part of `scan_dimen`; token parsing, signs, `true`
/// magnification, internal units, and assignment semantics live elsewhere.
pub fn scaled_from_decimal_parts(
    integer: i32,
    fraction: i32,
    unit: PhysicalUnit,
) -> Result<Scaled, DimensionError> {
    assert!(integer >= 0, "dimension integer part must be nonnegative");
    assert!(
        (0..=Scaled::UNITY).contains(&fraction),
        "dimension fraction out of range"
    );

    if unit == PhysicalUnit::Sp {
        return Scaled::from_raw(integer).check_dimension();
    }

    let (num, denom) = unit.point_ratio();
    let mut cur = integer;
    let mut frac = fraction;

    if unit != PhysicalUnit::Pt {
        let converted = xn_over_d(Scaled::from_raw(cur), num, denom)?;
        cur = converted.quotient.raw();
        frac = (num * frac + Scaled::UNITY * converted.remainder) / denom;
        cur += frac / Scaled::UNITY;
        frac %= Scaled::UNITY;
    }

    if cur >= (1 << 14) {
        return Err(DimensionError::TooLarge);
    }

    Scaled::from_raw(cur * Scaled::UNITY + frac).check_dimension()
}

impl Add for Scaled {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        match self.checked_add(rhs) {
            Some(value) => value,
            None => panic!("scaled addition overflow"),
        }
    }
}

impl Sub for Scaled {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        match self.checked_sub(rhs) {
            Some(value) => value,
            None => panic!("scaled subtraction overflow"),
        }
    }
}

impl Neg for Scaled {
    type Output = Self;

    fn neg(self) -> Self::Output {
        match self.checked_neg() {
            Some(value) => value,
            None => panic!("scaled negation overflow"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DimensionError, PhysicalUnit, Scaled, round_decimal_fraction, scaled_from_decimal_parts,
        xn_over_d,
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
            super::XnOverD {
                quotient: Scaled::from_raw(72),
                remainder: 27,
            }
        );
        assert_eq!(
            xn_over_d(Scaled::from_raw(-1), 7_227, 100)
                .expect("negative 1in integer conversion fits"),
            super::XnOverD {
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
            .expect("0.99999pt fits")
            .raw(),
            65_535
        );
        assert_eq!(
            scaled_from_decimal_parts(
                16_383,
                round_decimal_fraction(&[9, 9, 9, 9, 8]),
                PhysicalUnit::Pt
            )
            .expect("16383.99998pt fits exactly at max_dimen"),
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
}
