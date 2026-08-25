//! Dense source and token-list input-level ownership.
#![allow(dead_code)] // consumed by the ordered raw-delivery implementation issues

use core::marker::PhantomData;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use tex_state::DefinitionId;
use tex_state::packed_input::{InputFrameFlags, InputFrameKind};
use tex_state::token::{OriginId, Token, TracedTokenWord};

use crate::attempt::AttemptTokenListId;
use crate::macro_call::MacroActivationId;

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
#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) enum InputLevel<G> {
    Source(SourceLevel<G>),
    Tokens(TokenCursor<G>),
}

/// One registered-source level and its exact delivery identity.
#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct SourceLevel<G> {
    pub(crate) frame: PackedInputFrame,
    pub(crate) cursor: Box<SourceCursor>,
    /// tex.web §303's `name` classification for this level. A token-list
    /// level has no counterpart: §307 reuses `name` there as the eqtb address
    /// of the macro being expanded, which is why this lives on `SourceLevel`
    /// and not on [`InputLevel`].
    pub(crate) name_class: SourceNameClass,
    pub(crate) retirement: SourceRetirement,
    /// e-TeX §24.362's once-only token list, pushed above this source when
    /// natural EOF is first observed and before `end_file_reading`.
    pub(crate) every_eof: Option<tex_state::TokenListId<G>>,
    /// e-TeX 2.6 [23.328]'s `grp_stack[in_open]`/`if_stack[in_open]`: the
    /// live group and conditional boundary ancestry recorded when this
    /// level's `begin_file_reading` ran, compared against the current stacks
    /// at `end_file_reading` to drive `\tracingnesting`'s `file_warning`.
    /// `None` until the opener records it (this crate has no `Universe`
    /// access at construction time; see `CommandState::record_source_open_depths`).
    pub(crate) open_depths: Option<Box<SourceOpenDepths>>,
}

impl<G> SourceLevel<G> {
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
#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct TokenCursor<G> {
    pub(crate) payload: TokenPayload<G>,
    pub(crate) behavior: TokenBehavior,
    pub(crate) retirement: RetirementBehavior,
    pub(crate) trace: ReplayTrace,
    pub(crate) frame: PackedInputFrame,
}

impl<G> TokenCursor<G> {
    pub(crate) fn identity(&self) -> InputLevelId {
        InputLevelId(self.frame.identity())
    }

    pub(crate) fn position(&self) -> usize {
        self.frame.position() as usize
    }
}

/// Storage owning the tokens delivered by a token-list level.
#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) enum TokenPayload<G> {
    /// Compact coordinate into the generation-owned replay lane.
    Replay {
        replay: ReplayPayloadId<G>,
        len: u32,
    },
    /// Replacement replay borrowed from one command-admitted macro chunk.
    MacroReplacement {
        definition: DefinitionId<G>,
        len: u32,
    },
    /// One literal macro-argument replay range in generation-owned scratch.
    MacroArgument {
        replay: crate::execution_scratch::MacroReplayCursor<G>,
        len: u32,
    },
    /// One generation-durable token list, replayed through its stable chunk
    /// cursor without materializing an input-owned word buffer.
    DurableList {
        cursor: tex_state::TokenListCursor<G>,
        len: u32,
    },
    /// One attempt-local token list, replayed literally by range.
    AttemptList { list: AttemptTokenListId, len: u32 },
}

impl<G> Clone for TokenPayload<G> {
    fn clone(&self) -> Self {
        match self {
            Self::Replay { replay, len } => Self::Replay {
                replay: *replay,
                len: *len,
            },
            Self::MacroReplacement { definition, len } => Self::MacroReplacement {
                definition: definition.clone(),
                len: *len,
            },
            Self::MacroArgument { replay, len } => Self::MacroArgument {
                replay: *replay,
                len: *len,
            },
            Self::DurableList { cursor, len } => Self::DurableList {
                cursor: cursor.clone(),
                len: *len,
            },
            Self::AttemptList { list, len } => Self::AttemptList {
                list: *list,
                len: *len,
            },
        }
    }
}

impl<G> Clone for TokenCursor<G> {
    fn clone(&self) -> Self {
        Self {
            payload: self.payload.clone(),
            behavior: self.behavior.clone(),
            retirement: self.retirement,
            trace: self.trace.clone(),
            frame: self.frame,
        }
    }
}

impl<G> Clone for SourceLevel<G> {
    fn clone(&self) -> Self {
        Self {
            frame: self.frame,
            cursor: self.cursor.clone(),
            name_class: self.name_class,
            retirement: self.retirement,
            every_eof: self.every_eof.clone(),
            open_depths: self.open_depths.clone(),
        }
    }
}

impl<G> Clone for InputLevel<G> {
    fn clone(&self) -> Self {
        match self {
            Self::Source(source) => Self::Source(source.clone()),
            Self::Tokens(tokens) => Self::Tokens(tokens.clone()),
        }
    }
}

/// TeX82 one-word allocator ownership carried independently of Umber's
/// uniform packed host representation.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) enum PackedTokenOwnership {
    /// Replaying an immutable token list adds only TeX's list-stack reference.
    #[default]
    Stored,
    /// Inserted/generated scanner words own freshly allocated one-word cells.
    Transient,
    /// `back_input` owns freshly allocated cells plus source replay metadata.
    BackedUp,
}

const REPLAY_SEGMENT_ITEMS: usize = 256;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ReplayLaneMark {
    segments: u32,
    tail_used: u16,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ReplayLaneCursor {
    segment: u32,
    offset: u16,
}

#[derive(Debug, Eq, Hash, PartialEq)]
struct ReplaySegment<T> {
    values: Vec<T>,
}

impl<T> ReplaySegment<T> {
    fn new() -> Result<Self, crate::execution_scratch::ScratchError> {
        let mut values = Vec::new();
        values
            .try_reserve_exact(REPLAY_SEGMENT_ITEMS)
            .map_err(|_| crate::execution_scratch::ScratchError::AllocationFailed)?;
        Ok(Self { values })
    }
}

#[derive(Debug, Eq, Hash, PartialEq)]
struct ActiveReplaySegment<T> {
    storage: Arc<ReplaySegment<T>>,
    used: u16,
}

impl<T> Clone for ActiveReplaySegment<T> {
    fn clone(&self) -> Self {
        Self {
            storage: Arc::clone(&self.storage),
            used: self.used,
        }
    }
}

#[derive(Debug)]
struct SegmentedReplayLane<T> {
    active: Vec<ActiveReplaySegment<T>>,
    spare: Vec<Arc<ReplaySegment<T>>>,
}

impl<T> Default for SegmentedReplayLane<T> {
    fn default() -> Self {
        Self {
            active: Vec::new(),
            spare: Vec::new(),
        }
    }
}

impl<T> Clone for SegmentedReplayLane<T> {
    fn clone(&self) -> Self {
        Self {
            active: self.active.clone(),
            spare: Vec::new(),
        }
    }
}

impl<T: Eq> PartialEq for SegmentedReplayLane<T> {
    fn eq(&self, other: &Self) -> bool {
        self.active.len() == other.active.len()
            && self.active.iter().zip(&other.active).all(|(left, right)| {
                left.used == right.used
                    && left.storage.values[..usize::from(left.used)]
                        == right.storage.values[..usize::from(right.used)]
            })
    }
}

impl<T: Eq> Eq for SegmentedReplayLane<T> {}

impl<T: Hash> Hash for SegmentedReplayLane<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.active.len().hash(state);
        for segment in &self.active {
            segment.used.hash(state);
            segment.storage.values[..usize::from(segment.used)].hash(state);
        }
    }
}

impl<T> SegmentedReplayLane<T> {
    fn mark(&self) -> ReplayLaneMark {
        ReplayLaneMark {
            segments: self.active.len() as u32,
            tail_used: self.active.last().map_or(0, |segment| segment.used),
        }
    }

    fn push(
        &mut self,
        value: T,
    ) -> Result<ReplayLaneCursor, crate::execution_scratch::ScratchError> {
        let needs_segment = self.active.last().is_none_or(|segment| {
            usize::from(segment.used) == REPLAY_SEGMENT_ITEMS
                || Arc::strong_count(&segment.storage) != 1
        });
        if needs_segment {
            let high_water = self
                .active
                .len()
                .checked_add(self.spare.len())
                .and_then(|len| len.checked_add(1))
                .ok_or(crate::execution_scratch::ScratchError::CapacityOverflow)?;
            self.active
                .try_reserve(high_water - self.active.len())
                .map_err(|_| crate::execution_scratch::ScratchError::AllocationFailed)?;
            // Retirement may transfer every active segment here at once. Grow
            // this header vector when the segment is first admitted so the
            // hot LIFO pop path remains allocation-free at high water.
            self.spare
                .try_reserve(high_water - self.spare.len())
                .map_err(|_| crate::execution_scratch::ScratchError::AllocationFailed)?;
            let storage = match self.spare.pop() {
                Some(storage) if Arc::strong_count(&storage) == 1 => storage,
                _ => Arc::new(ReplaySegment::new()?),
            };
            self.active.push(ActiveReplaySegment { storage, used: 0 });
        }
        let segment_index = self.active.len() - 1;
        let segment = &mut self.active[segment_index];
        let offset = segment.used;
        let storage = Arc::get_mut(&mut segment.storage)
            .ok_or(crate::execution_scratch::ScratchError::InvalidCoordinate)?;
        if usize::from(offset) == storage.values.len() {
            storage.values.push(value);
        } else {
            storage.values[usize::from(offset)] = value;
        }
        segment.used += 1;
        Ok(ReplayLaneCursor {
            segment: segment_index as u32,
            offset,
        })
    }

    fn get(&self, start: ReplayLaneCursor, mut index: usize) -> Option<&T> {
        let mut segment_index = start.segment as usize;
        let mut offset = usize::from(start.offset);
        loop {
            let segment = self.active.get(segment_index)?;
            let available = usize::from(segment.used).checked_sub(offset)?;
            if index < available {
                return segment.storage.values.get(offset + index);
            }
            index -= available;
            segment_index += 1;
            offset = 0;
        }
    }

    fn restore(
        &mut self,
        mark: ReplayLaneMark,
    ) -> Result<(), crate::execution_scratch::ScratchError> {
        let segments = mark.segments as usize;
        if segments > self.active.len()
            || (segments == 0 && mark.tail_used != 0)
            || (segments > 0
                && usize::from(mark.tail_used) > self.active[segments - 1].storage.values.len())
        {
            return Err(crate::execution_scratch::ScratchError::InvalidCoordinate);
        }
        while self.active.len() > segments {
            let segment = self.active.pop().expect("active replay segment");
            self.spare.push(segment.storage);
        }
        if let Some(tail) = self.active.last_mut() {
            tail.used = mark.tail_used;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct ReplayPayloadId<G> {
    entry: u32,
    _generation: PhantomData<fn(&G) -> &G>,
}

impl<G> Copy for ReplayPayloadId<G> {}
impl<G> Clone for ReplayPayloadId<G> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<G> PartialEq for ReplayPayloadId<G> {
    fn eq(&self, other: &Self) -> bool {
        self.entry == other.entry
    }
}
impl<G> Eq for ReplayPayloadId<G> {}
impl<G> Hash for ReplayPayloadId<G> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.entry.hash(state);
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ReplaySpan {
    start: ReplayLaneCursor,
    len: u32,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ReplayEntry {
    word_mark: ReplayLaneMark,
    provenance_mark: ReplayLaneMark,
    body_words: ReplaySpan,
    body_provenance: Option<ReplaySpan>,
    prefix_words: Option<ReplaySpan>,
    prefix_provenance: Option<ReplaySpan>,
    ownership: PackedTokenOwnership,
}

impl ReplayEntry {
    fn len(&self) -> usize {
        self.prefix_words.map_or(0, |span| span.len as usize) + self.body_words.len as usize
    }
}

#[derive(Debug)]
pub(crate) struct ReplayLane<G> {
    entries: Vec<ReplayEntry>,
    words: SegmentedReplayLane<TracedTokenWord>,
    provenance: SegmentedReplayLane<Option<SourceProvenance>>,
    _generation: PhantomData<fn(&G) -> &G>,
}

impl<G> Default for ReplayLane<G> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            words: SegmentedReplayLane::default(),
            provenance: SegmentedReplayLane::default(),
            _generation: PhantomData,
        }
    }
}

impl<G> Clone for ReplayLane<G> {
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
            words: self.words.clone(),
            provenance: self.provenance.clone(),
            _generation: PhantomData,
        }
    }
}

impl<G> PartialEq for ReplayLane<G> {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries
            && self.words == other.words
            && self.provenance == other.provenance
    }
}
impl<G> Eq for ReplayLane<G> {}
impl<G> Hash for ReplayLane<G> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.entries.hash(state);
        self.words.hash(state);
        self.provenance.hash(state);
    }
}

impl<G> ReplayLane<G> {
    fn push_words(
        &mut self,
        tokens: impl IntoIterator<Item = BackedUpToken>,
        ownership: PackedTokenOwnership,
    ) -> Result<TokenPayload<G>, crate::execution_scratch::ScratchError> {
        let word_mark = self.words.mark();
        let provenance_mark = self.provenance.mark();
        let mut start = None;
        let mut provenance_start = None;
        let mut len = 0_u32;
        for token in tokens {
            start.get_or_insert(self.words.push(token.spelling)?);
            if ownership == PackedTokenOwnership::BackedUp {
                provenance_start.get_or_insert(self.provenance.push(token.source_provenance)?);
            }
            len = len
                .checked_add(1)
                .ok_or(crate::execution_scratch::ScratchError::CapacityOverflow)?;
        }
        let empty = ReplayLaneCursor {
            segment: word_mark.segments.saturating_sub(1),
            offset: word_mark.tail_used,
        };
        let entry = u32::try_from(self.entries.len())
            .map_err(|_| crate::execution_scratch::ScratchError::CapacityOverflow)?;
        self.entries
            .try_reserve(1)
            .map_err(|_| crate::execution_scratch::ScratchError::AllocationFailed)?;
        self.entries.push(ReplayEntry {
            word_mark,
            provenance_mark,
            body_words: ReplaySpan {
                start: start.unwrap_or(empty),
                len,
            },
            body_provenance: provenance_start.map(|start| ReplaySpan { start, len }),
            prefix_words: None,
            prefix_provenance: None,
            ownership,
        });
        Ok(TokenPayload::Replay {
            replay: ReplayPayloadId {
                entry,
                _generation: PhantomData,
            },
            len,
        })
    }

    pub(crate) fn get(
        &self,
        replay: ReplayPayloadId<G>,
        index: usize,
    ) -> Option<(TracedTokenWord, Option<SourceProvenance>)> {
        let entry = self.entries.get(replay.entry as usize)?;
        if index >= entry.len() {
            return None;
        }
        let prefix_len = entry.prefix_words.map_or(0, |span| span.len as usize);
        let (words, provenance, local) = if index < prefix_len {
            (entry.prefix_words?, entry.prefix_provenance, index)
        } else {
            (entry.body_words, entry.body_provenance, index - prefix_len)
        };
        Some((
            *self.words.get(words.start, local)?,
            provenance
                .and_then(|span| self.provenance.get(span.start, local))
                .copied()
                .flatten(),
        ))
    }

    pub(crate) fn ownership(&self, replay: ReplayPayloadId<G>) -> Option<PackedTokenOwnership> {
        self.entries
            .get(replay.entry as usize)
            .map(|entry| entry.ownership)
    }

    pub(crate) fn prepend_backed_up(
        &mut self,
        replay: ReplayPayloadId<G>,
        prefix: impl IntoIterator<Item = BackedUpToken>,
    ) -> Result<u32, crate::execution_scratch::ScratchError> {
        let expected = self
            .entries
            .len()
            .checked_sub(1)
            .ok_or(crate::execution_scratch::ScratchError::InvalidCoordinate)?;
        if replay.entry as usize != expected
            || self.entries[expected].ownership != PackedTokenOwnership::BackedUp
            || self.entries[expected].prefix_words.is_some()
        {
            return Err(crate::execution_scratch::ScratchError::InvalidCoordinate);
        }
        let mut word_start = None;
        let mut provenance_start = None;
        let mut len = 0_u32;
        for token in prefix {
            word_start.get_or_insert(self.words.push(token.spelling)?);
            provenance_start.get_or_insert(self.provenance.push(token.source_provenance)?);
            len = len
                .checked_add(1)
                .ok_or(crate::execution_scratch::ScratchError::CapacityOverflow)?;
        }
        if len != 0 {
            self.entries[expected].prefix_words = word_start.map(|start| ReplaySpan { start, len });
            self.entries[expected].prefix_provenance =
                provenance_start.map(|start| ReplaySpan { start, len });
        }
        Ok(len)
    }

    pub(crate) fn release(
        &mut self,
        replay: ReplayPayloadId<G>,
    ) -> Result<(), crate::execution_scratch::ScratchError> {
        if replay.entry as usize + 1 != self.entries.len() {
            return Err(crate::execution_scratch::ScratchError::InvalidCoordinate);
        }
        let entry = self.entries.pop().expect("validated replay entry");
        self.words.restore(entry.word_mark)?;
        self.provenance.restore(entry.provenance_mark)
    }
}

pub(crate) trait TokenPayloadSource<G> {
    fn admit(
        self,
        lane: &mut ReplayLane<G>,
    ) -> Result<TokenPayload<G>, crate::execution_scratch::ScratchError>;
}

impl<G> TokenPayloadSource<G> for TokenPayload<G> {
    fn admit(
        self,
        _lane: &mut ReplayLane<G>,
    ) -> Result<TokenPayload<G>, crate::execution_scratch::ScratchError> {
        Ok(self)
    }
}

pub(crate) struct TracedReplaySeed<I> {
    tokens: I,
    ownership: PackedTokenOwnership,
}
impl<G, I: Iterator<Item = TracedTokenWord>> TokenPayloadSource<G> for TracedReplaySeed<I> {
    fn admit(
        self,
        lane: &mut ReplayLane<G>,
    ) -> Result<TokenPayload<G>, crate::execution_scratch::ScratchError> {
        lane.push_words(
            self.tokens.map(|spelling| BackedUpToken {
                spelling,
                source_provenance: None,
            }),
            self.ownership,
        )
    }
}

pub(crate) struct SemanticReplaySeed<I> {
    tokens: I,
    origin: OriginId,
    ownership: PackedTokenOwnership,
}
impl<G, I: Iterator<Item = Token>> TokenPayloadSource<G> for SemanticReplaySeed<I> {
    fn admit(
        self,
        lane: &mut ReplayLane<G>,
    ) -> Result<TokenPayload<G>, crate::execution_scratch::ScratchError> {
        lane.push_words(
            self.tokens.map(|token| BackedUpToken {
                spelling: TracedTokenWord::pack(token, self.origin),
                source_provenance: None,
            }),
            self.ownership,
        )
    }
}

pub(crate) struct BackedReplaySeed<I> {
    tokens: I,
}
impl<G, I: Iterator<Item = BackedUpToken>> TokenPayloadSource<G> for BackedReplaySeed<I> {
    fn admit(
        self,
        lane: &mut ReplayLane<G>,
    ) -> Result<TokenPayload<G>, crate::execution_scratch::ScratchError> {
        lane.push_words(self.tokens, PackedTokenOwnership::BackedUp)
    }
}

pub(crate) struct StoredReplaySeed<'a, I> {
    tokens: &'a [Token],
    origins: I,
}
impl<G, I: Iterator<Item = OriginId>> TokenPayloadSource<G> for StoredReplaySeed<'_, I> {
    fn admit(
        mut self,
        lane: &mut ReplayLane<G>,
    ) -> Result<TokenPayload<G>, crate::execution_scratch::ScratchError> {
        lane.push_words(
            self.tokens.iter().copied().map(|token| BackedUpToken {
                spelling: TracedTokenWord::pack(
                    token,
                    self.origins.next().unwrap_or(OriginId::UNKNOWN),
                ),
                source_provenance: None,
            }),
            PackedTokenOwnership::Stored,
        )
    }
}

impl TokenPayload<()> {
    pub(crate) fn stored(
        tokens: &[Token],
        origins: impl IntoIterator<Item = OriginId>,
    ) -> StoredReplaySeed<'_, impl Iterator<Item = OriginId>> {
        StoredReplaySeed {
            tokens,
            origins: origins.into_iter(),
        }
    }

    pub(crate) fn stored_semantic(
        words: &[tex_state::token::TokenWord],
    ) -> SemanticReplaySeed<impl Iterator<Item = Token> + '_> {
        SemanticReplaySeed {
            tokens: words.iter().copied().map(|word| word.semantic_token()),
            origin: OriginId::UNKNOWN,
            ownership: PackedTokenOwnership::Stored,
        }
    }

    /// Packs one bounded insertion or scanner result directly into its sole
    /// live chunk representation.
    pub(crate) fn transient(
        tokens: impl IntoIterator<Item = TracedTokenWord>,
    ) -> TracedReplaySeed<impl Iterator<Item = TracedTokenWord>> {
        TracedReplaySeed {
            tokens: tokens.into_iter(),
            ownership: PackedTokenOwnership::Transient,
        }
    }

    /// Packs generated tokens that all carry one structural origin without
    /// forming a temporary strong owner for every position.
    pub(crate) fn transient_with_shared_origin(
        tokens: impl IntoIterator<Item = Token>,
        origin: OriginId,
    ) -> SemanticReplaySeed<impl Iterator<Item = Token>> {
        SemanticReplaySeed {
            tokens: tokens.into_iter(),
            origin,
            ownership: PackedTokenOwnership::Transient,
        }
    }

    /// Packs commands restored by `back_input` into the canonical chunk.
    pub(crate) fn backed_up(
        tokens: impl IntoIterator<Item = BackedUpToken>,
    ) -> BackedReplaySeed<impl Iterator<Item = BackedUpToken>> {
        BackedReplaySeed {
            tokens: tokens.into_iter(),
        }
    }
}

impl<G> TokenPayload<G> {
    pub(crate) fn durable(words: tex_state::TokenListView<G>) -> Self {
        Self::DurableList {
            cursor: words.cursor(),
            len: u32::try_from(words.len()).expect("durable token-list length exceeds u32"),
        }
    }

    pub(crate) fn frame_len(&self) -> usize {
        match self {
            Self::Replay { len, .. } => *len as usize,
            Self::MacroReplacement { len, .. } => *len as usize,
            Self::MacroArgument { len, .. } => *len as usize,
            Self::DurableList { len, .. } => *len as usize,
            Self::AttemptList { len, .. } => *len as usize,
        }
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
