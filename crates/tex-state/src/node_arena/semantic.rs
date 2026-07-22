use crate::ContentHash;
use crate::state_hash::StateHasher;

/// Versioned, allocation-independent identity of one immutable node list.
///
/// Runtime node handles, arena positions, generations, and provenance never
/// participate. Child lists contribute their own already-frozen identities.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct NodeSemanticId {
    fingerprint: u64,
    identity: ContentHash,
}

impl NodeSemanticId {
    #[must_use]
    pub(crate) const fn value(self) -> u64 {
        self.fingerprint
    }

    #[must_use]
    pub(crate) const fn fragment(self) -> crate::state_hash::StateHashFragment {
        crate::state_hash::StateHashFragment::from_parts(self.fingerprint, self.identity)
    }

    pub(crate) fn apply(self, hasher: &mut StateHasher) {
        hasher.u64(self.fingerprint);
        hasher.semantic_identity(self.identity);
    }

    /// Temporary identity used only while a frozen graph is being installed.
    /// The loader replaces it with the recomputed strong identity before the
    /// restored stores can escape.
    #[must_use]
    pub(crate) const fn unverified_frozen(fingerprint: u64) -> Self {
        Self {
            fingerprint,
            identity: ContentHash::new([0; 32]),
        }
    }

    #[must_use]
    pub(crate) fn empty() -> Self {
        NodeSemanticIdBuilder::new().finish()
    }

    #[cfg(test)]
    pub(super) fn testing(value: u64) -> Self {
        Self {
            fingerprint: value,
            identity: crate::state_hash::semantic_identity_bytes(
                b"umber-testing-node-id",
                &value.to_le_bytes(),
            ),
        }
    }
}

/// Current node semantic-identity scheme. Changing node tags, dependency
/// framing, or child-identity semantics requires a version migration.
pub(crate) const NODE_SEMANTIC_ID_VERSION: u8 = 3;
const NODE_STREAM_V3_DOMAIN: u64 = 0x6e6f_6433_5f73_7472;
const NODE_ID_V3_DOMAIN: u64 = 0x6e6f_6433_5f69_6465;

pub(crate) struct NodeSemanticIdBuilder {
    stream: StateHasher,
    len: usize,
}

impl NodeSemanticIdBuilder {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            stream: StateHasher::new(NODE_STREAM_V3_DOMAIN),
            len: 0,
        }
    }

    pub(crate) fn push(&mut self, encode: impl FnOnce(&mut StateHasher)) {
        encode(&mut self.stream);
        self.len += 1;
    }

    /// Appends one length-framed encoding for several logical nodes.
    pub(crate) fn push_run(&mut self, len: usize, encode: impl FnOnce(&mut StateHasher)) {
        debug_assert!(len > 0);
        encode(&mut self.stream);
        self.len += len;
    }

    #[must_use]
    pub(crate) fn finish(self) -> NodeSemanticId {
        let mut hasher = StateHasher::new(NODE_ID_V3_DOMAIN);
        hasher.u8(NODE_SEMANTIC_ID_VERSION);
        hasher.usize(self.len);
        self.stream.finish_fragment().apply(&mut hasher);
        let fragment = hasher.finish_fragment();
        NodeSemanticId {
            fingerprint: fragment.fingerprint(),
            identity: fragment.identity(),
        }
    }
}
