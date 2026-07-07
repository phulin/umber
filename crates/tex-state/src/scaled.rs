//! TeX scaled-point arithmetic substrate.

use core::ops::{Add, Neg, Sub};

/// A TeX scaled-point value.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Scaled(i32);

impl Scaled {
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
    use super::Scaled;

    #[test]
    fn scaled_add_sub_neg_and_checked_variants() {
        let a = Scaled::from_raw(10);
        let b = Scaled::from_raw(3);

        assert_eq!((a + b).raw(), 13);
        assert_eq!((a - b).raw(), 7);
        assert_eq!((-b).raw(), -3);
        assert_eq!(Scaled::MIN.raw(), i32::MIN);
        assert_eq!(Scaled::MAX.raw(), i32::MAX);

        assert_eq!(Scaled::MAX.checked_add(Scaled::from_raw(1)), None);
        assert_eq!(Scaled::from_raw(i32::MIN).checked_neg(), None);
    }
}
