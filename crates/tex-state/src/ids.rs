//! Opaque store handles.
//!
//! `TokenListId` is minted by the token store. `OriginListId` is minted by the
//! provenance store. `GlueId` is minted by the glue store. `NodeListId` is
//! minted by node arenas. `FontId` is minted by the loaded font store.
//! `SnapshotId` becomes real in State M3.

macro_rules! opaque_id {
    ($name:ident) => {
        #[derive(
            Clone,
            Copy,
            Debug,
            Eq,
            Hash,
            Ord,
            PartialEq,
            PartialOrd,
            serde::Deserialize,
            serde::Serialize,
        )]
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
opaque_id!(OriginListId);
opaque_id!(MacroDefinitionId);
opaque_id!(GlueId);
opaque_id!(FontId);
opaque_id!(SnapshotId);

impl GlueId {
    /// The canonical zero-glue id, pre-interned by every glue store.
    pub const ZERO: Self = Self(0);
}

impl TokenListId {
    /// The canonical empty token-list id, pre-interned by every token store.
    pub const EMPTY: Self = Self(0);
}

impl OriginListId {
    /// The canonical empty origin-list id, preallocated by every provenance store.
    pub const EMPTY: Self = Self(0);
}

/// A survivor arena root slot.
#[derive(
    Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Deserialize, serde::Serialize,
)]
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
#[derive(
    Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Deserialize, serde::Serialize,
)]
pub enum ArenaRef {
    Epoch,
    Survivor(SurvivorRootId),
}

/// A frozen node-list span.
///
/// The packed representation is private: consumers inspect only the logical
/// arena, start, and length. Arena constructors are the sole production minting
/// boundary.
#[repr(transparent)]
#[derive(
    Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Deserialize, serde::Serialize,
)]
pub struct NodeListId(u64);

const NODE_LIST_SURVIVOR_BIT: u64 = 1 << 63;
const NODE_LIST_EPOCH_LEN_MAX: u32 = (1 << 31) - 1;
const NODE_LIST_SURVIVOR_ROOT_MAX: u32 = (1 << 20) - 2;
const NODE_LIST_SURVIVOR_START_MAX: u32 = (1 << 21) - 1;
const NODE_LIST_SURVIVOR_LEN_MAX: u32 = (1 << 22) - 1;
const NODE_LIST_NONE_WORD: u64 = u64::MAX;

const _: [(); 8] = [(); core::mem::size_of::<NodeListId>()];

impl NodeListId {
    pub(crate) const fn new_epoch(start: u32, len: u32) -> Self {
        assert!(
            len <= NODE_LIST_EPOCH_LEN_MAX,
            "epoch node-list length exceeds encoding"
        );
        assert!(
            start.checked_add(len).is_some(),
            "epoch node-list span overflows storage index"
        );
        Self((start as u64) | ((len as u64) << 32))
    }

    pub(crate) const fn new_survivor(root: SurvivorRootId, start: u32, len: u32) -> Self {
        assert!(
            root.raw() <= NODE_LIST_SURVIVOR_ROOT_MAX,
            "survivor root id exceeds encoding"
        );
        assert!(
            start <= NODE_LIST_SURVIVOR_START_MAX,
            "survivor span start exceeds encoding"
        );
        assert!(
            len <= NODE_LIST_SURVIVOR_LEN_MAX,
            "survivor span length exceeds encoding"
        );
        Self(
            NODE_LIST_SURVIVOR_BIT
                | ((root.raw() as u64) << 43)
                | ((start as u64) << 22)
                | (len as u64),
        )
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
        if self.0 & NODE_LIST_SURVIVOR_BIT == 0 {
            ArenaRef::Epoch
        } else {
            ArenaRef::Survivor(SurvivorRootId::new(
                ((self.0 >> 43) & ((1 << 20) - 1)) as u32,
            ))
        }
    }

    #[must_use]
    pub const fn start(self) -> u32 {
        if self.0 & NODE_LIST_SURVIVOR_BIT == 0 {
            self.0 as u32
        } else {
            ((self.0 >> 22) & (NODE_LIST_SURVIVOR_START_MAX as u64)) as u32
        }
    }

    #[must_use]
    pub const fn len(self) -> u32 {
        if self.0 & NODE_LIST_SURVIVOR_BIT == 0 {
            ((self.0 >> 32) & (NODE_LIST_EPOCH_LEN_MAX as u64)) as u32
        } else {
            (self.0 & (NODE_LIST_SURVIVOR_LEN_MAX as u64)) as u32
        }
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    pub(crate) const fn encode_box_word(value: Option<Self>) -> u64 {
        match value {
            Some(id) => id.0,
            None => NODE_LIST_NONE_WORD,
        }
    }

    pub(crate) const fn decode_box_word(word: u64) -> Option<Self> {
        if word == NODE_LIST_NONE_WORD {
            None
        } else {
            assert!(
                word & NODE_LIST_SURVIVOR_BIT == 0
                    || ((word >> 43) & ((1 << 20) - 1)) <= NODE_LIST_SURVIVOR_ROOT_MAX as u64,
                "box word contains reserved survivor root id"
            );
            Some(Self(word))
        }
    }
}

#[cfg(test)]
mod tests;
