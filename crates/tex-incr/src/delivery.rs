//! Stable execution-trace delivery identities.

use std::sync::Arc;

use tex_lex::SourceDeliveryId;
use tex_state::ContentHash;

/// Stable occurrence identity of one token delivery.
///
/// Diagnostic origins never participate. Semantic content identities may be
/// shared, while the parent occurrence preserves ordering and duplicate-text
/// distinctions in the execution trace.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum DeliveryIdentity {
    SessionRoot(ContentHash),
    Source(SourceDeliveryId),
    Macro {
        definition: ContentHash,
        invocation: Arc<Self>,
        argument_path: Arc<[u8]>,
        token_index: u32,
    },
    TokenList {
        content: ContentHash,
        token_index: u32,
        parent: Arc<Self>,
    },
    Synthetic {
        kind: SyntheticDeliveryKind,
        parent: Arc<Self>,
    },
}

/// Versioned semantic category for inserted delivery.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SyntheticDeliveryKind(u16);

impl SyntheticDeliveryKind {
    #[must_use]
    pub const fn new(schema_code: u16) -> Self {
        Self(schema_code)
    }

    #[must_use]
    pub const fn schema_code(self) -> u16 {
        self.0
    }
}

#[cfg(test)]
mod tests;
