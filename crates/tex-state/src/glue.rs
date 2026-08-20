//! Copy-only glue identities and immutable glue values.

use crate::ids::GlueId;
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
}

/// Copy-only semantic identity for glue in the runtime value registry.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GlueSpecRef {
    id: GlueId,
}

impl GlueSpecRef {
    pub(crate) const fn new(id: GlueId) -> Self {
        Self { id }
    }

    #[must_use]
    pub const fn id(&self) -> GlueId {
        self.id
    }

    #[must_use]
    pub const fn raw(&self) -> u32 {
        self.id.raw()
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_new(id: GlueId) -> Self {
        Self::new(id)
    }
}

/// Copy-only conversion accepted by aggregate glue read boundaries.
pub trait GlueHandle {
    fn glue_id(&self) -> GlueId;
}

impl GlueHandle for GlueId {
    fn glue_id(&self) -> GlueId {
        *self
    }
}

impl GlueHandle for GlueSpecRef {
    fn glue_id(&self) -> GlueId {
        self.id
    }
}

impl<T: GlueHandle + ?Sized> GlueHandle for &T {
    fn glue_id(&self) -> GlueId {
        (*self).glue_id()
    }
}

impl From<GlueSpecRef> for GlueId {
    fn from(root: GlueSpecRef) -> Self {
        root.id()
    }
}

impl From<&GlueSpecRef> for GlueId {
    fn from(root: &GlueSpecRef) -> Self {
        root.id()
    }
}

#[cfg(any(test, feature = "testing"))]
pub fn testing_zero_glue_ref() -> GlueSpecRef {
    GlueSpecRef::new(GlueId::ZERO)
}
