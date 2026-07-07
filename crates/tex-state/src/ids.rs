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
                assert!(
                    self.len == 0,
                    "survivor node-list word encoding awaits umber2-2zl.4"
                );
                (1_u64 << 63) | u64::from(root.raw())
            }
        }
    }

    pub(crate) fn decode_word(word: u64) -> Self {
        if (word >> 63) == 0 {
            let start = word as u32;
            let len = (word >> 32) as u32;
            Self::new_epoch(start, len)
        } else {
            // TODO(umber2-2zl.4): decode full survivor node-list register words.
            Self::new_survivor(SurvivorRootId::new(word as u32), 0, 0)
        }
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
