//! Dense source and token-list input-level ownership.
#![allow(dead_code)] // consumed by the ordered raw-delivery implementation issues

use std::sync::Arc;

use tex_state::ids::{OriginListId, TokenListId};
use tex_state::token::TracedTokenWord;

use crate::macro_call::{MacroActivationId, MacroArgumentRange};

use super::source::SourceCursor;

/// Stable identity for one live input level.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct InputLevelId(pub(crate) u64);

/// One future-relevant input level.
///
/// Conditions, caches, scanner policy, and paragraph transitions cannot be
/// represented here. Both character profiles use this same level structure.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum InputLevel {
    Source(SourceLevel),
    Tokens(TokenCursor),
}

/// One registered-source level and its exact delivery identity.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SourceLevel {
    pub(crate) identity: InputLevelId,
    pub(crate) cursor: SourceCursor,
}

/// One token-list cursor.
///
/// The four classified fields deliberately keep storage ownership, delivery
/// semantics, end-of-level handling, and diagnostic explanation independent.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct TokenCursor {
    pub(crate) payload: TokenPayload,
    pub(crate) behavior: TokenBehavior,
    pub(crate) retirement: RetirementBehavior,
    pub(crate) trace: ReplayTrace,
    pub(crate) index: usize,
    pub(crate) identity: InputLevelId,
}

/// Storage owning the tokens delivered by a token-list level.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TokenPayload {
    /// Immutable semantic tokens and their parallel immutable origins.
    Stored {
        tokens: TokenListId,
        origins: OriginListId,
    },
    /// Tokens materialized for a bounded insertion or scanner operation.
    Transient(SharedTokenBuffer),
    /// One already materialized macro argument, replayed literally by range.
    ArgumentRange {
        buffer: SharedTokenBuffer,
        range: MacroArgumentRange,
    },
}

/// Shared ownership of a contiguous traced-token allocation.
///
/// Cloning a cursor or snapshot retains the allocation rather than copying its
/// tokens. A macro activation and its parameter cursors may share this value.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct SharedTokenBuffer(Arc<[TracedTokenWord]>);

impl SharedTokenBuffer {
    pub(crate) fn new(tokens: impl Into<Arc<[TracedTokenWord]>>) -> Self {
        Self(tokens.into())
    }

    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }
}

/// Semantic treatment applied while a token level delivers its payload.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TokenBehavior {
    Ordinary,
    /// Replacement text associated with the sole activation owner.
    MacroBody(MacroActivationId),
    /// Literal replay of an already substituted macro argument.
    Parameter,
    BackedUp(BackupTreatment),
    UTemplate(TemplateId),
    VTemplate(TemplateId),
}

/// One-delivery handling attached to explicitly backed-up input.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum BackupTreatment {
    Ordinary,
    SuppressExpandableControlSequence,
}

/// Typed identity of one alignment template.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct TemplateId(pub(crate) u64);

/// Action selected only when a token payload is exhausted.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum RetirementBehavior {
    Pop,
    StopAtEnd,
    RetainExhaustedVTemplate,
    /// The exhausted v-template has reported its end and awaits `do_endv`.
    AwaitingVTemplateRetirement,
    CloseScantokens,
}

/// Non-semantic explanation for why a token payload is being replayed.
///
/// This value is diagnostic/provenance state. It cannot select expansion,
/// parameter substitution, backup treatment, or retirement.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ReplayTrace {
    Stored(StoredReplayReason),
    Transient(TransientReplayReason),
    MacroReplacement,
    MacroParameter { slot: u8 },
    BackedUp,
    UTemplate,
    VTemplate,
}

/// Canonical explanations for immutable stored token-list replay.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum StoredReplayReason {
    TokenList,
    TokenRegister(u16),
    TokenParameter(u16),
    EveryPar,
    EveryMath,
    EveryDisplay,
    EveryHBox,
    EveryVBox,
    EveryJob,
    EveryCr,
    EveryEof,
    Mark,
    OutputRoutine,
    Write,
}

/// Canonical explanations for a materialized transient insertion.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TransientReplayReason {
    Inserted,
    Scantokens,
    ExpandedTokenList,
}

#[cfg(test)]
mod tests;
