//! Pure pdfTeX font-expansion arithmetic.
//!
//! This module mirrors the arithmetic split between `read_expand_font`,
//! `try_break`, and `hpack` in pdfTeX 1.40.27. It does not allocate generated
//! fonts; execution applies the returned discrete ratios at the mutation
//! boundary after line breaking.

use tex_state::scaled::Scaled;

/// Validated expansion limits for one base font, in thousandths.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FontExpansionSpec {
    stretch: u16,
    shrink: u16,
    step: u8,
    auto_expand: bool,
}

impl FontExpansionSpec {
    /// Applies pdfTeX's input clamps and step flooring.
    pub fn new(
        stretch: i32,
        shrink: i32,
        step: i32,
        auto_expand: bool,
    ) -> Result<Self, FontExpansionError> {
        let step = step.clamp(0, 100);
        if step == 0 {
            return Err(FontExpansionError::InvalidStep);
        }
        let stretch = stretch.clamp(0, 1000);
        let shrink = shrink.clamp(0, 500);
        let stretch = stretch - stretch % step;
        let shrink = shrink - shrink % step;
        if stretch == 0 && shrink == 0 {
            return Err(FontExpansionError::InvalidLimits);
        }
        Ok(Self {
            stretch: u16::try_from(stretch).expect("clamped stretch fits u16"),
            shrink: u16::try_from(shrink).expect("clamped shrink fits u16"),
            step: u8::try_from(step).expect("clamped step fits u8"),
            auto_expand,
        })
    }

    #[must_use]
    pub const fn stretch(self) -> i32 {
        self.stretch as i32
    }

    #[must_use]
    pub const fn shrink(self) -> i32 {
        self.shrink as i32
    }

    #[must_use]
    pub const fn step(self) -> i32 {
        self.step as i32
    }

    #[must_use]
    pub const fn auto_expand(self) -> bool {
        self.auto_expand
    }

    /// Selects the nearest legal derived-font ratio for one glyph.
    ///
    /// `line_ratio` is pdfTeX's normalized signed per-line ratio in
    /// `-1000..=1000`; `efcode` is already clamped to `0..=1000`.
    #[must_use]
    pub fn discrete_ratio(self, line_ratio: i32, efcode: i32) -> i16 {
        if line_ratio == 0 || efcode <= 0 {
            return 0;
        }
        let limit = if line_ratio > 0 {
            self.stretch()
        } else {
            self.shrink()
        };
        if limit == 0 {
            return 0;
        }
        let requested = round_ratio(
            i64::from(line_ratio) * i64::from(efcode.clamp(0, 1000)),
            i64::from(limit),
            1_000_000,
        );
        let selected = nearest_step(requested, self.step(), limit);
        i16::try_from(selected).expect("expansion ratio is bounded to -500..=1000")
    }
}

/// Fatal pdfTeX font-expansion configuration/paragraph failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FontExpansionError {
    InvalidStep,
    InvalidLimits,
    DifferentStep,
    DifferentStretchLimit,
    DifferentShrinkLimit,
}

impl std::fmt::Display for FontExpansionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidStep => "invalid step",
            Self::InvalidLimits => "invalid limit(s)",
            Self::DifferentStep => {
                "using fonts with different step of expansion in one paragraph is not allowed"
            }
            Self::DifferentStretchLimit => {
                "using fonts with different stretch limit of expansion in one paragraph is not allowed"
            }
            Self::DifferentShrinkLimit => {
                "using fonts with different shrink limit of expansion in one paragraph is not allowed"
            }
        })
    }
}

impl std::error::Error for FontExpansionError {}

/// Paragraph-wide compatibility state accumulated in source order.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ParagraphExpansion {
    step: Option<u8>,
    stretch: Option<u16>,
    shrink: Option<u16>,
}

impl ParagraphExpansion {
    pub fn observe(&mut self, spec: FontExpansionSpec) -> Result<(), FontExpansionError> {
        observe_equal(&mut self.step, spec.step, FontExpansionError::DifferentStep)?;
        if spec.stretch != 0 {
            observe_equal(
                &mut self.stretch,
                spec.stretch,
                FontExpansionError::DifferentStretchLimit,
            )?;
        }
        if spec.shrink != 0 {
            observe_equal(
                &mut self.shrink,
                spec.shrink,
                FontExpansionError::DifferentShrinkLimit,
            )?;
        }
        Ok(())
    }
}

fn observe_equal<T: Copy + Eq>(
    current: &mut Option<T>,
    value: T,
    error: FontExpansionError,
) -> Result<(), FontExpansionError> {
    match *current {
        Some(existing) if existing != value => Err(error),
        Some(_) => Ok(()),
        None => {
            *current = Some(value);
            Ok(())
        }
    }
}

/// Maximum stretch and shrink contributed by one glyph or font kern.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExpansionCapacity {
    pub stretch: Scaled,
    pub shrink: Scaled,
}

impl Default for ExpansionCapacity {
    fn default() -> Self {
        Self {
            stretch: Scaled::from_raw(0),
            shrink: Scaled::from_raw(0),
        }
    }
}

impl ExpansionCapacity {
    #[must_use]
    pub fn for_metric(natural: Scaled, spec: FontExpansionSpec, efcode: i32) -> Self {
        let efcode = efcode.clamp(0, 1000);
        if efcode == 0 {
            return Self::default();
        }
        let stretched = scaled_at_ratio(natural, spec.stretch());
        let shrunk = scaled_at_ratio(natural, -spec.shrink());
        Self {
            stretch: positive_scaled_ratio(stretched.raw() - natural.raw(), efcode, 1000),
            shrink: positive_scaled_ratio(natural.raw() - shrunk.raw(), efcode, 1000),
        }
    }
}

/// Calculates pdfTeX's normalized signed line expansion ratio.
///
/// Infinite-order glue wins before font expansion. The caller supplies that
/// fact because the packing kernel owns glue-order selection.
#[must_use]
pub fn line_expansion_ratio(
    shortfall: Scaled,
    capacity: ExpansionCapacity,
    has_infinite_adjustment: bool,
) -> i32 {
    if shortfall.raw() == 0 || has_infinite_adjustment {
        return 0;
    }
    let available = if shortfall.raw() > 0 {
        capacity.stretch
    } else {
        capacity.shrink
    };
    if available.raw() <= 0 {
        return 0;
    }
    divide_scaled(shortfall.raw(), available.raw(), 3).clamp(-1000, 1000)
}

/// Scales a metric by `(1000 + ratio) / 1000` with pdfTeX rounding.
#[must_use]
pub fn scaled_at_ratio(value: Scaled, ratio: i32) -> Scaled {
    Scaled::from_raw(
        i32::try_from(round_ratio(
            i64::from(value.raw()),
            i64::from(1000 + ratio),
            1000,
        ))
        .expect("font expansion of a legal metric remains representable"),
    )
}

fn positive_scaled_ratio(value: i32, numerator: i32, denominator: i32) -> Scaled {
    if value <= 0 {
        Scaled::from_raw(0)
    } else {
        Scaled::from_raw(
            i32::try_from(round_ratio(
                i64::from(value),
                i64::from(numerator),
                i64::from(denominator),
            ))
            .expect("capacity is bounded by a legal font metric"),
        )
    }
}

fn nearest_step(requested: i64, step: i32, limit: i32) -> i32 {
    let negative = requested < 0;
    let mut magnitude = requested.unsigned_abs() as i64;
    magnitude = magnitude.min(i64::from(limit));
    if magnitude < i64::from(limit) && magnitude % i64::from(step) != 0 {
        magnitude =
            i64::from(step) * round_ratio(magnitude, 1, i64::from(step)).unsigned_abs() as i64;
    }
    let magnitude = i32::try_from(magnitude.min(i64::from(limit))).expect("limit fits i32");
    if negative { -magnitude } else { magnitude }
}

fn round_ratio(value: i64, numerator: i64, denominator: i64) -> i64 {
    debug_assert!(denominator > 0);
    let product = i128::from(value) * i128::from(numerator);
    let denominator = i128::from(denominator);
    let rounded = (product.abs() + denominator / 2) / denominator;
    i64::try_from(if product < 0 { -rounded } else { rounded })
        .expect("bounded expansion arithmetic fits i64")
}

/// pdfTeX's `divide_scaled(s, m, 3)`, including half-up rounding.
fn divide_scaled(value: i32, divisor: i32, decimal_digits: u32) -> i32 {
    debug_assert!(divisor != 0);
    let scale = 10_i128.pow(decimal_digits);
    let numerator = i128::from(value) * scale;
    let denominator = i128::from(divisor);
    let negative = numerator.is_negative() != denominator.is_negative();
    let rounded = (numerator.abs() + denominator.abs() / 2) / denominator.abs();
    i32::try_from(if negative { -rounded } else { rounded })
        .expect("normalized expansion ratio fits i32")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sp(value: i32) -> Scaled {
        Scaled::from_raw(value)
    }

    #[test]
    fn specification_clamps_and_floors_limits_to_the_step() {
        let spec = FontExpansionSpec::new(1_500, 700, 30, true).expect("valid spec");
        assert_eq!(spec.stretch(), 990);
        assert_eq!(spec.shrink(), 480);
        assert_eq!(spec.step(), 30);
        assert!(spec.auto_expand());
        assert_eq!(
            FontExpansionSpec::new(10, 10, 0, true),
            Err(FontExpansionError::InvalidStep)
        );
        assert_eq!(
            FontExpansionSpec::new(-1, 9, 10, true),
            Err(FontExpansionError::InvalidLimits)
        );
    }

    #[test]
    fn paragraph_rejects_only_conflicting_nonzero_limits() {
        let mut paragraph = ParagraphExpansion::default();
        paragraph
            .observe(FontExpansionSpec::new(100, 0, 10, true).unwrap())
            .unwrap();
        paragraph
            .observe(FontExpansionSpec::new(0, 50, 10, true).unwrap())
            .unwrap();
        assert_eq!(
            paragraph.observe(FontExpansionSpec::new(100, 50, 20, true).unwrap()),
            Err(FontExpansionError::DifferentStep)
        );

        let mut paragraph = ParagraphExpansion::default();
        paragraph
            .observe(FontExpansionSpec::new(100, 50, 10, true).unwrap())
            .unwrap();
        assert_eq!(
            paragraph.observe(FontExpansionSpec::new(200, 50, 10, true).unwrap()),
            Err(FontExpansionError::DifferentStretchLimit)
        );
        assert_eq!(
            paragraph.observe(FontExpansionSpec::new(100, 100, 10, true).unwrap()),
            Err(FontExpansionError::DifferentShrinkLimit)
        );
    }

    #[test]
    fn metric_capacity_uses_endpoint_rounding_then_efcode() {
        let spec = FontExpansionSpec::new(100, 50, 10, true).unwrap();
        assert_eq!(
            ExpansionCapacity::for_metric(sp(65_536), spec, 800),
            ExpansionCapacity {
                stretch: sp(5_243),
                shrink: sp(2_622),
            }
        );
        assert_eq!(
            ExpansionCapacity::for_metric(sp(65_536), spec, 0),
            ExpansionCapacity::default()
        );
    }

    #[test]
    fn final_ratio_and_discrete_glyph_selection_match_pdftex_steps() {
        let spec = FontExpansionSpec::new(100, 50, 10, true).unwrap();
        assert_eq!(
            line_expansion_ratio(
                sp(500),
                ExpansionCapacity {
                    stretch: sp(2_000),
                    shrink: sp(1_000)
                },
                false
            ),
            250
        );
        assert_eq!(spec.discrete_ratio(250, 1000), 30);
        assert_eq!(spec.discrete_ratio(250, 800), 20);
        assert_eq!(spec.discrete_ratio(-1000, 1000), -50);
        assert_eq!(spec.discrete_ratio(1000, 0), 0);
        assert_eq!(
            line_expansion_ratio(
                sp(500),
                ExpansionCapacity {
                    stretch: sp(2_000),
                    shrink: sp(1_000)
                },
                true
            ),
            0
        );
    }
}
