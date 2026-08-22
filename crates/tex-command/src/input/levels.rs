//! Dense source and token-list input-level ownership.
#![allow(dead_code)] // consumed by the ordered raw-delivery implementation issues

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
// Boxing the packed chunk would add a separate allocation and owner to the
// canonical live representation this cutover is specifically designed to
// avoid. Compact coordinate-only variants are intentionally smaller.
#[allow(clippy::large_enum_variant)]
pub(crate) enum TokenPayload<G> {
    /// Chunk-owned packed words used by canonical source-adjacent replay,
    /// hooks, templates, insertions, and backup. The sparse roots are owned
    /// once by the chunk rather than by each input frame or delivered word.
    Packed(PackedTokenChunk),
    /// Replacement replay borrowed from one command-admitted macro chunk.
    MacroReplacement {
        definition: DefinitionId<G>,
        len: u32,
    },
    /// One already materialized macro argument, replayed literally by range.
    Argument { list: AttemptTokenListId, len: u32 },
}

impl<G> Clone for TokenPayload<G> {
    fn clone(&self) -> Self {
        match self {
            Self::Packed(chunk) => Self::Packed(chunk.clone()),
            Self::MacroReplacement { definition, len } => Self::MacroReplacement {
                definition: *definition,
                len: *len,
            },
            Self::Argument { list, len } => Self::Argument {
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
            every_eof: self.every_eof,
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

/// One packed token chunk and the cold source coordinates needed only when a
/// backed-up delivery is rendered. Ordinary delivery indexes the packed word
/// slice directly and does not clone this owner.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct PackedTokenChunk {
    words: Vec<TracedTokenWord>,
    /// Position-aligned only for backed-up physical-source tokens. Ordinary
    /// generated/stored runs canonically leave this empty: absence already
    /// denotes `None` and must not allocate a redundant per-position vector.
    source_provenance: Vec<Option<SourceProvenance>>,
    ownership: PackedTokenOwnership,
}

/// TeX82 one-word allocator ownership carried independently of Umber's
/// uniform packed host representation.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
enum PackedTokenOwnership {
    /// Replaying an immutable token list adds only TeX's list-stack reference.
    #[default]
    Stored,
    /// Inserted/generated scanner words own freshly allocated one-word cells.
    Transient,
    /// `back_input` owns freshly allocated cells plus source replay metadata.
    BackedUp,
}

impl PackedTokenChunk {
    fn from_stored(tokens: &[Token], origins: impl IntoIterator<Item = OriginId>) -> Self {
        let mut origins = origins.into_iter();
        let words = tokens
            .iter()
            .copied()
            .map(|token| TracedTokenWord::pack(token, origins.next().unwrap_or(OriginId::UNKNOWN)));
        Self {
            words: words.collect(),
            source_provenance: Vec::new(),
            ownership: PackedTokenOwnership::Stored,
        }
    }

    fn from_durable(words: &[tex_state::token::TokenWord]) -> Self {
        Self {
            words: words
                .iter()
                .copied()
                .map(|word| TracedTokenWord::from_parts(word, OriginId::UNKNOWN))
                .collect(),
            source_provenance: Vec::new(),
            ownership: PackedTokenOwnership::Stored,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.words.len()
    }

    pub(crate) fn get(&self, index: usize) -> Option<(TracedTokenWord, Option<SourceProvenance>)> {
        Some((
            *self.words.get(index)?,
            self.source_provenance.get(index).copied().flatten(),
        ))
    }

    pub(crate) fn word(&self, index: usize) -> Option<TracedTokenWord> {
        self.words.get(index).copied()
    }

    fn backed_up_token(&self, index: usize) -> Option<BackedUpToken> {
        if self.ownership != PackedTokenOwnership::BackedUp {
            return None;
        }
        Some(BackedUpToken {
            spelling: *self.words.get(index)?,
            source_provenance: self.source_provenance.get(index).copied().flatten(),
        })
    }

    pub(crate) fn source_provenance(&self) -> &[Option<SourceProvenance>] {
        debug_assert_eq!(self.ownership, PackedTokenOwnership::BackedUp);
        debug_assert_eq!(self.source_provenance.len(), self.words.len());
        &self.source_provenance
    }

    pub(crate) const fn is_backed_up(&self) -> bool {
        matches!(self.ownership, PackedTokenOwnership::BackedUp)
    }
}

impl<G> TokenPayload<G> {
    #[cfg(test)]
    #[allow(non_snake_case)]
    pub(crate) fn Transient(buffer: SharedTokenBuffer) -> Self {
        Self::transient(buffer.words().iter().copied())
    }

    pub(crate) fn stored(tokens: &[Token], origins: impl IntoIterator<Item = OriginId>) -> Self {
        Self::Packed(PackedTokenChunk::from_stored(tokens, origins))
    }

    pub(crate) fn durable(words: &[tex_state::token::TokenWord]) -> Self {
        Self::Packed(PackedTokenChunk::from_durable(words))
    }

    pub(crate) fn frame_len(&self) -> usize {
        match self {
            Self::Packed(chunk) => chunk.len(),
            Self::MacroReplacement { len, .. } => *len as usize,
            Self::Argument { len, .. } => *len as usize,
        }
    }

    /// Packs one bounded insertion or scanner result directly into its sole
    /// live chunk representation.
    pub(crate) fn transient(tokens: impl IntoIterator<Item = TracedTokenWord>) -> Self {
        Self::Packed(PackedTokenChunk {
            words: tokens.into_iter().collect(),
            source_provenance: Vec::new(),
            ownership: PackedTokenOwnership::Transient,
        })
    }

    /// Packs generated tokens that all carry one structural origin without
    /// forming a temporary strong owner for every position.
    pub(crate) fn transient_with_shared_origin(
        tokens: impl IntoIterator<Item = Token>,
        origin: OriginId,
    ) -> Self {
        Self::Packed(PackedTokenChunk {
            words: tokens
                .into_iter()
                .map(|token| TracedTokenWord::pack(token, origin))
                .collect(),
            source_provenance: Vec::new(),
            ownership: PackedTokenOwnership::Transient,
        })
    }

    /// Packs commands restored by `back_input` into the canonical chunk.
    pub(crate) fn backed_up(tokens: impl IntoIterator<Item = BackedUpToken>) -> Self {
        let mut words = Vec::new();
        let mut source_provenance = Vec::new();
        for token in tokens {
            words.push(token.spelling);
            source_provenance.push(token.source_provenance);
        }
        Self::Packed(PackedTokenChunk {
            words,
            source_provenance,
            ownership: PackedTokenOwnership::BackedUp,
        })
    }

    pub(crate) fn transient_words(&self) -> Option<&[TracedTokenWord]> {
        match self {
            Self::Packed(chunk) if chunk.ownership == PackedTokenOwnership::Transient => {
                Some(&chunk.words)
            }
            _ => None,
        }
    }

    pub(crate) fn transient_len(&self) -> Option<usize> {
        match self {
            Self::Packed(chunk) if chunk.ownership == PackedTokenOwnership::Transient => {
                Some(chunk.len())
            }
            _ => None,
        }
    }

    pub(crate) fn backed_up_len(&self) -> Option<usize> {
        match self {
            Self::Packed(chunk) if chunk.ownership == PackedTokenOwnership::BackedUp => {
                Some(chunk.len())
            }
            _ => None,
        }
    }

    pub(crate) fn backed_up_get(&self, index: usize) -> Option<BackedUpToken> {
        match self {
            Self::Packed(chunk) => chunk.backed_up_token(index),
            _ => None,
        }
    }

    pub(crate) fn is_backed_up(&self) -> bool {
        matches!(self, Self::Packed(chunk) if chunk.ownership == PackedTokenOwnership::BackedUp)
    }

    /// Prepends e-TeX aftergroup tokens, promoting inline storage when the
    /// resulting backed-up level contains multiple commands.
    pub(crate) fn prepend_backed_up(
        &mut self,
        prefix: impl IntoIterator<Item = BackedUpToken>,
    ) -> Option<()> {
        let mut prefix = prefix.into_iter().collect::<Vec<_>>();
        match self {
            Self::Packed(chunk) if chunk.ownership == PackedTokenOwnership::BackedUp => {
                let mut words = Vec::new();
                let mut provenance = Vec::new();
                for token in prefix.drain(..) {
                    words.push(token.spelling);
                    provenance.push(token.source_provenance);
                }
                words.append(&mut chunk.words);
                provenance.append(&mut chunk.source_provenance);
                chunk.words = words;
                chunk.source_provenance = provenance;
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
            Self::Packed(chunk) if chunk.ownership == PackedTokenOwnership::BackedUp => {
                for provenance in chunk.source_provenance.iter_mut().flatten() {
                    provenance.rehome(source, byte_delta)?;
                }
                Some(())
            }
            _ => None,
        }
    }

    pub(crate) fn adopt_matching_origins(&mut self, live: &Self) -> Option<()> {
        if let (Self::Packed(recorded), Self::Packed(live)) = (&*self, live) {
            if recorded.words.len() != live.words.len()
                || recorded
                    .words
                    .iter()
                    .zip(&live.words)
                    .any(|(recorded, live)| recorded.token() != live.token())
                || recorded.source_provenance != live.source_provenance
                || recorded.ownership != live.ownership
            {
                return None;
            }
            *self = Self::Packed(live.clone());
            return Some(());
        }
        None
    }
}

/// Test-fixture staging for callers that deliberately exercise the private
/// input boundary. It owns no shared runtime authority and is converted to a
/// packed chunk before a level exists.
#[cfg(test)]
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct SharedTokenBuffer(Vec<TracedTokenWord>);

#[cfg(test)]
impl SharedTokenBuffer {
    pub(crate) fn new(tokens: impl AsRef<[TracedTokenWord]>) -> Self {
        Self(tokens.as_ref().to_vec())
    }

    pub(crate) fn words(&self) -> &[TracedTokenWord] {
        &self.0
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
