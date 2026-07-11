//! TeX fixed-point arithmetic substrate.
//!
//! This crate owns arithmetic-only TeX data and helper routines shared by the
//! state layer, expansion scanners, and font metric parsing. It deliberately
//! has no dependency on state, font loading, or I/O crates.

use core::fmt;
use core::ops::{Add, Neg, Sub};

const XN_OVER_D_RADIX: i32 = 32_768;
const TFM_STORE_SCALED_LIMIT: i32 = 0o40000000;
const TFM_FIX_WORD_RADIX: i32 = 0o400;
const TFM_SIZE_LIMIT: i32 = 1 << 27;
const TFM_MAX_SCALE: i32 = 32_768;

const NX_PLUS_Y_MAX: Scaled = Scaled::MAX_DIMEN;

/// A TeX scaled-point value.
#[derive(
    Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Deserialize, serde::Serialize,
)]
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

const fn scaled_from_wide_saturating(value: i64) -> Scaled {
    if value > i32::MAX as i64 {
        Scaled::MAX
    } else if value < i32::MIN as i64 {
        Scaled::MIN
    } else {
        Scaled::from_raw(value as i32)
    }
}

/// Adds two scaled values with a widened intermediate and saturates only at
/// the representable `i32` boundary.
///
/// TeX's legal dimension range keeps ordinary semantic values away from this
/// boundary. The saturation makes defensive layout accumulation deterministic
/// instead of allowing a Rust debug overflow or release-mode wrap.
#[must_use]
pub const fn saturating_add(left: Scaled, right: Scaled) -> Scaled {
    scaled_from_wide_saturating(left.0 as i64 + right.0 as i64)
}

/// Subtracts two scaled values with a widened intermediate and saturates only
/// at the representable `i32` boundary.
#[must_use]
pub const fn saturating_sub(left: Scaled, right: Scaled) -> Scaled {
    scaled_from_wide_saturating(left.0 as i64 - right.0 as i64)
}

/// Multiplies a scaled value by an integer with a widened intermediate and
/// saturates only at the representable `i32` boundary.
#[must_use]
pub const fn saturating_mul(factor: i32, value: Scaled) -> Scaled {
    scaled_from_wide_saturating(factor as i64 * value.0 as i64)
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

/// Compatibility scale used when constructing or displaying glue-set ratios.
pub const GLUE_SET_RATIO_SCALE: i32 = 1_000_000;

/// Exact reduced glue-set ratio used by packed boxes and output drivers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct GlueSetRatio {
    numerator: i32,
    denominator: i32,
}

impl Default for GlueSetRatio {
    fn default() -> Self {
        Self::ZERO
    }
}

impl GlueSetRatio {
    /// Zero glue-set ratio.
    pub const ZERO: Self = Self {
        numerator: 0,
        denominator: 1,
    };

    /// Unit glue-set ratio.
    pub const UNITY: Self = Self {
        numerator: 1,
        denominator: 1,
    };

    /// Creates a glue-set ratio from the raw fixed-point representation.
    #[must_use]
    pub const fn from_raw(raw: i32) -> Self {
        Self::from_ratio_parts(raw, GLUE_SET_RATIO_SCALE)
    }

    /// Creates a ratio from exact numerator and denominator parts.
    #[must_use]
    pub const fn from_ratio_parts(numerator: i32, denominator: i32) -> Self {
        if numerator == 0 || denominator == 0 {
            return Self::ZERO;
        }
        let numerator = numerator.saturating_abs();
        let denominator = denominator.saturating_abs();
        let divisor = gcd_i32(numerator, denominator);
        Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        }
    }

    /// Returns the numerator of the reduced exact ratio.
    #[must_use]
    pub const fn numerator(self) -> i32 {
        self.numerator
    }

    /// Returns the positive denominator of the reduced exact ratio.
    #[must_use]
    pub const fn denominator(self) -> i32 {
        self.denominator
    }

    /// Returns a compatibility fixed-point approximation of this ratio.
    #[must_use]
    pub fn raw(self) -> i32 {
        let scaled = i128::from(self.numerator) * i128::from(GLUE_SET_RATIO_SCALE);
        let denominator = i128::from(self.denominator);
        let rounded = if scaled >= 0 {
            (scaled + denominator / 2) / denominator
        } else {
            -((-scaled + denominator / 2) / denominator)
        };
        i32::try_from(rounded.clamp(i128::from(i32::MIN), i128::from(i32::MAX)))
            .expect("clamped glue ratio fits i32")
    }

    /// Returns whether this ratio is zero.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.numerator == 0
    }

    /// Computes `numerator / denominator` with the fixed-point scale.
    #[must_use]
    pub fn from_scaled_ratio(numerator: Scaled, denominator: Scaled) -> Self {
        let denominator = i64::from(denominator.raw()).abs();
        if denominator == 0 {
            return Self::ZERO;
        }
        let numerator = i64::from(numerator.raw()).abs();
        Self::from_ratio_parts(
            i32::try_from(numerator).expect("TeX dimensions fit i32"),
            i32::try_from(denominator).expect("TeX dimensions fit i32"),
        )
    }
}

const fn gcd_i32(left: i32, right: i32) -> i32 {
    let mut left = left.unsigned_abs();
    let mut right = right.unsigned_abs();
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    if left == 0 { 1 } else { left as i32 }
}

/// Errors produced by TeX's arithmetic helper routines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArithmeticError {
    /// TeX's arithmetic error flag would have been set by overflow.
    Overflow,
    /// TeX's arithmetic error flag would have been set by division by zero.
    DivisionByZero,
}

impl fmt::Display for ArithmeticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Overflow => f.write_str("Arithmetic overflow"),
            Self::DivisionByZero => f.write_str("Division by zero"),
        }
    }
}

impl std::error::Error for ArithmeticError {}

/// Errors produced while converting TFM fixed-point metric values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TfmConversionError {
    /// A TFM fix_word used a sign byte other than 0 or 255.
    InvalidFixWord,
    /// The TFM design size is outside TeX's accepted range.
    InvalidDesignSize,
    /// A `\font ... at` size is outside TeX's accepted range.
    InvalidAtSize,
    /// A `\font ... scaled` factor is outside TeX's accepted range.
    InvalidScale,
    /// TeX's arithmetic error flag would have been set while scaling.
    ArithmeticOverflow,
}

impl fmt::Display for TfmConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFixWord => f.write_str("invalid TFM fix_word"),
            Self::InvalidDesignSize => f.write_str("invalid TFM design size"),
            Self::InvalidAtSize => f.write_str("invalid font at-size"),
            Self::InvalidScale => f.write_str("invalid font scaled factor"),
            Self::ArithmeticOverflow => f.write_str("TFM scaling arithmetic overflow"),
        }
    }
}

impl std::error::Error for TfmConversionError {}

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

/// Size override from a TeX `\font` definition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FontSizeSpec {
    /// Use the design size from the TFM header.
    Design,
    /// Use an explicit `at` size.
    At(Scaled),
    /// Scale the TFM design size by a per-mille `scaled` factor.
    Scale(i32),
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

/// Result of TeX's `x_over_n` scaled division.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct XOverN {
    /// The quotient `x / n`, rounded toward zero.
    pub quotient: Scaled,
    /// The signed remainder TeX stores after the division.
    pub remainder: Scaled,
}

/// TeX's signed halving convention.
///
/// Odd values are adjusted before division, so `half(-1) == 0` and
/// `half(-3) == -1`.
#[must_use]
pub const fn half(x: i32) -> i32 {
    if x % 2 != 0 && x > 0 {
        x / 2 + 1
    } else {
        x / 2
    }
}

/// Computes TeX's `mult_and_add(n, x, y, max_answer)` routine.
pub fn mult_and_add(
    n: i32,
    x: Scaled,
    y: Scaled,
    max_answer: Scaled,
) -> Result<Scaled, ArithmeticError> {
    let mut n = i64::from(n);
    let mut x = i64::from(x.raw());
    let y = i64::from(y.raw());
    let max_answer = i64::from(max_answer.raw());

    if n < 0 {
        x = -x;
        n = -n;
    }

    if n == 0 {
        return Ok(Scaled::from_raw(
            i32::try_from(y).expect("scaled y starts as i32"),
        ));
    }

    if x <= (max_answer - y) / n && -x <= (max_answer + y) / n {
        let value = n * x + y;
        let value = i32::try_from(value).map_err(|_| ArithmeticError::Overflow)?;
        Ok(Scaled::from_raw(value))
    } else {
        Err(ArithmeticError::Overflow)
    }
}

/// Computes TeX's `nx_plus_y(n, x, y)` macro.
pub fn nx_plus_y(n: i32, x: Scaled, y: Scaled) -> Result<Scaled, ArithmeticError> {
    mult_and_add(n, x, y, NX_PLUS_Y_MAX)
}

/// Computes TeX's `x_over_n(x, n)` routine.
pub fn x_over_n(x: Scaled, n: i32) -> Result<XOverN, ArithmeticError> {
    if n == 0 {
        return Err(ArithmeticError::DivisionByZero);
    }

    let mut x = i64::from(x.raw());
    let mut n = i64::from(n);
    let mut negative = false;

    if n < 0 {
        x = -x;
        n = -n;
        negative = true;
    }

    let (quotient, mut remainder) = if x >= 0 {
        (x / n, x % n)
    } else {
        let abs_x = -x;
        (-(abs_x / n), -(abs_x % n))
    };

    if negative {
        remainder = -remainder;
    }

    let quotient = i32::try_from(quotient).map_err(|_| ArithmeticError::Overflow)?;
    let remainder = i32::try_from(remainder).map_err(|_| ArithmeticError::Overflow)?;
    Ok(XOverN {
        quotient: Scaled::from_raw(quotient),
        remainder: Scaled::from_raw(remainder),
    })
}

/// Computes TeX's `xn_over_d(x, n, d)` routine.
///
/// `n` and `d` must be nonnegative 16-bit conversion factors, with `d > 0`.
/// The result preserves TeX's one-and-a-half-precision arithmetic and overflow test.
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

/// Applies TeX82's `true`-dimension magnification scaling to decimal parts.
///
/// This is the arithmetic from `scan_dimen` between recognizing `true` and
/// converting the physical unit. `mag` must already have passed TeX's
/// `prepare_mag` validation. TeX evaluates the fraction numerator before
/// dividing by `mag`; the widened intermediate is required because that
/// numerator can exceed `i32::MAX` for otherwise legal inputs.
pub fn scale_true_dimension_parts(
    integer: i32,
    fraction: i32,
    mag: i32,
) -> Result<(i32, i32), DimensionError> {
    assert!(integer >= 0, "dimension integer part must be nonnegative");
    assert!(
        (0..=Scaled::UNITY).contains(&fraction),
        "dimension fraction out of range"
    );
    assert!(
        (1..=32_768).contains(&mag),
        "magnification out of TeX range"
    );

    if mag == 1000 {
        return Ok((integer, fraction));
    }

    let converted = xn_over_d(Scaled::from_raw(integer), 1000, mag)?;
    let numerator =
        i64::from(1000 * fraction) + i64::from(Scaled::UNITY) * i64::from(converted.remainder);
    let scaled_fraction = numerator / i64::from(mag);
    let scaled_integer =
        i64::from(converted.quotient.raw()) + scaled_fraction / i64::from(Scaled::UNITY);
    let fraction = scaled_fraction % i64::from(Scaled::UNITY);

    Ok((
        i32::try_from(scaled_integer).expect("true-scaled integer parts fit i32"),
        i32::try_from(fraction).expect("true-scaled fractional parts fit i32"),
    ))
}

/// Applies TeX's TFM font-size rule for design/default, `at`, and `scaled`.
pub fn tfm_font_size(
    design_size: Scaled,
    spec: FontSizeSpec,
) -> Result<Scaled, TfmConversionError> {
    validate_tfm_design_size(design_size)?;

    match spec {
        FontSizeSpec::Design => Ok(design_size),
        FontSizeSpec::At(size) => {
            if size.raw() <= 0 || size.raw() >= TFM_SIZE_LIMIT {
                Err(TfmConversionError::InvalidAtSize)
            } else {
                Ok(size)
            }
        }
        FontSizeSpec::Scale(scale) => {
            if !(1..=TFM_MAX_SCALE).contains(&scale) {
                return Err(TfmConversionError::InvalidScale);
            }
            xn_over_d(design_size, scale, 1000)
                .map(|value| value.quotient)
                .map_err(|_| TfmConversionError::ArithmeticOverflow)
        }
    }
}

/// Converts a TFM metric `fix_word` to scaled points at the selected font size.
pub fn tfm_fix_word_to_scaled(
    bytes: [u8; 4],
    font_size: Scaled,
) -> Result<Scaled, TfmConversionError> {
    validate_tfm_metric_size(font_size)?;

    let [a, b, c, d] = bytes;
    let mut z = font_size.raw();
    let mut alpha = 16;
    while z >= TFM_STORE_SCALED_LIMIT {
        z /= 2;
        alpha += alpha;
    }

    let beta = 256 / alpha;
    alpha *= z;

    let z = i64::from(z);
    let beta = i64::from(beta);
    let alpha = i64::from(alpha);
    let sw = (((((i64::from(d) * z) / i64::from(TFM_FIX_WORD_RADIX)) + i64::from(c) * z)
        / i64::from(TFM_FIX_WORD_RADIX))
        + i64::from(b) * z)
        / beta;

    let value = match a {
        0 => sw,
        255 => sw - alpha,
        _ => return Err(TfmConversionError::InvalidFixWord),
    };

    let value = i32::try_from(value).map_err(|_| TfmConversionError::ArithmeticOverflow)?;
    Ok(Scaled::from_raw(value))
}

/// Converts TFM header word 1, the design-size fix_word, to TeX scaled points.
pub fn tfm_design_size_from_fix_word(bytes: [u8; 4]) -> Result<Scaled, TfmConversionError> {
    if bytes[0] > 127 {
        return Err(TfmConversionError::InvalidDesignSize);
    }

    let mut z = i32::from(bytes[0]) * TFM_FIX_WORD_RADIX + i32::from(bytes[1]);
    z = z * TFM_FIX_WORD_RADIX + i32::from(bytes[2]);
    z = z * 16 + i32::from(bytes[3] / 16);
    let design_size = Scaled::from_raw(z);
    validate_tfm_design_size(design_size)?;
    Ok(design_size)
}

const fn validate_tfm_design_size(size: Scaled) -> Result<(), TfmConversionError> {
    if size.0 < Scaled::UNITY || size.0 >= TFM_SIZE_LIMIT {
        Err(TfmConversionError::InvalidDesignSize)
    } else {
        Ok(())
    }
}

const fn validate_tfm_metric_size(size: Scaled) -> Result<(), TfmConversionError> {
    if size.0 <= 0 || size.0 >= TFM_SIZE_LIMIT {
        Err(TfmConversionError::InvalidAtSize)
    } else {
        Ok(())
    }
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
mod scaled_tests;
