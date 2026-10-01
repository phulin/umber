//! Dense source and token-list input-level ownership.

use core::marker::PhantomData;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use tex_state::packed_input::{InputFrameFlags, InputFrameKind};
use tex_state::token::{OriginId, Token, TokenWord, TracedTokenWord};

use super::source::{SourceCursor, SourceNameClass};
use crate::attempt::AttemptTokenListId;

/// Stable identity for one live input level.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct InputLevelId(pub(crate) u64);

pub(crate) use tex_state::packed_input::InputFrame as PackedInputFrame;

#[inline]
fn packed_frame_kind(behavior: &TokenBehavior, trace: &ReplayTrace) -> InputFrameKind {
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
        RetirementBehavior::RetainExhaustedVTemplate => InputFrameFlags::RETAIN_AT_END,
        RetirementBehavior::AwaitingVTemplateRetirement => {
            InputFrameFlags::RETAIN_AT_END.union(InputFrameFlags::AWAITING_V_TEMPLATE_RETIREMENT)
        }
        RetirementBehavior::Pop => InputFrameFlags::empty(),
    });
    PackedInputFrame::tokens(identity.0, len, packed_frame_kind(behavior, trace), flags)
}

/// One future-relevant input level.
///
/// Conditions, caches, scanner policy, and paragraph transitions cannot be
/// represented here. Both character profiles use this same level structure.
#[derive(Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub(crate) enum InputLevel<G> {
    Source(SourceLevel<G>),
    Resident(ResidentTokenRow<G>),
}

/// Specialized resident cursor over a definition arena replacement span.
#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct MacroBodyCursor<G> {
    pub(crate) body: tex_state::ResidentMacroBody<G>,
    pub(crate) arguments: Option<crate::execution_scratch::ArgumentSetId<G>>,
    pub(crate) name: tex_state::interner::Symbol,
    pub(crate) invocation: OriginId,
}

impl<G> MacroBodyCursor<G> {
    pub(crate) fn new(
        body: tex_state::ResidentMacroBody<G>,
        arguments: Option<crate::execution_scratch::ArgumentSetId<G>>,
        name: tex_state::interner::Symbol,
        invocation: OriginId,
    ) -> Self {
        Self {
            body,
            arguments,
            name,
            invocation,
        }
    }
}

/// One registered-source level and its exact delivery identity.
#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct SourceLevel<G> {
    pub(crate) frame: PackedInputFrame,
    /// Sole owner of all variable source state. Boxing keeps the row small;
    /// the resident lexer reaches the owner through one pointer.
    pub(crate) slot: Box<SourceSlot<G>>,
}

impl<G> SourceLevel<G> {
    pub(crate) fn identity(&self) -> InputLevelId {
        InputLevelId(self.frame.identity())
    }

    fn checkpoint_copy(&self) -> Self {
        Self {
            frame: self.frame,
            slot: Box::new(self.slot.checkpoint_copy()),
        }
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
    fn checkpoint_copy(&self) -> Self {
        Self {
            cursor: self.cursor.checkpoint_copy(),
            occupied_buffer_slots: self.occupied_buffer_slots,
            every_eof: self.every_eof.clone(),
            open_depths: self
                .open_depths
                .as_ref()
                .map(SourceOpenDepths::checkpoint_copy),
            name_class: self.name_class,
            retirement: self.retirement,
        }
    }

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

impl SourceOpenDepths {
    fn checkpoint_copy(&self) -> Self {
        Self {
            group_lineages: self.group_lineages.clone(),
            conditional_identities: self.conditional_identities.clone(),
        }
    }
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

/// One authoritative header shared by every token-backed input row.
///
/// Storage ownership remains in the concrete row variant. Everything that
/// selects delivery, rollback, retirement, provenance context, and the next
/// semantic word position lives here exactly once.
#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct TokenRowHeader<G> {
    pub(crate) frame: PackedInputFrame,
    generation: PhantomData<fn() -> G>,
}

#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct ResidentTokenRow<G> {
    pub(crate) header: TokenRowHeader<G>,
    pub(crate) storage: ResidentTokenStorage<G>,
}

#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) enum ResidentTokenStorage<G> {
    Replay {
        replay: ReplayPayloadId<G>,
        cursor: ResidentReplayCursor,
    },
    /// TeX82 §325's `back_input` of one delivered token. The single backed-up
    /// word lives inline in its row, so pushing and retiring the level admits
    /// and releases nothing in the replay lane.
    BackedUpWord(TracedTokenWord),
    Durable(tex_state::TokenListId<G>),
    Attempt(AttemptTokenListId),
    MacroBody(MacroBodyCursor<G>),
    MacroArgument(MacroArgumentCursor<G>),
}

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
}

impl<G> MacroArgumentCursor<G> {
    pub(crate) fn new(
        range: crate::execution_scratch::MacroArgumentRange<G>,
        slot: u8,
        origin_run: u32,
    ) -> Self {
        Self {
            range,
            slot,
            origin_run,
        }
    }

    #[inline(always)]
    pub(crate) fn advance_delivery(
        &mut self,
        position: u32,
        scratch: &crate::execution_scratch::ExecutionScratch<G>,
    ) -> Option<(TokenWord, OriginId)> {
        let absolute = self.range.start().checked_add(position)?;
        if absolute >= self.range.end() {
            return None;
        }
        let (word, origin) =
            scratch.admitted_argument_parts_at_sequential(absolute, &mut self.origin_run)?;
        Some((word, origin))
    }
}

impl<G> TokenRowHeader<G> {
    pub(crate) fn new(
        _behavior: TokenBehavior,
        _retirement: RetirementBehavior,
        _trace: ReplayTrace,
        frame: PackedInputFrame,
    ) -> Self {
        Self {
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

    pub(crate) fn behavior(&self) -> TokenBehavior {
        match self.frame.kind() {
            InputFrameKind::Parameter => TokenBehavior::Parameter,
            InputFrameKind::AlignmentUTemplate => TokenBehavior::UTemplate,
            InputFrameKind::AlignmentVTemplate | InputFrameKind::OmitTemplate => {
                TokenBehavior::VTemplate
            }
            InputFrameKind::BackedUp => TokenBehavior::BackedUp(
                if self
                    .frame
                    .flags()
                    .contains(InputFrameFlags::SUPPRESS_EXPANDABLE_CONTROL_SEQUENCE)
                {
                    BackupTreatment::SuppressExpandableControlSequence
                } else {
                    BackupTreatment::Ordinary
                },
            ),
            InputFrameKind::Recovery => TokenBehavior::Recovery,
            InputFrameKind::Source => unreachable!("token header cannot be a source"),
            InputFrameKind::Inserted
            | InputFrameKind::ExpandedTokenList
            | InputFrameKind::Macro
            | InputFrameKind::OutputRoutine
            | InputFrameKind::EveryPar
            | InputFrameKind::EveryMath
            | InputFrameKind::EveryDisplay
            | InputFrameKind::EveryHBox
            | InputFrameKind::EveryVBox
            | InputFrameKind::EveryJob
            | InputFrameKind::EveryCr
            | InputFrameKind::EveryEof
            | InputFrameKind::Mark
            | InputFrameKind::Write
            | InputFrameKind::UmberReplay => TokenBehavior::Ordinary,
        }
    }

    pub(crate) fn retirement(&self) -> RetirementBehavior {
        let flags = self.frame.flags();
        if flags.contains(InputFrameFlags::AWAITING_V_TEMPLATE_RETIREMENT) {
            RetirementBehavior::AwaitingVTemplateRetirement
        } else if flags.contains(InputFrameFlags::RETAIN_AT_END) {
            RetirementBehavior::RetainExhaustedVTemplate
        } else if flags.contains(InputFrameFlags::STOP_AT_END) {
            RetirementBehavior::StopAtEnd
        } else {
            RetirementBehavior::Pop
        }
    }

    pub(crate) fn set_retirement(&mut self, retirement: RetirementBehavior) {
        let retirement_flags = InputFrameFlags::STOP_AT_END
            .union(InputFrameFlags::RETAIN_AT_END)
            .union(InputFrameFlags::AWAITING_V_TEMPLATE_RETIREMENT);
        let mut flags = self.frame.flags().without(retirement_flags);
        flags = flags.union(match retirement {
            RetirementBehavior::Pop => InputFrameFlags::empty(),
            RetirementBehavior::StopAtEnd => InputFrameFlags::STOP_AT_END,
            RetirementBehavior::RetainExhaustedVTemplate => InputFrameFlags::RETAIN_AT_END,
            RetirementBehavior::AwaitingVTemplateRetirement => InputFrameFlags::RETAIN_AT_END
                .union(InputFrameFlags::AWAITING_V_TEMPLATE_RETIREMENT),
        });
        self.frame.set_flags(flags);
    }
}

impl<G> ResidentTokenRow<G> {
    pub(crate) fn trace(&self) -> ReplayTrace {
        match self.header.frame.kind() {
            InputFrameKind::Parameter => ReplayTrace::MacroParameter {
                slot: match &self.storage {
                    ResidentTokenStorage::MacroArgument(argument) => argument.slot,
                    _ => 0,
                },
            },
            InputFrameKind::AlignmentUTemplate => ReplayTrace::UTemplate,
            InputFrameKind::AlignmentVTemplate => ReplayTrace::VTemplate,
            InputFrameKind::OmitTemplate => ReplayTrace::OmitTemplate,
            InputFrameKind::BackedUp => ReplayTrace::BackedUp,
            InputFrameKind::Inserted | InputFrameKind::Recovery => ReplayTrace::Inserted,
            InputFrameKind::ExpandedTokenList => {
                ReplayTrace::Transient(TransientReplayReason::ExpandedTokenList)
            }
            InputFrameKind::Macro => ReplayTrace::MacroReplacement,
            InputFrameKind::OutputRoutine => ReplayTrace::Stored(StoredReplayReason::OutputRoutine),
            InputFrameKind::EveryPar => ReplayTrace::Stored(StoredReplayReason::EveryPar),
            InputFrameKind::EveryMath => ReplayTrace::Stored(StoredReplayReason::EveryMath),
            InputFrameKind::EveryDisplay => ReplayTrace::Stored(StoredReplayReason::EveryDisplay),
            InputFrameKind::EveryHBox => ReplayTrace::Stored(StoredReplayReason::EveryHBox),
            InputFrameKind::EveryVBox => ReplayTrace::Stored(StoredReplayReason::EveryVBox),
            InputFrameKind::EveryJob => ReplayTrace::Stored(StoredReplayReason::EveryJob),
            InputFrameKind::EveryCr => ReplayTrace::Stored(StoredReplayReason::EveryCr),
            InputFrameKind::EveryEof => ReplayTrace::Stored(StoredReplayReason::EveryEof),
            InputFrameKind::Mark => ReplayTrace::Stored(StoredReplayReason::Mark),
            InputFrameKind::Write => ReplayTrace::Stored(StoredReplayReason::Write),
            InputFrameKind::UmberReplay => ReplayTrace::Stored(StoredReplayReason::Discretionary),
            InputFrameKind::Source => unreachable!("resident row cannot be a source"),
        }
    }
}

impl<G> InputLevel<G> {
    pub(crate) fn trace(&self) -> Option<ReplayTrace> {
        match self {
            Self::Resident(row) => Some(row.trace()),
            Self::Source(_) => None,
        }
    }

    #[cfg(test)] // Macro and input lifecycle tests inspect the resident row.
    pub(crate) fn macro_body(&self) -> Option<&MacroBodyCursor<G>> {
        match self {
            Self::Resident(ResidentTokenRow {
                storage: ResidentTokenStorage::MacroBody(body),
                ..
            }) => Some(body),
            Self::Source(_)
            | Self::Resident(ResidentTokenRow {
                storage:
                    ResidentTokenStorage::Replay { .. }
                    | ResidentTokenStorage::BackedUpWord(_)
                    | ResidentTokenStorage::Durable(_)
                    | ResidentTokenStorage::Attempt(_)
                    | ResidentTokenStorage::MacroArgument(_),
                ..
            }) => None,
        }
    }

    /// Copies one row of a quiescent stack for a checkpoint capture.
    ///
    /// # Panics
    ///
    /// Panics on a macro, argument, or attempt row. Checkpoint quiescence
    /// excludes all three, since their storage does not outlive the operation
    /// that admitted them.
    pub(super) fn checkpoint_copy(&self) -> Self {
        match self {
            Self::Source(level) => Self::Source(level.checkpoint_copy()),
            Self::Resident(row) => Self::Resident(ResidentTokenRow {
                header: TokenRowHeader {
                    frame: row.header.frame,
                    generation: PhantomData,
                },
                storage: match &row.storage {
                    ResidentTokenStorage::Replay { replay, cursor } => {
                        ResidentTokenStorage::Replay {
                            replay: *replay,
                            cursor: *cursor,
                        }
                    }
                    ResidentTokenStorage::BackedUpWord(word) => {
                        ResidentTokenStorage::BackedUpWord(*word)
                    }
                    ResidentTokenStorage::Durable(list) => {
                        ResidentTokenStorage::Durable(list.clone())
                    }
                    ResidentTokenStorage::Attempt(_)
                    | ResidentTokenStorage::MacroBody(_)
                    | ResidentTokenStorage::MacroArgument(_) => {
                        unreachable!("checkpoint quiescence excludes operation-owned input rows")
                    }
                },
            }),
        }
    }

    pub(crate) fn stored_common(&self) -> Option<&TokenRowHeader<G>> {
        match self {
            Self::Resident(row) => Some(&row.header),
            Self::Source(_) => None,
        }
    }

    pub(crate) fn stored_common_mut(&mut self) -> Option<&mut TokenRowHeader<G>> {
        match self {
            Self::Resident(row) => Some(&mut row.header),
            Self::Source(_) => None,
        }
    }

    pub(crate) fn stored_position(&self) -> Option<usize> {
        self.stored_common().map(TokenRowHeader::position)
    }

    /// Tests a generic stored-token row's `loc=null` condition without
    /// reclassifying or loading its already-admitted storage domain.
    pub(crate) fn stored_is_exhausted(&self) -> Option<bool> {
        self.stored_common()
            .map(|header| header.frame.position() >= header.frame.limit())
    }

    #[cold]
    pub(crate) fn stored_span_cold(&self) -> Option<PackedTokenSpanHandle<G>> {
        match self {
            Self::Resident(ResidentTokenRow {
                header,
                storage: ResidentTokenStorage::Replay { replay, .. },
            }) => Some(PackedTokenSpanHandle::Replay {
                replay: *replay,
                len: header.frame.limit(),
            }),
            Self::Resident(ResidentTokenRow {
                header,
                storage: ResidentTokenStorage::Durable(list),
            }) => Some(PackedTokenSpanHandle::DurableList {
                list: list.clone(),
                len: header.frame.limit(),
            }),
            Self::Resident(ResidentTokenRow {
                header,
                storage: ResidentTokenStorage::Attempt(list),
            }) => Some(PackedTokenSpanHandle::AttemptList {
                list: *list,
                len: header.frame.limit(),
            }),
            Self::Resident(ResidentTokenRow {
                storage: ResidentTokenStorage::BackedUpWord(word),
                ..
            }) => Some(PackedTokenSpanHandle::Word(*word)),
            Self::Source(_)
            | Self::Resident(ResidentTokenRow {
                storage: ResidentTokenStorage::MacroBody(_) | ResidentTokenStorage::MacroArgument(_),
                ..
            }) => None,
        }
    }

    #[cold]
    pub(crate) fn stored_indexed_token_at_cold(
        &self,
        sources: PackedTokenSources<'_, G>,
        _state: &tex_state::CommandContext<'_, G>,
    ) -> Option<PackedTokenAt> {
        let span = self.stored_span_cold()?;
        sources.indexed_token_at_cold(&span, self.stored_position()?)
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
    /// One inline backed-up word.
    Word(TracedTokenWord),
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
            Self::Word(word) => Self::Word(*word),
        }
    }
}

/// Borrowed storage boundary for every immutable stored-token source.
pub(crate) struct PackedTokenSources<'a, G> {
    replay: &'a ReplayLane<G>,
    attempt: &'a crate::attempt::AttemptArena<G>,
}

impl<'a, G> PackedTokenSources<'a, G> {
    pub(crate) const fn new(
        replay: &'a ReplayLane<G>,
        attempt: &'a crate::attempt::AttemptArena<G>,
    ) -> Self {
        Self {
            replay,
            attempt,
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
            PackedTokenSpanHandle::Word(word) => {
                (index == 0).then(|| (word.token_word(), word.origin()))
            }
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
            Self::Resident(tokens) => tokens.header.frame.source_context(),
        }
    }

    pub(crate) fn set_source_context(
        &mut self,
        source: Option<tex_state::packed_input::SourceContext>,
    ) {
        match self {
            Self::Source(level) => level.frame.set_source_context(source),
            Self::Resident(tokens) => tokens.header.frame.set_source_context(source),
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

    const fn empty() -> Self {
        Self::EMPTY
    }
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
            return Some(ResidentReplayCursor::empty());
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
        debug_assert_ne!(cursor.remaining, 0);
        if cursor.offset == cursor.segment_end {
            self.refresh_resident_cursor(
                cursor,
                #[cfg(test)]
                segment_inspections,
            )?;
        }
        let segment = self.active.get(cursor.segment as usize)?;
        let value = segment.storage.values.get(usize::from(cursor.offset))?;
        cursor.offset = cursor.offset.checked_add(1)?;
        cursor.remaining -= 1;
        Some(value)
    }

    /// Crosses the uncommon exhausted-span or fixed-segment boundary.
    #[cold]
    #[inline(never)]
    fn refresh_resident_cursor(
        &self,
        cursor: &mut ResidentReplayCursor,
        #[cfg(test)] segment_inspections: &mut u64,
    ) -> Option<()> {
        debug_assert_eq!(cursor.offset, cursor.segment_end);
        debug_assert_ne!(cursor.remaining, 0);
        cursor.segment = cursor.segment.checked_add(1)?;
        cursor.offset = 0;
        let segment = self.active.get(cursor.segment as usize)?;
        cursor.segment_end = segment.used;
        #[cfg(test)]
        {
            *segment_inspections = segment_inspections.saturating_add(1);
        }
        Some(())
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
        if cursor.run == ResidentReplayRun::Prefix && cursor.remaining == 0 {
            self.refresh_resident_run(
                replay,
                cursor,
                #[cfg(test)]
                segment_inspections,
                #[cfg(test)]
                run_transitions,
            )?;
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

    /// Crosses the uncommon prefix-to-body boundary of a resident replay.
    #[cold]
    #[inline(never)]
    fn refresh_resident_run(
        &self,
        replay: ReplayPayloadId<G>,
        cursor: &mut ResidentReplayCursor,
        #[cfg(test)] segment_inspections: &mut u64,
        #[cfg(test)] run_transitions: &mut u64,
    ) -> Option<()> {
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
        Some(())
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
        self.reclaim_released_suffix()
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

    #[cfg(all(test, feature = "profiling"))]
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

#[cfg(test)] // The direct packed-span admission test uses this slice adapter.
pub(crate) struct StoredReplaySeed<'a, I> {
    tokens: &'a [Token],
    origins: I,
}
#[cfg(test)]
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
    #[cfg(test)]
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
            Self::Word(_) => 1,
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
