//! Future-relevant state and discardable runtime ownership.

use tex_state::CommandContext;
use tex_state::input::TracedTokenList;
use tex_state::token::TracedTokenWord;

use crate::AlignmentRecord;
use crate::conditionals::ConditionStack;
use crate::input::InputState;
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
    AlignmentCellDelimiter, AlignmentCellTemplates, AlignmentDeliveryState, AlignmentIdentity,
    AlignmentLifecycleError, AlignmentRequest, AlignmentRequestResult, CELL_ALIGN_STATE,
    ExpansionState, ScannerState,
};
use crate::profile::{
    CommandProfile, CommandProfileBoundary, CommandProfileFingerprint, CommandProfileMismatch,
};

/// Complete future-relevant state owned by the command machine.
///
/// This is the command half of an executor savepoint. It contains semantic
/// and rollback-coupled provenance state only: host capabilities, aggregate
/// engine state, call-local accumulators, and discardable accelerations are
/// deliberately absent.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct CommandState {
    pub(crate) input: InputState,
    pub(crate) parameters: ParameterState,
    pub(crate) scanner: ScannerState,
    pub(crate) conditions: ConditionStack,
    pub(crate) alignment: AlignmentDeliveryState,
    pub(crate) expansion: ExpansionState,
    pub(crate) transient: TransientState,
    pub(crate) replay_completions: Vec<InputLevelId>,
    /// Semantic diagnostics committed by command processing but rendered by
    /// the executor's World-facing diagnostic boundary.
    ///
    /// This queue is unconditional command state, not observation state.
    /// Consequently an unobserved episode has identical semantics, while the
    /// ordinary command snapshot makes a failed aggregate operation restore
    /// the queue together with the input transition that produced it.
    pub(crate) semantic_diagnostics: Vec<CommandSemanticDiagnostic>,
    /// TeX82 §527's rollback-coupled `name_in_progress` recursion guard.
    pub(crate) name_in_progress: bool,
    /// Named token-list levels installed since the executor last drained
    /// them, in push order.
    ///
    /// This is observation-owned but unconditional: every step that opens an
    /// episode drains it, so it cannot accumulate in a run nobody observes.
    /// It deliberately carries no "am I observed" flag -- that would be
    /// observation state living inside semantic state, which
    /// `absent_observer_has_no_delivery_or_snapshot_effect` and
    /// `math_episode_observation_does_not_change_frozen_command_state` exist
    /// to forbid, and which they caught when `umber2-johp.310` first tried
    /// it.
    ///
    /// tex.web installs these inside `begin_token_list`, where its trace
    /// observes them; Umber's executor asks command state to install them
    /// after the borrowed command-processor episode has ended, so the record
    /// waits here until the same operation publishes its other committed
    /// observations.
    pub(crate) named_token_list_pushes: Vec<(InputLevelId, StoredReplayReason)>,
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
    /// `named_token_list_pushes` above already is:
    /// [`CommandState::snapshot`](crate::CommandState::snapshot) clones this
    /// whole struct before a step runs, and
    /// [`CommandState::rollback`](crate::CommandState::rollback) replaces the
    /// whole struct wholesale (`*self = snapshot.state`) if that step is
    /// undone. A queued event from a rolled-back step is therefore restored
    /// away along with every other command-state mutation that step made --
    /// nothing prints a paren for an open that never committed -- while a
    /// committed step's events survive exactly as long as the rest of its
    /// committed state does, until the executor drains them with
    /// [`Self::take_file_framing_events`]. No per-field bookkeeping is
    /// needed because the whole-struct snapshot already covers it.
    pub(crate) file_framing_events: Vec<FileFramingEvent>,
    /// Rollback-coupled input coverage for the active outer paragraph.
    pub(crate) paragraph_input_transaction:
        Option<crate::paragraph::ActiveParagraphInputTransaction>,
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
    },
    /// TeX82 §391's compulsory macro-parameter-text mismatch.
    MacroPrefixMismatch(tex_state::interner::Symbol),
    /// TeX82 §415's missing-number recovery, deferred only when an earlier
    /// command-owned diagnostic is already waiting for executor output.
    ///
    /// §82 renders `show_context` when `error` completes, while §415 has
    /// already used §325's `back_error` to put the offending token back.
    /// The command stack is the sole owner of that backed-up level, so its
    /// display crosses the deferred-report boundary with the diagnostic.
    MissingNumber { context: String },
}

/// The output TeX's `runaway` procedure emits before its caller's error.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RunawayPrelude {
    pub heading: &'static str,
    pub partial: String,
}

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
pub enum CommandReplayDelivery {
    Command(crate::CurrentCommand),
    Completed(CommandReplayEpisode),
}

impl CommandState {
    /// Returns the number of live TeX input levels retained by this command state.
    #[must_use]
    pub fn input_level_count(&self) -> usize {
        self.input.levels.len()
    }

    pub(crate) const fn name_in_progress(&self) -> bool {
        self.name_in_progress
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
    pub fn output_open_context(&self, stores: &tex_state::CommandContext<'_>) -> String {
        self.input.output_open_context(stores, &self.parameters)
    }

    pub(crate) fn open_context_starts_with_print_ln(
        &self,
        stores: &tex_state::CommandContext<'_>,
    ) -> bool {
        self.input
            .open_context_starts_with_print_ln(stores, &self.parameters)
    }

    pub(crate) fn output_retiring_source_context(
        &self,
        source: &crate::input::SourceLevel,
        stores: &tex_state::CommandContext<'_>,
    ) -> String {
        self.input
            .output_retiring_source_context(source, stores, &self.parameters)
    }

    /// TeX82 §§1026/1028's context after the selected output list ends.
    ///
    /// Canonical delivery can retain a depleted cursor until the next fetch
    /// so its retirement remains observable at that boundary. The
    /// synchronous post-output error nevertheless sees the levels below it,
    /// exactly as it would after §1026's `end_token_list`.
    #[must_use]
    pub fn output_close_context(&self, stores: &tex_state::CommandContext<'_>) -> String {
        self.input.output_close_context(stores, &self.parameters)
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
    pub fn push_discretionary_episode(&mut self, tokens: TracedTokenList) -> CommandReplayEpisode {
        self.push_stored_episode(tokens, crate::input::StoredReplayReason::Discretionary)
    }

    fn push_stored_episode(
        &mut self,
        tokens: TracedTokenList,
        reason: StoredReplayReason,
    ) -> CommandReplayEpisode {
        let identity = self.push_token_level(
            TokenPayload::Stored {
                tokens: tokens.token_list(),
                origins: tokens.origin_list(),
            },
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
    pub(crate) fn take_replay_completion(
        &mut self,
        identity: InputLevelId,
    ) -> Option<CommandReplayEpisode> {
        let index = self
            .replay_completions
            .iter()
            .position(|candidate| *candidate == identity)?;
        self.replay_completions.swap_remove(index);
        Some(CommandReplayEpisode(identity))
    }

    /// Schedules a frozen `\everypar` list after canonical main control has
    /// completed TeX82's `new_graf` state transition.  Source ownership stays
    /// entirely inside command state; executor control never fabricates an
    /// input stack for token-list replay.
    pub fn push_everypar(&mut self, tokens: TracedTokenList) {
        self.push_named_token_list(tokens, StoredReplayReason::EveryPar);
    }

    /// Schedules the immutable math-entry hook after the stomach has entered
    /// the matching math-shift group.  The command machine owns this replay
    /// so macro expansion, origins, and retirement stay canonical.
    pub fn push_everymath(&mut self, tokens: TracedTokenList, display: bool) {
        self.push_named_token_list(
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
    pub fn push_everybox(&mut self, tokens: TracedTokenList, horizontal: bool) {
        self.push_named_token_list(
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
    pub fn push_everycr(&mut self, tokens: TracedTokenList) {
        self.push_named_token_list(tokens, StoredReplayReason::EveryCr);
    }

    /// Schedules the immutable `\everyjob` payload for tex.web §1030
    /// `main_control`'s prologue,
    /// `if every_job<>null then begin_token_list(every_job,every_job_text)`,
    /// which runs once before the first `big_switch` fetch.
    pub fn push_everyjob(&mut self, tokens: TracedTokenList) {
        self.push_named_token_list(tokens, StoredReplayReason::EveryJob);
    }

    /// Installs one tex.web §307-named token list and records its push.
    ///
    /// This is `begin_token_list` for the executor-requested hooks: the level
    /// carries the §307 `token_type` it was installed under, so both its push
    /// and its eventual retirement report that identity rather than the one
    /// token-list class every stored level used to share.
    fn push_named_token_list(&mut self, tokens: TracedTokenList, reason: StoredReplayReason) {
        let level = self.push_token_level(
            TokenPayload::Stored {
                tokens: tokens.token_list(),
                origins: tokens.origin_list(),
            },
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Stored(reason),
        );
        self.named_token_list_pushes.push((level, reason));
    }

    /// Takes the pushes of executor-requested named token lists, in order.
    ///
    /// The executor publishes them with the rest of the operation's committed
    /// records, which is where tex.web's own trace has them: inside the
    /// `new_graf`/`box_end`/`init_math` transition that installed the level.
    #[must_use]
    pub fn take_named_token_list_push_observations(&mut self) -> Vec<crate::InputRecord> {
        self.named_token_list_pushes
            .drain(..)
            .map(|(level, reason)| crate::InputRecord {
                transition: crate::InputTransition::Push,
                reason: crate::processor::stored_input_reason(reason),
                source_name: None,
                level: level.0,
                position: 0,
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
    pub fn render_file_framing_events(&mut self, context: &mut CommandContext<'_>) {
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
                align_state: self.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            })
    }

    /// Whether ordinary main control owns the next delivery strongly enough
    /// to use TeX82 §1038's character-loop lookahead.
    ///
    /// An active cell is delivered through the alignment scanner/replay
    /// boundary instead.  Even though its characters execute in ordinary
    /// horizontal main control, that boundary must retain token-at-a-time
    /// ownership so delimiter interception and replay retirement remain
    /// ordered with every delivered command.
    #[must_use]
    pub fn main_loop_batching_is_eligible(&self) -> bool {
        self.alignment.active_cell.is_none()
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
            AlignmentRequest::BeginCell {
                alignment,
                templates,
            } => {
                self.begin_alignment_cell(alignment, templates)?;
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
                self.install_alignment_cell_template(alignment)?;
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
    pub fn begin_alignment_cell(
        &mut self,
        alignment: AlignmentIdentity,
        templates: AlignmentCellTemplates,
    ) -> Result<(), AlignmentLifecycleError> {
        self.alignment.begin_cell(alignment, templates)
    }

    /// Installs the active cell's optional u-template after the executor's
    /// typed opener phase has completed command-owned brace replay.
    pub fn install_alignment_cell_template(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        let template = self.alignment.active_cell_template(alignment)?;
        if let Some(template) = template {
            let level = self.push_alignment_template(
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
        alignment: AlignmentIdentity,
        delimiter: AlignmentCellDelimiter,
    ) -> Result<(), AlignmentLifecycleError> {
        let template = self.alignment.v_template(alignment)?;
        // tex.web §789: `if cur_cmd=omit then begin_token_list(omit_template,
        // v_template) else begin_token_list(v_part(cur_align),v_template)`.
        // Both levels are `token_type=v_template`; only the list differs, and
        // that is what names the level in the pinned observer's trace.
        let omit = self.alignment.active_cell_is_omit(alignment);
        let level = self.push_alignment_template(
            template,
            TokenBehavior::VTemplate,
            RetirementBehavior::RetainExhaustedVTemplate,
            if omit {
                ReplayTrace::OmitTemplate
            } else {
                ReplayTrace::VTemplate
            },
        );
        self.alignment.begin_v_template(alignment, level, delimiter)
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
        let retained_v_template = |level: &InputLevel| {
            matches!(level,
                InputLevel::Tokens(cursor)
                    if cursor.identity == v_level
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
                    && matches!(&cursor.payload, TokenPayload::BackedUp(tokens) if tokens.get(cursor.index).is_none())
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
            expansion: ExpansionState {
                profile,
                ..ExpansionState::default()
            },
            ..Self::default()
        }
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
        self.input.registered_sources.push(source);
        Ok(id)
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
    /// This operation only clones retained immutable backing. It cannot search
    /// for files, invoke a host callback, or diagnose text encoding.
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
    /// `name` classification.
    pub fn open_registered_source_as(
        &mut self,
        source: tex_state::SourceId,
        name_class: SourceNameClass,
    ) -> Result<(), UnknownRegisteredSource> {
        let registered = self
            .input
            .registered_sources
            .iter()
            .find(|registered| registered.id == source)
            .cloned()
            .ok_or(UnknownRegisteredSource(source))?;
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
            .input
            .registered_sources
            .iter()
            .find(|registered| registered.id == source)
            .cloned()
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
            .input
            .registered_sources
            .iter()
            .find(|registered| registered.id == source)
            .cloned()
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
        assert_eq!(active.identity, identity);
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
            .input
            .registered_sources
            .iter()
            .find(|registered| registered.id == source)
            .cloned()
            .expect("a source registered above is present");
        let Some(InputLevel::Source(active)) = self.input.levels.last_mut() else {
            unreachable!("begin_read_line keeps its source level active during acquisition");
        };
        assert_eq!(
            active.identity, level,
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
        every_eof: Option<TracedTokenList>,
        numeric_name: u8,
    ) -> Result<InputLevelId, SourceRegistrationError> {
        assert!(matches!(numeric_name, 18 | 19));
        let source = self.register_source(registration)?;
        let registered = self
            .input
            .registered_sources
            .iter()
            .find(|registered| registered.id == source)
            .cloned()
            .expect("a source registered above is present");
        Ok(self.push_source_level(
            registered,
            SourceNameClass::Scantokens(numeric_name),
            crate::input::SourceRetirement::Pop,
            every_eof,
        ))
    }

    /// Pushes e-TeX §24.362's `\everyeof` above its exhausted pseudo-file.
    pub(crate) fn begin_pending_every_eof(&mut self, source: InputLevelId) -> Option<InputLevelId> {
        let InputLevel::Source(level) = self.input.levels.last_mut()? else {
            return None;
        };
        if level.identity != source {
            return None;
        }
        let every_eof = level.every_eof.take()?;
        if matches!(level.name_class, SourceNameClass::Scantokens(_)) {
            level.cursor.install_scantokens_eof_context_line();
        }
        Some(self.push_token_level(
            TokenPayload::Stored {
                tokens: every_eof.token_list(),
                origins: every_eof.origin_list(),
            },
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
        every_eof: Option<TracedTokenList>,
    ) -> InputLevelId {
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
            identity,
            cursor: SourceCursor::new(registered),
            name_class,
            retirement,
            scanner_at_open: self.scanner.clone(),
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
                && source.identity == level
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
            InputLevel::Source(source) if source.identity == level => {
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
        template: TracedTokenList,
        behavior: TokenBehavior,
        retirement: RetirementBehavior,
        trace: ReplayTrace,
    ) -> InputLevelId {
        self.push_token_level(
            TokenPayload::Stored {
                tokens: template.token_list(),
                origins: template.origin_list(),
            },
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
        let InputLevel::Source(level) = self.input.levels.last_mut()? else {
            return None;
        };
        level
            .cursor
            .load_next_line(endlinechar)
            .map(|line| line.physical)
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
    pub(crate) fn active_line_backing(
        &self,
    ) -> Option<(tex_state::SourceId, tex_state::source_map::SourceDescriptor)> {
        let Some(InputLevel::Source(level)) = self.input.levels.last() else {
            return None;
        };
        let backing = level.cursor.line_backing.as_ref()?;
        Some((backing.id, backing.source_descriptor()))
    }

    /// Captures the active source's immutable identity and bytes for detached
    /// observation without exposing the registered-source store.
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
        let force_eof = self.input.force_eof
            && self
                .input
                .levels
                .iter()
                .rev()
                .find_map(|level| match level {
                    InputLevel::Source(source) => Some(source.name_class == SourceNameClass::File),
                    InputLevel::Tokens(_) => None,
                })
                == Some(true);
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
        let force_eof = self.input.force_eof
            && self
                .input
                .levels
                .iter()
                .rev()
                .find_map(|level| match level {
                    InputLevel::Source(source) => Some(source.name_class == SourceNameClass::File),
                    InputLevel::Tokens(_) => None,
                })
                == Some(true);
        let (Some(cursor), mut lines) = self.active_source_cursor(profile) else {
            return SourceTokenizationStep::End;
        };
        cursor.next_unicode_step(endlinechar, force_eof, queries, &mut lines)
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
        let input = &mut self.input;
        let lines = crate::input::LineBackingRegistry {
            profile,
            next_identity: &mut input.next_source_identity,
        };
        let cursor = match input.levels.last_mut() {
            Some(InputLevel::Source(level)) => Some(&mut level.cursor),
            _ => None,
        };
        (cursor, lines)
    }

    /// Returns the immutable profile selected when this job was created.
    #[must_use]
    pub const fn profile(&self) -> CommandProfile {
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
/// Builder contents and rollback roots are semantic while live. Spare
/// capacity and reusable empty buffers instead belong to [`CommandRuntime`].
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
    pub(crate) tokens: Vec<TracedTokenWord>,
}

/// Discardable command-processing acceleration and measurements.
///
/// Replacing this value with [`CommandRuntime::default`] at any point cannot
/// change semantic events, diagnostics, effects, output, or `CommandState`.
/// It intentionally implements neither equality nor hashing, preventing it
/// from becoming part of semantic state comparisons by convenience.
#[derive(Debug, Default)]
#[allow(dead_code)] // caches are populated when command semantics are implemented
pub struct CommandRuntime {
    meaning_cache: MeaningCache,
    normalized_lines: LineNormalizationCache,
    transient_pool: TokenBufferPool,
    profiling: CommandProfiling,
}

#[derive(Debug, Default)]
#[allow(dead_code)] // ownership shell
struct MeaningCache {
    entries: Vec<MeaningCacheEntry>,
}

#[derive(Debug)]
#[allow(dead_code)] // ownership shell
struct MeaningCacheEntry {
    identity: u64,
    generation: u64,
}

#[derive(Debug, Default)]
#[allow(dead_code)] // ownership shell
struct LineNormalizationCache {
    entries: Vec<NormalizedLineCacheEntry>,
}

#[derive(Debug)]
#[allow(dead_code)] // ownership shell
struct NormalizedLineCacheEntry {
    content_identity: u64,
    normalized: Vec<u8>,
}

#[derive(Debug, Default)]
#[allow(dead_code)] // ownership shell
struct TokenBufferPool {
    buffers: Vec<Vec<TracedTokenWord>>,
}

#[derive(Debug, Default)]
#[allow(dead_code)] // ownership shell
struct CommandProfiling {
    raw_deliveries: u64,
    cache_hits: u64,
}

#[cfg(test)]
mod tests;
