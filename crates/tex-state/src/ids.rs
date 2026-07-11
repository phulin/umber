//! Opaque store handles.
//!
//! `TokenListId` is minted by the token store. `OriginListId` is minted by the
//! provenance store. `GlueId` is minted by the glue store. `NodeListId` is
//! minted by node arenas. `FontId` is minted by the loaded font store.
//! `SnapshotId` becomes real in State M3.

macro_rules! opaque_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(u32);

        impl $name {
            #[allow(dead_code)]
            #[allow(unused_comparisons)]
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

opaque_id!(SnapshotId);

macro_rules! semantic_id {
    ($name:ident, $namespace:expr, $builtin_slots:expr) => {
        #[repr(transparent)]
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(crate::identity::HandleIdentity);

        #[allow(dead_code, unused_comparisons)]
        impl $name {
            pub(crate) const fn new(raw: u32) -> Self {
                if raw < $builtin_slots {
                    Self(crate::identity::HandleIdentity::builtin(raw))
                } else {
                    Self(crate::identity::HandleIdentity::reserved(
                        $namespace,
                        core::num::NonZeroU32::MIN,
                        raw,
                    ))
                }
            }

            pub(crate) const fn from_identity(identity: crate::identity::HandleIdentity) -> Self {
                Self(identity)
            }

            pub(crate) const fn builtin(slot: u32) -> Self {
                Self(crate::identity::HandleIdentity::builtin(slot))
            }

            pub(crate) const fn identity(self) -> crate::identity::HandleIdentity {
                self.0
            }

            pub(crate) const fn is_stored(self) -> bool {
                self.0.namespace() == $namespace
            }

            /// Creates a placeholder id for tests that cover compact stored words.
            #[cfg(any(test, feature = "testing"))]
            #[must_use]
            pub const fn testing_new(raw: u32) -> Self {
                Self::new(raw)
            }

            /// Returns the dense store slot used by semantic DTOs and packed words.
            #[must_use]
            pub const fn raw(self) -> u32 {
                self.0.slot()
            }
        }
    };
}

semantic_id!(TokenListId, 10, 1);
semantic_id!(MacroDefinitionId, 11, 0);
semantic_id!(GlueId, 12, 1);
semantic_id!(FontId, 13, 1);
semantic_id!(OriginListId, 14, 1);

impl GlueId {
    /// The canonical zero-glue id, pre-interned by every glue store.
    pub const ZERO: Self = Self(crate::identity::HandleIdentity::builtin(0));
}

impl TokenListId {
    /// The canonical empty token-list id, pre-interned by every token store.
    pub const EMPTY: Self = Self(crate::identity::HandleIdentity::builtin(0));
}

impl OriginListId {
    /// The canonical empty origin-list id, preallocated by every provenance store.
    pub const EMPTY: Self = Self(crate::identity::HandleIdentity::builtin(0));
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

/// A frozen node-list handle.
///
/// Epoch handles contain only a generation-tagged allocation identity; their
/// compact `(start, len)` span is resolved by the owning node arena in O(1).
/// Survivor handles retain their self-contained packed span. Constructors are
/// the sole production minting boundary.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NodeListId(crate::identity::HandleIdentity);

const NODE_LIST_SURVIVOR_BIT: u64 = 1 << 63;
const NODE_LIST_EPOCH_LEN_MAX: u32 = (1 << 31) - 1;
const NODE_LIST_SURVIVOR_ROOT_MAX: u32 = (1 << 20) - 2;
const NODE_LIST_SURVIVOR_START_MAX: u32 = (1 << 21) - 1;
const NODE_LIST_SURVIVOR_LEN_MAX: u32 = (1 << 22) - 1;
const NODE_LIST_NONE_WORD: u64 = u64::MAX;
const NODE_LIST_SURVIVOR_NAMESPACE: u64 = 2;
const NODE_LIST_FORMAT_EPOCH_NAMESPACE: u64 = 3;
const NODE_LIST_FORMAT_SURVIVOR_NAMESPACE: u64 = 4;

const _: [(); 16] = [(); core::mem::size_of::<NodeListId>()];

impl NodeListId {
    pub(crate) const fn new_epoch(identity: crate::identity::HandleIdentity) -> Self {
        assert!(
            identity.namespace() != NODE_LIST_SURVIVOR_NAMESPACE
                && identity.namespace() != NODE_LIST_FORMAT_EPOCH_NAMESPACE
                && identity.namespace() != NODE_LIST_FORMAT_SURVIVOR_NAMESPACE,
            "epoch identity uses a reserved node-list namespace"
        );
        Self(identity)
    }

    #[cfg(any(test, feature = "testing"))]
    const fn packed_epoch_span(start: u32, len: u32) -> u64 {
        assert!(
            len <= NODE_LIST_EPOCH_LEN_MAX,
            "epoch node-list length exceeds encoding"
        );
        assert!(
            start.checked_add(len).is_some(),
            "epoch node-list span overflows storage index"
        );
        (start as u64) | ((len as u64) << 32)
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
        Self::from_reserved_word(
            NODE_LIST_SURVIVOR_NAMESPACE,
            NODE_LIST_SURVIVOR_BIT
                | ((root.raw() as u64) << 43)
                | ((start as u64) << 22)
                | (len as u64),
        )
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) const fn format_reference(arena: ArenaRef, start: u32, len: u32) -> Self {
        match arena {
            ArenaRef::Epoch => {
                let _ = Self::packed_epoch_span(start, len);
                let incremented = match len.checked_add(1) {
                    Some(value) => value,
                    None => panic!("epoch node-list length exceeds DTO encoding"),
                };
                let encoded_len = match core::num::NonZeroU32::new(incremented) {
                    Some(value) => value,
                    None => panic!("epoch node-list DTO length must be nonzero"),
                };
                Self(crate::identity::HandleIdentity::reserved(
                    NODE_LIST_FORMAT_EPOCH_NAMESPACE,
                    encoded_len,
                    start,
                ))
            }
            ArenaRef::Survivor(root) => Self::from_reserved_word(
                NODE_LIST_FORMAT_SURVIVOR_NAMESPACE,
                Self::new_survivor(root, start, len).reserved_word(),
            ),
        }
    }

    const fn from_reserved_word(namespace: u64, word: u64) -> Self {
        let upper = match core::num::NonZeroU32::new((word >> 32) as u32) {
            Some(value) => value,
            None => panic!("reserved node-list word has a zero upper half"),
        };
        Self(crate::identity::HandleIdentity::reserved(
            namespace,
            upper,
            word as u32,
        ))
    }

    const fn reserved_word(self) -> u64 {
        ((self.0.upper() as u64) << 32) | self.0.lower() as u64
    }

    pub(crate) const fn epoch_identity(self) -> crate::identity::HandleIdentity {
        assert!(
            self.0.namespace() != NODE_LIST_SURVIVOR_NAMESPACE
                && self.0.namespace() != NODE_LIST_FORMAT_EPOCH_NAMESPACE
                && self.0.namespace() != NODE_LIST_FORMAT_SURVIVOR_NAMESPACE,
            "node-list handle is not a live epoch identity"
        );
        self.0
    }

    pub(crate) const fn is_format_reference(self) -> bool {
        self.0.namespace() == NODE_LIST_FORMAT_EPOCH_NAMESPACE
            || self.0.namespace() == NODE_LIST_FORMAT_SURVIVOR_NAMESPACE
    }

    /// Creates a test-only epoch id without going through a node arena.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub const fn testing_epoch(start: u32, len: u32) -> Self {
        Self::format_reference(ArenaRef::Epoch, start, len)
    }

    /// Creates a test-only survivor id without going through a survivor arena.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub const fn testing_survivor(root: u32, start: u32, len: u32) -> Self {
        Self::new_survivor(SurvivorRootId::new(root), start, len)
    }

    #[must_use]
    pub const fn arena(self) -> ArenaRef {
        if self.0.namespace() != NODE_LIST_SURVIVOR_NAMESPACE
            && self.0.namespace() != NODE_LIST_FORMAT_SURVIVOR_NAMESPACE
        {
            ArenaRef::Epoch
        } else {
            ArenaRef::Survivor(SurvivorRootId::new(
                ((self.reserved_word() >> 43) & ((1 << 20) - 1)) as u32,
            ))
        }
    }

    #[must_use]
    pub(crate) const fn start(self) -> u32 {
        assert!(
            self.0.namespace() == NODE_LIST_SURVIVOR_NAMESPACE
                || self.0.namespace() == NODE_LIST_FORMAT_EPOCH_NAMESPACE
                || self.0.namespace() == NODE_LIST_FORMAT_SURVIVOR_NAMESPACE,
            "live epoch node-list spans are arena-owned"
        );
        if self.0.namespace() == NODE_LIST_FORMAT_EPOCH_NAMESPACE {
            return self.0.lower();
        }
        let word = self.reserved_word();
        if word & NODE_LIST_SURVIVOR_BIT == 0 {
            word as u32
        } else {
            ((word >> 22) & (NODE_LIST_SURVIVOR_START_MAX as u64)) as u32
        }
    }

    #[must_use]
    pub(crate) const fn len(self) -> u32 {
        assert!(
            self.0.namespace() == NODE_LIST_SURVIVOR_NAMESPACE
                || self.0.namespace() == NODE_LIST_FORMAT_EPOCH_NAMESPACE
                || self.0.namespace() == NODE_LIST_FORMAT_SURVIVOR_NAMESPACE,
            "live epoch node-list spans are arena-owned"
        );
        if self.0.namespace() == NODE_LIST_FORMAT_EPOCH_NAMESPACE {
            return self.0.upper() - 1;
        }
        let word = self.reserved_word();
        if word & NODE_LIST_SURVIVOR_BIT == 0 {
            ((word >> 32) & (NODE_LIST_EPOCH_LEN_MAX as u64)) as u32
        } else {
            (word & (NODE_LIST_SURVIVOR_LEN_MAX as u64)) as u32
        }
    }

    pub(crate) const fn encode_box_word(value: Option<Self>) -> u64 {
        match value {
            Some(id) => {
                assert!(
                    id.0.namespace() == NODE_LIST_SURVIVOR_NAMESPACE,
                    "box words require survivor node-list handles"
                );
                id.reserved_word()
            }
            None => NODE_LIST_NONE_WORD,
        }
    }

    pub(crate) const fn decode_box_word(word: u64) -> Option<Self> {
        if word == NODE_LIST_NONE_WORD {
            None
        } else {
            assert!(
                word & NODE_LIST_SURVIVOR_BIT != 0,
                "box word is not a survivor handle"
            );
            assert!(
                ((word >> 43) & ((1 << 20) - 1)) <= NODE_LIST_SURVIVOR_ROOT_MAX as u64,
                "box word contains reserved survivor root id"
            );
            Some(Self::from_reserved_word(NODE_LIST_SURVIVOR_NAMESPACE, word))
        }
    }
}

#[cfg(test)]
mod tests;
