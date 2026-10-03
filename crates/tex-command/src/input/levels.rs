//! Dense source and token-list input-level ownership.
//!
//! Immutable macro bodies and stored lists are retained directly by their
//! rows. Insertions and backups occupy the input stack's transient words;
//! arguments occupy reusable invocation regions owned by their macro body.
//! The common frame owns the sole delivery position for either storage.

use tex_state::interner::Symbol;
use tex_state::packed_input::{InputFrameFlags, InputFrameKind};
#[cfg(test)]
use tex_state::token::Token;
use tex_state::token::{OriginId, TokenWord, TracedTokenWord};

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

/// Packs a token-list frame reading `len` stack words from `start`.
#[inline]
pub(crate) fn packed_token_frame(
    identity: InputLevelId,
    start: u32,
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
    PackedInputFrame::tokens(
        identity.0,
        start,
        len,
        packed_frame_kind(behavior, trace),
        flags,
    )
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

/// One macro activation: TeX82 §390's `macro` token list.
///
/// Matched argument spans borrow its final pooled region. The immutable
/// definition owner supplies both replacement delivery and context display.
#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct MacroBodyCursor<G> {
    pub(crate) name: Symbol,
    pub(crate) invocation: OriginId,
    /// Sole owner of the invocation's final argument words, if parameterized.
    pub(crate) argument_region: Option<super::ArgumentRegionId>,
    pub(crate) body: tex_state::ResidentMacroBody<G>,
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

impl<G> MacroBodyCursor<G> {
    /// Argument ranges belong to this invocation's final region.
    #[inline(always)]
    pub(crate) fn argument_values<'a>(
        &self,
        arguments: &'a super::ArgumentArena<G>,
    ) -> &'a [super::ArgumentValue] {
        self.argument_region
            .map_or(&[], |region| arguments.values(region))
    }

    /// TeX82 §359's argument `#slot`, if this activation matched that many.
    #[inline(always)]
    pub(crate) fn argument_len(
        &self,
        arguments: &super::ArgumentArena<G>,
        slot: u8,
    ) -> Option<u32> {
        self.argument_values(arguments)
            .get(usize::from(slot).checked_sub(1)?)
            .map(super::ArgumentValue::len)
    }
}

/// A token-backed input row owns its native frame and actual token storage.
#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct ResidentTokenRow<G> {
    pub(crate) frame: PackedInputFrame,
    pub(crate) storage: ResidentTokenStorage<G>,
}

/// What a token-list row owns of the input stack's words.
#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) enum ResidentTokenStorage<G> {
    /// Owns `[frame.start(), frame.start() + frame.limit())`.
    List(PackedTokenOwnership),
    /// Reads one generation-durable immutable list without materializing it.
    StoredList(tex_state::TokenListId<G>),
    /// Owns its activation region and its argument spans.
    MacroBody(MacroBodyCursor<G>),
    /// Borrows one argument span of the enclosing macro activation, which
    /// cannot retire while this row lies above it.
    MacroArgument {
        slot: u8,
        region: super::ArgumentRegionId,
    },
}

impl<G> ResidentTokenRow<G> {
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
            InputFrameKind::Source => unreachable!("resident token frame cannot be a source"),
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

    pub(crate) fn trace(&self) -> ReplayTrace {
        match self.frame.kind() {
            InputFrameKind::Parameter => ReplayTrace::MacroParameter {
                slot: match self.storage {
                    ResidentTokenStorage::MacroArgument { slot, .. } => slot,
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
            _ => None,
        }
    }

    /// Copies frame coordinates and retains any immutable span owner. The
    /// aggregate capture also copies transient words these rows reference.
    pub(super) fn checkpoint_copy(&self) -> Self {
        match self {
            Self::Source(level) => Self::Source(level.checkpoint_copy()),
            Self::Resident(row) => Self::Resident(ResidentTokenRow {
                frame: row.frame,
                storage: match &row.storage {
                    ResidentTokenStorage::List(ownership) => ResidentTokenStorage::List(*ownership),
                    ResidentTokenStorage::StoredList(list) => {
                        ResidentTokenStorage::StoredList(list.clone())
                    }
                    ResidentTokenStorage::MacroBody(_)
                    | ResidentTokenStorage::MacroArgument { .. } => {
                        unreachable!("named checkpoints exclude macro input")
                    }
                },
            }),
        }
    }

    pub(crate) fn stored_common(&self) -> Option<&ResidentTokenRow<G>> {
        match self {
            Self::Resident(row) => Some(row),
            Self::Source(_) => None,
        }
    }

    pub(crate) fn stored_common_mut(&mut self) -> Option<&mut ResidentTokenRow<G>> {
        match self {
            Self::Resident(row) => Some(row),
            Self::Source(_) => None,
        }
    }

    /// Tests a token-list row's `loc=null` condition.
    pub(crate) fn stored_is_exhausted(&self) -> Option<bool> {
        self.stored_common()
            .map(|header| header.frame.position() >= header.frame.limit())
    }

    /// External source context inherited when this semantic input row became
    /// visible. Source owners remain exclusively in `InputStack`; this is the
    /// compact execution fact delivered commands need for checkpoint origin.
    pub(crate) const fn source_context(&self) -> Option<tex_state::packed_input::SourceContext> {
        match self {
            Self::Source(source) => source.frame.source_context(),
            Self::Resident(tokens) => tokens.frame.source_context(),
        }
    }

    pub(crate) fn set_source_context(
        &mut self,
        source: Option<tex_state::packed_input::SourceContext>,
    ) {
        match self {
            Self::Source(level) => level.frame.set_source_context(source),
            Self::Resident(tokens) => tokens.frame.set_source_context(source),
        }
    }
}

/// TeX82 one-word allocator ownership of a token-list row's words.
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
    /// Admits the level's words or immutable span and names its owner.
    fn admit(
        self,
        words: &mut Vec<TracedTokenWord>,
        sources: TokenSources<'_, G>,
    ) -> ResidentTokenStorage<G>;
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
    ) -> ResidentTokenStorage<G> {
        let start = self.start as usize;
        words.extend_from_slice(&sources.pending_insertions[start..]);
        sources.pending_insertions.truncate(start);
        ResidentTokenStorage::List(PackedTokenOwnership::Transient)
    }
}

impl<G> PackedTokenSpanSource<G> for tex_state::TokenListView<G> {
    #[inline]
    fn admit(
        self,
        _: &mut Vec<TracedTokenWord>,
        _: TokenSources<'_, G>,
    ) -> ResidentTokenStorage<G> {
        ResidentTokenStorage::StoredList(self.into_id())
    }
}

impl<G> PackedTokenSpanSource<G> for AttemptTokenListId {
    fn admit(
        self,
        words: &mut Vec<TracedTokenWord>,
        sources: TokenSources<'_, G>,
    ) -> ResidentTokenStorage<G> {
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
        ResidentTokenStorage::List(PackedTokenOwnership::Stored)
    }
}

impl<G> PackedTokenSpanSource<G> for TracedTokenWord {
    #[inline]
    fn admit(
        self,
        words: &mut Vec<TracedTokenWord>,
        _: TokenSources<'_, G>,
    ) -> ResidentTokenStorage<G> {
        words.push(self);
        ResidentTokenStorage::List(PackedTokenOwnership::BackedUp)
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
    ) -> ResidentTokenStorage<G> {
        words.extend(self.tokens);
        ResidentTokenStorage::List(self.ownership)
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
    ) -> ResidentTokenStorage<G> {
        push_untraced_words(words, self.words);
        ResidentTokenStorage::List(PackedTokenOwnership::Stored)
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
    ) -> ResidentTokenStorage<G> {
        words.extend(self.tokens.map(|token| token.spelling));
        ResidentTokenStorage::List(PackedTokenOwnership::BackedUp)
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
    ) -> ResidentTokenStorage<G> {
        words.extend(self.tokens.iter().map(|&token| {
            TracedTokenWord::pack(token, self.origins.next().unwrap_or(OriginId::UNKNOWN))
        }));
        ResidentTokenStorage::List(PackedTokenOwnership::Stored)
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
