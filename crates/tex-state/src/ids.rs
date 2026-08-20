//! Opaque store handles.
//!
//! `TokenListId` is minted by the runtime value registry. `OriginListId` is the compact
//! projection of a live reachability-owned origin list and is not independently
//! resolvable. `GlueId` and `MacroDefinitionId` are minted by the same runtime
//! value registry. `NodeListId` is minted by
//! node arenas. `FontId` is minted by the loaded font store. `SnapshotId`
//! becomes real in State M3.

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
    /// The canonical zero-glue id, pre-interned by every runtime registry.
    pub const ZERO: Self = Self(crate::identity::HandleIdentity::builtin(0));
}

impl TokenListId {
    /// The canonical empty token-list id, pre-interned by every runtime registry.
    pub const EMPTY: Self = Self(crate::identity::HandleIdentity::builtin(0));
}

impl OriginListId {
    /// The canonical empty structural origin-list projection.
    pub const EMPTY: Self = Self(crate::identity::HandleIdentity::builtin(0));
}

/// A compact coordinate namespace inside one structurally owned payload.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NodePayloadId(u32);

impl NodePayloadId {
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
    Owned(NodePayloadId),
}

/// A frozen node-list handle.
///
/// Production handles are borrow-scoped coordinates into a `NodeListRef`
/// payload. The epoch encoding is retained only for detached format keys and
/// test fixtures; it has no production lifetime authority.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NodeListId(crate::identity::HandleIdentity);

const NODE_LIST_OWNED_BIT: u64 = 1 << 63;
const NODE_LIST_EPOCH_LEN_MAX: u32 = (1 << 31) - 1;
const NODE_LIST_OWNED_ROOT_MAX: u32 = (1 << 20) - 2;
const NODE_LIST_OWNED_START_MAX: u32 = (1 << 21) - 1;
const NODE_LIST_OWNED_LEN_MAX: u32 = (1 << 22) - 1;
const NODE_LIST_NONE_WORD: u64 = u64::MAX;
const NODE_LIST_OWNED_NAMESPACE: u64 = 2;
const NODE_LIST_FORMAT_EPOCH_NAMESPACE: u64 = 3;
const NODE_LIST_FORMAT_OWNED_NAMESPACE: u64 = 4;

const _: [(); 16] = [(); core::mem::size_of::<NodeListId>()];

impl NodeListId {
    /// Whether this coordinate denotes the canonical empty node list.
    ///
    /// Live epoch handles intentionally do not expose their compact span, so
    /// callers must use this identity-aware query instead of reading `len`.
    pub(crate) fn is_empty(self) -> bool {
        if self.0.namespace() == NODE_LIST_OWNED_NAMESPACE
            || self.0.namespace() == NODE_LIST_FORMAT_EPOCH_NAMESPACE
            || self.0.namespace() == NODE_LIST_FORMAT_OWNED_NAMESPACE
        {
            self.len() == 0
        } else {
            self.0 == crate::identity::HandleIdentity::builtin(0)
        }
    }

    pub(crate) const fn new_owned(root: NodePayloadId, start: u32, len: u32) -> Self {
        assert!(
            root.raw() <= NODE_LIST_OWNED_ROOT_MAX,
            "node payload id exceeds encoding"
        );
        assert!(
            start <= NODE_LIST_OWNED_START_MAX,
            "owned span start exceeds encoding"
        );
        assert!(
            len <= NODE_LIST_OWNED_LEN_MAX,
            "owned span length exceeds encoding"
        );
        Self::from_reserved_word(
            NODE_LIST_OWNED_NAMESPACE,
            NODE_LIST_OWNED_BIT
                | ((root.raw() as u64) << 43)
                | ((start as u64) << 22)
                | (len as u64),
        )
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

    #[must_use]
    pub const fn arena(self) -> ArenaRef {
        if self.0.namespace() != NODE_LIST_OWNED_NAMESPACE
            && self.0.namespace() != NODE_LIST_FORMAT_OWNED_NAMESPACE
        {
            ArenaRef::Epoch
        } else {
            ArenaRef::Owned(NodePayloadId::new(
                ((self.reserved_word() >> 43) & ((1 << 20) - 1)) as u32,
            ))
        }
    }

    #[must_use]
    pub(crate) const fn start(self) -> u32 {
        assert!(
            self.0.namespace() == NODE_LIST_OWNED_NAMESPACE
                || self.0.namespace() == NODE_LIST_FORMAT_EPOCH_NAMESPACE
                || self.0.namespace() == NODE_LIST_FORMAT_OWNED_NAMESPACE,
            "live epoch node-list spans are arena-owned"
        );
        if self.0.namespace() == NODE_LIST_FORMAT_EPOCH_NAMESPACE {
            return self.0.lower();
        }
        let word = self.reserved_word();
        if word & NODE_LIST_OWNED_BIT == 0 {
            word as u32
        } else {
            ((word >> 22) & (NODE_LIST_OWNED_START_MAX as u64)) as u32
        }
    }

    #[must_use]
    pub(crate) const fn len(self) -> u32 {
        assert!(
            self.0.namespace() == NODE_LIST_OWNED_NAMESPACE
                || self.0.namespace() == NODE_LIST_FORMAT_EPOCH_NAMESPACE
                || self.0.namespace() == NODE_LIST_FORMAT_OWNED_NAMESPACE,
            "live epoch node-list spans are arena-owned"
        );
        if self.0.namespace() == NODE_LIST_FORMAT_EPOCH_NAMESPACE {
            return self.0.upper() - 1;
        }
        let word = self.reserved_word();
        if word & NODE_LIST_OWNED_BIT == 0 {
            ((word >> 32) & (NODE_LIST_EPOCH_LEN_MAX as u64)) as u32
        } else {
            (word & (NODE_LIST_OWNED_LEN_MAX as u64)) as u32
        }
    }

    pub(crate) const fn encode_box_word(value: Option<Self>) -> u64 {
        match value {
            Some(id) => {
                assert!(
                    id.0.namespace() == NODE_LIST_OWNED_NAMESPACE,
                    "box words require owned node-list coordinates"
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
                word & NODE_LIST_OWNED_BIT != 0,
                "box word is not an owned node-list coordinate"
            );
            assert!(
                ((word >> 43) & ((1 << 20) - 1)) <= NODE_LIST_OWNED_ROOT_MAX as u64,
                "box word contains a reserved node-payload id"
            );
            Some(Self::from_reserved_word(NODE_LIST_OWNED_NAMESPACE, word))
        }
    }
}
