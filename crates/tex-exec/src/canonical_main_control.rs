//! Production canonical main-control driver.
//!
//! This intentionally owns only typed execution mutations. Raw delivery,
//! expansion, macro calls, input nesting, and operand collection remain in
//! `tex-command`; no `InputStack` is accepted here.

use std::path::PathBuf;
use tex_command::{
    AlignmentCellDelimiter, AlignmentCellOpening, AlignmentCellTemplates, AlignmentDelivery,
    AlignmentIdentity, AlignmentRequest, AlignmentRequestResult, CanonicalMathRequest,
    CommandError, CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandProfile,
    CommandRuntime, CommandState, CommandStateSnapshot, FontLoadRequest, FontResource,
    ImmediateExtension, InputStreamRequest, MathDelimiterBoundary, MathDelimiterBoundaryKind,
    MathEpisodeRecovery, MathLimitKind, MathScriptKind, MathStyleKind, MathTextFieldKind,
    PdfAnnotationRequest, PdfColorStackActionRequest, PdfDestinationRequest,
    PdfDocumentFragmentRequest, PdfFormRequest, PdfGraphicsRequest, PdfImageRequest,
    PdfImageResource, PdfNavigationRequest, PdfObjectRequest, PdfReferenceObjectRequest,
    PdfStartLinkRequest, ScannedAccent, ScannedBoxConstruction, ScannedBoxKind,
    ScannedDiscretionary, ScannedDisplayDiagnostic, ScannedLeaderPayload, ScannedMathMuMaterial,
    ScannedPackingSpec, ScannedVSplit, SourceRegistration, SourceRegistrationError,
};
#[cfg(any(test, feature = "instrumentation"))]
use tex_command::{
    CommandObservation, CommandObserver, EffectRecord, MutationRecord, ObservedToken,
};
use tex_state::code_tables::{DelCode, LcCode, MathCode, SfCode, UcCode};
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::{FontId, NodeListId, TokenListId};
use tex_state::interner::Symbol;
use tex_state::macro_store::{MacroDefinitionProvenance, MacroMeaning};
use tex_state::math::{
    FractionThickness, LimitType, MathChar, MathChoice, MathField, MathFontSize, MathFraction,
    MathNoad, MathStyle, NoadClass, NoadKind,
};
use tex_state::meaning::{Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::node::{DiscKind, GlueKind, KernKind, LeaderPayload, Node, Whatsit};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, Token};
use tex_state::{
    ExpansionState, GroupKind, InputOpenState, InputReadState, ParagraphShapeLine, PrintSink,
    StreamSlot, TracedTokenList, Universe,
};
use tex_typeset::PackSpec;

use crate::mode::{AlignColumn, AlignState, AlignmentKind, AlignmentPackSpec};
use crate::{ExecError, Mode, ModeNest};

/// Production command main control with command-owned source consumption.
#[derive(Debug, Default)]
pub struct CanonicalMainControl {
    command: CommandState,
    runtime: CommandRuntime,
    capabilities: CommandHostCapabilities,
    modes: ModeNest,
    next_alignment_identity: u64,
    active_alignment: Option<ActiveReplayAlignment>,
    boxes: ReplayBoxes,
    output_dead_cycles: i32,
    completed_replay_episode: Option<tex_command::CommandReplayEpisode>,
    /// Detached DVI receipts whose artifact commits have survived an entire
    /// canonical aggregate operation. This is replay state so rollback drops
    /// it with the corresponding World artifact/effect roots.
    prepared_dvi_pages: Vec<crate::dispatch::PreparedDviPage>,
}

#[derive(Clone, Copy, Debug)]
struct SetBoxTarget {
    index: u16,
    global: bool,
}

#[derive(Clone, Copy, Debug)]
struct ActiveReplayBox {
    target: Option<SetBoxTarget>,
    ships_out: bool,
    opening_brace_replay: bool,
    body_opener_pending: bool,
    depth: u32,
    kind: ReplayBoxKind,
    packing: PackSpec,
    leader_kind: Option<GlueKind>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReplayBoxKind {
    HBox,
    VBox,
    VTop,
}

impl ReplayBoxKind {
    const fn horizontal(self) -> bool {
        matches!(self, Self::HBox)
    }

    const fn group_kind(self) -> GroupKind {
        match self {
            Self::HBox => GroupKind::HBox,
            Self::VBox => GroupKind::VBox,
            Self::VTop => GroupKind::VTop,
        }
    }
}

#[derive(Clone, Debug)]
struct ActiveReplayAlignment {
    identity: AlignmentIdentity,
    kind: AlignmentKind,
    columns: Vec<AlignmentCellTemplates>,
    repeat_start: Option<usize>,
    column: usize,
    preamble_opening_pending: bool,
    preamble_opening_replay_pending: bool,
    preamble_start_pending: bool,
    cell_opening_pending: bool,
    next_cell_opening_pending: bool,
    align_peek_pending: bool,
    align_peek_after_noalign: bool,
    noalign_depth: Option<u32>,
    /// Frozen cell material retained for lifecycle diagnostics. The actual
    /// row records live on the alignment level, exactly as TeX82 §775 does.
    captured_rows: Vec<Vec<NodeListId>>,
    tabskip: tex_state::ids::GlueId,
    cell_span: u16,
    row_open: bool,
    cell_open: bool,
}

#[derive(Clone, Debug, Default)]
struct ReplayBoxes {
    pending_setbox: Option<SetBoxTarget>,
    pending_shipout: bool,
    pending_leader: Option<(GlueKind, LeaderPayload)>,
    active_boxes: Vec<ActiveReplayBox>,
    suspended_alignments: Vec<ActiveReplayAlignment>,
    recovery_simple_group_pending: bool,
    recovery_simple_group_open: bool,
    ordinary_simple_group_depth: u32,
    output_routine_active: bool,
    output_routine_opening_pending: bool,
    terminal_output_shipout_complete: bool,
}

/// The only normal reason a canonical operation may be retried by its host.
///
/// The command core has already classified the unavailable resource, while
/// this value deliberately retains neither a command nor a host capability.
/// Retrying therefore starts a fresh TeX82 §§24--25 processor episode at the
/// enclosing main-control operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CanonicalResourceNeed {
    /// TeX82's `start_input` scanned this logical filename (§529 / §1030+),
    /// but the host has not supplied its immutable source registration.
    Input { name: String },
    /// TeX82's `new_font` completed its filename and size scan (§1254), but
    /// the host has not supplied the immutable font bytes.
    Font { request: FontLoadRequest },
    /// pdfTeX's `scan_image` completed an immutable request, but its retained
    /// bytes and validated metadata have not been supplied by the host.
    PdfImage { request: PdfImageRequest },
}

/// Outcome of one atomic canonical main-control operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CanonicalStepResult {
    Progress(MainControlStep),
    Suspended(CanonicalResourceNeed),
}

/// All replay-owned state that must move with a bounded canonical operation.
/// Host capabilities are intentionally absent: their borrow ends before this
/// checkpoint can be restored.
struct CanonicalStepSnapshot {
    command: CommandStateSnapshot,
    modes: ModeNest,
    next_alignment_identity: u64,
    active_alignment: Option<ActiveReplayAlignment>,
    boxes: ReplayBoxes,
    output_dead_cycles: i32,
    completed_replay_episode: Option<tex_command::CommandReplayEpisode>,
    prepared_dvi_pages: Vec<crate::dispatch::PreparedDviPage>,
    universe: tex_state::Snapshot,
}

#[cfg(any(test, feature = "instrumentation"))]
#[derive(Default)]
struct ObservationBuffer(Vec<CommandObservation>);

#[cfg(any(test, feature = "instrumentation"))]
impl ObservationBuffer {
    fn flush_into(self, observer: &mut dyn CommandObserver) {
        for observation in self.0 {
            observer.committed(observation);
        }
    }
}

#[cfg(any(test, feature = "instrumentation"))]
impl CommandObserver for ObservationBuffer {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

impl CanonicalMainControl {
    /// Creates command-owned state without changing the shared `Universe`.
    ///
    /// Composed sessions use this when their profile/format initializer has
    /// already installed primitive meanings. [`Self::tex82_initex`] remains
    /// the explicit fresh-INITEX constructor.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a fresh command machine pinned to the selected compatibility
    /// profile.  Format loading installs primitive meanings separately, then
    /// uses this same constructor as a cold session.
    #[must_use]
    pub fn with_profile(profile: CommandProfile) -> Self {
        Self {
            command: CommandState::new(profile),
            ..Self::default()
        }
    }

    /// Creates a fresh canonical TeX82 INITEX replay environment.
    ///
    /// The primitive definitions are installed from the engine's static TeX82
    /// registries, before any fixture or host source is registered.
    #[must_use]
    pub fn tex82_initex(stores: &mut Universe) -> Self {
        tex_expand::install_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        install_write_stopper(stores);
        Self {
            command: CommandState::new(CommandProfile::TEX82),
            next_alignment_identity: 1,
            ..Self::default()
        }
    }

    /// Borrows canonical command state for source registration and snapshots.
    #[must_use]
    pub fn command_mut(&mut self) -> &mut CommandState {
        &mut self.command
    }

    /// Returns the immutable profile of this command processor.
    #[must_use]
    pub const fn command_profile(&self) -> CommandProfile {
        self.command.profile()
    }

    /// Captures a quiescent named checkpoint for this command processor.
    pub fn capture_checkpoint(
        &self,
        boundary: crate::EngineBoundary,
        stores: &mut Universe,
        budget_counters: crate::ExecutionBudgetCounters,
    ) -> Result<crate::EngineCheckpoint, tex_command::CommandSummaryError> {
        crate::EngineCheckpoint::capture_canonical(
            boundary,
            &self.command,
            &self.modes,
            stores,
            budget_counters,
        )
    }

    /// Restores a named checkpoint into this command processor.  The
    /// checkpoint is quiescent, so command-owned replay episodes are reset
    /// rather than serialized into a durable format or editor boundary.
    pub fn restore_checkpoint(
        &mut self,
        checkpoint: &crate::EngineCheckpoint,
        stores: &mut Universe,
    ) -> Result<(), crate::CanonicalCheckpointRestoreError> {
        checkpoint.restore_canonical_state(&mut self.command, &mut self.modes, stores)?;
        self.active_alignment = None;
        self.boxes = ReplayBoxes::default();
        Ok(())
    }

    /// Borrows executor-installed host capabilities for the next operation.
    #[must_use]
    pub fn capabilities_mut(&mut self) -> &mut CommandHostCapabilities {
        &mut self.capabilities
    }

    fn resolve_font_resource(&self, scanned: ScannedStep) -> Result<ScannedStep, ExecError> {
        let ScannedStep::FontDefinition {
            request, global, ..
        } = scanned
        else {
            return Ok(scanned);
        };
        let path = canonical_font_path(&request.name);
        let resource =
            self.capabilities
                .font(&path)
                .ok_or_else(|| ExecError::MissingCanonicalFont {
                    request: request.clone(),
                })?;
        Ok(ScannedStep::FontDefinition {
            request,
            resource: Box::new(Some(resource)),
            global,
        })
    }

    fn resolve_input_stream_resource(
        &self,
        scanned: ScannedStep,
    ) -> Result<ScannedStep, ExecError> {
        let ScannedStep::InputStream {
            request,
            resource: _,
        } = scanned
        else {
            return Ok(scanned);
        };
        let resource = match &request {
            InputStreamRequest::Open { file_name, .. } => Some(
                self.capabilities
                    .input_resource(&file_name.name)
                    .ok_or_else(|| ExecError::MissingCanonicalInput {
                        name: file_name.name.clone(),
                    })?,
            ),
            InputStreamRequest::Close { .. } | InputStreamRequest::Read { .. } => None,
        };
        Ok(ScannedStep::InputStream { request, resource })
    }

    fn resolve_pdf_image_resource(
        &self,
        scanned: ScannedStep,
        stores: &Universe,
    ) -> Result<ScannedStep, ExecError> {
        let ScannedStep::PdfXImage { mut request, .. } = scanned else {
            return Ok(scanned);
        };
        // pdfTeX checks \pdfoutput before it enters `scan_image`; in DVI
        // mode this must be the diagnostic, not a host-resource suspension.
        if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
            return Ok(ScannedStep::PdfXImage {
                request,
                resource: PdfImageResource::Unavailable,
            });
        }
        request.page_box = canonical_pdf_image_page_box(stores, &request);
        let resource = self.capabilities.pdf_image(&request).ok_or_else(|| {
            ExecError::MissingCanonicalPdfImage {
                request: request.clone(),
            }
        })?;
        Ok(ScannedStep::PdfXImage { request, resource })
    }

    /// Registers and opens the one root source selected by the host before
    /// canonical main control starts.  Source acquisition is deliberately
    /// complete before this call: the command state retains only immutable
    /// bytes and never reaches back into a host input stack.
    pub fn register_root_source(
        &mut self,
        source: SourceRegistration,
    ) -> Result<tex_state::SourceId, SourceRegistrationError> {
        let id = self.command.register_source(source)?;
        // `id` was just allocated by this command state, so this can fail
        // only if the state implementation has violated its own invariant.
        self.command
            .open_registered_source(id)
            .expect("freshly registered source must be openable");
        Ok(id)
    }

    /// Refreshes executor-owned mode facts for the next processor borrow.
    ///
    /// This is intentionally call-local capability state rather than part of
    /// a command snapshot or durable session summary.
    pub fn refresh_host_capabilities(&mut self) {
        self.capabilities
            .set_conditional_state(self.modes.conditional_state());
    }

    fn snapshot_step(&self, stores: &mut Universe) -> CanonicalStepSnapshot {
        CanonicalStepSnapshot {
            command: self.command.snapshot(),
            modes: self.modes.clone(),
            next_alignment_identity: self.next_alignment_identity,
            active_alignment: self.active_alignment.clone(),
            boxes: self.boxes.clone(),
            output_dead_cycles: self.output_dead_cycles,
            completed_replay_episode: self.completed_replay_episode,
            prepared_dvi_pages: self.prepared_dvi_pages.clone(),
            universe: stores.snapshot(),
        }
    }

    fn rollback_step(&mut self, snapshot: CanonicalStepSnapshot, stores: &mut Universe) {
        // This snapshot was created by this command state immediately before
        // the operation, so a profile mismatch would be an internal invariant
        // failure rather than a recoverable host condition.
        self.command
            .rollback(snapshot.command)
            .expect("canonical step snapshot keeps its command profile");
        // CommandRuntime is deliberately non-cloneable: its caches and
        // profiling cannot become semantic or durable state. Its fresh value
        // is therefore the canonical retry restoration form.
        self.runtime = CommandRuntime::default();
        self.modes = snapshot.modes;
        self.next_alignment_identity = snapshot.next_alignment_identity;
        self.active_alignment = snapshot.active_alignment;
        self.boxes = snapshot.boxes;
        self.output_dead_cycles = snapshot.output_dead_cycles;
        self.completed_replay_episode = snapshot.completed_replay_episode;
        self.prepared_dvi_pages = snapshot.prepared_dvi_pages;
        stores.rollback(&snapshot.universe);
    }

    /// Drains committed canonical shipout receipts in artifact order.
    ///
    /// Each plan was prepared during shipout and is retained only after the
    /// enclosing aggregate operation commits; finalizers must not re-lower
    /// these pages from artifact bytes.
    #[must_use]
    pub fn take_prepared_dvi_pages(&mut self) -> Vec<crate::dispatch::PreparedDviPage> {
        std::mem::take(&mut self.prepared_dvi_pages)
    }

    /// Returns the replay projection of TeX's current execution mode.
    #[must_use]
    pub fn current_mode(&self) -> Mode {
        self.modes.current_mode()
    }

    /// Returns the structural alignment started by the most recent replayed
    /// `\halign` or `\valign`, if it has not yet been finished.
    #[must_use]
    pub fn active_alignment(&self) -> Option<AlignmentIdentity> {
        self.active_alignment
            .as_ref()
            .map(|alignment| alignment.identity)
    }

    /// Applies an executor-selected alignment lifecycle transition.
    ///
    /// The request contains no token spelling, so this cannot create another
    /// delimiter-classification or source-consumption path.
    pub fn apply_alignment_request(&mut self, request: AlignmentRequest) -> Result<(), ExecError> {
        let finished = matches!(request, AlignmentRequest::Finish(_));
        let preamble = matches!(request, AlignmentRequest::Preamble(_));
        let identity = match request {
            AlignmentRequest::Begin(identity)
            | AlignmentRequest::Preamble(identity)
            | AlignmentRequest::PrepareCellLookahead(identity)
            | AlignmentRequest::InstallCellTemplate(identity)
            | AlignmentRequest::InstallOmitCellTemplate(identity)
            | AlignmentRequest::FinishCell(identity)
            | AlignmentRequest::RecoverExtraTab(identity)
            | AlignmentRequest::Suspend(identity)
            | AlignmentRequest::Resume(identity)
            | AlignmentRequest::Finish(identity) => identity,
            AlignmentRequest::BeginCell { alignment, .. } => alignment,
        };
        self.command
            .apply_alignment_request(request)
            .map(|_| ())
            .map_err(|_| ExecError::MissingToken {
                context: "alignment lifecycle",
            })?;
        if finished
            && self.active_alignment.as_ref().map(|active| active.identity) == Some(identity)
        {
            self.active_alignment = None;
            if let Some(outer) = self.boxes.suspended_alignments.pop() {
                self.command
                    .apply_alignment_request(AlignmentRequest::Resume(outer.identity))
                    .map_err(|_| ExecError::MissingToken {
                        context: "nested alignment resumption",
                    })?;
                self.active_alignment = Some(outer);
            }
        }
        if preamble
            && let Some(active) = self.active_alignment.as_mut()
            && active.identity == identity
        {
            active.preamble_opening_pending = false;
            active.preamble_opening_replay_pending = false;
        }
        Ok(())
    }

    /// Delivers one expanded command for an active alignment cell.
    ///
    /// In particular, the opaque end-template event is returned to the same
    /// command processor episode that delivered it, so the processor alone
    /// backs up the delimiter and installs the selected v-template.
    pub fn alignment_step(
        &mut self,
        alignment: AlignmentIdentity,
        stores: &mut Universe,
    ) -> Result<ReplayStep, ExecError> {
        let snapshot = self.snapshot_step(stores);
        let result = self.alignment_step_once(alignment, stores);
        if result.is_err() {
            self.rollback_step(snapshot, stores);
        }
        result
    }

    fn alignment_step_once(
        &mut self,
        alignment: AlignmentIdentity,
        stores: &mut Universe,
    ) -> Result<ReplayStep, ExecError> {
        let mode = self.modes.current_mode();
        let innermost_group = stores.innermost_group_kind();
        let scanned = {
            let mut processor = CommandProcessor::new(
                &mut self.command,
                &mut self.runtime,
                stores.command_context(),
                CommandHostContext::new(&mut self.capabilities),
            );
            scan_alignment_delivery_step(
                &mut processor,
                alignment,
                &ReplayBoxes::default(),
                innermost_group,
                mode,
            )?
        };
        let scanned = self.resolve_font_resource(scanned)?;
        let scanned = self.resolve_input_stream_resource(scanned)?;
        let scanned = self.resolve_pdf_image_resource(scanned, stores)?;
        if let ScannedStep::ReplayCompleted(episode) = scanned {
            self.completed_replay_episode = Some(episode);
            return Ok(ReplayStep::Continue);
        }
        if let ScannedStep::Math(request) = scanned {
            return self.apply_canonical_math_request(request, stores);
        }
        if let ScannedStep::MathDelimiter(boundary) = scanned {
            return self.apply_canonical_math_delimiter(boundary, stores);
        }
        if let ScannedStep::MathShift { paired } = scanned {
            return self.apply_canonical_math_shift(paired, stores);
        }
        if let ScannedStep::Discretionary(discretionary) = scanned {
            return self.apply_discretionary(discretionary, stores);
        }
        let fires_afterassignment = scanned.fires_afterassignment();
        let result = apply_scanned_step(
            scanned,
            stores,
            &mut self.modes,
            &mut self.next_alignment_identity,
            &mut self.active_alignment,
            &mut self.command,
            &mut self.boxes,
            &mut self.prepared_dvi_pages,
        )?;
        if fires_afterassignment {
            schedule_afterassignment(&mut self.command, stores);
        }
        Ok(result)
    }

    /// Executes TeX82's three completed discretionary parts as isolated
    /// restricted-horizontal episodes. The command machine owns replay of the
    /// immutable lists, including expansion, grouping recovery, and origins;
    /// this layer owns only the temporary mode/group lifecycle and final node.
    fn apply_discretionary(
        &mut self,
        discretionary: ScannedDiscretionary,
        stores: &mut Universe,
    ) -> Result<ReplayStep, ExecError> {
        if matches!(
            self.modes.current_mode(),
            Mode::Vertical | Mode::InternalVertical
        ) {
            start_canonical_paragraph(&mut self.command, &mut self.modes, stores, true)?;
        }
        crate::assignments::flush_pending_hchars(&mut self.modes, stores)?;
        let pre = self.execute_discretionary_part(discretionary.pre_break.tokens, stores)?;
        let post = self.execute_discretionary_part(discretionary.post_break.tokens, stores)?;
        let replace = self.execute_discretionary_part(discretionary.replacement.tokens, stores)?;
        self.modes.current_list_mut().push(Node::Disc {
            kind: DiscKind::Discretionary,
            pre,
            post,
            replace,
        });
        Ok(ReplayStep::Continue)
    }

    fn execute_discretionary_part(
        &mut self,
        tokens: TracedTokenList,
        stores: &mut Universe,
    ) -> Result<tex_state::ids::NodeListId, ExecError> {
        stores.enter_group_with_kind(GroupKind::Disc);
        self.modes.push(Mode::RestrictedHorizontal);
        let episode = self.command.push_discretionary_episode(tokens);
        while self.command.replay_episode_is_active(episode) {
            let _ = self.step_once(stores)?;
        }
        crate::assignments::flush_pending_hchars(&mut self.modes, stores)?;
        let level = self.modes.pop()?;
        let nodes = stores.freeze_node_list(level.list().nodes());
        let aftergroup =
            stores
                .leave_group_with_kind(GroupKind::Disc)
                .map_err(|_| ExecError::MissingToken {
                    context: "discretionary group",
                })?;
        if !aftergroup.is_empty() {
            let traced: Vec<_> = aftergroup
                .into_iter()
                .map(|token| {
                    let origin = stores.inserted_origin(
                        tex_state::provenance::InsertedOriginKind::AfterGroup,
                        token,
                        tex_state::token::OriginId::UNKNOWN,
                    );
                    tex_state::token::TracedTokenWord::pack(token, origin)
                })
                .collect();
            let aftergroup = self
                .command
                .push_aftergroup(stores.finish_traced_token_list(&traced));
            while self.command.replay_episode_is_active(aftergroup) {
                let _ = self.step_once(stores)?;
            }
        }
        Ok(nodes)
    }

    /// Attempts one atomic canonical main-control operation.
    ///
    /// Missing retained input rolls back the complete aggregate operation and
    /// is returned as a typed suspension. All other failures are restored and
    /// remain ordinary diagnostics. In either case the next call creates a
    /// fresh command processor; no delivered `CurrentCommand` is retained.
    pub fn advance(&mut self, stores: &mut Universe) -> Result<CanonicalStepResult, ExecError> {
        let snapshot = self.snapshot_step(stores);
        match self.step_once(stores) {
            Ok(step) => Ok(CanonicalStepResult::Progress(step)),
            Err(error) => {
                self.rollback_step(snapshot, stores);
                match error {
                    ExecError::MissingCanonicalInput { name } => Ok(
                        CanonicalStepResult::Suspended(CanonicalResourceNeed::Input { name }),
                    ),
                    ExecError::MissingCanonicalFont { request } => Ok(
                        CanonicalStepResult::Suspended(CanonicalResourceNeed::Font { request }),
                    ),
                    ExecError::MissingCanonicalPdfImage { request } => Ok(
                        CanonicalStepResult::Suspended(CanonicalResourceNeed::PdfImage { request }),
                    ),
                    error => Err(error),
                }
            }
        }
    }

    /// Delivers and executes one replay command through the command processor.
    ///
    /// Compatibility wrapper for callers which have not yet adopted typed
    /// resource suspension. New production hosts should use [`Self::advance`].
    pub fn step(&mut self, stores: &mut Universe) -> Result<ReplayStep, ExecError> {
        match self.advance(stores)? {
            CanonicalStepResult::Progress(step) => Ok(step),
            CanonicalStepResult::Suspended(CanonicalResourceNeed::Input { .. }) => {
                Err(ExecError::MissingToken { context: "\\input" })
            }
            CanonicalStepResult::Suspended(CanonicalResourceNeed::Font { .. }) => {
                Err(ExecError::MissingToken {
                    context: "\\font resource",
                })
            }
            CanonicalStepResult::Suspended(CanonicalResourceNeed::PdfImage { .. }) => {
                Err(ExecError::MissingToken {
                    context: "\\pdfximage resource",
                })
            }
        }
    }

    fn step_once(&mut self, stores: &mut Universe) -> Result<ReplayStep, ExecError> {
        self.refresh_host_capabilities();
        let mode = self.modes.current_mode();
        let starts_paragraph = matches!(mode, Mode::Vertical | Mode::InternalVertical);
        let alignment_preamble = alignment_preamble(self.active_alignment.as_mut());
        let innermost_group = stores.innermost_group_kind();
        let max_dead_cycles = stores.int_param(IntParam::MAX_DEAD_CYCLES);
        let scanned = {
            let mut processor = CommandProcessor::new(
                &mut self.command,
                &mut self.runtime,
                stores.command_context(),
                CommandHostContext::new(&mut self.capabilities),
            );
            scan_replay_step(
                &mut processor,
                mode,
                starts_paragraph,
                &self.boxes,
                alignment_preamble,
                innermost_group,
                &mut self.output_dead_cycles,
                max_dead_cycles,
                self.modes.current_list().display_eq_no().is_some(),
            )?
        };
        let scanned = self.resolve_font_resource(scanned)?;
        let scanned = self.resolve_input_stream_resource(scanned)?;
        let scanned = self.resolve_pdf_image_resource(scanned, stores)?;
        if let ScannedStep::ReplayCompleted(episode) = scanned {
            self.completed_replay_episode = Some(episode);
            return Ok(ReplayStep::Continue);
        }
        if let ScannedStep::Math(request) = scanned {
            return self.apply_canonical_math_request(request, stores);
        }
        if let ScannedStep::MathDelimiter(boundary) = scanned {
            return self.apply_canonical_math_delimiter(boundary, stores);
        }
        if let ScannedStep::MathShift { paired } = scanned {
            return self.apply_canonical_math_shift(paired, stores);
        }
        if let ScannedStep::Discretionary(discretionary) = scanned {
            return self.apply_discretionary(discretionary, stores);
        }
        let fires_afterassignment = scanned.fires_afterassignment();
        let artifact_count = stores.world().artifact_commits().len();
        let result = apply_scanned_step(
            scanned,
            stores,
            &mut self.modes,
            &mut self.next_alignment_identity,
            &mut self.active_alignment,
            &mut self.command,
            &mut self.boxes,
            &mut self.prepared_dvi_pages,
        )?;
        if fires_afterassignment {
            schedule_afterassignment(&mut self.command, stores);
        }
        if stores.world().artifact_commits().len() != artifact_count {
            self.output_dead_cycles = 0;
        }
        self.fire_pending_page_output(stores)?;
        Ok(result)
    }

    /// TeX82 §§1006--1028's typed page/output boundary.  The page builder
    /// and packing stay here with the mode nest; `CommandProcessor` alone
    /// installs the selected output token-list replay.
    fn fire_pending_page_output(&mut self, stores: &mut Universe) -> Result<(), ExecError> {
        while !self.boxes.output_routine_active {
            let Some(fire_up) = stores.page_fire_up() else {
                break;
            };
            match crate::output::select_pending_page_output(stores, fire_up)? {
                crate::output::SelectedPageOutput::Default(page) => {
                    if let Some(receipt) = shipout_replay_box(page, stores)? {
                        self.prepared_dvi_pages.push(receipt);
                    }
                }
                crate::output::SelectedPageOutput::UserRoutine => {
                    CommandProcessor::new(
                        &mut self.command,
                        &mut self.runtime,
                        stores.command_context(),
                        CommandHostContext::new(&mut self.capabilities),
                    )
                    .begin_selected_output_routine()
                    .map_err(command_error)?;
                    stores.enter_group_with_kind(GroupKind::Output);
                    self.modes.push(Mode::InternalVertical);
                    self.boxes.output_routine_active = true;
                    self.boxes.output_routine_opening_pending = true;
                }
            }
        }
        Ok(())
    }

    fn execute_math_episode(
        &mut self,
        episode: tex_command::CommandReplayEpisode,
        stores: &mut Universe,
    ) -> Result<tex_state::ids::NodeListId, ExecError> {
        self.modes.push(Mode::Math);
        loop {
            self.completed_replay_episode = None;
            match self.step_once(stores)? {
                ReplayStep::End | ReplayStep::EndOfInput => {
                    return Err(ExecError::MissingToken {
                        context: "math replay episode",
                    });
                }
                ReplayStep::Continue => {}
            }
            if self.completed_replay_episode == Some(episode) {
                break;
            }
        }
        while canonical_left_group_open(&self.modes, stores) {
            stores.world_mut().write_text(
                PrintSink::TerminalAndLog,
                "\n! Missing \\right. inserted.\n",
            );
            self.apply_canonical_math_delimiter(
                MathDelimiterBoundary {
                    kind: MathDelimiterBoundaryKind::Right,
                    delimiter: tex_command::ScannedMathDelimiter {
                        code: 0,
                        recovered: true,
                        provenance: tex_command::StructuredProvenance {
                            primary: tex_state::token::OriginId::UNKNOWN,
                        },
                    },
                },
                stores,
            )?;
        }
        let level = self.modes.pop()?;
        finish_canonical_math_list(
            level.list().nodes(),
            level.list().incomplete_fraction(),
            stores,
        )
    }

    fn execute_math_field(
        &mut self,
        field: tex_command::MathFieldEpisode,
        stores: &mut Universe,
    ) -> Result<MathField, ExecError> {
        if matches!(field.recovery, MathEpisodeRecovery::MissingField) {
            return Ok(MathField::Empty);
        }
        let episode = self.command.push_math_field_episode(field);
        let list = self.execute_math_episode(episode, stores)?;
        Ok(simplify_canonical_math_field(stores, list))
    }

    fn execute_math_group(
        &mut self,
        group: tex_command::MathGroupEpisode,
        stores: &mut Universe,
    ) -> Result<tex_state::ids::NodeListId, ExecError> {
        let episode = self.command.push_math_group_episode(group);
        self.execute_math_episode(episode, stores)
    }

    fn apply_canonical_math_request(
        &mut self,
        request: CanonicalMathRequest,
        stores: &mut Universe,
    ) -> Result<ReplayStep, ExecError> {
        match request {
            CanonicalMathRequest::Character(value) => {
                append_canonical_math_char(
                    self.modes.current_list_mut(),
                    stores,
                    u32::from(value.code),
                    value.provenance.primary,
                )?;
            }
            CanonicalMathRequest::Delimiter(value) => {
                append_canonical_math_char(
                    self.modes.current_list_mut(),
                    stores,
                    value.code >> 12,
                    value.provenance.primary,
                )?;
            }
            CanonicalMathRequest::TextField(kind) => {
                let episode = self.command_scan_math_field(stores)?;
                let field = self.execute_math_field(episode, stores)?;
                self.modes
                    .current_list_mut()
                    .push(Node::MathNoad(MathNoad::new(
                        noad_kind_for_text(kind),
                        field,
                    )));
            }
            CanonicalMathRequest::Script(script) => {
                let episode = self.command_scan_math_script(script.kind, stores)?;
                let field = self.execute_math_field(episode, stores)?;
                attach_canonical_script(self.modes.current_list_mut(), stores, field, script.kind);
            }
            CanonicalMathRequest::Limits(kind) => {
                apply_canonical_limits(self.modes.current_list_mut(), stores, kind)
            }
            CanonicalMathRequest::Fraction(fraction) => {
                start_canonical_fraction(self.modes.current_list_mut(), stores, fraction)
            }
            CanonicalMathRequest::Style(style) => {
                self.modes
                    .current_list_mut()
                    .push(Node::MathStyle(match style {
                        MathStyleKind::Display => MathStyle::Display,
                        MathStyleKind::Text => MathStyle::Text,
                        MathStyleKind::Script => MathStyle::Script,
                        MathStyleKind::ScriptScript => MathStyle::ScriptScript,
                    }))
            }
            CanonicalMathRequest::Choice => {
                let choices = self.command_scan_math_choice(stores)?;
                let display = self.execute_math_group(choices.display, stores)?;
                let text = self.execute_math_group(choices.text, stores)?;
                let script = self.execute_math_group(choices.script, stores)?;
                let script_script = self.execute_math_group(choices.scriptscript, stores)?;
                self.modes
                    .current_list_mut()
                    .push(Node::MathChoice(MathChoice {
                        display,
                        text,
                        script,
                        script_script,
                    }));
            }
            CanonicalMathRequest::Radical(delimiter) => {
                let episode = self.command_scan_math_field(stores)?;
                let field = self.execute_math_field(episode, stores)?;
                self.modes
                    .current_list_mut()
                    .push(Node::MathNoad(MathNoad::new(
                        NoadKind::Radical {
                            delimiter: delimiter.code,
                        },
                        field,
                    )));
            }
            CanonicalMathRequest::Accent(accent) => {
                let episode = self.command_scan_math_field(stores)?;
                let field = self.execute_math_field(episode, stores)?;
                let accent =
                    canonical_math_char(stores, u32::from(accent.code), accent.provenance.primary)?
                        .1;
                self.modes
                    .current_list_mut()
                    .push(Node::MathNoad(MathNoad::new(
                        NoadKind::Accent { accent },
                        field,
                    )));
            }
            CanonicalMathRequest::MuMaterial(ScannedMathMuMaterial::Glue(glue)) => {
                self.modes.current_list_mut().push(Node::Glue {
                    spec: stores.intern_glue(glue),
                    kind: GlueKind::MuSkip,
                    leader: None,
                })
            }
            CanonicalMathRequest::MuMaterial(ScannedMathMuMaterial::Kern(amount)) => {
                self.modes.current_list_mut().push(Node::Kern {
                    amount,
                    kind: KernKind::Mu,
                })
            }
            CanonicalMathRequest::EquationNumber(number) => {
                if self.modes.current_mode() != Mode::DisplayMath {
                    stores.world_mut().write_text(
                        PrintSink::TerminalAndLog,
                        "\n! You can't use `\\eqno' in this mode.\n",
                    );
                } else {
                    let display = take_finished_canonical_math_list(&mut self.modes, stores)?;
                    stores.enter_group_with_kind(GroupKind::MathShift);
                    stores.set_int_param(IntParam::FAM, -1);
                    self.modes.push(Mode::Math);
                    self.modes
                        .current_list_mut()
                        .set_display_eq_no(crate::mode::DisplayEqNo {
                            side: match number.side {
                                tex_command::EquationNumberSide::Left => {
                                    crate::mode::EqNoSide::Left
                                }
                                tex_command::EquationNumberSide::Right => {
                                    crate::mode::EqNoSide::Right
                                }
                            },
                            display,
                        });
                }
            }
            CanonicalMathRequest::Family(_) => {}
        }
        Ok(ReplayStep::Continue)
    }

    fn apply_canonical_math_shift(
        &mut self,
        paired: bool,
        stores: &mut Universe,
    ) -> Result<ReplayStep, ExecError> {
        match self.modes.current_mode() {
            Mode::Horizontal | Mode::RestrictedHorizontal => {
                crate::assignments::flush_pending_hchars(&mut self.modes, stores)?;
                if paired && self.modes.current_mode() == Mode::Horizontal {
                    self.enter_canonical_display(stores)?;
                } else {
                    self.enter_canonical_math(false, stores);
                }
            }
            Mode::Math => {
                if self.modes.current_list().display_eq_no().is_some() {
                    if !paired {
                        stores.world_mut().write_text(
                            PrintSink::TerminalAndLog,
                            "\n! Display math should end with $$.\n",
                        );
                    }
                    self.finish_canonical_equation_number(stores)?;
                } else {
                    self.finish_canonical_inline_math(stores)?;
                }
            }
            Mode::DisplayMath => {
                if !paired {
                    stores.world_mut().write_text(
                        PrintSink::TerminalAndLog,
                        "\n! Display math should end with $$.\n",
                    );
                }
                self.finish_canonical_display_math(stores, None)?;
            }
            Mode::Vertical | Mode::InternalVertical => {
                unreachable!("vertical math shifts retry through ParagraphStart")
            }
        }
        Ok(ReplayStep::Continue)
    }

    fn enter_canonical_math(&mut self, display: bool, stores: &mut Universe) {
        stores.enter_group_with_kind(GroupKind::MathShift);
        stores.set_int_param(IntParam::FAM, -1);
        self.modes.push(if display {
            Mode::DisplayMath
        } else {
            Mode::Math
        });
        schedule_everymath(&mut self.command, stores, display);
    }

    fn enter_canonical_display(&mut self, stores: &mut Universe) -> Result<(), ExecError> {
        let paragraph =
            crate::assignments::interrupt_canonical_paragraph_for_display(&mut self.modes, stores)?;
        let dimensions = crate::assignments::display_line_dimensions(&self.modes, stores);
        let pre_display_size = paragraph
            .last_line
            .as_ref()
            .map_or(Scaled::from_raw(-Scaled::MAX_DIMEN.raw()), |line| {
                crate::math::display::pre_display_size(stores, line)
            });
        stores.set_dimen_param(DimenParam::PRE_DISPLAY_SIZE, pre_display_size);
        stores.set_dimen_param(DimenParam::DISPLAY_WIDTH, dimensions.width);
        stores.set_dimen_param(DimenParam::DISPLAY_INDENT, dimensions.indent);
        stores.set_int_param(
            IntParam::PRE_DISPLAY_DIRECTION,
            match paragraph.active_directions.last() {
                Some(tex_state::node::Direction::BeginL) => 1,
                Some(tex_state::node::Direction::BeginR) => -1,
                _ => 0,
            },
        );
        self.enter_canonical_math(true, stores);
        self.modes
            .current_list_mut()
            .set_display_interrupt(crate::mode::DisplayInterrupt {
                active_directions: paragraph.active_directions,
            });
        Ok(())
    }

    fn finish_canonical_inline_math(&mut self, stores: &mut Universe) -> Result<(), ExecError> {
        let content = take_finished_canonical_math_list(&mut self.modes, stores)?;
        let _ = self.modes.pop()?;
        let insert_penalties = self.modes.current_mode() == Mode::Horizontal;
        let (nodes, _) = crate::math::finish_inline_math_list_node(
            stores,
            tex_state::math::MathListNode {
                display: false,
                content,
            },
            insert_penalties,
        );
        self.modes.current_list_mut().append(nodes);
        self.modes.current_list_mut().set_space_factor(1000);
        let aftergroup = stores
            .leave_group_with_kind(GroupKind::MathShift)
            .map_err(|_| ExecError::MissingToken {
                context: "math shift group",
            })?;
        schedule_aftergroup(&mut self.command, stores, aftergroup);
        Ok(())
    }

    fn finish_canonical_equation_number(&mut self, stores: &mut Universe) -> Result<(), ExecError> {
        let content = take_finished_canonical_math_list(&mut self.modes, stores)?;
        let mut level = self.modes.pop()?;
        let eq = level
            .list_mut()
            .take_display_eq_no()
            .expect("equation number mode state");
        let finished = crate::math::display::finish_eq_no(stores, eq.side, content);
        let aftergroup = stores
            .leave_group_with_kind(GroupKind::MathShift)
            .map_err(|_| ExecError::MissingToken {
                context: "equation number group",
            })?;
        schedule_aftergroup(&mut self.command, stores, aftergroup);
        self.finish_canonical_display_math(stores, Some(finished))
    }

    fn finish_canonical_display_math(
        &mut self,
        stores: &mut Universe,
        eq_no: Option<crate::math::display::FinishedEqNo>,
    ) -> Result<(), ExecError> {
        let content = take_finished_canonical_math_list(&mut self.modes, stores)?;
        let mut level = self.modes.pop()?;
        let interrupt =
            level
                .list_mut()
                .take_display_interrupt()
                .ok_or(ExecError::MissingToken {
                    context: "display interrupt",
                })?;
        crate::math::display::finish_display_math(&mut self.modes, stores, content, eq_no)?;
        let aftergroup = stores
            .leave_group_with_kind(GroupKind::MathShift)
            .map_err(|_| ExecError::MissingToken {
                context: "display math group",
            })?;
        schedule_aftergroup(&mut self.command, stores, aftergroup);
        self.resume_canonical_display(stores, interrupt.active_directions)
    }

    fn resume_canonical_display(
        &mut self,
        stores: &mut Universe,
        directions: Vec<tex_state::node::Direction>,
    ) -> Result<(), ExecError> {
        let prev = self
            .modes
            .enclosing_vertical_prev_graf()
            .checked_add(3)
            .expect("display prev_graf overflow");
        self.modes.set_enclosing_vertical_prev_graf(prev);
        self.modes.push(Mode::Horizontal);
        self.modes.current_list_mut().set_space_factor(1000);
        self.modes
            .current_list_mut()
            .append(directions.into_iter().map(Node::Direction));
        crate::vertical::build_page_if_outer_vertical(&self.modes, stores)
    }

    fn apply_canonical_math_delimiter(
        &mut self,
        boundary: MathDelimiterBoundary,
        stores: &mut Universe,
    ) -> Result<ReplayStep, ExecError> {
        match boundary.kind {
            MathDelimiterBoundaryKind::Left => {
                self.modes.push(Mode::Math);
                self.modes
                    .current_list_mut()
                    .push(Node::MathNoad(MathNoad::new(
                        NoadKind::LeftDelimiter {
                            delimiter: boundary.delimiter.code,
                        },
                        MathField::Empty,
                    )));
            }
            MathDelimiterBoundaryKind::Middle => {
                if canonical_left_group_open(&self.modes, stores) {
                    self.modes
                        .current_list_mut()
                        .push(Node::MathNoad(MathNoad::new(
                            NoadKind::MiddleDelimiter {
                                delimiter: boundary.delimiter.code,
                            },
                            MathField::Empty,
                        )));
                } else {
                    stores
                        .world_mut()
                        .write_text(PrintSink::TerminalAndLog, "\n! Extra \\middle.\n");
                }
            }
            MathDelimiterBoundaryKind::Right => {
                if !canonical_left_group_open(&self.modes, stores) {
                    stores
                        .world_mut()
                        .write_text(PrintSink::TerminalAndLog, "\n! Extra \\right.\n");
                    return Ok(ReplayStep::Continue);
                }
                let content = take_finished_canonical_math_list(&mut self.modes, stores)?;
                let _ = self.modes.pop()?;
                let mut nodes: Vec<_> = stores
                    .nodes(content)
                    .into_iter()
                    .map(|node| node.to_owned())
                    .collect();
                nodes.push(Node::MathNoad(MathNoad::new(
                    NoadKind::RightDelimiter {
                        delimiter: boundary.delimiter.code,
                    },
                    MathField::Empty,
                )));
                let content = stores.freeze_node_list(&nodes);
                self.modes
                    .current_list_mut()
                    .push(Node::MathNoad(MathNoad::new(
                        NoadKind::Normal(NoadClass::Inner),
                        MathField::SubMlist(content),
                    )));
            }
        }
        Ok(ReplayStep::Continue)
    }

    fn command_scan_math_field(
        &mut self,
        stores: &mut Universe,
    ) -> Result<tex_command::MathFieldEpisode, ExecError> {
        CommandProcessor::new(
            &mut self.command,
            &mut self.runtime,
            stores.command_context(),
            CommandHostContext::new(&mut self.capabilities),
        )
        .scan_math_field_episode()
        .map_err(command_error)
    }

    fn command_scan_math_script(
        &mut self,
        kind: MathScriptKind,
        stores: &mut Universe,
    ) -> Result<tex_command::MathFieldEpisode, ExecError> {
        CommandProcessor::new(
            &mut self.command,
            &mut self.runtime,
            stores.command_context(),
            CommandHostContext::new(&mut self.capabilities),
        )
        .scan_math_script_attachment(kind)
        .map(|attachment| attachment.field)
        .map_err(command_error)
    }

    fn command_scan_math_choice(
        &mut self,
        stores: &mut Universe,
    ) -> Result<tex_command::MathChoiceEpisodes, ExecError> {
        CommandProcessor::new(
            &mut self.command,
            &mut self.runtime,
            stores.command_context(),
            CommandHostContext::new(&mut self.capabilities),
        )
        .scan_math_choice_episodes()
        .map_err(command_error)
    }

    /// Delivers and executes one replay command while forwarding committed
    /// command-owned observations in their original order.
    #[cfg(any(test, feature = "instrumentation"))]
    pub fn step_with_observer(
        &mut self,
        stores: &mut Universe,
        observer: &mut dyn CommandObserver,
    ) -> Result<ReplayStep, ExecError> {
        match self.advance_with_observer(stores, observer)? {
            CanonicalStepResult::Progress(step) => Ok(step),
            CanonicalStepResult::Suspended(CanonicalResourceNeed::Input { .. }) => {
                Err(ExecError::MissingToken { context: "\\input" })
            }
            CanonicalStepResult::Suspended(CanonicalResourceNeed::Font { .. }) => {
                Err(ExecError::MissingToken {
                    context: "\\font resource",
                })
            }
            CanonicalStepResult::Suspended(CanonicalResourceNeed::PdfImage { .. }) => {
                Err(ExecError::MissingToken {
                    context: "\\pdfximage resource",
                })
            }
        }
    }

    /// Atomic observed variant of [`Self::advance`]. Observations are held
    /// until both command delivery and executor application have committed.
    #[cfg(any(test, feature = "instrumentation"))]
    pub fn advance_with_observer(
        &mut self,
        stores: &mut Universe,
        observer: &mut dyn CommandObserver,
    ) -> Result<CanonicalStepResult, ExecError> {
        let snapshot = self.snapshot_step(stores);
        let mut pending = ObservationBuffer::default();
        match self.step_with_observer_once(stores, &mut pending) {
            Ok(step) => {
                pending.flush_into(observer);
                Ok(CanonicalStepResult::Progress(step))
            }
            Err(error) => {
                self.rollback_step(snapshot, stores);
                match error {
                    ExecError::MissingCanonicalInput { name } => Ok(
                        CanonicalStepResult::Suspended(CanonicalResourceNeed::Input { name }),
                    ),
                    ExecError::MissingCanonicalFont { request } => Ok(
                        CanonicalStepResult::Suspended(CanonicalResourceNeed::Font { request }),
                    ),
                    ExecError::MissingCanonicalPdfImage { request } => Ok(
                        CanonicalStepResult::Suspended(CanonicalResourceNeed::PdfImage { request }),
                    ),
                    error => Err(error),
                }
            }
        }
    }

    #[cfg(any(test, feature = "instrumentation"))]
    fn step_with_observer_once(
        &mut self,
        stores: &mut Universe,
        observer: &mut dyn CommandObserver,
    ) -> Result<ReplayStep, ExecError> {
        // Observation is an instrumentation boundary, not an alternate
        // execution mode. Keep the command processor's borrowed mode facts
        // identical to an unobserved step (notably for \ifhmode after a
        // paragraph-start transition).
        self.refresh_host_capabilities();
        let mode = self.modes.current_mode();
        let starts_paragraph = matches!(mode, Mode::Vertical | Mode::InternalVertical);
        let alignment_preamble = alignment_preamble(self.active_alignment.as_mut());
        let innermost_group = stores.innermost_group_kind();
        let max_dead_cycles = stores.int_param(IntParam::MAX_DEAD_CYCLES);
        let scanned = {
            let mut processor = CommandProcessor::new(
                &mut self.command,
                &mut self.runtime,
                stores.command_context(),
                CommandHostContext::new(&mut self.capabilities),
            )
            .with_observer(observer);
            scan_replay_step(
                &mut processor,
                mode,
                starts_paragraph,
                &self.boxes,
                alignment_preamble,
                innermost_group,
                &mut self.output_dead_cycles,
                max_dead_cycles,
                self.modes.current_list().display_eq_no().is_some(),
            )?
        };
        let scanned = self.resolve_font_resource(scanned)?;
        let scanned = self.resolve_input_stream_resource(scanned)?;
        let scanned = self.resolve_pdf_image_resource(scanned, stores)?;
        if let ScannedStep::ReplayCompleted(episode) = scanned {
            self.completed_replay_episode = Some(episode);
            return Ok(ReplayStep::Continue);
        }
        if let ScannedStep::Math(request) = scanned {
            return self.apply_canonical_math_request(request, stores);
        }
        if let ScannedStep::MathDelimiter(boundary) = scanned {
            return self.apply_canonical_math_delimiter(boundary, stores);
        }
        if let ScannedStep::Discretionary(discretionary) = scanned {
            return self.apply_discretionary(discretionary, stores);
        }
        let mutation = applied_mutation_observation(&scanned, stores);
        let begins_alignment = matches!(&scanned, ScannedStep::BeginAlignment { .. });
        let suspends_alignment = begins_alignment && self.active_alignment.is_some();
        let begins_alignment_cell = matches!(&scanned, ScannedStep::AlignmentPreambleStart { .. });
        let installs_u_template = match &scanned {
            ScannedStep::AlignmentCellOpening {
                alignment,
                opening: AlignmentCellOpening::Template,
            } => Some(*alignment),
            // `align_peek` already fetched and backed up the first nonblank
            // command before it calls TeX82's `init_col`.
            ScannedStep::AlignmentPeekCell {
                alignment,
                omit: false,
            } => Some(*alignment),
            _ => None,
        };
        let installs_omit_cell = match &scanned {
            ScannedStep::AlignmentCellOpening {
                alignment,
                opening: AlignmentCellOpening::Omit,
            } => Some(*alignment),
            ScannedStep::AlignmentPeekCell {
                alignment,
                omit: true,
            } => Some(*alignment),
            _ => None,
        };
        let finishes_alignment_cell = match &scanned {
            ScannedStep::AlignmentCellFinish { alignment } => {
                self.command.alignment_cell_finish_observations(*alignment)
            }
            _ => None,
        };
        let completes_alignment_cell = matches!(&scanned, ScannedStep::AlignmentCellFinish { .. });
        let finishes_alignment = match &scanned {
            ScannedStep::AlignmentFinish { alignment } => {
                self.command.alignment_finish_observation(*alignment)
            }
            _ => None,
        };
        let fires_afterassignment = scanned.fires_afterassignment();
        let result = apply_scanned_step(
            scanned.clone(),
            stores,
            &mut self.modes,
            &mut self.next_alignment_identity,
            &mut self.active_alignment,
            &mut self.command,
            &mut self.boxes,
            &mut self.prepared_dvi_pages,
        );
        if result.is_ok() && fires_afterassignment {
            schedule_afterassignment(&mut self.command, stores);
        }
        let extra_tab_recovery = result
            .as_ref()
            .ok()
            .filter(|_| completes_alignment_cell)
            .and_then(|_| self.command.take_alignment_extra_tab_recovery_observation());
        if result.is_ok() {
            let effect = applied_effect_observation(&scanned, stores);
            if suspends_alignment
                && let Some(alignment) = self.command.alignment_suspend_observation()
            {
                observer.committed(CommandObservation::Alignment(alignment));
            }
            if begins_alignment && let Some(alignment) = self.command.alignment_begin_observation()
            {
                observer.committed(CommandObservation::Alignment(alignment));
            }
            if begins_alignment_cell
                && let Some(alignment) = self.command.alignment_cell_begin_observation()
            {
                observer.committed(CommandObservation::Alignment(alignment));
            }
            if let Some(alignment) = installs_u_template
                && let Some(input) = self
                    .command
                    .alignment_u_template_push_observation(alignment)
            {
                observer.committed(CommandObservation::Input(input));
                if let Some(template) = self
                    .command
                    .alignment_u_template_push_alignment_observation(alignment)
                {
                    observer.committed(CommandObservation::Alignment(template));
                }
            }
            if let Some(alignment) = installs_omit_cell
                && let Some(omit) = self.command.alignment_omit_cell_observation(alignment)
            {
                observer.committed(CommandObservation::Alignment(omit));
            }
            if let Some(recovery) = extra_tab_recovery {
                observer.committed(CommandObservation::Alignment(recovery));
            }
            if let Some(finish) = finishes_alignment_cell {
                observer.committed(CommandObservation::Alignment(finish.state_change));
                if let Some(retirement) = finish.backed_up_endv_retirement {
                    observer.committed(CommandObservation::Input(retirement));
                }
                observer.committed(CommandObservation::Input(finish.v_template_retirement));
                observer.committed(CommandObservation::Alignment(finish.template_retirement));
            }
            if let Some(finish) = finishes_alignment {
                observer.committed(CommandObservation::Alignment(finish));
                if let Some(resume) = self.command.alignment_resume_observation() {
                    observer.committed(CommandObservation::Alignment(resume));
                }
            }
            if let Some(mutation) = mutation {
                observer.committed(CommandObservation::Mutation(mutation));
            }
            if let Some(effect) = effect {
                observer.committed(CommandObservation::Effect(effect));
            }
        }
        result
    }

    /// Scans TeX's initial terminal filename through the canonical command
    /// path, retaining every committed observation for the caller.
    #[cfg(any(test, feature = "instrumentation"))]
    pub fn scan_startup_file_name(
        &mut self,
        stores: &mut Universe,
        observer: &mut dyn CommandObserver,
    ) -> Result<String, ExecError> {
        let filename =
            {
                let mut processor = CommandProcessor::new(
                    &mut self.command,
                    &mut self.runtime,
                    stores.command_context(),
                    CommandHostContext::new(&mut self.capabilities),
                )
                .with_observer(observer);
                let first = processor.get_x_token().map_err(command_error)?.ok_or(
                    ExecError::MissingToken {
                        context: "terminal filename",
                    },
                )?;
                processor.back_input(first).map_err(command_error)?;
                let mut filename = String::new();
                loop {
                    let command = processor.get_x_token().map_err(command_error)?.ok_or(
                        ExecError::MissingToken {
                            context: "terminal filename",
                        },
                    )?;
                    match command.spelling().semantic_token() {
                        Token::Char {
                            cat: Catcode::Space,
                            ..
                        } => break filename,
                        Token::Char { ch, .. } => filename.push(ch),
                        _ => {
                            return Err(ExecError::MissingToken {
                                context: "terminal filename character",
                            });
                        }
                    }
                }
            };
        // The terminal line supplies only the startup filename.  It is not a
        // normal file-input level beneath the selected root, so retire its
        // exhausted source silently before main control starts.  The eventual
        // terminal stop is then emitted by command input after the root file
        // retires (TeX82 §46 final cleanup).
        let terminal_exhausted = CommandProcessor::new(
            &mut self.command,
            &mut self.runtime,
            stores.command_context(),
            CommandHostContext::new(&mut self.capabilities),
        )
        .get_x_token()
        .map_err(command_error)?
        .is_none();
        if !terminal_exhausted {
            return Err(ExecError::MissingToken {
                context: "terminal filename terminator",
            });
        }
        self.capabilities.set_startup_job_name(&filename);
        Ok(filename)
    }
}

/// Installs TeX82 §53's inaccessible outer `\\endwrite` sentinel.
///
/// It is a frozen primitive spelling backed by an empty outer macro so the
/// command core can observe its raw stopper delivery without exposing a
/// fixture-specific marker to ordinary source input.
fn install_write_stopper(stores: &mut Universe) {
    let empty = stores.intern_token_list(&[]);
    let definition = stores.intern_macro(MacroMeaning::new(MeaningFlags::OUTER, empty, empty));
    let meaning = Meaning::Macro {
        flags: MeaningFlags::OUTER,
        definition,
    };
    stores.install_primitive_meaning("endwrite", meaning);
}

/// The structural outcome of one canonical main-control operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MainControlStep {
    Continue,
    EndOfInput,
    End,
}

// The fixture suite retains its historical vocabulary locally.  This alias is
// deliberately unavailable to normal builds: production code names and uses
// the canonical driver directly.
#[cfg(test)]
type CommandReplayControl = CanonicalMainControl;

// Kept private while the implementation is migrated in place; callers only
// see `MainControlStep`.
type ReplayStep = MainControlStep;

fn canonical_math_char(
    stores: &Universe,
    code: u32,
    origin: tex_state::token::OriginId,
) -> Result<(NoadClass, MathChar), ExecError> {
    if code > 0x7fff {
        return Err(ExecError::InvalidCode {
            context: "\\mathchar",
            value: code as i32,
        });
    }
    let class = match (code >> 12) & 7 {
        0 => NoadClass::Ord,
        1 => NoadClass::Op,
        2 => NoadClass::Bin,
        3 => NoadClass::Rel,
        4 => NoadClass::Open,
        5 => NoadClass::Close,
        6 => NoadClass::Punct,
        _ => NoadClass::Ord,
    };
    let mut family = ((code >> 8) & 15) as u8;
    if ((code >> 12) & 7) == 7 {
        let fam = stores.int_param(IntParam::FAM);
        if (0..16).contains(&fam) {
            family = fam as u8;
        }
    }
    Ok((
        class,
        MathChar {
            family,
            character: char::from_u32(code & 0xff).unwrap_or('\0'),
            origin,
        },
    ))
}

fn append_canonical_math_char(
    list: &mut crate::ModeList,
    stores: &Universe,
    code: u32,
    origin: tex_state::token::OriginId,
) -> Result<(), ExecError> {
    let (class, character) = canonical_math_char(stores, code, origin)?;
    list.push(Node::MathNoad(MathNoad::new(
        NoadKind::Normal(class),
        MathField::MathChar(character),
    )));
    Ok(())
}

fn noad_kind_for_text(kind: MathTextFieldKind) -> NoadKind {
    match kind {
        MathTextFieldKind::Ord => NoadKind::Normal(NoadClass::Ord),
        MathTextFieldKind::Op => NoadKind::Normal(NoadClass::Op),
        MathTextFieldKind::Bin => NoadKind::Normal(NoadClass::Bin),
        MathTextFieldKind::Rel => NoadKind::Normal(NoadClass::Rel),
        MathTextFieldKind::Open => NoadKind::Normal(NoadClass::Open),
        MathTextFieldKind::Close => NoadKind::Normal(NoadClass::Close),
        MathTextFieldKind::Punct => NoadKind::Normal(NoadClass::Punct),
        MathTextFieldKind::Inner => NoadKind::Normal(NoadClass::Inner),
        MathTextFieldKind::Underline => NoadKind::Underline,
        MathTextFieldKind::Overline => NoadKind::Overline,
        MathTextFieldKind::VCenter => NoadKind::VCenter,
    }
}

fn simplify_canonical_math_field(stores: &Universe, list: tex_state::ids::NodeListId) -> MathField {
    let nodes = stores.nodes(list);
    if nodes.len() == 1
        && let Some(tex_state::node_arena::NodeRef::MathNoad(noad)) = nodes.first()
        && matches!(noad.kind, NoadKind::Normal(NoadClass::Ord))
        && matches!(noad.subscript, MathField::Empty)
        && matches!(noad.superscript, MathField::Empty)
    {
        noad.nucleus
    } else {
        MathField::SubMlist(list)
    }
}

fn attach_canonical_script(
    list: &mut crate::ModeList,
    stores: &mut Universe,
    field: MathField,
    kind: MathScriptKind,
) {
    let Some(mut node) = list.pop_last_node() else {
        let mut noad = MathNoad::new(NoadKind::Normal(NoadClass::Ord), MathField::Empty);
        if kind == MathScriptKind::Superscript {
            noad.superscript = field
        } else {
            noad.subscript = field
        };
        list.push(Node::MathNoad(noad));
        return;
    };
    if let Node::MathNoad(noad) = &mut node {
        let target = if kind == MathScriptKind::Superscript {
            &mut noad.superscript
        } else {
            &mut noad.subscript
        };
        if matches!(target, MathField::Empty) {
            *target = field;
            list.push(node);
            return;
        }
    }
    list.push(node);
    let mut noad = MathNoad::new(NoadKind::Normal(NoadClass::Ord), MathField::Empty);
    if kind == MathScriptKind::Superscript {
        noad.superscript = field
    } else {
        noad.subscript = field
    };
    list.push(Node::MathNoad(noad));
    stores
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, "\n! Double math script.\n");
}

fn apply_canonical_limits(list: &mut crate::ModeList, _stores: &mut Universe, kind: MathLimitKind) {
    if let Some(Node::MathNoad(noad)) = list.last_node_mut()
        && matches!(
            noad.kind,
            NoadKind::Normal(NoadClass::Op) | NoadKind::Operator(_)
        )
    {
        noad.kind = NoadKind::Operator(match kind {
            MathLimitKind::Limits => LimitType::Limits,
            MathLimitKind::NoLimits => LimitType::NoLimits,
            MathLimitKind::DisplayLimits => LimitType::DisplayLimits,
        });
    }
}

fn start_canonical_fraction(
    list: &mut crate::ModeList,
    stores: &mut Universe,
    fraction: tex_command::ScannedMathFraction,
) {
    if list.incomplete_fraction().is_some() {
        return;
    }
    let numerator = stores.freeze_node_list(&list.take_nodes());
    list.set_incomplete_fraction(crate::mode::IncompleteFraction {
        numerator,
        thickness: match fraction.thickness {
            Some(value) => FractionThickness::Explicit(value),
            None => FractionThickness::Default,
        },
        left_delimiter: fraction.left_delimiter.map(|value| value.code),
        right_delimiter: fraction.right_delimiter.map(|value| value.code),
    });
}

fn finish_canonical_math_list(
    nodes: &[Node],
    incomplete: Option<&crate::mode::IncompleteFraction>,
    stores: &mut Universe,
) -> Result<tex_state::ids::NodeListId, ExecError> {
    let mut output = nodes.to_vec();
    if let Some(fraction) = incomplete {
        let denominator = stores.freeze_node_list(&output);
        output = vec![Node::FractionNoad(MathFraction {
            numerator: fraction.numerator,
            denominator,
            thickness: fraction.thickness,
            left_delimiter: fraction.left_delimiter,
            right_delimiter: fraction.right_delimiter,
        })];
    }
    Ok(stores.freeze_node_list(&output))
}

fn take_finished_canonical_math_list(
    modes: &mut ModeNest,
    stores: &mut Universe,
) -> Result<tex_state::ids::NodeListId, ExecError> {
    let (nodes, incomplete) = {
        let list = modes.current_list_mut();
        (list.take_nodes(), list.take_incomplete_fraction())
    };
    finish_canonical_math_list(&nodes, incomplete.as_ref(), stores)
}

fn canonical_left_group_open(modes: &ModeNest, stores: &Universe) -> bool {
    let starts_left_node = |node: Option<&Node>| {
        matches!(
            node,
            Some(Node::MathNoad(MathNoad {
                kind: NoadKind::LeftDelimiter { .. },
                ..
            }))
        )
    };
    let starts_left_ref = |node: Option<tex_state::node_arena::NodeRef<'_>>| {
        matches!(
            node,
            Some(tex_state::node_arena::NodeRef::MathNoad(MathNoad {
                kind: NoadKind::LeftDelimiter { .. },
                ..
            }))
        )
    };
    starts_left_node(modes.current_list().nodes().first())
        || modes
            .current_list()
            .incomplete_fraction()
            .is_some_and(|fraction| starts_left_ref(stores.nodes(fraction.numerator).first()))
}

#[derive(Clone)]
enum ScannedStep {
    Continue,
    ReplayCompleted(tex_command::CommandReplayEpisode),
    Math(CanonicalMathRequest),
    MathDelimiter(MathDelimiterBoundary),
    MathFamily {
        family: tex_command::ScannedMathFamily,
        font: FontId,
        global: bool,
    },
    EndOfInput,
    Terminate,
    End,
    Count {
        index: u16,
        value: i32,
        global: bool,
    },
    Dimen {
        index: u16,
        value: Scaled,
        global: bool,
    },
    Skip {
        index: u16,
        value: GlueSpec,
        global: bool,
    },
    Muskip {
        index: u16,
        value: GlueSpec,
        global: bool,
    },
    HorizontalSkip {
        value: GlueSpec,
    },
    Kern {
        amount: Scaled,
    },
    CharacterCode {
        value: i32,
    },
    FixedHorizontalGlue {
        primitive: UnexpandablePrimitive,
    },
    ParagraphIndent {
        indent: bool,
    },
    ParagraphShape {
        lines: Vec<ParagraphShapeLine>,
        global: bool,
    },
    Toks {
        index: u16,
        tokens: TracedTokenList,
        global: bool,
    },
    IntParam {
        index: u16,
        value: i32,
        global: bool,
    },
    DimenParam {
        index: u16,
        value: Scaled,
        global: bool,
    },
    TokParam {
        index: u16,
        tokens: TracedTokenList,
        global: bool,
    },
    GlueParam {
        index: u16,
        value: GlueSpec,
        global: bool,
    },
    CodeTable {
        primitive: UnexpandablePrimitive,
        character: char,
        value: i32,
        global: bool,
    },
    FontSelect {
        font: FontId,
        selector: Option<Symbol>,
        global: bool,
    },
    FontDefinition {
        request: FontLoadRequest,
        resource: Box<Option<FontResource>>,
        global: bool,
    },
    InputStream {
        request: InputStreamRequest,
        resource: Option<SourceRegistration>,
    },
    PdfXImage {
        request: PdfImageRequest,
        resource: PdfImageResource,
    },
    PdfRefXImage {
        object: i32,
    },
    PdfGraphics(PdfGraphicsRequest),
    PdfObject(PdfObjectRequest),
    PdfReferenceObject(PdfReferenceObjectRequest),
    PdfForm(PdfFormRequest),
    PdfDocumentFragment(PdfDocumentFragmentRequest),
    PdfNavigation(PdfNavigationRequest),
    FontDimen {
        font: FontId,
        number: u32,
        value: Scaled,
    },
    FontInteger {
        font: FontId,
        skew: bool,
        value: i32,
    },
    DeferredOpenOut {
        stream: i32,
        file_name: String,
    },
    DeferredCloseOut {
        stream: i32,
    },
    DeferredWrite {
        stream: i32,
        tokens: TracedTokenList,
    },
    DeferredSpecial {
        tokens: TracedTokenList,
    },
    Arithmetic {
        primitive: UnexpandablePrimitive,
        target: ArithmeticTarget,
        operand: ArithmeticOperand,
        global: bool,
    },
    MacroDefinition {
        target: Symbol,
        flags: MeaningFlags,
        global: bool,
        parameter_text: TracedTokenList,
        replacement_text: TracedTokenList,
        definition_origin: tex_state::token::OriginId,
        missing_target: bool,
        malformed_parameter: bool,
    },
    Let {
        target: Symbol,
        source: Option<Symbol>,
        meaning: Meaning,
        global: bool,
    },
    AfterGroup(Token),
    AfterAssignment(Token),
    Rule {
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
        horizontal: bool,
    },
    Message {
        tokens: TracedTokenList,
        error: bool,
    },
    DisplayDiagnostic(ScannedDisplayDiagnostic),
    ShowBox {
        index: u16,
    },
    VSplit(ScannedVSplit),
    ImmediateExtension(ImmediateExtension),
    BoxRegister {
        index: u16,
        copy: bool,
        ships_out: bool,
    },
    Unbox {
        primitive: UnexpandablePrimitive,
        index: u16,
    },
    LastBox,
    Leaders {
        kind: GlueKind,
        payload: LeaderPayload,
        glue: GlueSpec,
    },
    LeaderRegister {
        kind: GlueKind,
        index: u16,
        copy: bool,
        glue: GlueSpec,
    },
    MissingLeaderPayload,
    LeadersNotFollowedByGlue,
    BeginShipout,
    BeginAlignment {
        vertical: bool,
    },
    AlignmentPreambleOpening {
        alignment: AlignmentIdentity,
    },
    AlignmentPreambleOpeningReplay {
        alignment: AlignmentIdentity,
    },
    AlignmentPreambleStart {
        alignment: AlignmentIdentity,
    },
    AlignmentCellOpening {
        alignment: AlignmentIdentity,
        opening: AlignmentCellOpening,
    },
    /// TeX82's `do_endv` completed a command-owned v-template.  Applying
    /// this result retires that exact frame before the backed-up delimiter
    /// resumes through `get_next`.
    AlignmentCellFinish {
        alignment: AlignmentIdentity,
    },
    /// TeX82 §37 delivered the alignment-closing right brace, so `fin_align`
    /// must complete before the outer suspended delivery context resumes.
    AlignmentFinish {
        alignment: AlignmentIdentity,
    },
    /// TeX82 §37 has consumed `\\noalign` and its compulsory opening brace.
    /// Command control owns both deliveries; the executor now owns the
    /// no-align group's structural entry.
    BeginNoAlign {
        alignment: AlignmentIdentity,
    },
    AlignmentRecovery {
        opens_simple_group: bool,
    },
    BeginSimpleGroup,
    EndSimpleGroup,
    BeginSemiSimpleGroup,
    EndSemiSimpleGroup,
    ExtraRightBrace,
    ExtraRightBraceEndsSemiSimpleGroup,
    ExtraEndGroup,
    BeginOrdinaryGroup,
    EndOrdinaryGroup,
    BeginOutputRoutine,
    OutputRoutineOpeningBrace,
    EndOutputRoutine,
    /// TeX82 §46 reaches the output-loop escape after the final-stop backup
    /// has been installed.  Main control now asks the typed page builder for
    /// the selected final page before canonical lowering; it never invents a
    /// synthetic box for this recovery path.
    ForcedOutputShipout,
    AlignmentPeekCell {
        alignment: AlignmentIdentity,
        omit: bool,
    },
    NoAlignBeginGroup {
        alignment: AlignmentIdentity,
    },
    NoAlignEndGroup {
        alignment: AlignmentIdentity,
    },
    SetBox(SetBoxTarget),
    BeginBox(ScannedBoxConstruction),
    BeginLeaderBox {
        construction: ScannedBoxConstruction,
        kind: GlueKind,
    },
    ReplayBoxOpeningBrace,
    BoxBeginGroup,
    BoxEndGroup {
        ships_out: bool,
    },
    Paragraph,
    MathShift {
        paired: bool,
    },
    ParagraphStart,
    Character {
        ch: char,
        cat: Catcode,
        origin: tex_state::token::OriginId,
    },
    Accent(ScannedAccent),
    Discretionary(ScannedDiscretionary),
}

impl ScannedStep {
    const fn fires_afterassignment(&self) -> bool {
        matches!(
            self,
            Self::Count { .. }
                | Self::Dimen { .. }
                | Self::Skip { .. }
                | Self::Muskip { .. }
                | Self::Toks { .. }
                | Self::IntParam { .. }
                | Self::DimenParam { .. }
                | Self::TokParam { .. }
                | Self::GlueParam { .. }
                | Self::CodeTable { .. }
                | Self::FontDimen { .. }
                | Self::FontInteger { .. }
                | Self::FontDefinition { .. }
                | Self::InputStream { .. }
                | Self::Arithmetic { .. }
                | Self::MacroDefinition { .. }
                | Self::Let { .. }
                | Self::ParagraphShape { .. }
        )
    }
}

/// A completed assignable quantity selector.  It is intentionally a semantic
/// selector, never a delivered command or a raw input handle.
#[derive(Clone, Copy, Debug)]
enum ArithmeticTarget {
    IntegerRegister(u16),
    DimensionRegister(u16),
    GlueRegister { index: u16, mu: bool },
    IntegerParameter(u16),
    DimensionParameter(u16),
    GlueParameter { index: u16, mu: bool },
}

#[derive(Clone, Copy, Debug)]
enum ArithmeticOperand {
    Integer(i32),
    Dimension(Scaled),
    Glue(GlueSpec),
}

/// Selects the one command-owned scanner that may consume input before
/// ordinary main control.  Alignment preamble setup validates and backs up
/// its opening brace twice through successive command-owned backup levels;
/// only the second replay reaches TeX82's live preamble scanner.
#[allow(clippy::too_many_arguments)] // owns the replay-only command/input seam
fn scan_replay_step(
    processor: &mut CommandProcessor<'_>,
    mode: Mode,
    starts_paragraph: bool,
    boxes: &ReplayBoxes,
    alignment_preamble: Option<(AlignmentIdentity, AlignmentPreamblePhase)>,
    innermost_group: Option<GroupKind>,
    output_dead_cycles: &mut i32,
    max_dead_cycles: i32,
    display_eq_no: bool,
) -> Result<ScannedStep, ExecError> {
    if let Some((alignment, phase)) = alignment_preamble {
        return match phase {
            AlignmentPreamblePhase::Opening => {
                processor
                    .scan_alignment_preamble_opening()
                    .map_err(command_error)?;
                Ok(ScannedStep::AlignmentPreambleOpening { alignment })
            }
            AlignmentPreamblePhase::ReplayOpening => {
                processor
                    .replay_alignment_preamble_opening()
                    .map_err(command_error)?;
                Ok(ScannedStep::AlignmentPreambleOpeningReplay { alignment })
            }
            AlignmentPreamblePhase::Start => {
                processor
                    .begin_alignment_preamble_scan()
                    .map_err(command_error)?;
                Ok(ScannedStep::AlignmentPreambleStart { alignment })
            }
            AlignmentPreamblePhase::CellOpening => {
                let opening = processor
                    .scan_alignment_cell_opening()
                    .map_err(command_error)?;
                Ok(ScannedStep::AlignmentCellOpening { alignment, opening })
            }
            AlignmentPreamblePhase::NextCellOpening => {
                let opening = processor
                    .scan_alignment_next_cell_opening()
                    .map_err(command_error)?;
                Ok(ScannedStep::AlignmentCellOpening { alignment, opening })
            }
            AlignmentPreamblePhase::AlignPeek { after_noalign } => {
                scan_alignment_peek(processor, alignment, after_noalign)
            }
            AlignmentPreamblePhase::NoAlignBody => {
                scan_noalign_body(processor, alignment, boxes, innermost_group, mode)
            }
            AlignmentPreamblePhase::CellDelivery => {
                scan_alignment_delivery_step(processor, alignment, boxes, innermost_group, mode)
            }
        };
    }
    scan_step(
        processor,
        mode,
        starts_paragraph,
        boxes,
        innermost_group,
        output_dead_cycles,
        max_dead_cycles,
        display_eq_no,
    )
}

#[derive(Clone, Copy)]
enum AlignmentPreamblePhase {
    Opening,
    ReplayOpening,
    Start,
    CellOpening,
    NextCellOpening,
    AlignPeek { after_noalign: bool },
    NoAlignBody,
    CellDelivery,
}

fn alignment_preamble(
    active: Option<&mut ActiveReplayAlignment>,
) -> Option<(AlignmentIdentity, AlignmentPreamblePhase)> {
    let active = active?;
    if active.preamble_opening_pending {
        Some((active.identity, AlignmentPreamblePhase::Opening))
    } else if active.preamble_opening_replay_pending {
        Some((active.identity, AlignmentPreamblePhase::ReplayOpening))
    } else if active.preamble_start_pending {
        Some((active.identity, AlignmentPreamblePhase::Start))
    } else if active.cell_opening_pending {
        Some((active.identity, AlignmentPreamblePhase::CellOpening))
    } else if active.next_cell_opening_pending {
        Some((active.identity, AlignmentPreamblePhase::NextCellOpening))
    } else if active.align_peek_pending {
        let after_noalign = active.align_peek_after_noalign;
        active.align_peek_after_noalign = false;
        Some((
            active.identity,
            AlignmentPreamblePhase::AlignPeek { after_noalign },
        ))
    } else if active.noalign_depth.is_some() {
        Some((active.identity, AlignmentPreamblePhase::NoAlignBody))
    } else {
        Some((active.identity, AlignmentPreamblePhase::CellDelivery))
    }
}

/// TeX82 §37's post-row lookahead.  This is deliberately separate from
/// `init_col`: `\\noalign` consumes its opening brace directly, whereas an
/// ordinary next-cell command is backed up for template installation.
fn scan_alignment_peek(
    processor: &mut CommandProcessor<'_>,
    alignment: AlignmentIdentity,
    after_noalign: bool,
) -> Result<ScannedStep, ExecError> {
    processor
        .begin_alignment_peek(after_noalign)
        .map_err(command_error)?;
    let command = next_non_space(processor)?.ok_or(ExecError::MissingToken {
        context: "alignment lookahead",
    })?;
    match command.meaning() {
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::NoAlign) => {
            processor
                .scan_alignment_noalign_opening()
                .map_err(command_error)?;
            Ok(ScannedStep::BeginNoAlign { alignment })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::CrCr) => Ok(ScannedStep::Continue),
        Meaning::CharToken {
            cat: Catcode::EndGroup,
            ..
        } => Ok(ScannedStep::AlignmentFinish { alignment }),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Omit) => {
            Ok(ScannedStep::AlignmentPeekCell {
                alignment,
                omit: true,
            })
        }
        _ => {
            processor.back_input(command).map_err(command_error)?;
            Ok(ScannedStep::AlignmentPeekCell {
                alignment,
                omit: false,
            })
        }
    }
}

fn scan_noalign_body(
    processor: &mut CommandProcessor<'_>,
    alignment: AlignmentIdentity,
    boxes: &ReplayBoxes,
    innermost_group: Option<GroupKind>,
    mode: Mode,
) -> Result<ScannedStep, ExecError> {
    let Some(command) = processor.get_x_token().map_err(command_error)? else {
        return Ok(ScannedStep::EndOfInput);
    };
    match command.meaning() {
        Meaning::CharToken {
            cat: Catcode::BeginGroup,
            ..
        } => Ok(ScannedStep::NoAlignBeginGroup { alignment }),
        Meaning::CharToken {
            cat: Catcode::EndGroup,
            ..
        } => Ok(ScannedStep::NoAlignEndGroup { alignment }),
        _ => scan_command(
            processor,
            command,
            false,
            MeaningFlags::EMPTY,
            mode,
            false,
            boxes,
            innermost_group,
            &mut 0,
            0,
            false,
        ),
    }
}

/// Delivers one active cell command through the command-owned alignment
/// boundary.  This remains separate from preamble and opener scans because a
/// completed scanner (such as a rule specification) can leave a backed-up
/// delimiter ready for the next main-control step.
fn scan_alignment_delivery_step(
    processor: &mut CommandProcessor<'_>,
    alignment: AlignmentIdentity,
    boxes: &ReplayBoxes,
    innermost_group: Option<GroupKind>,
    mode: Mode,
) -> Result<ScannedStep, ExecError> {
    match processor
        .get_x_alignment_delivery()
        .map_err(command_error)?
    {
        None => Ok(ScannedStep::EndOfInput),
        Some(AlignmentDelivery::Command(command)) => {
            if matches!(command.meaning(), Meaning::EndV) {
                // Replay's structural alignment group is deliberately not a
                // Universe group: the surrounding box owns that stack slot.
                // A recovery-opened simple group is the bounded exception
                // that TeX82 §1131 must close through `off_save` first.
                if boxes.recovery_simple_group_open {
                    let closer = match innermost_group {
                        Some(GroupKind::MathShift) => tex_state::token::Token::Char {
                            ch: '$',
                            cat: Catcode::MathShift,
                        },
                        Some(GroupKind::SemiSimple) => {
                            return Err(ExecError::MissingToken {
                                context: "endgroup off_save replay",
                            });
                        }
                        Some(_) => tex_state::token::Token::Char {
                            ch: '}',
                            cat: Catcode::EndGroup,
                        },
                        None => return Ok(ScannedStep::Continue),
                    };
                    processor
                        .recover_endv_off_save(command, closer)
                        .map_err(command_error)?;
                    return Ok(ScannedStep::Continue);
                }
                return Ok(ScannedStep::AlignmentCellFinish { alignment });
            }
            scan_command(
                processor,
                command,
                false,
                MeaningFlags::EMPTY,
                mode,
                false,
                boxes,
                innermost_group,
                &mut 0,
                0,
                false,
            )
        }
        Some(AlignmentDelivery::Event(event)) => {
            match event {
                tex_command::AlignmentDeliveryEvent::EndTemplate(_) => processor
                    .begin_alignment_v_template(alignment, event)
                    .map_err(command_error)?,
                tex_command::AlignmentDeliveryEvent::UnbalancedDelimiter(_) => {
                    let recovery = processor
                        .recover_alignment_unbalanced_delimiter(event)
                        .map_err(command_error)?;
                    return Ok(ScannedStep::AlignmentRecovery {
                        opens_simple_group: matches!(
                            recovery,
                            tex_state::token::Token::Char {
                                cat: Catcode::BeginGroup,
                                ..
                            }
                        ),
                    });
                }
                tex_command::AlignmentDeliveryEvent::ClosingBrace(_) => {
                    // TeX82 §1103 selects this executor-owned align_group
                    // branch. Raw brace backup/correction and frozen-\cr
                    // insertion remain entirely command-owned.
                    processor
                        .recover_alignment_closing_brace(event)
                        .map_err(command_error)?;
                }
            }
            Ok(ScannedStep::Continue)
        }
    }
}

#[allow(clippy::too_many_arguments)] // carries command-owned replay facts
fn scan_step(
    processor: &mut CommandProcessor<'_>,
    mode: Mode,
    starts_paragraph: bool,
    boxes: &ReplayBoxes,
    innermost_group: Option<GroupKind>,
    output_dead_cycles: &mut i32,
    max_dead_cycles: i32,
    display_eq_no: bool,
) -> Result<ScannedStep, ExecError> {
    let Some(delivery) = processor
        .get_x_token_with_replay_completion()
        .map_err(command_error)?
    else {
        return Ok(if boxes.terminal_output_shipout_complete {
            ScannedStep::Terminate
        } else {
            ScannedStep::EndOfInput
        });
    };
    let tex_command::CommandReplayDelivery::Command(mut command) = delivery else {
        let tex_command::CommandReplayDelivery::Completed(episode) = delivery else {
            unreachable!();
        };
        return Ok(ScannedStep::ReplayCompleted(episode));
    };
    let mut global = false;
    let mut flags = MeaningFlags::EMPTY;
    loop {
        match command.meaning() {
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Global) => global = true,
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Long) => {
                flags = flags | MeaningFlags::LONG
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Outer) => {
                flags = flags | MeaningFlags::OUTER
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Protected) => {
                flags = flags | MeaningFlags::PROTECTED
            }
            _ => break,
        }
        command = next_non_space(processor)?.ok_or(ExecError::MissingPrefixedCommand)?;
    }
    scan_command(
        processor,
        command,
        global,
        flags,
        mode,
        starts_paragraph,
        boxes,
        innermost_group,
        output_dead_cycles,
        max_dead_cycles,
        display_eq_no,
    )
}

fn leader_kind(primitive: UnexpandablePrimitive) -> GlueKind {
    match primitive {
        UnexpandablePrimitive::Leaders => GlueKind::Leaders,
        UnexpandablePrimitive::CLeaders => GlueKind::Cleaders,
        UnexpandablePrimitive::XLeaders => GlueKind::Xleaders,
        _ => unreachable!("leader scanner only receives leader primitives"),
    }
}

fn payload_from_node(node: Node) -> Option<LeaderPayload> {
    match node {
        Node::HList(node) => Some(LeaderPayload::HList(node)),
        Node::VList(node) => Some(LeaderPayload::VList(node)),
        Node::Rule {
            width,
            height,
            depth,
        } => Some(LeaderPayload::Rule {
            width,
            height,
            depth,
        }),
        _ => None,
    }
}

fn scan_leaders_step(
    processor: &mut CommandProcessor<'_>,
    primitive: UnexpandablePrimitive,
    mode: Mode,
) -> Result<ScannedStep, ExecError> {
    let kind = leader_kind(primitive);
    match processor.scan_leader_payload().map_err(command_error)? {
        ScannedLeaderPayload::Missing => Ok(ScannedStep::MissingLeaderPayload),
        ScannedLeaderPayload::Construction(construction) => {
            Ok(ScannedStep::BeginLeaderBox { construction, kind })
        }
        ScannedLeaderPayload::Rule(rule) => {
            let glue_command =
                processor
                    .get_x_token()
                    .map_err(command_error)?
                    .ok_or(ExecError::MissingToken {
                        context: "leader glue",
                    })?;
            let Some(glue) = scan_leader_glue_command(processor, glue_command, mode)? else {
                return Ok(ScannedStep::LeadersNotFollowedByGlue);
            };
            let payload = LeaderPayload::Rule {
                width: rule.width,
                height: rule.height,
                depth: rule.depth,
            };
            Ok(ScannedStep::Leaders {
                kind,
                payload,
                glue,
            })
        }
        // Register payloads must retain their destructive/copy ownership at
        // replay time.  Keep the command scanner's completed glue read, then
        // use the regular typed box read path to obtain the node.
        ScannedLeaderPayload::BoxRegister { index, copy } => {
            let index =
                u16::try_from(index).map_err(|_| ExecError::RegisterNumberOutOfRange(index))?;
            let glue_command =
                processor
                    .get_x_token()
                    .map_err(command_error)?
                    .ok_or(ExecError::MissingToken {
                        context: "leader glue",
                    })?;
            let Some(glue) = scan_leader_glue_command(processor, glue_command, mode)? else {
                return Ok(ScannedStep::LeadersNotFollowedByGlue);
            };
            Ok(ScannedStep::LeaderRegister {
                kind,
                index,
                copy,
                glue,
            })
        }
    }
}

fn scan_leader_glue_command(
    processor: &mut CommandProcessor<'_>,
    command: tex_command::CurrentCommand,
    mode: Mode,
) -> Result<Option<GlueSpec>, ExecError> {
    let horizontal = matches!(
        mode,
        Mode::Horizontal | Mode::RestrictedHorizontal | Mode::Math | Mode::DisplayMath
    );
    let primitive = match command.meaning() {
        Meaning::UnexpandablePrimitive(primitive) => primitive,
        _ => {
            processor.back_input(command).map_err(command_error)?;
            return Ok(None);
        }
    };
    if (horizontal && primitive == UnexpandablePrimitive::HSkip)
        || (!horizontal && primitive == UnexpandablePrimitive::VSkip)
    {
        return Ok(Some(
            processor.scan_glue(false).map_err(command_error)?.value,
        ));
    }
    let infinite = match (horizontal, primitive) {
        (true, UnexpandablePrimitive::HFil) | (false, UnexpandablePrimitive::VFil) => {
            Some((Order::Fil, false, false))
        }
        (true, UnexpandablePrimitive::HFill) | (false, UnexpandablePrimitive::VFill) => {
            Some((Order::Fill, false, false))
        }
        (true, UnexpandablePrimitive::HSs) | (false, UnexpandablePrimitive::VSs) => {
            Some((Order::Fil, false, true))
        }
        (true, UnexpandablePrimitive::HFilNeg) | (false, UnexpandablePrimitive::VFilNeg) => {
            Some((Order::Fil, true, false))
        }
        _ => None,
    };
    let Some((order, negative, shrink)) = infinite else {
        processor.back_input(command).map_err(command_error)?;
        return Ok(None);
    };
    let unit = Scaled::from_raw(if negative {
        -Scaled::UNITY
    } else {
        Scaled::UNITY
    });
    let zero = Scaled::from_raw(0);
    Ok(Some(if shrink {
        GlueSpec {
            width: zero,
            stretch: zero,
            stretch_order: Order::Normal,
            shrink: unit,
            shrink_order: order,
        }
    } else {
        GlueSpec {
            width: zero,
            stretch: unit,
            stretch_order: order,
            shrink: zero,
            shrink_order: Order::Normal,
        }
    }))
}

#[allow(clippy::too_many_arguments)] // mirrors TeX main-control dispatch inputs
fn scan_command(
    processor: &mut CommandProcessor<'_>,
    command: tex_command::CurrentCommand,
    global: bool,
    flags: MeaningFlags,
    mode: Mode,
    starts_paragraph: bool,
    boxes: &ReplayBoxes,
    innermost_group: Option<GroupKind>,
    output_dead_cycles: &mut i32,
    max_dead_cycles: i32,
    display_eq_no: bool,
) -> Result<ScannedStep, ExecError> {
    if let Meaning::UnexpandablePrimitive(
        primitive @ (UnexpandablePrimitive::TextFont
        | UnexpandablePrimitive::ScriptFont
        | UnexpandablePrimitive::ScriptScriptFont),
    ) = command.meaning()
    {
        let size = match primitive {
            UnexpandablePrimitive::TextFont => tex_command::MathFamilySize::Text,
            UnexpandablePrimitive::ScriptFont => tex_command::MathFamilySize::Script,
            UnexpandablePrimitive::ScriptScriptFont => tex_command::MathFamilySize::ScriptScript,
            _ => unreachable!(),
        };
        let family = processor.scan_math_family(size).map_err(command_error)?;
        let font = match processor.get_x_token().map_err(command_error)? {
            Some(font) => match font.meaning() {
                Meaning::Font(id) => id,
                _ => {
                    processor.back_input(font).map_err(command_error)?;
                    tex_state::font::NULL_FONT
                }
            },
            None => tex_state::font::NULL_FONT,
        };
        return Ok(ScannedStep::MathFamily {
            family,
            font,
            global,
        });
    }
    // Math operands are scanned exclusively by `tex-command`.  The replay
    // driver receives a typed scalar request and schedules any opaque field
    // episode only after this processor borrow has ended.
    if matches!(mode, Mode::Math | Mode::DisplayMath)
        && let Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Left
            | UnexpandablePrimitive::Right
            | UnexpandablePrimitive::Middle),
        ) = command.meaning()
    {
        let kind = match primitive {
            UnexpandablePrimitive::Left => MathDelimiterBoundaryKind::Left,
            UnexpandablePrimitive::Right => MathDelimiterBoundaryKind::Right,
            UnexpandablePrimitive::Middle => MathDelimiterBoundaryKind::Middle,
            _ => unreachable!(),
        };
        return Ok(ScannedStep::MathDelimiter(
            processor
                .scan_math_delimiter_boundary(kind)
                .map_err(command_error)?,
        ));
    }
    if matches!(mode, Mode::Math | Mode::DisplayMath)
        && let Meaning::UnexpandablePrimitive(primitive) = command.meaning()
        && let Some(request) = processor
            .scan_canonical_math_request(primitive)
            .map_err(command_error)?
    {
        return Ok(ScannedStep::Math(request));
    }
    if matches!(mode, Mode::Math | Mode::DisplayMath)
        && let Meaning::CharToken {
            cat: Catcode::Superscript,
            ..
        } = command.meaning()
    {
        return Ok(ScannedStep::Math(CanonicalMathRequest::Script(
            tex_command::ScannedMathScript {
                kind: MathScriptKind::Superscript,
                provenance: tex_command::StructuredProvenance {
                    primary: command.origin(),
                },
            },
        )));
    }
    if matches!(mode, Mode::Math | Mode::DisplayMath)
        && let Meaning::CharToken {
            cat: Catcode::Subscript,
            ..
        } = command.meaning()
    {
        return Ok(ScannedStep::Math(CanonicalMathRequest::Script(
            tex_command::ScannedMathScript {
                kind: MathScriptKind::Subscript,
                provenance: tex_command::StructuredProvenance {
                    primary: command.origin(),
                },
            },
        )));
    }
    // TeX82 permits \long/\outer only on macro definitions.  Keep this
    // check beside prefix collection so every ordinary assignment family has
    // the same legality rule and cannot silently discard a prefix.
    if flags != MeaningFlags::EMPTY
        && !matches!(
            command.meaning(),
            Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::Def
                    | UnexpandablePrimitive::Edef
                    | UnexpandablePrimitive::Gdef
                    | UnexpandablePrimitive::Xdef
            )
        )
    {
        return Err(ExecError::PrefixWithNonDefinition { origin: None });
    }
    // A constructed leader payload has just completed its box group.  The
    // following glue command is still raw input, so consume it here before
    // replay turns the frozen payload into a glue node.
    if let Some((kind, payload)) = boxes.pending_leader.as_ref() {
        let Some(glue) = scan_leader_glue_command(processor, command, mode)? else {
            return Ok(ScannedStep::LeadersNotFollowedByGlue);
        };
        return Ok(ScannedStep::Leaders {
            kind: *kind,
            payload: *payload,
            glue,
        });
    }
    if boxes.output_routine_opening_pending
        && matches!(
            command.meaning(),
            Meaning::CharToken {
                cat: Catcode::BeginGroup,
                ..
            }
        )
    {
        return Ok(ScannedStep::OutputRoutineOpeningBrace);
    }
    if boxes.output_routine_active
        && boxes.ordinary_simple_group_depth == 0
        && matches!(
            command.meaning(),
            Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            }
        )
    {
        return Ok(ScannedStep::EndOutputRoutine);
    }
    // `align_error`'s inserted brace is an actual execution group, even when
    // it appears inside a replayed box body.  It must therefore win over the
    // box body's brace-depth bookkeeping so §1131 can observe it at end-v.
    if boxes.recovery_simple_group_pending
        && matches!(
            command.meaning(),
            Meaning::CharToken {
                cat: Catcode::BeginGroup,
                ..
            }
        )
    {
        return Ok(ScannedStep::BeginSimpleGroup);
    }
    if boxes.recovery_simple_group_open
        && matches!(
            command.meaning(),
            Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            }
        )
    {
        return Ok(ScannedStep::EndSimpleGroup);
    }
    if boxes.ordinary_simple_group_depth > 0
        && matches!(
            command.meaning(),
            Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            }
        )
    {
        return Ok(ScannedStep::EndOrdinaryGroup);
    }
    if boxes
        .active_boxes
        .last()
        .is_some_and(|box_state| box_state.opening_brace_replay)
        && matches!(
            command.meaning(),
            Meaning::CharToken {
                cat: Catcode::BeginGroup,
                ..
            }
        )
    {
        processor
            .back_input_after_backup_replay(command)
            .map_err(command_error)?;
        return Ok(ScannedStep::ReplayBoxOpeningBrace);
    }
    if boxes
        .active_boxes
        .last()
        .is_some_and(|box_state| !box_state.opening_brace_replay)
    {
        match command.meaning() {
            Meaning::CharToken {
                cat: Catcode::BeginGroup,
                ..
            } => return Ok(ScannedStep::BoxBeginGroup),
            Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            } => {
                return Ok(ScannedStep::BoxEndGroup {
                    ships_out: boxes
                        .active_boxes
                        .last()
                        .is_some_and(|box_state| box_state.ships_out),
                });
            }
            _ => {}
        }
    }
    match command.meaning() {
        Meaning::CharToken {
            cat: Catcode::BeginGroup,
            ..
        } => Ok(ScannedStep::BeginOrdinaryGroup),
        Meaning::CharToken {
            cat: Catcode::EndGroup,
            ..
        } if innermost_group == Some(GroupKind::SemiSimple) => {
            Ok(ScannedStep::ExtraRightBraceEndsSemiSimpleGroup)
        }
        Meaning::CharToken {
            cat: Catcode::EndGroup,
            ..
        } => Ok(ScannedStep::ExtraRightBrace),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::BeginGroup) => {
            Ok(ScannedStep::BeginSemiSimpleGroup)
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::EndGroup)
            if innermost_group == Some(GroupKind::SemiSimple) =>
        {
            Ok(ScannedStep::EndSemiSimpleGroup)
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::EndGroup) => {
            if !processor.has_control_sequence_spelling(&command, "endgroup") {
                return Ok(ScannedStep::Continue);
            }
            if boxes.ordinary_simple_group_depth == 0 {
                processor.report_off_save_bottom_drop(&command);
                return Ok(ScannedStep::ExtraEndGroup);
            }
            processor
                .recover_off_save(
                    command,
                    Token::Char {
                        ch: '}',
                        cat: Catcode::EndGroup,
                    },
                )
                .map_err(command_error)?;
            Ok(ScannedStep::Continue)
        }
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::End | UnexpandablePrimitive::Dump,
        ) if mode == Mode::Horizontal => {
            // TeX82 §1095's `hmode+stop` case reaches `head_for_vmode`.
            // The command core owns both backups; replay merely processes
            // the resulting `\\par` and retries the stop in vertical mode.
            processor
                .recover_stop_for_vertical_mode(command)
                .map_err(command_error)?;
            Ok(ScannedStep::Continue)
        }
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::End | UnexpandablePrimitive::Dump,
        ) => {
            if boxes.terminal_output_shipout_complete {
                return Ok(ScannedStep::Continue);
            }
            if processor.output_routine_is_empty() {
                return Ok(ScannedStep::End);
            }
            // TeX82 §46's `its_all_over` does not discard the stop command
            // that has just been replayed from §1095's backup.  Command input
            // retires that exhausted level, backs `\end` up for the final
            // retry, and only then enters the output token list.
            if *output_dead_cycles >= max_dead_cycles {
                processor
                    .back_input_after_backup_replay(command)
                    .map_err(command_error)?;
                Ok(ScannedStep::ForcedOutputShipout)
            } else {
                processor
                    .begin_output_routine_after_stop(command)
                    .map_err(command_error)?;
                *output_dead_cycles += 1;
                Ok(ScannedStep::BeginOutputRoutine)
            }
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Count) => {
            let index = processor
                .scan_eight_bit_register_index()
                .map_err(command_error)?;
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_integer().map_err(command_error)?.value;
            Ok(ScannedStep::Count {
                index,
                value,
                global,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dimen) => {
            let index = processor
                .scan_eight_bit_register_index()
                .map_err(command_error)?;
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_dimension().map_err(command_error)?.value;
            Ok(ScannedStep::Dimen {
                index,
                value,
                global,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Skip) => {
            let index = processor
                .scan_eight_bit_register_index()
                .map_err(command_error)?;
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_glue(false).map_err(command_error)?.value;
            Ok(ScannedStep::Skip {
                index,
                value,
                global,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Muskip) => {
            let index = processor
                .scan_eight_bit_register_index()
                .map_err(command_error)?;
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_glue(true).map_err(command_error)?.value;
            Ok(ScannedStep::Muskip {
                index,
                value,
                global,
            })
        }
        // TeX82 §458 leaves `scan_glue` entirely in the command machine.
        // Main control receives only its completed typed specification, so a
        // u-template's numeric operand retains the canonical `back_input`
        // and replay sequence before this layer appends the glue node.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HSkip) => {
            let value = processor.scan_glue(false).map_err(command_error)?.value;
            Ok(ScannedStep::HorizontalSkip { value })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Kern) => {
            let amount = processor.scan_dimension().map_err(command_error)?.value;
            Ok(ScannedStep::Kern { amount })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char) => {
            let value = processor.scan_integer().map_err(command_error)?.value;
            Ok(ScannedStep::CharacterCode { value })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Accent) => Ok(ScannedStep::Accent(
            processor.scan_accent().map_err(command_error)?,
        )),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Discretionary) => Ok(
            ScannedStep::Discretionary(processor.scan_discretionary().map_err(command_error)?),
        ),
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::HFil
            | UnexpandablePrimitive::HFill
            | UnexpandablePrimitive::HSs
            | UnexpandablePrimitive::HFilNeg),
        ) => Ok(ScannedStep::FixedHorizontalGlue { primitive }),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Indent) => {
            Ok(ScannedStep::ParagraphIndent { indent: true })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::NoIndent) => {
            Ok(ScannedStep::ParagraphIndent { indent: false })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ParShape) => {
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let count = processor
                .scan_integer()
                .map_err(command_error)?
                .value
                .max(0) as usize;
            let mut lines = Vec::with_capacity(count);
            for _ in 0..count {
                lines.push(ParagraphShapeLine {
                    indent: processor.scan_dimension().map_err(command_error)?.value,
                    width: processor.scan_dimension().map_err(command_error)?.value,
                });
            }
            Ok(ScannedStep::ParagraphShape { lines, global })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Toks) => {
            let assignment = processor
                .scan_token_register_assignment()
                .map_err(command_error)?;
            Ok(ScannedStep::Toks {
                index: assignment.index,
                tokens: assignment.tokens,
                global,
            })
        }
        Meaning::IntParam(index) => {
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_integer().map_err(command_error)?.value;
            Ok(ScannedStep::IntParam {
                index,
                value,
                global,
            })
        }
        Meaning::DimenParam(index) => {
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_dimension().map_err(command_error)?.value;
            Ok(ScannedStep::DimenParam {
                index,
                value,
                global,
            })
        }
        Meaning::TokParam(index) => {
            let tokens = processor
                .scan_token_parameter_assignment()
                .map_err(command_error)?;
            Ok(ScannedStep::TokParam {
                index,
                tokens,
                global,
            })
        }
        Meaning::GlueParam(index) => {
            let assignment = processor
                .scan_glue_parameter_assignment(index, false)
                .map_err(command_error)?;
            Ok(ScannedStep::GlueParam {
                index: assignment.index,
                value: assignment.value,
                global,
            })
        }
        Meaning::MuGlueParam(index) => {
            let assignment = processor
                .scan_glue_parameter_assignment(index, true)
                .map_err(command_error)?;
            Ok(ScannedStep::GlueParam {
                index: assignment.index,
                value: assignment.value,
                global,
            })
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::OpenIn
            | UnexpandablePrimitive::CloseIn
            | UnexpandablePrimitive::Read
            | UnexpandablePrimitive::ReadLine),
        ) => Ok(ScannedStep::InputStream {
            request: processor
                .scan_input_stream_request(primitive)
                .map_err(command_error)?,
            resource: None,
        }),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Font) => {
            let request = processor.scan_font_definition().map_err(command_error)?;
            Ok(ScannedStep::FontDefinition {
                request,
                resource: Box::new(None),
                global,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfXImage) => {
            Ok(ScannedStep::PdfXImage {
                request: processor.scan_pdf_image_request().map_err(command_error)?,
                // This placeholder is replaced after the processor borrow;
                // it can never reach application.
                resource: PdfImageResource::Unavailable,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfRefXImage) => {
            Ok(ScannedStep::PdfRefXImage {
                object: processor.scan_integer().map_err(command_error)?.value,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfObject) => Ok(
            ScannedStep::PdfObject(processor.scan_pdf_object_request().map_err(command_error)?),
        ),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfReferenceObject) => {
            Ok(ScannedStep::PdfReferenceObject(
                processor
                    .scan_pdf_reference_object_request()
                    .map_err(command_error)?,
            ))
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::PdfXForm | UnexpandablePrimitive::PdfRefXForm),
        ) => Ok(ScannedStep::PdfForm(
            processor
                .scan_pdf_form_request(primitive)
                .map_err(command_error)?,
        )),
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::PdfInfo
            | UnexpandablePrimitive::PdfCatalog
            | UnexpandablePrimitive::PdfNames
            | UnexpandablePrimitive::PdfTrailer
            | UnexpandablePrimitive::PdfTrailerId),
        ) => Ok(ScannedStep::PdfDocumentFragment(
            processor
                .scan_pdf_document_fragment_request(primitive)
                .map_err(command_error)?,
        )),
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::PdfLiteral
            | UnexpandablePrimitive::PdfSetMatrix
            | UnexpandablePrimitive::PdfSave
            | UnexpandablePrimitive::PdfRestore
            | UnexpandablePrimitive::PdfColorStack
            | UnexpandablePrimitive::PdfSavePos),
        ) => Ok(ScannedStep::PdfGraphics(
            processor
                .scan_pdf_graphics_request(primitive)
                .map_err(command_error)?
                .expect("graphics primitive has a typed request"),
        )),
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::PdfAnnot
            | UnexpandablePrimitive::PdfStartLink
            | UnexpandablePrimitive::PdfEndLink
            | UnexpandablePrimitive::PdfDest
            | UnexpandablePrimitive::PdfThread
            | UnexpandablePrimitive::PdfStartThread
            | UnexpandablePrimitive::PdfEndThread),
        ) => Ok(ScannedStep::PdfNavigation(
            processor
                .scan_pdf_navigation_request(primitive)
                .map_err(command_error)?,
        )),
        Meaning::Font(font) => Ok(ScannedStep::FontSelect {
            font,
            selector: command.control_sequence(),
            global,
        }),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::FontDimen) => {
            let number = processor.scan_integer().map_err(command_error)?.value;
            let number =
                u32::try_from(number).map_err(|_| ExecError::RegisterNumberOutOfRange(number))?;
            if number == 0 {
                return Err(ExecError::RegisterNumberOutOfRange(0));
            }
            let font = processor.scan_font_selector().map_err(command_error)?;
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_dimension().map_err(command_error)?.value;
            Ok(ScannedStep::FontDimen {
                font,
                number,
                value,
            })
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::HyphenChar | UnexpandablePrimitive::SkewChar),
        ) => {
            let font = processor.scan_font_selector().map_err(command_error)?;
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_integer().map_err(command_error)?.value;
            Ok(ScannedStep::FontInteger {
                font,
                skew: primitive == UnexpandablePrimitive::SkewChar,
                value,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::OpenOut) => {
            let stream = processor.scan_integer().map_err(command_error)?.value;
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let file_name = processor.scan_file_name().map_err(command_error)?;
            Ok(ScannedStep::DeferredOpenOut {
                stream,
                file_name: file_name.name,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::CloseOut) => {
            Ok(ScannedStep::DeferredCloseOut {
                stream: processor.scan_integer().map_err(command_error)?.value,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Write) => {
            let stream = processor.scan_integer().map_err(command_error)?.value;
            let tokens = processor
                .scan_balanced_text(false)
                .map_err(command_error)?
                .tokens;
            Ok(ScannedStep::DeferredWrite { stream, tokens })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Special) => {
            Ok(ScannedStep::DeferredSpecial {
                tokens: processor.scan_special().map_err(command_error)?.tokens,
            })
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::CatCode
            | UnexpandablePrimitive::LcCode
            | UnexpandablePrimitive::UcCode
            | UnexpandablePrimitive::SfCode
            | UnexpandablePrimitive::MathCode
            | UnexpandablePrimitive::DelCode),
        ) => {
            let character = processor.scan_integer().map_err(command_error)?.value;
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_integer().map_err(command_error)?.value;
            let character = u32::try_from(character)
                .ok()
                .and_then(char::from_u32)
                .ok_or(ExecError::InvalidCode {
                    context: "code-table character",
                    value: character,
                })?;
            Ok(ScannedStep::CodeTable {
                primitive,
                character,
                value,
                global,
            })
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Advance
            | UnexpandablePrimitive::Multiply
            | UnexpandablePrimitive::Divide),
        ) => scan_arithmetic_assignment(processor, primitive, global),
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Def
            | UnexpandablePrimitive::Edef
            | UnexpandablePrimitive::Gdef
            | UnexpandablePrimitive::Xdef),
        ) => {
            let expanded = matches!(
                primitive,
                UnexpandablePrimitive::Edef | UnexpandablePrimitive::Xdef
            );
            let definition = processor
                .scan_macro_definition(expanded)
                .map_err(command_error)?;
            Ok(ScannedStep::MacroDefinition {
                target: definition.target,
                flags,
                global: global
                    || matches!(
                        primitive,
                        UnexpandablePrimitive::Gdef | UnexpandablePrimitive::Xdef
                    ),
                parameter_text: definition.parameter_text,
                replacement_text: definition.replacement_text,
                definition_origin: definition.provenance.primary,
                missing_target: definition.missing_target,
                malformed_parameter: definition.malformed_parameter,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Let) => {
            let assignment = processor
                .scan_let_assignment(false)
                .map_err(command_error)?;
            Ok(ScannedStep::Let {
                target: assignment.target,
                source: assignment.source,
                meaning: assignment.meaning,
                global,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::FutureLet) => {
            let assignment = processor.scan_let_assignment(true).map_err(command_error)?;
            Ok(ScannedStep::Let {
                target: assignment.target,
                source: assignment.source,
                meaning: assignment.meaning,
                global,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::AfterGroup) => {
            Ok(ScannedStep::AfterGroup(
                processor
                    .get_token()
                    .map_err(command_error)?
                    .ok_or(ExecError::MissingToken {
                        context: "\\aftergroup",
                    })?
                    .spelling()
                    .semantic_token(),
            ))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::AfterAssignment) => {
            Ok(ScannedStep::AfterAssignment(
                processor
                    .get_token()
                    .map_err(command_error)?
                    .ok_or(ExecError::MissingToken {
                        context: "\\afterassignment",
                    })?
                    .spelling()
                    .semantic_token(),
            ))
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Message | UnexpandablePrimitive::ErrMessage),
        ) => {
            let tokens = processor.scan_balanced_text(true).map_err(command_error)?;
            Ok(ScannedStep::Message {
                tokens: tokens.tokens,
                error: primitive == UnexpandablePrimitive::ErrMessage,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Show) => Ok(
            ScannedStep::DisplayDiagnostic(processor.scan_show().map_err(command_error)?),
        ),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ShowThe) => Ok(
            ScannedStep::DisplayDiagnostic(processor.scan_showthe().map_err(command_error)?),
        ),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ShowBox) => {
            let (index, _) = processor.scan_showbox().map_err(command_error)?;
            Ok(ScannedStep::ShowBox {
                index: u16::try_from(index)
                    .map_err(|_| ExecError::RegisterNumberOutOfRange(index))?,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Immediate) => {
            Ok(ScannedStep::ImmediateExtension(
                processor
                    .scan_immediate_extension()
                    .map_err(command_error)?,
            ))
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::HRule | UnexpandablePrimitive::VRule),
        ) => {
            let spec = processor.scan_rule_spec(primitive).map_err(command_error)?;
            Ok(ScannedStep::Rule {
                width: spec.width,
                height: spec.height,
                depth: spec.depth,
                horizontal: primitive == UnexpandablePrimitive::HRule,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SetBox) => {
            let assignment = processor.scan_setbox_assignment().map_err(command_error)?;
            let index = u16::try_from(assignment.index)
                .map_err(|_| ExecError::RegisterNumberOutOfRange(assignment.index))?;
            Ok(ScannedStep::SetBox(SetBoxTarget { index, global }))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VSplit) => Ok(ScannedStep::VSplit(
            processor.scan_vsplit().map_err(command_error)?,
        )),
        // TeX82 §1071's `make_box(box_code)` scans the register through
        // `scan_int` before handing the completed box-list operation to the
        // stomach. In particular, the first digit remains raw command input,
        // never an executor-side backup/replay artifact.
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Box | UnexpandablePrimitive::Copy),
        ) => {
            let register = processor.scan_box_register().map_err(command_error)?;
            let index = u16::try_from(register.index)
                .map_err(|_| ExecError::RegisterNumberOutOfRange(register.index))?;
            Ok(ScannedStep::BoxRegister {
                index,
                copy: primitive == UnexpandablePrimitive::Copy,
                ships_out: boxes.pending_shipout,
            })
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::UnHBox
            | UnexpandablePrimitive::UnHCopy
            | UnexpandablePrimitive::UnVBox
            | UnexpandablePrimitive::UnVCopy),
        ) => {
            let register = processor.scan_box_register().map_err(command_error)?;
            let index = u16::try_from(register.index)
                .map_err(|_| ExecError::RegisterNumberOutOfRange(register.index))?;
            Ok(ScannedStep::Unbox { primitive, index })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::LastBox) => Ok(ScannedStep::LastBox),
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Leaders
            | UnexpandablePrimitive::CLeaders
            | UnexpandablePrimitive::XLeaders),
        ) => scan_leaders_step(processor, primitive, mode),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Shipout) => {
            Ok(ScannedStep::BeginShipout)
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::HBox
            | UnexpandablePrimitive::VBox
            | UnexpandablePrimitive::VTop),
        ) => Ok(ScannedStep::BeginBox(
            processor
                .scan_box_construction(primitive)
                .map_err(command_error)?,
        )),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HAlign) => {
            Ok(ScannedStep::BeginAlignment { vertical: false })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VAlign) => {
            Ok(ScannedStep::BeginAlignment { vertical: true })
        }
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::Par | UnexpandablePrimitive::EndGraf,
        ) => Ok(ScannedStep::Paragraph),
        Meaning::CharToken {
            cat: Catcode::MathShift,
            ..
        } => {
            if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                // §1090 retries this exact shift after `new_graf`; probing it
                // here would run before `\everypar`.
                processor.back_input(command).map_err(command_error)?;
                Ok(ScannedStep::ParagraphStart)
            } else {
                let paired = matches!(
                    mode,
                    Mode::Horizontal | Mode::RestrictedHorizontal | Mode::DisplayMath
                ) || (mode == Mode::Math && display_eq_no);
                let paired = if paired {
                    processor.scan_paired_math_shift().map_err(command_error)?
                } else {
                    false
                };
                Ok(ScannedStep::MathShift { paired })
            }
        }
        Meaning::CharToken {
            cat: Catcode::Letter | Catcode::Other,
            ..
        } if starts_paragraph => {
            // TeX82 main_control backs up the first ordinary character before
            // new_graf. The command processor retains exact-delivery proof
            // for that operation; executor replay only applies its typed mode
            // transition after this borrow ends.
            processor.back_input(command).map_err(command_error)?;
            Ok(ScannedStep::ParagraphStart)
        }
        Meaning::CharToken {
            ch,
            cat: cat @ (Catcode::Letter | Catcode::Other | Catcode::Space),
        } => Ok(ScannedStep::Character {
            ch,
            cat,
            origin: command.spelling().origin(),
        }),
        _ => Ok(ScannedStep::Continue),
    }
}

/// Scans TeX82's `advance`/`multiply`/`divide` operand sequence wholly
/// through the command processor.  The target's meaning is classified here;
/// application only sees this completed typed description after the processor
/// borrow ends.
fn scan_arithmetic_assignment(
    processor: &mut CommandProcessor<'_>,
    primitive: UnexpandablePrimitive,
    global: bool,
) -> Result<ScannedStep, ExecError> {
    let target_command = processor
        .get_x_token()
        .map_err(command_error)?
        .ok_or(ExecError::UnsupportedAssignmentTarget)?;
    let target = match target_command.meaning() {
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Count) => {
            ArithmeticTarget::IntegerRegister(
                processor
                    .scan_eight_bit_register_index()
                    .map_err(command_error)?,
            )
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dimen) => {
            ArithmeticTarget::DimensionRegister(
                processor
                    .scan_eight_bit_register_index()
                    .map_err(command_error)?,
            )
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Skip) => {
            ArithmeticTarget::GlueRegister {
                index: processor
                    .scan_eight_bit_register_index()
                    .map_err(command_error)?,
                mu: false,
            }
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Muskip) => {
            ArithmeticTarget::GlueRegister {
                index: processor
                    .scan_eight_bit_register_index()
                    .map_err(command_error)?,
                mu: true,
            }
        }
        Meaning::CountRegister(index) => ArithmeticTarget::IntegerRegister(index),
        Meaning::DimenRegister(index) => ArithmeticTarget::DimensionRegister(index),
        Meaning::SkipRegister(index) => ArithmeticTarget::GlueRegister { index, mu: false },
        Meaning::MuskipRegister(index) => ArithmeticTarget::GlueRegister { index, mu: true },
        Meaning::IntParam(index) => ArithmeticTarget::IntegerParameter(index),
        Meaning::DimenParam(index) => ArithmeticTarget::DimensionParameter(index),
        Meaning::GlueParam(index) => ArithmeticTarget::GlueParameter { index, mu: false },
        Meaning::MuGlueParam(index) => ArithmeticTarget::GlueParameter { index, mu: true },
        _ => return Err(ExecError::UnsupportedAssignmentTarget),
    };
    let _ = processor.scan_keyword("by").map_err(command_error)?;
    let operand = match target {
        ArithmeticTarget::IntegerRegister(_) | ArithmeticTarget::IntegerParameter(_) => {
            ArithmeticOperand::Integer(processor.scan_integer().map_err(command_error)?.value)
        }
        ArithmeticTarget::DimensionRegister(_) | ArithmeticTarget::DimensionParameter(_) => {
            match primitive {
                UnexpandablePrimitive::Advance => ArithmeticOperand::Dimension(
                    processor.scan_dimension().map_err(command_error)?.value,
                ),
                UnexpandablePrimitive::Multiply | UnexpandablePrimitive::Divide => {
                    ArithmeticOperand::Integer(
                        processor.scan_integer().map_err(command_error)?.value,
                    )
                }
                _ => unreachable!("arithmetic primitive is filtered above"),
            }
        }
        ArithmeticTarget::GlueRegister { mu, .. } | ArithmeticTarget::GlueParameter { mu, .. } => {
            match primitive {
                UnexpandablePrimitive::Advance => {
                    ArithmeticOperand::Glue(processor.scan_glue(mu).map_err(command_error)?.value)
                }
                UnexpandablePrimitive::Multiply | UnexpandablePrimitive::Divide => {
                    ArithmeticOperand::Integer(
                        processor.scan_integer().map_err(command_error)?.value,
                    )
                }
                _ => unreachable!("arithmetic primitive is filtered above"),
            }
        }
    };
    Ok(ScannedStep::Arithmetic {
        primitive,
        target,
        operand,
        global,
    })
}

fn replay_text(tokens: &[tex_state::token::Token]) -> String {
    tokens
        .iter()
        .filter_map(|token| match token {
            tex_state::token::Token::Char { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect()
}

/// TeX's eight-bit extension payload convention, with UTF-8 retained for
/// extended host-profile characters exactly as the legacy byte boundary does.
fn tex_byte_text(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len());
    for ch in text.chars() {
        if let Ok(byte) = u8::try_from(ch as u32) {
            bytes.push(byte);
        } else {
            let mut encoded = [0; 4];
            bytes.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
        }
    }
    bytes
}

fn pdf_graphics_text(tokens: TracedTokenList, stores: &Universe) -> Vec<u8> {
    let mut text = String::new();
    for &token in stores.tokens(tokens.token_list()) {
        tex_expand::append_token_string_text(stores, token, &mut text);
    }
    tex_byte_text(&text)
}

fn pdf_navigation_identity(
    stores: &Universe,
    identifier: tex_state::PdfActionIdentifier,
) -> tex_state::PdfDestinationIdentity {
    match identifier {
        tex_state::PdfActionIdentifier::Number(number) => {
            tex_state::PdfDestinationIdentity::Number(number)
        }
        tex_state::PdfActionIdentifier::Name(tokens) => tex_state::PdfDestinationIdentity::Name(
            pdf_graphics_text(TracedTokenList::synthetic(tokens), stores),
        ),
        tex_state::PdfActionIdentifier::Raw(tokens) => tex_state::PdfDestinationIdentity::Name(
            pdf_graphics_text(TracedTokenList::synthetic(tokens), stores),
        ),
    }
}

fn apply_pdf_navigation_request(
    request: PdfNavigationRequest,
    stores: &mut Universe,
    modes: &mut ModeNest,
) -> Result<ReplayStep, ExecError> {
    match request {
        PdfNavigationRequest::Annotation(request) => {
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfannot"));
            }
            match request {
                PdfAnnotationRequest::Reserve => {
                    stores
                        .reserve_pdf_annotation()
                        .map_err(|_| ExecError::PdfObjectCapacity)?;
                }
                PdfAnnotationRequest::Define {
                    use_object,
                    dimensions,
                    entries,
                } => {
                    let data = tex_state::PdfAnnotationData {
                        dimensions,
                        entries: entries.tokens.token_list(),
                    };
                    let record = match use_object {
                        Some(object) => stores
                            .initialize_pdf_annotation(
                                u32::try_from(object)
                                    .map_err(|_| ExecError::PdfReferencedObjectNotFound)?,
                                data,
                            )
                            .map_err(|_| ExecError::PdfReferencedObjectNotFound)?,
                        None => stores
                            .create_pdf_annotation(data)
                            .map_err(|_| ExecError::PdfObjectCapacity)?,
                    };
                    modes
                        .current_list_mut()
                        .push(Node::Whatsit(Whatsit::PdfAnnotation {
                            object: record.object(),
                        }));
                }
            }
        }
        PdfNavigationRequest::StartLink(PdfStartLinkRequest {
            dimensions,
            attributes,
            action,
        }) => {
            if matches!(
                modes.current_mode(),
                Mode::Vertical | Mode::InternalVertical
            ) {
                return Err(ExecError::PdfLinkInVerticalMode("pdfstartlink"));
            }
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfstartlink"));
            }
            let record = stores
                .create_pdf_link(
                    dimensions,
                    attributes.map_or(TokenListId::EMPTY, |value| value.tokens.token_list()),
                    action,
                    stores.execution_group_depth(),
                )
                .map_err(|_| ExecError::PdfObjectCapacity)?;
            reserve_navigation_action_targets(stores, action)?;
            modes
                .current_list_mut()
                .push(Node::Whatsit(Whatsit::PdfLinkStart {
                    object: record.object(),
                }));
        }
        PdfNavigationRequest::EndLink => {
            if matches!(
                modes.current_mode(),
                Mode::Vertical | Mode::InternalVertical
            ) {
                return Err(ExecError::PdfLinkInVerticalMode("pdfendlink"));
            }
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfendlink"));
            }
            let open = stores
                .end_pdf_link()
                .ok_or(ExecError::PdfEndLinkWithoutStart)?;
            if open.nesting_depth != stores.execution_group_depth() {
                stores.world_mut().write_text(PrintSink::TerminalAndLog, "\npdfTeX warning: \\pdfendlink ended up in different nesting level than \\pdfstartlink\n");
            }
            modes
                .current_list_mut()
                .push(Node::Whatsit(Whatsit::PdfLinkEnd {
                    object: open.record.object(),
                }));
        }
        PdfNavigationRequest::Destination(PdfDestinationRequest {
            structure,
            identifier,
            kind,
        }) => {
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfdest"));
            }
            let identity = pdf_navigation_identity(stores, identifier);
            if stores
                .pdf_destination(&identity, structure.is_some())
                .is_some_and(tex_state::PdfDestinationRecord::defined)
            {
                crate::assignments::warn_pdf_destination_duplicate(stores, &identity);
                return Ok(ReplayStep::Continue);
            }
            modes
                .current_list_mut()
                .push(Node::Whatsit(Whatsit::PdfDestination(Box::new(
                    tex_state::node::PdfDestinationNode {
                        identifier,
                        structure,
                        kind,
                    },
                ))));
        }
        PdfNavigationRequest::Thread(tex_command::PdfThreadRequest {
            dimensions,
            attributes,
            identifier,
            running,
        }) => {
            let primitive = if running {
                "pdfstartthread"
            } else {
                "pdfthread"
            };
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode(primitive));
            }
            modes
                .current_list_mut()
                .push(Node::Whatsit(Whatsit::PdfThread(Box::new(
                    tex_state::node::PdfThreadNode {
                        identifier,
                        dimensions,
                        attributes: attributes
                            .map_or(TokenListId::EMPTY, |value| value.tokens.token_list()),
                        running,
                    },
                ))));
        }
        PdfNavigationRequest::EndThread => {
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfendthread"));
            }
            modes
                .current_list_mut()
                .push(Node::Whatsit(Whatsit::PdfEndThread));
        }
    }
    Ok(ReplayStep::Continue)
}

fn reserve_navigation_action_targets(
    stores: &mut Universe,
    action: tex_state::PdfActionSpec,
) -> Result<(), ExecError> {
    let (destination, structure, thread) = pdf_action_target_identities(stores, action);
    if let Some(identity) = thread {
        stores
            .reserve_pdf_thread(identity)
            .map_err(|_| ExecError::PdfObjectCapacity)?;
    }
    if let Some(identity) = destination {
        stores
            .reserve_pdf_destination(identity, false)
            .map_err(|_| ExecError::PdfObjectCapacity)?;
    }
    if let Some(identity) = structure {
        stores
            .reserve_pdf_destination(identity, true)
            .map_err(|_| ExecError::PdfObjectCapacity)?;
    }
    Ok(())
}

fn pdf_action_target_identities(
    stores: &Universe,
    action: tex_state::PdfActionSpec,
) -> (
    Option<tex_state::PdfDestinationIdentity>,
    Option<tex_state::PdfDestinationIdentity>,
    Option<tex_state::PdfDestinationIdentity>,
) {
    let destination = match action {
        tex_state::PdfActionSpec::GoTo(destination) if destination.file.is_none() => destination,
        tex_state::PdfActionSpec::Thread(thread) if thread.file.is_none() => {
            let identity = match thread.target {
                tex_state::PdfActionTarget::Destination(identifier) => {
                    Some(pdf_navigation_identity(stores, identifier))
                }
                tex_state::PdfActionTarget::Page { .. } => None,
            };
            return (None, None, identity);
        }
        _ => return (None, None, None),
    };
    let target = match destination.target {
        tex_state::PdfActionTarget::Destination(identifier) => {
            Some(pdf_navigation_identity(stores, identifier))
        }
        tex_state::PdfActionTarget::Page { .. } => None,
    };
    let structure = destination
        .structure
        .map(|identifier| pdf_navigation_identity(stores, identifier));
    (target, structure, None)
}

fn apply_pdf_graphics_request(
    request: PdfGraphicsRequest,
    stores: &mut Universe,
    modes: &mut ModeNest,
) -> Result<ReplayStep, ExecError> {
    use PdfColorStackActionRequest as Action;

    if !matches!(request, PdfGraphicsRequest::SavePosition)
        && stores.int_param(IntParam::PDF_OUTPUT) <= 0
    {
        let primitive = match request {
            PdfGraphicsRequest::Literal { .. } => "pdfliteral",
            PdfGraphicsRequest::SetMatrix { .. } => "pdfsetmatrix",
            PdfGraphicsRequest::Save => "pdfsave",
            PdfGraphicsRequest::Restore => "pdfrestore",
            PdfGraphicsRequest::ColorStack { .. } => "pdfcolorstack",
            PdfGraphicsRequest::SavePosition => unreachable!(),
        };
        return Err(ExecError::PdfExtensionInDviMode(primitive));
    }

    let node = match request {
        PdfGraphicsRequest::Literal {
            mode,
            deferred: true,
            text,
        } => Node::Whatsit(Whatsit::DeferredPdfLiteral {
            mode,
            tokens: text.tokens.token_list(),
        }),
        PdfGraphicsRequest::Literal { mode, text, .. } => Node::Whatsit(Whatsit::PdfLiteral {
            mode,
            payload: pdf_graphics_text(text.tokens, stores),
        }),
        PdfGraphicsRequest::SetMatrix { text } => Node::Whatsit(Whatsit::PdfSetMatrix {
            payload: pdf_graphics_text(text.tokens, stores),
        }),
        PdfGraphicsRequest::Save => Node::Whatsit(Whatsit::PdfSave),
        PdfGraphicsRequest::Restore => Node::Whatsit(Whatsit::PdfRestore),
        PdfGraphicsRequest::SavePosition => Node::Whatsit(Whatsit::PdfSavePos),
        PdfGraphicsRequest::ColorStack { id, action } => {
            let id = if id < 0 {
                stores.world_mut().write_text(
                    PrintSink::TerminalAndLog,
                    "Invalid negative color stack number\n",
                );
                0
            } else if !stores.has_pdf_color_stack(id as u32) {
                stores.world_mut().write_text(
                    PrintSink::TerminalAndLog,
                    &format!("Unknown color stack number {id}\n"),
                );
                0
            } else {
                id as u32
            };
            let Some(action) = action else {
                stores
                    .world_mut()
                    .write_text(PrintSink::TerminalAndLog, "Color stack action is missing\n");
                return Ok(ReplayStep::Continue);
            };
            let action = match action {
                Action::Set(text) => {
                    tex_state::PdfColorStackAction::Set(pdf_graphics_text(text.tokens, stores))
                }
                Action::Push(text) => {
                    tex_state::PdfColorStackAction::Push(pdf_graphics_text(text.tokens, stores))
                }
                Action::Pop => tex_state::PdfColorStackAction::Pop,
                Action::Current => tex_state::PdfColorStackAction::Current,
            };
            Node::Whatsit(Whatsit::PdfColorStack { id, action })
        }
    };
    modes.current_list_mut().push(node);
    Ok(ReplayStep::Continue)
}

fn apply_pdf_object_request(
    request: PdfObjectRequest,
    stores: &mut Universe,
    immediate: bool,
) -> Result<ReplayStep, ExecError> {
    if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
        return Err(ExecError::PdfExtensionInDviMode("pdfobj"));
    }
    match request {
        PdfObjectRequest::Reserve => {
            stores
                .reserve_pdf_raw_object()
                .map_err(|_| ExecError::PdfObjectCapacity)?;
        }
        PdfObjectRequest::Define {
            use_object,
            stream,
            stream_attr,
            file,
            data,
        } => {
            let requested = use_object.and_then(|raw| {
                u32::try_from(raw).ok().and_then(|raw| {
                    stores
                        .pdf_raw_object(raw)
                        .filter(|record| record.data().is_none())
                        .map(|r| r.id())
                })
            });
            let id = match requested {
                Some(id) => id,
                None => {
                    if use_object.is_some() {
                        stores.set_pdf_return_value(-1);
                        stores.world_mut().write_text(
                            PrintSink::TerminalAndLog,
                            "\npdfTeX warning (\\pdfobj): invalid object number being ignored\n",
                        );
                    }
                    stores
                        .reserve_pdf_raw_object()
                        .map_err(|_| ExecError::PdfObjectCapacity)?
                }
            };
            stores
                .initialize_pdf_raw_object(
                    id,
                    stream,
                    stream_attr.map(|text| text.tokens.token_list()),
                    file,
                    data.tokens.token_list(),
                    immediate,
                )
                .map_err(|_| ExecError::PdfReferencedObjectNotFound)?;
        }
    }
    Ok(ReplayStep::Continue)
}

fn apply_pdf_form_request(
    request: PdfFormRequest,
    stores: &mut Universe,
    modes: &mut ModeNest,
    immediate: bool,
) -> Result<ReplayStep, ExecError> {
    if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
        let name = match request {
            PdfFormRequest::Create { .. } => "pdfxform",
            PdfFormRequest::Reference { .. } => "pdfrefxform",
        };
        return Err(ExecError::PdfExtensionInDviMode(name));
    }
    match request {
        PdfFormRequest::Reference { object } => {
            let form = u32::try_from(object)
                .ok()
                .and_then(|object| stores.pdf_form(object))
                .ok_or(ExecError::PdfReferencedObjectNotFound)?;
            modes
                .current_list_mut()
                .push(Node::Whatsit(Whatsit::PdfRefXForm {
                    object: form.object(),
                    width: form.width(),
                    height: form.height(),
                    depth: form.depth(),
                }));
        }
        PdfFormRequest::Create {
            attr,
            resources,
            box_register,
        } => {
            let index = u16::try_from(box_register)
                .map_err(|_| ExecError::RegisterNumberOutOfRange(box_register))?;
            // pdfTeX allocates the form identity before it consumes the box.
            let identity = stores
                .reserve_pdf_form()
                .map_err(|_| ExecError::PdfObjectCapacity)?;
            let list = stores
                .take_box_reg_same_level(index)
                .ok_or(ExecError::PdfXFormVoidBox)?;
            let dimensions = match stores.nodes(list).first().map(|node| node.to_owned()) {
                Some(Node::HList(node) | Node::VList(node)) => {
                    (node.width, node.height, node.depth)
                }
                _ => return Err(ExecError::PdfXFormVoidBox),
            };
            stores
                .initialize_pdf_form(
                    identity,
                    list,
                    dimensions,
                    attr.map(|text| text.tokens.token_list()),
                    resources.map(|text| text.tokens.token_list()),
                    immediate,
                )
                .map_err(|_| ExecError::PdfObjectCapacity)?;
        }
    }
    Ok(ReplayStep::Continue)
}

fn replay_stream_slot(value: i32, stores: &mut Universe) -> StreamSlot {
    if (0..tex_state::world::STREAM_SLOT_COUNT as i32).contains(&value) {
        return StreamSlot::new(value as u8);
    }
    stores.world_mut().write_text(
        PrintSink::TerminalAndLog,
        &format!(
            "\n! Bad number ({value}).\nSince I expected to read a number between 0 and 15,\nI changed this one to zero.\n"
        ),
    );
    StreamSlot::new(0)
}

/// Converts one World-provided physical line to the immutable replacement
/// text of `\\read`/`\\readline`.  World owns the stream cursor; this helper
/// has no input level or file handle and therefore cannot become an alternate
/// source-consumption path.
fn canonical_read_line_tokens(line: &str, raw_catcodes: bool, stores: &Universe) -> Vec<Token> {
    let mut tokens = line
        .chars()
        .map(|ch| Token::Char {
            ch,
            cat: if raw_catcodes {
                if ch == ' ' {
                    Catcode::Space
                } else {
                    Catcode::Other
                }
            } else {
                stores.catcode(ch)
            },
        })
        .collect::<Vec<_>>();
    if let Ok(code) = u32::try_from(stores.int_param(IntParam::END_LINE_CHAR))
        && let Some(ch) = char::from_u32(code)
    {
        tokens.push(Token::Char {
            ch,
            cat: if raw_catcodes {
                if ch == ' ' {
                    Catcode::Space
                } else {
                    Catcode::Other
                }
            } else {
                stores.catcode(ch)
            },
        });
    }
    tokens
}

/// Collects TeX82's balanced `read_toks` text using only World-owned line
/// acquisition. The result is an immutable replacement list; neither a live
/// cursor nor an input level crosses the canonical replay boundary.
fn canonical_read_tokens(
    slot: StreamSlot,
    target: Symbol,
    raw_catcodes: bool,
    stores: &mut Universe,
) -> Result<Vec<Token>, ExecError> {
    let mut tokens = Vec::new();
    let mut depth = 0_u32;
    let mut first_terminal_line = true;
    loop {
        let stream_open = stores
            .world()
            .stream_bufs()
            .read_stream_target(slot)
            .is_some();
        let line = if stream_open {
            stores.world_mut().read_stream_line(slot)?
        } else {
            match stores.interaction_mode() {
                tex_state::InteractionMode::Batch | tex_state::InteractionMode::Nonstop => {
                    return Err(ExecError::ReadNotImplemented);
                }
                tex_state::InteractionMode::Scroll | tex_state::InteractionMode::ErrorStop => {}
            }
            if first_terminal_line {
                let prompt = tex_expand::token_text(stores, Token::Cs(target));
                stores
                    .world_mut()
                    .write_text(PrintSink::TerminalAndLog, &format!("\n{prompt}="));
                first_terminal_line = false;
            }
            Some(
                stores
                    .world_mut()
                    .read_terminal_line()?
                    .ok_or(ExecError::TerminalReadEof)?,
            )
        };
        let Some(line) = line else {
            return Err(ExecError::ReadNotImplemented);
        };
        let line_tokens = canonical_read_line_tokens(&line, raw_catcodes, stores);
        if raw_catcodes {
            return Ok(line_tokens);
        }
        for token in line_tokens {
            let meaning = match token {
                Token::Cs(symbol) => stores.meaning(symbol),
                _ => Meaning::Undefined,
            };
            if matches!(meaning, Meaning::Macro { flags, .. } if flags.contains(MeaningFlags::OUTER))
            {
                tokens.extend((0..depth).map(|_| Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                }));
                return Ok(tokens);
            }
            match token {
                Token::Char {
                    cat: Catcode::BeginGroup,
                    ..
                } => {
                    depth += 1;
                    tokens.push(token);
                }
                Token::Char {
                    cat: Catcode::EndGroup,
                    ..
                } if depth == 0 => return Ok(tokens),
                Token::Char {
                    cat: Catcode::EndGroup,
                    ..
                } => {
                    depth -= 1;
                    tokens.push(token);
                }
                _ => tokens.push(token),
            }
        }
        if depth == 0 {
            return Ok(tokens);
        }
        if stream_open
            && stores
                .world()
                .stream_bufs()
                .read_stream_target(slot)
                .is_none()
        {
            stores
                .world_mut()
                .write_text(PrintSink::TerminalAndLog, "\n! File ended within \\read.\n");
            tokens.extend((0..depth).map(|_| Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            }));
            return Ok(tokens);
        }
    }
}

fn replay_write_sink(value: i32) -> PrintSink {
    match value {
        0..=15 => PrintSink::Stream(StreamSlot::new(value as u8)),
        value if value < 0 => PrintSink::Log,
        _ => PrintSink::TerminalAndLog,
    }
}

fn replay_openout_target(name: String) -> String {
    let mut path = PathBuf::from(name);
    if path.extension().is_none() {
        path.set_extension("tex");
    }
    path.to_string_lossy().into_owned()
}

fn next_non_space(
    processor: &mut CommandProcessor<'_>,
) -> Result<Option<tex_command::CurrentCommand>, ExecError> {
    loop {
        let Some(command) = processor.get_x_token().map_err(command_error)? else {
            return Ok(None);
        };
        if !matches!(
            command.meaning(),
            Meaning::CharToken {
                cat: tex_state::token::Catcode::Space,
                ..
            }
        ) {
            return Ok(Some(command));
        }
    }
}

/// Captures executor-owned mutation data before structural application, then
/// emits it only after that application commits. This preserves the command
/// observer's canonical order without asking `tex-command` to inspect an
/// executor-owned aggregate mutation.
#[cfg(any(test, feature = "instrumentation"))]
fn applied_mutation_observation(
    scanned: &ScannedStep,
    stores: &Universe,
) -> Option<MutationRecord> {
    if let ScannedStep::Count {
        index,
        value,
        global,
    } = scanned
    {
        return Some(MutationRecord {
            target: "register",
            value: format!("count:{index}={value}"),
            key: None,
            tokens: None,
            global: *global,
        });
    }
    if let ScannedStep::Dimen {
        index,
        value,
        global,
    } = scanned
    {
        return Some(MutationRecord {
            target: "register",
            value: format!("scaled:{}", value.raw()),
            key: Some(format!("dimen:{index}")),
            tokens: None,
            global: *global,
        });
    }
    if let ScannedStep::Skip {
        index,
        value,
        global,
    } = scanned
    {
        return Some(MutationRecord {
            target: "register",
            value: format!(
                "glue:width={};stretch={};stretch_order={:?};shrink={};shrink_order={:?}",
                value.width.raw(),
                value.stretch.raw(),
                value.stretch_order,
                value.shrink.raw(),
                value.shrink_order,
            ),
            key: Some(format!("skip:{index}")),
            tokens: None,
            global: *global,
        });
    }
    if let ScannedStep::Toks {
        index,
        tokens,
        global,
    } = scanned
    {
        return Some(MutationRecord {
            target: "register",
            value: "tokens".into(),
            key: Some(format!("toks:{index}")),
            tokens: Some(
                stores
                    .tokens(tokens.token_list())
                    .iter()
                    .copied()
                    .map(|token| observed_macro_token(token, stores))
                    .collect(),
            ),
            global: *global,
        });
    }
    if let ScannedStep::TokParam {
        index,
        tokens,
        global,
    } = scanned
    {
        return Some(MutationRecord {
            target: "parameter",
            value: "tokens".into(),
            key: Some(format!("token_parameter:{index}")),
            tokens: Some(
                stores
                    .tokens(tokens.token_list())
                    .iter()
                    .copied()
                    .map(|token| observed_macro_token(token, stores))
                    .collect(),
            ),
            global: *global,
        });
    }
    if let ScannedStep::IntParam {
        index,
        value,
        global,
    } = scanned
    {
        return Some(MutationRecord {
            target: "parameter",
            value: format!("integer_parameter:{index}={value}"),
            key: None,
            tokens: None,
            global: *global,
        });
    }
    if let ScannedStep::GlueParam {
        index,
        value,
        global,
    } = scanned
    {
        return Some(MutationRecord {
            target: "parameter",
            value: format!(
                "glue:width={};stretch={};stretch_order={:?};shrink={};shrink_order={:?}",
                value.width.raw(),
                value.stretch.raw(),
                value.stretch_order,
                value.shrink.raw(),
                value.shrink_order,
            ),
            key: Some(format!("glue_parameter:{index}")),
            tokens: None,
            global: *global,
        });
    }
    if let ScannedStep::CodeTable {
        primitive,
        character,
        value,
        global,
    } = scanned
    {
        let (target, value) = match primitive {
            UnexpandablePrimitive::CatCode => {
                ("catcode", format!("{}={value}", u32::from(*character)))
            }
            UnexpandablePrimitive::LcCode => (
                "code_table",
                format!("lccode:{}={value}", u32::from(*character)),
            ),
            _ => unreachable!("only code-table primitives are scanned"),
        };
        return Some(MutationRecord {
            target,
            value,
            key: None,
            tokens: None,
            global: *global,
        });
    }
    if let ScannedStep::Let {
        target,
        source,
        meaning,
        global,
    } = scanned
    {
        return Some(MutationRecord {
            target: "meaning",
            value: let_mutation_value(*meaning, *source, stores),
            key: Some(stores.resolve(*target).to_owned()),
            tokens: None,
            global: *global,
        });
    }
    let ScannedStep::MacroDefinition {
        target,
        parameter_text,
        replacement_text,
        global,
        ..
    } = scanned
    else {
        return None;
    };
    let mut tokens = stores
        .tokens(parameter_text.token_list())
        .iter()
        .copied()
        .map(|token| match token {
            Token::Param(_) => ObservedToken::MacroMatch,
            token => observed_macro_token(token, stores),
        })
        .collect::<Vec<_>>();
    tokens.push(ObservedToken::MacroEndMatch);
    tokens.extend(
        stores
            .tokens(replacement_text.token_list())
            .iter()
            .copied()
            .map(|token| observed_macro_token(token, stores)),
    );
    Some(MutationRecord {
        target: "meaning",
        value: "macro definition".into(),
        key: Some(stores.resolve(*target).to_owned()),
        tokens: Some(tokens),
        global: *global,
    })
}

/// Captures an executor-owned observable effect before application, then
/// emits it only after that application commits through the replay seam.
fn canonical_pdf_image_dimensions(
    source: &tex_state::PdfExternalImageSource,
    width: Option<Scaled>,
    height: Option<Scaled>,
    depth: Option<Scaled>,
) -> tex_state::PdfExternalImageDimensions {
    let natural_width = source.natural_width;
    let natural_height = source.natural_height;
    let (width, height) = match (width, height) {
        (Some(width), Some(height)) => (width, height),
        (Some(width), None) if natural_width.raw() != 0 => (
            width,
            Scaled::from_raw(
                (i64::from(natural_height.raw()) * i64::from(width.raw())
                    / i64::from(natural_width.raw())) as i32,
            ),
        ),
        (None, Some(height)) if natural_height.raw() != 0 => (
            Scaled::from_raw(
                (i64::from(natural_width.raw()) * i64::from(height.raw())
                    / i64::from(natural_height.raw())) as i32,
            ),
            height,
        ),
        (Some(width), None) => (width, natural_height),
        (None, Some(height)) => (natural_width, height),
        (None, None) => (natural_width, natural_height),
    };
    tex_state::PdfExternalImageDimensions {
        width,
        height,
        depth: depth.unwrap_or_else(|| Scaled::from_raw(0)),
    }
}

/// Applies pdfTeX's live `\pdfpagebox` and `\pdfforcepagebox` state after
/// command-owned source scanning but before the immutable host request is
/// exposed. This keeps `CommandProcessor` independent of `Universe` while
/// ensuring the host sees the effective page-box identity.
fn canonical_pdf_image_page_box(
    stores: &Universe,
    request: &PdfImageRequest,
) -> tex_command::PdfImagePageBox {
    let page_box = |value| match value {
        1 => tex_command::PdfImagePageBox::Media,
        2 => tex_command::PdfImagePageBox::Crop,
        3 => tex_command::PdfImagePageBox::Bleed,
        4 => tex_command::PdfImagePageBox::Trim,
        5 => tex_command::PdfImagePageBox::Art,
        _ => tex_command::PdfImagePageBox::Crop,
    };
    let forced = stores.int_param(IntParam::PDF_FORCE_PAGE_BOX);
    if forced > 0 {
        page_box(forced)
    } else if request.page_box_explicit {
        request.page_box
    } else {
        page_box(stores.int_param(IntParam::PDF_PAGE_BOX))
    }
}

#[cfg(any(test, feature = "instrumentation"))]
fn applied_effect_observation(scanned: &ScannedStep, stores: &Universe) -> Option<EffectRecord> {
    match scanned {
        ScannedStep::Message { tokens, .. } => Some(EffectRecord {
            kind: "message",
            detail: replay_text(stores.tokens(tokens.token_list())),
            tokens: None,
        }),
        ScannedStep::ImmediateExtension(ImmediateExtension::Write { stream, tokens }) => {
            Some(EffectRecord {
                kind: "write",
                detail: format!("stream:{stream}\0"),
                tokens: Some(
                    stores
                        .tokens(tokens.token_list())
                        .iter()
                        .copied()
                        .map(|token| observed_macro_token(token, stores))
                        .collect(),
                ),
            })
        }
        ScannedStep::ImmediateExtension(ImmediateExtension::OpenOut { .. }) => {
            match stores.world().effect_records().last()? {
                tex_state::EffectRecord::StreamOpen { slot, target } => Some(EffectRecord {
                    kind: "open",
                    detail: format!("stream:{}\0{}", slot.raw(), target.path().to_string_lossy()),
                    tokens: None,
                }),
                _ => None,
            }
        }
        ScannedStep::ImmediateExtension(ImmediateExtension::CloseOut { .. }) => {
            match stores.world().effect_records().last()? {
                tex_state::EffectRecord::StreamClose { slot } => Some(EffectRecord {
                    kind: "close",
                    detail: format!("stream:{}", slot.raw()),
                    tokens: None,
                }),
                _ => None,
            }
        }
        ScannedStep::BoxRegister {
            ships_out: true, ..
        }
        | ScannedStep::BoxEndGroup { ships_out: true }
        | ScannedStep::ForcedOutputShipout => Some(EffectRecord {
            kind: "shipout",
            detail: format!("dvi\0{}", stores.world().artifact_commits().len()),
            tokens: None,
        }),
        ScannedStep::Terminate => Some(EffectRecord {
            kind: "terminate",
            detail: "engine\0".into(),
            tokens: None,
        }),
        _ => None,
    }
}

/// TeX82 §1071 completes `box_end` synchronously: a `\shipout` box is
/// published while its command-owned terminator backup is still live.  The
/// artifact kernel receives only an already-published detached input summary;
/// it never receives a legacy source stack or scans the command operand.
fn shipout_replay_box(
    node: Node,
    stores: &mut Universe,
) -> Result<Option<crate::dispatch::PreparedDviPage>, ExecError> {
    let mut execution = crate::ExecutionContext::new("texput");
    let input_summary = stores.input_summary().clone();
    let receipt = crate::assignments::shipout_node_with_input_summary(
        node,
        input_summary,
        stores,
        &mut execution,
    )?;
    // TeX82's `ship_out` clears the consecutive-dead-output counter (§643).
    // Canonical lowering bypasses the legacy executor's bookkeeping, so keep
    // the page-state transition at the typed shipout boundary.
    stores.set_page_integer(tex_state::page::PageInteger::DeadCycles, 0);
    Ok(receipt)
}

#[cfg(any(test, feature = "instrumentation"))]
fn let_mutation_value(meaning: Meaning, source: Option<Symbol>, stores: &Universe) -> String {
    match meaning {
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::BeginGroup) => "begin_group".into(),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::EndGroup) => "end_group".into(),
        _ => source.map_or_else(
            || format!("{meaning:?}"),
            |source| stores.resolve(source).to_owned(),
        ),
    }
}

#[cfg(any(test, feature = "instrumentation"))]
fn observed_macro_token(token: Token, stores: &Universe) -> ObservedToken {
    match token {
        Token::Char { ch, cat } => ObservedToken::Character {
            character: ch,
            catcode: cat,
        },
        Token::Cs(symbol) => ObservedToken::ControlSequence(stores.resolve(symbol).to_owned()),
        Token::Param(slot) => ObservedToken::Parameter(slot),
        Token::Frozen(_) if token.is_frozen_end_template() => ObservedToken::FrozenEndTemplate,
        Token::Frozen(_) if token.is_frozen_endv() => ObservedToken::FrozenEndV,
        Token::Frozen(frozen) => frozen
            .primitive_index()
            .map_or(ObservedToken::FrozenOther, ObservedToken::FrozenPrimitive),
    }
}

/// Applies TeX82 `fin_col`'s saved-delimiter selection after `do_endv`.
///
/// The delimiter was classified and retained by `tex-command` at the original
/// `get_next` boundary.  This code receives only its typed outcome, chooses
/// the next frozen template pair, and lets command-owned lookahead/back-up
/// prepare the next entry.
fn begin_next_replay_alignment_cell(
    alignment: AlignmentIdentity,
    delimiter: AlignmentCellDelimiter,
    command: &mut CommandState,
    active_alignment: &mut Option<ActiveReplayAlignment>,
    modes: &mut ModeNest,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    let active = active_alignment
        .as_mut()
        .filter(|active| active.identity == alignment)
        .ok_or(ExecError::MissingToken {
            context: "active replay alignment",
        })?;
    // Focused lifecycle tests may construct a command-state cell directly,
    // without replaying a preamble.  There is then no executor template
    // selection to perform after the otherwise complete command transition.
    if active.columns.is_empty() {
        return Ok(());
    }
    if delimiter == AlignmentCellDelimiter::Span {
        active.cell_span = active
            .cell_span
            .checked_add(1)
            .ok_or(ExecError::ArithmeticOverflow)?;
    } else {
        capture_replay_alignment_cell(active, modes, stores)?;
    }
    let next_column = match delimiter {
        AlignmentCellDelimiter::Tab | AlignmentCellDelimiter::Span => active
            .column
            .checked_add(1)
            .ok_or(ExecError::ArithmeticOverflow)?,
        AlignmentCellDelimiter::Row => 0,
    };
    if next_column >= active.columns.len()
        && active.repeat_start.is_none()
        && matches!(
            delimiter,
            AlignmentCellDelimiter::Tab | AlignmentCellDelimiter::Span
        )
    {
        let recovered = command
            .apply_alignment_request(AlignmentRequest::RecoverExtraTab(alignment))
            .map_err(|_| ExecError::MissingToken {
                context: "alignment extra-tab recovery",
            })?;
        debug_assert!(matches!(
            recovered,
            AlignmentRequestResult::ExtraTabRecovered
        ));
        finish_replay_alignment_row(active, modes, stores)?;
        active.column = 0;
        active.align_peek_pending = true;
        return Ok(());
    }
    active.column = if next_column < active.columns.len() {
        next_column
    } else if let Some(repeat_start) = active.repeat_start {
        let repeat_len =
            active
                .columns
                .len()
                .checked_sub(repeat_start)
                .ok_or(ExecError::MissingToken {
                    context: "alignment periodic-preamble boundary",
                })?;
        if repeat_len == 0 {
            return Err(ExecError::MissingToken {
                context: "alignment periodic-preamble columns",
            });
        }
        repeat_start + (next_column - repeat_start) % repeat_len
    } else {
        next_column
    };
    let templates = active
        .columns
        .get(active.column)
        .copied()
        .ok_or(ExecError::MissingToken {
            context: "next alignment preamble column",
        })?;
    match delimiter {
        AlignmentCellDelimiter::Row => {
            finish_replay_alignment_row(active, modes, stores)?;
            active.align_peek_pending = true;
        }
        AlignmentCellDelimiter::Tab | AlignmentCellDelimiter::Span => {
            command
                .apply_alignment_request(AlignmentRequest::BeginCell {
                    alignment,
                    templates,
                })
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment next-cell lifecycle",
                })?;
            if delimiter == AlignmentCellDelimiter::Tab {
                begin_replay_alignment_cell(active, modes)?;
            }
            active.next_cell_opening_pending = true;
        }
    }
    Ok(())
}

fn replay_alignment_mode(kind: AlignmentKind) -> Mode {
    match kind {
        AlignmentKind::HAlign => Mode::InternalVertical,
        AlignmentKind::VAlign => Mode::RestrictedHorizontal,
    }
}

fn replay_alignment_row_mode(kind: AlignmentKind) -> Mode {
    match kind {
        // Keep a row frame below the cell so a recovered paragraph can
        // return to the alignment without consuming the outer list
        // prematurely. `finish_replay_alignment` owns the later canonical
        // unset-row conversion and final packing.
        AlignmentKind::HAlign => Mode::InternalVertical,
        AlignmentKind::VAlign => Mode::InternalVertical,
    }
}

fn replay_alignment_cell_mode(kind: AlignmentKind) -> Mode {
    match kind {
        AlignmentKind::HAlign => Mode::Horizontal,
        AlignmentKind::VAlign => Mode::InternalVertical,
    }
}

fn begin_replay_alignment_cell(
    active: &mut ActiveReplayAlignment,
    modes: &mut ModeNest,
) -> Result<(), ExecError> {
    if !active.row_open {
        modes.push(replay_alignment_row_mode(active.kind));
        modes.current_list_mut().push(Node::Glue {
            spec: active.tabskip,
            kind: GlueKind::TabSkip,
            leader: None,
        });
        active.captured_rows.push(Vec::new());
        active.row_open = true;
    }
    if active.cell_open {
        return Err(ExecError::MissingToken {
            context: "active replay alignment cell",
        });
    }
    modes.push(replay_alignment_cell_mode(active.kind));
    active.cell_span = 1;
    active.cell_open = true;
    Ok(())
}

fn capture_replay_alignment_cell(
    active: &mut ActiveReplayAlignment,
    modes: &mut ModeNest,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    if !active.cell_open {
        return Ok(());
    }
    crate::assignments::flush_pending_hchars(modes, stores)?;
    let mut cell = modes.pop()?;
    let material = if active.kind == AlignmentKind::HAlign {
        crate::math::finish_math_lists_owned(stores, cell.list_mut().take_nodes(), false)
    } else {
        cell.list_mut().take_nodes()
    };
    let material = stores.freeze_node_list(&material);
    active
        .captured_rows
        .last_mut()
        .ok_or(ExecError::MissingToken {
            context: "active replay alignment row",
        })?
        .push(material);
    let cell = crate::align::packaging::make_unset_node(
        stores,
        material,
        crate::align::packaging::cell_unset_kind(active.kind),
        active.cell_span,
    );
    modes.current_list_mut().push(cell);
    modes.current_list_mut().push(Node::Glue {
        spec: active.tabskip,
        kind: GlueKind::TabSkip,
        leader: None,
    });
    active.cell_open = false;
    Ok(())
}

fn finish_replay_alignment_row(
    active: &mut ActiveReplayAlignment,
    modes: &mut ModeNest,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    capture_replay_alignment_cell(active, modes, stores)?;
    if !active.row_open {
        return Ok(());
    }
    crate::assignments::flush_pending_hchars(modes, stores)?;
    let mut row = modes.pop()?;
    let children = stores.freeze_node_list(&row.list_mut().take_nodes());
    let row = crate::align::packaging::make_unset_node(
        stores,
        children,
        crate::align::packaging::row_unset_kind(active.kind),
        1,
    );
    modes.current_list_mut().push(row);
    active.row_open = false;
    Ok(())
}

fn finish_replay_alignment(
    active: &mut ActiveReplayAlignment,
    modes: &mut ModeNest,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    finish_replay_alignment_row(active, modes, stores)?;
    let mut alignment = modes.pop()?;
    let rows = alignment.list_mut().take_nodes();
    let columns = active
        .columns
        .iter()
        .map(|templates| AlignColumn {
            u_template: templates
                .u_template
                .expect("canonical columns retain u templates")
                .token_list(),
            v_template: templates.v_template.token_list(),
        })
        .collect();
    let state = AlignState::new(
        active.kind,
        AlignmentPackSpec::Natural,
        columns,
        vec![active.tabskip; active.columns.len().saturating_add(1)],
        active.tabskip,
        active.repeat_start,
    );
    let finished = crate::align::widths::finish_alignment(&state, &rows, stores)?;
    crate::align::append_finished_alignment(
        modes,
        stores,
        crate::align::FinishedAlignment {
            nodes: finished,
            aux_prev_depth: alignment.list().prev_depth(),
        },
    );
    crate::vertical::build_page_if_outer_vertical(modes, stores)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)] // applies the complete canonical replay state atomically
fn apply_scanned_step(
    scanned: ScannedStep,
    stores: &mut Universe,
    modes: &mut ModeNest,
    next_alignment_identity: &mut u64,
    active_alignment: &mut Option<ActiveReplayAlignment>,
    command: &mut CommandState,
    boxes: &mut ReplayBoxes,
    prepared_dvi_pages: &mut Vec<crate::dispatch::PreparedDviPage>,
) -> Result<ReplayStep, ExecError> {
    match scanned {
        ScannedStep::Continue => Ok(ReplayStep::Continue),
        // These are intercepted by `CanonicalMainControl::step_once`, where
        // the owning opaque episode and mutable replay driver are available.
        ScannedStep::ReplayCompleted(_) | ScannedStep::Math(_) | ScannedStep::MathDelimiter(_) => {
            Ok(ReplayStep::Continue)
        }
        ScannedStep::MathFamily {
            family,
            font,
            global,
        } => {
            let size = match family.size {
                tex_command::MathFamilySize::Text => MathFontSize::Text,
                tex_command::MathFamilySize::Script => MathFontSize::Script,
                tex_command::MathFamilySize::ScriptScript => MathFontSize::ScriptScript,
            };
            stores.set_math_family_font(
                size,
                family.family,
                font,
                assignment_global(global, stores),
            );
            Ok(ReplayStep::Continue)
        }
        ScannedStep::EndOfInput | ScannedStep::Terminate => Ok(ReplayStep::EndOfInput),
        ScannedStep::End => Ok(ReplayStep::End),
        ScannedStep::Count {
            index,
            value,
            global,
        } => {
            let global = assignment_global(global, stores);
            if global {
                stores.set_count_global(index, value);
            } else {
                stores.set_count(index, value);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Dimen {
            index,
            value,
            global,
        } => {
            let global = assignment_global(global, stores);
            if global {
                stores.set_dimen_global(index, value);
            } else {
                stores.set_dimen(index, value);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Skip {
            index,
            value,
            global,
        } => {
            let global = assignment_global(global, stores);
            let value = stores.intern_glue(value);
            if global {
                stores.set_skip_global(index, value);
            } else {
                stores.set_skip(index, value);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Muskip {
            index,
            value,
            global,
        } => {
            let value = stores.intern_glue(value);
            if assignment_global(global, stores) {
                stores.set_muskip_global(index, value);
            } else {
                stores.set_muskip(index, value);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::HorizontalSkip { value } => {
            if matches!(
                modes.current_mode(),
                Mode::Vertical | Mode::InternalVertical
            ) {
                start_canonical_paragraph(command, modes, stores, true)?;
            }
            crate::assignments::flush_pending_hchars(modes, stores)?;
            modes.current_list_mut().push(Node::Glue {
                spec: stores.intern_glue(value),
                kind: GlueKind::Normal,
                leader: None,
            });
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Kern { amount } => {
            if matches!(
                modes.current_mode(),
                Mode::Vertical | Mode::InternalVertical
            ) {
                start_canonical_paragraph(command, modes, stores, true)?;
            }
            crate::assignments::flush_pending_hchars(modes, stores)?;
            modes.current_list_mut().push(Node::Kern {
                amount,
                kind: KernKind::Explicit,
            });
            Ok(ReplayStep::Continue)
        }
        ScannedStep::CharacterCode { value } => {
            let ch = u32::try_from(value).ok().and_then(char::from_u32).ok_or(
                ExecError::InvalidCode {
                    context: "\\char",
                    value,
                },
            )?;
            if matches!(
                modes.current_mode(),
                Mode::Vertical | Mode::InternalVertical
            ) {
                start_canonical_paragraph(command, modes, stores, true)?;
            }
            crate::assignments::append_canonical_character(
                modes,
                stores,
                ch,
                tex_state::token::OriginId::UNKNOWN,
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::FixedHorizontalGlue { primitive } => {
            if matches!(
                modes.current_mode(),
                Mode::Vertical | Mode::InternalVertical
            ) {
                start_canonical_paragraph(command, modes, stores, true)?;
            }
            crate::assignments::flush_pending_hchars(modes, stores)?;
            modes.current_list_mut().push(Node::Glue {
                spec: stores.intern_glue(crate::assignments::fixed_infinite_glue(primitive)),
                kind: GlueKind::Normal,
                leader: None,
            });
            Ok(ReplayStep::Continue)
        }
        ScannedStep::ParagraphIndent { indent } => {
            start_canonical_paragraph(command, modes, stores, indent)?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::ParagraphShape { lines, global } => {
            stores.set_paragraph_shape(&lines, global);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Toks {
            index,
            tokens,
            global,
        } => {
            let global = assignment_global(global, stores);
            if global {
                stores.set_toks_global(index, tokens.token_list());
            } else {
                stores.set_toks(index, tokens.token_list());
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::IntParam {
            index,
            value,
            global,
        } => {
            let global = assignment_global(global, stores);
            let parameter = IntParam::new(index);
            if global {
                stores.set_int_param_global(parameter, value);
            } else {
                stores.set_int_param(parameter, value);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::DimenParam {
            index,
            value,
            global,
        } => {
            let parameter = DimenParam::new(index);
            if assignment_global(global, stores) {
                stores.set_dimen_param_global(parameter, value);
            } else {
                stores.set_dimen_param(parameter, value);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::TokParam {
            index,
            tokens,
            global,
        } => {
            let global = assignment_global(global, stores);
            let parameter = TokParam::new(index);
            if global {
                stores.set_tok_param_global(parameter, tokens.token_list());
            } else {
                stores.set_tok_param(parameter, tokens.token_list());
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::GlueParam {
            index,
            value,
            global,
        } => {
            let global = assignment_global(global, stores);
            let parameter = GlueParam::new(index);
            let value = stores.intern_glue(value);
            if global {
                stores.set_glue_param_global(parameter, value);
            } else {
                stores.set_glue_param(parameter, value);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::CodeTable {
            primitive,
            character,
            value,
            global,
        } => {
            let global = assignment_global(global, stores);
            match primitive {
                UnexpandablePrimitive::CatCode => {
                    let value = match value {
                        0 => Catcode::Escape,
                        1 => Catcode::BeginGroup,
                        2 => Catcode::EndGroup,
                        3 => Catcode::MathShift,
                        4 => Catcode::AlignmentTab,
                        5 => Catcode::EndLine,
                        6 => Catcode::Parameter,
                        7 => Catcode::Superscript,
                        8 => Catcode::Subscript,
                        9 => Catcode::Ignored,
                        10 => Catcode::Space,
                        11 => Catcode::Letter,
                        12 => Catcode::Other,
                        13 => Catcode::Active,
                        14 => Catcode::Comment,
                        15 => Catcode::Invalid,
                        _ => {
                            return Err(ExecError::InvalidCode {
                                context: "\\catcode",
                                value,
                            });
                        }
                    };
                    if global {
                        stores.set_catcode_global(character, value);
                    } else {
                        stores.set_catcode(character, value);
                    }
                }
                UnexpandablePrimitive::LcCode => {
                    let value = u32::try_from(value).map_err(|_| ExecError::InvalidCode {
                        context: "\\lccode",
                        value,
                    })? as LcCode;
                    if global {
                        stores.set_lccode_global(character, value);
                    } else {
                        stores.set_lccode(character, value);
                    }
                }
                UnexpandablePrimitive::UcCode => {
                    let value = checked_character_code(value, "\\uccode")? as UcCode;
                    if global {
                        stores.set_uccode_global(character, value);
                    } else {
                        stores.set_uccode(character, value);
                    }
                }
                UnexpandablePrimitive::SfCode => {
                    let value = u16::try_from(value)
                        .ok()
                        .filter(|value| *value <= 32_767)
                        .ok_or(ExecError::InvalidCode {
                            context: "\\sfcode",
                            value,
                        })? as SfCode;
                    if global {
                        stores.set_sfcode_global(character, value);
                    } else {
                        stores.set_sfcode(character, value);
                    }
                }
                UnexpandablePrimitive::MathCode => {
                    let value = u32::try_from(value)
                        .ok()
                        .filter(|value| *value <= 32_768)
                        .ok_or(ExecError::InvalidCode {
                            context: "\\mathcode",
                            value,
                        })? as MathCode;
                    if global {
                        stores.set_mathcode_global(character, value);
                    } else {
                        stores.set_mathcode(character, value);
                    }
                }
                UnexpandablePrimitive::DelCode => {
                    let value = (-1..=0xFF_FFFF)
                        .contains(&value)
                        .then_some(value as DelCode)
                        .ok_or(ExecError::InvalidCode {
                            context: "\\delcode",
                            value,
                        })?;
                    if global {
                        stores.set_delcode_global(character, value);
                    } else {
                        stores.set_delcode(character, value);
                    }
                }
                _ => unreachable!("only code-table primitives are scanned"),
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::FontSelect {
            font,
            selector,
            global,
        } => {
            if assignment_global(global, stores) {
                if let Some(selector) = selector {
                    stores.set_current_font_selector_global(selector, font);
                } else {
                    stores.set_current_font_global(font);
                }
            } else if let Some(selector) = selector {
                stores.set_current_font_selector(selector, font);
            } else {
                stores.set_current_font(font);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::FontDefinition {
            request,
            resource,
            global,
        } => {
            let resource =
                (*resource).expect("font resource is resolved after the processor borrow");
            if matches!(resource, FontResource::Unavailable) {
                if assignment_global(global, stores) {
                    stores.set_meaning_global(
                        request.target,
                        Meaning::Font(tex_state::font::NULL_FONT),
                    );
                } else {
                    stores.set_meaning(request.target, Meaning::Font(tex_state::font::NULL_FONT));
                }
                return Ok(ReplayStep::Continue);
            }
            let loaded = load_canonical_font(&request, resource)?;
            let id = stores.try_intern_font_with_identifier(loaded, request.target)?;
            if assignment_global(global, stores) {
                stores.set_meaning_global(request.target, Meaning::Font(id));
            } else {
                stores.set_meaning(request.target, Meaning::Font(id));
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::InputStream { request, resource } => {
            match request {
                InputStreamRequest::Open { stream, file_name } => {
                    let slot = replay_stream_slot(stream, stores);
                    let resource =
                        resource.expect("openin resource resolves after processor borrow");
                    stores
                        .world_mut()
                        .set_memory_file(&file_name.name, resource.bytes().to_vec())?;
                    let content = InputReadState::read_input_file(
                        &mut stores.input_open_context(),
                        std::path::Path::new(&file_name.name),
                    )?;
                    stores.world_mut().open_in_content(slot, &content)?;
                }
                InputStreamRequest::Close { stream } => {
                    let slot = replay_stream_slot(stream, stores);
                    stores.world_mut().close_in(slot);
                }
                InputStreamRequest::Read {
                    stream,
                    target,
                    raw_catcodes,
                } => {
                    let slot = replay_stream_slot(stream, stores);
                    let tokens = canonical_read_tokens(slot, target, raw_catcodes, stores)?;
                    let replacement = stores.intern_token_list(&tokens);
                    let parameters = stores.intern_token_list(&[]);
                    stores.set_macro_meaning(
                        target,
                        MacroMeaning::new(MeaningFlags::EMPTY, parameters, replacement),
                    );
                }
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::PdfXImage { request, resource } => {
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfximage"));
            }
            let PdfImageResource::Available(source) = resource else {
                return Err(ExecError::PdfImageOpen {
                    name: request.name,
                    message: "image is unavailable".to_owned(),
                });
            };
            let dimensions = canonical_pdf_image_dimensions(
                &source,
                request.width,
                request.height,
                request.depth,
            );
            stores
                .allocate_pdf_external_image(source, dimensions)
                .map_err(|_| ExecError::PdfObjectCapacity)?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::PdfRefXImage { object } => {
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfrefximage"));
            }
            let image = u32::try_from(object)
                .ok()
                .and_then(|raw| tex_state::PdfExternalImageId::new(raw).ok())
                .and_then(|id| stores.pdf_external_image_record(id))
                .ok_or(ExecError::PdfReferencedObjectNotFound)?;
            let dimensions = image.dimensions();
            modes
                .current_list_mut()
                .push(Node::Whatsit(Whatsit::PdfRefXImage {
                    object: image.id().raw(),
                    width: dimensions.width,
                    height: dimensions.height,
                    depth: dimensions.depth,
                }));
            Ok(ReplayStep::Continue)
        }
        ScannedStep::PdfGraphics(request) => apply_pdf_graphics_request(request, stores, modes),
        ScannedStep::PdfNavigation(request) => apply_pdf_navigation_request(request, stores, modes),
        ScannedStep::PdfObject(request) => apply_pdf_object_request(request, stores, false),
        ScannedStep::PdfReferenceObject(request) => {
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfrefobj"));
            }
            let object = u32::try_from(request.object)
                .ok()
                .filter(|object| stores.pdf_raw_object(*object).is_some())
                .ok_or(ExecError::PdfReferencedObjectNotFound)?;
            modes
                .current_list_mut()
                .push(Node::Whatsit(Whatsit::PdfReferenceObject { object }));
            Ok(ReplayStep::Continue)
        }
        ScannedStep::PdfForm(request) => apply_pdf_form_request(request, stores, modes, false),
        ScannedStep::PdfDocumentFragment(request) => {
            let dvi_only_error = matches!(request.kind, tex_state::PdfDocumentFragmentKind::Names);
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                if dvi_only_error {
                    return Err(ExecError::PdfExtensionInDviMode("pdfnames"));
                }
                return Ok(ReplayStep::Continue);
            }
            stores.append_pdf_document_fragment(request.kind, request.text.tokens.token_list());
            if let Some(action) = request.open_action {
                if stores.pdf_catalog_open_action().is_some() {
                    return Err(ExecError::PdfDuplicateOpenAction);
                }
                let (destination, structure, thread) = pdf_action_target_identities(stores, action);
                stores
                    .set_pdf_catalog_open_action_with_targets(
                        action,
                        destination,
                        structure,
                        thread,
                    )
                    .map_err(|_| ExecError::PdfObjectCapacity)?;
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::FontDimen {
            font,
            number,
            value,
        } => {
            stores.set_font_dimen(font, number, value)?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::FontInteger { font, skew, value } => {
            if skew {
                stores.set_font_skew_char(font, value);
            } else {
                stores.set_font_hyphen_char(font, value);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::DeferredOpenOut { stream, file_name } => {
            modes
                .current_list_mut()
                .push(Node::Whatsit(Whatsit::OpenOut {
                    slot: replay_stream_slot(stream, stores),
                    path: file_name,
                }));
            Ok(ReplayStep::Continue)
        }
        ScannedStep::DeferredCloseOut { stream } => {
            modes
                .current_list_mut()
                .push(Node::Whatsit(Whatsit::CloseOut {
                    slot: replay_stream_slot(stream, stores),
                }));
            Ok(ReplayStep::Continue)
        }
        ScannedStep::DeferredWrite { stream, tokens } => {
            modes
                .current_list_mut()
                .push(Node::Whatsit(Whatsit::DeferredWrite {
                    sink: replay_write_sink(stream),
                    tokens: tokens.token_list(),
                }));
            Ok(ReplayStep::Continue)
        }
        ScannedStep::DeferredSpecial { tokens } => {
            let mut text = String::new();
            for &token in stores.tokens(tokens.token_list()) {
                tex_expand::append_token_string_text(stores, token, &mut text);
            }
            modes
                .current_list_mut()
                .push(Node::Whatsit(Whatsit::Special {
                    class: "dvi".to_owned(),
                    payload: tex_byte_text(&text),
                }));
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Arithmetic {
            primitive,
            target,
            operand,
            global,
        } => {
            apply_arithmetic(
                primitive,
                target,
                operand,
                assignment_global(global, stores),
                stores,
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::MacroDefinition {
            target,
            flags,
            global,
            parameter_text,
            replacement_text,
            definition_origin,
            missing_target,
            malformed_parameter,
        } => {
            if missing_target {
                stores.world_mut().write_text(
                    PrintSink::TerminalAndLog,
                    "\n! Missing control sequence inserted.\nPlease don't say `\\def cs{...}', say `\\def\\cs{...}'.\nI've inserted an inaccessible control sequence so that your\ndefinition will be completed without mixing me up too badly.\n",
                );
            }
            if malformed_parameter {
                stores.world_mut().write_text(
                    PrintSink::TerminalAndLog,
                    "\n! Illegal parameter number in definition.\nYou meant to type ## instead of #, right?\n",
                );
            }
            let meaning = MacroMeaning::new(
                flags,
                parameter_text.token_list(),
                replacement_text.token_list(),
            );
            let provenance = MacroDefinitionProvenance::new(
                definition_origin,
                parameter_text.origin_list(),
                replacement_text.origin_list(),
            );
            if global {
                stores.set_macro_meaning_global_with_provenance(target, meaning, provenance);
            } else {
                stores.set_macro_meaning_with_provenance(target, meaning, provenance);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Let {
            target,
            source,
            meaning,
            global,
        } => {
            let _ = source;
            if global {
                stores.set_meaning_global(target, meaning);
            } else {
                stores.set_meaning(target, meaning);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::AfterGroup(token) => {
            stores.push_aftergroup(token);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::AfterAssignment(token) => {
            stores.set_afterassignment(token);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Rule {
            width,
            height,
            depth,
            horizontal,
        } => {
            if horizontal
                && matches!(
                    modes.current_mode(),
                    Mode::Horizontal | Mode::RestrictedHorizontal
                )
            {
                let _ = modes.pop()?;
            }
            modes.current_list_mut().push(Node::Rule {
                width,
                height,
                depth,
            });
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Message { tokens, error } => {
            let text = replay_text(stores.tokens(tokens.token_list()));
            if error {
                stores
                    .world_mut()
                    .write_text(PrintSink::TerminalAndLog, &format!("\n! {text}.\n"));
            } else {
                stores
                    .world_mut()
                    .write_text(PrintSink::TerminalAndLog, &text);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::DisplayDiagnostic(diagnostic) => {
            stores
                .world_mut()
                .write_text(PrintSink::TerminalAndLog, &diagnostic.text);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::ShowBox { index } => {
            crate::diagnostics::execute_showbox(stores, index);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::ImmediateExtension(extension) => {
            match extension {
                ImmediateExtension::Continue => {}
                ImmediateExtension::OpenOut { stream, file_name } => {
                    let stream = replay_stream_slot(stream, stores);
                    stores
                        .world_mut()
                        .open_out(stream, replay_openout_target(file_name.name));
                }
                ImmediateExtension::Write { stream, tokens } => {
                    let sink = replay_write_sink(stream);
                    let text = replay_text(stores.tokens(tokens.token_list()));
                    stores.world_mut().write_text(sink, &text);
                }
                ImmediateExtension::CloseOut { stream } => {
                    let stream = replay_stream_slot(stream, stores);
                    stores.world_mut().close_out(stream);
                }
                ImmediateExtension::PdfObject(request) => {
                    if matches!(request, PdfObjectRequest::Reserve) {
                        apply_pdf_object_request(request, stores, false)?;
                        return Err(ExecError::PdfImmediateReservedObject);
                    }
                    apply_pdf_object_request(request, stores, true)?;
                }
                ImmediateExtension::PdfForm(request) => {
                    apply_pdf_form_request(request, stores, modes, true)?;
                }
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::SetBox(target) => {
            boxes.pending_setbox = Some(target);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::VSplit(split) => {
            if split.missing_to {
                stores.world_mut().write_text(
                    PrintSink::TerminalAndLog,
                    "\n! Missing `to' inserted.\nI'm working on `\\vsplit<box number> to <dimen>';\nwill look for the <dimen> next.\n",
                );
            }
            let index = u16::try_from(split.index)
                .map_err(|_| ExecError::RegisterNumberOutOfRange(split.index))?;
            let node = crate::assignments::split_vbox_register(stores, index, split.height)?;
            let target = boxes.pending_setbox.take();
            if let (Some(target), Some(node)) = (target, node) {
                let boxed = stores.freeze_node_list(std::slice::from_ref(&node));
                if target.global {
                    stores.set_box_reg_global(target.index, boxed);
                } else {
                    stores.set_box_reg(target.index, boxed);
                }
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BoxRegister {
            index,
            copy,
            ships_out,
        } => {
            let id = if copy {
                stores.box_reg(index)
            } else {
                stores.take_box_reg_same_level(index)
            };
            if copy && let Some(id) = id {
                stores.pin_survivor(id);
            }
            if ships_out {
                boxes.pending_shipout = false;
                if let Some(node) =
                    id.and_then(|id| stores.nodes(id).first().map(|node| node.to_owned()))
                    && let Some(receipt) = shipout_replay_box(node, stores)?
                {
                    prepared_dvi_pages.push(receipt);
                }
            } else {
                crate::assignments::execute_scanned_box_register(
                    if copy {
                        UnexpandablePrimitive::Copy
                    } else {
                        UnexpandablePrimitive::Box
                    },
                    id,
                    modes,
                    stores,
                )?;
                crate::vertical::build_page_if_outer_vertical(modes, stores)?;
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Unbox { primitive, index } => {
            crate::assignments::execute_scanned_unbox(primitive, index, modes, stores)?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::LastBox => {
            crate::assignments::execute_scanned_last_box(modes, stores)?;
            crate::vertical::build_page_if_outer_vertical(modes, stores)?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Leaders {
            kind,
            payload,
            glue,
        } => {
            boxes.pending_leader = None;
            let spec = stores.intern_glue(glue);
            crate::vertical::append_node_to_current_list(
                modes,
                stores,
                Node::Glue {
                    spec,
                    kind,
                    leader: Some(payload),
                },
            )?;
            crate::vertical::build_page_if_outer_vertical(modes, stores)?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::LeaderRegister {
            kind,
            index,
            copy,
            glue,
        } => {
            let id = if copy {
                stores.box_reg(index)
            } else {
                stores.take_box_reg_same_level(index)
            };
            if copy && let Some(id) = id {
                stores.pin_survivor(id);
            }
            if let Some(payload) = id
                .and_then(|id| stores.nodes(id).first().map(|node| node.to_owned()))
                .and_then(payload_from_node)
            {
                let spec = stores.intern_glue(glue);
                crate::vertical::append_node_to_current_list(
                    modes,
                    stores,
                    Node::Glue {
                        spec,
                        kind,
                        leader: Some(payload),
                    },
                )?;
                crate::vertical::build_page_if_outer_vertical(modes, stores)?;
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::MissingLeaderPayload => {
            stores.world_mut().write_text(
                PrintSink::TerminalAndLog,
                "\n! A <box> was supposed to be here.\nI'm ignoring these leaders.\n",
            );
            Ok(ReplayStep::Continue)
        }
        ScannedStep::LeadersNotFollowedByGlue => {
            boxes.pending_leader = None;
            stores.world_mut().write_text(
                PrintSink::TerminalAndLog,
                "\n! Leaders not followed by proper glue.\nYou should say `\\leaders <box or rule><hskip or vskip>'.\nI'm ignoring these leaders.\n",
            );
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BeginShipout => {
            boxes.pending_shipout = true;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BeginBox(construction) => {
            let target = boxes.pending_setbox.take();
            let ships_out = std::mem::take(&mut boxes.pending_shipout);
            let kind = match construction.kind {
                ScannedBoxKind::HBox => ReplayBoxKind::HBox,
                ScannedBoxKind::VBox => ReplayBoxKind::VBox,
                ScannedBoxKind::VTop => ReplayBoxKind::VTop,
            };
            let packing = match construction.packing {
                ScannedPackingSpec::Natural => PackSpec::Natural,
                ScannedPackingSpec::Exactly(size) => PackSpec::Exactly(size),
                ScannedPackingSpec::Spread(size) => PackSpec::Spread(size),
            };
            stores.enter_group_with_kind(kind.group_kind());
            modes.push(if kind.horizontal() {
                Mode::RestrictedHorizontal
            } else {
                Mode::InternalVertical
            });
            boxes.active_boxes.push(ActiveReplayBox {
                target,
                ships_out,
                opening_brace_replay: construction.opening_brace_replay,
                body_opener_pending: construction.opening_brace_replay,
                depth: 1,
                kind,
                packing,
                leader_kind: None,
            });
            schedule_everybox(command, stores, kind.horizontal());
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BeginLeaderBox {
            construction,
            kind: leader_kind,
        } => {
            let kind = match construction.kind {
                ScannedBoxKind::HBox => ReplayBoxKind::HBox,
                ScannedBoxKind::VBox => ReplayBoxKind::VBox,
                ScannedBoxKind::VTop => ReplayBoxKind::VTop,
            };
            let packing = match construction.packing {
                ScannedPackingSpec::Natural => PackSpec::Natural,
                ScannedPackingSpec::Exactly(size) => PackSpec::Exactly(size),
                ScannedPackingSpec::Spread(size) => PackSpec::Spread(size),
            };
            stores.enter_group_with_kind(kind.group_kind());
            modes.push(if kind.horizontal() {
                Mode::RestrictedHorizontal
            } else {
                Mode::InternalVertical
            });
            boxes.active_boxes.push(ActiveReplayBox {
                target: None,
                ships_out: false,
                opening_brace_replay: construction.opening_brace_replay,
                body_opener_pending: construction.opening_brace_replay,
                depth: 1,
                kind,
                packing,
                leader_kind: Some(leader_kind),
            });
            schedule_everybox(command, stores, kind.horizontal());
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BeginSimpleGroup => {
            stores.enter_group_with_kind(GroupKind::Simple);
            boxes.recovery_simple_group_pending = false;
            boxes.recovery_simple_group_open = true;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::EndSimpleGroup => {
            stores
                .leave_group_with_kind(GroupKind::Simple)
                .map_err(|_| ExecError::MissingToken {
                    context: "simple recovery group",
                })?;
            boxes.recovery_simple_group_open = false;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BeginOutputRoutine => {
            stores.enter_group_with_kind(GroupKind::Output);
            modes.push(Mode::InternalVertical);
            boxes.output_routine_active = true;
            boxes.output_routine_opening_pending = true;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::OutputRoutineOpeningBrace => {
            boxes.output_routine_opening_pending = false;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::EndOutputRoutine => {
            if matches!(
                modes.current_mode(),
                Mode::Horizontal | Mode::RestrictedHorizontal
            ) {
                let _ = modes.pop()?;
            }
            let output_level = modes.pop()?;
            stores
                .leave_group_with_kind(GroupKind::Output)
                .map_err(|_| ExecError::MissingToken {
                    context: "output routine group",
                })?;
            boxes.output_routine_active = false;
            crate::output::resume_page_builder_after_output(
                stores,
                output_level.list().nodes().to_vec(),
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::ForcedOutputShipout => {
            // TeX82 §46 appends its end-job contribution trio and delegates
            // page selection to §1006's `build_page`; §1016 then packs the
            // chosen material into \\box255 for the default shipout.  The
            // helper has no input capability and returns only that typed box.
            let page = crate::output::take_final_default_output_page(stores)?;
            if let Some(receipt) = shipout_replay_box(page, stores)? {
                prepared_dvi_pages.push(receipt);
            }
            boxes.terminal_output_shipout_complete = true;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BeginOrdinaryGroup => {
            stores.enter_group_with_kind(GroupKind::Simple);
            boxes.ordinary_simple_group_depth += 1;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BeginSemiSimpleGroup => {
            stores.enter_group_with_kind(GroupKind::SemiSimple);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::EndSemiSimpleGroup => {
            let aftergroup = stores
                .leave_group_with_kind(GroupKind::SemiSimple)
                .map_err(|_| ExecError::MissingToken {
                    context: "semi simple group",
                })?;
            schedule_aftergroup(command, stores, aftergroup);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::ExtraRightBrace => {
            stores
                .world_mut()
                .write_text(PrintSink::TerminalAndLog, "\n! Too many }'s.\n");
            Ok(ReplayStep::Continue)
        }
        ScannedStep::ExtraRightBraceEndsSemiSimpleGroup => {
            stores.world_mut().write_text(
                PrintSink::TerminalAndLog,
                "\n! Extra }, or forgotten \\endgroup.\n",
            );
            let aftergroup = stores
                .leave_group_with_kind(GroupKind::SemiSimple)
                .map_err(|_| ExecError::MissingToken {
                    context: "semi simple group right-brace recovery",
                })?;
            schedule_aftergroup(command, stores, aftergroup);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::ExtraEndGroup => {
            stores
                .world_mut()
                .write_text(PrintSink::TerminalAndLog, "\n! Extra \\endgroup.\n");
            Ok(ReplayStep::Continue)
        }
        ScannedStep::EndOrdinaryGroup => {
            let aftergroup = stores
                .leave_group_with_kind(GroupKind::Simple)
                .map_err(|_| ExecError::MissingToken {
                    context: "ordinary simple group",
                })?;
            boxes.ordinary_simple_group_depth = boxes
                .ordinary_simple_group_depth
                .checked_sub(1)
                .expect("ordinary simple group depth is nonzero");
            schedule_aftergroup(command, stores, aftergroup);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::AlignmentRecovery { opens_simple_group } => {
            boxes.recovery_simple_group_pending = opens_simple_group;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::ReplayBoxOpeningBrace => {
            let box_state = boxes
                .active_boxes
                .last_mut()
                .ok_or(ExecError::MissingToken {
                    context: "box opening brace",
                })?;
            box_state.opening_brace_replay = false;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BoxBeginGroup => {
            let box_state = boxes
                .active_boxes
                .last_mut()
                .ok_or(ExecError::MissingToken {
                    context: "box group",
                })?;
            if box_state.body_opener_pending {
                // The first replayed brace is the box body's required opener;
                // its scope is represented by the VBox group entered above.
                box_state.body_opener_pending = false;
            } else {
                box_state.depth = box_state.depth.saturating_add(1);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BoxEndGroup { ships_out } => {
            let box_state = boxes
                .active_boxes
                .last_mut()
                .ok_or(ExecError::MissingToken {
                    context: "box group",
                })?;
            if box_state.depth > 1 {
                box_state.depth -= 1;
                return Ok(ReplayStep::Continue);
            }
            let box_state = boxes.active_boxes.pop().expect("active box was checked");
            let level = modes.pop()?;
            let children = stores.freeze_node_list(level.list().nodes());
            let node = if box_state.kind.horizontal() {
                Node::HList(crate::assignments::hpack_with_overfull_rule(
                    stores,
                    children,
                    box_state.packing,
                ))
            } else {
                Node::VList(match box_state.kind {
                    ReplayBoxKind::VBox => {
                        crate::packing_params::vpack(
                            stores,
                            children,
                            box_state.packing,
                            crate::packing_params::vpack_params(stores),
                        )
                        .node
                    }
                    ReplayBoxKind::VTop => {
                        crate::packing_params::vtop(
                            stores,
                            children,
                            box_state.packing,
                            crate::packing_params::vpack_params(stores),
                        )
                        .node
                    }
                    ReplayBoxKind::HBox => unreachable!("horizontal box was handled above"),
                })
            };
            let boxed = stores.freeze_node_list(std::slice::from_ref(&node));
            stores
                .leave_group_with_kind(box_state.kind.group_kind())
                .map_err(|_| ExecError::MissingToken {
                    context: "box group",
                })?;
            if let Some(kind) = box_state.leader_kind {
                let payload = payload_from_node(node).ok_or(ExecError::MissingToken {
                    context: "leader box payload",
                })?;
                boxes.pending_leader = Some((kind, payload));
            } else if ships_out {
                debug_assert!(box_state.ships_out);
                if let Some(receipt) = shipout_replay_box(node, stores)? {
                    prepared_dvi_pages.push(receipt);
                }
            } else if let Some(target) = box_state.target {
                if target.global {
                    stores.set_box_reg_global(target.index, boxed);
                } else {
                    stores.set_box_reg(target.index, boxed);
                }
            } else {
                modes.current_list_mut().push(node);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BeginAlignment { vertical } => {
            if let Some(outer) = active_alignment.take() {
                command
                    .apply_alignment_request(AlignmentRequest::Suspend(outer.identity))
                    .map_err(|_| ExecError::MissingToken {
                        context: "nested alignment suspension",
                    })?;
                boxes.suspended_alignments.push(outer);
            }
            let identity = AlignmentIdentity::new(*next_alignment_identity);
            *next_alignment_identity = next_alignment_identity.wrapping_add(1);
            command
                .apply_alignment_request(AlignmentRequest::Begin(identity))
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment lifecycle",
                })?;
            *active_alignment = Some(ActiveReplayAlignment {
                identity,
                kind: if vertical {
                    AlignmentKind::VAlign
                } else {
                    AlignmentKind::HAlign
                },
                columns: Vec::new(),
                repeat_start: None,
                column: 0,
                preamble_opening_pending: true,
                preamble_opening_replay_pending: false,
                preamble_start_pending: false,
                cell_opening_pending: false,
                next_cell_opening_pending: false,
                align_peek_pending: false,
                align_peek_after_noalign: false,
                noalign_depth: None,
                captured_rows: Vec::new(),
                tabskip: stores.glue_param(GlueParam::TAB_SKIP),
                cell_span: 1,
                row_open: false,
                cell_open: false,
            });
            modes.push(replay_alignment_mode(if vertical {
                AlignmentKind::VAlign
            } else {
                AlignmentKind::HAlign
            }));
            Ok(ReplayStep::Continue)
        }
        ScannedStep::AlignmentPreambleOpening { alignment } => {
            command
                .apply_alignment_request(AlignmentRequest::Preamble(alignment))
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment preamble lifecycle",
                })?;
            if let Some(active) = active_alignment.as_mut()
                && active.identity == alignment
            {
                active.preamble_opening_pending = false;
                active.preamble_opening_replay_pending = true;
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::AlignmentPreambleOpeningReplay { alignment } => {
            if let Some(active) = active_alignment.as_mut()
                && active.identity == alignment
            {
                active.preamble_opening_replay_pending = false;
                active.preamble_start_pending = true;
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::AlignmentPreambleStart { alignment } => {
            let preamble = command
                .take_completed_alignment_preamble(alignment)
                .map_err(|_| ExecError::MissingToken {
                    context: "completed alignment preamble",
                })?;
            if preamble.columns.is_empty() {
                return Err(ExecError::MissingToken {
                    context: "first alignment preamble column",
                });
            }
            // `init_row` reaches `align_peek` before `init_col` selects the
            // first cell. Keep the first pair validated here, but defer
            // `BeginCell` until that lookahead has classified the next token.
            // In particular, a recovered preamble may be followed directly
            // by `}`, which `align_peek` passes to `fin_align`.
            if let Some(active) = active_alignment.as_mut()
                && active.identity == alignment
            {
                active.columns = preamble.columns;
                active.repeat_start = preamble.repeat_start;
                active.column = 0;
                active.preamble_start_pending = false;
                // TeX82 §777 enters `align_peek` after `init_row`, before
                // `init_col` gets a cell opener.  This distinction matters
                // when §23 recovered a runaway preamble with frozen `\\cr`:
                // the following right brace belongs to `fin_align`, not to a
                // u-template lookahead that must be backed up.  Keep this
                // post-preamble probe command-owned so ordinary first-cell
                // input still reaches the existing typed init-col path.
                active.align_peek_pending = true;
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BeginNoAlign { alignment } => {
            let active = active_alignment
                .as_mut()
                .filter(|active| active.identity == alignment)
                .ok_or(ExecError::MissingToken {
                    context: "active replay alignment",
                })?;
            active.align_peek_pending = false;
            active.noalign_depth = Some(1);
            stores.enter_group_with_kind(GroupKind::NoAlign);
            if matches!(
                modes.current_mode(),
                Mode::Horizontal | Mode::RestrictedHorizontal
            ) {
                let _ = modes.pop()?;
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::AlignmentPeekCell { alignment, omit } => {
            let active = active_alignment
                .as_mut()
                .filter(|active| active.identity == alignment)
                .ok_or(ExecError::MissingToken {
                    context: "active replay alignment",
                })?;
            let templates =
                active
                    .columns
                    .get(active.column)
                    .copied()
                    .ok_or(ExecError::MissingToken {
                        context: "next alignment preamble column",
                    })?;
            command
                .apply_alignment_request(AlignmentRequest::BeginCell {
                    alignment,
                    templates,
                })
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment next-row lifecycle",
                })?;
            begin_replay_alignment_cell(active, modes)?;
            active.align_peek_pending = false;
            if omit {
                command
                    .apply_alignment_request(AlignmentRequest::PrepareCellLookahead(alignment))
                    .map_err(|_| ExecError::MissingToken {
                        context: "alignment omit lookahead lifecycle",
                    })?;
                command
                    .apply_alignment_request(AlignmentRequest::InstallOmitCellTemplate(alignment))
                    .map_err(|_| ExecError::MissingToken {
                        context: "alignment omit-cell lifecycle",
                    })?;
            } else {
                // TeX82 §37 now calls `init_col`, which immediately pushes
                // the selected u-template above the command backed up by
                // `align_peek`. A second lookahead would re-deliver that
                // command before the template is installed.
                command
                    .apply_alignment_request(AlignmentRequest::InstallCellTemplate(alignment))
                    .map_err(|_| ExecError::MissingToken {
                        context: "alignment next-row cell-template lifecycle",
                    })?;
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::NoAlignBeginGroup { alignment } => {
            let active = active_alignment
                .as_mut()
                .filter(|active| active.identity == alignment)
                .ok_or(ExecError::MissingToken {
                    context: "active replay alignment",
                })?;
            let depth = active.noalign_depth.ok_or(ExecError::MissingToken {
                context: "noalign group",
            })?;
            active.noalign_depth = Some(depth.saturating_add(1));
            Ok(ReplayStep::Continue)
        }
        ScannedStep::NoAlignEndGroup { alignment } => {
            let active = active_alignment
                .as_mut()
                .filter(|active| active.identity == alignment)
                .ok_or(ExecError::MissingToken {
                    context: "active replay alignment",
                })?;
            let depth = active.noalign_depth.ok_or(ExecError::MissingToken {
                context: "noalign group",
            })?;
            if depth > 1 {
                active.noalign_depth = Some(depth - 1);
                return Ok(ReplayStep::Continue);
            }
            active.noalign_depth = None;
            active.align_peek_pending = true;
            active.align_peek_after_noalign = true;
            stores
                .leave_group_with_kind(GroupKind::NoAlign)
                .map_err(|_| ExecError::MissingToken {
                    context: "noalign group",
                })?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::AlignmentCellOpening { alignment, opening } => {
            command
                .apply_alignment_request(match opening {
                    AlignmentCellOpening::Template => {
                        AlignmentRequest::InstallCellTemplate(alignment)
                    }
                    AlignmentCellOpening::Omit => {
                        AlignmentRequest::InstallOmitCellTemplate(alignment)
                    }
                })
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment cell-template lifecycle",
                })?;
            if let Some(active) = active_alignment.as_mut()
                && active.identity == alignment
            {
                active.cell_opening_pending = false;
                active.next_cell_opening_pending = false;
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::AlignmentCellFinish { alignment } => {
            let finished = command
                .apply_alignment_request(AlignmentRequest::FinishCell(alignment))
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment end-v lifecycle",
                })?;
            let AlignmentRequestResult::FinishedCell(finished) = finished else {
                unreachable!("FinishCell returns its saved delimiter");
            };
            begin_next_replay_alignment_cell(
                alignment,
                finished.delimiter,
                command,
                active_alignment,
                modes,
                stores,
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::AlignmentFinish { alignment } => {
            if active_alignment.as_ref().map(|active| active.identity) != Some(alignment) {
                return Err(ExecError::MissingToken {
                    context: "active replay alignment",
                });
            }
            let active = active_alignment
                .as_mut()
                .expect("active replay alignment was checked");
            finish_replay_alignment(active, modes, stores)?;
            command
                .apply_alignment_request(AlignmentRequest::Finish(alignment))
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment finish lifecycle",
                })?;
            *active_alignment = None;
            if let Some(outer) = boxes.suspended_alignments.pop() {
                command
                    .apply_alignment_request(AlignmentRequest::Resume(outer.identity))
                    .map_err(|_| ExecError::MissingToken {
                        context: "nested alignment resumption",
                    })?;
                *active_alignment = Some(outer);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Paragraph => {
            if matches!(
                modes.current_mode(),
                Mode::Vertical | Mode::InternalVertical
            ) {
                crate::assignments::normal_paragraph(modes, stores);
                crate::vertical::build_page_if_outer_vertical(modes, stores)?;
            } else {
                let mut memo = crate::paragraph_memo::NoParagraphMemoConsumer;
                crate::assignments::end_paragraph_with_consumer(modes, stores, &mut memo)?;
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::MathShift { .. } => {
            unreachable!("canonical math shifts are applied by CanonicalMainControl")
        }
        ScannedStep::ParagraphStart => {
            start_canonical_paragraph(command, modes, stores, true)?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Character { ch, cat, origin } => {
            if matches!(modes.current_mode(), Mode::Math | Mode::DisplayMath) {
                if !matches!(cat, Catcode::Space) {
                    let code = stores.mathcode(ch);
                    if code != 0x8000 {
                        append_canonical_math_char(modes.current_list_mut(), stores, code, origin)?;
                    }
                }
                return Ok(ReplayStep::Continue);
            }
            match cat {
                Catcode::Space => {
                    if matches!(
                        modes.current_mode(),
                        Mode::Vertical | Mode::InternalVertical
                    ) {
                        // Alignment-cell delivery reaches this branch without
                        // the outer `starts_paragraph` probe, but TeX82 still
                        // enters horizontal mode before ordinary text.
                        start_canonical_paragraph(command, modes, stores, true)?;
                    }
                    if matches!(
                        modes.current_mode(),
                        Mode::Horizontal | Mode::RestrictedHorizontal
                    ) {
                        crate::assignments::append_canonical_space(modes, stores)?;
                    }
                }
                Catcode::Letter | Catcode::Other => {
                    if matches!(
                        modes.current_mode(),
                        Mode::Vertical | Mode::InternalVertical
                    ) {
                        start_canonical_paragraph(command, modes, stores, true)?;
                    }
                    crate::assignments::append_canonical_character(modes, stores, ch, origin)?;
                }
                _ => unreachable!("canonical character scan restricts catcodes"),
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Accent(accent) => {
            if matches!(
                modes.current_mode(),
                Mode::Vertical | Mode::InternalVertical
            ) {
                start_canonical_paragraph(command, modes, stores, true)?;
            }
            apply_scanned_accent(modes, stores, accent)
        }
        // `step_once` consumes the three command-owned episodes while its
        // aggregate snapshot is live. Observed replay is not an alternate
        // production execution path, so reaching this arm is an invariant.
        ScannedStep::Discretionary(_) => {
            unreachable!("discretionary is applied by CanonicalMainControl")
        }
    }
}

/// Moves TeX's FIFO group-exit payload into a command-owned replay level only
/// after `Universe` has restored the enclosing group state.
fn schedule_aftergroup(command: &mut CommandState, stores: &mut Universe, tokens: Vec<Token>) {
    if tokens.is_empty() {
        return;
    }
    let traced: Vec<_> = tokens
        .into_iter()
        .map(|token| {
            let origin = stores.inserted_origin(
                tex_state::provenance::InsertedOriginKind::AfterGroup,
                token,
                tex_state::token::OriginId::UNKNOWN,
            );
            tex_state::token::TracedTokenWord::pack(token, origin)
        })
        .collect();
    let _ = command.push_aftergroup(stores.finish_traced_token_list(&traced));
}

/// Releases the single pending after-assignment token only after the typed
/// assignment has committed. Its replay remains entirely command-owned.
fn schedule_afterassignment(command: &mut CommandState, stores: &mut Universe) {
    let Some(token) = stores.take_afterassignment() else {
        return;
    };
    let origin = stores.inserted_origin(
        tex_state::provenance::InsertedOriginKind::AfterAssignment,
        token,
        tex_state::token::OriginId::UNKNOWN,
    );
    let traced = [tex_state::token::TracedTokenWord::pack(token, origin)];
    let _ = command.push_afterassignment(stores.finish_traced_token_list(&traced));
}

fn assignment_global(explicit_global: bool, stores: &Universe) -> bool {
    match stores.int_param(IntParam::GLOBAL_DEFS).cmp(&0) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => explicit_global,
    }
}

fn checked_character_code(value: i32, context: &'static str) -> Result<u32, ExecError> {
    u32::try_from(value)
        .ok()
        .and_then(char::from_u32)
        .map(|character| character as u32)
        .ok_or(ExecError::InvalidCode { context, value })
}

fn apply_arithmetic(
    primitive: UnexpandablePrimitive,
    target: ArithmeticTarget,
    operand: ArithmeticOperand,
    global: bool,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    match (target, operand) {
        (ArithmeticTarget::IntegerRegister(index), ArithmeticOperand::Integer(rhs)) => {
            let value = arithmetic_integer(primitive, stores.count(index), rhs)?;
            if global {
                stores.set_count_global(index, value);
            } else {
                stores.set_count(index, value);
            }
        }
        (ArithmeticTarget::IntegerParameter(index), ArithmeticOperand::Integer(rhs)) => {
            let parameter = IntParam::new(index);
            let value = arithmetic_integer(primitive, stores.int_param(parameter), rhs)?;
            if global {
                stores.set_int_param_global(parameter, value);
            } else {
                stores.set_int_param(parameter, value);
            }
        }
        (ArithmeticTarget::DimensionRegister(index), operand) => {
            let value = arithmetic_dimension(primitive, stores.dimen(index), operand)?;
            if global {
                stores.set_dimen_global(index, value);
            } else {
                stores.set_dimen(index, value);
            }
        }
        (ArithmeticTarget::DimensionParameter(index), operand) => {
            let parameter = DimenParam::new(index);
            let value = arithmetic_dimension(primitive, stores.dimen_param(parameter), operand)?;
            if global {
                stores.set_dimen_param_global(parameter, value);
            } else {
                stores.set_dimen_param(parameter, value);
            }
        }
        (ArithmeticTarget::GlueRegister { index, mu }, operand) => {
            let old = stores.glue(if mu {
                stores.muskip(index)
            } else {
                stores.skip(index)
            });
            let value = stores.intern_glue(arithmetic_glue(primitive, old, operand)?);
            if mu {
                if global {
                    stores.set_muskip_global(index, value);
                } else {
                    stores.set_muskip(index, value);
                }
            } else if global {
                stores.set_skip_global(index, value);
            } else {
                stores.set_skip(index, value);
            }
        }
        (ArithmeticTarget::GlueParameter { index, .. }, operand) => {
            let parameter = GlueParam::new(index);
            let old = stores.glue(stores.glue_param(parameter));
            let value = stores.intern_glue(arithmetic_glue(primitive, old, operand)?);
            if global {
                stores.set_glue_param_global(parameter, value);
            } else {
                stores.set_glue_param(parameter, value);
            }
        }
        _ => return Err(ExecError::UnsupportedAssignmentTarget),
    }
    Ok(())
}

fn arithmetic_integer(
    primitive: UnexpandablePrimitive,
    old: i32,
    rhs: i32,
) -> Result<i32, ExecError> {
    match primitive {
        UnexpandablePrimitive::Advance => old.checked_add(rhs),
        UnexpandablePrimitive::Multiply => old.checked_mul(rhs),
        UnexpandablePrimitive::Divide => old.checked_div(rhs),
        _ => None,
    }
    .ok_or(ExecError::ArithmeticOverflow)
}

fn arithmetic_dimension(
    primitive: UnexpandablePrimitive,
    old: Scaled,
    operand: ArithmeticOperand,
) -> Result<Scaled, ExecError> {
    match (primitive, operand) {
        (UnexpandablePrimitive::Advance, ArithmeticOperand::Dimension(rhs)) => old.checked_add(rhs),
        (UnexpandablePrimitive::Multiply, ArithmeticOperand::Integer(rhs)) => {
            old.raw().checked_mul(rhs).map(Scaled::from_raw)
        }
        (UnexpandablePrimitive::Divide, ArithmeticOperand::Integer(rhs)) => {
            old.raw().checked_div(rhs).map(Scaled::from_raw)
        }
        _ => None,
    }
    .ok_or(ExecError::ArithmeticOverflow)
}

fn arithmetic_glue(
    primitive: UnexpandablePrimitive,
    old: GlueSpec,
    operand: ArithmeticOperand,
) -> Result<GlueSpec, ExecError> {
    match (primitive, operand) {
        (UnexpandablePrimitive::Advance, ArithmeticOperand::Glue(rhs)) => Ok(GlueSpec {
            width: old
                .width
                .checked_add(rhs.width)
                .ok_or(ExecError::ArithmeticOverflow)?,
            stretch: glue_component_add(
                old.stretch,
                old.stretch_order,
                rhs.stretch,
                rhs.stretch_order,
            )?
            .0,
            stretch_order: glue_component_add(
                old.stretch,
                old.stretch_order,
                rhs.stretch,
                rhs.stretch_order,
            )?
            .1,
            shrink: glue_component_add(old.shrink, old.shrink_order, rhs.shrink, rhs.shrink_order)?
                .0,
            shrink_order: glue_component_add(
                old.shrink,
                old.shrink_order,
                rhs.shrink,
                rhs.shrink_order,
            )?
            .1,
        }),
        (UnexpandablePrimitive::Multiply, ArithmeticOperand::Integer(rhs)) => {
            glue_scale(old, rhs, false)
        }
        (UnexpandablePrimitive::Divide, ArithmeticOperand::Integer(rhs)) => {
            glue_scale(old, rhs, true)
        }
        _ => Err(ExecError::UnsupportedAssignmentTarget),
    }
}

fn glue_component_add(
    left: Scaled,
    left_order: Order,
    right: Scaled,
    right_order: Order,
) -> Result<(Scaled, Order), ExecError> {
    if left_order == right_order {
        return Ok((
            left.checked_add(right)
                .ok_or(ExecError::ArithmeticOverflow)?,
            left_order,
        ));
    }
    Ok(if left_order > right_order {
        (left, left_order)
    } else {
        (right, right_order)
    })
}

fn glue_scale(spec: GlueSpec, factor: i32, divide: bool) -> Result<GlueSpec, ExecError> {
    let scale = |value: Scaled| {
        let raw = if divide {
            value.raw().checked_div(factor)
        } else {
            value.raw().checked_mul(factor)
        };
        raw.map(Scaled::from_raw)
            .ok_or(ExecError::ArithmeticOverflow)
    };
    Ok(GlueSpec {
        width: scale(spec.width)?,
        stretch: scale(spec.stretch)?,
        stretch_order: spec.stretch_order,
        shrink: scale(spec.shrink)?,
        shrink_order: spec.shrink_order,
    })
}

fn apply_scanned_accent(
    modes: &mut ModeNest,
    stores: &mut Universe,
    scanned: ScannedAccent,
) -> Result<ReplayStep, ExecError> {
    crate::assignments::flush_pending_hchars(modes, stores)?;
    let accent = u8::try_from(scanned.accent).map_err(|_| ExecError::InvalidCode {
        context: "\\accent",
        value: scanned.accent,
    })?;
    let accent_font = stores.current_font();
    let Some(accent_metrics) = stores.font_char_metrics(accent_font, accent) else {
        report_missing_character(stores, accent_font, char::from(accent));
        return Ok(ReplayStep::Continue);
    };
    let Some(base) = scanned.base else {
        modes.current_list_mut().push(Node::Char {
            font: accent_font,
            ch: char::from(accent),
            origin: scanned.accent_provenance.primary,
        });
        return Ok(ReplayStep::Continue);
    };
    let base_font = stores.current_font();
    let Some(base_metrics) = stores.font_char_metrics(base_font, base.character) else {
        report_missing_character(stores, base_font, char::from(base.character));
        modes.current_list_mut().push(Node::Char {
            font: accent_font,
            ch: char::from(accent),
            origin: scanned.accent_provenance.primary,
        });
        modes.current_list_mut().set_space_factor(1000);
        return Ok(ReplayStep::Continue);
    };
    let accent_x_height = stores.font_parameter(accent_font, 5);
    let accent_slant = stores.font_parameter(accent_font, 1);
    let base_slant = stores.font_parameter(base_font, 1);
    let delta = tex_state::scaled::text_accent_delta(
        base_metrics.width,
        accent_metrics.width,
        base_metrics.height,
        base_slant,
        accent_x_height,
        accent_slant,
    );
    modes.current_list_mut().push(Node::Kern {
        amount: delta,
        kind: KernKind::Accent,
    });
    let accent_node = Node::Char {
        font: accent_font,
        ch: char::from(accent),
        origin: scanned.accent_provenance.primary,
    };
    if base_metrics.height == accent_x_height {
        modes.current_list_mut().push(accent_node);
    } else {
        let children = stores.freeze_node_list(&[accent_node]);
        let mut boxed =
            crate::assignments::hpack_with_overfull_rule(stores, children, PackSpec::Natural);
        boxed.shift = accent_x_height
            .checked_sub(base_metrics.height)
            .ok_or(ExecError::ArithmeticOverflow)?;
        modes.current_list_mut().push(Node::HList(boxed));
    }
    modes.current_list_mut().push(Node::Kern {
        amount: Scaled::from_raw(-accent_metrics.width.raw() - delta.raw()),
        kind: KernKind::Accent,
    });
    modes.current_list_mut().push(Node::Char {
        font: base_font,
        ch: char::from(base.character),
        origin: base.provenance.primary,
    });
    modes.current_list_mut().set_space_factor(1000);
    Ok(ReplayStep::Continue)
}

fn canonical_font_path(name: &str) -> PathBuf {
    let mut path = PathBuf::from(name);
    if !name.starts_with("opentype:") && path.extension().is_none() {
        path.set_extension("tfm");
    }
    path
}

fn load_canonical_font(
    request: &FontLoadRequest,
    resource: FontResource,
) -> Result<tex_fonts::LoadedFont, ExecError> {
    let display_name = request.name.strip_suffix(".tfm").unwrap_or(&request.name);
    let from_tfm = |metrics: tex_state::world::FileContent,
                    opentype: Option<tex_fonts::OpenTypeProgramSelection>,
                    mapped: Option<(
        tex_fonts::OpenTypeProgramSelection,
        tex_fonts::LegacyEncodingMap,
    )>|
     -> Result<tex_fonts::LoadedFont, ExecError> {
        let tfm = tex_fonts::TfmFont::parse_with_size(metrics.bytes(), request.size)?;
        let parameters = tfm
            .parameters
            .values
            .iter()
            .map(|parameter| parameter.value)
            .collect();
        let mut font = tex_fonts::LoadedFont::new(
            display_name,
            metrics.path().to_owned(),
            metrics.hash().bytes(),
            tfm.header.checksum,
            tfm.header.design_size,
            tfm.font_size,
            parameters,
            tfm.font_metrics(),
        );
        if let Some((selection, encoding_map)) = mapped {
            font = font.with_mapped_opentype(selection, encoding_map);
        } else if let Some(selection) = opentype {
            font = font.with_opentype(selection);
        }
        Ok(font)
    };
    match resource {
        FontResource::Unavailable => unreachable!("unavailable resources recover before parsing"),
        FontResource::Tfm { metrics, opentype } => from_tfm(metrics, opentype, None),
        FontResource::MappedTfm {
            metrics,
            opentype,
            encoding_map,
        } => from_tfm(metrics, None, Some((opentype, encoding_map))),
        FontResource::ClassicTfmFallback { metrics } => {
            Ok(from_tfm(metrics, None, None)?.with_classic_mapping_fallback())
        }
        FontResource::OpenType(selection) => {
            let design_size = Scaled::from_raw(10 * Scaled::UNITY);
            let size = tex_state::scaled::tfm_font_size(design_size, request.size)
                .map_err(|_| ExecError::ArithmeticOverflow)?;
            Ok(tex_fonts::LoadedFont::new_opentype(
                request
                    .name
                    .strip_prefix("opentype:")
                    .unwrap_or(&request.name),
                request
                    .name
                    .strip_prefix("opentype:")
                    .unwrap_or(&request.name),
                design_size,
                size,
                selection,
            ))
        }
    }
}

fn report_missing_character(stores: &mut Universe, font: tex_state::ids::FontId, ch: char) {
    if stores.int_param(IntParam::new(36)) <= 0 {
        return;
    }
    let text = format!(
        "Missing character: There is no {} in font {}!\\n",
        ch.escape_default(),
        stores.font_name(font)
    );
    stores
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, &text);
}

/// TeX82 §1095 `new_graf`: command control has already made any required
/// backup, then this typed transition installs the indent and schedules the
/// immutable `\everypar` payload through the same command state.
fn start_canonical_paragraph(
    command: &mut CommandState,
    modes: &mut ModeNest,
    stores: &mut Universe,
    indent: bool,
) -> Result<(), ExecError> {
    crate::assignments::start_canonical_paragraph(modes, stores, indent)?;
    let everypar = stores.tok_param(TokParam::EVERY_PAR);
    if !stores.tokens(everypar).is_empty() {
        let origin = stores.bootstrap_origin();
        let traced: Vec<_> = stores
            .tokens(everypar)
            .iter()
            .copied()
            .map(|token| tex_state::token::TracedTokenWord::pack(token, origin))
            .collect();
        command.push_everypar(stores.finish_traced_token_list(&traced));
    }
    Ok(())
}

/// Schedules an every-box list after replay has entered its scoped group and
/// mode.  The immutable traced list is owned by command state, preserving the
/// ordinary macro, recovery, retirement, and provenance path for hook tokens.
fn schedule_everybox(command: &mut CommandState, stores: &mut Universe, horizontal: bool) {
    let parameter = if horizontal {
        TokParam::EVERY_HBOX
    } else {
        TokParam::EVERY_VBOX
    };
    let tokens = stores.tok_param(parameter);
    if stores.tokens(tokens).is_empty() {
        return;
    }
    let tokens: Vec<_> = stores.tokens(tokens).to_vec();
    let traced: Vec<_> = tokens
        .into_iter()
        .map(|token| {
            let origin = stores.inserted_origin(
                tex_state::provenance::InsertedOriginKind::TokenListReplay(if horizontal {
                    tex_state::TokenListReplayKind::EveryHBox
                } else {
                    tex_state::TokenListReplayKind::EveryVBox
                }),
                token,
                stores.bootstrap_origin(),
            );
            tex_state::token::TracedTokenWord::pack(token, origin)
        })
        .collect();
    command.push_everybox(stores.finish_traced_token_list(&traced), horizontal);
}

fn schedule_everymath(command: &mut CommandState, stores: &mut Universe, display: bool) {
    let parameter = if display {
        TokParam::EVERY_DISPLAY
    } else {
        TokParam::EVERY_MATH
    };
    let tokens = stores.tok_param(parameter);
    if stores.tokens(tokens).is_empty() {
        return;
    }
    let origin = stores.bootstrap_origin();
    let traced: Vec<_> = stores
        .tokens(tokens)
        .iter()
        .copied()
        .map(|token| tex_state::token::TracedTokenWord::pack(token, origin))
        .collect();
    command.push_everymath(stores.finish_traced_token_list(&traced), display);
}

fn command_error(error: CommandError) -> ExecError {
    match error {
        CommandError::MissingInput(name) => ExecError::MissingCanonicalInput { name },
        CommandError::PdfNavigation(message) => ExecError::PdfNavigation(message),
        _ => ExecError::MissingToken {
            context: "command processor",
        },
    }
}

#[cfg(test)]
#[path = "command_replay/tests.rs"]
mod tests;
