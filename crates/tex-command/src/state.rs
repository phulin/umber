//! Future-relevant state and discardable scratch allocation ownership.

use std::hash::{Hash, Hasher};
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tex_state::CommandContext;
use tex_state::token::TracedTokenWord;
use tex_state::{GroupFrame, GroupKind, StateError};

use crate::AlignmentRecord;
use crate::conditionals::ConditionStack;
use crate::input::InputState;
use crate::input::{CompactSourceStepQueries, CompactSourceTokenizationStep};
use crate::input::{
    FileFramingEvent, InputLevel, InputLevelId, PhysicalLine, RegisteredSource,
    RegisteredSourceKind, SourceCharacter, SourceCursor, SourceLevel, SourceNameClass,
    SourceRegistration, SourceRegistrationError, SourceTokenizationStep,
};
use crate::input::{
    ReplayTrace, RetirementBehavior, StoredReplayReason, TokenBehavior, TokenPayload,
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

#[cfg(test)]
thread_local! {
    static COMMAND_ROOT_CLONES: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn command_root_clone_count() -> usize {
    COMMAND_ROOT_CLONES.get()
}

#[cfg(test)]
pub(crate) fn reset_command_root_clone_count() {
    COMMAND_ROOT_CLONES.set(0);
}

fn stored_replay_name(reason: StoredReplayReason) -> &'static str {
    match reason {
        StoredReplayReason::EveryPar => "everypar",
        StoredReplayReason::EveryMath => "everymath",
        StoredReplayReason::EveryDisplay => "everydisplay",
        StoredReplayReason::EveryHBox => "everyhbox",
        StoredReplayReason::EveryVBox => "everyvbox",
        StoredReplayReason::EveryJob => "everyjob",
        StoredReplayReason::EveryCr => "everycr",
        StoredReplayReason::OutputRoutine
        | StoredReplayReason::EveryEof
        | StoredReplayReason::Mark
        | StoredReplayReason::Write
        | StoredReplayReason::Discretionary => {
            unreachable!("only executor-requested named lists are queued here")
        }
    }
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
    /// Executor-owned stored levels that remain live in the input stack.
    pub(crate) replay_completions: Vec<InputLevelId>,
    /// Retired stored levels whose descendants still own input above the
    /// enclosing source.  TeX82 §§390 can retire such a level immediately
    /// before installing a macro replacement, and main control may end the
    /// processor borrow on an unexpandable replacement command.  The pending
    /// completion fence is therefore future-relevant command state.
    pub(crate) pending_replay_completions: Vec<InputLevelId>,
    /// Semantic diagnostics committed by command processing but rendered by
    /// the executor's World-facing diagnostic boundary.
    ///
    /// This queue is unconditional command state, not observation state.
    /// Consequently an unobserved episode has identical semantics, while the
    /// ordinary command snapshot makes a failed aggregate operation restore
    /// the queue together with the input transition that produced it.
    pub(crate) semantic_diagnostics: Vec<CommandSemanticDiagnostic>,
    /// TeX82 §§280--282 `insert_token` payloads paired with the exact state
    /// save level that owns them. Frames and words are generation-branded by
    /// this aggregate root; no payload registry or per-value owner exists.
    pub(crate) group_payloads: Vec<CommandGroupPayload<G>>,
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
    /// tex.web §537/§362 file-bracketing transitions, in the order they
    /// happened, waiting for the engine to render them as `(name`/`)`.
    ///
    /// This lives on [`CommandState`] rather than on the short-lived
    /// [`crate::CommandProcessor`] borrow that
    /// `take_restricted_integer_recoveries` uses, and deliberately so. A
    /// file's open and its eventual retirement are not generally the same
    /// executor step -- a source stays live, and typically outlives many
    /// processor episodes, between `\input` and its last line -- so a
    /// processor-local accumulator would already have lost the open event by
    /// the time the close event is due, or would need its own cross-step
    /// carry mechanism duplicating this one.
    ///
    /// Placing it here is safe under rollback for the same reason
    /// `named_token_list_pushes` above already is: checkpoint publication
    /// explicitly forks an isolated aggregate root, and rollback clones that
    /// retained root into the live command machine before truncating attempt
    /// storage. A queued event from a rolled-back step therefore disappears
    /// with every other command-state mutation from that step, while a
    /// committed step's events survive until the executor drains them with
    /// [`Self::take_file_framing_events`].
    pub(crate) file_framing_events: Vec<FileFramingEvent>,
}

impl<G> Clone for CommandStateRoots<G> {
    fn clone(&self) -> Self {
        #[cfg(test)]
        COMMAND_ROOT_CLONES.set(COMMAND_ROOT_CLONES.get().saturating_add(1));
        Self {
            engine_semantics: self.engine_semantics,
            input: self.input.clone(),
            parameters: self.parameters.clone(),
            scanner: self.scanner.clone(),
            conditions: self.conditions.clone(),
            alignment: self.alignment.clone(),
            expansion: self.expansion.clone(),
            transient: self.transient.clone(),
            replay_completions: self.replay_completions.clone(),
            pending_replay_completions: self.pending_replay_completions.clone(),
            semantic_diagnostics: self.semantic_diagnostics.clone(),
            group_payloads: self.group_payloads.clone(),
            afterassignment: self.afterassignment,
            name_in_progress: self.name_in_progress,
            pending_input_open: self.pending_input_open.clone(),
            named_token_list_pushes: self.named_token_list_pushes.clone(),
            file_framing_events: self.file_framing_events.clone(),
        }
    }
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
    pub(crate) timeline: Arc<crate::snapshot::CommandTimeline<G>>,
    /// Runtime-only TeX82 stack accounting. Snapshot roots deliberately omit
    /// this tracker so high-water marks survive rollback without becoming
    /// command semantics or checkpoint identity.
    pub(crate) usage: CommandUsageTracker,
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
}

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
            engine_semantics: CommandEngineSemantics::default(),
            input: InputState::default(),
            parameters: ParameterState::default(),
            scanner: ScannerState::default(),
            conditions: ConditionStack::default(),
            alignment: AlignmentDeliveryState::default(),
            expansion: ExpansionState::default(),
            transient: TransientState::default(),
            replay_completions: Vec::new(),
            pending_replay_completions: Vec::new(),
            semantic_diagnostics: Vec::new(),
            group_payloads: Vec::new(),
            afterassignment: None,
            name_in_progress: false,
            pending_input_open: None,
            named_token_list_pushes: Vec::new(),
            file_framing_events: Vec::new(),
        }
    }
}

impl<G> Default for CommandState<G> {
    fn default() -> Self {
        Self {
            roots: CommandStateRoots::default(),
            timeline: Arc::new(crate::snapshot::CommandTimeline::default()),
            usage: CommandUsageTracker::default(),
            attempt: crate::CommandAttempt::default(),
            scratch: crate::execution_scratch::ExecutionScratch::default(),
            active_attempt_operation: None,
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

#[derive(Clone, Default)]
pub(crate) struct CommandUsageTracker {
    maxima: Arc<Mutex<CommandStackUsage>>,
    terminal_buffer_slots: Arc<AtomicUsize>,
}

impl std::fmt::Debug for CommandUsageTracker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CommandUsageTracker(..)")
    }
}

impl PartialEq for CommandUsageTracker {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for CommandUsageTracker {}

impl Hash for CommandUsageTracker {
    fn hash<H: Hasher>(&self, _state: &mut H) {}
}

impl CommandUsageTracker {
    pub(crate) fn record_input_push(&self, input_ptr: usize) {
        let mut usage = self
            .maxima
            .lock()
            .expect("command usage mutex is not poisoned");
        usage.input_stack = usage.input_stack.max(input_ptr);
    }

    pub(crate) fn record_parameter_push(&self, param_ptr_after: usize) {
        let mut usage = self
            .maxima
            .lock()
            .expect("command usage mutex is not poisoned");
        usage.parameter_stack = usage.parameter_stack.max(param_ptr_after);
    }

    pub(crate) fn record_buffer_usage(&self, buffer_positions: usize) {
        let mut usage = self
            .maxima
            .lock()
            .expect("command usage mutex is not poisoned");
        usage.buffer_stack = usage.buffer_stack.max(buffer_positions);
    }

    pub(crate) fn get(&self) -> CommandStackUsage {
        *self
            .maxima
            .lock()
            .expect("command usage mutex is not poisoned")
    }

    fn set_terminal_buffer_slots(&self, slots: usize) {
        self.terminal_buffer_slots.store(slots, Ordering::Relaxed);
    }

    fn terminal_buffer_slots(&self) -> usize {
        self.terminal_buffer_slots.load(Ordering::Relaxed)
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
    /// render it faithfully after the borrow ends. `identity` is the same
    /// `back_error` accounting code recorded in `pending_diagnostics`, kept
    /// so a report and its recovery remain correlatable.
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
    pub(crate) tokens: Vec<CommandPayload<G>>,
    /// State-journal position of this level's newest §276 push.
    ///
    /// Tokens remain command-owned. This scalar only orders their newest
    /// physical save word against state-owned group/restore records.
    latest_aftergroup_position: Option<u32>,
}

impl<G> Clone for CommandGroupPayload<G> {
    fn clone(&self) -> Self {
        Self {
            frame: self.frame,
            tokens: self.tokens.clone(),
            latest_aftergroup_position: self.latest_aftergroup_position,
        }
    }
}

impl<G> CommandGroupPayload<G> {
    const fn new(frame: GroupFrame) -> Self {
        Self {
            frame,
            tokens: Vec::new(),
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
        self.group_payloads.push(CommandGroupPayload::new(frame));
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
        group.tokens.push(CommandPayload::new(spelling));
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
                    words.saturating_add(group.tokens.len()),
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
            .pop()
            .expect("validated command group frame remains present");
        let aftergroup = group
            .tokens
            .into_iter()
            .map(|payload| payload.spelling)
            .collect();
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
    ) -> Result<&[tex_state::token::TracedTokenWord], crate::AttemptError> {
        self.attempt.arena().token_words(id)
    }

    /// Promotes one declared token-list escape root into generation-durable
    /// storage. No unrelated attempt row is inspected or copied.
    pub fn promote_attempt_token_list(
        &self,
        universe: &mut tex_state::Universe<G>,
        id: crate::AttemptTokenListId,
    ) -> Result<tex_state::TokenListId<G>, crate::AttemptError> {
        let promotion = self.promote_attempt_roots(
            universe,
            crate::AttemptPromotionRoots::new(core::slice::from_ref(&id), &[], &[], &[]),
        )?;
        Ok(promotion.token_lists[0].clone())
    }

    /// Atomically promotes every declared attempt-local root into this
    /// generation's durable stores.
    ///
    /// The command attempt validates the complete request before reserving or
    /// publishing destination rows. On success, every receipt vector retains
    /// the corresponding request slice's order, including duplicates.
    pub fn promote_attempt_roots(
        &self,
        universe: &mut tex_state::Universe<G>,
        roots: crate::AttemptPromotionRoots<'_, G>,
    ) -> Result<crate::AttemptPromotionReceipt<G>, crate::AttemptError> {
        let promotion = self.attempt.arena().promote(
            universe,
            crate::attempt::AttemptEscapeRoots {
                token_lists: roots.token_lists,
                glue: roots.glue,
                definitions: roots.definitions,
                provenance: roots.provenance,
            },
        )?;
        Ok(crate::AttemptPromotionReceipt {
            token_lists: promotion.token_lists,
            glue: promotion.glue,
            definitions: promotion.definitions,
            provenance: promotion.provenance,
        })
    }

    /// Promotes one declared macro-definition root and its schema-owned text
    /// into the current generation's definition arena.
    pub fn promote_attempt_definition(
        &self,
        universe: &mut tex_state::Universe<G>,
        id: crate::AttemptDefinitionId,
    ) -> Result<tex_state::DefinitionId<G>, crate::AttemptError> {
        self.attempt.arena().promote_definition(universe, id)
    }

    /// Captures every attempt-local table and subordinate builder cursor for
    /// an executor operation.
    pub fn begin_attempt_operation(&mut self) -> crate::CommandAttemptOperation {
        assert!(
            self.active_attempt_operation.is_none(),
            "direct command operations do not nest"
        );
        let mark = self
            .attempt
            .begin_operation(self.parameters.activations.len())
            .expect("command operation scope capacity is bounded");
        self.active_attempt_operation = Some(mark);
        crate::CommandAttemptOperation::new(mark)
    }

    /// Opens one move-only synchronous child of the active direct operation.
    ///
    /// The child is attempt scratch only. Callers may consume its values while
    /// it is live, but must detach their final non-attempt result before
    /// [`Self::close_attempt_child_scope`] consumes the receipt. Semantic
    /// command mutations deliberately remain in the parent operation.
    pub fn begin_attempt_child_scope(
        &mut self,
    ) -> Result<crate::CommandAttemptChildScope, crate::AttemptError> {
        self.active_attempt_operation
            .ok_or(crate::AttemptError::InvalidCoordinate)?;
        let owner = self.attempt.begin_child_scope()?;
        Ok(crate::CommandAttemptChildScope::new(owner))
    }

    /// Consumes and closes exactly one synchronous LIFO child scope.
    pub fn close_attempt_child_scope(
        &mut self,
        scope: crate::CommandAttemptChildScope,
    ) -> Result<(), crate::AttemptError> {
        self.attempt.close_child_scope(scope.into_owner())
    }

    pub(crate) fn begin_attempt_scanner_scope(
        &mut self,
    ) -> Result<crate::attempt::OwnedAttemptScope, crate::AttemptError> {
        self.attempt.begin_child_scope()
    }

    pub(crate) fn defer_attempt_scope_retirement(
        &mut self,
        scope: crate::attempt::OwnedAttemptScope,
    ) -> Result<(), crate::AttemptError> {
        self.active_attempt_operation
            .ok_or(crate::AttemptError::InvalidCoordinate)?;
        if self.attempt.child_scope_is_direct_operation_child(&scope) {
            self.attempt.defer_child_to_operation(scope)
        } else {
            self.attempt.close_child_scope(scope)
        }
    }

    pub(crate) fn discard_attempt_scope_suffix(
        &mut self,
        scope: crate::attempt::OwnedAttemptScope,
    ) -> Result<(), crate::AttemptError> {
        self.attempt.close_child_scope(scope)
    }

    /// Rejects the attempt-local suffix created after `mark`.
    ///
    /// Executor aggregate rollback restores semantic roots before invoking
    /// this method, so no surviving command coordinate can name the suffix.
    pub fn rollback_attempt_operation(
        &mut self,
        operation: crate::CommandAttemptOperation,
    ) -> Result<(), crate::AttemptError> {
        let mark = operation.coordinate();
        if self.active_attempt_operation != Some(mark) {
            return Err(crate::AttemptError::InvalidCoordinate);
        }
        while self.parameters.activations.len() > mark.macro_depth() {
            let activation = self
                .parameters
                .retire_last_activation()
                .ok_or(crate::AttemptError::InvalidCoordinate)?;
            self.scratch
                .pop_macro_frame(activation.arguments.frame())
                .map_err(|_| crate::AttemptError::InvalidCoordinate)?;
        }
        self.attempt.rollback_operation(mark)?;
        self.active_attempt_operation = None;
        Ok(())
    }

    /// Commits the exact direct-operation/scanner scope. Macro frames live in
    /// the disjoint generation-owned scratch lanes until input retirement.
    pub fn commit_attempt_operation(
        &mut self,
        operation: crate::CommandAttemptOperation,
    ) -> Result<(), crate::AttemptError> {
        let mark = operation.coordinate();
        if self.active_attempt_operation != Some(mark) {
            return Err(crate::AttemptError::InvalidCoordinate);
        }
        self.attempt.commit_operation(mark)?;
        self.active_attempt_operation = None;
        Ok(())
    }

    /// Moves the complete operation arena into a resource continuation.
    pub fn suspend_attempt<R>(
        &mut self,
        universe: &tex_state::Universe<G>,
        operation: crate::CommandAttemptOperation,
        resume: crate::AttemptResumePoint,
        pending: R,
    ) -> Result<crate::PendingCommandAttempt<G, R>, crate::AttemptSuspendFailure> {
        let opening = operation.coordinate();
        if self.active_attempt_operation != Some(opening) {
            return Err(crate::AttemptSuspendFailure::new(
                operation,
                crate::AttemptSuspendError::StaleMark(crate::AttemptError::InvalidCoordinate),
            ));
        }
        if let Err(error) = self.attempt.arena().validate_mark(opening.attempt_mark()) {
            return Err(crate::AttemptSuspendFailure::new(
                operation,
                crate::AttemptSuspendError::StaleMark(error),
            ));
        }
        if let Err(error) = self.attempt.validate_operation(opening) {
            return Err(crate::AttemptSuspendFailure::new(
                operation,
                crate::AttemptSuspendError::StaleMark(error),
            ));
        }
        let generation = match universe.generation_owner() {
            Ok(generation) => generation,
            Err(error) => {
                return Err(crate::AttemptSuspendFailure::new(
                    operation,
                    crate::AttemptSuspendError::Generation(error),
                ));
            }
        };
        let attempt = core::mem::take(&mut self.attempt);
        Ok(crate::PendingCommandAttempt::new_at_validated_mark(
            attempt, generation, operation, resume, pending,
        ))
    }

    /// Reinstalls a returned arena after validating its coarse generation.
    #[allow(
        clippy::result_large_err,
        reason = "stale admission must return the complete move-only continuation without a lifecycle allocation"
    )]
    pub fn resume_attempt<R>(
        &mut self,
        universe: &tex_state::Universe<G>,
        pending: crate::PendingCommandAttempt<G, R>,
    ) -> Result<
        (crate::CommandAttemptOperation, crate::AttemptResumePoint, R),
        crate::PendingCommandAttempt<G, R>,
    > {
        if !self.attempt.is_empty()
            || self.active_attempt_operation != Some(pending.operation_coordinate())
        {
            return Err(pending);
        }
        let (attempt, operation, resume, pending) = pending.resume(universe)?;
        self.attempt = attempt;
        Ok((operation, resume, pending))
    }

    pub(crate) fn observe_active_source_dependencies(&self, state: &mut CommandContext<'_, G>) {
        if !state.tracked_region_is_active() {
            return;
        }
        let Some(InputLevel::Source(source)) = self.input.levels.last() else {
            return;
        };
        crate::input::observe_immutable_source(state, source);
    }

    /// Publishes the command-owned dependency roots read by one processor
    /// episode. Complex continuations without a complete canonical projection
    /// poison the outer region before the processor can inspect them.
    pub(crate) fn observe_tracked_dependencies(&self, state: &mut CommandContext<'_, G>) {
        if !state.tracked_region_is_active() {
            return;
        }
        let Some((line, mut stack)) = crate::input::tracked_input_projection(&self.input, state)
        else {
            state.unsupported_command_state();
            return;
        };
        let supported_continuation = self.parameters.activations.is_empty()
            && self.scratch.is_quiescent()
            && self.scanner.is_quiescent()
            && self.alignment == AlignmentDeliveryState::<G>::default()
            && self.transient == TransientState::default()
            && self.replay_completions.is_empty()
            && self.pending_replay_completions.is_empty()
            && self.semantic_diagnostics.is_empty()
            && !self.name_in_progress
            && self.named_token_list_pushes.is_empty()
            && self.file_framing_events.is_empty()
            && self.expansion.pending_diagnostics.is_empty()
            && self.expansion.observed_dependencies.is_empty()
            && self.expansion.semantic_barriers.is_empty();
        if !supported_continuation {
            state.unsupported_command_state();
            return;
        }
        stack ^= self.profile().fingerprint().get();
        stack = stack.rotate_left(17) ^ self.expansion.cumulative_expansions;
        stack = stack.rotate_left(17) ^ self.expansion.next_resource_resolution;
        state.observe_command_projection(
            tex_state::DependencyKey::InputLine,
            tex_state::DependencyValue::Projection {
                schema: 1,
                fingerprint: line,
            },
        );
        state.observe_command_projection(
            tex_state::DependencyKey::InputStack,
            tex_state::DependencyValue::Projection {
                schema: 1,
                fingerprint: stack,
            },
        );

        let (level, ty, branch) = self.conditions.current_etex_values();
        for (field, value) in [
            (tex_state::DependencyEngineField::ConditionLevel, level),
            (tex_state::DependencyEngineField::ConditionType, ty),
            (tex_state::DependencyEngineField::ConditionBranch, branch),
        ] {
            state.observe_command_projection(
                tex_state::DependencyKey::Engine(field),
                tex_state::DependencyValue::Integer(i64::from(value)),
            );
        }
        state.observe_command_projection(
            tex_state::DependencyKey::Engine(tex_state::DependencyEngineField::ConditionStack),
            tex_state::DependencyValue::Projection {
                schema: 1,
                fingerprint: self.conditions.tracked_stack_projection(),
            },
        );
    }

    /// Returns the number of live TeX input levels retained by this command state.
    #[must_use]
    pub fn input_level_count(&self) -> usize {
        self.input.levels.len()
    }

    /// Returns runtime-only TeX82 command-stack maxima for §1334.
    #[must_use]
    pub fn stack_usage(&self) -> CommandStackUsage {
        self.usage.get()
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
            })
            .collect();
        (self.input.levels.len(), tail)
    }

    pub(crate) fn name_in_progress(&self) -> bool {
        self.name_in_progress
    }

    pub(crate) fn take_pending_input_open(&mut self) -> Option<crate::ScannedFileName> {
        self.pending_input_open.take()
    }

    pub(crate) fn retain_pending_input_open(&mut self, file_name: crate::ScannedFileName) {
        debug_assert!(self.pending_input_open.is_none());
        self.pending_input_open = Some(file_name);
    }

    pub(crate) fn begin_file_name(&mut self) -> Result<(), crate::CommandError> {
        if self.name_in_progress {
            return Err(crate::CommandError::input_invariant());
        }
        self.name_in_progress = true;
        Ok(())
    }

    pub(crate) fn end_file_name(&mut self) {
        self.name_in_progress = false;
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

    /// Retains TeX82 §331's bottom terminal buffer for §310 after the
    /// startup acquisition level has been retired.
    pub fn set_terminal_context_line(&mut self, line: impl Into<String>) {
        let line = line.into();
        // TeX82 §31's initial terminal `input_ln` starts at buffer index zero;
        // §1334 subsequently prints `max_buf_stack+1`.
        if !line.is_empty() {
            self.usage.record_buffer_usage(line.chars().count() + 1);
        }
        self.usage.set_terminal_buffer_slots(line.chars().count());
        self.input.terminal_context_line = Some(line);
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
            TokenPayload::durable(words),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Stored(reason),
        );
        self.replay_completions.push(identity);
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
        let index = self
            .replay_completions
            .iter()
            .position(|candidate| *candidate == identity)?;
        self.replay_completions.remove(index);
        self.pending_replay_completions.push(identity);
        Some(CommandReplayEpisode(identity))
    }

    /// Whether an episode completion may surface without cutting off input
    /// levels created while expanding its final token.
    ///
    /// TeX82 §390 retires a depleted token list before pushing a macro's
    /// replacement text. Input identities are allocated monotonically, so a
    /// level newer than the retired episode is one of those descendants. The
    /// completion boundary must remain pending until every such level retires;
    /// an older enclosing level must never be fetched first.
    pub(crate) fn replay_completion_is_ready(&self, episode: CommandReplayEpisode) -> bool {
        self.input
            .levels
            .last()
            .is_none_or(|level| crate::input::input_level_identity(level) < episode.0)
    }

    /// Whether a retired executor-owned level has no remaining descendants.
    pub(crate) fn has_ready_replay_completion(&self) -> bool {
        self.pending_replay_completions
            .iter()
            .copied()
            .any(|identity| self.replay_completion_is_ready(CommandReplayEpisode(identity)))
    }

    /// Claims the first retired ownership boundary whose descendants are gone.
    pub(crate) fn take_ready_replay_completion(&mut self) -> Option<CommandReplayEpisode> {
        let index = self
            .pending_replay_completions
            .iter()
            .position(|&identity| {
                self.replay_completion_is_ready(CommandReplayEpisode(identity))
            })?;
        Some(CommandReplayEpisode(
            self.pending_replay_completions.remove(index),
        ))
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
            TokenPayload::durable(words),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Stored(reason),
        );
        self.named_token_list_pushes.push((level, reason, tokens));
    }

    /// Takes the pushes of executor-requested named token lists, in order.
    ///
    /// The executor publishes them with the rest of the operation's committed
    /// records, which is where tex.web's own trace has them: inside the
    /// `new_graf`/`box_end`/`init_math` transition that installed the level.
    #[must_use]
    pub fn publish_named_token_list_pushes(
        &mut self,
        state: &mut tex_state::CommandContext<'_, G>,
        diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    ) -> Vec<crate::InputRecord> {
        self.named_token_list_pushes
            .drain(..)
            .map(|(level, reason, tokens)| {
                // TeX82 §§323 and 1145 trace a named token list at
                // `begin_token_list`, while its token_type still identifies
                // the list.  Publishing at the executor/command-state seam
                // preserves that context even when the list has one token
                // and is exhausted by the next main-control delivery.
                if state.int_param(tex_state::env::banks::IntParam::TRACING_MACROS) > 1 {
                    let mut text = String::new();
                    crate::processor::expand::append_print_esc_text(
                        state,
                        stored_replay_name(reason),
                        &mut text,
                    );
                    text.push_str("->");
                    for word in state.token_list(tokens) {
                        let token = word.token().expect("durable token word is valid");
                        crate::processor::expand::append_token_list_token_text(
                            state, token, &mut text,
                        );
                    }
                    let mut output = state.begin_diagnostic(diagnostic_effects);
                    output.print_nl(&text);
                    output.end(false);
                }
                crate::InputRecord {
                    transition: crate::InputTransition::Push,
                    reason: crate::processor::stored_input_reason(reason),
                    source_name: None,
                    source: None,
                    level: level.0,
                    position: 0,
                }
            })
            .collect()
    }

    /// Takes semantic diagnostics committed by completed command episodes.
    ///
    /// The executor drains this inside the same aggregate operation that ran
    /// the episode. If a later action suspends or fails, aggregate rollback
    /// restores both this queue and the input cursor from the pre-step
    /// snapshot, so retry reproduces the diagnostic exactly once.
    #[must_use]
    pub fn take_semantic_diagnostics(&mut self) -> Vec<CommandSemanticDiagnostic> {
        self.semantic_diagnostics.drain(..).collect()
    }
    /// Drains the queued §537/§362 file-bracketing transitions, in order,
    /// without rendering them.
    ///
    /// The queue exists because §537's push and §362's pop are input-stack
    /// operations, and the input stack is reached from places that hold no
    /// `Universe`. Prefer [`Self::render_file_framing_events`], which is the
    /// same drain followed by the print; this raw form is for callers that
    /// only want to observe the transitions.
    #[must_use]
    pub fn take_file_framing_events(&mut self) -> Vec<FileFramingEvent> {
        std::mem::take(&mut self.file_framing_events)
    }

    /// Drains the queue and prints tex.web's `(name` and `)` bracketing for
    /// each transition, in order.
    ///
    /// Callers must drain at every point where tex.web itself would already
    /// have printed, not merely once per step. §362 is why:
    ///
    /// ```text
    /// print_char(")"); decr(open_parens); ... end_file_reading;
    /// check_outer_validity;
    /// ```
    ///
    /// `check_outer_validity` reports `Incomplete \if...` and the runaway
    /// family from inside `get_next`, so a `)` left queued until the step
    /// ends puts that diagnostic *inside* a file bracket tex.web had already
    /// closed. §54's `open_parens` therefore lives on `World`
    /// ([`tex_state::file_framing`]) and both the command core and the engine
    /// driver render through it.
    ///
    /// Draining after a rolled-back step is harmless but pointless: rollback
    /// restores the whole [`CommandState`] to its pre-step value and the
    /// `Universe` snapshot takes both the prints and `open_parens` back with
    /// it.
    pub fn render_file_framing_events(&mut self, context: &mut CommandContext<'_, G>) {
        for event in self.file_framing_events.drain(..) {
            match event {
                FileFramingEvent::Open { name } => context.print_file_open(&name),
                FileFramingEvent::Close => context.print_file_close(),
            }
        }
    }

    /// Returns the committed observation for an executor-applied alignment
    /// begin transition.
    ///
    /// The executor supplies the structural transition, while command state
    /// remains the owner of its align-state and stable alignment identity.
    /// Keeping that projection here prevents replay instrumentation from
    /// reconstructing either value from raw input.
    #[must_use]
    pub fn alignment_begin_observation(&self) -> Option<AlignmentRecord> {
        self.alignment
            .active_alignment
            .map(|alignment| AlignmentRecord {
                transition: "begin",
                alignment: Some(alignment.raw()),
                nesting: self.alignment_observation_nesting(),
                align_state: self.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            })
    }

    /// Reports whether TeX82's alignment-aware `get_next` path is active.
    ///
    /// Main-control accelerations that bypass scalar raw delivery must use
    /// this command-owned predicate rather than reconstructing alignment
    /// activity from the executor's mode nest.
    #[must_use]
    pub fn alignment_scanner_is_active(&self) -> bool {
        self.alignment.active_alignment.is_some()
    }

    /// One-based portable nesting for the active or just-suspended alignment.
    #[must_use]
    pub fn alignment_observation_nesting(&self) -> Option<u32> {
        u32::try_from(self.alignment.align_stack.len())
            .ok()
            .filter(|depth| *depth != 0)
    }

    /// Returns the committed observation for a command-owned outer alignment
    /// suspension. The executor chooses the structural boundary, while this
    /// state remains the sole owner of the saved delivery snapshot.
    ///
    /// The reported `align_state` is the outer running brace count that TeX82
    /// §772's `push_alignment` saved, read back from the top `align_stack`
    /// entry.  The live `align_state` is already the nested alignment's
    /// `-1000000` by the time this observation is committed, because §774's
    /// `init_align` overwrites it immediately after the save.
    #[must_use]
    pub fn alignment_suspend_observation(&self) -> Option<AlignmentRecord> {
        let saved = self.alignment.align_stack.last().copied();
        self.alignment
            .suspended
            .last()
            .map(|suspended| AlignmentRecord {
                transition: "suspend",
                alignment: Some(suspended.alignment.raw()),
                nesting: u32::try_from(self.alignment.suspended.len())
                    .ok()
                    .filter(|depth| *depth != 0),
                align_state: saved.unwrap_or(self.alignment.align_state),
                delimiter: None,
                previous_align_state: None,
            })
    }

    /// Returns the committed observation after a saved outer alignment has
    /// resumed its command-owned delivery state.
    #[must_use]
    pub fn alignment_resume_observation(&self) -> Option<AlignmentRecord> {
        self.alignment
            .active_alignment
            .map(|alignment| AlignmentRecord {
                transition: "resume",
                alignment: Some(alignment.raw()),
                nesting: self.alignment_observation_nesting(),
                align_state: self.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            })
    }

    /// Returns the committed observation for TeX82 `fin_align` immediately
    /// before it removes the active delivery context.  `align_peek` has
    /// already delivered the closing brace at this point; the executor only
    /// requests the structural finish and never classifies that token.
    #[must_use]
    pub fn alignment_finish_observation(
        &self,
        alignment: AlignmentIdentity,
    ) -> Option<AlignmentRecord> {
        (self.alignment.active_alignment == Some(alignment)).then_some(AlignmentRecord {
            transition: "finish",
            alignment: Some(alignment.raw()),
            nesting: self.alignment_observation_nesting(),
            align_state: self.alignment.align_state,
            delimiter: None,
            previous_align_state: None,
        })
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
        self.alignment.begin_alignment(alignment);
    }

    /// Re-enters the preamble sentinel while scanning another alignment column.
    pub fn set_alignment_preamble_phase(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
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

    /// Returns the state transition committed by TeX82's omit-cell branch.
    #[must_use]
    pub fn alignment_omit_cell_observation(
        &self,
        alignment: AlignmentIdentity,
    ) -> Option<AlignmentRecord> {
        let cell = self.alignment.active_cell.as_ref()?;
        (cell.alignment == alignment).then_some(AlignmentRecord {
            transition: "state_change",
            alignment: Some(alignment.raw()),
            nesting: self.alignment_observation_nesting(),
            align_state: self.alignment.align_state,
            delimiter: None,
            previous_align_state: cell.omit_previous_align_state,
        })
    }

    /// Returns the committed input push for a just-installed u-template.
    ///
    /// The level identity is allocated by the state transition itself, so
    /// instrumentation can report the canonical input lifecycle without
    /// reconstructing a template push from executor state or token contents.
    #[must_use]
    pub fn alignment_u_template_push_observation(
        &self,
        alignment: AlignmentIdentity,
    ) -> Option<crate::InputRecord> {
        let cell = self.alignment.active_cell.as_ref()?;
        (cell.alignment == alignment).then_some(())?;
        cell.u_level.map(|level| crate::InputRecord {
            transition: crate::InputTransition::Push,
            reason: crate::InputReason::AlignmentUTemplate,
            source_name: None,
            source: None,
            level: level.0,
            position: 0,
        })
    }

    /// Returns the command-owned alignment transition paired with the
    /// u-template input push.
    #[must_use]
    pub fn alignment_u_template_push_alignment_observation(
        &self,
        alignment: AlignmentIdentity,
    ) -> Option<crate::AlignmentRecord> {
        self.alignment_u_template_push_observation(alignment)
            .map(|_| crate::AlignmentRecord {
                transition: "u_template_push",
                alignment: Some(alignment.raw()),
                nesting: self.alignment_observation_nesting(),
                align_state: self.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            })
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

    /// Returns the committed observation for an executor-selected first cell.
    ///
    /// The executor requests the transition, while command state remains the
    /// source of the resulting `align_state`; this avoids deriving an event
    /// from either template contents or raw input.
    #[must_use]
    pub fn alignment_cell_begin_observation(&self) -> Option<AlignmentRecord> {
        self.alignment
            .active_cell
            .as_ref()
            .map(|cell| AlignmentRecord {
                transition: "state_change",
                alignment: Some(cell.alignment.raw()),
                nesting: self.alignment_observation_nesting(),
                align_state: self.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            })
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
                TokenPayload::transient([]),
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
        self.alignment
            .begin_v_template(alignment, level, delimiter, delimiter_line)
    }

    /// Returns the committed v-template push made after a command-owned
    /// delimiter interception.
    #[must_use]
    pub fn alignment_v_template_push_observation(
        &self,
        alignment: AlignmentIdentity,
    ) -> Option<crate::InputRecord> {
        let cell = self.alignment.active_cell.as_ref()?;
        (cell.alignment == alignment).then_some(())?;
        cell.v_level.map(|level| crate::InputRecord {
            transition: crate::InputTransition::Push,
            reason: crate::InputReason::AlignmentVTemplate,
            source_name: None,
            source: None,
            level: level.0,
            position: 0,
        })
    }

    /// Returns the template lifecycle transition paired with the v-template
    /// input push, without exposing template tokens to the executor.
    #[must_use]
    pub fn alignment_v_template_push_alignment_observation(
        &self,
        alignment: AlignmentIdentity,
    ) -> Option<AlignmentRecord> {
        self.alignment_v_template_push_observation(alignment)
            .map(|_| AlignmentRecord {
                transition: if self
                    .alignment
                    .active_cell
                    .as_ref()
                    .is_some_and(|cell| cell.omit)
                {
                    "omit_template_push"
                } else {
                    "v_template_push"
                },
                alignment: Some(alignment.raw()),
                nesting: self.alignment_observation_nesting(),
                // TeX82's v-template insertion (`init_col`) begins the
                // token list before assigning the post-insertion sentinel.
                // The command state is already guarded against a second
                // delimiter, but the committed lifecycle records that
                // canonical pre-sentinel point.
                align_state: CELL_ALIGN_STATE,
                delimiter: None,
                previous_align_state: None,
            })
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
                    && matches!(cursor.payload,
                        TokenPayload::Replay { replay, len }
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
        self.alignment.finish_cell(alignment, level)
    }

    /// Returns TeX82 §791 `fin_col`'s `align_state:=1000000`, published after
    /// `FinishCell` commits.
    ///
    /// The retirement of the exhausted v-template is _not_ published here.
    /// §1131's `do_endv` pops nothing; §357's `end_token_list` retires the
    /// frame whenever `get_next` next reaches it, and observes it there.
    #[must_use]
    pub fn alignment_cell_finish_observation(
        &self,
        alignment: AlignmentIdentity,
    ) -> Option<crate::AlignmentRecord> {
        let cell = self.alignment.active_cell.as_ref()?;
        if cell.alignment != alignment || cell.v_level.is_none() {
            return None;
        }
        Some(crate::AlignmentRecord {
            transition: "state_change",
            alignment: Some(alignment.raw()),
            nesting: self.alignment_observation_nesting(),
            // `finish_cell` assigns the v-template sentinel after `do_endv`
            // has proven the retained input shape. This observation is
            // captured before that typed request commits.
            align_state: 1_000_000,
            delimiter: None,
            previous_align_state: None,
        })
    }

    /// Takes the command-owned observation published when `fin_col` changes
    /// an exhausted saved tab or span into a row ending.
    pub fn take_alignment_extra_tab_recovery_observation(
        &mut self,
    ) -> Option<crate::AlignmentRecord> {
        let alignment = self.alignment.extra_tab_recovery.take()?;
        Some(crate::AlignmentRecord {
            transition: "extra_tab",
            alignment: Some(alignment.raw()),
            nesting: self.alignment_observation_nesting(),
            align_state: 1_000_000,
            delimiter: None,
            previous_align_state: None,
        })
    }

    /// Suspends the complete outer raw-delivery context for a nested alignment.
    pub fn suspend_alignment(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        self.alignment.suspend_alignment(alignment)
    }

    /// Restores the exact outer raw-delivery context after a nested alignment.
    pub fn resume_alignment(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        self.alignment.resume_alignment(alignment)
    }

    /// Finishes an alignment delivery context after all of its cells retire.
    pub fn finish_alignment(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        self.alignment.finish_alignment(alignment)
    }

    /// Creates a fresh command job with an immutable semantic profile.
    ///
    /// No API changes the profile after construction. Snapshot, summary,
    /// format, and checkpoint restoration validate their recorded profile
    /// identity against this value.
    #[must_use]
    pub fn new(profile: CommandProfile) -> Self {
        Self {
            roots: CommandStateRoots {
                engine_semantics: CommandEngineSemantics::for_profile(profile),
                expansion: ExpansionState {
                    profile,
                    ..ExpansionState::default()
                },
                ..CommandStateRoots::default()
            },
            ..Self::default()
        }
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
        self.input.next_source_identity += 1;
        let previous = self.input.pending_sources.insert(raw, source);
        debug_assert!(previous.is_none(), "source identities are unique");
        Ok(id)
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

    pub(crate) fn prepare_started_input(&mut self, endlinechar: i32) -> Option<PhysicalLine> {
        let InputLevel::Source(level) = self.input.levels.last_mut()? else {
            return None;
        };
        // TeX82 §§537--538 acquire line 1 immediately after the successful
        // file-open attempt. `input_ln` represents even an empty file as one
        // empty opening line.
        level.cursor.pending_acquired_line = true;
        level
            .cursor
            .load_next_line(endlinechar)
            .map(|line| line.physical)
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
        );
        let Some(InputLevel::Source(active)) = self.input.levels.last_mut() else {
            unreachable!("the inserted replacement source was just pushed");
        };
        assert_eq!(active.identity(), identity);
        active.cursor.pending_acquired_line = true;
        Ok(())
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
        let Some(InputLevel::Source(active)) = self.input.levels.last_mut() else {
            unreachable!("begin_read_line keeps its source level active during acquisition");
        };
        assert_eq!(
            active.identity(),
            level,
            "begin_read_line keeps the exact source level active during acquisition"
        );
        active.name_class = name_class;
        active.cursor.backing = registered;
        active.cursor.pending_acquired_line = true;
        Ok(())
    }

    /// Opens e-TeX 2.6 etex.ch §53a's generated `\scantokens` pseudo-file.
    pub(crate) fn open_scantokens(
        &mut self,
        registration: SourceRegistration,
        every_eof: Option<tex_state::TokenListId<G>>,
        numeric_name: u8,
    ) -> Result<InputLevelId, SourceRegistrationError> {
        assert!(matches!(numeric_name, 18 | 19));
        let source = self.register_source(registration)?;
        let registered = self
            .take_registered_source(source)
            .expect("a source registered above is present");
        Ok(self.push_source_level(
            registered,
            SourceNameClass::Scantokens(numeric_name),
            crate::input::SourceRetirement::Pop,
            every_eof,
        ))
    }

    /// Pushes e-TeX §24.362's `\everyeof` above its exhausted pseudo-file.
    pub(crate) fn begin_pending_every_eof(
        &mut self,
        stores: &tex_state::CommandContext<'_, G>,
        source: InputLevelId,
    ) -> Option<InputLevelId> {
        let InputLevel::Source(level) = self.input.levels.last_mut()? else {
            return None;
        };
        if level.identity() != source {
            return None;
        }
        let every_eof = level.every_eof.take()?;
        if matches!(level.name_class, SourceNameClass::Scantokens(_)) {
            level.cursor.install_scantokens_eof_context_line();
        }
        let words = stores.token_list(every_eof.clone());
        Some(self.push_token_level(
            TokenPayload::durable(words),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Stored(StoredReplayReason::EveryEof),
        ))
    }

    /// Pushes one source level and queues any canonical file-like opening.
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
    ) -> InputLevelId {
        // TeX82 §321 checks `input_ptr` before `push_input` increments it.
        self.usage.record_input_push(self.input.levels.len());
        let identity = InputLevelId(self.input.next_level_identity);
        self.input.next_level_identity = self.input.next_level_identity.wrapping_add(1);
        let framing_name = match name_class {
            SourceNameClass::File
                if registered.framing == crate::SourceFramingPolicy::Canonical =>
            {
                registered
                    .framing_name
                    .clone()
                    .or_else(|| registered.name.clone())
            }
            SourceNameClass::File => None,
            SourceNameClass::Scantokens(19) => Some(" ".into()),
            SourceNameClass::Terminal
            | SourceNameClass::ReadStream(_)
            | SourceNameClass::Scantokens(_) => None,
        };
        if let Some(name) = framing_name {
            self.file_framing_events
                .push(FileFramingEvent::Open { name });
        }
        self.input.levels.push(InputLevel::Source(SourceLevel {
            frame: crate::input::PackedInputFrame::source(identity.0, registered.id),
            cursor: Box::new(SourceCursor::new(registered)),
            name_class,
            retirement,
            every_eof,
            open_depths: None,
        }));
        identity
    }

    /// e-TeX 2.6 [23.328]'s `grp_stack[in_open]:=cur_boundary;
    /// if_stack[in_open]:=cond_ptr`, represented by their full enclosing
    /// identity chains and recorded by the opener because
    /// `push_source_level` has no `Universe` access to read the live group
    /// depth itself. A no-op if `level` is not a live source level (for
    /// example, it has already been retired).
    pub(crate) fn record_source_open_depths(
        &mut self,
        level: InputLevelId,
        group_lineages: Box<[u64]>,
        conditional_identities: Box<[u64]>,
    ) {
        for entry in &mut self.input.levels {
            if let InputLevel::Source(source) = entry
                && source.identity() == level
            {
                source.open_depths = Some(Box::new(crate::input::SourceOpenDepths {
                    group_lineages,
                    conditional_identities,
                }));
                return;
            }
        }
    }

    /// The `\tracingnesting` open-depth record [`Self::record_source_open_depths`]
    /// attached to a still-live source level, read before retirement removes it.
    pub(crate) fn source_open_depths(
        &self,
        level: InputLevelId,
    ) -> Option<crate::input::SourceOpenDepths> {
        self.input.levels.iter().find_map(|entry| match entry {
            InputLevel::Source(source) if source.identity() == level => {
                source.open_depths.as_deref().cloned()
            }
            _ => None,
        })
    }

    pub(crate) fn current_source_open_depths(&self) -> Option<crate::input::SourceOpenDepths> {
        self.input
            .levels
            .iter()
            .rev()
            .find_map(|entry| match entry {
                InputLevel::Source(source) => source.open_depths.as_deref().cloned(),
                _ => None,
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
                InputLevel::Tokens(_) => None,
            })
            .is_some();
        if has_source {
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
            TokenPayload::durable(stores.token_list(template)),
            behavior,
            retirement,
            trace,
        )
    }

    /// Splits and normalizes the next physical line on the active source.
    ///
    /// LF, CR, and CRLF are retained as distinct physical metadata. TeX
    /// trailing spaces are removed and the current `endlinechar` is captured
    /// for this line without tokenizing any characters.
    pub fn load_next_source_line(&mut self, endlinechar: i32) -> Option<PhysicalLine> {
        let physical = {
            let InputLevel::Source(level) = self.input.levels.last_mut()? else {
                return None;
            };
            level
                .cursor
                .load_next_line(endlinechar)
                .map(|line| line.physical)?
        };
        self.record_active_buffer_line_usage();
        Some(physical)
    }

    /// Records the §31 buffer index reached by the active physical line.
    /// Every enclosing source retains its normalized line in TeX's shared
    /// buffer; token-list levels consume no buffer positions.
    fn record_active_buffer_line_usage(&self) {
        let mut lines = self.active_buffer_lines();
        let Some(_) = lines.pop() else {
            return;
        };
        let outer_slots = lines.into_iter().fold(0_usize, |total, (len, endline)| {
            total
                .saturating_add(len)
                .saturating_add(usize::from(endline))
                .saturating_add(1)
        });
        let Some(InputLevel::Source(active)) = self.input.levels.last() else {
            return;
        };
        let buffer_start = 1_usize
            .saturating_add(self.usage.terminal_buffer_slots())
            .saturating_add(outer_slots);
        if let Some(positions) = crate::input::source_line_buffer_high_water(
            &active.cursor,
            Some(active.name_class),
            buffer_start,
        ) {
            self.usage.record_buffer_usage(positions);
        }
    }

    pub(crate) fn record_csname_buffer_usage(&self, name_len: usize) {
        if name_len == 0 {
            return;
        }
        let occupied =
            self.active_buffer_lines()
                .into_iter()
                .fold(0_usize, |total, (len, endline)| {
                    total
                        .saturating_add(len)
                        .saturating_add(usize::from(endline))
                        .saturating_add(1)
                });
        // §374 starts at `first`; §1334 adds one to the greatest written
        // buffer index.
        self.usage.record_buffer_usage(
            self.usage
                .terminal_buffer_slots()
                .saturating_add(occupied)
                .saturating_add(name_len)
                .saturating_add(2),
        );
    }

    fn active_buffer_lines(&self) -> Vec<(usize, bool)> {
        self.input
            .levels
            .iter()
            .filter_map(source_buffer_line)
            .collect()
    }

    /// Reads one byte-domain character or decoded Unicode scalar from the
    /// active normalized line with its exact physical range.
    pub fn next_source_character(&mut self) -> Option<SourceCharacter> {
        let InputLevel::Source(level) = self.input.levels.last_mut()? else {
            return None;
        };
        let backing = level.cursor.current_backing();
        let mode = backing.mode;
        let bytes = std::sync::Arc::clone(&backing.bytes);
        level.cursor.line.as_mut()?.next_character(mode, &bytes)
    }

    /// Names the backing TeX82 §363 installed over the active line, if any.
    ///
    /// A replacement line is real immutable input with an identity of its
    /// own, so the aggregate source map has to learn about it before any
    /// token located in it is reported. Returning `None` is the ordinary
    /// case: the line came from the file, which is registered already.
    pub(crate) fn take_active_line_registration(
        &mut self,
    ) -> Option<(tex_state::SourceId, tex_state::source_map::SourceDescriptor)> {
        let Some(InputLevel::Source(level)) = self.input.levels.last_mut() else {
            return None;
        };
        if level.cursor.line_backing_registered {
            return None;
        }
        let backing = level.cursor.line_backing.as_ref()?;
        let registration = (backing.id, backing.source_descriptor());
        level.cursor.line_backing_registered = true;
        Some(registration)
    }

    /// Captures the active source's immutable identity and bytes for detached
    /// observation without exposing the live source level.
    pub(crate) fn active_source_snapshot(
        &self,
    ) -> Option<crate::observation::OpenedSourceSnapshot> {
        let Some(InputLevel::Source(level)) = self.input.levels.last() else {
            return None;
        };
        let backing = level.cursor.current_backing();
        Some(crate::observation::OpenedSourceSnapshot {
            id: backing.id,
            bytes: std::sync::Arc::clone(&backing.bytes),
        })
    }

    /// Retires the active normalized line so the next physical line may load.
    pub fn finish_source_line(&mut self) {
        if let Some(InputLevel::Source(level)) = self.input.levels.last_mut() {
            level.cursor.finish_line();
        }
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
        let force_eof = self.source_force_eof();
        let (Some(cursor), mut lines) = self.active_source_cursor(profile) else {
            return SourceTokenizationStep::End;
        };
        cursor.next_exact_byte_step(endlinechar, force_eof, queries, &mut lines)
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
        let force_eof = self.source_force_eof();
        let (Some(cursor), mut lines) = self.active_source_cursor(profile) else {
            return SourceTokenizationStep::End;
        };
        cursor.next_unicode_step(endlinechar, force_eof, queries, &mut lines)
    }

    /// Tokenizes one exact-byte source step directly into compact command
    /// delivery identity while the transient control-sequence name is live.
    pub(crate) fn next_compact_exact_source_step(
        &mut self,
        endlinechar: i32,
        queries: &mut dyn CompactSourceStepQueries,
    ) -> CompactSourceTokenizationStep {
        let profile = self.profile();
        assert_eq!(
            profile.character_mode(),
            crate::CharacterMode::EightBitExact,
            "exact-byte tokenization requires an exact-byte command profile"
        );
        let force_eof = self.source_force_eof();
        let (Some(cursor), mut lines) = self.active_source_cursor(profile) else {
            return CompactSourceTokenizationStep::End;
        };
        cursor.next_compact_exact_byte_step(endlinechar, force_eof, queries, &mut lines)
    }

    /// Tokenizes one Unicode source step directly into compact command
    /// delivery identity while the transient control-sequence name is live.
    pub(crate) fn next_compact_unicode_source_step(
        &mut self,
        endlinechar: i32,
        queries: &mut dyn CompactSourceStepQueries,
    ) -> CompactSourceTokenizationStep {
        let profile = self.profile();
        assert_eq!(
            profile.character_mode(),
            crate::CharacterMode::UnicodeExtended,
            "Unicode tokenization requires a UnicodeExtended command profile"
        );
        let force_eof = self.source_force_eof();
        let (Some(cursor), mut lines) = self.active_source_cursor(profile) else {
            return CompactSourceTokenizationStep::End;
        };
        cursor.next_compact_unicode_step(endlinechar, force_eof, queries, &mut lines)
    }

    fn source_force_eof(&self) -> bool {
        self.input.force_eof
            && self
                .input
                .levels
                .iter()
                .rev()
                .find_map(|level| match level {
                    InputLevel::Source(source) => Some(source.name_class == SourceNameClass::File),
                    InputLevel::Tokens(_) => None,
                })
                == Some(true)
    }

    /// Borrows the active source cursor beside the source-identity counter.
    ///
    /// TeX82 §363's replacement line needs an identity allocated while the
    /// cursor that will read it is already borrowed, so the two disjoint
    /// fields are handed out together rather than the cursor alone.
    fn active_source_cursor(
        &mut self,
        profile: CommandProfile,
    ) -> (
        Option<&mut crate::input::SourceCursor>,
        crate::input::LineBackingRegistry<'_>,
    ) {
        // This method runs for every physical token. The old
        // `active_buffer_lines` projection allocated a temporary Vec merely
        // to reverse it. The last input level is the source cursor we are
        // about to advance; fold the lower levels in place and do not count
        // that cursor's current line toward its own buffer base. Skipping the
        // level rather than the first _present line_ is also correct while a
        // newly opened top source has not loaded its first line yet.
        let occupied_below_active =
            self.input
                .levels
                .iter()
                .rev()
                .skip(1)
                .fold(0_usize, |total, level| {
                    let Some((len, endline)) = source_buffer_line(level) else {
                        return total;
                    };
                    total
                        .saturating_add(len)
                        .saturating_add(usize::from(endline))
                        .saturating_add(1)
                });
        let buffer_start = 1_usize
            .saturating_add(self.usage.terminal_buffer_slots())
            .saturating_add(occupied_below_active);
        let name_class = self.input.levels.last().and_then(|level| match level {
            InputLevel::Source(source) => Some(source.name_class),
            InputLevel::Tokens(_) => None,
        });
        let usage = self.usage.clone();
        let input = &mut self.input;
        let lines = crate::input::LineBackingRegistry {
            profile,
            next_identity: &mut input.next_source_identity,
            usage,
            buffer_start,
            name_class,
        };
        let cursor = match input.levels.last_mut() {
            Some(InputLevel::Source(level)) => Some(level.cursor.as_mut()),
            _ => None,
        };
        (cursor, lines)
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

fn source_buffer_line<G>(level: &InputLevel<G>) -> Option<(usize, bool)> {
    let InputLevel::Source(source) = level else {
        return None;
    };
    let line = source.cursor.line.as_ref()?;
    let start = line.physical.content_range().start();
    let retained = line.retained_end.saturating_sub(start);
    let len = match source.cursor.current_backing().mode {
        crate::CharacterMode::EightBitExact => usize::try_from(retained).unwrap_or(usize::MAX),
        crate::CharacterMode::UnicodeExtended => {
            let start = usize::try_from(start).unwrap_or(usize::MAX);
            let end = usize::try_from(line.retained_end).unwrap_or(usize::MAX);
            source
                .cursor
                .current_backing()
                .bytes
                .get(start..end)
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
                .map_or(usize::MAX, |text| text.chars().count())
        }
    };
    Some((len, line.endline.is_some()))
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
