//! Dense source and token-list input-level ownership.
//!
//! Immutable macro bodies and stored lists are retained directly by their
//! rows. Insertions and backups occupy the input stack's transient words;
//! arguments occupy reusable invocation regions owned by their macro body.
//! The common frame owns the sole delivery position for either storage.

use tex_state::interner::Symbol;
use tex_state::packed_input::{InputFrameFlags, InputFrameKind};
use tex_state::token::{OriginId, TracedTokenWord};

use super::source::{SourceCursor, SourceNameClass};

/// Stable identity for one live input level.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct InputLevelId(pub(crate) u64);

pub(crate) use tex_state::packed_input::InputFrame as PackedInputFrame;

/// One authoritative input frame and its actual backing owner.
#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct InputLevel<G> {
    pub(crate) frame: PackedInputFrame,
    pub(crate) storage: InputStorage<G>,
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

/// What a token-list row owns of the input stack's words.
#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) enum InputStorage<G> {
    /// Sole owner of variable external source state.
    Source(Box<SourceSlot<G>>),
    /// Owns an individual replay spelling without a shared-vector allocation.
    Word {
        word: TracedTokenWord,
        kind: InputFrameKind,
        flags: InputFrameFlags,
    },
    /// Owns the final span of the shared transient word vector.
    List {
        start: u32,
        ownership: PackedTokenOwnership,
        kind: InputFrameKind,
        flags: InputFrameFlags,
    },
    /// Reads one generation-durable immutable list without materializing it.
    StoredList {
        list: tex_state::TokenListId<G>,
        kind: InputFrameKind,
        flags: InputFrameFlags,
    },
    /// Owns its activation region and its argument spans.
    MacroBody(MacroBodyCursor<G>),
    /// Borrows one argument span of the enclosing macro activation, which
    /// cannot retire while this row lies above it.
    MacroArgument {
        slot: u8,
        region: super::ArgumentRegionId,
    },
}

impl<G> InputLevel<G> {
    #[inline(always)]
    pub(crate) fn kind(&self) -> InputFrameKind {
        match &self.storage {
            InputStorage::Source(_) => InputFrameKind::Source,
            InputStorage::MacroBody(_) => InputFrameKind::Macro,
            InputStorage::MacroArgument { .. } => InputFrameKind::Parameter,
            InputStorage::Word { kind, .. }
            | InputStorage::List { kind, .. }
            | InputStorage::StoredList { kind, .. } => *kind,
        }
    }

    #[inline(always)]
    pub(crate) fn flags(&self) -> InputFrameFlags {
        match &self.storage {
            InputStorage::Word { flags, .. }
            | InputStorage::List { flags, .. }
            | InputStorage::StoredList { flags, .. } => *flags,
            _ => InputFrameFlags::empty(),
        }
    }

    pub(crate) fn identity(&self) -> InputLevelId {
        InputLevelId(self.frame.identity())
    }

    pub(crate) fn position(&self) -> usize {
        self.frame.position() as usize
    }

    pub(crate) fn behavior(&self) -> TokenBehavior {
        match self.kind() {
            InputFrameKind::Parameter => TokenBehavior::Parameter,
            InputFrameKind::AlignmentUTemplate => TokenBehavior::UTemplate,
            InputFrameKind::AlignmentVTemplate | InputFrameKind::OmitTemplate => {
                TokenBehavior::VTemplate
            }
            InputFrameKind::BackedUp => TokenBehavior::BackedUp(
                if self
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
        let flags = self.flags();
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
        let mut flags = self.flags().without(retirement_flags);
        flags = flags.union(match retirement {
            RetirementBehavior::Pop => InputFrameFlags::empty(),
            RetirementBehavior::StopAtEnd => InputFrameFlags::STOP_AT_END,
            RetirementBehavior::RetainExhaustedVTemplate => InputFrameFlags::RETAIN_AT_END,
            RetirementBehavior::AwaitingVTemplateRetirement => InputFrameFlags::RETAIN_AT_END
                .union(InputFrameFlags::AWAITING_V_TEMPLATE_RETIREMENT),
        });
        match &mut self.storage {
            InputStorage::Word { flags: saved, .. }
            | InputStorage::List { flags: saved, .. }
            | InputStorage::StoredList { flags: saved, .. } => *saved = flags,
            _ => unreachable!("only replay lists change retirement treatment"),
        }
    }

    pub(crate) fn trace(&self) -> Option<ReplayTrace> {
        Some(match self.kind() {
            InputFrameKind::Parameter => ReplayTrace::MacroParameter {
                slot: match self.storage {
                    InputStorage::MacroArgument { slot, .. } => slot,
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
            InputFrameKind::Source => return None,
        })
    }
}

impl<G> InputLevel<G> {
    pub(crate) fn source_slot(&self) -> Option<&SourceSlot<G>> {
        match &self.storage {
            InputStorage::Source(slot) => Some(slot),
            _ => None,
        }
    }
    #[cfg(test)]
    pub(crate) fn macro_body(&self) -> Option<&MacroBodyCursor<G>> {
        match &self.storage {
            InputStorage::MacroBody(body) => Some(body),
            _ => None,
        }
    }
    /// Copies the actual frame and admitted checkpoint owners.
    pub(super) fn checkpoint_copy(&self) -> Self {
        Self {
            frame: self.frame,
            storage: match &self.storage {
                InputStorage::Source(slot) => {
                    InputStorage::Source(Box::new(slot.checkpoint_copy()))
                }
                InputStorage::Word { word, kind, flags } => InputStorage::Word {
                    word: *word,
                    kind: *kind,
                    flags: *flags,
                },
                InputStorage::List {
                    start,
                    ownership,
                    kind,
                    flags,
                } => InputStorage::List {
                    start: *start,
                    ownership: *ownership,
                    kind: *kind,
                    flags: *flags,
                },
                InputStorage::StoredList { list, kind, flags } => InputStorage::StoredList {
                    list: list.clone(),
                    kind: *kind,
                    flags: *flags,
                },
                InputStorage::MacroBody(_) | InputStorage::MacroArgument { .. } => {
                    unreachable!("named checkpoints exclude macro input")
                }
            },
        }
    }
    /// Tests a token-list row's loc=null condition; source exhaustion is lexical.
    pub(crate) fn stored_is_exhausted(&self) -> Option<bool> {
        (!matches!(self.storage, InputStorage::Source(_)))
            .then_some(self.frame.position() >= self.frame.limit())
    }
    pub(crate) const fn source_context(&self) -> Option<tex_state::packed_input::SourceContext> {
        self.frame.source_context()
    }
    pub(crate) fn set_source_context(
        &mut self,
        source: Option<tex_state::packed_input::SourceContext>,
    ) {
        self.frame.set_source_context(source);
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
