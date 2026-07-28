//! Dense source and token-list input-level ownership.
#![allow(dead_code)] // consumed by the ordered raw-delivery implementation issues

use std::sync::Arc;

use tex_state::ids::{OriginListId, TokenListId};
use tex_state::token::TracedTokenWord;

use crate::macro_call::{MacroActivationId, MacroArgumentRange};

use super::{
    lines::SourceProvenance,
    source::{SourceCursor, SourceNameClass},
};

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
    /// tex.web §303's `name` classification for this level. A token-list
    /// level has no counterpart: §307 reuses `name` there as the eqtb address
    /// of the macro being expanded, which is why this lives on `SourceLevel`
    /// and not on [`InputLevel`].
    pub(crate) name_class: SourceNameClass,
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
    /// One command restored by TeX's `back_input`, retaining the committed
    /// physical spelling range if it originally came from registered source.
    BackedUp(SharedBackedUpBuffer),
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

    pub(crate) fn get(&self, index: usize) -> Option<TracedTokenWord> {
        self.0.get(index).copied()
    }
}

/// Shared ownership of commands restored by `back_input` or scanner replay.
///
/// The range is delivery metadata, separate from the token's semantic and
/// opaque-origin identity, so reusing it cannot affect fixture identities.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct SharedBackedUpBuffer(Arc<[BackedUpToken]>);

impl SharedBackedUpBuffer {
    pub(crate) fn new(tokens: impl Into<Arc<[BackedUpToken]>>) -> Self {
        Self(tokens.into())
    }

    pub(crate) fn get(&self, index: usize) -> Option<BackedUpToken> {
        self.0.get(index).copied()
    }
}

/// One restored command plus the source range committed at its first delivery.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct BackedUpToken {
    pub(crate) spelling: TracedTokenWord,
    pub(crate) source_provenance: Option<SourceProvenance>,
}

/// Semantic treatment applied while a token level delivers its payload.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TokenBehavior {
    Ordinary,
    /// A TeX recovery insertion that must retire before a scanner backs its
    /// consumed token up for ordinary replay.
    Recovery,
    /// Replacement text associated with the sole activation owner.
    MacroBody(MacroActivationId),
    /// Literal replay of an already substituted macro argument.
    Parameter,
    BackedUp(BackupTreatment),
    UTemplate,
    VTemplate,
}

/// One-delivery handling attached to explicitly backed-up input.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum BackupTreatment {
    Ordinary,
    SuppressExpandableControlSequence,
}

/// Action selected only when a token payload is exhausted.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum RetirementBehavior {
    Pop,
    StopAtEnd,
    RetainExhaustedVTemplate,
    /// The exhausted v-template has reported its frozen `end_template`
    /// boundary. tex.web §§325/390 still refuse to drain it for stack
    /// conservation and §1131's `do_endv` still expects to find it, but
    /// §357's `end_token_list` pops it as soon as `get_next` reaches it.
    AwaitingVTemplateRetirement,
    CloseScantokens,
}

/// Non-semantic explanation for why a token payload is being replayed.
///
/// This value is diagnostic/provenance state. It cannot select expansion,
/// parameter substitution, backup treatment, or retirement.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ReplayTrace {
    /// TeX82 §307's `inserted` token type: the level TeX82 §323's `ins_list`
    /// installs.
    ///
    /// This is a token _type_, not a storage strategy, so it is independent of
    /// whether the payload is a fresh transient buffer (§470's `conv_toks`
    /// renders one) or an immutable stored list (§467's `ins_the_toks` shares
    /// §465's copy). Nesting it under [`ReplayTrace::Transient`] conflated the
    /// two and let §467's inserted level be installed as an ordinary stored
    /// token list.
    Inserted,
    Stored(StoredReplayReason),
    Transient(TransientReplayReason),
    MacroReplacement,
    MacroParameter {
        slot: u8,
    },
    BackedUp,
    UTemplate,
    VTemplate,
    /// tex.web §789's `begin_token_list(omit_template,v_template)`: an
    /// `\omit` entry installs the shared constant list `omit_template`
    /// instead of the column's ⟨v_j⟩ part. Both are `token_type=v_template`
    /// (§307), so this is a trace distinction only -- exactly the one the
    /// pinned observer makes with `start=omit_template` when it names a
    /// retiring level.
    OmitTemplate,
}

/// Canonical explanations for immutable stored token-list replay.
///
/// The first block is one tex.web §307 `token_type` each -- the token lists
/// TeX82 installs with `begin_token_list` and names in its own input trace.
/// The second block is Umber's own: replay levels the command state owns for
/// material tex.web reads live, which therefore have no §307 identity to
/// borrow.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum StoredReplayReason {
    /// §307 `output_text=6`.
    OutputRoutine,
    /// §307 `every_par_text=7`.
    EveryPar,
    /// §307 `every_math_text=8`.
    EveryMath,
    /// §307 `every_display_text=9`.
    EveryDisplay,
    /// §307 `every_hbox_text=10`.
    EveryHBox,
    /// §307 `every_vbox_text=11`.
    EveryVBox,
    /// §307 `every_job_text=12`.
    EveryJob,
    /// §307 `every_cr_text=13`.
    EveryCr,
    /// §307 `mark_text=14`.
    Mark,
    /// §307 `write_text=15`.
    Write,
    Discretionary,
    AfterAssignment,
}

/// Canonical explanations for a materialized transient insertion that is not
/// TeX82 §307's `inserted` token type (which is [`ReplayTrace::Inserted`]).
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TransientReplayReason {
    Scantokens,
    ExpandedTokenList,
}

#[cfg(test)]
mod tests;
