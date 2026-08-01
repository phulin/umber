//! Production canonical main-control driver.
//!
//! This intentionally owns only typed execution mutations. Raw delivery,
//! expansion, macro calls, input nesting, and operand collection remain in
//! `tex-command`; no `InputStack` is accepted here.

use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::Arc;
use tex_command::{
    AlignmentCellDelimiter, AlignmentCellOpening, AlignmentCellTemplates, AlignmentDelivery,
    AlignmentIdentity, AlignmentRequest, AlignmentRequestResult, CanonicalMathRequest,
    CommandError, CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandProfile,
    CommandRuntime, CommandState, CommandStateSnapshot, FatalError, FontLoadRequest, FontResource,
    HyphenationDataKind, ImmediateExtension, InputStreamRequest, MathDelimiterBoundary,
    MathDelimiterBoundaryKind, MathFieldBody, MathLimitKind, MathScriptKind, MathStyleKind,
    MathTextFieldKind, PdfAnnotationRequest, PdfColorStackActionRequest, PdfDestinationRequest,
    PdfDocumentFragmentRequest, PdfFormRequest, PdfGraphicsRequest, PdfImageRequest,
    PdfImageResource, PdfNavigationRequest, PdfObjectRequest, PdfOutlineRequest,
    PdfReferenceObjectRequest, PdfStartLinkRequest, RestrictedIntegerClass, ScannedAccent,
    ScannedAccentBase, ScannedBoxConstruction, ScannedBoxKind, ScannedBoxShift,
    ScannedBoxShiftPayload, ScannedDiscretionaryOpening, ScannedDisplayDiagnostic,
    ScannedInsertConstruction, ScannedLeaderPayload, ScannedMathMuMaterial, ScannedPackingSpec,
    ScannedVSplit, SourceRegistration, SourceRegistrationError,
};
use tex_command::{
    CommandObservation, CommandObserver, EffectRecord, GeometryRecord, MutationRecord,
    ObservedToken, ParameterClass, TokenListRecord, canonical_names::glue_order_name,
    parameter_mutation_key_for_dialect,
};
use tex_state::GeometryObservation;
use tex_state::code_tables::{DelCode, LcCode, MathCode, SfCode, UcCode};
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::{FontId, GlueId, NodeListId, TokenListId};
use tex_state::interner::Symbol;
use tex_state::macro_store::{MacroDefinitionProvenance, MacroMeaning};
use tex_state::math::{
    FractionThickness, LimitType, MathChar, MathChoice, MathField, MathFontSize, MathFraction,
    MathNoad, MathStyle, NoadClass, NoadKind,
};
use tex_state::meaning::{Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::node::{DiscKind, GlueKind, KernKind, LeaderPayload, Node, Whatsit};
use tex_state::page::{PageDimension, PageInteger};
use tex_state::scaled::Scaled;
use tex_state::token::TracedTokenWord;
use tex_state::token::{Catcode, Token};
use tex_state::{
    ExpansionState, GroupKind, InputOpenState, InputReadState, ParagraphShapeLine,
    PenaltyArrayKind, PrintSink, StreamSlot, TracedTokenList, Universe,
};
use tex_typeset::PackSpec;

use crate::mode::{AlignColumn, AlignState, AlignmentKind, AlignmentPackSpec};
use crate::vertical::is_outer_vertical;
use crate::{ExecError, Mode, ModeNest};

type PreparedDviPages = Arc<Vec<crate::dispatch::PreparedDviPage>>;

fn push_prepared_dvi_page(pages: &mut PreparedDviPages, page: crate::dispatch::PreparedDviPage) {
    Arc::make_mut(pages).push(page);
}

fn take_prepared_dvi_pages(pages: &mut PreparedDviPages) -> Vec<crate::dispatch::PreparedDviPage> {
    Arc::try_unwrap(std::mem::take(pages)).unwrap_or_else(|shared| shared.as_ref().clone())
}

/// Production command main control with command-owned source consumption.
#[derive(Debug, Default)]
pub struct CanonicalMainControl {
    command: CommandState,
    runtime: CommandRuntime,
    fuel: tex_command::CommandFuelLedger,
    capabilities: CommandHostCapabilities,
    modes: ModeNest,
    next_alignment_identity: u64,
    active_alignment: Option<ActiveReplayAlignment>,
    boxes: ReplayBoxes,
    active_discretionaries: Vec<ActiveDiscretionary>,
    /// True while `main_control` is parked at TeX82 §1034's
    /// `main_loop_lookahead` rather than at §1030's `big_switch`.
    ///
    /// TeX's inner character loop appends a character and then fetches the
    /// next command from §1038's lookahead, which starts with a bare
    /// `get_next`; only `big_switch` uses `get_x_token`. Umber executes one
    /// command per `step_once`, so the label it would have jumped to has to
    /// be carried across steps explicitly.
    main_loop_active: bool,
    /// The last mode printed by TeX82 §1030's `show_cur_cmd_chr`.
    ///
    /// Zero is not a TeX mode, so `None` is WEB's initial `shown_mode=0`.
    /// This is diagnostic runtime state: it survives ordinary command steps
    /// but participates in an atomic step rollback.
    shown_mode: Option<Mode>,
    /// False until TeX82 §1030's `main_control` prologue has run.
    ///
    /// `main_control` is entered once per job and opens with
    /// `if every_job<>null then begin_token_list(every_job,every_job_text)`,
    /// before `big_switch` fetches anything. Umber executes one command per
    /// step, so the prologue is carried as a one-shot on the first step rather
    /// than by a distinct entry call every driver would have to remember.
    main_control_entered: bool,
    /// tex.web's `init`/`tini` compile-time split as a session flag.
    ///
    /// tex.web builds INITEX and production TeX from the same source with
    /// `init`-guarded code removed from the latter, so §1252's `\patterns`
    /// and §1335's `\dump` have entirely different behavior in the two
    /// binaries. Umber has one binary, so the distinction is the session's:
    /// [`CanonicalMainControl::tex82_initex`] builds an INITEX session and
    /// every other constructor a production one.
    initex: bool,
    /// Set when this session was started from a dumped format, so §61/§536's
    /// banner can name it (`(preloaded format=…)`) the way a real `-fmt` run
    /// does. `None` leaves the banner to `initex` above. Framing-only: no
    /// execution decision reads it.
    preloaded_format: Option<crate::job::PreloadedFormat>,
    /// Engine identity used only for §61/§536 startup framing.
    ///
    /// Most jobs use the command profile's dialect. Reference-backed jobs may
    /// deliberately exercise an older semantic profile in a newer engine
    /// binary, so backend banner identity is an independent typed fact.
    engine_binary: Option<crate::job::EngineBinaryIdentity>,
    /// True after TeX82 §1335 has successfully completed an INITEX `\dump`.
    ///
    /// This is a committed termination receipt for the host boundary; it is
    /// not format serialization itself.
    dumped_format: Option<crate::job::FormatDumpReceipt>,
    /// TeX82 §331's `**` line, retained for §313's pseudoprint of it.
    ///
    /// §331 opens the base terminal level over the line naming the job's root
    /// file and never closes it, so the line stays pseudoprintable in
    /// `buffer` for the whole run -- §360's `*` prompt reads over it, and a
    /// failed read (§71) leaves it untouched. Umber retires the base level as
    /// soon as `scan_startup_file_name` has consumed it, so the line has to be
    /// kept here for [`crate::job::prompt_for_more_input`] to render.
    /// Empty when no startup line was scanned, which is §313's own display for
    /// a base level whose consumed and pending text are both empty.
    startup_terminal_line: String,
    /// Observations produced by `fire_pending_page_output` after the current
    /// step's own records. Drained by every step, observed or not.
    page_output_observations: Vec<CommandObservation>,
    /// The commit buffer for the operation in flight, occupied exactly while
    /// an observed operation is running.
    ///
    /// It is engine state rather than a parameter because a single operation
    /// runs more than one `CommandProcessor` episode: a host-applied step
    /// (`docs/tex_command_core.md` §33.5) runs nested math-field,
    /// math-script, and `\mathchoice` episodes of its own while it executes.
    /// Holding the slot here lets [`command_processor`] be the only place an
    /// episode is constructed, so every episode of one operation is observed
    /// or none is. Observation is an instrumentation boundary, not an
    /// alternate execution mode.
    operation_observations: ObservationSlot,
    completed_replay_episode: Option<tex_command::CommandReplayEpisode>,
    /// Detached DVI receipts whose artifact commits have survived an entire
    /// canonical aggregate operation. This is replay state so rollback drops
    /// it with the corresponding World artifact/effect roots.
    prepared_dvi_pages: PreparedDviPages,
    /// Named safe boundaries committed by the last aggregate operation.  The
    /// host drains these only after `advance` has committed, so a resource
    /// suspension never leaks a checkpoint from its rolled-back operation.
    completed_boundaries: Vec<crate::EngineBoundary>,
    /// TeX82 §76's `history=fatal_error_stop`, carrying §93/§94/§95's payload.
    ///
    /// `succumb` ends the job through §81's `jump_out`, which a library engine
    /// cannot spell as leaving the process. This latch is the canonical
    /// equivalent: once it is set the session is terminal, every further
    /// operation reports [`MainControlStep::End`] without delivering a
    /// command, and the host reads the cause from [`Self::fatal_error`].
    fatal: Option<FatalError>,
    /// tex.web's job-framing state: see [`crate::job`] and
    /// `docs/job_framing.md`.
    job: crate::job::JobFraming,
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
    kind: ReplayBoxKind,
    group_kind: GroupKind,
    packing: PackSpec,
    leader_kind: Option<GlueKind>,
    /// TeX82 §1073's `shift_amount`, already sign-adjusted at scan time, for
    /// a box construction reached through `\raise`/`\lower`/`\moveleft`/
    /// `\moveright`. Applied once the body is packaged, immediately before
    /// the ordinary (non-register, non-shipout, non-leader) append in
    /// `BoxEndGroup`; always `None` for `\setbox`/`\shipout`/leader/insert
    /// bodies, since none of those box-openers can themselves be wrapped by
    /// a shift (`scan_box`'s `cur_cmd=make_box` requirement excludes `vmove`
    /// and `hmove`).
    shift: Option<Scaled>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReplayBoxKind {
    HBox,
    VBox,
    VTop,
    /// TeX82 §1167's `mmode+vcenter` body. It shares §645's `scan_spec`
    /// prefix, §1083's `push_nest; mode:=-vmode` internal vertical list, and
    /// §1085's `end_graf` before packaging with `\vbox`; §1168 then differs
    /// only in disposal, appending a `vcenter_noad` whose nucleus is the
    /// packaged box instead of running §1075's `box_end`.
    VCenter,
    /// TeX82 §1099/§1100's `\insert<class>{...}` or `\vadjust{...}` body --
    /// the latter shares the exact same construction with `class` fixed at
    /// 255 (`begin_insert_or_adjust`'s `if cur_cmd=vadjust then
    /// cur_val:=255`). This reuses the box body-closing machinery
    /// (`active_boxes`, `BoxEndGroup`) purely to recognize the body's own
    /// closing brace; its group kind, closing action, and
    /// page-builder interaction are entirely different from an ordinary vbox
    /// and are handled by a dedicated branch in `BoxEndGroup`
    /// (`finish_insert_or_adjust_group`, which also picks `ins_node` vs.
    /// `adjust_node` by this class).
    Insert(u16, bool),
}

impl ReplayBoxKind {
    /// The one mapping from a scanned §645 `scan_spec` construction to the
    /// replay kind that closes it. `\leaders`' payload scan and §1073's
    /// box-shift scan both require `cur_cmd=make_box` (§1073/§1078), so
    /// neither can deliver `\vcenter`; sharing this mapping keeps that fact
    /// stated once instead of once per call site.
    const fn from_scanned(kind: ScannedBoxKind) -> Self {
        match kind {
            ScannedBoxKind::HBox => Self::HBox,
            ScannedBoxKind::VBox => Self::VBox,
            ScannedBoxKind::VTop => Self::VTop,
            ScannedBoxKind::VCenter => Self::VCenter,
        }
    }

    const fn horizontal(self) -> bool {
        matches!(self, Self::HBox)
    }

    const fn group_kind(self) -> GroupKind {
        match self {
            Self::HBox => GroupKind::HBox,
            Self::VBox => GroupKind::VBox,
            Self::VTop => GroupKind::VTop,
            Self::VCenter => GroupKind::VCenter,
            Self::Insert(..) => GroupKind::Insert,
        }
    }
}

#[derive(Clone, Debug)]
struct ActiveReplayAlignment {
    identity: AlignmentIdentity,
    kind: AlignmentKind,
    /// TeX82 §645's `scan_spec` result, kept from `init_align` until §805
    /// packages the preamble prototype box with it.
    packing: AlignmentPackSpec,
    columns: Vec<AlignmentCellTemplates>,
    repeat_start: Option<usize>,
    column: usize,
    preamble_opening_pending: bool,
    preamble_start_pending: bool,
    cell_opening_pending: bool,
    next_cell_opening_pending: bool,
    align_peek_pending: bool,
    align_peek_after_noalign: bool,
    noalign_open: bool,
    /// Frozen cell material retained for lifecycle diagnostics. The actual
    /// row records live on the alignment level, exactly as TeX82 §775 does.
    captured_rows: Vec<Vec<NodeListId>>,
    tabskips: Vec<tex_state::ids::GlueId>,
    default_tabskip: tex_state::ids::GlueId,
    /// TeX82 §786's `cur_head`/`cur_tail` holding list: the insertions, marks,
    /// and `\vadjust` contents §796's `hpack` migrated out of this row's
    /// columns, waiting for §799 `fin_row` to append them after the row.
    row_migrations: Vec<Node>,
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
    output_routine_active: bool,
    output_routine_opening_pending: bool,
}

#[derive(Clone, Debug)]
struct ActiveDiscretionary {
    parts: Vec<NodeListId>,
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
    mode_savepoint: crate::mode::ModeSavepoint,
    next_alignment_identity: u64,
    active_alignment: Option<ActiveReplayAlignment>,
    boxes: ReplayBoxes,
    active_discretionaries: Vec<ActiveDiscretionary>,
    main_loop_active: bool,
    shown_mode: Option<Mode>,
    completed_replay_episode: Option<tex_command::CommandReplayEpisode>,
    prepared_dvi_pages: PreparedDviPages,
    completed_boundaries: Vec<crate::EngineBoundary>,
    /// A step's §537/§362 open-paren accounting is engine state outside
    /// `Universe`, exactly like `next_alignment_identity` and `boxes` above:
    /// a step that prints `(name` and then rolls back must have
    /// `open_parens` roll back with it too, or a later `\end` would close a
    /// paren that was never really opened.
    job: crate::job::JobFraming,
    universe: tex_state::Snapshot,
}

impl CanonicalStepSnapshot {
    fn capture(control: &mut CanonicalMainControl, stores: &mut Universe) -> Self {
        Self {
            command: control.command.snapshot(),
            mode_savepoint: control.modes.begin_journal(),
            next_alignment_identity: control.next_alignment_identity,
            active_alignment: control.active_alignment.clone(),
            boxes: control.boxes.clone(),
            active_discretionaries: control.active_discretionaries.clone(),
            main_loop_active: control.main_loop_active,
            shown_mode: control.shown_mode,
            completed_replay_episode: control.completed_replay_episode,
            prepared_dvi_pages: control.prepared_dvi_pages.clone(),
            completed_boundaries: control.completed_boundaries.clone(),
            job: control.job,
            universe: stores.snapshot(),
        }
    }

    fn rollback(self, control: &mut CanonicalMainControl, stores: &mut Universe) {
        // Roll the aggregate owner back before reinstalling command roots that
        // may contain OriginIds allocated from that owner. No intermediate
        // state with a restored command and a newer provenance timeline is
        // observable outside this method.
        stores.rollback(&self.universe);
        control
            .command
            .rollback(self.command)
            .expect("canonical step snapshot keeps its command profile");
        // CommandRuntime is deliberately non-cloneable: its caches and
        // profiling cannot become semantic or durable state. Its fresh value
        // is therefore the canonical retry restoration form.
        control.runtime = CommandRuntime::default();
        control
            .modes
            .rollback_journal(self.mode_savepoint)
            .expect("canonical step owns the innermost mode savepoint");
        control.next_alignment_identity = self.next_alignment_identity;
        control.active_alignment = self.active_alignment;
        control.boxes = self.boxes;
        control.active_discretionaries = self.active_discretionaries;
        control.main_loop_active = self.main_loop_active;
        control.shown_mode = self.shown_mode;
        control.completed_replay_episode = self.completed_replay_episode;
        control.prepared_dvi_pages = self.prepared_dvi_pages;
        control.completed_boundaries = self.completed_boundaries;
        control.job = self.job;
    }

    fn can_rollback(&self, stores: &Universe) -> bool {
        stores.can_rollback_to(&self.universe)
    }

    fn commit(self, control: &mut CanonicalMainControl) {
        control
            .modes
            .commit_journal(self.mode_savepoint)
            .expect("canonical step owns the innermost mode savepoint");
    }
}

/// Where one command-processor episode publishes its committed records.
///
/// An episode with no observer carries `None`. The slot is still a parameter
/// of [`command_processor`] so that no episode can be constructed without
/// stating which commit buffer it belongs to.
type ObservationSlot = Option<ObservationBuffer>;

#[derive(Debug, Default)]
struct ObservationBuffer(Vec<CommandObservation>);

impl ObservationBuffer {
    fn flush_into(self, observer: &mut dyn CommandObserver) {
        for observation in self.0 {
            observer.committed(observation);
        }
    }
}

impl CommandObserver for ObservationBuffer {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

/// Constructs the one kind of command-processor episode canonical main
/// control ever runs.
///
/// This is the only `CommandProcessor::new` call in `tex-exec`, and the
/// architecture test in `crates/tex-exec/tests/it.rs` pins it there. A single
/// main-control operation runs several episodes -- the delivery episode
/// itself, plus the nested math-field, math-script, and `\mathchoice`
/// episodes a host-applied step (`docs/tex_command_core.md` §33.5) runs while
/// it executes -- and each construction used to decide independently whether
/// to install the operation's observer. The nested math episodes never did,
/// so TeX82 §1151 `scan_math`'s whole braced script field was scanned with
/// zero observations while the identical unobserved run behaved the same
/// (Beads `umber2-johp.195`). Deciding once, here, is what makes that class
/// of divergence unrepresentable: observation is an instrumentation boundary,
/// not an alternate execution mode.
///
/// The borrows are passed individually rather than as `&mut self` so that a
/// caller can still lend main control's disjoint replay state -- notably
/// `boxes` -- to the scanner it drives with the returned processor.
/// The command machine's four borrowed halves, bundled.
///
/// [`command_processor`] deliberately takes them apart so that main control
/// can lend its other disjoint replay state at the same time. A helper that
/// needs to build a processor of its own -- rather than being handed one --
/// takes this instead, so passing the command machine along costs one
/// parameter instead of four.
struct CommandMachine<'a> {
    state: &'a mut CommandState,
    runtime: &'a mut CommandRuntime,
    fuel: &'a mut tex_command::CommandFuel,
    capabilities: &'a mut CommandHostCapabilities,
    observations: &'a mut ObservationSlot,
    /// tex.web's `init`/`tini` compile-time split, which Umber carries as a
    /// session flag: §1252's `\patterns` and §1335's `\dump` are the two
    /// commands whose whole behavior it selects.
    initex: bool,
}

impl CommandMachine<'_> {
    fn processor<'a>(&'a mut self, stores: &'a mut Universe) -> CommandProcessor<'a> {
        command_processor(
            self.state,
            self.runtime,
            self.fuel,
            self.capabilities,
            self.observations,
            stores,
        )
    }
}

fn command_processor<'a>(
    command: &'a mut CommandState,
    runtime: &'a mut CommandRuntime,
    fuel: &'a mut tex_command::CommandFuel,
    capabilities: &'a mut CommandHostCapabilities,
    observations: &'a mut ObservationSlot,
    stores: &'a mut Universe,
) -> CommandProcessor<'a> {
    let processor = CommandProcessor::new(
        command,
        runtime,
        stores.command_context(),
        CommandHostContext::new(capabilities),
    )
    .with_fuel(fuel);
    match observations.as_mut() {
        Some(buffer) => processor.with_observer(buffer),
        None => processor,
    }
}

impl CanonicalMainControl {
    pub const DEFAULT_FUEL_LIMIT: u64 = tex_command::DEFAULT_COMMAND_FUEL_LIMIT;

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

    /// Creates INITEX command state for a profile whose primitive meanings
    /// the composed driver has already installed in `stores`.
    #[must_use]
    pub fn prepared_initex(profile: CommandProfile) -> Self {
        Self {
            command: CommandState::new(profile),
            next_alignment_identity: 1,
            initex: true,
            ..Self::default()
        }
    }

    /// Creates a fresh canonical TeX82 INITEX replay environment.
    ///
    /// The primitive definitions are installed from the engine's static TeX82
    /// registries, before any fixture or host source is registered.
    #[must_use]
    pub fn tex82_initex(stores: &mut Universe) -> Self {
        tex_command::install_tex82_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        Self {
            command: CommandState::new(CommandProfile::TEX82),
            next_alignment_identity: 1,
            initex: true,
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

    /// Replaces the command-work ledger before or during a run.
    ///
    /// Installing a new limit intentionally starts a new accounting episode;
    /// ordinary rollback and runtime reset never call this.
    pub fn set_fuel_limit(&mut self, limit: u64) -> Result<(), tex_command::CommandFuelLimitError> {
        self.fuel = tex_command::CommandFuelLedger::new(limit)?;
        Ok(())
    }

    #[must_use]
    pub const fn fuel_limit(&self) -> u64 {
        self.fuel.limit()
    }

    #[must_use]
    pub const fn fuel_burned(&self) -> u64 {
        self.fuel.burned()
    }

    /// Reports whether this job terminated through an effective INITEX
    /// `\dump`, so the host can publish the same `RunResult` contract as the
    /// retired executor without inspecting command delivery.
    #[must_use]
    pub const fn dumped_format(&self) -> bool {
        self.dumped_format.is_some()
    }

    /// Returns §1328's engine-owned identity after a successful INITEX dump.
    #[must_use]
    pub fn format_dump_receipt(&self) -> Option<&crate::job::FormatDumpReceipt> {
        self.dumped_format.as_ref()
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

    /// tex.web §61/§534/§536, plus etex.ch's "entering extended mode" notice:
    /// prints the start-up banner, that notice when this session's profile is
    /// [`CommandProfile::ETEX26`], and the `**` first line. Idempotent --
    /// only the first call prints anything. Call this before registering the
    /// root source (e.g. before [`Self::register_root_source`]), so the
    /// banner and `**` line precede the root file's own `(`. See
    /// [`crate::job`].
    pub fn begin_job(&mut self, stores: &mut Universe, first_line: &str) {
        let binary = self
            .engine_binary
            .unwrap_or_else(|| match self.command_profile().dialect() {
                tex_command::CommandDialect::Tex82 => crate::job::EngineBinaryIdentity::Tex82,
                tex_command::CommandDialect::Etex26 => crate::job::EngineBinaryIdentity::Etex26,
                tex_command::CommandDialect::Pdftex14027 => {
                    crate::job::EngineBinaryIdentity::Pdftex14027
                }
            });
        let etex = self.command_profile() == CommandProfile::ETEX26;
        // §534's `**` line is exactly what §313 pseudoprints for the base
        // terminal level; a driver that frames the job here rather than
        // scanning the line through `scan_startup_file_name` supplies it here.
        first_line.clone_into(&mut self.startup_terminal_line);
        crate::job::begin_job(
            &mut self.job,
            stores,
            &mut self.capabilities,
            self.initex,
            self.preloaded_format.as_ref(),
            crate::job::JobEngineFraming {
                binary,
                extended_mode: etex,
            },
            first_line,
        );
    }

    /// Prints and accounts for the retained driver's already-open root input.
    ///
    /// This is TeX82 §537's opening boundary. Keeping its `open_parens`
    /// mutation beside the print lets §1335 close an input abandoned by
    /// `\end` or `\dump`, just as it closes command-opened inputs.
    pub fn open_startup_input(&mut self, stores: &mut Universe, name: &str) {
        crate::job::open_startup_input(stores, name);
    }

    /// Declares that this session was restored from a dumped format, so
    /// [`Self::begin_job`] frames it as `-fmt=<name>` rather than as INITEX.
    ///
    /// Framing only: nothing about execution changes, and a session that
    /// never calls this is framed exactly as before.
    pub fn set_preloaded_format(&mut self, format: crate::job::PreloadedFormat) {
        self.preloaded_format = Some(format);
    }

    /// Selects the engine binary identity used by startup framing.
    pub fn set_engine_binary(&mut self, binary: crate::job::EngineBinaryIdentity) {
        self.engine_binary = Some(binary);
    }

    /// tex.web §1333 `close_files_and_terminate`'s prints: §642's DVI page
    /// report and the transcript-closing note. Call this once, after a step
    /// has reported [`MainControlStep::End`] and, if the job shipped any
    /// pages, after serializing them to a `.dvi` file so `dvi` can report its
    /// name and exact length. `dvi` is `None` when the job shipped no pages;
    /// see [`crate::DviJobOutput`].
    ///
    /// # Panics
    ///
    /// If the job shipped pages and `dvi` is `None`. §642 prints the DVI
    /// file's exact byte length, which no engine-level state holds, so the
    /// alternative to refusing here is printing a fabricated number.
    pub fn finish_job(&mut self, stores: &mut Universe, dvi: Option<crate::DviJobOutput>) {
        crate::job::finish_job(stores, self.capabilities.job_name(), dvi);
    }

    /// Renders whatever §537/§362 bracketing the command core queued but had
    /// no `Universe` in hand to print.
    ///
    /// The command core renders every event at the point tex.web prints it
    /// whenever it can -- §362's `)` has to precede the
    /// `check_outer_validity` diagnostic printed a line later inside
    /// `get_next` -- so what reaches here is the residue. Every step driver
    /// (`step_once`, `alignment_step_once`, `step_with_observer_once`) calls
    /// this once, immediately after it reports the step's other diagnostics.
    fn drain_file_framing_events(&mut self, stores: &mut Universe) {
        self.command
            .render_file_framing_events(&mut stores.command_context());
    }

    /// tex.web §1335 `final_cleanup`'s tail, run once a step has produced
    /// [`ReplayStep::End`]: closing every still-open paren, reporting
    /// unfinished conditionals, the "(see the transcript file..." note, and
    /// the `\dump`-outside-INITEX note, in that exact order. The first of
    /// those needs `self`'s job-framing state, which the free function
    /// `apply_scanned_step` that scans `ScannedStep::End` does not have; the
    /// other three used to run inside that free function and are moved here,
    /// not copied, so they run after the paren close instead of before it.
    fn end_of_job_final_cleanup(
        &mut self,
        stores: &mut Universe,
        dump: bool,
        incomplete_conditions: Vec<tex_command::IncompleteCondition>,
    ) {
        crate::job::close_open_parens(stores);
        crate::job::report_unclosed_groups(stores);
        report_incomplete_conditions(stores, incomplete_conditions);
        crate::job::print_history_note(stores);
        if dump && !self.initex {
            // TeX82 §§1328/1335: INITEX enters `store_fmt_file`, whose first
            // observable transition is the format-file announcement and new
            // `format_ident`. The production binary instead keeps only the
            // `print_nl` that says dumping is unavailable.
            stores
                .printer()
                .print_nl("(\\dump is performed only by INITEX)");
        }
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
            mut request,
            resource: _,
        } = scanned
        else {
            return Ok(scanned);
        };
        let resource = match &mut request {
            InputStreamRequest::Open { file_name, .. } => {
                // tex.web §1275: `if cur_ext="" then cur_ext:=".tex";
                // pack_cur_name`. The packed name is what is opened, so it is
                // written back into the request rather than recomputed.
                file_name.components.apply_default_extension(".tex");
                let packed_name = file_name.packed();
                // §1275's `if a_open_in(read_file[n])` leaves the stream
                // closed when the file does not open, but Umber resolves
                // inputs through the host first: an unregistered name
                // suspends the step so the driver can acquire it, and only a
                // host that reports the file absent reaches the closed-stream
                // outcome.
                Some(
                    self.capabilities
                        .input_resource(&packed_name)
                        .ok_or(ExecError::MissingCanonicalInput { name: packed_name })?,
                )
            }
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
    pub fn refresh_host_capabilities(&mut self, stores: &Universe) {
        self.capabilities
            .set_conditional_state(self.modes.conditional_state());
        self.capabilities.set_space_factor(
            matches!(
                self.modes.current_mode(),
                Mode::Horizontal | Mode::RestrictedHorizontal
            )
            .then(|| self.modes.current_list().space_factor()),
        );
        // tex.web §418's `set_aux` twin of `space_factor`: `\prevdepth` is
        // readable only in vertical mode, where an unset `prev_depth` is
        // §215's `ignore_depth` initial value.
        self.capabilities.set_prev_depth(
            matches!(
                self.modes.current_mode(),
                Mode::Vertical | Mode::InternalVertical
            )
            .then(|| {
                self.modes
                    .current_list()
                    .prev_depth()
                    .unwrap_or_else(|| crate::mode::ignored_depth(stores))
            }),
        );
        // tex.web §422's `set_prev_graf` walks up to the nearest enclosing
        // vertical level rather than testing the current mode.
        self.capabilities
            .set_prev_graf(Some(self.modes.enclosing_vertical_prev_graf()));
        self.capabilities
            .set_last_node(self.last_node_value(stores));
        self.capabilities
            .set_last_node_type(self.last_node_type_value(stores));
    }

    /// TeX82 §424's "Fetch an item in the current node, if appropriate": the
    /// current list's tail-node classification consumed by
    /// `\lastpenalty`/`\lastkern`/`\lastskip`.
    ///
    /// The outer vertical list is special (matching `\unskip`'s existing
    /// `is_outer_vertical`/`page_has_last_glue` precedent from
    /// umber2-johp.81, reused here rather than duplicated):
    /// `append_vertical_contribution` moves every node contributed at that
    /// level straight to the page builder's contribution list instead of
    /// `ModeNest`'s own list, so this mode nest's list is never the right
    /// place to look. tex.web's real tail there is `contrib_head`, a fixed
    /// address in `is_char_node`'s address range, which is why its
    /// `scan_something_internal` falls through to `last_penalty`/
    /// `last_kern`/`last_glue` (updated together by §996 whenever the page
    /// builder sweeps a node onto the page) exactly when the contribution
    /// list has been swept empty; while it is nonempty, the real
    /// contribution tail governs, just as it does for `\unskip`.
    fn last_node_value(&self, stores: &Universe) -> Option<tex_command::LastNodeItem> {
        if is_outer_vertical(&self.modes) {
            return match stores.page_contribution_tail() {
                Some(node) => Self::classify_last_node(stores, node),
                None => match stores.page_last_node_type() {
                    11 => Some(tex_command::LastNodeItem::Glue(stores.page_last_skip())),
                    12 => Some(tex_command::LastNodeItem::Kern(stores.page_last_kern())),
                    13 => Some(tex_command::LastNodeItem::Penalty(
                        stores.page_last_penalty(),
                    )),
                    _ => None,
                },
            };
        }
        self.modes
            .current_list()
            .nodes()
            .last()
            .and_then(|node| Self::classify_last_node(stores, node))
    }

    /// e-TeX 2.6 `etex.ch` [26.424]'s `find_effective_tail` result for
    /// `\lastnodetype`.
    fn last_node_type_value(&self, stores: &Universe) -> i32 {
        if is_outer_vertical(&self.modes) {
            return stores
                .page_contribution_tail()
                .map_or_else(|| stores.page_last_node_type(), Node::etex_type);
        }
        // Batched horizontal characters are already semantic character nodes
        // even though Umber has not materialized their shaped run yet.
        if self.modes.current_list().pending_hchars().is_some() {
            return 0;
        }
        self.modes
            .current_list()
            .nodes()
            .last()
            .map_or(-1, Node::etex_type)
    }

    /// Classifies one real node as a `\lastpenalty`/`\lastkern`/`\lastskip`
    /// tail, resolving a glue node's stored specification and distinguishing
    /// TeX82's `mu_glue` subtype (an explicit `\mskip`, matched here by
    /// [`GlueKind::MuSkip`]) so `\lastskip` reads it at `mu_val` level. Any
    /// other node shape (including a character, which tex.web excludes via
    /// `is_char_node`) has no matching case, exactly like tex.web's
    /// `case cur_chr of ... end {there are no other cases}`.
    fn classify_last_node(stores: &Universe, node: &Node) -> Option<tex_command::LastNodeItem> {
        match node {
            Node::Penalty(value) => Some(tex_command::LastNodeItem::Penalty(*value)),
            Node::Kern { amount, .. } => Some(tex_command::LastNodeItem::Kern(*amount)),
            Node::Glue {
                spec,
                kind: GlueKind::MuSkip,
                ..
            } => Some(tex_command::LastNodeItem::MuGlue(stores.glue(*spec))),
            Node::Glue { spec, .. } => Some(tex_command::LastNodeItem::Glue(stores.glue(*spec))),
            _ => None,
        }
    }

    fn snapshot_step(&mut self, stores: &mut Universe) -> CanonicalStepSnapshot {
        CanonicalStepSnapshot::capture(self, stores)
    }

    fn commit_step(&mut self, snapshot: CanonicalStepSnapshot) {
        snapshot.commit(self);
    }

    /// Lends the whole command machine at once, for helpers that build their
    /// own processor rather than being handed one. A caller that must keep
    /// another of main control's fields borrowed at the same time builds the
    /// bundle from those fields directly instead.
    fn command_machine(&mut self) -> CommandMachine<'_> {
        CommandMachine {
            state: &mut self.command,
            runtime: &mut self.runtime,
            fuel: self.fuel.fuel_mut(),
            capabilities: &mut self.capabilities,
            observations: &mut self.operation_observations,
            initex: self.initex,
        }
    }

    /// Takes TeX82 §1030's parking decision for the step just scanned, and
    /// clears the outgoing parking so nested episodes run from `big_switch`.
    ///
    /// Every step driver takes this before applying its step and gives it back
    /// to [`Self::resume_main_control_parking`] afterwards. The rule is stated
    /// here once: three drivers used to spell it out inline, and a rule spelled
    /// three times is a rule two of them can be missing.
    fn suspend_main_control_parking(&mut self, scanned: &ScannedStep) -> MainControlParking {
        let parking = MainControlParking {
            character: scanned.main_loop_character(),
            resumes_interrupted_fetch: matches!(scanned, ScannedStep::AlignmentTemplateEntered),
        };
        if !parking.resumes_interrupted_fetch {
            self.main_loop_active = false;
        }
        parking
    }

    /// Records which of TeX82 §1030's two fetch labels `main_control` is
    /// parked at now that this step has been applied.
    ///
    /// §1034's `main_loop` is reached only from `hmode`, so the mode tested
    /// is the one the step left behind: §1090's `vmode+letter` opens a
    /// paragraph first and arrives in horizontal mode, while `mmode+letter`
    /// (§1154) appends a math char and never enters the loop at all.
    ///
    /// A character the current font does not contain never reaches the
    /// lookahead either: §1036's `main_loop_move+2` issues `char_warning`,
    /// frees the would-be node, and jumps straight back to `big_switch`. With
    /// `\nullfont` selected -- §552 gives it `font_bc=1`, `font_ec=0`, so no
    /// character at all exists -- that is every character in the document.
    fn resume_main_control_parking(&mut self, parking: MainControlParking, stores: &Universe) {
        if parking.resumes_interrupted_fetch {
            return;
        }
        self.main_loop_active = parking.character.is_some_and(|character| {
            matches!(
                self.modes.current_mode(),
                Mode::Horizontal | Mode::RestrictedHorizontal
            ) && stores
                .font(stores.current_font())
                .character_exists(character)
        });
    }

    fn rollback_step(&mut self, snapshot: CanonicalStepSnapshot, stores: &mut Universe) {
        snapshot.rollback(self, stores);
    }

    /// Drains committed canonical shipout receipts in artifact order.
    ///
    /// Each plan was prepared during shipout and is retained only after the
    /// enclosing aggregate operation commits; finalizers must not re-lower
    /// these pages from artifact bytes.
    #[must_use]
    pub fn take_prepared_dvi_pages(&mut self) -> Vec<crate::dispatch::PreparedDviPage> {
        take_prepared_dvi_pages(&mut self.prepared_dvi_pages)
    }

    /// Drains named boundaries that became safe during committed aggregate
    /// operations.  This is deliberately an event receipt, not a request for
    /// the host to inspect modes or dispatch source tokens.
    #[must_use]
    pub fn take_completed_boundaries(&mut self) -> Vec<crate::EngineBoundary> {
        std::mem::take(&mut self.completed_boundaries)
    }

    /// Returns the replay projection of TeX's current execution mode.
    #[must_use]
    pub fn current_mode(&self) -> Mode {
        self.modes.current_mode()
    }

    /// Returns the mode nest's current list, so a crate test can assert on the
    /// material main control has built without shipping a page first.
    #[cfg(test)]
    pub(crate) fn current_list(&self) -> &crate::ModeList {
        self.modes.current_list()
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
        }
        Ok(())
    }

    /// Runs TeX82 §1030 `main_control`'s prologue exactly once per job.
    ///
    /// tex.web enters `main_control` with
    /// `if every_job<>null then begin_token_list(every_job,every_job_text)`
    /// and only then reaches `big_switch`, so the hook's tokens precede every
    /// command the job delivers. `scan_startup_file_name` (§1337's
    /// `@<Get the first line of input and prepare to start@>`) still runs
    /// before this, exactly as it does in tex.web's main program.
    ///
    /// Returns whether this call was the entry, so an observed step publishes
    /// the prologue's push only on the step that produced it.
    fn enter_main_control(&mut self, stores: &mut Universe) -> bool {
        // Seeds `line` before the first command is delivered; every step
        // republishes it after delivery (see `step_once`).
        stores.set_current_input_line(
            i32::try_from(self.command.current_file_line_number()).unwrap_or(i32::MAX),
        );
        if std::mem::replace(&mut self.main_control_entered, true) {
            return false;
        }
        schedule_everyjob(&mut self.command, stores);
        true
    }

    /// Appends already-committed records to the operation's commit buffer.
    /// They are published only when the whole operation commits.
    fn observe_committed(&mut self, records: impl IntoIterator<Item = CommandObservation>) {
        if let Some(buffer) = self.operation_observations.as_mut() {
            buffer.0.extend(records);
        }
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
        match result {
            Ok(step) => {
                self.commit_step(snapshot);
                Ok(step)
            }
            Err(error) => {
                if error.as_fatal().is_some() {
                    self.commit_step(snapshot);
                } else {
                    self.rollback_step(snapshot, stores);
                }
                Err(error)
            }
        }
    }

    fn alignment_step_once(
        &mut self,
        alignment: AlignmentIdentity,
        stores: &mut Universe,
    ) -> Result<ReplayStep, ExecError> {
        self.drain_file_framing_events(stores);
        let mode = self.modes.current_mode();
        let innermost_group = stores.innermost_group_kind();
        let main_loop_active = self.main_loop_active;
        let job_is_all_over = crate::output::job_is_all_over(stores);
        let mut diagnostics = Vec::new();
        let scanned = {
            let mut processor = command_processor(
                &mut self.command,
                &mut self.runtime,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
                &mut self.operation_observations,
                stores,
            );
            let scanned = scan_alignment_delivery_step(
                &mut processor,
                alignment,
                &ReplayBoxes::default(),
                innermost_group,
                mode,
                job_is_all_over,
                main_loop_active,
                &mut diagnostics,
            )?;
            diagnostics.extend(
                processor
                    .take_semantic_diagnostics()
                    .into_iter()
                    .map(PendingDiagnostic::Command),
            );
            scanned
        };
        // tex.web's `line` is maintained by `get_next` as it moves to a new
        // input line, so it is already the delivered command's own line by
        // the time that command is applied. Publish it here, after delivery,
        // rather than at the step's start: §660/§675's box diagnostics and
        // §1091's `mode_line` both name the line the command is *on*, and a
        // command that is the first thing on a line is scanned by a step
        // that began on the previous one.
        stores.set_current_input_line(
            i32::try_from(self.command.current_file_line_number()).unwrap_or(i32::MAX),
        );
        report_pending_diagnostics(stores, diagnostics, &mut self.shown_mode)?;
        self.drain_file_framing_events(stores);
        let scanned = self.resolve_font_resource(scanned)?;
        let scanned = self.resolve_input_stream_resource(scanned)?;
        let scanned = self.resolve_pdf_image_resource(scanned, stores)?;
        let parking = self.suspend_main_control_parking(&scanned);
        let scanned = match self.apply_host_owned_step(scanned, stores) {
            ControlFlow::Break(applied) => return applied,
            ControlFlow::Continue(scanned) => scanned,
        };
        let fires_afterassignment = scanned.fires_afterassignment();
        let dumped_format = self.initex && matches!(scanned, ScannedStep::End { dump: true, .. });
        let end_tail = match &scanned {
            ScannedStep::End {
                dump,
                incomplete_conditions,
            } => Some((*dump, incomplete_conditions.clone())),
            _ => None,
        };
        let result = apply_scanned_step(
            scanned,
            stores,
            &mut self.modes,
            &mut self.next_alignment_identity,
            &mut self.active_alignment,
            &mut CommandMachine {
                state: &mut self.command,
                runtime: &mut self.runtime,
                fuel: self.fuel.fuel_mut(),
                capabilities: &mut self.capabilities,
                observations: &mut self.operation_observations,
                initex: self.initex,
            },
            &mut self.boxes,
            &mut self.prepared_dvi_pages,
        )?;
        if dumped_format {
            self.dumped_format = Some(crate::job::FormatDumpReceipt::new(
                self.capabilities.job_name().to_owned(),
                stores.int_param(IntParam::YEAR),
                stores.int_param(IntParam::MONTH),
                stores.int_param(IntParam::DAY),
            ));
        }
        if let (ReplayStep::End, Some((dump, incomplete_conditions))) = (&result, end_tail) {
            self.end_of_job_final_cleanup(stores, dump, incomplete_conditions);
        } else if matches!(result, ReplayStep::EndOfInput) {
            crate::job::prompt_for_more_input(stores, &self.startup_terminal_line);
        }
        self.resume_main_control_parking(parking, stores);
        if fires_afterassignment {
            schedule_afterassignment(
                &mut self.command,
                &mut self.runtime,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
                &mut self.operation_observations,
                stores,
            )?;
        }
        Ok(result)
    }

    /// Applies the scanned steps `CanonicalMainControl` owns itself instead of
    /// routing through [`apply_scanned_step`], and hands every other step back
    /// unchanged.
    ///
    /// Every step-delivery entry point -- unobserved, observed, and alignment
    /// -- routes through this single match, so the host-applied set is stated
    /// exactly once. It used to be an `if let` chain copied into each entry
    /// point, and the observed copy was missing [`ScannedStep::MathShift`]:
    /// an observed `$` fell through to `apply_scanned_step`'s `unreachable!()`
    /// while the identical unobserved `$` was applied correctly
    /// (umber2-johp.118). Add a host-applied step here, never at a call site.
    ///
    /// Each arm runs nested command-owned episodes whose own last character
    /// would otherwise leave `main_loop_active` set. None of them is a §1030
    /// `main_loop` entry, so all of them resume at `big_switch`.
    fn apply_host_owned_step(
        &mut self,
        scanned: ScannedStep,
        stores: &mut Universe,
    ) -> ControlFlow<Result<ReplayStep, ExecError>, ScannedStep> {
        let applied = match scanned {
            ScannedStep::ReplayCompleted(episode) => {
                self.completed_replay_episode = Some(episode);
                Ok(ReplayStep::Continue)
            }
            ScannedStep::Math(request) => self.apply_canonical_math_request(request, stores),
            ScannedStep::MathDelimiter(boundary) => {
                self.apply_canonical_math_delimiter(boundary, stores)
            }
            // TeX82 §1137's `hmode+math_shift: init_math` and §1193's
            // `mmode+math_shift: if cur_group=math_shift_group then
            // after_math else off_save`. §1090 backs a `vmode+math_shift` up
            // and runs `new_graf(true)` first, so vertical mode never reaches
            // this step.
            ScannedStep::MathShift { paired } => self.apply_canonical_math_shift(paired, stores),
            ScannedStep::DiscretionaryOpening(opening) => self.begin_discretionary(opening, stores),
            ScannedStep::DiscretionaryPartEnd => self.finish_discretionary_part(stores),
            ScannedStep::DiscretionaryHyphen { origin } => {
                self.apply_discretionary_hyphen(origin, stores)
            }
            // TeX82 §1123's `make_accent` runs §1270's `do_assignments`
            // between the accent code and §1124's base character, so it
            // executes whole commands of its own before it can finish.
            ScannedStep::Accent(accent) => self.apply_accent(accent, stores),
            scanned => return ControlFlow::Continue(scanned),
        };
        self.main_loop_active = false;
        ControlFlow::Break(applied)
    }

    /// The page/output tail every step ends with, for the host-owned steps
    /// [`Self::apply_host_owned_step`] applies instead of `apply_scanned_step`.
    ///
    /// tex.web has no deferral here at all: §1005's `fire_up(...)` runs inside
    /// §994's `build_page`, and §1012's `fire_up` reaches §1025's
    /// `begin_token_list(output_routine,output_text)` before `build_page`
    /// returns to whatever contributed. Umber buffers that push and performs it
    /// in `fire_pending_page_output` at the end of the step instead, so that
    /// tail is the whole of the mechanism and it has to run after *every*
    /// step. §1030's math (`init_math`/`after_math`), math-delimiter,
    /// math-shift, discretionary and replay-completion cases returned before
    /// it, so a page frozen inside one of them -- §1200's
    /// `resume_after_display` ends with `if nest_ptr=1 then build_page` -- kept
    /// `page_fire_up` pending until some later ordinary command's step, and
    /// `\output` was entered that many deliveries late.
    fn finish_host_owned_step(
        &mut self,
        applied: Result<ReplayStep, ExecError>,
        artifact_count: usize,
        _effect_count: usize,
        _prepared_page_count: usize,
        stores: &mut Universe,
    ) -> Result<ReplayStep, ExecError> {
        let applied = match applied {
            Ok(applied) => applied,
            Err(error) => {
                self.page_output_observations.clear();
                return Err(error);
            }
        };
        // Only an episode this step actually starts publishes records: when
        // nothing is pending, draining the command state's held named
        // token-list pushes here would reorder pushes another step owns.
        let opens_output_episode =
            stores.page_fire_up().is_some() && !self.boxes.output_routine_active;
        self.fire_pending_page_output(stores)?;
        {
            if opens_output_episode {
                // Same order as the ordinary tail: the named token-list push
                // command state held across the transition, then the shipouts
                // it committed, then the episode's own records.
                let mut records: Vec<CommandObservation> = self
                    .command
                    .take_named_token_list_push_observations()
                    .into_iter()
                    .map(CommandObservation::Input)
                    .collect();
                records.extend(
                    committed_shipout_observations(artifact_count, stores)
                        .into_iter()
                        .map(CommandObservation::Effect),
                );
                records.extend(
                    committed_stream_effect_observations(
                        _effect_count,
                        _prepared_page_count,
                        stores,
                        &self.prepared_dvi_pages,
                    )
                    .into_iter()
                    .map(CommandObservation::Effect),
                );
                records.append(&mut self.page_output_observations);
                self.observe_committed(records);
            }
            self.page_output_observations.clear();
        }
        if stores.world().artifact_commits().len() != artifact_count {
            self.completed_boundaries
                .push(crate::EngineBoundary::ShipoutComplete);
        }
        Ok(applied)
    }

    /// Enters TeX82 §1117's live `disc_group` after the command processor has
    /// consumed only its opening brace.
    fn begin_discretionary(
        &mut self,
        _opening: ScannedDiscretionaryOpening,
        stores: &mut Universe,
    ) -> Result<ReplayStep, ExecError> {
        if matches!(
            self.modes.current_mode(),
            Mode::Vertical | Mode::InternalVertical
        ) {
            start_canonical_paragraph(&mut self.command, &mut self.modes, stores, true)?;
        }
        crate::assignments::flush_pending_hchars_with_fuel(
            &mut self.modes,
            stores,
            self.fuel.fuel_mut(),
        )?;
        self.open_discretionary_part(stores)?;
        self.active_discretionaries
            .push(ActiveDiscretionary { parts: Vec::new() });
        Ok(ReplayStep::Continue)
    }

    fn open_discretionary_part(&mut self, stores: &mut Universe) -> Result<(), ExecError> {
        // TeX82 §216 checks nest capacity before saving the current semantic
        // level. Fatal overflow is committed by canonical main control, so
        // this fallible operation must precede both halves of the live
        // discretionary lifecycle: no rejected opener may leave a disc_group
        // without its restricted-horizontal mode.
        self.modes.push(Mode::RestrictedHorizontal)?;
        stores.enter_group_with_kind_at_line(
            GroupKind::Disc,
            self.command.current_file_line_number(),
        );
        Ok(())
    }

    /// Implements §1120's `build_discretionary`: finish the current live
    /// restricted-horizontal list, `unsave`, and either scan the next opening
    /// brace or append the completed three-part node.
    fn finish_discretionary_part(
        &mut self,
        stores: &mut Universe,
    ) -> Result<ReplayStep, ExecError> {
        let level =
            crate::assignments::commit_current_list(&mut self.modes, stores, self.fuel.fuel_mut())?;
        let nodes = stores.freeze_node_list(level.list().nodes());
        let aftergroup =
            stores
                .leave_group_with_kind(GroupKind::Disc)
                .map_err(|_| ExecError::MissingToken {
                    context: "discretionary group",
                })?;
        schedule_aftergroup(&mut self.command_machine(), stores, aftergroup)?;

        let part_count = {
            let active = self
                .active_discretionaries
                .last_mut()
                .ok_or(ExecError::MissingToken {
                    context: "active discretionary",
                })?;
            active.parts.push(nodes);
            active.parts.len()
        };
        if part_count < 3 {
            let mut diagnostics = Vec::new();
            {
                let mut processor = command_processor(
                    &mut self.command,
                    &mut self.runtime,
                    self.fuel.fuel_mut(),
                    &mut self.capabilities,
                    &mut self.operation_observations,
                    stores,
                );
                let _ = processor
                    .scan_discretionary_opening()
                    .map_err(command_error)?;
                diagnostics.extend(
                    processor
                        .take_semantic_diagnostics()
                        .into_iter()
                        .map(PendingDiagnostic::Command),
                );
            }
            report_pending_diagnostics(stores, diagnostics, &mut self.shown_mode)?;
            self.open_discretionary_part(stores)?;
            return Ok(ReplayStep::Continue);
        }
        let active = self
            .active_discretionaries
            .pop()
            .expect("three parts require an active discretionary");
        let [pre, post, replace]: [NodeListId; 3] = active
            .parts
            .try_into()
            .expect("discretionary completes after exactly three parts");
        self.modes.current_list_mutation().push(Node::Disc {
            kind: DiscKind::Discretionary,
            pre,
            post,
            replace,
        });
        Ok(ReplayStep::Continue)
    }

    /// Executes TeX82 §1113's `append_discretionary` shorthand for `\-`.
    fn apply_discretionary_hyphen(
        &mut self,
        origin: tex_state::token::OriginId,
        stores: &mut Universe,
    ) -> Result<ReplayStep, ExecError> {
        if matches!(
            self.modes.current_mode(),
            Mode::Vertical | Mode::InternalVertical
        ) {
            start_canonical_paragraph(&mut self.command, &mut self.modes, stores, true)?;
        }
        crate::assignments::flush_pending_hchars_with_fuel(
            &mut self.modes,
            stores,
            self.fuel.fuel_mut(),
        )?;
        let font = stores.current_font();
        let hyphen = u8::try_from(stores.font_hyphen_char(font))
            .ok()
            .map(char::from)
            .unwrap_or('-');
        let pre = stores.freeze_node_list(&[Node::Char {
            font,
            ch: hyphen,
            origin,
        }]);
        let empty = stores.freeze_node_list(&[]);
        self.modes.current_list_mutation().push(Node::Disc {
            kind: DiscKind::ExplicitHyphen,
            pre,
            post: empty,
            replace: empty,
        });
        Ok(ReplayStep::Continue)
    }

    /// Attempts one atomic canonical main-control operation.
    ///
    /// Missing retained input rolls back the complete aggregate operation and
    /// is returned as a typed suspension. All other failures are restored and
    /// remain ordinary diagnostics. In either case the next call creates a
    /// fresh command processor; no delivered `CurrentCommand` is retained.
    pub fn advance(&mut self, stores: &mut Universe) -> Result<CanonicalStepResult, ExecError> {
        if self.fatal.is_some() {
            return Ok(CanonicalStepResult::Progress(MainControlStep::End));
        }
        let snapshot = self.snapshot_step(stores);
        match self.step_once(stores, None) {
            Ok(step) => {
                self.commit_step(snapshot);
                Ok(CanonicalStepResult::Progress(step))
            }
            Err(error) => {
                if let Some(fatal) = error.as_fatal() {
                    self.commit_step(snapshot);
                    return Ok(CanonicalStepResult::Progress(self.succumb(fatal)));
                }
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

    fn step_once(
        &mut self,
        stores: &mut Universe,
        redispatch: Option<tex_command::CurrentCommand>,
    ) -> Result<ReplayStep, ExecError> {
        self.drain_file_framing_events(stores);
        self.enter_main_control(stores);
        self.refresh_host_capabilities(stores);
        let mode = self.modes.current_mode();
        let outer_paragraph_was_active = mode == Mode::Horizontal && self.modes.depth() == 2;
        let alignment_preamble = alignment_preamble(self.active_alignment.as_mut());
        let innermost_group = stores.innermost_group_kind();
        let job_is_all_over = crate::output::job_is_all_over(stores);
        let mut diagnostics = Vec::new();
        let scanned = {
            let mut processor = command_processor(
                &mut self.command,
                &mut self.runtime,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
                &mut self.operation_observations,
                stores,
            );
            let scanned = match redispatch {
                Some(command) => dispatch_main_control_command(
                    &mut processor,
                    command,
                    mode,
                    &self.boxes,
                    innermost_group,
                    job_is_all_over,
                    self.modes.current_list().display_eq_no().is_some(),
                    &mut diagnostics,
                )?,
                None => scan_replay_step(
                    &mut processor,
                    mode,
                    &self.boxes,
                    alignment_preamble,
                    innermost_group,
                    job_is_all_over,
                    self.modes.current_list().display_eq_no().is_some(),
                    self.main_loop_active,
                    &mut diagnostics,
                )?,
            };
            diagnostics.extend(
                processor
                    .take_semantic_diagnostics()
                    .into_iter()
                    .map(PendingDiagnostic::Command),
            );
            scanned
        };
        // tex.web's `line` is maintained by `get_next` as it moves to a new
        // input line, so it is already the delivered command's own line by
        // the time that command is applied. Publish it here, after delivery,
        // rather than at the step's start: §660/§675's box diagnostics and
        // §1091's `mode_line` both name the line the command is *on*, and a
        // command that is the first thing on a line is scanned by a step
        // that began on the previous one.
        stores.set_current_input_line(
            i32::try_from(self.command.current_file_line_number()).unwrap_or(i32::MAX),
        );
        report_pending_diagnostics(stores, diagnostics, &mut self.shown_mode)?;
        self.drain_file_framing_events(stores);
        let scanned = self.resolve_font_resource(scanned)?;
        let scanned = self.resolve_input_stream_resource(scanned)?;
        let scanned = self.resolve_pdf_image_resource(scanned, stores)?;
        let parking = self.suspend_main_control_parking(&scanned);
        let artifact_count = stores.world().artifact_commits().len();
        let effect_count = stores.world().effect_records().len();
        let prepared_page_count = self.prepared_dvi_pages.len();
        let scanned = match self.apply_host_owned_step(scanned, stores) {
            ControlFlow::Break(applied) => {
                return self.finish_host_owned_step(
                    applied,
                    artifact_count,
                    effect_count,
                    prepared_page_count,
                    stores,
                );
            }
            ControlFlow::Continue(scanned) => scanned,
        };
        let fires_afterassignment = scanned.fires_afterassignment();
        let dumped_format = self.initex && matches!(scanned, ScannedStep::End { dump: true, .. });
        let end_tail = match &scanned {
            ScannedStep::End {
                dump,
                incomplete_conditions,
            } => Some((*dump, incomplete_conditions.clone())),
            _ => None,
        };
        let result = apply_scanned_step(
            scanned,
            stores,
            &mut self.modes,
            &mut self.next_alignment_identity,
            &mut self.active_alignment,
            &mut CommandMachine {
                state: &mut self.command,
                runtime: &mut self.runtime,
                fuel: self.fuel.fuel_mut(),
                capabilities: &mut self.capabilities,
                observations: &mut self.operation_observations,
                initex: self.initex,
            },
            &mut self.boxes,
            &mut self.prepared_dvi_pages,
        )?;
        if dumped_format {
            self.dumped_format = Some(crate::job::FormatDumpReceipt::new(
                self.capabilities.job_name().to_owned(),
                stores.int_param(IntParam::YEAR),
                stores.int_param(IntParam::MONTH),
                stores.int_param(IntParam::DAY),
            ));
        }
        if let (ReplayStep::End, Some((dump, incomplete_conditions))) = (&result, end_tail) {
            self.end_of_job_final_cleanup(stores, dump, incomplete_conditions);
        } else if matches!(result, ReplayStep::EndOfInput) {
            crate::job::prompt_for_more_input(stores, &self.startup_terminal_line);
        }
        self.resume_main_control_parking(parking, stores);
        if fires_afterassignment {
            schedule_afterassignment(
                &mut self.command,
                &mut self.runtime,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
                &mut self.operation_observations,
                stores,
            )?;
        }
        self.fire_pending_page_output(stores)?;
        self.page_output_observations.clear();
        if stores.world().artifact_commits().len() != artifact_count {
            self.completed_boundaries
                .push(crate::EngineBoundary::ShipoutComplete);
        }
        if outer_paragraph_was_active
            && self.modes.current_mode() == Mode::Vertical
            && self.modes.depth() == 1
        {
            self.completed_boundaries
                .push(crate::EngineBoundary::OuterParagraphEnd);
        }
        Ok(result)
    }

    /// Executes one command inside an aggregate host-owned episode.
    ///
    /// TeX82 §1211's `prefixed_command` remains the assignment dispatcher
    /// when §1228's numeric assignments occur inside a replayed math or
    /// discretionary field. If the enclosing operation is observed, route
    /// the nested command through the same executor-observation seam so its
    /// committed `word_define` is not reduced to command/scanner records.
    fn nested_step_once(
        &mut self,
        stores: &mut Universe,
        redispatch: Option<tex_command::CurrentCommand>,
    ) -> Result<ReplayStep, ExecError> {
        if self.operation_observations.is_some() {
            return self.step_with_observer_once(stores, redispatch);
        }
        self.step_once(stores, redispatch)
    }

    /// TeX82 §1123's `make_accent`.
    ///
    /// The accent's font is `cur_font` *before* §1270's `do_assignments`, and
    /// §1124 re-reads `cur_font` for the base character. That is the whole
    /// point of plain.tex's `\t`
    /// (``\def\t#1{{\edef\next{\the\font}\the\textfont1\accent"7F\next#1}}``):
    /// the tie accent comes from the math italic font `\the\textfont1`
    /// selected, and the base character from the text font `\next` restores.
    ///
    /// This is a host-owned step because §1270's loop body is
    /// `prefixed_command` -- it executes whole commands between the two
    /// scans -- and because tex.web executes each of them *in place*. A
    /// scanner that stopped on the assignment and backed it up instead would
    /// push a backup level, emit a recovery record and deliver the command
    /// twice (`umber2-johp.196`, `umber2-johp.264`).
    fn apply_accent(
        &mut self,
        scanned: ScannedAccent,
        stores: &mut Universe,
    ) -> Result<ReplayStep, ExecError> {
        if matches!(
            self.modes.current_mode(),
            Mode::Vertical | Mode::InternalVertical
        ) {
            start_canonical_paragraph(&mut self.command, &mut self.modes, stores, true)?;
        }
        crate::assignments::flush_pending_hchars_with_fuel(
            &mut self.modes,
            stores,
            self.fuel.fuel_mut(),
        )?;
        let accent = u8::try_from(scanned.accent).map_err(|_| ExecError::InvalidCode {
            context: "\\accent",
            value: scanned.accent,
        })?;
        let accent_font = stores.current_font();
        // §1123's `p:=new_character(f,cur_val); if p<>null then`: a missing
        // accent character skips `do_assignments` and the base lookahead
        // entirely, so nothing after this point runs.
        let Some(accent_metrics) = stores.font_char_metrics(accent_font, accent) else {
            report_missing_character(stores, accent_font, char::from(accent));
            return Ok(ReplayStep::Continue);
        };
        let base = self.do_assignments_then_accent_base(stores)?;
        apply_accent_nodes(
            &mut self.modes,
            stores,
            AccentPlacement {
                accent,
                accent_font,
                accent_metrics,
                accent_origin: scanned.accent_provenance.primary,
                base,
            },
        )
    }

    /// TeX82 §1270's `do_assignments` followed by §1124's classification of
    /// the token it stops on.
    ///
    /// §1270 is `loop begin <Get the next non-blank non-relax non-call token>;
    /// if cur_cmd<=max_non_prefixed_command then return; ...prefixed_command...
    /// end`, and §1124 then reads that same `cur_cmd`/`cur_chr` -- there is no
    /// second fetch and no `back_input` between them. Only §1124's own `else`
    /// branch backs a command up.
    fn do_assignments_then_accent_base(
        &mut self,
        stores: &mut Universe,
    ) -> Result<Option<(u8, tex_state::token::OriginId)>, ExecError> {
        // None of §1270's assignments is a §1030 `main_loop` entry.
        self.main_loop_active = false;
        loop {
            let outcome = {
                let mut processor = command_processor(
                    &mut self.command,
                    &mut self.runtime,
                    self.fuel.fuel_mut(),
                    &mut self.capabilities,
                    &mut self.operation_observations,
                    stores,
                );
                let outcome = processor.scan_accent_base();
                outcome.map_err(command_error)?
            };
            match outcome {
                ScannedAccentBase::Character {
                    character,
                    provenance,
                } => return Ok(Some((character, provenance.primary))),
                ScannedAccentBase::Missing => return Ok(None),
                ScannedAccentBase::Assignment(command) => {
                    // §1211's `prefixed_command` is exactly what main
                    // control's big case routes every code above
                    // `max_non_prefixed_command` to, so this dispatches the
                    // delivered command in place rather than re-fetching it.
                    match self.nested_step_once(stores, Some(command))? {
                        ReplayStep::Continue => {}
                        ReplayStep::End | ReplayStep::EndOfInput => return Ok(None),
                    }
                    self.main_loop_active = false;
                }
            }
        }
    }

    /// TeX82 §§1006--1028's typed page/output boundary.  The page builder
    /// and packing stay here with the mode nest; `CommandProcessor` alone
    /// installs the selected output token-list replay.
    ///
    /// Every step runs this, observed or not: §994's `build_page` and §1012's
    /// `fire_up` are part of executing the command that contributed, not an
    /// instrumentation-only extra.  The `\output` token-list push it performs
    /// is buffered rather than observed directly, so an observed step can
    /// flush it after that step's own mutation and effect records.
    fn fire_pending_page_output(&mut self, stores: &mut Universe) -> Result<(), ExecError> {
        while !self.boxes.output_routine_active {
            let Some(fire_up) = stores.page_fire_up() else {
                break;
            };
            match crate::output::select_pending_page_output(stores, fire_up)? {
                crate::output::SelectedPageOutput::Default(page) => {
                    let mut command = CommandMachine {
                        state: &mut self.command,
                        runtime: &mut self.runtime,
                        fuel: self.fuel.fuel_mut(),
                        capabilities: &mut self.capabilities,
                        observations: &mut self.operation_observations,
                        initex: self.initex,
                    };
                    if let Some(receipt) = shipout_replay_box(page, stores, &mut command)? {
                        push_prepared_dvi_page(&mut self.prepared_dvi_pages, receipt);
                    }
                }
                crate::output::SelectedPageOutput::UserRoutine => {
                    // This episode belongs to the step that contributed the
                    // page, but an observed step publishes it only after its
                    // own mutation and effect records.  Redirect the commit
                    // buffer for the episode's duration instead of letting
                    // this call site decide whether to observe at all.
                    let enclosing = self.operation_observations.take();
                    if enclosing.is_some() {
                        self.operation_observations = Some(ObservationBuffer::default());
                    }
                    let mut processor = command_processor(
                        &mut self.command,
                        &mut self.runtime,
                        self.fuel.fuel_mut(),
                        &mut self.capabilities,
                        &mut self.operation_observations,
                        stores,
                    );
                    processor
                        .retire_completed_right_brace_backup()
                        .map_err(command_error)?;
                    let opened = processor.begin_selected_output_routine();
                    let opened = opened.map_err(command_error);
                    if enclosing.is_some() {
                        let deferred =
                            std::mem::replace(&mut self.operation_observations, enclosing)
                                .unwrap_or_default();
                        self.page_output_observations.extend(deferred.0);
                    }
                    opened?;
                    stores.enter_group_with_kind_at_line(
                        GroupKind::Output,
                        self.command.current_file_line_number(),
                    );
                    self.modes.push(Mode::InternalVertical)?;
                    self.boxes.output_routine_active = true;
                    self.boxes.output_routine_opening_pending = true;
                }
            }
        }
        Ok(())
    }

    /// Runs one live `push_math` group to its closing brace.
    ///
    /// Both of TeX82's braced mlist openers work this way. §1153 is
    /// ``back_input; scan_left_brace; saved(0):=p; incr(save_ptr);
    /// push_math(math_group)`` and §1172/§1174 are
    /// ``push_math(math_choice_group); scan_left_brace``: in neither case is
    /// the body absorbed, it is ordinary input that main control reads, and
    /// the matching arm of `handle_right_brace` (§1186 for `math_group`,
    /// §1174's `build_choices` for `math_choice_group`) closes it on the
    /// matching `}`. The mandatory brace has already been consumed by the
    /// scanner that requested this group; this opens `push_math`'s save
    /// level and mode level and steps until the brace that closes *this*
    /// level arrives.
    fn execute_live_math_group(
        &mut self,
        kind: GroupKind,
        stores: &mut Universe,
    ) -> Result<tex_state::ids::NodeListId, ExecError> {
        // The depth sampled before `push_math`, not the innermost group
        // kind, is what identifies this group's own closing brace: a nested
        // subformula opens another `math_group`, and any brace group inside
        // the body opens a `simple_group`.
        let enclosing_depth = stores.group_depth();
        stores.enter_group_with_kind_at_line(kind, self.command.current_file_line_number());
        self.modes.push(Mode::Math)?;
        self.main_loop_active = false;
        while stores.group_depth() > enclosing_depth {
            match self.nested_step_once(stores, None)? {
                ReplayStep::End | ReplayStep::EndOfInput => {
                    return Err(ExecError::MissingToken {
                        context: "math group closing brace",
                    });
                }
                ReplayStep::Continue => {}
            }
        }
        self.finish_math_level(stores)
    }

    /// Closes any `\left` group TeX82 §1192 would have to recover, then pops
    /// the math mode level and finishes its mlist (§1184's `fin_mlist`).
    fn finish_math_level(
        &mut self,
        stores: &mut Universe,
    ) -> Result<tex_state::ids::NodeListId, ExecError> {
        self.main_loop_active = false;
        while canonical_left_group_open(&self.modes, stores) {
            // The `\right.` applied below is exactly the closer §1065 selects
            // for `math_left_group`, so the report is §1064's `off_save`.
            let context = self.command.output_open_context(&stores.command_context());
            report_escaped_error(
                stores,
                "Missing ",
                "right.",
                " inserted",
                &OFF_SAVE_HELP,
                context,
            )?;
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
        let level =
            crate::assignments::commit_current_list(&mut self.modes, stores, self.fuel.fuel_mut())?;
        finish_canonical_math_list(
            level.list().nodes(),
            level.list().incomplete_fraction(),
            stores,
        )
    }

    /// Opens and runs one `\mathchoice` branch: TeX82 §1172/§1174's
    /// ``push_math(math_choice_group); scan_left_brace`` followed by the live
    /// body main control reads until `build_choices` closes it.
    fn execute_math_choice_branch(
        &mut self,
        stores: &mut Universe,
    ) -> Result<tex_state::ids::NodeListId, ExecError> {
        self.command_scan_math_choice_group(stores)?;
        self.execute_live_math_group(GroupKind::MathChoice, stores)
    }

    /// Stores one completed TeX82 §1151 field.
    ///
    /// §1151 ends with `math_type(p):=math_char; character(p):=qi(c mod 256)`
    /// and §1151's own `fam` rule -- it never builds a noad, so `c`'s class
    /// bits are deliberately dropped here. The scalar case is a value, not
    /// deferred input: the command processor has already read, expanded, and
    /// classified everything the field consumed, so nothing is replayed and
    /// no input level is opened (`umber2-johp.265`).
    fn execute_math_field(
        &mut self,
        field: tex_command::MathFieldEpisode,
        stores: &mut Universe,
    ) -> Result<MathField, ExecError> {
        match field.body {
            MathFieldBody::Missing => Ok(MathField::Empty),
            MathFieldBody::Character(code) => Ok(MathField::MathChar(
                canonical_math_char(stores, u32::from(code), field.provenance.primary)?.1,
            )),
            MathFieldBody::OpenGroup => {
                let list = self.execute_live_math_group(GroupKind::Math, stores)?;
                Ok(collapse_singleton_math_group(stores, list))
            }
        }
    }

    fn apply_canonical_math_request(
        &mut self,
        request: CanonicalMathRequest,
        stores: &mut Universe,
    ) -> Result<ReplayStep, ExecError> {
        match request {
            CanonicalMathRequest::Character(value) => {
                append_canonical_math_char(
                    self.modes.current_list_mutation(),
                    stores,
                    u32::from(value.code),
                    value.provenance.primary,
                )?;
            }
            CanonicalMathRequest::Delimiter(value) => {
                append_canonical_math_char(
                    self.modes.current_list_mutation(),
                    stores,
                    value.code >> 12,
                    value.provenance.primary,
                )?;
            }
            CanonicalMathRequest::TextField(kind) => {
                let episode = self.command_scan_math_field(stores)?;
                let field = self.execute_math_field(episode, stores)?;
                // TeX82 §1186's second brace simplification: when a braced
                // field contains exactly one accent noad and is the nucleus
                // of an Ord atom, replace that Ord atom by the accent itself.
                // Following scripts must attach to the accent, not to a
                // wrapper whose converted nucleus and scripts become sibling
                // boxes.
                if kind == MathTextFieldKind::Ord
                    && let MathField::SubMlist(list) = field
                    && let [Node::MathNoad(accent)] = stores.nodes(list).to_vec().as_slice()
                    && matches!(accent.kind, NoadKind::Accent { .. })
                {
                    self.modes
                        .current_list_mutation()
                        .push(Node::MathNoad(accent.clone()));
                } else {
                    self.modes
                        .current_list_mutation()
                        .push(Node::MathNoad(MathNoad::new(
                            noad_kind_for_text(kind),
                            field,
                        )));
                }
            }
            CanonicalMathRequest::Script(script) => {
                let target = reserve_canonical_script_target(
                    self.modes.current_list_mutation(),
                    stores,
                    script.kind,
                )?;
                let episode = self.command_scan_math_field(stores)?;
                let field = self.execute_math_field(episode, stores)?;
                fill_canonical_script_target(self.modes.current_list_mutation(), target, field);
            }
            CanonicalMathRequest::Limits(kind) => {
                if !apply_canonical_limits(self.modes.current_list_mutation(), kind) {
                    // §1159 falls through to the error only when the tail is
                    // not an `op_noad`; the switch is dropped and the job
                    // continues.
                    let context = self.command.output_open_context(&stores.command_context());
                    let mut report = stores.print_err("Limit controls must follow a math operator");
                    report.help(&["I'm ignoring this misplaced \\limits or \\nolimits command."]);
                    report.context(context);
                    report.error().jump_out()?;
                }
            }
            CanonicalMathRequest::Fraction(fraction) => {
                start_canonical_fraction(self.modes.current_list_mutation(), stores, fraction)
            }
            CanonicalMathRequest::Style(style) => {
                self.modes
                    .current_list_mutation()
                    .push(Node::MathStyle(match style {
                        MathStyleKind::Display => MathStyle::Display,
                        MathStyleKind::Text => MathStyle::Text,
                        MathStyleKind::Script => MathStyle::Script,
                        MathStyleKind::ScriptScript => MathStyle::ScriptScript,
                    }))
            }
            CanonicalMathRequest::Choice => {
                // TeX82 §1172's `append_choices` opens the first branch with
                // `push_math(math_choice_group); scan_left_brace`, and
                // §1174's `build_choices` repeats exactly that after storing
                // each finished mlist. All four branches are therefore live
                // `math_choice_group` bodies read by ordinary main control,
                // never token lists absorbed ahead of construction: absorbing
                // them backs the opening brace up a second time (an extra
                // `backed_up` input level TeX never pushes) and reorders
                // every input level the branch body itself opens.
                let display = self.execute_math_choice_branch(stores)?;
                let text = self.execute_math_choice_branch(stores)?;
                let script = self.execute_math_choice_branch(stores)?;
                let script_script = self.execute_math_choice_branch(stores)?;
                self.modes
                    .current_list_mutation()
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
                    .current_list_mutation()
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
                    .current_list_mutation()
                    .push(Node::MathNoad(MathNoad::new(
                        NoadKind::Accent { accent },
                        field,
                    )));
            }
            CanonicalMathRequest::MuMaterial(ScannedMathMuMaterial::Glue(glue)) => {
                self.modes.current_list_mutation().push(Node::Glue {
                    spec: stores.intern_glue(glue),
                    kind: GlueKind::MuSkip,
                    leader: None,
                })
            }
            CanonicalMathRequest::MuMaterial(ScannedMathMuMaterial::Kern(amount)) => {
                self.modes.current_list_mutation().push(Node::Kern {
                    amount,
                    kind: KernKind::Mu,
                })
            }
            CanonicalMathRequest::EquationNumber(number) => {
                if self.modes.current_mode() == Mode::DisplayMath
                    && let Some((nodes, aux_prev_depth)) =
                        self.modes.current_list_mutation().take_display_alignment()
                {
                    // TeX82 §§812 and 1206 require the next non-assignment
                    // command after a display alignment to be `$$`. §1207's
                    // recovery inserts that closer before retrying the
                    // offending command, so `\eqno` must not consume the
                    // finished rows as an ordinary display mlist.
                    let context = self.command.output_open_context(&stores.command_context());
                    crate::error_report::report_error(
                        stores,
                        "Missing $$ inserted",
                        &[
                            "Displays can use special alignments (like \\eqalignno)",
                            "only if nothing but the alignment itself is between $$'s.",
                        ],
                        context,
                    )?;
                    self.finish_canonical_display_alignment(
                        stores,
                        crate::align::FinishedAlignment {
                            nodes,
                            aux_prev_depth,
                        },
                    )?;
                    // The retry §1207 arranges lands in the paragraph the
                    // display interrupted, where §1049 lists `eq_no` under
                    // `non_math` and so answers with `report_illegal_case`.
                    let primitive = match number.side {
                        tex_command::EquationNumberSide::Left => "leqno",
                        tex_command::EquationNumberSide::Right => "eqno",
                    };
                    let token = Token::Cs(stores.intern(primitive).symbol());
                    let context = self.command.output_open_context(&stores.command_context());
                    crate::diagnostics::report_illegal_case_with_context(
                        stores,
                        token,
                        Mode::Horizontal,
                        Some(context),
                    )?;
                    return Ok(ReplayStep::Continue);
                }
                if self.modes.current_mode() != Mode::DisplayMath {
                    // §1140's `mmode+eq_no` is guarded by `privileged`, and
                    // §1049 lists `eq_no` under `non_math`; both failures end
                    // in §1050's `report_illegal_case`, which names the mode
                    // the command was actually used in.
                    let primitive = match number.side {
                        tex_command::EquationNumberSide::Left => "leqno",
                        tex_command::EquationNumberSide::Right => "eqno",
                    };
                    let token = Token::Cs(stores.intern(primitive).symbol());
                    let mode = self.modes.current_mode();
                    let context = self.command.output_open_context(&stores.command_context());
                    crate::diagnostics::report_illegal_case_with_context(
                        stores,
                        token,
                        mode,
                        Some(context),
                    )?;
                } else {
                    let display = take_finished_canonical_math_list(&mut self.modes, stores)?;
                    stores.enter_group_with_kind_at_line(
                        GroupKind::MathShift,
                        self.command.current_file_line_number(),
                    );
                    stores.set_int_param(IntParam::FAM, -1);
                    self.modes.push(Mode::Math)?;
                    self.modes.current_list_mutation().set_display_eq_no(
                        crate::mode::DisplayEqNo {
                            side: match number.side {
                                tex_command::EquationNumberSide::Left => {
                                    crate::mode::EqNoSide::Left
                                }
                                tex_command::EquationNumberSide::Right => {
                                    crate::mode::EqNoSide::Right
                                }
                            },
                            display,
                        },
                    );
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
                crate::assignments::flush_pending_hchars_with_fuel(
                    &mut self.modes,
                    stores,
                    self.fuel.fuel_mut(),
                )?;
                // §1138 already applied its own `mode>0` test while probing:
                // in restricted horizontal mode the second `$` was backed up
                // rather than consumed, so `paired` is false there and this
                // must not retest the mode and disagree with the backup.
                if paired {
                    self.enter_canonical_display(stores)?;
                } else {
                    self.enter_canonical_math(false, stores)?;
                }
            }
            Mode::Math => {
                if self.modes.current_list().display_eq_no().is_some() {
                    if !paired {
                        report_unpaired_display_end(&self.command, stores)?;
                    }
                    self.finish_canonical_equation_number(stores)?;
                } else {
                    self.finish_canonical_inline_math(stores)?;
                }
            }
            Mode::DisplayMath => {
                if !paired {
                    report_unpaired_display_end(&self.command, stores)?;
                }
                self.finish_canonical_display_math(stores, None)?;
            }
            Mode::Vertical | Mode::InternalVertical => {
                unreachable!("vertical math shifts retry through ParagraphStart")
            }
        }
        Ok(ReplayStep::Continue)
    }

    fn enter_canonical_math(
        &mut self,
        display: bool,
        stores: &mut Universe,
    ) -> Result<(), ExecError> {
        stores.enter_group_with_kind_at_line(
            GroupKind::MathShift,
            self.command.current_file_line_number(),
        );
        stores.set_int_param(IntParam::FAM, -1);
        self.modes.push(if display {
            Mode::DisplayMath
        } else {
            Mode::Math
        })?;
        schedule_everymath(&mut self.command, stores, display);
        Ok(())
    }

    fn enter_canonical_display(&mut self, stores: &mut Universe) -> Result<(), ExecError> {
        let paragraph = crate::assignments::interrupt_canonical_paragraph_for_display(
            &mut self.modes,
            stores,
            self.fuel.fuel_mut(),
        )?;
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
        self.enter_canonical_math(true, stores)?;
        self.modes
            .current_list_mutation()
            .set_display_interrupt(crate::mode::DisplayInterrupt {
                active_directions: paragraph.active_directions,
            });
        Ok(())
    }

    fn finish_canonical_inline_math(&mut self, stores: &mut Universe) -> Result<(), ExecError> {
        let mut content = take_finished_canonical_math_list(&mut self.modes, stores)?;
        let math_font_context = self.command.output_open_context(&stores.command_context());
        if crate::math::reject_invalid_math_fonts(stores, math_font_context)? {
            content = stores.freeze_node_list(&[]);
        }
        let _ =
            crate::assignments::commit_current_list(&mut self.modes, stores, self.fuel.fuel_mut())?;
        let insert_penalties = self.modes.current_mode() == Mode::Horizontal;
        let (nodes, _) = crate::math::finish_inline_math_list_node(
            stores,
            tex_state::math::MathListNode {
                display: false,
                content,
            },
            insert_penalties,
        );
        self.modes.current_list_mutation().append(nodes);
        self.modes.current_list_mutation().set_space_factor(1000);
        let aftergroup = stores
            .leave_group_with_kind(GroupKind::MathShift)
            .map_err(|_| ExecError::MissingToken {
                context: "math shift group",
            })?;
        schedule_aftergroup(&mut self.command_machine(), stores, aftergroup)?;
        Ok(())
    }

    fn finish_canonical_equation_number(&mut self, stores: &mut Universe) -> Result<(), ExecError> {
        let mut content = take_finished_canonical_math_list(&mut self.modes, stores)?;
        let mut level =
            crate::assignments::commit_current_list(&mut self.modes, stores, self.fuel.fuel_mut())?;
        let mut eq = level
            .list_mutation()
            .take_display_eq_no()
            .expect("equation number mode state");
        let math_font_context = self.command.output_open_context(&stores.command_context());
        if crate::math::reject_invalid_math_fonts(stores, math_font_context)? {
            content = stores.freeze_node_list(&[]);
            eq.display = stores.freeze_node_list(&[]);
        }
        let finished = crate::math::display::finish_eq_no(stores, eq.side, content);
        let aftergroup = stores
            .leave_group_with_kind(GroupKind::MathShift)
            .map_err(|_| ExecError::MissingToken {
                context: "equation number group",
            })?;
        schedule_aftergroup(&mut self.command_machine(), stores, aftergroup)?;
        // TeX82 §1194's equation-number branch assigns `p:=fin_mlist(null)`
        // a second time after boxing `a`: the display must be finished from
        // the saved outer formula, not from the now-empty display mode list.
        self.finish_canonical_display_math_content(stores, eq.display, Some(finished))
    }

    fn finish_canonical_display_math(
        &mut self,
        stores: &mut Universe,
        eq_no: Option<crate::math::display::FinishedEqNo>,
    ) -> Result<(), ExecError> {
        // TeX82 §812 routes a display alignment to §§1206–1207 instead of
        // §1199's ordinary math-list lowering and hpack. The finished rows
        // are already vertical display material; math-packing them collapses
        // a multi-row alignment to the height and depth of one horizontal box.
        if let Some((nodes, aux_prev_depth)) =
            self.modes.current_list_mutation().take_display_alignment()
        {
            debug_assert!(eq_no.is_none());
            return self.finish_canonical_display_alignment(
                stores,
                crate::align::FinishedAlignment {
                    nodes,
                    aux_prev_depth,
                },
            );
        }
        let content = take_finished_canonical_math_list(&mut self.modes, stores)?;
        self.finish_canonical_display_math_content(stores, content, eq_no)
    }

    fn finish_canonical_display_alignment(
        &mut self,
        stores: &mut Universe,
        finished: crate::align::FinishedAlignment,
    ) -> Result<(), ExecError> {
        let mut level =
            crate::assignments::commit_current_list(&mut self.modes, stores, self.fuel.fuel_mut())?;
        let interrupt =
            level
                .list_mutation()
                .take_display_interrupt()
                .ok_or(ExecError::MissingToken {
                    context: "display alignment interrupt",
                })?;
        crate::math::display::finish_display_alignment(&mut self.modes, stores, finished)?;
        let aftergroup = stores
            .leave_group_with_kind(GroupKind::MathShift)
            .map_err(|_| ExecError::MissingToken {
                context: "display alignment group",
            })?;
        schedule_aftergroup(&mut self.command_machine(), stores, aftergroup)?;
        self.resume_canonical_display(stores, interrupt.active_directions)
    }

    fn finish_canonical_display_math_content(
        &mut self,
        stores: &mut Universe,
        mut content: tex_state::ids::NodeListId,
        eq_no: Option<crate::math::display::FinishedEqNo>,
    ) -> Result<(), ExecError> {
        // TeX82 §1194 performs this check before every display `fin_mlist`,
        // including the saved outer mlist after an equation number.
        let math_font_context = self.command.output_open_context(&stores.command_context());
        if crate::math::reject_invalid_math_fonts(stores, math_font_context)? {
            content = stores.freeze_node_list(&[]);
        }
        let mut level =
            crate::assignments::commit_current_list(&mut self.modes, stores, self.fuel.fuel_mut())?;
        let interrupt =
            level
                .list_mutation()
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
        schedule_aftergroup(&mut self.command_machine(), stores, aftergroup)?;
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
        self.modes.push(Mode::Horizontal)?;
        // §1200's `push_nest` sets `mode_line:=line` like every other one, so
        // the paragraph fragment that follows a display reports its own
        // over/underfull lines as §663's "in paragraph at lines A--B" rather
        // than falling back to "detected at line B" for want of a
        // `pack_begin_line`.
        stores.push_paragraph_start_line(stores.current_input_line());
        self.modes.current_list_mutation().set_space_factor(1000);
        self.modes
            .current_list_mutation()
            .append(directions.into_iter().map(Node::Direction));
        self.scan_canonical_optional_space(stores)?;
        crate::math::display::build_page_after_display_resume(&self.modes, stores)
    }

    /// TeX82 §443's `@<Scan an optional space@>`: `get_x_token; if
    /// cur_cmd<>spacer then back_input`.
    ///
    /// §1200's `resume_after_display` ends with this scan, after `push_nest`
    /// and before its `build_page`. Skipping it left the space that follows a
    /// closing `$$` to reach main control as ordinary interword glue, so the
    /// resumed paragraph was no longer null and §1096's `if head=tail then
    /// pop_nest {null paragraphs are ignored}` never fired: the enclosing
    /// vertical list gained an empty line box and its interline glue
    /// (`umber2-johp.231`). The scan is a plain `get_x_token`, so a macro
    /// following the display is expanded here exactly as TeX82 expands it.
    fn scan_canonical_optional_space(&mut self, stores: &mut Universe) -> Result<(), ExecError> {
        let mut machine = self.command_machine();
        let mut processor = machine.processor(stores);
        let fetched = processor.get_x_token();
        match fetched {
            Ok(Some(command))
                if !matches!(
                    command.meaning(),
                    Meaning::CharToken {
                        cat: Catcode::Space,
                        ..
                    }
                ) =>
            {
                processor.back_input(command).map_err(command_error)
            }
            Ok(_) => Ok(()),
            Err(err) => Err(command_error(err)),
        }
    }

    fn apply_canonical_math_delimiter(
        &mut self,
        boundary: MathDelimiterBoundary,
        stores: &mut Universe,
    ) -> Result<ReplayStep, ExecError> {
        match boundary.kind {
            MathDelimiterBoundaryKind::Left => {
                // TeX82 §1191's `push_math(math_left_group)` opens both a
                // mode level and a save-stack level. Keeping those owners
                // paired lets §1193 route a premature `$` through §1027's
                // `off_save`, which inserts `\right.` before retrying it.
                stores.enter_group_with_kind_at_line(
                    GroupKind::MathLeft,
                    self.command.current_file_line_number(),
                );
                self.modes.push(Mode::Math)?;
                self.modes
                    .current_list_mutation()
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
                        .current_list_mutation()
                        .push(Node::MathNoad(MathNoad::new(
                            NoadKind::MiddleDelimiter {
                                delimiter: boundary.delimiter.code,
                            },
                            MathField::Empty,
                        )));
                } else {
                    // etex.ch [48.1192] splits §1192's report by noad type.
                    let context = self.command.output_open_context(&stores.command_context());
                    report_escaped_error(
                        stores,
                        "Extra ",
                        "middle",
                        "",
                        &["I'm ignoring a \\middle that had no matching \\left."],
                        context,
                    )?;
                }
            }
            MathDelimiterBoundaryKind::Right => {
                if !canonical_left_group_open(&self.modes, stores) {
                    // TeX82 §1192's `<Try to recover from mismatched \right>`
                    // in its `math_shift_group` arm.
                    let context = self.command.output_open_context(&stores.command_context());
                    report_escaped_error(
                        stores,
                        "Extra ",
                        "right",
                        "",
                        &["I'm ignoring a \\right that had no matching \\left."],
                        context,
                    )?;
                    return Ok(ReplayStep::Continue);
                }
                let content = take_finished_canonical_math_list(&mut self.modes, stores)?;
                let _ = crate::assignments::commit_current_list(
                    &mut self.modes,
                    stores,
                    self.fuel.fuel_mut(),
                )?;
                let aftergroup =
                    stores
                        .leave_group_with_kind(GroupKind::MathLeft)
                        .map_err(|_| ExecError::MissingToken {
                            context: "math left group",
                        })?;
                schedule_aftergroup(&mut self.command_machine(), stores, aftergroup)?;
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
                    .current_list_mutation()
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
        let mut processor = command_processor(
            &mut self.command,
            &mut self.runtime,
            self.fuel.fuel_mut(),
            &mut self.capabilities,
            &mut self.operation_observations,
            stores,
        );
        let scanned = processor.scan_math_field_episode();
        scanned.map_err(command_error)
    }

    /// TeX82 §1172/§1174's `scan_left_brace` for one `\mathchoice` branch.
    /// §403 recovery opens the group anyway, so the recovered flag is
    /// diagnostic only.
    fn command_scan_math_choice_group(&mut self, stores: &mut Universe) -> Result<bool, ExecError> {
        let mut processor = command_processor(
            &mut self.command,
            &mut self.runtime,
            self.fuel.fuel_mut(),
            &mut self.capabilities,
            &mut self.operation_observations,
            stores,
        );
        let scanned = processor.scan_math_choice_group();
        scanned.map_err(command_error)
    }

    /// Delivers and executes one replay command while forwarding committed
    /// command-owned observations in their original order.
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
    pub fn advance_with_observer(
        &mut self,
        stores: &mut Universe,
        observer: &mut dyn CommandObserver,
    ) -> Result<CanonicalStepResult, ExecError> {
        if self.fatal.is_some() {
            return Ok(CanonicalStepResult::Progress(MainControlStep::End));
        }
        let geometry_start = observer.observes_geometry().then(|| {
            stores.enable_geometry_observation();
            stores.geometry_observation_len()
        });
        let snapshot = self.snapshot_step(stores);
        // Occupying the slot is what makes this operation observed. Every
        // command-processor episode the operation runs, including the nested
        // ones a host-applied step runs, publishes into this one buffer.
        self.operation_observations = Some(ObservationBuffer::default());
        let stepped = self.step_with_observer_once(stores, None);
        let mut pending = self.operation_observations.take().unwrap_or_default();
        if let Some(geometry_start) = geometry_start {
            pending.0.extend(
                stores
                    .geometry_observations_since(geometry_start)
                    .iter()
                    .copied()
                    .map(Self::geometry_observation),
            );
        }
        match stepped {
            Ok(step) => {
                self.commit_step(snapshot);
                pending.flush_into(observer);
                Ok(CanonicalStepResult::Progress(step))
            }
            Err(error) => {
                if let Some(fatal) = error.as_fatal() {
                    // §81 `jump_out` does not undo anything the job already
                    // committed, so the partial step stands; the observations
                    // it published are flushed ahead of the fatal record.
                    self.commit_step(snapshot);
                    pending.flush_into(observer);
                    let step = self.succumb(fatal);
                    observer.committed(CommandObservation::Diagnostic(fatal.record()));
                    observer.committed(CommandObservation::Effect(engine_termination_effect()));
                    return Ok(CanonicalStepResult::Progress(step));
                }
                if !snapshot.can_rollback(stores) {
                    // tex.web §283's `unsave` consumes the enclosing save
                    // level. An error reached after that exit cannot restore
                    // the pre-operation group timeline; preserve the state
                    // TeX has already committed and report the real error.
                    self.commit_step(snapshot);
                    return Err(error);
                }
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

    fn geometry_observation(observation: GeometryObservation) -> CommandObservation {
        let record = match observation {
            GeometryObservation::Hpack {
                width_sp,
                height_sp,
                depth_sp,
            } => GeometryRecord::Hpack {
                width_sp,
                height_sp,
                depth_sp,
            },
            GeometryObservation::Vpack {
                width_sp,
                height_sp,
                depth_sp,
            } => GeometryRecord::Vpack {
                width_sp,
                height_sp,
                depth_sp,
            },
            GeometryObservation::Shipout {
                page_width_sp,
                page_height_sp,
                counts,
            } => GeometryRecord::Shipout {
                page_width_sp,
                page_height_sp,
                counts,
            },
        };
        CommandObservation::Geometry(record)
    }

    /// TeX82 §93's `succumb`: `history:=fatal_error_stop; jump_out`.
    ///
    /// `jump_out` cuts across every active procedure level and lands at
    /// `end_of_TEX`, where §1332's `close_files_and_terminate` finishes the
    /// job. A library engine has no process to leave, so the driver -- the
    /// only frame that corresponds to `end_of_TEX` -- latches the terminal
    /// state and reports the job over. Nothing is rolled back: `jump_out`
    /// abandons the current procedure, it does not undo it.
    fn succumb(&mut self, fatal: FatalError) -> MainControlStep {
        self.fatal.get_or_insert(fatal);
        MainControlStep::End
    }

    /// The fatal error that ended this session, if §93's `succumb` ran.
    ///
    /// `Some` is exactly TeX82 §76's `history=fatal_error_stop`: the job did
    /// not run to `\end`, and no further operation will deliver a command.
    #[must_use]
    pub const fn fatal_error(&self) -> Option<FatalError> {
        self.fatal
    }

    fn step_with_observer_once(
        &mut self,
        stores: &mut Universe,
        redispatch: Option<tex_command::CurrentCommand>,
    ) -> Result<ReplayStep, ExecError> {
        // Observation is an instrumentation boundary, not an alternate
        // execution mode. Keep the command processor's borrowed mode facts
        // identical to an unobserved step (notably for \ifhmode after a
        // paragraph-start transition).
        if self.enter_main_control(stores) {
            // §1030's prologue precedes `big_switch`, so its push is published
            // ahead of the first command this step delivers rather than with
            // the step's own applied records.
            let entry_records: Vec<CommandObservation> = self
                .command
                .take_named_token_list_push_observations()
                .into_iter()
                .map(CommandObservation::Input)
                .collect();
            self.observe_committed(entry_records);
        }
        self.drain_file_framing_events(stores);
        self.refresh_host_capabilities(stores);
        let mode = self.modes.current_mode();
        let alignment_preamble = alignment_preamble(self.active_alignment.as_mut());
        let innermost_group = stores.innermost_group_kind();
        let job_is_all_over = crate::output::job_is_all_over(stores);
        let mut diagnostics = Vec::new();
        let scanned = {
            let mut processor = command_processor(
                &mut self.command,
                &mut self.runtime,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
                &mut self.operation_observations,
                stores,
            );
            let scanned = match redispatch {
                Some(command) => dispatch_main_control_command(
                    &mut processor,
                    command,
                    mode,
                    &self.boxes,
                    innermost_group,
                    job_is_all_over,
                    self.modes.current_list().display_eq_no().is_some(),
                    &mut diagnostics,
                )?,
                None => scan_replay_step(
                    &mut processor,
                    mode,
                    &self.boxes,
                    alignment_preamble,
                    innermost_group,
                    job_is_all_over,
                    self.modes.current_list().display_eq_no().is_some(),
                    self.main_loop_active,
                    &mut diagnostics,
                )?,
            };
            diagnostics.extend(
                processor
                    .take_semantic_diagnostics()
                    .into_iter()
                    .map(PendingDiagnostic::Command),
            );
            scanned
        };
        // tex.web's `line` is maintained by `get_next` as it moves to a new
        // input line, so it is already the delivered command's own line by
        // the time that command is applied. Publish it here, after delivery,
        // rather than at the step's start: §660/§675's box diagnostics and
        // §1091's `mode_line` both name the line the command is *on*, and a
        // command that is the first thing on a line is scanned by a step
        // that began on the previous one.
        stores.set_current_input_line(
            i32::try_from(self.command.current_file_line_number()).unwrap_or(i32::MAX),
        );
        report_pending_diagnostics(stores, diagnostics, &mut self.shown_mode)?;
        self.drain_file_framing_events(stores);
        let scanned = self.resolve_font_resource(scanned)?;
        let scanned = self.resolve_input_stream_resource(scanned)?;
        let scanned = self.resolve_pdf_image_resource(scanned, stores)?;
        let parking = self.suspend_main_control_parking(&scanned);
        let artifact_count = stores.world().artifact_commits().len();
        let effect_count = stores.world().effect_records().len();
        let prepared_page_count = self.prepared_dvi_pages.len();
        let scanned = match self.apply_host_owned_step(scanned, stores) {
            ControlFlow::Break(applied) => {
                return self.finish_host_owned_step(
                    applied,
                    artifact_count,
                    effect_count,
                    prepared_page_count,
                    stores,
                );
            }
            ControlFlow::Continue(scanned) => scanned,
        };
        let scanned = match scanned {
            ScannedStep::ShowGroups { diagnostic: None } => ScannedStep::ShowGroups {
                diagnostic: Some(detached_showgroups(
                    stores,
                    &self.modes,
                    &self.active_alignment,
                    &self.boxes,
                )),
            },
            scanned => scanned,
        };
        let mutation = applied_mutation_observation(&scanned, stores, self.command_profile());
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
                self.command.alignment_cell_finish_observation(*alignment)
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
            &mut CommandMachine {
                state: &mut self.command,
                runtime: &mut self.runtime,
                fuel: self.fuel.fuel_mut(),
                capabilities: &mut self.capabilities,
                observations: &mut self.operation_observations,
                initex: self.initex,
            },
            &mut self.boxes,
            &mut self.prepared_dvi_pages,
        );
        if result.is_ok() && self.initex && matches!(scanned, ScannedStep::End { dump: true, .. }) {
            self.dumped_format = Some(crate::job::FormatDumpReceipt::new(
                self.capabilities.job_name().to_owned(),
                stores.int_param(IntParam::YEAR),
                stores.int_param(IntParam::MONTH),
                stores.int_param(IntParam::DAY),
            ));
        }
        if let (
            Ok(ReplayStep::End),
            ScannedStep::End {
                dump,
                incomplete_conditions,
            },
        ) = (&result, &scanned)
        {
            self.end_of_job_final_cleanup(stores, *dump, incomplete_conditions.clone());
        } else if matches!(result, Ok(ReplayStep::EndOfInput)) {
            crate::job::prompt_for_more_input(stores, &self.startup_terminal_line);
        }
        if result.is_ok() {
            self.resume_main_control_parking(parking, stores);
        }
        if result.is_ok() {
            self.fire_pending_page_output(stores)?;
        }
        let extra_tab_recovery = result
            .as_ref()
            .ok()
            .filter(|_| completes_alignment_cell)
            .and_then(|_| self.command.take_alignment_extra_tab_recovery_observation());
        if result.is_ok() {
            // These records are produced by applying the step, after the
            // command-processor episode's own borrow has ended. They are
            // collected first and appended to the operation's commit buffer
            // in one place, so that the buffer is never borrowed while
            // command state is still being read.
            let mut records: Vec<CommandObservation> = Vec::new();
            // tex.web observes a named token-list level inside
            // `begin_token_list`, which runs in the middle of the transition
            // that installs it (`new_graf`, `box_end`, `init_math`). Command
            // state holds the push until the transition's borrow ends, so it
            // is published ahead of the transition's own committed records.
            records.extend(
                self.command
                    .take_named_token_list_push_observations()
                    .into_iter()
                    .map(CommandObservation::Input),
            );
            let effects = committed_stream_effect_observations(
                effect_count,
                prepared_page_count,
                stores,
                &self.prepared_dvi_pages,
            );
            let effect = applied_effect_observation(&scanned, stores);
            if suspends_alignment
                && let Some(alignment) = self.command.alignment_suspend_observation()
            {
                records.push(CommandObservation::Alignment(alignment));
            }
            if begins_alignment && let Some(alignment) = self.command.alignment_begin_observation()
            {
                records.push(CommandObservation::Alignment(alignment));
            }
            if begins_alignment_cell
                && let Some(alignment) = self.command.alignment_cell_begin_observation()
            {
                records.push(CommandObservation::Alignment(alignment));
            }
            if let Some(alignment) = installs_u_template
                && let Some(input) = self
                    .command
                    .alignment_u_template_push_observation(alignment)
            {
                records.push(CommandObservation::Input(input));
                if let Some(template) = self
                    .command
                    .alignment_u_template_push_alignment_observation(alignment)
                {
                    records.push(CommandObservation::Alignment(template));
                }
            }
            if let Some(alignment) = installs_omit_cell
                && let Some(omit) = self.command.alignment_omit_cell_observation(alignment)
            {
                records.push(CommandObservation::Alignment(omit));
            }
            if let Some(recovery) = extra_tab_recovery {
                records.push(CommandObservation::Alignment(recovery));
            }
            if let Some(finish) = finishes_alignment_cell {
                records.push(CommandObservation::Alignment(finish));
            }
            if let Some(finish) = finishes_alignment {
                records.push(CommandObservation::Alignment(finish));
                if let Some(resume) = self.command.alignment_resume_observation() {
                    records.push(CommandObservation::Alignment(resume));
                }
            }
            if let Some(protected) = protected_macro_definition_observation(&scanned, stores) {
                records.push(CommandObservation::TokenList(protected));
            }
            if let Some(mutation) = mutation {
                records.push(CommandObservation::Mutation(mutation.resolve(stores)));
            }
            // §1378's live-file closes are part of termination and precede
            // the replay driver's synthetic terminal marker. Other command
            // effects retain their established command-before-host-delta
            // ordering.
            if matches!(scanned, ScannedStep::End { .. }) {
                records.extend(effects.into_iter().map(CommandObservation::Effect));
                if let Some(effect) = effect {
                    records.push(CommandObservation::Effect(effect));
                }
            } else {
                if let Some(effect) = effect {
                    records.push(CommandObservation::Effect(effect));
                }
                records.extend(effects.into_iter().map(CommandObservation::Effect));
            }
            for shipout in committed_shipout_observations(artifact_count, stores) {
                records.push(CommandObservation::Effect(shipout));
            }
            records.append(&mut self.page_output_observations);
            self.observe_committed(records);
        }
        // TeX82 §1211 commits the assignment inside its case arm, then
        // reaches §1269's `done:` and `back_input`. Publish the mutation
        // before the replay-level push for that saved token.
        if result.is_ok() && fires_afterassignment {
            schedule_afterassignment(
                &mut self.command,
                &mut self.runtime,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
                &mut self.operation_observations,
                stores,
            )?;
        }
        self.page_output_observations.clear();
        result
    }

    /// Scans TeX's initial terminal filename through the canonical command
    /// path, retaining every committed observation for the caller.
    pub fn scan_startup_file_name(
        &mut self,
        stores: &mut Universe,
        observer: &mut dyn CommandObserver,
    ) -> Result<String, ExecError> {
        self.operation_observations = Some(ObservationBuffer::default());
        let scanned = self.scan_startup_file_name_once(stores);
        self.operation_observations
            .take()
            .unwrap_or_default()
            .flush_into(observer);
        scanned
    }

    fn scan_startup_file_name_once(&mut self, stores: &mut Universe) -> Result<String, ExecError> {
        let filename =
            {
                let mut processor = command_processor(
                    &mut self.command,
                    &mut self.runtime,
                    self.fuel.fuel_mut(),
                    &mut self.capabilities,
                    &mut self.operation_observations,
                    stores,
                );
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
        // retires (TeX82 §46 final cleanup).  Vacating the slot is what makes
        // this one episode silent, and it is deliberate rather than an
        // omitted observer at the construction site.
        let silenced = self.operation_observations.take();
        let mut processor = command_processor(
            &mut self.command,
            &mut self.runtime,
            self.fuel.fuel_mut(),
            &mut self.capabilities,
            &mut self.operation_observations,
            stores,
        );
        let exhausted = processor.get_x_token();
        let exhausted = exhausted.map_err(command_error);
        self.operation_observations = silenced;
        let terminal_exhausted = exhausted?.is_none();
        if !terminal_exhausted {
            return Err(ExecError::MissingToken {
                context: "terminal filename terminator",
            });
        }
        self.capabilities.set_startup_job_name(&filename);
        self.startup_terminal_line.clone_from(&filename);
        Ok(filename)
    }
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
    mut list: crate::mode::ModeListMutation<'_>,
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

/// TeX82 §1155's `set_math_char`, entered from every §1154 case that derives
/// its code from a character's `math_code`.
///
/// ```text
/// procedure set_math_char(@!c:integer);
/// begin if c>=@'100000 then
///   @<Treat |cur_chr| as an active character@>
/// else  begin p:=new_noad; ... end;
/// end;
/// ```
///
/// The `c>=@'100000` branch is not a diagnostic or a discard: §1155's own
/// commentary says "the |cur_chr| is treated as an active character and
/// nothing is appended", and §1152 then expands that active character in
/// place and backs its result up for main control to reread. Plain TeX's
/// ``\mathcode`\'="8000`` is the reason the branch exists at all, so a math
/// list built without it silently loses every `\prime`.
///
/// Only §1154's `letter`/`other_char`/`char_given`/`char_num` cases can
/// reach the branch: §1224's `\mathchardef` and §436's `scan_fifteen_bit_int`
/// bound `\mathchar` and `\mathaccent` to fifteen bits, and §437's
/// `scan_twenty_seven_bit_int` bounds `\delimiter`'s `cur_val div @'10000`
/// to the same range, so those callers append unconditionally.
fn set_canonical_math_char(
    ch: char,
    origin: tex_state::token::OriginId,
    stores: &mut Universe,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_>,
) -> Result<(), ExecError> {
    let code = stores.mathcode(ch);
    if code >= 0x8000 {
        let mut processor = command.processor(stores);
        let treated = processor.treat_as_active_character(ch, origin);
        treated.map_err(command_error)?;
        return Ok(());
    }
    append_canonical_math_char(modes.current_list_mutation(), stores, code, origin)
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
    }
}

/// tex.web §687's `scripts_allowed(#)==(type(#)>=ord_noad)and(type(#)<left_noad)`.
///
/// The bound admits exactly the noad types from `ord_noad` through
/// `vcenter_noad` and excludes everything else in an mlist: every ordinary
/// node type (glue, kern, penalty, rule, disc, whatsit, ...) and both
/// `style_node` and `choice_node` sort below `ord_noad`, while `left_noad`
/// and `right_noad` sort at or above the upper bound.  e-TeX's `\middle`
/// (etex.ch's `middle_noad`) is a `right_noad` carrying a distinguishing
/// `subtype`, so the same bound excludes it without a separate test.
fn canonical_scripts_allowed(node: &Node) -> bool {
    match node {
        Node::MathNoad(noad) => !matches!(
            noad.kind,
            NoadKind::LeftDelimiter { .. }
                | NoadKind::RightDelimiter { .. }
                | NoadKind::MiddleDelimiter { .. }
        ),
        _ => false,
    }
}

pub(crate) fn canonical_script_field_mut(
    noad: &mut MathNoad,
    kind: MathScriptKind,
) -> &mut MathField {
    match kind {
        MathScriptKind::Superscript => &mut noad.superscript,
        MathScriptKind::Subscript => &mut noad.subscript,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CanonicalScriptTarget {
    pub(crate) node_index: usize,
    kind: MathScriptKind,
}

/// tex.web §1176's `sub_sup`.
///
/// ```text
/// begin t:=empty; p:=null;
/// if tail<>head then if scripts_allowed(tail) then
///   begin p:=supscr(tail)+cur_cmd-sup_mark; t:=math_type(p); end;
/// if (p=null)or(t<>empty) then <Insert a dummy noad to be sub/superscripted>;
/// scan_math(p);
/// ```
///
/// The returned index is the Rust counterpart of §1176's pointer `p`.
/// Reserving it before §1151's `scan_math` is observable: §1177's diagnostic
/// precedes every side effect of the field scan, and material appended while a
/// recovered or nested field executes cannot move the eventual attachment to a
/// newer tail.
pub(crate) fn reserve_canonical_script_target(
    mut list: crate::mode::ModeListMutation<'_>,
    stores: &mut Universe,
    kind: MathScriptKind,
) -> Result<CanonicalScriptTarget, ExecError> {
    // `t<>empty`: the tail was eligible but already carries this script.
    let tail_index = list.nodes().len().checked_sub(1);
    let (eligible, occupied) = match tail_index.and_then(|index| list.nodes().get(index)) {
        Some(node) if canonical_scripts_allowed(node) => {
            let Node::MathNoad(noad) = node else {
                unreachable!("canonical_scripts_allowed admits only noads")
            };
            let occupied = match kind {
                MathScriptKind::Superscript => !matches!(noad.superscript, MathField::Empty),
                MathScriptKind::Subscript => !matches!(noad.subscript, MathField::Empty),
            };
            (true, occupied)
        }
        _ => (false, false),
    };

    let node_index = if eligible && !occupied {
        tail_index.expect("eligible tail has an index")
    } else {
        let index = list.nodes().len();
        list.push(Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::Empty,
        )));
        index
    };

    if occupied {
        let (message, help) = match kind {
            MathScriptKind::Superscript => (
                "Double superscript",
                "I treat `x^1^2' essentially like `x^1{}^2'.",
            ),
            MathScriptKind::Subscript => (
                "Double subscript",
                "I treat `x_1_2' essentially like `x_1{}_2'.",
            ),
        };
        let mut report = stores.print_err(message);
        report.help(&[help]);
        report.error().jump_out()?;
    }

    Ok(CanonicalScriptTarget { node_index, kind })
}

pub(crate) fn fill_canonical_script_target(
    mut list: crate::mode::ModeListMutation<'_>,
    target: CanonicalScriptTarget,
    field: MathField,
) {
    list.with_node_mut(target.node_index, |node| {
        let Node::MathNoad(noad) = node else {
            unreachable!("reserved canonical script target must remain a noad")
        };
        let reserved = canonical_script_field_mut(noad, target.kind);
        debug_assert!(matches!(reserved, MathField::Empty));
        *reserved = field;
    })
    .expect("reserved canonical script target must remain present");
}

fn apply_canonical_limits(
    mut list: crate::mode::ModeListMutation<'_>,
    kind: MathLimitKind,
) -> bool {
    // TeX82 §1159's `math_limit_switch`: the subtype is set only when
    // `head<>tail` *and* the tail is an `op_noad`. `with_last_node_mut`
    // returns `None` for the empty list, which is `head=tail`.
    list.with_last_node_mut(|node| {
        let Node::MathNoad(noad) = node else {
            return false;
        };
        if !matches!(
            noad.kind,
            NoadKind::Normal(NoadClass::Op) | NoadKind::Operator(_)
        ) {
            return false;
        }
        noad.kind = NoadKind::Operator(match kind {
            MathLimitKind::Limits => LimitType::Limits,
            MathLimitKind::NoLimits => LimitType::NoLimits,
            MathLimitKind::DisplayLimits => LimitType::DisplayLimits,
        });
        true
    })
    .unwrap_or(false)
}

fn start_canonical_fraction(
    mut list: crate::mode::ModeListMutation<'_>,
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
        // TeX82 §§1185 and 1191–1192: `\left` owns the math-left group, while
        // an incomplete fraction owns only the material after its opening
        // delimiter.  Keep that structural delimiter outside the fraction
        // noad when the fraction is completed before `\right`.
        let mut numerator_nodes: Vec<_> = stores
            .nodes(fraction.numerator)
            .into_iter()
            .map(|node| node.to_owned())
            .collect();
        let leading_left = matches!(
            numerator_nodes.first(),
            Some(Node::MathNoad(MathNoad {
                kind: NoadKind::LeftDelimiter { .. },
                ..
            }))
        )
        .then(|| numerator_nodes.remove(0));
        let numerator = if leading_left.is_some() {
            stores.freeze_node_list(&numerator_nodes)
        } else {
            fraction.numerator
        };
        let fraction = Node::FractionNoad(MathFraction {
            numerator,
            denominator,
            thickness: fraction.thickness,
            left_delimiter: fraction.left_delimiter,
            right_delimiter: fraction.right_delimiter,
        });
        output = leading_left.into_iter().chain([fraction]).collect();
    }
    Ok(stores.freeze_node_list(&output))
}

/// TeX82 §1186's `math_group` singleton-Ord simplification.
///
/// After §1153 has tentatively classified a braced field as `sub_mlist`,
/// `handle_right_brace` removes braces around exactly one undecorated Ord
/// noad by copying its nucleus field into the destination. This preserves an
/// author box as `sub_box` instead of wrapping it in a second natural hpack.
fn collapse_singleton_math_group(stores: &Universe, list: tex_state::ids::NodeListId) -> MathField {
    let mut nodes = stores.nodes(list).into_iter();
    if let Some(tex_state::node_arena::NodeRef::MathNoad(noad)) = nodes.next()
        && nodes.next().is_none()
        && noad.kind == NoadKind::Normal(NoadClass::Ord)
        && matches!(noad.subscript, MathField::Empty)
        && matches!(noad.superscript, MathField::Empty)
    {
        return noad.nucleus.clone();
    }
    MathField::SubMlist(list)
}

fn take_finished_canonical_math_list(
    modes: &mut ModeNest,
    stores: &mut Universe,
) -> Result<tex_state::ids::NodeListId, ExecError> {
    let (nodes, incomplete) = {
        let mut list = modes.current_list_mutation();
        (list.take_nodes(), list.take_incomplete_fraction())
    };
    finish_canonical_math_list(&nodes, incomplete.as_ref(), stores)
}

/// TeX82 §1064's `off_save` help, shared by all four closers §1065 selects.
const OFF_SAVE_HELP: [&str; 5] = [
    "I've inserted something that you may have forgotten.",
    "(See the <inserted text> above.)",
    "With luck, this will get me unwedged. But if you",
    "really didn't forget anything, try typing `2' now; then",
    "my insertion and my current dilemma will both disappear.",
];

/// [`crate::error_report::report_error`] for a message tex.web assembles as
/// `print_err(prefix)`, §63's `print_esc(escaped)`, and `print(suffix)`.
///
/// Spelling the control sequence with `print_esc` rather than a literal
/// backslash is what keeps the report honest under a changed `\escapechar`.
fn report_escaped_error(
    stores: &mut Universe,
    prefix: &str,
    escaped: &str,
    suffix: &str,
    help: &[&str],
    context: String,
) -> Result<(), ExecError> {
    let mut report = stores.print_err(prefix);
    report.print_esc(escaped).print(suffix);
    report.help(help).context(context);
    report.error().jump_out()?;
    Ok(())
}

/// TeX82 §1084's `scan_box` recovery for a command that is not a box.
///
/// §1084 reports through `back_error`, and every caller here has already had
/// the rejected command backed up during scanning, so only the report is
/// left.
fn report_missing_box(command: &CommandState, stores: &mut Universe) -> Result<(), ExecError> {
    let context = command.output_open_context(&stores.command_context());
    crate::error_report::report_error(
        stores,
        "A <box> was supposed to be here",
        &[
            "I was expecting to see \\hbox or \\vbox or \\copy or \\box or",
            "something like that. So you might find something missing in",
            "your output. But keep trying; you can fix this later.",
        ],
        context,
    )?;
    Ok(())
}

/// TeX82 §1082's `scan_keyword("to")` recovery in `\vsplit`.
fn report_missing_vsplit_to(
    command: &CommandState,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    let context = command.output_open_context(&stores.command_context());
    crate::error_report::report_error(
        stores,
        "Missing `to' inserted",
        &[
            "I'm working on `\\vsplit<box number> to <dimen>';",
            "will look for the <dimen> next.",
        ],
        context,
    )?;
    Ok(())
}

/// TeX82 §1197's `<Check that another `$` follows>`.
///
/// §1197 reaches this through `back_error`, and the scanner's probe
/// (`scan_display_end_math_shift`) has already put the offending token back,
/// so only the report itself is left to issue.
fn report_unpaired_display_end(
    command: &CommandState,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    let context = command.output_open_context(&stores.command_context());
    crate::error_report::report_error(
        stores,
        "Display math should end with $$",
        &[
            "The `$' that I just saw supposedly matches a previous `$$'.",
            "So I shall assume that you typed `$$' both times.",
        ],
        context,
    )?;
    Ok(())
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

/// TeX82 §1030's parking decision for one scanned step, taken before the step
/// is applied and spent after it.
///
/// It is taken early because applying a step can run nested command episodes,
/// and those start at `big_switch` however the enclosing step was fetched.
#[derive(Clone, Copy)]
struct MainControlParking {
    /// The character this step appended, if it was one of §1030's four
    /// `main_loop` entries.
    character: Option<char>,
    /// Whether the step was not a `main_control` case at all, but §342's
    /// resumption of a `get_next` that is still in progress. Such a step
    /// leaves parking exactly as it found it.
    resumes_interrupted_fetch: bool,
}

/// The closer TeX82 §1065 selects for `cur_group`, in the form its report
/// prints it: `print_esc` for the two frozen control sequences, `print_char`
/// for the two literal characters.
#[derive(Clone, Copy)]
enum OffSaveCloser {
    EndGroup,
    MathShift,
    NullRight,
    RightBrace,
}

impl OffSaveCloser {
    fn print(self, report: &mut tex_state::print::ErrorReport<'_>) {
        match self {
            Self::EndGroup => report.print_esc("endgroup"),
            Self::MathShift => report.print_char('$'),
            Self::NullRight => report.print_esc("right."),
            Self::RightBrace => report.print_char('}'),
        };
    }
}

/// TeX82 §1069's `case cur_group of`: the group opener a stray `}` was
/// probably standing in for.
///
/// This is deliberately not [`OffSaveCloser`]. §1064 inserts a closer and says
/// what it inserted, so its `math_left_group` arm is `\right.` -- a complete
/// command. §1069 deletes the brace and only names what was forgotten, so its
/// arm is the bare `\right`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ForgottenGroupOpener {
    /// `semi_simple_group`.
    EndGroup,
    /// `math_shift_group`.
    MathShift,
    /// `math_left_group`.
    Right,
}

impl ForgottenGroupOpener {
    fn print(self, report: &mut tex_state::print::ErrorReport<'_>) {
        match self {
            Self::EndGroup => report.print_esc("endgroup"),
            Self::MathShift => report.print_char('$'),
            Self::Right => report.print_esc("right"),
        };
    }
}

#[derive(Clone)]
enum ScannedStep {
    Continue,
    Relax,
    /// e-TeX 2.6 `etex.ch` [17.3822--3880]'s `hmode+valign` extension:
    /// the four nonzero `valign` modifiers are text-direction nodes when
    /// `\TeXXeTstate>0`; otherwise `eTeX_enabled` diagnoses and ignores them.
    TextDirection {
        direction: tex_state::node::Direction,
        enabled: bool,
    },
    AlignPeekRestart {
        alignment: AlignmentIdentity,
    },
    /// TeX82 §370's undefined-control-sequence expansion error. The gullet
    /// delivers this only because its ordinary expanded-command loop has not
    /// yet claimed `Meaning::Undefined`; main control still preserves §370's
    /// observable report and drop behavior explicitly.
    UndefinedControlSequence,
    /// TeX82 §1128's `abs(align_state)>2` recovery: report the delivered
    /// delimiter and drop it without a backup or inserted brace.
    MisplacedAlignmentDelimiter {
        token: Token,
    },
    /// TeX82 §342 has just run §789's
    /// ``@<Insert the ⟨v_j⟩ template and |goto restart|@>``.
    ///
    /// This is not a `main_control` step at all: §789 runs *inside* `get_next`
    /// and jumps back to its own `restart` label, so the fetch that triggered
    /// it is still in progress. Umber routes the push out to the executor
    /// because the alignment's identity lives here, but must not treat the
    /// round trip as a return to §1030's `big_switch` -- main control is still
    /// parked wherever it was, and §1030's two fetch labels disagree about the
    /// `end_template` that a ⟨v_j⟩ template is about to deliver: §380's
    /// `get_x_token` rewrites it to `endv` in place, while §1038's `x_token`
    /// reaches §366 `expand` and §375 backs up a separate `frozen_endv` token
    /// to be reread.
    AlignmentTemplateEntered,
    MissingMathShift,
    ReplayCompleted(tex_command::CommandReplayEpisode),
    Math(CanonicalMathRequest),
    MathDelimiter(MathDelimiterBoundary),
    MathFamily {
        family: tex_command::ScannedMathFamily,
        font: FontId,
        global: bool,
    },
    EndOfInput,
    /// TeX82 §1045's `vmode+stop: if its_all_over then return` -- the only
    /// exit from `main_control`. §1335's `final_cleanup` has already unwound
    /// the abandoned input stack, so the job-termination effect is this
    /// step's, not a later input-exhaustion step's.
    ///
    /// §1335's `c` -- 0 for `\end`, 1 for `\dump` -- selects the whole tail
    /// of `final_cleanup`, so it is carried rather than discarded.
    End {
        dump: bool,
        incomplete_conditions: Vec<tex_command::IncompleteCondition>,
    },
    /// TeX82 §1051's `privileged` failure for `\\end`/`\\dump` in internal
    /// vertical mode (`mode<0`): `report_illegal_case`, and the job keeps
    /// running. Shares the recovery shape of `IllegalBoxShift` and friends.
    IllegalStop {
        token: Token,
    },
    /// TeX82 §1045's `any_mode(mac_param): report_illegal_case`. A bare
    /// parameter command is diagnosed and discarded in every mode; it does
    /// not terminate main control.
    IllegalMacroParameter {
        token: Token,
    },
    /// TeX82 §1135's `cs_error`: a stray `\endcsname` is diagnosed and
    /// ignored exactly once, without changing input or mode state.
    ExtraEndCsName,
    /// TeX82 §1054's `its_all_over` false branch: the backed-up stop stays
    /// live while `\\hbox to \\hsize{}`, `\\vfill`, and
    /// `\\penalty-'10000000000` are appended to the contribution list and
    /// §994's `build_page` runs. Whether that fires `\\output` is §1005's
    /// decision.
    EjectResidualPage,
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
    /// TeX82 §1055's `assign_box_dimen: alter_box_dimen(cur_chr)` (`\wd`,
    /// `\ht`, `\dp`): `scan_eight_bit_int`, `scan_optional_equals`, then
    /// `scan_normal_dimen` set the given dimension on the box register's
    /// visible node if it is not void; a void or absent register is a
    /// documented no-op (§1055 skips the mutation entirely rather than
    /// erroring), matched by `Universe::set_box_dimension`'s own early
    /// return.
    BoxDimensionAssignment {
        index: u16,
        dimension: tex_state::BoxDimension,
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
    VerticalSkip {
        value: GlueSpec,
    },
    Kern {
        amount: Scaled,
    },
    /// TeX82 §1102's `any_mode(break_penalty): append_penalty` -- `\penalty`
    /// is legal in every mode (vertical, horizontal, and math alike) with no
    /// mode switch of its own, unlike `\hskip`/`\kern`'s vertical-mode
    /// paragraph-start recovery above.
    Penalty {
        amount: i32,
    },
    CharacterCode {
        value: i32,
        suppress_left_boundary: bool,
    },
    /// TeX82 §1105's `any_mode(remove_item): delete_last` -- `\unpenalty`,
    /// `\unkern`, and `\unskip` are legal in every mode with no scan of their
    /// own (the removed node, if any, is selected purely by matching the
    /// primitive against the current list's tail).
    DeleteLast(UnexpandablePrimitive),
    /// TeX82 §1264's `new_interaction`: `\batchmode`/`\nonstopmode`/
    /// `\scrollmode`/`\errorstopmode` carry no operand of their own -- the
    /// target `InteractionMode` is selected from the delivered primitive at
    /// apply time, mirroring `DeleteLast` above.
    SetInteractionMode(UnexpandablePrimitive),
    /// e-TeX 2.6 etex.ch §3736's assignable `\interactionmode` primitive.
    SetInteractionModeValue(i32),
    /// TeX82 §1112's `hmode+ital_corr: append_italic_correction` (the
    /// procedure itself is §1113) or its math-mode twin (§1112's
    /// `mmode+ital_corr: tail_append(new_kern(0))`); which applies is
    /// resolved from the live mode at apply time since neither takes an
    /// operand. Vertical mode is instead `IllegalItalicCorrection` below
    /// (`\/` is one of §1111's "Forbidden cases", not a paragraph-starting
    /// command).
    ItalicCorrection,
    /// TeX82 §1111's "Forbidden cases" `you_cant`/`report_illegal_case`
    /// recovery for `\/` in vertical or internal-vertical mode
    /// (`vmode+ital_corr`).
    IllegalItalicCorrection {
        token: Token,
    },
    /// TeX82 §1038's lookahead consumes `no_boundary` after a character run
    /// by setting `bchar:=non_char`, suppressing only that run's right
    /// boundary processing. The §1030 big-switch occurrence has different
    /// semantics and is folded into its following command during scanning.
    /// §1045's math-mode occurrence is a no-op.
    NoBoundary {
        suppress_right: bool,
    },
    /// TeX82 §1171's `mmode+non_script: tail_append(new_glue(zero_glue));
    /// subtype(tail):=cond_math_glue`. Legal only in math/display-math mode;
    /// §1046's `non_math(non_script)` routes every other mode through
    /// `MissingMathShift` instead (`scan_command` selects between the two
    /// before this step is produced).
    NonScript,
    /// TeX82 §1030's `hmode+ex_space,mmode+ex_space: goto append_normal_space`
    /// and §1090's `vmode+ex_space: back_input; new_graf(true)` -- `\ ` (the
    /// explicit control-space primitive) starts a paragraph in vertical mode
    /// exactly like a letter, then always appends the plain interword glue
    /// regardless of the current space factor.
    ControlSpace,
    /// TeX82 §1243's `set_aux` assignment (`alter_aux`) for the vertical-mode
    /// modifier: `\prevdepth=<dimen>` sets the enclosing vertical list's
    /// `prev_depth`, the field `append_to_vlist` (§679) reads to decide
    /// baselineskip/lineskip insertion before the next box. Legal only in
    /// vertical or internal-vertical mode; §1243's `report_illegal_case`
    /// (`abs(mode)<>vmode`) otherwise leaves the value alone.
    PrevDepth {
        value: Scaled,
    },
    /// TeX82 §1243's `set_aux` assignment for the horizontal-mode modifier:
    /// `\spacefactor=<number>` sets the current horizontal list's space
    /// factor. Legal only in horizontal or restricted-horizontal mode;
    /// §1243's `report_illegal_case` (`abs(mode)<>hmode`) otherwise leaves
    /// the value alone. A scanned value outside `1..32767` is a "Bad space
    /// factor" diagnostic that likewise leaves the space factor unchanged.
    SpaceFactor {
        value: i32,
    },
    /// TeX82 §1243 checks `cur_chr<>abs(mode)` before calling either
    /// `scan_optional_equals` or `scan_int`. Thus an illegal-mode
    /// `\spacefactor` reports `report_illegal_case` while preserving the
    /// very next token as an ordinary main-control command.
    IllegalSpaceFactor {
        token: Token,
    },
    /// TeX82 §1244's `set_prev_graf` (`alter_prev_graf`): `\prevgraf=<number>`
    /// is `any_mode` and walks the mode nest up to its nearest enclosing
    /// vertical level (`while abs(nest[p].mode_field)<>vmode do decr(p)`),
    /// setting that level's `prev_graf` (paragraph count so far) directly --
    /// unlike `\spacefactor`/`\prevdepth`, it never reports an illegal case. A
    /// negative scanned value is a "Bad \prevgraf" diagnostic that leaves the
    /// count unchanged.
    PrevGraf {
        value: i32,
    },
    /// TeX82 §1242's `set_page_dimen: alter_page_so_far`, whose body is
    /// §1245: `c:=cur_chr; scan_optional_equals; scan_normal_dimen;
    /// page_so_far[c]:=cur_val`. This is `\pagegoal`, `\pagetotal`,
    /// `\pagestretch`, `\pagefilstretch`, `\pagefillstretch`,
    /// `\pagefilllstretch`, `\pageshrink`, and `\pagedepth`.
    ///
    /// There is deliberately no `global` field: §1242 states outright that
    /// "these definitions are always global", and `page_so_far` is a plain
    /// engine array rather than an `eqtb` entry, so neither the `\global`
    /// prefix nor `\globaldefs` can reach it and no save-stack entry is
    /// pushed. `PrevDepth`/`SpaceFactor`/`PrevGraf` above and
    /// `BoxDimensionAssignment` below are scoped identically for the same
    /// reason.
    PageDimension {
        dimension: PageDimension,
        value: Scaled,
    },
    /// TeX82 §1242's `set_page_int: alter_integer`, whose body is §1246:
    /// `c:=cur_chr; scan_optional_equals; scan_int; if c=0 then
    /// dead_cycles:=cur_val else insert_penalties:=cur_val`. This is
    /// `\deadcycles` and `\insertpenalties`.
    ///
    /// Unscoped for the same §1242 reason as `PageDimension` above.
    PageInteger {
        integer: PageInteger,
        value: i32,
    },
    FixedHorizontalGlue {
        primitive: UnexpandablePrimitive,
    },
    FixedVerticalGlue {
        primitive: UnexpandablePrimitive,
    },
    ParagraphIndent {
        indent: bool,
    },
    ParagraphShape {
        lines: Vec<ParagraphShapeLine>,
        global: bool,
    },
    PenaltyArray {
        kind: PenaltyArrayKind,
        values: Vec<i32>,
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
    /// pdftex.web §1585's `\pdfsetrandomseed`: the command scanner has
    /// consumed one ordinary integer and normalized its sign; application
    /// replaces the ungrouped job RNG state atomically.
    PdfSetRandomSeed {
        seed: i32,
    },
    /// pdftex.web §1586's `\pdfresettimer`: there is no operand; application
    /// atomically rebases the ungrouped job timer to the deterministic
    /// monotonic sample already held by `World`.
    PdfResetTimer,
    /// pdftex.web §§1594–1596's operand-free interword-space controls.
    /// Application appends an ordered whatsit after flushing any pending
    /// horizontal character run; shipout traversal owns the toggle state.
    PdfInterwordSpace(tex_state::node::PdfAccessibilityControl),
    /// pdftex.web §§1597–1598's operand-free running-link shipout controls.
    /// Application appends an ordered whatsit; PDF traversal owns the
    /// initially-enabled policy for continuation annotations.
    PdfRunningLink(bool),
    /// pdftex.web §1599's expanded balanced text selecting the global,
    /// job-owned fallback font name used by accessible-space PDF output.
    PdfSpaceFont(TracedTokenList),
    PdfGraphics(PdfGraphicsRequest),
    PdfObject(PdfObjectRequest),
    PdfReferenceObject(PdfReferenceObjectRequest),
    PdfForm(PdfFormRequest),
    PdfDocumentFragment(PdfDocumentFragmentRequest),
    PdfNavigation(PdfNavigationRequest),
    FontDimen {
        font: FontId,
        /// tex.web §578's `n`, unrecovered. A number at or below zero
        /// resolves to §578's scratch `fmem_ptr` and is reported by §579, so
        /// the scan must carry it rather than reject it.
        number: i32,
        value: Scaled,
        /// §82's `show_context` at the point §578 decided the number was
        /// unusable -- after the font identifier, before `=<dimen>`. `None`
        /// when §578 accepted it, which is also what says no §579 report is
        /// due.
        recovery_context: Option<String>,
    },
    FontInteger {
        font: FontId,
        skew: bool,
        value: i32,
    },
    DeferredOpenOut {
        stream: u8,
        file_name: String,
    },
    DeferredCloseOut {
        stream: tex_command::WriteStreamSelector,
    },
    DeferredWrite {
        stream: tex_command::WriteStreamSelector,
        tokens: TracedTokenList,
    },
    DeferredSpecial {
        tokens: TracedTokenList,
    },
    /// TeX82 §1377's `@<Implement \setlanguage@>`, reached from §1348's
    /// `do_extension` on `set_language_code` (§1344's `extension` modifier
    /// 5). Unlike §1376's `fix_language`, which appends a §1341
    /// `language_node` only when the language actually changes, §1377
    /// appends one unconditionally -- `\setlanguage` is an explicit request,
    /// so a same-language `\setlanguage` still produces a whatsit.
    ///
    /// `language` is `cur_val` exactly as `scan_int` left it; §1377's own
    /// `<=0`/`>255` normalization to `clang` is performed at the apply seam
    /// together with `norm_min` (§1091), because both write mode-nest and
    /// list state the scan phase does not own.
    SetLanguage {
        language: i32,
    },
    /// TeX82 §1377's `if abs(mode)<>hmode then report_illegal_case`.
    ///
    /// The mode test precedes both `new_whatsit` and `scan_int`, so an
    /// out-of-mode `\setlanguage` scans no operand at all -- matching
    /// `IllegalBoxShift`/`IllegalInsertOrAdjust`'s same-shaped recovery, and
    /// unlike `\prevdepth`'s §1243 check, which runs after its value is
    /// scanned.
    IllegalSetLanguage {
        token: Token,
    },
    Arithmetic {
        primitive: UnexpandablePrimitive,
        target: ArithmeticTarget,
        operand: ArithmeticOperand,
        global: bool,
    },
    /// TeX82 §1236's recoverable invalid-target return from
    /// `do_register_command`. The target command has been consumed, but no
    /// operand is scanned and no value is changed.
    InvalidArithmeticTarget {
        primitive: UnexpandablePrimitive,
        target: tex_command::PrintCommand,
    },
    MacroDefinition {
        target: Symbol,
        flags: MeaningFlags,
        global: bool,
        parameter_text: TracedTokenList,
        replacement_text: TracedTokenList,
        definition_origin: tex_state::token::OriginId,
        missing_target: bool,
    },
    CharacterDefinition {
        primitive: UnexpandablePrimitive,
        target: Symbol,
        /// `cur_val` after §434/§436's recover-to-zero.
        value: i32,
        global: bool,
    },
    /// TeX82 §1252's `hyph_data` command: `\patterns` (`chr_code=1`) installs
    /// pattern data through §960's `new_patterns`; `\hyphenation`
    /// (`chr_code=0`) installs exception words through §934's
    /// `new_hyph_exceptions`. The scan carries §935's raw exception words or
    /// §962's normalized pattern specs; the flag selects which table they
    /// populate.
    HyphenationData {
        words: Vec<Vec<char>>,
        pattern_specs: Vec<tex_state::hyphenation::PatternSpec>,
        patterns: bool,
        /// §82's context for whichever of the two `\patterns` rejections the
        /// apply seam raises. Both report before the braced group is read --
        /// §960's `trie_not_ready=false` branch before §473 discards it, and
        /// §1252's production branch before its own `repeat get_token` flush --
        /// so the context has to be captured at the pre-scan cursor, with
        /// `\patterns` behind it and the group still ahead.
        rejection_context: String,
        /// Whether §960's trie was already built, which is the half of the
        /// rejection test the command core can see. The other half -- whether
        /// tex.web's `init`/`tini` split would have produced this binary at
        /// all -- is the session's, and is applied with it.
        trie_built: bool,
    },
    RegisterDefinition {
        primitive: UnexpandablePrimitive,
        target: Symbol,
        index: u16,
        global: bool,
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
    /// TeX82 §1293's `show_lists_code` branch of `show_whatever`, which takes
    /// no operand: `begin_diagnostic; show_activities`. The mode is carried
    /// so the committed effect can name the nest it reported.
    ShowLists,
    /// e-TeX 2.6 `etex.ch` [17.3623--3671]'s `\\showtokens`: command
    /// processing has removed the compulsory braces and frozen the
    /// unexpanded balanced interior. Replay owns only diagnostic rendering.
    ShowTokens {
        tokens: TracedTokenList,
    },
    /// e-TeX 2.6 `etex.ch` [17.3703--3732]'s read-only conditional-stack
    /// diagnostic, detached in innermost-to-outermost traversal order.
    ShowIfs {
        conditions: Vec<tex_command::ActiveCondition>,
    },
    /// e-TeX 2.6 [49.1292]'s operand-free, any-mode `\showgroups`.
    ShowGroups {
        diagnostic: Option<crate::diagnostics::ShowGroupsDiagnostic>,
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
    /// e-TeX 2.6 `etex.ch` [45.999]'s operand-free extensions of TeX82's
    /// `un_vbox` command. The selected saved list is detached and spliced
    /// into the current list atomically; unlike `\unvbox`, no register
    /// number is scanned.
    SavedVerticalDiscards(UnexpandablePrimitive),
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
        packing: ScannedPackingSpec,
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
    /// TeX82 §1068's `handle_right_brace` for a brace the current group
    /// cannot account for. `forgotten` is §1069's `case cur_group of` -- the
    /// opener the brace was probably standing in for -- and `None` is §1068's
    /// own `bottom_level` arm, "Too many }'s".
    ExtraRightBrace {
        forgotten: Option<ForgottenGroupOpener>,
    },
    /// TeX82 §1186: the closing brace of a `math_group` opened by §1153's
    /// `push_math`, or §1174's `build_choices` closing a `math_choice_group`
    /// opened by §1172/§1174. Applying it only `unsave`s; the nested loop in
    /// `execute_live_math_group` notices the level is gone and finishes the
    /// mlist.
    EndMathGroup(GroupKind),
    /// TeX82 §1064's general `off_save`: the innermost group could not
    /// accommodate the scanned command, so `scan_off_save` already chose and
    /// inserted the matching closer ahead of the backed-up command
    /// (`CommandProcessor::recover_off_save`). The execute phase only prints
    /// §1064's report naming what was inserted; the closer travels as the
    /// typed choice §1065 already made rather than as rendered text, because
    /// two of the four spell a control sequence and so must be printed
    /// through §63's `print_esc` under the live `\\escapechar`.
    OffSave(OffSaveCloser),
    /// TeX82 §§1064/1066's bottom-level `off_save`: no enclosing group
    /// existed to close, so the offending command was dropped outright
    /// (`CommandProcessor::report_off_save_bottom_drop` already ran); the
    /// execute phase prints "Extra `<command>`" naming its own spelling.
    OffSaveBottomDrop {
        token: Token,
    },
    BeginOrdinaryGroup,
    EndOrdinaryGroup,
    OutputRoutineOpeningBrace,
    EndOutputRoutine,
    AlignmentPeekCell {
        alignment: AlignmentIdentity,
        omit: bool,
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
    /// TeX82 §1073's box-shift prefixes (`\raise`, `\lower`, `\moveleft`,
    /// `\moveright`): the already-signed shift amount (tex.web's
    /// `box_context`) plus `scan_box`'s own `make_box` operand (§1084).
    /// `ScannedBoxShiftPayload::Construction` shares the `BeginBox`/
    /// `BoxEndGroup` body-closing machinery; the other
    /// variants resolve to a node immediately, exactly like `\box<n>`,
    /// `\lastbox`, and `\vsplit` do outside a shift.
    BoxShift(ScannedBoxShift),
    /// TeX82's "Forbidden cases" `you_cant`/`report_illegal_case` recovery
    /// for a box-shift prefix used in the wrong mode (`vmode+vmove`,
    /// `hmode+hmove`, `mmode+hmove`): the dimension is never scanned, unlike
    /// `\prevdepth`'s mode check, which runs only after its value is
    /// scanned.
    IllegalBoxShift {
        token: Token,
    },
    /// TeX82 §1099's `begin_insert_or_adjust` for `\insert` and `\vadjust`.
    /// The class-number bound check (0..=255) and the reserved-255 recovery
    /// both need `Universe` diagnostics, so replay receives only the raw
    /// scanned class (fixed at 255 for `\vadjust`); the body's mandatory
    /// opening brace was already consumed by `scan_left_brace`. It shares the
    /// same brace-matching machinery as `BeginBox`/`BoxEndGroup`.
    BeginInsert(ScannedInsertConstruction),
    /// TeX82's "Forbidden cases" `vmode+vadjust`: `\vadjust` never reaches
    /// its mandatory `scan_left_brace` in vertical mode, matching
    /// `IllegalBoxShift`/`IllegalItalicCorrection`'s same-shaped recovery.
    IllegalInsertOrAdjust {
        token: Token,
    },
    /// TeX82 §1144's `@<Forbidden cases@>=non_math(eq_no)` (added to the
    /// shared Forbidden-cases list first built at §1048): `\eqno`/`\leqno`
    /// outside math mode take `report_illegal_case` ("You can't use
    /// `\eqno' in ... mode") rather than §1047's `insert_dollar_sign`, even
    /// though tex.web registers them under the same `eq_no` command code as
    /// the math-request vocabulary `scan_canonical_math_request` otherwise
    /// dispatches. Reaching this arm proves `mode` is not
    /// `Math`/`DisplayMath` (that gate would have consumed the primitive
    /// first via `Request::EquationNumber`), matching
    /// `IllegalBoxShift`/`IllegalItalicCorrection`/`IllegalInsertOrAdjust`'s
    /// same-shaped recovery. `mmode+eq_no` itself (gated by
    /// `privileged`/`cur_group`) is unaffected.
    IllegalEqNo {
        token: Token,
    },
    /// TeX82 §1048's `@<Forbidden cases@>=...,any_mode(last_item),...` (the
    /// same module `IllegalBoxShift`'s `vmode+vmove`/`hmode+hmove`/
    /// `mmode+hmove` triple comes from, and that `IllegalEqNo`'s §1144
    /// addition later extends): `\lastpenalty`, `\lastkern`, and `\lastskip`
    /// have no assignment form and no standalone typesetting meaning in any
    /// mode -- they are legal only as an internal-value operand inside a
    /// scan (`CommandProcessor::internal_value_from_command`'s `LastPenalty`/
    /// `LastKern`/`LastSkip` arms). Reaching main control with one of these
    /// as the delivered command therefore always means `report_illegal_case`,
    /// matching `IllegalBoxShift`/`IllegalInsertOrAdjust`/`IllegalEqNo`'s
    /// same-shaped recovery.
    IllegalLastItem {
        token: Token,
        context: String,
    },
    BoxEndGroup {
        ships_out: bool,
    },
    /// TeX82 §1101 and e-TeX 2.6 `etex.ch` [26.424]'s `make_mark`: a fully
    /// expanded balanced general text, appended as the selected mark class.
    Mark {
        class: u16,
        tokens: TracedTokenList,
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
        suppress_left_boundary: bool,
    },
    Accent(ScannedAccent),
    DiscretionaryOpening(ScannedDiscretionaryOpening),
    DiscretionaryPartEnd,
    DiscretionaryHyphen {
        origin: tex_state::token::OriginId,
    },
}

impl ScannedStep {
    const fn fires_afterassignment(&self) -> bool {
        matches!(
            self,
            Self::Count { .. }
                | Self::Dimen { .. }
                | Self::BoxDimensionAssignment { .. }
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
                | Self::InvalidArithmeticTarget { .. }
                | Self::MacroDefinition { .. }
                | Self::CharacterDefinition { .. }
                | Self::RegisterDefinition { .. }
                | Self::Let { .. }
                | Self::ParagraphShape { .. }
                | Self::PenaltyArray { .. }
                | Self::FontSelect { .. }
                | Self::MathFamily { .. }
                | Self::SetBox(..)
                | Self::PrevDepth { .. }
                | Self::SpaceFactor { .. }
                | Self::PrevGraf { .. }
                | Self::PageDimension { .. }
                | Self::PageInteger { .. }
                | Self::HyphenationData { .. }
                | Self::SetInteractionMode(..)
        )
    }

    /// The character TeX82 §1030 hands to §1034's `main_loop`, if this step
    /// is one of its four entries: `hmode+letter`, `hmode+other_char`,
    /// `hmode+char_given`, or `hmode+char_num`.
    ///
    /// These are the only cases of the big `case` statement that do not end
    /// at `goto big_switch`. The mode and font tests §1030/§1036 also impose
    /// are applied by the caller against the state the step *left* behind,
    /// because §1090's `vmode+letter` starts a paragraph first and only then
    /// reaches the same `main_loop`.
    fn main_loop_character(&self) -> Option<char> {
        match *self {
            Self::Character {
                ch,
                cat: Catcode::Letter | Catcode::Other,
                ..
            } => Some(ch),
            Self::CharacterCode { value, .. } => u32::try_from(value).ok().and_then(char::from_u32),
            _ => None,
        }
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
    boxes: &ReplayBoxes,
    alignment_preamble: Option<(AlignmentIdentity, AlignmentPreamblePhase)>,
    innermost_group: Option<GroupKind>,
    job_is_all_over: bool,
    display_eq_no: bool,
    main_loop_active: bool,
    diagnostics: &mut Vec<PendingDiagnostic>,
) -> Result<ScannedStep, ExecError> {
    if let Some((alignment, phase)) = alignment_preamble {
        return match phase {
            AlignmentPreamblePhase::Opening => {
                let packing = processor
                    .scan_alignment_preamble_opening()
                    .map_err(command_error)?;
                Ok(ScannedStep::AlignmentPreambleOpening { alignment, packing })
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
            AlignmentPreamblePhase::NoAlignBody => scan_noalign_body(
                processor,
                alignment,
                boxes,
                innermost_group,
                mode,
                job_is_all_over,
                diagnostics,
            ),
            AlignmentPreamblePhase::CellDelivery => scan_alignment_delivery_step(
                processor,
                alignment,
                boxes,
                innermost_group,
                mode,
                job_is_all_over,
                main_loop_active,
                diagnostics,
            ),
        };
    }
    scan_step(
        processor,
        mode,
        boxes,
        innermost_group,
        job_is_all_over,
        display_eq_no,
        main_loop_active,
        diagnostics,
    )
}

#[derive(Clone, Copy)]
enum AlignmentPreamblePhase {
    Opening,
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
    } else if active.noalign_open {
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
    _after_noalign: bool,
) -> Result<ScannedStep, ExecError> {
    processor
        .begin_alignment_peek(_after_noalign)
        .map_err(command_error)?;
    let (command, pending_expanded_delivery) = processor
        .next_alignment_lookahead()
        .map_err(command_error)?
        .ok_or(ExecError::MissingToken {
            context: "alignment lookahead",
        })?;
    match command.meaning() {
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::NoAlign) => {
            if pending_expanded_delivery {
                processor.commit_alignment_lookahead_delivery(&command);
            }
            processor
                .scan_alignment_noalign_opening()
                .map_err(command_error)?;
            Ok(ScannedStep::BeginNoAlign { alignment })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::CrCr) => {
            if pending_expanded_delivery {
                processor.commit_alignment_lookahead_delivery(&command);
            }
            Ok(ScannedStep::AlignPeekRestart { alignment })
        }
        Meaning::CharToken {
            cat: Catcode::EndGroup,
            ..
        } => {
            if pending_expanded_delivery {
                processor.commit_alignment_lookahead_delivery(&command);
            }
            Ok(ScannedStep::AlignmentFinish { alignment })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Omit) => {
            if pending_expanded_delivery {
                processor.commit_alignment_lookahead_delivery(&command);
            }
            Ok(ScannedStep::AlignmentPeekCell {
                alignment,
                omit: true,
            })
        }
        _ => {
            processor
                .back_alignment_lookahead(command, pending_expanded_delivery)
                .map_err(command_error)?;
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
    job_is_all_over: bool,
    diagnostics: &mut Vec<PendingDiagnostic>,
) -> Result<ScannedStep, ExecError> {
    let Some(command) = processor.get_x_token().map_err(command_error)? else {
        return Ok(ScannedStep::EndOfInput);
    };
    queue_command_trace(processor, mode, &command, diagnostics);
    match command.meaning() {
        Meaning::CharToken {
            cat: Catcode::EndGroup,
            ..
        } if innermost_group == Some(GroupKind::NoAlign) => {
            Ok(ScannedStep::NoAlignEndGroup { alignment })
        }
        // A `\noalign` body is ordinary main control between its braces
        // (TeX82 §785's `no_align_group`), so it dispatches through the same
        // §1030 `reswitch:`/§1211 prefix path as any other step.
        _ => dispatch_main_control_command(
            processor,
            command,
            mode,
            boxes,
            innermost_group,
            job_is_all_over,
            false,
            diagnostics,
        ),
    }
}

/// Delivers one active cell command through the command-owned alignment
/// boundary.  This remains separate from preamble and opener scans because a
/// completed scanner (such as a rule specification) can leave a backed-up
/// delimiter ready for the next main-control step.
#[allow(clippy::too_many_arguments)] // carries command-owned replay facts
fn scan_alignment_delivery_step(
    processor: &mut CommandProcessor<'_>,
    alignment: AlignmentIdentity,
    boxes: &ReplayBoxes,
    innermost_group: Option<GroupKind>,
    mode: Mode,
    job_is_all_over: bool,
    main_loop_active: bool,
    diagnostics: &mut Vec<PendingDiagnostic>,
) -> Result<ScannedStep, ExecError> {
    match processor
        .get_x_alignment_delivery(main_loop_active)
        .map_err(command_error)?
    {
        None => Ok(ScannedStep::EndOfInput),
        // An executor-owned replay episode (a math field/group/choice branch
        // or discretionary part) retired mid-cell. This must be reported
        // exactly like ordinary `scan_step`'s `ReplayCompleted` case, rather
        // than falling through to interpret whatever the cascade found next
        // as this cell's own content: that next token can belong to the
        // *enclosing* cell/field context, not the just-retired episode.
        Some(AlignmentDelivery::Completed(episode)) => Ok(ScannedStep::ReplayCompleted(episode)),
        Some(AlignmentDelivery::Command(command)) => {
            queue_command_trace(processor, mode, &command, diagnostics);
            // TeX82 §1132 dispatches every right brace seen with an active
            // `align_group` through the missing-\cr recovery, independent of
            // `align_state`. The command-owned fast path emits a structural
            // ClosingBrace event at the ordinary cell depth, but §1096's
            // `off_save` can insert the brace after `align_state` is already
            // negative. That brace must still back up behind frozen `\cr`;
            // treating it as an ordinary extra brace makes `\par` repeat
            // `off_save` forever while recovery input levels accumulate.
            if innermost_group == Some(GroupKind::Align)
                && matches!(
                    command.meaning(),
                    Meaning::CharToken {
                        cat: Catcode::EndGroup,
                        ..
                    }
                )
            {
                processor
                    .recover_alignment_closing_brace(
                        tex_command::AlignmentDeliveryEvent::ClosingBrace(command),
                    )
                    .map_err(command_error)?;
                return Ok(ScannedStep::Continue);
            }
            if matches!(command.meaning(), Meaning::EndV) {
                // TeX82 §§1046-1047 route `mmode+endv` through
                // `insert_dollar_sign`, just like every other command that
                // reaches an alignment v-template before its math mode has
                // closed. The synthesized `$` closes math first; the backed
                // up `endv` is then redelivered in the cell's h/v mode and
                // reaches §1131 below.
                if matches!(mode, Mode::Math | Mode::DisplayMath) {
                    processor
                        .recover_missing_math_shift(command)
                        .map_err(command_error)?;
                    return Ok(ScannedStep::MissingMathShift);
                }
                // Replay's structural alignment group is deliberately not a
                // Universe group: the surrounding box owns that stack slot.
                // A recovery-opened simple group is the bounded exception
                // that TeX82 §1131 must close through `off_save` first.
                if boxes.recovery_simple_group_open {
                    return scan_off_save(processor, command, innermost_group);
                }
                return Ok(ScannedStep::AlignmentCellFinish { alignment });
            }
            // An alignment cell's body is ordinary main control bounded by
            // §1130's `vmode+endv,hmode+endv: do_endv`, not a dispatcher of
            // its own, so it takes
            // the same §1030 `reswitch:`/§1211 prefix path as any other step.
            dispatch_main_control_command(
                processor,
                command,
                mode,
                boxes,
                innermost_group,
                job_is_all_over,
                false,
                diagnostics,
            )
        }
        Some(AlignmentDelivery::Event(event)) => {
            match event {
                tex_command::AlignmentDeliveryEvent::EndTemplate(_) => {
                    processor
                        .begin_alignment_v_template(alignment, event)
                        .map_err(command_error)?;
                    return Ok(ScannedStep::AlignmentTemplateEntered);
                }
                tex_command::AlignmentDeliveryEvent::ClosingBrace(_) => {
                    // TeX82 §1132 selects this executor-owned align_group
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
    boxes: &ReplayBoxes,
    innermost_group: Option<GroupKind>,
    job_is_all_over: bool,
    display_eq_no: bool,
    main_loop_active: bool,
    diagnostics: &mut Vec<PendingDiagnostic>,
) -> Result<ScannedStep, ExecError> {
    // TeX82 §1030 has two fetch labels, not one. `big_switch` uses
    // `get_x_token`; §1034's inner character loop instead re-enters at
    // §1038's `main_loop_lookahead`, whose bare `get_next` is what keeps a
    // run of adjacent characters from being delivered through expansion.
    let delivery = if main_loop_active {
        processor.main_loop_lookahead()
    } else {
        processor.get_x_token_with_replay_completion()
    };
    let Some(delivery) = delivery.map_err(command_error)? else {
        return Ok(ScannedStep::EndOfInput);
    };
    let tex_command::CommandReplayDelivery::Command(command) = delivery else {
        let tex_command::CommandReplayDelivery::Completed(episode) = delivery else {
            unreachable!();
        };
        return Ok(ScannedStep::ReplayCompleted(episode));
    };
    queue_command_trace(processor, mode, &command, diagnostics);
    if main_loop_active
        && matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal)
        && matches!(
            command.meaning(),
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::NoBoundary)
        )
    {
        return Ok(ScannedStep::NoBoundary {
            suppress_right: true,
        });
    }
    dispatch_main_control_command(
        processor,
        command,
        mode,
        boxes,
        innermost_group,
        job_is_all_over,
        display_eq_no,
        diagnostics,
    )
}

/// Dispatches one already-fetched command through TeX82 §1030's `reswitch:`
/// label and the big case below it.
///
/// This is the shared tail of *every* main-control step, whatever fetched the
/// command: §1030's own `get_x_token`, §1038's `main_loop_lookahead`, or an
/// alignment cell's template-aware delivery. tex.web has no second dispatcher
/// for alignment bodies -- §785's `align_peek` and §1130's `endv` case only
/// bound a cell, and everything between those bounds runs through the same
/// `main_control` big case -- so a caller that reaches `scan_command` without
/// passing through here is dispatching a *narrowed* main control that silently
/// drops whatever this function handles (`umber2-johp.208`).
///
/// Two things are handled here rather than in `scan_command` because tex.web
/// handles them before its big case reaches an assignment:
///
/// - §1211 `prefixed_command`'s `while cur_cmd=prefix` loop. §1210 routes
///   `any_mode(prefix)` -- so `\global`/`\long`/`\outer` (and e-TeX's
///   `\protected`) are prefixes in every mode, never mode-dispatched
///   primitives, and the accumulated `a` is what the assignment cases below
///   consult. Hoisting the loop above `scan_command` keeps that single
///   accumulation point, but only if every dispatch path runs it.
/// - §1045's `any_mode(ignore_spaces): begin <Get the next non-blank non-call
///   token>; goto reswitch; end`.
#[allow(clippy::too_many_arguments)] // carries command-owned replay facts
fn dispatch_main_control_command(
    processor: &mut CommandProcessor<'_>,
    mut command: tex_command::CurrentCommand,
    mode: Mode,
    boxes: &ReplayBoxes,
    innermost_group: Option<GroupKind>,
    job_is_all_over: bool,
    display_eq_no: bool,
    diagnostics: &mut Vec<PendingDiagnostic>,
) -> Result<ScannedStep, ExecError> {
    // §1030's `reswitch:` label sits *above* the big case, not at the fetch:
    // a case that has already fetched its own replacement command dispatches
    // that command in place. `goto reswitch` is therefore not `back_input`,
    // and a case using it pushes no input level and delivers nothing twice.
    // This loop is that label.
    let mut suppress_left_boundary = false;
    loop {
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
            command = processor
                .next_non_blank_non_relax_x_token()
                .map_err(command_error)?
                .ok_or(ExecError::MissingPrefixedCommand)?;
            // §1211's `if cur_cmd<=max_non_prefixed_command then <Discard
            // erroneous prefixes and return>`: §209's partition, not a
            // hand-listed set of assignment families.
            if !tex_command::exceeds_max_non_prefixed_command(command.meaning()) {
                let printed = tex_command::PrintCommand::from_current(&command);
                // §1212's `back_error`: the substantive command is retained
                // and re-delivered without the discarded prefixes.
                processor.back_input(command).map_err(command_error)?;
                // `back_error` is `back_input` *then* `error`, so §82 renders
                // the context with the backed-up level already on the stack.
                let etex = processor.profile().capabilities().supports_etex();
                diagnostics.push(PendingDiagnostic::PrefixOnNonPrefixedCommand(
                    printed,
                    processor.error_context(),
                    etex,
                ));
                return Ok(ScannedStep::Continue);
            }
        }
        // §1213's `<Discard the prefixes \long and \outer if they are
        // irrelevant>`. §1214 deliberately leaves `a` unadjusted, so the
        // command still runs; only the report is owed. eTeX's `\protected`
        // is prefix code 8, which §1213's `a mod 4<>0` excludes.
        if flags.bits() & (MeaningFlags::LONG | MeaningFlags::OUTER).bits() != 0
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
            let etex = processor.profile().capabilities().supports_etex();
            diagnostics.push(PendingDiagnostic::IrrelevantLongOuterPrefix(
                tex_command::PrintCommand::from_current(&command),
                processor.error_context(),
                etex,
            ));
        }
        // §406's helper is `repeat get_x_token until cur_cmd<>spacer` --
        // exactly `next_non_space` -- and the command it leaves in `cur_cmd`
        // is then dispatched by the case itself. Backing it up instead would
        // push a backup level, emit a recovery record, and deliver that
        // command a second time, none of which TeX82 does
        // (`umber2-johp.196`).
        if matches!(
            command.meaning(),
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::IgnoreSpaces)
        ) {
            let Some(next) = processor.next_non_blank_x_token().map_err(command_error)? else {
                return Ok(ScannedStep::EndOfInput);
            };
            command = next;
            continue;
        }
        if matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal)
            && matches!(
                command.meaning(),
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::NoBoundary)
            )
        {
            let Some(next) = processor.get_x_token().map_err(command_error)? else {
                return Ok(ScannedStep::Continue);
            };
            suppress_left_boundary = matches!(
                next.meaning(),
                Meaning::CharToken {
                    cat: Catcode::Letter | Catcode::Other,
                    ..
                } | Meaning::CharGiven(_)
                    | Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char)
            );
            command = next;
            continue;
        }
        // TeX82 §1214 resolves `\globaldefs` exactly once, before entering
        // §1211's assignment case. Every scanner-time provisional
        // definition, committed application, and mutation observation below
        // therefore receives the same effective value rather than
        // independently consulting live state at a later seam.
        let global = effective_global(
            processor.int_param(IntParam::GLOBAL_DEFS),
            global
                || matches!(
                    command.meaning(),
                    Meaning::UnexpandablePrimitive(
                        UnexpandablePrimitive::Gdef | UnexpandablePrimitive::Xdef
                    )
                ),
        );
        let mut scanned = scan_command(
            processor,
            command,
            global,
            flags,
            mode,
            boxes,
            innermost_group,
            job_is_all_over,
            display_eq_no,
        )?;
        if suppress_left_boundary {
            match &mut scanned {
                ScannedStep::Character {
                    suppress_left_boundary,
                    ..
                }
                | ScannedStep::CharacterCode {
                    suppress_left_boundary,
                    ..
                } => *suppress_left_boundary = true,
                _ => {}
            }
        }
        return Ok(scanned);
    }
}

/// TeX82 §1030's `if tracing_commands>0 then show_cur_cmd_chr` immediately
/// after `big_switch` fetches. Commands fetched by a case before `goto
/// reswitch` -- including §1211's prefix loop -- do not pass this boundary.
fn queue_command_trace(
    processor: &CommandProcessor<'_>,
    mode: Mode,
    command: &tex_command::CurrentCommand,
    diagnostics: &mut Vec<PendingDiagnostic>,
) {
    if processor.int_param(IntParam::TRACING_COMMANDS) > 0 {
        diagnostics.push(PendingDiagnostic::CommandTrace(
            mode,
            tex_command::PrintCommand::from_current(command),
        ));
    }
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

/// Recognizes membership in TeX82 §1090's shared vertical-mode
/// `back_input; new_graf(true)` case, listed there as
/// `vmode+letter,vmode+other_char,vmode+char_num,vmode+char_given,`
/// `vmode+math_shift,vmode+un_hbox,vmode+vrule,vmode+accent,`
/// `vmode+discretionary,vmode+hskip,vmode+valign,vmode+ex_space,`
/// `vmode+no_boundary`.
///
/// The caller has already established that the mode is (internal) vertical:
/// tex.web's big case is `case abs(mode)+cur_cmd of`, so `vmode+x` covers both
/// `vmode` and `-vmode`. Membership is decided purely from the delivered
/// command, exactly as tex.web decides it from `cur_cmd`.
fn starts_paragraph_in_vertical_mode(meaning: Meaning) -> bool {
    match meaning {
        // `vmode+letter`, `vmode+other_char`, and `vmode+math_shift`. A
        // `spacer` is deliberately absent: §1045's `vmode+spacer: do_nothing`
        // leaves vertical mode untouched, and every other category code
        // (braces, `#`, `^`, `_`, `~`) has its own case elsewhere.
        Meaning::CharToken { cat, .. } => {
            matches!(cat, Catcode::Letter | Catcode::Other | Catcode::MathShift)
        }
        // `vmode+char_given`: a `\chardef`'d token (§1224 installs it as
        // `char_given`), which §1090 treats exactly like `char_num`.
        Meaning::CharGiven(_) => true,
        Meaning::UnexpandablePrimitive(primitive) => matches!(
            primitive,
            // `vmode+char_num`: §265's `primitive("char",char_num,0)`.
            UnexpandablePrimitive::Char
                // `vmode+un_hbox`: §1107 installs `\unhbox` and
                // `\unhcopy` under the one `un_hbox` command code. `un_vbox`
                // is not in this group -- `\unvbox` legitimately appends an
                // unboxed vertical list to the enclosing vertical list.
                | UnexpandablePrimitive::UnHBox
                | UnexpandablePrimitive::UnHCopy
                // `vmode+vrule`: §265's `primitive("vrule",vrule,0)`.
                // `\hrule` is instead §1056's `vmode+hrule:
                // tail_append(scan_rule_spec)`, which stays in vertical mode.
                | UnexpandablePrimitive::VRule
                // `vmode+accent`: §265's `primitive("accent",accent,0)`.
                | UnexpandablePrimitive::Accent
                // `vmode+discretionary`: §1114 installs both `\-`
                // (chr 1) and `\discretionary` (chr 0) as `discretionary`.
                | UnexpandablePrimitive::Discretionary
                | UnexpandablePrimitive::DiscretionaryHyphen
                // `vmode+hskip`: §1058 installs `\hskip`, `\hfil`,
                // `\hfill`, `\hss`, and `\hfilneg` under the one `hskip`
                // command code. `\kern` is `kern`, not `hskip`, and §1057's
                // `vmode+kern` appends to the vertical list instead.
                | UnexpandablePrimitive::HSkip
                | UnexpandablePrimitive::HFil
                | UnexpandablePrimitive::HFill
                | UnexpandablePrimitive::HSs
                | UnexpandablePrimitive::HFilNeg
                // `vmode+valign`: §265's `primitive("valign",valign,0)`.
                // e-TeX 2.6 [53a.3826--3883] deliberately gives all four
                // text-direction primitives this same command code with
                // nonzero modifiers. TeX §1090 dispatches by command code,
                // so they also start a paragraph before their hmode action.
                | UnexpandablePrimitive::VAlign
                | UnexpandablePrimitive::BeginL
                | UnexpandablePrimitive::EndL
                | UnexpandablePrimitive::BeginR
                | UnexpandablePrimitive::EndR
                // `vmode+ex_space`: §265's `primitive("␣",ex_space,0)`.
                | UnexpandablePrimitive::ControlSpace
                // `vmode+no_boundary`: §265's
                // `primitive("noboundary",no_boundary,0)`.
                | UnexpandablePrimitive::NoBoundary
        ),
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)] // mirrors TeX main-control dispatch inputs
fn scan_command(
    processor: &mut CommandProcessor<'_>,
    command: tex_command::CurrentCommand,
    global: bool,
    flags: MeaningFlags,
    mode: Mode,
    boxes: &ReplayBoxes,
    innermost_group: Option<GroupKind>,
    job_is_all_over: bool,
    display_eq_no: bool,
) -> Result<ScannedStep, ExecError> {
    if let Meaning::UnexpandablePrimitive(
        primitive @ (UnexpandablePrimitive::TextFont
        | UnexpandablePrimitive::ScriptFont
        | UnexpandablePrimitive::ScriptScriptFont),
    ) = command.meaning()
    {
        let size = tex_command::MathFamilySize::of_primitive(primitive)
            .expect("the outer match restricts this to `def_family`");
        let family = processor.scan_math_family(size).map_err(command_error)?;
        // tex.web section 23069-23070 (def_family): scan_four_bit_int is
        // followed by scan_optional_equals before scan_font_ident. Skipping
        // this let a literal `=` in `\textfont0=\tenrm` fall through to
        // ordinary main control instead of being consumed here.
        let _ = processor.scan_optional_equals().map_err(command_error)?;
        // TeX82 §1234's `def_family` calls §578 `scan_font_ident`; the font
        // identifier is an assignment operand, not a `set_font` command.
        // Using the typed scanner also commits its lookahead consumption so
        // the identifier cannot be backed up and replayed by main control.
        let font = processor.scan_font_selector().map_err(command_error)?;
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
        && let Some(request) = processor
            .scan_canonical_math_request(&command)
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
    // TeX82 §1068 dispatches a right brace from the current `cur_group`.
    // An ancestor simple group must not make a nested box's body closer look
    // like an ordinary group closer.
    if innermost_group == Some(GroupKind::Simple)
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
    // TeX82 §1186's `math_group` arm of `handle_right_brace` (the brace that
    // closes a subformula scanned by §1151's `scan_math`) and §1174's
    // `math_choice_group` arm (the brace that closes one `\mathchoice`
    // branch). §1153 and §1172/§1174 opened these levels with `push_math`,
    // so each pair really does bracket a save-stack level and its closer must
    // not fall through to the ordinary or box brace arms below.
    if let Some(kind @ (GroupKind::Math | GroupKind::MathChoice)) = innermost_group
        && matches!(
            command.meaning(),
            Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            }
        )
    {
        return Ok(ScannedStep::EndMathGroup(kind));
    }
    if innermost_group == Some(GroupKind::Disc)
        && matches!(
            command.meaning(),
            Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            }
        )
    {
        return Ok(ScannedStep::DiscretionaryPartEnd);
    }
    // TeX82 §1150's `mmode+left_brace`: a bare explicit brace encountered
    // directly in math mode starts a subformula that becomes the nucleus of a
    // freshly appended noad -- `tail_append(new_noad); back_input;
    // scan_math(nucleus(tail))` -- rather than an ordinary `simple_group`
    // scope. This must be checked before the general brace arms below: a math
    // formula nested inside an active box body (for example plain.tex's
    // `\maketable` macro, which replays its whole `\halign` argument inside
    // `\setbox1=\vbox{#2}`) otherwise had its bare `{`/`}` swallowed with no
    // noad ever appended, so a following `^`/`_` incorrectly saw the
    // *enclosing* list's last node (an ordinary character, from *outside*
    // the formula) as its attachment target instead of a fresh empty
    // nucleus. Reusing the existing `TextField(Ord)` request (the same
    // completed-field plumbing `\mathord{...}` already drives) is exact:
    // `scan_math`'s brace case and `\mathord`'s explicit field scan both
    // bottom out in one §1153 `math_group`/`fin_mlist` cycle, and an
    // Ord-classified noad is what an unornamented brace group produces. A
    // box's own mandatory opening brace never reaches this dispatch at all:
    // `scan_left_brace` (TeX82 §403) consumed it while the construction was
    // still being scanned.
    if matches!(mode, Mode::Math | Mode::DisplayMath)
        && let Meaning::CharToken {
            cat: Catcode::BeginGroup,
            ..
        } = command.meaning()
    {
        processor.back_input(command).map_err(command_error)?;
        return Ok(ScannedStep::Math(CanonicalMathRequest::TextField(
            MathTextFieldKind::Ord,
        )));
    }
    // TeX82 §1068's `handle_right_brace` dispatches purely on `cur_group`, so
    // a box body's own closing brace is exactly the one delivered while the
    // innermost group is still the group `scan_spec`/`begin_insert_or_adjust`
    // opened for that body. Braces nested inside the body opened ordinary
    // `simple_group` levels of their own (§1063), and §1069's `simple_group:
    // unsave` -- reached through the `EndOrdinaryGroup` arm above -- closes
    // those. No separate brace-depth count is kept: the save stack already
    // holds every open level, and counting braces instead silently skipped
    // `unsave`, losing both the nested group's local restores and the
    // `\aftergroup` tokens §282 backs up when it pops.
    if let Some(box_state) = boxes.active_boxes.last()
        && innermost_group == Some(box_state.group_kind)
        && matches!(
            command.meaning(),
            Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            }
        )
    {
        return Ok(ScannedStep::BoxEndGroup {
            ships_out: box_state.ships_out,
        });
    }
    // TeX82 §1016 opens `output_group` before replaying the braced output
    // token list. A box body nested in that list owns its closing brace first;
    // only the live output group can close the enclosing output routine.
    if boxes.output_routine_active
        && innermost_group == Some(GroupKind::Output)
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
    // TeX82 §1090's `@<Cases of |main_control| that build boxes and lists@>`
    // opens with one shared vertical-mode case, not thirteen separate ones:
    //
    //     vmode+letter,vmode+other_char,vmode+char_num,vmode+char_given,
    //     vmode+math_shift,vmode+un_hbox,vmode+vrule,
    //     vmode+accent,vmode+discretionary,vmode+hskip,vmode+valign,
    //     vmode+ex_space,vmode+no_boundary:
    //       begin back_input; new_graf(true); end;
    //
    // Every member takes the same two actions in the same order, *before* any
    // operand of its own is looked at: the triggering command is pushed back
    // (§325 `back_input`, which opens a `backed_up` input level), and §1091
    // `new_graf` then opens the paragraph and pushes `\everypar`. The backed-up
    // command is redelivered afterwards and dispatched again, now in horizontal
    // mode, where it scans its operand.
    //
    // Scanning the operand here instead -- `\char`'s character number,
    // `\hskip`'s glue, `\vrule`'s rule spec, `\accent`'s accent number and base
    // character, `\discretionary`'s three lists -- reads it in vertical mode,
    // before `\everypar` has run and before the paragraph's horizontal list
    // exists, and skips the backup level and redelivery entirely.
    if matches!(mode, Mode::Vertical | Mode::InternalVertical)
        && starts_paragraph_in_vertical_mode(command.meaning())
    {
        processor.back_input(command).map_err(command_error)?;
        return Ok(ScannedStep::ParagraphStart);
    }
    match command.meaning() {
        Meaning::CharToken {
            cat: Catcode::BeginGroup,
            ..
        } => Ok(ScannedStep::BeginOrdinaryGroup),
        // TeX82 §1068's `handle_right_brace` sends three of its `cur_group`
        // cases to §1069's `extra_right_brace`, which names the group opener
        // the brace was mistaken for; every other unmatched brace is §1068's
        // own `bottom_level` arm.
        Meaning::CharToken {
            cat: Catcode::EndGroup,
            ..
        } => Ok(ScannedStep::ExtraRightBrace {
            forgotten: match innermost_group {
                Some(GroupKind::SemiSimple) => Some(ForgottenGroupOpener::EndGroup),
                Some(GroupKind::MathShift) => Some(ForgottenGroupOpener::MathShift),
                Some(GroupKind::MathLeft) => Some(ForgottenGroupOpener::Right),
                _ => None,
            },
        }),
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
            scan_off_save(processor, command, innermost_group)
        }
        // TeX82 §1094's `hmode+stop,...: head_for_vmode`. §1095's
        // unrestricted branch (`mode>0`) backs the stop up, then backs an
        // inserted `\\par` up ahead of it, so the stop is retried in the
        // enclosing vertical mode. The command core owns both backups;
        // replay merely processes the resulting `\\par`.
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::End | UnexpandablePrimitive::Dump,
        ) if mode == Mode::Horizontal => {
            processor
                .recover_stop_for_vertical_mode(command)
                .map_err(command_error)?;
            Ok(ScannedStep::Continue)
        }
        // §1095's restricted-`hmode` branch (`mode<0`, e.g. inside an
        // `\\hbox`): `if cur_cmd<>hrule then off_save`. `\\par` has no
        // meaning there, so §1064's fully general recovery closes the
        // enclosing group instead, exactly as the `\\vskip` family above.
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::End | UnexpandablePrimitive::Dump,
        ) if mode == Mode::RestrictedHorizontal => {
            scan_off_save(processor, command, innermost_group)
        }
        // §1046's "math-only cases in non-math modes, or vice versa" table
        // lists `mmode+stop`, so §1047's `insert_dollar_sign` closes the math
        // first and retries the stop in the resulting mode.
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::End | UnexpandablePrimitive::Dump,
        ) if matches!(mode, Mode::Math | Mode::DisplayMath) => {
            processor
                .recover_missing_math_shift(command)
                .map_err(command_error)?;
            Ok(ScannedStep::MissingMathShift)
        }
        // §1045's `vmode+stop: if its_all_over then return` -- "this is the
        // only way out" of `main_control`. §1054's `its_all_over` is the one
        // general mechanism: the job ends only when the current page and the
        // contribution list are both empty and the last output was not a dead
        // cycle. Otherwise the stop is backed up and residual material is
        // ejected by appending `\\hbox to \\hsize{}`, `\\vfill`, and
        // `\\penalty-'10000000000` and calling §994's `build_page`; whether
        // that reaches `\\output` at all, and with what `\\box255`, is
        // §1005/§1012's decision, never this dispatch's.
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::End | UnexpandablePrimitive::Dump,
        ) => {
            // §1051's `privileged`: `mode>0` only. Internal vertical mode
            // (inside a `\\vbox`, an `\\insert`, or `\\output` itself) reports
            // an illegal case and leaves the job running.
            if mode != Mode::Vertical {
                return Ok(ScannedStep::IllegalStop {
                    token: command.spelling().semantic_token(),
                });
            }
            if job_is_all_over {
                // §1335's `final_cleanup` unwinds the input stack that
                // `main_control`'s return has abandoned.
                let incomplete_conditions = processor.final_cleanup();
                return Ok(ScannedStep::End {
                    dump: matches!(
                        command.meaning(),
                        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dump)
                    ),
                    incomplete_conditions,
                });
            }
            processor.back_input(command).map_err(command_error)?;
            Ok(ScannedStep::EjectResidualPage)
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
        Meaning::CountRegister(index) => {
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
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Wd
            | UnexpandablePrimitive::Ht
            | UnexpandablePrimitive::Dp),
        ) => {
            let index = processor
                .scan_eight_bit_register_index()
                .map_err(command_error)?;
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_dimension().map_err(command_error)?.value;
            let dimension = match primitive {
                UnexpandablePrimitive::Wd => tex_state::BoxDimension::Width,
                UnexpandablePrimitive::Ht => tex_state::BoxDimension::Height,
                UnexpandablePrimitive::Dp => tex_state::BoxDimension::Depth,
                _ => unreachable!(),
            };
            Ok(ScannedStep::BoxDimensionAssignment {
                index,
                dimension,
                value,
                global,
            })
        }
        Meaning::DimenRegister(index) => {
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
        Meaning::SkipRegister(index) => {
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
        Meaning::MuskipRegister(index) => {
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
        // TeX82 §1102's `any_mode(break_penalty): append_penalty` (§1103:
        // `scan_int; tail_append(new_penalty(cur_val))`). `\penalty` never
        // switches mode -- it appends directly to whatever list (main
        // vertical, horizontal, restricted horizontal, or math) is current.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Penalty) => {
            let amount = processor.scan_integer().map_err(command_error)?.value;
            Ok(ScannedStep::Penalty { amount })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ControlSpace) => {
            Ok(ScannedStep::ControlSpace)
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PrevDepth) => {
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_dimension().map_err(command_error)?.value;
            Ok(ScannedStep::PrevDepth { value })
        }
        // TeX82 §1265's `any_mode(set_interaction): prefixed_command` ->
        // `new_interaction` (§1264): `interaction:=cur_chr`. The four
        // primitives differ only in the fixed `chr_code` each was installed
        // with (§1264's four `primitive("...",set_interaction,...)` calls),
        // so there is no operand scan of any kind -- the target level is
        // selected purely from which primitive was delivered, exactly like
        // `\unpenalty`/`\unkern`/`\unskip` above. `interaction` is a plain
        // global Pascal variable outside `eqtb`, so this assignment is never
        // grouped/undone and ignores `\global`/`\globaldefs` entirely, unlike
        // ordinary parameter assignments.
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::BatchMode
            | UnexpandablePrimitive::NonstopMode
            | UnexpandablePrimitive::ScrollMode
            | UnexpandablePrimitive::ErrorStopMode),
        ) => Ok(ScannedStep::SetInteractionMode(primitive)),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::InteractionMode) => {
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_integer().map_err(command_error)?.value;
            Ok(ScannedStep::SetInteractionModeValue(value))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SpaceFactor) => {
            if matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal) {
                let _ = processor.scan_optional_equals().map_err(command_error)?;
                let value = processor.scan_integer().map_err(command_error)?.value;
                Ok(ScannedStep::SpaceFactor { value })
            } else {
                Ok(ScannedStep::IllegalSpaceFactor {
                    token: command.spelling().semantic_token(),
                })
            }
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PrevGraf) => {
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_integer().map_err(command_error)?.value;
            Ok(ScannedStep::PrevGraf { value })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char) => {
            let value = processor.scan_integer().map_err(command_error)?.value;
            Ok(ScannedStep::CharacterCode {
                value,
                suppress_left_boundary: false,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Accent) => Ok(ScannedStep::Accent(
            processor.scan_accent().map_err(command_error)?,
        )),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Discretionary) => {
            Ok(ScannedStep::DiscretionaryOpening(
                processor
                    .scan_discretionary_opening()
                    .map_err(command_error)?,
            ))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::DiscretionaryHyphen) => {
            Ok(ScannedStep::DiscretionaryHyphen {
                origin: command.origin(),
            })
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::HFil
            | UnexpandablePrimitive::HFill
            | UnexpandablePrimitive::HSs
            | UnexpandablePrimitive::HFilNeg),
        ) => Ok(ScannedStep::FixedHorizontalGlue { primitive }),
        // `\vskip`/`\vfil`/`\vfill`/`\vss`/`\vfilneg` are legal only in
        // vertical mode. TeX82 §1046's "math-only cases in non-math modes, or
        // vice versa" table lists `mmode+vskip` (and the fil variants) among
        // the cases §1047's `insert_dollar_sign` recovers from, identically
        // to `mmode+hrule` above.
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::VSkip
            | UnexpandablePrimitive::VFil
            | UnexpandablePrimitive::VFill
            | UnexpandablePrimitive::VSs
            | UnexpandablePrimitive::VFilNeg,
        ) if matches!(mode, Mode::Math | Mode::DisplayMath) => {
            processor
                .recover_missing_math_shift(command)
                .map_err(command_error)?;
            Ok(ScannedStep::MissingMathShift)
        }
        // §1095's `head_for_vmode` distinguishes unrestricted `hmode`
        // (`mode>=0`) from restricted `hmode` (`mode<0`, e.g. inside an
        // `\hbox`): only the unrestricted case takes the simple
        // "back up, insert `\par`, retry" path that
        // `recover_stop_for_vertical_mode` implements.
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::VSkip
            | UnexpandablePrimitive::VFil
            | UnexpandablePrimitive::VFill
            | UnexpandablePrimitive::VSs
            | UnexpandablePrimitive::VFilNeg,
        ) if mode == Mode::Horizontal => {
            processor
                .recover_stop_for_vertical_mode(command)
                .map_err(command_error)?;
            Ok(ScannedStep::Continue)
        }
        // §1095's `head_for_vmode`'s restricted-`hmode` branch
        // (`mode<0`): `if cur_cmd<>hrule then off_save`. Unlike the
        // unrestricted case above, restricted horizontal mode (e.g. inside
        // an `\hbox`) cannot simply insert `\par` and retry -- `\par` has no
        // meaning there -- so TeX instead runs the fully general §1064
        // `off_save` recovery against whatever group the `\hbox` (or other
        // box-making construct) opened.
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::VSkip
            | UnexpandablePrimitive::VFil
            | UnexpandablePrimitive::VFill
            | UnexpandablePrimitive::VSs
            | UnexpandablePrimitive::VFilNeg,
        ) if mode == Mode::RestrictedHorizontal => {
            scan_off_save(processor, command, innermost_group)
        }
        // TeX82 §1057's `vmode+vskip: append_glue` (using `abs(mode)`, so both
        // outer `Vertical` and `InternalVertical` match `vmode`).
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VSkip)
            if matches!(mode, Mode::Vertical | Mode::InternalVertical) =>
        {
            let value = processor.scan_glue(false).map_err(command_error)?.value;
            Ok(ScannedStep::VerticalSkip { value })
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::VFil
            | UnexpandablePrimitive::VFill
            | UnexpandablePrimitive::VSs
            | UnexpandablePrimitive::VFilNeg),
        ) if matches!(mode, Mode::Vertical | Mode::InternalVertical) => {
            Ok(ScannedStep::FixedVerticalGlue { primitive })
        }
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
        // e-TeX 2.6 change [49.1248] extends TeX82 §1248's `set_shape`:
        // after the optional equals and integer count, the four penalty-array
        // selectors scan exactly `max(count, 0)` integer values. Keeping the
        // complete scan in this typed step preserves retry atomicity.
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::InterLinePenalties
            | UnexpandablePrimitive::ClubPenalties
            | UnexpandablePrimitive::WidowPenalties
            | UnexpandablePrimitive::DisplayWidowPenalties),
        ) => {
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let count = processor
                .scan_integer()
                .map_err(command_error)?
                .value
                .max(0) as usize;
            let mut values = Vec::new();
            values
                .try_reserve_exact(count)
                .map_err(|_| ExecError::ArithmeticOverflow)?;
            for _ in 0..count {
                values.push(processor.scan_integer().map_err(command_error)?.value);
            }
            let kind = match primitive {
                UnexpandablePrimitive::InterLinePenalties => PenaltyArrayKind::InterLine,
                UnexpandablePrimitive::ClubPenalties => PenaltyArrayKind::Club,
                UnexpandablePrimitive::WidowPenalties => PenaltyArrayKind::Widow,
                UnexpandablePrimitive::DisplayWidowPenalties => PenaltyArrayKind::DisplayWidow,
                _ => unreachable!("outer match restricts primitive to e-TeX penalty arrays"),
            };
            Ok(ScannedStep::PenaltyArray {
                kind,
                values,
                global,
            })
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
        Meaning::ToksRegister(index) => Ok(ScannedStep::Toks {
            index,
            tokens: processor
                .scan_token_register_value()
                .map_err(command_error)?,
            global,
        }),
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
        // TeX82 §1210 lists `set_page_dimen` and `set_page_int` among
        // `prefixed_command`'s ordinary assignment forms, and §1242 routes
        // them to `alter_page_so_far` (§1245) and `alter_integer` (§1246).
        // Both scan exactly like the `\dimen`/`\count` parameter arms above,
        // and both deliberately drop `global`: §1242's own comment ("these
        // definitions are always global") applies because `page_so_far`,
        // `dead_cycles`, and `insert_penalties` are engine variables rather
        // than `eqtb` entries, so neither `\global` nor `\globaldefs` has
        // anything to scope.
        Meaning::PageDimension(dimension) => {
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_dimension().map_err(command_error)?.value;
            Ok(ScannedStep::PageDimension { dimension, value })
        }
        Meaning::PageInteger(integer) => {
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_integer().map_err(command_error)?.value;
            Ok(ScannedStep::PageInteger { integer, value })
        }
        Meaning::TokParam(index) => {
            let tokens = processor
                .scan_token_parameter_assignment(TokParam::new(index))
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
        ) => {
            // §1214 fixes the effective scope before §1225 calls
            // `read_toks`; carry that scope across the typed apply seam.
            Ok(ScannedStep::InputStream {
                request: processor
                    .scan_input_stream_request(primitive, global)
                    .map_err(command_error)?,
                resource: None,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Font) => {
            // TeX82 §1257's `define(u,set_font,null_font)` precedes the file
            // name scan, so like §1224's provisional `\relax` it takes the
            // scope the eventual definition would take, `\globaldefs`
            // included.
            let request = processor
                .scan_font_definition(global)
                .map_err(command_error)?;
            Ok(ScannedStep::FontDefinition {
                request,
                resource: Box::new(None),
                global,
            })
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::PdfXImage | UnexpandablePrimitive::PdfRefXImage),
        ) => {
            // pdftex.web §§1551–1552 begin both image cases with
            // `check_pdfoutput`, before version checking, image-object
            // allocation, every rule/attr/named/page/colorspace/page-box/file
            // scan, host image lookup, reference validation, whatsit
            // allocation, or list mutation.
            if processor.int_param(IntParam::PDF_OUTPUT) <= 0 {
                let name = match primitive {
                    UnexpandablePrimitive::PdfXImage => "pdfximage",
                    UnexpandablePrimitive::PdfRefXImage => "pdfrefximage",
                    _ => unreachable!(),
                };
                return Err(ExecError::PdfExtensionInDviMode(name));
            }
            if primitive == UnexpandablePrimitive::PdfRefXImage {
                return Ok(ScannedStep::PdfRefXImage {
                    object: processor.scan_integer().map_err(command_error)?.value,
                });
            }
            Ok(ScannedStep::PdfXImage {
                request: processor.scan_pdf_image_request().map_err(command_error)?,
                // This placeholder is replaced after the processor borrow;
                // it can never reach application.
                resource: PdfImageResource::Unavailable,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfSetRandomSeed) => {
            let seed = processor.scan_integer().map_err(command_error)?.value;
            Ok(ScannedStep::PdfSetRandomSeed {
                seed: seed.saturating_abs(),
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfResetTimer) => {
            Ok(ScannedStep::PdfResetTimer)
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfInterwordSpaceOn) => {
            Ok(ScannedStep::PdfInterwordSpace(
                tex_state::node::PdfAccessibilityControl::InterwordSpaceOn,
            ))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfInterwordSpaceOff) => {
            Ok(ScannedStep::PdfInterwordSpace(
                tex_state::node::PdfAccessibilityControl::InterwordSpaceOff,
            ))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfFakeSpace) => Ok(
            ScannedStep::PdfInterwordSpace(tex_state::node::PdfAccessibilityControl::FakeSpace),
        ),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfRunningLinkOn) => {
            Ok(ScannedStep::PdfRunningLink(true))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfRunningLinkOff) => {
            Ok(ScannedStep::PdfRunningLink(false))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfSpaceFont) => {
            Ok(ScannedStep::PdfSpaceFont(
                processor
                    .scan_balanced_text(true)
                    .map_err(command_error)?
                    .tokens,
            ))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfObject) => {
            // pdftex.web §§1535 and 1542 call `check_pdfoutput` before
            // `reserveobjnum`, `useobjnum`, the integer, stream/attr/file
            // options, body scan, or allocation.
            if processor.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfobj"));
            }
            Ok(ScannedStep::PdfObject(
                processor.scan_pdf_object_request().map_err(command_error)?,
            ))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfReferenceObject) => {
            // pdftex.web §1544 calls `check_pdfoutput` before `scan_int`,
            // object validation, whatsit allocation, or list mutation.
            if processor.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfrefobj"));
            }
            Ok(ScannedStep::PdfReferenceObject(
                processor
                    .scan_pdf_reference_object_request()
                    .map_err(command_error)?,
            ))
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::PdfXForm | UnexpandablePrimitive::PdfRefXForm),
        ) => {
            // pdftex.web §§1548–1549 call `check_pdfoutput` before form-object
            // allocation, either option scan, the box-register/integer scan,
            // object validation, whatsit allocation, or list mutation.
            if processor.int_param(IntParam::PDF_OUTPUT) <= 0 {
                let name = match primitive {
                    UnexpandablePrimitive::PdfXForm => "pdfxform",
                    UnexpandablePrimitive::PdfRefXForm => "pdfrefxform",
                    _ => unreachable!(),
                };
                return Err(ExecError::PdfExtensionInDviMode(name));
            }
            Ok(ScannedStep::PdfForm(
                processor
                    .scan_pdf_form_request(primitive)
                    .map_err(command_error)?,
            ))
        }
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
            | UnexpandablePrimitive::PdfSavePos
            | UnexpandablePrimitive::PdfSnapRefPoint
            | UnexpandablePrimitive::PdfSnapY
            | UnexpandablePrimitive::PdfSnapYComp),
        ) => {
            if matches!(
                primitive,
                UnexpandablePrimitive::PdfSnapRefPoint
                    | UnexpandablePrimitive::PdfSnapY
                    | UnexpandablePrimitive::PdfSnapYComp
            ) && processor.int_param(IntParam::PDF_OUTPUT) <= 0
            {
                let name = match primitive {
                    UnexpandablePrimitive::PdfSnapRefPoint => "pdfsnaprefpoint",
                    UnexpandablePrimitive::PdfSnapY => "pdfsnapy",
                    UnexpandablePrimitive::PdfSnapYComp => "pdfsnapycomp",
                    _ => unreachable!(),
                };
                return Err(ExecError::PdfExtensionInDviMode(name));
            }
            Ok(ScannedStep::PdfGraphics(
                processor
                    .scan_pdf_graphics_request(primitive)
                    .map_err(command_error)?
                    .expect("graphics primitive has a typed request"),
            ))
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::PdfAnnot
            | UnexpandablePrimitive::PdfStartLink
            | UnexpandablePrimitive::PdfEndLink
            | UnexpandablePrimitive::PdfOutline
            | UnexpandablePrimitive::PdfDest
            | UnexpandablePrimitive::PdfThread
            | UnexpandablePrimitive::PdfStartThread
            | UnexpandablePrimitive::PdfEndThread),
        ) => {
            if matches!(
                primitive,
                UnexpandablePrimitive::PdfAnnot
                    | UnexpandablePrimitive::PdfStartLink
                    | UnexpandablePrimitive::PdfEndLink
                    | UnexpandablePrimitive::PdfOutline
                    | UnexpandablePrimitive::PdfDest
                    | UnexpandablePrimitive::PdfThread
                    | UnexpandablePrimitive::PdfStartThread
                    | UnexpandablePrimitive::PdfEndThread
            ) && processor.int_param(IntParam::PDF_OUTPUT) <= 0
            {
                let name = match primitive {
                    UnexpandablePrimitive::PdfAnnot => "pdfannot",
                    UnexpandablePrimitive::PdfStartLink => "pdfstartlink",
                    UnexpandablePrimitive::PdfEndLink => "pdfendlink",
                    UnexpandablePrimitive::PdfOutline => "pdfoutline",
                    UnexpandablePrimitive::PdfDest => "pdfdest",
                    UnexpandablePrimitive::PdfThread => "pdfthread",
                    UnexpandablePrimitive::PdfStartThread => "pdfstartthread",
                    UnexpandablePrimitive::PdfEndThread => "pdfendthread",
                    _ => unreachable!(),
                };
                return Err(ExecError::PdfExtensionInDviMode(name));
            }
            Ok(ScannedStep::PdfNavigation(
                processor
                    .scan_pdf_navigation_request(primitive)
                    .map_err(command_error)?,
            ))
        }
        Meaning::Font(font) => Ok(ScannedStep::FontSelect {
            font,
            selector: command.control_sequence(),
            global,
        }),
        // tex.web §578's `find_font_dimen` scans the number *and* the font
        // identifier before it decides the number is unusable, and §1253 then
        // scans `=<dimen>` either way; the whole assignment is consumed even
        // when §579 rejects it.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::FontDimen) => {
            let number = processor.scan_integer().map_err(command_error)?.value;
            let font = processor.scan_font_selector().map_err(command_error)?;
            // §579 reports from inside `find_font_dimen`, so its `show_context`
            // splits here -- after the font identifier and before `=<dimen>`.
            let recovery_context =
                (!processor.font_dimen_writable(font, number)).then(|| processor.error_context());
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_dimension().map_err(command_error)?.value;
            Ok(ScannedStep::FontDimen {
                font,
                number,
                value,
                recovery_context,
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
            let stream = processor
                .scan_restricted_integer(RestrictedIntegerClass::FourBit)
                .map_err(command_error)?
                .value as u8;
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let file_name = processor.scan_file_name().map_err(command_error)?;
            Ok(ScannedStep::DeferredOpenOut {
                stream,
                file_name: file_name.packed(),
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::CloseOut) => {
            Ok(ScannedStep::DeferredCloseOut {
                stream: processor.scan_write_stream().map_err(command_error)?,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Write) => {
            // TeX82 §1350's `new_write_whatsit` normalizes the stream number
            // before storing it in `write_stream(tail)`, for the deferred
            // whatsit exactly as for the `\immediate` one.
            let stream = processor.scan_write_stream().map_err(command_error)?;
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
        // TeX82 §1377's `@<Implement \setlanguage@>`, the `set_language_code`
        // limb of §1348's `do_extension`. The mode test comes first and
        // guards the `scan_int`, so the operand is read only when
        // `abs(mode)=hmode` -- horizontal or restricted horizontal here,
        // tex.web's `hmode` and `-hmode`.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SetLanguage) => {
            if matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal) {
                Ok(ScannedStep::SetLanguage {
                    language: processor.scan_integer().map_err(command_error)?.value,
                })
            } else {
                Ok(ScannedStep::IllegalSetLanguage {
                    token: command.spelling().semantic_token(),
                })
            }
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::CatCode
            | UnexpandablePrimitive::LcCode
            | UnexpandablePrimitive::UcCode
            | UnexpandablePrimitive::SfCode
            | UnexpandablePrimitive::MathCode
            | UnexpandablePrimitive::DelCode),
        ) => {
            // TeX82 §1230 selects the table entry with §434's
            // `scan_char_num`, including its out-of-range recovery to
            // character zero. The assigned value has the table-specific
            // bound below; it is a distinct operand and must not inherit the
            // selector's recovery.
            let character = processor
                .scan_restricted_integer(RestrictedIntegerClass::CharacterCode)
                .map_err(command_error)?
                .value;
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_integer().map_err(command_error)?.value;
            let character =
                char::from_u32(character as u32).expect("scan_char_num returns a valid character");
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
                global,
                parameter_text: definition.parameter_text,
                replacement_text: definition.replacement_text,
                definition_origin: definition.provenance.primary,
                missing_target: definition.missing_target,
            })
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::CharDef | UnexpandablePrimitive::MathCharDef),
        ) => {
            // TeX82 §1224 installs the scanner-time `\relax` through
            // `define`, so it has the same effective scope as the eventual
            // definition, including `\globaldefs`. This remains main-control
            // scope policy; the command processor only receives the selected
            // provisional scope while it owns raw operand delivery.
            // §1224's case: `char_def_code` scans §434's `scan_char_num` and
            // `math_char_def_code` scans §436's `scan_fifteen_bit_int`.
            let class = match primitive {
                UnexpandablePrimitive::CharDef => RestrictedIntegerClass::CharacterCode,
                UnexpandablePrimitive::MathCharDef => RestrictedIntegerClass::FifteenBit,
                _ => {
                    unreachable!("outer match restricts primitive to §1224's character shorthands")
                }
            };
            let definition = processor
                .scan_character_definition(class, global)
                .map_err(command_error)?;
            Ok(ScannedStep::CharacterDefinition {
                primitive,
                target: definition.target,
                value: definition.value,
                global,
            })
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::CountDef
            | UnexpandablePrimitive::DimenDef
            | UnexpandablePrimitive::SkipDef
            | UnexpandablePrimitive::MuskipDef
            | UnexpandablePrimitive::ToksDef),
        ) => {
            let definition = processor
                .scan_register_definition(global)
                .map_err(command_error)?;
            Ok(ScannedStep::RegisterDefinition {
                primitive,
                target: definition.target,
                index: definition.index,
                global,
            })
        }
        // TeX82 §1288's `shift_case` is entirely a command-core operation:
        // `scan_toks`, a `\uccode`/`\lccode` rewrite, and `back_list`. It
        // reaches no stomach state, so it completes inside the command
        // processor and its `back_list` push stays on the observed path.
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Uppercase | UnexpandablePrimitive::Lowercase),
        ) => {
            processor
                .shift_case(primitive == UnexpandablePrimitive::Uppercase)
                .map_err(command_error)?;
            Ok(ScannedStep::Continue)
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
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ShowTokens) => {
            let text = processor.scan_showtokens().map_err(command_error)?;
            Ok(ScannedStep::ShowTokens {
                tokens: text.tokens,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ShowIfs) => {
            Ok(ScannedStep::ShowIfs {
                conditions: processor.active_conditions(),
            })
        }
        // TeX82 §1290's `any_mode(xray): show_whatever` puts every \show
        // family in every mode; §1293's `show_lists_code` case reads no
        // operand at all.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ShowLists) => {
            Ok(ScannedStep::ShowLists)
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ShowGroups) => {
            Ok(ScannedStep::ShowGroups { diagnostic: None })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ShowBox) => {
            let (index, _) = processor.scan_showbox().map_err(command_error)?;
            Ok(ScannedStep::ShowBox { index })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Immediate) => {
            let extension = processor
                .scan_immediate_extension(processor.int_param(IntParam::PDF_OUTPUT) > 0)
                .map_err(command_error)?;
            if let ImmediateExtension::PdfImage(request) = extension {
                Ok(ScannedStep::PdfXImage {
                    request,
                    resource: PdfImageResource::Unavailable,
                })
            } else {
                Ok(ScannedStep::ImmediateExtension(extension))
            }
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HRule)
            if matches!(mode, Mode::Math | Mode::DisplayMath) =>
        {
            // TeX82 §1046 lists `mmode+hrule` among the "math-only cases in
            // non-math modes, or vice versa"; unlike `mmode+vrule` (§1056,
            // handled below), `\hrule` never reaches `scan_rule_spec` while
            // in math mode. §1047's `insert_dollar_sign` closes math with an
            // inserted `$` and replays `\hrule` in the resulting mode.
            processor
                .recover_missing_math_shift(command)
                .map_err(command_error)?;
            Ok(ScannedStep::MissingMathShift)
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
            Ok(ScannedStep::SetBox(SetBoxTarget {
                index: assignment.index,
                global,
            }))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VSplit) => Ok(ScannedStep::VSplit(
            processor.scan_vsplit().map_err(command_error)?,
        )),
        // TeX82 §1079's `make_box(box_code)` scans the register through
        // `scan_int` before handing the completed box-list operation to the
        // stomach. In particular, the first digit remains raw command input,
        // never an executor-side backup/replay artifact.
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Box | UnexpandablePrimitive::Copy),
        ) => {
            let register = processor.scan_box_register().map_err(command_error)?;
            Ok(ScannedStep::BoxRegister {
                index: register.index,
                copy: primitive == UnexpandablePrimitive::Copy,
                ships_out: boxes.pending_shipout,
            })
        }
        // TeX82 §1095's `hmode+un_vbox: head_for_vmode` ends an unrestricted
        // paragraph and retries the command in vertical mode. As with every
        // `head_for_vmode` command, this happens before `make_box` (§1079)
        // scans the register operand.
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::UnVBox
            | UnexpandablePrimitive::UnVCopy
            | UnexpandablePrimitive::PageDiscards
            | UnexpandablePrimitive::SplitDiscards,
        ) if mode == Mode::Horizontal => {
            processor
                .recover_stop_for_vertical_mode(command)
                .map_err(command_error)?;
            Ok(ScannedStep::Continue)
        }
        // The restricted-horizontal branch of §1095 cannot end a paragraph,
        // so it runs §§1064--1066 `off_save`. The recovered command is
        // retried only after the enclosing group has been closed; its
        // register operand must remain unread until that retry.
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::UnVBox
            | UnexpandablePrimitive::UnVCopy
            | UnexpandablePrimitive::PageDiscards
            | UnexpandablePrimitive::SplitDiscards,
        ) if mode == Mode::RestrictedHorizontal => {
            scan_off_save(processor, command, innermost_group)
        }
        // e-TeX 2.6 `etex.ch` [15.208, 45.999] assigns both saved-discard
        // enquiries the `un_vbox` command code with modifiers above
        // `copy_code`. TeX82 §1046 consequently routes their math-mode
        // occurrence through `insert_dollar_sign` before `unpackage` can
        // splice the saved list.
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::PageDiscards | UnexpandablePrimitive::SplitDiscards,
        ) if matches!(mode, Mode::Math | Mode::DisplayMath) => {
            processor
                .recover_missing_math_shift(command)
                .map_err(command_error)?;
            Ok(ScannedStep::MissingMathShift)
        }
        Meaning::UnexpandablePrimitive(
            primitive
            @ (UnexpandablePrimitive::PageDiscards | UnexpandablePrimitive::SplitDiscards),
        ) => Ok(ScannedStep::SavedVerticalDiscards(primitive)),
        // `\unhbox`/`\unhcopy` in (internal) vertical mode never reach here:
        // `starts_paragraph_in_vertical_mode` routes `vmode+un_hbox` through
        // §1090's shared backup above, before this register operand is ever
        // scanned. `\unvbox`/`\unvcopy` are not in that group.
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::UnHBox
            | UnexpandablePrimitive::UnHCopy
            | UnexpandablePrimitive::UnVBox
            | UnexpandablePrimitive::UnVCopy),
        ) => {
            let register = processor.scan_box_register().map_err(command_error)?;
            Ok(ScannedStep::Unbox {
                primitive,
                index: register.index,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::LastBox) => Ok(ScannedStep::LastBox),
        // TeX82's main-control dispatch on `abs(mode)+cur_cmd` (tex.web
        // §1073): `\raise`/`\lower` (`vmove`) are legal only outside vertical
        // mode (`hmode+vmove`, `mmode+vmove`); `\moveleft`/`\moveright`
        // (`hmove`) are legal only inside it (`vmode+hmove`). The three
        // remaining combinations (`vmode+vmove`, `hmode+hmove`,
        // `mmode+hmove`) are tex.web's "Forbidden cases" list and never
        // reach `scan_normal_dimen` at all -- only `report_illegal_case`
        // fires, so the dimension must not be scanned here.
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Raise | UnexpandablePrimitive::Lower),
        ) => {
            if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                Ok(ScannedStep::IllegalBoxShift {
                    token: command.spelling().semantic_token(),
                })
            } else {
                Ok(ScannedStep::BoxShift(
                    processor.scan_box_shift(primitive).map_err(command_error)?,
                ))
            }
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::MoveLeft | UnexpandablePrimitive::MoveRight),
        ) => {
            if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                Ok(ScannedStep::BoxShift(
                    processor.scan_box_shift(primitive).map_err(command_error)?,
                ))
            } else {
                Ok(ScannedStep::IllegalBoxShift {
                    token: command.spelling().semantic_token(),
                })
            }
        }
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
        // TeX82 §1167's `mmode+vcenter`:
        //
        //     mmode+vcenter: begin scan_spec(vcenter_group,false);
        //       normal_paragraph;
        //       push_nest; mode:=-vmode; prev_depth:=ignore_depth;
        //       if every_vbox<>null then
        //         begin_token_list(every_vbox,every_vbox_text);
        //       end;
        //
        // `\vcenter` is a *box* opener, not a math-text field: its body is an
        // internal vertical list built by the same §645 `scan_spec` prefix and
        // the same `push_nest; mode:=-vmode` as `\vbox`, and §1168 packages it
        // with `vpack` before wrapping it in a `vcenter_noad`. Scanning it as
        // a `math_group` field instead (an mlist) silently loses every
        // vertical-mode construction a `\vcenter` body is built from -- above
        // all `\halign`, which §1130 admits only in vertical mode, so plain's
        // `\pmatrix`/`\matrix`/`\cases`/`\eqalign` (all `\vcenter{\ialign{
        // ...}}`) collapsed to their `\mathstrut` alone (`umber2-johp.260`).
        //
        // Outside math mode `\vcenter` never reaches here: §1046's
        // `non_math(vcenter)` sends it through `insert_dollar_sign`, which is
        // the `P::VCenter` arm of the exhaustive fallback below.
        Meaning::UnexpandablePrimitive(primitive @ UnexpandablePrimitive::VCenter)
            if matches!(mode, Mode::Math | Mode::DisplayMath) =>
        {
            Ok(ScannedStep::BeginBox(
                processor
                    .scan_box_construction(primitive)
                    .map_err(command_error)?,
            ))
        }
        // TeX82 §1099's `begin_insert_or_adjust` -- any_mode(insert). `\insert`
        // is legal in every mode with no mode switch of its own, exactly like
        // `\penalty` and `\mark` above.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Insert) => {
            Ok(ScannedStep::BeginInsert(
                processor
                    .scan_insert_construction(false)
                    .map_err(command_error)?,
            ))
        }
        // TeX82 §1099's `begin_insert_or_adjust` with `cur_val:=255` fixed
        // (`if cur_cmd=vadjust then cur_val:=255`) rather than scanned --
        // `\vadjust` shares `\insert`'s exact class-255 body construction,
        // recognized in `finish_insert_or_adjust_group` below. Unlike
        // `\insert`, `\vadjust` is restricted to `hmode+vadjust`/
        // `mmode+vadjust`; `vmode+vadjust` is one of tex.web's "Forbidden
        // cases" (`@<Forbidden...@>=`), so vertical mode never reaches
        // `scan_box_group_opening` at all.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VAdjust) => {
            if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                Ok(ScannedStep::IllegalInsertOrAdjust {
                    token: command.spelling().semantic_token(),
                })
            } else {
                Ok(ScannedStep::BeginInsert(
                    processor
                        .scan_insert_construction(true)
                        .map_err(command_error)?,
                ))
            }
        }
        // TeX82 §1101's `make_mark` -- any_mode(mark). `p:=scan_toks(false,
        // true)`: a fully expanded balanced general text, exactly like
        // `\special`'s and `\message`'s bodies. Plain `\mark` fixes class
        // zero; the e-TeX numbered variant below scans its class first.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Mark) => Ok(ScannedStep::Mark {
            class: 0,
            tokens: processor
                .scan_balanced_text(true)
                .map_err(command_error)?
                .tokens,
        }),
        // e-TeX 2.6 `etex.ch` [26.424]'s `make_mark`: `\marks` first scans
        // one extended register number (recovering an invalid selector to
        // class zero), then performs TeX82's expanded mark-text scan.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Marks) => {
            let class = processor
                .scan_extended_register_index()
                .map_err(command_error)?;
            Ok(ScannedStep::Mark {
                class,
                tokens: processor
                    .scan_balanced_text(true)
                    .map_err(command_error)?
                    .tokens,
            })
        }
        // TeX82 §1095's `hmode+halign: head_for_vmode` ends an unrestricted
        // paragraph and retries the alignment in vertical mode.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HAlign)
            if mode == Mode::Horizontal =>
        {
            processor
                .recover_stop_for_vertical_mode(command)
                .map_err(command_error)?;
            Ok(ScannedStep::Continue)
        }
        // Restricted horizontal mode cannot end a paragraph, so §§1064--1066
        // close the enclosing group before retrying the same `\halign`.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HAlign)
            if mode == Mode::RestrictedHorizontal =>
        {
            scan_off_save(processor, command, innermost_group)
        }
        // `\halign` is legal directly in vertical mode (TeX82's
        // `vmode+halign:init_align`).
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HAlign) => {
            if matches!(mode, Mode::Math | Mode::DisplayMath)
                && innermost_group != Some(GroupKind::MathShift)
            {
                scan_off_save(processor, command, innermost_group)
            } else {
                Ok(ScannedStep::BeginAlignment { vertical: false })
            }
        }
        // Only `hmode+valign` reaches here: §1090 lists `vmode+valign` (unlike
        // `vmode+halign` above), so the shared backup already turned a bare
        // `\valign` in (internal) vertical mode into a paragraph start, and
        // the redelivered token arrives as `hmode+valign` -- embedded
        // alignment material inside the resulting paragraph's horizontal list.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VAlign) => {
            if matches!(mode, Mode::Math | Mode::DisplayMath)
                && innermost_group != Some(GroupKind::MathShift)
            {
                scan_off_save(processor, command, innermost_group)
            } else {
                Ok(ScannedStep::BeginAlignment { vertical: true })
            }
        }
        // TeX82 §1096: `hmode+par_end` first runs `off_save` when
        // `align_state<0`, then retries the same `\par` after the inserted
        // group closer. A malformed alignment entry can otherwise absorb all
        // following vertical material into its last cell.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par)
            if matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal)
                && processor.paragraph_end_needs_alignment_recovery() =>
        {
            scan_off_save(processor, command, innermost_group)
        }
        // TeX82 §§1046--1047 classify `mmode+par_end` as a math-mode
        // mismatch: insert `$`, then rescan the same `\par` after the math
        // list has closed. Treating it as an ordinary paragraph terminator
        // leaves the math group open and lets subsequent recovery close
        // unrelated groups instead.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par)
            if matches!(mode, Mode::Math | Mode::DisplayMath) =>
        {
            processor
                .recover_missing_math_shift(command)
                .map_err(command_error)?;
            Ok(ScannedStep::MissingMathShift)
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par) => Ok(ScannedStep::Paragraph),
        // TeX82 §1193 closes math only at `math_shift_group`; a `$` inside
        // any nested math group first runs §1064's `off_save`, which inserts
        // that group's required closer and retries this same shift.
        Meaning::CharToken {
            cat: Catcode::MathShift,
            ..
        } if matches!(mode, Mode::Math | Mode::DisplayMath)
            && innermost_group != Some(GroupKind::MathShift) =>
        {
            scan_off_save(processor, command, innermost_group)
        }
        Meaning::CharToken {
            cat: Catcode::MathShift,
            ..
        } => match mode {
            // §1090's shared backup already retried this exact shift after
            // `new_graf`; probing it in vertical mode would run before
            // `\everypar`.
            Mode::Vertical | Mode::InternalVertical => {
                unreachable!("§1090 backs a vertical-mode math shift up first")
            }
            // §1138 `init_math`: `hmode+math_shift`, for either sign of
            // `hmode`. The probe is `get_token`, and only `mode>0` -- the
            // unrestricted horizontal mode -- may consume the second `$`.
            Mode::Horizontal | Mode::RestrictedHorizontal => {
                let paired = processor
                    .scan_init_math_display_pair(mode == Mode::Horizontal)
                    .map_err(command_error)?;
                Ok(ScannedStep::MathShift { paired })
            }
            // §1194 `after_math` reaches §1197's `get_x_token` probe twice
            // over: once for a closing display (`m>=0` with `a=null`) and
            // once for a closing equation number (`mode=-m`).
            Mode::DisplayMath => {
                let paired = processor
                    .scan_display_end_math_shift()
                    .map_err(command_error)?;
                Ok(ScannedStep::MathShift { paired })
            }
            Mode::Math if display_eq_no => {
                let paired = processor
                    .scan_display_end_math_shift()
                    .map_err(command_error)?;
                Ok(ScannedStep::MathShift { paired })
            }
            // §1194's `m<0` closes inline math through `@<Finish math in
            // text@>`, which probes nothing at all.
            Mode::Math => Ok(ScannedStep::MathShift { paired: false }),
        },
        // §1090's shared backup already handled `vmode+letter` and
        // `vmode+other_char`, so a letter or other character reaching here is
        // in horizontal or math mode. `vmode+spacer` is §1045's `do_nothing`
        // and is the one category code of the three that stays here.
        Meaning::CharToken {
            ch,
            cat: cat @ (Catcode::Letter | Catcode::Other | Catcode::Space),
        } => Ok(ScannedStep::Character {
            ch,
            cat,
            origin: command.spelling().origin(),
            suppress_left_boundary: false,
        }),
        // TeX82 §1105's `any_mode(remove_item): delete_last`. No operand of
        // its own; `\unpenalty`/`\unkern`/`\unskip` differ only in which node
        // type is a removal target, decided at apply time against the live
        // mode nest and `Universe`.
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::UnPenalty
            | UnexpandablePrimitive::UnKern
            | UnexpandablePrimitive::UnSkip),
        ) => Ok(ScannedStep::DeleteLast(primitive)),
        // TeX82 §1111's "Forbidden cases" (`vmode+ital_corr`) vs. §1112's
        // `hmode+ital_corr`/`mmode+ital_corr`. Mode legality is decided here
        // (only `scan_command` sees `command` to back it up before the
        // Forbidden-case diagnostic); the actual append is mode-sensitive
        // apply-time work with no scan of its own.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ItalicCorrection) => {
            if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                Ok(ScannedStep::IllegalItalicCorrection {
                    token: command.spelling().semantic_token(),
                })
            } else {
                Ok(ScannedStep::ItalicCorrection)
            }
        }
        // §1090's `vmode+no_boundary` was already backed up above, so only
        // §1030's `hmode+no_boundary` and §1045's `mmode+no_boundary`
        // (`do_nothing`) reach here; both need only the live mode at apply
        // time, with no scan of their own.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::NoBoundary) => {
            Ok(ScannedStep::NoBoundary {
                suppress_right: false,
            })
        }
        // TeX82 §1171's `mmode+non_script` vs. §1046's `non_math(non_script)`
        // recovery, exactly mirroring the `\vskip`-in-math-mode gate above
        // (`recover_missing_math_shift` already implements §1047's
        // `insert_dollar_sign` generically for any command).
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::NonScript) => {
            if matches!(mode, Mode::Math | Mode::DisplayMath) {
                Ok(ScannedStep::NonScript)
            } else {
                processor
                    .recover_missing_math_shift(command)
                    .map_err(command_error)?;
                Ok(ScannedStep::MissingMathShift)
            }
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Patterns | UnexpandablePrimitive::Hyphenation),
        ) => {
            // TeX82 §960's `new_patterns` (`\patterns`) and §934's
            // `new_hyph_exceptions` (`\hyphenation`) each require §403's
            // `scan_left_brace` and then classify a `get_x_token` loop's
            // deliveries (§961, §935) as word characters, word boundaries, or
            // the closing brace. Neither absorbs a balanced text, so neither
            // enters §473's `absorbing` scanner status.
            //
            // §1252's INITEX-only guard on `\patterns` is applied at the
            // apply seam, not here: its production branch flushes the same
            // braced group (`repeat get_token until cur_cmd=right_brace`)
            // that the scan already consumes, and only the session -- not the
            // command core -- knows which binary tex.web's `init`/`tini`
            // split would have produced.
            let patterns = primitive == UnexpandablePrimitive::Patterns;
            // Captured for both seams before anything of the group is read:
            // the two rejections §1252 can raise each report at this cursor.
            let rejection_context = processor.error_context();
            let trie_built = patterns && !processor.hyphenation_patterns_open();
            if trie_built {
                // TeX82 §960's `trie_not_ready=false` branch diagnoses and
                // discards with `scan_toks(false,false)`. Unlike §961's
                // pattern-word loop, §473 therefore enters `absorbing`
                // before §403 reads the opening brace.
                let _ = processor.scan_balanced_text(false).map_err(command_error)?;
                return Ok(ScannedStep::HyphenationData {
                    words: Vec::new(),
                    pattern_specs: Vec::new(),
                    patterns: true,
                    rejection_context,
                    trie_built,
                });
            }
            let scanned = processor
                .scan_hyphenation_data(if patterns {
                    HyphenationDataKind::Patterns
                } else {
                    HyphenationDataKind::Exceptions
                })
                .map_err(command_error)?;
            Ok(ScannedStep::HyphenationData {
                words: scanned.words,
                pattern_specs: scanned.patterns,
                patterns,
                rejection_context,
                trie_built,
            })
        }
        // Every other `Meaning::UnexpandablePrimitive` reaching this point has
        // no named dispatch arm above (or is legal only in a mode this
        // `command` was not delivered in). `scan_unclassified_primitive` is
        // written as an exhaustive match over `UnexpandablePrimitive`
        // specifically so that a newly added variant fails to compile here
        // instead of silently falling through to a silent
        // `ScannedStep::Continue` -- see umber2-johp.69 and
        // `docs/tex_command_core.md`'s dispatch-completeness invariant.
        Meaning::UnexpandablePrimitive(primitive) => {
            scan_unclassified_primitive(processor, command, primitive, mode)
        }
        // Every other `Meaning` variant reaching this point has no named
        // dispatch arm above. `scan_unclassified_meaning` applies the same
        // remedy one level up the meaning word (umber2-johp.108): it is an
        // exhaustive match over `Meaning` -- and, inside its `CharToken`
        // case, over `Catcode` -- so a newly added variant fails to compile
        // there instead of reaching a silent `ScannedStep::Continue` here.
        meaning => scan_unclassified_meaning(processor, command, meaning, mode, innermost_group),
    }
}

/// TeX82 §1335 reports and frees unfinished conditionals innermost-first.
fn report_incomplete_conditions(
    stores: &mut Universe,
    incomplete: impl IntoIterator<Item = tex_command::IncompleteCondition>,
) {
    let mut printer = stores.printer();
    for condition in incomplete {
        printer
            .print_nl("(")
            .print_esc("end occurred ")
            .print("when ")
            .print_esc(condition.kind_name());
        if condition.source_line() != 0 {
            printer
                .print(" on line ")
                .print_int(i32::try_from(condition.source_line()).unwrap_or(i32::MAX));
        }
        printer.print(" was incomplete)");
    }
}

/// Runs TeX82 §1064's `off_save`, in full generality across every group
/// kind, not just the `RestrictedHorizontal` `\vskip` family that is this
/// function's first caller.
///
/// `off_save` recovers from a command that the current (innermost) group
/// cannot accommodate. Per §1066, a `bottom_level` group (no group open at
/// all) simply drops the command with an "Extra `<command>`" diagnostic --
/// there is nothing to close, so nothing is backed up or replayed. Otherwise
/// §1065 selects one of four closers to insert ahead of the backed-up
/// command, matching `cur_group`: a `semi_simple_group` needs the frozen,
/// redefinition-proof `\endgroup` control sequence (a plain `}` cannot close
/// it); a `math_shift_group` needs `$`; a `math_left_group` needs the
/// two-token `\right.` (frozen `\right` followed by a `.` other-character,
/// mirroring tex.web's `get_avail`-built two-node list); every other group
/// kind (box-making groups among them, the only case reachable from
/// restricted horizontal mode today) needs an ordinary `}`. Selecting and
/// inserting the closer is command-owned
/// (`CommandProcessor::recover_off_save`/`report_off_save_bottom_drop`); the
/// execute phase (`apply_scanned_step`) only prints the matching text once
/// the returned `ScannedStep` is applied.
fn scan_off_save(
    processor: &mut CommandProcessor<'_>,
    command: tex_command::CurrentCommand,
    innermost_group: Option<GroupKind>,
) -> Result<ScannedStep, ExecError> {
    let Some(kind) = innermost_group else {
        let token = command.spelling().semantic_token();
        processor.report_off_save_bottom_drop(&command);
        return Ok(ScannedStep::OffSaveBottomDrop { token });
    };
    match kind {
        GroupKind::SemiSimple => {
            let endgroup = processor
                .frozen_primitive_token("endgroup")
                .map_err(command_error)?;
            processor
                .recover_off_save(command, &[endgroup])
                .map_err(command_error)?;
            Ok(ScannedStep::OffSave(OffSaveCloser::EndGroup))
        }
        GroupKind::MathShift => {
            let dollar = Token::Char {
                ch: '$',
                cat: Catcode::MathShift,
            };
            processor
                .recover_off_save(command, &[dollar])
                .map_err(command_error)?;
            Ok(ScannedStep::OffSave(OffSaveCloser::MathShift))
        }
        GroupKind::MathLeft => {
            let right = processor
                .frozen_primitive_token("right")
                .map_err(command_error)?;
            let dot = Token::Char {
                ch: '.',
                cat: Catcode::Other,
            };
            processor
                .recover_off_save(command, &[right, dot])
                .map_err(command_error)?;
            Ok(ScannedStep::OffSave(OffSaveCloser::NullRight))
        }
        _ => {
            let right_brace = Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            };
            processor
                .recover_off_save(command, &[right_brace])
                .map_err(command_error)?;
            Ok(ScannedStep::OffSave(OffSaveCloser::RightBrace))
        }
    }
}

/// Classifies every `UnexpandablePrimitive` variant that reaches
/// `scan_command`'s final fallback arm.
///
/// This match is deliberately written over the full `UnexpandablePrimitive`
/// enum, not just the ~140 variants that currently lack a dispatch arm
/// above: the `unreachable` bucket below exists so that removing (or
/// mode-narrowing) one of `scan_command`'s existing named arms, or adding a
/// brand new primitive variant, fails to compile until this function is
/// updated with a deliberate decision. This is the mechanism umber2-johp.69
/// asked for: an unimplemented or wrong-mode primitive must stop the run at
/// its true site with a named error, never silently succeed while leaving
/// its own operand tokens (if any) in the input stream to be typeset as
/// literal text -- exactly how umber2-johp.67's `\patterns` bug and
/// umber2-johp.68's `\penalty` bug both escaped detection.
///
/// # Buckets
///
/// - `unreachable!()`: this primitive already has an explicit, mode-complete
///   dispatch arm earlier in `scan_command`'s outer match (including the
///   early math/family gates before that match). It can never actually
///   reach this function; if it does, `scan_command` was edited to narrow or
///   remove that arm without updating this classifier, which is exactly the
///   defect this function exists to catch -- panicking here is preferable to
///   silently reverting to the swallowed-primitive behavior.
/// - `unreachable!()` for the prefixes and `\ignorespaces`: these have no
///   `scan_command` arm at all and must not have one, because tex.web
///   consumes them above its big case (§1211's prefix loop, §1045's
///   `reswitch`). `dispatch_main_control_command` is where that happens, so
///   reaching this function names a caller that bypassed it.
/// - `Err(ExecError::UnimplementedPrimitive { .. })`: this primitive has no
///   dispatch at all yet in canonical main control, or is dispatched only
///   conditionally elsewhere (for example the math-noad family routed
///   through `scan_canonical_math_request`, or `\left`/`\right`/`\middle`'s
///   math-delimiter gate) and was reached outside that context, or is a
///   e-TeX/pdfTeX extension whose canonical routing has not been written.
///   Per umber2-johp.69's scope, this function does not implement any of
///   these; it only makes each one fail loudly and names it so follow-on
///   work can be tracked as ordinary chain links (see umber2-johp.74).
/// - `insert_dollar_sign` recovery: this primitive is a member of TeX82
///   §1046's "math-only cases in non-math modes" table (`non_math(...)` in
///   tex.web) -- it is dispatched correctly by `scan_canonical_math_request`
///   or the `\left`/`\right`/`\middle` gate above whenever `mode` actually is
///   `Math`/`DisplayMath`, so reaching this function at all proves `mode` is
///   not math. §1047's `insert_dollar_sign` backs the offending command up
///   behind a synthesized `$` (umber2-johp.56/.79's
///   `CommandProcessor::recover_missing_math_shift`, already used by the
///   `mmode+hrule`/`mmode+vskip`/`non_math(non_script)` arms above) so the
///   next two deliveries close math and replay the command in the resulting
///   mode. `\eqno`/`\leqno` are deliberately excluded from this bucket:
///   tex.web's separate `@<Forbidden cases@>=non_math(eq_no)` (§1144) routes
///   them through `report_illegal_case` ("You can't use `\eqno' in ...
///   mode") instead, via their own dedicated `ScannedStep::IllegalEqNo` arm
///   below (umber2-johp.88).
fn scan_unclassified_primitive(
    processor: &mut CommandProcessor<'_>,
    command: tex_command::CurrentCommand,
    primitive: UnexpandablePrimitive,
    mode: Mode,
) -> Result<ScannedStep, ExecError> {
    use UnexpandablePrimitive as P;
    match primitive {
        P::Accent
        | P::Advance
        | P::AfterAssignment
        | P::AfterGroup
        | P::BeginGroup
        | P::Box
        | P::CLeaders
        | P::CatCode
        | P::Char
        | P::CharDef
        | P::CloseIn
        | P::CloseOut
        | P::ControlSpace
        | P::Copy
        | P::Count
        | P::CountDef
        | P::Def
        | P::DelCode
        | P::Dimen
        | P::DimenDef
        | P::Discretionary
        | P::Divide
        | P::Dump
        | P::Edef
        | P::End
        | P::EndGroup
        | P::ErrMessage
        | P::Font
        | P::FontDimen
        | P::FutureLet
        | P::Gdef
        | P::HAlign
        | P::HBox
        | P::HFil
        | P::HFilNeg
        | P::HFill
        | P::HRule
        | P::HSkip
        | P::HSs
        | P::HyphenChar
        | P::Hyphenation
        | P::Immediate
        | P::Indent
        | P::Insert
        | P::InteractionMode
        | P::Kern
        | P::LastBox
        | P::LcCode
        | P::Leaders
        | P::Let
        | P::Lower
        | P::Lowercase
        | P::Marks
        | P::MathCharDef
        | P::MathCode
        | P::Message
        | P::MoveLeft
        | P::MoveRight
        | P::Multiply
        | P::Muskip
        | P::MuskipDef
        | P::NoIndent
        | P::OpenIn
        | P::OpenOut
        | P::Par
        | P::ParShape
        | P::Patterns
        | P::PageDiscards
        | P::PdfAnnot
        | P::PdfCatalog
        | P::PdfColorStack
        | P::PdfDest
        | P::PdfEndLink
        | P::PdfEndThread
        | P::PdfInfo
        | P::PdfLiteral
        | P::PdfNames
        | P::PdfObject
        | P::PdfOutline
        | P::PdfInterwordSpaceOff
        | P::PdfInterwordSpaceOn
        | P::PdfFakeSpace
        | P::PdfRunningLinkOff
        | P::PdfRunningLinkOn
        | P::PdfSpaceFont
        | P::PdfRefXForm
        | P::PdfRefXImage
        | P::PdfReferenceObject
        | P::PdfRestore
        | P::PdfSave
        | P::PdfSavePos
        | P::PdfSnapRefPoint
        | P::PdfSnapY
        | P::PdfSnapYComp
        | P::PdfResetTimer
        | P::PdfSetRandomSeed
        | P::PdfSetMatrix
        | P::PdfStartLink
        | P::PdfStartThread
        | P::PdfThread
        | P::PdfTrailer
        | P::PdfTrailerId
        | P::PdfXForm
        | P::PdfXImage
        | P::ItalicCorrection
        | P::NoBoundary
        | P::NonScript
        | P::Penalty
        | P::PrevDepth
        | P::PrevGraf
        | P::Raise
        | P::Read
        | P::ReadLine
        | P::ScriptFont
        | P::ScriptScriptFont
        | P::SetBox
        | P::SfCode
        | P::Shipout
        | P::Show
        | P::ShowBox
        | P::ShowGroups
        | P::ShowLists
        | P::ShowThe
        | P::ShowTokens
        | P::ShowIfs
        | P::SkewChar
        | P::Skip
        | P::SkipDef
        | P::Special
        | P::TextFont
        | P::Toks
        | P::ToksDef
        | P::UcCode
        | P::UnHBox
        | P::UnHCopy
        | P::UnKern
        | P::UnPenalty
        | P::UnSkip
        | P::UnVBox
        | P::UnVCopy
        | P::SplitDiscards
        | P::Uppercase
        | P::VAlign
        | P::VBox
        | P::VRule
        | P::VSplit
        | P::VTop
        | P::Wd
        | P::Ht
        | P::Dp
        | P::Write
        | P::XLeaders
        | P::Xdef
        | P::Mark
        | P::VAdjust
        | P::SetLanguage
        | P::BatchMode
        | P::ClubPenalties
        | P::DisplayWidowPenalties
        | P::InterLinePenalties
        | P::WidowPenalties
        | P::NonstopMode
        | P::ScrollMode
        | P::ErrorStopMode => unreachable!(
            "UnexpandablePrimitive::{primitive:?} has an explicit, mode-complete \
             scan_command dispatch arm and must never reach the exhaustive fallback"
        ),
        // Consumed by `dispatch_main_control_command` *before* the big case,
        // exactly as tex.web consumes them: §1211 `prefixed_command`'s
        // `while cur_cmd=prefix` loop absorbs `\global`/`\long`/`\outer` (and
        // e-TeX's `\protected`) into the accumulator `a` that the assignment
        // cases then read, and §1045's `any_mode(ignore_spaces)` re-enters
        // §1030's `reswitch:` with the next non-blank non-call token. None of
        // them is a mode-dispatched primitive -- §1210 files the prefixes
        // under `any_mode` -- so `scan_command` has, and must have, no arm for
        // them. Reaching this arm means some caller dispatched a command
        // without going through `dispatch_main_control_command`, which is the
        // narrowed-main-control defect of `umber2-johp.208`.
        P::Global | P::Long | P::Outer | P::Protected | P::IgnoreSpaces => unreachable!(
            "UnexpandablePrimitive::{primitive:?} is consumed by \
             dispatch_main_control_command before scan_command; reaching \
             scan_command means a caller bypassed the shared main-control step"
        ),
        // TeX82 §1046's `non_math(...)` table: each of these primitives is a
        // math-noad, math-style, or math-delimiter command whose *only*
        // canonical dispatch is `scan_canonical_math_request` (or the
        // `\left`/`\right`/`\middle` gate) under `Mode::Math`/`DisplayMath`.
        // Reaching this arm therefore proves `mode` is not math, which is
        // exactly tex.web's non-math table; §1047's `insert_dollar_sign`
        // recovers uniformly for the whole family via the same
        // `recover_missing_math_shift` helper the `mmode+hrule`/`mmode+vskip`/
        // `non_math(non_script)` arms above already use.
        P::Above
        | P::AboveWithDelims
        | P::Atop
        | P::AtopWithDelims
        | P::Delimiter
        | P::DisplayLimits
        | P::DisplayStyle
        | P::Left
        | P::Limits
        | P::MKern
        | P::MSkip
        | P::MathAccent
        | P::MathBin
        | P::MathChar
        | P::MathChoice
        | P::MathClose
        | P::MathInner
        | P::MathOp
        | P::MathOpen
        | P::MathOrd
        | P::MathPunct
        | P::MathRel
        | P::Middle
        | P::NoLimits
        | P::Over
        | P::OverWithDelims
        | P::Overline
        | P::Radical
        | P::Right
        | P::ScriptScriptStyle
        | P::ScriptStyle
        | P::TextStyle
        | P::Underline
        | P::VCenter => {
            processor
                .recover_missing_math_shift(command)
                .map_err(command_error)?;
            Ok(ScannedStep::MissingMathShift)
        }
        // TeX82 §1144's `@<Forbidden cases@>=non_math(eq_no)`: unlike the
        // math-noad family immediately above, `\eqno`/`\leqno` outside math
        // mode take `report_illegal_case` ("You can't use `\eqno' in ...
        // mode"), not `insert_dollar_sign` -- tex.web lists them under the
        // separate Forbidden-cases module even though they share the same
        // `eq_no` command code as the math-request vocabulary. Reaching this
        // arm proves `mode` is not `Math`/`DisplayMath` (that gate would
        // have consumed the primitive first via
        // `scan_canonical_math_request`'s `Request::EquationNumber`);
        // `mmode+eq_no` itself (gated by `privileged`/`cur_group`) is
        // unaffected.
        P::EqNo | P::LeftEqNo => Ok(ScannedStep::IllegalEqNo {
            token: command.spelling().semantic_token(),
        }),
        // TeX82 §1048's `any_mode(last_item)` Forbidden case: see
        // `ScannedStep::IllegalLastItem`. These internal-only quantities
        // reach this function
        // only when delivered standalone (not mid-scan, where
        // `internal_value_from_command` already consumes them), exactly
        // like `\eqno`/`\leqno` above.
        P::LastKern
        | P::LastPenalty
        | P::LastSkip
        | P::FontCharWd
        | P::FontCharHt
        | P::FontCharDp
        | P::FontCharIc
        | P::ParShapeLength
        | P::ParShapeIndent
        | P::ParShapeDimen
        | P::NumExpr
        | P::DimExpr
        | P::GlueExpr
        | P::MuExpr
        | P::GlueStretch
        | P::GlueShrink
        | P::GlueStretchOrder
        | P::GlueShrinkOrder
        | P::GlueToMu
        | P::MuToGlue => Ok(ScannedStep::IllegalLastItem {
            token: command.spelling().semantic_token(),
            context: processor.error_context(),
        }),
        // TeX82 §1126's `any_mode(car_ret), any_mode(tab_mark): align_error`.
        // `\cr` and `\crcr` carry the `car_ret` command code (chr `cr_code`
        // and `cr_cr_code`); `\span` carries `tab_mark` with chr `span_code`.
        // `get_next` (§342) only diverts them into a v-template when
        // `align_state=0`, so every other occurrence -- inside an alignment
        // cell whose braces are unbalanced, or outside any alignment at all --
        // is main control's to recover through §1127.
        P::Cr | P::CrCr | P::Span => scan_align_error(processor, command),
        // TeX82 §1126's `any_mode(no_align): no_align_error` and
        // `any_mode(omit): omit_error` (§1129). Both routines are a
        // `print_err`/`help2`/`error` triple and nothing else, so ignoring the
        // primitive is §1129's complete action; only the diagnostic is missing
        // (umber2-johp.110).
        P::NoAlign | P::Omit => Ok(ScannedStep::Continue),
        // e-TeX 2.6 `etex.ch` [17.3822--3880] adds four nonzero modifiers to
        // TeX82's `valign` command code. In horizontal mode `eTeX_enabled`
        // first checks `TeXXeT_state>0`; only the enabled branch appends the
        // corresponding zero-width math node. The ordinary zero modifier
        // remains `\valign` and is dispatched above as an alignment.
        P::BeginL | P::BeginR | P::EndL | P::EndR => {
            let direction = match primitive {
                P::BeginL => tex_state::node::Direction::BeginL,
                P::BeginR => tex_state::node::Direction::BeginR,
                P::EndL => tex_state::node::Direction::EndL,
                P::EndR => tex_state::node::Direction::EndR,
                _ => unreachable!("text-direction primitive matched above"),
            };
            Ok(ScannedStep::TextDirection {
                direction,
                enabled: processor.int_param(IntParam::TEX_XET_STATE) > 0,
            })
        }
        P::DiscretionaryHyphen
        | P::GlobalDefs
        | P::LetterspaceFont
        | P::PdfCopyFont
        | P::PdfEfCode
        | P::PdfFontAttr
        | P::PdfFontExpand
        | P::PdfGlyphToUnicode
        | P::PdfIncludeChars
        | P::PdfKnacCode
        | P::PdfKnbcCode
        | P::PdfKnbsCode
        | P::PdfLpCode
        | P::PdfMapFile
        | P::PdfMapLine
        | P::PdfNoBuiltinToUnicode
        | P::PdfNoLigatures
        | P::PdfRpCode
        | P::PdfShbsCode
        | P::PdfStbsCode
        | P::PdfTagCode
        | P::PdfTeXUnimplemented
        | P::QuitVMode
        | P::SpaceFactor
        | P::VFil
        | P::VFilNeg
        | P::VFill
        | P::VSkip
        | P::VSs => Err(ExecError::UnimplementedPrimitive {
            primitive,
            mode,
            origin: command.origin(),
        }),
    }
}

/// Classifies every `Meaning` variant that `scan_command`'s outer match does
/// not name, so that "no dispatch arm" can never again mean "succeeded and
/// consumed nothing".
///
/// This is `scan_unclassified_primitive`'s sibling one level up the meaning
/// word (umber2-johp.108). That function made the
/// `Meaning::UnexpandablePrimitive` payload compile-time exhaustive, but the
/// outer `Meaning` match kept an ordinary `_ => Ok(ScannedStep::Continue)`
/// wildcard, which became the remaining hiding place: an unrouted meaning
/// left its own operand tokens in the input to be typeset as literal text
/// arbitrarily far from the real defect (umber2-johp.106's `\pagegoal=100pt`
/// is the canonical example). Matching `Meaning` exhaustively here -- and
/// `Catcode` exhaustively inside the `CharToken` case -- converts each such
/// gap into either a deliberate, cited routing decision or a loud, named
/// failure, and makes a newly added variant a build failure.
///
/// # Buckets
///
/// - `Ok(...)`: tex.web routes this meaning somewhere canonical main control
///   already implements generically, cited per arm. Two of these arms
///   reproduce the cited section's *action* while its diagnostic is still
///   missing; both say so and name umber2-johp.110.
/// - `unreachable!()`: `scan_command`'s outer match already has an
///   unconditional named arm for this case, so it cannot arrive here. If it
///   does, that arm was narrowed without updating this classifier, which is
///   exactly the defect this function exists to catch.
/// - `insert_dollar_sign` recovery: this meaning is a member of TeX82
///   §1046's "math-only cases in non-math modes" table (`math_given` and the
///   `sup_mark`/`sub_mark` character categories). Each is dispatched
///   correctly by `scan_command`'s math gates whenever `mode` actually is
///   `Math`/`DisplayMath`, so reaching this function proves `mode` is not
///   math; §1047's `insert_dollar_sign` recovers it through the same
///   `recover_missing_math_shift` the primitive classifier's identical
///   bucket uses.
/// - `Err(ExecError::UnimplementedMeaning { .. })`: canonical main control
///   has no routing for this meaning yet, or the meaning should be
///   unreachable by a gullet invariant and the error names the broken
///   invariant exactly. Per umber2-johp.108's scope this function implements
///   none of them; it only makes each one fail loudly, tracked as
///   umber2-johp.111.
fn scan_unclassified_meaning(
    processor: &mut CommandProcessor<'_>,
    command: tex_command::CurrentCommand,
    meaning: Meaning,
    mode: Mode,
    innermost_group: Option<GroupKind>,
) -> Result<ScannedStep, ExecError> {
    match meaning {
        // TeX82 §1045's `any_mode(relax): do_nothing`. `\relax` -- and the
        // frozen relax `\noexpand` substitutes for its operand (§358) -- is
        // the one meaning for which "consume nothing and proceed" is the
        // whole specified behavior.
        Meaning::Relax => Ok(ScannedStep::Relax),
        // TeX82 §1048's Forbidden case `any_mode(last_item)`:
        // `report_illegal_case`. `Meaning::InternalInteger` is tex.web's
        // `last_item` command code with an operand other than
        // `\lastpenalty`/`\lastkern`/`\lastskip` (`\badness`,
        // `\inputlineno`, e-TeX's `\currentgrouplevel` family, pdfTeX's
        // `\pdflastxpos` family, ...). Like those three -- which
        // `scan_unclassified_primitive` already routes to the same
        // `ScannedStep` -- these are legal only as an internal-value operand
        // inside a scan, never as a delivered main-control command.
        Meaning::InternalInteger(_) => Ok(ScannedStep::IllegalLastItem {
            token: command.spelling().semantic_token(),
            context: processor.error_context(),
        }),
        Meaning::CharToken { ch, cat } => {
            scan_unclassified_char_token(processor, command, ch, cat, mode)
        }
        // `scan_command`'s outer match ends with an unconditional
        // `Meaning::UnexpandablePrimitive(primitive)` arm delegating to
        // `scan_unclassified_primitive`, so this payload never reaches here.
        Meaning::UnexpandablePrimitive(_) => {
            unreachable!("unexpandable primitives are classified by scan_unclassified_primitive")
        }
        // TeX82 §1210's `register`, `assign_int`/`assign_dimen`/
        // `assign_glue`/`assign_mu_glue`, `toks_register`/`assign_toks`, and
        // `set_font` assignment forms: `scan_command`'s outer match names
        // every one of them unconditionally.
        Meaning::CountRegister(_)
        | Meaning::DimenRegister(_)
        | Meaning::SkipRegister(_)
        | Meaning::MuskipRegister(_)
        | Meaning::ToksRegister(_)
        | Meaning::IntParam(_)
        | Meaning::DimenParam(_)
        | Meaning::GlueParam(_)
        | Meaning::MuGlueParam(_)
        | Meaning::TokParam(_)
        | Meaning::PageDimension(_)
        | Meaning::PageInteger(_)
        | Meaning::Font(_) => {
            unreachable!("scan_command names this assignment meaning unconditionally")
        }
        // TeX82 gives a `\chardef`'d character exactly the same three-mode
        // behavior as `\char`: §1090's `vmode+char_given` starts a
        // paragraph, §1034's `main_loop` typesets it in horizontal mode, and
        // §1154's `mmode+char_given: set_math_char(ho(math_code(cur_chr)))`
        // appends a math-char noad. tex.web keeps the two interchangeable
        // right down to §1038's ligature lookahead, which accepts
        // `char_given` and `char_num` at the same label, so this reuses
        // `\char`'s own already-dispatched `ScannedStep`; the only
        // difference is that the character code is already known and needs
        // no `scan_char_num`.
        Meaning::CharGiven(ch) => Ok(ScannedStep::CharacterCode {
            value: ch as i32,
            suppress_left_boundary: false,
        }),
        // TeX82 §1046's `non_math(math_given): insert_dollar_sign`, the same
        // recovery the whole math-only vocabulary takes outside math mode.
        // Reaching this arm proves `mode` is not `Math`/`DisplayMath`:
        // §1154's `mmode+math_given` is dispatched by `scan_command`'s
        // `scan_canonical_math_request` gate, which consumes the meaning
        // before its outer match runs.
        Meaning::MathCharGiven(_) => {
            processor
                .recover_missing_math_shift(command)
                .map_err(command_error)?;
            Ok(ScannedStep::MissingMathShift)
        }
        // TeX82 §370's `Complain about an undefined macro` reports and drops
        // the token. Umber's gullet currently delivers this meaning to main
        // control, which preserves that same observable transition here.
        Meaning::Undefined => Ok(ScannedStep::UndefinedControlSequence),
        // A macro is expanded by `get_x_token` (§380) and `\noexpand` turns
        // one into a frozen relax (§358), so neither should ever be
        // delivered as an unexpandable command. `\endcsname` is the one
        // deliberately unexpandable `ExpandablePrimitive`; TeX82 §1135's
        // `cs_error` gives it "Extra \endcsname", which is not routed here.
        Meaning::ExpandablePrimitive(tex_state::meaning::ExpandablePrimitive::EndCsName) => {
            Ok(ScannedStep::ExtraEndCsName)
        }
        Meaning::Macro { .. } | Meaning::ExpandablePrimitive(_) => {
            Err(unimplemented_meaning(&command, meaning, mode))
        }
        // TeX82 §1130's `vmode+endv,hmode+endv: do_endv` (§1131) and §1046's
        // `mmode+endv: insert_dollar_sign`. `scan_alignment_delivery_step`
        // implements the in-alignment half of §1131 before it ever calls
        // `scan_command`; an `endv` reaching main control by any other route
        // ("a devious user might force an `endv` command to occur just about
        // anywhere", §1131) has no dispatch.
        Meaning::EndV => {
            if matches!(mode, Mode::Math | Mode::DisplayMath) {
                processor
                    .recover_missing_math_shift(command)
                    .map_err(command_error)?;
                Ok(ScannedStep::MissingMathShift)
            } else {
                scan_off_save(processor, command, innermost_group)
            }
        }
        // An opcode `tex-state`'s meaning decoder itself does not recognize.
        Meaning::Unknown(_) => Err(unimplemented_meaning(&command, meaning, mode)),
    }
}

/// Classifies the character-token category codes that `scan_command`'s outer
/// match does not name, exhaustively over [`Catcode`].
///
/// See [`scan_unclassified_meaning`] for the bucket definitions.
fn scan_unclassified_char_token(
    processor: &mut CommandProcessor<'_>,
    command: tex_command::CurrentCommand,
    ch: char,
    cat: Catcode,
    mode: Mode,
) -> Result<ScannedStep, ExecError> {
    match cat {
        // TeX82 §1046's `non_math(sup_mark)`/`non_math(sub_mark)`:
        // §1047's `insert_dollar_sign` backs the command up behind a
        // synthesized `$`. Reaching this arm proves `mode` is not
        // `Math`/`DisplayMath`, since `scan_command`'s superscript/subscript
        // gates consume both categories before its outer match in those two
        // modes.
        Catcode::Superscript | Catcode::Subscript => {
            processor
                .recover_missing_math_shift(command)
                .map_err(command_error)?;
            Ok(ScannedStep::MissingMathShift)
        }
        // TeX82 §1045's `any_mode(mac_param): report_illegal_case`. A bare
        // parameter token has no operand of its own; the command is consumed
        // after the diagnostic and main control continues.
        Catcode::Parameter => Ok(ScannedStep::IllegalMacroParameter {
            token: command.spelling().semantic_token(),
        }),
        // TeX82 §1126's `any_mode(tab_mark)` (a category-4 character token)
        // and `any_mode(car_ret)` (a category-5 one, which `get_next`'s
        // §344 end-of-line handling normally consumes before delivery).
        // Both command codes take §1127's `align_error`.
        Catcode::AlignmentTab | Catcode::EndLine => scan_align_error(processor, command),
        // Category codes that never become a delivered command: `get_next`
        // (§341-§356) consumes escape characters into control-sequence
        // spellings, resolves active characters to their own meanings, drops
        // ignored and comment characters, and reports invalid characters at
        // the lexer boundary.
        Catcode::Escape
        | Catcode::Active
        | Catcode::Ignored
        | Catcode::Comment
        | Catcode::Invalid => Err(unimplemented_meaning(
            &command,
            Meaning::CharToken { ch, cat },
            mode,
        )),
        // `scan_command`'s outer match names all five of these
        // unconditionally.
        Catcode::BeginGroup
        | Catcode::EndGroup
        | Catcode::MathShift
        | Catcode::Space
        | Catcode::Letter
        | Catcode::Other => {
            unreachable!("scan_command names this character category unconditionally")
        }
    }
}

/// TeX82 §1126's `any_mode(car_ret), any_mode(tab_mark): align_error`.
///
/// This is the single entry point for every command tex.web routes to
/// `align_error`: the `car_ret` command code (`\cr`, `\crcr`, and a category-5
/// character token) and the `tab_mark` command code (`\span` and a category-4
/// character token). §1127 chooses between dropping the delimiter (§1128, when
/// `abs(align_state)>2`) and backing it up behind an inserted brace, entirely
/// from the command-owned `align_state`; main control only records whether the
/// inserted brace opens a recovery simple group for §1131's `off_save`.
fn scan_align_error(
    processor: &mut CommandProcessor<'_>,
    command: tex_command::CurrentCommand,
) -> Result<ScannedStep, ExecError> {
    let token = command.spelling().semantic_token();
    match processor
        .recover_align_error(command)
        .map_err(command_error)?
    {
        None => Ok(ScannedStep::MisplacedAlignmentDelimiter { token }),
        Some(recovery) => Ok(ScannedStep::AlignmentRecovery {
            opens_simple_group: matches!(
                recovery,
                tex_state::token::Token::Char {
                    cat: Catcode::BeginGroup,
                    ..
                }
            ),
        }),
    }
}

fn unimplemented_meaning(
    command: &tex_command::CurrentCommand,
    meaning: Meaning,
    mode: Mode,
) -> ExecError {
    ExecError::UnimplementedMeaning {
        meaning,
        mode,
        origin: command.origin(),
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
        _ => {
            return Ok(ScannedStep::InvalidArithmeticTarget {
                primitive,
                target: tex_command::PrintCommand::from_current(&target_command),
            });
        }
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

#[cfg(test)]
fn replay_text(tokens: &[tex_state::token::Token]) -> String {
    tokens
        .iter()
        .filter_map(|token| match token {
            tex_state::token::Token::Char { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect()
}

fn canonical_write_text(tokens: &[Token], stores: &Universe) -> String {
    let mut text = String::new();
    for &token in tokens {
        tex_expand::append_token_string_text(stores, token, &mut text);
    }
    let mut text = crate::diagnostics::print_text_with_newlinechar(stores, &text);
    text.push('\n');
    text
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
    fuel: &mut tex_command::CommandFuel,
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
                    crate::assignments::append_whatsit(
                        modes,
                        stores,
                        fuel,
                        Whatsit::PdfAnnotation {
                            object: record.object(),
                        },
                    )?;
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
            crate::assignments::append_whatsit(
                modes,
                stores,
                fuel,
                Whatsit::PdfLinkStart {
                    object: record.object(),
                },
            )?;
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
            crate::assignments::append_whatsit(
                modes,
                stores,
                fuel,
                Whatsit::PdfLinkEnd {
                    object: open.record.object(),
                },
            )?;
        }
        PdfNavigationRequest::Outline(PdfOutlineRequest {
            attributes,
            action,
            count,
            title,
        }) => {
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfoutline"));
            }
            stores
                .create_pdf_outline(
                    attributes.map_or(TokenListId::EMPTY, |value| value.tokens.token_list()),
                    action,
                    count,
                    title.tokens.token_list(),
                )
                .map_err(|_| ExecError::PdfObjectCapacity)?;
            reserve_navigation_action_targets(stores, action)?;
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
            crate::assignments::append_whatsit(
                modes,
                stores,
                fuel,
                Whatsit::PdfDestination(Box::new(tex_state::node::PdfDestinationNode {
                    identifier,
                    structure,
                    kind,
                })),
            )?;
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
            crate::assignments::append_whatsit(
                modes,
                stores,
                fuel,
                Whatsit::PdfThread(Box::new(tex_state::node::PdfThreadNode {
                    identifier,
                    dimensions,
                    attributes: attributes
                        .map_or(TokenListId::EMPTY, |value| value.tokens.token_list()),
                    running,
                })),
            )?;
        }
        PdfNavigationRequest::EndThread => {
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfendthread"));
            }
            crate::assignments::append_whatsit(modes, stores, fuel, Whatsit::PdfEndThread)?;
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
    command: &CommandState,
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
            PdfGraphicsRequest::SnapReferencePoint => "pdfsnaprefpoint",
            PdfGraphicsRequest::SnapY { .. } => "pdfsnapy",
            PdfGraphicsRequest::SnapYComp { .. } => "pdfsnapycomp",
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
        PdfGraphicsRequest::SnapReferencePoint => Node::Whatsit(Whatsit::PdfSnapRefPoint),
        PdfGraphicsRequest::SnapY { glue } => {
            if glue.width.raw() < 0 {
                return Err(ExecError::PdfNavigation(
                    "pdfTeX error (ext1): negative snap glue",
                ));
            }
            Node::Whatsit(Whatsit::PdfSnapY {
                glue: stores.intern_glue(glue),
            })
        }
        PdfGraphicsRequest::SnapYComp { ratio } => Node::Whatsit(Whatsit::PdfSnapYComp { ratio }),
        PdfGraphicsRequest::ColorStack { id, action } => {
            // pdftex.web's `<Implement \pdfcolorstack>` reports all three of
            // these through `print_err`/`error`, so each is a counted error
            // with a context display, not a bare note.
            let id = if id < 0 {
                let context = command.output_open_context(&stores.command_context());
                crate::error_report::report_error(
                    stores,
                    "Invalid negative color stack number",
                    &[
                        "I'll use default color stack 0 here.",
                        "Proceed, with fingers crossed.",
                    ],
                    context,
                )?;
                0
            } else if !stores.has_pdf_color_stack(id as u32) {
                let context = command.output_open_context(&stores.command_context());
                crate::error_report::report_error(
                    stores,
                    &format!("Unknown color stack number {id}"),
                    &[
                        "Allocate and initialize a color stack with \\pdfcolorstackinit.",
                        "I'll use default color stack 0 here.",
                        "Proceed, with fingers crossed.",
                    ],
                    context,
                )?;
                0
            } else {
                id as u32
            };
            let Some(action) = action else {
                let context = command.output_open_context(&stores.command_context());
                crate::error_report::report_error(
                    stores,
                    "Color stack action is missing",
                    &[
                        "The expected actions for \\pdfcolorstack:",
                        "    set, push, pop, current",
                        "I'll ignore the color stack command.",
                    ],
                    context,
                )?;
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
    modes.current_list_mutation().push(node);
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
    fuel: &mut tex_command::CommandFuel,
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
            crate::assignments::append_whatsit(
                modes,
                stores,
                fuel,
                Whatsit::PdfRefXForm {
                    object: form.object(),
                    width: form.width(),
                    height: form.height(),
                    depth: form.depth(),
                },
            )?;
        }
        PdfFormRequest::Create {
            attr,
            resources,
            box_register,
        } => {
            // pdfTeX allocates the form identity before it consumes the box.
            let identity = stores
                .reserve_pdf_form()
                .map_err(|_| ExecError::PdfObjectCapacity)?;
            let list = stores
                .take_box_reg_same_level(box_register)
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

/// Selects TeX82 §1370 `write_out`'s destination for a stream number that
/// §1350's `new_write_whatsit` has already normalized into `0..=17`.
///
/// §1342: `write_open[17]` stands for every negative stream and
/// `write_open[16]` for every stream above 15, and both are permanently
/// closed. §1370 therefore sends 17 to the log alone (`if (j=17) and
/// (selector=term_and_log) then selector:=log_only`) and 16 to the terminal
/// and log.
fn replay_write_sink(value: tex_command::WriteStreamSelector) -> PrintSink {
    match value {
        tex_command::WriteStreamSelector::Stream(slot) => PrintSink::Stream(StreamSlot::new(slot)),
        tex_command::WriteStreamSelector::Negative => PrintSink::Log,
        tex_command::WriteStreamSelector::AboveRange => PrintSink::TerminalAndLog,
    }
}

/// Converts a stream number already normalized by its command-owned
/// restricted scan. Replay never owns range recovery or its diagnostic.
fn replay_stream_slot(value: i32) -> StreamSlot {
    debug_assert!((0..tex_state::world::STREAM_SLOT_COUNT as i32).contains(&value));
    StreamSlot::new(value as u8)
}

fn replay_openout_target(name: String) -> String {
    let mut path = PathBuf::from(name);
    if path.extension().is_none() {
        path.set_extension("tex");
    }
    path.to_string_lossy().into_owned()
}

/// A committed `eqtb` mutation captured for the command observer, together
/// with when its observed value becomes readable.
///
/// Most assignments know their committed value while they are still a
/// `ScannedStep`, so the record is captured before structural application and
/// merely held until that application commits, preserving the observer's
/// canonical order. TeX82 §1236's `do_register_command` is the exception: it
/// folds the target's *current* `eqtb` value into the result, so its record
/// can only be read after the single `word_define`/`define` exit commits.
enum PendingMutation {
    Captured(MutationRecord),
    Arithmetic {
        target: ArithmeticTarget,
        global: bool,
        profile: CommandProfile,
    },
}

impl PendingMutation {
    fn resolve(self, stores: &Universe) -> MutationRecord {
        match self {
            Self::Captured(record) => record,
            Self::Arithmetic {
                target,
                global,
                profile,
            } => committed_arithmetic_mutation(target, global, stores, profile),
        }
    }
}

/// Serializes a committed glue value the way the reference instrumentation's
/// `umber_trace_glue_value` does.
fn glue_mutation_value(value: &GlueSpec) -> String {
    format!(
        "glue:width={};stretch={};stretch_order={};shrink={};shrink_order={}",
        value.width.raw(),
        value.stretch.raw(),
        glue_order_name(value.stretch_order),
        value.shrink.raw(),
        glue_order_name(value.shrink_order),
    )
}

/// Reads back the value TeX82 §1236's `do_register_command` committed at its
/// single exit.
///
/// §1236 computes `cur_val` from the target's current value (§1238's
/// `cur_val+eqtb[l].int`, §1239's glue sum, §1240's `mult_integers`/
/// `x_over_n`) and then commits it exactly once -- `word_define(l,cur_val)`
/// for `int_val`/`dimen_val` targets, `define(l,glue_ref,cur_val)` for
/// `glue_val`/`mu_val` ones. The observed record therefore carries the
/// *result*, never the scanned operand, and is read after application rather
/// than before it. An `arith_error` return in §1236 leaves `eqtb` untouched
/// and is observed as no mutation at all, which falls out of resolving this
/// only when application succeeded.
fn committed_arithmetic_mutation(
    target: ArithmeticTarget,
    global: bool,
    stores: &Universe,
    profile: CommandProfile,
) -> MutationRecord {
    match target {
        ArithmeticTarget::IntegerRegister(index) => MutationRecord {
            target: "register",
            value: format!("count:{index}={}", stores.count(index)),
            key: None,
            tokens: None,
            global,
        },
        ArithmeticTarget::DimensionRegister(index) => MutationRecord {
            target: "register",
            value: format!("scaled:{}", stores.dimen(index).raw()),
            key: Some(format!("dimen:{index}")),
            tokens: None,
            global,
        },
        ArithmeticTarget::GlueRegister { index, mu } => MutationRecord {
            target: "register",
            value: glue_mutation_value(&stores.glue(if mu {
                stores.muskip(index)
            } else {
                stores.skip(index)
            })),
            key: Some(format!("{}:{index}", if mu { "muskip" } else { "skip" })),
            tokens: None,
            global,
        },
        ArithmeticTarget::IntegerParameter(index) => MutationRecord {
            target: "parameter",
            value: format!(
                "{}={}",
                parameter_mutation_key_for_dialect(
                    profile.dialect(),
                    ParameterClass::Integer,
                    index,
                ),
                stores.int_param(IntParam::new(index))
            ),
            key: None,
            tokens: None,
            global,
        },
        ArithmeticTarget::DimensionParameter(index) => MutationRecord {
            target: "parameter",
            value: format!(
                "scaled:{}",
                stores.dimen_param(DimenParam::new(index)).raw()
            ),
            key: Some(parameter_mutation_key_for_dialect(
                profile.dialect(),
                ParameterClass::Dimension,
                index,
            )),
            tokens: None,
            global,
        },
        // TeX82 keeps `\thinmuskip`/`\medmuskip`/`\thickmuskip` in the same
        // `glue_par` block as the ordinary glue parameters (§224), and the
        // reference instrumentation names that whole block
        // `glue_parameter:<n>`; the `mu` flag only selected which scanner
        // §1236 used for the operand.
        ArithmeticTarget::GlueParameter { index, .. } => MutationRecord {
            target: "parameter",
            value: glue_mutation_value(&stores.glue(stores.glue_param(GlueParam::new(index)))),
            key: Some(parameter_mutation_key_for_dialect(
                profile.dialect(),
                ParameterClass::Glue,
                index,
            )),
            tokens: None,
            global,
        },
    }
}

/// Classifies the committed `eqtb` mutation, if any, that applying a scanned
/// step performs, for the command observer.
///
/// TeX82 §1211's `prefixed_command` is TeX's single assignment dispatcher,
/// and the reference instrumentation observes exactly the `eq_define`,
/// `eq_word_define`, `geq_define`, and `geq_word_define` writes (§277-§279)
/// that run inside it. Its `umber_trace_eq_mutation`/`umber_trace_word_mutation`
/// classifiers then keep only the `eqtb` regions they can name: control
/// sequence meanings, the glue/token/integer/dimension parameter blocks, the
/// `\count`/`\dimen`/`\skip`/`\muskip`/`\toks` registers, and the
/// `\catcode`/`\lccode`/`\uccode`/`\sfcode`/`\mathcode`/`\delcode` tables.
///
/// The match is exhaustive over [`ScannedStep`] deliberately (umber2-johp.124,
/// the same defect shape removed by umber2-johp.69/.97/.108/.123). This was an
/// `if let` chain ending in `None`, and a step that fell off its end did not
/// merely get mislabeled -- it produced *no* event where the oracle produced
/// one, which desynchronizes every following event in the trace. A newly
/// added step now has to state which bucket below it belongs to.
///
/// # Buckets
///
/// - `Some(PendingMutation::Captured(..))`: the assignment's committed value
///   is already known from the scanned step.
/// - `Some(PendingMutation::Arithmetic { .. })`: §1236's result is only
///   readable after application; see [`committed_arithmetic_mutation`].
/// - `None`: the step writes no `eqtb` location the reference instrumentation
///   names -- either it is not an `eqtb` write at all, or it lands in a region
///   `umber_trace_eq_mutation` deliberately declines to serialize. Each arm
///   cites which.
/// - `unreachable!()`: [`CanonicalMainControl::apply_host_owned_step`] applies
///   the step before this classifier runs, so reaching it means that routing
///   was removed without updating this classifier.
fn applied_mutation_observation(
    scanned: &ScannedStep,
    stores: &Universe,
    profile: CommandProfile,
) -> Option<PendingMutation> {
    // e-TeX §§277-278 return before changing the save stack when extended
    // mode locally reassigns an identical eqtb value. Suppress the
    // corresponding observer record at the same semantic boundary;
    // otherwise instrumentation reports a mutation the engine did not
    // canonically perform.
    if etex_redundant_local_definition_step(stores, scanned) {
        return None;
    }
    let captured = match scanned {
        // -- Registers: §1226's `toks_register` and §1228's `register` cases,
        // whose `eqtb` slots the instrumentation names `count:<n>`,
        // `dimen:<n>`, `skip:<n>`, `muskip:<n>`, and `toks:<n>`.
        ScannedStep::Count {
            index,
            value,
            global,
        } => MutationRecord {
            target: "register",
            value: format!("count:{index}={value}"),
            key: None,
            tokens: None,
            global: *global,
        },
        ScannedStep::Dimen {
            index,
            value,
            global,
        } => MutationRecord {
            target: "register",
            value: format!("scaled:{}", value.raw()),
            key: Some(format!("dimen:{index}")),
            tokens: None,
            global: *global,
        },
        ScannedStep::Skip {
            index,
            value,
            global,
        } => MutationRecord {
            target: "register",
            value: glue_mutation_value(value),
            key: Some(format!("skip:{index}")),
            tokens: None,
            global: *global,
        },
        ScannedStep::Muskip {
            index,
            value,
            global,
        } => MutationRecord {
            target: "register",
            value: glue_mutation_value(value),
            key: Some(format!("muskip:{index}")),
            tokens: None,
            global: *global,
        },
        ScannedStep::Toks {
            index,
            tokens,
            global,
        } => MutationRecord {
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
        },
        // -- Parameters: §1226/§1227's token-list parameters and §1228's
        // `assign_int`/`assign_dimen`/`assign_glue`/`assign_mu_glue` cases.
        ScannedStep::IntParam {
            index,
            value,
            global,
        } => MutationRecord {
            target: "parameter",
            value: format!(
                "{}={value}",
                parameter_mutation_key_for_dialect(
                    profile.dialect(),
                    ParameterClass::Integer,
                    *index,
                )
            ),
            key: None,
            tokens: None,
            global: *global,
        },
        ScannedStep::DimenParam {
            index,
            value,
            global,
        } => MutationRecord {
            target: "parameter",
            value: format!("scaled:{}", value.raw()),
            key: Some(parameter_mutation_key_for_dialect(
                profile.dialect(),
                ParameterClass::Dimension,
                *index,
            )),
            tokens: None,
            global: *global,
        },
        ScannedStep::GlueParam {
            index,
            value,
            global,
        } => MutationRecord {
            target: "parameter",
            value: glue_mutation_value(value),
            key: Some(parameter_mutation_key_for_dialect(
                profile.dialect(),
                ParameterClass::Glue,
                *index,
            )),
            tokens: None,
            global: *global,
        },
        ScannedStep::TokParam {
            index,
            tokens,
            global,
        } => MutationRecord {
            target: "parameter",
            value: "tokens".into(),
            key: Some(parameter_mutation_key_for_dialect(
                profile.dialect(),
                ParameterClass::Token,
                *index,
            )),
            tokens: Some(
                stores
                    .tokens(tokens.token_list())
                    .iter()
                    .copied()
                    .map(|token| observed_macro_token(token, stores))
                    .collect(),
            ),
            global: *global,
        },
        // -- Code tables: §1230's `def_code` command. tex.web §232 lists
        // cat_code, lc_code, uc_code, sf_code, math_code, and del_code as the
        // eqtb code-table regions it writes; every scan arm that produces
        // `ScannedStep::CodeTable` must have a corresponding arm here.
        ScannedStep::CodeTable {
            primitive,
            character,
            value,
            global,
        } => {
            let (target, value) = match primitive {
                UnexpandablePrimitive::CatCode => {
                    ("catcode", format!("{}={value}", u32::from(*character)))
                }
                UnexpandablePrimitive::LcCode => (
                    "code_table",
                    format!("lccode:{}={value}", u32::from(*character)),
                ),
                UnexpandablePrimitive::UcCode => (
                    "code_table",
                    format!("uccode:{}={value}", u32::from(*character)),
                ),
                UnexpandablePrimitive::SfCode => (
                    "code_table",
                    format!("sfcode:{}={value}", u32::from(*character)),
                ),
                UnexpandablePrimitive::MathCode => (
                    "code_table",
                    format!("mathcode:{}={value}", u32::from(*character)),
                ),
                UnexpandablePrimitive::DelCode => (
                    "code_table",
                    format!("delcode:{}={value}", u32::from(*character)),
                ),
                _ => unreachable!("only code-table primitives are scanned"),
            };
            MutationRecord {
                target,
                value,
                key: None,
                tokens: None,
                global: *global,
            }
        }
        // -- Meanings: §1221's `let`, §1224's `shorthand_def`, and §1218's
        // macro `def`. §1224's provisional `define(p,relax,256)` is committed
        // and observed by the command-owned scanner that performs it, so only
        // the final meaning is observed here.
        // §1221's `let` commits `define(p, eq_type(q), equiv(q))`: the copied
        // *meaning*, never the source control sequence's spelling. The
        // observation must therefore name the meaning the same way raw
        // delivery would, which is what `meaning_mutation_value` does
        // (`umber2-johp.141`).
        ScannedStep::Let {
            target,
            meaning,
            global,
            ..
        } => {
            let (value, tokens) = meaning_mutation_value(*meaning, stores);
            MutationRecord {
                target: "meaning",
                value,
                key: Some(stores.resolve(*target).to_owned()),
                tokens,
                global: *global,
            }
        }
        ScannedStep::CharacterDefinition {
            primitive,
            target,
            value,
            global,
            ..
        } => {
            // §1224 defines the meaning from `cur_val` *after* §434/§436's
            // recovery, so the observed mutation carries the recovered value.
            let value = match primitive {
                UnexpandablePrimitive::CharDef => format!("character:{value}"),
                UnexpandablePrimitive::MathCharDef => format!("integer:{value}"),
                _ => unreachable!("character-definition step carries only §1224 primitives"),
            };
            MutationRecord {
                target: "meaning",
                value,
                key: Some(stores.resolve(*target).to_owned()),
                tokens: None,
                global: *global,
            }
        }
        ScannedStep::RegisterDefinition {
            primitive,
            target,
            global,
            ..
        } => {
            let value = match primitive {
                UnexpandablePrimitive::CountDef => "assign_int",
                UnexpandablePrimitive::DimenDef => "assign_dimen",
                UnexpandablePrimitive::SkipDef => "assign_glue",
                UnexpandablePrimitive::MuskipDef => "assign_mu_glue",
                UnexpandablePrimitive::ToksDef => "assign_toks",
                _ => unreachable!("register-definition step carries only §1224 primitives"),
            };
            MutationRecord {
                target: "meaning",
                value: value.into(),
                key: Some(stores.resolve(*target).to_owned()),
                tokens: None,
                global: *global,
            }
        }
        ScannedStep::MacroDefinition {
            target,
            flags,
            parameter_text,
            replacement_text,
            global,
            ..
        } => MutationRecord {
            target: "meaning",
            value: "macro definition".into(),
            key: Some(stores.resolve(*target).to_owned()),
            tokens: Some(observed_stored_macro_body(
                *flags,
                parameter_text.token_list(),
                replacement_text.token_list(),
                stores,
            )),
            global: *global,
        },
        // -- §1236's `do_register_command`, whose committed value is only
        // readable after application.
        ScannedStep::Arithmetic { target, global, .. } => {
            return Some(PendingMutation::Arithmetic {
                target: *target,
                global: *global,
                profile,
            });
        }
        // -- Assignments that do write `eqtb`, into a region the reference
        // instrumentation deliberately declines to name: §1241's `set_box`
        // writes `box_base`, §1217's `set_font` writes `cur_font_loc`,
        // §1234's `def_family` writes `math_font_base`, and §1248's
        // `set_shape` writes `par_shape_loc`, whose equivalent is a pointer
        // to a scaled list rather than a serializable value.
        // `umber_trace_eq_mutation` classifies each of these as family -1 and
        // returns without an event, so Umber must stay silent for exactly
        // these four.
        ScannedStep::SetBox(..)
        | ScannedStep::FontSelect { .. }
        | ScannedStep::MathFamily { .. }
        | ScannedStep::ParagraphShape { .. }
        | ScannedStep::PenaltyArray { .. } => return None,
        // -- Assignments whose committed state lives outside `eqtb` entirely,
        // so no `eq_define`/`eq_word_define` runs and no mutation is observed
        // on either side: §1247's `alter_box_dimen` (a box node's `mem`
        // fields), §1243's `alter_aux` and §1244's `alter_prev_graf` (the mode
        // `nest`), §1245's `alter_page_so_far` and §1246's `alter_integer`
        // (`page_so_far`, `dead_cycles`, `insert_penalties`), §1253's
        // `assign_font_dimen`/`assign_font_int` (`font_info`, `hyphen_char`,
        // `skew_char`), §1252's `hyph_data` (the pattern trie and exception
        // table), and §1265's `new_interaction` (the `interaction` global).
        ScannedStep::BoxDimensionAssignment { .. }
        | ScannedStep::PrevDepth { .. }
        | ScannedStep::SpaceFactor { .. }
        | ScannedStep::IllegalSpaceFactor { .. }
        | ScannedStep::PrevGraf { .. }
        | ScannedStep::PageDimension { .. }
        | ScannedStep::PageInteger { .. }
        | ScannedStep::FontDimen { .. }
        | ScannedStep::FontInteger { .. }
        | ScannedStep::HyphenationData { .. }
        | ScannedStep::SetInteractionMode(..)
        | ScannedStep::SetInteractionModeValue(..) => return None,
        // -- TeX82 §1257's `new_font` observes only the provisional
        // `define(u,set_font,null_font)`: its common ending writes the loaded
        // font number directly with `equiv(u):=f`. e-TeX change [49.1257]
        // deliberately replaces that direct write with
        // `define(u,set_font,f)` for e-TeX tracing, and pdfTeX inherits the
        // same change. The command scanner already observes the provisional
        // definition; only an e-TeX-capable dialect observes this applied
        // final definition after all scanner records. Holding the record in
        // the operation buffer also makes resource suspension discard both
        // definitions and a fresh retry publish the profile-exact sequence.
        ScannedStep::FontDefinition {
            request, global, ..
        } => {
            if !profile.capabilities().supports_etex() {
                return None;
            }
            MutationRecord {
                target: "meaning",
                value: "set_font".into(),
                key: Some(stores.resolve(request.target).to_owned()),
                tokens: None,
                global: *global,
            }
        }
        // -- §1225's `read_to_cs` runs `define(p,call,cur_val)` after
        // `read_toks`. Open/close mutate stream state rather than `eqtb`;
        // read installs §482's collected list as a parameterless macro.
        ScannedStep::InputStream {
            request:
                InputStreamRequest::Read {
                    target,
                    global,
                    tokens,
                    ..
                },
            ..
        } => MutationRecord {
            target: "meaning",
            value: "macro definition".into(),
            key: Some(stores.resolve(*target).to_owned()),
            tokens: Some(observed_read_body(tokens.token_list(), stores)),
            global: *global,
        },
        ScannedStep::InputStream {
            request: InputStreamRequest::Open { .. } | InputStreamRequest::Close { .. },
            ..
        } => return None,
        // -- Steps that perform no assignment at all: mode and list building,
        // box and alignment structure, grouping, diagnostics, recovery, and
        // the pdfTeX extension requests. None of them reaches §1211's
        // `prefixed_command`, so the reference engine has
        // `umber_mutation_command` false throughout and emits nothing.
        ScannedStep::Continue
        | ScannedStep::Relax
        | ScannedStep::AlignPeekRestart { .. }
        | ScannedStep::AlignmentTemplateEntered
        | ScannedStep::MissingMathShift
        | ScannedStep::EndOfInput
        | ScannedStep::End { .. }
        | ScannedStep::IllegalStop { .. }
        | ScannedStep::IllegalMacroParameter { .. }
        | ScannedStep::ExtraEndCsName
        | ScannedStep::EjectResidualPage
        | ScannedStep::HorizontalSkip { .. }
        | ScannedStep::VerticalSkip { .. }
        | ScannedStep::Kern { .. }
        | ScannedStep::Penalty { .. }
        | ScannedStep::CharacterCode { .. }
        | ScannedStep::DeleteLast(..)
        | ScannedStep::ItalicCorrection
        | ScannedStep::IllegalItalicCorrection { .. }
        | ScannedStep::NoBoundary { .. }
        | ScannedStep::NonScript
        | ScannedStep::ControlSpace
        | ScannedStep::FixedHorizontalGlue { .. }
        | ScannedStep::FixedVerticalGlue { .. }
        | ScannedStep::ParagraphIndent { .. }
        | ScannedStep::PdfXImage { .. }
        | ScannedStep::PdfRefXImage { .. }
        | ScannedStep::PdfSetRandomSeed { .. }
        | ScannedStep::PdfResetTimer
        | ScannedStep::PdfInterwordSpace(..)
        | ScannedStep::PdfRunningLink(..)
        | ScannedStep::PdfSpaceFont(..)
        | ScannedStep::PdfGraphics(..)
        | ScannedStep::PdfObject(..)
        | ScannedStep::PdfReferenceObject(..)
        | ScannedStep::PdfForm(..)
        | ScannedStep::PdfDocumentFragment(..)
        | ScannedStep::PdfNavigation(..)
        | ScannedStep::DeferredOpenOut { .. }
        | ScannedStep::DeferredCloseOut { .. }
        | ScannedStep::DeferredWrite { .. }
        | ScannedStep::DeferredSpecial { .. }
        | ScannedStep::SetLanguage { .. }
        | ScannedStep::IllegalSetLanguage { .. }
        | ScannedStep::AfterGroup(..)
        | ScannedStep::AfterAssignment(..)
        | ScannedStep::Rule { .. }
        | ScannedStep::Message { .. }
        | ScannedStep::DisplayDiagnostic(..)
        | ScannedStep::ShowBox { .. }
        | ScannedStep::ShowLists
        | ScannedStep::ShowTokens { .. }
        | ScannedStep::ShowIfs { .. }
        | ScannedStep::ShowGroups { .. }
        | ScannedStep::VSplit(..)
        | ScannedStep::ImmediateExtension(..)
        | ScannedStep::BoxRegister { .. }
        | ScannedStep::Unbox { .. }
        | ScannedStep::SavedVerticalDiscards(..)
        | ScannedStep::LastBox
        | ScannedStep::Leaders { .. }
        | ScannedStep::LeaderRegister { .. }
        | ScannedStep::MissingLeaderPayload
        | ScannedStep::LeadersNotFollowedByGlue
        | ScannedStep::BeginShipout
        | ScannedStep::BeginAlignment { .. }
        | ScannedStep::AlignmentPreambleOpening { .. }
        | ScannedStep::AlignmentPreambleStart { .. }
        | ScannedStep::MisplacedAlignmentDelimiter { .. }
        | ScannedStep::AlignmentCellOpening { .. }
        | ScannedStep::AlignmentCellFinish { .. }
        | ScannedStep::AlignmentFinish { .. }
        | ScannedStep::BeginNoAlign { .. }
        | ScannedStep::AlignmentRecovery { .. }
        | ScannedStep::BeginSimpleGroup
        | ScannedStep::EndSimpleGroup
        | ScannedStep::BeginSemiSimpleGroup
        | ScannedStep::EndSemiSimpleGroup
        | ScannedStep::ExtraRightBrace { .. }
        | ScannedStep::OffSave(..)
        | ScannedStep::OffSaveBottomDrop { .. }
        | ScannedStep::BeginOrdinaryGroup
        | ScannedStep::EndOrdinaryGroup
        | ScannedStep::EndMathGroup(..)
        | ScannedStep::OutputRoutineOpeningBrace
        | ScannedStep::EndOutputRoutine
        | ScannedStep::AlignmentPeekCell { .. }
        | ScannedStep::NoAlignEndGroup { .. }
        | ScannedStep::BeginBox(..)
        | ScannedStep::BeginLeaderBox { .. }
        | ScannedStep::UndefinedControlSequence
        | ScannedStep::BoxShift(..)
        | ScannedStep::IllegalBoxShift { .. }
        | ScannedStep::BeginInsert(..)
        | ScannedStep::IllegalInsertOrAdjust { .. }
        | ScannedStep::IllegalEqNo { .. }
        | ScannedStep::IllegalLastItem { .. }
        | ScannedStep::InvalidArithmeticTarget { .. }
        | ScannedStep::BoxEndGroup { .. }
        | ScannedStep::Mark { .. }
        | ScannedStep::TextDirection { .. }
        | ScannedStep::Paragraph
        | ScannedStep::ParagraphStart
        | ScannedStep::Character { .. } => return None,
        // -- Applied by `CanonicalMainControl::apply_host_owned_step` before
        // this classifier runs, either because the step is applied through its
        // own typed request path or because it ends the replay episode
        // outright.
        ScannedStep::ReplayCompleted(..)
        | ScannedStep::Math(..)
        | ScannedStep::MathDelimiter(..)
        | ScannedStep::MathShift { .. }
        | ScannedStep::DiscretionaryOpening(..)
        | ScannedStep::DiscretionaryPartEnd
        | ScannedStep::DiscretionaryHyphen { .. }
        | ScannedStep::Accent(..) => {
            unreachable!("apply_host_owned_step applies this step before classifying mutations")
        }
    };
    Some(PendingMutation::Captured(captured))
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

/// TeX82 §640's `dvi_out(eop); incr(total_pages)` is the one place a page
/// reaches the `.dvi` file, and §638's `ship_out` is the one routine that
/// reaches it.  The shipout effect therefore belongs to the page commit, not
/// to any command: §1075's `box_end` reaches `ship_out` for an explicit
/// `\shipout`, and §1012's `fire_up` reaches it again through §1025 for every
/// page the page builder ejects with a null `\output`.  Deriving the
/// observation from the committed-artifact delta covers both entry points --
/// and any later one -- by construction, so no command needs to know that it
/// happened to ship a page.
///
/// `total_pages` is incremented before the trace, so the published number is
/// the one-based ordinal of the page just committed.
fn committed_shipout_observations(before: usize, stores: &Universe) -> Vec<EffectRecord> {
    (before..stores.world().artifact_commits().len())
        .map(|committed| EffectRecord {
            kind: "shipout",
            detail: format!("dvi\0{}", committed.saturating_add(1)),
            source: None,
            tokens: None,
        })
        .collect()
}

/// TeX82 §1374 performs open/close effects in `out_what`, whether §1375
/// reached it immediately or a whatsit reached it during later shipout.
/// Observe the committed `tex_state::EffectRecord` delta, not the command
/// spelling, so both entry paths publish the same ordered event exactly once.
fn committed_stream_effect_observations(
    before: usize,
    prepared_before: usize,
    stores: &Universe,
    prepared_pages: &[crate::dispatch::PreparedDviPage],
) -> Vec<EffectRecord> {
    let shipped = &prepared_pages[prepared_before..];
    let direct = stores
        .world()
        .effect_records()
        .get(before..)
        .unwrap_or_default();
    let records: Box<dyn Iterator<Item = &tex_state::EffectRecord> + '_> = if shipped.is_empty() {
        Box::new(direct.iter())
    } else {
        Box::new(
            shipped
                .iter()
                .flat_map(|page| page.committed_effects.iter()),
        )
    };
    records.filter_map(stream_effect_observation).collect()
}

fn stream_effect_observation(record: &tex_state::EffectRecord) -> Option<EffectRecord> {
    match record {
        tex_state::EffectRecord::StreamOpen { slot, target } => Some(EffectRecord {
            kind: "open",
            detail: format!("stream:{}\0{}", slot.raw(), target.path().to_string_lossy()),
            source: None,
            tokens: None,
        }),
        tex_state::EffectRecord::StreamClose { slot } => Some(EffectRecord {
            kind: "close",
            detail: format!("stream:{}\0", slot.raw()),
            source: None,
            tokens: None,
        }),
        _ => None,
    }
}

fn write_effect_detail(sink: PrintSink) -> String {
    let stream = match sink {
        PrintSink::Stream(slot) => i32::from(slot.raw()),
        // TeX82 §§1342/1370 reserve selector 16 for writes above the stream
        // range and 17 for negative writes. `replay_write_sink` lowers those
        // selectors to their terminal/log routing before shipout.
        PrintSink::Terminal | PrintSink::TerminalAndLog => 16,
        PrintSink::Log => 17,
    };
    format!("stream:{stream}\0")
}

fn applied_effect_observation(scanned: &ScannedStep, stores: &Universe) -> Option<EffectRecord> {
    match scanned {
        ScannedStep::Message { tokens, .. } => Some(EffectRecord {
            kind: "message",
            // TeX82 §1279 observes the string produced by
            // `token_show(def_ref)`, not a character-only projection of the
            // expanded list. Control-sequence tokens can deliberately survive
            // expansion through `\noexpand` and must retain `print_cs`'s
            // spelling and separator.
            detail: message_text(stores, tokens.token_list()),
            source: None,
            tokens: None,
        }),
        ScannedStep::ShowTokens { tokens } => Some(EffectRecord {
            kind: "showtokens",
            detail: show_tokens_text(stores, tokens.token_list()),
            source: None,
            tokens: Some(
                stores
                    .tokens(tokens.token_list())
                    .iter()
                    .copied()
                    .map(|token| observed_macro_token(token, stores))
                    .collect(),
            ),
        }),
        ScannedStep::ShowIfs { conditions } => Some(EffectRecord {
            kind: "showifs",
            detail: render_showifs(conditions),
            source: None,
            tokens: None,
        }),
        ScannedStep::ShowGroups {
            diagnostic: Some(diagnostic),
        } => Some(EffectRecord {
            kind: "showgroups",
            detail: crate::diagnostics::render_showgroups(diagnostic),
            source: None,
            tokens: None,
        }),
        ScannedStep::ShowGroups { diagnostic: None } => None,
        ScannedStep::ImmediateExtension(ImmediateExtension::Write { stream, tokens }) => {
            Some(EffectRecord {
                kind: "write",
                detail: format!("stream:{}\0", stream.normalized_number()),
                source: None,
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
        // TeX82 §1335's `final_cleanup` and §1333's
        // `close_files_and_terminate` run once `its_all_over` has returned
        // true, so the job-termination effect belongs to that step and to no
        // other normal command.
        ScannedStep::End { .. } => Some(engine_termination_effect()),
        _ => None,
    }
}

fn engine_termination_effect() -> EffectRecord {
    EffectRecord {
        kind: "terminate",
        detail: "engine\0".into(),
        source: None,
        tokens: None,
    }
}

/// TeX82 §1075 completes `box_end` synchronously: a `\shipout` box is
/// published while its command-owned terminator backup is still live.  The
/// artifact kernel receives only an already-published detached input summary;
/// it never receives a legacy source stack or scans the command operand.
///
/// TeX82 §638's `ship_out` progress marker, opening half: the
/// `\tracingoutput` announcement, a leading separator, `[`, and the
/// nonzero-trimmed `\count0..\count9` values. Under `\tracingoutput>0` this
/// also closes the bracket and dumps the box, because §638 does:
///
/// ```text
/// if tracing_output>0 then
///   begin print_char("]"); begin_diagnostic; show_box(p); end_diagnostic(true);
///   end;
/// <Ship box p out>;
/// if eqtb[int_base+tracing_output_code].int<=0 then print_char("]");
/// ```
///
/// Everything here therefore precedes the page write, and
/// [`print_ship_out_marker_close`] follows it. tex.web's interleave is not
/// cosmetic: a `\write` whatsit inside the box prints *between* the two
/// halves, so `[7` opens the bracket, the write's text follows, and `]`
/// closes it.
fn print_ship_out_marker_open(
    stores: &mut Universe,
    tracing_output: i32,
    counts: &[i32; 10],
    traced_node: Option<&Node>,
) {
    let last = (1..=9usize).rev().find(|&j| counts[j] != 0).unwrap_or(0);
    if tracing_output > 0 {
        let mut printer = stores.printer();
        printer.print_nl("");
        printer.print_ln();
        printer.print("Completed box being shipped out");
    }
    {
        let mut printer = stores.printer();
        let term = printer.terminal_offset();
        let log = printer.log_offset();
        if term > tex_state::print::MAX_PRINT_LINE.saturating_sub(9) {
            printer.print_ln();
        } else if term > 0 || log > 0 {
            printer.print_char(' ');
        }
        printer.print_char('[');
        for (index, &value) in counts.iter().enumerate().take(last + 1) {
            printer.print_int(value);
            if index < last {
                printer.print_char('.');
            }
        }
    }
    if let Some(node) = traced_node {
        stores.printer().print_char(']');
        let frozen = stores.freeze_node_list(std::slice::from_ref(node));
        let text = crate::node_dump::dump_node_list(
            stores,
            frozen,
            crate::node_dump::DumpConfig::read(stores),
        );
        let mut diagnostic = stores.begin_diagnostic();
        diagnostic.print_rendered(&text);
        diagnostic.end(true);
    }
}

/// §638's `if eqtb[int_base+tracing_output_code].int<=0 then print_char("]")`,
/// run after the page has been written.
fn print_ship_out_marker_close(stores: &mut Universe, tracing_output: i32) {
    if tracing_output <= 0 {
        stores.printer().print_char(']');
    }
}

fn shipout_replay_box(
    node: Node,
    stores: &mut Universe,
    command: &mut CommandMachine<'_>,
) -> Result<Option<crate::dispatch::PreparedDviPage>, ExecError> {
    // §638's `[` marker reports the page's `\count0`..`\count9` and, under
    // `\tracingoutput`, dumps the shipped box. Both are read before the page
    // is replayed, because replaying it is what changes them.
    let tracing_output = stores.int_param(IntParam::TRACING_OUTPUT);
    let counts: [i32; 10] =
        std::array::from_fn(|index| stores.count(u16::try_from(index).expect("0..=9 fits u16")));
    let traced_node = (tracing_output > 0).then(|| node.clone());
    let mut expansion = tex_expand::ExpansionContext::new("texput");
    let input_summary = stores.input_summary().clone();
    let output_open_context = command.state.output_open_context(&stores.command_context());
    // §1372's recovery reports from inside the write expansion, after the
    // enclosing `command` borrow has moved into the closure below, so the
    // context it needs is captured here alongside the shipout's own copy.
    let unbalanced_context = output_open_context.clone();
    // Effects live at this point are genuine whatsit output carried forward
    // from before the page; everything after it -- §638's own marker
    // included -- belongs to this shipout and must not be swept into the
    // page's serialized content.
    let pending_end = stores.world().effect_records().len();
    print_ship_out_marker_open(stores, tracing_output, &counts, traced_node.as_ref());
    let effect_start = stores.world().effect_records().len();
    let mut effect_cursor = effect_start;
    let mut write_diagnostics = Vec::new();
    let mut expand_write =
        |stores: &mut Universe, sink: PrintSink, tokens: tex_state::ids::TokenListId| {
            // TeX82 §§1374--1375 execute an open/close whatsit in `out_what`
            // before moving to the next whatsit. A following write expands only
            // after those effects have happened, so publish the committed prefix
            // before its nested command episode contributes observations.
            if let Some(observations) = command.observations.as_mut() {
                observations.0.extend(
                    stores.world().effect_records()[effect_cursor..]
                        .iter()
                        .filter_map(stream_effect_observation)
                        .map(CommandObservation::Effect),
                );
            }
            effect_cursor = stores.world().effect_records().len();
            let traced = stores
                .tokens(tokens)
                .iter()
                .copied()
                .map(|token| TracedTokenWord::pack(token, tex_state::token::OriginId::UNKNOWN))
                .collect::<Vec<_>>();
            let traced = stores.finish_traced_token_list(&traced);
            let expanded = {
                let mut processor = command.processor(stores);
                let expanded = processor.expand_write_text(traced).map_err(command_error)?;
                write_diagnostics.extend(
                    processor
                        .take_semantic_diagnostics()
                        .into_iter()
                        .map(PendingDiagnostic::Command),
                );
                expanded
            };
            if let Some(observations) = command.observations.as_mut() {
                observations
                    .0
                    .push(CommandObservation::Effect(EffectRecord {
                        kind: "write",
                        detail: write_effect_detail(sink),
                        source: None,
                        tokens: Some(
                            stores
                                .tokens(expanded.tokens.token_list())
                                .iter()
                                .copied()
                                .map(|token| observed_macro_token(token, stores))
                                .collect(),
                        ),
                    }));
            }
            if expanded.unbalanced {
                // TeX82 §1372's `<Recover from an unbalanced write command>`.
                crate::error_report::report_error(
                    stores,
                    "Unbalanced write command",
                    &[
                        "On this page there's a \\write with fewer real {'s than }'s.",
                        "I can't handle that very well; good luck.",
                    ],
                    unbalanced_context.clone(),
                )?;
            }
            let mut text = String::new();
            for &token in stores.tokens(expanded.tokens.token_list()) {
                tex_expand::append_token_string_text(stores, token, &mut text);
            }
            let mut text = crate::diagnostics::print_text_with_newlinechar(stores, &text);
            text.push('\n');
            Ok(Some(text))
        };
    let mut receipt = crate::assignments::shipout_node_with_input_summary(
        node,
        input_summary,
        crate::assignments::ShipoutOrigin {
            output_open_context: Some(output_open_context),
            pending_end,
        },
        stores,
        &mut expansion,
        true,
        &mut expand_write,
    )?;
    if let Some(receipt) = receipt.as_mut() {
        receipt.committed_effects = receipt.committed_effects[effect_cursor - effect_start..]
            .to_vec()
            .into_boxed_slice();
    }
    // TeX82 §1370 expands deferred writes during `ship_out`; §§82 and 90
    // render any recoverable scanner errors before shipout returns. The
    // expansion itself runs inside Umber's artifact transaction, so render
    // the command-owned reports only after that transaction has committed:
    // otherwise its transcript effects are consumed as staging scratch.
    // `write_diagnostics` contains scanner recoveries, never §1030 command
    // traces; writes execute outside main control's `shown_mode` owner.
    report_pending_diagnostics(stores, write_diagnostics, &mut None)?;
    print_ship_out_marker_close(stores, tracing_output);
    // The closing bracket prints after `shipout_node_with_input_summary`'s
    // own transaction has committed, so without this call it would sit as a
    // live, uncommitted effect suffix that a later `\shipout` would find at
    // the exact point `direct::stage_shipout` reads its carried-forward
    // effects. `pending_end` above already excludes this page's own marker
    // from that read; committing here keeps the *next* page's `pending_end`
    // from including this one's trailing `]` (`umber2-alfh.10`, confirmed
    // against
    // `canonical_effect_free_shipout_memo_republishes_one_aligned_receipt`'s
    // two identical `\shipout\copy0` calls). It is a no-op under retained
    // sessions, which consume their effect suffix on export instead.
    //
    // This is not free: a checkpoint/retry session (`crates/umber`'s
    // `CanonicalEngineSession`) can still roll this whole step back if a
    // *later* command turns out to need a resource this speculative run did
    // not have, and `commit_effects` materializes into
    // `World::memory_terminal_output`/`memory_log_output`, which that
    // rollback does not undo -- confirmed to duplicate the marker under
    // `retained_session_retries_input_without_duplicate_effect_or_receipt`.
    // `umber2-v4dx` tracks giving rollback that reach.
    stores.commit_effects(stores.world().effect_pos())?;
    // TeX82's `ship_out` clears the consecutive-dead-output counter (§638).
    // Canonical lowering bypasses the legacy executor's bookkeeping, so keep
    // the page-state transition at the typed shipout boundary.
    stores.set_page_integer(tex_state::page::PageInteger::DeadCycles, 0);
    Ok(receipt)
}

#[cfg(test)]
pub(crate) fn test_shipout_replay_box(
    node: Node,
    stores: &mut Universe,
) -> Result<Option<crate::dispatch::PreparedDviPage>, ExecError> {
    let mut fuel = tex_command::CommandFuelLedger::default();
    let mut command = CommandMachine {
        state: &mut CommandState::default(),
        runtime: &mut CommandRuntime::default(),
        fuel: fuel.fuel_mut(),
        capabilities: &mut CommandHostCapabilities::default(),
        observations: &mut None,
        initex: true,
    };
    shipout_replay_box(node, stores, &mut command)
}

/// Renders a committed meaning the way the reference instrumentation's
/// `umber_trace_meaning_value` does.
///
/// tex.web stores a meaning as an `(eq_type, equiv)` pair and names it by its
/// command code, so the canonical rendering is a three-way split on the
/// command, never on how the meaning was reached:
///
/// - a macro (`eq_type >= call`) is its whole §294 body -- parameter text,
///   the `end_match` that separates the two halves, then replacement text;
/// - §208's `char_given` and `math_given` carry the shorthand code stored by
///   §1224 as a typed scalar;
/// - everything else is §207/§208's command name for the `eq_type`.
///
/// It must never fall back to a spelling (the source control sequence of a
/// `\let`) or to a Rust `Debug` rendering: both name where the meaning came
/// from rather than what it is (`umber2-johp.141`).
fn meaning_mutation_value(
    meaning: Meaning,
    stores: &Universe,
) -> (String, Option<Vec<ObservedToken>>) {
    match meaning {
        Meaning::Macro { definition, flags } => {
            let macro_meaning = stores.macro_definition(definition);
            (
                "macro definition".into(),
                Some(observed_stored_macro_body(
                    flags,
                    macro_meaning.parameter_text(),
                    macro_meaning.replacement_text(),
                    stores,
                )),
            )
        }
        Meaning::CharGiven(character) => (format!("character:{}", u32::from(character)), None),
        Meaning::MathCharGiven(code) => (format!("integer:{code}"), None),
        meaning => (
            tex_command::canonical_names::meaning_command_name(meaning),
            None,
        ),
    }
}

/// e-TeX change section [49] inserts `protected_token` at the front of a
/// protected macro's stored body immediately before `define`. The reference
/// semantic seam reports the unmarked body at that insertion boundary; the
/// following meaning mutation reports the actual marked stored body.
fn protected_macro_definition_observation(
    scanned: &ScannedStep,
    stores: &Universe,
) -> Option<TokenListRecord> {
    let ScannedStep::MacroDefinition {
        flags,
        parameter_text,
        replacement_text,
        ..
    } = scanned
    else {
        return None;
    };
    flags
        .contains(MeaningFlags::PROTECTED)
        .then(|| TokenListRecord {
            transition: "complete",
            purpose: "protected_macro",
            tokens: observed_macro_body(
                parameter_text.token_list(),
                replacement_text.token_list(),
                stores,
            ),
        })
}

/// The macro body as stored by TeX82 §294 and e-TeX change section [49].
fn observed_stored_macro_body(
    flags: MeaningFlags,
    parameter_text: TokenListId,
    replacement_text: TokenListId,
    stores: &Universe,
) -> Vec<ObservedToken> {
    let mut tokens = observed_macro_body(parameter_text, replacement_text, stores);
    if flags.contains(MeaningFlags::PROTECTED) {
        // e-TeX's `protected_token` is `other_token + "1"` where
        // `other_token` is command/category 14 (`comment`) times 256.
        tokens.insert(
            0,
            ObservedToken::Character {
                character: '\u{1}',
                catcode: tex_state::token::Catcode::Comment,
            },
        );
    }
    tokens
}

/// §294's stored macro body: parameter text, the separating `end_match`, then
/// replacement text, as one token sequence.
fn observed_macro_body(
    parameter_text: TokenListId,
    replacement_text: TokenListId,
    stores: &Universe,
) -> Vec<ObservedToken> {
    let mut tokens = stores
        .tokens(parameter_text)
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
            .tokens(replacement_text)
            .iter()
            .copied()
            .map(|token| observed_macro_token(token, stores)),
    );
    tokens
}

/// §482 constructs a parameterless macro body for §1225's `define`.
fn observed_read_body(replacement_text: TokenListId, stores: &Universe) -> Vec<ObservedToken> {
    let mut tokens = vec![ObservedToken::MacroEndMatch];
    tokens.extend(
        stores
            .tokens(replacement_text)
            .iter()
            .copied()
            .map(|token| observed_macro_token(token, stores)),
    );
    tokens
}

fn observed_macro_token(token: Token, stores: &Universe) -> ObservedToken {
    match token {
        // §353 gives an active character the control sequence
        // `active_base + c`, so §365's `cur_tok` stores it as
        // `cs_token_flag + cur_cs` and its §289 spelling is the single
        // character, never a character token with command code 13.
        Token::Char {
            ch,
            cat: tex_state::token::Catcode::Active,
        } => ObservedToken::ControlSequence(ch.to_string()),
        Token::Char { ch, cat } => ObservedToken::Character {
            character: ch,
            catcode: cat,
        },
        Token::Cs(symbol) => ObservedToken::ControlSequence(stores.resolve(symbol).to_owned()),
        Token::Param(slot) => ObservedToken::Parameter(slot),
        Token::Frozen(_) if token.is_frozen_end_template() => ObservedToken::FrozenEndTemplate,
        Token::Frozen(_) if token.is_frozen_endv() => ObservedToken::FrozenEndV,
        // A frozen primitive is one of tex.web's frozen control sequences, so
        // it is observed by the spelling tex.web assigns its `text`, never by
        // an engine-local slot index a transport would have to render.
        Token::Frozen(_) => stores
            .frozen_primitive_meaning(token)
            .and_then(|meaning| stores.primitive_name(meaning))
            .map(str::to_owned)
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
    command: &mut CommandMachine<'_>,
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
    // selection to perform after the otherwise complete command transition,
    // and no §774 entry save level to replace either.
    if active.columns.is_empty() {
        return Ok(());
    }
    if delimiter == AlignmentCellDelimiter::Span {
        active.cell_span = active
            .cell_span
            .checked_add(1)
            .ok_or(ExecError::ArithmeticOverflow)?;
    } else {
        capture_replay_alignment_cell(active, modes, stores, command.fuel)?;
    }
    let next_column = match delimiter {
        AlignmentCellDelimiter::Tab | AlignmentCellDelimiter::Span => active
            .column
            .checked_add(1)
            .ok_or(ExecError::ArithmeticOverflow)?,
        AlignmentCellDelimiter::Row => 0,
    };
    let extra_tab_recovery = next_column >= active.columns.len()
        && active.repeat_start.is_none()
        && matches!(
            delimiter,
            AlignmentCellDelimiter::Tab | AlignmentCellDelimiter::Span
        );
    // TeX82 §791's `if extra_info(cur_align)<>span_code then begin unsave;
    // new_save_level(align_group)`: every entry that does not continue through
    // `\span` replaces the §774 entry save level, discarding the cell's local
    // assignments. §792's extra-tab recovery rewrites `extra_info` to
    // `cr_code` *before* that test, so a `\span` whose column does not exist
    // ends the entry -- and its save level -- after all.
    //
    // §791 unsaves before `@<Package an unset box for the current column@>`;
    // here the packaging (`capture_replay_alignment_cell`) runs first because
    // it also flushes the cell's pending characters, which TeX had already
    // appended with the in-cell `cur_font`. `hpack`/`vpackage` at natural size
    // read no restorable parameter, so the two orders agree.
    if delimiter != AlignmentCellDelimiter::Span || extra_tab_recovery {
        replace_alignment_entry_save_level(command, stores)?;
    }
    if extra_tab_recovery {
        let recovered = command
            .state
            .apply_alignment_request(AlignmentRequest::RecoverExtraTab(alignment))
            .map_err(|_| ExecError::MissingToken {
                context: "alignment extra-tab recovery",
            })?;
        debug_assert!(matches!(
            recovered,
            AlignmentRequestResult::ExtraTabRecovered
        ));
        finish_replay_alignment_row(active, modes, stores, command.fuel)?;
        active.column = 0;
        // §792's extra-tab recovery rewrites `extra_info` to `cr_code`, so
        // §791's `fin_col` returns true and §1131's `do_endv` runs `fin_row`
        // here too -- including its `\everycr` push.
        schedule_everycr(command.state, stores);
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
            finish_replay_alignment_row(active, modes, stores, command.fuel)?;
            // TeX82 §799 `fin_row` closes with
            // `if every_cr<>null then begin_token_list(every_cr,every_cr_text);
            // align_peek`, so the hook is installed before the lookahead that
            // starts the next row reads a token.
            schedule_everycr(command.state, stores);
            active.align_peek_pending = true;
        }
        AlignmentCellDelimiter::Tab | AlignmentCellDelimiter::Span => {
            command
                .state
                .apply_alignment_request(AlignmentRequest::BeginCell {
                    alignment,
                    templates,
                })
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment next-cell lifecycle",
                })?;
            if delimiter == AlignmentCellDelimiter::Tab {
                begin_replay_alignment_cell(active, modes, stores)?;
            }
            active.next_cell_opening_pending = true;
        }
    }
    Ok(())
}

/// Applies TeX82 §791 `fin_col`'s `unsave; new_save_level(align_group)`.
///
/// The pair is what makes an alignment entry a scope: assignments a cell makes
/// -- a font selection such as plain.tex's `\bf`, a `\fam`, any local register
/// -- must not survive the `&` or `\cr` that ends it. §1063's `unsave` also
/// releases the level's `\aftergroup` tokens, so they are backed up here just
/// as every other canonical group exit does.
fn replace_alignment_entry_save_level(
    command: &mut CommandMachine<'_>,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    let aftergroup = leave_alignment_save_level(stores, "alignment entry group")?;
    enter_canonical_group(stores, command.state, GroupKind::Align);
    schedule_aftergroup(command, stores, aftergroup)
}

/// One of TeX82 §800 `fin_align`'s `unsave`s, or §791's.
fn leave_alignment_save_level(
    stores: &mut Universe,
    context: &'static str,
) -> Result<Vec<Token>, ExecError> {
    stores
        .leave_group_with_kind(GroupKind::Align)
        .map_err(|_| ExecError::MissingToken { context })
}

/// TeX82 §800's internal save-stack checks at the start of `fin_align`.
///
/// Unlike an ordinary group-closing command, reaching either check without
/// the expected `align_group` means the engine's own alignment state is
/// inconsistent. TeX therefore calls `confusion`, with a distinct site for
/// the entry level and the whole-alignment level.
fn leave_fin_align_save_level(
    stores: &mut Universe,
    confusion_site: &'static str,
) -> Result<Vec<Token>, ExecError> {
    stores
        .leave_group_with_kind(GroupKind::Align)
        .map_err(|_| ExecError::Fatal(FatalError::confusion(confusion_site)))
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
        // TeX82 §768: `init_row` changes an \halign from internal vertical
        // to restricted horizontal mode, and §769's `init_span` preserves
        // that mode on the cell's fresh semantic level.
        AlignmentKind::HAlign => Mode::RestrictedHorizontal,
        AlignmentKind::VAlign => Mode::InternalVertical,
    }
}

fn begin_replay_alignment_cell(
    active: &mut ActiveReplayAlignment,
    modes: &mut ModeNest,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    if !active.row_open {
        modes.push(replay_alignment_row_mode(active.kind))?;
        modes.current_list_mutation().push(Node::Glue {
            spec: active
                .tabskips
                .first()
                .copied()
                .unwrap_or(active.default_tabskip),
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
    modes.push(replay_alignment_cell_mode(active.kind))?;
    crate::align::init_span_aux(modes, stores);
    active.cell_span = 1;
    active.cell_open = true;
    Ok(())
}

fn capture_replay_alignment_cell(
    active: &mut ActiveReplayAlignment,
    modes: &mut ModeNest,
    stores: &mut Universe,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    if !active.cell_open {
        return Ok(());
    }

    // TeX82 §1131's `do_endv` runs `end_graf` before §791's `fin_col`.
    // This is a no-op for an \halign cell's restricted horizontal level,
    // but a \valign cell is internal vertical and may have a paragraph open
    // above it. Close that paragraph before popping and packaging the cell;
    // otherwise the paragraph is mistaken for the cell, leaving the actual
    // cell and row levels on the mode nest after `fin_align`.
    if active.kind == AlignmentKind::VAlign {
        crate::assignments::end_paragraph_with_fuel(modes, stores, fuel)?;
    }

    // Canonical alignment packaging still defers that paragraph's lowering,
    // but §815's negative pretolerance makes its immediate transition into
    // the hyphenating pass certain. Publish §919's one-way trie lifecycle at
    // this canonical boundary, before `align_peek` fetches what follows.
    if matches!(
        modes.current_mode(),
        Mode::Horizontal | Mode::RestrictedHorizontal
    ) && stores.int_param(IntParam::PRETOLERANCE) < 0
    {
        stores.close_hyphenation_patterns();
    }
    let mut cell = crate::assignments::commit_current_list(modes, stores, fuel)?;
    let material = if active.kind == AlignmentKind::HAlign {
        // TeX82 §796 packs an `\halign` column with `adjust_tail:=cur_tail`,
        // so §651/§655 remove its insertions, marks, and `\vadjust` contents
        // from the column and hold them on the row's migration list; §799
        // appends them after the packaged row. A `\valign` column is
        // `vpackage`d with `adjust_tail` null and migrates nothing.
        let material =
            crate::math::finish_math_lists_owned(stores, cell.list_mutation().take_nodes(), false);
        let (retained, mut pre_migrated, migrated) =
            crate::assignments::split_hpack_migrations(stores, material);
        pre_migrated.extend(migrated);
        active.row_migrations.extend(pre_migrated);
        retained
    } else {
        cell.list_mutation().take_nodes()
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
        crate::align::packaging::UnsetPackContext::Cell,
    )?;
    modes.current_list_mutation().push(cell);
    modes.current_list_mutation().push(Node::Glue {
        spec: active
            .tabskips
            .get(active.column.saturating_add(1))
            .copied()
            .unwrap_or(active.default_tabskip),
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
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    capture_replay_alignment_cell(active, modes, stores, fuel)?;
    if !active.row_open {
        return Ok(());
    }

    let mut row = crate::assignments::commit_current_list(modes, stores, fuel)?;
    let children = stores.freeze_node_list(&row.list_mutation().take_nodes());
    let row = crate::align::packaging::make_unset_node(
        stores,
        children,
        crate::align::packaging::row_unset_kind(active.kind),
        1,
        crate::align::packaging::UnsetPackContext::Row,
    )?;
    // TeX82 §799's `fin_row`: `p:=hpack(link(head),natural,...); pop_nest;
    // append_to_vlist(p)`. The completed (still unset) row joins the
    // alignment's own vertical list through §679 `append_to_vlist`, so the
    // interline glue between two rows is the ordinary `\baselineskip`/
    // `\lineskip` decision against the running `prev_depth` -- not a bare
    // splice. §807's unset-to-set conversion changes only widths and glue
    // set, never a row's height or depth, so computing the glue here is
    // exactly what tex.web computes. A bare push produced rows stacked with
    // no interline glue at all, which is why plain's `\pmatrix`/`\matrix`/
    // `\cases`/`\eqalign`/`\halign` bodies came out short by one
    // `\baselineskip` per row (`umber2-johp.260`).
    match active.kind {
        AlignmentKind::HAlign => {
            crate::vertical::append_node_to_vertical_list(modes, stores, row)?;
        }
        AlignmentKind::VAlign => {
            // TeX82 §799's other branch is a plain horizontal splice:
            // `link(tail):=p; tail:=p; space_factor:=1000`. A valign row
            // must not pass through §679's vertical baseline calculation;
            // doing so inserts baselineskip between rows, and a surrounding
            // hpack then counts that vertical glue as horizontal cell width.
            modes.current_list_mutation().push(row);
            modes.current_list_mutation().set_space_factor(1000);
        }
    }
    // §799 continues `if cur_head<>cur_tail then begin link(tail):=link(cur_head);
    // tail:=cur_tail end`: the migrated material is spliced immediately after the
    // row, as a plain list splice with no interline glue of its own.
    for node in std::mem::take(&mut active.row_migrations) {
        crate::vertical::append_vertical_contribution(modes, stores, node);
    }
    active.row_open = false;
    Ok(())
}

/// Carries TeX82 §645's `spec_code`/`cur_val` pair from the command-owned
/// `scan_spec` to the alignment state §805 packs the prototype box with.
fn alignment_pack_spec(packing: ScannedPackingSpec) -> AlignmentPackSpec {
    match packing {
        ScannedPackingSpec::Natural => AlignmentPackSpec::Natural,
        ScannedPackingSpec::Exactly(size) => AlignmentPackSpec::Exactly(size),
        ScannedPackingSpec::Spread(size) => AlignmentPackSpec::Spread(size),
    }
}

fn finish_replay_alignment(
    active: &mut ActiveReplayAlignment,
    modes: &mut ModeNest,
    stores: &mut Universe,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    finish_replay_alignment_row(active, modes, stores, fuel)?;
    let mut alignment = crate::assignments::commit_current_list(modes, stores, fuel)?;
    let rows = alignment.list_mutation().take_nodes();
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
        active.packing,
        columns,
        active.tabskips.clone(),
        active.default_tabskip,
        active.repeat_start,
    );
    // TeX82 §800: `if nest[nest_ptr-1].mode_field=mmode then o:=display_indent
    // else o:=0`. The alignment level has just been popped, so the current mode
    // is the enclosing one §800 inspects.
    let offset = if modes.current_mode() == Mode::DisplayMath {
        stores.dimen_param(DimenParam::DISPLAY_INDENT)
    } else {
        Scaled::from_raw(0)
    };
    let finished = crate::align::widths::finish_alignment(&state, &rows, offset, stores)?;
    let aux_prev_depth = alignment.list().prev_depth();
    if modes.current_mode() == Mode::DisplayMath {
        // Preserve §812's `(p,q,aux_save)` handoff until the closing `$$`
        // has run §§1206–1207's assignment and delimiter scan.
        modes
            .current_list_mutation()
            .set_display_alignment(finished, aux_prev_depth);
    } else {
        crate::align::append_finished_alignment(
            modes,
            stores,
            crate::align::FinishedAlignment {
                nodes: finished,
                aux_prev_depth,
            },
        );
    }
    crate::vertical::build_page_if_outer_vertical(modes, stores)?;
    Ok(())
}

/// Resolves e-TeX 2.6 [49.1292]'s coupled save-stack/mode-nest traversal into
/// an immutable diagnostic value. Unrestricted horizontal levels do not own
/// groups and are skipped by the WEB traversal; the remaining contexts are
/// paired outermost-first without changing either live stack.
fn detached_showgroups(
    stores: &Universe,
    modes: &ModeNest,
    active_alignment: &Option<ActiveReplayAlignment>,
    boxes: &ReplayBoxes,
) -> crate::diagnostics::ShowGroupsDiagnostic {
    use crate::diagnostics::{ShowGroupFrame, ShowGroupsDiagnostic};

    let frames = stores.group_frames().collect::<Vec<_>>();
    let summary = modes.summary();
    let semantic_modes = summary
        .levels()
        .iter()
        .filter_map(|level| (level.mode() != Mode::Horizontal).then_some(level.mode()))
        .collect::<Vec<_>>();
    let mut mode_index = 1usize;
    let mut align_level = 0usize;
    let mut box_index = 0usize;
    let align_kind = active_alignment.as_ref().map(|active| active.kind);
    let mut rendered = Vec::with_capacity(frames.len());
    for (index, frame) in frames.into_iter().enumerate() {
        let kind = frame.kind();
        let mode = semantic_modes.get(mode_index).copied();
        let context = match kind {
            GroupKind::Simple | GroupKind::Math => "{".to_owned(),
            GroupKind::SemiSimple => "\\begingroup".to_owned(),
            GroupKind::HBox
            | GroupKind::AdjustedHBox
            | GroupKind::VBox
            | GroupKind::VTop
            | GroupKind::VCenter
            | GroupKind::Insert => {
                let context = boxes
                    .active_boxes
                    .get(box_index)
                    .map_or_else(|| fallback_group_context(kind).to_owned(), show_box_context);
                box_index = box_index.saturating_add(1);
                context
            }
            GroupKind::Output => "\\output{".to_owned(),
            GroupKind::Disc => "\\discretionary{".to_owned(),
            GroupKind::MathChoice => "\\mathchoice{".to_owned(),
            GroupKind::MathShift => match mode {
                Some(Mode::DisplayMath) => "$$".to_owned(),
                _ => "$".to_owned(),
            },
            GroupKind::MathLeft => "\\left".to_owned(),
            GroupKind::NoAlign => "\\noalign{".to_owned(),
            GroupKind::Align => {
                let context = if align_level == 0 {
                    match align_kind {
                        Some(AlignmentKind::VAlign) => "\\valign{",
                        _ => "\\halign{",
                    }
                } else {
                    "align entry"
                };
                align_level = align_level.saturating_add(1);
                context.to_owned()
            }
        };
        if matches!(
            kind,
            GroupKind::HBox
                | GroupKind::AdjustedHBox
                | GroupKind::VBox
                | GroupKind::VTop
                | GroupKind::Output
                | GroupKind::Math
                | GroupKind::Disc
                | GroupKind::Insert
                | GroupKind::VCenter
                | GroupKind::MathChoice
                | GroupKind::MathShift
                | GroupKind::MathLeft
        ) {
            mode_index = mode_index.saturating_add(1);
        }
        rendered.push(ShowGroupFrame {
            kind,
            level: index + 1,
            entered_line: frame.entered_line(),
            context,
        });
    }
    ShowGroupsDiagnostic { frames: rendered }
}

fn fallback_group_context(kind: GroupKind) -> &'static str {
    match kind {
        GroupKind::HBox | GroupKind::AdjustedHBox => "\\hbox{",
        GroupKind::VBox => "\\vbox{",
        GroupKind::VTop => "\\vtop{",
        GroupKind::VCenter => "\\vcenter{",
        GroupKind::Insert => "\\insert{",
        _ => "{",
    }
}

fn show_box_context(active: &ActiveReplayBox) -> String {
    let mut context = String::new();
    if let Some(target) = active.target {
        if target.global {
            context.push_str("\\global");
        }
        context.push_str("\\setbox");
        context.push_str(&target.index.to_string());
        context.push('=');
    } else if active.ships_out {
        context.push_str("\\shipout");
    } else if let Some(kind) = active.leader_kind {
        context.push_str(match kind {
            GlueKind::Leaders => "\\leaders",
            GlueKind::Cleaders => "\\cleaders",
            GlueKind::Xleaders => "\\xleaders",
            _ => "\\leaders",
        });
    }
    match active.kind {
        ReplayBoxKind::HBox => context.push_str("\\hbox"),
        ReplayBoxKind::VBox => context.push_str("\\vbox"),
        ReplayBoxKind::VTop => context.push_str("\\vtop"),
        ReplayBoxKind::VCenter => context.push_str("\\vcenter"),
        ReplayBoxKind::Insert(255, pre) => {
            context.push_str(if pre { "\\vadjust pre" } else { "\\vadjust" });
        }
        ReplayBoxKind::Insert(class, _) => {
            context.push_str("\\insert");
            context.push_str(&class.to_string());
        }
    }
    match active.packing {
        PackSpec::Natural => {}
        PackSpec::Exactly(size) => {
            context.push_str(" to");
            context.push_str(&crate::node_dump::format_scaled_for_diagnostics(size));
            context.push_str("pt");
        }
        PackSpec::Spread(size) => {
            context.push_str(" spread");
            context.push_str(&crate::node_dump::format_scaled_for_diagnostics(size));
            context.push_str("pt");
        }
    }
    context.push('{');
    context
}

fn enter_canonical_group(stores: &mut Universe, command: &CommandState, kind: GroupKind) {
    stores.enter_group_with_kind_at_line(kind, command.current_file_line_number());
}

#[allow(clippy::too_many_arguments)] // applies the complete canonical replay state atomically
fn apply_scanned_step(
    scanned: ScannedStep,
    stores: &mut Universe,
    modes: &mut ModeNest,
    next_alignment_identity: &mut u64,
    active_alignment: &mut Option<ActiveReplayAlignment>,
    command: &mut CommandMachine<'_>,
    boxes: &mut ReplayBoxes,
    prepared_dvi_pages: &mut PreparedDviPages,
) -> Result<ReplayStep, ExecError> {
    match scanned {
        ScannedStep::Continue | ScannedStep::AlignmentTemplateEntered => Ok(ReplayStep::Continue),
        ScannedStep::Relax => {
            // TeX82 §1030 reaches §1045's do-nothing arm only after leaving
            // the ligature loop. The command itself has no list effect, but
            // it is still a word boundary: `?\\relax\\char96` must not form
            // the `?`` ligature across the relax.
            crate::assignments::flush_pending_hchars_with_fuel(modes, stores, command.fuel)?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::TextDirection { direction, enabled } => {
            if enabled {
                crate::assignments::flush_pending_hchars_with_fuel(modes, stores, command.fuel)?;
                modes
                    .current_list_mutation()
                    .push(Node::Direction(direction));
            } else {
                let name = match direction {
                    tex_state::node::Direction::BeginL => "beginL",
                    tex_state::node::Direction::EndL => "endL",
                    tex_state::node::Direction::BeginR => "beginR",
                    tex_state::node::Direction::EndR => "endR",
                };
                // etex.ch's `eTeX_enabled`: one report for every optional
                // feature, so the help names the disabled feature generally
                // rather than the primitive the message already named.
                let context = command.state.output_open_context(&stores.command_context());
                report_escaped_error(
                    stores,
                    "Improper ",
                    name,
                    "",
                    &["Sorry, this optional e-TeX feature has been disabled."],
                    context,
                )?;
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::AlignPeekRestart { alignment } => {
            let active = active_alignment
                .as_mut()
                .filter(|active| active.identity == alignment)
                .ok_or(ExecError::MissingToken {
                    context: "alignment restart lookahead",
                })?;
            active.align_peek_pending = true;
            active.align_peek_after_noalign = true;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::MissingMathShift => {
            // TeX82 §1047's `insert_dollar_sign` diagnostic; the matching
            // input recovery (backing up the offending command behind an
            // inserted `$`) already ran in `recover_missing_math_shift`.
            let context = command.state.output_open_context(&stores.command_context());
            crate::error_report::report_error(
                stores,
                "Missing $ inserted",
                &[
                    "I've inserted a begin-math/end-math symbol since I think",
                    "you left one out. Proceed, with fingers crossed.",
                ],
                context,
            )?;
            Ok(ReplayStep::Continue)
        }
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
            stores.set_math_family_font(
                MathFontSize::from(family.size),
                family.family,
                font,
                global,
            );
            Ok(ReplayStep::Continue)
        }
        ScannedStep::EndOfInput => Ok(ReplayStep::EndOfInput),
        ScannedStep::End {
            dump,
            incomplete_conditions: _,
        } => {
            // §1335's final_cleanup tail -- closing every still-open paren,
            // reporting `incomplete_conditions`, the "(see the transcript
            // file..." note, and this same `dump` flag's
            // `(\dump is performed only by INITEX)` note, in that exact
            // order -- runs in `CanonicalMainControl::end_of_job_final_cleanup`
            // once this returns: the paren close needs `self`'s job-framing
            // state, which this free function does not have, and tex.web
            // orders it first. `incomplete_conditions` is discarded here
            // rather than used, because the caller re-derives it from the
            // `ScannedStep::End` it matched before moving `scanned` into this
            // call (see `step_once` and its siblings).
            //
            // TeX82 §1335's INITEX tail releases `last_glue` before
            // `store_fmt_file`; e-TeX 2.6's [45.999] change may meanwhile
            // have retained top-of-page glue, kerns, and penalties in
            // `page_discards`, which are deliberately absent from the
            // format. `its_all_over` (§1054) already proved that the page and
            // contribution lists contain no live material. Normalize the
            // remaining page-builder scalars while preserving both e-TeX
            // discard lists, so the host-side format encoder still rejects
            // genuine page material instead of mistaking `last_penalty` or
            // `last_node_type` for it.
            if dump && command.initex && crate::output::job_is_all_over(stores) {
                stores.start_new_page();
            }
            // TeX82 §1378 closes every still-open numbered output file after
            // `final_cleanup`. The two normalized fallback selectors are not
            // file slots (§1342), so the state boundary exposes only 0..15
            // here. `close_out` preserves §1378's `if write_open[k]` guard:
            // never-opened and already-closed slots produce no close effect.
            // This runs here, synchronously with `\end`/`\dump` applying,
            // rather than in the driver-facing §1333 `finish_job` that prints
            // §642's DVI report: it is a `World` state effect, not a print,
            // and it has already happened by the time a driver can call
            // `finish_job` (which only runs after this step has already
            // returned), so its position here can't reorder anything
            // `finish_job` prints. Leaving it here also keeps it exactly
            // where `effects::tests::
            // output_stream_final_cleanup_closes_only_live_numbered_files`
            // already observes it, synchronous with the terminating step.
            for raw in 0..tex_state::world::STREAM_SLOT_COUNT as u8 {
                stores.world_mut().close_out(StreamSlot::new(raw));
            }
            Ok(ReplayStep::End)
        }
        ScannedStep::Count {
            index,
            value,
            global,
        } => {
            let old = stores.count(index);
            if global {
                stores.set_count_global(index, value);
            } else if !etex_redundant_local_word_assignment(stores, old, value) {
                stores.set_count(index, value);
            }
            crate::assignments::tracing::trace_int_register(stores, index, global, old, value);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Dimen {
            index,
            value,
            global,
        } => {
            let old = stores.dimen(index);
            if global {
                stores.set_dimen_global(index, value);
            } else if !etex_redundant_local_word_assignment(stores, old, value) {
                stores.set_dimen(index, value);
            }
            crate::assignments::tracing::trace_dimen_register(stores, index, global, old, value);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BoxDimensionAssignment {
            index,
            dimension,
            value,
            global,
        } => {
            // `Universe::set_box_dimension{,_global}` share one body: TeX82
            // §1055's `alter_box_dimen` mutates the visible box node
            // directly rather than through the save stack, so the assignment
            // prefix does not change which binding level is affected.
            if global {
                stores.set_box_dimension_global(index, dimension, value);
            } else {
                stores.set_box_dimension(index, dimension, value);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Skip {
            index,
            value,
            global,
        } => {
            let old = stores.skip(index);
            let value = stores.intern_glue(value);
            if global {
                stores.set_skip_global(index, value);
            } else {
                stores.set_skip(index, value);
            }
            crate::assignments::tracing::trace_glue_register(stores, index, global, old, value);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Muskip {
            index,
            value,
            global,
        } => {
            let old = stores.muskip(index);
            let value = stores.intern_glue(value);
            if global {
                stores.set_muskip_global(index, value);
            } else {
                stores.set_muskip(index, value);
            }
            crate::assignments::tracing::trace_muglue_register(stores, index, global, old, value);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::HorizontalSkip { value } => {
            if matches!(
                modes.current_mode(),
                Mode::Vertical | Mode::InternalVertical
            ) {
                start_canonical_paragraph(command.state, modes, stores, true)?;
            }
            crate::assignments::flush_pending_hchars_with_fuel(modes, stores, command.fuel)?;
            modes.current_list_mutation().push(Node::Glue {
                spec: stores.intern_glue(value),
                kind: GlueKind::Normal,
                leader: None,
            });
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Kern { amount } => {
            // TeX82 §1057's `any_mode(kern),mmode+mkern: append_kern`
            // (§1061: `tail_append(new_kern(cur_val)); subtype(tail):=s`).
            // Unlike `\hskip` (§1090's `head_for_vmode`, which is genuinely
            // `vmode+hskip`-listed), `\kern` has no mode-specific dispatch
            // entry at all -- it is legal in every mode and appends directly,
            // with no paragraph start and no page-builder call. The outer
            // vertical list is represented by the page contribution queue,
            // so it still uses the shared contribution splice (contrast
            // `\penalty`, §1103, which also calls `build_page` there).
            crate::assignments::flush_pending_hchars_with_fuel(modes, stores, command.fuel)?;
            crate::vertical::append_vertical_contribution(
                modes,
                stores,
                Node::Kern {
                    amount,
                    kind: KernKind::Explicit,
                },
            );
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Penalty { amount } => {
            // TeX82 §1103's `append_penalty`: `tail_append(new_penalty(cur_val))`
            // in whichever list is current, then `if mode=vmode then
            // build_page` -- i.e. only in *outer* vertical mode, not internal
            // vertical mode, matching `append_vertical_contribution`'s own
            // `is_outer_vertical` gate and (unlike `\vskip`'s `append_glue`,
            // §1057) always followed by a page-builder call in that case.
            crate::assignments::flush_pending_hchars_with_fuel(modes, stores, command.fuel)?;
            crate::vertical::append_vertical_contribution(modes, stores, Node::Penalty(amount));
            crate::vertical::build_page_if_outer_vertical(modes, stores)?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::DeleteLast(primitive) => {
            crate::assignments::execute_delete_last(primitive, modes, stores, command.fuel)?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::SetInteractionMode(primitive) => {
            let mode = match primitive {
                UnexpandablePrimitive::BatchMode => tex_state::InteractionMode::Batch,
                UnexpandablePrimitive::NonstopMode => tex_state::InteractionMode::Nonstop,
                UnexpandablePrimitive::ScrollMode => tex_state::InteractionMode::Scroll,
                UnexpandablePrimitive::ErrorStopMode => tex_state::InteractionMode::ErrorStop,
                _ => unreachable!("only the four interaction-mode primitives are scanned"),
            };
            // TeX82 §1264's `new_interaction`: `print_ln` under the *old*
            // interaction mode's selector, unconditionally, before
            // `interaction:=cur_chr` takes effect. Skipping it left whichever
            // channel's column tracking stale until something else happened
            // to force a newline later -- invisible while every diagnostic
            // wrote both channels in lockstep, but a real divergence once
            // `\tracingonline<=0` redirects one channel alone (`umber2-
            // alfh.9`): the terminal's stale column then forces an extra,
            // unwanted newline into the log too, the first time anything
            // prints through the restored `term_and_log` selector.
            stores.printer().print_ln();
            stores.set_interaction_mode(mode);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::SetInteractionModeValue(value) => {
            let mode = match value {
                0 => tex_state::InteractionMode::Batch,
                1 => tex_state::InteractionMode::Nonstop,
                2 => tex_state::InteractionMode::Scroll,
                3 => tex_state::InteractionMode::ErrorStop,
                value => {
                    crate::diagnostics::report_bad_interaction_mode(stores, value)?;
                    return Ok(ReplayStep::Continue);
                }
            };
            // See the sibling `SetInteractionMode` arm's comment.
            stores.printer().print_ln();
            stores.set_interaction_mode(mode);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::ItalicCorrection => {
            match modes.current_mode() {
                Mode::Horizontal | Mode::RestrictedHorizontal => {
                    crate::assignments::append_italic_correction_with_fuel(
                        modes,
                        stores,
                        command.fuel,
                    )?;
                }
                Mode::Math | Mode::DisplayMath => {
                    // TeX82 §1112: `mmode+ital_corr: tail_append(new_kern(0));`
                    // -- `new_kern`'s default subtype (`normal`) is never
                    // overridden here (unlike hmode's italic-correction kern,
                    // or an explicit `\kern`), so it must not become a legal
                    // kern-then-glue line-break point.
                    modes.current_list_mutation().push(Node::Kern {
                        amount: Scaled::from_raw(0),
                        kind: KernKind::Font,
                    });
                }
                Mode::Vertical | Mode::InternalVertical => {
                    unreachable!("vertical \\/ is scanned as IllegalItalicCorrection")
                }
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::IllegalItalicCorrection { token } => {
            let context = command.state.output_open_context(&stores.command_context());
            crate::diagnostics::report_illegal_case_with_context(
                stores,
                token,
                modes.current_mode(),
                Some(context),
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::IllegalMacroParameter { token } => {
            let context = command.state.output_open_context(&stores.command_context());
            crate::diagnostics::report_illegal_case_with_context(
                stores,
                token,
                modes.current_mode(),
                Some(context),
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::ExtraEndCsName => {
            // TeX82 §1135's `cs_error`.
            let context = command.state.output_open_context(&stores.command_context());
            report_escaped_error(
                stores,
                "Extra ",
                "endcsname",
                "",
                &["I'm ignoring this, since I wasn't doing a \\csname."],
                context,
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::NoBoundary { suppress_right } => {
            if suppress_right {
                crate::assignments::flush_pending_hchars_without_right_boundary(
                    modes,
                    stores,
                    command.fuel,
                )?;
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::NonScript => {
            // TeX82 §1171: a zero glue with the `cond_math_glue` subtype.
            let spec = stores.intern_glue(GlueSpec::ZERO);
            modes.current_list_mutation().push(Node::Glue {
                spec,
                kind: GlueKind::NonScript,
                leader: None,
            });
            Ok(ReplayStep::Continue)
        }
        ScannedStep::CharacterCode {
            value,
            suppress_left_boundary,
        } => {
            let ch = u32::try_from(value).ok().and_then(char::from_u32).ok_or(
                ExecError::InvalidCode {
                    context: "\\char",
                    value,
                },
            )?;
            if matches!(modes.current_mode(), Mode::Math | Mode::DisplayMath) {
                // TeX82 `main_control`'s `mmode+char_num` (§1154) scans the
                // character number and then calls `set_math_char` (§1155)
                // with its `math_code`, exactly like the sibling
                // `mmode+letter`/`mmode+other_char`/`mmode+char_given` cases:
                // it appends a math-char noad and never begins or continues
                // a horizontal list from math mode.
                set_canonical_math_char(
                    ch,
                    tex_state::token::OriginId::UNKNOWN,
                    stores,
                    modes,
                    command,
                )?;
                return Ok(ReplayStep::Continue);
            }
            if matches!(
                modes.current_mode(),
                Mode::Vertical | Mode::InternalVertical
            ) {
                start_canonical_paragraph(command.state, modes, stores, true)?;
            }
            modes
                .current_list_mutation()
                .set_no_boundary(suppress_left_boundary);
            crate::assignments::append_canonical_character_with_fuel(
                modes,
                stores,
                ch,
                tex_state::token::OriginId::UNKNOWN,
                command.fuel,
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::ControlSpace => {
            match modes.current_mode() {
                Mode::Math | Mode::DisplayMath => {
                    // TeX82 §1030's `mmode+ex_space: goto append_normal_space`
                    // (§1041) appends real interword glue in math mode, unlike
                    // an ordinary `mmode+spacer`, which §1045 makes a no-op.
                    let spec = crate::assignments::control_space_glue_spec(stores);
                    modes.current_list_mutation().push(Node::Glue {
                        spec: stores.intern_glue(spec),
                        kind: GlueKind::Normal,
                        leader: None,
                    });
                }
                Mode::Vertical | Mode::InternalVertical => {
                    start_canonical_paragraph(command.state, modes, stores, true)?;
                    crate::assignments::append_canonical_control_space_with_fuel(
                        modes,
                        stores,
                        command.fuel,
                    )?;
                }
                _ => {
                    crate::assignments::append_canonical_control_space_with_fuel(
                        modes,
                        stores,
                        command.fuel,
                    )?;
                }
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::PrevDepth { value } => {
            if matches!(
                modes.current_mode(),
                Mode::Vertical | Mode::InternalVertical
            ) {
                modes.current_list_mutation().set_prev_depth(value);
            } else {
                // TeX82 §1243's `alter_aux`: `if cur_chr<>abs(mode) then
                // report_illegal_case`, which prints "You can't use
                // `\prevdepth' in ... mode" and otherwise leaves the value
                // alone -- it does not raise an executor error.
                let token = Token::Cs(stores.intern("prevdepth").symbol());
                crate::diagnostics::report_illegal_case(stores, token, modes.current_mode())?;
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::SpaceFactor { value } => {
            debug_assert!(matches!(
                modes.current_mode(),
                Mode::Horizontal | Mode::RestrictedHorizontal
            ));
            // TeX82 §1243's `alter_aux`: `if (cur_val<=0)or(cur_val>32767)
            // then int_error(cur_val) else space_factor:=cur_val` -- an
            // out-of-range value is diagnosed and left unchanged rather than
            // clamped.
            if (1..=32767).contains(&value) {
                modes.current_list_mutation().set_space_factor(value);
            } else {
                // §91's `int_error` appends ` (value)` to the message before
                // §82 completes the report, so the value is not part of the
                // `print_err` text.
                let context = command.state.output_open_context(&stores.command_context());
                let mut report = stores.print_err("Bad space factor");
                report
                    .help(&["I allow only values in the range 1..32767 here."])
                    .context(context);
                report.int_error(value).jump_out()?;
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::IllegalSpaceFactor { token } => {
            let context = command.state.output_open_context(&stores.command_context());
            crate::diagnostics::report_illegal_case_with_context(
                stores,
                token,
                modes.current_mode(),
                Some(context),
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::PrevGraf { value } => {
            // TeX82 §1244's `alter_prev_graf`: `\prevgraf` is `any_mode` (it
            // walks the mode nest up to its nearest enclosing vertical level
            // rather than checking the current mode), unlike `\spacefactor`/
            // `\prevdepth`'s §1243 `report_illegal_case`.
            if value < 0 {
                let context = command.state.output_open_context(&stores.command_context());
                let mut report = stores.print_err("Bad ");
                report
                    .print_esc("prevgraf")
                    .help(&["I allow only nonnegative values here."])
                    .context(context);
                report.int_error(value).jump_out()?;
            } else {
                modes.set_enclosing_vertical_prev_graf(value);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::PageDimension { dimension, value } => {
            // TeX82 §1245's `alter_page_so_far`: a direct
            // `page_so_far[c]:=cur_val` store with no mode check, no
            // diagnostic, and no save-stack entry (§1242: "these definitions
            // are always global"). The page builder reads the same slots.
            stores.set_page_dimension(dimension, value);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::PageInteger { integer, value } => {
            // TeX82 §1246's `alter_integer`, scoped exactly like
            // `alter_page_so_far` above. `\deadcycles` in particular is what
            // §1024's output-routine loop guard compares against
            // `\maxdeadcycles`, so a wrong value here is only visible once a
            // page ships.
            stores.set_page_integer(integer, value);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::FixedHorizontalGlue { primitive } => {
            if matches!(
                modes.current_mode(),
                Mode::Vertical | Mode::InternalVertical
            ) {
                start_canonical_paragraph(command.state, modes, stores, true)?;
            }
            crate::assignments::flush_pending_hchars_with_fuel(modes, stores, command.fuel)?;
            modes.current_list_mutation().push(Node::Glue {
                spec: stores.intern_glue(crate::assignments::fixed_infinite_glue(primitive)),
                kind: GlueKind::Normal,
                leader: None,
            });
            Ok(ReplayStep::Continue)
        }
        ScannedStep::VerticalSkip { value } => {
            // TeX82 §1054's `vmode+vskip: append_glue` (§1057): unlike
            // `\hskip` in vertical mode, `\vskip` never starts a paragraph --
            // the scan side (`scan_command`) only produces this step when the
            // mode is already `Vertical` or `InternalVertical`. §1057 also
            // notes `append_glue` deliberately never calls `build_page`
            // itself ("it is used in at least one place where that would be
            // a mistake"), unlike `append_penalty` (§1103); no page build
            // follows here.
            let spec = stores.intern_glue(value);
            crate::vertical::append_node_to_current_list(
                modes,
                stores,
                Node::Glue {
                    spec,
                    kind: GlueKind::Normal,
                    leader: None,
                },
                command.fuel,
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::FixedVerticalGlue { primitive } => {
            // See `ScannedStep::VerticalSkip` above: same §1054/§1057
            // `append_glue`, no paragraph start, no page build.
            let spec = stores.intern_glue(crate::assignments::fixed_infinite_glue(primitive));
            crate::vertical::append_node_to_current_list(
                modes,
                stores,
                Node::Glue {
                    spec,
                    kind: GlueKind::Normal,
                    leader: None,
                },
                command.fuel,
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::ParagraphIndent { indent } => {
            // TeX82 §1090 routes only `vmode+start_par` to §1091 `new_graf`,
            // the single site that pushes `\everypar`. §1092 routes both
            // `hmode+start_par` and `mmode+start_par` to §1093
            // `indent_in_hmode`, which appends the paragraph-indent box (as
            // an ordinary `sub_box` noad in math mode) without beginning a
            // paragraph -- so an `\indent` inside a paragraph already under
            // way must not replay `\everypar` a second time.
            if matches!(
                modes.current_mode(),
                Mode::Vertical | Mode::InternalVertical
            ) {
                start_canonical_paragraph(command.state, modes, stores, indent)?;
            } else {
                crate::assignments::indent_in_hmode(modes, stores, indent, command.fuel)?;
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::ParagraphShape { lines, global } => {
            // TeX82 §1214's "Adjust for the setting of `\globaldefs`" runs
            // unconditionally before `prefixed_command`'s `case cur_cmd of
            // @<Assignments@> endcases`, so it applies uniformly to all
            // thirty assignment forms §1210 dispatches -- `set_shape`
            // (§1248's `define(par_shape_loc,shape_ref,p)`) among them,
            // since `\parshape` is an ordinary `eqtb` entry that `define`
            // scopes through the save stack. This was the third canonical
            // apply arm (after `\def`/`\edef`/`\gdef`/`\xdef` and
            // `\let`/`\futurelet`) that passed the raw `\global` prefix bit
            // straight through and silently ignored a nonzero
            // `\globaldefs`; it was missed by both earlier sweeps because
            // `set_shape` belongs to neither definition family.
            stores.set_paragraph_shape(&lines, global);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::PenaltyArray {
            kind,
            values,
            global,
        } => {
            // e-TeX 2.6 change [49.1248] commits every selector through
            // `define(q, shape_ref, p)`, so this uses the same save-stack and
            // `\globaldefs`-adjusted scope bit as TeX82 §1214/§1248.
            stores.set_penalty_array(kind, &values, global);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Toks {
            index,
            tokens,
            global,
        } => {
            let old = stores.toks(index);
            let new = tokens.token_list();
            if global {
                stores.set_toks_global(index, new);
            } else {
                stores.set_toks(index, new);
            }
            crate::assignments::tracing::trace_toks_register(stores, index, global, old, new);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::IntParam {
            index,
            value,
            global,
        } => {
            let parameter = IntParam::new(index);
            let old = stores.int_param(parameter);
            let tracing_before = stores.int_param(IntParam::TRACING_ASSIGNS) > 0;
            if global {
                stores.set_int_param_global(parameter, value);
            } else if !etex_redundant_local_word_assignment(stores, old, value) {
                stores.set_int_param(parameter, value);
            }
            crate::assignments::tracing::trace_int_param(
                stores,
                index,
                tracing_before,
                global,
                old,
                value,
            );
            Ok(ReplayStep::Continue)
        }
        ScannedStep::DimenParam {
            index,
            value,
            global,
        } => {
            let parameter = DimenParam::new(index);
            let old = stores.dimen_param(parameter);
            if global {
                stores.set_dimen_param_global(parameter, value);
            } else if !etex_redundant_local_word_assignment(stores, old, value) {
                stores.set_dimen_param(parameter, value);
            }
            crate::assignments::tracing::trace_dimen_param(stores, index, global, old, value);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::TokParam {
            index,
            tokens,
            global,
        } => {
            let parameter = TokParam::new(index);
            let old = stores.tok_param(parameter);
            let new = tokens.token_list();
            if global {
                stores.set_tok_param_global(parameter, new);
            } else {
                stores.set_tok_param(parameter, new);
            }
            crate::assignments::tracing::trace_tok_param(stores, index, global, old, new);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::GlueParam {
            index,
            value,
            global,
        } => {
            let parameter = GlueParam::new(index);
            let old = stores.glue_param(parameter);
            let new = if global {
                let new = stores.intern_glue(value);
                stores.set_glue_param_global(parameter, new);
                new
            } else if !etex_redundant_local_zero_glue_assignment(stores, old, &value) {
                let new = stores.intern_glue(value);
                stores.set_glue_param(parameter, new);
                new
            } else {
                old
            };
            crate::assignments::tracing::trace_glue_param(stores, index, global, old, new);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::CodeTable {
            primitive,
            character,
            value,
            global,
        } => {
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
                    let old = stores.catcode(character);
                    if global {
                        stores.set_catcode_global(character, value);
                    } else if !etex_redundant_local_word_assignment(stores, old, value) {
                        stores.set_catcode(character, value);
                    }
                    crate::assignments::tracing::trace_code(
                        stores,
                        "catcode",
                        character,
                        global,
                        old as i32,
                        value as i32,
                    );
                }
                UnexpandablePrimitive::LcCode => {
                    let value = u32::try_from(value).map_err(|_| ExecError::InvalidCode {
                        context: "\\lccode",
                        value,
                    })? as LcCode;
                    let old = stores.lccode(character);
                    if global {
                        stores.set_lccode_global(character, value);
                    } else if !etex_redundant_local_word_assignment(stores, old, value) {
                        stores.set_lccode(character, value);
                    }
                    crate::assignments::tracing::trace_code(
                        stores,
                        "lccode",
                        character,
                        global,
                        old as i32,
                        value as i32,
                    );
                }
                UnexpandablePrimitive::UcCode => {
                    let value = checked_character_code(value, "\\uccode")? as UcCode;
                    let old = stores.uccode(character);
                    if global {
                        stores.set_uccode_global(character, value);
                    } else if !etex_redundant_local_word_assignment(stores, old, value) {
                        stores.set_uccode(character, value);
                    }
                    crate::assignments::tracing::trace_code(
                        stores,
                        "uccode",
                        character,
                        global,
                        old as i32,
                        value as i32,
                    );
                }
                UnexpandablePrimitive::SfCode => {
                    let value = u16::try_from(value)
                        .ok()
                        .filter(|value| *value <= 32_767)
                        .ok_or(ExecError::InvalidCode {
                            context: "\\sfcode",
                            value,
                        })? as SfCode;
                    let old = stores.sfcode(character);
                    if global {
                        stores.set_sfcode_global(character, value);
                    } else if !etex_redundant_local_word_assignment(stores, old, value) {
                        stores.set_sfcode(character, value);
                    }
                    crate::assignments::tracing::trace_code(
                        stores,
                        "sfcode",
                        character,
                        global,
                        i32::from(old),
                        i32::from(value),
                    );
                }
                UnexpandablePrimitive::MathCode => {
                    let value = u32::try_from(value)
                        .ok()
                        .filter(|value| *value <= 32_768)
                        .ok_or(ExecError::InvalidCode {
                            context: "\\mathcode",
                            value,
                        })? as MathCode;
                    let old = stores.mathcode(character);
                    if global {
                        stores.set_mathcode_global(character, value);
                    } else if !etex_redundant_local_word_assignment(stores, old, value) {
                        stores.set_mathcode(character, value);
                    }
                    crate::assignments::tracing::trace_code(
                        stores,
                        "mathcode",
                        character,
                        global,
                        old as i32,
                        value as i32,
                    );
                }
                UnexpandablePrimitive::DelCode => {
                    let value = (-1..=0xFF_FFFF)
                        .contains(&value)
                        .then_some(value as DelCode)
                        .ok_or(ExecError::InvalidCode {
                            context: "\\delcode",
                            value,
                        })?;
                    let old = stores.delcode(character);
                    if global {
                        stores.set_delcode_global(character, value);
                    } else if !etex_redundant_local_word_assignment(stores, old, value) {
                        stores.set_delcode(character, value);
                    }
                    crate::assignments::tracing::trace_code(
                        stores, "delcode", character, global, old, value,
                    );
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
            if global {
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
            // TeX82 §1258/§1259 report an illegal `at`/`scaled` size and
            // continue with the replaced value; §1257 then loads the font
            // normally. The replacement is the scanner's, the report this
            // seam's.
            if let Some(recovery) = &request.size_recovery {
                report_font_size_recovery(stores, recovery)?;
            }
            let resource =
                (*resource).expect("font resource is resolved after the processor borrow");
            if matches!(resource, FontResource::Unavailable) {
                if global {
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
            let id = match stores.try_intern_font_with_identifier(loaded, request.target) {
                Ok(id) => id,
                Err(tex_state::FontParameterError::TooManyFonts { .. }) => {
                    let selector = stores.resolve(request.target).to_owned();
                    crate::assignments::fonts::report_font_capacity(
                        stores,
                        &selector,
                        &request.name,
                        request.size,
                    )?;
                    if global {
                        stores.set_meaning_global(
                            request.target,
                            Meaning::Font(tex_state::font::NULL_FONT),
                        );
                    } else {
                        stores
                            .set_meaning(request.target, Meaning::Font(tex_state::font::NULL_FONT));
                    }
                    return Ok(ReplayStep::Continue);
                }
                Err(error) => return Err(error.into()),
            };
            if global {
                stores.set_meaning_global(request.target, Meaning::Font(id));
            } else {
                stores.set_meaning(request.target, Meaning::Font(id));
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::InputStream { request, resource } => {
            match request {
                InputStreamRequest::Open {
                    stream, file_name, ..
                } => {
                    let slot = replay_stream_slot(stream);
                    let packed_name = file_name.packed();
                    // §1275 closes any stream already open on `n` before it
                    // tries to open the new file, whichever command this is.
                    stores.world_mut().close_in(slot);
                    let Some(resource) = resource else {
                        return Ok(ReplayStep::Continue);
                    };
                    stores
                        .world_mut()
                        .set_memory_file(&packed_name, resource.bytes().to_vec())?;
                    let content = InputReadState::read_input_file(
                        &mut stores.input_open_context(),
                        std::path::Path::new(&packed_name),
                    )?;
                    stores.world_mut().open_in_content(slot, &content)?;
                }
                InputStreamRequest::Close { stream, .. } => {
                    let slot = replay_stream_slot(stream);
                    stores.world_mut().close_in(slot);
                }
                // TeX82 §482 has already collected the list inside the
                // command core, which also reported §1225's missing-`to`
                // recovery at the point tex.web reports it; the definition is
                // all that is left.
                InputStreamRequest::Read {
                    target,
                    global,
                    tokens,
                    ..
                } => {
                    let parameters = stores.intern_token_list(&[]);
                    let meaning =
                        MacroMeaning::new(MeaningFlags::EMPTY, parameters, tokens.token_list());
                    if global {
                        stores.set_macro_meaning_global(target, meaning);
                    } else {
                        stores.set_macro_meaning(target, meaning);
                    }
                }
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::PdfXImage { request, resource } => {
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfximage"));
            }
            let source = match resource {
                PdfImageResource::Available(source) => source,
                PdfImageResource::Unavailable => {
                    return Err(ExecError::PdfImageOpen {
                        name: request.name,
                        message: "image is unavailable".to_owned(),
                    });
                }
                PdfImageResource::Invalid(message) => {
                    return Err(ExecError::PdfImageOpen {
                        name: request.name,
                        message,
                    });
                }
            };
            let dimensions = canonical_pdf_image_dimensions(
                &source,
                request.width,
                request.height,
                request.depth,
            );
            stores
                .allocate_pdf_external_image(source, dimensions, request.color_space_object)
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
            crate::assignments::append_whatsit(
                modes,
                stores,
                command.fuel,
                Whatsit::PdfRefXImage {
                    object: image.id().raw(),
                    width: dimensions.width,
                    height: dimensions.height,
                    depth: dimensions.depth,
                },
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::PdfSetRandomSeed { seed } => {
            stores.world_mut().set_pdf_random_seed(seed);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::PdfResetTimer => {
            stores.world_mut().reset_pdf_timer();
            Ok(ReplayStep::Continue)
        }
        ScannedStep::PdfInterwordSpace(control) => {
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                let name = match control {
                    tex_state::node::PdfAccessibilityControl::InterwordSpaceOn => {
                        "pdfinterwordspaceon"
                    }
                    tex_state::node::PdfAccessibilityControl::InterwordSpaceOff => {
                        "pdfinterwordspaceoff"
                    }
                    tex_state::node::PdfAccessibilityControl::FakeSpace => "pdffakespace",
                };
                return Err(ExecError::PdfExtensionInDviMode(name));
            }
            crate::assignments::append_whatsit(
                modes,
                stores,
                command.fuel,
                Whatsit::PdfAccessibility(control),
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::PdfRunningLink(enabled) => {
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode(if enabled {
                    "pdfrunninglinkon"
                } else {
                    "pdfrunninglinkoff"
                }));
            }
            crate::assignments::append_whatsit(
                modes,
                stores,
                command.fuel,
                Whatsit::PdfRunningLink(enabled),
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::PdfSpaceFont(tokens) => {
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfspacefont"));
            }
            stores.set_pdf_space_font_name(pdf_graphics_text(tokens, stores));
            Ok(ReplayStep::Continue)
        }
        ScannedStep::PdfGraphics(request) => {
            apply_pdf_graphics_request(request, stores, modes, command.state)
        }
        ScannedStep::PdfNavigation(request) => {
            apply_pdf_navigation_request(request, stores, modes, command.fuel)
        }
        ScannedStep::PdfObject(request) => apply_pdf_object_request(request, stores, false),
        ScannedStep::PdfReferenceObject(request) => {
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfrefobj"));
            }
            let object = u32::try_from(request.object)
                .ok()
                .filter(|object| stores.pdf_raw_object(*object).is_some())
                .ok_or(ExecError::PdfReferencedObjectNotFound)?;
            crate::assignments::append_whatsit(
                modes,
                stores,
                command.fuel,
                Whatsit::PdfReferenceObject { object },
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::PdfForm(request) => {
            apply_pdf_form_request(request, stores, modes, command.fuel, false)
        }
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
            recovery_context,
        } => {
            // tex.web §578's `find_font_dimen` resolves an unusable parameter
            // number to the scratch location `fmem_ptr`; §579 then reports
            // "Font x has only n fontdimen parameters" and §1253 still runs
            // `scan_optional_equals; scan_normal_dimen; font_info[k].sc:=
            // cur_val` into that scratch cell, so the font is unchanged and
            // the job continues. Only §580's grow path, which
            // `set_font_dimen` implements, can add a parameter.
            //
            // The scan already made §578's decision and captured §579's
            // context there, so this only writes or reports.
            match recovery_context {
                Some(context) => report_font_parameter_recovery(stores, font, context)?,
                None => {
                    let number = u32::try_from(number)
                        .expect("a writable parameter number is a positive u32");
                    stores
                        .set_font_dimen(font, number, value)
                        .expect("§578 accepted this parameter number during the scan");
                }
            }
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
            crate::assignments::append_whatsit(
                modes,
                stores,
                command.fuel,
                Whatsit::OpenOut {
                    slot: StreamSlot::new(stream),
                    path: file_name,
                },
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::DeferredCloseOut { stream } => {
            crate::assignments::append_whatsit(
                modes,
                stores,
                command.fuel,
                Whatsit::CloseOut {
                    slot: stream.stream_slot(),
                },
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::DeferredWrite { stream, tokens } => {
            crate::assignments::append_whatsit(
                modes,
                stores,
                command.fuel,
                Whatsit::DeferredWrite {
                    sink: replay_write_sink(stream),
                    tokens: tokens.token_list(),
                },
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::DeferredSpecial { tokens } => {
            let mut text = String::new();
            for &token in stores.tokens(tokens.token_list()) {
                tex_expand::append_token_string_text(stores, token, &mut text);
            }
            crate::assignments::append_whatsit(
                modes,
                stores,
                command.fuel,
                Whatsit::Special {
                    class: "dvi".to_owned(),
                    payload: tex_byte_text(&text),
                },
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::SetLanguage { language } => {
            // TeX82 §1377, verbatim:
            //
            //   new_whatsit(language_node,small_node_size);
            //   scan_int;
            //   if cur_val<=0 then clang:=0
            //   else if cur_val>255 then clang:=0
            //   else clang:=cur_val;
            //   what_lang(tail):=clang;
            //   what_lhm(tail):=norm_min(left_hyphen_min);
            //   what_rhm(tail):=norm_min(right_hyphen_min);
            //
            // Both out-of-range directions recover to language zero; only
            // `1..=255` survives. The pending character run is flushed
            // first, before `clang` moves, so it hyphenates under the
            // language that was current while it was being built.
            let clang = u8::try_from(language).unwrap_or(0);
            let left_hyphen_min =
                crate::assignments::norm_min(stores.int_param(IntParam::LEFT_HYPHEN_MIN));
            let right_hyphen_min =
                crate::assignments::norm_min(stores.int_param(IntParam::RIGHT_HYPHEN_MIN));
            crate::assignments::append_whatsit(
                modes,
                stores,
                command.fuel,
                Whatsit::Language {
                    language: clang,
                    left_hyphen_min,
                    right_hyphen_min,
                },
            )?;
            modes.current_list_mutation().set_hyphen_language(clang);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::IllegalSetLanguage { token } => {
            let context = command.state.output_open_context(&stores.command_context());
            crate::diagnostics::report_illegal_case_with_context(
                stores,
                token,
                modes.current_mode(),
                Some(context),
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Arithmetic {
            primitive,
            target,
            operand,
            global,
        } => {
            // TeX82 §1236 sets `arith_error` and, when it is set, reports
            // "Arithmetic overflow" and `return`s *before* `word_define`, so
            // the target keeps its old value and the job continues. Every
            // arm of `apply_arithmetic` computes its value before writing it,
            // so the target is provably unwritten on this path.
            match apply_arithmetic(primitive, target, operand, global, stores) {
                Err(ExecError::ArithmeticOverflow) => {
                    let context = command.state.output_open_context(&stores.command_context());
                    let mut report = stores.print_err("Arithmetic overflow");
                    report.help(&[
                        "I can't carry out that multiplication or division,",
                        "since the result is out of range.",
                    ]);
                    report.context(context);
                    report.error().jump_out()?;
                }
                other => other?,
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::InvalidArithmeticTarget { primitive, target } => {
            // TeX82 §1236 prints this error and returns from
            // `do_register_command`; §1269's common `done` path still gets
            // to replay a pending `\afterassignment` token.
            let target = tex_command::print_cmd_chr_text(&stores.command_context(), target);
            let primitive = printed_command(stores, Meaning::UnexpandablePrimitive(primitive));
            let context = command.state.output_open_context(&stores.command_context());
            let mut report = stores.print_err("You can't use `");
            report.print(&target).print("' after ").print(&primitive);
            report.help(&["I'm forgetting what you said and not changing anything."]);
            report.context(context);
            report.error().jump_out()?;
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
        } => {
            if missing_target {
                // TeX82 §1215's `get_r_token`; the frozen `\inaccessible`
                // insertion it recovers with is already the scanner's.
                let context = command.state.output_open_context(&stores.command_context());
                crate::error_report::report_error(
                    stores,
                    "Missing control sequence inserted",
                    &[
                        "Please don't say `\\def cs{...}', say `\\def\\cs{...}'.",
                        "I've inserted an inaccessible control sequence so that your",
                        "definition will be completed without mixing me up too badly.",
                        "You can recover graciously from this error, if you're",
                        "careful; see exercise 27.2 in The TeXbook.",
                    ],
                    context,
                )?;
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
            // TeX82 §1211's generic `prefixed_command` global-scope
            // resolution (the same `\globaldefs` override every other
            // assignment receives from §1214) applies to
            // `\def`/`\edef`/`\gdef`/`\xdef` exactly like any other
            // assignment; `global` here already folds in `\gdef`/`\xdef`'s
            // own forced-global chr_code (see the scan arm above), and
            // `global` already is the final effective bit.
            crate::assignments::tracing::trace_meaning_write(
                stores,
                Token::Cs(target),
                // TeX82's `\def` family always allocates a fresh definition,
                // so real TeX never reports "reassigning" here even for a
                // byte-identical redefinition -- see
                // `crate::assignments::tracing::trace_meaning_write`'s docs.
                true,
                global,
                |stores| {
                    if global {
                        stores
                            .set_macro_meaning_global_with_provenance(target, meaning, provenance);
                    } else {
                        stores.set_macro_meaning_with_provenance(target, meaning, provenance);
                    }
                },
            );
            Ok(ReplayStep::Continue)
        }
        ScannedStep::CharacterDefinition {
            primitive,
            target,
            value,
            global,
            ..
        } => {
            let meaning = match primitive {
                UnexpandablePrimitive::CharDef => Meaning::CharGiven(
                    char::from_u32(value as u32)
                        .expect("§434 recovers a character code to a character"),
                ),
                UnexpandablePrimitive::MathCharDef => Meaning::MathCharGiven(value as u16),
                _ => unreachable!("character-definition step carries only §1224 primitives"),
            };
            if global {
                stores.set_meaning_global(target, meaning);
            } else if !etex_redundant_local_word_assignment(stores, stores.meaning(target), meaning)
            {
                stores.set_meaning(target, meaning);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::RegisterDefinition {
            primitive,
            target,
            index,
            global,
        } => {
            let meaning = match primitive {
                UnexpandablePrimitive::CountDef => Meaning::CountRegister(index),
                UnexpandablePrimitive::DimenDef => Meaning::DimenRegister(index),
                UnexpandablePrimitive::SkipDef => Meaning::SkipRegister(index),
                UnexpandablePrimitive::MuskipDef => Meaning::MuskipRegister(index),
                UnexpandablePrimitive::ToksDef => Meaning::ToksRegister(index),
                _ => unreachable!("register-definition step carries only §1224 primitives"),
            };
            if global {
                stores.set_meaning_global(target, meaning);
            } else {
                stores.set_meaning(target, meaning);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::HyphenationData {
            words,
            pattern_specs,
            patterns,
            rejection_context,
            trie_built,
        } => {
            // TeX82 §1252 rejects `\patterns` for two different reasons, and
            // the two do not share a message. The `init`/`tini` split comes
            // first: a production binary has no `new_patterns` to call, so it
            // reports "Patterns can be loaded only by INITEX" with `help0`
            // and flushes the braced group. Only INITEX reaches §960, whose
            // own `trie_not_ready=false` guard is the "Too late" one, and it
            // does carry help. `\hyphenation` is legal in both binaries.
            if patterns && !command.initex {
                let mut report = stores.print_err("Patterns can be loaded only by INITEX");
                report.context(rejection_context);
                report.error().jump_out()?;
                return Ok(ReplayStep::Continue);
            }
            if trie_built {
                let mut report = stores.print_err("Too late for \\patterns");
                report.help(&["All patterns must be given before typesetting begins."]);
                report.context(rejection_context);
                report.error().jump_out()?;
                return Ok(ReplayStep::Continue);
            }
            // Both halves of §§935/963's diagnostics were already reported by
            // the live scan, where §82 could still show the character that
            // caused them; installing is all that is left here.
            if patterns {
                crate::assignments::apply_scanned_patterns(stores, pattern_specs);
            } else {
                crate::assignments::apply_scanned_hyphenation_exceptions(stores, words);
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
            // TeX82 `\let`/`\futurelet` are ordinary `prefixed_command`
            // assignments too (§1221), so `\globaldefs` must override their
            // scope exactly like every other assignment kind's
            // effective-scope resolution above -- this was the second (with
            // `\def`/`\edef`/`\gdef`/`\xdef`) canonical apply arm that used
            // the raw `\global` prefix bit directly and silently ignored a
            // nonzero `\globaldefs`.
            let changed =
                !etex_redundant_local_word_assignment(stores, stores.meaning(target), meaning);
            crate::assignments::tracing::trace_meaning_write(
                stores,
                Token::Cs(target),
                changed,
                global,
                |stores| {
                    if global {
                        stores.set_meaning_global(target, meaning);
                    } else {
                        stores.set_meaning(target, meaning);
                    }
                },
            );
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
        } => apply_scanned_rule(command, modes, stores, width, height, depth, horizontal),
        ScannedStep::Message { tokens, error } => {
            // TeX82 §1279's `issue_message` renders the scanned list through
            // `token_show` into one string and then hands it to §1280 or
            // §1283; neither branch formats or routes its own output.
            let text = message_text(stores, tokens.token_list());
            if error {
                let context = command.state.output_open_context(&stores.command_context());
                issue_error_message(stores, &text, context)?;
            } else {
                issue_terminal_message(stores, &text);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::DisplayDiagnostic(diagnostic) => {
            // TeX82 §§62/1294/1297 begin the display with `print_nl(">␣")`,
            // which closes a partial selected line but does not add a blank
            // line when both selected sinks are already at column zero.
            // The scanned value carries exactly that line's content; replay
            // owns the selector-sensitive transition and decodes no textual
            // envelope.
            let context = command.state.output_open_context(&stores.command_context());
            print_display_content(stores, &diagnostic.content);
            crate::diagnostics::complete_show(stores, false, Some(context))?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::ShowBox { index } => {
            let context = command.state.output_open_context(&stores.command_context());
            crate::diagnostics::execute_showbox(stores, index, context)?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::ShowLists => {
            let context = command.state.output_open_context(&stores.command_context());
            crate::diagnostics::execute_showlists(stores, modes, context)?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::ShowTokens { tokens } => {
            // e-TeX's odd xray modifier reaches `the_toks`, then TeX82
            // §1297 prints `token_show(temp_head)` and takes the common
            // `\show` completion path.
            let context = command.state.output_open_context(&stores.command_context());
            let text = show_tokens_text(stores, tokens.token_list());
            // §1297 opens with §62's `print_nl(">␣")`, whose break is
            // conditional on a selected sink already having an open column.
            // An unconditional newline here left a blank line above the
            // display whenever the file's own `(` had just closed one.
            stores.printer().print_nl("> ").print_rendered(&text);
            crate::diagnostics::complete_show(stores, false, Some(context))?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::ShowIfs { conditions } => {
            // etex.ch [17.3720]'s `show_ifs` is a `begin_diagnostic` form
            // like `\showbox`/`\showlists`/`\showgroups`, not a direct
            // print: see `tex-exec::diagnostics`'s module doc for why the
            // dump must be routed through §245's redirection rather than
            // written straight to both channels.
            let context = command.state.output_open_context(&stores.command_context());
            let mut diagnostic = stores.begin_diagnostic();
            diagnostic.print_nl("").print_ln();
            diagnostic.print_rendered(&render_showifs(&conditions));
            diagnostic.end(true);
            crate::diagnostics::complete_show(stores, true, Some(context))?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::ShowGroups {
            diagnostic: Some(diagnostic),
        } => {
            let context = command.state.output_open_context(&stores.command_context());
            crate::diagnostics::execute_canonical_showgroups(stores, &diagnostic, context)?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::ShowGroups { diagnostic: None } => {
            let diagnostic = detached_showgroups(stores, modes, active_alignment, boxes);
            let context = command.state.output_open_context(&stores.command_context());
            crate::diagnostics::execute_canonical_showgroups(stores, &diagnostic, context)?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::ImmediateExtension(extension) => {
            match extension {
                ImmediateExtension::Continue => {}
                ImmediateExtension::PdfExtensionInDviMode(primitive) => {
                    let name = match primitive {
                        UnexpandablePrimitive::PdfObject => "pdfobj",
                        UnexpandablePrimitive::PdfXForm => "pdfxform",
                        UnexpandablePrimitive::PdfXImage => "pdfximage",
                        _ => unreachable!("only immediate PDF extensions reach this result"),
                    };
                    return Err(ExecError::PdfExtensionInDviMode(name));
                }
                ImmediateExtension::OpenOut { stream, file_name } => {
                    let target = replay_openout_target(file_name.packed());
                    stores
                        .world_mut()
                        .open_out(StreamSlot::new(stream), target.clone());
                    crate::diagnostics::report_openout(stores, stream, &target);
                }
                ImmediateExtension::Write { stream, tokens } => {
                    let sink = replay_write_sink(stream);
                    let text = canonical_write_text(stores.tokens(tokens.token_list()), stores);
                    stores.world_mut().write_text(sink, &text);
                }
                ImmediateExtension::CloseOut { stream } => {
                    if let Some(stream) = stream.stream_slot() {
                        stores.world_mut().close_out(stream);
                    }
                }
                ImmediateExtension::PdfObject(request) => {
                    if matches!(request, PdfObjectRequest::Reserve) {
                        apply_pdf_object_request(request, stores, false)?;
                        return Err(ExecError::PdfImmediateReservedObject);
                    }
                    apply_pdf_object_request(request, stores, true)?;
                }
                ImmediateExtension::PdfForm(request) => {
                    apply_pdf_form_request(request, stores, modes, command.fuel, true)?;
                }
                ImmediateExtension::PdfImage(_) => {
                    unreachable!("immediate image requests are normalized before resolution")
                }
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::SetBox(target) => {
            // §1214's `<Adjust for the setting of \globaldefs>` runs inside
            // `prefixed_command`, before §1241 scans the box, so `global` in
            // §1241's `if global then n:=256+cur_val` is the *effective*
            // scope. Resolving it at `box_end` instead would read
            // `\globaldefs` as the box body left it.
            boxes.pending_setbox = Some(target);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::VSplit(split) => {
            if split.missing_to {
                report_missing_vsplit_to(command.state, stores)?;
            }
            let node = crate::assignments::split_vbox_register(stores, split.index, split.height)?;
            let context = boxes.take_box_context(false);
            box_end(context, node, modes, stores, prepared_dvi_pages, command)?;
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
            let node = crate::assignments::first_box_node(stores, id);
            let context = boxes.take_box_context(ships_out);
            box_end(context, node, modes, stores, prepared_dvi_pages, command)?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Unbox { primitive, index } => {
            crate::assignments::execute_scanned_unbox(
                primitive,
                index,
                modes,
                stores,
                command.fuel,
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::SavedVerticalDiscards(primitive) => {
            crate::assignments::execute_scanned_saved_vertical_discards(
                primitive,
                modes,
                stores,
                command.fuel,
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::LastBox => {
            let node = crate::assignments::take_last_box(modes, stores, command.fuel)?;
            let context = boxes.take_box_context(false);
            box_end(context, node, modes, stores, prepared_dvi_pages, command)?;
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
                command.fuel,
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
                    command.fuel,
                )?;
                crate::vertical::build_page_if_outer_vertical(modes, stores)?;
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::MissingLeaderPayload => {
            // A leader payload is scanned by §1084's `scan_box` like any
            // other box context, so a non-box command there gets §1084's own
            // report, not a leader-specific one.
            report_missing_box(command.state, stores)?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::LeadersNotFollowedByGlue => {
            boxes.pending_leader = None;
            // TeX82 §1078's `back_error`; `scan_leader_glue_command` has
            // already put the command that was not glue back.
            let context = command.state.output_open_context(&stores.command_context());
            crate::error_report::report_error(
                stores,
                "Leaders not followed by proper glue",
                &[
                    "You should say `\\leaders <box or rule><hskip or vskip>'.",
                    "I found the <box or rule>, but there's no suitable",
                    "<hskip or vskip>, so I'm ignoring these leaders.",
                ],
                context,
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BeginShipout => {
            boxes.pending_shipout = true;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BeginBox(construction) => {
            let target = boxes.pending_setbox.take();
            let ships_out = std::mem::take(&mut boxes.pending_shipout);
            let kind = ReplayBoxKind::from_scanned(construction.kind);
            let packing = match construction.packing {
                ScannedPackingSpec::Natural => PackSpec::Natural,
                ScannedPackingSpec::Exactly(size) => PackSpec::Exactly(size),
                ScannedPackingSpec::Spread(size) => PackSpec::Spread(size),
            };
            // TeX82 §1083 uses `adjusted_hbox_group` only when the hbox will
            // be appended (`box_context<box_flag`) in either vertical mode
            // (`abs(mode)=vmode`). A register or shipout construction, and
            // an ordinary hbox in a nonvertical mode, use `hbox_group`.
            let group_kind = if kind == ReplayBoxKind::HBox
                && target.is_none()
                && !ships_out
                && matches!(
                    modes.current_mode(),
                    Mode::Vertical | Mode::InternalVertical
                ) {
                GroupKind::AdjustedHBox
            } else {
                kind.group_kind()
            };
            enter_canonical_group(stores, command.state, group_kind);
            modes.push(if kind.horizontal() {
                Mode::RestrictedHorizontal
            } else {
                Mode::InternalVertical
            })?;
            // TeX82 §§1051--1052 and §1167 run `normal_paragraph` after
            // opening every internal-vertical box body. In particular, a
            // `\vbox`/`\vtop` must not inherit the enclosing `\parshape`.
            if !kind.horizontal() {
                crate::assignments::normal_paragraph(modes, stores);
            }
            boxes.active_boxes.push(ActiveReplayBox {
                target,
                ships_out,
                kind,
                group_kind,
                packing,
                leader_kind: None,
                shift: None,
            });
            schedule_everybox(command.state, stores, kind.horizontal());
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BeginInsert(construction) => {
            // TeX82 §1099's `begin_insert_or_adjust`: `scan_eight_bit_int`
            // has already applied its range clamp and queued the canonical
            // "Bad register code" report. The additional `\insert255`
            // rejection ("box 255 is special") runs here. `\vadjust` set
            // `class:=255` directly
            // (`is_vadjust`), without ever calling `scan_eight_bit_int`, so
            // neither diagnostic applies to it -- 255 is its correct,
            // already-valid sentinel class, not a user-typed `\insert255`.
            let mut class = construction.class;
            if !construction.is_vadjust && class == 255 {
                let mut report = stores.print_err("You can't ");
                report
                    .print_esc("insert")
                    .print_int(255)
                    .help(&["I'm changing to \\insert0; box 255 is special."]);
                if let Some(context) = construction.reserved_class_context {
                    report.context(context);
                }
                report.error().jump_out()?;
                class = 0;
            }
            let class = class as u16;
            enter_canonical_group(stores, command.state, GroupKind::Insert);
            modes.push(Mode::InternalVertical)?;
            // §1099: `normal_paragraph` resets \parshape/\looseness/\hangindent/
            // \hangafter local to the just-opened insert group, exactly like
            // `begin_box` does for `\vbox`/`\vtop` (§1051-2).
            crate::assignments::normal_paragraph(modes, stores);
            boxes.active_boxes.push(ActiveReplayBox {
                target: None,
                ships_out: false,
                kind: ReplayBoxKind::Insert(class, construction.pre),
                group_kind: GroupKind::Insert,
                packing: PackSpec::Natural,
                leader_kind: None,
                shift: None,
            });
            // Unlike `\hbox`/`\vbox`/`\vtop`, §1099 never begins the
            // `\everyhbox`/`\everyvbox` token list for an insertion body.
            Ok(ReplayStep::Continue)
        }
        ScannedStep::IllegalInsertOrAdjust { token } => {
            let context = command.state.output_open_context(&stores.command_context());
            crate::diagnostics::report_illegal_case_with_context(
                stores,
                token,
                modes.current_mode(),
                Some(context),
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::IllegalEqNo { token } => {
            let context = command.state.output_open_context(&stores.command_context());
            crate::diagnostics::report_illegal_case_with_context(
                stores,
                token,
                modes.current_mode(),
                Some(context),
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::IllegalLastItem { token, context } => {
            crate::diagnostics::report_illegal_case_with_context(
                stores,
                token,
                modes.current_mode(),
                Some(context),
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::UndefinedControlSequence => {
            let context = command.state.output_open_context(&stores.command_context());
            crate::diagnostics::report_undefined_control_sequence(stores, Some(context))?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::MisplacedAlignmentDelimiter { token } => {
            let context = command.state.output_open_context(&stores.command_context());
            crate::diagnostics::report_misplaced_alignment_delimiter(stores, token, Some(context))?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Mark { class, tokens } => {
            // No `build_page` call afterward (unlike `\penalty`/`\insert`):
            // TeX82 §1101 and e-TeX 2.6 [26.424]'s `make_mark` append the
            // node in every mode and leave page building to a later trigger.
            crate::assignments::flush_pending_hchars_with_fuel(modes, stores, command.fuel)?;
            crate::vertical::append_vertical_contribution(
                modes,
                stores,
                Node::Mark {
                    class,
                    tokens: tokens.token_list(),
                },
            );
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BeginLeaderBox {
            construction,
            kind: leader_kind,
        } => {
            let kind = ReplayBoxKind::from_scanned(construction.kind);
            let packing = match construction.packing {
                ScannedPackingSpec::Natural => PackSpec::Natural,
                ScannedPackingSpec::Exactly(size) => PackSpec::Exactly(size),
                ScannedPackingSpec::Spread(size) => PackSpec::Spread(size),
            };
            enter_canonical_group(stores, command.state, kind.group_kind());
            modes.push(if kind.horizontal() {
                Mode::RestrictedHorizontal
            } else {
                Mode::InternalVertical
            })?;
            if !kind.horizontal() {
                crate::assignments::normal_paragraph(modes, stores);
            }
            boxes.active_boxes.push(ActiveReplayBox {
                target: None,
                ships_out: false,
                kind,
                group_kind: kind.group_kind(),
                packing,
                leader_kind: Some(leader_kind),
                shift: None,
            });
            schedule_everybox(command.state, stores, kind.horizontal());
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BoxShift(shift) => {
            apply_box_shift(shift, command.state, modes, stores, boxes, command.fuel)
        }
        ScannedStep::IllegalBoxShift { token } => {
            let context = command.state.output_open_context(&stores.command_context());
            crate::diagnostics::report_illegal_case_with_context(
                stores,
                token,
                modes.current_mode(),
                Some(context),
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BeginSimpleGroup => {
            enter_canonical_group(stores, command.state, GroupKind::Simple);
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
        ScannedStep::OutputRoutineOpeningBrace => {
            boxes.output_routine_opening_pending = false;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::EndOutputRoutine => {
            // TeX82 §1026 retires the output token list, then runs §1096's
            // `end_graf` before it unsaves the output group. A non-null
            // paragraph left open by \output must be line-broken into this
            // internal vertical list; merely popping it discards the paragraph.
            // `end_paragraph` is the shared spelling of §1096: it ignores
            // non-horizontal modes and pops a null paragraph without a line.
            crate::assignments::end_paragraph_with_fuel(modes, stores, command.fuel)?;
            let output_level =
                crate::assignments::commit_current_list(modes, stores, command.fuel)?;
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
        ScannedStep::EjectResidualPage => {
            // TeX82 §1054's `its_all_over` false branch. The stop is already
            // backed up; appending the end-job trio and running §994's
            // `build_page` is all the ejection this step performs. §1005's
            // `@<Check if node p is a new champion breakpoint...@>` decides
            // whether the `-'10000000000` penalty fires §1012's `fire_up`,
            // and §1025 alone ever starts `\output`.
            crate::output::append_end_job_contributions(stores);
            crate::page_builder::build_page(stores)?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::IllegalStop { token } => {
            // TeX82 §1051's `privileged`: `\end`/`\dump` below outer
            // vertical mode reports and is discarded, exactly like the other
            // Forbidden cases.
            let context = command.state.output_open_context(&stores.command_context());
            crate::diagnostics::report_illegal_case_with_context(
                stores,
                token,
                modes.current_mode(),
                Some(context),
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BeginOrdinaryGroup => {
            // TeX82 §1038's main-loop lookahead ends the current ligature
            // run when the next expanded command is not a character. A brace
            // is therefore a real text boundary on both entry and exit:
            // `{f}i` must not form `fi` across the closing brace.
            crate::assignments::flush_pending_hchars_with_fuel(modes, stores, command.fuel)?;
            enter_canonical_group(stores, command.state, GroupKind::Simple);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BeginSemiSimpleGroup => {
            crate::assignments::flush_pending_hchars_with_fuel(modes, stores, command.fuel)?;
            enter_canonical_group(stores, command.state, GroupKind::SemiSimple);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::EndSemiSimpleGroup => {
            crate::assignments::flush_pending_hchars_with_fuel(modes, stores, command.fuel)?;
            let aftergroup = stores
                .leave_group_with_kind(GroupKind::SemiSimple)
                .map_err(|_| ExecError::MissingToken {
                    context: "semi simple group",
                })?;
            schedule_aftergroup(command, stores, aftergroup)?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::ExtraRightBrace { forgotten: None } => {
            // TeX82 §1068's `bottom_level` arm of `handle_right_brace`.
            let context = command.state.output_open_context(&stores.command_context());
            crate::error_report::report_error(
                stores,
                "Too many }'s",
                &[
                    "You've closed more groups than you opened.",
                    "Such booboos are generally harmless, so keep going.",
                ],
                context,
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::ExtraRightBrace {
            forgotten: Some(forgotten),
        } => {
            // TeX82 §1069's `extra_right_brace` reports and discards the
            // mismatched brace. It does not `unsave` the group it names.
            let context = command.state.output_open_context(&stores.command_context());
            let mut report = stores.print_err("Extra }, or forgotten ");
            forgotten.print(&mut report);
            report.help(&[
                "I've deleted a group-closing symbol because it seems to be",
                "spurious, as in `$x}$'. But perhaps the } is legitimate and",
                "you forgot something else, as in `\\hbox{$x}'. In such cases",
                "the way to recover is to insert both the forgotten and the",
                "deleted material, e.g., by typing `I$}'.",
            ]);
            report.context(context);
            report.error().jump_out()?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::OffSave(closer) => {
            // `scan_off_save` already ran the input recovery (backing up the
            // command behind its chosen closer); this only prints TeX82
            // §1064's report naming what §1065 inserted.
            let context = command.state.output_open_context(&stores.command_context());
            let mut report = stores.print_err("Missing ");
            closer.print(&mut report);
            report
                .print(" inserted")
                .help(&OFF_SAVE_HELP)
                .context(context);
            report.error().jump_out()?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::OffSaveBottomDrop { token } => {
            // TeX82 §1066: "print_err("Extra "); print_cmd_chr(cur_cmd,
            // cur_chr)". `scan_off_save` already dropped the command itself
            // (no backup, nothing to replay); this only names it.
            let name = tex_command::command_token_text(&mut stores.command_context(), token);
            let context = command.state.output_open_context(&stores.command_context());
            crate::error_report::report_error(
                stores,
                &format!("Extra {name}"),
                &["Things are pretty mixed up, but I think the worst is over."],
                context,
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::EndOrdinaryGroup => {
            crate::assignments::flush_pending_hchars_with_fuel(modes, stores, command.fuel)?;
            let aftergroup = stores
                .leave_group_with_kind(GroupKind::Simple)
                .map_err(|_| ExecError::MissingToken {
                    context: "ordinary simple group",
                })?;
            schedule_aftergroup(command, stores, aftergroup)?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::EndMathGroup(kind) => {
            // TeX82 §1186 and §1174's `build_choices` both open with
            // `unsave`. Everything after it -- popping `saved`, `fin_mlist`,
            // and storing the result in the field or branch -- belongs to the
            // scanner that opened the group, so `execute_live_math_group`
            // performs it once its level is gone.
            let aftergroup =
                stores
                    .leave_group_with_kind(kind)
                    .map_err(|_| ExecError::MissingToken {
                        context: "math group",
                    })?;
            schedule_aftergroup(command, stores, aftergroup)?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::AlignmentRecovery { opens_simple_group } => {
            boxes.recovery_simple_group_pending = opens_simple_group;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BoxEndGroup { ships_out } => {
            let box_state = boxes.active_boxes.pop().ok_or(ExecError::MissingToken {
                context: "box group",
            })?;
            if let ReplayBoxKind::Insert(class, pre) = box_state.kind {
                return finish_insert_or_adjust_group(class, pre, modes, stores, command);
            }
            // TeX82 §1085's `handle_right_brace` runs `end_graf` (§1096) for
            // `vbox_group` and `vtop_group` -- and only for those two -- before
            // `package`: `hbox_group` and `adjusted_hbox_group` package
            // immediately. A vertical box whose body still has a paragraph open
            // when its closing brace arrives must therefore line-break that
            // paragraph into the box's own vertical list first. Without this,
            // `modes.pop()` below took the still-open *horizontal* level for the
            // box body and packaged its hlist material directly, so
            // `\vbox{\noindent A}` produced `\vbox(0.0+0.0)x0.0` holding a bare
            // char node -- and left the box's real internal-vertical level open
            // on the mode nest (`umber2-johp.232`).
            if !box_state.kind.horizontal() {
                crate::assignments::end_paragraph_with_fuel(modes, stores, command.fuel)?;
            }
            // TeX82's main-control loop appends every character (and its
            // resolved ligature/kern chain) to the current list synchronously
            // as it is scanned, so by the time `handle_right_brace` reaches
            // `package` (§1086) to `hpack`/`vpack` a finished box, the list
            // is already complete. Umber batches a run of pending horizontal
            // characters (for ligature/kerning/shaping) in
            // `ModeList::pending_hchars` rather than materializing nodes
            // immediately, so any box-body list a `}` is about to freeze must
            // first flush that batch -- exactly like every other site that
            // treats a list as finished (`execute_discretionary_part`,
            // `capture_replay_alignment_cell`, `finish_replay_alignment_row`).
            // Without this, a box whose body ends in a bare character run
            // with no trailing glue/kern/space to force an earlier flush
            // (e.g. `\hbox{c}`, or plain.tex's `\setbox\z@\hbox{#1}` inside
            // `\c`) silently packages an empty list: the pending characters
            // are dropped along with the popped mode level instead of ever
            // becoming node.

            let level = crate::assignments::commit_current_list(modes, stores, command.fuel)?;
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
                    ReplayBoxKind::VCenter => {
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
                    ReplayBoxKind::Insert(_, _) => {
                        unreachable!(
                            "insert/adjust bodies return through finish_insert_or_adjust_group above"
                        )
                    }
                })
            };
            let boxed = stores.freeze_node_list(std::slice::from_ref(&node));
            stores
                .leave_group_with_kind(box_state.group_kind)
                .map_err(|_| ExecError::MissingToken {
                    context: "box group",
                })?;
            // TeX82 §1168's `vcenter_group` case of `handle_right_brace`:
            //
            //     vcenter_group: begin end_graf; unsave; save_ptr:=save_ptr-2;
            //       p:=vpack(link(head),saved(1),saved(0)); pop_nest;
            //       tail_append(new_noad); type(tail):=vcenter_noad;
            //       math_type(nucleus(tail)):=sub_box; info(nucleus(tail)):=p;
            //       end;
            //
            // The packaged box becomes a `vcenter_noad` nucleus on the
            // enclosing mlist. It never reaches §1075's `box_end`: §1073's
            // `scan_box` admits only `cur_cmd=make_box`, so a `\vcenter` can
            // be neither a `\setbox` target, a `\shipout` operand, a leader
            // payload, nor a `\raise`/`\lower` operand, and the whole box
            // context every other branch below classifies is inapplicable.
            if box_state.kind == ReplayBoxKind::VCenter {
                modes
                    .current_list_mutation()
                    .push(Node::MathNoad(MathNoad::new(
                        NoadKind::VCenter,
                        MathField::SubBox(boxed),
                    )));
                return Ok(ReplayStep::Continue);
            }
            if let Some(kind) = box_state.leader_kind {
                let payload = payload_from_node(node).ok_or(ExecError::MissingToken {
                    context: "leader box payload",
                })?;
                boxes.pending_leader = Some((kind, payload));
            } else if ships_out {
                debug_assert!(box_state.ships_out);
                if let Some(receipt) = shipout_replay_box(node, stores, command)? {
                    push_prepared_dvi_page(prepared_dvi_pages, receipt);
                }
            } else if let Some(target) = box_state.target {
                if target.global {
                    stores.set_box_reg_global(target.index, boxed);
                } else {
                    stores.set_box_reg(target.index, boxed);
                }
            } else {
                // TeX82 §1076's `box_end` branch for an ordinary
                // (non-register, non-shipout, non-leader) box appends the
                // freshly built box to whatever list is currently open,
                // exactly like `\box<n>` (`box_end`'s `BoxContext::Append`
                // above): baseline-skip insertion, migration extraction, and
                // (in outer vertical mode) page-builder contribution all
                // apply. A bare `modes.current_list_mutation().push(node)` here
                // bypassed all of that, silently dropping every standalone
                // `\hbox`/`\vbox`/`\vtop` (and macros built on them, such as
                // plain.tex's `\centerline`) appended directly in vertical
                // mode: the node landed in the mode-nest list rather than the
                // page contribution list the page builder actually drains.
                //
                // TeX82 §1073's box-shift prefixes (`\raise`/`\lower`/
                // `\moveleft`/`\moveright`) reach exactly this branch: their
                // wrapped `\hbox`/`\vbox`/`\vtop` can never itself be a
                // `\setbox` target, a `\shipout` operand, or a leader payload
                // (`scan_box`'s `cur_cmd=make_box` requirement excludes
                // `vmove`/`hmove`), so `box_state.shift` is only ever set
                // here.
                let mut node = node;
                if let Some(delta) = box_state.shift {
                    crate::assignments::apply_box_shift_delta(&mut node, delta)?;
                }
                crate::assignments::append_box_node_to_current_list(
                    modes,
                    stores,
                    node,
                    command.fuel,
                )?;
                crate::vertical::build_page_if_outer_vertical(modes, stores)?;
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BeginAlignment { vertical } => {
            // TeX82 §774's display-math entry accepts an alignment only when
            // the current formula is empty. `flush_math` owns both the
            // material and an incomplete fraction before `push_nest` opens
            // the alignment list; retaining either here makes §812's display
            // alignment handoff collide with pre-alignment math material.
            if modes.current_mode() == Mode::DisplayMath {
                let has_formula = !modes.current_list().nodes().is_empty()
                    || modes.current_list().incomplete_fraction().is_some();
                if has_formula {
                    let primitive = if vertical { "\\valign" } else { "\\halign" };
                    let context = command.state.output_open_context(&stores.command_context());
                    let mut report = stores.print_err(&format!("Improper {primitive} inside $$'s"));
                    report.help(&[
                        "Displays can use special alignments (like \\eqalignno)",
                        "only if nothing but the alignment itself is between $$'s.",
                        "So I've deleted the formulas that preceded this alignment.",
                    ]);
                    report.context(context);
                    report.error().jump_out()?;
                    let mut list = modes.current_list_mutation();
                    list.take_nodes();
                    list.take_incomplete_fraction();
                }
            }
            if let Some(outer) = active_alignment.take() {
                command
                    .state
                    .apply_alignment_request(AlignmentRequest::Suspend(outer.identity))
                    .map_err(|_| ExecError::MissingToken {
                        context: "nested alignment suspension",
                    })?;
                boxes.suspended_alignments.push(outer);
            }
            let identity = AlignmentIdentity::new(*next_alignment_identity);
            *next_alignment_identity = next_alignment_identity.wrapping_add(1);
            command
                .state
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
                packing: AlignmentPackSpec::Natural,
                columns: Vec::new(),
                repeat_start: None,
                column: 0,
                preamble_opening_pending: true,
                preamble_start_pending: false,
                cell_opening_pending: false,
                next_cell_opening_pending: false,
                align_peek_pending: false,
                align_peek_after_noalign: false,
                noalign_open: false,
                captured_rows: Vec::new(),
                tabskips: vec![stores.glue_param(GlueParam::TAB_SKIP)],
                default_tabskip: stores.glue_param(GlueParam::TAB_SKIP),
                row_migrations: Vec::new(),
                cell_span: 1,
                row_open: false,
                cell_open: false,
            });
            // TeX82 §774's `init_align` runs `push_nest` and then only
            // *negates* an ordinary vertical mode, so the alignment's own
            // list inherits that list's `aux` (`prev_depth`). Display math is
            // the deliberate exception: its `aux` is `incompleat_noad`, so
            // §774 reaches through it to `nest[nest_ptr-2].aux_field.sc`, the
            // enclosing vertical list's `prev_depth`. Umber's independent
            // mode levels do not copy either value on push, so select the
            // canonical source explicitly before opening the alignment.
            let enclosing_prev_depth = if modes.current_mode() == Mode::DisplayMath {
                modes.enclosing_vertical_prev_depth()
            } else {
                modes.current_list().prev_depth()
            };
            modes.push(replay_alignment_mode(if vertical {
                AlignmentKind::VAlign
            } else {
                AlignmentKind::HAlign
            }))?;
            if let Some(prev_depth) = enclosing_prev_depth {
                modes.current_list_mutation().set_prev_depth(prev_depth);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::AlignmentPreambleOpening { alignment, packing } => {
            command
                .state
                .apply_alignment_request(AlignmentRequest::Preamble(alignment))
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment preamble lifecycle",
                })?;
            if let Some(active) = active_alignment.as_mut()
                && active.identity == alignment
            {
                active.packing = alignment_pack_spec(packing);
                active.preamble_opening_pending = false;
                active.preamble_start_pending = true;
            }
            // TeX82 §774's `init_align` reaches the preamble through §645's
            // `scan_spec(align_group,false)`, whose `new_save_level(c)` opens
            // the save level that brackets the alignment as a whole. §800's
            // `fin_align` removes it with the second of its two `unsave`s.
            enter_canonical_group(stores, command.state, GroupKind::Align);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::AlignmentPreambleStart { alignment } => {
            let preamble = command
                .state
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
                active.tabskips = preamble
                    .tabskips
                    .into_iter()
                    .map(|spec| stores.intern_glue(spec))
                    .collect();
                active.default_tabskip = stores.intern_glue(preamble.default_tabskip);
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
            // TeX82 §774 closes `init_align` with a second
            // `new_save_level(align_group)`, the level that brackets one
            // alignment *entry*. §791's `fin_col` replaces it at every `&`,
            // `\\span`-free column end, and `\\cr`, so an assignment made in a
            // cell -- `\\bf`, `\\tt`, a `\\fam`, any local register -- is
            // restored before the next entry begins. §800's first `unsave`
            // removes the last one.
            enter_canonical_group(stores, command.state, GroupKind::Align);
            // §774 then runs
            // `if every_cr<>null then begin_token_list(every_cr,every_cr_text)`
            // before its own `align_peek`, exactly as §799 does at every later
            // row boundary. The push follows the entry save level, so the hook
            // is scoped to the entry it opens.
            schedule_everycr(command.state, stores);
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
            active.noalign_open = true;
            enter_canonical_group(stores, command.state, GroupKind::NoAlign);
            // TeX82 §785 leaves the alignment's own mode level in place when
            // `\noalign` opens. It calls `normal_paragraph` only for an
            // h-alignment's internal-vertical mode; a v-alignment is already
            // in restricted horizontal mode, but that level is the alignment
            // list itself, not a paragraph to pop.
            if modes.current_mode() == Mode::InternalVertical {
                crate::assignments::normal_paragraph(modes, stores);
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
                .state
                .apply_alignment_request(AlignmentRequest::BeginCell {
                    alignment,
                    templates,
                })
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment next-row lifecycle",
                })?;
            begin_replay_alignment_cell(active, modes, stores)?;
            active.align_peek_pending = false;
            if omit {
                command
                    .state
                    .apply_alignment_request(AlignmentRequest::PrepareCellLookahead(alignment))
                    .map_err(|_| ExecError::MissingToken {
                        context: "alignment omit lookahead lifecycle",
                    })?;
                command
                    .state
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
                    .state
                    .apply_alignment_request(AlignmentRequest::InstallCellTemplate(alignment))
                    .map_err(|_| ExecError::MissingToken {
                        context: "alignment next-row cell-template lifecycle",
                    })?;
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::NoAlignEndGroup { alignment } => {
            let active = active_alignment
                .as_mut()
                .filter(|active| active.identity == alignment)
                .ok_or(ExecError::MissingToken {
                    context: "active replay alignment",
                })?;
            if !active.noalign_open {
                return Err(ExecError::MissingToken {
                    context: "noalign group",
                });
            }
            active.noalign_open = false;
            active.align_peek_pending = true;
            active.align_peek_after_noalign = true;
            // TeX82 §1133's whole `no_align_group` case of `handle_right_brace`
            // is `end_graf; unsave; align_peek`. A `\noalign` body is ordinary
            // internal vertical material, so anything horizontal in it (a
            // character, an `\hskip`, an `\indent`) starts a paragraph through
            // §1090 exactly as it would anywhere else in vertical mode, and the
            // closing brace is what line-breaks it back onto the alignment's own
            // vertical list. Without `end_graf` the paragraph stayed open across
            // the brace, so the following rows were built on the horizontal
            // level and `fin_align` popped that level instead of the alignment
            // (`umber2-usol`).
            crate::assignments::end_paragraph_with_fuel(modes, stores, command.fuel)?;
            stores
                .leave_group_with_kind(GroupKind::NoAlign)
                .map_err(|_| ExecError::MissingToken {
                    context: "noalign group",
                })?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::AlignmentCellOpening { alignment, opening } => {
            command
                .state
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
                .state
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
            // TeX82 §800's `fin_align` opens with two `unsave`s -- "that
            // |align_group| was for individual entries", then "that
            // |align_group| was for the whole alignment" -- before it
            // determines the column widths and packages the prototype box.
            let entry_aftergroup = leave_fin_align_save_level(stores, "align1")?;
            let alignment_aftergroup = leave_fin_align_save_level(stores, "align0")?;
            let active = active_alignment
                .as_mut()
                .expect("active replay alignment was checked");
            finish_replay_alignment(active, modes, stores, command.fuel)?;
            schedule_aftergroup(command, stores, entry_aftergroup)?;
            schedule_aftergroup(command, stores, alignment_aftergroup)?;
            command
                .state
                .apply_alignment_request(AlignmentRequest::Finish(alignment))
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment finish lifecycle",
                })?;
            *active_alignment = None;
            if let Some(outer) = boxes.suspended_alignments.pop() {
                command
                    .state
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
                crate::assignments::end_paragraph_with_consumer_and_fuel(
                    modes,
                    stores,
                    &mut memo,
                    command.fuel,
                )?;
            }
            Ok(ReplayStep::Continue)
        }
        // TeX82 §1137 and §1193 need the mode nest, the save stack, and the
        // command processor's token-list scheduling together, so
        // `CanonicalMainControl::apply_host_owned_step` applies this step for
        // every delivery entry point before `apply_scanned_step` runs.
        ScannedStep::MathShift { .. } => {
            unreachable!("apply_host_owned_step applies canonical math shifts")
        }
        ScannedStep::ParagraphStart => {
            start_canonical_paragraph(command.state, modes, stores, true)?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Character {
            ch,
            cat,
            origin,
            suppress_left_boundary,
        } => {
            if matches!(modes.current_mode(), Mode::Math | Mode::DisplayMath) {
                if !matches!(cat, Catcode::Space) {
                    // TeX82 §1154's `mmode+letter,mmode+other_char:
                    // set_math_char(ho(math_code(cur_chr)))`.
                    set_canonical_math_char(ch, origin, stores, modes, command)?;
                }
                return Ok(ReplayStep::Continue);
            }
            match cat {
                // TeX82 §1045's `any_mode(relax),vmode+spacer,mmode+spacer,
                // mmode+no_boundary:do_nothing` leaves vertical mode
                // untouched by an ordinary space; only `start_par`, a
                // letter/other/char_num/char_given, or an explicit
                // box/rule/etc. triggers `new_graf` via §1090's
                // `back_input; new_graf(true)`. A space therefore never
                // itself opens a paragraph here.
                Catcode::Space => {
                    if matches!(
                        modes.current_mode(),
                        Mode::Horizontal | Mode::RestrictedHorizontal
                    ) {
                        crate::assignments::append_canonical_space_with_fuel(
                            modes,
                            stores,
                            command.fuel,
                        )?;
                    }
                }
                Catcode::Letter | Catcode::Other => {
                    if matches!(
                        modes.current_mode(),
                        Mode::Vertical | Mode::InternalVertical
                    ) {
                        start_canonical_paragraph(command.state, modes, stores, true)?;
                    }
                    modes
                        .current_list_mutation()
                        .set_no_boundary(suppress_left_boundary);
                    crate::assignments::append_canonical_character_with_fuel(
                        modes,
                        stores,
                        ch,
                        origin,
                        command.fuel,
                    )?;
                }
                _ => unreachable!("canonical character scan restricts catcodes"),
            }
            Ok(ReplayStep::Continue)
        }
        // `step_once` consumes the command-owned episodes while its aggregate
        // snapshot is live. Observed replay is not an alternate production
        // execution path, so reaching these arms is an invariant.
        ScannedStep::DiscretionaryOpening(_) | ScannedStep::DiscretionaryPartEnd => {
            unreachable!("discretionary is applied by CanonicalMainControl")
        }
        ScannedStep::DiscretionaryHyphen { .. } => {
            unreachable!("discretionary hyphen is applied by CanonicalMainControl")
        }
        ScannedStep::Accent(_) => {
            unreachable!("accent is applied by CanonicalMainControl")
        }
    }
}

fn print_display_content(stores: &mut Universe, content: &str) {
    stores.printer().print_nl("").print_rendered(content);
}

/// TeX82 §282's `insert_token` arm, the only way an `\aftergroup` token ever
/// re-enters the input, plus e-TeX 2.6 etex.ch [15.282]'s optimized form.
///
/// §282 is `unsave`'s `@<Clear off top level from |save_stack|@>`: it walks
/// the level downwards and, for every `insert_token` entry, runs
/// §326 `@<Insert token |p| into \TeX's input@>`. TeX82 applies one full
/// `back_input` per token. In extended mode e-TeX applies that full operation
/// only to the first token and links every remaining token directly onto the
/// resulting `backed_up` list.
///
/// Because §282 clears the level from the top down while `\aftergroup` saved
/// from the bottom up, the last-saved token is backed up first and ends up
/// deepest, so rereading restores save order. `Universe` hands the payload
/// over in save order, so backing it up in reverse reproduces both the input
/// structure and the order `unsave` observes it in.
fn schedule_aftergroup(
    command: &mut CommandMachine<'_>,
    stores: &mut Universe,
    tokens: Vec<Token>,
) -> Result<(), ExecError> {
    if tokens.is_empty() {
        return Ok(());
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
        .collect::<Vec<_>>();
    command
        .processor(stores)
        .back_input_aftergroup_tokens(traced)
        .map_err(command_error)
}

/// Releases the single pending after-assignment token only after the typed
/// assignment has committed. TeX82 §1269 assigns it to `cur_tok` and invokes
/// §325 `back_input`, so it must use the ordinary canonical backup level.
fn schedule_afterassignment(
    command: &mut CommandState,
    runtime: &mut CommandRuntime,
    fuel: &mut tex_command::CommandFuel,
    capabilities: &mut CommandHostCapabilities,
    observations: &mut ObservationSlot,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    let Some(token) = stores.take_afterassignment() else {
        return Ok(());
    };
    let origin = stores.inserted_origin(
        tex_state::provenance::InsertedOriginKind::AfterAssignment,
        token,
        tex_state::token::OriginId::UNKNOWN,
    );
    let mut processor =
        command_processor(command, runtime, fuel, capabilities, observations, stores);
    let result = processor.back_input_token(tex_state::token::TracedTokenWord::pack(token, origin));
    result.map_err(command_error)
}

/// Applies TeX82 §1214's `\globaldefs` override to a prefixed assignment's
/// scope: a positive `\globaldefs` forces `global_defs`, a negative one forces
/// local scope, and zero leaves the `\global` prefix in charge.
fn effective_global(global_defs: i32, explicit_global: bool) -> bool {
    match global_defs.cmp(&0) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => explicit_global,
    }
}

/// Whether e-TeX §275's local `eq_word_define` returns as a reassignment.
///
/// Global definitions always execute, and TeX82 does not have the extended
/// mode shortcut. Keeping this predicate beside assignment application makes
/// the save-stack and observation decisions share one canonical condition.
fn etex_redundant_local_word_assignment<T: Eq>(
    stores: &Universe,
    current: T,
    replacement: T,
) -> bool {
    stores.int_param(IntParam::ETEX_EXTENDED_MODE) > 0 && current == replacement
}

/// Whether e-TeX §§277-278 return before an observed assignment step touches
/// either the save stack or its `eqtb` location.
fn etex_redundant_local_definition_step(stores: &Universe, scanned: &ScannedStep) -> bool {
    match scanned {
        ScannedStep::Let {
            target,
            meaning,
            global: false,
            ..
        } => etex_redundant_local_word_assignment(stores, stores.meaning(*target), *meaning),
        ScannedStep::Count {
            index,
            value,
            global: false,
        } => etex_redundant_local_word_assignment(stores, stores.count(*index), *value),
        ScannedStep::Dimen {
            index,
            value,
            global: false,
        } => etex_redundant_local_word_assignment(stores, stores.dimen(*index), *value),
        ScannedStep::IntParam {
            index,
            value,
            global: false,
        } => etex_redundant_local_word_assignment(
            stores,
            stores.int_param(IntParam::new(*index)),
            *value,
        ),
        ScannedStep::DimenParam {
            index,
            value,
            global: false,
        } => etex_redundant_local_word_assignment(
            stores,
            stores.dimen_param(DimenParam::new(*index)),
            *value,
        ),
        ScannedStep::GlueParam {
            index,
            value,
            global: false,
        } => etex_redundant_local_zero_glue_assignment(
            stores,
            stores.glue_param(GlueParam::new(*index)),
            value,
        ),
        ScannedStep::CodeTable {
            primitive,
            character,
            value,
            global: false,
        } => match primitive {
            UnexpandablePrimitive::CatCode => match *value {
                0 => Some(Catcode::Escape),
                1 => Some(Catcode::BeginGroup),
                2 => Some(Catcode::EndGroup),
                3 => Some(Catcode::MathShift),
                4 => Some(Catcode::AlignmentTab),
                5 => Some(Catcode::EndLine),
                6 => Some(Catcode::Parameter),
                7 => Some(Catcode::Superscript),
                8 => Some(Catcode::Subscript),
                9 => Some(Catcode::Ignored),
                10 => Some(Catcode::Space),
                11 => Some(Catcode::Letter),
                12 => Some(Catcode::Other),
                13 => Some(Catcode::Active),
                14 => Some(Catcode::Comment),
                15 => Some(Catcode::Invalid),
                _ => None,
            }
            .is_some_and(|value| {
                etex_redundant_local_word_assignment(stores, stores.catcode(*character), value)
            }),
            UnexpandablePrimitive::LcCode => u32::try_from(*value).is_ok_and(|value| {
                etex_redundant_local_word_assignment(stores, stores.lccode(*character), value)
            }),
            UnexpandablePrimitive::UcCode => u32::try_from(*value).is_ok_and(|value| {
                etex_redundant_local_word_assignment(stores, stores.uccode(*character), value)
            }),
            UnexpandablePrimitive::SfCode => u16::try_from(*value).is_ok_and(|value| {
                etex_redundant_local_word_assignment(stores, stores.sfcode(*character), value)
            }),
            UnexpandablePrimitive::MathCode => u32::try_from(*value).is_ok_and(|value| {
                etex_redundant_local_word_assignment(stores, stores.mathcode(*character), value)
            }),
            UnexpandablePrimitive::DelCode => {
                etex_redundant_local_word_assignment(stores, stores.delcode(*character), *value)
            }
            _ => unreachable!("only code-table primitives are scanned"),
        },
        _ => false,
    }
}

/// Whether e-TeX §277 sees the same canonical zero-glue pointer on both sides
/// of a local `eq_define`.
///
/// TeX82 §1237's `trap_zero_glue` replaces every all-zero scanned
/// specification with `zero_glue` before `define`. Equal nonzero glue values
/// are deliberately not reassignment-identical: separately scanned literals
/// occupy different glue-spec nodes in TeX even though Umber hash-conses their
/// immutable contents.
fn etex_redundant_local_zero_glue_assignment(
    stores: &Universe,
    current: GlueId,
    replacement: &GlueSpec,
) -> bool {
    stores.int_param(IntParam::ETEX_EXTENDED_MODE) > 0
        && current == GlueId::ZERO
        && *replacement == GlueSpec::ZERO
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
    mut left_order: Order,
    right: Scaled,
    mut right_order: Order,
) -> Result<(Scaled, Order), ExecError> {
    // TeX82 §1238 first normalizes a zero component on the newly scanned
    // specification before comparing its order, and only lets the stored
    // component replace it when that stored component is nonzero. Normalizing
    // both operands expresses the same value-based rule without depending on
    // which side happened to be scanned: a zero `fill` must never erase a
    // nonzero `fil` component during `\advance`.
    if left.raw() == 0 {
        left_order = Order::Normal;
    }
    if right.raw() == 0 {
        right_order = Order::Normal;
    }
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

/// Replays TeX82's distinct vertical and horizontal rule paths.
///
/// TeX82 §1095 routes `\hrule` through `head_for_vmode`, so an ordinary
/// horizontal paragraph must finish before its rule reaches the page builder.
/// In vertical mode the rule is a direct contribution and resets `prev_depth`;
/// `\vrule`, conversely, enters horizontal mode before it appends its node.
///
/// `\hrule` in math mode never reaches this function: `scan_command`
/// intercepts `mmode+hrule` before scanning a rule spec at all (TeX82 §1046)
/// and replays §1047's `insert_dollar_sign` instead. `\vrule` in math mode
/// (§1056's `mmode+vrule`) is an ordinary direct contribution and falls
/// through the `else` branch below like any other mode.
/// Applies a scanned TeX82 §1073 box-shift prefix (`\raise`, `\lower`,
/// `\moveleft`, `\moveright`). `ScannedBoxShiftPayload::Construction` opens
/// the same `BoxEndGroup` body-closing episode as an
/// ordinary `\hbox`/`\vbox`/`\vtop` (`BeginBox`/`BeginLeaderBox`'s twin),
/// deferring the shift until `BoxEndGroup` packages the body; every other
/// payload resolves to a node immediately and is shifted and appended right
/// here, exactly like `\box<n>`, `\lastbox`, and `\vsplit` do outside a
/// shift.
fn apply_box_shift(
    shift: ScannedBoxShift,
    command: &mut CommandState,
    modes: &mut ModeNest,
    stores: &mut Universe,
    boxes: &mut ReplayBoxes,
    fuel: &mut tex_command::CommandFuel,
) -> Result<ReplayStep, ExecError> {
    match shift.payload {
        ScannedBoxShiftPayload::Missing => {
            // `scan_box`'s own "A <box> was supposed to be here" recovery
            // (tex.web §1084); the rejected command has already been backed
            // up by `scan_box_shift_payload` for ordinary replay.
            report_missing_box(command, stores)?;
            Ok(ReplayStep::Continue)
        }
        ScannedBoxShiftPayload::BoxRegister { index, copy } => {
            let id = if copy {
                stores.box_reg(index)
            } else {
                stores.take_box_reg_same_level(index)
            };
            if copy && let Some(id) = id {
                stores.pin_survivor(id);
            }
            let node = crate::assignments::first_box_node(stores, id);
            append_shifted_box(modes, stores, node, shift.delta, fuel)?;
            Ok(ReplayStep::Continue)
        }
        ScannedBoxShiftPayload::LastBox => {
            let node = crate::assignments::take_last_box(modes, stores, fuel)?;
            append_shifted_box(modes, stores, node, shift.delta, fuel)?;
            Ok(ReplayStep::Continue)
        }
        ScannedBoxShiftPayload::VSplit(split) => {
            if split.missing_to {
                report_missing_vsplit_to(command, stores)?;
            }
            let node = crate::assignments::split_vbox_register(stores, split.index, split.height)?;
            append_shifted_box(modes, stores, node, shift.delta, fuel)?;
            Ok(ReplayStep::Continue)
        }
        ScannedBoxShiftPayload::Construction(construction) => {
            let kind = ReplayBoxKind::from_scanned(construction.kind);
            let packing = match construction.packing {
                ScannedPackingSpec::Natural => PackSpec::Natural,
                ScannedPackingSpec::Exactly(size) => PackSpec::Exactly(size),
                ScannedPackingSpec::Spread(size) => PackSpec::Spread(size),
            };
            let group_kind = if kind == ReplayBoxKind::HBox {
                GroupKind::AdjustedHBox
            } else {
                kind.group_kind()
            };
            enter_canonical_group(stores, command, group_kind);
            modes.push(if kind.horizontal() {
                Mode::RestrictedHorizontal
            } else {
                Mode::InternalVertical
            })?;
            if !kind.horizontal() {
                crate::assignments::normal_paragraph(modes, stores);
            }
            boxes.active_boxes.push(ActiveReplayBox {
                target: None,
                ships_out: false,
                kind,
                group_kind,
                packing,
                leader_kind: None,
                shift: Some(shift.delta),
            });
            schedule_everybox(command, stores, kind.horizontal());
            Ok(ReplayStep::Continue)
        }
    }
}

/// TeX82 §1071's `box_context`, for the box constructions §1079's `begin_box`
/// resolves to a `cur_box` immediately -- `box_code`, `copy_code`,
/// `last_box_code`, and `vsplit_code`, which all fall through to the shared
/// `box_end(box_context)` call at the end of `begin_box`.
///
/// tex.web encodes the context as one integer and lets `box_end` classify it
/// (`box_context<box_flag`, `<ship_out_flag`, `=ship_out_flag`, or greater).
/// Enumerating it here keeps that single classification, so no producer can
/// silently implement only part of the context space: before this existed,
/// `\box`/`\copy` and `\lastbox` recognized only the append and `\shipout`
/// contexts and dropped `\setbox`'s entirely, leaving `\setbox0\lastbox`
/// re-appending its box and voiding the destination register
/// (`umber2-johp.263`).
///
/// The leader context (`box_context>ship_out_flag`, §1078) is not represented:
/// §1078 has to scan the *following* glue command before it can build its
/// node, so the command scanner resolves leader payloads as their own
/// `ScannedStep`s with the glue already attached.
#[derive(Clone, Copy, Debug)]
enum BoxContext {
    /// `box_context<box_flag`: §1076's "Append box `cur_box` to the current
    /// list, shifted by `box_context`". The plain append is a zero shift.
    Append(Scaled),
    /// `box_flag<=box_context<ship_out_flag`: §1077's "Store `cur_box` in a
    /// box register", `eq_define`/`geq_define` by `\setbox`/`\global\setbox`.
    SetBox(SetBoxTarget),
    /// `box_context=ship_out_flag`: §1075's `ship_out(cur_box)`.
    ShipOut,
}

impl ReplayBoxes {
    /// Resolves the pending `box_context` for a box that reaches `box_end`
    /// immediately, consuming it exactly like tex.web's single-use integer.
    ///
    /// `\shipout` and `\setbox` cannot both be pending on well-formed input:
    /// §1084's `scan_box` accepts only a `make_box` command after `\setbox`,
    /// and `\shipout` is `leader_ship`, so `\setbox0\shipout...` never gets
    /// past the "A <box> was supposed to be here" recovery. The pending
    /// `\setbox` target is still consumed either way so a recovered input
    /// cannot leave it to capture an unrelated later box.
    fn take_box_context(&mut self, ships_out: bool) -> BoxContext {
        let target = self.pending_setbox.take();
        if ships_out {
            self.pending_shipout = false;
            return BoxContext::ShipOut;
        }
        match target {
            Some(target) => BoxContext::SetBox(target),
            None => BoxContext::Append(Scaled::from_raw(0)),
        }
    }
}

/// TeX82 §1075's `box_end`: the one place a resolved `cur_box` is disposed of
/// according to its context. `\hbox`/`\vbox`/`\vtop` bodies reach the same
/// three dispositions through `BoxEndGroup`, which cannot share this entry
/// point because §1083 defers them to their group's closing brace.
fn box_end(
    context: BoxContext,
    node: Option<Node>,
    modes: &mut ModeNest,
    stores: &mut Universe,
    prepared_dvi_pages: &mut PreparedDviPages,
    command: &mut CommandMachine<'_>,
) -> Result<(), ExecError> {
    match context {
        BoxContext::Append(delta) => append_shifted_box(modes, stores, node, delta, command.fuel),
        // §1077 defines the register unconditionally: a void `cur_box` makes
        // the destination void, it does not leave the old value in place.
        BoxContext::SetBox(target) => {
            match node {
                Some(node) => {
                    let boxed = stores.freeze_node_list(std::slice::from_ref(&node));
                    if target.global {
                        stores.set_box_reg_global(target.index, boxed);
                    } else {
                        stores.set_box_reg(target.index, boxed);
                    }
                }
                None if target.global => stores.clear_box_reg_global(target.index),
                None => stores.clear_box_reg(target.index),
            }
            Ok(())
        }
        // §1075 guards `ship_out` with `cur_box<>null`.
        BoxContext::ShipOut => {
            if let Some(node) = node
                && let Some(receipt) = shipout_replay_box(node, stores, command)?
            {
                push_prepared_dvi_page(prepared_dvi_pages, receipt);
            }
            Ok(())
        }
    }
}

/// Applies TeX82 §1073's `shift_amount(cur_box):=box_context` to an already
/// scanned box, then appends it exactly like an ordinary standalone box
/// (`\box<n>`'s bare append, or `BoxEndGroup`'s final branch). A void box is
/// a no-op, matching `box_end`'s `if cur_box<>null` guard.
fn append_shifted_box(
    modes: &mut ModeNest,
    stores: &mut Universe,
    node: Option<Node>,
    delta: Scaled,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    let Some(mut node) = node else {
        return Ok(());
    };
    crate::assignments::apply_box_shift_delta(&mut node, delta)?;
    crate::assignments::append_box_node_to_current_list(modes, stores, node, fuel)?;
    crate::vertical::build_page_if_outer_vertical(modes, stores)
}

fn apply_scanned_rule(
    command: &mut CommandMachine<'_>,
    modes: &mut ModeNest,
    stores: &mut Universe,
    width: Option<Scaled>,
    height: Option<Scaled>,
    depth: Option<Scaled>,
    horizontal: bool,
) -> Result<ReplayStep, ExecError> {
    let node = Node::Rule {
        width,
        height,
        depth,
    };
    if horizontal {
        match modes.current_mode() {
            Mode::Vertical | Mode::InternalVertical => {}
            Mode::Horizontal => {
                let mut memo = crate::paragraph_memo::NoParagraphMemoConsumer;
                crate::assignments::end_paragraph_with_consumer_and_fuel(
                    modes,
                    stores,
                    &mut memo,
                    command.fuel,
                )?;
            }
            Mode::RestrictedHorizontal => {
                // TeX82 §1095's `head_for_vmode`: an `\hrule` in restricted
                // horizontal mode is the one command there that does not go
                // through `off_save`.
                let context = command.state.output_open_context(&stores.command_context());
                report_escaped_error(
                    stores,
                    "You can't use `",
                    "hrule",
                    "' here except with leaders",
                    &[
                        "To put a horizontal rule in an hbox or an alignment,",
                        "you should use \\leaders or \\hrulefill (see The TeXbook).",
                    ],
                    context,
                )?;
                return Ok(ReplayStep::Continue);
            }
            mode => {
                return Err(ExecError::UnimplementedTypesetting {
                    mode,
                    token: Token::Cs(stores.intern("hrule").symbol()),
                    origin: tex_state::token::OriginId::UNKNOWN,
                    operation: "\\hrule",
                });
            }
        }
        crate::vertical::append_vertical_contribution(modes, stores, node);
        modes
            .current_list_mutation()
            .set_prev_depth(crate::mode::ignored_depth(stores));
        // TeX82 §1056's `append_rule` stops after `tail_append` and resetting
        // `prev_depth` in vertical mode. Unlike §1075's box append and §1103's
        // penalty append, it deliberately does not call `build_page`; the
        // next command with an explicit page-builder tail owns that visit.
    } else {
        if matches!(
            modes.current_mode(),
            Mode::Vertical | Mode::InternalVertical
        ) {
            start_canonical_paragraph(command.state, modes, stores, true)?;
        }
        // TeX82 §1054 reaches `append_rule` only after main_control has
        // finished the current word. Materialize Umber's pending character
        // run before appending the rule so a `\vrule` cannot split a word and
        // move its final character behind the rule node.
        crate::assignments::flush_pending_hchars_with_fuel(modes, stores, command.fuel)?;
        modes.current_list_mutation().push(node);
        // TeX82 §1056 resets `space_factor` after a rule in either
        // horizontal mode. This matters when a zero-sfcode closer follows
        // the rule: it must inherit 1000, not sentence spacing from text
        // before the rule.
        if matches!(
            modes.current_mode(),
            Mode::Horizontal | Mode::RestrictedHorizontal
        ) {
            modes.current_list_mutation().set_space_factor(1000);
        }
    }
    Ok(ReplayStep::Continue)
}

/// TeX82 §1123's list-building tail, with §1125's kerns.
struct AccentPlacement {
    accent: u8,
    accent_font: tex_state::ids::FontId,
    accent_metrics: tex_state::font::CharMetrics,
    accent_origin: tex_state::token::OriginId,
    /// §1124's `q`: the base character and its origin, or `null`.
    base: Option<(u8, tex_state::token::OriginId)>,
}

/// Appends §1123's `link(tail):=p; tail:=p; space_factor:=1000`, preceded by
/// §1125's accent kerns when §1124 produced a base character.
fn apply_accent_nodes(
    modes: &mut ModeNest,
    stores: &mut Universe,
    placement: AccentPlacement,
) -> Result<ReplayStep, ExecError> {
    let AccentPlacement {
        accent,
        accent_font,
        accent_metrics,
        accent_origin,
        base,
    } = placement;
    let accent_node = Node::Char {
        font: accent_font,
        ch: char::from(accent),
        origin: accent_origin,
    };
    // §1124's `f:=cur_font` is re-read *after* `do_assignments`, so the base
    // character is set in whatever font those assignments left selected.
    let base_font = stores.current_font();
    let base = base.and_then(|(character, origin)| {
        let Some(metrics) = stores.font_char_metrics(base_font, character) else {
            report_missing_character(stores, base_font, char::from(character));
            return None;
        };
        Some((character, origin, metrics))
    });
    let Some((character, base_origin, base_metrics)) = base else {
        modes.current_list_mutation().push(accent_node);
        modes.current_list_mutation().set_space_factor(1000);
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
    modes.current_list_mutation().push(Node::Kern {
        amount: delta,
        kind: KernKind::Accent,
    });
    if base_metrics.height == accent_x_height {
        modes.current_list_mutation().push(accent_node);
    } else {
        let children = stores.freeze_node_list(&[accent_node]);
        let mut boxed =
            crate::assignments::hpack_with_overfull_rule(stores, children, PackSpec::Natural);
        boxed.shift = accent_x_height
            .checked_sub(base_metrics.height)
            .ok_or(ExecError::ArithmeticOverflow)?;
        modes.current_list_mutation().push(Node::HList(boxed));
    }
    modes.current_list_mutation().push(Node::Kern {
        amount: Scaled::from_raw(-accent_metrics.width.raw() - delta.raw()),
        kind: KernKind::Accent,
    });
    modes.current_list_mutation().push(Node::Char {
        font: base_font,
        ch: char::from(character),
        origin: base_origin,
    });
    modes.current_list_mutation().set_space_factor(1000);
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
    if stores.int_param(IntParam::TRACING_LOST_CHARS) <= 0 {
        return;
    }
    let font_name = stores.font_name(font).to_owned();
    let mut diagnostic = stores.begin_diagnostic();
    diagnostic
        .print_nl("Missing character: There is no ")
        .print_char(ch)
        .print(" in font ")
        .print(&font_name)
        .print_char('!');
    diagnostic.end(false);
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

/// Closes a `\insert<class>{...}` or `\vadjust{...}` body: TeX82 §1099/§1100's
/// shared `insert_group` case of `handle_right_brace`.
///
/// `end_graf` first finishes any paragraph left open inside the body (§1100:
/// `end_graf` runs before anything else, exactly like
/// `vbox_group`/`vtop_group`). `\splittopskip`, `\splitmaxdepth`, and
/// `\floatingpenalty` are read at their current (still-local) values before
/// `unsave` -- assignments to those parameters made inside the body govern
/// its own splitting, exactly as tex.web's `q:=split_top_skip;
/// d:=split_max_depth; f:=floating_penalty; unsave` orders it. The body is
/// then packed with TeX82's `vpack` macro (`vpackage(p,h,m,max_dimen)`):
/// unconstrained depth, but the *current* `\vbadness`/`\vfuzz` -- unlike an
/// ordinary `\vbox`, neither `\insert` nor `\vadjust` ever suppresses those
/// parameters.
///
/// §1100 then branches on `saved(0)` (`class`, here): `class<255` builds an
/// `ins_node` whose `height` field is the packed natural height+depth
/// (TeX82's `size`, consumed only by the page builder's splitting
/// arithmetic, `crate::page_builder`); `class=255` (`\vadjust`) instead
/// builds an `adjust_node` carrying only the packed content -- `q`/`d`/`f`
/// are still read above (mirroring tex.web's unconditional `q:=...; d:=...;
/// f:=...` before the branch) but never stored, matching
/// `delete_glue_ref(q)`'s discard. Either node is appended to whatever list
/// was open when `\insert`/`\vadjust` began -- the enclosing mode's list, not
/// a side channel -- exactly like `\mark` and `\penalty` above. `nest_ptr=0`
/// (`is_outer_vertical`) then invokes `build_page`, matching §1099's `if
/// nest_ptr=0 then build_page` (`\vadjust` never actually reaches this since
/// it is forbidden in outer vertical mode).
fn finish_insert_or_adjust_group(
    class: u16,
    pre: bool,
    modes: &mut ModeNest,
    stores: &mut Universe,
    command: &mut CommandMachine<'_>,
) -> Result<ReplayStep, ExecError> {
    crate::assignments::end_paragraph_with_fuel(modes, stores, command.fuel)?;
    let split_top_skip = stores.glue_param(GlueParam::SPLIT_TOP_SKIP);
    let split_max_depth = stores.dimen_param(DimenParam::SPLIT_MAX_DEPTH);
    let floating_penalty = stores.int_param(IntParam::FLOATING_PENALTY);
    stores
        .leave_group_with_kind(GroupKind::Insert)
        .map_err(|_| ExecError::MissingToken {
            context: "insert group",
        })?;
    let level = crate::assignments::commit_current_list(modes, stores, command.fuel)?;
    let content = stores.freeze_node_list(level.list().nodes());
    let params = tex_typeset::VpackParams {
        box_max_depth: Scaled::MAX_DIMEN,
        ..crate::packing_params::vpack_params(stores)
    };
    let packed = crate::packing_params::vpack(stores, content, PackSpec::Natural, params);
    crate::assignments::flush_pending_hchars_with_fuel(modes, stores, command.fuel)?;
    let node = if class == 255 {
        Node::Adjust(tex_state::node::AdjustNode { content, pre })
    } else {
        let size = packed
            .node
            .height
            .checked_add(packed.node.depth)
            .ok_or(ExecError::ArithmeticOverflow)?;
        Node::Ins {
            class,
            size,
            split_top_skip,
            split_max_depth,
            floating_penalty,
            content,
        }
    };
    crate::vertical::append_vertical_contribution(modes, stores, node);
    crate::vertical::build_page_if_outer_vertical(modes, stores)?;
    Ok(ReplayStep::Continue)
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

/// Runs TeX82 §774 `init_align`'s and §799 `fin_row`'s shared
/// `if every_cr<>null then begin_token_list(every_cr,every_cr_text)`.
///
/// Both sections push `\everycr` immediately before `align_peek`, so the hook
/// supplies the tokens that lookahead classifies -- typically plain.tex's
/// `\noalign{...}`. §785's `align_peek` itself never pushes it, and neither
/// does §1133's `no_align_group` case of `handle_right_brace`, which reaches
/// `align_peek` a second time after a `\noalign` body.
fn schedule_everycr(command: &mut CommandState, stores: &mut Universe) {
    let tokens = stores.tok_param(TokParam::EVERY_CR);
    if stores.tokens(tokens).is_empty() {
        return;
    }
    let tokens: Vec<_> = stores.tokens(tokens).to_vec();
    let traced: Vec<_> = tokens
        .into_iter()
        .map(|token| {
            let origin = stores.inserted_origin(
                tex_state::provenance::InsertedOriginKind::TokenListReplay(
                    tex_state::TokenListReplayKind::EveryCr,
                ),
                token,
                stores.bootstrap_origin(),
            );
            tex_state::token::TracedTokenWord::pack(token, origin)
        })
        .collect();
    command.push_everycr(stores.finish_traced_token_list(&traced));
}

/// Runs TeX82 §1030 `main_control`'s prologue,
/// `if every_job<>null then begin_token_list(every_job,every_job_text)`.
///
/// `\everyjob` is read once, before `big_switch` fetches anything, so the hook
/// is owned by the entry into `main_control` rather than by any command.
/// `Universe::take_pending_every_job` is the one-shot that distinguishes a job
/// started from a format image (where the parameter the format dumped is live
/// at entry) from the INITEX job that built it and from a resumed timeline
/// that already passed this point.
fn schedule_everyjob(command: &mut CommandState, stores: &mut Universe) {
    let tokens = stores.take_pending_every_job();
    if stores.tokens(tokens).is_empty() {
        return;
    }
    let tokens: Vec<_> = stores.tokens(tokens).to_vec();
    let traced: Vec<_> = tokens
        .into_iter()
        .map(|token| {
            let origin = stores.inserted_origin(
                tex_state::provenance::InsertedOriginKind::TokenListReplay(
                    tex_state::TokenListReplayKind::EveryJob,
                ),
                token,
                stores.bootstrap_origin(),
            );
            tex_state::token::TracedTokenWord::pack(token, origin)
        })
        .collect();
    command.push_everyjob(stores.finish_traced_token_list(&traced));
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

/// A tex.web recoverable-error report that scanning detects but only the
/// stomach can print.
///
/// The remaining reports here are ones whose semantic transition completes
/// before the World-facing executor sees them. A scan that can print at the
/// point of detection does -- §§433-437's range recovery moved into
/// `scan_restricted_integer` for exactly that reason -- because a queued
/// report lands after everything the rest of the step emits, including
/// §362's `)`.
#[derive(Clone, Debug, Eq, PartialEq)]
enum PendingDiagnostic {
    /// TeX82 §§299/1030's `show_cur_cmd_chr` at `reswitch`.
    CommandTrace(Mode, tex_command::PrintCommand),
    /// A command-owned diagnostic whose semantic transition completed before
    /// the World-facing executor could render it.
    Command(tex_command::CommandSemanticDiagnostic),
    /// tex.web §1212's `<Discard erroneous prefixes and return>`.
    ///
    /// The `bool` is `eTeX_ex`: etex.ch rewrites `help_line[0]` to name
    /// `\protected` alongside `\long`, `\outer`, and `\global`.
    PrefixOnNonPrefixedCommand(tex_command::PrintCommand, String, bool),
    /// tex.web §1213's `<Discard the prefixes \long and \outer if they are
    /// irrelevant>`.
    ///
    /// The `bool` is `eTeX_ex`, which here rewrites the *message* as well as
    /// the help: etex.ch prints `' or `\protected'` before `' with `'.
    IrrelevantLongOuterPrefix(tex_command::PrintCommand, String, bool),
}

/// tex.web §298's `print_cmd_chr` for the meanings the reports above name.
fn printed_command(stores: &Universe, meaning: Meaning) -> String {
    match meaning {
        Meaning::CharToken { ch, .. } => ch.to_string(),
        Meaning::CountRegister(index) => format!("\\count{index}"),
        Meaning::DimenRegister(index) => format!("\\dimen{index}"),
        Meaning::SkipRegister(index) => format!("\\skip{index}"),
        Meaning::MuskipRegister(index) => format!("\\muskip{index}"),
        Meaning::ToksRegister(index) => format!("\\toks{index}"),
        Meaning::Undefined => "undefined".into(),
        meaning => stores
            .primitive_name(meaning)
            .map_or_else(|| "\\relax".into(), |name| format!("\\{name}")),
    }
}

/// Prints each report a completed scan owes, in detection order.
fn report_pending_diagnostics(
    stores: &mut Universe,
    diagnostics: Vec<PendingDiagnostic>,
    shown_mode: &mut Option<Mode>,
) -> Result<(), ExecError> {
    for diagnostic in diagnostics {
        match diagnostic {
            PendingDiagnostic::Command(tex_command::CommandSemanticDiagnostic::Trace {
                text,
                force_newline,
            }) => {
                let mut output = stores.begin_diagnostic();
                if force_newline {
                    output.print_ln().print(&text);
                } else {
                    output.print_nl(&text);
                }
                output.end(false);
            }
            PendingDiagnostic::CommandTrace(mode, command) => {
                let command = tex_command::print_cmd_chr_text(&stores.command_context(), command);
                let mut output = stores.begin_diagnostic();
                output.print_nl("{");
                if *shown_mode != Some(mode) {
                    output.print(mode_text_for_command_trace(mode)).print(": ");
                    *shown_mode = Some(mode);
                }
                output.print(&command).print_char('}');
                output.end(false);
            }
            PendingDiagnostic::Command(
                tex_command::CommandSemanticDiagnostic::UndefinedControlSequence { context },
            ) => crate::diagnostics::report_undefined_control_sequence(stores, Some(context))?,
            PendingDiagnostic::Command(
                tex_command::CommandSemanticDiagnostic::MacroPrefixMismatch(symbol),
            ) => {
                let name = stores.resolve(symbol).to_owned();
                let kind = stores.control_sequence_kind(symbol);
                let mut report = stores.print_err("Use of ");
                report
                    .sprint_cs(kind, &name)
                    .print(" doesn't match its definition");
                report.help(&[
                    "If you say, e.g., `\\def\\a1{...}', then you must always",
                    "put `1' after `\\a', since control sequence names are",
                    "made up of letters only. The macro here has not been",
                    "followed by the required stuff, so I'm ignoring it.",
                ]);
                report.error().jump_out()?;
            }
            PendingDiagnostic::Command(tex_command::CommandSemanticDiagnostic::Recoverable {
                message,
                help,
                context,
                ..
            }) => {
                let mut report = stores.print_err(&message);
                report.help(help).context(context);
                report.error().jump_out()?;
            }
            PendingDiagnostic::Command(tex_command::CommandSemanticDiagnostic::MissingNumber {
                context,
            }) => {
                let mut report = stores.print_err("Missing number, treated as zero");
                report
                    .help(&[
                        "A number should have been here; I inserted `0'.",
                        "(If you can't figure out why I needed to see a number,",
                        "look up `weird error' in the index to The TeXbook.)",
                    ])
                    .context(context);
                report.error().jump_out()?;
            }
            PendingDiagnostic::PrefixOnNonPrefixedCommand(command, context, etex) => {
                let command = tex_command::print_cmd_chr_text(&stores.command_context(), command);
                let mut report = stores.print_err("You can't use a prefix with `");
                report.print(&command).print_char('\'');
                report.help(if etex {
                    &["I'll pretend you didn't say \\long or \\outer or \\global or \\protected."]
                } else {
                    &["I'll pretend you didn't say \\long or \\outer or \\global."]
                });
                report.context(context);
                report.error().jump_out()?;
            }
            PendingDiagnostic::IrrelevantLongOuterPrefix(command, context, etex) => {
                let command = tex_command::print_cmd_chr_text(&stores.command_context(), command);
                let mut report = stores.print_err("You can't use `");
                report.print_esc("long").print("' or `").print_esc("outer");
                if etex {
                    report.print("' or `").print_esc("protected");
                }
                report.print("' with `").print(&command).print_char('\'');
                report.help(if etex {
                    &["I'll pretend you didn't say \\long or \\outer or \\protected here."]
                } else {
                    &["I'll pretend you didn't say \\long or \\outer here."]
                });
                report.context(context);
                report.error().jump_out()?;
            }
        }
    }
    Ok(())
}

fn mode_text_for_command_trace(mode: Mode) -> &'static str {
    match mode {
        Mode::Vertical => "vertical mode",
        Mode::InternalVertical => "internal vertical mode",
        Mode::Horizontal => "horizontal mode",
        Mode::RestrictedHorizontal => "restricted horizontal mode",
        Mode::Math => "math mode",
        Mode::DisplayMath => "display math mode",
    }
}

/// Reports TeX82 §1258's and §1259's illegal font-size recoveries.
fn report_font_size_recovery(
    stores: &mut Universe,
    recovery: &tex_command::FontSizeRecovery,
) -> Result<(), ExecError> {
    match recovery {
        tex_command::FontSizeRecovery::ImproperAtSize { size, context } => {
            let mut report = stores.print_err("Improper `at' size (");
            report.print_scaled(*size).print("pt), replaced by 10pt");
            report
                .help(&[
                    "I can only handle fonts at positive sizes that are",
                    "less than 2048pt, so I've changed what you said to 10pt.",
                ])
                .context(context.clone());
            report.error().jump_out()?;
        }
        tex_command::FontSizeRecovery::IllegalMagnification { value, context } => {
            let mut report = stores.print_err("Illegal magnification has been changed to 1000");
            report
                .help(&["The magnification ratio must be between 1 and 32768."])
                .context(context.clone());
            report.int_error(*value).jump_out()?;
        }
    }
    Ok(())
}

/// TeX82 §1279's `token_show(def_ref)` into `new_string`.
fn message_text(stores: &Universe, tokens: tex_state::ids::TokenListId) -> String {
    let mut text = String::new();
    for &token in stores.tokens(tokens) {
        tex_expand::append_token_string_text(stores, token, &mut text);
    }
    crate::diagnostics::print_text_with_newlinechar(stores, &text)
}

/// TeX82 §1297's `token_show(temp_head)` through the active selector.
fn show_tokens_text(stores: &Universe, tokens: tex_state::ids::TokenListId) -> String {
    let newlinechar = u32::try_from(stores.int_param(IntParam::NEWLINE_CHAR))
        .ok()
        .filter(|&code| code <= u8::MAX.into())
        .and_then(char::from_u32);
    let mut text = String::new();
    for &token in stores.tokens(tokens) {
        tex_expand::append_token_selector_text(stores, token, newlinechar, &mut text);
    }
    text
}

/// e-TeX 2.6 `etex.ch` [17.3715--3732]'s exact `show_ifs` traversal.
/// The `### level N: ...` body only, joined by `\n` -- not etex.ch
/// [17.3720]'s leading `print_nl(""); print_ln`, which needs live column
/// state this pure builder does not have. The canonical `ScannedStep::ShowIfs`
/// site prints those two calls itself, through the open diagnostic, before
/// printing this body.
fn render_showifs(conditions: &[tex_command::ActiveCondition]) -> String {
    if conditions.is_empty() {
        return "### no active conditionals".to_owned();
    }
    let mut text = String::new();
    let mut level = conditions.len();
    for (index, condition) in conditions.iter().enumerate() {
        if index > 0 {
            text.push('\n');
        }
        text.push_str("### level ");
        text.push_str(&level.to_string());
        text.push_str(": \\");
        if condition.inverted() {
            text.push_str("unless\\");
        }
        text.push_str(condition.kind_name());
        if condition.else_branch() {
            text.push_str("\\else");
        }
        if condition.source_line() != 0 {
            text.push_str(" entered on line ");
            text.push_str(&condition.source_line().to_string());
        }
        level -= 1;
    }
    text
}

/// TeX82 §1280's `<Print string s on the terminal>`.
fn issue_terminal_message(stores: &mut Universe, text: &str) {
    let mut printer = stores.printer();
    if printer.terminal_offset() + text.chars().count()
        > tex_state::print::MAX_PRINT_LINE.saturating_sub(2)
    {
        printer.print_ln();
    } else if printer.terminal_offset() > 0 || printer.log_offset() > 0 {
        printer.print_char(' ');
    }
    printer.print_rendered(text);
}

/// TeX82 §1283's `<Print string s as an error message>`.
fn issue_error_message(
    stores: &mut Universe,
    text: &str,
    context: String,
) -> Result<(), ExecError> {
    let err_help = stores.tok_param(TokParam::ERR_HELP);
    let rendered = (!stores.tokens(err_help).is_empty()).then(|| message_text(stores, err_help));
    let interactive = stores.interaction_mode() == tex_state::InteractionMode::ErrorStop;
    let long_help_seen = stores
        .world_mut()
        .error_channel_mut()
        .take_long_help_seen(rendered.is_none() && !interactive);
    let mut report = stores.print_err("");
    report.print_rendered(text);
    match rendered {
        Some(rendered) => {
            report.use_err_help(rendered);
        }
        None if long_help_seen => {
            report.help(&["(That was another \\errmessage.)"]);
        }
        None => {
            report.help(&[
                "This error message was generated by an \\errmessage",
                "command, so I can't give any explicit help.",
                "Pretend that you're Hercule Poirot: Examine all clues,",
                "and deduce the truth by order and method.",
            ]);
        }
    }
    report.context(context);
    report.error().jump_out()?;
    Ok(())
}

/// Reports TeX82 §579's `<Issue an error message if cur_val=fmem_ptr>`.
///
/// Every `FontParameterError` is a way of landing on §578's `fmem_ptr`
/// fallback -- a number at or below zero, a number past the font's table when
/// the font is not the last one loaded, or a capacity bound -- so all of them
/// report the same §579 message and leave the font untouched.
fn report_font_parameter_recovery(
    stores: &mut Universe,
    font: tex_state::ids::FontId,
    context: String,
) -> Result<(), ExecError> {
    let name = stores.font_name(font).to_owned();
    let count = i32::try_from(stores.font_parameter_count(font)).unwrap_or(i32::MAX);
    let mut report = stores.print_err("Font ");
    report
        .print_esc(&name)
        .print(" has only ")
        .print_int(count)
        .print(" fontdimen parameters");
    report.help(&[
        "To increase the number of font parameters, you must",
        "use \\fontdimen immediately after the \\font is loaded.",
    ]);
    report.context(context);
    report.error().jump_out()?;
    Ok(())
}

/// Reports TeX82 §433-§437's `print_err`/`help2`/`int_error` recovery text.
///
/// The recovery itself belongs to the restricted scan (`tex_command`'s
/// `RestrictedIntegerClass`); only the terminal report is a stomach-side
/// effect, because the command core owns no `World` text sink.
/// Converts a command-core failure into its `ExecError` counterpart,
/// preserving the originating `CommandError` variant and message. Only
/// `MissingInput` and `PdfNavigation` map onto dedicated `ExecError` variants
/// shared with other producers; every other variant is carried through
/// verbatim via `ExecError::Command` so it names itself instead of collapsing
/// into a generic `MissingToken`. This match is written one arm per variant
/// (no wildcard) so adding a new `CommandError` variant fails to compile here
/// until it is explicitly handled.
fn command_error(error: CommandError) -> ExecError {
    match error {
        CommandError::MissingInput(name) => ExecError::MissingCanonicalInput { name },
        CommandError::PdfNavigation(message) => ExecError::PdfNavigation(message),
        // §93 `succumb` is not a command failure to be re-described; it keeps
        // its own identity all the way up to the driver.
        CommandError::Fatal(fatal) => ExecError::Fatal(fatal),
        CommandError::FuelExhausted { .. }
        | CommandError::InputInvariant(_)
        | CommandError::StaleDelivery
        | CommandError::MacroPrefixMismatch
        | CommandError::ParagraphInMacroArgument
        | CommandError::OuterInMacroArgument
        | CommandError::UnsupportedExpandablePrimitive(_) => ExecError::Command(error),
    }
}

#[cfg(test)]
#[path = "command_replay/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "canonical_main_control/tests.rs"]
mod direct_tests;

#[cfg(test)]
#[path = "effects/tests.rs"]
mod effects_tests;

#[cfg(test)]
#[path = "whatsits/tests.rs"]
mod whatsits_tests;
