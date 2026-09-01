//! Dense source and token-list input-level ownership.
#![allow(dead_code)] // consumed by the ordered raw-delivery implementation issues

use core::marker::PhantomData;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use tex_state::packed_input::{InputFrameFlags, InputFrameKind};
use tex_state::token::{OriginId, Token, TokenWord, TracedTokenWord};

use super::{
    lines::SourceLexCursor,
    source::{RegisteredSource, SourceCursor, SourceCursorExecutionState, SourceNameClass},
};
use crate::attempt::AttemptTokenListId;

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
    ReplayTokens(ReplayTokenCursor<G>),
    DurableTokens(DurableTokenCursor<G>),
    AttemptTokens(AttemptTokenCursor<G>),
    /// Resident immutable replacement span. This row is the macro call: it
    /// carries the non-owning definition coordinate, the optional local
    /// region lease, and (only for parameterized macros) the argument set.
    MacroBody(MacroBodyCursor<G>),
    /// Literal replay of one directly indexed macro-argument lane range.
    MacroArgument(MacroArgumentCursor<G>),
}

/// Compact cursor scalars shared by the two specialized macro span rows.
///
/// This deliberately is not `PackedInputFrame`: macro delivery has no
/// storage-kind or retirement dispatch, and therefore does not pay for the
/// generic token-list wrapper.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ResidentSpanCursor {
    identity: u64,
    position: u32,
    limit: u32,
    source: Option<tex_state::packed_input::SourceContext>,
}

impl ResidentSpanCursor {
    pub(crate) fn new(identity: InputLevelId, len: usize) -> Self {
        Self {
            identity: identity.0,
            position: 0,
            limit: u32::try_from(len).expect("macro span exceeds u32"),
            source: None,
        }
    }

    pub(crate) const fn identity(self) -> InputLevelId {
        InputLevelId(self.identity)
    }

    pub(crate) const fn position(self) -> usize {
        self.position as usize
    }

    pub(crate) const fn len(self) -> usize {
        self.limit as usize
    }

    pub(crate) const fn source_context(self) -> Option<tex_state::packed_input::SourceContext> {
        self.source
    }

    pub(crate) fn set_source_context(
        &mut self,
        source: Option<tex_state::packed_input::SourceContext>,
    ) {
        self.source = source;
    }

    fn advance(&mut self) -> Option<u32> {
        let position = self.position;
        (position < self.limit).then(|| {
            self.position += 1;
            position
        })
    }

    fn swap_position(&mut self, position: &mut u32) {
        core::mem::swap(&mut self.position, position);
    }
}

/// Specialized resident cursor over a definition arena replacement span.
#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct MacroBodyCursor<G> {
    pub(crate) body: tex_state::ResidentMacroBody<G>,
    pub(crate) arguments: Option<crate::execution_scratch::ArgumentSetId<G>>,
    pub(crate) name: tex_state::interner::Symbol,
    pub(crate) invocation: OriginId,
    pub(crate) frame: ResidentSpanCursor,
}

impl<G> MacroBodyCursor<G> {
    pub(crate) fn identity(&self) -> InputLevelId {
        self.frame.identity()
    }

    pub(crate) fn position(&self) -> usize {
        self.frame.position()
    }

    pub(crate) const fn active_source(&self) -> Option<tex_state::packed_input::SourceContext> {
        self.frame.source_context()
    }

    pub(crate) fn token_at(
        &self,
        _state: &tex_state::CommandContext<'_, G>,
    ) -> Option<PackedTokenAt> {
        self.body
            .word(self.position())
            .map(|word| (word, OriginId::UNKNOWN))
    }

    /// Advances the opaque resident replacement cursor by one packed word.
    ///
    /// The current definition access is the narrow integration seam for
    /// opaque stable definition spans. Delivery owns no view, decoded
    /// coordinate, or result wrapper; the caller handles Param before lending
    /// the returned word to its final command destination.
    #[inline(always)]
    pub(super) fn advance_word(
        &mut self,
        _state: &tex_state::CommandContext<'_, G>,
    ) -> Result<Option<TokenWord>, ()> {
        let position = self.frame.position as usize;
        let Some(word) = self.body.word(position) else {
            return Ok(None);
        };
        if self.frame.advance() != Some(position as u32) {
            return Err(());
        }
        Ok(Some(word))
    }
}

/// One registered-source level and its exact delivery identity.
#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct SourceLevel<G> {
    pub(crate) frame: PackedInputFrame,
    /// Stable owner slot for all variable source state. The input row carries
    /// one pointer to this authoritative owner; checkpoint execution state
    /// contains only its checked key and reversible cursor/owner values.
    pub(crate) slot: SourceSlotKey,
    pub(crate) generation: PhantomData<fn() -> G>,
}

impl<G> SourceLevel<G> {
    pub(crate) fn identity(&self) -> InputLevelId {
        InputLevelId(self.frame.identity())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SourceSlotKey(pub(crate) crate::timeline::PayloadHandle);

impl SourceSlotKey {
    pub(crate) const fn new(handle: crate::timeline::PayloadHandle) -> Self {
        Self(handle)
    }
}

#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct SourceSlot<G> {
    pub(crate) cursor: SourceCursor,
    /// Cached contribution of the currently loaded line to TeX's shared
    /// `buffer`. Only cold line/backing owner transitions may update it.
    pub(crate) occupied_buffer_slots: usize,
    /// e-TeX §24.362's once-only token list, pushed above this source when
    /// natural EOF is first observed and before `end_file_reading`.
    pub(crate) every_eof: Option<tex_state::TokenListId<G>>,
    /// e-TeX 2.6 [23.328]'s source-opening group/conditional ancestry.
    pub(crate) open_depths: Option<SourceOpenDepths>,
    /// tex.web §303's `name` classification and §360 retirement rule belong
    /// to the same sole owner as the backing they classify.
    pub(crate) name_class: SourceNameClass,
    pub(crate) retirement: SourceRetirement,
}

impl<G> SourceSlot<G> {
    pub(crate) fn new(
        cursor: SourceCursor,
        every_eof: Option<tex_state::TokenListId<G>>,
        open_depths: Option<SourceOpenDepths>,
        name_class: SourceNameClass,
        retirement: SourceRetirement,
    ) -> Self {
        Self {
            cursor,
            occupied_buffer_slots: 0,
            every_eof,
            open_depths,
            name_class,
            retirement,
        }
    }
}

/// e-TeX 2.6's `grp_stack`/`if_stack` entry for one open source level.
#[derive(Debug, Eq, Hash, PartialEq)]
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
    pub(crate) behavior: TokenBehavior,
    pub(crate) retirement: RetirementBehavior,
    pub(crate) trace: ReplayTrace,
    pub(crate) frame: PackedInputFrame,
    generation: PhantomData<fn() -> G>,
}

#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct ReplayTokenCursor<G> {
    pub(crate) replay: ReplayPayloadId<G>,
    pub(crate) len: u32,
    pub(crate) resident: ResidentReplayCursor,
    pub(crate) common: TokenCursor<G>,
}

#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct DurableTokenCursor<G> {
    pub(crate) list: tex_state::TokenListId<G>,
    pub(crate) len: u32,
    pub(crate) common: TokenCursor<G>,
}

#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct AttemptTokenCursor<G> {
    pub(crate) list: AttemptTokenListId,
    pub(crate) len: u32,
    pub(crate) common: TokenCursor<G>,
}

macro_rules! impl_common_token_cursor {
    ($cursor:ident) => {
        impl<G> core::ops::Deref for $cursor<G> {
            type Target = TokenCursor<G>;

            fn deref(&self) -> &Self::Target {
                &self.common
            }
        }

        impl<G> core::ops::DerefMut for $cursor<G> {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.common
            }
        }
    };
}

impl_common_token_cursor!(ReplayTokenCursor);
impl_common_token_cursor!(DurableTokenCursor);
impl_common_token_cursor!(AttemptTokenCursor);

/// First-class input cursor over one admitted absolute macro-argument range.
///
/// Unlike a generic packed-token cursor, delivery has no storage-domain
/// dispatch: admission validates the owning frame and exact argument once,
/// after which the owner-protected half-open range indexes the fixed-chunk
/// lane directly. The same frame is the lineage inherited by child token
/// lists.
#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct MacroArgumentCursor<G> {
    pub(crate) range: crate::execution_scratch::MacroArgumentRange<G>,
    pub(crate) slot: u8,
    /// Index of the provenance run containing the current absolute word.
    /// It advances only when sequential replay crosses a run boundary.
    pub(crate) origin_run: u32,
    pub(crate) frame: ResidentSpanCursor,
}

impl<G> MacroArgumentCursor<G> {
    pub(crate) fn identity(&self) -> InputLevelId {
        self.frame.identity()
    }

    pub(crate) fn position(&self) -> usize {
        self.frame.position()
    }

    pub(crate) const fn active_source(&self) -> Option<tex_state::packed_input::SourceContext> {
        self.frame.source_context()
    }

    pub(crate) fn argument_set(&self) -> crate::execution_scratch::ArgumentSetId<G> {
        self.range.frame()
    }

    pub(crate) fn token_at(
        &self,
        scratch: &crate::execution_scratch::ExecutionScratch<G>,
    ) -> Option<PackedTokenAt> {
        scratch
            .admitted_argument_word(self.range, self.position())
            .ok()
            .map(|word| (word.token_word(), word.origin()))
    }

    /// Tests TeX82's `loc=null` condition from the admitted scalar bounds.
    pub(crate) fn is_exhausted(&self) -> bool {
        self.position() >= self.range.len() as usize
    }

    #[inline(always)]
    pub(super) fn advance_word(
        &mut self,
        scratch: &crate::execution_scratch::ExecutionScratch<G>,
    ) -> Result<Option<TracedTokenWord>, ()> {
        let position = self.frame.position() as u32;
        let Ok(word) = scratch.admitted_argument_word_sequential(
            self.range,
            position as usize,
            &mut self.origin_run,
        ) else {
            return Ok(None);
        };
        if self.frame.advance() != Some(position) {
            return Err(());
        }
        Ok(Some(word))
    }
}

impl<G> TokenCursor<G> {
    pub(crate) fn new(
        behavior: TokenBehavior,
        retirement: RetirementBehavior,
        trace: ReplayTrace,
        frame: PackedInputFrame,
    ) -> Self {
        Self {
            behavior,
            retirement,
            trace,
            frame,
            generation: PhantomData,
        }
    }

    pub(crate) fn identity(&self) -> InputLevelId {
        InputLevelId(self.frame.identity())
    }

    pub(crate) fn position(&self) -> usize {
        self.frame.position() as usize
    }

    /// Peeks without advancing for stack-conservation and lifecycle checks.
    #[cold]
    pub(crate) fn indexed_token_at_cold(
        &self,
        span: &PackedTokenSpanHandle<G>,
        sources: PackedTokenSources<'_, G>,
        _state: &tex_state::CommandContext<'_, G>,
    ) -> Option<PackedTokenAt> {
        sources.indexed_token_at_cold(span, self.position())
    }
}

impl<G> InputLevel<G> {
    pub(crate) fn stored_common(&self) -> Option<&TokenCursor<G>> {
        match self {
            Self::ReplayTokens(cursor) => Some(&cursor.common),
            Self::DurableTokens(cursor) => Some(&cursor.common),
            Self::AttemptTokens(cursor) => Some(&cursor.common),
            _ => None,
        }
    }

    pub(crate) fn stored_common_mut(&mut self) -> Option<&mut TokenCursor<G>> {
        match self {
            Self::ReplayTokens(cursor) => Some(&mut cursor.common),
            Self::DurableTokens(cursor) => Some(&mut cursor.common),
            Self::AttemptTokens(cursor) => Some(&mut cursor.common),
            _ => None,
        }
    }

    #[cold]
    pub(crate) fn stored_span_cold(&self) -> Option<PackedTokenSpanHandle<G>> {
        match self {
            Self::ReplayTokens(cursor) => Some(PackedTokenSpanHandle::Replay {
                replay: cursor.replay,
                len: cursor.len,
            }),
            Self::DurableTokens(cursor) => Some(PackedTokenSpanHandle::DurableList {
                list: cursor.list.clone(),
                len: cursor.len,
            }),
            Self::AttemptTokens(cursor) => Some(PackedTokenSpanHandle::AttemptList {
                list: cursor.list,
                len: cursor.len,
            }),
            _ => None,
        }
    }

    #[cold]
    pub(crate) fn stored_indexed_token_at_cold(
        &self,
        sources: PackedTokenSources<'_, G>,
        state: &tex_state::CommandContext<'_, G>,
    ) -> Option<PackedTokenAt> {
        let common = self.stored_common()?;
        let span = self.stored_span_cold()?;
        common.indexed_token_at_cold(&span, sources, state)
    }
}

/// Canonical packed word plus its storage-independent diagnostic coordinates.
///
/// This value-returning view is reserved for non-delivering lifecycle probes.
/// The default command-delivery path writes into the caller's final
/// [`crate::CurrentCommand`] instead.
pub(crate) type PackedTokenAt = (TokenWord, OriginId);

/// Typed lifetime handle for one immutable packed-token span.
///
/// The source domain is selected exactly once when the input level is
/// created. Cold inspection uses indexed lookup; delivery carries a sequential
/// replay coordinate beside the input frame. The variants are a
/// storage-boundary lifetime distinction, not separate delivery objects.
#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) enum PackedTokenSpanHandle<G> {
    /// Compact coordinate into the generation-owned replay lane.
    Replay {
        replay: ReplayPayloadId<G>,
        len: u32,
    },
    /// One generation-durable immutable token list.
    DurableList {
        list: tex_state::TokenListId<G>,
        len: u32,
    },
    /// One attempt-local token list, replayed literally by range.
    AttemptList { list: AttemptTokenListId, len: u32 },
}

impl<G> Clone for PackedTokenSpanHandle<G> {
    fn clone(&self) -> Self {
        match self {
            Self::Replay { replay, len } => Self::Replay {
                replay: *replay,
                len: *len,
            },
            Self::DurableList { list, len } => Self::DurableList {
                list: list.clone(),
                len: *len,
            },
            Self::AttemptList { list, len } => Self::AttemptList {
                list: *list,
                len: *len,
            },
        }
    }
}

/// Borrowed storage boundary for every immutable stored-token source.
pub(crate) struct PackedTokenSources<'a, G> {
    replay: &'a ReplayLane<G>,
    attempt: &'a crate::attempt::AttemptArena<G>,
    #[cfg(any(test, feature = "profiling"))]
    benchmark_macro: Option<&'a [TokenWord]>,
}

impl<'a, G> PackedTokenSources<'a, G> {
    pub(crate) const fn new(
        replay: &'a ReplayLane<G>,
        attempt: &'a crate::attempt::AttemptArena<G>,
    ) -> Self {
        Self {
            replay,
            attempt,
            #[cfg(any(test, feature = "profiling"))]
            benchmark_macro: None,
        }
    }

    #[cfg(any(test, feature = "profiling"))]
    const fn with_benchmark_macro(mut self, words: &'a [TokenWord]) -> Self {
        self.benchmark_macro = Some(words);
        self
    }

    #[cfg(any(test, feature = "profiling"))]
    fn benchmark_token_at(
        &self,
        span: &PackedTokenSpanHandle<G>,
        index: usize,
    ) -> Option<PackedTokenAt> {
        match span {
            PackedTokenSpanHandle::Replay { replay, .. } => self
                .replay
                .indexed_get_cold(*replay, index)
                .map(|word| (word.token_word(), word.origin())),
            PackedTokenSpanHandle::AttemptList { list, .. } => self
                .attempt
                .token_word(*list, index)
                .ok()
                .map(|word| (word.token_word(), word.origin())),
            PackedTokenSpanHandle::DurableList { list, .. } => {
                list.word_at(index).map(|word| (word, OriginId::UNKNOWN))
            }
        }
    }

    /// Peeks through an admitted span without advancing its input frame.
    #[cold]
    pub(crate) fn indexed_token_at_cold(
        &self,
        span: &PackedTokenSpanHandle<G>,
        index: usize,
    ) -> Option<PackedTokenAt> {
        match span {
            PackedTokenSpanHandle::Replay { replay, .. } => self
                .replay
                .indexed_get_cold(*replay, index)
                .map(|word| (word.token_word(), word.origin())),
            PackedTokenSpanHandle::AttemptList { list, .. } => self
                .attempt
                .token_word(*list, index)
                .ok()
                .map(|word| (word.token_word(), word.origin())),
            PackedTokenSpanHandle::DurableList { list, .. } => {
                list.word_at(index).map(|word| (word, OriginId::UNKNOWN))
            }
        }
    }
}

/// Checkpoint-local execution state for one stable input payload.
///
/// Token-source identity, immutable token spans, replay classification, and
/// source nesting ancestry stay in the admitted [`InputLevel`]. The hot token
/// path journals only the fixed frame and retirement phase. Source line state
/// is larger because terminal replacement and `\read` admission can change
/// its backing, but it is captured only on the first source mutation in one
/// legal checkpoint interval.
#[derive(Clone, Copy, Debug)]
pub(crate) enum InputLevelInlineState {
    TokenPosition {
        position: u32,
        replay_cursor: Option<ResidentReplayCursor>,
    },
    Tokens {
        frame: PackedInputFrame,
        retirement: RetirementBehavior,
    },
    MacroBody {
        position: u32,
    },
    MacroArgument {
        position: u32,
        origin_run: u32,
    },
}

impl InputLevelInlineState {
    pub(crate) const fn token_position(
        position: u32,
        replay_cursor: Option<ResidentReplayCursor>,
    ) -> Self {
        Self::TokenPosition {
            position,
            replay_cursor,
        }
    }

    pub(crate) const fn new(frame: PackedInputFrame, retirement: RetirementBehavior) -> Self {
        Self::Tokens { frame, retirement }
    }

    pub(crate) const fn macro_span(position: usize) -> Self {
        Self::MacroBody {
            position: position as u32,
        }
    }

    pub(crate) const fn macro_argument(position: usize, origin_run: u32) -> Self {
        Self::MacroArgument {
            position: position as u32,
            origin_run,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SourceLexExecutionState {
    slot: SourceSlotKey,
    position: u32,
    cursor: SourceLexCursor,
    line_loaded: bool,
    backing_registered: bool,
    line_backing_registered: bool,
}

impl SourceLexExecutionState {
    pub(crate) fn capture<G>(source: &SourceLevel<G>, slot: &SourceSlot<G>) -> Self {
        Self {
            slot: source.slot,
            position: source.frame.position(),
            cursor: slot
                .cursor
                .line
                .as_ref()
                .map_or(SourceLexCursor::EMPTY, |line| line.cursor),
            line_loaded: slot.cursor.line.is_some(),
            backing_registered: slot.cursor.backing_registered,
            line_backing_registered: slot.cursor.line_backing_registered,
        }
    }

    pub(crate) fn rehome_offsets(
        &mut self,
        slot: SourceSlotKey,
        map: super::source::SourceOffsetMap,
    ) {
        if self.slot == slot {
            self.cursor.byte_cursor = map.map(self.cursor.byte_cursor);
        }
    }
}

#[derive(Debug)]
pub(crate) enum SourceLevelExecutionState<G> {
    Cursor {
        slot: SourceSlotKey,
        position: u32,
        cursor: SourceCursorExecutionState,
    },
    EveryEof {
        slot: SourceSlotKey,
        position: u32,
        cursor: SourceCursorExecutionState,
        every_eof: Option<tex_state::TokenListId<G>>,
    },
    Backing {
        slot: SourceSlotKey,
        position: u32,
        cursor: SourceCursorExecutionState,
        backing: RegisteredSource,
        name_class: SourceNameClass,
    },
    /// Editor restart substitutes only the immutable physical backing. The
    /// normalized current line and every other source cursor field remain the
    /// checkpoint's semantic state.
    PhysicalBacking {
        slot: SourceSlotKey,
        backing: RegisteredSource,
        backing_registered: bool,
    },
}

impl<G> SourceLevelExecutionState<G> {
    pub(crate) fn rehome_physical_backing(
        &mut self,
        slot: SourceSlotKey,
        accepted: &[u8],
        replacement: &RegisteredSource,
    ) {
        let (state_slot, backing) = match self {
            Self::Backing { slot, backing, .. } | Self::PhysicalBacking { slot, backing, .. } => {
                (*slot, backing)
            }
            Self::Cursor { .. } | Self::EveryEof { .. } => return,
        };
        if state_slot == slot && backing.is_editor_backing(accepted) {
            backing.clone_from(replacement);
        }
    }

    pub(crate) fn rehome_offsets(
        &mut self,
        root_slot: SourceSlotKey,
        map: super::source::SourceOffsetMap,
    ) {
        match self {
            Self::Cursor { slot, cursor, .. }
            | Self::EveryEof { slot, cursor, .. }
            | Self::Backing { slot, cursor, .. }
                if *slot == root_slot =>
            {
                cursor.rehome_offsets(map);
            }
            Self::PhysicalBacking { .. } => {}
            Self::Cursor { .. } | Self::EveryEof { .. } | Self::Backing { .. } => {}
        }
    }

    pub(crate) fn cursor(source: &SourceLevel<G>, slot: &mut SourceSlot<G>) -> Self {
        Self::Cursor {
            slot: source.slot,
            position: source.frame.position(),
            cursor: slot.cursor.take_execution_state(),
        }
    }

    pub(crate) fn every_eof(source: &SourceLevel<G>, slot: &mut SourceSlot<G>) -> Self {
        Self::EveryEof {
            slot: source.slot,
            position: source.frame.position(),
            cursor: slot.cursor.take_execution_state(),
            every_eof: slot.every_eof.take(),
        }
    }

    pub(crate) fn backing(
        source: &SourceLevel<G>,
        slot: &mut SourceSlot<G>,
        replacement: RegisteredSource,
    ) -> Self {
        let backing = std::mem::replace(&mut slot.cursor.backing, replacement);
        Self::Backing {
            slot: source.slot,
            position: source.frame.position(),
            cursor: slot.cursor.take_execution_state(),
            backing,
            name_class: slot.name_class,
        }
    }

    pub(crate) fn physical_backing(
        source: &SourceLevel<G>,
        slot: &mut SourceSlot<G>,
        replacement: RegisteredSource,
    ) -> Self {
        let backing = std::mem::replace(&mut slot.cursor.backing, replacement);
        let backing_registered = std::mem::replace(&mut slot.cursor.backing_registered, false);
        Self::PhysicalBacking {
            slot: source.slot,
            backing,
            backing_registered,
        }
    }
}

impl<G> InputLevel<G> {
    /// External source context inherited when this semantic input row became
    /// visible. Source owners remain exclusively in `InputStack`; this is the
    /// compact execution fact delivered commands need for checkpoint origin.
    pub(crate) const fn source_context(&self) -> Option<tex_state::packed_input::SourceContext> {
        match self {
            Self::Source(source) => source.frame.source_context(),
            Self::ReplayTokens(tokens) => tokens.common.frame.source_context(),
            Self::DurableTokens(tokens) => tokens.common.frame.source_context(),
            Self::AttemptTokens(tokens) => tokens.common.frame.source_context(),
            Self::MacroBody(body) => body.frame.source_context(),
            Self::MacroArgument(argument) => argument.frame.source_context(),
        }
    }

    pub(crate) fn set_source_context(
        &mut self,
        source: Option<tex_state::packed_input::SourceContext>,
    ) {
        match self {
            Self::Source(level) => level.frame.set_source_context(source),
            Self::ReplayTokens(tokens) => tokens.frame.set_source_context(source),
            Self::DurableTokens(tokens) => tokens.frame.set_source_context(source),
            Self::AttemptTokens(tokens) => tokens.frame.set_source_context(source),
            Self::MacroBody(body) => body.frame.set_source_context(source),
            Self::MacroArgument(argument) => argument.frame.set_source_context(source),
        }
    }

    pub(crate) fn swap_input_inline_state(&mut self, state: &mut InputLevelInlineState) {
        match self {
            Self::ReplayTokens(tokens) => match state {
                InputLevelInlineState::TokenPosition {
                    position,
                    replay_cursor,
                } => {
                    tokens.frame.swap_position(position);
                    let restored = replay_cursor
                        .replace(tokens.resident)
                        .expect("replay row inverse retains replay coordinate");
                    tokens.resident = restored;
                }
                InputLevelInlineState::Tokens { frame, retirement } => {
                    std::mem::swap(&mut tokens.frame, frame);
                    std::mem::swap(&mut tokens.retirement, retirement);
                }
                InputLevelInlineState::MacroBody { .. }
                | InputLevelInlineState::MacroArgument { .. } => {
                    unreachable!("token row inverse kind changed")
                }
            },
            Self::DurableTokens(tokens) => match state {
                InputLevelInlineState::TokenPosition {
                    position,
                    replay_cursor,
                } => {
                    debug_assert!(replay_cursor.is_none());
                    tokens.frame.swap_position(position);
                }
                InputLevelInlineState::Tokens { frame, retirement } => {
                    std::mem::swap(&mut tokens.frame, frame);
                    std::mem::swap(&mut tokens.retirement, retirement);
                }
                InputLevelInlineState::MacroBody { .. }
                | InputLevelInlineState::MacroArgument { .. } => {
                    unreachable!("token row inverse kind changed")
                }
            },
            Self::AttemptTokens(tokens) => match state {
                InputLevelInlineState::TokenPosition {
                    position,
                    replay_cursor,
                } => {
                    debug_assert!(replay_cursor.is_none());
                    tokens.frame.swap_position(position);
                }
                InputLevelInlineState::Tokens { frame, retirement } => {
                    std::mem::swap(&mut tokens.frame, frame);
                    std::mem::swap(&mut tokens.retirement, retirement);
                }
                InputLevelInlineState::MacroBody { .. }
                | InputLevelInlineState::MacroArgument { .. } => {
                    unreachable!("token row inverse kind changed")
                }
            },
            Self::MacroBody(body) => {
                let InputLevelInlineState::MacroBody { position } = state else {
                    unreachable!("macro body inverse kind changed")
                };
                body.frame.swap_position(position);
            }
            Self::MacroArgument(argument) => {
                let InputLevelInlineState::MacroArgument {
                    position,
                    origin_run,
                } = state
                else {
                    unreachable!("macro argument inverse kind changed")
                };
                argument.frame.swap_position(position);
                std::mem::swap(&mut argument.origin_run, origin_run);
            }
            Self::Source(_) => unreachable!("a source frame uses the source lexer lane"),
        }
    }
}

impl<G> SourceLevel<G> {
    pub(crate) fn swap_lex_state(
        &mut self,
        slot: &mut SourceSlot<G>,
        state: &mut SourceLexExecutionState,
    ) {
        assert_eq!(self.slot, state.slot, "source inverse names the live slot");
        self.frame.swap_position(&mut state.position);
        std::mem::swap(
            &mut slot.cursor.backing_registered,
            &mut state.backing_registered,
        );
        std::mem::swap(
            &mut slot.cursor.line_backing_registered,
            &mut state.line_backing_registered,
        );
        match slot.cursor.line.as_mut() {
            Some(line) if state.line_loaded => std::mem::swap(&mut line.cursor, &mut state.cursor),
            None if !state.line_loaded => {}
            _ => unreachable!("compact source mutation cannot replace line ownership"),
        }
    }

    pub(crate) fn swap_execution_state(
        &mut self,
        owner: &mut SourceSlot<G>,
        state: &mut SourceLevelExecutionState<G>,
    ) {
        match state {
            SourceLevelExecutionState::Cursor {
                slot,
                position,
                cursor,
            } => {
                assert_eq!(self.slot, *slot, "source inverse names the live slot");
                self.frame.swap_position(position);
                owner.cursor.swap_execution_state(cursor);
            }
            SourceLevelExecutionState::EveryEof {
                slot,
                position,
                cursor,
                every_eof,
            } => {
                assert_eq!(self.slot, *slot, "source inverse names the live slot");
                self.frame.swap_position(position);
                owner.cursor.swap_execution_state(cursor);
                std::mem::swap(&mut owner.every_eof, every_eof);
            }
            SourceLevelExecutionState::Backing {
                slot,
                position,
                cursor,
                backing,
                name_class,
            } => {
                assert_eq!(self.slot, *slot, "source inverse names the live slot");
                self.frame.swap_position(position);
                owner.cursor.swap_execution_state(cursor);
                std::mem::swap(&mut owner.cursor.backing, backing);
                std::mem::swap(&mut owner.name_class, name_class);
            }
            SourceLevelExecutionState::PhysicalBacking {
                slot,
                backing,
                backing_registered,
            } => {
                assert_eq!(self.slot, *slot, "source inverse names the live slot");
                std::mem::swap(&mut owner.cursor.backing, backing);
                std::mem::swap(&mut owner.cursor.backing_registered, backing_registered);
            }
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

/// Current physical replay run and segment coordinate for one resident input
/// row. `remaining` and `segment_end` are scalar boundary coordinates, so a
/// warm word neither rescans the run prefix nor rereads segment length.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ResidentReplayCursor {
    run: ResidentReplayRun,
    segment: u32,
    remaining: u32,
    offset: u16,
    segment_end: u16,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ResidentReplayRun {
    Empty,
    Prefix,
    SegmentedBody,
    OwnedBody,
}

impl ResidentReplayCursor {
    pub(crate) const EMPTY: Self = Self {
        run: ResidentReplayRun::Empty,
        segment: 0,
        remaining: 0,
        offset: 0,
        segment_end: 0,
    };
}

#[derive(Debug)]
pub(crate) struct ReplayTransientMark {
    entries: usize,
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
    fn retained_bytes(&self) -> usize {
        let segment_bytes = self
            .active
            .len()
            .saturating_add(self.spare.len())
            .saturating_mul(
                std::mem::size_of::<ReplaySegment<T>>()
                    .saturating_add(REPLAY_SEGMENT_ITEMS.saturating_mul(std::mem::size_of::<T>())),
            );
        std::mem::size_of::<Self>()
            .saturating_add(
                self.active
                    .capacity()
                    .saturating_mul(std::mem::size_of::<ActiveReplaySegment<T>>()),
            )
            .saturating_add(
                self.spare
                    .capacity()
                    .saturating_mul(std::mem::size_of::<Arc<ReplaySegment<T>>>()),
            )
            .saturating_add(segment_bytes)
    }

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

    fn resident_cursor(
        &self,
        start: ReplayLaneCursor,
        len: u32,
        run: ResidentReplayRun,
    ) -> Option<ResidentReplayCursor> {
        if len == 0 {
            return Some(ResidentReplayCursor::EMPTY);
        }
        let segment = self.active.get(start.segment as usize)?;
        (start.offset < segment.used).then_some(ResidentReplayCursor {
            run,
            segment: start.segment,
            remaining: len,
            offset: start.offset,
            segment_end: segment.used,
        })
    }

    #[inline(always)]
    fn advance_sequential(
        &self,
        cursor: &mut ResidentReplayCursor,
        #[cfg(test)] segment_inspections: &mut u64,
    ) -> Option<&T> {
        if cursor.remaining == 0 {
            return None;
        }
        if cursor.offset == cursor.segment_end {
            cursor.segment = cursor.segment.checked_add(1)?;
            cursor.offset = 0;
            let segment = self.active.get(cursor.segment as usize)?;
            cursor.segment_end = segment.used;
            #[cfg(test)]
            {
                *segment_inspections = segment_inspections.saturating_add(1);
            }
        }
        let value = self
            .active
            .get(cursor.segment as usize)?
            .storage
            .values
            .get(usize::from(cursor.offset))?;
        cursor.offset = cursor.offset.checked_add(1)?;
        cursor.remaining -= 1;
        Some(value)
    }

    /// Cold indexed lookup for diagnostics, checkpoint projection, and test
    /// inspection. Resident delivery must use `advance_sequential`.
    #[cold]
    fn indexed_get_cold(&self, start: ReplayLaneCursor, mut index: usize) -> Option<&T> {
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

/// Generation-branded mutable destination for one escaping inserted list.
///
/// The destination is allocated in the replay owner before scanning starts.
/// Finishing moves its storage header into the ordered replay entries and
/// publishes only the resulting coordinate; no attempt-local list or token
/// promotion copy exists.
#[derive(Debug)]
pub(crate) struct ReplayInputBuilderId<G> {
    slot: u32,
    _generation: PhantomData<fn(&G) -> &G>,
}

impl<G> Copy for ReplayInputBuilderId<G> {}
impl<G> Clone for ReplayInputBuilderId<G> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<G> PartialEq for ReplayInputBuilderId<G> {
    fn eq(&self, other: &Self) -> bool {
        self.slot == other.slot
    }
}
impl<G> Eq for ReplayInputBuilderId<G> {}
impl<G> Hash for ReplayInputBuilderId<G> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.slot.hash(state);
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ReplaySpan {
    start: ReplayLaneCursor,
    len: u32,
}

#[derive(Debug, Eq, Hash, PartialEq)]
struct OwnedReplayWords {
    lane: SegmentedReplayLane<TracedTokenWord>,
    len: u32,
}

impl Clone for OwnedReplayWords {
    fn clone(&self) -> Self {
        Self {
            lane: self.lane.clone(),
            len: self.len,
        }
    }
}

impl OwnedReplayWords {
    #[cold]
    fn indexed_get_cold(&self, index: usize) -> Option<&TracedTokenWord> {
        self.lane.indexed_get_cold(
            ReplayLaneCursor {
                segment: 0,
                offset: 0,
            },
            index,
        )
    }

    fn clear(&mut self) -> Result<(), crate::execution_scratch::ScratchError> {
        let next_reuse = self
            .lane
            .active
            .len()
            .checked_add(self.lane.spare.len())
            .and_then(|len| len.checked_add(1))
            .ok_or(crate::execution_scratch::ScratchError::CapacityOverflow)?;
        self.lane
            .active
            .try_reserve(next_reuse.saturating_sub(self.lane.active.len()))
            .map_err(|_| crate::execution_scratch::ScratchError::AllocationFailed)?;
        self.lane
            .spare
            .try_reserve(next_reuse.saturating_sub(self.lane.spare.len()))
            .map_err(|_| crate::execution_scratch::ScratchError::AllocationFailed)?;
        self.lane.restore(ReplayLaneMark {
            segments: 0,
            tail_used: 0,
        })?;
        self.len = 0;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum ReplayBodyWords {
    Segmented(ReplaySpan),
    Owned(OwnedReplayWords),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ReplayEntry {
    word_mark: ReplayLaneMark,
    body_words: ReplayBodyWords,
    prefix_words: Option<ReplaySpan>,
    ownership: PackedTokenOwnership,
    released: bool,
}

impl ReplayEntry {
    fn len(&self) -> usize {
        let body = match &self.body_words {
            ReplayBodyWords::Segmented(span) => span.len,
            ReplayBodyWords::Owned(words) => words.len,
        };
        self.prefix_words.map_or(0, |span| span.len as usize) + body as usize
    }
}

#[derive(Debug)]
pub(crate) struct ReplayLane<G> {
    entries: Vec<ReplayEntry>,
    input_builders: Vec<OwnedReplayWords>,
    spare_input_builders: Vec<OwnedReplayWords>,
    input_builder_high_water: usize,
    words: SegmentedReplayLane<TracedTokenWord>,
    transient_depth: u32,
    _generation: PhantomData<fn(&G) -> &G>,
}

impl<G> Default for ReplayLane<G> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            input_builders: Vec::new(),
            spare_input_builders: Vec::new(),
            input_builder_high_water: 0,
            words: SegmentedReplayLane::default(),
            transient_depth: 0,
            _generation: PhantomData,
        }
    }
}

impl<G> Clone for ReplayLane<G> {
    fn clone(&self) -> Self {
        let input_builder_high_water = self
            .entries
            .iter()
            .filter(|entry| matches!(&entry.body_words, ReplayBodyWords::Owned(_)))
            .count()
            .saturating_add(self.input_builders.len());
        Self {
            entries: self.entries.clone(),
            input_builders: self.input_builders.clone(),
            spare_input_builders: Vec::with_capacity(input_builder_high_water),
            input_builder_high_water,
            words: self.words.clone(),
            transient_depth: 0,
            _generation: PhantomData,
        }
    }
}

impl<G> PartialEq for ReplayLane<G> {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries
            && self.input_builders == other.input_builders
            && self.words == other.words
    }
}
impl<G> Eq for ReplayLane<G> {}
impl<G> Hash for ReplayLane<G> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.entries.hash(state);
        self.input_builders.hash(state);
        self.words.hash(state);
    }
}

impl<G> ReplayLane<G> {
    pub(crate) fn retained_bytes(&self) -> usize {
        let owned = self
            .entries
            .iter()
            .map(|entry| match &entry.body_words {
                ReplayBodyWords::Owned(words) => words.lane.retained_bytes(),
                ReplayBodyWords::Segmented(_) => 0,
            })
            .sum::<usize>();
        let builders = self
            .input_builders
            .iter()
            .chain(self.spare_input_builders.iter())
            .map(|words| words.lane.retained_bytes())
            .sum::<usize>();
        std::mem::size_of::<Self>()
            .saturating_add(
                self.entries
                    .capacity()
                    .saturating_mul(std::mem::size_of::<ReplayEntry>()),
            )
            .saturating_add(
                self.input_builders
                    .capacity()
                    .saturating_mul(std::mem::size_of::<OwnedReplayWords>()),
            )
            .saturating_add(
                self.spare_input_builders
                    .capacity()
                    .saturating_mul(std::mem::size_of::<OwnedReplayWords>()),
            )
            .saturating_add(self.words.retained_bytes())
            .saturating_add(owned)
            .saturating_add(builders)
    }

    fn push_words(
        &mut self,
        tokens: impl IntoIterator<Item = BackedUpToken>,
        ownership: PackedTokenOwnership,
    ) -> Result<PackedTokenSpanHandle<G>, crate::execution_scratch::ScratchError> {
        let word_mark = self.words.mark();
        let mut start = None;
        let mut len = 0_u32;
        for token in tokens {
            start.get_or_insert(self.words.push(token.spelling)?);
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
            body_words: ReplayBodyWords::Segmented(ReplaySpan {
                start: start.unwrap_or(empty),
                len,
            }),
            prefix_words: None,
            ownership,
            released: false,
        });
        Ok(PackedTokenSpanHandle::Replay {
            replay: ReplayPayloadId {
                entry,
                _generation: PhantomData,
            },
            len,
        })
    }

    /// Allocates the final owner for one escaping inserted token list.
    pub(crate) fn begin_input_builder(
        &mut self,
    ) -> Result<ReplayInputBuilderId<G>, crate::execution_scratch::ScratchError> {
        let slot = u32::try_from(self.input_builders.len())
            .map_err(|_| crate::execution_scratch::ScratchError::CapacityOverflow)?;
        self.input_builders
            .try_reserve(1)
            .map_err(|_| crate::execution_scratch::ScratchError::AllocationFailed)?;
        let words = match self.spare_input_builders.pop() {
            Some(words) => words,
            None => {
                let next_high_water = self
                    .input_builder_high_water
                    .checked_add(1)
                    .ok_or(crate::execution_scratch::ScratchError::CapacityOverflow)?;
                self.spare_input_builders
                    .try_reserve(next_high_water.saturating_sub(self.spare_input_builders.len()))
                    .map_err(|_| crate::execution_scratch::ScratchError::AllocationFailed)?;
                self.input_builder_high_water = next_high_water;
                OwnedReplayWords {
                    lane: SegmentedReplayLane::default(),
                    len: 0,
                }
            }
        };
        self.input_builders.push(words);
        Ok(ReplayInputBuilderId {
            slot,
            _generation: PhantomData,
        })
    }

    /// Appends directly to an escaping inserted list's final storage.
    pub(crate) fn push_input_builder_word(
        &mut self,
        builder: ReplayInputBuilderId<G>,
        word: TracedTokenWord,
    ) -> Result<(), crate::execution_scratch::ScratchError> {
        let words = self
            .input_builders
            .get_mut(builder.slot as usize)
            .ok_or(crate::execution_scratch::ScratchError::InvalidCoordinate)?;
        words.lane.push(word)?;
        words.len = words
            .len
            .checked_add(1)
            .ok_or(crate::execution_scratch::ScratchError::CapacityOverflow)?;
        Ok(())
    }

    pub(crate) fn input_builder_get(
        &self,
        builder: ReplayInputBuilderId<G>,
        index: usize,
    ) -> Option<&TracedTokenWord> {
        self.input_builders
            .get(builder.slot as usize)?
            .indexed_get_cold(index)
    }

    pub(crate) fn input_builder_len(&self, builder: ReplayInputBuilderId<G>) -> Option<u32> {
        self.input_builders
            .get(builder.slot as usize)
            .map(|words| words.len)
    }

    /// Publishes an escaping list without copying its words.
    pub(crate) fn finish_input_builder(
        &mut self,
        builder: ReplayInputBuilderId<G>,
    ) -> Result<PackedTokenSpanHandle<G>, crate::execution_scratch::ScratchError> {
        if builder.slot as usize + 1 != self.input_builders.len() {
            return Err(crate::execution_scratch::ScratchError::InvalidCoordinate);
        }
        let entry = u32::try_from(self.entries.len())
            .map_err(|_| crate::execution_scratch::ScratchError::CapacityOverflow)?;
        self.entries
            .try_reserve(1)
            .map_err(|_| crate::execution_scratch::ScratchError::AllocationFailed)?;
        let words = self
            .input_builders
            .pop()
            .expect("validated escaping input builder remains live");
        let len = words.len;
        self.entries.push(ReplayEntry {
            word_mark: self.words.mark(),
            body_words: ReplayBodyWords::Owned(words),
            prefix_words: None,
            ownership: PackedTokenOwnership::Transient,
            released: false,
        });
        Ok(PackedTokenSpanHandle::Replay {
            replay: ReplayPayloadId {
                entry,
                _generation: PhantomData,
            },
            len,
        })
    }

    pub(crate) fn discard_input_builder(
        &mut self,
        builder: ReplayInputBuilderId<G>,
    ) -> Result<(), crate::execution_scratch::ScratchError> {
        if builder.slot as usize + 1 != self.input_builders.len() {
            return Err(crate::execution_scratch::ScratchError::InvalidCoordinate);
        }
        let mut words = self
            .input_builders
            .pop()
            .expect("validated escaping input builder remains live");
        words.clear()?;
        self.spare_input_builders.push(words);
        Ok(())
    }

    pub(crate) fn resident_cursor(
        &self,
        replay: ReplayPayloadId<G>,
    ) -> Option<ResidentReplayCursor> {
        let entry = self.entries.get(replay.entry as usize)?;
        if entry.released {
            return None;
        }
        if let Some(prefix) = entry.prefix_words.filter(|prefix| prefix.len != 0) {
            return self
                .words
                .resident_cursor(prefix.start, prefix.len, ResidentReplayRun::Prefix);
        }
        self.body_resident_cursor(entry)
    }

    fn body_resident_cursor(&self, entry: &ReplayEntry) -> Option<ResidentReplayCursor> {
        match &entry.body_words {
            ReplayBodyWords::Segmented(words) => {
                self.words
                    .resident_cursor(words.start, words.len, ResidentReplayRun::SegmentedBody)
            }
            ReplayBodyWords::Owned(words) => words.lane.resident_cursor(
                ReplayLaneCursor {
                    segment: 0,
                    offset: 0,
                },
                words.len,
                ResidentReplayRun::OwnedBody,
            ),
        }
    }

    #[inline(always)]
    pub(crate) fn advance_sequential(
        &self,
        replay: ReplayPayloadId<G>,
        cursor: &mut ResidentReplayCursor,
        #[cfg(test)] segment_inspections: &mut u64,
        #[cfg(test)] run_transitions: &mut u64,
    ) -> Option<TracedTokenWord> {
        if cursor.remaining == 0 && cursor.run == ResidentReplayRun::Prefix {
            let entry = self.entries.get(replay.entry as usize)?;
            if entry.released {
                return None;
            }
            *cursor = self.body_resident_cursor(entry)?;
            #[cfg(test)]
            {
                *run_transitions = run_transitions.saturating_add(1);
                if cursor.run != ResidentReplayRun::Empty {
                    *segment_inspections = segment_inspections.saturating_add(1);
                }
            }
        }
        match cursor.run {
            ResidentReplayRun::Empty => None,
            ResidentReplayRun::Prefix | ResidentReplayRun::SegmentedBody => self
                .words
                .advance_sequential(
                    cursor,
                    #[cfg(test)]
                    segment_inspections,
                )
                .copied(),
            ResidentReplayRun::OwnedBody => {
                self.entries
                    .get(replay.entry as usize)
                    .and_then(|entry| match &entry.body_words {
                        ReplayBodyWords::Owned(words) if !entry.released => words
                            .lane
                            .advance_sequential(
                                cursor,
                                #[cfg(test)]
                                segment_inspections,
                            )
                            .copied(),
                        ReplayBodyWords::Segmented(_) | ReplayBodyWords::Owned(_) => None,
                    })
            }
        }
    }

    /// Cold indexed lookup for diagnostic rendering, semantic projection, and
    /// lifecycle probes. Ordinary resident delivery has no indexed replay API.
    #[cold]
    pub(crate) fn indexed_get_cold(
        &self,
        replay: ReplayPayloadId<G>,
        index: usize,
    ) -> Option<TracedTokenWord> {
        let entry = self.entries.get(replay.entry as usize)?;
        if entry.released {
            return None;
        }
        if index >= entry.len() {
            return None;
        }
        let prefix_len = entry.prefix_words.map_or(0, |span| span.len as usize);
        if index < prefix_len {
            let words = entry.prefix_words?;
            return self.words.indexed_get_cold(words.start, index).copied();
        }
        let local = index - prefix_len;
        let spelling = match &entry.body_words {
            ReplayBodyWords::Segmented(words) => {
                *self.words.indexed_get_cold(words.start, local)?
            }
            ReplayBodyWords::Owned(words) => *words.indexed_get_cold(local)?,
        };
        Some(spelling)
    }

    pub(crate) fn ownership(&self, replay: ReplayPayloadId<G>) -> Option<PackedTokenOwnership> {
        self.entries
            .get(replay.entry as usize)
            .filter(|entry| !entry.released)
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
        let mut len = 0_u32;
        for token in prefix {
            word_start.get_or_insert(self.words.push(token.spelling)?);
            len = len
                .checked_add(1)
                .ok_or(crate::execution_scratch::ScratchError::CapacityOverflow)?;
        }
        if len != 0 {
            self.entries[expected].prefix_words = word_start.map(|start| ReplaySpan { start, len });
        }
        Ok(len)
    }

    pub(crate) fn release(
        &mut self,
        replay: ReplayPayloadId<G>,
    ) -> Result<(), crate::execution_scratch::ScratchError> {
        let index = replay.entry as usize;
        if index >= self.entries.len()
            || self.entries[index].released
            || self.entries[index + 1..]
                .iter()
                .any(|entry| !entry.released)
        {
            return Err(crate::execution_scratch::ScratchError::InvalidCoordinate);
        }
        self.entries[index].released = true;
        if self.transient_depth == 0 {
            self.reclaim_released_suffix()?;
        }
        Ok(())
    }

    pub(crate) fn begin_transient(&mut self) -> ReplayTransientMark {
        self.transient_depth = self
            .transient_depth
            .checked_add(1)
            .expect("nested replay rollback depth is bounded");
        ReplayTransientMark {
            entries: self.entries.len(),
        }
    }

    pub(crate) fn rollback_transient(&mut self, mark: ReplayTransientMark) {
        let first = mark.entries.min(self.entries.len());
        for entry in &mut self.entries[first..] {
            entry.released = true;
        }
    }

    pub(crate) fn reactivate(&mut self, replay: ReplayPayloadId<G>) {
        if let Some(entry) = self.entries.get_mut(replay.entry as usize) {
            entry.released = false;
        }
    }

    pub(crate) fn end_transient(&mut self) -> Result<(), crate::execution_scratch::ScratchError> {
        self.transient_depth = self
            .transient_depth
            .checked_sub(1)
            .ok_or(crate::execution_scratch::ScratchError::InvalidCoordinate)?;
        if self.transient_depth == 0 {
            self.reclaim_released_suffix()?;
        }
        Ok(())
    }

    fn reclaim_released_suffix(&mut self) -> Result<(), crate::execution_scratch::ScratchError> {
        while self.entries.last().is_some_and(|entry| entry.released) {
            let entry = self
                .entries
                .pop()
                .expect("released replay suffix remains live");
            self.words.restore(entry.word_mark)?;
            if let ReplayBodyWords::Owned(mut words) = entry.body_words {
                words.clear()?;
                self.spare_input_builders.push(words);
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn input_builder_storage_counts(&self) -> (usize, usize, usize, usize, usize) {
        let owned_entries = self
            .entries
            .iter()
            .filter(|entry| matches!(&entry.body_words, ReplayBodyWords::Owned(_)))
            .count();
        let active_segments = self
            .input_builders
            .iter()
            .chain(self.entries.iter().filter_map(|entry| {
                let ReplayBodyWords::Owned(words) = &entry.body_words else {
                    return None;
                };
                Some(words)
            }))
            .map(|words| words.lane.active.len())
            .sum();
        let spare_segments = self
            .spare_input_builders
            .iter()
            .map(|words| words.lane.spare.len())
            .sum();
        (
            self.input_builders.len(),
            owned_entries,
            self.spare_input_builders.len(),
            active_segments,
            spare_segments,
        )
    }
}

pub(crate) trait PackedTokenSpanSource<G> {
    fn admit(
        self,
        lane: &mut ReplayLane<G>,
    ) -> Result<PackedTokenSpanHandle<G>, crate::execution_scratch::ScratchError>;
}

impl<G> PackedTokenSpanSource<G> for PackedTokenSpanHandle<G> {
    fn admit(
        self,
        _lane: &mut ReplayLane<G>,
    ) -> Result<PackedTokenSpanHandle<G>, crate::execution_scratch::ScratchError> {
        Ok(self)
    }
}

pub(crate) struct TracedReplaySeed<I> {
    tokens: I,
    ownership: PackedTokenOwnership,
}
impl<G, I: Iterator<Item = TracedTokenWord>> PackedTokenSpanSource<G> for TracedReplaySeed<I> {
    fn admit(
        self,
        lane: &mut ReplayLane<G>,
    ) -> Result<PackedTokenSpanHandle<G>, crate::execution_scratch::ScratchError> {
        lane.push_words(
            self.tokens.map(|spelling| BackedUpToken { spelling }),
            self.ownership,
        )
    }
}

pub(crate) struct SemanticReplaySeed<I> {
    tokens: I,
    origin: OriginId,
    ownership: PackedTokenOwnership,
}
impl<G, I: Iterator<Item = Token>> PackedTokenSpanSource<G> for SemanticReplaySeed<I> {
    fn admit(
        self,
        lane: &mut ReplayLane<G>,
    ) -> Result<PackedTokenSpanHandle<G>, crate::execution_scratch::ScratchError> {
        lane.push_words(
            self.tokens.map(|token| BackedUpToken {
                spelling: TracedTokenWord::pack(token, self.origin),
            }),
            self.ownership,
        )
    }
}

pub(crate) struct BackedReplaySeed<I> {
    tokens: I,
}
impl<G, I: Iterator<Item = BackedUpToken>> PackedTokenSpanSource<G> for BackedReplaySeed<I> {
    fn admit(
        self,
        lane: &mut ReplayLane<G>,
    ) -> Result<PackedTokenSpanHandle<G>, crate::execution_scratch::ScratchError> {
        lane.push_words(self.tokens, PackedTokenOwnership::BackedUp)
    }
}

pub(crate) struct StoredReplaySeed<'a, I> {
    tokens: &'a [Token],
    origins: I,
}
impl<G, I: Iterator<Item = OriginId>> PackedTokenSpanSource<G> for StoredReplaySeed<'_, I> {
    fn admit(
        mut self,
        lane: &mut ReplayLane<G>,
    ) -> Result<PackedTokenSpanHandle<G>, crate::execution_scratch::ScratchError> {
        lane.push_words(
            self.tokens.iter().copied().map(|token| BackedUpToken {
                spelling: TracedTokenWord::pack(
                    token,
                    self.origins.next().unwrap_or(OriginId::UNKNOWN),
                ),
            }),
            PackedTokenOwnership::Stored,
        )
    }
}

impl PackedTokenSpanHandle<()> {
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

impl<G> PackedTokenSpanHandle<G> {
    pub(crate) fn durable(words: tex_state::TokenListView<G>) -> Self {
        let len = u32::try_from(words.len()).expect("durable token-list length exceeds u32");
        Self::DurableList {
            list: words.into_id(),
            len,
        }
    }

    pub(crate) fn frame_len(&self) -> usize {
        match self {
            Self::Replay { len, .. } => *len as usize,
            Self::DurableList { len, .. } => *len as usize,
            Self::AttemptList { len, .. } => *len as usize,
        }
    }
}

/// Focused profiling harness for the uniform packed stored-token cursor.
///
/// Construction admits each real storage/lifetime domain once. [`Self::run`]
/// then measures only canonical `TokenWord` access, scalar advancement, exact
/// end-of-span retirement, and scalar rollback; it creates no alternate
/// command-delivery semantics.
#[cfg(any(test, feature = "profiling"))]
pub struct MixedPackedCursorBenchmark<G> {
    spans: [PackedTokenSpanHandle<G>; 4],
    behaviors: [TokenBehavior; 4],
    macro_argument: crate::execution_scratch::MacroArgumentRange<G>,
    positions: [u32; 5],
    replay: ReplayLane<G>,
    attempt: crate::attempt::AttemptArena<G>,
    scratch: crate::execution_scratch::ExecutionScratch<G>,
    semantic: [TokenWord; 4],
}

/// Absolute work receipt from one warmed mixed-source cursor run.
#[cfg(any(test, feature = "profiling"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MixedPackedCursorReceipt {
    pub calls: u64,
    pub retirements: u64,
    pub rollbacks: u64,
    pub checksum: u64,
}

#[cfg(any(test, feature = "profiling"))]
impl<G> MixedPackedCursorBenchmark<G> {
    /// Admits replay, macro replacement/argument, attempt, and durable spans.
    pub fn new(universe: &mut tex_state::Universe<G>) -> Self {
        let semantic = ['r', 'm', 'a', 'd'].map(|ch| {
            TokenWord::pack(Token::Char {
                ch,
                cat: crate::Catcode::Letter,
            })
        });
        let traced = semantic.map(|word| TracedTokenWord::from_parts(word, OriginId::UNKNOWN));

        let durable = universe
            .allocate_token_list(&semantic)
            .expect("mixed-cursor durable list");
        let durable = {
            let context = universe.command_context().expect("command context");
            PackedTokenSpanHandle::durable(context.token_list(durable))
        };

        let mut replay = ReplayLane::default();
        let replay_span = replay
            .push_words(
                traced.map(|spelling| BackedUpToken { spelling }),
                PackedTokenOwnership::Transient,
            )
            .expect("mixed-cursor replay span");

        let mut attempt = crate::attempt::AttemptArena::default();
        let attempt_list = attempt
            .allocate_token_list(traced)
            .expect("mixed-cursor attempt span");

        let mut scratch = crate::execution_scratch::ExecutionScratch::default();
        let matching = scratch
            .begin_macro_match()
            .expect("mixed-cursor macro frame");
        let mut buffer = scratch
            .begin_argument_writer(&matching)
            .expect("mixed-cursor argument buffer");
        for word in traced {
            scratch
                .append_argument_token(
                    &mut buffer,
                    crate::token_collector::ClassifiedToken::from_word(word, None),
                    true,
                )
                .expect("mixed-cursor argument word");
        }
        scratch
            .publish_argument(buffer)
            .expect("mixed-cursor argument range");
        let frame = scratch
            .commit_macro_match(matching)
            .expect("mixed-cursor sealed argument");
        let range = scratch
            .argument_range(frame, 1)
            .expect("mixed-cursor live frame")
            .expect("mixed-cursor first argument");
        Self {
            spans: [
                replay_span.clone(),
                replay_span,
                PackedTokenSpanHandle::AttemptList {
                    list: attempt_list,
                    len: semantic.len() as u32,
                },
                durable,
            ],
            behaviors: [
                TokenBehavior::Ordinary,
                TokenBehavior::Ordinary,
                TokenBehavior::Ordinary,
                TokenBehavior::Ordinary,
            ],
            macro_argument: range,
            // Keep the rollback mark nonzero so restoration proves an exact
            // cursor rather than merely resetting empty spans.
            positions: [1; 5],
            replay,
            attempt,
            scratch,
            semantic,
        }
    }

    /// Runs `rounds * 5` packed accesses and restores the exact opening
    /// scalar cursors after every source has crossed real span ends.
    pub fn run(&mut self, rounds: u32) -> MixedPackedCursorReceipt {
        let opening = self.positions;
        let sources = PackedTokenSources::new(&self.replay, &self.attempt)
            .with_benchmark_macro(&self.semantic);
        let mut checksum = 0_u64;
        let mut retirements = 0_u64;
        for _ in 0..rounds {
            for ((span, _behavior), position) in self
                .spans
                .iter()
                .zip(self.behaviors)
                .zip(&mut self.positions[..4])
            {
                let (word, origin) = sources
                    .benchmark_token_at(span, *position as usize)
                    .expect("mixed packed cursor remains within its span");
                debug_assert_eq!(origin, OriginId::UNKNOWN);
                checksum = checksum.wrapping_add(u64::from(word.raw()));
                *position += 1;
                if *position as usize == span.frame_len() {
                    *position = 0;
                    retirements += 1;
                }
            }
            let position = &mut self.positions[4];
            let word = self
                .scratch
                .admitted_argument_word(self.macro_argument, *position as usize)
                .expect("mixed direct macro cursor remains within its span");
            checksum = checksum.wrapping_add(u64::from(word.token_word().raw()));
            *position += 1;
            if *position == self.macro_argument.len() {
                *position = 0;
                retirements += 1;
            }
        }
        self.positions = opening;
        for ((span, _behavior), position) in self
            .spans
            .iter()
            .zip(self.behaviors)
            .zip(self.positions[..4].iter().copied())
        {
            let _ = sources
                .benchmark_token_at(span, position as usize)
                .expect("rollback restores the exact packed cursor");
        }
        let _ = self
            .scratch
            .admitted_argument_word(self.macro_argument, self.positions[4] as usize)
            .expect("rollback restores the exact direct macro cursor");
        MixedPackedCursorReceipt {
            calls: u64::from(rounds) * 5,
            retirements,
            rollbacks: 1,
            checksum,
        }
    }
}

/// Focused profiling harness for one long sealed macro-argument span.
///
/// The span crosses several fixed execution-scratch chunks. [`Self::run`]
/// exercises only direct absolute range lookup and scalar cursor restoration.
#[cfg(any(test, feature = "profiling"))]
pub struct LongMacroArgumentCursorBenchmark<G> {
    range: crate::execution_scratch::MacroArgumentRange<G>,
    position: u32,
    scratch: crate::execution_scratch::ExecutionScratch<G>,
}

/// Absolute work receipt from one warmed long-argument cursor run.
#[cfg(any(test, feature = "profiling"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LongMacroArgumentCursorReceipt {
    pub calls: u64,
    pub retirements: u64,
    pub rollbacks: u64,
    pub checksum: u64,
}

#[cfg(any(test, feature = "profiling"))]
impl<G> Default for LongMacroArgumentCursorBenchmark<G> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "profiling"))]
impl<G> LongMacroArgumentCursorBenchmark<G> {
    /// Seals one 16,385-word argument, crossing five 4,096-word chunks.
    pub fn new() -> Self {
        const WORDS: u32 = 16_385;
        let mut scratch = crate::execution_scratch::ExecutionScratch::default();
        let matching = scratch
            .begin_macro_match()
            .expect("long-argument macro frame");
        let mut buffer = scratch
            .begin_argument_writer(&matching)
            .expect("long-argument buffer");
        for index in 0..WORDS {
            let semantic = TokenWord::pack(Token::Char {
                ch: char::from(b'a' + (index % 26) as u8),
                cat: crate::Catcode::Letter,
            });
            scratch
                .append_argument_token(
                    &mut buffer,
                    crate::token_collector::ClassifiedToken::from_word(
                        TracedTokenWord::from_parts(semantic, OriginId::UNKNOWN),
                        None,
                    ),
                    true,
                )
                .expect("long-argument word");
        }
        scratch
            .publish_argument(buffer)
            .expect("long-argument range");
        let frame = scratch
            .commit_macro_match(matching)
            .expect("long-argument sealed frame");
        let range = scratch
            .argument_range(frame, 1)
            .expect("long-argument live frame")
            .expect("long-argument first range");
        Self {
            range,
            // A nonzero opening proves exact scalar restoration.
            position: 1,
            scratch,
        }
    }

    /// Performs `calls` bounded indexed reads and restores the opening scalar.
    pub fn run(&mut self, calls: u32) -> LongMacroArgumentCursorReceipt {
        let opening = self.position;
        let mut checksum = 0_u64;
        let mut retirements = 0_u64;
        for _ in 0..calls {
            let word = self
                .scratch
                .admitted_argument_word(self.range, self.position as usize)
                .expect("long macro-argument cursor remains within its span");
            checksum = checksum.wrapping_add(u64::from(word.token_word().raw()));
            self.position += 1;
            if self.position == self.range.len() {
                self.position = 0;
                retirements += 1;
            }
        }
        self.position = opening;
        let _ = self
            .scratch
            .admitted_argument_word(self.range, self.position as usize)
            .expect("rollback restores the exact long-argument cursor");
        LongMacroArgumentCursorReceipt {
            calls: u64::from(calls),
            retirements,
            rollbacks: 1,
            checksum,
        }
    }
}

/// One restored command spelling with its stable packed origin coordinate.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct BackedUpToken {
    pub(crate) spelling: TracedTokenWord,
}

/// Semantic treatment applied while a token level delivers its payload.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TokenBehavior {
    Ordinary,
    /// A TeX recovery insertion that must retire before a scanner backs its
    /// consumed token up for ordinary replay.
    Recovery,
    /// Replacement text associated with the sole activation owner.
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
#[derive(Debug, Eq, Hash, PartialEq)]
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
