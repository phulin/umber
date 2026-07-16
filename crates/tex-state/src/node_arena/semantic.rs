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
    pub(crate) const fn fragment(self) -> crate::state_hash::StateHashFragment {
        crate::state_hash::StateHashFragment::from_parts(self.fingerprint, self.identity)
    }

    pub(crate) fn apply(self, hasher: &mut StateHasher) {
        hasher.u64(self.fingerprint);
        hasher.strong_identity(self.identity);
    }

    /// Constructs an identity whose bytes were independently validated by a
    /// frozen-format decoder.
    #[must_use]
    pub(crate) const fn from_validated(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub(crate) fn empty() -> Self {
        NodeSemanticIdBuilder::new().finish()
    }

    #[cfg(test)]
    pub(super) fn testing(value: u64) -> Self {
        Self {
            fingerprint: value,
            identity: crate::state_hash::strong_identity_bytes(
                b"umber-testing-node-id",
                &value.to_le_bytes(),
            ),
        }
    }
}

/// Current node semantic-identity scheme. Changing node tags, dependency
/// framing, or child-identity semantics requires a version migration.
pub(crate) const NODE_SEMANTIC_ID_VERSION: u8 = 1;
const NODE_STREAM_V1_DOMAIN: u64 = 0x6e6f_6431_5f73_7472;
const NODE_ID_V1_DOMAIN: u64 = 0x6e6f_6431_5f69_6465;

pub(crate) struct NodeSemanticIdBuilder {
    stream: StateHasher,
    len: usize,
}

impl NodeSemanticIdBuilder {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            stream: StateHasher::new(NODE_STREAM_V1_DOMAIN),
            len: 0,
        }
    }

    pub(crate) fn push(&mut self, encode: impl FnOnce(&mut StateHasher)) {
        encode(&mut self.stream);
        self.len += 1;
    }

    #[must_use]
    pub(crate) fn finish(self) -> NodeSemanticId {
        let mut hasher = StateHasher::new(NODE_ID_V1_DOMAIN);
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
