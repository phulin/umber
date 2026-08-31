//! Future-relevant state and discardable scratch allocation ownership.

use std::ops::{Deref, DerefMut};

use tex_state::CommandContext;
use tex_state::token::TracedTokenWord;
use tex_state::{GroupFrame, GroupKind, StateError};

use crate::conditionals::ConditionStack;
use crate::input::InputState;
#[cfg(test)]
use crate::input::{CompactSourceStepQueries, CompactSourceTokenizationStep};
use crate::input::{
    InputLevel, InputLevelId, PhysicalLine, RegisteredSource, RegisteredSourceKind,
    SourceCharacter, SourceCursor, SourceNameClass, SourceRegistration, SourceRegistrationError,
    SourceTokenizationStep,
};
use crate::input::{
    PackedTokenSpanHandle, ReplayTrace, RetirementBehavior, StoredReplayReason, TokenBehavior,
};
use crate::macro_call::ParameterState;
use crate::processor::{
    AlignmentCellDelimiter, AlignmentDeliveryState, AlignmentIdentity, AlignmentLifecycleError,
    AlignmentRequest, AlignmentRequestResult, CELL_ALIGN_STATE, ExpansionState,
    PreparedAlignmentCellTemplates, ScannerState,
};
use crate::profile::{
    CommandEngineSemantics, CommandProfile, CommandProfileBoundary, CommandProfileFingerprint,
    CommandProfileMismatch,
};

mod attempt_transition;
mod executor_publication;
mod projection;

#[cfg(test)]
thread_local! {
    static RETIREMENT_TOP_SOURCE_CHECKS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn retirement_top_source_checks() -> u64 {
    RETIREMENT_TOP_SOURCE_CHECKS.with(std::cell::Cell::get)
}

/// Complete future-relevant state owned by the command machine.
///
/// This is the command half of an executor savepoint. It contains semantic
/// and rollback-coupled provenance state only: host capabilities, aggregate
/// engine state, call-local accumulators, and discardable accelerations are
/// deliberately absent.
#[derive(Debug)]
#[doc(hidden)]
pub struct CommandStateRoots<G> {
    /// Demand latch for fixed-size semantic root publication. The root itself
    /// is read from bounded logical-stack projections at named quiescent
    /// boundaries; no input payload or timeline row is traversed.
    pub(crate) reachable_state_identity_enabled: bool,
    /// Canonical compiled implementation executing the format's command
    /// profile. Unlike the profile, this is job configuration and is not part
    /// of portable format identity.
    pub(crate) engine_semantics: CommandEngineSemantics,
    pub(crate) input: InputState<G>,
    pub(crate) parameters: ParameterState<G>,
    pub(crate) scanner: ScannerState,
    pub(crate) conditions: ConditionStack,
    pub(crate) alignment: AlignmentDeliveryState<G>,
    pub(crate) expansion: ExpansionState,
    pub(crate) transient: TransientState,
    /// Executor-owned replay completion fences, each named by the exact input
    /// level whose retirement must publish it. TeX82 §§325/390 transfer the
    /// fence from a depleted stored level to the newer backup or macro body
    /// they install, so unrelated resident commands never poll readiness.
    pub(crate) replay_completions: Vec<ReplayCompletionFence>,
    /// Semantic diagnostics committed by command processing but rendered by
    /// the executor's World-facing diagnostic boundary.
    ///
    /// This queue is unconditional command state, not observation state.
    /// Consequently an unobserved episode has identical semantics, while the
    /// ordinary command snapshot makes a failed aggregate operation restore
    /// the queue together with the input transition that produced it.
    pub(crate) semantic_diagnostics: Vec<CommandSemanticDiagnostic>,
    /// Most recent direct source spelling, retained only for typed diagnostic
    /// attribution. It never affects command semantics or rendered context.
    pub(crate) last_diagnostic_location: Option<crate::SourceLocation>,
    /// TeX82 §§280--282 `insert_token` payloads paired with the exact state
    /// save level that owns them. Frames and words are generation-branded by
    /// this aggregate root; no payload registry or per-value owner exists.
    pub(crate) group_payloads: crate::timeline::LogicalStack<CommandGroupPayload<G>>,
    /// Generation-owned append lane addressed by compact group-local spans.
    pub(crate) aftergroup_payloads: crate::timeline::LogicalStack<CommandPayload<G>>,
    /// TeX82 §1269's single pending token. The traced spelling remains in the
    /// same command root as input and group payloads, so rollback cannot leave
    /// an uncheckpointed side value behind.
    pub(crate) afterassignment: Option<CommandPayload<G>>,
    /// TeX82 §527's rollback-coupled `name_in_progress` recursion guard.
    pub(crate) name_in_progress: bool,
    /// A fully scanned `\input` filename waiting for immutable host
    /// acquisition. Retrying the opener consumes this value instead of
    /// delivering or scanning its operand again.
    pub(crate) pending_input_open: Option<crate::ScannedFileName>,
    /// Named token-list levels installed since the executor last drained
    /// them, in push order.
    ///
    /// This is publication-owned but unconditional: every step that opens an
    /// episode drains it, so it cannot accumulate across executor episodes.
    /// It deliberately carries no "am I observed" flag -- that would be
    /// observation state living inside semantic state, which
    /// `absent_observer_has_no_delivery_or_snapshot_effect` and
    /// `math_episode_observation_does_not_change_frozen_command_state` exist
    /// to forbid, and which they caught when `umber2-johp.310` first tried
    /// it.
    ///
    /// tex.web installs these inside `begin_token_list`, where its trace and
    /// observer see them; Umber's executor asks command state to install them
    /// after the borrowed command-processor episode has ended, so the record
    /// waits here until the same operation publishes its trace and other
    /// committed observations.
    pub(crate) named_token_list_pushes:
        Vec<(InputLevelId, StoredReplayReason, tex_state::TokenListId<G>)>,
}

/// Complete future-relevant state owned by the command machine.
///
/// Named checkpoints retain explicitly forked immutable aggregate roots. The
/// live root stays exclusively owned, so ordinary mutation is a direct borrow
/// with no ownership admission or copy-on-write branch. Checkpoint owners use
/// private non-atomic sharing because they remain thread-confined.
#[derive(Debug)]
pub struct CommandState<G> {
    pub(crate) roots: CommandStateRoots<G>,
    pub(crate) timeline: crate::snapshot::CommandTimeline<G>,
    /// Runtime-only TeX82 stack maxima. Snapshot roots deliberately omit
    /// these scalars so high-water marks survive rollback without becoming
    /// command semantics or checkpoint identity.
    pub(crate) stack_usage: CommandStackUsage,
    /// Retained width of TeX82 §31's bottom terminal buffer. This is live
    /// session accounting used to compose later nested buffer high waters.
    pub(crate) terminal_buffer_slots: usize,
    /// Runtime-only source-owner incarnation allocator. Unlike semantic input
    /// identities, this counter is never rolled back, so an ordered source
    /// inverse cannot name a later occupant of the same physical stack row.
    /// Storage for scanner, expansion, and retry coordinates in the current
    /// operation. Checkpoints retain its bounded mark, never its payload.
    pub(crate) attempt: crate::CommandAttempt<G>,
    /// Current-generation reusable packed execution lanes. Macro activations
    /// and typed operation continuations retain only private
    /// generation-branded frame indices into this owner.
    pub(crate) scratch: crate::execution_scratch::ExecutionScratch<G>,
    /// One direct-operation child scope. It stays installed across an
    /// in-process resource suspension and is consumed only by commit or
    /// rollback; named checkpoints require this field to be empty.
    pub(crate) active_attempt_operation: Option<crate::CommandAttemptMark>,
    /// Assertion-bearing proof that ordinary input bypasses out-parameter
    /// interception. Shipping builds contain neither the counters nor updates.
    #[cfg(test)]
    pub(crate) raw_delivery_path_counters: RawDeliveryPathCounters,
    /// Assertion-bearing proof for the shared resident token collector. These
    /// counters are operational test/profiling evidence, never semantic state.
    #[cfg(test)]
    pub(crate) token_collector_path_counters: TokenCollectorPathCounters,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RawDeliveryPathCounters {
    pub(crate) source_direct: u64,
    pub(crate) stored_direct: u64,
    pub(crate) macro_argument_direct: u64,
    pub(crate) out_parameter_interceptions: u64,
    pub(crate) resident_transitions: u64,
    pub(crate) intermediate_result_redispatches: u64,
    pub(crate) whole_input_frame_copies: u64,
    pub(crate) resident_ordinary_retirements: u64,
    pub(crate) exhaustion_status_relays: u64,
    pub(crate) retirement_top_lookups: u64,
    pub(crate) retirement_owner_validations: u64,
    pub(crate) retirement_whole_token_copies: u64,
    pub(crate) cold_source_retirements: u64,
    pub(crate) conservation_retirements: u64,
    pub(crate) replay_completion_retirement_checks: u64,
    pub(crate) replay_completion_claims: u64,
    pub(crate) replay_completion_transfers: u64,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TokenCollectorPathCounters {
    pub(crate) collectors_started: u64,
    pub(crate) raw_classifications: u64,
    pub(crate) collector_appends: u64,
    pub(crate) state_updates: u64,
    pub(crate) phase_transitions: u64,
    pub(crate) duplicate_phase_dispatches: u64,
    pub(crate) fact_rescans: u64,
    pub(crate) settlements: u64,
    pub(crate) whole_token_list_copies: u64,
    pub(crate) whole_command_copies: u64,
    pub(crate) whole_frame_copies: u64,
}

/// Compact coordinate for TeX82's live §310 input display.
///
/// It is valid only while both the command timeline owner and authoritative
/// input-stack revision remain unchanged. It owns no strings, rows, or
/// backing and therefore cannot prolong any input lifetime.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DiagnosticContextCoordinate {
    owner: u64,
    input_revision: u64,
    scalar_revision: u64,
}

/// A diagnostic coordinate no longer names the live input projection it was
/// captured from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StaleDiagnosticContext;

impl<G> Deref for CommandState<G> {
    type Target = CommandStateRoots<G>;

    fn deref(&self) -> &Self::Target {
        &self.roots
    }
}

impl<G> DerefMut for CommandState<G> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.roots
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PendingFileEnquiry {
    pub(crate) request: crate::FileEnquiryRequest,
    pub(crate) offset: i32,
    pub(crate) length: i32,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PendingExpansion<G> {
    pub(crate) command: crate::CurrentCommand<G>,
    pub(crate) resume: PendingExpansionResume,
    pub(crate) child:
        Option<crate::execution_scratch::ChildContinuation<G, PendingExpansionChildDestination>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PendingExpansionChildDestination {
    Dispatch,
}

/// Exact operand/result destination retained by one suspended expansion.
///
/// File-enquiry requests are deliberately part of the move-only expansion
/// frame rather than a command-state mailbox. A nested enquiry can therefore
/// resume its own request before its caller continues scanning its operand.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum PendingExpansionResume {
    Dispatch,
    CsName {
        name: String,
    },
    IfCsName {
        condition: crate::processor::status::ConditionId,
        inverted: bool,
        name: String,
    },
    Conditional {
        condition: crate::processor::status::ConditionId,
        inverted: bool,
        kind: crate::conditionals::ConditionalKind,
        phase: crate::conditionals::PendingConditionalScanPhase,
    },
    Number {
        roman: bool,
    },
    PdfFontSize,
    PdfMarginKern {
        primitive: tex_state::meaning::ExpandablePrimitive,
    },
    The,
    FontName,
    MarkClass {
        primitive: tex_state::meaning::ExpandablePrimitive,
    },
    PdfInsertHeight,
    PdfUniformDeviate,
    PdfXImageObject,
    PdfXImageCoordinate {
        object: u32,
    },
    PdfXFormName,
    PdfPageRef,
    PdfLastMatch,
    PdfMatchOptions {
        case_insensitive: bool,
        subcount: u32,
        phase: u8,
    },
    PdfColorStackInitOptions {
        restore_at_page_start: bool,
        phase: u8,
    },
    PdfFileDumpOptions {
        offset: i32,
        length: i32,
        phase: u8,
    },
    PdfMdFiveSumFile,
    PdfMatchPattern {
        case_insensitive: bool,
        subcount: u32,
    },
    PdfMatchHaystack {
        case_insensitive: bool,
        subcount: u32,
        pattern: crate::attempt::AttemptTokenListId,
    },
    PdfColorStackInitText {
        restore_at_page_start: bool,
        mode: tex_state::PdfColorStackMode,
    },
    PdfFileDumpText {
        offset: i32,
        length: i32,
    },
    PdfFileDump(PendingFileEnquiry),
    PdfFileSize(PendingFileEnquiry),
    PdfFileModificationDate(PendingFileEnquiry),
    PdfMdFiveSumText {
        file: bool,
    },
    PdfMdFiveSum(PendingFileEnquiry),
}

impl<G> PendingExpansion<G> {
    pub(crate) fn take_child(&mut self) -> Option<crate::execution_scratch::ScannerFrameKey<G>> {
        self.child.take().map(|child| child.restore().0)
    }
}

impl<G> Default for CommandStateRoots<G> {
    fn default() -> Self {
        Self {
            reachable_state_identity_enabled: false,
            engine_semantics: CommandEngineSemantics::default(),
            input: InputState::default(),
            parameters: ParameterState::default(),
            scanner: ScannerState::default(),
            conditions: ConditionStack::default(),
            alignment: AlignmentDeliveryState::default(),
            expansion: ExpansionState::default(),
            transient: TransientState::default(),
            replay_completions: Vec::new(),
            semantic_diagnostics: Vec::new(),
            last_diagnostic_location: None,
            group_payloads: crate::timeline::LogicalStack::default(),
            aftergroup_payloads: crate::timeline::LogicalStack::default(),
            afterassignment: None,
            name_in_progress: false,
            pending_input_open: None,
            named_token_list_pushes: Vec::new(),
        }
    }
}

impl<G> Default for CommandState<G> {
    fn default() -> Self {
        Self {
            roots: CommandStateRoots::default(),
            timeline: crate::snapshot::CommandTimeline::default(),
            stack_usage: CommandStackUsage::default(),
            terminal_buffer_slots: 0,
            attempt: crate::CommandAttempt::default(),
            scratch: crate::execution_scratch::ExecutionScratch::default(),
            active_attempt_operation: None,
            #[cfg(test)]
            raw_delivery_path_counters: RawDeliveryPathCounters::default(),
            #[cfg(test)]
            token_collector_path_counters: TokenCollectorPathCounters::default(),
        }
    }
}

/// TeX82 §§31/321/374/390 command-owned stack maxima for §1334.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CommandStackUsage {
    pub input_stack: usize,
    pub parameter_stack: usize,
    pub buffer_stack: usize,
}

impl CommandStackUsage {
    pub(crate) fn record_parameter_push(&mut self, param_ptr_after: usize) {
        self.parameter_stack = self.parameter_stack.max(param_ptr_after);
    }

    pub(crate) fn record_buffer_usage(&mut self, buffer_positions: usize) {
        self.buffer_stack = self.buffer_stack.max(buffer_positions);
    }
}

/// A recoverable command-owned semantic diagnostic awaiting executor output.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum CommandSemanticDiagnostic {
    /// TeX82's non-error diagnostic text produced while expansion owns the
    /// live macro invocation and argument buffers. Rendering is deferred to
    /// `tex-exec` so §245's selector and `\tracingonline` remain authoritative.
    Trace { text: String, force_newline: bool },
    /// A non-interactive pdfTeX expansion diagnostic that must be rendered
    /// after the conversion's temporary string selector has been restored.
    PdfExpansionMessage { text: String },
    /// TeX82 §370's undefined-control-sequence expansion error.
    ///
    /// §370 reports through §82, which renders `show_context` against the
    /// command stack that is still live inside the borrowed processor
    /// episode. The display therefore crosses the deferred-report boundary
    /// with the diagnostic, exactly as [`Self::MissingNumber`]'s does.
    UndefinedControlSequence { context: String },
    /// A recoverable command-owned error whose message, help and context the
    /// command core composed at the point of failure.
    ///
    /// `tex-command` never prints (see this crate's `AGENTS.md`), and the
    /// levels §82 displays are live only inside the borrowed processor
    /// episode. Composing the whole report here is what lets the executor
    /// render it faithfully after the borrow ends. `identity` correlates the
    /// report with the recovery that produced it without a second ledger.
    Recoverable {
        identity: u64,
        /// TeX82 §306's selector-routed heading and partial token list,
        /// printed immediately before the ordinary error report.
        runaway: Option<RunawayPrelude>,
        message: String,
        help: &'static [&'static str],
        context: String,
        /// The scanned value for reports canonically completed by TeX82
        /// §81's `int_error`, rather than its ordinary `error` routine.
        integer_error: Option<i32>,
    },
    /// TeX82 §391's compulsory macro-parameter-text mismatch.
    MacroPrefixMismatch {
        macro_name: tex_state::interner::Symbol,
        context: String,
    },
    /// TeX82 §415's missing-number recovery, deferred only when an earlier
    /// command-owned diagnostic is already waiting for executor output.
    ///
    /// §82 renders `show_context` when `error` completes, while §415 has
    /// already used §325's `back_error` to put the offending token back.
    /// The command stack is the sole owner of that backed-up level, so its
    /// display crosses the deferred-report boundary with the diagnostic.
    MissingNumber { context: String },
    /// TeX82 §§578--579's failed `find_font_dimen(false)` enquiry.
    ///
    /// The scanner owns both the bound decision and the live input context;
    /// the executor owns selector-aware rendering and the font identifier.
    FontDimenUnavailable {
        font: tex_state::ids::FontId,
        context: String,
    },
}

/// The output TeX's `runaway` procedure emits before its caller's error.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RunawayPrelude {
    pub heading: &'static str,
    pub partial: String,
}

/// One traced token admitted into the command generation.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct CommandPayload<G> {
    spelling: TracedTokenWord,
    brand: core::marker::PhantomData<fn(&G) -> &G>,
}

impl<G> Clone for CommandPayload<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for CommandPayload<G> {}

impl<G> crate::timeline::LogicalStackElement for CommandPayload<G> {
    type State = ();

    fn capture_state(&self) -> Self::State {}

    fn swap_state(&mut self, (): &mut Self::State) {}
}

impl<G> CommandPayload<G> {
    const fn new(spelling: TracedTokenWord) -> Self {
        Self {
            spelling,
            brand: core::marker::PhantomData,
        }
    }
}

/// Ordered `\aftergroup` payload for one exact TeX save level.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct CommandGroupPayload<G> {
    pub(crate) frame: GroupFrame,
    pub(crate) token_start: usize,
    pub(crate) token_top: usize,
    brand: core::marker::PhantomData<fn(&G) -> &G>,
    /// State-journal position of this level's newest §276 push.
    ///
    /// Tokens remain command-owned. This scalar only orders their newest
    /// physical save word against state-owned group/restore records.
    pub(crate) latest_aftergroup_position: Option<u32>,
}

impl<G> Clone for CommandGroupPayload<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for CommandGroupPayload<G> {}

impl<G> crate::timeline::LogicalStackElement for CommandGroupPayload<G> {
    type State = (usize, Option<u32>);

    fn capture_state(&self) -> Self::State {
        (self.token_top, self.latest_aftergroup_position)
    }

    fn swap_state(&mut self, state: &mut Self::State) {
        std::mem::swap(&mut self.token_top, &mut state.0);
        std::mem::swap(&mut self.latest_aftergroup_position, &mut state.1);
    }
}

impl<G> CommandGroupPayload<G> {
    const fn new(frame: GroupFrame, token_start: usize) -> Self {
        Self {
            frame,
            token_start,
            token_top: token_start,
            brand: core::marker::PhantomData,
            latest_aftergroup_position: None,
        }
    }
}

/// One synchronized state/command group close.
///
/// The restoration receipt is consumed by the executor before it replays the
/// returned `\aftergroup` tokens. Both values are owned and borrow-free, but
/// remain admitted-generation-local and must not enter a cold summary.
pub struct CommandGroupExit<G> {
    restorations: tex_state::GroupRestorationReceipt<G>,
    aftergroup: Vec<TracedTokenWord>,
}

impl<G> CommandGroupExit<G> {
    #[must_use]
    pub fn restorations(&self) -> &tex_state::GroupRestorationReceipt<G> {
        &self.restorations
    }

    #[must_use]
    pub fn into_aftergroup(self) -> Vec<TracedTokenWord> {
        self.aftergroup
    }
}

/// Failure to coordinate generation-bound command payloads with TeX's state
/// save journal. Every variant is detected before either owner mutates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandGroupError {
    State(StateError),
    StaleGroupState,
    NoOpenGroup,
    GroupMismatch {
        expected: GroupKind,
        actual: Option<GroupKind>,
    },
}

impl core::fmt::Display for CommandGroupError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::State(error) => write!(formatter, "state group operation failed: {error:?}"),
            Self::StaleGroupState => {
                formatter.write_str("command group payloads do not match the state save journal")
            }
            Self::NoOpenGroup => formatter.write_str("no TeX group is open for this payload"),
            Self::GroupMismatch { expected, actual } => write!(
                formatter,
                "expected {expected:?} command group, found {actual:?}"
            ),
        }
    }
}

impl std::error::Error for CommandGroupError {}

/// Opaque boundary for one executor-requested immutable token-list episode.
///
/// The command machine retains the input-level identity so the executor can
/// drive a completed list without observing raw input-stack structure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandReplayEpisode(InputLevelId);

/// Ownership edge from an executor replay episode to the exact input level
/// whose retirement makes that episode complete.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ReplayCompletionFence {
    episode: InputLevelId,
    owner: InputLevelId,
}

/// One expanded delivery from the episode-aware command boundary.
///
/// A completed stored episode is delivered on its own, after command-owned
/// retirement (and its observation/provenance effects) but before any token
/// from the enclosing input level is fetched.  This lets a stomach consumer
/// finalize its isolated mode or group without peeking at, or backing up,
/// parent source.
#[derive(Debug)]
pub enum CommandReplayDelivery<G> {
    Command(crate::CurrentCommand<G>),
    Completed(CommandReplayEpisode),
}

impl<G> CommandState<G> {
    fn validate_group_payloads(
        &self,
        state: &CommandContext<'_, G>,
    ) -> Result<(), CommandGroupError> {
        let state_frames = state.group_frames();
        let matches = state_frames.len() == self.group_payloads.len()
            && state_frames
                .iter()
                .zip(&self.group_payloads)
                .all(|(state, command)| *state == command.frame);
        if matches {
            Ok(())
        } else {
            Err(CommandGroupError::StaleGroupState)
        }
    }

    /// Opens one state save level and its generation-bound command payload
    /// frame as a single validated transition.
    pub fn begin_group(
        &mut self,
        state: &mut CommandContext<'_, G>,
        kind: GroupKind,
        entered_line: u32,
    ) -> Result<GroupFrame, CommandGroupError> {
        self.validate_group_payloads(state)?;
        let frame = state
            .begin_group(kind, entered_line)
            .map_err(CommandGroupError::State)?;
        let aftergroup_top = self.aftergroup_payloads.len();
        self.group_payloads
            .push(CommandGroupPayload::new(frame, aftergroup_top));
        Ok(frame)
    }

    /// Saves one traced `\aftergroup` spelling on the innermost exact save
    /// level. The state and command stacks are checked before the append.
    pub fn save_aftergroup(
        &mut self,
        state: &CommandContext<'_, G>,
        spelling: TracedTokenWord,
    ) -> Result<(), CommandGroupError> {
        self.validate_group_payloads(state)?;
        let Some(group) = self.group_payloads.last_mut() else {
            return Err(CommandGroupError::NoOpenGroup);
        };
        group.latest_aftergroup_position = Some(state.save_stack_order_position());
        group.token_top += 1;
        self.aftergroup_payloads.push(CommandPayload::new(spelling));
        Ok(())
    }

    /// Command-owned live §276 words and the state-journal coordinate of
    /// their newest push. The fold borrows existing payloads and allocates
    /// nothing.
    #[must_use]
    pub fn aftergroup_save_stack_projection(&self) -> (usize, Option<u32>) {
        self.group_payloads
            .iter()
            .fold((0_usize, None), |(words, latest), group| {
                (
                    words.saturating_add(group.token_top - group.token_start),
                    latest.max(group.latest_aftergroup_position),
                )
            })
    }

    /// Restores one exact state save level and returns its ordered restoration
    /// receipt plus `\aftergroup` payload in save order. Both owners and the
    /// expected kind are validated before state restoration begins; after it
    /// succeeds, removing the already-proven command frame is infallible.
    pub fn end_group(
        &mut self,
        state: &mut CommandContext<'_, G>,
        expected: GroupKind,
    ) -> Result<CommandGroupExit<G>, CommandGroupError> {
        self.validate_group_payloads(state)?;
        let actual = self.group_payloads.last().map(|group| group.frame.kind());
        if actual != Some(expected) {
            return Err(CommandGroupError::GroupMismatch { expected, actual });
        }
        let restorations = state
            .end_group(expected)
            .map_err(CommandGroupError::State)?;
        let group = self
            .group_payloads
            .pop_copy()
            .expect("validated command group frame remains present");
        let aftergroup = self.aftergroup_payloads[group.token_start..group.token_top]
            .iter()
            .map(|payload| payload.spelling)
            .collect();
        assert!(self.aftergroup_payloads.truncate_top(group.token_start));
        Ok(CommandGroupExit {
            restorations,
            aftergroup,
        })
    }

    /// Replaces TeX82 §1269's pending `\afterassignment` token inside the
    /// checkpointed command root.
    pub fn set_afterassignment(
        &mut self,
        state: &CommandContext<'_, G>,
        spelling: TracedTokenWord,
    ) -> Result<(), CommandGroupError> {
        self.validate_group_payloads(state)?;
        self.timeline.record_afterassignment(self.afterassignment);
        self.afterassignment = Some(CommandPayload::new(spelling));
        Ok(())
    }

    /// Takes the pending `\afterassignment` token after validating that this
    /// command root still accompanies the admitted state group stack.
    pub fn take_afterassignment(
        &mut self,
        state: &CommandContext<'_, G>,
    ) -> Result<Option<TracedTokenWord>, CommandGroupError> {
        self.validate_group_payloads(state)?;
        self.timeline.record_afterassignment(self.afterassignment);
        Ok(self.afterassignment.take().map(|payload| payload.spelling))
    }

    /// Whether an assignment token remains pending in this command root.
    #[must_use]
    pub fn has_afterassignment(&self) -> bool {
        self.afterassignment.is_some()
    }

    /// Resolves one operation-local token-list coordinate while the owning
    /// attempt remains installed.
    pub fn attempt_token_words(
        &self,
        id: crate::AttemptTokenListId,
    ) -> Result<crate::attempt::AttemptTokenListView<'_>, crate::AttemptError> {
        self.attempt.arena().token_words(id)
    }

    /// Atomically promotes every attempt-local root exposed by one resident
    /// destination into this generation's durable stores.
    ///
    /// The command attempt validates the complete request before reserving or
    /// publishing destination rows. Successful publication writes final
    /// owners into that same destination without an aggregate receipt.
    pub fn promote_attempt_roots_into<D>(
        &mut self,
        universe: &mut tex_state::Universe<G>,
        destination: &mut D,
    ) -> Result<(), crate::AttemptError>
    where
        D: crate::AttemptPromotionDestination<G>,
    {
        self.attempt.arena_mut().promote_into(universe, destination)
    }

    /// Promotes one declared macro-definition root and its schema-owned text
    /// into the current generation's definition arena.
    pub fn promote_attempt_definition(
        &mut self,
        universe: &mut tex_state::Universe<G>,
        id: crate::AttemptDefinitionId,
    ) -> Result<tex_state::DefinitionId<G>, crate::AttemptError> {
        self.attempt.arena_mut().promote_definition(universe, id)
    }

    /// Returns the number of live TeX input levels retained by this command state.
    #[must_use]
    pub fn input_level_count(&self) -> usize {
        self.input.levels.len()
    }

    /// Returns runtime-only TeX82 command-stack maxima for §1334.
    #[must_use]
    pub fn stack_usage(&self) -> CommandStackUsage {
        self.stack_usage
    }

    /// Returns a content-free, innermost-first tail of the live input stack
    /// for failure diagnostics. No tokens, source names, or source lines cross
    /// this boundary.
    #[must_use]
    pub fn diagnostic_input_context(&self, limit: usize) -> (usize, Vec<&'static str>) {
        let tail = self
            .input
            .levels
            .iter()
            .rev()
            .take(limit)
            .map(|level| match level {
                InputLevel::Source(_) => "source",
                InputLevel::Tokens(cursor) => match &cursor.trace {
                    ReplayTrace::MacroReplacement => "macro-body",
                    ReplayTrace::MacroParameter { .. } => "macro-argument",
                    ReplayTrace::BackedUp => "backed-up",
                    ReplayTrace::Inserted => "inserted",
                    ReplayTrace::UTemplate => "alignment-u-template",
                    ReplayTrace::VTemplate | ReplayTrace::OmitTemplate => "alignment-v-template",
                    ReplayTrace::Stored(_) => "stored-token-list",
                    ReplayTrace::Transient(_) => "transient-token-list",
                },
                InputLevel::MacroArgument(_) => "macro-argument",
            })
            .collect();
        (self.input.levels.len(), tail)
    }

    pub(crate) fn name_in_progress(&self) -> bool {
        self.name_in_progress
    }

    pub(crate) fn take_pending_input_open(&mut self) -> Option<crate::ScannedFileName> {
        let pending = self.pending_input_open.take();
        self.timeline.record_pending_input_open(pending.clone());
        pending
    }

    pub(crate) fn retain_pending_input_open(&mut self, file_name: crate::ScannedFileName) {
        debug_assert!(self.pending_input_open.is_none());
        self.timeline
            .record_pending_input_open(self.pending_input_open.clone());
        self.pending_input_open = Some(file_name);
    }

    pub(crate) fn begin_file_name(&mut self) -> Result<(), crate::CommandError> {
        if self.name_in_progress {
            return Err(crate::CommandError::input_invariant());
        }
        self.timeline.record_name_in_progress(self.name_in_progress);
        self.name_in_progress = true;
        Ok(())
    }

    pub(crate) fn end_file_name(&mut self) {
        self.timeline.record_name_in_progress(self.name_in_progress);
        self.name_in_progress = false;
    }

    pub(crate) fn record_alignment_phase(&mut self) {
        self.timeline.record_align_state(self.alignment.align_state);
    }

    /// Opens the smallest rollback-coupled scalar mutation for the standalone
    /// checkpoint allocation gate.
    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    pub fn profile_first_timeline_mutation(&mut self) {
        self.begin_file_name()
            .expect("profiling fixture begins outside filename scanning");
    }

    /// Rewrites one rollback-coupled scalar repeatedly inside one checkpoint
    /// interval, exercising dense first-touch coalescing without semantic I/O.
    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    pub fn profile_repeated_timeline_mutations(&mut self, mutations: usize) {
        for _ in 0..mutations {
            self.timeline.record_name_in_progress(self.name_in_progress);
            self.name_in_progress = !self.name_in_progress;
        }
    }

    /// Rewrites one token-frame execution phase repeatedly while its immutable
    /// input payload stays admitted once in the logical-stack row.
    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    pub fn profile_repeated_input_frame_mutations(&mut self, mutations: usize) {
        for _ in 0..mutations {
            let mutated = self.input.levels.toggle_top_token_retirement();
            assert!(mutated, "profiling fixture keeps a token frame on top");
        }
    }

    /// Installs the first physical line for the standalone source-history
    /// allocation gate before its observable checkpoint.
    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    pub fn profile_prepare_source_line(&mut self, endlinechar: i32) {
        self.input
            .levels
            .mutate_top_source(|source, slot| {
                let stored = crate::input::SourceLevelExecutionState::cursor(source, slot);
                slot.cursor
                    .load_next_line(endlinechar)
                    .expect("profiling source has a first line");
                (stored, ())
            })
            .expect("profiling fixture keeps a source frame on top");
    }

    /// Rewrites only the copy-small source lexer cursor. One checkpoint
    /// interval must retain one checked stored-state handle and no owner copy.
    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    pub fn profile_repeated_source_lex_mutations(&mut self, mutations: usize) {
        for _ in 0..mutations {
            self.input
                .levels
                .mutate_top_source_lex(|_, slot| {
                    let line = slot
                        .cursor
                        .line
                        .as_mut()
                        .expect("profiling fixture keeps one loaded line");
                    line.cursor.lexer_state = match line.cursor.lexer_state {
                        crate::LexerState::MidLine => crate::LexerState::SkipBlanks,
                        crate::LexerState::SkipBlanks | crate::LexerState::NewLine => {
                            crate::LexerState::MidLine
                        }
                    };
                })
                .expect("profiling fixture keeps a source frame on top");
        }
    }

    /// Moves one loaded-line owner into ordered rollback history and installs
    /// the next physical line without cloning its backing or spelling arena.
    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    pub fn profile_advance_source_line(&mut self, endlinechar: i32) {
        let loaded = self
            .input
            .levels
            .mutate_top_source(|source, slot| {
                let stored = crate::input::SourceLevelExecutionState::cursor(source, slot);
                let loaded = slot.cursor.load_next_line(endlinechar).is_some();
                (stored, loaded)
            })
            .expect("profiling fixture keeps a source frame on top");
        assert!(loaded, "profiling source has a second line");
    }

    /// Captures one compact source inverse, reuses its physical row for a
    /// token frame, and mutates that replacement. The ordered journal must
    /// preserve the source incarnation without cloning either full row.
    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    pub fn profile_source_lex_then_token_row_reuse(
        &mut self,
        stores: &tex_state::CommandContext<'_, G>,
        tokens: tex_state::TokenListId<G>,
    ) {
        self.profile_repeated_source_lex_mutations(1);
        self.profile_replace_source_row_with_token(stores, tokens);
    }

    /// Moves one cold source owner into history before reusing its physical
    /// row for a token frame. This is the owner-bearing counterpart of
    /// [`Self::profile_source_lex_then_token_row_reuse`].
    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    pub fn profile_source_owner_then_token_row_reuse(
        &mut self,
        stores: &tex_state::CommandContext<'_, G>,
        tokens: tex_state::TokenListId<G>,
        endlinechar: i32,
    ) {
        self.profile_advance_source_line(endlinechar);
        self.profile_replace_source_row_with_token(stores, tokens);
    }

    #[cfg(feature = "profiling")]
    fn profile_replace_source_row_with_token(
        &mut self,
        stores: &tex_state::CommandContext<'_, G>,
        tokens: tex_state::TokenListId<G>,
    ) {
        self.pop_input_level_at_end_of_job()
            .expect("profiling ordered reuse retires its source frame");
        let words = stores.token_list(tokens);
        self.push_token_level(
            crate::input::PackedTokenSpanHandle::durable(words),
            crate::input::TokenBehavior::Ordinary,
            crate::input::RetirementBehavior::Pop,
            crate::input::ReplayTrace::Stored(crate::input::StoredReplayReason::EveryPar),
        );
        let mutated = self
            .input
            .levels
            .set_top_token_retirement(crate::input::RetirementBehavior::StopAtEnd);
        assert!(mutated, "profiling ordered reuse installs a token frame");
    }

    /// Reuses one physical input-stack row repeatedly after its current
    /// interval has reached the requested high water. No retained checkpoint
    /// observes an intermediate frame, so the loop must allocate and append
    /// no rollback history after the caller's one-transition warmup.
    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    pub fn profile_repeated_input_level_reuse(
        &mut self,
        stores: &tex_state::CommandContext<'_, G>,
        tokens: tex_state::TokenListId<G>,
        transitions: usize,
    ) {
        for _ in 0..transitions {
            let words = stores.token_list(tokens.clone());
            self.push_token_level(
                crate::input::PackedTokenSpanHandle::durable(words),
                crate::input::TokenBehavior::Ordinary,
                crate::input::RetirementBehavior::Pop,
                crate::input::ReplayTrace::Stored(crate::input::StoredReplayReason::EveryPar),
            );
            self.pop_input_level_at_end_of_job()
                .expect("profiling reuse retires its just-pushed token frame");
        }
    }

    /// Returns structural packed-journal evidence for standalone gates.
    #[doc(hidden)]
    #[must_use]
    pub fn profile_timeline_counters(&self) -> crate::CommandTimelineCounters {
        let mut counters = self.timeline.packed_journal_counters();
        for stack in [
            self.input.levels.counters(),
            self.parameters.activations.counters(),
            self.conditions.frames.counters(),
            self.group_payloads.counters(),
            self.aftergroup_payloads.counters(),
            self.alignment.align_stack.counters(),
            self.alignment.suspended.counters(),
        ] {
            counters.logical_payload_admissions = counters
                .logical_payload_admissions
                .saturating_add(stack.payload_admissions);
            counters.full_frame_history_clones = counters
                .full_frame_history_clones
                .saturating_add(stack.full_payload_history_clones);
            counters.logical_records = counters.logical_records.saturating_add(stack.undo_records);
            counters.logical_record_bytes = counters
                .logical_record_bytes
                .saturating_add(stack.undo_record_bytes);
            counters.logical_coalesced_mutations = counters
                .logical_coalesced_mutations
                .saturating_add(stack.coalesced_mutations);
            counters.logical_stored_state_captures = counters
                .logical_stored_state_captures
                .saturating_add(stack.stored_state_captures);
            counters.logical_owner_swaps = counters
                .logical_owner_swaps
                .saturating_add(stack.owner_swaps);
            counters.displaced_payloads = counters
                .displaced_payloads
                .saturating_add(u64::from(stack.displaced_payloads));
            counters.displaced_reuses = counters
                .displaced_reuses
                .saturating_add(stack.displaced_reuses);
            counters.selected_rewind_records = counters
                .selected_rewind_records
                .saturating_add(stack.selected_rewind_records);
            counters.candidate_reject_records = counters
                .candidate_reject_records
                .saturating_add(stack.candidate_reject_records);
            counters.accepted_redo_records = counters
                .accepted_redo_records
                .saturating_add(stack.accepted_redo_records);
            counters.candidate_chunks_released = counters
                .candidate_chunks_released
                .saturating_add(stack.candidate_chunks_released);
            counters.accepted_chunks_released = counters
                .accepted_chunks_released
                .saturating_add(stack.accepted_chunks_released);
        }
        counters
    }

    /// Reads the scalar used by the standalone rollback/fork isolation gate.
    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    #[must_use]
    pub fn profile_name_in_progress(&self) -> bool {
        self.name_in_progress
    }

    /// Current TeX82 `line`, or zero for token-list and `\read` input.
    ///
    /// Save-level owners use this only to preserve e-TeX's `saved(-1)`
    /// diagnostic metadata; it is not part of command semantics.
    #[must_use]
    pub fn current_file_line_number(&self) -> u32 {
        u32::try_from(self.input.current_file_line_number()).unwrap_or(0)
    }

    #[must_use]
    pub fn current_file_source_id(&self) -> Option<tex_state::SourceId> {
        self.input.current_file_source_id()
    }

    /// Returns the bottom physical source row while command input still owns
    /// it. Compact source context deliberately survives source retirement for
    /// diagnostics, but an editor restart requires this owner-backed row.
    #[doc(hidden)]
    #[must_use]
    pub fn live_physical_root_source_id(&self) -> Option<tex_state::SourceId> {
        self.input.levels.iter().find_map(|level| {
            let crate::input::InputLevel::Source(source) = level else {
                return None;
            };
            Some(
                self.input
                    .levels
                    .source_level_slot(source)
                    .cursor
                    .current_backing()
                    .id,
            )
        })
    }

    /// Most recent direct source location committed by command delivery.
    #[must_use]
    pub fn last_diagnostic_location(&self) -> Option<crate::SourceLocation> {
        self.last_diagnostic_location
    }

    /// Resets the focused active-source delivery counters.
    #[doc(hidden)]
    #[cfg(any(test, feature = "profiling"))]
    pub fn profile_reset_input_source_context_counters(&self) {
        crate::input::reset_input_source_context_counters();
    }

    /// Returns `(top reads, ancestry rows, owner-slot lookups, lexer-slot
    /// borrows)` for focused active-source delivery measurements.
    #[doc(hidden)]
    #[cfg(any(test, feature = "profiling"))]
    #[must_use]
    pub fn profile_input_source_context_counters(&self) -> (u64, u64, u64, u64) {
        let counters = crate::input::input_source_context_counters();
        (
            counters.top_reads,
            counters.ancestry_rows,
            counters.owner_slot_lookups,
            counters.source_lex_slot_borrows,
        )
    }

    /// Resets the focused resident-input cursor mutation counters.
    #[doc(hidden)]
    #[cfg(any(test, feature = "profiling"))]
    pub fn profile_reset_input_cursor_mutation_counters(&mut self) {
        self.input.levels.reset_cursor_mutation_counters();
    }

    /// Returns `(typed top accesses, first touches, coalesced transitions,
    /// closure dispatches)` for the direct resident-input mutation boundary.
    #[doc(hidden)]
    #[cfg(any(test, feature = "profiling"))]
    #[must_use]
    pub fn profile_input_cursor_mutation_counters(&self) -> (u64, u64, u64, u64) {
        let counters = self.input.levels.cursor_mutation_counters();
        (
            counters.typed_top_accesses,
            counters.first_touch_transitions,
            counters.coalesced_transitions,
            counters.closure_dispatches,
        )
    }

    /// Returns `(top selections, source entries, stored-token entries,
    /// macro-argument entries)` for the typed resident-input front.
    #[doc(hidden)]
    #[cfg(any(test, feature = "profiling"))]
    #[must_use]
    pub fn profile_resident_input_branch_counters(&self) -> (u64, u64, u64, u64) {
        let counters = self.input.levels.cursor_mutation_counters();
        (
            counters.typed_top_accesses,
            counters.source_branch_entries,
            counters.stored_token_branch_entries,
            counters.macro_argument_branch_entries,
        )
    }

    /// Resets the focused ordinary raw-delivery path counters.
    #[doc(hidden)]
    #[cfg(test)]
    pub fn profile_reset_raw_delivery_path_counters(&mut self) {
        self.raw_delivery_path_counters = RawDeliveryPathCounters::default();
    }

    /// Returns `(source direct, stored direct, literal macro-argument direct,
    /// out-parameter interceptions)` for focused raw-delivery measurements.
    #[doc(hidden)]
    #[cfg(test)]
    #[must_use]
    pub fn profile_raw_delivery_path_counters(&self) -> (u64, u64, u64, u64) {
        let counters = self.raw_delivery_path_counters;
        (
            counters.source_direct,
            counters.stored_direct,
            counters.macro_argument_direct,
            counters.out_parameter_interceptions,
        )
    }

    /// Returns `(resident transitions, intermediate-result redispatches,
    /// whole-input-frame copies)` for focused raw-delivery gates.
    #[doc(hidden)]
    #[cfg(test)]
    #[must_use]
    pub fn profile_resident_delivery_transition_counters(&self) -> (u64, u64, u64) {
        let counters = self.raw_delivery_path_counters;
        (
            counters.resident_transitions,
            counters.intermediate_result_redispatches,
            counters.whole_input_frame_copies,
        )
    }

    /// Returns structural resident-retirement facts in field order: ordinary
    /// pops, status relays, repeated top lookups, repeated owner validations,
    /// whole-token copies, cold source retirements, conservation retirements.
    #[doc(hidden)]
    #[cfg(test)]
    #[must_use]
    pub fn profile_resident_retirement_counters(&self) -> (u64, u64, u64, u64, u64, u64, u64) {
        let counters = self.raw_delivery_path_counters;
        (
            counters.resident_ordinary_retirements,
            counters.exhaustion_status_relays,
            counters.retirement_top_lookups,
            counters.retirement_owner_validations,
            counters.retirement_whole_token_copies,
            counters.cold_source_retirements,
            counters.conservation_retirements,
        )
    }

    /// Returns `(retirement checks, ownership claims, descendant transfers)` for
    /// the ownership-directed replay-completion boundary.
    #[doc(hidden)]
    #[cfg(test)]
    #[must_use]
    pub fn profile_replay_completion_counters(&self) -> (u64, u64, u64) {
        let counters = self.raw_delivery_path_counters;
        (
            counters.replay_completion_retirement_checks,
            counters.replay_completion_claims,
            counters.replay_completion_transfers,
        )
    }

    /// Resets focused shared token-collector counters.
    #[doc(hidden)]
    #[cfg(test)]
    pub fn profile_reset_token_collector_path_counters(&mut self) {
        self.token_collector_path_counters = TokenCollectorPathCounters::default();
    }

    /// Returns classification/append/state/phase/rescan/settlement and
    /// whole-value-copy facts for focused shared-collector gates.
    #[doc(hidden)]
    #[cfg(test)]
    #[must_use]
    pub fn profile_token_collector_path_counters(
        &self,
    ) -> (u64, u64, u64, u64, u64, u64, u64, u64, u64, u64, u64) {
        let counters = self.token_collector_path_counters;
        (
            counters.collectors_started,
            counters.raw_classifications,
            counters.collector_appends,
            counters.state_updates,
            counters.phase_transitions,
            counters.duplicate_phase_dispatches,
            counters.fact_rescans,
            counters.settlements,
            counters.whole_token_list_copies,
            counters.whole_command_copies,
            counters.whole_frame_copies,
        )
    }

    /// Captures TeX82 §530's current input display before deferred shipout
    /// releases the command processor borrow.
    #[must_use]
    pub fn output_open_context(&self, stores: &tex_state::CommandContext<'_, G>) -> String {
        self.input.output_open_context(
            stores,
            &self.parameters,
            self.attempt.arena(),
            &self.scratch,
        )
    }

    /// Captures the allocation-free live-input coordinate used until a real
    /// diagnostic publication boundary asks for owned text.
    #[must_use]
    pub fn diagnostic_context_coordinate(&self) -> DiagnosticContextCoordinate {
        DiagnosticContextCoordinate {
            owner: self.timeline.owner_id(),
            input_revision: self.input.levels.context_revision(),
            scalar_revision: self.input.context_revision,
        }
    }

    /// Materializes a previously captured live-input coordinate exactly once.
    pub fn render_diagnostic_context(
        &self,
        coordinate: DiagnosticContextCoordinate,
        stores: &tex_state::CommandContext<'_, G>,
    ) -> Result<String, StaleDiagnosticContext> {
        if coordinate.owner != self.timeline.owner_id()
            || coordinate.input_revision != self.input.levels.context_revision()
            || coordinate.scalar_revision != self.input.context_revision
        {
            return Err(StaleDiagnosticContext);
        }
        Ok(self.output_open_context(stores))
    }

    /// Retains TeX82 §331's bottom terminal buffer for §310 after the
    /// startup acquisition level has been retired.
    pub fn set_terminal_context_line(&mut self, line: impl Into<String>) {
        let line = line.into();
        // TeX82 §31's initial terminal `input_ln` starts at buffer index zero;
        // §1334 subsequently prints `max_buf_stack+1`.
        if !line.is_empty() {
            self.stack_usage
                .record_buffer_usage(line.chars().count() + 1);
        }
        self.terminal_buffer_slots = line.chars().count();
        self.input.terminal_context_line = Some(line);
        self.input.context_revision = self.input.context_revision.wrapping_add(1).max(1);
    }

    pub(crate) fn open_context_starts_with_print_ln(
        &self,
        stores: &tex_state::CommandContext<'_, G>,
    ) -> bool {
        self.input.open_context_starts_with_print_ln(
            stores,
            &self.parameters,
            self.attempt.arena(),
            &self.scratch,
        )
    }

    pub(crate) fn output_retiring_source_context(
        &self,
        source: &crate::input::SourceLevel<G>,
        stores: &tex_state::CommandContext<'_, G>,
    ) -> String {
        self.input.output_retiring_source_context(
            source,
            stores,
            &self.parameters,
            self.attempt.arena(),
            &self.scratch,
        )
    }

    /// TeX82 §§1026/1028's context after the selected output list ends.
    ///
    /// Canonical delivery can retain a depleted cursor until the next fetch
    /// so its retirement remains observable at that boundary. The
    /// synchronous post-output error nevertheless sees the levels below it,
    /// exactly as it would after §1026's `end_token_list`.
    #[must_use]
    pub fn output_close_context(&self, stores: &tex_state::CommandContext<'_, G>) -> String {
        self.input.output_close_context(
            stores,
            &self.parameters,
            self.attempt.arena(),
            &self.scratch,
        )
    }

    /// Whether TeX82 §1370's artificial deferred-write input is live.
    pub(crate) fn expanding_deferred_write(&self) -> bool {
        self.input.levels.iter().any(|level| {
            matches!(
                level,
                InputLevel::Tokens(cursor)
                    if cursor.trace == ReplayTrace::Stored(StoredReplayReason::Write)
            )
        })
    }

    /// Schedules one completed `\\discretionary` part for canonical replay.
    ///
    /// This is deliberately a stored command level, not an executor-owned
    /// input stack: macro expansion, recovery, provenance, and retirement all
    /// remain command-owned while the stomach supplies the restricted hmode
    /// lifecycle.
    pub fn push_discretionary_episode(
        &mut self,
        stores: &tex_state::CommandContext<'_, G>,
        tokens: tex_state::TokenListId<G>,
    ) -> CommandReplayEpisode {
        self.push_stored_episode(
            stores,
            tokens,
            crate::input::StoredReplayReason::Discretionary,
        )
    }

    /// Schedules one source-isolated output-text expansion episode.
    ///
    /// Completion is delivered before the surrounding source resumes, so a
    /// shipout host cannot accidentally consume the command following the
    /// page or PDF form it is staging.
    pub fn push_output_replay_episode(
        &mut self,
        stores: &tex_state::CommandContext<'_, G>,
        tokens: tex_state::TokenListId<G>,
    ) -> CommandReplayEpisode {
        self.push_stored_episode(stores, tokens, crate::input::StoredReplayReason::Write)
    }

    fn push_stored_episode(
        &mut self,
        stores: &tex_state::CommandContext<'_, G>,
        tokens: tex_state::TokenListId<G>,
        reason: StoredReplayReason,
    ) -> CommandReplayEpisode {
        let words = stores.token_list(tokens.clone());
        let identity = self.push_token_level(
            PackedTokenSpanHandle::durable(words),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Stored(reason),
        );
        self.replay_completions.push(ReplayCompletionFence {
            episode: identity,
            owner: identity,
        });
        CommandReplayEpisode(identity)
    }

    /// Whether the requested immutable replay level is still live.
    #[must_use]
    pub fn replay_episode_is_active(&self, episode: CommandReplayEpisode) -> bool {
        self.input
            .levels
            .iter()
            .any(|level| crate::input::input_level_identity(level) == episode.0)
    }

    /// Claims the completion boundary for an executor-requested stored level
    /// after command-owned retirement. Input replay explanations remain
    /// diagnostic-only; this independent state records only the typed
    /// executor delivery contract.
    pub(crate) fn complete_replay(
        &mut self,
        identity: InputLevelId,
    ) -> Option<CommandReplayEpisode> {
        #[cfg(test)]
        {
            self.raw_delivery_path_counters
                .replay_completion_retirement_checks = self
                .raw_delivery_path_counters
                .replay_completion_retirement_checks
                .saturating_add(1);
        }
        let completion = self.replay_completions.last().copied()?;
        if completion.owner != identity {
            return None;
        }
        #[cfg(test)]
        {
            self.raw_delivery_path_counters.replay_completion_claims = self
                .raw_delivery_path_counters
                .replay_completion_claims
                .saturating_add(1);
        }
        self.replay_completions.pop();
        Some(CommandReplayEpisode(completion.episode))
    }

    /// Transfers a completion produced by §325/§390 stack conservation to the
    /// exact newer level installed by that same transition.
    pub(crate) fn defer_replay_completion(
        &mut self,
        episode: Option<CommandReplayEpisode>,
        owner: InputLevelId,
    ) {
        if let Some(CommandReplayEpisode(episode)) = episode {
            #[cfg(test)]
            {
                self.raw_delivery_path_counters.replay_completion_transfers = self
                    .raw_delivery_path_counters
                    .replay_completion_transfers
                    .saturating_add(1);
            }
            self.replay_completions
                .push(ReplayCompletionFence { episode, owner });
        }
    }

    /// Schedules a frozen `\everypar` list after canonical main control has
    /// completed TeX82's `new_graf` state transition.  Source ownership stays
    /// entirely inside command state; executor control never fabricates an
    /// input stack for token-list replay.
    pub fn push_everypar(
        &mut self,
        stores: &tex_state::CommandContext<'_, G>,
        tokens: tex_state::TokenListId<G>,
    ) {
        self.push_named_token_list(stores, tokens, StoredReplayReason::EveryPar);
    }

    /// Schedules the immutable math-entry hook after the stomach has entered
    /// the matching math-shift group.  The command machine owns this replay
    /// so macro expansion, origins, and retirement stay canonical.
    pub fn push_everymath(
        &mut self,
        stores: &tex_state::CommandContext<'_, G>,
        tokens: tex_state::TokenListId<G>,
        display: bool,
    ) {
        self.push_named_token_list(
            stores,
            tokens,
            if display {
                StoredReplayReason::EveryDisplay
            } else {
                StoredReplayReason::EveryMath
            },
        );
    }

    /// Schedules the immutable `\everyhbox` or `\everyvbox` payload after
    /// canonical replay has entered the corresponding box group and mode.
    pub fn push_everybox(
        &mut self,
        stores: &tex_state::CommandContext<'_, G>,
        tokens: tex_state::TokenListId<G>,
        horizontal: bool,
    ) {
        self.push_named_token_list(
            stores,
            tokens,
            if horizontal {
                StoredReplayReason::EveryHBox
            } else {
                StoredReplayReason::EveryVBox
            },
        );
    }

    /// Schedules the immutable `\everycr` payload for tex.web §774
    /// `init_align`'s and §799 `fin_row`'s shared
    /// `if every_cr<>null then begin_token_list(every_cr,every_cr_text)`,
    /// which both run immediately before `align_peek`.
    pub fn push_everycr(
        &mut self,
        stores: &tex_state::CommandContext<'_, G>,
        tokens: tex_state::TokenListId<G>,
    ) {
        self.push_named_token_list(stores, tokens, StoredReplayReason::EveryCr);
    }

    /// Schedules the immutable `\everyjob` payload for tex.web §1030
    /// `main_control`'s prologue,
    /// `if every_job<>null then begin_token_list(every_job,every_job_text)`,
    /// which runs once before the first `big_switch` fetch.
    pub fn push_everyjob(
        &mut self,
        stores: &tex_state::CommandContext<'_, G>,
        tokens: tex_state::TokenListId<G>,
    ) {
        self.push_named_token_list(stores, tokens, StoredReplayReason::EveryJob);
    }

    /// Installs one tex.web §307-named token list and records its push.
    ///
    /// This is `begin_token_list` for the executor-requested hooks: the level
    /// carries the §307 `token_type` it was installed under, so both its push
    /// and its eventual retirement report that identity rather than the one
    /// token-list class every stored level used to share.
    fn push_named_token_list(
        &mut self,
        stores: &tex_state::CommandContext<'_, G>,
        tokens: tex_state::TokenListId<G>,
        reason: StoredReplayReason,
    ) {
        let words = stores.token_list(tokens.clone());
        let level = self.push_token_level(
            PackedTokenSpanHandle::durable(words),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Stored(reason),
        );
        self.named_token_list_pushes.push((level, reason, tokens));
    }

    /// Applies an executor-owned structural alignment request.
    ///
    /// This is the only lifecycle entry point required by `tex-exec`.  It has
    /// no token input, so it cannot duplicate `get_next` delimiter or brace
    /// classification.  Starting a v-template is intentionally absent: that
    /// transition requires an [`crate::AlignmentDeliveryEvent`] and is owned
    /// by [`crate::CommandProcessor`].
    pub fn apply_alignment_request(
        &mut self,
        stores: &tex_state::CommandContext<'_, G>,
        request: AlignmentRequest,
    ) -> Result<AlignmentRequestResult, AlignmentLifecycleError> {
        match request {
            AlignmentRequest::Begin(alignment) => {
                self.begin_alignment(alignment);
                Ok(AlignmentRequestResult::Applied)
            }
            AlignmentRequest::Preamble(alignment) => {
                self.set_alignment_preamble_phase(alignment)?;
                Ok(AlignmentRequestResult::Applied)
            }
            AlignmentRequest::PrepareCellLookahead(alignment) => {
                if self.alignment.active_alignment != Some(alignment) {
                    return Err(AlignmentLifecycleError::WrongAlignment);
                }
                self.prepare_alignment_cell_lookahead()?;
                Ok(AlignmentRequestResult::Applied)
            }
            AlignmentRequest::InstallCellTemplate(alignment) => {
                self.install_alignment_cell_template(stores, alignment)?;
                Ok(AlignmentRequestResult::Applied)
            }
            AlignmentRequest::InstallOmitCellTemplate(alignment) => {
                self.install_alignment_omit_cell_template(alignment)?;
                Ok(AlignmentRequestResult::Applied)
            }
            AlignmentRequest::FinishCell(alignment) => Ok(AlignmentRequestResult::FinishedCell(
                self.finish_alignment_cell(alignment)?,
            )),
            AlignmentRequest::RecoverExtraTab(alignment) => {
                self.record_alignment_phase();
                self.alignment.recover_extra_tab(alignment)?;
                Ok(AlignmentRequestResult::ExtraTabRecovered)
            }
            AlignmentRequest::Suspend(alignment) => {
                self.suspend_alignment(alignment)?;
                Ok(AlignmentRequestResult::Applied)
            }
            AlignmentRequest::Resume(alignment) => {
                self.resume_alignment(alignment)?;
                Ok(AlignmentRequestResult::Applied)
            }
            AlignmentRequest::Finish(alignment) => {
                self.finish_alignment(alignment)?;
                Ok(AlignmentRequestResult::Applied)
            }
        }
    }

    /// Begins an executor-owned structural alignment at the canonical preamble
    /// sentinel. Delimiter classification remains exclusively in `get_next`.
    pub fn begin_alignment(&mut self, alignment: AlignmentIdentity) {
        self.record_alignment_phase();
        self.alignment.begin_alignment(alignment);
    }

    /// Re-enters the preamble sentinel while scanning another alignment column.
    pub fn set_alignment_preamble_phase(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        self.record_alignment_phase();
        self.alignment.set_preamble_phase(alignment)
    }

    /// Marks one cell's executor-selected templates active and establishes the
    /// body brace-depth base. This operation does not inspect input tokens.
    ///
    /// The source opening brace must be delivered and backed up through a
    /// command processor before [`Self::install_alignment_cell_template`]
    /// installs the optional u-template.
    pub fn begin_prepared_alignment_cell(
        &mut self,
        alignment: AlignmentIdentity,
        templates: PreparedAlignmentCellTemplates<G>,
    ) -> Result<(), AlignmentLifecycleError> {
        self.record_alignment_phase();
        self.alignment.begin_cell(alignment, templates)
    }

    /// Installs the active cell's optional u-template after the executor's
    /// typed opener phase has completed command-owned brace replay.
    pub fn install_alignment_cell_template(
        &mut self,
        stores: &tex_state::CommandContext<'_, G>,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        let template = self.alignment.active_cell_template(alignment)?;
        self.record_alignment_phase();
        if let Some(template) = template {
            let level = self.push_alignment_template(
                stores,
                template,
                TokenBehavior::UTemplate,
                RetirementBehavior::Pop,
                ReplayTrace::UTemplate,
            );
            self.alignment.attach_u_template(alignment, level)?;
        } else {
            self.alignment.mark_u_template_installed(alignment)?;
        }
        Ok(())
    }

    /// Restores `align_peek`'s lookahead sentinel before `init_col` consumes
    /// the selected entry's first nonblank command.
    pub(crate) fn prepare_alignment_cell_lookahead(
        &mut self,
    ) -> Result<(), AlignmentLifecycleError> {
        let _alignment = self
            .alignment
            .active_alignment
            .ok_or(AlignmentLifecycleError::NoActiveAlignment)?;
        self.record_alignment_phase();
        self.alignment.align_state = 1_000_000;
        Ok(())
    }

    /// Completes TeX82 `init_col`'s `cur_cmd=omit` branch.
    ///
    /// The expanded omit command was already delivered by command processing.
    /// It is neither backed up nor followed by a u-template input level; the
    /// next delivery is cell-body input at the zero sentinel (TeX82 §37).
    pub fn install_alignment_omit_cell_template(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        let previous_align_state = self.alignment.align_state;
        self.record_alignment_phase();
        let cell = self.alignment.active_cell_mut(alignment)?;
        if cell.u_template_installed {
            return Err(AlignmentLifecycleError::UTemplateAlreadyInstalled);
        }
        cell.u_template_installed = true;
        cell.omit = true;
        cell.omit_previous_align_state = Some(previous_align_state);
        self.alignment.align_state = CELL_ALIGN_STATE;
        Ok(())
    }

    /// Transfers one completed raw preamble to the executor for structural
    /// column selection. The returned templates remain frozen command-owned
    /// values; no raw preamble token is exposed.
    pub fn take_completed_alignment_preamble(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<crate::AlignmentPreamble, AlignmentLifecycleError> {
        self.alignment.take_completed_preamble(alignment)
    }

    /// Starts the selected cell's v-template after `end_template` main control
    /// has backed up the intercepted delimiter. The suffix is an ordinary
    /// input level, so definitions and macro expansion inside it restart via
    /// the canonical raw-delivery loop.
    pub fn begin_alignment_v_template(
        &mut self,
        stores: &tex_state::CommandContext<'_, G>,
        alignment: AlignmentIdentity,
        delimiter: AlignmentCellDelimiter,
        delimiter_line: u32,
    ) -> Result<(), AlignmentLifecycleError> {
        let template = self.alignment.v_template(alignment)?;
        // tex.web §789: `if cur_cmd=omit then begin_token_list(omit_template,
        // v_template) else begin_token_list(v_part(cur_align),v_template)`.
        // Both levels are `token_type=v_template`; only the list differs, and
        // that is what names the level in the pinned observer's trace.
        let level = match template {
            None => self.push_token_level(
                PackedTokenSpanHandle::transient([]),
                TokenBehavior::VTemplate,
                RetirementBehavior::RetainExhaustedVTemplate,
                ReplayTrace::OmitTemplate,
            ),
            Some(template) => self.push_alignment_template(
                stores,
                template,
                TokenBehavior::VTemplate,
                RetirementBehavior::RetainExhaustedVTemplate,
                ReplayTrace::VTemplate,
            ),
        };
        self.record_alignment_phase();
        self.alignment
            .begin_v_template(alignment, level, delimiter, delimiter_line)
    }

    /// Completes one alignment entry at the executor's `do_endv` boundary.
    ///
    /// tex.web §1131's `do_endv` only *inspects* the input stack: it walks
    /// `base_ptr` down past exhausted token lists to prove that the frame it
    /// reaches is the v-template, and `fatal_error`s otherwise. It pops
    /// nothing. §357's `end_token_list` pops the exhausted v-template the
    /// next time `get_next` reaches it, which with a non-empty `\everycr` is
    /// not until §799's every-cr list and the whole `\noalign` body it may
    /// contain have been read.
    pub fn finish_alignment_cell(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<crate::FinishedAlignmentCell, AlignmentLifecycleError> {
        let level = self.alignment.active_v_template_level(alignment)?;
        self.prove_endv_input_shape(level)?;
        // TeX82 §791 changes `align_state` in `fin_col` before `get_next`
        // reaches the exhausted scanner backup and retained v-template.
        self.record_alignment_phase();
        self.alignment.finish_cell(alignment, level)
    }

    /// Performs TeX82 §1131 `do_endv`'s input-stack walk.
    ///
    /// The optional upper level is the exhausted one-token `back_input`
    /// replay produced by a scanner. The full store-aware §1131 walk is
    /// owned by `CommandProcessor::finish_alignment_cell`; this state-only
    /// proof remains for direct structural request tests that have no token
    /// store capability.
    fn prove_endv_input_shape(&self, v_level: InputLevelId) -> Result<(), AlignmentLifecycleError> {
        let retained_v_template = |level: &InputLevel<G>| {
            matches!(level,
                InputLevel::Tokens(cursor)
                    if cursor.identity() == v_level
                        && matches!(cursor.behavior, TokenBehavior::VTemplate)
                        && matches!(cursor.retirement, RetirementBehavior::AwaitingVTemplateRetirement)
            )
        };
        let Some(top) = self.input.levels.last() else {
            return Err(AlignmentLifecycleError::VTemplateNotExhausted);
        };
        if retained_v_template(top) {
            return Ok(());
        }
        let exhausted_backed_up_endv = matches!(top,
            InputLevel::Tokens(cursor)
                if matches!(cursor.behavior, TokenBehavior::BackedUp(_))
                    && matches!(cursor.span,
                        PackedTokenSpanHandle::Replay { replay, len }
                            if self.input.replay.ownership(replay)
                                == Some(crate::input::PackedTokenOwnership::BackedUp)
                                && cursor.position() >= len as usize)
        );
        if exhausted_backed_up_endv
            && self
                .input
                .levels
                .get(self.input.levels.len().saturating_sub(2))
                .is_some_and(retained_v_template)
        {
            Ok(())
        } else {
            Err(AlignmentLifecycleError::VTemplateNotExhausted)
        }
    }

    pub(crate) fn finish_alignment_cell_after_input_proof(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<crate::FinishedAlignmentCell, AlignmentLifecycleError> {
        let level = self.alignment.active_v_template_level(alignment)?;
        self.record_alignment_phase();
        self.alignment.finish_cell(alignment, level)
    }

    /// Suspends the complete outer raw-delivery context for a nested alignment.
    pub fn suspend_alignment(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        self.record_alignment_phase();
        self.alignment.suspend_alignment(alignment)
    }

    /// Restores the exact outer raw-delivery context after a nested alignment.
    pub fn resume_alignment(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        self.record_alignment_phase();
        self.alignment.resume_alignment(alignment)
    }

    /// Finishes an alignment delivery context after all of its cells retire.
    pub fn finish_alignment(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        self.record_alignment_phase();
        self.alignment.finish_alignment(alignment)
    }

    /// Creates a fresh command job with an immutable semantic profile.
    ///
    /// No API changes the profile after construction. Snapshot, summary,
    /// format, and checkpoint restoration validate their recorded profile
    /// identity against this value.
    #[must_use]
    pub fn new(profile: CommandProfile) -> Self {
        let mut state = Self::default();
        state.roots.engine_semantics = CommandEngineSemantics::for_profile(profile);
        state.roots.expansion.profile = profile;
        state
    }

    /// Selects the canonical compiled implementation executing this job.
    ///
    /// A newer implementation may execute an older format/profile, but the
    /// reverse combination is not canonical and is rejected.
    pub fn set_engine_semantics(&mut self, engine: CommandEngineSemantics) {
        assert!(
            engine.supports(self.profile()),
            "command engine semantics must support the loaded command profile"
        );
        self.engine_semantics = engine;
    }

    /// Returns the canonical compiled implementation executing this job.
    #[must_use]
    pub fn engine_semantics(&self) -> CommandEngineSemantics {
        self.engine_semantics
    }

    /// Registers complete immutable backing without consulting host policy.
    ///
    /// Registration validates Unicode before allocating an identity. It does
    /// not open an input level or perform any tokenization.
    pub fn register_source(
        &mut self,
        registration: SourceRegistration,
    ) -> Result<tex_state::SourceId, SourceRegistrationError> {
        let raw = u32::try_from(self.input.next_source_identity)
            .map_err(|_| SourceRegistrationError::SourceIdentityExhausted)?;
        let id = tex_state::SourceId::new(raw);
        let source = RegisteredSource::register(id, self.profile(), registration)?;
        self.timeline
            .record_next_source_identity(self.input.next_source_identity);
        self.input.next_source_identity += 1;
        let previous = self.input.pending_sources.insert(raw, source);
        debug_assert!(previous.is_none(), "source identities are unique");
        Ok(id)
    }

    /// Rebinds one live generated source after a checkpoint fork.
    ///
    /// Validation precedes mutation. The input stack then records the old
    /// owner in its existing ordered journal, so rejection restores the
    /// accepted bytes and acceptance releases them with the obsolete suffix.
    #[doc(hidden)]
    pub fn rebind_generated_source(
        &mut self,
        source: tex_state::SourceId,
        bytes: std::sync::Arc<[u8]>,
        unchanged_prefix: usize,
    ) -> Result<tex_state::SourceId, SourceRegistrationError> {
        let accepted = self
            .input
            .levels
            .physical_source(source)
            .expect("the retained root source remains live at a restart boundary");
        if unchanged_prefix > accepted.bytes.len()
            || unchanged_prefix > bytes.len()
            || accepted.bytes[..unchanged_prefix] != bytes[..unchanged_prefix]
        {
            return Err(SourceRegistrationError::RestartPrefixMismatch {
                anchor: u64::try_from(unchanged_prefix).unwrap_or(u64::MAX),
            });
        }
        let raw = u32::try_from(self.input.next_source_identity)
            .map_err(|_| SourceRegistrationError::SourceIdentityExhausted)?;
        let id = tex_state::SourceId::new(raw);
        let replacement = accepted.rebind_generated(id, bytes)?;
        self.timeline
            .record_next_source_identity(self.input.next_source_identity);
        self.input.next_source_identity += 1;
        let rebound = self
            .input
            .levels
            .rebind_physical_source(source, replacement);
        debug_assert!(rebound, "the validated root source remains present");
        Ok(id)
    }

    /// Rehomes the accepted editor backing after convergence. This runs only
    /// after the current candidate has been rejected and does not create a
    /// command-journal branch or another generation.
    #[doc(hidden)]
    pub fn rehome_generated_editor_source(
        &mut self,
        accepted: &[u8],
        bytes: std::sync::Arc<[u8]>,
        old_start: usize,
        old_end: usize,
        new_end: usize,
    ) -> Result<(), SourceRegistrationError> {
        if !self
            .input
            .levels
            .rehome_generated_source(accepted, bytes, old_start, old_end, new_end)?
        {
            return Err(SourceRegistrationError::RestartPrefixMismatch { anchor: 0 });
        }
        Ok(())
    }

    fn take_registered_source(
        &mut self,
        source: tex_state::SourceId,
    ) -> Result<RegisteredSource, UnknownRegisteredSource> {
        let registered = self
            .input
            .pending_sources
            .remove(&source.raw())
            .ok_or(UnknownRegisteredSource(source))?;
        Ok(registered)
    }

    /// Opens an already registered source as a text file, the way tex.web
    /// §537's `start_input` does.
    ///
    /// §537 is how TeX reaches every `\input` file _and_ the job's own root
    /// file, so [`SourceNameClass::File`] is the classification of an ordinary
    /// open. The terminal and `\read`'s streams are the two levels TeX opens
    /// some other way (§331 and §483); they use
    /// [`Self::open_registered_source_as`].
    ///
    /// This operation consumes the registered immutable backing. A source ID
    /// is therefore openable exactly once; the live level becomes its sole
    /// command-state owner. It cannot search for files, invoke a host
    /// callback, or diagnose text encoding.
    pub fn open_registered_source(
        &mut self,
        source: tex_state::SourceId,
    ) -> Result<(), UnknownRegisteredSource> {
        self.open_registered_source_as(source, SourceNameClass::File)
    }

    /// Opens a nested file with its e-TeX nesting ancestry already owned by
    /// the frame that becomes visible on the input stack.
    pub(crate) fn open_registered_file_with_depths(
        &mut self,
        source: tex_state::SourceId,
        open_depths: crate::input::SourceOpenDepths,
    ) -> Result<(InputLevelId, Option<std::sync::Arc<str>>), UnknownRegisteredSource> {
        let registered = self.take_registered_source(source)?;
        let framing_name = registered.canonical_framing_name();
        let identity = self.push_source_level(
            registered,
            SourceNameClass::File,
            crate::input::SourceRetirement::Pop,
            None,
            Some(open_depths),
        );
        Ok((identity, framing_name))
    }

    /// Opens an already registered source under an explicit tex.web §303
    /// `name` classification, consuming the source's pending backing.
    pub fn open_registered_source_as(
        &mut self,
        source: tex_state::SourceId,
        name_class: SourceNameClass,
    ) -> Result<(), UnknownRegisteredSource> {
        let registered = self.take_registered_source(source)?;
        self.push_source_level(
            registered,
            name_class,
            crate::input::SourceRetirement::Pop,
            None,
            None,
        );
        Ok(())
    }

    /// Opens one acquired line as tex.web §483's `\read` pseudo-file.
    ///
    /// §483 runs `begin_file_reading; name:=m+1` around a single line, then
    /// `state:=new_line` and `loop get_token` until the line ends. The level
    /// is an ordinary source level -- live category codes, control-sequence
    /// spelling, and `^^` notation all apply -- and differs only in what its
    /// exhaustion means (§360's `cur_tok=0`), which is why the difference is
    /// carried as its retirement and nothing else.
    pub(crate) fn begin_read_line(&mut self) -> Result<InputLevelId, SourceRegistrationError> {
        let source = self.register_source(SourceRegistration::new(
            RegisteredSourceKind::ReadLine,
            &b""[..],
        ))?;
        let registered = self
            .take_registered_source(source)
            .expect("a source registered above is present");
        let identity = self.push_source_level(
            registered,
            SourceNameClass::Terminal,
            crate::input::SourceRetirement::EndReadLine,
            None,
            None,
        );
        Ok(identity)
    }

    /// Pushes tex.web §87's replacement line above the suspended input.
    pub(crate) fn open_error_insert_line(
        &mut self,
        bytes: impl Into<std::sync::Arc<[u8]>>,
    ) -> Result<(), SourceRegistrationError> {
        let source = self.register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            bytes,
        ))?;
        let registered = self
            .take_registered_source(source)
            .expect("a source registered above is present");
        let identity = self.push_source_level(
            registered,
            SourceNameClass::Terminal,
            crate::input::SourceRetirement::Pop,
            None,
            None,
        );
        self.input
            .levels
            .mutate_top_source(|active, slot| {
                assert_eq!(active.identity(), identity);
                let stored = crate::input::SourceLevelExecutionState::cursor(active, slot);
                slot.cursor.pending_acquired_line = true;
                (stored, ())
            })
            .expect("the inserted replacement source was just pushed");
        Ok(())
    }

    /// Returns the canonical §537 framing name of one live file source.
    ///
    /// Startup drivers call this at the selector-visible root-open boundary;
    /// the name remains owned by the source level rather than by a pending
    /// effect carrier.
    #[must_use]
    pub fn live_file_framing_name(&self, source: tex_state::SourceId) -> Option<&str> {
        self.input.levels.iter().find_map(|level| {
            let InputLevel::Source(level) = level else {
                return None;
            };
            let backing = self
                .input
                .levels
                .source_level_slot(level)
                .cursor
                .current_backing();
            (backing.id == source
                && self.input.levels.source_level_slot(level).name_class == SourceNameClass::File
                && backing.framing == crate::SourceFramingPolicy::Canonical)
                .then(|| backing.framing_name.as_deref().or(backing.name.as_deref()))
                .flatten()
        })
    }

    /// Installs the immutable bytes acquired for an already-active §483
    /// `begin_file_reading` level.
    pub(crate) fn finish_read_line(
        &mut self,
        level: InputLevelId,
        name_class: SourceNameClass,
        bytes: impl Into<std::sync::Arc<[u8]>>,
    ) -> Result<(), SourceRegistrationError> {
        let source = self.register_source(SourceRegistration::new(
            RegisteredSourceKind::ReadLine,
            bytes,
        ))?;
        let registered = self
            .take_registered_source(source)
            .expect("a source registered above is present");
        self.input
            .levels
            .mutate_top_source(|active, slot| {
                assert_eq!(
                    active.identity(),
                    level,
                    "begin_read_line keeps the exact source level active during acquisition"
                );
                let stored =
                    crate::input::SourceLevelExecutionState::backing(active, slot, registered);
                slot.name_class = name_class;
                slot.cursor.pending_acquired_line = true;
                (stored, ())
            })
            .expect("begin_read_line keeps its source level active during acquisition");
        Ok(())
    }

    /// Opens e-TeX 2.6 etex.ch §53a's generated `\scantokens` pseudo-file.
    pub(crate) fn open_scantokens(
        &mut self,
        registration: SourceRegistration,
        every_eof: Option<tex_state::TokenListId<G>>,
        numeric_name: u8,
        open_depths: crate::input::SourceOpenDepths,
    ) -> Result<(InputLevelId, Option<std::sync::Arc<str>>), SourceRegistrationError> {
        assert!(matches!(numeric_name, 18 | 19));
        let source = self.register_source(registration)?;
        let registered = self
            .take_registered_source(source)
            .expect("a source registered above is present");
        let framing_name = (numeric_name == 19).then(|| std::sync::Arc::from(" "));
        let identity = self.push_source_level(
            registered,
            SourceNameClass::Scantokens(numeric_name),
            crate::input::SourceRetirement::Pop,
            every_eof,
            Some(open_depths),
        );
        Ok((identity, framing_name))
    }

    /// Pushes e-TeX §24.362's `\everyeof` above its exhausted pseudo-file.
    pub(crate) fn begin_pending_every_eof(
        &mut self,
        stores: &tex_state::CommandContext<'_, G>,
        source: InputLevelId,
    ) -> Option<InputLevelId> {
        let InputLevel::Source(level) = self.input.levels.last()? else {
            return None;
        };
        if level.identity() != source {
            return None;
        }
        let every_eof = self
            .input
            .levels
            .source_level_slot(level)
            .every_eof
            .as_ref()?
            .clone();
        let retained_line = self
            .input
            .levels
            .mutate_top_source(|level, slot| {
                let stored = crate::input::SourceLevelExecutionState::every_eof(level, slot);
                if matches!(slot.name_class, SourceNameClass::Scantokens(_)) {
                    slot.cursor.install_scantokens_eof_context_line();
                }
                let retained_line = slot
                    .cursor
                    .line
                    .as_ref()
                    .map(|line| line.physical.number().min(i32::MAX as u64) as i32);
                (stored, retained_line)
            })
            .expect("the checked everyeof source remains on top");
        if let Some(retained_line) = retained_line {
            self.set_retained_file_line_number(retained_line);
        }
        let words = stores.token_list(every_eof);
        Some(self.push_token_level(
            PackedTokenSpanHandle::durable(words),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Stored(StoredReplayReason::EveryEof),
        ))
    }

    /// Pushes one source level and returns any canonical file-like opening.
    ///
    /// This is the one place a source level enters the input stack, so
    /// Ordinary files use their resolved name. e-TeX's traced `\scantokens`
    /// pseudo-file uses one space as its name; numeric name 18 remains silent.
    fn push_source_level(
        &mut self,
        registered: RegisteredSource,
        name_class: SourceNameClass,
        retirement: crate::input::SourceRetirement,
        every_eof: Option<tex_state::TokenListId<G>>,
        open_depths: Option<crate::input::SourceOpenDepths>,
    ) -> InputLevelId {
        let identity = self.allocate_input_level_identity();
        let enclosing_source = self.input.levels.current_source_context();
        let resolved_role = registered
            .role
            .or_else(|| enclosing_source.map(tex_state::packed_input::SourceContext::role))
            .unwrap_or(crate::SourceRole::GeneratedInput);
        let own_source = tex_state::packed_input::SourceContext::new(registered.id, resolved_role);
        let source_context = match name_class {
            SourceNameClass::File | SourceNameClass::Scantokens(_) => Some(own_source),
            SourceNameClass::Terminal | SourceNameClass::ReadStream(_) => enclosing_source,
        };
        let mut frame = crate::input::PackedInputFrame::source(identity.0, own_source);
        frame.set_source_context(source_context);
        self.stack_usage.input_stack = self.stack_usage.input_stack.max(self.input.levels.len());
        self.input.levels.push_source(
            frame,
            crate::input::SourceSlot::new(
                SourceCursor::new(registered),
                every_eof,
                open_depths,
                name_class,
                retirement,
            ),
        );
        self.set_retained_file_line_number(0);
        identity
    }

    pub(crate) fn current_source_open_depths(&self) -> Option<&crate::input::SourceOpenDepths> {
        self.input
            .levels
            .iter()
            .rev()
            .find_map(|entry| match entry {
                InputLevel::Source(source) => self
                    .input
                    .levels
                    .source_level_slot(source)
                    .open_depths
                    .as_ref(),
                _ => None,
            })
    }

    /// Borrows opening ancestry only when `identity` names the current top
    /// source. Input retirement has already validated this same top identity;
    /// ordinary token and macro retirement must not search through their
    /// enclosing input stack for source-only state.
    pub(crate) fn top_source_open_depths(
        &self,
        identity: InputLevelId,
    ) -> Option<&crate::input::SourceOpenDepths> {
        #[cfg(test)]
        RETIREMENT_TOP_SOURCE_CHECKS.with(|checks| checks.set(checks.get().saturating_add(1)));
        self.input.levels.last().and_then(|entry| match entry {
            InputLevel::Source(source) if source.identity() == identity => self
                .input
                .levels
                .source_level_slot(source)
                .open_depths
                .as_ref(),
            InputLevel::Source(_) | InputLevel::Tokens(_) | InputLevel::MacroArgument(_) => None,
        })
    }

    /// Applies TeX's `\endinput` retirement request to the active physical
    /// source.  The remainder of its current line is still tokenized; no
    /// later physical line may be loaded.
    pub(crate) fn end_current_source_after_current_line(&mut self) -> bool {
        let has_source = self
            .input
            .levels
            .iter()
            .rev()
            .find_map(|level| match level {
                InputLevel::Source(level) => Some(level),
                InputLevel::Tokens(_) | InputLevel::MacroArgument(_) => None,
            })
            .is_some();
        if has_source {
            self.timeline.record_force_eof(self.input.force_eof);
            self.input.force_eof = true;
        }
        has_source
    }

    fn push_alignment_template(
        &mut self,
        stores: &tex_state::CommandContext<'_, G>,
        template: tex_state::TokenListId<G>,
        behavior: TokenBehavior,
        retirement: RetirementBehavior,
        trace: ReplayTrace,
    ) -> InputLevelId {
        self.push_token_level(
            PackedTokenSpanHandle::durable(stores.token_list(template)),
            behavior,
            retirement,
            trace,
        )
    }

    pub(crate) fn record_csname_buffer_usage(&mut self, name_len: usize) {
        if name_len == 0 {
            return;
        }
        let occupied = self.input.levels.occupied_source_buffer_slots();
        // §374 starts at `first`; §1334 adds one to the greatest written
        // buffer index.
        self.stack_usage.record_buffer_usage(
            self.terminal_buffer_slots
                .saturating_add(occupied)
                .saturating_add(name_len)
                .saturating_add(2),
        );
    }

    /// Reads one byte-domain character or decoded Unicode scalar from the
    /// active normalized line with its exact physical range.
    pub fn next_source_character(&mut self) -> Option<SourceCharacter> {
        self.input.levels.mutate_top_source_lex(|_, slot| {
            let cursor = &mut slot.cursor;
            let backing = match cursor.line_backing.as_ref() {
                Some(replacement) => replacement,
                None => &cursor.backing,
            };
            let mode = backing.mode;
            let bytes = backing.bytes.as_ref();
            cursor.line.as_mut()?.next_character(mode, bytes)
        })?
    }

    /// Captures the active source's immutable identity and bytes for detached
    /// observation without exposing the live source level.
    pub(crate) fn active_source_snapshot(
        &self,
    ) -> Option<crate::observation::OpenedSourceSnapshot> {
        let Some(InputLevel::Source(level)) = self.input.levels.last() else {
            return None;
        };
        let backing = self
            .input
            .levels
            .source_level_slot(level)
            .cursor
            .current_backing();
        Some(crate::observation::OpenedSourceSnapshot {
            id: backing.id,
            bytes: std::sync::Arc::clone(&backing.bytes),
        })
    }

    /// Retires the active normalized line so the next physical line may load.
    pub fn finish_source_line(&mut self) {
        let _ = self.input.levels.mutate_top_source(|level, slot| {
            let stored = crate::input::SourceLevelExecutionState::cursor(level, slot);
            slot.cursor.finish_line();
            (stored, ())
        });
    }

    /// Tokenizes one exact-byte source step using the caller's live catcodes.
    ///
    /// The callback is queried independently for every classified character;
    /// it is not retained or cached across tokens. Invalid characters are
    /// returned as recoverable steps after their complete spelling is
    /// consumed.
    ///
    /// # Panics
    ///
    /// Panics when called for the separately implemented Unicode character
    /// profile.
    pub fn next_exact_source_step(
        &mut self,
        endlinechar: i32,
        queries: &mut dyn crate::SourceStepQueries,
    ) -> SourceTokenizationStep {
        let profile = self.profile();
        assert_eq!(
            profile.character_mode(),
            crate::CharacterMode::EightBitExact,
            "exact-byte tokenization requires an exact-byte command profile"
        );
        loop {
            let force_eof = self.source_force_eof();
            let Some(step) = self.input.levels.mutate_top_source_lex(|_, slot| {
                slot.cursor.next_exact_byte_step(force_eof, queries)
            }) else {
                return SourceTokenizationStep::End;
            };
            match step {
                crate::input::CursorSourceTokenizationStep::Token(token) => {
                    return SourceTokenizationStep::Token(token);
                }
                crate::input::CursorSourceTokenizationStep::InvalidCharacter(invalid) => {
                    return SourceTokenizationStep::InvalidCharacter(invalid);
                }
                crate::input::CursorSourceTokenizationStep::End => {
                    return SourceTokenizationStep::End;
                }
                crate::input::CursorSourceTokenizationStep::NeedLine => {}
            }
            if self
                .acquire_active_source_line_with_queries(endlinechar, queries, true)
                .is_none()
            {
                return SourceTokenizationStep::End;
            }
        }
    }

    /// Tokenizes one Unicode-scalar source step using the caller's live code
    /// table.
    ///
    /// The callback receives only Unicode-domain [`crate::CharacterCode`]
    /// values, including synthetic `endlinechar` and superscript-reduction
    /// results. Sparse-table defaults belong to the aggregate code table, not
    /// this tokenizer.
    ///
    /// # Panics
    ///
    /// Panics when called for an exact-byte command profile.
    pub fn next_unicode_source_step(
        &mut self,
        endlinechar: i32,
        queries: &mut dyn crate::SourceStepQueries,
    ) -> SourceTokenizationStep {
        let profile = self.profile();
        assert_eq!(
            profile.character_mode(),
            crate::CharacterMode::UnicodeExtended,
            "Unicode tokenization requires a UnicodeExtended command profile"
        );
        loop {
            let force_eof = self.source_force_eof();
            let Some(step) = self
                .input
                .levels
                .mutate_top_source_lex(|_, slot| slot.cursor.next_unicode_step(force_eof, queries))
            else {
                return SourceTokenizationStep::End;
            };
            match step {
                crate::input::CursorSourceTokenizationStep::Token(token) => {
                    return SourceTokenizationStep::Token(token);
                }
                crate::input::CursorSourceTokenizationStep::InvalidCharacter(invalid) => {
                    return SourceTokenizationStep::InvalidCharacter(invalid);
                }
                crate::input::CursorSourceTokenizationStep::End => {
                    return SourceTokenizationStep::End;
                }
                crate::input::CursorSourceTokenizationStep::NeedLine => {}
            }
            if self
                .acquire_active_source_line_with_queries(endlinechar, queries, true)
                .is_none()
            {
                return SourceTokenizationStep::End;
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn next_compact_exact_source_step(
        &mut self,
        endlinechar: i32,
        queries: &mut dyn CompactSourceStepQueries,
    ) -> CompactSourceTokenizationStep {
        loop {
            let force_eof = self.source_force_eof();
            let Some(step) = self.input.levels.mutate_top_source_lex(|_, slot| {
                slot.cursor.next_compact_exact_byte_step(force_eof, queries)
            }) else {
                return CompactSourceTokenizationStep::End;
            };
            if !matches!(step, CompactSourceTokenizationStep::NeedLine) {
                return step;
            }
            if self
                .acquire_active_source_line_with_queries(endlinechar, queries, true)
                .is_none()
            {
                return CompactSourceTokenizationStep::End;
            }
        }
    }

    pub(crate) fn source_force_eof(&self) -> bool {
        self.input.force_eof
            && self
                .input
                .levels
                .iter()
                .rev()
                .find_map(|level| match level {
                    InputLevel::Source(source) => Some(
                        self.input.levels.source_level_slot(source).name_class
                            == SourceNameClass::File,
                    ),
                    InputLevel::Tokens(_) | InputLevel::MacroArgument(_) => None,
                })
                == Some(true)
    }

    fn acquire_active_source_line_with_queries(
        &mut self,
        endlinechar: i32,
        queries: &mut dyn crate::SourceStepQueries,
        firm: bool,
    ) -> Option<PhysicalLine> {
        self.acquire_input_top_line_with_queries(endlinechar, firm, false, queries)
            .ok()
            .flatten()
    }

    /// Returns the immutable profile selected when this job was created.
    #[must_use]
    pub fn profile(&self) -> CommandProfile {
        self.expansion.profile
    }

    /// Returns the profile component required in portable format identity.
    #[must_use]
    pub fn format_profile_fingerprint(&self) -> CommandProfileFingerprint {
        self.profile().fingerprint()
    }

    /// Rejects a format image produced for a different command profile.
    pub fn validate_format_profile(
        &self,
        found: CommandProfileFingerprint,
    ) -> Result<(), CommandProfileMismatch> {
        self.profile()
            .validate_fingerprint(CommandProfileBoundary::Format, found)
    }

    /// Returns the profile component required in incremental checkpoint identity.
    #[must_use]
    pub fn checkpoint_profile_fingerprint(&self) -> CommandProfileFingerprint {
        self.profile().fingerprint()
    }

    /// Rejects a checkpoint produced for a different command profile.
    pub fn validate_checkpoint_profile(
        &self,
        found: CommandProfileFingerprint,
    ) -> Result<(), CommandProfileMismatch> {
        self.profile()
            .validate_fingerprint(CommandProfileBoundary::Checkpoint, found)
    }
}

/// An input level referred to a source absent from retained registration.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct UnknownRegisteredSource(tex_state::SourceId);

impl UnknownRegisteredSource {
    /// Returns the missing source identity.
    #[must_use]
    pub const fn source(self) -> tex_state::SourceId {
        self.0
    }
}

impl std::fmt::Display for UnknownRegisteredSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "source identity {} is not registered",
            self.0.raw()
        )
    }
}

impl std::error::Error for UnknownRegisteredSource {}

/// Live temporary data referenced by persistent command state.
///
/// Builder contents and rollback roots are semantic while live.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct TransientState {
    pub(crate) builders: Vec<LiveTokenBuilder>,
    pub(crate) rollback_roots: Vec<u64>,
    pub(crate) next_builder_identity: u64,
    /// Nesting of the call-local expansion episode currently borrowing the
    /// command machine. This records only quiescence, never a continuation,
    /// accumulator, fuel scope, host capability, or processor borrow.
    pub(crate) active_expansion_depth: u32,
}

/// One semantic token builder named by a scanner-status variant.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct LiveTokenBuilder {
    pub(crate) identity: u64,
    pub(crate) tokens: crate::attempt::AttemptTokenBufferId,
}

#[cfg(test)]
mod tests;
