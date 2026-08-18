//! Dense source and token-list input-level ownership.
#![allow(dead_code)] // consumed by the ordered raw-delivery implementation issues

use std::sync::Arc;

use smallvec::SmallVec;
use tex_state::ids::MacroDefinitionId;
use tex_state::packed_input::{InputFrameFlags, InputFrameKind};
use tex_state::provenance::{OriginListRef, OriginRef};
use tex_state::token::{RootedTracedTokenBuffer, RootedTracedTokenWord, TracedTokenWord};
use tex_state::token_store::TokenListRef;

use crate::macro_call::{MacroActivationId, MacroArgumentRange};

use super::{
    lines::SourceProvenance,
    source::{SourceCursor, SourceNameClass},
};

/// Stable identity for one live input level.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct InputLevelId(pub(crate) u64);

pub(crate) use tex_state::packed_input::InputFrame as PackedInputFrame;

fn packed_frame_kind(behavior: &TokenBehavior, trace: &ReplayTrace) -> InputFrameKind {
    match behavior {
        TokenBehavior::Parameter => InputFrameKind::Parameter,
        TokenBehavior::UTemplate => InputFrameKind::AlignmentUTemplate,
        TokenBehavior::VTemplate => InputFrameKind::AlignmentVTemplate,
        TokenBehavior::BackedUp(_) => InputFrameKind::BackedUp,
        TokenBehavior::Recovery => InputFrameKind::Inserted,
        TokenBehavior::MacroBody(_) => InputFrameKind::Macro,
        TokenBehavior::Ordinary => match trace {
            ReplayTrace::Inserted | ReplayTrace::Transient(_) => InputFrameKind::Inserted,
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
            ReplayTrace::VTemplate | ReplayTrace::OmitTemplate => {
                InputFrameKind::AlignmentVTemplate
            }
        },
    }
}

pub(crate) fn packed_token_frame(
    identity: InputLevelId,
    len: usize,
    behavior: &TokenBehavior,
    retirement: RetirementBehavior,
    trace: &ReplayTrace,
) -> PackedInputFrame {
    let len = u32::try_from(len).expect("input token chunk exceeds the packed offset domain");
    let mut flags = match behavior {
        TokenBehavior::BackedUp(BackupTreatment::SuppressExpandableControlSequence) => {
            InputFrameFlags::SUPPRESS_EXPANDABLE_CONTROL_SEQUENCE
        }
        _ => InputFrameFlags::empty(),
    };
    flags = flags.union(match retirement {
        RetirementBehavior::StopAtEnd => InputFrameFlags::STOP_AT_END,
        RetirementBehavior::RetainExhaustedVTemplate
        | RetirementBehavior::AwaitingVTemplateRetirement => InputFrameFlags::RETAIN_AT_END,
        RetirementBehavior::Pop => InputFrameFlags::empty(),
    });
    PackedInputFrame::tokens(identity.0, len, packed_frame_kind(behavior, trace), flags)
}

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
    pub(crate) frame: PackedInputFrame,
    pub(crate) cursor: SourceCursor,
    /// tex.web §303's `name` classification for this level. A token-list
    /// level has no counterpart: §307 reuses `name` there as the eqtb address
    /// of the macro being expanded, which is why this lives on `SourceLevel`
    /// and not on [`InputLevel`].
    pub(crate) name_class: SourceNameClass,
    pub(crate) retirement: SourceRetirement,
    /// e-TeX §24.362's once-only token list, pushed above this source when
    /// natural EOF is first observed and before `end_file_reading`.
    pub(crate) every_eof: Option<tex_state::TracedTokenList>,
    /// e-TeX 2.6 [23.328]'s `grp_stack[in_open]`/`if_stack[in_open]`: the
    /// live group and conditional boundary ancestry recorded when this
    /// level's `begin_file_reading` ran, compared against the current stacks
    /// at `end_file_reading` to drive `\tracingnesting`'s `file_warning`.
    /// `None` until the opener records it (this crate has no `Universe`
    /// access at construction time; see `CommandState::record_source_open_depths`).
    pub(crate) open_depths: Option<Box<SourceOpenDepths>>,
}

impl SourceLevel {
    pub(crate) fn identity(&self) -> InputLevelId {
        InputLevelId(self.frame.identity())
    }
}

/// e-TeX 2.6's `grp_stack`/`if_stack` entry for one open source level.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SourceOpenDepths {
    pub(crate) group_lineages: Box<[u64]>,
    pub(crate) conditional_identities: Box<[u64]>,
}

/// What exhausting a source level does, per tex.web §360.
///
/// §360 branches on `name`, the level's file identity: `if name>17 then
/// <read the next line, or end the file>` and otherwise, for a `\read`
/// pseudo-file, `if not terminal_input then {\read line has ended} begin
/// cur_cmd:=0; cur_chr:=0; return; end`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) enum SourceRetirement {
    /// §362's `name>17`: `end_file_reading` and resume the enclosing level.
    #[default]
    Pop,
    /// §483's `name:=m+1`: one acquired line, whose exhaustion is §360's
    /// `cur_tok=0` and ends the `\read` collection rather than falling
    /// through to whatever was being read before.
    EndReadLine,
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
    pub(crate) frame: PackedInputFrame,
}

impl TokenCursor {
    pub(crate) fn identity(&self) -> InputLevelId {
        InputLevelId(self.frame.identity())
    }

    pub(crate) fn position(&self) -> usize {
        self.frame.position() as usize
    }
}

/// Storage owning the tokens delivered by a token-list level.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TokenPayload {
    /// Chunk-owned packed words used by canonical source-adjacent replay,
    /// hooks, templates, insertions, and backup. The sparse roots are owned
    /// once by the chunk rather than by each input frame or delivered word.
    Packed(PackedTokenChunk),
    /// Immutable semantic tokens and their parallel immutable origins.
    Stored {
        tokens: TokenListRef,
        origins: OriginListRef,
    },
    /// Replacement replay borrowed from one command-admitted macro chunk.
    MacroReplacement {
        admitted: u32,
        definition: MacroDefinitionId,
        len: u32,
    },
    /// Tokens materialized for a bounded insertion or scanner operation.
    Transient(SharedTokenBuffer),
    /// One transient token stored directly in its input level.
    InlineTransient(RootedTracedTokenWord),
    /// One command restored by TeX's `back_input`, retaining the committed
    /// physical spelling range if it originally came from registered source.
    BackedUp(SharedBackedUpBuffer),
    /// One backed-up command stored directly in its input level.
    InlineBackedUp(RootedBackedUpToken),
    /// One already materialized macro argument, replayed literally by range.
    ArgumentRange {
        arguments: crate::macro_call::MacroArguments,
        range: MacroArgumentRange,
    },
}

/// One packed token chunk and the cold source coordinates needed only when a
/// backed-up delivery is rendered. Ordinary delivery indexes the packed word
/// slice directly and does not clone this owner.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct PackedTokenChunk {
    words: RootedTracedTokenBuffer,
    source_provenance: SmallVec<[Option<SourceProvenance>; 2]>,
    backed_up: bool,
}

impl PackedTokenChunk {
    fn from_payload(payload: TokenPayload) -> TokenPayload {
        match payload {
            TokenPayload::Packed(chunk) => TokenPayload::Packed(chunk),
            TokenPayload::Transient(buffer) => TokenPayload::Packed(Self {
                words: RootedTracedTokenBuffer::new(buffer.rooted_words()),
                source_provenance: smallvec::smallvec![None; buffer.len()],
                backed_up: false,
            }),
            TokenPayload::InlineTransient(word) => TokenPayload::Packed(Self {
                words: RootedTracedTokenBuffer::new([word]),
                source_provenance: smallvec::smallvec![None],
                backed_up: false,
            }),
            TokenPayload::BackedUp(buffer) => {
                let len = buffer.words().len();
                let rooted = (0..len).map(|index| {
                    let token = buffer
                        .get_rooted(index)
                        .expect("index from packed backup length");
                    let (token, root) = token.into_parts();
                    RootedTracedTokenWord::from_word(token.spelling, root)
                });
                let source_provenance = buffer
                    .words()
                    .iter()
                    .map(|token| token.source_provenance)
                    .collect();
                TokenPayload::Packed(Self {
                    words: RootedTracedTokenBuffer::new(rooted),
                    source_provenance,
                    backed_up: true,
                })
            }
            TokenPayload::InlineBackedUp(token) => {
                let (token, root) = token.into_parts();
                TokenPayload::Packed(Self {
                    words: RootedTracedTokenBuffer::new([RootedTracedTokenWord::from_word(
                        token.spelling,
                        root,
                    )]),
                    source_provenance: smallvec::smallvec![token.source_provenance],
                    backed_up: true,
                })
            }
            payload @ (TokenPayload::Stored { .. }
            | TokenPayload::MacroReplacement { .. }
            | TokenPayload::ArgumentRange { .. }) => payload,
        }
    }

    fn from_stored(tokens: TokenListRef, origins: OriginListRef) -> Self {
        let words = tokens
            .tokens()
            .iter()
            .copied()
            .enumerate()
            .map(|(index, token)| {
                RootedTracedTokenWord::new(
                    token,
                    origins.root(index).unwrap_or_else(OriginRef::unknown),
                )
            });
        let len = tokens.tokens().len();
        Self {
            words: RootedTracedTokenBuffer::new(words),
            source_provenance: smallvec::smallvec![None; len],
            backed_up: false,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.words.len()
    }

    pub(crate) fn get(
        &self,
        index: usize,
    ) -> Option<(RootedTracedTokenWord, Option<SourceProvenance>)> {
        Some((
            self.words.get_rooted(index)?,
            self.source_provenance.get(index).copied().flatten(),
        ))
    }

    pub(crate) fn word(&self, index: usize) -> Option<TracedTokenWord> {
        self.words.get(index)
    }

    fn backed_up_token(&self, index: usize) -> Option<BackedUpToken> {
        if !self.backed_up {
            return None;
        }
        Some(BackedUpToken {
            spelling: self.words.get(index)?,
            source_provenance: self.source_provenance.get(index).copied().flatten(),
        })
    }

    pub(crate) fn rooted_words(&self) -> impl ExactSizeIterator<Item = RootedTracedTokenWord> + '_ {
        self.words.rooted_words()
    }

    pub(crate) fn source_provenance(&self) -> &[Option<SourceProvenance>] {
        &self.source_provenance
    }

    pub(crate) const fn is_backed_up(&self) -> bool {
        self.backed_up
    }
}

impl TokenPayload {
    pub(crate) fn packed_for_frame(self, behavior: &TokenBehavior) -> Self {
        match self {
            Self::Stored { tokens, origins }
                if !matches!(behavior, TokenBehavior::MacroBody(_)) =>
            {
                Self::Packed(PackedTokenChunk::from_stored(tokens, origins))
            }
            payload => PackedTokenChunk::from_payload(payload),
        }
    }

    pub(crate) fn frame_len(&self) -> usize {
        match self {
            Self::Packed(chunk) => chunk.len(),
            Self::Stored { tokens, .. } => tokens.tokens().len(),
            Self::MacroReplacement { len, .. } => *len as usize,
            Self::Transient(words) => words.len(),
            Self::InlineTransient(_) | Self::InlineBackedUp(_) => 1,
            Self::BackedUp(words) => words.words().len(),
            Self::ArgumentRange { range, .. } => range.end().saturating_sub(range.start()),
        }
    }

    /// Selects inline storage for one transient token and shared storage for
    /// empty or multi-token payloads. The fixed two-token case constructs its
    /// shared slice directly from an array; longer iterators materialize the
    /// unbounded owned buffer once.
    pub(crate) fn transient(tokens: impl IntoIterator<Item = TracedTokenWord>) -> Self {
        let mut tokens = tokens.into_iter();
        let Some(first) = tokens.next() else {
            return Self::Transient(SharedTokenBuffer::default());
        };
        let Some(second) = tokens.next() else {
            return Self::InlineTransient(RootedTracedTokenWord::unowned(first));
        };
        let Some(third) = tokens.next() else {
            return Self::Transient(SharedTokenBuffer::new([first, second]));
        };
        let (lower, _) = tokens.size_hint();
        let mut shared = Vec::with_capacity(lower.saturating_add(3));
        shared.extend([first, second, third]);
        shared.extend(tokens);
        Self::Transient(SharedTokenBuffer::new(shared))
    }

    /// Selects shared structural storage for rooted transient positions.
    /// Rooted singletons deliberately do not use the raw inline form.
    pub(crate) fn transient_rooted(
        tokens: impl IntoIterator<Item = RootedTracedTokenWord>,
    ) -> Self {
        let mut tokens = tokens.into_iter();
        let Some(first) = tokens.next() else {
            return Self::Transient(SharedTokenBuffer::default());
        };
        let Some(second) = tokens.next() else {
            return Self::InlineTransient(first);
        };
        Self::Transient(SharedTokenBuffer::new_rooted(
            [first, second].into_iter().chain(tokens),
        ))
    }

    /// Selects inline storage for one backed-up command and shared storage for
    /// empty or multi-token payloads.
    pub(crate) fn backed_up(tokens: impl IntoIterator<Item = BackedUpToken>) -> Self {
        let mut tokens = tokens.into_iter();
        let Some(first) = tokens.next() else {
            return Self::BackedUp(SharedBackedUpBuffer::default());
        };
        let Some(second) = tokens.next() else {
            return Self::InlineBackedUp(RootedBackedUpToken::unowned(first));
        };
        let (lower, _) = tokens.size_hint();
        let mut shared = Vec::with_capacity(lower.saturating_add(2));
        shared.extend([first, second]);
        shared.extend(tokens);
        Self::BackedUp(SharedBackedUpBuffer::new(shared))
    }

    pub(crate) fn backed_up_rooted(tokens: impl IntoIterator<Item = RootedBackedUpToken>) -> Self {
        let mut tokens = tokens.into_iter();
        let Some(first) = tokens.next() else {
            return Self::BackedUp(SharedBackedUpBuffer::default());
        };
        let Some(second) = tokens.next() else {
            return Self::InlineBackedUp(first);
        };
        Self::BackedUp(SharedBackedUpBuffer::new_rooted(
            [first, second].into_iter().chain(tokens),
        ))
    }

    pub(crate) fn transient_words(&self) -> Option<&[TracedTokenWord]> {
        match self {
            Self::Packed(chunk) if !chunk.backed_up => Some(chunk.words.words()),
            Self::Transient(words) => Some(words.words()),
            Self::InlineTransient(_) => None,
            _ => None,
        }
    }

    pub(crate) fn transient_len(&self) -> Option<usize> {
        match self {
            Self::Packed(chunk) if !chunk.backed_up => Some(chunk.len()),
            Self::Transient(words) => Some(words.len()),
            Self::InlineTransient(_) => Some(1),
            _ => None,
        }
    }

    pub(crate) fn backed_up_words(&self) -> Option<&[BackedUpToken]> {
        match self {
            Self::Packed(_) => None,
            Self::BackedUp(words) => Some(words.words()),
            Self::InlineBackedUp(_) => None,
            _ => None,
        }
    }

    pub(crate) fn backed_up_len(&self) -> Option<usize> {
        match self {
            Self::Packed(chunk) if chunk.backed_up => Some(chunk.len()),
            Self::BackedUp(words) => Some(words.words().len()),
            Self::InlineBackedUp(_) => Some(1),
            _ => None,
        }
    }

    pub(crate) fn backed_up_get(&self, index: usize) -> Option<BackedUpToken> {
        match self {
            Self::Packed(chunk) => chunk.backed_up_token(index),
            Self::BackedUp(words) => words.get(index),
            Self::InlineBackedUp(word) => (index == 0).then(|| word.token()),
            _ => None,
        }
    }

    pub(crate) fn is_backed_up(&self) -> bool {
        matches!(self, Self::Packed(chunk) if chunk.backed_up)
            || matches!(self, Self::BackedUp(_) | Self::InlineBackedUp(_))
    }

    /// Prepends e-TeX aftergroup tokens, promoting inline storage when the
    /// resulting backed-up level contains multiple commands.
    pub(crate) fn prepend_backed_up(
        &mut self,
        prefix: impl IntoIterator<Item = RootedBackedUpToken>,
    ) -> Option<()> {
        let mut prefix = prefix.into_iter().collect::<Vec<_>>();
        match self {
            Self::Packed(chunk) if chunk.backed_up => {
                let mut words = RootedTracedTokenBuffer::default();
                let mut provenance = SmallVec::new();
                for token in prefix.drain(..) {
                    let (token, root) = token.into_parts();
                    words.push(RootedTracedTokenWord::from_word(token.spelling, root));
                    provenance.push(token.source_provenance);
                }
                words.append_buffer(std::mem::take(&mut chunk.words));
                provenance.extend(chunk.source_provenance.drain(..));
                chunk.words = words;
                chunk.source_provenance = provenance;
            }
            Self::Packed(_) => return None,
            Self::BackedUp(words) => words.prepend(prefix),
            Self::InlineBackedUp(word) => {
                prefix.push(word.clone());
                *self = Self::backed_up_rooted(prefix);
            }
            _ => return None,
        }
        Some(())
    }

    pub(crate) fn rehome_backed_up_source(
        &mut self,
        source: tex_state::SourceId,
        byte_delta: i64,
    ) -> Option<()> {
        match self {
            Self::Packed(chunk) if chunk.backed_up => {
                for provenance in chunk.source_provenance.iter_mut().flatten() {
                    provenance.rehome(source, byte_delta)?;
                }
                Some(())
            }
            Self::BackedUp(words) => words.rehome_source(source, byte_delta),
            Self::InlineBackedUp(word) => {
                if let Some(provenance) = &mut word.token.source_provenance {
                    provenance.rehome(source, byte_delta)?;
                }
                Some(())
            }
            _ => None,
        }
    }

    pub(crate) fn adopt_matching_origins(&mut self, live: &Self) -> Option<()> {
        if let (Self::Packed(recorded), Self::Packed(live)) = (&*self, live) {
            if recorded.words.words().len() != live.words.words().len()
                || recorded
                    .words
                    .words()
                    .iter()
                    .zip(live.words.words())
                    .any(|(recorded, live)| recorded.token() != live.token())
                || recorded.source_provenance != live.source_provenance
                || recorded.backed_up != live.backed_up
            {
                return None;
            }
            *self = Self::Packed(live.clone());
            return Some(());
        }
        if let (Self::InlineTransient(recorded), Self::InlineTransient(live)) = (&*self, live) {
            if recorded.word().token() != live.word().token() {
                return None;
            }
            *self = Self::InlineTransient(live.clone());
            return Some(());
        }
        if let (Self::InlineBackedUp(recorded), Self::InlineBackedUp(live)) = (&*self, live) {
            if recorded.token().spelling.token() != live.token().spelling.token()
                || recorded.token().source_provenance != live.token().source_provenance
            {
                return None;
            }
            *self = Self::InlineBackedUp(live.clone());
            return Some(());
        }
        if let (Some(recorded), Some(live_words)) = (self.transient_words(), live.transient_words())
        {
            if recorded.len() != live_words.len()
                || recorded
                    .iter()
                    .zip(live_words)
                    .any(|(recorded, live)| recorded.token() != live.token())
            {
                return None;
            }
            *self = live.clone();
            return Some(());
        }
        let (Some(recorded), Some(live_words)) = (self.backed_up_words(), live.backed_up_words())
        else {
            return None;
        };
        if recorded.len() != live_words.len()
            || recorded.iter().zip(live_words).any(|(recorded, live)| {
                recorded.spelling.token() != live.spelling.token()
                    || recorded.source_provenance != live.source_provenance
            })
        {
            return None;
        }
        *self = live.clone();
        Some(())
    }
}

/// Shared ownership of a contiguous traced-token allocation.
///
/// Cloning a cursor or snapshot retains the allocation rather than copying its
/// tokens. A macro activation and its parameter cursors may share this value.
#[derive(Debug, Eq, Hash, PartialEq)]
struct SharedTokenBufferValue {
    /// Inline-small scanner storage retained directly by the shared owner.
    /// Its roots remain sorted and distinct; direct, fallback, and unknown
    /// positions are represented by their packed words alone.
    buffer: tex_state::token::RootedTracedTokenBuffer,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SharedTokenBuffer(Arc<SharedTokenBufferValue>);

#[cfg(test)]
pub(crate) struct SharedTokenBufferWeak(std::sync::Weak<SharedTokenBufferValue>);

#[cfg(test)]
impl SharedTokenBufferWeak {
    pub(crate) fn is_live(&self) -> bool {
        self.0.upgrade().is_some()
    }
}

impl Default for SharedTokenBuffer {
    fn default() -> Self {
        Self::new([])
    }
}

impl SharedTokenBuffer {
    #[cfg(test)]
    pub(crate) fn downgrade(&self) -> SharedTokenBufferWeak {
        SharedTokenBufferWeak(Arc::downgrade(&self.0))
    }

    /// Builds a rootless buffer. Arena-backed words are rejected because a
    /// raw id cannot confer ownership on its provenance atom.
    pub(crate) fn new(tokens: impl AsRef<[TracedTokenWord]>) -> Self {
        Self::new_rooted(
            tokens
                .as_ref()
                .iter()
                .copied()
                .map(RootedTracedTokenWord::unowned),
        )
    }

    pub(crate) fn new_rooted(tokens: impl IntoIterator<Item = RootedTracedTokenWord>) -> Self {
        Self(Arc::new(SharedTokenBufferValue {
            buffer: tex_state::token::RootedTracedTokenBuffer::new(tokens),
        }))
    }

    /// Freezes a scanner-owned buffer by transferring its paired word and
    /// provenance-root allocations. Macro matching is the hot caller: it has
    /// already established the same sorted-root invariant while collecting
    /// arguments, so replay need not iterate, clone roots, and rebuild words.
    pub(crate) fn from_rooted_buffer(buffer: tex_state::token::RootedTracedTokenBuffer) -> Self {
        Self(Arc::new(SharedTokenBufferValue { buffer }))
    }

    pub(crate) fn len(&self) -> usize {
        self.0.buffer.len()
    }

    pub(crate) fn get(&self, index: usize) -> Option<TracedTokenWord> {
        self.0.buffer.get(index)
    }

    pub(crate) fn get_rooted(&self, index: usize) -> Option<RootedTracedTokenWord> {
        self.0.buffer.get_rooted(index)
    }

    pub(crate) fn words(&self) -> &[TracedTokenWord] {
        self.0.buffer.words()
    }

    pub(crate) fn rooted_words(&self) -> impl ExactSizeIterator<Item = RootedTracedTokenWord> + '_ {
        (0..self.len()).map(|index| {
            self.get_rooted(index)
                .expect("index from the exact shared-buffer length")
        })
    }

    pub(crate) fn adopt_matching_origins(&mut self, live: &Self) -> Option<()> {
        if self.0.buffer.len() != live.0.buffer.len()
            || self
                .0
                .buffer
                .words()
                .iter()
                .zip(live.0.buffer.words().iter())
                .any(|(recorded, live)| recorded.token() != live.token())
        {
            return None;
        }
        self.0 = Arc::clone(&live.0);
        Some(())
    }
}

/// Shared ownership of commands restored by `back_input` or scanner replay.
///
/// The range is delivery metadata, separate from the token's semantic and
/// opaque-origin identity, so reusing it cannot affect fixture identities.
#[derive(Debug, Eq, Hash, PartialEq)]
struct SharedBackedUpBufferValue {
    tokens: Box<[BackedUpToken]>,
    roots: Box<[OriginRef]>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SharedBackedUpBuffer(Arc<SharedBackedUpBufferValue>);

impl Default for SharedBackedUpBuffer {
    fn default() -> Self {
        Self::new([])
    }
}

impl SharedBackedUpBuffer {
    pub(crate) fn new(tokens: impl AsRef<[BackedUpToken]>) -> Self {
        Self::new_rooted(
            tokens
                .as_ref()
                .iter()
                .copied()
                .map(RootedBackedUpToken::unowned),
        )
    }

    pub(crate) fn new_rooted(tokens: impl IntoIterator<Item = RootedBackedUpToken>) -> Self {
        let mut words = Vec::new();
        let mut roots = Vec::new();
        for rooted in tokens {
            let (token, root) = rooted.into_parts();
            words.push(token);
            if root.record().is_some() {
                match roots.binary_search_by_key(&root.id(), OriginRef::id) {
                    Ok(_) => {}
                    Err(index) => roots.insert(index, root),
                }
            }
        }
        Self(Arc::new(SharedBackedUpBufferValue {
            tokens: words.into_boxed_slice(),
            roots: roots.into_boxed_slice(),
        }))
    }

    pub(crate) fn get(&self, index: usize) -> Option<BackedUpToken> {
        self.0.tokens.get(index).copied()
    }

    pub(crate) fn get_rooted(&self, index: usize) -> Option<RootedBackedUpToken> {
        let token = self.get(index)?;
        let root = self
            .0
            .roots
            .binary_search_by_key(&token.spelling.origin(), OriginRef::id)
            .map_or_else(
                |_| OriginRef::direct(token.spelling.origin()),
                |index| self.0.roots[index].clone(),
            );
        Some(RootedBackedUpToken { token, root })
    }

    pub(crate) fn words(&self) -> &[BackedUpToken] {
        &self.0.tokens
    }

    /// Prepends tokens to e-TeX's active optimized `backed_up` list.
    pub(crate) fn prepend(&mut self, prefix: impl IntoIterator<Item = RootedBackedUpToken>) {
        let mut tokens = prefix.into_iter().collect::<Vec<_>>();
        tokens.extend((0..self.0.tokens.len()).map(|index| {
            self.get_rooted(index)
                .expect("index from exact backed-up-buffer length")
        }));
        *self = Self::new_rooted(tokens);
    }

    pub(crate) fn rehome_source(
        &mut self,
        source: tex_state::SourceId,
        byte_delta: i64,
    ) -> Option<()> {
        let mut tokens = self.0.tokens.to_vec();
        for token in &mut tokens {
            if let Some(provenance) = &mut token.source_provenance {
                provenance.rehome(source, byte_delta)?;
            }
        }
        let rooted = tokens.into_iter().enumerate().map(|(index, token)| {
            let old = self
                .get_rooted(index)
                .expect("rehome preserves backed-up-buffer length");
            RootedBackedUpToken::new(token, old.origin_ref().clone())
        });
        *self = Self::new_rooted(rooted);
        Some(())
    }

    pub(crate) fn adopt_matching_origins(&mut self, live: &Self) -> Option<()> {
        if self.0.tokens.len() != live.0.tokens.len() {
            return None;
        }
        let mut tokens = self.0.tokens.to_vec();
        for (recorded, live) in tokens.iter_mut().zip(live.0.tokens.iter()) {
            if recorded.spelling.token() != live.spelling.token()
                || recorded.source_provenance != live.source_provenance
            {
                return None;
            }
            recorded.spelling = tex_state::token::TracedTokenWord::pack(
                live.spelling.token()?,
                live.spelling.origin(),
            );
        }
        let rooted = tokens.into_iter().enumerate().map(|(index, token)| {
            RootedBackedUpToken::new(
                token,
                live.get_rooted(index)
                    .expect("matching buffers have equal lengths")
                    .origin_ref()
                    .clone(),
            )
        });
        *self = Self::new_rooted(rooted);
        Some(())
    }
}

/// One restored command plus the source range committed at its first delivery.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct BackedUpToken {
    pub(crate) spelling: TracedTokenWord,
    pub(crate) source_provenance: Option<SourceProvenance>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RootedBackedUpToken {
    token: BackedUpToken,
    root: OriginRef,
}

impl RootedBackedUpToken {
    pub(crate) fn new(token: BackedUpToken, root: OriginRef) -> Self {
        assert_eq!(token.spelling.origin(), root.id());
        Self { token, root }
    }

    pub(crate) fn unowned(token: BackedUpToken) -> Self {
        let rooted = RootedTracedTokenWord::unowned(token.spelling);
        Self {
            token,
            root: rooted.into_parts().1,
        }
    }

    pub(crate) const fn token(&self) -> BackedUpToken {
        self.token
    }

    pub(crate) fn origin_ref(&self) -> &OriginRef {
        &self.root
    }

    pub(crate) fn into_parts(self) -> (BackedUpToken, OriginRef) {
        (self.token, self.root)
    }
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
    /// e-TeX §22.307's `every_eof_text`.
    EveryEof,
    /// §307 `mark_text=14`.
    Mark,
    /// §307 `write_text=15`.
    Write,
    Discretionary,
}

/// Canonical explanations for a materialized transient insertion that is not
/// TeX82 §307's `inserted` token type (which is [`ReplayTrace::Inserted`]).
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TransientReplayReason {
    ExpandedTokenList,
}

#[cfg(test)]
mod tests;
