//! Production main-control driver.
//!
//! This intentionally owns only typed execution mutations. Raw delivery,
//! expansion, macro calls, input nesting, and operand collection remain in
//! `tex-command`; no independent source stack is accepted here.

use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::Arc;
use tex_command::{
    AlignmentCellDelimiter, AlignmentCellOpening, AlignmentDelivery, AlignmentIdentity,
    AlignmentRequest, AlignmentRequestResult, CommandError, CommandHostCapabilities,
    CommandHostContext, CommandProcessor, CommandProfile, CommandState, FatalError,
    FontLoadRequest, FontResource, GeneratedFontKind, HyphenationDataKind, ImmediateExtension,
    MathDelimiterBoundary, MathDelimiterBoundaryKind, MathFieldBody, MathLimitKind, MathRequest,
    MathScriptKind, MathStyleKind, MathTextFieldKind, PdfImageRequest, PdfImageResource,
    PdfReferenceObjectRequest, PreparedAlignmentCellTemplates, RegisteredSourceKind,
    RestrictedIntegerClass, ScannedAccent, ScannedAccentBase, ScannedBoxConstruction,
    ScannedBoxKind, ScannedBoxShift, ScannedBoxShiftPayload, ScannedDiscretionaryOpening,
    ScannedDisplayDiagnostic, ScannedGeneratedFontDefinition, ScannedInsertConstruction,
    ScannedLeaderPayload, ScannedMathMuMaterial, ScannedPackingSpec, ScannedSetBoxPath,
    ScannedVSplit, SourceRegistration, SourceRegistrationError,
};
use tex_command::{
    CommandObservation, CommandObserver, EffectRecord, GeometryRecord, MutationRecord,
    MutationTarget, ObservationEffectKind, ObservationValue, ObservedToken, ParameterClass,
    TokenListRecord, parameter_mutation_key_for_dialect,
};
use tex_state::GlueId;
use tex_state::code_tables::{DelCode, LcCode, MathCode, SfCode, UcCode};
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::FontId;
use tex_state::interner::{ControlSequenceKind, Symbol, SymbolId};
use tex_state::math::{
    FractionThickness, LimitType, MathChar, MathChoice, MathField, MathFontSize, MathFraction,
    MathNoad, MathStyle, NoadClass, NoadKind,
};
use tex_state::meaning::{Meaning, MeaningFlags, ResolvedMeaning, UnexpandablePrimitive};
use tex_state::node::{DiscKind, GlueKind, KernKind, LeaderPayload, Node, Whatsit};
use tex_state::page::{PageDimension, PageInteger};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, Token};
use tex_state::token::{OriginId, TracedTokenWord};
use tex_state::{
    CommandContext, DependencyEngineField, DependencyKey, DependencyRegionError, DependencyValue,
    GroupKind, InputReadState, ObservedDependency, ParagraphShapeLine, PenaltyArrayKind, PrintSink,
    StreamSlot, TrackedRegionBarrier, Universe,
};
use tex_typeset::PackSpec;

use crate::assignments::committer::AssignmentCommitter;
use crate::assignments::tracing as assignment_tracing;
use crate::error::DiagnosticSite;
use crate::execution_receipt::{
    ConsumedExecutionReceipt, ExecutionReceipt, MAX_EXECUTION_RECEIPT_RECORDS, OperationTermination,
};
use crate::font_support::{
    FontLoadFailure, GlyphToUnicodeParse, parse_glyph_to_unicode, report_font_capacity,
    report_font_not_loadable_with_context, warn_pdf_destination_duplicate,
};
use crate::interpreter::{InterpreterProcessor, PersistentInterpreter};
use crate::mode::{AlignColumn, AlignState, AlignmentKind, AlignmentPackSpec};
use crate::vertical::is_outer_vertical;
use crate::{ExecError, Mode, ModeNest};

mod cold;
mod hot_apply;

use cold::*;

type PreparedDviPages = Arc<Vec<crate::dispatch::PreparedDviPage>>;

/// TeX82 §1176's live `math_shift_group` context as observed by e-TeX
/// [49.1292]. Equation-number groups retain §1177's saved side.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MathShiftContext {
    Inline,
    Display,
    EqNo(crate::mode::EqNoSide),
}

fn push_prepared_dvi_page(pages: &mut PreparedDviPages, page: crate::dispatch::PreparedDviPage) {
    Arc::make_mut(pages).push(page);
}

fn take_prepared_dvi_pages(pages: &mut PreparedDviPages) -> Vec<crate::dispatch::PreparedDviPage> {
    Arc::try_unwrap(std::mem::take(pages)).unwrap_or_else(|shared| shared.as_ref().clone())
}

const fn static_meaning<G>(meaning: ResolvedMeaning<G>) -> Meaning {
    match meaning {
        ResolvedMeaning::Static(meaning) => meaning,
        ResolvedMeaning::Macro { .. } => Meaning::Undefined,
    }
}

#[derive(Debug)]
struct ImmediatePrint {
    sink: PrintSink,
    text: String,
}

#[derive(Debug)]
struct PreparedShipout {
    node: Node,
}

/// Production command main control with command-owned source consumption.
#[derive(Debug)]
pub struct MainControl<G> {
    /// The one session-lived canonical input, expansion, and scanner owner.
    /// Individual [`CommandProcessor`] values are borrow-only facades over
    /// this interpreter and cannot survive semantic or host barriers.
    command: PersistentInterpreter<G>,
    /// Operational memo service owned by the execution/session layer.
    pure_memo: Arc<std::sync::Mutex<tex_state::PureMemoRuntime>>,
    pure_memo_initialized: bool,
    fuel: tex_command::CommandFuelLedger,
    capabilities: CommandHostCapabilities,
    /// Host output capability for shipout traversal. `None` preserves the
    /// profile default: TeX/e-TeX emit DVI, while pdfTeX defers pages to its
    /// PDF driver. Virtual multi-output sessions set this explicitly.
    emit_dvi_override: Option<bool>,
    modes: ModeNest,
    /// Maximum typed §273/§275 checked pre-push save-stack projection. Runtime
    /// diagnostics do not participate in rollback, identity, or formats.
    max_save_stack: usize,
    next_alignment_identity: u64,
    active_alignment: Option<ActiveReplayAlignment<G>>,
    boxes: ReplayBoxes<G>,
    active_discretionaries: Vec<ActiveDiscretionary>,
    /// TeX82 §1174's `saved(-2)` branch count for each live `\mathchoice`,
    /// outermost first. e-TeX [49.1292] observes it through `\showgroups`.
    active_math_choices: Vec<usize>,
    /// e-TeX [48.1191]'s saved delimiter identity for each live
    /// `math_left_group`, outermost first. `true` denotes `\middle`.
    active_math_left_boundaries: Vec<bool>,
    /// Live `math_shift_group` openers, outermost first.
    active_math_shifts: Vec<MathShiftContext>,
    /// Physical glue-store identity plus the canonical pointer source of the
    /// last skip-register definition. The second component is `None` when
    /// scanning allocated a fresh TeX glue node that Umber subsequently
    /// hash-consed with an equal existing node.
    skip_pointer_sources: Vec<Option<(GlueId<G>, Option<GlueId<G>>)>>,
    /// Mu-glue counterpart of [`Self::skip_pointer_sources`]. e-TeX's
    /// `\gluetomu` and `\mutoglue` conversions retain the source pointer, so
    /// the two register banks need the same identity accounting.
    muskip_pointer_sources: Vec<Option<(GlueId<G>, Option<GlueId<G>>)>>,
    /// True while `main_control` is parked at TeX82 §1034's
    /// `main_loop_lookahead` rather than at §1030's `big_switch`.
    ///
    /// TeX's inner character loop appends a character and then fetches the
    /// next command from §1038's lookahead, which starts with a bare
    /// `get_next`; only `big_switch` uses `get_x_token`. Umber executes one
    /// command per operation, so the label it would have jumped to has to
    /// be carried across steps explicitly.
    main_loop_active: bool,
    /// TeX82's temporary `set_box_allowed:=false` ownership while §1270
    /// executes assignments after a display alignment or an accent.
    set_box_forbidden_depth: u8,
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
    /// True while TeX82 §1054's end-job material is still queued for §994's
    /// page builder.
    ///
    /// A nonempty contribution list does not imply that the end-job trio has
    /// already been appended: it may be the residual material that made
    /// `its_all_over` false in the first place. Keep this continuation
    /// explicit across command-sized steps so a suffix resumed after
    /// shipout is not given a duplicate trio either.
    end_job_ejection_pending: bool,
    /// tex.web's `init`/`tini` compile-time split as a session flag.
    ///
    /// tex.web builds INITEX and production TeX from the same source with
    /// `init`-guarded code removed from the latter, so §1252's `\patterns`
    /// and §1335's `\dump` have entirely different behavior in the two
    /// binaries. Umber has one binary, so the distinction is the session's:
    /// [`MainControl::tex82_initex`] builds an INITEX session and
    /// every other constructor a production one.
    initex: bool,
    /// Set when this session was started from a dumped format, so §61/§536's
    /// banner can name it (`(preloaded format=…)`) the way a real `-fmt` run
    /// does. `None` leaves the banner to `initex` above. Framing-only: no
    /// execution decision reads it.
    preloaded_format: Option<crate::job::PreloadedFormat>,
    /// Engine identity used for §61/§536 startup framing and canonical
    /// compiled semantics that remain active across a loaded older format.
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
    /// Whether §360's most recently acquired terminal line was empty. `None`
    /// means no `*`-prompt line has been acquired yet, so the startup line's
    /// emptiness supplies the first value.
    terminal_line_was_empty: Option<bool>,
    /// Whether root exhaustion is TeX82 §360's missing-`\end` path or an
    /// explicit host-owned fragment boundary.
    root_completion: RootCompletionPolicy,
    /// The ordered publication transaction produced by
    /// `fire_pending_page_output` after the contributing step's own records.
    /// It first receives any earlier named-list pushes still owned by that
    /// direct step, then TeX82 §§1025/323's `output_text` push. Every step
    /// drains it before the following step can deliver the routine's
    /// scanner-owned opening brace.
    page_output_observations: ObservationBuffer,
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
    /// Live-store boundaries used to close the typed receipt before the
    /// direct operation commits. Absent for ordinary execution.
    operation_receipt_start: Option<OperationReceiptStart>,
    /// Observation evidence moved across a typed resource suspension.
    ///
    /// Preflight has committed delivery and scanning when one of the narrow
    /// resource continuations is installed, so an observed retry cannot
    /// discard that prefix and reconstruct it by replay. Moving the sole
    /// buffer owner keeps publication atomic without cloning any evidence
    /// operation.
    suspended_operation_observation: Option<(ObservationBuffer, OperationReceiptStart)>,
    completed_replay_episode: Option<tex_command::CommandReplayEpisode>,
    /// Detached DVI receipts whose artifact commits have survived an entire
    /// direct operation. This replay state is published only after its
    /// corresponding World artifact/effect roots commit.
    prepared_dvi_pages: PreparedDviPages,
    immediate_prints: Vec<ImmediatePrint>,
    prepared_shipout: Option<PreparedShipout>,
    /// Named safe boundaries committed by the last direct operation. The
    /// host drains these only after `advance` has committed, so a resource
    /// suspension never leaks a checkpoint from its rolled-back operation.
    completed_boundaries: Vec<crate::EngineBoundary>,
    /// A committed artifact prefix waiting for the outer main-control owner.
    ///
    /// TeX82 §§1025--1026 may execute several `ship_out` calls while an
    /// output routine owns the input, mode, and save-stack continuations.
    /// Those commits form one outer completion and cannot publish a durable
    /// checkpoint until the routine and every other nested builder unwind.
    pending_shipout_boundary: bool,
    /// Source site of the most recent typed resource suspension. This is
    /// retained outside snapshots so a host protocol no-progress invariant
    /// can still identify the command whose retry failed to advance.
    pending_resource_site: Option<OriginId>,
    /// Settled command retained across a typed resource retry.
    /// Preflight has already committed its delivery, so retry resumes operand
    /// scanning without cloning or replaying earlier ordinary commands.
    pending_preflight_command: Option<PendingPreflightCommand<G>>,
    /// Fully scanned resource operation retained across host acquisition.
    /// The command/input cursor is already committed, so retry resolves this
    /// typed operand and never replays delivery or scanning work.
    pending_resource_operation: Option<PendingResourceOperation<G>>,
    /// Alignment-owned delivery retained across immutable host suspension.
    /// Command input and expansion continuations remain in `CommandState<G>`;
    /// this tag names the executor entry point that must resume them.
    pending_alignment_delivery: Option<PendingAlignmentDelivery>,
    /// Diagnostic-host assignment retained after expanded delivery has
    /// committed. Retrying resumes either its exact settled command/cursor or
    /// its fully scanned operation without fetching another diagnostic token.
    pending_diagnostic_operation: Option<PendingDiagnosticOperation<G>>,
    /// TeX82 §76's `history=fatal_error_stop`, carrying §93/§94/§95's payload.
    ///
    /// `succumb` ends the job through §81's `jump_out`, which a library engine
    /// cannot spell as leaving the process. This latch is the canonical
    /// equivalent: once it is set the session is terminal, every further
    /// operation reports [`MainControlStep::End`] without delivering a
    /// command, and the host reads the cause from [`Self::fatal_error`].
    fatal: Option<FatalError>,
    /// Source evidence retained when a fatal crossed a diagnostic capture
    /// seam. Diagnostic session drivers surface this exact location to their
    /// caller; complete-job drivers retain TeX's terminal completion semantics.
    captured_fatal_origin: Option<(
        DiagnosticSite,
        Option<crate::FrozenDiagnosticOrigin>,
        Option<crate::FrozenDiagnosticContext>,
    )>,
    /// First recoverable error's bounded stack evidence. Trace-only
    /// diagnostics do not populate it.
    first_causal_context: Option<crate::FrozenDiagnosticContext>,
    /// tex.web's job-framing state: see [`crate::job`] and
    /// `docs/job_framing.md`.
    job: crate::job::JobFraming,
    /// Effect-record boundary immediately before §1335 final-cleanup
    /// framing, extended through later pdfTeX navigation warnings when they
    /// exist. Drivers can project a root body without deleting framing.
    job_body_effect_end: Option<tex_state::EffectPos>,
    /// Guards the host-owned pdfTeX close-files pass against repeated
    /// `EngineSession::finish` calls after termination.
    pdf_navigation_finalized: bool,
    /// Operational accounting only; snapshots and durable checkpoints never
    /// observe it.
    advance_telemetry: AdvanceTelemetry,
    /// Monotonic evidence for bounded semantic episode admission and return.
    /// Like command fuel, this is operational and rollback never refunds it.
    episode_telemetry: crate::EpisodeTelemetry,
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
    shift: Option<ReplayBoxShift>,
}

#[derive(Clone, Copy, Debug)]
struct ReplayBoxShift {
    delta: Scaled,
    axis: BoxShiftAxis,
}

#[derive(Clone, Copy, Debug)]
enum BoxShiftAxis {
    Horizontal,
    Vertical,
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

    /// Integer words placed below this body's §273 level boundary.
    const fn save_stack_spec_words(self) -> usize {
        match self {
            // §1083 calls §645's `scan_spec(..., true)`, preserving the box
            // context in addition to the packing kind and dimension.
            Self::HBox | Self::VBox | Self::VTop => 3,
            // §1167 uses `scan_spec(..., false)`, so only the packing pair is
            // below the vcenter boundary.
            Self::VCenter => 2,
            // §1099 opens insert_group directly and saves no box spec.
            Self::Insert(..) => 0,
        }
    }
}

#[derive(Clone, Debug)]
struct ActiveReplayAlignment<G> {
    identity: AlignmentIdentity,
    kind: AlignmentKind,
    /// TeX82 §774's `save_cs_ptr`: the delivered control sequence whose
    /// meaning began this alignment, retained for §338 runaway diagnostics.
    owner: Option<tex_state::interner::Symbol>,
    /// TeX82 §645's `scan_spec` result, kept from `init_align` until §805
    /// packages the preamble prototype box with it.
    packing: AlignmentPackSpec,
    columns: Vec<PreparedAlignmentCellTemplates<G>>,
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
    captured_rows: Vec<Vec<tex_state::node_arena::PageListId>>,
    tabskips: Vec<tex_state::glue::GlueSpec>,
    default_tabskip: tex_state::glue::GlueSpec,
    /// TeX82 §786's `cur_head`/`cur_tail` holding list: the insertions, marks,
    /// and `\vadjust` contents §796's `hpack` migrated out of this row's
    /// columns, waiting for §799 `fin_row` to append them after the row.
    row_migrations: Vec<Node>,
    cell_span: u16,
    row_open: bool,
    cell_open: bool,
}

#[derive(Clone, Debug)]
struct ReplayBoxes<G> {
    pending_setbox: Option<SetBoxTarget>,
    pending_shipout: bool,
    pending_leader: Option<(GlueKind, LeaderPayload)>,
    active_boxes: Vec<ActiveReplayBox>,
    suspended_alignments: Vec<ActiveReplayAlignment<G>>,
    recovery_simple_group_pending: bool,
    recovery_simple_group_open: bool,
    output_routine_active: bool,
    output_routine_opening_pending: bool,
}

impl<G> Default for ReplayBoxes<G> {
    fn default() -> Self {
        Self {
            pending_setbox: None,
            pending_shipout: false,
            pending_leader: None,
            active_boxes: Vec::new(),
            suspended_alignments: Vec::new(),
            recovery_simple_group_pending: false,
            recovery_simple_group_open: false,
            output_routine_active: false,
            output_routine_opening_pending: false,
        }
    }
}

#[derive(Clone, Debug)]
struct ActiveDiscretionary {
    parts: Vec<tex_state::node_arena::PageListId>,
    rejected: bool,
}

/// The only normal reason a operation may be retried by its host.
///
/// The command core has already classified the unavailable resource, while
/// this value deliberately retains neither a command nor a host capability.
/// Retrying therefore starts a fresh TeX82 §§24--25 processor episode at the
/// enclosing main-control operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourceNeed {
    /// TeX82's `start_input` scanned this logical filename (§529 / §1030+),
    /// but the host has not supplied its immutable source registration.
    Input { name: String, original_name: String },
    /// A non-opening `\openin` or pdfTeX file enquiry needs bytes or
    /// authoritative absence.
    InputProbe {
        request: tex_command::FileEnquiryRequest,
    },
    /// TeX82's `new_font` completed its filename and size scan (§1254), but
    /// the host has not supplied the immutable font bytes.
    Font { request: FontLoadRequest },
    /// pdfTeX's `scan_image` completed an immutable request, but its retained
    /// bytes and validated metadata have not been supplied by the host.
    PdfImage { request: PdfImageRequest },
}

/// Outcome of one atomic main-control operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StepResult {
    Progress(MainControlStep),
    Suspended(ResourceNeed),
}

/// One committed ordinary step and the detached dependency evidence attempted
/// for exactly that operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackedStepResult {
    pub step: StepResult,
    /// `None` means the operation suspended or failed and the recorder was
    /// abandoned before direct-operation completion.
    pub region: Option<Result<TrackedRegionRecord, DependencyRegionError>>,
}

/// Detached dependency evidence from one completed direct command episode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackedRegionRecord {
    observations: Vec<ObservedDependency>,
}

impl TrackedRegionRecord {
    fn new(observations: Vec<ObservedDependency>) -> Self {
        Self { observations }
    }

    #[must_use]
    pub fn observations(&self) -> &[ObservedDependency] {
        &self.observations
    }
}

/// Host decision sampled immediately before a direct operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdvanceReadiness {
    Ready,
    /// A host interrupt was observed between direct operations. TeX's
    /// instruction dialog runs before the untouched command stream resumes.
    Interrupted,
    Cancelled,
}

/// Outcome of a cancellation-aware advance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdvanceOutcome {
    Step(StepResult),
    Cancelled,
}

/// Non-semantic accounting for canonical operation and legacy-savepoint
/// profiling fields.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AdvanceTelemetry {
    pub attempts: u64,
    pub commits: u64,
    pub rollbacks: u64,
    /// Main-control deliveries discarded by typed resource suspension.
    pub resource_replayed_delivered_tokens: u64,
    /// Main-control dispatches discarded by typed resource suspension.
    pub resource_replayed_dispatches: u64,
    pub live_savepoints: u64,
    pub maximum_live_savepoints: u64,
}

/// One retained diagnostic-expansion operation. Assignments are executed by
/// main control; every other expanded spelling is returned intact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticStep {
    Token {
        spelling: TracedTokenWord,
        meaning: Meaning,
        control_sequence: Option<tex_state::interner::Symbol>,
        source_provenance: Option<tex_command::SourceProvenance>,
    },
    Assignment,
    EndOfInput,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiagnosticStepResult {
    Progress(DiagnosticStep),
    Suspended(ResourceNeed),
}

/// Where one command-processor episode publishes its committed records.
///
/// An episode with no observer carries `None`. The slot is still a parameter
/// of [`command_processor`] so that no episode can be constructed without
/// stating which commit buffer it belongs to.
type ObservationSlot = Option<ObservationBuffer>;

/// The small command-delivery choice at the front of one operation.
///
/// Delivery selects only how the next completed command enters main control;
/// preparation, application, publication, and evidence are shared.
enum OperationDelivery<G> {
    Replay(Option<tex_command::CurrentCommand<G>>),
    /// TeX82 §1038's main-loop lookahead delivered this command with bare
    /// `get_next`; it must not acquire an expanded-delivery observation when
    /// the scanner borrow resumes.
    Raw {
        command: tex_command::CurrentCommand<G>,
        cursor: Option<tex_command::CommandDeliveryCursor>,
    },
    /// Expansion settled in the processor borrow that produced this command,
    /// including its canonical expanded observation. This covers both raw
    /// preflight and an in-place TeX82 `goto reswitch`/§1270 handoff.
    Settled {
        command: tex_command::CurrentCommand<G>,
        cursor: Option<tex_command::CommandDeliveryCursor>,
    },
    Expanding {
        command: tex_command::CurrentCommand<G>,
        main_loop: bool,
        cursor: tex_command::CommandDeliveryCursor,
    },
    /// `\immediate` already consumed its recursive PDF command before a
    /// pre-operand DVI rejection. Retry resumes that PDF operand scanner
    /// directly; rewinding the outer command/input aggregate is unnecessary.
    ImmediatePdfRetry(UnexpandablePrimitive),
    Alignment(AlignmentIdentity),
    AlignmentRetry {
        alignment: Option<AlignmentIdentity>,
        cursor: tex_command::CommandDeliveryCursor,
    },
    /// Ordinary ranked delivery and operand scanning completed inside the
    /// preflight processor borrow. The typed family operand is the real Rust
    /// borrow barrier before semantic state application; no command or
    /// universal scanned-step DTO crosses it.
    Hot(hot_apply::HotOperation<G>),
    /// Delivery completed during mutation-free capability preflight. The
    /// semantic step still runs through the sole executor below.
    Prepared(Box<ColdOperation<G>>),
}

#[derive(Clone, Debug)]
enum PendingPreflightCommand<G> {
    Settled {
        command: tex_command::CurrentCommand<G>,
        cursor: Option<tex_command::CommandDeliveryCursor>,
    },
    Raw {
        command: tex_command::CurrentCommand<G>,
        cursor: Option<tex_command::CommandDeliveryCursor>,
    },
    Expanding {
        command: tex_command::CurrentCommand<G>,
        main_loop: bool,
        cursor: tex_command::CommandDeliveryCursor,
    },
    ImmediatePdfRetry(UnexpandablePrimitive),
}

impl<G> PendingPreflightCommand<G> {
    fn with_retry_expansion(self, retry: Option<tex_command::CurrentCommand<G>>) -> Self {
        let Some(retry) = retry else {
            return self;
        };
        match self {
            Self::Expanding {
                main_loop, cursor, ..
            } => Self::Expanding {
                command: retry,
                main_loop,
                cursor,
            },
            other => other,
        }
    }

    fn with_cursor(self, cursor: tex_command::CommandDeliveryCursor) -> Self {
        match self {
            Self::Settled { command, .. } => Self::Settled {
                command,
                cursor: Some(cursor),
            },
            Self::Raw { command, .. } => Self::Raw {
                command,
                cursor: Some(cursor),
            },
            Self::Expanding {
                command, main_loop, ..
            } => Self::Expanding {
                command,
                main_loop,
                cursor,
            },
            Self::ImmediatePdfRetry(primitive) => Self::ImmediatePdfRetry(primitive),
        }
    }
}

struct PreflightDelivery<G> {
    delivery: OperationDelivery<G>,
    capabilities: crate::transaction_protocol::CommandCapabilities,
}

struct PreparedColdOperation<G> {
    scanned: PreparedColdCommand<G>,
    alignment_preamble: Option<PreparedAlignmentPreamble<G>>,
    outer_paragraph_was_active: bool,
    artifact_count: usize,
    effect_count: usize,
    prepared_page_count: usize,
}

struct PreparedAlignmentPreamble<G> {
    alignment: AlignmentIdentity,
    columns: Vec<PreparedAlignmentCellTemplates<G>>,
    tabskips: Vec<GlueSpec>,
    default_tabskip: GlueSpec,
    repeat_start: Option<usize>,
}

/// The result of the unified dispatch seam.
///
/// Common unexpandable families are already applied when this value is
/// returned. Cold and barrier families retain the existing prepared value.
enum OperationReadiness<G> {
    Applied(Result<ReplayStep, ExecError>),
    Prepared(PreparedColdOperation<G>),
}

/// One command after canonical delivery and operand scanning.
///
/// The hot variant is a family-sized borrow-release operand. Only the cold
/// variant materializes a typed cold operation.
enum ScannedOperation<G> {
    Hot(hot_apply::HotOperation<G>),
    Cold(ColdOperation<G>),
}

impl<G> From<ColdOperation<G>> for ScannedOperation<G> {
    fn from(scanned: ColdOperation<G>) -> Self {
        Self::Cold(scanned)
    }
}

struct PendingResourceOperation<G> {
    scanned: Box<ColdOperation<G>>,
    capabilities: crate::transaction_protocol::CommandCapabilities,
    attempt: tex_command::CommandAttemptMark,
}

#[derive(Clone, Copy, Debug)]
struct PendingAlignmentDelivery {
    alignment: Option<AlignmentIdentity>,
    cursor: tex_command::CommandDeliveryCursor,
}

enum PendingDiagnosticOperation<G> {
    Assignment {
        command: tex_command::CurrentCommand<G>,
        cursor: tex_command::CommandDeliveryCursor,
        attempt: tex_command::CommandAttemptMark,
    },
    Prepared {
        scanned: Box<ColdOperation<G>>,
        attempt: tex_command::CommandAttemptMark,
    },
}

impl<G> std::fmt::Debug for PendingDiagnosticOperation<G> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Assignment { .. } => "PendingDiagnosticOperation::<G>::Assignment",
            Self::Prepared { .. } => "PendingDiagnosticOperation::<G>::Prepared",
        })
    }
}

impl<G> std::fmt::Debug for PendingResourceOperation<G> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingResourceOperation<G>")
            .field("family", &self.capabilities.family())
            .finish_non_exhaustive()
    }
}

struct UnavailablePreparedResource<G> {
    error: ExecError,
    scanned: Box<ColdOperation<G>>,
}

struct PrepareOperationError<G> {
    error: Box<ExecError>,
    unavailable: Option<Box<ColdOperation<G>>>,
    cursor: Option<tex_command::CommandDeliveryCursor>,
}

impl<G> PrepareOperationError<G> {
    fn with_cursor(error: ExecError, cursor: tex_command::CommandDeliveryCursor) -> Self {
        Self {
            error: Box::new(error),
            unavailable: None,
            cursor: Some(cursor),
        }
    }
}

#[derive(Clone, Copy)]
struct DirectFailureContext {
    operations: usize,
    initial_artifacts: usize,
    initial_boundaries: usize,
    initial_effect_pos: tex_state::EffectPos,
}

/// Fixed-size rollback coordinates for one direct command operation.
///
/// Mode roots are restored first; only then may the attempt and page-arena
/// suffixes be truncated.
#[derive(Clone, Copy, Debug)]
struct DirectOperationMark<G> {
    state: tex_state::JournalCursor<G>,
    mode: crate::mode::ModeJournalCursor,
    attempt: tex_command::CommandAttemptMark,
    page: tex_state::node_arena::NodeArenaCursor<tex_state::node_arena::PageLifetime>,
}

/// Whether a completed direct-operation frame still owns attempt-local
/// coordinates through a typed retry continuation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DirectAttemptDisposition {
    /// Every declared escape root has been promoted and installed. Command
    /// state now recomputes its remaining live coordinates and reclaims only
    /// the unreachable suffix.
    ReclaimUnreachable,
    /// A typed continuation still owns attempt-local coordinates. The exact
    /// opening mark moves with that continuation and is discarded only after
    /// its resumed operation commits or rolls back.
    RetainForRetry,
}

impl<G> From<ExecError> for PrepareOperationError<G> {
    fn from(error: ExecError) -> Self {
        Self {
            error: Box::new(error),
            unavailable: None,
            cursor: None,
        }
    }
}

impl<G> From<Box<UnavailablePreparedResource<G>>> for PrepareOperationError<G> {
    fn from(unavailable: Box<UnavailablePreparedResource<G>>) -> Self {
        let unavailable = *unavailable;
        Self {
            error: Box::new(unavailable.error),
            unavailable: Some(unavailable.scanned),
            cursor: None,
        }
    }
}

#[derive(Clone, Copy)]
enum OperationTransaction {
    Advance,
    Alignment,
    Nested,
}

const MAX_OPERATION_EVIDENCE_RECORDS: usize = 1_000_000;

#[derive(Debug)]
struct ObservationBuffer {
    records: Vec<CommandObservation>,
    attempted: usize,
    overflowed: bool,
    receipt_attempted: usize,
    receipt_overflowed: bool,
    receipt: ExecutionReceipt,
}

#[derive(Clone, Copy, Debug)]
struct OperationReceiptStart {
    effect: u64,
    artifact: usize,
}

/// Explicit live-observer boundary for detached shipout geometry.
struct MainControlShipoutGeometrySink<'a, G> {
    command: &'a PersistentInterpreter<G>,
    observations: &'a mut ObservationSlot,
}

impl<G> crate::shipout::ShipoutGeometrySink for MainControlShipoutGeometrySink<'_, G> {
    fn committed_shipout_geometry(&mut self, geometry: crate::shipout::ShipoutGeometry) {
        let Some(observations) = self.observations.as_mut() else {
            return;
        };
        observations.committed(CommandObservation::Geometry(GeometryRecord::Shipout {
            page_width_sp: geometry.page_width_sp,
            page_height_sp: geometry.page_height_sp,
            counts: geometry.counts,
            line: self.command.current_file_line_number(),
            source: self.command.current_file_source_id(),
        }));
    }
}

impl Default for ObservationBuffer {
    fn default() -> Self {
        Self {
            records: Vec::new(),
            attempted: 0,
            overflowed: false,
            receipt_attempted: 1,
            receipt_overflowed: false,
            receipt: ExecutionReceipt::default(),
        }
    }
}

impl ObservationBuffer {
    fn consume_into(self, observer: Option<&mut dyn CommandObserver>) -> ConsumedExecutionReceipt {
        if let Some(observer) = observer {
            for observation in self.records {
                observer.committed(observation);
            }
        }
        self.receipt.consume()
    }

    fn extend(&mut self, records: impl IntoIterator<Item = CommandObservation>) {
        for record in records {
            self.committed(record);
        }
    }

    fn append(&mut self, other: &mut Self) {
        let omitted = other.attempted.saturating_sub(other.records.len());
        self.overflowed |= other.overflowed;
        self.receipt_overflowed |= other.receipt_overflowed;
        for record in other.records.drain(..) {
            self.committed(record);
        }
        let consumed = std::mem::take(&mut other.receipt).consume();
        debug_assert!(consumed.records <= MAX_EXECUTION_RECEIPT_RECORDS);
        self.attempted = self.attempted.saturating_add(omitted);
        self.receipt_attempted = self.receipt_attempted.saturating_add(
            other
                .receipt_attempted
                .saturating_sub(other.receipt.record_count()),
        );
        self.overflowed |= self.attempted > MAX_OPERATION_EVIDENCE_RECORDS;
        self.receipt_overflowed |= self.receipt_attempted > MAX_EXECUTION_RECEIPT_RECORDS;
    }

    fn append_to(&mut self, records: &mut Vec<CommandObservation>) {
        records.append(&mut self.records);
        let consumed = std::mem::take(&mut self.receipt).consume();
        debug_assert!(consumed.records <= MAX_EXECUTION_RECEIPT_RECORDS);
        self.attempted = 0;
        self.overflowed = false;
        self.receipt_attempted = 1;
        self.receipt_overflowed = false;
    }

    fn clear(&mut self) {
        self.records.clear();
        let consumed = std::mem::take(&mut self.receipt).consume();
        debug_assert!(consumed.records <= MAX_EXECUTION_RECEIPT_RECORDS);
        self.attempted = 0;
        self.overflowed = false;
        self.receipt_attempted = 1;
        self.receipt_overflowed = false;
    }

    fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    fn limit_error(&self) -> Option<ExecError> {
        if self.overflowed {
            Some(ExecError::ResourceBudgetExceeded {
                resource: "operation evidence records",
                limit: MAX_OPERATION_EVIDENCE_RECORDS as u64,
                attempted: self.attempted.try_into().unwrap_or(u64::MAX),
            })
        } else if self.receipt_overflowed {
            Some(ExecError::ResourceBudgetExceeded {
                resource: "operation receipt records",
                limit: self.receipt.limit().try_into().unwrap_or(u64::MAX),
                attempted: self.receipt_attempted.try_into().unwrap_or(u64::MAX),
            })
        } else {
            None
        }
    }

    fn record_receipt(&mut self, append: impl FnOnce(&mut ExecutionReceipt) -> bool) {
        self.receipt_attempted = self.receipt_attempted.saturating_add(1);
        if !append(&mut self.receipt) {
            self.receipt_overflowed = true;
        }
    }

    fn record_world_effect(&mut self, effect: tex_state::EffectRecord) {
        self.record_receipt(|receipt| receipt.record_world_effect(effect));
    }

    fn record_artifact(&mut self, artifact: tex_state::ContentHash) {
        self.record_receipt(|receipt| receipt.record_artifact(artifact));
    }

    fn record_resource(&mut self, resource: ResourceNeed) {
        self.record_receipt(|receipt| receipt.record_resource(resource));
    }
}

impl CommandObserver for ObservationBuffer {
    fn committed(&mut self, observation: CommandObservation) {
        self.attempted = self.attempted.saturating_add(1);
        if self.records.len() < MAX_OPERATION_EVIDENCE_RECORDS {
            if matches!(
                observation,
                CommandObservation::Mutation(_)
                    | CommandObservation::Diagnostic(_)
                    | CommandObservation::Effect(_)
            ) {
                self.receipt_attempted = self.receipt_attempted.saturating_add(1);
                if !self.receipt.capture_observation(&observation) {
                    self.receipt_overflowed = true;
                }
            }
            self.records.push(observation);
        } else {
            self.overflowed = true;
        }
    }
}

/// Constructs the one kind of command-processor episode canonical main
/// control ever runs.
///
/// This is the only production processor-borrow helper in `tex-exec`, and the
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
struct CommandMachine<'a, G> {
    state: &'a mut PersistentInterpreter<G>,
    fuel: &'a mut tex_command::CommandFuel,
    capabilities: &'a mut CommandHostCapabilities,
    observations: &'a mut ObservationSlot,
    assignment_receipts: Option<&'a mut Vec<MutationRecord>>,
    shown_mode: &'a mut Option<Mode>,
    /// tex.web's `init`/`tini` compile-time split, which Umber carries as a
    /// session flag: §1252's `\patterns` and §1335's `\dump` are the two
    /// commands whose whole behavior it selects.
    initex: bool,
    emit_dvi_override: Option<bool>,
    immediate_prints: &'a mut Vec<ImmediatePrint>,
    prepared_shipout: &'a mut Option<PreparedShipout>,
}

impl<G> CommandMachine<'_, G> {
    fn processor<'episode, 'admission>(
        &'episode mut self,
        context: tex_state::CommandContext<'admission, G>,
    ) -> InterpreterProcessor<'episode, 'admission, G> {
        let observer = self
            .observations
            .as_mut()
            .map(|buffer| buffer as &mut dyn CommandObserver);
        self.state.processor(
            context,
            CommandHostContext::new(self.capabilities),
            self.fuel,
            observer,
        )
    }

    fn shipout_geometry_sink(&mut self) -> MainControlShipoutGeometrySink<'_, G> {
        MainControlShipoutGeometrySink {
            command: self.state,
            observations: self.observations,
        }
    }

    fn retain_assignment_receipt(
        &mut self,
        receipt: crate::assignments::committer::MutationReceipt,
    ) {
        // Receipt payloads are detached instrumentation.  In particular,
        // indexed registers defer formatting their canonical key until this
        // operation actually has an observer; the ordinary execution path
        // must not pay to serialize a mutation that nobody can consume.
        if self.assignment_receipts.is_none() && self.observations.is_none() {
            return;
        }
        let Some(record) = receipt.into_record() else {
            return;
        };
        if let Some(receipts) = self.assignment_receipts.as_mut() {
            receipts.push(record);
        } else if let Some(observations) = self.observations.as_mut() {
            observations.committed(CommandObservation::Mutation(record));
        }
    }

    /// Whether this operation has a consumer for detached mutation evidence.
    /// Hot semantic handlers consult this before resolving names or walking
    /// macro bodies; live state and `\tracingassigns` remain unconditional.
    const fn observes_mutations(&self) -> bool {
        self.assignment_receipts.is_some() || self.observations.is_some()
    }
}

fn command_processor<'episode, 'admission, G>(
    command: &'episode mut PersistentInterpreter<G>,
    fuel: &'episode mut tex_command::CommandFuel,
    capabilities: &'episode mut CommandHostCapabilities,
    observations: &'episode mut ObservationSlot,
    stores: CommandContext<'admission, G>,
) -> InterpreterProcessor<'episode, 'admission, G> {
    let observer = observations
        .as_mut()
        .map(|buffer| buffer as &mut dyn CommandObserver);
    command.processor(
        stores,
        CommandHostContext::new(capabilities),
        fuel,
        observer,
    )
}

impl<G> Default for MainControl<G> {
    fn default() -> Self {
        Self {
            command: PersistentInterpreter::default(),
            pure_memo: Arc::default(),
            pure_memo_initialized: false,
            fuel: tex_command::CommandFuelLedger::default(),
            capabilities: CommandHostCapabilities::default(),
            emit_dvi_override: None,
            modes: ModeNest::default(),
            max_save_stack: 0,
            next_alignment_identity: 0,
            active_alignment: None,
            boxes: ReplayBoxes::default(),
            active_discretionaries: Vec::new(),
            active_math_choices: Vec::new(),
            active_math_left_boundaries: Vec::new(),
            active_math_shifts: Vec::new(),
            skip_pointer_sources: Vec::new(),
            muskip_pointer_sources: Vec::new(),
            main_loop_active: false,
            set_box_forbidden_depth: 0,
            shown_mode: None,
            main_control_entered: false,
            end_job_ejection_pending: false,
            initex: false,
            preloaded_format: None,
            engine_binary: None,
            dumped_format: None,
            startup_terminal_line: String::new(),
            terminal_line_was_empty: None,
            root_completion: RootCompletionPolicy::default(),
            page_output_observations: ObservationBuffer::default(),
            operation_observations: None,
            operation_receipt_start: None,
            suspended_operation_observation: None,
            completed_replay_episode: None,
            prepared_dvi_pages: PreparedDviPages::default(),
            immediate_prints: Vec::new(),
            prepared_shipout: None,
            completed_boundaries: Vec::new(),
            pending_shipout_boundary: false,
            pending_resource_site: None,
            pending_preflight_command: None,
            pending_resource_operation: None,
            pending_alignment_delivery: None,
            pending_diagnostic_operation: None,
            fatal: None,
            captured_fatal_origin: None,
            first_causal_context: None,
            job: crate::job::JobFraming::default(),
            job_body_effect_end: None,
            pdf_navigation_finalized: false,
            advance_telemetry: AdvanceTelemetry::default(),
            episode_telemetry: crate::EpisodeTelemetry::default(),
        }
    }
}

impl<G> MainControl<G> {
    /// Replaces the execution-owned memo service between candidate runs.
    pub fn install_pure_memo_runtime(&mut self, runtime: tex_state::PureMemoRuntime) {
        self.pure_memo = Arc::new(std::sync::Mutex::new(runtime));
        self.pure_memo_initialized = true;
    }

    /// Returns the execution-owned memo service to its session owner.
    pub fn take_pure_memo_runtime(&mut self) -> tex_state::PureMemoRuntime {
        let runtime = std::mem::take(&mut self.pure_memo);
        self.pure_memo_initialized = false;
        Arc::try_unwrap(runtime)
            .expect("Universe<G> retains only a weak memo capability")
            .into_inner()
            .expect("memo runtime mutex is not poisoned")
    }

    #[must_use]
    pub fn pure_memo_stats(&self) -> tex_state::PureMemoStats {
        self.pure_memo
            .lock()
            .expect("memo runtime mutex is not poisoned")
            .stats()
    }

    /// Attaches this execution-owned service before candidate setup records
    /// carried history outside the command step loop.
    pub fn attach_pure_memo_capability(&self, stores: &mut Universe<G>) {
        stores.attach_pure_memo_capability(&self.pure_memo);
    }

    /// Reports direct-operation accounting without exposing the mode journal
    /// or any rollback capability. The savepoint fields remain schema-stable
    /// profiling counters and stay zero on the canonical path.
    #[must_use]
    pub const fn advance_telemetry(&self) -> AdvanceTelemetry {
        self.advance_telemetry
    }

    /// Reports why bounded semantic episodes returned to their live session.
    #[must_use]
    pub const fn episode_telemetry(&self) -> crate::EpisodeTelemetry {
        self.episode_telemetry
    }

    pub(crate) fn record_external_episode_barrier(
        &mut self,
        barrier: crate::SemanticEpisodeBarrier,
    ) {
        self.episode_telemetry.record_semantic_barrier(barrier);
    }

    /// Samples host cancellation before mutating command, mode, or Universe<G>
    /// state.
    pub fn advance_when(
        &mut self,
        stores: &mut Universe<G>,
        readiness: AdvanceReadiness,
    ) -> Result<AdvanceOutcome, ExecError> {
        if readiness == AdvanceReadiness::Cancelled {
            self.episode_telemetry
                .record_semantic_barrier(crate::SemanticEpisodeBarrier::Cancellation);
            return Ok(AdvanceOutcome::Cancelled);
        }
        if readiness == AdvanceReadiness::Interrupted {
            self.pause_for_instructions(stores)?;
        }
        self.advance(stores).map(AdvanceOutcome::Step)
    }

    fn pause_for_instructions(&mut self, stores: &mut Universe<G>) -> Result<(), ExecError> {
        // tex.web §330: an injected interruption always enters ErrorStop,
        // reports against the live canonical input cursor, and leaves that
        // cursor untouched when the dialog returns.
        stores.set_interaction_mode(tex_state::InteractionMode::ErrorStop);
        self.refresh_host_capabilities(stores);
        let processor = command_processor(
            &mut self.command,
            self.fuel.fuel_mut(),
            &mut self.capabilities,
            &mut self.operation_observations,
            stores.command_context().expect("live generation"),
        );
        let context = processor.error_context();
        let mut command_context = processor.into_context();
        crate::error_report::report_error(
            &mut command_context,
            "Interruption",
            &[
                "You rang?",
                "Try to insert some instructions for me (e.g., `I\\showlists'),",
                "unless you just want to quit by typing `X'.",
            ],
            context,
        )
    }

    fn local_glue_pointer_reassigned<T, D>(
        &self,
        stores: &mut Universe<G>,
        scanned: &ColdOperation<G, T, D>,
    ) -> bool {
        let context = stores.command_context().expect("live generation");
        let (index, value, source_identity, source_is_target, physical, pointer_sources) =
            match scanned {
                ColdOperation::Skip {
                    index,
                    value,
                    source_identity,
                    source_skip_index,
                    global: false,
                    ..
                } => (
                    *index,
                    value,
                    source_identity,
                    *source_skip_index == Some(*index),
                    match context.glue_register(*index).ok().flatten() {
                        Some(physical) => physical,
                        None => return false,
                    },
                    &self.skip_pointer_sources,
                ),
                ColdOperation::Muskip {
                    index,
                    value,
                    source_identity,
                    global: false,
                    ..
                } => (
                    *index,
                    value,
                    source_identity,
                    false,
                    match context.muskip(*index) {
                        Some(physical) => physical,
                        None => return false,
                    },
                    &self.muskip_pointer_sources,
                ),
                _ => return false,
            };
        if context.glue(physical) == GlueSpec::ZERO && *value == GlueSpec::ZERO {
            // TeX82 §1237's `trap_zero_glue` canonicalizes every scanned
            // zero specification before e-TeX [19.277] compares pointers.
            return true;
        }
        let Some(source_identity) = source_identity else {
            return false;
        };
        if source_is_target {
            return true;
        }
        let canonical_source = pointer_sources
            .get(usize::from(index))
            .and_then(|entry| *entry)
            .filter(|(recorded_physical, _)| *recorded_physical == physical)
            .map_or(Some(physical), |(_, source)| source);
        canonical_source == Some(*source_identity)
    }

    fn etex_redundant_local_glue_assignment<T, D>(
        &self,
        stores: &mut Universe<G>,
        scanned: &ColdOperation<G, T, D>,
    ) -> bool {
        stores
            .command_context()
            .expect("live generation")
            .int_param(IntParam::ETEX_EXTENDED_MODE)
            > 0
            && self.local_glue_pointer_reassigned(stores, scanned)
    }

    pub const DEFAULT_FUEL_LIMIT: u64 = tex_command::DEFAULT_COMMAND_FUEL_LIMIT;

    /// Creates command-owned state without changing the shared `Universe<G>`.
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
            command: PersistentInterpreter::new(profile),
            ..Self::default()
        }
    }

    /// Creates INITEX command state for a profile whose primitive meanings
    /// the composed driver has already installed in `stores`.
    #[must_use]
    pub fn prepared_initex(profile: CommandProfile) -> Self {
        Self {
            command: PersistentInterpreter::new(profile),
            next_alignment_identity: 1,
            initex: true,
            ..Self::default()
        }
    }

    /// Creates a fresh TeX82 INITEX environment.
    ///
    /// The primitive definitions are installed from the engine's static TeX82
    /// registries, before any fixture or host source is registered.
    #[must_use]
    pub fn tex82_initex(stores: &mut Universe<G>) -> Self {
        tex_command::install_tex82_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        Self {
            command: PersistentInterpreter::new(CommandProfile::TEX82),
            next_alignment_identity: 1,
            initex: true,
            ..Self::default()
        }
    }

    /// Borrows command state for source registration and snapshots.
    #[must_use]
    pub fn command_mut(&mut self) -> &mut CommandState<G> {
        self.command.state_mut()
    }

    /// Returns the number of live TeX input levels.
    #[must_use]
    pub fn input_level_count(&self) -> usize {
        self.command.input_level_count()
    }

    /// Returns the immutable profile of this command processor.
    #[must_use]
    pub fn command_profile(&self) -> CommandProfile {
        self.command.state().profile()
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

    /// Returns the monotonic scalar command-work vector for this session.
    #[must_use]
    pub const fn command_work(&self) -> tex_command::CommandWorkCounters {
        self.fuel.work()
    }

    /// Selects whether shipout also prepares DVI page receipts.
    ///
    /// This is immutable host policy for a retained run, not TeX state, and
    /// therefore remains outside formats and command checkpoints.
    pub fn set_dvi_output(&mut self, enabled: bool) {
        self.emit_dvi_override = Some(enabled);
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
        &mut self,
        boundary: crate::EngineBoundary,
        stores: &mut Universe<G>,
        budget_counters: crate::ExecutionBudgetCounters,
    ) -> Result<crate::EngineCheckpoint<G>, tex_command::CommandSummaryError> {
        if self.has_external_attempt_owner() {
            return Err(tex_command::CommandSummaryError::AttemptSuspended);
        }
        crate::EngineCheckpoint::capture_checkpoint(
            boundary,
            &mut self.command,
            &mut self.modes,
            stores,
            budget_counters,
            false,
        )
    }

    /// Captures a quiescent named checkpoint with the strong optional state
    /// identity required by incremental suffix-adoption comparisons.
    pub fn capture_checkpoint_with_exact_identity(
        &mut self,
        boundary: crate::EngineBoundary,
        stores: &mut Universe<G>,
        budget_counters: crate::ExecutionBudgetCounters,
    ) -> Result<crate::EngineCheckpoint<G>, tex_command::CommandSummaryError> {
        if self.has_external_attempt_owner() {
            return Err(tex_command::CommandSummaryError::AttemptSuspended);
        }
        crate::EngineCheckpoint::capture_checkpoint(
            boundary,
            &mut self.command,
            &mut self.modes,
            stores,
            budget_counters,
            true,
        )
    }

    /// Restores a named checkpoint into this command processor.  The
    /// checkpoint is quiescent, so command-owned replay episodes are reset
    /// rather than serialized into a durable format or editor boundary.
    pub fn restore_checkpoint(
        &mut self,
        checkpoint: &crate::EngineCheckpoint<G>,
        stores: &mut Universe<G>,
    ) -> Result<(), crate::CheckpointRestoreError> {
        if self.has_external_attempt_owner() {
            return Err(crate::CheckpointRestoreError::AttemptSuspended);
        }
        checkpoint.restore_state(&mut self.command, &mut self.modes, stores)?;
        self.active_alignment = None;
        self.boxes = ReplayBoxes::default();
        self.pending_shipout_boundary = false;
        self.fatal = None;
        self.captured_fatal_origin = None;
        Ok(())
    }

    /// Reports attempt-arena roots retained by executor continuations rather
    /// than by [`CommandState`]. These owners must be rejected before a named
    /// checkpoint asks command state to census and reclaim its own roots.
    ///
    /// Pending preflight commands and alignment deliveries are deliberately
    /// absent: their live attempt coordinates remain in `CommandState`. The
    /// two scanned-operation continuations below each carry the exact opening
    /// [`tex_command::CommandAttemptMark`] outside command state.
    fn has_external_attempt_owner(&self) -> bool {
        self.pending_resource_operation.is_some() || self.pending_diagnostic_operation.is_some()
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
    pub fn begin_job(&mut self, stores: &mut Universe<G>, first_line: &str) {
        self.begin_job_for_input(stores, first_line, first_line);
    }

    /// Frames a startup invocation whose echoed §534 line also contains
    /// driver syntax such as web2c's `&format`, while §§528--529 derive the
    /// immutable job name from the separately parsed input filename.
    pub fn begin_job_for_input(
        &mut self,
        stores: &mut Universe<G>,
        first_line: &str,
        input_name: &str,
    ) {
        self.begin_job_with_terminal_banner(stores, first_line, input_name, true);
    }

    /// Opens the transcript and catches up §534 after a driver already
    /// emitted §61's terminal headline before interactive root acquisition.
    pub fn begin_job_after_terminal_headline(
        &mut self,
        stores: &mut Universe<G>,
        first_line: &str,
    ) {
        self.begin_job_after_terminal_headline_for_input(stores, first_line, first_line);
    }

    /// Catch-up variant retaining the complete terminal line independently
    /// from the filename selected from it.
    pub fn begin_job_after_terminal_headline_for_input(
        &mut self,
        stores: &mut Universe<G>,
        first_line: &str,
        input_name: &str,
    ) {
        self.begin_job_with_terminal_banner(stores, first_line, input_name, false);
    }

    fn begin_job_with_terminal_banner(
        &mut self,
        stores: &mut Universe<G>,
        first_line: &str,
        input_name: &str,
        print_terminal_banner: bool,
    ) {
        let binary = self.engine_binary.unwrap_or_else(|| {
            crate::job::EngineBinaryIdentity::for_profile(self.command_profile())
        });
        let etex = self.command_profile() == CommandProfile::ETEX26;
        // §534's `**` line is exactly what §313 pseudoprints for the base
        // terminal level; a driver that frames the job here rather than
        // scanning the line through `scan_startup_file_name` supplies it here.
        first_line.clone_into(&mut self.startup_terminal_line);
        self.command.set_terminal_context_line(first_line);
        let engine = crate::job::JobEngineFraming {
            binary,
            extended_mode: etex,
        };
        if print_terminal_banner {
            crate::job::begin_job_with_terminal_banner(
                &mut self.job,
                stores,
                &mut self.capabilities,
                self.initex,
                self.preloaded_format.as_ref(),
                engine,
                crate::job::StartupLineFraming {
                    first_line,
                    input_name,
                    terminal_banner: true,
                },
            );
        } else {
            crate::job::begin_job_with_terminal_banner(
                &mut self.job,
                stores,
                &mut self.capabilities,
                self.initex,
                self.preloaded_format.as_ref(),
                engine,
                crate::job::StartupLineFraming {
                    first_line,
                    input_name,
                    terminal_banner: false,
                },
            );
        }
    }

    /// Prints and accounts for the retained driver's already-open root input.
    ///
    /// This is TeX82 §537's opening boundary. Keeping its `open_parens`
    /// mutation beside the print lets §1335 close an input abandoned by
    /// `\end` or `\dump`, just as it closes command-opened inputs.
    pub fn open_startup_input(&mut self, stores: &mut Universe<G>, name: &str) {
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

    /// Selects the engine binary identity used by startup framing and shared
    /// compiled command semantics.
    pub fn set_engine_binary(&mut self, binary: crate::job::EngineBinaryIdentity) {
        self.command
            .set_engine_semantics(binary.command_semantics());
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
    pub fn finish_job(
        &mut self,
        stores: &mut Universe<G>,
        dvi: Option<crate::DviJobOutput>,
        pdf: Option<&mut crate::PdfJobFinalizationReport>,
    ) {
        let usage = stores
            .command_context()
            .expect("job usage admission")
            .detach_engine_usage_statistics();
        crate::job::finish_job(
            stores,
            self.command_profile(),
            self.engine_binary.unwrap_or_else(|| {
                crate::job::EngineBinaryIdentity::for_profile(self.command_profile())
            }),
            usage,
            self.capabilities.job_name(),
            dvi,
            pdf,
        );
    }

    /// Selects TeX82 §§532--533's lazy DVI output name at the first point a
    /// driver needs to serialize shipped pages.
    pub fn dvi_output_name(&mut self, stores: &mut Universe<G>) -> Result<String, ExecError> {
        self.job
            .output
            .dvi_name(stores, self.capabilities.job_name())
            .map(str::to_owned)
            .map_err(|error| {
                ExecError::InvalidShipoutArtifact(format!(
                    "unable to open DVI output name: {error:?}"
                ))
            })
    }

    /// Renders whatever §537/§362 bracketing the command core queued but had
    /// no `Universe<G>` in hand to print.
    ///
    /// The command core renders every event at the point tex.web prints it
    /// whenever it can -- §362's `)` has to precede the
    /// `check_outer_validity` diagnostic printed a line later inside
    /// `get_next` -- so what reaches here is the residue. Every step driver
    /// calls this once, immediately after it reports the operation's other
    /// diagnostics.
    fn drain_file_framing_events(&mut self, stores: &mut Universe<G>) {
        self.command
            .render_file_framing_events(&mut stores.command_context().expect("live generation"));
    }

    /// tex.web §1335 `final_cleanup`'s tail, run once a step has produced
    /// [`ReplayStep::End`]: closing every still-open paren, reporting
    /// unfinished conditionals, the "(see the transcript file..." note, and
    /// the `\dump`-outside-INITEX note, in that exact order. The first of
    /// those needs `self`'s job-framing state, which the free function
    /// `apply_cold_operation` that handles `ColdOperation::<G>::End` does not have; the
    /// other three used to run inside that free function and are moved here,
    /// not copied, so they run after the paren close instead of before it.
    fn end_of_job_final_cleanup(
        &mut self,
        stores: &mut Universe<G>,
        dump: bool,
        incomplete_conditions: Vec<tex_command::IncompleteCondition>,
    ) {
        self.job_body_effect_end = Some(stores.world().effect_pos());
        stores
            .world_mut()
            .begin_terminal_publication(tex_state::TerminalPublicationPhase::CloseOpenParens);
        crate::job::close_open_parens(stores);
        stores.world_mut().commit_terminal_publication();
        stores
            .world_mut()
            .begin_terminal_publication(tex_state::TerminalPublicationPhase::Notices);
        let group_depth = stores
            .command_context()
            .expect("final group-depth admission")
            .execution_group_depth();
        crate::job::report_unclosed_groups(stores, group_depth);
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
        stores.world_mut().commit_terminal_publication();
    }

    /// pdftex.web §§794--798 and §1600's PDF-writer diagnostics, after
    /// the `\end` ejection has made the last page and its navigation records
    /// visible. This is deliberately later than TeX82 §1335's generic
    /// `final_cleanup`, matching pdfTeX's `close_files_and_terminate` order.
    pub fn finalize_pdf_navigation(&mut self, stores: &mut Universe<G>) {
        if std::mem::replace(&mut self.pdf_navigation_finalized, true) {
            return;
        }
        let missing = {
            let context = stores.command_context().expect("live generation");
            if !self.command_profile().capabilities().supports_pdftex()
                || context.int_param(IntParam::PDF_OUTPUT) <= 0
                || context.int_param(IntParam::PDF_DRAFT_MODE) != 0
                || context.pdf_page_count() == 0
            {
                return;
            }
            context.detach_pdf_navigation_warnings()
        };
        if missing.is_empty() {
            return;
        }
        stores.world_mut().begin_terminal_publication(
            tex_state::TerminalPublicationPhase::PdfFinalizationNotices,
        );
        let reported = crate::job::report_pdf_navigation_warnings(stores, &missing);
        stores.world_mut().commit_terminal_publication();
        if reported {
            // Retained root-body runs return their selected effect slice
            // without exporting it to the World backend. Extend that slice
            // through pdfTeX's later close-files diagnostics only when this
            // pass actually emitted them.
            self.job_body_effect_end = Some(stores.world().effect_pos());
        }
    }

    fn resolve_font_resource(
        &self,
        scanned: ColdOperation<G>,
        stores: &mut Universe<G>,
    ) -> Result<ColdOperation<G>, Box<UnavailablePreparedResource<G>>> {
        let ColdOperation::<G>::FontDefinition {
            request, global, ..
        } = scanned
        else {
            return Ok(scanned);
        };
        stores.poison_dependency_region(TrackedRegionBarrier::UnsupportedHostCapability);
        let path = crate::canonical_font_resource_path(&request.name);
        let Some(resource) = self.capabilities.font(&path) else {
            return Err(Box::new(UnavailablePreparedResource::<G> {
                error: ExecError::MissingFont {
                    request: request.clone(),
                },
                scanned: Box::new(ColdOperation::<G>::FontDefinition {
                    request,
                    resource: Box::new(None),
                    global,
                }),
            }));
        };
        Ok(ColdOperation::<G>::FontDefinition {
            request,
            resource: Box::new(Some(resource)),
            global,
        })
    }

    fn resolve_input_stream_resource(
        &self,
        scanned: ColdOperation<G>,
        stores: &mut Universe<G>,
    ) -> Result<ColdOperation<G>, Box<UnavailablePreparedResource<G>>> {
        let ColdOperation::<G>::InputStream {
            mut request,
            resource: _,
        } = scanned
        else {
            return Ok(scanned);
        };
        stores.poison_dependency_region(TrackedRegionBarrier::UnsupportedHostCapability);
        let resource = match &mut request {
            RootedInputStreamRequest::Open { file_name, .. } => {
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
                match self.capabilities.input_probe_resource(&packed_name) {
                    Some(resource) => Some(resource.source().clone()),
                    None if self.capabilities.input_probe_is_unavailable(&packed_name) => None,
                    None => {
                        let error_request = tex_command::FileEnquiryRequest::new(
                            packed_name,
                            tex_command::FileEnquiryIntent::OpenInProbe,
                        );
                        return Err(Box::new(UnavailablePreparedResource::<G> {
                            error: ExecError::MissingInputProbe {
                                request: error_request,
                            },
                            scanned: Box::new(ColdOperation::<G>::InputStream {
                                request,
                                resource: None,
                            }),
                        }));
                    }
                }
            }
            RootedInputStreamRequest::Close { .. } | RootedInputStreamRequest::Read { .. } => None,
        };
        Ok(ColdOperation::<G>::InputStream { request, resource })
    }

    fn resolve_pdf_image_resource(
        &self,
        scanned: ColdOperation<G>,
        stores: &mut Universe<G>,
    ) -> Result<ColdOperation<G>, Box<UnavailablePreparedResource<G>>> {
        let ColdOperation::<G>::PdfXImage { mut request, .. } = scanned else {
            return Ok(scanned);
        };
        stores.poison_dependency_region(TrackedRegionBarrier::UnsupportedHostCapability);
        // pdfTeX checks \pdfoutput before it enters `scan_image`; in DVI
        // mode this must be the diagnostic, not a host-resource suspension.
        let mut context = stores.command_context().expect("live generation");
        if context.int_param(IntParam::PDF_OUTPUT) <= 0 {
            return Ok(ColdOperation::<G>::PdfXImage {
                request,
                resource: PdfImageResource::Unavailable,
            });
        }
        apply_pdf_image_compatibility_policy(&mut context);
        request.page_box = pdf_image_page_box(&context, &request);
        drop(context);
        let host_request = PdfImageRequest {
            name: request.name.clone(),
            width: request.width,
            height: request.height,
            depth: request.depth,
            page: request.page.clone(),
            color_space_object: request.color_space_object,
            page_box: request.page_box,
            page_box_explicit: request.page_box_explicit,
            attr: request.attr,
        };
        let Some(resource) = self.capabilities.pdf_image(&host_request) else {
            return Err(Box::new(UnavailablePreparedResource::<G> {
                error: ExecError::MissingPdfImage {
                    request: host_request,
                },
                scanned: Box::new(ColdOperation::<G>::PdfXImage {
                    request,
                    resource: PdfImageResource::Unavailable,
                }),
            }));
        };
        Ok(ColdOperation::<G>::PdfXImage { request, resource })
    }

    /// Registers and opens the one root source selected by the host before
    /// main control starts.  Source acquisition is deliberately
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

    /// Publishes source-open framing after root registration without
    /// consuming the first command, allowing hosts to checkpoint JobStart.
    pub fn flush_pending_file_framing(&mut self, stores: &mut Universe<G>) {
        self.drain_file_framing_events(stores);
    }

    /// Selects whether exhaustion of the registered root ends an authored
    /// fragment or enters TeX82 §360's missing-`\end` handling.
    pub fn set_root_completion_policy(&mut self, policy: RootCompletionPolicy) {
        self.root_completion = policy;
    }

    /// Performs one §360 terminal-input episode after a complete job reaches
    /// root EOF without `\end` or `\dump`.
    ///
    /// One accepted line is installed as a real terminal source and returned
    /// to main control for ordinary tokenization and fuel accounting. Fatal
    /// EOF is latched as §93's terminal `End`, so no driver can advance the
    /// exhausted source again.
    fn handle_root_end_of_input(&mut self, stores: &mut Universe<G>) -> ReplayStep {
        let previous_line_was_empty = self
            .terminal_line_was_empty
            .unwrap_or(self.startup_terminal_line.is_empty());
        let action = {
            let mut context = stores.command_context().expect("live generation");
            crate::job::prompt_for_more_input(
                &mut context,
                &self.startup_terminal_line,
                previous_line_was_empty,
            )
        };
        match action {
            crate::job::EndOfInputAction::Line(line) => {
                self.terminal_line_was_empty = Some(line.is_empty());
                self.command.set_terminal_context_line(&line);
                let source =
                    SourceRegistration::new(RegisteredSourceKind::Generated, line.into_bytes());
                let id = self
                    .command
                    .register_source(source)
                    .expect("finite command fuel bounds terminal source identities");
                self.command
                    .open_registered_source_as(id, tex_command::SourceNameClass::Terminal)
                    .expect("fresh terminal source must be openable");
                ReplayStep::Continue
            }
            crate::job::EndOfInputAction::Fatal(fatal) => self.succumb(fatal),
        }
    }

    /// Registers the startup root and immediately renders its §537 opening
    /// after the driver has opened the transcript.
    pub fn register_startup_root_source(
        &mut self,
        stores: &mut Universe<G>,
        source: SourceRegistration,
        startup_name: &str,
    ) -> Result<tex_state::SourceId, SourceRegistrationError> {
        let has_resolved_name = source.name().is_some();
        // The host supplies the already-acquired startup source, but TeX82
        // reached it through §§516--520's `end_name`. The following §537
        // `a_make_name_string` result is immediately flushed when it is last.
        let path = std::path::Path::new(startup_name);
        stores
            .command_context()
            .expect("startup accounting requires a live generation")
            .record_retained_strings(tex_state::RetainedStringAllocation {
                strings: 1
                    + usize::from(
                        path.parent()
                            .is_some_and(|area| !area.as_os_str().is_empty()),
                    )
                    + usize::from(path.extension().is_some()),
                characters: startup_name.len(),
            });
        let id = self.register_root_source(source)?;
        if has_resolved_name {
            self.command.render_file_framing_events(
                &mut stores.command_context().expect("live generation"),
            );
        } else {
            crate::job::open_startup_input_after_log(stores, startup_name);
        }
        Ok(id)
    }

    /// Accounts for a host-retained root that bypassed §526's live filename
    /// scan but still crossed §537's opened-name boundary.
    pub fn record_retained_startup_strings(
        &mut self,
        stores: &mut Universe<G>,
        requested_name: &str,
        resolved_name: Option<&str>,
    ) {
        if self.initex
            && let Some(stem) = std::path::Path::new(requested_name)
                .file_stem()
                .and_then(|value| value.to_str())
        {
            // §§534--536 retain the startup name component as `job_name`
            // and the transcript's opened name before §537 retains the
            // requested and host-resolved input names below.
            let mut context = stores
                .command_context()
                .expect("startup accounting requires a live generation");
            context.record_retained_strings(tex_state::RetainedStringAllocation::one(stem));
            context.record_retained_strings(tex_state::RetainedStringAllocation::one(&format!(
                "{stem}.log"
            )));
        }
        stores
            .command_context()
            .expect("startup accounting requires a live generation")
            .record_retained_strings(tex_state::RetainedStringAllocation::one(requested_name));
        if let Some(resolved_name) = resolved_name
            && resolved_name != requested_name
        {
            stores
                .command_context()
                .expect("startup accounting requires a live generation")
                .record_retained_strings(tex_state::RetainedStringAllocation::one(resolved_name));
        }
    }

    /// Refreshes executor-owned mode facts for the next processor borrow.
    ///
    /// This is intentionally call-local capability state rather than part of
    /// a command snapshot or durable session summary.
    pub fn refresh_host_capabilities(&mut self, stores: &mut Universe<G>) {
        self.capabilities
            .set_conditional_state(self.modes.conditional_state());
        self.capabilities.set_space_factor(
            matches!(
                self.modes.current_mode(),
                Mode::Horizontal | Mode::RestrictedHorizontal
            )
            .then(|| self.modes.current_list().space_factor()),
        );
        let ignored_depth = {
            let stores = stores.command_context().expect("live generation");
            crate::mode::ignored_depth(&stores)
        };
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
                    .unwrap_or(ignored_depth)
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
        let insertion_heights = stores
            .command_context()
            .expect("live generation")
            .page_insertions()
            .iter()
            .map(|insertion| (insertion.class(), insertion.height()))
            .collect::<Vec<_>>();
        self.capabilities
            .set_page_insertion_heights(insertion_heights);
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
    fn last_node_value(&self, stores: &mut Universe<G>) -> Option<tex_command::LastNodeItem> {
        let context = stores.command_context().expect("live generation");
        if is_outer_vertical(&self.modes) {
            return match crate::effective_tail::EffectiveTail::find(
                context.page_contributions().iter(),
            ) {
                Some(tail) => Self::classify_last_node(&context, tail.node()),
                None => match context.page_last_node_type() {
                    11 => context
                        .page_last_skip()
                        .map(tex_command::LastNodeItem::Glue),
                    12 => Some(tex_command::LastNodeItem::Kern(context.page_last_kern())),
                    13 => Some(tex_command::LastNodeItem::Penalty(
                        context.page_last_penalty(),
                    )),
                    _ => None,
                },
            };
        }
        if self.modes.current_list().pending_hchars().is_some() {
            return None;
        }
        crate::effective_tail::EffectiveTail::find(self.modes.current_list().nodes().iter())
            .and_then(|tail| Self::classify_last_node(&context, tail.node()))
    }

    /// e-TeX 2.6 `etex.ch` [26.424]'s `find_effective_tail` result for
    /// `\lastnodetype`.
    fn last_node_type_value(&self, stores: &mut Universe<G>) -> i32 {
        if is_outer_vertical(&self.modes) {
            let context = stores.command_context().expect("live generation");
            return crate::effective_tail::EffectiveTail::find(context.page_contributions().iter())
                .map_or_else(
                    || context.page_last_node_type(),
                    |tail| tail.node().etex_type(),
                );
        }
        // Batched horizontal characters are already semantic character nodes
        // even though Umber has not materialized their shaped run yet.
        if self.modes.current_list().pending_hchars().is_some() {
            return 0;
        }
        crate::effective_tail::EffectiveTail::find(self.modes.current_list().nodes().iter())
            .map_or(-1, |tail| tail.node().etex_type())
    }

    /// Classifies one real node as a `\lastpenalty`/`\lastkern`/`\lastskip`
    /// tail, resolving a glue node's stored specification and distinguishing
    /// TeX82's `mu_glue` subtype (an explicit `\mskip`, matched here by
    /// [`GlueKind::MuSkip`]) so `\lastskip` reads it at `mu_val` level. Any
    /// other node shape (including a character, which tex.web excludes via
    /// `is_char_node`) has no matching case, exactly like tex.web's
    /// `case cur_chr of ... end {there are no other cases}`.
    fn classify_last_node(
        stores: &tex_state::CommandContext<'_, G>,
        node: &Node,
    ) -> Option<tex_command::LastNodeItem> {
        match node {
            Node::Penalty(value) => Some(tex_command::LastNodeItem::Penalty(*value)),
            Node::Kern { amount, .. } => Some(tex_command::LastNodeItem::Kern(*amount)),
            Node::Glue {
                spec,
                kind: GlueKind::MuSkip,
                ..
            } => Some(tex_command::LastNodeItem::MuGlue(*spec)),
            Node::Glue { spec, .. } => Some(tex_command::LastNodeItem::Glue(*spec)),
            // TeX82 keeps a discretionary's no-break replacement nodes in
            // the surrounding list (§1119), immediately after the disc node.
            // Umber freezes that physical suffix as the disc's `replace`
            // child list, so §424's tail enquiry must look through the
            // container to preserve TeX's physical-tail view.  This is
            // intentionally distinct from §1105 deletion, which refuses to
            // remove a discretionary replacement suffix.
            Node::Disc { replace, .. } => stores
                .page_node_list(*replace)
                .expect("discretionary replacement belongs to the live page arena")
                .nodes()
                .to_vec()
                .pop()
                .and_then(|node| Self::classify_last_node(stores, &node)),
            _ => None,
        }
    }

    /// Lends the whole command machine at once, for helpers that build their
    /// own processor rather than being handed one. A caller that must keep
    /// another of main control's fields borrowed at the same time builds the
    /// bundle from those fields directly instead.
    fn command_machine(&mut self) -> CommandMachine<'_, G> {
        CommandMachine {
            state: &mut self.command,
            fuel: self.fuel.fuel_mut(),
            capabilities: &mut self.capabilities,
            observations: &mut self.operation_observations,
            assignment_receipts: None,
            shown_mode: &mut self.shown_mode,
            initex: self.initex,
            emit_dvi_override: self.emit_dvi_override,
            immediate_prints: &mut self.immediate_prints,
            prepared_shipout: &mut self.prepared_shipout,
        }
    }

    /// Takes TeX82 §1030's parking decision for the step just scanned, and
    /// clears the outgoing parking so nested episodes run from `big_switch`.
    ///
    /// Every step driver takes this before applying its step and gives it back
    /// to [`Self::resume_main_control_parking`] afterwards. The rule is stated
    /// here once: three drivers used to spell it out inline, and a rule spelled
    /// three times is a rule two of them can be missing.
    fn suspend_main_control_parking<T, D>(
        &mut self,
        scanned: &ColdOperation<G, T, D>,
    ) -> MainControlParking {
        let parking = MainControlParking {
            character: match scanned {
                ColdOperation::Character {
                    ch,
                    cat: Catcode::Letter | Catcode::Other,
                    ..
                } => Some(*ch),
                ColdOperation::CharacterCode { value, .. } => {
                    u32::try_from(*value).ok().and_then(char::from_u32)
                }
                _ => None,
            },
            resumes_interrupted_fetch: matches!(scanned, ColdOperation::AlignmentTemplateEntered),
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
    fn resume_main_control_parking(
        &mut self,
        parking: MainControlParking,
        stores: &mut Universe<G>,
    ) {
        if parking.resumes_interrupted_fetch {
            return;
        }
        let context = stores.command_context().expect("live generation");
        self.main_loop_active = parking.character.is_some_and(|character| {
            matches!(
                self.modes.current_mode(),
                Mode::Horizontal | Mode::RestrictedHorizontal
            ) && u8::try_from(u32::from(character)).ok().is_some_and(|code| {
                context
                    .font_char_metrics(context.current_font(), code)
                    .is_some()
            })
        });
    }

    fn publish_pdf_fatal_error(
        stores: &mut Universe<G>,
        error: &ExecError,
    ) -> Result<(), ExecError> {
        if error.is_pdftex_navigation_fatal() {
            crate::job::report_pdf_fatal_error(stores, &error.to_string());
            stores.publish_effect_prefix(stores.world().effect_pos())?;
        }
        Ok(())
    }

    fn finish_resource_preflight_failure(
        &mut self,
        stores: &mut Universe<G>,
        error: ExecError,
    ) -> Result<StepResult, ExecError> {
        let error = {
            let mut context = stores.command_context().expect("live generation");
            error.freeze_diagnostic_origin(&mut context, self.command.diagnostic_input_context(8))
        };
        if let Some(fatal) = error.as_fatal() {
            let context = self
                .command
                .output_open_context(&stores.command_context().expect("live generation"));
            crate::diagnostics::report_irrecoverable_error(stores, fatal, context);
            self.captured_fatal_origin = match &error {
                ExecError::Captured { site, frozen, .. } if fatal != FatalError::TooManyErrors => {
                    Some((
                        *site,
                        frozen
                            .as_deref()
                            .and_then(|evidence| evidence.origin.clone()),
                        self.first_causal_context.clone().or_else(|| {
                            frozen
                                .as_deref()
                                .and_then(|evidence| evidence.context.clone())
                        }),
                    ))
                }
                _ => None,
            };
            self.observe_committed([
                CommandObservation::Diagnostic(fatal.record()),
                CommandObservation::Effect(engine_termination_effect()),
            ]);
            let evidence_error =
                self.admit_observed_receipt(stores, OperationTermination::Fatal(fatal));
            let terminal = self.succumb(fatal);
            return evidence_error.map_or(Ok(StepResult::Progress(terminal)), Err);
        }
        match error {
            ExecError::Captured {
                error,
                site,
                frozen,
            } => match *error {
                ExecError::MissingInput {
                    name,
                    original_name,
                } => {
                    self.pending_resource_site = site.primary_origin();
                    Ok(self.observed_suspension(ResourceNeed::Input {
                        name,
                        original_name,
                    }))
                }
                ExecError::MissingInputProbe { request } => {
                    self.pending_resource_site = site.primary_origin();
                    Ok(self.observed_suspension(ResourceNeed::InputProbe { request }))
                }
                error => Err(ExecError::Captured {
                    error: Box::new(error),
                    site,
                    frozen,
                }),
            },
            ExecError::MissingInput {
                name,
                original_name,
            } => Ok(self.observed_suspension(ResourceNeed::Input {
                name,
                original_name,
            })),
            ExecError::MissingInputProbe { request } => {
                Ok(self.observed_suspension(ResourceNeed::InputProbe { request }))
            }
            ExecError::MissingFont { request } => {
                Ok(self.observed_suspension(ResourceNeed::Font { request }))
            }
            ExecError::MissingPdfImage { request } => {
                Ok(self.observed_suspension(ResourceNeed::PdfImage { request }))
            }
            error => Err(error),
        }
    }

    fn observed_suspension(&mut self, need: ResourceNeed) -> StepResult {
        if let Some(pending) = self.operation_observations.as_mut() {
            pending.record_resource(need.clone());
            pending
                .receipt
                .set_termination(OperationTermination::Suspended);
        }
        StepResult::Suspended(need)
    }

    /// Returns the command site retained for the most recent resource need.
    #[must_use]
    pub const fn pending_resource_site(&self) -> Option<OriginId> {
        self.pending_resource_site
    }

    /// Drains committed shipout receipts in artifact order.
    ///
    /// Each plan was prepared during shipout and is retained only after the
    /// enclosing direct operation commits; finalizers must not re-lower these
    /// pages from artifact bytes.
    #[must_use]
    pub fn take_prepared_dvi_pages(&mut self) -> Vec<crate::dispatch::PreparedDviPage> {
        take_prepared_dvi_pages(&mut self.prepared_dvi_pages)
    }

    /// Drains named boundaries that became safe during committed direct
    /// operations. This is deliberately an event receipt, not a request for
    /// the host to inspect modes or dispatch source tokens.
    #[must_use]
    pub fn take_completed_boundaries(&mut self) -> Vec<crate::EngineBoundary> {
        std::mem::take(&mut self.completed_boundaries)
    }

    /// Records newly committed artifacts and releases their one outermost
    /// checkpoint boundary only after all continuation-owning work unwinds.
    fn finish_shipout_publication(
        &mut self,
        artifact_count: usize,
        _effect_count: usize,
        stores: &mut Universe<G>,
    ) {
        self.pending_shipout_boundary |= stores.world().artifact_commits().len() != artifact_count;
        if self.pending_shipout_boundary
            && !self.boxes.output_routine_active
            && self.modes.depth() == 1
            && stores
                .command_context()
                .expect("live generation")
                .execution_group_depth()
                == 0
        {
            self.completed_boundaries
                .push(crate::EngineBoundary::ShipoutComplete);
            self.pending_shipout_boundary = false;
        }
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
    pub fn apply_alignment_request(
        &mut self,
        stores: &mut Universe<G>,
        request: AlignmentRequest,
    ) -> Result<(), ExecError> {
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
        };
        self.command
            .apply_alignment_request(&stores.command_context().expect("live generation"), request)
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
                    .apply_alignment_request(
                        &stores.command_context().expect("live generation"),
                        AlignmentRequest::Resume(outer.identity),
                    )
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
    fn enter_main_control(&mut self, stores: &mut Universe<G>) -> bool {
        // Seeds `line` before the first command is delivered; every step
        // republishes it after delivery (see `apply_operation`).
        if std::mem::replace(&mut self.main_control_entered, true) {
            return false;
        }
        let mut context = stores.command_context().expect("live generation");
        schedule_everyjob(&mut self.command, &mut context);
        true
    }

    /// Appends already-committed records to the operation's commit buffer.
    /// They are published only when the whole operation commits.
    fn observe_committed(&mut self, records: impl IntoIterator<Item = CommandObservation>) {
        if let Some(buffer) = self.operation_observations.as_mut() {
            buffer.extend(records);
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
        stores: &mut Universe<G>,
    ) -> Result<ReplayStep, ExecError> {
        match self.execute_operation(
            stores,
            OperationDelivery::<G>::Alignment(alignment),
            OperationTransaction::Alignment,
            1,
            None,
        )? {
            StepResult::Progress(step) => Ok(step),
            StepResult::Suspended(_) => Err(ExecError::MissingToken {
                context: "alignment resource",
            }),
        }
    }

    /// Applies the scanned steps `MainControl` owns itself instead of
    /// routing through [`apply_cold_operation`], and hands every other step back
    /// unchanged.
    ///
    /// Every step-delivery entry point -- unobserved, observed, and alignment
    /// -- routes through this single match, so the host-applied set is stated
    /// exactly once. It used to be an `if let` chain copied into each entry
    /// point, and the observed copy was missing [`ColdOperation::<G>::MathShift`]:
    /// an observed `$` fell through to `apply_cold_operation`'s `unreachable!()`
    /// while the identical unobserved `$` was applied correctly
    /// (umber2-johp.118). Add a host-applied step here, never at a call site.
    ///
    /// Each arm runs nested command-owned episodes whose own last character
    /// would otherwise leave `main_loop_active` set. None of them is a §1030
    /// `main_loop` entry, so all of them resume at `big_switch`.
    fn apply_host_owned_step(
        &mut self,
        scanned: PreparedColdCommand<G>,
        stores: &mut Universe<G>,
    ) -> ControlFlow<Result<ReplayStep, ExecError>, PreparedColdCommand<G>> {
        let applied = match scanned {
            ColdOperation::ReplayCompleted(episode) => {
                self.completed_replay_episode = Some(episode);
                Ok(ReplayStep::Continue)
            }
            ColdOperation::Math(request) => self.apply_math_request(request, stores),
            ColdOperation::DisplayAlignmentRecovery => {
                self.recover_display_alignment_closer(stores)
            }
            ColdOperation::MathDelimiter(boundary) => self.apply_math_delimiter(boundary, stores),
            // TeX82 §1137's `hmode+math_shift: init_math` and §1193's
            // `mmode+math_shift: if cur_group=math_shift_group then
            // after_math else off_save`. §1090 backs a `vmode+math_shift` up
            // and runs `new_graf(true)` first, so vertical mode never reaches
            // this step.
            ColdOperation::MathShift { pairing } => self.apply_math_shift(pairing, stores),
            ColdOperation::DiscretionaryOpening(opening) => {
                self.begin_discretionary(opening, stores)
            }
            ColdOperation::DiscretionaryPartEnd => self.finish_discretionary_part(stores),
            ColdOperation::DiscretionaryHyphen { origin } => {
                self.apply_discretionary_hyphen(origin, stores)
            }
            // TeX82 §1123's `make_accent` runs §1270's `do_assignments`
            // between the accent code and §1124's base character, so it
            // executes whole commands of its own before it can finish.
            ColdOperation::Accent(accent) => self.apply_accent(accent, stores),
            ColdOperation::InputStream { request, resource } => match request {
                RootedInputStreamRequest::Open {
                    stream, file_name, ..
                } => {
                    let slot = replay_stream_slot(stream);
                    let packed_name = file_name.packed();
                    stores.world_mut().close_in(slot);
                    if let Some(resource) = resource {
                        if let Err(error) = stores
                            .world_mut()
                            .set_memory_file(&packed_name, resource.bytes().to_vec())
                        {
                            return ControlFlow::Break(Err(error.into()));
                        }
                        let content = match InputReadState::read_input_file(
                            &mut stores.input_open_context(),
                            std::path::Path::new(&packed_name),
                        ) {
                            Ok(content) => content,
                            Err(error) => return ControlFlow::Break(Err(error.into())),
                        };
                        if let Err(error) = stores.world_mut().open_in_content(slot, &content) {
                            return ControlFlow::Break(Err(error.into()));
                        }
                    }
                    Ok(ReplayStep::Continue)
                }
                RootedInputStreamRequest::Close { stream, .. } => {
                    stores.world_mut().close_in(replay_stream_slot(stream));
                    Ok(ReplayStep::Continue)
                }
                request @ RootedInputStreamRequest::Read { .. } => {
                    return ControlFlow::Continue(ColdOperation::InputStream { request, resource });
                }
            },
            ColdOperation::PdfSetRandomSeed { seed } => {
                stores.world_mut().set_pdf_random_seed(seed);
                Ok(ReplayStep::Continue)
            }
            ColdOperation::PdfResetTimer => {
                stores.world_mut().reset_pdf_timer();
                Ok(ReplayStep::Continue)
            }
            scanned => return ControlFlow::Continue(scanned),
        };
        // TeX82 §§994/1005 run `fire_up` inside the host-owned operation's
        // `build_page` call.  In particular, §1200's display resumption has
        // already installed its horizontal level when page fire-up enters
        // §1025's output routine.  Do this before handing the completed step
        // back to any driver: the unobserved driver returns directly here,
        // while observed drivers have a later publication-only tail.
        let applied = applied.and_then(|step| {
            self.fire_pending_page_output(stores)?;
            Ok(step)
        });
        self.main_loop_active = false;
        ControlFlow::Break(applied)
    }

    /// The page/output tail every step ends with, for the host-owned steps
    /// [`Self::apply_host_owned_step`] applies instead of `apply_cold_operation`.
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
        outer_paragraph_was_active: bool,
        artifact_count: usize,
        _effect_count: usize,
        _prepared_page_count: usize,
        stores: &mut Universe<G>,
    ) -> Result<ReplayStep, ExecError> {
        let applied = match applied {
            Ok(applied) => applied,
            Err(error) => {
                self.page_output_observations.clear();
                return Err(error);
            }
        };
        // A host-owned application can itself run `fire_pending_page_output`
        // before reaching this common tail. Retain that already-opened
        // episode here instead of asking only whether another fire-up is
        // currently pending; otherwise its observations survive into the
        // next command step and that command's raw delivery overtakes them.
        let page_fire_up_pending = stores
            .command_context()
            .expect("live generation")
            .page_fire_up()
            .is_some();
        let opens_output_batch = !self.page_output_observations.is_empty()
            || (page_fire_up_pending && !self.boxes.output_routine_active);
        self.fire_pending_page_output(stores)?;
        {
            #[cfg(feature = "profiling")]
            tex_state::measurement::record_hot_core_phase(
                tex_state::measurement::HotCorePhase::EvidencePublication,
            );
            #[cfg(feature = "profiling")]
            let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
                tex_state::measurement::HotCoreAllocationOwner::EvidencePublication,
            );
            // Host-owned transitions are still complete main-control steps.
            // In particular, §1145's display-math `init_math` installs
            // `every_display` here, and §323 traces that list before the next
            // command is fetched. Leaving the push queued until an ordinary
            // step reverses those events: the hook's final command is traced
            // and executed before its own `begin_token_list` trace.
            let mut records: Vec<CommandObservation> = self
                .command
                .publish_named_token_list_pushes(
                    &mut stores.command_context().expect("live generation"),
                )
                .into_iter()
                .map(CommandObservation::Input)
                .collect();
            if opens_output_batch {
                // Same order as the ordinary tail: the named token-list push
                // command state held across the transition, then the shipouts
                // it committed, then the episode's own records.
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
                self.page_output_observations.append_to(&mut records);
            }
            self.observe_committed(records);
            self.page_output_observations.clear();
        }
        self.finish_shipout_publication(artifact_count, _effect_count, stores);
        self.finish_paragraph_boundary(outer_paragraph_was_active, stores);
        Ok(applied)
    }

    /// Publishes the ordinary cold paragraph boundary after `end_graf`.
    fn finish_paragraph_boundary(
        &mut self,
        outer_paragraph_was_active: bool,
        _stores: &mut Universe<G>,
    ) {
        if outer_paragraph_was_active
            && self.modes.current_mode() == Mode::Vertical
            && self.modes.depth() == 1
        {
            self.completed_boundaries
                .push(crate::EngineBoundary::OuterParagraphEnd);
        }
    }

    /// Enters TeX82 §1117's live `disc_group` after the command processor has
    /// consumed only its opening brace.
    fn begin_discretionary(
        &mut self,
        _opening: ScannedDiscretionaryOpening,
        stores: &mut Universe<G>,
    ) -> Result<ReplayStep, ExecError> {
        if matches!(
            self.modes.current_mode(),
            Mode::Vertical | Mode::InternalVertical
        ) {
            let mut context = stores.command_context().expect("live generation");
            start_paragraph(&mut self.command, &mut self.modes, &mut context, true)?;
        }
        {
            let mut context = stores.command_context().expect("live generation");
            crate::box_runtime::flush_pending_hchars_with_fuel(
                &mut self.modes,
                &mut context,
                self.fuel.fuel_mut(),
            )?;
        }
        self.open_discretionary_part(stores)?;
        self.active_discretionaries.push(ActiveDiscretionary {
            parts: Vec::new(),
            rejected: false,
        });
        Ok(ReplayStep::Continue)
    }

    fn open_discretionary_part(&mut self, stores: &mut Universe<G>) -> Result<(), ExecError> {
        // TeX82 §216 checks nest capacity before saving the current semantic
        // level. Fatal overflow is committed by main control, so
        // this fallible operation must precede both halves of the live
        // discretionary lifecycle: no rejected opener may leave a disc_group
        // without its restricted-horizontal mode.
        self.modes.push_at_line(
            Mode::RestrictedHorizontal,
            self.command
                .current_file_line_number()
                .try_into()
                .unwrap_or(i32::MAX),
        )?;
        let mut context = stores.command_context().expect("live generation");
        enter_group(&mut context, &mut self.command, GroupKind::Disc);
        Ok(())
    }

    /// Implements §1120's `build_discretionary`: finish the current live
    /// restricted-horizontal list, `unsave`, and either scan the next opening
    /// brace or append the completed three-part node.
    fn finish_discretionary_part(
        &mut self,
        stores: &mut Universe<G>,
    ) -> Result<ReplayStep, ExecError> {
        let level = {
            let mut context = stores.command_context().expect("live generation");
            crate::box_runtime::commit_current_list(
                &mut self.modes,
                &mut context,
                self.fuel.fuel_mut(),
            )?
        };
        // TeX82 §1121 advances `q` across the admissible prefix and, on the
        // first forbidden node `p`, severs `link(q)`. Thus the prefix remains
        // this discretionary part while `show_box(p)` reports and flushes the
        // entire suffix beginning at the offending node.
        let first_forbidden = level.list().nodes().iter().position(|node| {
            !matches!(
                node,
                Node::Char { .. }
                    | Node::Lig { .. }
                    | Node::Kern { .. }
                    | Node::Rule { .. }
                    | Node::HList(_)
                    | Node::VList(_)
            )
        });
        let prefix_end = first_forbidden.unwrap_or(level.list().nodes().len());
        let (nodes, deleted) = {
            let context = stores.command_context().expect("live generation");
            let mut stores = LinearCommandContext::new(context);
            let nodes = stores.publish_page_nodes(level.list().nodes()[..prefix_end].to_vec());
            let deleted = first_forbidden
                .map(|index| stores.publish_page_nodes(level.list().nodes()[index..].to_vec()));
            let aftergroup = leave_group_payloads(&mut stores, &mut self.command, GroupKind::Disc)
                .map_err(|_| ExecError::MissingToken {
                    context: "discretionary group",
                })?;
            schedule_aftergroup(&mut self.command_machine(), &mut stores, aftergroup)?;
            (nodes, deleted)
        };

        let (part_count, replacement_too_long) = {
            let active = self
                .active_discretionaries
                .last_mut()
                .ok_or(ExecError::MissingToken {
                    context: "active discretionary",
                })?;
            active.parts.push(nodes);
            let part_count = active.parts.len();
            let replacement_too_long = part_count == 3 && prefix_end > 127;
            active.rejected |= replacement_too_long;
            (part_count, replacement_too_long)
        };
        if let Some(deleted) = deleted {
            let mut stores = stores.command_context().expect("live generation");
            let context = self.command.output_open_context(&stores);
            report_improper_discretionary(&mut stores, deleted, context)?;
        }
        if replacement_too_long {
            let mut stores = stores.command_context().expect("live generation");
            let context = self.command.output_open_context(&stores);
            crate::error_report::report_error(
                &mut stores,
                "Discretionary list is too long",
                &["Wow---I never thought anybody would tweak me here."],
                context,
            )?;
        }
        if part_count < 3 {
            let mut diagnostics = Vec::new();
            {
                let mut processor = command_processor(
                    &mut self.command,
                    self.fuel.fuel_mut(),
                    &mut self.capabilities,
                    &mut self.operation_observations,
                    stores.command_context().expect("live generation"),
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
            self.capture_first_causal_context(stores, &diagnostics);
            {
                let mut context = stores.command_context().expect("live generation");
                report_pending_diagnostics(&mut context, diagnostics)?;
            }
            self.open_discretionary_part(stores)?;
            return Ok(ReplayStep::Continue);
        }
        let active = self
            .active_discretionaries
            .pop()
            .expect("three parts require an active discretionary");
        if active.rejected {
            return Ok(ReplayStep::Continue);
        }
        let [pre, post, mut replace]: [tex_state::node_arena::PageListId; 3] = active
            .parts
            .try_into()
            .expect("discretionary completes after exactly three parts");
        if matches!(self.modes.current_mode(), Mode::Math | Mode::DisplayMath)
            && !replace.is_empty()
        {
            // TeX82 §1120 diagnoses and deletes only a nonempty third part
            // in math mode; the discretionary and its first two parts survive.
            let mut command_context = stores.command_context().expect("live generation");
            let context = self.command.output_open_context(&command_context);
            report_escaped_error(
                &mut command_context,
                "Illegal math ",
                "discretionary",
                "",
                &[
                    "Sorry: The third part of a discretionary break must be",
                    "empty, in math formulas. I had to delete your third part.",
                ],
                context,
            )?;
            replace = tex_state::node_arena::PageListId::empty();
        }
        let physical_replace_count = stores
            .command_context()
            .expect("live generation")
            .page_node_list(replace)
            .expect("discretionary replacement is a live page list")
            .len()
            .try_into()
            .expect("TeX discretionary replacement count fits a quarterword");
        self.modes.current_list_mutation().push(Node::Disc {
            kind: DiscKind::Discretionary,
            pre,
            post,
            replace,
            physical_replace_count,
        });
        Ok(ReplayStep::Continue)
    }

    /// Executes TeX82 §1113's `append_discretionary` shorthand for `\-`.
    fn apply_discretionary_hyphen(
        &mut self,
        origin: OriginId,
        stores: &mut Universe<G>,
    ) -> Result<ReplayStep, ExecError> {
        if matches!(
            self.modes.current_mode(),
            Mode::Vertical | Mode::InternalVertical
        ) {
            let mut context = stores.command_context().expect("live generation");
            start_paragraph(&mut self.command, &mut self.modes, &mut context, true)?;
        }
        let pre = {
            let mut stores = stores.command_context().expect("live generation");
            crate::box_runtime::flush_pending_hchars_with_fuel(
                &mut self.modes,
                &mut stores,
                self.fuel.fuel_mut(),
            )?;
            let font = stores.current_font();
            match u8::try_from(stores.font_hyphen_char(font)) {
                Ok(hyphen) if stores.font_char_metrics(font, hyphen).is_some() => stores
                    .publish_page_nodes(vec![Node::Char {
                        font,
                        ch: char::from(hyphen),
                        origin,
                    }]),
                Ok(hyphen) => {
                    // TeX82 §1113 delegates the in-range hyphen to §581's
                    // `new_character`: an absent glyph warns and leaves the
                    // pre-break list empty.
                    crate::diagnostics::report_missing_character_warning(
                        &mut stores,
                        font,
                        char::from(hyphen),
                        self.command_profile() == CommandProfile::ETEX26,
                    );
                    stores.publish_page_nodes(Vec::new())
                }
                Err(_) => stores.publish_page_nodes(Vec::new()),
            }
        };
        let empty = tex_state::node_arena::PageListId::empty();
        self.modes.current_list_mutation().push(Node::Disc {
            kind: DiscKind::ExplicitHyphen,
            pre,
            post: empty.clone(),
            replace: empty,
            physical_replace_count: 0,
        });
        Ok(ReplayStep::Continue)
    }

    /// Records TeX's checked save-stack high-water projection after one
    /// direct main-control operation.
    fn record_save_stack_usage(&mut self, stores: &mut Universe<G>) {
        // TeX82 §§645/1083 keeps ordinary box specs immediately below their
        // §273 boundaries. Vcenters and insertions deliberately have smaller
        // projections (§§1167/1099), so derive the words from each live kind.
        let box_spec_words = self
            .boxes
            .active_boxes
            .iter()
            .map(|active| active.kind.save_stack_spec_words())
            .fold(0_usize, usize::saturating_add);
        let checked = stores
            .command_context()
            .expect("save-stack admission")
            .execution_group_depth()
            .saturating_add(box_spec_words);
        self.max_save_stack = self.max_save_stack.max(checked);
    }

    #[allow(clippy::too_many_arguments)]
    fn episode_commit_boundary(
        &self,
        stores: &Universe<G>,
        applied: &Result<ReplayStep, ExecError>,
        operations: usize,
        max_operations: usize,
        initial_boundaries: usize,
        initial_effect_pos: tex_state::EffectPos,
        initial_artifacts: usize,
        initial_format_dump: bool,
        initial_diagnostic: bool,
        initial_error_count: i32,
        tracked: bool,
    ) -> Option<crate::EpisodeCommitBoundary> {
        #[cfg(feature = "profiling")]
        tex_state::measurement::record_hot_core_phase(
            tex_state::measurement::HotCorePhase::BarrierDecision,
        );
        if applied.is_err() {
            return None;
        }
        if self.dumped_format.is_some() != initial_format_dump {
            return Some(crate::EpisodeCommitBoundary::Semantic(
                crate::SemanticEpisodeBarrier::Format,
            ));
        }
        if self.fatal.is_some() {
            return Some(crate::EpisodeCommitBoundary::Semantic(
                crate::SemanticEpisodeBarrier::Diagnostic,
            ));
        }
        if self.first_causal_context.is_some() != initial_diagnostic
            || stores.world().error_channel().error_count() != initial_error_count
        {
            return Some(crate::EpisodeCommitBoundary::Semantic(
                crate::SemanticEpisodeBarrier::Diagnostic,
            ));
        }
        if matches!(applied, Ok(ReplayStep::End | ReplayStep::EndOfInput)) {
            return Some(crate::EpisodeCommitBoundary::Terminal);
        }
        if stores.world().artifact_commits().len() != initial_artifacts {
            return Some(crate::EpisodeCommitBoundary::Semantic(
                crate::SemanticEpisodeBarrier::Output,
            ));
        }
        if self.completed_boundaries.len() != initial_boundaries {
            let boundary = self.completed_boundaries[initial_boundaries];
            return Some(crate::EpisodeCommitBoundary::NamedCheckpoint(boundary));
        }
        if stores.world().effect_pos() != initial_effect_pos {
            return Some(crate::EpisodeCommitBoundary::Semantic(
                crate::SemanticEpisodeBarrier::Effect,
            ));
        }
        if self.operation_observations.is_some() {
            return Some(crate::EpisodeCommitBoundary::Semantic(
                crate::SemanticEpisodeBarrier::Observer,
            ));
        }
        if tracked {
            return Some(crate::EpisodeCommitBoundary::Semantic(
                crate::SemanticEpisodeBarrier::StateIdentity,
            ));
        }
        (operations >= max_operations).then_some(crate::EpisodeCommitBoundary::SliceLimit)
    }

    fn command_requires_transaction(
        &self,
        stores: &mut Universe<G>,
        capabilities: crate::transaction_protocol::CommandCapabilities,
        delivery: &OperationDelivery<G>,
    ) -> bool {
        if !matches!(
            capabilities.preflight(),
            crate::transaction_protocol::CommandPreflight::Ordinary(_)
        ) {
            return true;
        }
        // pdfTeX's `check_pdfoutput` fails before operand scanning. ErrorStop
        // can change `\pdfoutput` and retry that untouched command, so DVI
        // mode retains the retry transaction; an enabled ordinary PDF
        // command cannot take that recovery edge and commits directly.
        if capabilities
            .mutation()
            .contains(crate::transaction_protocol::StateOwners::PDF)
            && stores
                .command_context()
                .expect("live generation")
                .int_param(IntParam::PDF_OUTPUT)
                <= 0
        {
            return true;
        }
        if matches!(
            self.modes.current_mode(),
            Mode::Vertical | Mode::InternalVertical
        ) && matches!(
            delivery,
                OperationDelivery::<G>::Replay(Some(command))
                if matches!(
                    command.meaning(),
                    ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                        UnexpandablePrimitive::PdfStartLink
                    ))
                )
        ) {
            return true;
        }
        if matches!(delivery, OperationDelivery::<G>::Expanding { .. }) {
            return true;
        }
        // A right brace normally owns only the save stack and group state,
        // but the brace that packages an active box can also run the page or
        // explicit-shipout pipeline. Route that dynamic continuation through
        // typed preparation so its PDF, effect, and output capabilities are
        // known before direct semantic apply. Braces inside the box remain
        // ordinary because their innermost save-stack group does not name the
        // box body.
        if let OperationDelivery::<G>::Replay(Some(command)) = delivery
            && matches!(
                command.meaning(),
                ResolvedMeaning::Static(Meaning::CharToken {
                    cat: Catcode::EndGroup,
                    ..
                })
            )
            && self.boxes.active_boxes.last().is_some_and(|active| {
                stores
                    .command_context()
                    .expect("live generation")
                    .innermost_group_kind()
                    == Some(active.group_kind)
            })
        {
            return true;
        }
        matches!(
            delivery,
            OperationDelivery::<G>::Replay(Some(command))
                if matches!(
                    command.meaning(),
                    ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                        UnexpandablePrimitive::Global
                            | UnexpandablePrimitive::Long
                            | UnexpandablePrimitive::Outer
                            | UnexpandablePrimitive::Protected
                            | UnexpandablePrimitive::IgnoreSpaces
                            | UnexpandablePrimitive::NoBoundary
                    ))
                )
        )
    }

    fn record_direct_episode_commit(
        &mut self,
        stores: &mut Universe<G>,
        operations: usize,
        boundary: crate::EpisodeCommitBoundary,
        initial_artifacts: usize,
        initial_boundaries: usize,
        initial_effect_pos: tex_state::EffectPos,
    ) {
        if matches!(
            boundary,
            crate::EpisodeCommitBoundary::NamedCheckpoint(_)
                | crate::EpisodeCommitBoundary::Terminal
                | crate::EpisodeCommitBoundary::Semantic(
                    crate::SemanticEpisodeBarrier::Effect
                        | crate::SemanticEpisodeBarrier::Observer
                        | crate::SemanticEpisodeBarrier::Diagnostic
                        | crate::SemanticEpisodeBarrier::Format
                        | crate::SemanticEpisodeBarrier::Output
                        | crate::SemanticEpisodeBarrier::StateIdentity
                )
        ) {
            self.modes.publish_node_sidecars(stores);
        }
        self.episode_telemetry
            .record_commit(crate::EpisodeCommit::new(
                operations
                    .try_into()
                    .expect("bounded episode operation count fits u16"),
                boundary,
            ));
        if stores.world().artifact_commits().len() != initial_artifacts
            && boundary
                != crate::EpisodeCommitBoundary::Semantic(crate::SemanticEpisodeBarrier::Output)
        {
            self.episode_telemetry
                .record_semantic_barrier(crate::SemanticEpisodeBarrier::Output);
        }
        if self.completed_boundaries.len() != initial_boundaries
            && !matches!(boundary, crate::EpisodeCommitBoundary::NamedCheckpoint(_))
        {
            self.episode_telemetry
                .record_semantic_barrier(crate::SemanticEpisodeBarrier::Checkpoint);
        }
        if stores.world().effect_pos() != initial_effect_pos
            && boundary
                != crate::EpisodeCommitBoundary::Semantic(crate::SemanticEpisodeBarrier::Effect)
        {
            self.episode_telemetry
                .record_semantic_barrier(crate::SemanticEpisodeBarrier::Effect);
        }
        self.advance_telemetry.commits += 1;
    }

    fn begin_direct_operation(&mut self, stores: &Universe<G>) -> DirectOperationMark<G> {
        DirectOperationMark {
            state: stores
                .journal_cursor()
                .expect("live generation has a state journal"),
            mode: self.modes.begin_journal(),
            attempt: self.command.begin_attempt_operation(),
            page: stores.page_node_cursor(),
        }
    }

    fn commit_direct_operation(&mut self, stores: &mut Universe<G>, mark: DirectOperationMark<G>) {
        self.finish_direct_operation(stores, mark, DirectAttemptDisposition::ReclaimUnreachable);
    }

    fn retain_direct_operation_for_retry(
        &mut self,
        stores: &mut Universe<G>,
        mark: DirectOperationMark<G>,
    ) {
        self.finish_direct_operation(stores, mark, DirectAttemptDisposition::RetainForRetry);
    }

    fn finish_direct_operation(
        &mut self,
        _stores: &mut Universe<G>,
        mark: DirectOperationMark<G>,
        attempt: DirectAttemptDisposition,
    ) {
        self.modes
            .commit_journal(mark.mode)
            .expect("direct operation owns the top mode journal frame");
        if attempt == DirectAttemptDisposition::ReclaimUnreachable {
            self.command
                .reclaim_attempt_operation(mark.attempt)
                .expect("direct operation owns valid command-attempt coordinates");
        }
    }

    fn discard_direct_operation(&mut self, stores: &mut Universe<G>, mark: DirectOperationMark<G>) {
        stores
            .restore_state(mark.state)
            .expect("direct operation state cursor belongs to the live generation");
        self.modes
            .rollback_journal(mark.mode)
            .expect("direct operation owns the top mode journal frame");
        self.command
            .reclaim_attempt_operation(mark.attempt)
            .expect("rollback roots own valid command-attempt coordinates");
        stores
            .truncate_page_nodes(mark.page)
            .expect("direct operation page cursor belongs to the live page arena");
    }

    fn finish_direct_failure(
        &mut self,
        stores: &mut Universe<G>,
        operation_mark: DirectOperationMark<G>,
        error: ExecError,
        context: DirectFailureContext,
    ) -> Result<StepResult, ExecError> {
        let DirectFailureContext {
            operations,
            initial_artifacts,
            initial_boundaries,
            initial_effect_pos,
        } = context;
        let error = {
            let mut stores = stores.command_context().expect("live generation");
            error.freeze_diagnostic_origin(&mut stores, self.command.diagnostic_input_context(8))
        };
        let Some(fatal) = error.as_fatal() else {
            self.discard_direct_operation(stores, operation_mark);
            return Err(error);
        };
        let context = self
            .command
            .output_open_context(&stores.command_context().expect("live generation"));
        crate::diagnostics::report_irrecoverable_error(stores, fatal, context);
        self.captured_fatal_origin = match &error {
            ExecError::Captured { site, frozen, .. } if fatal != FatalError::TooManyErrors => {
                Some((
                    site.clone(),
                    frozen
                        .as_deref()
                        .and_then(|evidence| evidence.origin.clone()),
                    self.first_causal_context.clone().or_else(|| {
                        frozen
                            .as_deref()
                            .and_then(|evidence| evidence.context.clone())
                    }),
                ))
            }
            _ => None,
        };
        self.observe_committed([
            CommandObservation::Diagnostic(fatal.record()),
            CommandObservation::Effect(engine_termination_effect()),
        ]);
        let evidence_error =
            self.admit_observed_receipt(stores, OperationTermination::Fatal(fatal));
        self.commit_direct_operation(stores, operation_mark);
        self.record_direct_episode_commit(
            stores,
            operations,
            crate::EpisodeCommitBoundary::Semantic(crate::SemanticEpisodeBarrier::Diagnostic),
            initial_artifacts,
            initial_boundaries,
            initial_effect_pos,
        );
        let terminal = self.succumb(fatal);
        evidence_error.map_or(Ok(StepResult::Progress(terminal)), Err)
    }

    /// Executes successful ordinary commands directly on canonical state.
    /// Group entry and exit, deferred effects, and nonpublishing PDF ledger
    /// mutations are ordinary journal/save-stack mutations, so the bounded
    /// loop deliberately has no group-depth stop and owns no retry snapshot.
    /// Only a capability with an explicit transaction specification, dynamic
    /// output continuation, or unsettled expansion leaves this path.
    fn execute_direct_episode(
        &mut self,
        stores: &mut Universe<G>,
        max_operations: usize,
        mut initial_delivery: Option<OperationDelivery<G>>,
        mut tracked_region: Option<&mut Option<Result<TrackedRegionRecord, DependencyRegionError>>>,
    ) -> Result<StepResult, ExecError> {
        let initial_boundaries = self.completed_boundaries.len();
        let initial_effect_pos = stores.world().effect_pos();
        let initial_artifacts = stores.world().artifact_commits().len();
        let initial_format_dump = self.dumped_format.is_some();
        let initial_diagnostic = self.first_causal_context.is_some();
        let initial_error_count = stores.world().error_channel().error_count();
        let mut operations = 0_usize;
        let mut direct_attempt_recorded = false;
        let mut episode_tracked_mark = if tracked_region.is_some() {
            match stores.begin_dependency_region() {
                Ok(mark) => Some(mark),
                Err(error) => {
                    if let Some(outcome) = tracked_region.as_deref_mut() {
                        *outcome = Some(Err(error));
                    }
                    None
                }
            }
        } else {
            None
        };

        loop {
            // Private revisions require every scanner-time immutable
            // allocation to belong to one fixed-size operation suffix. The
            // mark therefore opens before delivery preflight, while semantic
            // owners still remain untouched until prepared apply.
            let mut operation_mark = self.begin_direct_operation(stores);
            let preflight = if let Some(pending) = self.pending_resource_operation.take() {
                operation_mark.attempt = pending.attempt;
                PreflightDelivery::<G> {
                    delivery: OperationDelivery::<G>::Prepared(pending.scanned),
                    capabilities: pending.capabilities,
                }
            } else if let Some(delivery) = initial_delivery.take() {
                PreflightDelivery::<G> {
                    delivery,
                    capabilities:
                        crate::transaction_protocol::canonical_static_command_capabilities(
                            Meaning::Relax,
                        ),
                }
            } else if let Some(pending) = self.pending_alignment_delivery.take() {
                PreflightDelivery::<G> {
                    delivery: OperationDelivery::<G>::AlignmentRetry {
                        alignment: pending.alignment,
                        cursor: pending.cursor,
                    },
                    capabilities:
                        crate::transaction_protocol::canonical_static_command_capabilities(
                            Meaning::Relax,
                        ),
                }
            } else if let Some(command) = self.pending_preflight_command.take() {
                match command {
                    PendingPreflightCommand::<G>::Settled { command, cursor } => {
                        PreflightDelivery::<G> {
                            capabilities:
                                crate::transaction_protocol::canonical_command_capabilities(
                                    command.meaning(),
                                ),
                            delivery: OperationDelivery::<G>::Settled { command, cursor },
                        }
                    }
                    PendingPreflightCommand::<G>::Raw { command, cursor } => {
                        PreflightDelivery::<G> {
                            capabilities:
                                crate::transaction_protocol::canonical_command_capabilities(
                                    command.meaning(),
                                ),
                            delivery: OperationDelivery::<G>::Raw { command, cursor },
                        }
                    }
                    PendingPreflightCommand::<G>::Expanding {
                        command,
                        main_loop,
                        cursor,
                    } => PreflightDelivery::<G> {
                        capabilities: crate::transaction_protocol::canonical_command_capabilities(
                            command.meaning(),
                        ),
                        delivery: OperationDelivery::<G>::Expanding {
                            command,
                            main_loop,
                            cursor,
                        },
                    },
                    PendingPreflightCommand::<G>::ImmediatePdfRetry(primitive) => {
                        let meaning = Meaning::UnexpandablePrimitive(primitive);
                        PreflightDelivery::<G> {
                            capabilities:
                                crate::transaction_protocol::canonical_static_command_capabilities(
                                    meaning,
                                ),
                            delivery: OperationDelivery::<G>::ImmediatePdfRetry(primitive),
                        }
                    }
                }
            } else {
                let preflight = match self.preflight_replay_delivery(stores) {
                    Ok(preflight) => preflight,
                    Err(error) => {
                        if let Some(mark) = episode_tracked_mark.take() {
                            let _ = stores.abandon_dependency_region(mark);
                        }
                        if execution_error_is_fuel(&error) {
                            self.episode_telemetry
                                .record_semantic_barrier(crate::SemanticEpisodeBarrier::Fuel);
                        }
                        let result = self.finish_resource_preflight_failure(stores, error);
                        if matches!(result, Ok(StepResult::Suspended(_))) {
                            self.advance_telemetry.rollbacks += 1;
                            #[cfg(feature = "profiling")]
                            self.episode_telemetry
                                .record_rollback(crate::SemanticEpisodeBarrier::Resource, 1);
                            #[cfg(not(feature = "profiling"))]
                            self.episode_telemetry
                                .record_rollback(crate::SemanticEpisodeBarrier::Resource);
                        }
                        if matches!(result, Ok(StepResult::Suspended(_))) {
                            self.retain_direct_operation_for_retry(stores, operation_mark);
                        } else {
                            self.commit_direct_operation(stores, operation_mark);
                        }
                        return result;
                    }
                };
                preflight.expect("alignment delivery has a direct preflight")
            };

            let alignment_delivery = match &preflight.delivery {
                OperationDelivery::<G>::Alignment(alignment) => Some(Some(*alignment)),
                OperationDelivery::<G>::AlignmentRetry { alignment, .. } => Some(*alignment),
                OperationDelivery::<G>::Replay(None)
                    if self.active_alignment.is_some()
                        || (self.modes.current_mode() == Mode::DisplayMath
                            && self.modes.current_list().has_display_alignment()) =>
                {
                    Some(None)
                }
                _ => None,
            };

            if let crate::transaction_protocol::CommandPreflight::Resource(_) =
                preflight.capabilities.preflight()
                && !(stores.int_param(IntParam::PDF_OUTPUT) <= 0
                    && matches!(
                        &preflight.delivery,
                        OperationDelivery::<G>::Replay(Some(command))
                            if matches!(
                                command.meaning(),
                                ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                                    UnexpandablePrimitive::PdfXImage
                                ))
                            )
                    ))
            {
                if operations != 0 {
                    self.record_direct_episode_commit(
                        stores,
                        operations,
                        crate::EpisodeCommitBoundary::SliceLimit,
                        initial_artifacts,
                        initial_boundaries,
                        initial_effect_pos,
                    );
                }
                self.episode_telemetry.record_attempt();
                self.advance_telemetry.attempts += 1;
                let tracked_mark = episode_tracked_mark.take();
                let retry_command = match &preflight.delivery {
                    OperationDelivery::<G>::Replay(Some(command)) => {
                        Some(PendingPreflightCommand::<G>::Settled {
                            command: command.clone(),
                            cursor: None,
                        })
                    }
                    OperationDelivery::<G>::Settled { command, cursor } => {
                        Some(PendingPreflightCommand::<G>::Settled {
                            command: command.clone(),
                            cursor: *cursor,
                        })
                    }
                    OperationDelivery::<G>::Raw { command, cursor } => {
                        Some(PendingPreflightCommand::<G>::Raw {
                            command: command.clone(),
                            cursor: *cursor,
                        })
                    }
                    OperationDelivery::<G>::Expanding {
                        command,
                        main_loop,
                        cursor,
                    } => Some(PendingPreflightCommand::<G>::Expanding {
                        command: command.clone(),
                        main_loop: *main_loop,
                        cursor: *cursor,
                    }),
                    OperationDelivery::<G>::ImmediatePdfRetry(primitive) => {
                        Some(PendingPreflightCommand::<G>::ImmediatePdfRetry(*primitive))
                    }
                    OperationDelivery::<G>::Replay(None)
                    | OperationDelivery::<G>::Alignment(_)
                    | OperationDelivery::<G>::AlignmentRetry { .. }
                    | OperationDelivery::<G>::Hot(_)
                    | OperationDelivery::<G>::Prepared(_) => None,
                };
                let prepared = match self.prepare_operation(stores, preflight.delivery) {
                    Ok(prepared) => prepared,
                    Err(failure) => {
                        if let Some(mark) = tracked_mark {
                            let _ = stores.abandon_dependency_region(mark);
                        }
                        if let Some(scanned) = failure.unavailable {
                            self.pending_resource_operation = Some(PendingResourceOperation::<G> {
                                scanned,
                                capabilities: preflight.capabilities,
                                attempt: operation_mark.attempt,
                            });
                            self.advance_telemetry.rollbacks += 1;
                            #[cfg(feature = "profiling")]
                            self.episode_telemetry
                                .record_rollback(crate::SemanticEpisodeBarrier::Resource, 1);
                            #[cfg(not(feature = "profiling"))]
                            self.episode_telemetry
                                .record_rollback(crate::SemanticEpisodeBarrier::Resource);
                            let result =
                                self.finish_resource_preflight_failure(stores, *failure.error);
                            self.retain_direct_operation_for_retry(stores, operation_mark);
                            return result;
                        }
                        let result = self.finish_resource_preflight_failure(stores, *failure.error);
                        if matches!(result, Ok(StepResult::Suspended(_))) {
                            self.pending_alignment_delivery = alignment_delivery
                                .zip(failure.cursor)
                                .map(|(alignment, cursor)| PendingAlignmentDelivery {
                                    alignment,
                                    cursor,
                                });
                            let retry_expansion = self.command.pending_expansion_command().cloned();
                            self.pending_preflight_command = retry_command.map(|retry| {
                                let retry = retry.with_retry_expansion(retry_expansion);
                                match failure.cursor {
                                    Some(cursor) => retry.with_cursor(cursor),
                                    None => retry,
                                }
                            });
                            self.advance_telemetry.rollbacks += 1;
                            #[cfg(feature = "profiling")]
                            self.episode_telemetry
                                .record_rollback(crate::SemanticEpisodeBarrier::Resource, 1);
                            #[cfg(not(feature = "profiling"))]
                            self.episode_telemetry
                                .record_rollback(crate::SemanticEpisodeBarrier::Resource);
                        }
                        if matches!(result, Ok(StepResult::Suspended(_))) {
                            self.retain_direct_operation_for_retry(stores, operation_mark);
                        } else {
                            self.commit_direct_operation(stores, operation_mark);
                        }
                        return result;
                    }
                };
                let applied = self.apply_ready_operation(stores, prepared);
                self.record_save_stack_usage(stores);
                let boundary = self.episode_commit_boundary(
                    stores,
                    &applied,
                    1,
                    1,
                    initial_boundaries,
                    initial_effect_pos,
                    initial_artifacts,
                    initial_format_dump,
                    initial_diagnostic,
                    initial_error_count,
                    tracked_region.is_some(),
                );
                let step = match applied {
                    Ok(step) => step,
                    Err(error) => {
                        return self.finish_direct_failure(
                            stores,
                            operation_mark,
                            error,
                            DirectFailureContext {
                                operations: 1,
                                initial_artifacts,
                                initial_boundaries,
                                initial_effect_pos,
                            },
                        );
                    }
                };
                if let Some(error) =
                    self.admit_observed_receipt(stores, operation_termination(step, self.fatal))
                {
                    self.commit_direct_operation(stores, operation_mark);
                    return Err(error);
                }
                self.commit_direct_operation(stores, operation_mark);
                let tracked_result = tracked_mark.map(|mark| {
                    stores
                        .finish_dependency_region(mark)
                        .map(TrackedRegionRecord::new)
                });
                self.record_direct_episode_commit(
                    stores,
                    1,
                    boundary.unwrap_or(crate::EpisodeCommitBoundary::SliceLimit),
                    initial_artifacts,
                    initial_boundaries,
                    initial_effect_pos,
                );
                if let (Some(outcome), Some(result)) =
                    (tracked_region.as_deref_mut(), tracked_result)
                {
                    *outcome = Some(result);
                }
                return Ok(StepResult::Progress(step));
            }

            if matches!(
                preflight.capabilities.preflight(),
                crate::transaction_protocol::CommandPreflight::Transaction(_)
            ) || self.command_requires_transaction(
                stores,
                preflight.capabilities,
                &preflight.delivery,
            ) {
                if operations != 0 {
                    self.record_direct_episode_commit(
                        stores,
                        operations,
                        crate::EpisodeCommitBoundary::SliceLimit,
                        initial_artifacts,
                        initial_boundaries,
                        initial_effect_pos,
                    );
                }
                let tracked_mark = episode_tracked_mark.take();
                let mut retry_command = match &preflight.delivery {
                    OperationDelivery::<G>::Replay(Some(command)) => {
                        Some(PendingPreflightCommand::<G>::Settled {
                            command: command.clone(),
                            cursor: None,
                        })
                    }
                    OperationDelivery::<G>::Settled { command, cursor } => {
                        Some(PendingPreflightCommand::<G>::Settled {
                            command: command.clone(),
                            cursor: *cursor,
                        })
                    }
                    OperationDelivery::<G>::Raw { command, cursor } => {
                        Some(PendingPreflightCommand::<G>::Raw {
                            command: command.clone(),
                            cursor: *cursor,
                        })
                    }
                    OperationDelivery::<G>::Expanding {
                        command,
                        main_loop,
                        cursor,
                    } => Some(PendingPreflightCommand::<G>::Expanding {
                        command: command.clone(),
                        main_loop: *main_loop,
                        cursor: *cursor,
                    }),
                    OperationDelivery::<G>::ImmediatePdfRetry(primitive) => {
                        Some(PendingPreflightCommand::<G>::ImmediatePdfRetry(*primitive))
                    }
                    OperationDelivery::<G>::Replay(None)
                    | OperationDelivery::<G>::Alignment(_)
                    | OperationDelivery::<G>::AlignmentRetry { .. }
                    | OperationDelivery::<G>::Hot(_)
                    | OperationDelivery::<G>::Prepared(_) => None,
                };
                if let crate::transaction_protocol::CommandPreflight::Transaction(transaction) =
                    preflight.capabilities.preflight()
                {
                    let transaction = transaction.transaction();
                    transaction
                        .admit(transaction.projection())
                        .expect("preflight owns the exact narrow projection");
                }
                let prepared = match self.prepare_operation(stores, preflight.delivery) {
                    Ok(prepared) => prepared,
                    Err(failure) => {
                        if let Some(mark) = tracked_mark {
                            let _ = stores.abandon_dependency_region(mark);
                        }
                        if let Some(scanned) = failure.unavailable {
                            self.pending_resource_operation = Some(PendingResourceOperation::<G> {
                                scanned,
                                capabilities: preflight.capabilities,
                                attempt: operation_mark.attempt,
                            });
                            let result =
                                self.finish_resource_preflight_failure(stores, *failure.error);
                            self.retain_direct_operation_for_retry(stores, operation_mark);
                            return result;
                        }
                        let result = self.finish_resource_preflight_failure(stores, *failure.error);
                        match result {
                            Ok(step @ StepResult::Suspended(_)) => {
                                self.pending_alignment_delivery = alignment_delivery
                                    .zip(failure.cursor)
                                    .map(|(alignment, cursor)| PendingAlignmentDelivery {
                                        alignment,
                                        cursor,
                                    });
                                let retry_expansion =
                                    self.command.pending_expansion_command().cloned();
                                self.pending_preflight_command = retry_command.map(|retry| {
                                    let retry = retry.with_retry_expansion(retry_expansion);
                                    match failure.cursor {
                                        Some(cursor) => retry.with_cursor(cursor),
                                        None => retry,
                                    }
                                });
                                self.retain_direct_operation_for_retry(stores, operation_mark);
                                return Ok(step);
                            }
                            Ok(step) => {
                                self.commit_direct_operation(stores, operation_mark);
                                return Ok(step);
                            }
                            Err(error) => {
                                let retry_expansion =
                                    self.command.pending_expansion_command().cloned();
                                self.pending_preflight_command = retry_command.map(|retry| {
                                    let retry = retry.with_retry_expansion(retry_expansion);
                                    match failure.cursor {
                                        Some(cursor) => retry.with_cursor(cursor),
                                        None => retry,
                                    }
                                });
                                self.retain_direct_operation_for_retry(stores, operation_mark);
                                Self::publish_pdf_fatal_error(stores, &error)?;
                                return Err(error);
                            }
                        }
                    }
                };
                if let OperationReadiness::<G>::Prepared(PreparedColdOperation::<G> {
                    scanned:
                        ColdOperation::ImmediateExtension(
                            RootedImmediateExtension::PdfExtensionInDviMode(primitive),
                        ),
                    ..
                }) = &prepared
                {
                    retry_command =
                        Some(PendingPreflightCommand::<G>::ImmediatePdfRetry(*primitive));
                }
                self.episode_telemetry.record_attempt();
                self.advance_telemetry.attempts += 1;
                let applied = self.apply_ready_operation(stores, prepared);
                self.record_save_stack_usage(stores);
                let boundary = self.episode_commit_boundary(
                    stores,
                    &applied,
                    1,
                    1,
                    initial_boundaries,
                    initial_effect_pos,
                    initial_artifacts,
                    initial_format_dump,
                    initial_diagnostic,
                    initial_error_count,
                    tracked_region.is_some(),
                );
                let step = match applied {
                    Ok(step) => step,
                    Err(error) => {
                        if let Some(mark) = tracked_mark {
                            if error.as_fatal().is_some() {
                                stores.poison_dependency_region(
                                    TrackedRegionBarrier::FatalPartialCommit,
                                );
                                let result = stores
                                    .finish_dependency_region(mark)
                                    .map(TrackedRegionRecord::new);
                                if let Some(outcome) = tracked_region.as_deref_mut() {
                                    *outcome = Some(result);
                                }
                            } else {
                                let _ = stores.abandon_dependency_region(mark);
                            }
                        }
                        if error.as_fatal().is_none() {
                            self.pending_preflight_command = retry_command;
                            self.discard_direct_operation(stores, operation_mark);
                            self.advance_telemetry.commits += 1;
                            let error = {
                                let mut context =
                                    stores.command_context().expect("diagnostic admission");
                                error.freeze_diagnostic_origin(
                                    &mut context,
                                    self.command.diagnostic_input_context(8),
                                )
                            };
                            Self::publish_pdf_fatal_error(stores, &error)?;
                            return Err(error);
                        }
                        return self.finish_direct_failure(
                            stores,
                            operation_mark,
                            error,
                            DirectFailureContext {
                                operations: 1,
                                initial_artifacts,
                                initial_boundaries,
                                initial_effect_pos,
                            },
                        );
                    }
                };
                if let Some(error) =
                    self.admit_observed_receipt(stores, operation_termination(step, self.fatal))
                {
                    self.commit_direct_operation(stores, operation_mark);
                    return Err(error);
                }
                self.commit_direct_operation(stores, operation_mark);
                let tracked_result = tracked_mark.map(|mark| {
                    stores
                        .finish_dependency_region(mark)
                        .map(TrackedRegionRecord::new)
                });
                self.record_direct_episode_commit(
                    stores,
                    1,
                    boundary.unwrap_or(crate::EpisodeCommitBoundary::SliceLimit),
                    initial_artifacts,
                    initial_boundaries,
                    initial_effect_pos,
                );
                if let (Some(outcome), Some(result)) =
                    (tracked_region.as_deref_mut(), tracked_result)
                {
                    *outcome = Some(result);
                }
                return Ok(StepResult::Progress(step));
            }

            if !direct_attempt_recorded {
                self.episode_telemetry.record_attempt();
                self.advance_telemetry.attempts += 1;
                direct_attempt_recorded = true;
            }
            let tracked_mark = episode_tracked_mark.take();
            let preserves_undefined_for_executor_diagnostic = matches!(
                &preflight.delivery,
                OperationDelivery::<G>::Replay(Some(command))
                    | OperationDelivery::<G>::Settled { command, .. }
                    if matches!(
                        command.meaning(),
                        ResolvedMeaning::Static(Meaning::Undefined | Meaning::Unknown(_))
                    )
            );
            // Capability preflight preserves an undefined command instead of
            // diagnosing it inside expansion. The executor reports that one
            // settled command without entering a second ErrorStop input
            // dialogue; recovery input still belongs to diagnostics raised
            // by operand scanning and semantic application.
            let saved_interaction = preserves_undefined_for_executor_diagnostic
                .then(|| stores.interaction_mode())
                .filter(|mode| *mode == tex_state::InteractionMode::ErrorStop);
            if saved_interaction.is_some() {
                stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
            }
            let retry_command = match &preflight.delivery {
                OperationDelivery::<G>::Replay(Some(command)) => {
                    Some(PendingPreflightCommand::<G>::Settled {
                        command: command.clone(),
                        cursor: None,
                    })
                }
                OperationDelivery::<G>::Settled { command, cursor } => {
                    Some(PendingPreflightCommand::<G>::Settled {
                        command: command.clone(),
                        cursor: *cursor,
                    })
                }
                OperationDelivery::<G>::Raw { command, cursor } => {
                    Some(PendingPreflightCommand::<G>::Raw {
                        command: command.clone(),
                        cursor: *cursor,
                    })
                }
                OperationDelivery::<G>::Expanding {
                    command,
                    main_loop,
                    cursor,
                } => Some(PendingPreflightCommand::<G>::Expanding {
                    command: command.clone(),
                    main_loop: *main_loop,
                    cursor: *cursor,
                }),
                OperationDelivery::<G>::ImmediatePdfRetry(primitive) => {
                    Some(PendingPreflightCommand::<G>::ImmediatePdfRetry(*primitive))
                }
                OperationDelivery::<G>::Replay(None)
                | OperationDelivery::<G>::Alignment(_)
                | OperationDelivery::<G>::AlignmentRetry { .. }
                | OperationDelivery::<G>::Hot(_)
                | OperationDelivery::<G>::Prepared(_) => None,
            };
            let prepared = match self.prepare_operation(stores, preflight.delivery) {
                Ok(prepared) => prepared,
                Err(failure) => {
                    if let Some(interaction) = saved_interaction {
                        stores.set_interaction_mode(interaction);
                    }
                    if let Some(mark) = tracked_mark {
                        let _ = stores.abandon_dependency_region(mark);
                    }
                    if let Some(scanned) = failure.unavailable {
                        self.pending_resource_operation = Some(PendingResourceOperation::<G> {
                            scanned,
                            capabilities: preflight.capabilities,
                            attempt: operation_mark.attempt,
                        });
                    }
                    let result = self.finish_resource_preflight_failure(stores, *failure.error);
                    if matches!(result, Ok(StepResult::Suspended(_)))
                        && self.pending_resource_operation.is_none()
                    {
                        self.pending_alignment_delivery = alignment_delivery
                            .zip(failure.cursor)
                            .map(|(alignment, cursor)| PendingAlignmentDelivery {
                                alignment,
                                cursor,
                            });
                        let retry_expansion = self.command.pending_expansion_command().cloned();
                        self.pending_preflight_command = retry_command.map(|retry| {
                            let retry = retry.with_retry_expansion(retry_expansion);
                            match failure.cursor {
                                Some(cursor) => retry.with_cursor(cursor),
                                None => retry,
                            }
                        });
                    }
                    if matches!(result, Ok(StepResult::Suspended(_))) {
                        self.retain_direct_operation_for_retry(stores, operation_mark);
                    } else {
                        self.commit_direct_operation(stores, operation_mark);
                    }
                    return match result {
                        Err(error) => {
                            let error = {
                                let mut context =
                                    stores.command_context().expect("diagnostic admission");
                                error.freeze_diagnostic_origin(
                                    &mut context,
                                    self.command.diagnostic_input_context(8),
                                )
                            };
                            Self::publish_pdf_fatal_error(stores, &error)?;
                            Err(error)
                        }
                        result => result,
                    };
                }
            };
            let applied = self.apply_ready_operation(stores, prepared);
            if let Some(interaction) = saved_interaction {
                stores.set_interaction_mode(interaction);
            }
            operations += 1;
            self.record_save_stack_usage(stores);
            let boundary = self.episode_commit_boundary(
                stores,
                &applied,
                operations,
                max_operations,
                initial_boundaries,
                initial_effect_pos,
                initial_artifacts,
                initial_format_dump,
                initial_diagnostic,
                initial_error_count,
                tracked_region.is_some(),
            );
            let step = match applied {
                Ok(step) => step,
                Err(error) => {
                    if let Some(mark) = tracked_mark {
                        if error.as_fatal().is_some() {
                            stores
                                .poison_dependency_region(TrackedRegionBarrier::FatalPartialCommit);
                            let result = stores
                                .finish_dependency_region(mark)
                                .map(TrackedRegionRecord::new);
                            if let Some(outcome) = tracked_region.as_deref_mut() {
                                *outcome = Some(result);
                            }
                        } else {
                            let _ = stores.abandon_dependency_region(mark);
                        }
                    }
                    return self.finish_direct_failure(
                        stores,
                        operation_mark,
                        error,
                        DirectFailureContext {
                            operations,
                            initial_artifacts,
                            initial_boundaries,
                            initial_effect_pos,
                        },
                    );
                }
            };
            if let Some(error) =
                self.admit_observed_receipt(stores, operation_termination(step, self.fatal))
            {
                self.commit_direct_operation(stores, operation_mark);
                return Err(error);
            }
            self.commit_direct_operation(stores, operation_mark);
            let tracked_result = tracked_mark.map(|mark| {
                stores
                    .finish_dependency_region(mark)
                    .map(TrackedRegionRecord::new)
            });
            if let Some(boundary) = boundary {
                self.record_direct_episode_commit(
                    stores,
                    operations,
                    boundary,
                    initial_artifacts,
                    initial_boundaries,
                    initial_effect_pos,
                );
                if let (Some(outcome), Some(result)) =
                    (tracked_region.as_deref_mut(), tracked_result)
                {
                    *outcome = Some(result);
                }
                return Ok(StepResult::Progress(step));
            }
        }
    }

    fn execute_operation(
        &mut self,
        stores: &mut Universe<G>,
        delivery: OperationDelivery<G>,
        transaction: OperationTransaction,
        max_operations: usize,
        tracked_region: Option<&mut Option<Result<TrackedRegionRecord, DependencyRegionError>>>,
    ) -> Result<StepResult, ExecError> {
        if self.operation_observations.is_none() {
            // A caller may resume an observed resource suspension through
            // the unobserved API. The semantic continuation is independent
            // of instrumentation, so drop the moved evidence owner instead
            // of publishing it into some later unrelated observed step.
            self.suspended_operation_observation = None;
        }
        if matches!(transaction, OperationTransaction::Nested) {
            let result = self.apply_operation(stores, delivery);
            self.record_save_stack_usage(stores);
            if result.is_ok()
                && let Some(error) = self.operation_evidence_limit_error()
            {
                return Err(error);
            }
            return result.map(StepResult::Progress);
        }
        let initial_delivery =
            matches!(transaction, OperationTransaction::Alignment).then_some(delivery);
        self.execute_direct_episode(stores, max_operations, initial_delivery, tracked_region)
    }

    fn capture_first_causal_context(
        &mut self,
        stores: &mut Universe<G>,
        diagnostics: &[PendingDiagnostic<G>],
    ) {
        if self.first_causal_context.is_none()
            && let Some(cause_kind) = diagnostics.iter().find_map(PendingDiagnostic::causal_kind)
        {
            let context = stores.command_context().expect("diagnostic admission");
            self.first_causal_context = Some(crate::FrozenDiagnosticContext::capture(
                &context,
                self.command.diagnostic_input_context(8),
                cause_kind,
            ));
        }
    }

    fn operation_evidence_limit_error(&self) -> Option<ExecError> {
        self.operation_observations
            .as_ref()
            .and_then(ObservationBuffer::limit_error)
            .or_else(|| self.page_output_observations.limit_error())
    }

    /// Closes every live receipt category and performs the append-bound check
    /// that must precede any operation commit.
    fn admit_observed_receipt(
        &mut self,
        stores: &Universe<G>,
        termination: OperationTermination,
    ) -> Option<ExecError> {
        let (Some(start), Some(pending)) = (
            self.operation_receipt_start,
            self.operation_observations.as_mut(),
        ) else {
            return self.operation_evidence_limit_error();
        };
        let live_effects = stores.world().effect_records();
        let effect_base = stores
            .world()
            .effect_pos()
            .raw()
            .saturating_sub(live_effects.len().try_into().unwrap_or(u64::MAX));
        let effect_start = start
            .effect
            .saturating_sub(effect_base)
            .try_into()
            .unwrap_or(usize::MAX)
            .min(live_effects.len());
        for effect in &live_effects[effect_start..] {
            pending.record_world_effect(effect.clone());
        }
        for artifact in &stores.world().artifact_commits()[start.artifact..] {
            pending.record_artifact(*artifact);
        }
        pending.receipt.set_termination(termination);
        self.operation_evidence_limit_error()
    }

    /// Attempts one atomic main-control operation.
    ///
    /// Missing retained input is returned as a typed suspension after the
    /// exact command/input continuation has been retained. The next call
    /// creates a fresh processor borrow and resumes that continuation without
    /// redelivering the command.
    pub fn advance(&mut self, stores: &mut Universe<G>) -> Result<StepResult, ExecError> {
        if !self.pure_memo_initialized {
            let runtime = stores.take_pure_memo_config().map_or_else(
                tex_state::PureMemoRuntime::default,
                tex_state::PureMemoRuntime::new,
            );
            self.install_pure_memo_runtime(runtime);
        }
        stores.attach_pure_memo_capability(&self.pure_memo);
        if self.fatal.is_some() {
            return Ok(StepResult::Progress(MainControlStep::End));
        }
        self.execute_operation(
            stores,
            OperationDelivery::<G>::Replay(None),
            OperationTransaction::Advance,
            1,
            None,
        )
    }

    /// Advances one production driver chunk under a single bounded retry
    /// point. The public one-operation [`Self::advance`] contract remains
    /// available to diagnostic and focused-test callers.
    pub fn advance_episode(&mut self, stores: &mut Universe<G>) -> Result<StepResult, ExecError> {
        if !self.pure_memo_initialized {
            let runtime = stores.take_pure_memo_config().map_or_else(
                tex_state::PureMemoRuntime::default,
                tex_state::PureMemoRuntime::new,
            );
            self.install_pure_memo_runtime(runtime);
        }
        stores.attach_pure_memo_capability(&self.pure_memo);
        if self.fatal.is_some() {
            return Ok(StepResult::Progress(MainControlStep::End));
        }
        self.execute_operation(
            stores,
            OperationDelivery::<G>::Replay(None),
            OperationTransaction::Advance,
            256,
            None,
        )
    }

    /// Attempts one ordinary main-control operation while collecting detached
    /// semantic dependency evidence for that operation only.
    ///
    /// Recording failure never changes the TeX result. A committed supported
    /// operation returns a record, a fail-closed barrier returns its typed
    /// rejection, and rollback or resource suspension returns no region.
    pub fn advance_with_tracked_region(
        &mut self,
        stores: &mut Universe<G>,
    ) -> Result<TrackedStepResult, ExecError> {
        if !self.pure_memo_initialized {
            let runtime = stores.take_pure_memo_config().map_or_else(
                tex_state::PureMemoRuntime::default,
                tex_state::PureMemoRuntime::new,
            );
            self.install_pure_memo_runtime(runtime);
        }
        stores.attach_pure_memo_capability(&self.pure_memo);
        if self.fatal.is_some() {
            return Ok(TrackedStepResult {
                step: StepResult::Progress(MainControlStep::End),
                region: None,
            });
        }
        let mut region = None;
        let step = self.execute_operation(
            stores,
            OperationDelivery::<G>::Replay(None),
            OperationTransaction::Advance,
            1,
            Some(&mut region),
        )?;
        Ok(TrackedStepResult { step, region })
    }

    /// Expands one command for an analysis host without entering ordinary
    /// typesetting. TeX82 §1270's command-code partition still routes every
    /// assignment through §1211 `prefixed_command`; other spellings, including
    /// undefined control sequences, are returned to the host with provenance.
    pub fn diagnostic_expand_step(
        &mut self,
        stores: &mut Universe<G>,
    ) -> Result<DiagnosticStepResult, ExecError> {
        let mut operation_mark = self.begin_direct_operation(stores);
        let continuation = self.pending_diagnostic_operation.take();
        let assignment = match continuation {
            Some(PendingDiagnosticOperation::<G>::Prepared { scanned, attempt }) => {
                operation_mark.attempt = attempt;
                Some((OperationDelivery::<G>::Prepared(scanned), None))
            }
            Some(PendingDiagnosticOperation::<G>::Assignment {
                command,
                cursor,
                attempt,
            }) => {
                operation_mark.attempt = attempt;
                let retry = (command.clone(), cursor);
                Some((
                    OperationDelivery::<G>::Settled {
                        command,
                        cursor: Some(cursor),
                    },
                    Some(retry),
                ))
            }
            None => {
                self.refresh_host_capabilities(stores);
                let (command, cursor) = {
                    let mut processor = command_processor(
                        &mut self.command,
                        self.fuel.fuel_mut(),
                        &mut self.capabilities,
                        &mut self.operation_observations,
                        stores.command_context().expect("live generation"),
                    );
                    let command = processor
                        .get_x_token_preserving_undefined()
                        .map_err(command_error);
                    (command, processor.delivery_cursor())
                };
                let command = match command {
                    Ok(command) => command,
                    Err(error) => {
                        let result = self.finish_resource_preflight_failure(stores, error);
                        if matches!(result, Ok(StepResult::Suspended(_))) {
                            self.retain_direct_operation_for_retry(stores, operation_mark);
                        } else {
                            self.commit_direct_operation(stores, operation_mark);
                        }
                        return match result? {
                            StepResult::Suspended(need) => {
                                Ok(DiagnosticStepResult::Suspended(need))
                            }
                            StepResult::Progress(_) => {
                                unreachable!("diagnostic expansion failure made progress")
                            }
                        };
                    }
                };
                let Some(command) = command else {
                    self.commit_direct_operation(stores, operation_mark);
                    return Ok(DiagnosticStepResult::Progress(DiagnosticStep::EndOfInput));
                };
                if !tex_command::exceeds_max_non_prefixed_command(static_meaning(command.meaning()))
                {
                    let step = DiagnosticStep::Token {
                        spelling: command.spelling(),
                        meaning: static_meaning(command.meaning()),
                        control_sequence: command.control_sequence(),
                        source_provenance: command.source_provenance(),
                    };
                    self.commit_direct_operation(stores, operation_mark);
                    return Ok(DiagnosticStepResult::Progress(step));
                }
                let retry = (command.clone(), cursor);
                Some((
                    OperationDelivery::<G>::Settled {
                        command,
                        cursor: Some(cursor),
                    },
                    Some(retry),
                ))
            }
        };
        let (delivery, retry) = assignment.expect("diagnostic assignment continuation");
        let mode_mark = self.modes.begin_journal();
        let prepared = match self.prepare_operation(stores, delivery) {
            Ok(prepared) => prepared,
            Err(failure) => {
                if let Some(scanned) = failure.unavailable {
                    self.pending_diagnostic_operation =
                        Some(PendingDiagnosticOperation::<G>::Prepared {
                            scanned,
                            attempt: operation_mark.attempt,
                        });
                } else if let Some((command, cursor)) = retry {
                    let command = self
                        .command
                        .pending_expansion_command()
                        .cloned()
                        .unwrap_or(command);
                    self.pending_diagnostic_operation =
                        Some(PendingDiagnosticOperation::<G>::Assignment {
                            command,
                            cursor: failure.cursor.unwrap_or(cursor),
                            attempt: operation_mark.attempt,
                        });
                }
                let result = self.finish_resource_preflight_failure(stores, *failure.error);
                if !matches!(result, Ok(StepResult::Suspended(_))) {
                    self.pending_diagnostic_operation = None;
                }
                self.modes
                    .rollback_journal(mode_mark)
                    .expect("diagnostic assignment owns the mode mark");
                if matches!(result, Ok(StepResult::Suspended(_))) {
                    self.retain_direct_operation_for_retry(stores, operation_mark);
                } else {
                    self.commit_direct_operation(stores, operation_mark);
                }
                return match result? {
                    StepResult::Suspended(need) => Ok(DiagnosticStepResult::Suspended(need)),
                    StepResult::Progress(_) => {
                        unreachable!("diagnostic assignment failure made progress")
                    }
                };
            }
        };
        match self.apply_ready_operation(stores, prepared) {
            Ok(_) => {
                self.modes
                    .commit_journal(mode_mark)
                    .expect("diagnostic assignment owns the mode mark");
                self.commit_direct_operation(stores, operation_mark);
                Ok(DiagnosticStepResult::Progress(DiagnosticStep::Assignment))
            }
            Err(error) => {
                self.modes
                    .rollback_journal(mode_mark)
                    .expect("diagnostic assignment owns the mode mark");
                self.discard_direct_operation(stores, operation_mark);
                let mut stores = stores.command_context().expect("live generation");
                Err(error.freeze_diagnostic_origin(
                    &mut stores,
                    self.command.diagnostic_input_context(8),
                ))
            }
        }
    }

    /// Delivers and executes one replay command through the command processor.
    ///
    /// Compatibility wrapper for callers which have not yet adopted typed
    /// resource suspension. New production hosts should use [`Self::advance`].
    pub fn step(&mut self, stores: &mut Universe<G>) -> Result<ReplayStep, ExecError> {
        let result = self.advance(stores).map_err(|error| match error {
            ExecError::Captured { error, .. } => *error,
            error => error,
        })?;
        match result {
            StepResult::Progress(step) => Ok(step),
            StepResult::Suspended(ResourceNeed::Input { .. }) => {
                Err(ExecError::MissingToken { context: "\\input" })
            }
            StepResult::Suspended(ResourceNeed::InputProbe { .. }) => {
                Err(ExecError::MissingToken {
                    context: "pdfTeX file enquiry",
                })
            }
            StepResult::Suspended(ResourceNeed::Font { .. }) => Err(ExecError::MissingToken {
                context: "\\font resource",
            }),
            StepResult::Suspended(ResourceNeed::PdfImage { .. }) => Err(ExecError::MissingToken {
                context: "\\pdfximage resource",
            }),
        }
    }

    /// Executes one command inside a host-owned direct episode.
    ///
    /// TeX82 §1211's `prefixed_command` remains the assignment dispatcher
    /// when §1228's numeric assignments occur inside a replayed math or
    /// discretionary field. If the enclosing operation is observed, route
    /// the nested command through the same executor-observation seam so its
    /// committed `word_define` is not reduced to command/scanner records.
    fn execute_nested_operation(
        &mut self,
        stores: &mut Universe<G>,
        settled: Option<tex_command::CurrentCommand<G>>,
    ) -> Result<ReplayStep, ExecError> {
        let delivery = settled.map_or(OperationDelivery::<G>::Replay(None), |command| {
            OperationDelivery::<G>::Settled {
                command,
                cursor: None,
            }
        });
        match self.execute_operation(stores, delivery, OperationTransaction::Nested, 1, None)? {
            StepResult::Progress(step) => Ok(step),
            StepResult::Suspended(_) => unreachable!("nested operations do not own rollback"),
        }
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
        stores: &mut Universe<G>,
    ) -> Result<ReplayStep, ExecError> {
        if matches!(
            self.modes.current_mode(),
            Mode::Vertical | Mode::InternalVertical
        ) {
            let mut context = stores.command_context().expect("live generation");
            start_paragraph(&mut self.command, &mut self.modes, &mut context, true)?;
        }
        let mut context = stores.command_context().expect("live generation");
        crate::box_runtime::flush_pending_hchars_with_fuel(
            &mut self.modes,
            &mut context,
            self.fuel.fuel_mut(),
        )?;
        let accent = u8::try_from(scanned.accent).map_err(|_| ExecError::InvalidCode {
            context: "\\accent",
            value: scanned.accent,
        })?;
        let accent_font = context.current_font();
        // §1123's `p:=new_character(f,cur_val); if p<>null then`: a missing
        // accent character skips `do_assignments` and the base lookahead
        // entirely, so nothing after this point runs.
        let Some(accent_metrics) = context.font_char_metrics(accent_font, accent) else {
            crate::diagnostics::report_missing_character_warning(
                &mut context,
                accent_font,
                char::from(accent),
                self.command_profile() == CommandProfile::ETEX26,
            );
            return Ok(ReplayStep::Continue);
        };
        drop(context);
        let base = self.do_assignments_then_accent_base(stores)?;
        let accent_origin = scanned.accent_provenance.primary;
        let etex_extended = self.command_profile() == CommandProfile::ETEX26;
        let mut context = stores.command_context().expect("live generation");
        apply_accent_nodes(
            &mut self.modes,
            &mut context,
            etex_extended,
            AccentPlacement {
                accent,
                accent_font,
                accent_metrics,
                accent_origin,
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
        stores: &mut Universe<G>,
    ) -> Result<Option<(u8, tex_state::token::OriginId)>, ExecError> {
        // None of §1270's assignments is a §1030 `main_loop` entry.
        self.main_loop_active = false;
        loop {
            let outcome = {
                let mut processor = command_processor(
                    &mut self.command,
                    self.fuel.fuel_mut(),
                    &mut self.capabilities,
                    &mut self.operation_observations,
                    stores.command_context().expect("live generation"),
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
                    self.set_box_forbidden_depth += 1;
                    let step = self.execute_nested_operation(stores, Some(command));
                    self.set_box_forbidden_depth -= 1;
                    match step? {
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
    fn fire_pending_page_output(&mut self, stores: &mut Universe<G>) -> Result<(), ExecError> {
        while !self.boxes.output_routine_active {
            let selected = {
                let mut context = stores.command_context().expect("live generation");
                let Some(fire_up) = context.page_fire_up() else {
                    break;
                };
                let error_context = crate::diagnostics::ExecutionDiagnosticContext::source_free(
                    self.command.output_open_context(&context),
                );
                crate::page_output::select_pending_page_output(
                    &mut context,
                    fire_up,
                    error_context,
                )?
            };
            match selected {
                crate::page_output::SelectedPageOutput::Default(page) => {
                    let mut command = CommandMachine {
                        state: &mut self.command,
                        fuel: self.fuel.fuel_mut(),
                        capabilities: &mut self.capabilities,
                        observations: &mut self.operation_observations,
                        assignment_receipts: None,
                        shown_mode: &mut self.shown_mode,
                        initex: self.initex,
                        emit_dvi_override: self.emit_dvi_override,
                        immediate_prints: &mut self.immediate_prints,
                        prepared_shipout: &mut self.prepared_shipout,
                    };
                    let publication = shipout_replay_box(page, stores, &mut command)?;
                    if let Some(receipt) = publication.and_then(|publication| publication.dvi) {
                        push_prepared_dvi_page(&mut self.prepared_dvi_pages, receipt);
                    }
                    break;
                }
                crate::page_output::SelectedPageOutput::UserRoutine => {
                    let enclosing = self.operation_observations.take();
                    if enclosing.is_some() {
                        self.operation_observations = Some(ObservationBuffer::default());
                        self.operation_observations
                            .as_mut()
                            .expect("observed page-output episode has a buffer")
                            .extend(
                                self.command
                                    .publish_named_token_list_pushes(
                                        &mut stores.command_context().expect("live generation"),
                                    )
                                    .into_iter()
                                    .map(CommandObservation::Input),
                            );
                    }
                    let mut processor = command_processor(
                        &mut self.command,
                        self.fuel.fuel_mut(),
                        &mut self.capabilities,
                        &mut self.operation_observations,
                        stores.command_context().expect("live generation"),
                    );
                    processor
                        .retire_completed_right_brace_backup()
                        .map_err(command_error)?;
                    let opened = processor
                        .begin_selected_output_routine()
                        .map_err(command_error);
                    // The selected output routine is now installed in the
                    // persistent interpreter. Retire its borrow facade before
                    // opening the matching semantic group and mode barriers.
                    drop(processor);
                    if enclosing.is_some() {
                        let mut deferred =
                            std::mem::replace(&mut self.operation_observations, enclosing)
                                .unwrap_or_default();
                        self.page_output_observations.append(&mut deferred);
                    }
                    opened?;
                    let mut context = stores.command_context().expect("live generation");
                    enter_group(&mut context, &mut self.command, GroupKind::Output);
                    self.modes.push_at_line(
                        Mode::InternalVertical,
                        -i32::try_from(self.command.current_file_line_number()).unwrap_or(i32::MAX),
                    )?;
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
        stores: &mut Universe<G>,
    ) -> Result<tex_state::node_arena::PageListId, ExecError> {
        // The depth sampled before `push_math`, not the innermost group
        // kind, is what identifies this group's own closing brace: a nested
        // subformula opens another `math_group`, and any brace group inside
        // the body opens a `simple_group`.
        let enclosing_depth = stores
            .command_context()
            .expect("live generation")
            .group_frames()
            .len();
        enter_group(
            &mut stores.command_context().expect("math-group admission"),
            &mut self.command,
            kind,
        );
        self.modes.push_at_line(
            Mode::Math,
            self.command
                .current_file_line_number()
                .try_into()
                .unwrap_or(i32::MAX),
        )?;
        self.main_loop_active = false;
        while stores
            .command_context()
            .expect("live generation")
            .group_frames()
            .len()
            > enclosing_depth
        {
            match self.execute_nested_operation(stores, None)? {
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
        stores: &mut Universe<G>,
    ) -> Result<tex_state::node_arena::PageListId, ExecError> {
        self.main_loop_active = false;
        while left_group_open(&self.modes, stores) {
            // The `\right.` applied below is exactly the closer §1065 selects
            // for `math_left_group`, so the report is §1064's `off_save`.
            let mut command_context = stores.command_context().expect("live generation");
            let context = self.command.output_open_context(&command_context);
            report_escaped_error(
                &mut command_context,
                "Missing ",
                "right.",
                " inserted",
                &OFF_SAVE_HELP,
                context,
            )?;
            drop(command_context);
            self.apply_math_delimiter(
                MathDelimiterBoundary {
                    kind: MathDelimiterBoundaryKind::Right,
                    delimiter: tex_command::ScannedMathDelimiter {
                        code: 0,
                        recovered: true,
                        missing_delimiter: false,
                        provenance: tex_command::StructuredProvenance {
                            primary: tex_state::token::OriginId::UNKNOWN,
                        },
                    },
                },
                stores,
            )?;
        }
        let mut context = stores.command_context().expect("math-list admission");
        let level = crate::box_runtime::commit_current_list(
            &mut self.modes,
            &mut context,
            self.fuel.fuel_mut(),
        )?;
        finish_math_list(
            level.list().nodes(),
            level.list().incomplete_fraction(),
            &mut context,
        )
    }

    /// Opens and runs one `\mathchoice` branch: TeX82 §1172/§1174's
    /// ``push_math(math_choice_group); scan_left_brace`` followed by the live
    /// body main control reads until `build_choices` closes it.
    fn execute_math_choice_branch(
        &mut self,
        stores: &mut Universe<G>,
    ) -> Result<tex_state::node_arena::PageListId, ExecError> {
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
        stores: &mut Universe<G>,
    ) -> Result<MathField, ExecError> {
        match field.body {
            MathFieldBody::Missing => Ok(MathField::Empty),
            MathFieldBody::Character(code) => Ok(MathField::MathChar(
                math_char(
                    &stores.command_context().expect("live generation"),
                    u32::from(code),
                    field.provenance.primary,
                )?
                .1,
            )),
            MathFieldBody::OpenGroup => {
                let list = self.execute_live_math_group(GroupKind::Math, stores)?;
                let context = stores.command_context().expect("live generation");
                Ok(collapse_singleton_math_group(&context, list))
            }
        }
    }

    fn apply_math_request(
        &mut self,
        request: MathRequest,
        stores: &mut Universe<G>,
    ) -> Result<ReplayStep, ExecError> {
        match request {
            MathRequest::Character(value) => {
                append_math_char(
                    self.modes.current_list_mutation(),
                    &stores.command_context().expect("live generation"),
                    u32::from(value.code),
                    value.provenance.primary,
                )?;
            }
            MathRequest::Delimiter(value) => {
                append_math_char(
                    self.modes.current_list_mutation(),
                    &stores.command_context().expect("live generation"),
                    value.code >> 12,
                    value.provenance.primary,
                )?;
            }
            MathRequest::TextField(kind) => {
                // TeX82 §1151's caller has already allocated the noad and
                // passes `nucleus(tail)` to `scan_math`.  This is observable
                // while a braced field is being scanned: `\showlists` in the
                // nested math level must still display the enclosing noad in
                // its parent level, with an empty subsidiary field.  Reserve
                // that exact parent-list position before entering the live
                // group, then fill it after the scan completes.
                let node_index = self.modes.current_list().nodes().len();
                self.modes
                    .current_list_mutation()
                    .push(Node::MathNoad(MathNoad::new(
                        noad_kind_for_text(kind),
                        MathField::Empty,
                    )));
                let episode = self.command_scan_math_field(stores)?;
                let field = self.execute_math_field(episode, stores)?;
                // TeX82 §1186's second brace simplification: when a braced
                // field contains exactly one accent noad and is the nucleus
                // of an Ord atom, replace that Ord atom by the accent itself.
                // Following scripts must attach to the accent, not to a
                // wrapper whose converted nucleus and scripts become sibling
                // boxes.
                if kind == MathTextFieldKind::Ord
                    && let MathField::SubMlist(ref list) = field
                    && let [Node::MathNoad(accent)] = stores
                        .page_node_list(*list)
                        .expect("math field belongs to the live page arena")
                        .nodes()
                    && matches!(accent.kind, NoadKind::Accent { .. })
                {
                    self.modes
                        .current_list_mutation()
                        .with_node_mut(node_index, |node| {
                            *node = Node::MathNoad(accent.clone());
                        })
                        .expect("reserved math noad must remain present");
                } else {
                    self.modes
                        .current_list_mutation()
                        .with_node_mut(node_index, |node| {
                            let Node::MathNoad(noad) = node else {
                                unreachable!("reserved math noad must remain a noad")
                            };
                            debug_assert!(matches!(noad.nucleus, MathField::Empty));
                            noad.nucleus = field;
                        })
                        .expect("reserved math noad must remain present");
                }
            }
            MathRequest::Script(script) => {
                let target =
                    reserve_script_target(self.modes.current_list_mutation(), stores, script.kind)?;
                let episode = self.command_scan_math_field(stores)?;
                let field = self.execute_math_field(episode, stores)?;
                fill_script_target(self.modes.current_list_mutation(), target, field);
            }
            MathRequest::Limits(kind) => {
                if !apply_limits(self.modes.current_list_mutation(), kind) {
                    // §1159 falls through to the error only when the tail is
                    // not an `op_noad`; the switch is dropped and the job
                    // continues.
                    let context = self
                        .command
                        .output_open_context(&stores.command_context().expect("live generation"));
                    let mut report = stores.print_err("Limit controls must follow a math operator");
                    report.help(&["I'm ignoring this misplaced \\limits or \\nolimits command."]);
                    report.context(context);
                    report.error().jump_out()?;
                }
            }
            MathRequest::Fraction(fraction) => {
                if !start_fraction(self.modes.current_list_mutation(), stores, fraction) {
                    let context = self
                        .command
                        .output_open_context(&stores.command_context().expect("live generation"));
                    let mut report = stores.print_err("Ambiguous; you need another { and }");
                    report.help(&[
                        "I'm ignoring this fraction specification, since I don't",
                        "know whether a construction like `x \\over y \\over z'",
                        "means `{x \\over y} \\over z' or `x \\over {y \\over z}'.",
                    ]);
                    report.context(context);
                    report.error().jump_out()?;
                }
            }
            MathRequest::Style(style) => {
                self.modes
                    .current_list_mutation()
                    .push(Node::MathStyle(match style {
                        MathStyleKind::Display => MathStyle::Display,
                        MathStyleKind::Text => MathStyle::Text,
                        MathStyleKind::Script => MathStyle::Script,
                        MathStyleKind::ScriptScript => MathStyle::ScriptScript,
                    }))
            }
            MathRequest::Choice => {
                // TeX82 §1172's `append_choices` opens the first branch with
                // `push_math(math_choice_group); scan_left_brace`, and
                // §1174's `build_choices` repeats exactly that after storing
                // each finished mlist. All four branches are therefore live
                // `math_choice_group` bodies read by ordinary main control,
                // never token lists absorbed ahead of construction: absorbing
                // them backs the opening brace up a second time (an extra
                // `backed_up` input level TeX never pushes) and reorders
                // every input level the branch body itself opens.
                self.active_math_choices.push(0);
                let branches = (|| {
                    let display = self.execute_math_choice_branch(stores)?;
                    *self
                        .active_math_choices
                        .last_mut()
                        .expect("live math choice") = 1;
                    let text = self.execute_math_choice_branch(stores)?;
                    *self
                        .active_math_choices
                        .last_mut()
                        .expect("live math choice") = 2;
                    let script = self.execute_math_choice_branch(stores)?;
                    *self
                        .active_math_choices
                        .last_mut()
                        .expect("live math choice") = 3;
                    let script_script = self.execute_math_choice_branch(stores)?;
                    Ok::<_, ExecError>((display, text, script, script_script))
                })();
                self.active_math_choices.pop();
                let (display, text, script, script_script) = branches?;
                self.modes
                    .current_list_mutation()
                    .push(Node::MathChoice(MathChoice {
                        display,
                        text,
                        script,
                        script_script,
                    }));
            }
            MathRequest::Radical(delimiter) => {
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
            MathRequest::Accent { character } => {
                let accent = if let Some(accent) = character {
                    accent
                } else {
                    // TeX82 §1110 reports before `math_ac` advances to
                    // §436's `scan_fifteen_bit_int`. In particular, §82's
                    // context must still own an exhausted token-list level
                    // whose last command was the text `\accent`.
                    let context = self
                        .command
                        .output_open_context(&stores.command_context().expect("live generation"));
                    let mut report =
                        stores.print_err("Please use \\mathaccent for accents in math mode");
                    report.help(&[
                        "I'm changing \\accent to \\mathaccent here; wish me luck.",
                        "(Accents are not the same in formulas as they are in text.)",
                    ]);
                    report.context(context);
                    report.error().jump_out()?;
                    self.command_scan_math_character(stores)?
                };
                let episode = self.command_scan_math_field(stores)?;
                let field = self.execute_math_field(episode, stores)?;
                let accent = math_char(
                    &stores.command_context().expect("live generation"),
                    u32::from(accent.code),
                    accent.provenance.primary,
                )?
                .1;
                self.modes
                    .current_list_mutation()
                    .push(Node::MathNoad(MathNoad::new(
                        NoadKind::Accent { accent },
                        field,
                    )));
            }
            MathRequest::MuMaterial(ScannedMathMuMaterial::Glue(glue)) => {
                self.modes.current_list_mutation().push(Node::Glue {
                    spec: glue,
                    kind: GlueKind::MuSkip,
                    leader: None,
                })
            }
            MathRequest::MuMaterial(ScannedMathMuMaterial::Kern(amount)) => {
                self.modes.current_list_mutation().push(Node::Kern {
                    amount,
                    kind: KernKind::Mu,
                })
            }
            MathRequest::EquationNumber(number) => {
                if self.modes.current_mode() != Mode::DisplayMath {
                    // §1140's `mmode+eq_no` is guarded by `privileged`, and
                    // §1049 lists `eq_no` under `non_math`; both failures end
                    // in §1050's `report_illegal_case`, which names the mode
                    // the command was actually used in.
                    let primitive = match number.side {
                        tex_command::EquationNumberSide::Left => "leqno",
                        tex_command::EquationNumberSide::Right => "eqno",
                    };
                    let mode = self.modes.current_mode();
                    let mut command_context = stores.command_context().expect("live generation");
                    let token = Token::Cs(
                        command_context
                            .known_control_sequence(primitive)
                            .expect("equation-number primitives are installed"),
                    );
                    let context = self.command.output_open_context(&command_context);
                    crate::diagnostics::report_illegal_case_with_context(
                        &mut command_context,
                        token,
                        mode,
                        Some(context),
                    )?;
                } else {
                    let display = take_finished_math_list(&mut self.modes, stores)?;
                    let mut context = stores.command_context().expect("live generation");
                    enter_group(&mut context, &mut self.command, GroupKind::MathShift);
                    context
                        .assign_int_param(IntParam::FAM, -1, tex_state::AssignmentScope::Local)
                        .expect("family parameter is admitted");
                    self.modes.push_at_line(
                        Mode::Math,
                        self.command
                            .current_file_line_number()
                            .try_into()
                            .unwrap_or(i32::MAX),
                    )?;
                    let side = match number.side {
                        tex_command::EquationNumberSide::Left => crate::mode::EqNoSide::Left,
                        tex_command::EquationNumberSide::Right => crate::mode::EqNoSide::Right,
                    };
                    let shift = MathShiftContext::EqNo(side);
                    self.active_math_shifts.push(shift);
                    self.modes
                        .current_list_mutation()
                        .set_display_eq_no(crate::mode::DisplayEqNo { side, display });
                }
            }
            MathRequest::Family(_) => {}
        }
        Ok(ReplayStep::Continue)
    }

    /// TeX82 §1207's missing-display-closer recovery for any non-math-shift
    /// command left by do_assignments.
    fn recover_display_alignment_closer(
        &mut self,
        stores: &mut Universe<G>,
    ) -> Result<ReplayStep, ExecError> {
        let Some((nodes, aux_prev_depth)) =
            self.modes.current_list_mutation().take_display_alignment()
        else {
            return Err(ExecError::MissingToken {
                context: "display alignment recovery",
            });
        };

        // §1207 calls back_error before resume_after_display. The backup
        // level must already be live when §82 renders context, and ordinary
        // main control must retry the command in the resumed paragraph.
        let context = self
            .command
            .output_open_context(&stores.command_context().expect("live generation"));
        crate::error_report::report_error(
            &mut stores
                .command_context()
                .expect("display diagnostic admission"),
            "Missing $$ inserted",
            &[
                "Displays can use special alignments (like \\eqalignno)",
                "only if nothing but the alignment itself is between $$'s.",
            ],
            context,
        )?;
        self.finish_display_alignment(
            stores,
            crate::align::FinishedAlignment {
                nodes,
                aux_prev_depth,
                aux_space_factor: None,
            },
        )?;
        Ok(ReplayStep::Continue)
    }

    fn apply_math_shift(
        &mut self,
        pairing: MathShiftPairing,
        stores: &mut Universe<G>,
    ) -> Result<ReplayStep, ExecError> {
        match self.modes.current_mode() {
            Mode::Horizontal | Mode::RestrictedHorizontal => {
                debug_assert_ne!(pairing, MathShiftPairing::ProbeDisplayEnd);
                crate::box_runtime::flush_pending_hchars_with_fuel(
                    &mut self.modes,
                    &mut stores.command_context().expect("math-shift admission"),
                    self.fuel.fuel_mut(),
                )?;
                // §1138 already applied its own `mode>0` test while probing:
                // in restricted horizontal mode the second `$` was backed up
                // rather than consumed, so `paired` is false there and this
                // must not retest the mode and disagree with the backup.
                if pairing == MathShiftPairing::Paired {
                    self.enter_display(stores)?;
                } else {
                    self.enter_math_level(false, stores)?;
                    schedule_everymath(
                        &mut self.command,
                        &mut stores.command_context().expect("everymath admission"),
                        false,
                    );
                }
            }
            Mode::Math => {
                if self.modes.current_list().display_eq_no().is_some() {
                    debug_assert_eq!(pairing, MathShiftPairing::ProbeDisplayEnd);
                    let content = self.prepare_math_list(stores)?;
                    let eq = self.finish_equation_number_mlist(stores)?;
                    let paired = self.scan_display_end(stores)?;
                    if !paired {
                        report_unpaired_display_end(&self.command, stores)?;
                    }
                    let (display, finished) =
                        self.finish_equation_number_group(stores, eq, content)?;
                    self.finish_display_math_content(stores, display, Some(finished), false, None)?;
                } else {
                    debug_assert_eq!(pairing, MathShiftPairing::Unpaired);
                    self.finish_inline_math(stores)?;
                }
            }
            Mode::DisplayMath => {
                debug_assert_eq!(pairing, MathShiftPairing::ProbeDisplayEnd);
                if let Some((nodes, aux_prev_depth)) =
                    self.modes.current_list_mutation().take_display_alignment()
                {
                    let paired = self.scan_display_end(stores)?;
                    if !paired {
                        report_unpaired_display_end(&self.command, stores)?;
                    }
                    self.finish_display_alignment(
                        stores,
                        crate::align::FinishedAlignment {
                            nodes,
                            aux_prev_depth,
                            aux_space_factor: None,
                        },
                    )?;
                    return Ok(ReplayStep::Continue);
                }
                let (content, display_level) = self.prepare_display_math_list(stores)?;
                let paired = self.scan_display_end(stores)?;
                if !paired {
                    report_unpaired_display_end(&self.command, stores)?;
                }
                self.finish_display_math_content(stores, content, None, true, Some(display_level))?;
            }
            Mode::Vertical | Mode::InternalVertical => {
                unreachable!("vertical math shifts retry through ParagraphStart")
            }
        }
        Ok(ReplayStep::Continue)
    }

    /// TeX82 §1194 checks the current formula's math fonts before `fin_mlist`
    /// and before §1197 probes for the second display-closing `$`.
    fn prepare_math_list(
        &mut self,
        stores: &mut Universe<G>,
    ) -> Result<tex_state::node_arena::PageListId, ExecError> {
        let rejected = {
            let mut context = stores.command_context().expect("math-font admission");
            let math_font_context = self.command.output_open_context(&context);
            crate::math::reject_invalid_math_fonts(&mut context, math_font_context)?
        };
        let content = take_finished_math_list(&mut self.modes, stores)?;
        Ok(if rejected {
            tex_state::node_arena::PageListId::empty()
        } else {
            content
        })
    }

    /// TeX82 §§1185/1194/1197's display `fin_mlist(null)`: detach the
    /// completed display mode before expanding the required second `$`.
    fn prepare_display_math_list(
        &mut self,
        stores: &mut Universe<G>,
    ) -> Result<
        (
            tex_state::node_arena::PageListId,
            crate::mode::ModeLevelSummary,
        ),
        ExecError,
    > {
        let content = self.prepare_math_list(stores)?;
        let level = crate::box_runtime::commit_current_list(
            &mut self.modes,
            &mut stores.command_context().expect("math-list admission"),
            self.fuel.fuel_mut(),
        )?;
        Ok((content, level))
    }

    fn scan_display_end(&mut self, stores: &mut Universe<G>) -> Result<bool, ExecError> {
        // TeX82 §§1185/1194's `fin_mlist` has already popped the display
        // level. Publish that live nest before §1197's nested expansion
        // episode: capabilities were last sampled at the start of the outer
        // main-control step, when the current mode was still display math.
        self.refresh_host_capabilities(stores);
        let mode = self.modes.current_mode();
        let shown_mode = self.shown_mode;
        let context = stores
            .command_context()
            .expect("display-end scan requires a live generation");
        let mut machine = self.command_machine();
        let mut processor = machine.processor(context);
        prepare_command_trace(&mut processor, mode, shown_mode);
        let paired = processor
            .scan_display_end_math_shift()
            .map_err(command_error)?;
        let command_trace_printed = processor.command_trace_printed();
        drop(processor);
        if command_trace_printed {
            *machine.shown_mode = Some(mode);
        }
        Ok(paired)
    }

    fn enter_math_level(
        &mut self,
        display: bool,
        stores: &mut Universe<G>,
    ) -> Result<(), ExecError> {
        let mut context = stores.command_context().expect("live generation");
        enter_group(&mut context, &mut self.command, GroupKind::MathShift);
        context
            .assign_int_param(IntParam::FAM, -1, tex_state::AssignmentScope::Local)
            .expect("family parameter is admitted");
        drop(context);
        self.modes.push_at_line(
            if display {
                Mode::DisplayMath
            } else {
                Mode::Math
            },
            self.command
                .current_file_line_number()
                .try_into()
                .unwrap_or(i32::MAX),
        )?;
        let shift = if display {
            MathShiftContext::Display
        } else {
            MathShiftContext::Inline
        };
        self.active_math_shifts.push(shift);
        Ok(())
    }

    fn enter_display(&mut self, stores: &mut Universe<G>) -> Result<(), ExecError> {
        let (paragraph, dimensions, pre_display_size, prototype, extended) = {
            let mut context = stores.command_context().expect("live generation");
            let error_context = crate::diagnostics::ExecutionDiagnosticContext::source_free(
                self.command.output_open_context(&context),
            );
            let paragraph = crate::paragraph_end::interrupt_paragraph_for_display(
                &mut self.modes,
                &mut context,
                self.fuel.fuel_mut(),
                error_context,
            )?;
            let dimensions = crate::paragraph_end::display_line_dimensions(&self.modes, &context);
            let pre_display_size = paragraph
                .last_line
                .as_ref()
                .map_or(Scaled::from_raw(-Scaled::MAX_DIMEN.raw()), |line| {
                    crate::math::display::pre_display_size(&context, line)
                });
            let extended = context.int_param(IntParam::ETEX_EXTENDED_MODE) > 0;
            let prototype = if extended {
                paragraph.last_line.as_ref().map(|line| {
                    crate::math::display::display_line_prototype(&mut context, line.clone())
                })
            } else {
                None
            };
            (paragraph, dimensions, pre_display_size, prototype, extended)
        };
        // TeX82 §1145 opens `math_shift_group` before these local parameter
        // definitions, so §283 restores all of them when the display ends.
        // `\everydisplay` is scheduled only after the definitions are live.
        self.enter_math_level(true, stores)?;
        let mut context = stores.command_context().expect("live generation");
        for (parameter, value) in [
            (DimenParam::PRE_DISPLAY_SIZE, pre_display_size),
            (DimenParam::DISPLAY_WIDTH, dimensions.width),
            (DimenParam::DISPLAY_INDENT, dimensions.indent),
        ] {
            context
                .assign_dimen_param(parameter, value, tex_state::AssignmentScope::Local)
                .expect("display dimension parameter is admitted");
        }
        // e-TeX 2.6 [32.1145] adds this definition only in extended mode;
        // TeX82 compatibility mode has no corresponding save-stack word.
        if extended {
            context
                .assign_int_param(
                    IntParam::PRE_DISPLAY_DIRECTION,
                    match paragraph.active_directions.last() {
                        Some(tex_state::node::Direction::BeginL) => 1,
                        Some(tex_state::node::Direction::BeginR) => -1,
                        _ => 0,
                    },
                    tex_state::AssignmentScope::Local,
                )
                .expect("display direction parameter is admitted");
        }
        schedule_everymath(&mut self.command, &mut context, true);
        self.modes
            .current_list_mutation()
            .set_display_interrupt(crate::mode::DisplayInterrupt {
                active_directions: paragraph.active_directions,
                prototype,
            });
        Ok(())
    }

    fn finish_inline_math(&mut self, stores: &mut Universe<G>) -> Result<(), ExecError> {
        let mut content = take_finished_math_list(&mut self.modes, stores)?;
        let mut context =
            LinearCommandContext::new(stores.command_context().expect("inline-math admission"));
        let diagnostic_text = self.command.output_open_context(&context);
        let conversion_error_context =
            crate::math::MathConversionErrorContext::new(diagnostic_text.clone());
        if crate::math::reject_invalid_math_fonts(&mut context, diagnostic_text.clone())? {
            content = tex_state::node_arena::PageListId::empty();
        }
        let _ = crate::box_runtime::commit_current_list(
            &mut self.modes,
            &mut context,
            self.fuel.fuel_mut(),
        )?;
        let insert_penalties = self.modes.current_mode() == Mode::Horizontal;
        let (nodes, _) = crate::math::finish_inline_math_list_node(
            &mut context,
            tex_state::math::MathListNode {
                display: false,
                content,
            },
            insert_penalties,
            conversion_error_context,
        );
        self.modes.current_list_mutation().append(nodes);
        self.modes.current_list_mutation().set_space_factor(1000);
        let aftergroup =
            leave_group_payloads(&mut context, &mut self.command, GroupKind::MathShift).map_err(
                |_| ExecError::MissingToken {
                    context: "math shift group",
                },
            )?;
        self.active_math_shifts.pop();
        schedule_aftergroup(&mut self.command_machine(), &mut context, aftergroup)?;
        Ok(())
    }

    fn finish_equation_number_mlist(
        &mut self,
        stores: &mut Universe<G>,
    ) -> Result<crate::mode::DisplayEqNo, ExecError> {
        let mut level = crate::box_runtime::commit_current_list(
            &mut self.modes,
            &mut stores.command_context().expect("equation-number admission"),
            self.fuel.fuel_mut(),
        )?;
        let eq = level
            .list_mutation()
            .take_display_eq_no()
            .expect("equation number mode state");
        // TeX82 §§1193/1197 restore the saved display mlist and mode before
        // expanding the required second `$`. Equation-number packing and
        // §1197 recovery both follow that probe, while its group remains live.
        Ok(eq)
    }

    fn finish_equation_number_group(
        &mut self,
        stores: &mut Universe<G>,
        eq: crate::mode::DisplayEqNo,
        content: tex_state::node_arena::PageListId,
    ) -> Result<
        (
            tex_state::node_arena::PageListId,
            crate::math::display::FinishedEqNo,
        ),
        ExecError,
    > {
        let mut context =
            LinearCommandContext::new(stores.command_context().expect("equation-number admission"));
        let diagnostic_text = self.command.output_open_context(&context);
        let conversion_error_context =
            crate::math::MathConversionErrorContext::new(diagnostic_text.clone());
        let diagnostic_context =
            crate::diagnostics::ExecutionDiagnosticContext::source_free(diagnostic_text);
        let finished = crate::math::display::finish_eq_no(
            &mut context,
            &diagnostic_context,
            eq.side,
            content,
            Some(&conversion_error_context),
        );
        let aftergroup =
            leave_group_payloads(&mut context, &mut self.command, GroupKind::MathShift).map_err(
                |_| ExecError::MissingToken {
                    context: "equation number group",
                },
            )?;
        self.active_math_shifts.pop();
        schedule_aftergroup(&mut self.command_machine(), &mut context, aftergroup)?;
        Ok((eq.display, finished))
    }

    fn finish_display_alignment(
        &mut self,
        stores: &mut Universe<G>,
        finished: crate::align::FinishedAlignment,
    ) -> Result<(), ExecError> {
        self.finish_display_alignment_inner(stores, finished, true)
    }

    fn finish_display_alignment_inner(
        &mut self,
        stores: &mut Universe<G>,
        finished: crate::align::FinishedAlignment,
        scan_optional_space: bool,
    ) -> Result<(), ExecError> {
        let mut context = LinearCommandContext::new(
            stores
                .command_context()
                .expect("display-alignment admission"),
        );
        let mut level = crate::box_runtime::commit_current_list(
            &mut self.modes,
            &mut context,
            self.fuel.fuel_mut(),
        )?;
        let interrupt =
            level
                .list_mutation()
                .take_display_interrupt()
                .ok_or(ExecError::MissingToken {
                    context: "display alignment interrupt",
                })?;
        crate::math::display::finish_display_alignment(&mut self.modes, &mut context, finished)?;
        let aftergroup =
            leave_group_payloads(&mut context, &mut self.command, GroupKind::MathShift).map_err(
                |_| ExecError::MissingToken {
                    context: "display alignment group",
                },
            )?;
        self.active_math_shifts.pop();
        schedule_aftergroup(&mut self.command_machine(), &mut context, aftergroup)?;
        drop(context);
        self.resume_display_inner(stores, interrupt.active_directions, scan_optional_space)
    }

    fn finish_display_math_content(
        &mut self,
        stores: &mut Universe<G>,
        mut content: tex_state::node_arena::PageListId,
        eq_no: Option<crate::math::display::FinishedEqNo>,
        fonts_checked: bool,
        display_level: Option<crate::mode::ModeLevelSummary>,
    ) -> Result<(), ExecError> {
        let mut context =
            LinearCommandContext::new(stores.command_context().expect("display-math admission"));
        let diagnostic_text = self.command.output_open_context(&context);
        let conversion_error_context =
            crate::math::MathConversionErrorContext::new(diagnostic_text.clone());
        let diagnostic_context =
            crate::diagnostics::ExecutionDiagnosticContext::source_free(diagnostic_text.clone());
        // TeX82 §1194 performs this check before every display `fin_mlist`,
        // including the saved outer mlist after an equation number.
        if !fonts_checked && crate::math::reject_invalid_math_fonts(&mut context, diagnostic_text)?
        {
            content = tex_state::node_arena::PageListId::empty();
        }
        let mut level = match display_level {
            Some(level) => level,
            None => crate::box_runtime::commit_current_list(
                &mut self.modes,
                &mut context,
                self.fuel.fuel_mut(),
            )?,
        };
        let interrupt =
            level
                .list_mutation()
                .take_display_interrupt()
                .ok_or(ExecError::MissingToken {
                    context: "display interrupt",
                })?;
        crate::math::display::finish_display_math(
            &mut self.modes,
            &mut context,
            &diagnostic_context,
            content,
            eq_no,
            interrupt.prototype,
            Some(&conversion_error_context),
        )?;
        let aftergroup =
            leave_group_payloads(&mut context, &mut self.command, GroupKind::MathShift).map_err(
                |_| ExecError::MissingToken {
                    context: "display math group",
                },
            )?;
        self.active_math_shifts.pop();
        schedule_aftergroup(&mut self.command_machine(), &mut context, aftergroup)?;
        drop(context);
        self.resume_display(stores, interrupt.active_directions)
    }

    fn resume_display(
        &mut self,
        stores: &mut Universe<G>,
        directions: Vec<tex_state::node::Direction>,
    ) -> Result<(), ExecError> {
        self.resume_display_inner(stores, directions, true)
    }

    fn resume_display_inner(
        &mut self,
        stores: &mut Universe<G>,
        directions: Vec<tex_state::node::Direction>,
        scan_optional_space: bool,
    ) -> Result<(), ExecError> {
        let prev = self
            .modes
            .enclosing_vertical_prev_graf()
            .checked_add(3)
            .expect("display prev_graf overflow");
        self.modes.set_enclosing_vertical_prev_graf(prev);
        self.modes.push_at_line(
            Mode::Horizontal,
            self.command
                .current_file_line_number()
                .try_into()
                .unwrap_or(i32::MAX),
        )?;
        // §1200's `push_nest` sets `mode_line:=line` like every other one, so
        // the paragraph fragment that follows a display reports its own
        // over/underfull lines as §663's "in paragraph at lines A--B" rather
        // than falling back to "detected at line B" for want of a
        // `pack_begin_line`.
        let (language, left, right) = {
            let context = stores.command_context().expect("live generation");
            crate::box_runtime::hmode::current_hyphen_context(&context)
        };
        self.modes
            .current_list_mutation()
            .set_hyphen_context(language, left, right);
        self.modes.current_list_mutation().set_space_factor(1000);
        self.modes
            .current_list_mutation()
            .append(directions.into_iter().map(Node::Direction));
        if scan_optional_space {
            self.scan_optional_space(stores)?;
        }
        let mut context = stores.command_context().expect("display-resume admission");
        let error_context = self.command.output_open_context(&context);
        crate::math::display::build_page_after_display_resume(
            &self.modes,
            &mut context,
            &error_context,
        )
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
    fn scan_optional_space(&mut self, stores: &mut Universe<G>) -> Result<(), ExecError> {
        let mode = self.modes.current_mode();
        let shown_mode = self.shown_mode;
        let context = stores
            .command_context()
            .expect("optional-space scan requires a live generation");
        let mut machine = self.command_machine();
        let mut processor = machine.processor(context);
        let mut diagnostics = Vec::new();
        // TeX82 §§299/1200: resume_after_display has already pushed the new
        // horizontal mode when its scanner expands this token. The expansion
        // therefore owns the same pending mode prefix as every other
        // get_x_token boundary, including §1197's staged display-end probe.
        prepare_command_trace(&mut processor, mode, shown_mode);
        let fetched = processor.get_x_token();
        let command_trace_printed = processor.command_trace_printed();
        match fetched {
            Ok(Some(command))
                if !matches!(
                    command.meaning(),
                    ResolvedMeaning::Static(Meaning::CharToken {
                        cat: Catcode::Space,
                        ..
                    })
                ) =>
            {
                processor.back_input(command).map_err(command_error)?;
            }
            Ok(_) => {}
            Err(err) => return Err(command_error(err)),
        }
        diagnostics.extend(
            processor
                .take_semantic_diagnostics()
                .into_iter()
                .map(PendingDiagnostic::Command),
        );
        drop(processor);
        if command_trace_printed {
            *machine.shown_mode = Some(mode);
        }
        // §1200 performs this expanded fetch synchronously before §1125's
        // page builder. Diagnostics produced by expansion therefore belong
        // to this nested scanner boundary; leaving them on CommandState<G> lets
        // the following outer main-control step report them only after
        // build_page has emitted its tracingpages state.
        self.capture_first_causal_context(stores, &diagnostics);
        let mut context = stores.command_context().expect("live generation");
        report_pending_diagnostics(&mut context, diagnostics)
    }

    fn apply_math_delimiter(
        &mut self,
        boundary: MathDelimiterBoundary,
        stores: &mut Universe<G>,
    ) -> Result<ReplayStep, ExecError> {
        match boundary.kind {
            MathDelimiterBoundaryKind::Left => {
                // TeX82 §1191's `push_math(math_left_group)` opens both a
                // mode level and a save-stack level. Keeping those owners
                // paired lets §1193 route a premature `$` through §1027's
                // `off_save`, which inserts `\right.` before retrying it.
                enter_group(
                    &mut stores.command_context().expect("math-left admission"),
                    &mut self.command,
                    GroupKind::MathLeft,
                );
                self.modes.push_at_line(
                    Mode::Math,
                    self.command
                        .current_file_line_number()
                        .try_into()
                        .unwrap_or(i32::MAX),
                )?;
                self.active_math_left_boundaries.push(false);
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
                if left_group_open(&self.modes, stores) {
                    // e-TeX 2.6 [48.1191] treats `\middle` as a boundary
                    // between two consecutive `math_left_group` segments:
                    // `fin_mlist; unsave; push_math(math_left_group)`.  In
                    // particular, assignments made since the preceding
                    // `\left` or `\middle` are restored before the next
                    // segment starts.
                    let content = take_finished_math_list(&mut self.modes, stores)?;
                    let mut context = LinearCommandContext::new(
                        stores.command_context().expect("math-middle admission"),
                    );
                    let _ = crate::box_runtime::commit_current_list(
                        &mut self.modes,
                        &mut context,
                        self.fuel.fuel_mut(),
                    )?;
                    let aftergroup =
                        leave_group_payloads(&mut context, &mut self.command, GroupKind::MathLeft)
                            .map_err(|_| ExecError::MissingToken {
                                context: "math left group",
                            })?;
                    self.active_math_left_boundaries.pop();
                    schedule_aftergroup(&mut self.command_machine(), &mut context, aftergroup)?;

                    enter_group(&mut context, &mut self.command, GroupKind::MathLeft);
                    self.modes.push_at_line(
                        Mode::Math,
                        self.command
                            .current_file_line_number()
                            .try_into()
                            .unwrap_or(i32::MAX),
                    )?;
                    self.active_math_left_boundaries.push(true);
                    let segment = context
                        .page_node_list(content)
                        .expect("math segment belongs to the live page arena")
                        .nodes()
                        .to_vec();
                    self.modes
                        .current_list_mutation()
                        .append(segment.into_iter().chain([Node::MathNoad(MathNoad::new(
                            NoadKind::MiddleDelimiter {
                                delimiter: boundary.delimiter.code,
                            },
                            MathField::Empty,
                        ))]));
                } else {
                    // etex.ch [48.1192] splits §1192's report by noad type.
                    let mut command_context = stores.command_context().expect("live generation");
                    let context = self.command.output_open_context(&command_context);
                    report_escaped_error(
                        &mut command_context,
                        "Extra ",
                        "middle",
                        "",
                        &["I'm ignoring a \\middle that had no matching \\left."],
                        context,
                    )?;
                }
            }
            MathDelimiterBoundaryKind::Right => {
                if !left_group_open(&self.modes, stores) {
                    // TeX82 §1192's `<Try to recover from mismatched \right>`
                    // in its `math_shift_group` arm.
                    let mut command_context = stores.command_context().expect("live generation");
                    let context = self.command.output_open_context(&command_context);
                    report_escaped_error(
                        &mut command_context,
                        "Extra ",
                        "right",
                        "",
                        &["I'm ignoring a \\right that had no matching \\left."],
                        context,
                    )?;
                    return Ok(ReplayStep::Continue);
                }
                let content = take_finished_math_list(&mut self.modes, stores)?;
                let mut context = LinearCommandContext::new(
                    stores.command_context().expect("math-right admission"),
                );
                let _ = crate::box_runtime::commit_current_list(
                    &mut self.modes,
                    &mut context,
                    self.fuel.fuel_mut(),
                )?;
                let aftergroup =
                    leave_group_payloads(&mut context, &mut self.command, GroupKind::MathLeft)
                        .map_err(|_| ExecError::MissingToken {
                            context: "math left group",
                        })?;
                self.active_math_left_boundaries.pop();
                schedule_aftergroup(&mut self.command_machine(), &mut context, aftergroup)?;
                let mut nodes = context
                    .page_node_list(content)
                    .expect("math segment belongs to the live page arena")
                    .nodes()
                    .to_vec();
                nodes.push(Node::MathNoad(MathNoad::new(
                    NoadKind::RightDelimiter {
                        delimiter: boundary.delimiter.code,
                    },
                    MathField::Empty,
                )));
                let content = context.publish_page_nodes(nodes);
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
        stores: &mut Universe<G>,
    ) -> Result<tex_command::MathFieldEpisode, ExecError> {
        let mut processor = command_processor(
            &mut self.command,
            self.fuel.fuel_mut(),
            &mut self.capabilities,
            &mut self.operation_observations,
            stores.command_context().expect("live generation"),
        );
        let scanned = processor.scan_math_field_episode();
        scanned.map_err(command_error)
    }

    /// Runs TeX82 §436's `scan_fifteen_bit_int` after an executor-owned
    /// diagnostic has completed, as required by §1110's text-accent path.
    fn command_scan_math_character(
        &mut self,
        stores: &mut Universe<G>,
    ) -> Result<tex_command::ScannedMathCharacter, ExecError> {
        let mut processor = command_processor(
            &mut self.command,
            self.fuel.fuel_mut(),
            &mut self.capabilities,
            &mut self.operation_observations,
            stores.command_context().expect("live generation"),
        );
        processor.scan_math_character().map_err(command_error)
    }

    /// TeX82 §1172/§1174's `scan_left_brace` for one `\mathchoice` branch.
    /// §403 recovery opens the group anyway, so the recovered flag is
    /// diagnostic only.
    fn command_scan_math_choice_group(
        &mut self,
        stores: &mut Universe<G>,
    ) -> Result<bool, ExecError> {
        let mut processor = command_processor(
            &mut self.command,
            self.fuel.fuel_mut(),
            &mut self.capabilities,
            &mut self.operation_observations,
            stores.command_context().expect("live generation"),
        );
        let scanned = processor.scan_math_choice_group();
        scanned.map_err(command_error)
    }

    /// Delivers and executes one replay command while forwarding committed
    /// command-owned observations in their original order.
    pub fn step_with_observer(
        &mut self,
        stores: &mut Universe<G>,
        observer: &mut dyn CommandObserver,
    ) -> Result<ReplayStep, ExecError> {
        match self.advance_with_observer(stores, observer)? {
            StepResult::Progress(step) => Ok(step),
            StepResult::Suspended(ResourceNeed::Input { .. }) => {
                Err(ExecError::MissingToken { context: "\\input" })
            }
            StepResult::Suspended(ResourceNeed::InputProbe { .. }) => {
                Err(ExecError::MissingToken {
                    context: "pdfTeX file enquiry",
                })
            }
            StepResult::Suspended(ResourceNeed::Font { .. }) => Err(ExecError::MissingToken {
                context: "\\font resource",
            }),
            StepResult::Suspended(ResourceNeed::PdfImage { .. }) => Err(ExecError::MissingToken {
                context: "\\pdfximage resource",
            }),
        }
    }

    /// Atomic observed variant of [`Self::advance`]. Observations are held
    /// until both command delivery and executor application have committed.
    pub fn advance_with_observer(
        &mut self,
        stores: &mut Universe<G>,
        observer: &mut dyn CommandObserver,
    ) -> Result<StepResult, ExecError> {
        if !self.pure_memo_initialized {
            let runtime = stores.take_pure_memo_config().map_or_else(
                tex_state::PureMemoRuntime::default,
                tex_state::PureMemoRuntime::new,
            );
            self.install_pure_memo_runtime(runtime);
        }
        stores.attach_pure_memo_capability(&self.pure_memo);
        if self.fatal.is_some() {
            return Ok(StepResult::Progress(MainControlStep::End));
        }
        let effect_start = stores.world().effect_pos();
        let artifact_start = stores.world().artifact_commits().len();
        // Occupying the slot is what makes this operation observed. Every
        // command-processor episode the operation runs, including the nested
        // ones a host-applied step runs, publishes into this one buffer.
        if let Some((pending, start)) = self.suspended_operation_observation.take() {
            self.operation_observations = Some(pending);
            self.operation_receipt_start = Some(start);
        } else {
            self.operation_observations = Some(ObservationBuffer::default());
            self.operation_receipt_start = Some(OperationReceiptStart {
                effect: effect_start.raw(),
                artifact: artifact_start,
            });
        }
        let stepped = self.execute_operation(
            stores,
            OperationDelivery::<G>::Replay(None),
            OperationTransaction::Advance,
            1,
            None,
        );
        let mut pending = self.operation_observations.take().unwrap_or_default();
        let receipt_start = self.operation_receipt_start.take();
        if matches!(stepped, Ok(StepResult::Suspended(_)))
            && (self.pending_resource_operation.is_some()
                || self.pending_preflight_command.is_some()
                || self.pending_alignment_delivery.is_some())
        {
            let start = receipt_start.expect("observed operation owns a receipt start");
            self.suspended_operation_observation = Some((pending, start));
            return stepped;
        }
        match &stepped {
            Ok(StepResult::Progress(_)) => {}
            Ok(StepResult::Suspended(_)) => {}
            Err(error) => pending.receipt.set_termination(
                error
                    .as_fatal()
                    .map_or(OperationTermination::Failed, OperationTermination::Fatal),
            ),
        }
        let expected_termination = if let Some(fatal) = self.fatal {
            OperationTermination::Fatal(fatal)
        } else {
            match &stepped {
                Ok(StepResult::Progress(MainControlStep::Continue)) => {
                    OperationTermination::Continue
                }
                Ok(StepResult::Progress(MainControlStep::End)) => OperationTermination::End,
                Ok(StepResult::Progress(MainControlStep::EndOfInput)) => {
                    OperationTermination::EndOfInput
                }
                Ok(StepResult::Suspended(_)) => OperationTermination::Suspended,
                Err(error) => error
                    .as_fatal()
                    .map_or(OperationTermination::Failed, OperationTermination::Fatal),
            }
        };
        let publish = matches!(stepped, Ok(StepResult::Progress(_)));
        let consumed = pending.consume_into(publish.then_some(observer));
        debug_assert!(consumed.records <= MAX_EXECUTION_RECEIPT_RECORDS);
        debug_assert_eq!(consumed.termination, expected_termination);
        stepped
    }

    /// TeX82 §93's `succumb`: `history:=fatal_error_stop; jump_out`.
    ///
    /// `jump_out` cuts across every active procedure level and lands at
    /// `end_of_TEX`, where §1332's `close_files_and_terminate` finishes the
    /// job. A library engine has no process to leave, so the driver -- the
    /// only frame that corresponds to `end_of_TEX` -- latches the terminal
    /// state and reports the job over. Nothing is rolled back: `jump_out`
    /// abandons the current procedure, it does not undo it.
    pub(crate) fn succumb(&mut self, fatal: FatalError) -> MainControlStep {
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

    /// Reconstructs the source-bearing fatal for a diagnostic session owner.
    pub(crate) fn captured_fatal_error(&self) -> Option<ExecError> {
        let fatal = self.fatal?;
        let (site, frozen, context) = self.captured_fatal_origin.as_ref()?;
        Some(ExecError::Captured {
            error: Box::new(ExecError::Fatal(fatal)),
            site: *site,
            frozen: Some(Box::new(crate::FrozenDiagnosticEvidence {
                origin: frozen.clone(),
                context: context.clone(),
            })),
        })
    }

    /// Delivers, expands, and (for ranked ordinary families) scans the next
    /// command before choosing rollback authority.
    ///
    /// Delivery, expansion, tracing, and ordinary hot scanning mutate only
    /// the command/input machine, so they share one processor borrow. A typed
    /// hot operand releases that borrow before semantic application. Resource,
    /// transaction, diagnostic, and alignment cases remain explicit barriers
    /// and retain only their exact continuation tag.
    fn preflight_replay_delivery(
        &mut self,
        stores: &mut Universe<G>,
    ) -> Result<Option<PreflightDelivery<G>>, ExecError> {
        let mode = self.modes.current_mode();
        if self.active_alignment.is_some()
            || (mode == Mode::DisplayMath && self.modes.current_list().has_display_alignment())
        {
            return Ok(Some(PreflightDelivery::<G> {
                delivery: OperationDelivery::<G>::Replay(None),
                capabilities: crate::transaction_protocol::canonical_static_command_capabilities(
                    Meaning::Relax,
                ),
            }));
        }

        if self.enter_main_control(stores) {
            let entry_records: Vec<CommandObservation> = self
                .command
                .publish_named_token_list_pushes(
                    &mut stores.command_context().expect("live generation"),
                )
                .into_iter()
                .map(CommandObservation::Input)
                .collect();
            self.observe_committed(entry_records);
        }
        self.drain_file_framing_events(stores);
        self.refresh_host_capabilities(stores);

        let innermost_group = stores
            .command_context()
            .expect("live generation")
            .innermost_group_kind();
        let mut diagnostics = Vec::new();
        let raw_main_loop_delivery = self.main_loop_active;
        let (delivery, settled_in_preflight, trace_reported, fused_hot, fused_retry, fused_error) = {
            let mut processor = command_processor(
                &mut self.command,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
                &mut self.operation_observations,
                stores.command_context().expect("live generation"),
            );
            processor
                .apply_error_stop_recovery()
                .map_err(command_error)?;
            let delivery = processor
                .get_next_with_replay_completion()
                .map_err(command_error)?;
            let mut settled_in_preflight = false;
            let mut delivery = match delivery {
                Some(tex_command::CommandReplayDelivery::Command(command))
                    if matches!(
                        command.meaning(),
                        ResolvedMeaning::Macro { .. }
                            | ResolvedMeaning::Static(Meaning::ExpandablePrimitive(_))
                            | ResolvedMeaning::Static(Meaning::Undefined | Meaning::Unknown(_))
                    ) =>
                {
                    prepare_command_trace(&mut processor, mode, self.shown_mode);
                    match processor.settle_current_command(command) {
                        Ok(settled) => {
                            settled_in_preflight = true;
                            settled.map(tex_command::CommandReplayDelivery::Command)
                        }
                        Err(error) => {
                            // The expansion driver moves its live command into
                            // command state only after an actual immutable-host
                            // suspension. Fuel and semantic failures have no
                            // retry command and must not clone one speculatively.
                            let retry =
                                processor
                                    .pending_expansion_command()
                                    .cloned()
                                    .map(|command| PendingPreflightCommand::<G>::Expanding {
                                        command,
                                        main_loop: self.main_loop_active,
                                        cursor: processor.delivery_cursor(),
                                    });
                            drop(processor);
                            self.pending_preflight_command = retry;
                            return Err(command_error(error));
                        }
                    }
                }
                delivery => delivery,
            };
            diagnostics.extend(
                processor
                    .take_semantic_diagnostics()
                    .into_iter()
                    .map(PendingDiagnostic::Command),
            );
            // TeX82 §§299/367 advance `shown_mode` as soon as expansion
            // prints a command trace. A recoverable expansion diagnostic is a
            // reporting barrier below, but it does not undo that trace-state
            // transition: the following settled command must not print the
            // same mode prefix again in a fresh processor facade.
            if processor.command_trace_printed() {
                self.shown_mode = Some(mode);
            }
            let mut trace_reported = false;
            let mut fused_hot = None;
            let mut fused_retry = None;
            let mut fused_error = None;
            // Diagnostics are a real reporting barrier: preserve their
            // established ordering before command tracing or operand work.
            // The common diagnostic-free path continues in this same borrow.
            if diagnostics.is_empty()
                && let Some(tex_command::CommandReplayDelivery::Command(command)) = delivery.take()
            {
                let continues_main_loop = self.main_loop_active
                    && matches!(
                        command.meaning(),
                        ResolvedMeaning::Static(
                            Meaning::CharToken {
                                cat: Catcode::Letter | Catcode::Other,
                                ..
                            } | Meaning::CharGiven(_)
                                | Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char)
                        )
                    );
                if !continues_main_loop {
                    prepare_command_trace(&mut processor, mode, self.shown_mode);
                    report_main_control_command_trace(
                        &mut processor,
                        mode,
                        &command,
                        &self.boxes,
                        &mut self.shown_mode,
                    );
                    trace_reported = true;
                }
                if direct_hot_candidate(mode, &self.boxes, innermost_group, &command) {
                    if !settled_in_preflight {
                        processor.observe_expanded_delivery(&command);
                    }
                    #[cfg(feature = "profiling")]
                    tex_state::measurement::record_hot_core_phase(
                        tex_state::measurement::HotCorePhase::DeliveryAndScan,
                    );
                    #[cfg(feature = "profiling")]
                    let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
                        tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan,
                    );
                    match scan_direct_hot_command(&mut processor, &command, innermost_group) {
                        Ok(operation) => {
                            let meaning = command.meaning();
                            diagnostics.extend(
                                processor
                                    .take_semantic_diagnostics()
                                    .into_iter()
                                    .map(PendingDiagnostic::Command),
                            );
                            fused_hot = Some((operation, meaning));
                        }
                        Err(error) => {
                            let cursor = processor.delivery_cursor();
                            let retry_expansion = processor.pending_expansion_command().cloned();
                            fused_retry = Some(
                                PendingPreflightCommand::<G>::Settled {
                                    command,
                                    cursor: Some(cursor),
                                }
                                .with_retry_expansion(retry_expansion),
                            );
                            fused_error = Some(error);
                        }
                    }
                } else {
                    delivery = Some(tex_command::CommandReplayDelivery::Command(command));
                }
            }
            (
                delivery,
                settled_in_preflight,
                trace_reported,
                fused_hot,
                fused_retry,
                fused_error,
            )
        };
        if let Some(error) = fused_error {
            if execution_error_needs_command_retry(&error) {
                self.pending_preflight_command = fused_retry;
            }
            return Err(error);
        }
        self.capture_first_causal_context(stores, &diagnostics);
        {
            let mut context = stores.command_context().expect("live generation");
            report_pending_diagnostics(&mut context, diagnostics)?;
        }
        self.drain_file_framing_events(stores);

        if let Some((operation, meaning)) = fused_hot {
            return Ok(Some(PreflightDelivery::<G> {
                delivery: OperationDelivery::<G>::Hot(operation),
                capabilities: crate::transaction_protocol::canonical_command_capabilities(meaning),
            }));
        }

        let passive =
            || crate::transaction_protocol::canonical_static_command_capabilities(Meaning::Relax);
        let Some(delivery) = delivery else {
            return Ok(Some(PreflightDelivery::<G> {
                delivery: OperationDelivery::<G>::Prepared(Box::new(
                    ColdOperation::<G>::EndOfInput,
                )),
                capabilities: passive(),
            }));
        };
        let tex_command::CommandReplayDelivery::Command(command) = delivery else {
            let tex_command::CommandReplayDelivery::Completed(episode) = delivery else {
                unreachable!();
            };
            return Ok(Some(PreflightDelivery::<G> {
                delivery: OperationDelivery::<G>::Prepared(Box::new(
                    ColdOperation::<G>::ReplayCompleted(episode),
                )),
                capabilities: passive(),
            }));
        };

        let continues_main_loop = self.main_loop_active
            && matches!(
                command.meaning(),
                ResolvedMeaning::Static(
                    Meaning::CharToken {
                        cat: Catcode::Letter | Catcode::Other,
                        ..
                    } | Meaning::CharGiven(_)
                        | Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char)
                )
            );
        if !continues_main_loop && !trace_reported {
            let mut processor = command_processor(
                &mut self.command,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
                &mut self.operation_observations,
                stores.command_context().expect("live generation"),
            );
            prepare_command_trace(&mut processor, mode, self.shown_mode);
            report_main_control_command_trace(
                &mut processor,
                mode,
                &command,
                &self.boxes,
                &mut self.shown_mode,
            );
        }

        if self.main_loop_active
            && matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal)
            && matches!(
                command.meaning(),
                ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::NoBoundary
                ))
            )
            && self.operation_observations.is_none()
        {
            return Ok(Some(PreflightDelivery::<G> {
                delivery: OperationDelivery::<G>::Prepared(Box::new(
                    ColdOperation::<G>::NoBoundary {
                        suppress_right: true,
                    },
                )),
                capabilities: crate::transaction_protocol::canonical_command_capabilities(
                    command.meaning(),
                ),
            }));
        }
        let capabilities =
            crate::transaction_protocol::canonical_command_capabilities(command.meaning());
        Ok(Some(PreflightDelivery::<G> {
            delivery: if settled_in_preflight {
                OperationDelivery::<G>::Settled {
                    command,
                    cursor: None,
                }
            } else if raw_main_loop_delivery && continues_main_loop {
                OperationDelivery::<G>::Raw {
                    command,
                    cursor: None,
                }
            } else {
                OperationDelivery::<G>::Replay(Some(command))
            },
            capabilities,
        }))
    }

    /// Returns the effect cursor immediately before final-cleanup framing.
    #[must_use]
    pub const fn job_body_effect_end(&self) -> Option<tex_state::EffectPos> {
        self.job_body_effect_end
    }

    fn apply_operation(
        &mut self,
        stores: &mut Universe<G>,
        delivery: OperationDelivery<G>,
    ) -> Result<ReplayStep, ExecError> {
        let readiness = self
            .prepare_operation(stores, delivery)
            .map_err(|failure| *failure.error)?;
        self.apply_ready_operation(stores, readiness)
    }

    fn apply_ready_operation(
        &mut self,
        stores: &mut Universe<G>,
        readiness: OperationReadiness<G>,
    ) -> Result<ReplayStep, ExecError> {
        match readiness {
            OperationReadiness::<G>::Applied(result) => result,
            OperationReadiness::<G>::Prepared(prepared) => {
                self.apply_prepared_operation(stores, prepared)
            }
        }
    }

    /// Completes one canonical operation after mutation-free capability
    /// preflight. Common unexpandable families scan and apply here without a
    /// universal DTO; cold and barrier families return a prepared value after
    /// immutable resource resolution.
    fn prepare_operation(
        &mut self,
        stores: &mut Universe<G>,
        delivery: OperationDelivery<G>,
    ) -> Result<OperationReadiness<G>, PrepareOperationError<G>> {
        let mode = self.modes.current_mode();
        let mode_fingerprint = self.modes.summary().semantic_fingerprint(stores);
        let last_node_type = self.last_node_type_value(stores);
        if let Ok(mut context) = stores.command_context()
            && context.tracked_region_is_active()
        {
            let mode_key = DependencyKey::Engine(DependencyEngineField::Mode);
            let inner_key = DependencyKey::Engine(DependencyEngineField::InnerMode);
            let last_node_key = DependencyKey::Engine(DependencyEngineField::LastNodeType);
            // Executor-owned mode facts have no state-layer mutation facade.
            // Advance their conservative generation once per observed outer
            // operation so validation always compares the canonical value
            // after another operation has had a chance to mutate the nest.
            context.observe_changed_command_projection(
                mode_key,
                DependencyValue::Projection {
                    schema: 1,
                    fingerprint: mode_fingerprint,
                },
            );
            context.observe_changed_command_projection(
                inner_key,
                DependencyValue::Bool(mode.is_inner()),
            );
            context.observe_changed_command_projection(
                last_node_key,
                DependencyValue::Integer(i64::from(last_node_type)),
            );
            let insertions = context.page_insertions().len();
            context.observe_changed_command_projection(
                DependencyKey::Engine(DependencyEngineField::PageInsertions),
                DependencyValue::Integer(i64::try_from(insertions).unwrap_or(i64::MAX)),
            );
        }
        // Observation is an instrumentation boundary, not an alternate
        // execution mode. Keep the command processor's borrowed mode facts
        // identical to an unobserved step (notably for \ifhmode after a
        // paragraph-start transition).
        if matches!(&delivery, OperationDelivery::<G>::Replay(_)) && self.enter_main_control(stores)
        {
            // §1030's prologue precedes `big_switch`, so its push is published
            // ahead of the first command this step delivers rather than with
            // the step's own applied records.
            let entry_records: Vec<CommandObservation> = self
                .command
                .publish_named_token_list_pushes(
                    &mut stores.command_context().expect("live generation"),
                )
                .into_iter()
                .map(CommandObservation::Input)
                .collect();
            self.observe_committed(entry_records);
        }
        self.drain_file_framing_events(stores);
        self.refresh_host_capabilities(stores);
        let outer_paragraph_was_active = mode == Mode::Horizontal && self.modes.depth() == 2;
        let alignment_preamble = alignment_preamble(self.active_alignment.as_mut());
        let (innermost_group, job_is_all_over) = {
            let context = stores.command_context().expect("live generation");
            (
                context.innermost_group_kind(),
                crate::page_output::job_is_all_over(&context),
            )
        };
        let mut diagnostics = Vec::new();
        let scanned = if let OperationDelivery::<G>::Hot(operation) = delivery {
            ScannedOperation::<G>::Hot(operation)
        } else {
            #[cfg(feature = "profiling")]
            tex_state::measurement::record_hot_core_phase(
                tex_state::measurement::HotCorePhase::DeliveryAndScan,
            );
            #[cfg(feature = "profiling")]
            let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
                tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan,
            );
            let mut processor = command_processor(
                &mut self.command,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
                &mut self.operation_observations,
                stores.command_context().expect("live generation"),
            );
            let display_alignment_tail = matches!(&delivery, OperationDelivery::<G>::Replay(None))
                && mode == Mode::DisplayMath
                && self.modes.current_list().has_display_alignment();
            let scanned = (|| -> Result<ScannedOperation<G>, ExecError> {
                Ok(match delivery {
                    OperationDelivery::<G>::Replay(Some(command)) => {
                        processor.resume_current_command(&command);
                        processor.observe_expanded_delivery(&command);
                        dispatch_main_control_command(
                            &mut processor,
                            command,
                            mode,
                            &self.boxes,
                            innermost_group,
                            job_is_all_over,
                            self.modes.current_list().display_eq_no().is_some(),
                            &mut self.shown_mode,
                            &mut diagnostics,
                            None,
                            self.set_box_forbidden_depth == 0,
                        )?
                    }
                    OperationDelivery::<G>::Settled { command, cursor } => {
                        processor.resume_current_command(&command);
                        if let Some(cursor) = cursor {
                            processor.resume_delivery_cursor(cursor);
                        }
                        dispatch_main_control_command(
                            &mut processor,
                            command,
                            mode,
                            &self.boxes,
                            innermost_group,
                            job_is_all_over,
                            self.modes.current_list().display_eq_no().is_some(),
                            &mut self.shown_mode,
                            &mut diagnostics,
                            None,
                            self.set_box_forbidden_depth == 0,
                        )?
                    }
                    OperationDelivery::<G>::Raw { command, cursor } => {
                        processor.resume_current_command(&command);
                        if let Some(cursor) = cursor {
                            processor.resume_delivery_cursor(cursor);
                        }
                        dispatch_main_control_command(
                            &mut processor,
                            command,
                            mode,
                            &self.boxes,
                            innermost_group,
                            job_is_all_over,
                            self.modes.current_list().display_eq_no().is_some(),
                            &mut self.shown_mode,
                            &mut diagnostics,
                            None,
                            self.set_box_forbidden_depth == 0,
                        )?
                    }
                    OperationDelivery::<G>::Replay(None) if display_alignment_tail => {
                        match processor
                            .next_do_assignments_command()
                            .map_err(command_error)?
                        {
                            Some(command) => match command.meaning() {
                                meaning
                                    if tex_command::exceeds_max_non_prefixed_command(
                                        static_meaning(meaning),
                                    ) || matches!(
                                        meaning,
                                        ResolvedMeaning::Static(Meaning::CharToken {
                                            cat: Catcode::MathShift,
                                            ..
                                        })
                                    ) =>
                                {
                                    dispatch_main_control_command(
                                        &mut processor,
                                        command,
                                        mode,
                                        &self.boxes,
                                        innermost_group,
                                        job_is_all_over,
                                        false,
                                        &mut self.shown_mode,
                                        &mut diagnostics,
                                        None,
                                        false,
                                    )?
                                }
                                _ => {
                                    processor.back_input(command).map_err(command_error)?;
                                    ColdOperation::<G>::DisplayAlignmentRecovery.into()
                                }
                            },
                            None => ColdOperation::<G>::EndOfInput.into(),
                        }
                    }
                    OperationDelivery::<G>::Replay(None) => scan_replay_step(
                        &mut processor,
                        mode,
                        &self.boxes,
                        alignment_preamble,
                        innermost_group,
                        job_is_all_over,
                        self.modes.current_list().display_eq_no().is_some(),
                        self.main_loop_active,
                        &mut self.shown_mode,
                        &mut diagnostics,
                    )?,
                    OperationDelivery::<G>::Expanding {
                        command,
                        main_loop,
                        cursor,
                    } => {
                        processor.resume_delivery_cursor(cursor);
                        prepare_command_trace(&mut processor, mode, self.shown_mode);
                        settle_preflight_step(
                            &mut processor,
                            command,
                            main_loop,
                            mode,
                            &self.boxes,
                            innermost_group,
                            job_is_all_over,
                            self.modes.current_list().display_eq_no().is_some(),
                            &mut self.shown_mode,
                            &mut diagnostics,
                        )?
                    }
                    OperationDelivery::<G>::ImmediatePdfRetry(primitive) => match primitive {
                        UnexpandablePrimitive::PdfObject => ColdOperation::<G>::ImmediateExtension(
                            ImmediateExtension::PdfObject(
                                processor.scan_pdf_object_request().map_err(command_error)?,
                            )
                            .into(),
                        )
                        .into(),
                        UnexpandablePrimitive::PdfXForm => ColdOperation::<G>::ImmediateExtension(
                            ImmediateExtension::PdfForm(
                                processor
                                    .scan_pdf_form_request(UnexpandablePrimitive::PdfXForm)
                                    .map_err(command_error)?,
                            )
                            .into(),
                        )
                        .into(),
                        UnexpandablePrimitive::PdfXImage => ColdOperation::<G>::PdfXImage {
                            request: processor
                                .scan_pdf_image_request()
                                .map_err(command_error)?
                                .into(),
                            resource: PdfImageResource::Unavailable,
                        }
                        .into(),
                        _ => unreachable!("only immediate PDF retries reach this delivery"),
                    },
                    OperationDelivery::<G>::Alignment(alignment) => scan_alignment_delivery_step(
                        &mut processor,
                        alignment,
                        &ReplayBoxes::default(),
                        innermost_group,
                        mode,
                        job_is_all_over,
                        self.main_loop_active,
                        &mut self.shown_mode,
                        &mut diagnostics,
                    )?,
                    OperationDelivery::<G>::AlignmentRetry { alignment, cursor } => {
                        processor.resume_delivery_cursor(cursor);
                        match alignment {
                            Some(alignment) => scan_alignment_delivery_step(
                                &mut processor,
                                alignment,
                                &ReplayBoxes::default(),
                                innermost_group,
                                mode,
                                job_is_all_over,
                                self.main_loop_active,
                                &mut self.shown_mode,
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
                                &mut self.shown_mode,
                                &mut diagnostics,
                            )?,
                        }
                    }
                    OperationDelivery::<G>::Hot(_) => {
                        unreachable!("pre-scanned hot delivery bypasses processor construction")
                    }
                    OperationDelivery::<G>::Prepared(scanned) => (*scanned).into(),
                })
            })();
            let cursor = processor.delivery_cursor();
            let scanned =
                scanned.map_err(|error| PrepareOperationError::<G>::with_cursor(error, cursor))?;
            #[cfg(feature = "profiling")]
            if matches!(scanned, ScannedOperation::<G>::Cold(_)) {
                tex_state::measurement::record_hot_core_materialization(
                    tex_state::measurement::HotCoreMaterialization::ScannedStep,
                );
            }
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
        if self.first_causal_context.is_none() && stores.world().error_channel().error_count() > 0 {
            let context = stores.command_context().expect("live generation");
            self.first_causal_context = Some(crate::FrozenDiagnosticContext::capture(
                &context,
                self.command.diagnostic_input_context(8),
                "command-error",
            ));
        }
        self.capture_first_causal_context(stores, &diagnostics);
        {
            let mut context = stores.command_context().expect("live generation");
            report_pending_diagnostics(&mut context, diagnostics)?;
        }
        self.drain_file_framing_events(stores);
        let scanned = match scanned {
            ScannedOperation::<G>::Cold(scanned) => scanned,
            ScannedOperation::<G>::Hot(operation) => {
                let artifact_count = stores.world().artifact_commits().len();
                let effect_count = stores.world().effect_records().len();
                let prepared_page_count = self.prepared_dvi_pages.len();
                return Ok(OperationReadiness::<G>::Applied(self.apply_hot_operation(
                    stores,
                    operation,
                    outer_paragraph_was_active,
                    artifact_count,
                    effect_count,
                    prepared_page_count,
                )));
            }
        };
        let scanned = self.resolve_font_resource(scanned, stores)?;
        let scanned = self.resolve_input_stream_resource(scanned, stores)?;
        let scanned = self.resolve_pdf_image_resource(scanned, stores)?;
        let completed_preamble = match &scanned {
            ColdOperation::AlignmentPreambleStart { alignment } => Some((
                *alignment,
                self.command
                    .state_mut()
                    .take_completed_alignment_preamble(*alignment)
                    .map_err(|_| ExecError::MissingToken {
                        context: "completed alignment preamble",
                    })?,
            )),
            _ => None,
        };
        let alignment_roots = completed_preamble
            .as_ref()
            .map(|(_, preamble)| {
                let mut roots = Vec::with_capacity(preamble.columns.len() * 2);
                for templates in &preamble.columns {
                    roots.extend(templates.u_template);
                    roots.push(templates.v_template);
                }
                roots
            })
            .unwrap_or_default();
        let (scanned, promoted_alignment_roots) =
            prepare_cold_operation(scanned, &self.command, stores, &alignment_roots).map_err(
                |_| ExecError::MissingToken {
                    context: "cold operation root preparation",
                },
            )?;
        let alignment_preamble = completed_preamble.map(|(alignment, preamble)| {
            let mut promoted = promoted_alignment_roots.into_iter();
            let columns = preamble
                .columns
                .iter()
                .map(|templates| PreparedAlignmentCellTemplates {
                    u_template: templates
                        .u_template
                        .map(|_| promoted.next().expect("promoted u-template")),
                    v_template: promoted.next().expect("promoted v-template"),
                })
                .collect();
            debug_assert!(promoted.next().is_none());
            PreparedAlignmentPreamble {
                alignment,
                columns,
                tabskips: preamble.tabskips,
                default_tabskip: preamble.default_tabskip,
                repeat_start: preamble.repeat_start,
            }
        });
        #[cfg(feature = "profiling")]
        tex_state::measurement::record_hot_core_materialization(
            tex_state::measurement::HotCoreMaterialization::PreparedOperation,
        );
        Ok(OperationReadiness::<G>::Prepared(PreparedColdOperation::<
            G,
        > {
            scanned,
            alignment_preamble,
            outer_paragraph_was_active,
            artifact_count: stores.world().artifact_commits().len(),
            effect_count: stores.world().effect_records().len(),
            prepared_page_count: self.prepared_dvi_pages.len(),
        }))
    }

    /// Applies a measured common operation without constructing the universal
    /// scan/preparation DTOs. `CommandProcessor` has released its borrow, but
    /// the enclosing direct-operation transaction and persistent interpreter
    /// remain the same ones that performed delivery and scanning.
    #[allow(clippy::too_many_arguments)]
    fn apply_hot_operation(
        &mut self,
        stores: &mut Universe<G>,
        operation: hot_apply::HotOperation<G>,
        outer_paragraph_was_active: bool,
        artifact_count: usize,
        effect_count: usize,
        prepared_page_count: usize,
    ) -> Result<ReplayStep, ExecError> {
        self.main_loop_active = false;
        #[cfg(feature = "profiling")]
        tex_state::measurement::record_hot_core_phase(
            tex_state::measurement::HotCorePhase::SemanticApply,
        );
        #[cfg(feature = "profiling")]
        let _semantic_allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::SemanticApply,
        );
        let observing = self.operation_observations.is_some();
        let mut assignment_receipts = observing.then(Vec::new);
        let fires_afterassignment = operation.fires_afterassignment();
        let operation = hot_apply::prepare(operation, &self.command, stores).map_err(|_| {
            ExecError::MissingToken {
                context: "hot operation root preparation",
            }
        })?;
        let context = stores
            .command_context()
            .map_err(|_| ExecError::MissingToken {
                context: "hot operation admission",
            })?;
        let result = hot_apply::apply(
            &operation,
            context,
            &mut self.modes,
            &mut CommandMachine {
                state: &mut self.command,
                fuel: self.fuel.fuel_mut(),
                capabilities: &mut self.capabilities,
                observations: &mut self.operation_observations,
                assignment_receipts: assignment_receipts.as_mut(),
                shown_mode: &mut self.shown_mode,
                initex: self.initex,
                emit_dvi_override: self.emit_dvi_override,
                immediate_prints: &mut self.immediate_prints,
                prepared_shipout: &mut self.prepared_shipout,
            },
        );
        if result.is_ok() {
            self.fire_pending_page_output(stores)?;
        }
        #[cfg(feature = "profiling")]
        tex_state::measurement::record_hot_core_phase(
            tex_state::measurement::HotCorePhase::EvidencePublication,
        );
        #[cfg(feature = "profiling")]
        let _evidence_allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::EvidencePublication,
        );
        if result.is_ok() {
            let mut records = self
                .command
                .publish_named_token_list_pushes(
                    &mut stores.command_context().expect("live generation"),
                )
                .into_iter()
                .map(CommandObservation::Input)
                .collect::<Vec<_>>();
            records.extend(
                assignment_receipts
                    .into_iter()
                    .flatten()
                    .map(CommandObservation::Mutation),
            );
            records.extend(
                committed_stream_effect_observations(
                    effect_count,
                    prepared_page_count,
                    stores,
                    &self.prepared_dvi_pages,
                )
                .into_iter()
                .map(CommandObservation::Effect),
            );
            for shipout in committed_shipout_observations(artifact_count, stores) {
                records.push(CommandObservation::Effect(shipout));
            }
            self.page_output_observations.append_to(&mut records);
            self.observe_committed(records);
        }
        if result.is_ok() && fires_afterassignment {
            let context = stores
                .command_context()
                .map_err(|_| ExecError::MissingToken {
                    context: "afterassignment admission",
                })?;
            schedule_afterassignment(
                &mut self.command,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
                &mut self.operation_observations,
                context,
            )?;
        }
        self.page_output_observations.clear();
        if result.is_ok() {
            self.finish_shipout_publication(artifact_count, effect_count, stores);
            self.finish_paragraph_boundary(outer_paragraph_was_active, stores);
        }
        result
    }

    fn apply_prepared_operation(
        &mut self,
        stores: &mut Universe<G>,
        prepared: PreparedColdOperation<G>,
    ) -> Result<ReplayStep, ExecError> {
        let PreparedColdOperation::<G> {
            scanned,
            alignment_preamble,
            outer_paragraph_was_active,
            artifact_count,
            effect_count,
            prepared_page_count,
        } = prepared;
        let parking = self.suspend_main_control_parking(&scanned);
        #[cfg(feature = "profiling")]
        tex_state::measurement::record_hot_core_phase(
            tex_state::measurement::HotCorePhase::SemanticApply,
        );
        #[cfg(feature = "profiling")]
        let _semantic_allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::SemanticApply,
        );
        let scanned = match self.apply_host_owned_step(scanned, stores) {
            ControlFlow::Break(applied) => {
                return self.finish_host_owned_step(
                    applied,
                    outer_paragraph_was_active,
                    artifact_count,
                    effect_count,
                    prepared_page_count,
                    stores,
                );
            }
            ControlFlow::Continue(scanned) => scanned,
        };
        if let Some(preamble) = alignment_preamble {
            if preamble.columns.is_empty() {
                return Err(ExecError::MissingToken {
                    context: "first alignment preamble column",
                });
            }
            let active = self
                .active_alignment
                .as_mut()
                .filter(|active| active.identity == preamble.alignment)
                .ok_or(ExecError::MissingToken {
                    context: "active alignment preamble",
                })?;
            active.columns = preamble.columns;
            active.tabskips = preamble.tabskips;
            active.default_tabskip = preamble.default_tabskip;
            active.repeat_start = preamble.repeat_start;
            active.column = 0;
            active.preamble_start_pending = false;
            active.align_peek_pending = true;
        }
        let mut scanned = match scanned {
            ColdOperation::ShowGroups { diagnostic: None } => {
                let context = stores.command_context().expect("live generation");
                ColdOperation::ShowGroups {
                    diagnostic: Some(detached_showgroups(
                        &context,
                        &self.active_alignment,
                        &self.boxes,
                        &self.active_discretionaries,
                        &self.active_math_choices,
                        &self.active_math_left_boundaries,
                        &self.active_math_shifts,
                    )),
                }
            }
            scanned => scanned,
        };
        let reassigning_glue = self.local_glue_pointer_reassigned(stores, &scanned);
        let redundant_glue = self.etex_redundant_local_glue_assignment(stores, &scanned);
        match &mut scanned {
            ColdOperation::Skip {
                redundant,
                reassigning,
                ..
            }
            | ColdOperation::Muskip {
                redundant,
                reassigning,
                ..
            } => {
                *redundant = redundant_glue;
                *reassigning = reassigning_glue;
            }
            _ => {}
        }
        let observing = self.operation_observations.is_some();
        let mut assignment_receipts = observing.then(Vec::new);
        let begins_alignment = matches!(&scanned, ColdOperation::BeginAlignment { .. });
        let suspends_alignment = begins_alignment && self.active_alignment.is_some();
        let begins_alignment_cell =
            matches!(&scanned, ColdOperation::AlignmentPreambleStart { .. });
        let installs_u_template = match &scanned {
            ColdOperation::AlignmentCellOpening {
                alignment,
                opening: AlignmentCellOpening::Template,
            } => Some(*alignment),
            // `align_peek` already fetched and backed up the first nonblank
            // command before it calls TeX82's `init_col`.
            ColdOperation::AlignmentPeekCell {
                alignment,
                omit: false,
            } => Some(*alignment),
            _ => None,
        };
        let installs_omit_cell = match &scanned {
            ColdOperation::AlignmentCellOpening {
                alignment,
                opening: AlignmentCellOpening::Omit,
            } => Some(*alignment),
            ColdOperation::AlignmentPeekCell {
                alignment,
                omit: true,
            } => Some(*alignment),
            _ => None,
        };
        let finishes_alignment_cell = match &scanned {
            ColdOperation::AlignmentCellFinish { alignment } => {
                self.command.alignment_cell_finish_observation(*alignment)
            }
            _ => None,
        };
        let completes_alignment_cell =
            matches!(&scanned, ColdOperation::AlignmentCellFinish { .. });
        let finishes_alignment = match &scanned {
            ColdOperation::AlignmentFinish { alignment } => {
                self.command.alignment_finish_observation(*alignment)
            }
            _ => None,
        };
        let fires_afterassignment = matches!(
            scanned,
            ColdOperation::Count { .. }
                | ColdOperation::Dimen { .. }
                | ColdOperation::BoxDimensionAssignment { .. }
                | ColdOperation::Skip { .. }
                | ColdOperation::Muskip { .. }
                | ColdOperation::Toks { .. }
                | ColdOperation::IntParam { .. }
                | ColdOperation::DimenParam { .. }
                | ColdOperation::TokParam { .. }
                | ColdOperation::GlueParam { .. }
                | ColdOperation::CodeTable { .. }
                | ColdOperation::PdfFontCode { .. }
                | ColdOperation::PdfNoLigatures { .. }
                | ColdOperation::FontDimen { .. }
                | ColdOperation::FontInteger { .. }
                | ColdOperation::FontDefinition { .. }
                | ColdOperation::GeneratedFontDefinition { .. }
                | ColdOperation::InputStream { .. }
                | ColdOperation::Arithmetic { .. }
                | ColdOperation::InvalidArithmeticTarget { .. }
                | ColdOperation::CharacterDefinition { .. }
                | ColdOperation::RegisterDefinition { .. }
                | ColdOperation::ParagraphShape { .. }
                | ColdOperation::PenaltyArray { .. }
                | ColdOperation::FontSelect { .. }
                | ColdOperation::MathFamily { .. }
                | ColdOperation::SetBox { .. }
                | ColdOperation::PrevDepth { .. }
                | ColdOperation::SpaceFactor { .. }
                | ColdOperation::PrevGraf { .. }
                | ColdOperation::PageDimension { .. }
                | ColdOperation::PageInteger { .. }
                | ColdOperation::HyphenationData { .. }
                | ColdOperation::SetInteractionMode(..)
        );
        let glue_assignment = match &scanned {
            ColdOperation::Skip {
                index,
                source_identity,
                ..
            } => Some((false, *index, *source_identity)),
            ColdOperation::Muskip {
                index,
                source_identity,
                ..
            } => Some((true, *index, *source_identity)),
            _ => None,
        };
        let end = match &scanned {
            ColdOperation::End {
                dump,
                incomplete_conditions,
            } => Some((*dump, incomplete_conditions.clone())),
            _ => None,
        };
        let effect = {
            let context = stores.command_context().expect("live generation");
            applied_effect_observation(&scanned, &context)
        };
        let mut command = CommandMachine {
            state: &mut self.command,
            fuel: self.fuel.fuel_mut(),
            capabilities: &mut self.capabilities,
            observations: &mut self.operation_observations,
            assignment_receipts: assignment_receipts.as_mut(),
            shown_mode: &mut self.shown_mode,
            initex: self.initex,
            emit_dvi_override: self.emit_dvi_override,
            immediate_prints: &mut self.immediate_prints,
            prepared_shipout: &mut self.prepared_shipout,
        };
        let context = stores
            .command_context()
            .map_err(|_| ExecError::MissingToken {
                context: "cold operation admission",
            })?;
        let mut result = apply_cold_operation(
            scanned,
            context,
            &mut self.modes,
            &mut self.next_alignment_identity,
            &mut self.active_alignment,
            &mut command,
            &mut self.boxes,
            &self.active_discretionaries,
            &self.active_math_choices,
            &self.active_math_left_boundaries,
            &self.active_math_shifts,
            &mut self.prepared_dvi_pages,
            &mut self.end_job_ejection_pending,
        );
        if result.is_ok() {
            for print in command.immediate_prints.drain(..) {
                stores.world_mut().write_text(print.sink, &print.text);
            }
            if let Some(shipout) = command.prepared_shipout.take()
                && let Some(receipt) = shipout_replay_box(shipout.node, stores, &mut command)?
                    .and_then(|publication| publication.dvi)
            {
                push_prepared_dvi_page(&mut self.prepared_dvi_pages, receipt);
            }
        } else {
            command.immediate_prints.clear();
            *command.prepared_shipout = None;
        }
        if result.is_ok()
            && !redundant_glue
            && let Some((index, physical, source_identity, pointer_sources)) = {
                let context = stores.command_context().expect("live generation");
                match glue_assignment {
                    Some((false, index, source_identity)) => {
                        context.glue_register(index).ok().flatten().map(|physical| {
                            (
                                index,
                                physical,
                                source_identity,
                                &mut self.skip_pointer_sources,
                            )
                        })
                    }
                    Some((true, index, source_identity)) => context.muskip(index).map(|physical| {
                        (
                            index,
                            physical,
                            source_identity,
                            &mut self.muskip_pointer_sources,
                        )
                    }),
                    _ => None,
                }
            }
        {
            if pointer_sources.len() <= usize::from(index) {
                pointer_sources.resize(usize::from(index) + 1, None);
            }
            pointer_sources[usize::from(index)] = Some((physical, source_identity));
        }
        if result.is_ok() && self.initex && end.as_ref().is_some_and(|(dump, _)| *dump) {
            let context = stores.command_context().expect("live generation");
            let receipt = crate::job::FormatDumpReceipt::new(
                self.capabilities.job_name().to_owned(),
                context.int_param(IntParam::YEAR),
                context.int_param(IntParam::MONTH),
                context.int_param(IntParam::DAY),
            );
            drop(context);
            // TeX82 §1328 builds and retains `format_ident` before §1309
            // serializes the string pool. The host publication filename is
            // made only for display and immediately flushed.
            stores
                .command_context()
                .expect("format accounting requires a live generation")
                .record_retained_strings(tex_state::RetainedStringAllocation::one(
                    &receipt.pool_string(),
                ));
            self.dumped_format = Some(receipt);
        }
        if let (Ok(ReplayStep::End), Some((dump, incomplete_conditions))) = (&result, end.as_ref())
        {
            self.end_of_job_final_cleanup(stores, *dump, incomplete_conditions.clone());
        } else if matches!(result, Ok(ReplayStep::EndOfInput))
            && self.root_completion == RootCompletionPolicy::RequireTeXEnd
        {
            result = Ok(self.handle_root_end_of_input(stores));
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
        #[cfg(feature = "profiling")]
        tex_state::measurement::record_hot_core_phase(
            tex_state::measurement::HotCorePhase::EvidencePublication,
        );
        #[cfg(feature = "profiling")]
        let _evidence_allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::EvidencePublication,
        );
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
                    .publish_named_token_list_pushes(
                        &mut stores.command_context().expect("live generation"),
                    )
                    .into_iter()
                    .map(CommandObservation::Input),
            );
            let effects = committed_stream_effect_observations(
                effect_count,
                prepared_page_count,
                stores,
                &self.prepared_dvi_pages,
            );
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
            records.extend(
                assignment_receipts
                    .into_iter()
                    .flatten()
                    .map(CommandObservation::Mutation),
            );
            // §1378's live-file closes are part of termination and precede
            // the replay driver's synthetic terminal marker. Other command
            // effects retain their established command-before-host-delta
            // ordering.
            if end.is_some() {
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
            self.page_output_observations.append_to(&mut records);
            self.observe_committed(records);
        }
        // TeX82 §1211 commits the assignment inside its case arm, then
        // reaches §1269's `done:` and `back_input`. Publish the mutation
        // before the replay-level push for that saved token.
        if result.is_ok() && fires_afterassignment {
            let context = stores
                .command_context()
                .map_err(|_| ExecError::MissingToken {
                    context: "afterassignment admission",
                })?;
            schedule_afterassignment(
                &mut self.command,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
                &mut self.operation_observations,
                context,
            )?;
        }
        self.page_output_observations.clear();
        if result.is_ok() {
            self.finish_shipout_publication(artifact_count, effect_count, stores);
            self.finish_paragraph_boundary(outer_paragraph_was_active, stores);
        }
        result
    }

    /// Scans TeX's initial terminal filename through the canonical command
    /// path, retaining every committed observation for the caller.
    pub fn scan_startup_file_name(
        &mut self,
        stores: &mut Universe<G>,
        observer: &mut dyn CommandObserver,
    ) -> Result<String, ExecError> {
        self.operation_observations = Some(ObservationBuffer::default());
        let scanned = self.scan_startup_file_name_once(stores);
        self.operation_observations
            .take()
            .unwrap_or_default()
            .consume_into(Some(observer));
        scanned
    }

    fn scan_startup_file_name_once(
        &mut self,
        stores: &mut Universe<G>,
    ) -> Result<String, ExecError> {
        let filename =
            {
                let mut processor = command_processor(
                    &mut self.command,
                    self.fuel.fuel_mut(),
                    &mut self.capabilities,
                    &mut self.operation_observations,
                    stores.command_context().expect("live generation"),
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
            self.fuel.fuel_mut(),
            &mut self.capabilities,
            &mut self.operation_observations,
            stores.command_context().expect("live generation"),
        );
        let exhausted = processor.get_x_token();
        let exhausted = exhausted.map_err(command_error);
        drop(processor);
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

/// TeX82 §1121's improper-discretionary report. The completed part is
/// frozen before validation so its detached node list remains available to
/// `show_box` even though recovery rejects the enclosing discretionary.
fn report_improper_discretionary<G>(
    stores: &mut CommandContext<'_, G>,
    deleted: tex_state::node_arena::PageListId,
    context: String,
) -> Result<(), ExecError> {
    let text = crate::node_dump::dump_page_list(
        stores,
        deleted,
        crate::node_dump::DumpConfig::read(stores),
    );

    let mut report = stores.print_err("Improper discretionary list");
    report
        .help(&["Discretionary lists must contain only boxes and kerns."])
        .context(context);
    report.error().jump_out()?;

    let mut diagnostic = stores.begin_diagnostic();
    diagnostic
        .print("The following discretionary sublist has been deleted:")
        .print_ln()
        .print_rendered(&text);
    diagnostic.end(true);
    Ok(())
}

/// The structural outcome of one main-control operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MainControlStep {
    Continue,
    EndOfInput,
    End,
}

/// Host contract for exhaustion of the registered root source.
///
/// TeX jobs require §1045's `\end` or `\dump`; physical EOF instead enters
/// §360's terminal-input path. Editor and property-test fragments are a
/// separate host abstraction whose authored root boundary intentionally
/// returns without running TeX's final cleanup.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RootCompletionPolicy {
    #[default]
    RequireTeXEnd,
    StopAtRootEof,
}

// The fixture suite retains its historical vocabulary locally.  This alias is
// Kept private while the implementation is migrated in place; callers only
// see `MainControlStep`.
type ReplayStep = MainControlStep;

fn math_char<G>(
    stores: &CommandContext<'_, G>,
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

fn append_math_char<G>(
    mut list: crate::mode::ModeListMutation<'_>,
    stores: &CommandContext<'_, G>,
    code: u32,
    origin: tex_state::token::OriginId,
) -> Result<(), ExecError> {
    let (class, character) = math_char(stores, code, origin)?;
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
fn set_math_char<G>(
    ch: char,
    origin: tex_state::token::OriginId,
    mut stores: CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, G>,
) -> Result<(), ExecError> {
    let code = stores.mathcode(ch);
    stores.observe_command_projection(
        tex_state::DependencyKey::Code {
            table: tex_state::DependencyCodeTable::Mathcode,
            scalar: ch.into(),
        },
        DependencyValue::Integer(i64::from(code)),
    );
    if code >= 0x8000 {
        let mut processor = command.processor(stores);
        let treated = processor.treat_as_active_character(ch, origin);
        treated.map_err(command_error)?;
        return Ok(());
    }
    append_math_char(modes.current_list_mutation(), &stores, code, origin)
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
fn scripts_allowed(node: &Node) -> bool {
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

pub(crate) fn script_field_mut(noad: &mut MathNoad, kind: MathScriptKind) -> &mut MathField {
    match kind {
        MathScriptKind::Superscript => &mut noad.superscript,
        MathScriptKind::Subscript => &mut noad.subscript,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ScriptTarget {
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
pub(crate) fn reserve_script_target<G>(
    mut list: crate::mode::ModeListMutation<'_>,
    stores: &mut Universe<G>,
    kind: MathScriptKind,
) -> Result<ScriptTarget, ExecError> {
    // `t<>empty`: the tail was eligible but already carries this script.
    let tail_index = list.nodes().len().checked_sub(1);
    let (eligible, occupied) = match tail_index.and_then(|index| list.nodes().get(index)) {
        Some(node) if scripts_allowed(node) => {
            let Node::MathNoad(noad) = node else {
                unreachable!("scripts_allowed admits only noads")
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

    Ok(ScriptTarget { node_index, kind })
}

pub(crate) fn fill_script_target(
    mut list: crate::mode::ModeListMutation<'_>,
    target: ScriptTarget,
    field: MathField,
) {
    list.with_node_mut(target.node_index, |node| {
        let Node::MathNoad(noad) = node else {
            unreachable!("reserved canonical script target must remain a noad")
        };
        let reserved = script_field_mut(noad, target.kind);
        debug_assert!(matches!(reserved, MathField::Empty));
        *reserved = field;
    })
    .expect("reserved canonical script target must remain present");
}

fn apply_limits(mut list: crate::mode::ModeListMutation<'_>, kind: MathLimitKind) -> bool {
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

fn start_fraction<G>(
    mut list: crate::mode::ModeListMutation<'_>,
    stores: &mut Universe<G>,
    fraction: tex_command::ScannedMathFraction,
) -> bool {
    if list.incomplete_fraction().is_some() {
        return false;
    }
    let numerator = stores.publish_page_nodes(&list.take_nodes());
    list.set_incomplete_fraction(crate::mode::IncompleteFraction {
        numerator,
        thickness: match fraction.thickness {
            Some(value) => FractionThickness::Explicit(value),
            None => FractionThickness::Default,
        },
        left_delimiter: fraction.left_delimiter.map(|value| value.code),
        right_delimiter: fraction.right_delimiter.map(|value| value.code),
    });
    true
}

fn finish_math_list<G>(
    nodes: &[Node],
    incomplete: Option<&crate::mode::IncompleteFraction>,
    stores: &mut CommandContext<'_, G>,
) -> Result<tex_state::node_arena::PageListId, ExecError> {
    let mut output = nodes.to_vec();
    if let Some(fraction) = incomplete {
        let denominator = stores.publish_page_nodes(output.clone());
        // TeX82 §1185 and e-TeX [48.1185]: `delim_ptr` identifies the most
        // recent `\left` or `\middle` in a math-left group.  Completion moves
        // only the nodes after that boundary into the numerator, then links
        // the fraction noad immediately after the boundary.
        let mut numerator_nodes = stores
            .page_node_list(fraction.numerator)
            .expect("fraction numerator belongs to the live page arena")
            .nodes()
            .to_vec();
        let boundary = numerator_nodes.iter().rposition(|node| {
            matches!(
                node,
                Node::MathNoad(MathNoad {
                    kind: NoadKind::LeftDelimiter { .. } | NoadKind::MiddleDelimiter { .. },
                    ..
                })
            )
        });
        let (prefix, numerator_nodes) = if let Some(boundary) = boundary {
            let numerator = numerator_nodes.split_off(boundary + 1);
            (numerator_nodes, numerator)
        } else {
            (Vec::new(), numerator_nodes)
        };
        let numerator = if prefix.is_empty() {
            fraction.numerator.clone()
        } else {
            stores.publish_page_nodes(numerator_nodes)
        };
        let fraction = Node::FractionNoad(MathFraction {
            numerator,
            denominator,
            thickness: fraction.thickness,
            left_delimiter: fraction.left_delimiter,
            right_delimiter: fraction.right_delimiter,
        });
        output = prefix.into_iter().chain([fraction]).collect();
    }
    let output = stores.publish_page_nodes(output);
    Ok(output)
}

/// TeX82 §1186's `math_group` singleton-Ord simplification.
///
/// After §1153 has tentatively classified a braced field as `sub_mlist`,
/// `handle_right_brace` removes braces around exactly one undecorated Ord
/// noad by copying its nucleus field into the destination. This preserves an
/// author box as `sub_box` instead of wrapping it in a second natural hpack.
fn collapse_singleton_math_group<G>(
    stores: &CommandContext<'_, G>,
    list: tex_state::node_arena::PageListId,
) -> MathField {
    let nodes = stores
        .page_node_list(list)
        .expect("math group belongs to the live page arena")
        .nodes();
    if let [Node::MathNoad(noad)] = nodes
        && noad.kind == NoadKind::Normal(NoadClass::Ord)
        && matches!(noad.subscript, MathField::Empty)
        && matches!(noad.superscript, MathField::Empty)
    {
        return noad.nucleus.clone();
    }
    MathField::SubMlist(list)
}

fn take_finished_math_list<G>(
    modes: &mut ModeNest,
    stores: &mut Universe<G>,
) -> Result<tex_state::node_arena::PageListId, ExecError> {
    let (nodes, incomplete) = {
        let mut list = modes.current_list_mutation();
        (list.take_nodes(), list.take_incomplete_fraction())
    };
    finish_math_list(
        &nodes,
        incomplete.as_ref(),
        &mut stores.command_context().expect("math-list admission"),
    )
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
fn report_escaped_error<G>(
    stores: &mut CommandContext<'_, G>,
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
fn report_missing_box<G>(
    command: &CommandState<G>,
    stores: &mut CommandContext<'_, G>,
) -> Result<(), ExecError> {
    let context = command.output_open_context(stores);
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

/// TeX82 §1084's `box_context < box_flag` recovery for `\setbox`.
fn report_improper_setbox<G>(
    context: String,
    stores: &mut CommandContext<'_, G>,
) -> Result<(), ExecError> {
    report_escaped_error(
        stores,
        "Improper ",
        "setbox",
        "",
        &[
            "Sorry, \\setbox is not allowed after \\halign in a display,",
            "or between \\accent and an accented character.",
        ],
        context,
    )
}

/// TeX82 §1082's `scan_keyword("to")` recovery in `\vsplit`.
fn report_missing_vsplit_to<G>(
    context: &str,
    stores: &mut CommandContext<'_, G>,
) -> Result<(), ExecError> {
    crate::error_report::report_error(
        stores,
        "Missing `to' inserted",
        &[
            "I'm working on `\\vsplit<box number> to <dimen>';",
            "will look for the <dimen> next.",
        ],
        context.to_owned(),
    )?;
    Ok(())
}

/// TeX82 §1197's `<Check that another `$` follows>`.
///
/// §1197 reaches this through `back_error`, and the scanner's probe
/// (`scan_display_end_math_shift`) has already put the offending token back,
/// so only the report itself is left to issue.
fn report_unpaired_display_end<G>(
    command: &CommandState<G>,
    stores: &mut Universe<G>,
) -> Result<(), ExecError> {
    let mut stores = stores
        .command_context()
        .expect("display diagnostic admission");
    let context = command.output_open_context(&stores);
    crate::error_report::report_error(
        &mut stores,
        "Display math should end with $$",
        &[
            "The `$' that I just saw supposedly matches a previous `$$'.",
            "So I shall assume that you typed `$$' both times.",
        ],
        context,
    )?;
    Ok(())
}

fn left_group_open<G>(modes: &ModeNest, stores: &mut Universe<G>) -> bool {
    // e-TeX etex.ch [48.1192] admits `\middle` through the same
    // `math_left_group` case as `\right`.  Seeing a leading left noad is not
    // sufficient: a simple group nested inside that left/right group is an
    // invalid context and must take the `Extra \middle` recovery arm.
    if stores
        .command_context()
        .expect("live generation")
        .innermost_group_kind()
        != Some(GroupKind::MathLeft)
    {
        return false;
    }
    let starts_left_node = |node: Option<&Node>| {
        matches!(
            node,
            Some(Node::MathNoad(MathNoad {
                kind: NoadKind::LeftDelimiter { .. },
                ..
            }))
        )
    };
    starts_left_node(modes.current_list().nodes().first())
        || modes
            .current_list()
            .incomplete_fraction()
            .is_some_and(|fraction| {
                stores
                    .page_node_list(fraction.numerator)
                    .ok()
                    .and_then(|list| list.nodes().first())
                    .is_some_and(|node| starts_left_node(Some(node)))
            })
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
    fn print<G>(self, report: &mut tex_state::print::ErrorReport<'_, G>) {
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
    fn print<G>(self, report: &mut tex_state::print::ErrorReport<'_, G>) {
        match self {
            Self::EndGroup => report.print_esc("endgroup"),
            Self::MathShift => report.print_char('$'),
            Self::Right => report.print_esc("right"),
        };
    }
}

/// Selects the one command-owned scanner that may consume input before
/// ordinary main control. Alignment preamble setup validates and backs up its
/// opening brace twice through successive command-owned backup levels; only
/// the second replay reaches TeX82's live preamble scanner.
#[allow(clippy::too_many_arguments)] // owns the replay-only command/input seam
fn scan_replay_step<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    mode: Mode,
    boxes: &ReplayBoxes<G>,
    alignment_preamble: Option<(AlignmentIdentity, AlignmentPreamblePhase)>,
    innermost_group: Option<GroupKind>,
    job_is_all_over: bool,
    display_eq_no: bool,
    main_loop_active: bool,
    shown_mode: &mut Option<Mode>,
    diagnostics: &mut Vec<PendingDiagnostic<G>>,
) -> Result<ScannedOperation<G>, ExecError> {
    if let Some((alignment, phase)) = alignment_preamble {
        return match phase {
            AlignmentPreamblePhase::Opening => {
                // TeX82 §§299/367: `scan_spec` expands its optional
                // dimension while `init_align` is already in the alignment's
                // newly pushed mode.  An expandable command such as `\the`
                // therefore crosses `show_cur_cmd_chr` here, before ordinary
                // main control gets another command.  Carry the same pending
                // mode prefix and `shown_mode` update that `scan_step` owns
                // around its `get_x_token` boundary.
                prepare_command_trace(processor, mode, *shown_mode);
                let packing = processor
                    .scan_alignment_preamble_opening()
                    .map_err(command_error)?;
                if processor.command_trace_printed() {
                    *shown_mode = Some(mode);
                }
                Ok(ColdOperation::<G>::AlignmentPreambleOpening { alignment, packing }.into())
            }
            AlignmentPreamblePhase::Start { owner } => {
                // TeX82 §§299, 367, 759, and 774: `init_align` has already
                // pushed the alignment mode when §759 expands the token after
                // `\span`. This scanner episode is the first processor that
                // can print a command after that push, so it owns the pending
                // mode prefix just like the packing-spec episode above.
                prepare_command_trace(processor, mode, *shown_mode);
                processor
                    .begin_alignment_preamble_scan(owner)
                    .map_err(command_error)?;
                if processor.command_trace_printed() {
                    *shown_mode = Some(mode);
                }
                Ok(ColdOperation::<G>::AlignmentPreambleStart { alignment }.into())
            }
            AlignmentPreamblePhase::CellOpening => {
                let opening = processor
                    .scan_alignment_cell_opening()
                    .map_err(command_error)?;
                Ok(ColdOperation::<G>::AlignmentCellOpening { alignment, opening }.into())
            }
            AlignmentPreamblePhase::NextCellOpening => {
                let opening = processor
                    .scan_alignment_next_cell_opening()
                    .map_err(command_error)?;
                Ok(ColdOperation::<G>::AlignmentCellOpening { alignment, opening }.into())
            }
            AlignmentPreamblePhase::AlignPeek { after_noalign } => {
                scan_alignment_peek(processor, alignment, after_noalign).map(Into::into)
            }
            AlignmentPreamblePhase::NoAlignBody => scan_noalign_body(
                processor,
                alignment,
                boxes,
                innermost_group,
                mode,
                job_is_all_over,
                shown_mode,
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
                shown_mode,
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
        shown_mode,
        diagnostics,
    )
}

#[derive(Clone, Copy)]
enum AlignmentPreamblePhase {
    Opening,
    Start {
        owner: Option<tex_state::interner::Symbol>,
    },
    CellOpening,
    NextCellOpening,
    AlignPeek {
        after_noalign: bool,
    },
    NoAlignBody,
    CellDelivery,
}

fn alignment_preamble<G>(
    active: Option<&mut ActiveReplayAlignment<G>>,
) -> Option<(AlignmentIdentity, AlignmentPreamblePhase)> {
    let active = active?;
    if active.preamble_opening_pending {
        Some((active.identity, AlignmentPreamblePhase::Opening))
    } else if active.preamble_start_pending {
        Some((
            active.identity,
            AlignmentPreamblePhase::Start {
                owner: active.owner,
            },
        ))
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
fn scan_alignment_peek<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    alignment: AlignmentIdentity,
    _after_noalign: bool,
) -> Result<ColdOperation<G>, ExecError> {
    processor
        .begin_alignment_peek(_after_noalign)
        .map_err(command_error)?;
    let lookahead = processor
        .next_alignment_lookahead()
        .map_err(command_error)?
        .ok_or(ExecError::MissingToken {
            context: "alignment lookahead",
        })?;
    match lookahead.command().meaning() {
        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::NoAlign)) => {
            let _ = processor.commit_alignment_lookahead_delivery(lookahead);
            processor
                .scan_alignment_noalign_opening()
                .map_err(command_error)?;
            Ok(ColdOperation::<G>::BeginNoAlign { alignment })
        }
        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::CrCr)) => {
            let _ = processor.commit_alignment_lookahead_delivery(lookahead);
            Ok(ColdOperation::<G>::AlignPeekRestart { alignment })
        }
        ResolvedMeaning::Static(Meaning::CharToken {
            cat: Catcode::EndGroup,
            ..
        }) => {
            let _ = processor.commit_alignment_lookahead_delivery(lookahead);
            Ok(ColdOperation::<G>::AlignmentFinish { alignment })
        }
        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Omit)) => {
            let _ = processor.commit_alignment_lookahead_delivery(lookahead);
            Ok(ColdOperation::<G>::AlignmentPeekCell {
                alignment,
                omit: true,
            })
        }
        _ => {
            processor
                .back_alignment_lookahead(lookahead)
                .map_err(command_error)?;
            Ok(ColdOperation::<G>::AlignmentPeekCell {
                alignment,
                omit: false,
            })
        }
    }
}

#[allow(clippy::too_many_arguments)] // carries command-owned noalign replay facts
fn scan_noalign_body<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    alignment: AlignmentIdentity,
    boxes: &ReplayBoxes<G>,
    innermost_group: Option<GroupKind>,
    mode: Mode,
    job_is_all_over: bool,
    shown_mode: &mut Option<Mode>,
    diagnostics: &mut Vec<PendingDiagnostic<G>>,
) -> Result<ScannedOperation<G>, ExecError> {
    prepare_command_trace(processor, mode, *shown_mode);
    let Some(command) = processor.get_x_token().map_err(command_error)? else {
        return Ok(ColdOperation::<G>::EndOfInput.into());
    };
    report_main_control_command_trace(processor, mode, &command, boxes, shown_mode);
    match command.meaning() {
        ResolvedMeaning::Static(Meaning::CharToken {
            cat: Catcode::EndGroup,
            ..
        }) if innermost_group == Some(GroupKind::NoAlign) => {
            if partoken_context_replays(processor, mode, 2) {
                processor
                    .insert_partoken_before(command)
                    .map_err(command_error)?;
                return Ok(ColdOperation::<G>::Continue.into());
            }
            Ok(ColdOperation::<G>::NoAlignEndGroup { alignment }.into())
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
            shown_mode,
            diagnostics,
            None,
            true,
        ),
    }
}

/// Delivers one active cell command through the command-owned alignment
/// boundary.  This remains separate from preamble and opener scans because a
/// completed scanner (such as a rule specification) can leave a backed-up
/// delimiter ready for the next main-control step.
#[allow(clippy::too_many_arguments)] // carries command-owned replay facts
fn scan_alignment_delivery_step<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    alignment: AlignmentIdentity,
    boxes: &ReplayBoxes<G>,
    innermost_group: Option<GroupKind>,
    mode: Mode,
    job_is_all_over: bool,
    main_loop_active: bool,
    shown_mode: &mut Option<Mode>,
    diagnostics: &mut Vec<PendingDiagnostic<G>>,
) -> Result<ScannedOperation<G>, ExecError> {
    prepare_command_trace(processor, mode, *shown_mode);
    match processor
        .get_x_alignment_delivery(main_loop_active)
        .map_err(command_error)?
    {
        None => Ok(ColdOperation::<G>::EndOfInput.into()),
        // An executor-owned replay episode (a math field/group/choice branch
        // or discretionary part) retired mid-cell. This must be reported
        // exactly like ordinary `scan_step`'s `ReplayCompleted` case, rather
        // than falling through to interpret whatever the cascade found next
        // as this cell's own content: that next token can belong to the
        // *enclosing* cell/field context, not the just-retired episode.
        Some(AlignmentDelivery::Completed(episode)) => {
            Ok(ColdOperation::<G>::ReplayCompleted(episode).into())
        }
        Some(AlignmentDelivery::Command(command)) => {
            // TeX82 §§1034/1038 keeps an adjacent character fetched by
            // `main_loop_lookahead` inside `main_loop`, even when §789's
            // u-template/body handoff lies between the two characters. The
            // lookahead is a raw delivery owned by alignment control, but it
            // does not create a second §1030 `reswitch` trace boundary.
            let continues_main_loop = main_loop_active
                && matches!(
                    command.meaning(),
                    ResolvedMeaning::Static(
                        Meaning::CharToken {
                            cat: Catcode::Letter | Catcode::Other,
                            ..
                        } | Meaning::CharGiven(_)
                            | Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char)
                    )
                );
            if !continues_main_loop {
                report_main_control_command_trace(processor, mode, &command, boxes, shown_mode);
            }
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
                    ResolvedMeaning::Static(Meaning::CharToken {
                        cat: Catcode::EndGroup,
                        ..
                    })
                )
            {
                processor
                    .recover_alignment_closing_brace(
                        tex_command::AlignmentDeliveryEvent::ClosingBrace(command),
                    )
                    .map_err(command_error)?;
                return Ok(ColdOperation::<G>::MissingAlignmentCr.into());
            }
            if matches!(command.meaning(), ResolvedMeaning::Static(Meaning::EndV)) {
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
                    return Ok(ColdOperation::<G>::MissingMathShift.into());
                }
                if partoken_context_replays(processor, mode, 2) {
                    processor
                        .insert_partoken_before(command)
                        .map_err(command_error)?;
                    return Ok(ColdOperation::<G>::Continue.into());
                }
                // TeX82 §1131 accepts end-v only when `cur_group=align_group`.
                // The replay driver tracks align-error's inserted `{`
                // separately because its structural alignment boundary is
                // executor-owned. Ordinary `\begingroup` is nevertheless a
                // real `semi_simple_group` save-stack level, and must close
                // through §§1064--1065 `off_save` before the same end-v is
                // replayed. Other intervening groups are intercepted by their
                // owning mode/box delivery paths before reaching this cell
                // finish boundary.
                if boxes.recovery_simple_group_open
                    || innermost_group == Some(GroupKind::SemiSimple)
                {
                    return scan_off_save(processor, command, innermost_group).map(Into::into);
                }
                return Ok(ColdOperation::<G>::AlignmentCellFinish { alignment }.into());
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
                shown_mode,
                diagnostics,
                Some(alignment),
                true,
            )
        }
        Some(AlignmentDelivery::Event(event)) => {
            scan_alignment_delivery_event(processor, alignment, event).map(Into::into)
        }
    }
}

/// Applies a raw-delivery alignment boundary surfaced while TeX82 main
/// control owns an active entry.
///
/// Most boundaries come from `scan_alignment_delivery_step`'s initial
/// `get_x_token`, but §1045's `\ignorespaces` performs another expanded fetch
/// before returning to `reswitch`. TeX82 §342 still inserts the v-template at
/// that nested fetch, so the split executor must receive the same typed event
/// instead of dispatching the resulting frozen `\endv` as ordinary content.
fn scan_alignment_delivery_event<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    alignment: AlignmentIdentity,
    event: tex_command::AlignmentDeliveryEvent<G>,
) -> Result<ColdOperation<G>, ExecError> {
    match event {
        tex_command::AlignmentDeliveryEvent::EndTemplate(_) => {
            processor
                .begin_alignment_v_template(alignment, event)
                .map_err(command_error)?;
            Ok(ColdOperation::<G>::AlignmentTemplateEntered)
        }
        tex_command::AlignmentDeliveryEvent::ClosingBrace(_) => {
            // TeX82 §1132 selects this executor-owned align_group branch. Raw
            // brace backup/correction and frozen-\cr insertion remain entirely
            // command-owned.
            processor
                .recover_alignment_closing_brace(event)
                .map_err(command_error)?;
            Ok(ColdOperation::<G>::MissingAlignmentCr)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn settle_preflight_step<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    command: tex_command::CurrentCommand<G>,
    main_loop: bool,
    mode: Mode,
    boxes: &ReplayBoxes<G>,
    innermost_group: Option<GroupKind>,
    job_is_all_over: bool,
    display_eq_no: bool,
    shown_mode: &mut Option<Mode>,
    diagnostics: &mut Vec<PendingDiagnostic<G>>,
) -> Result<ScannedOperation<G>, ExecError> {
    let delivery = processor
        .settle_preflight_command(command, main_loop)
        .map_err(command_error)?;
    let Some(delivery) = delivery else {
        return Ok(ColdOperation::<G>::EndOfInput.into());
    };
    let tex_command::CommandReplayDelivery::Command(command) = delivery else {
        let tex_command::CommandReplayDelivery::Completed(episode) = delivery else {
            unreachable!();
        };
        return Ok(ColdOperation::<G>::ReplayCompleted(episode).into());
    };
    let continues_main_loop = main_loop
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(
                Meaning::CharToken {
                    cat: Catcode::Letter | Catcode::Other,
                    ..
                } | Meaning::CharGiven(_)
                    | Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char)
            )
        );
    if !continues_main_loop {
        report_main_control_command_trace(processor, mode, &command, boxes, shown_mode);
    }
    if main_loop
        && matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal)
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::NoBoundary
            ))
        )
    {
        return Ok(ColdOperation::<G>::NoBoundary {
            suppress_right: true,
        }
        .into());
    }
    dispatch_main_control_command(
        processor,
        command,
        mode,
        boxes,
        innermost_group,
        job_is_all_over,
        display_eq_no,
        shown_mode,
        diagnostics,
        None,
        true,
    )
}

#[allow(clippy::too_many_arguments)] // carries command-owned replay facts
fn scan_step<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    mode: Mode,
    boxes: &ReplayBoxes<G>,
    innermost_group: Option<GroupKind>,
    job_is_all_over: bool,
    display_eq_no: bool,
    main_loop_active: bool,
    shown_mode: &mut Option<Mode>,
    diagnostics: &mut Vec<PendingDiagnostic<G>>,
) -> Result<ScannedOperation<G>, ExecError> {
    // TeX82 §1030 has two fetch labels, not one. `big_switch` uses
    // `get_x_token`; §1034's inner character loop instead re-enters at
    // §1038's `main_loop_lookahead`, whose bare `get_next` is what keeps a
    // run of adjacent characters from being delivered through expansion.
    prepare_command_trace(processor, mode, *shown_mode);
    let delivery = if main_loop_active {
        processor.main_loop_lookahead()
    } else {
        processor.get_x_token_with_replay_completion()
    };
    let Some(delivery) = delivery.map_err(command_error)? else {
        return Ok(ColdOperation::<G>::EndOfInput.into());
    };
    let tex_command::CommandReplayDelivery::Command(command) = delivery else {
        let tex_command::CommandReplayDelivery::Completed(episode) = delivery else {
            unreachable!();
        };
        return Ok(ColdOperation::<G>::ReplayCompleted(episode).into());
    };
    // TeX82 §§1034/1038 keeps a fetched character inside `main_loop`;
    // it reaches neither `reswitch` nor §1030's command trace. A
    // non-character fetched by the same lookahead does go to `reswitch` and
    // must retain the ordinary trace boundary.
    let continues_main_loop = main_loop_active
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(
                Meaning::CharToken {
                    cat: Catcode::Letter | Catcode::Other,
                    ..
                } | Meaning::CharGiven(_)
                    | Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char)
            )
        );
    if !continues_main_loop {
        report_main_control_command_trace(processor, mode, &command, boxes, shown_mode);
    }
    if main_loop_active
        && matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal)
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::NoBoundary
            ))
        )
    {
        return Ok(ColdOperation::<G>::NoBoundary {
            suppress_right: true,
        }
        .into());
    }
    dispatch_main_control_command(
        processor,
        command,
        mode,
        boxes,
        innermost_group,
        job_is_all_over,
        display_eq_no,
        shown_mode,
        diagnostics,
        None,
        true,
    )
}

fn execution_error_needs_command_retry(error: &ExecError) -> bool {
    match error {
        ExecError::Captured { error, .. } => execution_error_needs_command_retry(error),
        ExecError::MissingInput { .. }
        | ExecError::MissingInputProbe { .. }
        | ExecError::MissingFont { .. }
        | ExecError::MissingPdfImage { .. } => true,
        _ => false,
    }
}

/// Whether a settled command can reach the ranked hot scanner without first
/// crossing a transaction, resource, diagnostic, or contextual dispatcher
/// barrier. Prefixes deliberately do not qualify: their substantive command
/// is not known until the transactional prefix loop has run.
fn direct_hot_candidate<G>(
    mode: Mode,
    boxes: &ReplayBoxes<G>,
    innermost_group: Option<GroupKind>,
    command: &tex_command::CurrentCommand<G>,
) -> bool {
    if boxes.pending_leader.is_some() {
        return false;
    }
    match command.meaning() {
        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::Def
            | UnexpandablePrimitive::Edef
            | UnexpandablePrimitive::Gdef
            | UnexpandablePrimitive::Xdef
            | UnexpandablePrimitive::Let
            | UnexpandablePrimitive::FutureLet
            | UnexpandablePrimitive::CatCode
            | UnexpandablePrimitive::BeginGroup,
        )) => true,
        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::EndGroup,
        )) => innermost_group == Some(GroupKind::SemiSimple),
        ResolvedMeaning::Static(Meaning::CharToken {
            cat: Catcode::BeginGroup,
            ..
        }) => {
            !matches!(mode, Mode::Math | Mode::DisplayMath)
                && !boxes.output_routine_opening_pending
                && !boxes.recovery_simple_group_pending
        }
        ResolvedMeaning::Static(Meaning::CharToken {
            cat: Catcode::EndGroup,
            ..
        }) => innermost_group == Some(GroupKind::Simple) && !boxes.recovery_simple_group_open,
        _ => false,
    }
}

/// Scans a command proven by [`direct_hot_candidate`] to have no contextual
/// dispatcher work before the ranked hot family. The command remains borrowed
/// so an actual immutable-resource suspension can move it into the exact retry
/// continuation without a speculative clone.
fn scan_direct_hot_command<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    command: &tex_command::CurrentCommand<G>,
    innermost_group: Option<GroupKind>,
) -> Result<hot_apply::HotOperation<G>, ExecError> {
    #[cfg(feature = "profiling")]
    {
        tex_state::measurement::record_hot_core_command_family(hot_core_command_family(
            command.meaning(),
        ));
        if let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(primitive)) =
            command.meaning()
        {
            tex_state::measurement::record_hot_core_unexpandable_opcode(
                usize::try_from(primitive.operand())
                    .expect("unexpandable primitive operand fits usize"),
            );
        }
    }
    if innermost_group == Some(GroupKind::Simple)
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            })
        )
    {
        return Ok(hot_apply::HotOperation::<G>::end_ordinary_group());
    }
    let global = effective_global(
        processor.int_param(IntParam::GLOBAL_DEFS),
        matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::Gdef | UnexpandablePrimitive::Xdef
            ))
        ),
    );
    match hot_apply::scan(
        processor,
        command,
        global,
        MeaningFlags::EMPTY,
        innermost_group,
    ) {
        Ok(Some(operation)) => Ok(operation),
        Ok(None) => unreachable!("direct hot candidate reaches the ranked hot scanner"),
        Err(error) => Err(error.capture_command_origin(command.origin())),
    }
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
fn dispatch_main_control_command<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    mut command: tex_command::CurrentCommand<G>,
    mode: Mode,
    boxes: &ReplayBoxes<G>,
    innermost_group: Option<GroupKind>,
    job_is_all_over: bool,
    display_eq_no: bool,
    shown_mode: &mut Option<Mode>,
    diagnostics: &mut Vec<PendingDiagnostic<G>>,
    alignment: Option<AlignmentIdentity>,
    set_box_allowed: bool,
) -> Result<ScannedOperation<G>, ExecError> {
    // TeX82 §1078 uses §404's non-blank, non-relax fetch after every leader
    // payload. Constructed boxes close in a separate replay step, so the first
    // token after the box has already reached this dispatcher. Finish §404
    // here without exposing its filler to main control or command tracing.
    if boxes.pending_leader.is_some()
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                } | Meaning::Relax
            )
        )
    {
        command = processor
            .next_non_blank_non_relax_x_token()
            .map_err(command_error)?
            .ok_or(ExecError::MissingToken {
                context: "leader glue",
            })?;
    }
    let origin = command.origin();
    dispatch_main_control_command_inner(
        processor,
        command,
        mode,
        boxes,
        innermost_group,
        job_is_all_over,
        display_eq_no,
        shown_mode,
        diagnostics,
        alignment,
        set_box_allowed,
    )
    .map_err(|error| error.capture_command_origin(origin))
}

#[cfg(feature = "profiling")]
fn hot_core_command_family<G>(
    meaning: ResolvedMeaning<G>,
) -> tex_state::measurement::HotCoreCommandFamily {
    use tex_state::measurement::HotCoreCommandFamily as Family;

    match meaning {
        ResolvedMeaning::Static(
            Meaning::CharGiven(_) | Meaning::CharToken { .. } | Meaning::MathCharGiven(_),
        ) => Family::Character,
        ResolvedMeaning::Static(Meaning::Relax) => Family::Relax,
        ResolvedMeaning::Static(Meaning::Undefined) => Family::Undefined,
        ResolvedMeaning::Macro { .. } => Family::Macro,
        ResolvedMeaning::Static(Meaning::ExpandablePrimitive(_)) => Family::ExpandablePrimitive,
        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(_)) => Family::UnexpandablePrimitive,
        ResolvedMeaning::Static(
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
            | Meaning::PageInteger(_),
        ) => Family::RegisterOrParameter,
        ResolvedMeaning::Static(Meaning::Font(_)) => Family::Font,
        ResolvedMeaning::Static(Meaning::InternalInteger(_)) => Family::InternalQuantity,
        ResolvedMeaning::Static(Meaning::EndV) => Family::EndTemplate,
        ResolvedMeaning::Static(Meaning::Unknown(_)) => Family::Unknown,
    }
}

#[allow(clippy::too_many_arguments)] // carries command-owned replay facts
fn dispatch_main_control_command_inner<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    mut command: tex_command::CurrentCommand<G>,
    mode: Mode,
    boxes: &ReplayBoxes<G>,
    innermost_group: Option<GroupKind>,
    job_is_all_over: bool,
    display_eq_no: bool,
    shown_mode: &mut Option<Mode>,
    diagnostics: &mut Vec<PendingDiagnostic<G>>,
    alignment: Option<AlignmentIdentity>,
    set_box_allowed: bool,
) -> Result<ScannedOperation<G>, ExecError> {
    // TeX82 §1078 fetches the command following a completed leader payload
    // inside `box_end`, before control returns to §1030's `big_switch` or
    // §1211's prefix loop. Split replay finishes the box in one step and
    // delivers that command in the next, so classify it at this same outer
    // boundary. In particular, a non-glue `\global` is the command that
    // `back_error` must restore; allowing it into the prefix loop first would
    // consume and restore the following assignment instead.
    if let Some((kind, payload)) = boxes.pending_leader.as_ref() {
        let Some(glue) = scan_leader_glue_command(processor, command, mode)? else {
            return Ok(ColdOperation::<G>::LeadersNotFollowedByGlue.into());
        };
        return Ok(ColdOperation::<G>::Leaders {
            kind: *kind,
            payload: payload.clone(),
            glue,
        }
        .into());
    }
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
            #[cfg(feature = "profiling")]
            {
                tex_state::measurement::record_hot_core_command_family(hot_core_command_family(
                    command.meaning(),
                ));
                if let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(primitive)) =
                    command.meaning()
                {
                    tex_state::measurement::record_hot_core_unexpandable_opcode(
                        usize::try_from(primitive.operand())
                            .expect("unexpandable primitive operand fits usize"),
                    );
                }
            }
            match command.meaning() {
                ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::Global,
                )) => global = true,
                ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::Long,
                )) => flags = flags | MeaningFlags::LONG,
                ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::Outer,
                )) => flags = flags | MeaningFlags::OUTER,
                ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::Protected,
                )) => flags = flags | MeaningFlags::PROTECTED,
                _ => break,
            }
            command = processor
                .next_non_blank_non_relax_x_token()
                .map_err(command_error)?
                .ok_or(ExecError::MissingPrefixedCommand)?;
            // §1211's `if cur_cmd<=max_non_prefixed_command then <Discard
            // erroneous prefixes and return>`: §209's partition, not a
            // hand-listed set of assignment families.
            if !tex_command::exceeds_max_non_prefixed_command(static_meaning(command.meaning())) {
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
                return Ok(ColdOperation::<G>::Continue.into());
            }
        }
        // §1213's `<Discard the prefixes \long and \outer if they are
        // irrelevant>`. §1214 deliberately leaves `a` unadjusted, so the
        // command still runs; only the report is owed. eTeX's `\protected`
        // is prefix code 8, which §1213's `a mod 4<>0` excludes.
        if flags.bits() & (MeaningFlags::LONG | MeaningFlags::OUTER).bits() != 0
            && !matches!(
                command.meaning(),
                ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::Def
                        | UnexpandablePrimitive::Edef
                        | UnexpandablePrimitive::Gdef
                        | UnexpandablePrimitive::Xdef
                ))
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
            ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::IgnoreSpaces
            ))
        ) {
            let next = if let Some(alignment) = alignment {
                loop {
                    match processor
                        .get_x_alignment_delivery(false)
                        .map_err(command_error)?
                    {
                        None => return Ok(ColdOperation::<G>::EndOfInput.into()),
                        Some(AlignmentDelivery::Completed(episode)) => {
                            return Ok(ColdOperation::<G>::ReplayCompleted(episode).into());
                        }
                        Some(AlignmentDelivery::Event(event)) => {
                            return scan_alignment_delivery_event(processor, alignment, event)
                                .map(Into::into);
                        }
                        Some(AlignmentDelivery::Command(next))
                            if matches!(
                                next.meaning(),
                                ResolvedMeaning::Static(Meaning::CharToken {
                                    cat: Catcode::Space,
                                    ..
                                })
                            ) => {}
                        Some(AlignmentDelivery::Command(next)) => break next,
                    }
                }
            } else {
                let Some(next) = processor.next_non_blank_x_token().map_err(command_error)? else {
                    return Ok(ColdOperation::<G>::EndOfInput.into());
                };
                next
            };
            command = next;
            report_command_trace(processor, mode, &command, shown_mode);
            continue;
        }
        if matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal)
            && matches!(
                command.meaning(),
                ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::NoBoundary
                ))
            )
        {
            let Some(next) = processor.get_x_token().map_err(command_error)? else {
                return Ok(ColdOperation::<G>::Continue.into());
            };
            suppress_left_boundary = matches!(
                next.meaning(),
                ResolvedMeaning::Static(
                    Meaning::CharToken {
                        cat: Catcode::Letter | Catcode::Other,
                        ..
                    } | Meaning::CharGiven(_)
                        | Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char)
                )
            );
            command = next;
            report_command_trace(processor, mode, &command, shown_mode);
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
                    ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                        UnexpandablePrimitive::Gdef | UnexpandablePrimitive::Xdef
                    ))
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
            set_box_allowed,
            shown_mode,
        )?;
        if suppress_left_boundary
            && let ScannedOperation::<G>::Cold(
                ColdOperation::<G>::Character {
                    suppress_left_boundary,
                    ..
                }
                | ColdOperation::<G>::CharacterCode {
                    suppress_left_boundary,
                    ..
                },
            ) = &mut scanned
        {
            *suppress_left_boundary = true;
        }
        return Ok(scanned);
    }
}

/// TeX82 §1030's `if tracing_commands>0 then show_cur_cmd_chr` at `reswitch`,
/// reached after `big_switch` and after cases such as §1045 `ignore_spaces`
/// fetch a replacement command. §1211's prefix loop does not return to that
/// label, so its internal fetches remain untraced.
fn report_command_trace<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    mode: Mode,
    command: &tex_command::CurrentCommand<G>,
    shown_mode: &mut Option<Mode>,
) {
    if processor.int_param(IntParam::TRACING_COMMANDS) > 0 {
        *shown_mode = Some(mode);
        processor.print_command_trace(tex_command::PrintCommand::from_current(command));
    }
}

/// Applies TeX82's §1030 main-control trace boundary to a fetched command.
///
/// A constructed leader payload is one exception: after its box closes,
/// §1078's `box_end` fetches the following glue inside the leader case and
/// never returns to `big_switch`. The split replay lifecycle leaves that
/// internal fetch to the next processor episode, so `pending_leader` retains
/// the canonical boundary distinction and suppresses only §1030's settled
/// unexpandable-command trace. Expansion tracing performed by `get_x_token`
/// remains unchanged.
///
/// The opening brace of an output routine is the other exception. TeX82
/// §1025 consumes it with `scan_left_brace` before entering §1030, whereas
/// split replay delivers it as an explicit step. Suppressing that delivery
/// also leaves the mode prefix pending for the first command in the routine.
///
/// A `\shipout` box constructor is likewise scanner-owned: §§1075/1084 call
/// `scan_box` from the already-traced `leader_ship` case, so its `\hbox`,
/// `\vbox`, or `\vtop` never returns to §1030's `reswitch`. Split replay
/// retains `pending_shipout` across that internal fetch.
fn report_main_control_command_trace<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    mode: Mode,
    command: &tex_command::CurrentCommand<G>,
    boxes: &ReplayBoxes<G>,
    shown_mode: &mut Option<Mode>,
) {
    // Expansion can invoke §299 itself for e-TeX's `\tracingifs`. Its mode
    // prefix and `shown_mode` transition precede this settled command.
    if processor.command_trace_printed() {
        *shown_mode = Some(mode);
    }
    let output_routine_opening = boxes.output_routine_opening_pending
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::BeginGroup,
                ..
            })
        );
    let shipout_box_constructor = boxes.pending_shipout
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::HBox
                    | UnexpandablePrimitive::VBox
                    | UnexpandablePrimitive::VTop
            ))
        );
    if boxes.pending_leader.is_none() && !output_routine_opening && !shipout_box_constructor {
        report_command_trace(processor, mode, command, shown_mode);
    }
}

fn prepare_command_trace<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    mode: Mode,
    shown_mode: Option<Mode>,
) {
    // The mode text is owned because expansion may retain it until a later
    // command in this processor episode.  Do not allocate that ownership when
    // neither command nor conditional tracing can consume the prefix: this is
    // the ordinary production path and this boundary is crossed once per
    // main-control operation.
    let tracing_can_consume_prefix = processor.int_param(IntParam::TRACING_COMMANDS) > 0
        || processor.int_param(IntParam::TRACING_IFS) > 0;
    let mode_prefix = (tracing_can_consume_prefix && shown_mode != Some(mode))
        .then(|| mode_text_for_command_trace(mode).into());
    processor.set_command_trace_mode_prefix(mode_prefix);
}

fn scan_leaders_step<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    primitive: UnexpandablePrimitive,
    mode: Mode,
) -> Result<ColdOperation<G>, ExecError> {
    let kind = crate::box_runtime::leader_glue_kind(primitive);
    match processor.scan_leader_payload().map_err(command_error)? {
        ScannedLeaderPayload::Missing => Ok(ColdOperation::<G>::MissingLeaderPayload),
        ScannedLeaderPayload::Construction(construction) => {
            Ok(ColdOperation::<G>::BeginLeaderBox { construction, kind })
        }
        ScannedLeaderPayload::Rule(rule) => {
            let glue_command = processor
                .next_non_blank_non_relax_x_token()
                .map_err(command_error)?
                .ok_or(ExecError::MissingToken {
                    context: "leader glue",
                })?;
            let Some(glue) = scan_leader_glue_command(processor, glue_command, mode)? else {
                return Ok(ColdOperation::<G>::LeadersNotFollowedByGlue);
            };
            let payload = LeaderPayload::Rule {
                width: rule.width,
                height: rule.height,
                depth: rule.depth,
            };
            Ok(ColdOperation::<G>::Leaders {
                kind,
                payload,
                glue,
            })
        }
        // Register payloads must retain their destructive/copy ownership at
        // replay time.  Keep the command scanner's completed glue read, then
        // use the regular typed box read path to obtain the node.
        ScannedLeaderPayload::BoxRegister { index, copy } => {
            let glue_command = processor
                .next_non_blank_non_relax_x_token()
                .map_err(command_error)?
                .ok_or(ExecError::MissingToken {
                    context: "leader glue",
                })?;
            let Some(glue) = scan_leader_glue_command(processor, glue_command, mode)? else {
                return Ok(ColdOperation::<G>::LeadersNotFollowedByGlue);
            };
            Ok(ColdOperation::<G>::LeaderRegister {
                kind,
                index,
                copy,
                glue,
            })
        }
    }
}

fn scan_leader_glue_command<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    command: tex_command::CurrentCommand<G>,
    mode: Mode,
) -> Result<Option<GlueSpec>, ExecError> {
    let horizontal = matches!(
        mode,
        Mode::Horizontal | Mode::RestrictedHorizontal | Mode::Math | Mode::DisplayMath
    );
    let primitive = match command.meaning() {
        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(primitive)) => primitive,
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
fn starts_paragraph_in_vertical_mode<G>(meaning: ResolvedMeaning<G>) -> bool {
    match meaning {
        // `vmode+letter`, `vmode+other_char`, and `vmode+math_shift`. A
        // `spacer` is deliberately absent: §1045's `vmode+spacer: do_nothing`
        // leaves vertical mode untouched, and every other category code
        // (braces, `#`, `^`, `_`, `~`) has its own case elsewhere.
        ResolvedMeaning::Static(Meaning::CharToken { cat, .. }) => {
            matches!(cat, Catcode::Letter | Catcode::Other | Catcode::MathShift)
        }
        // `vmode+char_given`: a `\chardef`'d token (§1224 installs it as
        // `char_given`), which §1090 treats exactly like `char_num`.
        ResolvedMeaning::Static(Meaning::CharGiven(_)) => true,
        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(primitive)) => matches!(
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
fn scan_command<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    command: tex_command::CurrentCommand<G>,
    global: bool,
    flags: MeaningFlags,
    mode: Mode,
    boxes: &ReplayBoxes<G>,
    innermost_group: Option<GroupKind>,
    job_is_all_over: bool,
    display_eq_no: bool,
    set_box_allowed: bool,
    shown_mode: &mut Option<Mode>,
) -> Result<ScannedOperation<G>, ExecError> {
    if let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
        primitive @ (UnexpandablePrimitive::TextFont
        | UnexpandablePrimitive::ScriptFont
        | UnexpandablePrimitive::ScriptScriptFont),
    )) = command.meaning()
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
        return Ok(ColdOperation::<G>::MathFamily {
            family,
            font,
            global,
        }
        .into());
    }
    // Math operands are scanned exclusively by `tex-command`.  The replay
    // driver receives a typed scalar request and schedules any opaque field
    // episode only after this processor borrow has ended.
    if matches!(mode, Mode::Math | Mode::DisplayMath)
        && let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Left
            | UnexpandablePrimitive::Right
            | UnexpandablePrimitive::Middle),
        )) = command.meaning()
    {
        let kind = match primitive {
            UnexpandablePrimitive::Left => MathDelimiterBoundaryKind::Left,
            UnexpandablePrimitive::Right => MathDelimiterBoundaryKind::Right,
            UnexpandablePrimitive::Middle => MathDelimiterBoundaryKind::Middle,
            _ => unreachable!(),
        };
        return Ok(ColdOperation::<G>::MathDelimiter(
            processor
                .scan_math_delimiter_boundary(kind)
                .map_err(command_error)?,
        )
        .into());
    }
    if matches!(mode, Mode::Math | Mode::DisplayMath)
        && let Some(request) = processor
            .scan_math_request(&command)
            .map_err(command_error)?
    {
        return Ok(ColdOperation::<G>::Math(request).into());
    }
    if matches!(mode, Mode::Math | Mode::DisplayMath)
        && let ResolvedMeaning::Static(Meaning::CharToken {
            cat: Catcode::Superscript,
            ..
        }) = command.meaning()
    {
        return Ok(
            ColdOperation::<G>::Math(MathRequest::Script(tex_command::ScannedMathScript {
                kind: MathScriptKind::Superscript,
                provenance: tex_command::StructuredProvenance {
                    primary: command.origin(),
                },
            }))
            .into(),
        );
    }
    if matches!(mode, Mode::Math | Mode::DisplayMath)
        && let ResolvedMeaning::Static(Meaning::CharToken {
            cat: Catcode::Subscript,
            ..
        }) = command.meaning()
    {
        return Ok(
            ColdOperation::<G>::Math(MathRequest::Script(tex_command::ScannedMathScript {
                kind: MathScriptKind::Subscript,
                provenance: tex_command::StructuredProvenance {
                    primary: command.origin(),
                },
            }))
            .into(),
        );
    }

    if boxes.output_routine_opening_pending
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::BeginGroup,
                ..
            })
        )
    {
        return Ok(ColdOperation::<G>::OutputRoutineOpeningBrace.into());
    }
    // `align_error`'s inserted brace is an actual execution group, even when
    // it appears inside a replayed box body.  It must therefore win over the
    // box body's brace-depth bookkeeping so §1131 can observe it at end-v.
    if boxes.recovery_simple_group_pending
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::BeginGroup,
                ..
            })
        )
    {
        return Ok(ColdOperation::<G>::BeginSimpleGroup.into());
    }
    if boxes.recovery_simple_group_open
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            })
        )
    {
        return Ok(ColdOperation::<G>::EndSimpleGroup.into());
    }
    // TeX82 §1068 dispatches a right brace from the current `cur_group`.
    // An ancestor simple group must not make a nested box's body closer look
    // like an ordinary group closer.
    if innermost_group == Some(GroupKind::Simple)
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            })
        )
    {
        return Ok(ScannedOperation::<G>::Hot(
            hot_apply::HotOperation::<G>::end_ordinary_group(),
        ));
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
            ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            })
        )
    {
        return Ok(ColdOperation::<G>::EndMathGroup(kind).into());
    }
    if innermost_group == Some(GroupKind::Disc)
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            })
        )
    {
        return Ok(ColdOperation::<G>::DiscretionaryPartEnd.into());
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
        && let ResolvedMeaning::Static(Meaning::CharToken {
            cat: Catcode::BeginGroup,
            ..
        }) = command.meaning()
    {
        processor.back_input(command).map_err(command_error)?;
        return Ok(ColdOperation::<G>::Math(MathRequest::TextField(MathTextFieldKind::Ord)).into());
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
            ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            })
        )
    {
        let threshold = match box_state.kind {
            ReplayBoxKind::VBox | ReplayBoxKind::VTop | ReplayBoxKind::VCenter => 1,
            ReplayBoxKind::Insert(..) => 2,
            ReplayBoxKind::HBox => 0,
        };
        if threshold != 0 && partoken_context_replays(processor, mode, threshold) {
            processor
                .insert_partoken_before(command)
                .map_err(command_error)?;
            return Ok(ColdOperation::<G>::Continue.into());
        }
        return Ok(ColdOperation::<G>::BoxEndGroup {
            ships_out: box_state.ships_out,
        }
        .into());
    }
    // TeX82 §1016 opens `output_group` before replaying the braced output
    // token list. A box body nested in that list owns its closing brace first;
    // only the live output group can close the enclosing output routine.
    if boxes.output_routine_active
        && innermost_group == Some(GroupKind::Output)
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            })
        )
    {
        if partoken_context_replays(processor, mode, 2) {
            processor
                .insert_partoken_before(command)
                .map_err(command_error)?;
            return Ok(ColdOperation::<G>::Continue.into());
        }
        return Ok(ColdOperation::<G>::EndOutputRoutine.into());
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
        return Ok(ColdOperation::<G>::ParagraphStart.into());
    }
    if let Some(operation) = hot_apply::scan(processor, &command, global, flags, innermost_group)? {
        return Ok(ScannedOperation::<G>::Hot(operation));
    }
    scan_cold_operation(
        processor,
        command,
        global,
        mode,
        boxes,
        innermost_group,
        job_is_all_over,
        display_eq_no,
        set_box_allowed,
        shown_mode,
    )
    .map(Into::into)
}

#[allow(clippy::too_many_arguments)]
fn operation_termination(step: ReplayStep, fatal: Option<FatalError>) -> OperationTermination {
    fatal.map_or_else(
        || match step {
            ReplayStep::Continue => OperationTermination::Continue,
            ReplayStep::End => OperationTermination::End,
            ReplayStep::EndOfInput => OperationTermination::EndOfInput,
        },
        OperationTermination::Fatal,
    )
}

fn execution_error_is_fuel(error: &ExecError) -> bool {
    match error {
        ExecError::Captured { error, .. } => execution_error_is_fuel(error),
        ExecError::CumulativeFuelExceeded { .. }
        | ExecError::Command(CommandError::FuelExhausted { .. }) => true,
        _ => false,
    }
}

#[cfg(test)]
mod discretionary_hyphen_tests {
    use super::*;
    use tex_state::AssignmentScope;

    #[test]
    fn disabled_hyphen_char_leaves_pre_break_empty() {
        // TeX82 §1113 inserts the current font's hyphen character only in
        // 0..256. In particular, -1 disables the visible pre-break.
        crate::test_harness::with_plain_universe(|stores| {
            crate::test_harness::with_admitted(stores, |context| {
                context.set_font_hyphen_char(context.current_font(), -1);
            });
            let mut control = MainControl::tex82_initex(stores);
            control
                .register_root_source(tex_command::SourceRegistration::new(
                    tex_command::RegisteredSourceKind::Generated,
                    br"\noindent\-\end".to_vec(),
                ))
                .expect("register canonical source");

            assert_eq!(
                control.step(stores).expect("paragraph start"),
                MainControlStep::Continue
            );
            assert_eq!(
                control.step(stores).expect("explicit hyphen"),
                MainControlStep::Continue
            );
            let Some(Node::Disc {
                kind: DiscKind::ExplicitHyphen,
                pre,
                ..
            }) = control.modes.current_list().nodes().last()
            else {
                panic!("canonical replay appended an explicit discretionary hyphen");
            };
            assert!(pre.is_empty());
        });
    }

    #[test]
    fn missing_hyphen_glyph_leaves_pre_break_empty() {
        // TeX82 §§581/1113: an in-range hyphen character is constructed via
        // `new_character`, which warns and returns null for an absent glyph.
        crate::test_harness::with_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            crate::test_harness::with_admitted(stores, |context| {
                context
                    .assign_int_param(IntParam::TRACING_LOST_CHARS, 1, AssignmentScope::Local)
                    .expect("tracing lost characters assignment");
                context
                    .assign_int_param(IntParam::TRACING_ONLINE, 1, AssignmentScope::Local)
                    .expect("tracing online assignment");
                context.set_font_hyphen_char(context.current_font(), i32::from(b'?'));
            });
            control
                .register_root_source(tex_command::SourceRegistration::new(
                    tex_command::RegisteredSourceKind::Generated,
                    br"\noindent\-\end".to_vec(),
                ))
                .expect("register canonical source");

            assert_eq!(
                control.step(stores).expect("paragraph start"),
                MainControlStep::Continue
            );
            assert_eq!(
                control.step(stores).expect("explicit hyphen"),
                MainControlStep::Continue
            );
            let Some(Node::Disc { pre, .. }) = control.modes.current_list().nodes().last() else {
                panic!("canonical replay appended an explicit discretionary hyphen");
            };
            assert!(pre.is_empty());
        });
    }

    #[test]
    fn deferred_write_trace_precedes_unbalanced_report() {
        // TeX82 §§1369--1372: `write_out` traces the write-text token list
        // and expands its condition before testing the frozen `\endwrite`
        // stopper. Atomic shipout staging must retain that live-call order.
        crate::test_harness::with_plain_universe(|stores| {
            *stores.world_mut() = tex_state::World::memory();
            let mut control = MainControl::tex82_initex(stores);
            control
            .register_root_source(tex_command::SourceRegistration::new(
                tex_command::RegisteredSourceKind::Generated,
                br"\nonstopmode\tracingmacros=2\shipout\hbox{\write16{\if01{\else unbal}\fi}}\end"
                    .to_vec(),
            ))
            .expect("register canonical source");
            while let MainControlStep::Continue =
                control.step(stores).expect("write source executes")
            {}
            let log = String::from_utf8_lossy(
                stores
                    .world()
                    .memory_log_output()
                    .expect("memory world retains committed log output"),
            );
            let trace = log
                .find("write->")
                .filter(|&start| log[start..].contains("unbal"))
                .unwrap_or_else(|| panic!("write trace is visible: {log:?}"));
            let report = log
                .find("Unbalanced write command")
                .unwrap_or_else(|| panic!("write report is visible: {log:?}"));
            assert!(trace < report, "{log:?}");
        });
    }
}

#[cfg(test)]
#[path = "main_control/tests.rs"]
mod direct_tests;
