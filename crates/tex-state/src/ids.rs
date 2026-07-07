//! Opaque store handles.
//!
//! `TokenListId` becomes a real token-store handle in State M2. `GlueId` and
//! `NodeListId` become real glue/node arena handles in State M2. `FontId`
//! becomes real in the fonts epic. `SnapshotId` becomes real in State M3.

macro_rules! opaque_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(u32);

        impl $name {
            #[allow(dead_code)]
            pub(crate) const fn new(raw: u32) -> Self {
                Self(raw)
            }

            #[must_use]
            pub const fn raw(self) -> u32 {
                self.0
            }
        }
    };
}

opaque_id!(TokenListId);
opaque_id!(GlueId);
opaque_id!(NodeListId);
opaque_id!(FontId);
opaque_id!(SnapshotId);

#[cfg(test)]
mod tests {
    use super::{FontId, GlueId, NodeListId, SnapshotId, TokenListId};

    #[test]
    fn placeholder_ids_preserve_raw_values_inside_the_crate() {
        assert_eq!(TokenListId::new(1).raw(), 1);
        assert_eq!(GlueId::new(2).raw(), 2);
        assert_eq!(NodeListId::new(3).raw(), 3);
        assert_eq!(FontId::new(4).raw(), 4);
        assert_eq!(SnapshotId::new(5).raw(), 5);
    }
}
