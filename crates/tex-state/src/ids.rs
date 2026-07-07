//! Opaque store handles.
//!
//! `TokenListId` is minted by the token store. `GlueId` is minted by the glue
//! store. `NodeListId` is minted by node arenas. `FontId` becomes real in the
//! fonts epic. `SnapshotId` becomes real in State M3.

macro_rules! opaque_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(u32);

        impl $name {
            #[allow(dead_code)]
            pub(crate) const fn new(raw: u32) -> Self {
                Self(raw)
            }

            /// Creates a placeholder id for tests that cover raw Env storage.
            #[cfg(any(test, feature = "testing"))]
            #[must_use]
            pub const fn testing_new(raw: u32) -> Self {
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
opaque_id!(FontId);
opaque_id!(SnapshotId);

impl GlueId {
    /// The canonical zero-glue id, pre-interned by every glue store.
    pub const ZERO: Self = Self(0);
}

/// A survivor arena root slot.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SurvivorRootId(u32);

impl SurvivorRootId {
    pub(crate) const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// Creates a placeholder root for tests that cover raw Env storage.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub const fn testing_new(raw: u32) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// The arena namespace for a frozen node-list span.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ArenaRef {
    Epoch,
    Survivor(SurvivorRootId),
}

/// A frozen node-list span.
///
/// PERF: keep this unpacked for M2 clarity; the fastpaths epic can pack it if
/// profiling shows register pressure from the larger handle.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NodeListId {
    arena: ArenaRef,
    start: u32,
    len: u32,
}

impl NodeListId {
    pub(crate) const fn new_epoch(start: u32, len: u32) -> Self {
        Self {
            arena: ArenaRef::Epoch,
            start,
            len,
        }
    }

    pub(crate) const fn new_survivor(root: SurvivorRootId, start: u32, len: u32) -> Self {
        Self {
            arena: ArenaRef::Survivor(root),
            start,
            len,
        }
    }

    /// Creates a test-only epoch id without going through a node arena.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub const fn testing_epoch(start: u32, len: u32) -> Self {
        Self::new_epoch(start, len)
    }

    /// Creates a test-only survivor id without going through a survivor arena.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub const fn testing_survivor(root: u32, start: u32, len: u32) -> Self {
        Self::new_survivor(SurvivorRootId::new(root), start, len)
    }

    #[must_use]
    pub const fn arena(self) -> ArenaRef {
        self.arena
    }

    #[must_use]
    pub const fn start(self) -> u32 {
        self.start
    }

    #[must_use]
    pub const fn len(self) -> u32 {
        self.len
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    pub(crate) fn encode_word(self) -> u64 {
        match self.arena {
            ArenaRef::Epoch => u64::from(self.start) | (u64::from(self.len) << 32),
            ArenaRef::Survivor(root) => {
                assert!(root.raw() < (1 << 20), "survivor root id exceeds encoding");
                assert!(
                    self.start < (1 << 21),
                    "survivor span start exceeds encoding"
                );
                assert!(self.len < (1 << 22), "survivor span len exceeds encoding");
                (1_u64 << 63)
                    | (u64::from(root.raw()) << 43)
                    | (u64::from(self.start) << 22)
                    | u64::from(self.len)
            }
        }
    }

    pub(crate) fn decode_word(word: u64) -> Self {
        if (word >> 63) == 0 {
            let start = word as u32;
            let len = (word >> 32) as u32;
            Self::new_epoch(start, len)
        } else {
            let root = ((word >> 43) & ((1 << 20) - 1)) as u32;
            let start = ((word >> 22) & ((1 << 21) - 1)) as u32;
            let len = (word & ((1 << 22) - 1)) as u32;
            Self::new_survivor(SurvivorRootId::new(root), start, len)
        }
    }

    pub(crate) fn encode_box_word(value: Option<Self>) -> u64 {
        value.map_or(0, |id| {
            id.encode_word()
                .checked_add(1)
                .expect("node-list box-register word cannot encode u64::MAX")
        })
    }

    pub(crate) fn decode_box_word(word: u64) -> Option<Self> {
        (word != 0).then(|| Self::decode_word(word - 1))
    }
}

#[cfg(test)]
mod tests {
    use super::{FontId, GlueId, NodeListId, SnapshotId, TokenListId};

    #[test]
    fn placeholder_ids_preserve_raw_values_inside_the_crate() {
        assert_eq!(TokenListId::new(1).raw(), 1);
        assert_eq!(GlueId::new(2).raw(), 2);
        let nodes = NodeListId::new_epoch(3, 4);
        assert_eq!(nodes.start(), 3);
        assert_eq!(nodes.len(), 4);
        assert_eq!(FontId::new(4).raw(), 4);
        assert_eq!(SnapshotId::new(5).raw(), 5);
    }
}
