//! Immutable TeX glue values.

use crate::scaled::Scaled;

/// The infinity order attached to stretch or shrink components.
#[derive(
    Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Deserialize, serde::Serialize,
)]
#[repr(u8)]
pub enum Order {
    Normal = 0,
    Fil = 1,
    Fill = 2,
    Filll = 3,
}

/// An immutable TeX glue specification.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GlueSpec {
    pub width: Scaled,
    pub stretch: Scaled,
    pub stretch_order: Order,
    pub shrink: Scaled,
    pub shrink_order: Order,
}

impl GlueSpec {
    pub const ZERO: Self = Self {
        width: Scaled::from_raw(0),
        stretch: Scaled::from_raw(0),
        stretch_order: Order::Normal,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
    };

    /// TeX82 §1237's `trap_zero_glue` ignores the orders of zero components.
    #[must_use]
    pub const fn has_zero_components(self) -> bool {
        self.width.raw() == 0 && self.stretch.raw() == 0 && self.shrink.raw() == 0
    }
}
