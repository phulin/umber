//! Direct word-source admission into the final replay input row.

use super::levels::{
    BackedUpToken, BackupTreatment, InputLevel, InputLevelId, InputStorage, PackedInputFrame,
    PackedTokenOwnership, ReplayTrace, RetirementBehavior, StoredReplayReason, TokenBehavior,
    TransientReplayReason,
};
use crate::attempt::AttemptTokenListId;
use tex_state::packed_input::{InputFrameFlags, InputFrameKind, SourceContext};
#[cfg(test)]
use tex_state::token::Token;
use tex_state::token::{OriginId, TokenWord, TracedTokenWord};

#[inline]
pub(super) fn replay_kind(behavior: &TokenBehavior, trace: &ReplayTrace) -> InputFrameKind {
    // TeX82 §789 gives both an ordinary v-part and `omit_template` the
    // `v_template` token behavior. The trace is the sole distinction that
    // must survive packing so retirement can report which list actually ran.
    if matches!(
        (behavior, trace),
        (TokenBehavior::VTemplate, ReplayTrace::OmitTemplate)
    ) {
        return InputFrameKind::OmitTemplate;
    }
    match behavior {
        TokenBehavior::Parameter => InputFrameKind::Parameter,
        TokenBehavior::UTemplate => InputFrameKind::AlignmentUTemplate,
        TokenBehavior::VTemplate => InputFrameKind::AlignmentVTemplate,
        TokenBehavior::BackedUp(_) => InputFrameKind::BackedUp,
        TokenBehavior::Recovery => InputFrameKind::Recovery,
        TokenBehavior::Ordinary => match trace {
            ReplayTrace::Inserted => InputFrameKind::Inserted,
            ReplayTrace::Transient(TransientReplayReason::ExpandedTokenList) => {
                InputFrameKind::ExpandedTokenList
            }
            ReplayTrace::Stored(reason) => match reason {
                StoredReplayReason::OutputRoutine => InputFrameKind::OutputRoutine,
                StoredReplayReason::EveryPar => InputFrameKind::EveryPar,
                StoredReplayReason::EveryMath => InputFrameKind::EveryMath,
                StoredReplayReason::EveryDisplay => InputFrameKind::EveryDisplay,
                StoredReplayReason::EveryHBox => InputFrameKind::EveryHBox,
                StoredReplayReason::EveryVBox => InputFrameKind::EveryVBox,
                StoredReplayReason::EveryJob => InputFrameKind::EveryJob,
                StoredReplayReason::EveryCr => InputFrameKind::EveryCr,
                StoredReplayReason::EveryEof => InputFrameKind::EveryEof,
                StoredReplayReason::Mark => InputFrameKind::Mark,
                StoredReplayReason::Write => InputFrameKind::Write,
                StoredReplayReason::Discretionary => InputFrameKind::UmberReplay,
            },
            ReplayTrace::MacroReplacement => InputFrameKind::Macro,
            ReplayTrace::MacroParameter { .. } => InputFrameKind::Parameter,
            ReplayTrace::BackedUp => InputFrameKind::BackedUp,
            ReplayTrace::UTemplate => InputFrameKind::AlignmentUTemplate,
            ReplayTrace::VTemplate => InputFrameKind::AlignmentVTemplate,
            ReplayTrace::OmitTemplate => InputFrameKind::OmitTemplate,
        },
    }
}

#[inline]
pub(super) fn replay_flags(
    behavior: &TokenBehavior,
    retirement: RetirementBehavior,
) -> InputFrameFlags {
    let flags = match behavior {
        TokenBehavior::BackedUp(BackupTreatment::SuppressExpandableControlSequence) => {
            InputFrameFlags::SUPPRESS_EXPANDABLE_CONTROL_SEQUENCE
        }
        _ => InputFrameFlags::empty(),
    };
    flags.union(match retirement {
        RetirementBehavior::StopAtEnd => InputFrameFlags::STOP_AT_END,
        RetirementBehavior::RetainExhaustedVTemplate => InputFrameFlags::RETAIN_AT_END,
        RetirementBehavior::AwaitingVTemplateRetirement => {
            InputFrameFlags::RETAIN_AT_END.union(InputFrameFlags::AWAITING_V_TEMPLATE_RETIREMENT)
        }
        RetirementBehavior::Pop => InputFrameFlags::empty(),
    })
}

/// Appends an immutable semantic replay span when a copied transient sink is required.
#[inline]
pub(crate) fn push_untraced_words(words: &mut Vec<TracedTokenWord>, list: &[TokenWord]) {
    words.extend(
        list.iter()
            .map(|&word| TracedTokenWord::from_parts(word, OriginId::UNKNOWN)),
    );
}

/// Storage a token-list source may copy its words from.
pub(crate) struct TokenSources<'a, G> {
    pub(crate) attempt: &'a mut crate::attempt::AttemptArena<G>,
    pub(crate) pending_insertions: &'a mut Vec<TracedTokenWord>,
}

/// A source of words for one new token-list level.
pub(crate) trait PackedTokenSpanSource<G> {
    /// Admits words and constructs their final input row with its real extent.
    fn admit(
        self,
        words: &mut Vec<TracedTokenWord>,
        sources: TokenSources<'_, G>,
        identity: InputLevelId,
        source: Option<SourceContext>,
        kind: InputFrameKind,
        flags: InputFrameFlags,
    ) -> InputLevel<G>;
}

/// The escaping inserted list `scan_toks` collected at the end of
/// [`super::InputState::pending_insertions`], from `start` on.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PendingInsertion {
    pub(crate) start: u32,
}

impl<G> PackedTokenSpanSource<G> for PendingInsertion {
    fn admit(
        self,
        words: &mut Vec<TracedTokenWord>,
        sources: TokenSources<'_, G>,
        identity: InputLevelId,
        source: Option<SourceContext>,
        kind: InputFrameKind,
        flags: InputFrameFlags,
    ) -> InputLevel<G> {
        let list_start = words.len();
        assert!(list_start <= u32::MAX as usize, "input words exceed u32");
        let list_start = list_start as u32;
        let start = self.start as usize;
        words.extend_from_slice(&sources.pending_insertions[start..]);
        sources.pending_insertions.truncate(start);
        InputLevel {
            frame: PackedInputFrame::tokens(identity.0, words.len() - list_start as usize, source),
            storage: InputStorage::List {
                start: list_start,
                ownership: PackedTokenOwnership::Transient,
                kind,
                flags,
            },
        }
    }
}

impl<G> PackedTokenSpanSource<G> for tex_state::TokenListView<G> {
    #[inline]
    fn admit(
        self,
        _: &mut Vec<TracedTokenWord>,
        _: TokenSources<'_, G>,
        identity: InputLevelId,
        source: Option<SourceContext>,
        kind: InputFrameKind,
        flags: InputFrameFlags,
    ) -> InputLevel<G> {
        InputLevel {
            frame: PackedInputFrame::tokens(identity.0, self.len(), source),
            storage: InputStorage::StoredList {
                list: self.into_id(),
                kind,
                flags,
            },
        }
    }
}

impl<G> PackedTokenSpanSource<G> for AttemptTokenListId {
    fn admit(
        self,
        words: &mut Vec<TracedTokenWord>,
        sources: TokenSources<'_, G>,
        identity: InputLevelId,
        source: Option<SourceContext>,
        kind: InputFrameKind,
        flags: InputFrameFlags,
    ) -> InputLevel<G> {
        let list_start = words.len();
        assert!(list_start <= u32::MAX as usize, "input words exceed u32");
        let list_start = list_start as u32;
        words.extend(
            sources
                .attempt
                .token_words(&self)
                .expect("admitted attempt list")
                .iter()
                .copied(),
        );
        sources
            .attempt
            .discard_token_list(self)
            .expect("admitted attempt list consumed");
        InputLevel {
            frame: PackedInputFrame::tokens(identity.0, words.len() - list_start as usize, source),
            storage: InputStorage::List {
                start: list_start,
                ownership: PackedTokenOwnership::Stored,
                kind,
                flags,
            },
        }
    }
}

impl<G> PackedTokenSpanSource<G> for TracedTokenWord {
    #[inline]
    fn admit(
        self,
        _: &mut Vec<TracedTokenWord>,
        _: TokenSources<'_, G>,
        identity: InputLevelId,
        source: Option<SourceContext>,
        kind: InputFrameKind,
        flags: InputFrameFlags,
    ) -> InputLevel<G> {
        InputLevel {
            frame: PackedInputFrame::tokens(identity.0, 1, source),
            storage: InputStorage::Word {
                word: self,
                kind,
                flags,
            },
        }
    }
}

pub(crate) struct TracedReplaySeed<I> {
    tokens: I,
    ownership: PackedTokenOwnership,
}
impl<G, I: Iterator<Item = TracedTokenWord>> PackedTokenSpanSource<G> for TracedReplaySeed<I> {
    fn admit(
        self,
        words: &mut Vec<TracedTokenWord>,
        _: TokenSources<'_, G>,
        identity: InputLevelId,
        source: Option<SourceContext>,
        kind: InputFrameKind,
        flags: InputFrameFlags,
    ) -> InputLevel<G> {
        let list_start = words.len();
        assert!(list_start <= u32::MAX as usize, "input words exceed u32");
        let list_start = list_start as u32;
        words.extend(self.tokens);
        InputLevel {
            frame: PackedInputFrame::tokens(identity.0, words.len() - list_start as usize, source),
            storage: InputStorage::List {
                start: list_start,
                ownership: self.ownership,
                kind,
                flags,
            },
        }
    }
}

pub(crate) struct SemanticReplaySeed<'a> {
    words: &'a [TokenWord],
}
impl<G> PackedTokenSpanSource<G> for SemanticReplaySeed<'_> {
    fn admit(
        self,
        words: &mut Vec<TracedTokenWord>,
        _: TokenSources<'_, G>,
        identity: InputLevelId,
        source: Option<SourceContext>,
        kind: InputFrameKind,
        flags: InputFrameFlags,
    ) -> InputLevel<G> {
        let list_start = words.len();
        assert!(list_start <= u32::MAX as usize, "input words exceed u32");
        let list_start = list_start as u32;
        push_untraced_words(words, self.words);
        InputLevel {
            frame: PackedInputFrame::tokens(identity.0, words.len() - list_start as usize, source),
            storage: InputStorage::List {
                start: list_start,
                ownership: PackedTokenOwnership::Stored,
                kind,
                flags,
            },
        }
    }
}

pub(crate) struct BackedReplaySeed<I> {
    tokens: I,
}
impl<G, I: Iterator<Item = BackedUpToken>> PackedTokenSpanSource<G> for BackedReplaySeed<I> {
    fn admit(
        self,
        words: &mut Vec<TracedTokenWord>,
        _: TokenSources<'_, G>,
        identity: InputLevelId,
        source: Option<SourceContext>,
        kind: InputFrameKind,
        flags: InputFrameFlags,
    ) -> InputLevel<G> {
        let list_start = words.len();
        assert!(list_start <= u32::MAX as usize, "input words exceed u32");
        let list_start = list_start as u32;
        words.extend(self.tokens.map(|token| token.spelling));
        InputLevel {
            frame: PackedInputFrame::tokens(identity.0, words.len() - list_start as usize, source),
            storage: InputStorage::List {
                start: list_start,
                ownership: PackedTokenOwnership::BackedUp,
                kind,
                flags,
            },
        }
    }
}

#[cfg(test)] // The direct packed-span admission test uses this slice adapter.
pub(crate) struct StoredReplaySeed<'a, I> {
    tokens: &'a [Token],
    origins: I,
}
#[cfg(test)]
impl<G, I: Iterator<Item = OriginId>> PackedTokenSpanSource<G> for StoredReplaySeed<'_, I> {
    fn admit(
        mut self,
        words: &mut Vec<TracedTokenWord>,
        _: TokenSources<'_, G>,
        identity: InputLevelId,
        source: Option<SourceContext>,
        kind: InputFrameKind,
        flags: InputFrameFlags,
    ) -> InputLevel<G> {
        let list_start = words.len();
        assert!(list_start <= u32::MAX as usize, "input words exceed u32");
        let list_start = list_start as u32;
        words.extend(self.tokens.iter().map(|&token| {
            TracedTokenWord::pack(token, self.origins.next().unwrap_or(OriginId::UNKNOWN))
        }));
        InputLevel {
            frame: PackedInputFrame::tokens(identity.0, words.len() - list_start as usize, source),
            storage: InputStorage::List {
                start: list_start,
                ownership: PackedTokenOwnership::Stored,
                kind,
                flags,
            },
        }
    }
}

#[cfg(test)]
pub(crate) fn stored_replay(
    tokens: &[Token],
    origins: impl IntoIterator<Item = OriginId>,
) -> StoredReplaySeed<'_, impl Iterator<Item = OriginId>> {
    StoredReplaySeed {
        tokens,
        origins: origins.into_iter(),
    }
}

pub(crate) fn semantic_replay(words: &[TokenWord]) -> SemanticReplaySeed<'_> {
    SemanticReplaySeed { words }
}

/// One bounded insertion or scanner result.
pub(crate) fn transient_replay(
    tokens: impl IntoIterator<Item = TracedTokenWord>,
) -> TracedReplaySeed<impl Iterator<Item = TracedTokenWord>> {
    TracedReplaySeed {
        tokens: tokens.into_iter(),
        ownership: PackedTokenOwnership::Transient,
    }
}

/// Commands restored by `back_input` or a matched keyword prefix.
pub(crate) fn backed_up_replay(
    tokens: impl IntoIterator<Item = BackedUpToken>,
) -> BackedReplaySeed<impl Iterator<Item = BackedUpToken>> {
    BackedReplaySeed {
        tokens: tokens.into_iter(),
    }
}
