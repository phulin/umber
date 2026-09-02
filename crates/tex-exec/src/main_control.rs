//! Production main-control driver.
//!
//! This intentionally owns only typed execution mutations. Raw delivery,
//! expansion, macro calls, input nesting, and operand collection remain in
//! `tex-command`; no independent source stack is accepted here.

use std::collections::VecDeque;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::Arc;
use tex_command::{
    AlignmentCellDelimiter, AlignmentCellOpening, AlignmentIdentity, AlignmentRequest,
    AlignmentRequestResult, CommandError, CommandHostCapabilities, CommandHostContext,
    CommandProcessor, CommandProfile, CommandState, FatalError, FontLoadRequest, FontResource,
    GeneratedFontKind, HyphenationDataKind, ImmediateExtension, MathDelimiterBoundary,
    MathDelimiterBoundaryKind, MathFieldBody, MathLimitKind, MathRequest, MathScriptKind,
    MathStyleKind, MathTextFieldKind, PdfImageRequest, PdfImageResource, PdfReferenceObjectRequest,
    PreparedAlignmentCellTemplates, RegisteredSourceKind, RestrictedIntegerClass, ScannedAccent,
    ScannedAccentBase, ScannedBoxConstruction, ScannedBoxKind, ScannedBoxShift,
    ScannedBoxShiftPayload, ScannedDiscretionaryOpening, ScannedDisplayDiagnostic,
    ScannedGeneratedFontDefinition, ScannedInsertConstruction, ScannedLeaderPayload,
    ScannedMathMuMaterial, ScannedPackingSpec, ScannedSetBoxPath, ScannedVSplit,
    SourceRegistration, SourceRegistrationError,
};
use tex_command::{
    CommandObservation, CommandObserver, EffectRecord, GeometryRecord, MutationRecord,
    MutationTarget, ObservationEffectKind, ObservationValue, ObservedToken, ParameterClass,
    TokenListRecord, parameter_mutation_key_for_dialect,
};
use tex_state::GlueId;
use tex_state::code_tables::{DelCode, LcCode, MathCode, SfCode, UcCode};
use tex_state::diagnostic::DiagnosticEffects;
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
use tex_state::page::{PageDimension, PageFireUp, PageInteger};
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

/// Consumes one destination-directed scalar result at its producing call
/// site. The successful path moves only the typed value; a real cold edge
/// moves the error into the surrounding operation result and leaves the exact
/// scanner child in the operation frame's existing continuation field.
macro_rules! take_operation_scalar {
    ($frame:expr, $status:expr, $phase:expr, $suspended:expr, $take:ident) => {{
        match $status {
            tex_command::ScalarScanStatus::Complete => {
                *$suspended = None;
                $frame.$take()
            }
            tex_command::ScalarScanStatus::Suspended => {
                *$suspended = Some($phase);
                return Err(command_error($frame.take_error()));
            }
            tex_command::ScalarScanStatus::Failed => {
                *$suspended = None;
                return Err(command_error($frame.take_error()));
            }
        }
    }};
}

/// Writes a completed cold leaf at its final caller-owned destination.
macro_rules! write_cold_scan {
    ($cold:expr, $operation:expr $(,)?) => {{
        assert!(
            $cold.operation.is_none(),
            "one operation frame owns one cold leaf"
        );
        $cold.operation = Some($operation);
    }};
}

/// Completes one cold scan by constructing its semantic leaf directly in the
/// caller-owned slot. The scanner returns only success/failure control; the
/// large operation value never crosses a helper return ABI.
macro_rules! complete_cold_scan {
    ($cold:expr, $operation:expr $(,)?) => {{
        write_cold_scan!($cold, $operation);
        Ok(())
    }};
}

mod cold;
mod command_episode;
mod delivery;
mod executor_facts;
mod hot_apply;
mod settlement;

use cold::*;
use command_episode::*;
use delivery::*;
use executor_facts::ExecutorHostFacts;
use executor_facts::{OperationPreparation, OperationPreparationScope};
use settlement::*;

type PreparedDviPages = Vec<crate::dispatch::PreparedDviPage>;
type GluePointerSource<G> = Option<(GlueId<G>, Option<GlueId<G>>)>;

/// TeX82 §1176's live `math_shift_group` context as observed by e-TeX
/// [49.1292]. Equation-number groups retain §1177's saved side.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MathShiftContext {
    Inline,
    Display,
    EqNo(crate::mode::EqNoSide),
}

fn push_prepared_dvi_page(pages: &mut PreparedDviPages, page: crate::dispatch::PreparedDviPage) {
    pages.push(page);
}

fn take_prepared_dvi_pages(pages: &mut PreparedDviPages) -> Vec<crate::dispatch::PreparedDviPage> {
    std::mem::take(pages)
}

fn static_meaning<G>(meaning: ResolvedMeaning<G>) -> Meaning {
    match meaning {
        ResolvedMeaning::Static(meaning) => meaning,
        ResolvedMeaning::Macro { .. } => Meaning::Undefined,
    }
}

/// Writes TeX82 §404's next nonblank, non-relax expanded command into the
/// executor's final command slot.
///
/// The command core owns every delivery and expansion transition. Main
/// control owns only this reswitch/prefix classification and never returns a
/// completed command through an intermediate convenience envelope.
fn next_non_blank_non_relax_x_token_into<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    destination: &mut Option<tex_command::CurrentCommand<G>>,
) -> Result<tex_command::DeliveryStatus, CommandError> {
    loop {
        match processor.get_x_token_into(destination)? {
            tex_command::DeliveryStatus::End => return Ok(tex_command::DeliveryStatus::End),
            tex_command::DeliveryStatus::Command => {}
            _ => unreachable!("ordinary expanded delivery returns only commands"),
        }
        let command = destination
            .as_ref()
            .expect("command status initializes destination");
        if !matches!(
            static_meaning(command.meaning()),
            Meaning::CharToken {
                cat: Catcode::Space,
                ..
            } | Meaning::Relax
        ) {
            return Ok(tex_command::DeliveryStatus::Command);
        }
        *destination = None;
    }
}

/// Writes TeX82 §406's next nonblank expanded command into the reswitch
/// destination while preserving `\relax`.
fn next_non_blank_x_token_into<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    destination: &mut Option<tex_command::CurrentCommand<G>>,
) -> Result<tex_command::DeliveryStatus, CommandError> {
    loop {
        match processor.get_x_token_into(destination)? {
            tex_command::DeliveryStatus::End => return Ok(tex_command::DeliveryStatus::End),
            tex_command::DeliveryStatus::Command => {}
            _ => unreachable!("ordinary expanded delivery returns only commands"),
        }
        let command = destination
            .as_ref()
            .expect("command status initializes destination");
        if !matches!(
            static_meaning(command.meaning()),
            Meaning::CharToken {
                cat: Catcode::Space,
                ..
            }
        ) {
            return Ok(tex_command::DeliveryStatus::Command);
        }
        *destination = None;
    }
}

#[derive(Debug)]
struct ImmediatePrint {
    sink: PrintSink,
    text: String,
    encoded: Option<Vec<u8>>,
    max_print_line: usize,
    ensure_line_start: bool,
}

#[derive(Debug)]
pub(crate) enum PreparedShipoutSource {
    Page(Node),
}

#[derive(Debug)]
pub(crate) struct PreparedShipout {
    pub(crate) source: PreparedShipoutSource,
    pub(crate) region: Option<tex_state::node_region::PageClosureBuildMark>,
}

/// The exact parent-list field that TeX82 §1153 saved before `push_math`.
///
/// The target remains in the parent mode level while ordinary main control
/// executes the braced field. Keeping this typed structural continuation lets
/// every body command cross the normal host-resource suspension seam without
/// replaying the opener or reconstructing a caller destination from command
/// order.
#[derive(Clone, Copy, Debug)]
enum ActiveMathFieldTarget {
    Nucleus {
        node_index: usize,
        simplify_accent: bool,
    },
    Script(ScriptTarget),
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
    /// Cold-bound direct handles into the immutable primitive registry.
    /// These are process-local accelerators, not TeX state or format data.
    primitive_registry_len: Option<usize>,
    pdf_ignore_depth: Option<tex_state::PrimitiveHandle<G>>,
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
    /// TeX82 §1153's `saved(0)` field destinations for live `math_group`
    /// levels, outermost first. The save stack owns scope restoration; this
    /// executor-side vector owns only the typed parent-list destination.
    active_math_fields: Vec<ActiveMathFieldTarget>,
    /// e-TeX [48.1191]'s saved delimiter identity for each live
    /// `math_left_group`, outermost first. `true` denotes `\middle`.
    active_math_left_boundaries: Vec<bool>,
    /// Live `math_shift_group` openers, outermost first.
    active_math_shifts: Vec<MathShiftContext>,
    /// Physical glue-store identity plus the canonical pointer source of the
    /// last skip-register definition. The second component is `None` when
    /// scanning allocated a fresh TeX glue node that Umber subsequently
    /// hash-consed with an equal existing node.
    skip_pointer_sources: Vec<GluePointerSource<G>>,
    /// Mu-glue counterpart of [`Self::skip_pointer_sources`]. e-TeX's
    /// `\gluetomu` and `\mutoglue` conversions retain the source pointer, so
    /// the two register banks need the same identity accounting.
    muskip_pointer_sources: Vec<GluePointerSource<G>>,
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
    /// Scalar identity of the one host-selected root main input file.
    ///
    /// This is editor/restart and startup-framing identity, not checkpoint
    /// retention policy. Boundary formation freezes the independently carried
    /// source role; token provenance and macro names never participate.
    root_main_source: Option<tex_state::SourceId>,
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
    /// A detached page has committed, but a live mode still owns page-region
    /// coordinates. Succession waits until that mode closure is consumed into
    /// PageBuilder state; it never copies or scans the mode payload.
    page_region_succession_pending: bool,
    /// Named safe boundaries committed by the last direct operation. The
    /// host drains these only after `advance` has committed, so a resource
    /// suspension never leaks a checkpoint from its rolled-back operation.
    completed_boundaries: Vec<crate::EngineBoundary>,
    /// Move-only restart eligibility produced alongside retained-role outer
    /// paragraph and shipout evidence.
    completed_checkpoint_eligibilities: Vec<crate::checkpoint::CheckpointEligibility>,
    /// The sole job-start restart capability. Restored controls do not regain
    /// it, and successful initial capture consumes it permanently.
    job_start_eligibility: Option<crate::checkpoint::CheckpointEligibility>,
    /// Ordered named-boundary intents waiting for command-owned scanner,
    /// macro, resource, and structural continuations to become quiescent.
    pending_named_boundaries: VecDeque<PendingNamedBoundary>,
    /// Source site of the most recent typed resource suspension. This is
    /// retained outside snapshots so a host protocol no-progress invariant
    /// can still identify the command whose retry failed to advance.
    pending_resource_site: Option<OriginId>,
    /// Exact direct-operation capability and caller destination retained
    /// across a typed delivery retry. Preflight has already committed its
    /// delivery, so retry resumes without reconstructing owner coordinates
    /// from command state or inspecting unrelated pending slots.
    pending_direct_operation: Option<PendingDirectOperation<G>>,
    /// Fully scanned resource operation retained across host acquisition.
    /// The command/input cursor is already committed, so retry resolves this
    /// typed operand and never replays delivery or scanning work.
    pending_resource_operation: Option<PendingResourceOperation<G>>,
    /// Diagnostic-host assignment retained after expanded delivery has
    /// committed. Retrying resumes either its exact settled command/cursor or
    /// its fully scanned operation without fetching another diagnostic token.
    pending_diagnostic_operation: Option<PendingDiagnosticOperation<G>>,
    /// Terminal step armed only by [`crate::CanonicalStepRunner`] after the
    /// final operation and all named-boundary publication have committed.
    /// Once armed, ordinary execution cannot continue; output detachment
    /// consumes the corresponding ledger receipt and closes this state.
    terminal_revision_step: Option<MainControlStep>,
    terminal_revision_closed: bool,
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

#[derive(Debug)]
struct PendingSetBox {
    target: SetBoxTarget,
    region: tex_state::node_region::PageClosureBuildMark,
}

#[derive(Debug)]
struct ActiveReplayBox {
    target: Option<PendingSetBox>,
    shipout_region: Option<tex_state::node_region::PageClosureBuildMark>,
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
    /// Scalar lifecycle evidence for the rows and cells whose material has
    /// already moved into the alignment level. The page arena is the sole
    /// owner of that material; diagnostics must not retain a second root
    /// topology beside TeX82 §775's alignment list.
    captured_row_count: usize,
    captured_cell_count: usize,
    tabskips: Vec<tex_state::glue::GlueSpec>,
    default_tabskip: tex_state::glue::GlueSpec,
    /// TeX82 §786's `cur_head`/`cur_tail` holding list: the insertions, marks,
    /// and `\vadjust` contents §796's `hpack` migrated out of this row's
    /// columns, waiting for §799 `fin_row` to append them after the row.
    row_migrations: tex_state::page_node_arena::PageListSpan,
    cell_span: u16,
    row_open: bool,
    cell_open: bool,
}

#[derive(Debug)]
struct ReplayBoxes<G> {
    pending_setbox: Option<PendingSetBox>,
    pending_shipout: Option<tex_state::node_region::PageClosureBuildMark>,
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
            pending_shipout: None,
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

impl<G> ReplayBoxes<G> {
    fn format_dump_is_quiescent(&self) -> bool {
        self.pending_setbox.is_none()
            && self.pending_shipout.is_none()
            && self.pending_leader.is_none()
            && self.active_boxes.is_empty()
            && self.suspended_alignments.is_empty()
            && !self.recovery_simple_group_pending
            && !self.recovery_simple_group_open
            && !self.output_routine_active
            && !self.output_routine_opening_pending
    }
}

#[derive(Clone, Debug)]
struct ActiveDiscretionary {
    parts: Vec<tex_state::page_node_arena::PageListSpan>,
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
    host_facts: CommandMachineHostFacts<'a, G>,
    observations: &'a mut ObservationSlot,
    assignment_receipts: Option<&'a mut Vec<MutationRecord>>,
    diagnostic_effects: &'a mut DiagnosticEffects,
    shown_mode: &'a mut Option<Mode>,
    /// tex.web's `init`/`tini` compile-time split, which Umber carries as a
    /// session flag: §1252's `\patterns` and §1335's `\dump` are the two
    /// commands whose whole behavior it selects.
    initex: bool,
    emit_dvi_override: Option<bool>,
    immediate_prints: &'a mut Vec<ImmediatePrint>,
    prepared_shipout: &'a mut Option<PreparedShipout>,
    pending_show_completion: Option<PendingShowCompletion>,
    pending_outer_page_build_context: Option<String>,
    output_routine_active: bool,
}

enum CommandMachineHostFacts<'a, G> {
    Live(ExecutorHostFacts<'a, G>),
    /// Semantic application can perform source-only command operations while
    /// mutating the mode nest. A nested expansion explicitly lends that nest
    /// through `with_processor_for_modes`, so no stale projection is cached.
    Detached {
        pdf_ignore_depth: Option<tex_state::PrimitiveHandle<G>>,
        telemetry: &'a mut crate::EpisodeTelemetry,
    },
    /// Hot semantic application cannot scan or expand. Keeping that boundary
    /// explicit lets it borrow the mode nest mutably without manufacturing a
    /// stale projection merely to fill an unused provider.
    Forbidden,
}

impl<G> tex_command::CommandHostFacts<G> for CommandMachineHostFacts<'_, G> {
    fn conditional_state(&mut self) -> tex_command::ConditionalState {
        match self {
            Self::Live(facts) => tex_command::CommandHostFacts::conditional_state(facts),
            Self::Detached { .. } => {
                panic!("nested expansion must lend its live mode facts")
            }
            Self::Forbidden => panic!("semantic application cannot query command host facts"),
        }
    }

    fn space_factor(&mut self) -> Option<i32> {
        match self {
            Self::Live(facts) => tex_command::CommandHostFacts::space_factor(facts),
            Self::Detached { .. } => {
                panic!("nested expansion must lend its live mode facts")
            }
            Self::Forbidden => panic!("semantic application cannot query command host facts"),
        }
    }

    fn prev_depth(&mut self, state: &CommandContext<'_, G>) -> Option<Scaled> {
        match self {
            Self::Live(facts) => tex_command::CommandHostFacts::prev_depth(facts, state),
            Self::Detached { .. } => {
                panic!("nested expansion must lend its live mode facts")
            }
            Self::Forbidden => panic!("semantic application cannot query command host facts"),
        }
    }

    fn prev_graf(&mut self) -> Option<i32> {
        match self {
            Self::Live(facts) => tex_command::CommandHostFacts::prev_graf(facts),
            Self::Detached { .. } => {
                panic!("nested expansion must lend its live mode facts")
            }
            Self::Forbidden => panic!("semantic application cannot query command host facts"),
        }
    }

    fn last_node(&mut self, state: &CommandContext<'_, G>) -> Option<tex_command::LastNodeItem> {
        match self {
            Self::Live(facts) => tex_command::CommandHostFacts::last_node(facts, state),
            Self::Detached { .. } => {
                panic!("nested expansion must lend its live mode facts")
            }
            Self::Forbidden => panic!("semantic application cannot query command host facts"),
        }
    }

    fn last_node_type(&mut self, state: &CommandContext<'_, G>) -> i32 {
        match self {
            Self::Live(facts) => tex_command::CommandHostFacts::last_node_type(facts, state),
            Self::Detached { .. } => {
                panic!("nested expansion must lend its live mode facts")
            }
            Self::Forbidden => panic!("semantic application cannot query command host facts"),
        }
    }
}

struct PendingShowCompletion {
    long: bool,
    context: String,
}

impl<G> CommandMachine<'_, G> {
    fn with_processor_for_modes<R>(
        &mut self,
        context: &mut tex_state::CommandContext<'_, G>,
        modes: &ModeNest,
        use_processor: impl FnOnce(&mut InterpreterProcessor<'_, '_, G>) -> R,
    ) -> R {
        let Self {
            state,
            fuel,
            capabilities,
            host_facts,
            observations,
            diagnostic_effects,
            output_routine_active,
            ..
        } = self;
        match host_facts {
            CommandMachineHostFacts::Live(facts) => {
                let observer = observations
                    .as_mut()
                    .map(|buffer| buffer as &mut dyn CommandObserver);
                let mut processor = state.processor(
                    context,
                    CommandHostContext::with_facts(capabilities, facts),
                    fuel,
                    observer,
                    diagnostic_effects,
                );
                processor.set_output_routine_active(*output_routine_active);
                use_processor(&mut processor)
            }
            CommandMachineHostFacts::Detached {
                pdf_ignore_depth,
                telemetry,
            } => {
                let observer = observations
                    .as_mut()
                    .map(|buffer| buffer as &mut dyn CommandObserver);
                let mut facts = ExecutorHostFacts {
                    modes,
                    pdf_ignore_depth: *pdf_ignore_depth,
                    telemetry,
                };
                let mut processor = state.processor(
                    context,
                    CommandHostContext::with_facts(capabilities, &mut facts),
                    fuel,
                    observer,
                    diagnostic_effects,
                );
                processor.set_output_routine_active(*output_routine_active);
                use_processor(&mut processor)
            }
            CommandMachineHostFacts::Forbidden => {
                panic!("hot semantic application cannot construct a processor")
            }
        }
    }

    fn with_processor_for_modes_and_diagnostics<R>(
        &mut self,
        context: &mut tex_state::CommandContext<'_, G>,
        modes: &ModeNest,
        diagnostic_effects: &mut DiagnosticEffects,
        use_processor: impl FnOnce(&mut InterpreterProcessor<'_, '_, G>) -> R,
    ) -> R {
        let Self {
            state,
            fuel,
            capabilities,
            host_facts,
            observations,
            output_routine_active,
            ..
        } = self;
        match host_facts {
            CommandMachineHostFacts::Live(facts) => {
                let observer = observations
                    .as_mut()
                    .map(|buffer| buffer as &mut dyn CommandObserver);
                let mut processor = state.processor(
                    context,
                    CommandHostContext::with_facts(capabilities, facts),
                    fuel,
                    observer,
                    diagnostic_effects,
                );
                processor.set_output_routine_active(*output_routine_active);
                use_processor(&mut processor)
            }
            CommandMachineHostFacts::Detached {
                pdf_ignore_depth,
                telemetry,
            } => {
                let observer = observations
                    .as_mut()
                    .map(|buffer| buffer as &mut dyn CommandObserver);
                let mut facts = ExecutorHostFacts {
                    modes,
                    pdf_ignore_depth: *pdf_ignore_depth,
                    telemetry,
                };
                let mut processor = state.processor(
                    context,
                    CommandHostContext::with_facts(capabilities, &mut facts),
                    fuel,
                    observer,
                    diagnostic_effects,
                );
                processor.set_output_routine_active(*output_routine_active);
                use_processor(&mut processor)
            }
            CommandMachineHostFacts::Forbidden => {
                panic!("hot semantic application cannot construct a processor")
            }
        }
    }

    fn publish_named_token_list_pushes(&mut self, context: &mut tex_state::CommandContext<'_, G>) {
        let observer = self
            .observations
            .as_mut()
            .map(|buffer| buffer as &mut dyn CommandObserver);
        self.state
            .publish_named_token_list_pushes(context, self.diagnostic_effects, observer);
    }

    fn defer_show_completion(&mut self, long: bool, context: String) {
        assert!(
            self.pending_show_completion.is_none(),
            "one command operation completes at most one show diagnostic"
        );
        self.pending_show_completion = Some(PendingShowCompletion { long, context });
    }

    fn processor<'episode, 'admission>(
        &'episode mut self,
        context: &'episode mut tex_state::CommandContext<'admission, G>,
    ) -> InterpreterProcessor<'episode, 'admission, G> {
        let observer = self
            .observations
            .as_mut()
            .map(|buffer| buffer as &mut dyn CommandObserver);
        let mut processor = self.state.processor(
            context,
            CommandHostContext::with_facts(self.capabilities, &mut self.host_facts),
            self.fuel,
            observer,
            self.diagnostic_effects,
        );
        processor.set_output_routine_active(self.output_routine_active);
        processor
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

    /// Retains one semantic record at the current hot-operation commit seam.
    ///
    /// Definition scanning records the replacement text through the command
    /// processor. e-TeX then installs its protected-macro marker while the
    /// hot operation owns the live definition, so that marker's canonical
    /// token-list transition belongs immediately before the meaning mutation.
    fn retain_hot_observation(&mut self, observation: CommandObservation) {
        if let Some(observations) = self.observations.as_mut() {
            observations.committed(observation);
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
    host_facts: &'episode mut dyn tex_command::CommandHostFacts<G>,
    observations: &'episode mut ObservationSlot,
    diagnostic_effects: &'episode mut DiagnosticEffects,
    stores: &'episode mut CommandContext<'admission, G>,
) -> InterpreterProcessor<'episode, 'admission, G> {
    let observer = observations
        .as_mut()
        .map(|buffer| buffer as &mut dyn CommandObserver);
    command.processor(
        stores,
        CommandHostContext::with_facts(capabilities, host_facts),
        fuel,
        observer,
        diagnostic_effects,
    )
}

fn publish_named_token_list_pushes<G>(
    command: &mut PersistentInterpreter<G>,
    context: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    observations: &mut ObservationSlot,
) {
    let observer = observations
        .as_mut()
        .map(|buffer| buffer as &mut dyn CommandObserver);
    command.publish_named_token_list_pushes(context, diagnostic_effects, observer);
}

impl<G> Default for MainControl<G> {
    fn default() -> Self {
        Self {
            command: PersistentInterpreter::default(),
            pure_memo: Arc::default(),
            pure_memo_initialized: false,
            fuel: tex_command::CommandFuelLedger::default(),
            capabilities: CommandHostCapabilities::default(),
            primitive_registry_len: None,
            pdf_ignore_depth: None,
            emit_dvi_override: None,
            modes: ModeNest::default(),
            max_save_stack: 0,
            next_alignment_identity: 0,
            active_alignment: None,
            boxes: ReplayBoxes::default(),
            active_discretionaries: Vec::new(),
            active_math_choices: Vec::new(),
            active_math_fields: Vec::new(),
            active_math_left_boundaries: Vec::new(),
            active_math_shifts: Vec::new(),
            skip_pointer_sources: Vec::new(),
            muskip_pointer_sources: Vec::new(),
            main_loop_active: false,
            set_box_forbidden_depth: 0,
            shown_mode: None,
            main_control_entered: false,
            initex: false,
            preloaded_format: None,
            engine_binary: None,
            dumped_format: None,
            startup_terminal_line: String::new(),
            terminal_line_was_empty: None,
            root_completion: RootCompletionPolicy::default(),
            root_main_source: None,
            page_output_observations: ObservationBuffer::default(),
            operation_observations: None,
            operation_receipt_start: None,
            suspended_operation_observation: None,
            completed_replay_episode: None,
            prepared_dvi_pages: PreparedDviPages::default(),
            immediate_prints: Vec::new(),
            prepared_shipout: None,
            page_region_succession_pending: false,
            completed_boundaries: Vec::new(),
            completed_checkpoint_eligibilities: Vec::new(),
            job_start_eligibility: Some(crate::checkpoint::CheckpointEligibility::job_start()),
            pending_named_boundaries: VecDeque::new(),
            pending_resource_site: None,
            pending_direct_operation: None,
            pending_resource_operation: None,
            pending_diagnostic_operation: None,
            terminal_revision_step: None,
            terminal_revision_closed: false,
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
    /// Preflights the executor half of production page succession. The state
    /// half consumes the move-only receipt and carries its complete PageBuilder
    /// owner; no raw page root crosses this aggregate boundary.
    pub(crate) fn prepare_page_region_succession(
        &self,
        stores: &mut Universe<G>,
    ) -> Result<(), tex_state::UniverseError> {
        let modes = {
            let context = stores.command_context()?;
            self.modes
                .preflight_page_region_succession(&context)
                .ok_or(tex_state::UniverseError::State(
                    tex_state::StateError::InvalidCursor,
                ))?
        };
        stores.prepare_page_region_after_output(modes)
    }

    fn finish_pending_page_region_succession(&mut self, stores: &mut Universe<G>) {
        if !self.page_region_succession_pending || self.modes.retains_page_node_handles() {
            return;
        }
        self.prepare_page_region_succession(stores)
            .expect("rootless mode lists admit the current page region");
        stores
            .commit_page_region_after_output()
            .expect("prepared page succession commits");
        self.page_region_succession_pending = false;
    }

    pub(crate) fn from_checkpoint_fork(command: CommandState<G>, modes: ModeNest) -> Self {
        Self {
            command: PersistentInterpreter::from_state(command),
            modes,
            job_start_eligibility: None,
            ..Self::default()
        }
    }

    /// Extracts the moved command/mode owner for the aggregate settlement
    /// barrier. Destination owners must settle before this receipt commits or
    /// returns the source-side mode ranges.
    #[doc(hidden)]
    pub(crate) fn into_checkpoint_candidate_parts(
        self,
    ) -> (CommandState<G>, PreparedCheckpointControl) {
        #[cfg(feature = "profiling")]
        let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::GenerationBoundary,
        );
        let Self { command, modes, .. } = self;
        let command = command.into_state();
        (command, PreparedCheckpointControl { modes })
    }

    /// Parks a quiescent command owner from an independently materialized
    /// JobStart lane. No mode or command fork exists in this case, so there is
    /// deliberately no settlement receipt: retained checkpoints already own
    /// their rootless mode summaries and the live terminal mode nest can drop.
    pub(crate) fn into_independent_parked_command(self) -> CommandState<G> {
        debug_assert!(self.command.named_boundary_is_quiescent());
        self.command.into_state()
    }

    /// Convenience barrier used by owner-local tests. Aggregate callers use
    /// [`Self::prepare_checkpoint_candidate`] so destination owners settle
    /// first.
    pub fn accept_checkpoint_candidate(self) {
        let (mut command, control) = self.into_checkpoint_candidate_parts();
        control.accept();
        command.accept_checkpoint_candidate();
    }

    /// Returns command and mode roots through their rejection paths before
    /// aggregate state rejection. Consuming `self` prevents later use of a
    /// partially settled command machine.
    pub fn reject_checkpoint_candidate(self) {
        let (mut command, control) = self.into_checkpoint_candidate_parts();
        control.reject();
        command.reject_checkpoint_candidate();
    }

    /// Settles a quiescent command/mode candidate while returning the live
    /// control owner. The consuming transition cannot run twice.
    pub(crate) fn into_accepted_checkpoint_candidate(mut self) -> Self {
        #[cfg(feature = "profiling")]
        let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::GenerationBoundary,
        );
        self.modes.accept_checkpoint_candidate_in_place();
        self.command.state_mut().accept_checkpoint_candidate();
        self
    }

    /// Discards any candidate-only direct-operation continuation before the
    /// command timeline returns to its accepted checkpoint lineage.
    ///
    /// Resource suspension deliberately keeps the sole command-attempt owner
    /// live while committing the state and mode operation journals. Aggregate
    /// rejection must therefore return the moved attempt arena and roll back
    /// that exact operation before rejecting the command checkpoint fork.
    pub(crate) fn cancel_external_attempt_for_checkpoint_settlement(
        &mut self,
        stores: &Universe<G>,
    ) {
        let operation = if let Some(pending) = self.pending_resource_operation.take() {
            let (operation, resume, _pending) = self
                .command
                .resume_attempt(stores, pending.attempt)
                .unwrap_or_else(|_| {
                    panic!("resource continuation belongs to the rejected generation")
                });
            debug_assert_eq!(resume, SUSPENDED_RESOURCE_RESUME);
            Some(operation)
        } else if let Some(pending) = self.pending_direct_operation.take() {
            match pending.state {
                PendingDirectState::Fresh => None,
                PendingDirectState::Retained(operation) => Some(operation),
            }
        } else {
            self.pending_diagnostic_operation
                .take()
                .map(|pending| pending.operation)
        };
        if let Some(operation) = operation {
            self.command
                .rollback_attempt_operation(operation)
                .expect("rejected continuation owns its command-attempt operation");
        }
    }

    pub(crate) fn into_rejected_checkpoint_command_with_state(
        mut self,
        stores: &Universe<G>,
    ) -> CommandState<G> {
        self.cancel_external_attempt_for_checkpoint_settlement(stores);
        let (mut command, control) = self.into_checkpoint_candidate_parts();
        control.reject();
        command.reject_checkpoint_candidate();
        command
    }

    pub(crate) fn arm_terminal_revision(&mut self, step: MainControlStep) {
        debug_assert!(matches!(
            step,
            MainControlStep::End | MainControlStep::EndOfInput
        ));
        debug_assert!(!self.terminal_revision_closed);
        debug_assert!(self.terminal_revision_step.is_none());
        self.terminal_revision_step = Some(step);
    }

    pub(crate) fn terminal_revision_is_quiescent(&self, step: MainControlStep) -> bool {
        self.terminal_revision_step == Some(step)
            && !self.terminal_revision_closed
            && self.command.named_boundary_is_quiescent()
            && !self.has_external_attempt_owner()
            && self.active_alignment.is_none()
            && self.operation_observations.is_none()
            && self.operation_receipt_start.is_none()
            && self.suspended_operation_observation.is_none()
            && self.prepared_shipout.is_none()
            && !self.page_region_succession_pending
            && self.immediate_prints.is_empty()
            && self.pending_named_boundaries.is_empty()
            && self.pending_resource_site.is_none()
            && self.pending_direct_operation.is_none()
    }

    pub(crate) fn close_terminal_revision(&mut self, step: MainControlStep) {
        debug_assert!(self.terminal_revision_is_quiescent(step));
        self.terminal_revision_closed = true;
    }

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
        self.ensure_primitive_handles(stores);
        let mut diagnostic_effects = DiagnosticEffects::new();
        let mut command_context = stores.command_context().expect("live generation");
        let mut host_facts = ExecutorHostFacts {
            modes: &self.modes,
            pdf_ignore_depth: self.pdf_ignore_depth,
            telemetry: &mut self.episode_telemetry,
        };
        let processor = command_processor(
            &mut self.command,
            self.fuel.fuel_mut(),
            &mut self.capabilities,
            &mut host_facts,
            &mut self.operation_observations,
            &mut diagnostic_effects,
            &mut command_context,
        );
        let context = processor.error_context();
        processor.retire();
        let result = crate::error_report::report_error(
            &mut command_context,
            &mut diagnostic_effects,
            "Interruption",
            &[
                "You rang?",
                "Try to insert some instructions for me (e.g., `I\\showlists'),",
                "unless you just want to quit by typing `X'.",
            ],
            context,
        );
        drop(command_context);
        stores
            .world_mut()
            .publish_diagnostic_effects(diagnostic_effects);
        result
    }

    fn local_glue_pointer_reassigned<T, D>(
        context: &CommandContext<'_, G>,
        scanned: &ColdOperation<G, T, D>,
        skip_pointer_sources: &[GluePointerSource<G>],
        muskip_pointer_sources: &[GluePointerSource<G>],
    ) -> bool {
        let (index, value, source_identity, source_is_target, physical, pointer_sources) =
            match scanned {
                ColdOperation::Skip {
                    index,
                    value,
                    source_identity,
                    source_register,
                    global: false,
                    ..
                } => (
                    *index,
                    value,
                    source_identity,
                    *source_register == Some((false, *index)),
                    match context.glue_register(*index).ok().flatten() {
                        Some(physical) => physical,
                        None => return false,
                    },
                    skip_pointer_sources,
                ),
                ColdOperation::Muskip {
                    index,
                    value,
                    source_identity,
                    source_register,
                    global: false,
                    ..
                } => (
                    *index,
                    value,
                    source_identity,
                    *source_register == Some((true, *index)),
                    match context.muskip(*index) {
                        Some(physical) => physical,
                        None => return false,
                    },
                    muskip_pointer_sources,
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
        context: &CommandContext<'_, G>,
        scanned: &ColdOperation<G, T, D>,
        skip_pointer_sources: &[GluePointerSource<G>],
        muskip_pointer_sources: &[GluePointerSource<G>],
    ) -> bool {
        context.int_param(IntParam::ETEX_EXTENDED_MODE) > 0
            && Self::local_glue_pointer_reassigned(
                context,
                scanned,
                skip_pointer_sources,
                muskip_pointer_sources,
            )
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
        let mut control = Self {
            command: PersistentInterpreter::new(CommandProfile::TEX82),
            next_alignment_identity: 1,
            initex: true,
            ..Self::default()
        };
        control.bind_primitive_handles(stores);
        control
    }

    /// Binds process-local accelerators after INITEX installation or format
    /// registry reconstruction has installed the complete driver profile.
    pub fn bind_primitive_handles(&mut self, stores: &Universe<G>) {
        self.primitive_registry_len = Some(stores.primitive_registry_len());
        self.pdf_ignore_depth = stores.primitive_handle("pdfignoreddimen");
    }

    fn ensure_primitive_handles(&mut self, stores: &Universe<G>) {
        if self.primitive_registry_len != Some(stores.primitive_registry_len()) {
            self.bind_primitive_handles(stores);
        }
    }

    /// Borrows command state for source registration and snapshots.
    #[must_use]
    pub fn command_mut(&mut self) -> &mut CommandState<G> {
        self.command.state_mut()
    }

    /// Demand-free command checkpoint ownership and settlement work.
    #[doc(hidden)]
    #[must_use]
    pub fn command_timeline_counters(&self) -> tex_command::CommandTimelineCounters {
        self.command.state().profile_timeline_counters()
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

    /// Captures and consumes one successful quiescent INITEX dump transition.
    ///
    /// The state image is built before the receipt is taken, so every
    /// rejection is mutation-free and a successful transition is observable
    /// exactly once.
    pub fn take_format_dump(
        &mut self,
        stores: &Universe<G>,
    ) -> Result<Option<crate::DetachedFormatDump>, crate::FormatDumpError> {
        if self.dumped_format.is_none() {
            return Ok(None);
        }
        if !self.command.format_dump_is_quiescent() {
            return Err(crate::FormatDumpError::LiveCommandState);
        }
        if self.has_external_attempt_owner()
            || self.active_alignment.is_some()
            || !self.boxes.format_dump_is_quiescent()
            || !self.active_discretionaries.is_empty()
            || !self.active_math_choices.is_empty()
            || !self.active_math_fields.is_empty()
            || !self.active_math_left_boundaries.is_empty()
            || !self.active_math_shifts.is_empty()
            || self.main_loop_active
            || self.set_box_forbidden_depth != 0
            || self.operation_observations.is_some()
            || self.operation_receipt_start.is_some()
            || self.suspended_operation_observation.is_some()
            || self.prepared_shipout.is_some()
            || self.page_region_succession_pending
            || !self.prepared_dvi_pages.is_empty()
            || !self.immediate_prints.is_empty()
            || self.pending_resource_site.is_some()
            || self.pending_direct_operation.is_some()
        {
            return Err(crate::FormatDumpError::LiveExecutorState);
        }
        if self.modes.depth() != 1
            || self.modes.current_mode() != Mode::Vertical
            || !self.modes.current_list().is_empty()
        {
            return Err(crate::FormatDumpError::LiveModeState);
        }
        let image = stores
            .capture_format_image()
            .map_err(crate::FormatDumpError::State)?;
        self.command.close_format_dump_boundary();
        let receipt = self
            .dumped_format
            .take()
            .expect("successful dump capture retains its receipt");
        Ok(Some(crate::DetachedFormatDump { image, receipt }))
    }

    /// Consumes this fresh command processor's sole job-start receipt.
    pub fn capture_job_start_checkpoint(
        &mut self,
        stores: &mut Universe<G>,
        budget_counters: crate::ExecutionBudgetCounters,
    ) -> Result<crate::EngineCheckpoint<G>, tex_command::CommandSummaryError> {
        if self.has_external_attempt_owner() {
            return Err(tex_command::CommandSummaryError::AttemptSuspended);
        }
        let eligibility = self
            .take_job_start_eligibility()
            .ok_or(tex_command::CommandSummaryError::AttemptSuspended)?;
        let result = crate::EngineCheckpoint::capture_checkpoint(
            eligibility,
            &mut self.command,
            &mut self.modes,
            stores,
            budget_counters,
        );
        if result.is_err() {
            self.job_start_eligibility =
                Some(crate::checkpoint::CheckpointEligibility::job_start());
        }
        result
    }

    pub(crate) fn capture_checkpoint_with_identity_demand(
        &mut self,
        eligibility: crate::checkpoint::CheckpointEligibility,
        stores: &mut Universe<G>,
        budget_counters: crate::ExecutionBudgetCounters,
        wants_reachable_state_identity: bool,
    ) -> Result<crate::EngineCheckpoint<G>, tex_command::CommandSummaryError> {
        if self.has_external_attempt_owner() {
            return Err(tex_command::CommandSummaryError::AttemptSuspended);
        }
        crate::EngineCheckpoint::capture_checkpoint_with_identity_demand(
            eligibility,
            &mut self.command,
            &mut self.modes,
            stores,
            budget_counters,
            wants_reachable_state_identity,
        )
    }

    #[cfg(test)]
    pub(crate) fn capture_checkpoint(
        &mut self,
        boundary: crate::EngineBoundary,
        stores: &mut Universe<G>,
        budget_counters: crate::ExecutionBudgetCounters,
    ) -> Result<crate::EngineCheckpoint<G>, tex_command::CommandSummaryError> {
        let eligibility = match boundary {
            crate::EngineBoundary::JobStart => {
                crate::checkpoint::CheckpointEligibility::job_start()
            }
            crate::EngineBoundary::OuterParagraphEnd => {
                crate::checkpoint::CheckpointEligibility::named(boundary)
            }
            crate::EngineBoundary::ShipoutComplete => {
                crate::checkpoint::CheckpointEligibility::named(boundary)
            }
        };
        self.capture_checkpoint_with_identity_demand(eligibility, stores, budget_counters, false)
    }

    /// Selects maintained convergence identity before an incremental session
    /// begins ordinary execution.
    #[doc(hidden)]
    pub fn enable_reachable_state_identity(&mut self, stores: &mut Universe<G>) {
        let _ = self.command.enable_reachable_state_identity();
        self.modes.enable_reachable_state_identity();
        stores.enable_reachable_state_identity();
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
        self.pending_named_boundaries.clear();
        self.fatal = None;
        self.captured_fatal_origin = None;
        Ok(())
    }

    /// Reports command-operation owners retained by executor continuations.
    /// These owners must be rejected before a named checkpoint asks command
    /// state to census and reclaim its roots. A prepared resource continuation
    /// moves out the complete attempt; direct and diagnostic continuations
    /// leave the arena installed but move its non-`Copy` operation capability
    /// together with the exact caller destination. Command state's validating
    /// coordinate cannot reconstruct that caller owner.
    fn has_external_attempt_owner(&self) -> bool {
        self.pending_resource_operation.is_some()
            || self.pending_diagnostic_operation.is_some()
            || self.pending_direct_operation.is_some()
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
        stores.set_engine_capacity_profile(binary.capacity_profile());
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

    /// Records and prints a retained root whose one-shot job framing has
    /// already opened the transcript.
    pub fn open_startup_input_after_log(&mut self, stores: &mut Universe<G>, name: &str) {
        crate::job::open_startup_input_after_log(stores, name);
    }

    /// Declares that this session was restored from a dumped format, so
    /// [`Self::begin_job`] frames it as `-fmt=<name>` rather than as INITEX.
    ///
    /// Framing only: nothing about execution changes, and a session that
    /// never calls this is framed exactly as before.
    pub fn set_preloaded_format(&mut self, format: crate::job::PreloadedFormat) {
        self.preloaded_format = Some(format);
    }

    /// Restores the host-selected INITEX framing bit after an aggregate
    /// `JobStart` checkpoint is forked into a new revision generation.
    /// Command and runtime semantics come from the checkpoint; this bit is
    /// operational job framing and is deliberately absent from it.
    pub fn set_initex_mode(&mut self, initex: bool) {
        debug_assert!(self.startup_terminal_line.is_empty());
        debug_assert!(self.root_main_source.is_none());
        self.initex = initex;
    }

    /// Selects the engine binary identity used by startup framing and shared
    /// compiled command semantics.
    pub fn set_engine_binary(&mut self, binary: crate::job::EngineBinaryIdentity) {
        self.command
            .set_engine_semantics(binary.command_semantics());
        self.engine_binary = Some(binary);
    }

    /// Installs the executable-owned immutable store capacities before a
    /// pre-job aggregate checkpoint is captured. Normal startup repeats this
    /// selection idempotently when it frames the root input.
    pub fn prepare_job_start_stores(&self, stores: &mut Universe<G>) {
        let binary = self.engine_binary.unwrap_or_else(|| {
            crate::job::EngineBinaryIdentity::for_profile(self.command_profile())
        });
        stores.set_engine_capacity_profile(binary.capacity_profile());
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
        let mut usage = stores
            .command_context()
            .expect("job usage admission")
            .detach_engine_usage_statistics();
        let command_usage = self.command.stack_usage();
        usage.input_stack = command_usage.input_stack;
        usage.nest_stack = self.modes.maximum_saved_depth();
        usage.parameter_stack = command_usage.parameter_stack;
        usage.buffer_stack = command_usage.buffer_stack;
        // TeX82 §1334 prints `max_save_stack+6`; §273's conservative
        // check reserves room for six subsequent unchecked words.
        usage.save_stack = self.max_save_stack.saturating_add(6);
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
        scanned: &mut ColdOperation<G>,
        stores: &mut Universe<G>,
    ) -> Result<(), ExecError> {
        let ColdOperation::<G>::FontDefinition { request, .. } = scanned else {
            return Ok(());
        };
        stores.poison_dependency_region(TrackedRegionBarrier::UnsupportedHostCapability);
        let path = crate::canonical_font_resource_path(&request.name);
        if self.capabilities.font(&path).is_none() {
            return Err(ExecError::MissingFont {
                request: request.clone(),
            });
        }
        Ok(())
    }

    fn resolve_input_stream_resource(
        &self,
        scanned: &mut ColdOperation<G>,
        stores: &mut Universe<G>,
    ) -> Result<(), ExecError> {
        let ColdOperation::<G>::InputStream { request, resource } = scanned else {
            return Ok(());
        };
        stores.poison_dependency_region(TrackedRegionBarrier::UnsupportedHostCapability);
        let resolved_source = match request {
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
                        return Err(ExecError::MissingInputProbe {
                            request: error_request,
                        });
                    }
                }
            }
            RootedInputStreamRequest::Close { .. } | RootedInputStreamRequest::Read { .. } => None,
        };
        *resource = resolved_source;
        Ok(())
    }

    fn resolve_pdf_image_resource(
        &self,
        scanned: &mut ColdOperation<G>,
        stores: &mut Universe<G>,
    ) -> Result<(), ExecError> {
        let ColdOperation::<G>::PdfXImage { request, resource } = scanned else {
            return Ok(());
        };
        stores.poison_dependency_region(TrackedRegionBarrier::UnsupportedHostCapability);
        // pdfTeX checks \pdfoutput before it enters `scan_image`; in DVI
        // mode this must be the diagnostic, not a host-resource suspension.
        let mut context = stores.command_context().expect("live generation");
        if context.int_param(IntParam::PDF_OUTPUT) <= 0 {
            *resource = PdfImageResource::Unavailable;
            return Ok(());
        }
        apply_pdf_image_compatibility_policy(&mut context);
        request.page_box = pdf_image_page_box(&context, request);
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
            attr: request.attr.as_ref().map(|root| {
                root.attempt_id()
                    .expect("PDF image resource resolution precedes root preparation")
            }),
        };
        let Some(resolved_resource) = self.capabilities.pdf_image(&host_request) else {
            return Err(ExecError::MissingPdfImage {
                request: host_request,
            });
        };
        *resource = resolved_resource;
        Ok(())
    }

    /// Registers and opens the one root source selected by the host before
    /// main control starts.  Source acquisition is deliberately
    /// complete before this call: the command state retains only immutable
    /// bytes and never reaches back into a host input stack.
    pub fn register_root_source(
        &mut self,
        source: SourceRegistration,
    ) -> Result<tex_state::SourceId, SourceRegistrationError> {
        debug_assert!(
            self.root_main_source.is_none(),
            "one MainControl has exactly one root main source"
        );
        let source = if source.role().is_some() {
            source
        } else {
            source.with_role(tex_command::SourceRole::RootDocument)
        };
        let id = self.command.register_source(source)?;
        // `id` was just allocated by this command state, so this can fail
        // only if the state implementation has violated its own invariant.
        self.command
            .open_registered_source(id)
            .expect("freshly registered source must be openable");
        self.root_main_source = Some(id);
        Ok(id)
    }

    /// Returns the physical root row owned by command input and therefore
    /// restorable by a named checkpoint. Compact source context can outlive
    /// that row for diagnostics, so it is not sufficient for restart.
    fn restartable_root_source_identity(&self) -> Option<tex_state::SourceId> {
        self.command.live_physical_root_source_id()
    }

    /// Substitutes the edited root buffer after an aggregate checkpoint fork.
    /// The command input owner journals the old backing; this scalar root id
    /// changes only in the candidate's restored MainControl.
    #[doc(hidden)]
    pub fn rebind_root_source_for_editor(
        &mut self,
        bytes: std::sync::Arc<[u8]>,
        unchanged_prefix: usize,
    ) -> Result<(), SourceRegistrationError> {
        let accepted = self
            .restartable_root_source_identity()
            .expect("a rooted checkpoint retains its main source identity");
        let current = self
            .command
            .rebind_generated_source(accepted, bytes, unchanged_prefix)?;
        self.root_main_source = Some(current);
        Ok(())
    }

    /// Renders the registered root's §537 opening at the driver's startup
    /// boundary without advancing input.
    pub fn open_registered_root_framing(&mut self, stores: &mut Universe<G>) {
        let source = self
            .root_main_source
            .expect("root framing requires a registered root source");
        let Some(name) = self.command.live_file_framing_name(source) else {
            return;
        };
        stores
            .command_context()
            .expect("live generation")
            .print_file_open(name);
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
        let components = tex_command::FileNameComponents::from_tex_name(startup_name);
        let mut context = stores
            .command_context()
            .expect("startup accounting requires a live generation");
        for component in [&components.area, &components.name, &components.extension] {
            if !component.is_empty() {
                context.slow_make_string_pool_string(component);
            }
        }
        drop(context);
        let id = self.register_root_source(source)?;
        if has_resolved_name {
            self.open_registered_root_framing(stores);
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
        if self.initex {
            let components = tex_command::FileNameComponents::from_tex_name(requested_name);
            let stem = components.name;
            // §§534--536 retain the startup name component as `job_name`
            // and the transcript's opened name before §537 retains the
            // requested and host-resolved input names below.
            let mut context = stores
                .command_context()
                .expect("startup accounting requires a live generation");
            if !stem.is_empty() {
                context.make_string_pool_string(&stem);
                context.make_string_pool_string(&format!("{stem}.log"));
            }
        }
        stores
            .command_context()
            .expect("startup accounting requires a live generation")
            .make_string_pool_string(requested_name);
        if let Some(resolved_name) = resolved_name
            && resolved_name != requested_name
        {
            stores
                .command_context()
                .expect("startup accounting requires a live generation")
                .make_string_pool_string(resolved_name);
        }
    }

    /// Prepares executor-owned mode facts for one topology-stable operation.
    ///
    /// The returned scope proves that delivery, scanning, and application use
    /// the same effective-tail projection. It cannot enter an operation frame
    /// or a suspension, so a cold re-entry prepares from authoritative state
    /// again.
    /// Lends the whole command machine at once, for helpers that build their
    /// own processor rather than being handed one. A caller that must keep
    /// another of main control's fields borrowed at the same time builds the
    /// bundle from those fields directly instead.
    fn command_machine<'operation>(
        &'operation mut self,
        diagnostic_effects: &'operation mut DiagnosticEffects,
    ) -> CommandMachine<'operation, G> {
        CommandMachine {
            state: &mut self.command,
            fuel: self.fuel.fuel_mut(),
            capabilities: &mut self.capabilities,
            host_facts: CommandMachineHostFacts::Live(ExecutorHostFacts {
                modes: &self.modes,
                pdf_ignore_depth: self.pdf_ignore_depth,
                telemetry: &mut self.episode_telemetry,
            }),
            observations: &mut self.operation_observations,
            assignment_receipts: None,
            diagnostic_effects,
            shown_mode: &mut self.shown_mode,
            initex: self.initex,
            emit_dvi_override: self.emit_dvi_override,
            immediate_prints: &mut self.immediate_prints,
            prepared_shipout: &mut self.prepared_shipout,
            pending_show_completion: None,
            pending_outer_page_build_context: None,
            output_routine_active: self.boxes.output_routine_active,
        }
    }

    /// Takes TeX82 §1030's parking decision for the step just scanned, and
    /// clears the outgoing parking so nested episodes run from `big_switch`.
    ///
    /// Every step driver takes this before applying its step and settles it
    /// through [`MainControlParking::post_apply`] before the callback-scoped
    /// command admission closes. The rule is stated here once: three drivers
    /// used to spell it out inline, and a rule spelled three times is a rule
    /// two of them can be missing.
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

    /// Returns the command site retained for the most recent resource need.
    #[must_use]
    pub const fn pending_resource_site(&self) -> Option<OriginId> {
        self.pending_resource_site
    }

    /// Releases the diagnostic-only site retained for the resource need that
    /// has just been answered by the outer ledger. The command attempt itself
    /// remains installed until the next canonical step resumes it.
    pub(crate) fn acknowledge_resource_need(&mut self) {
        self.pending_resource_site = None;
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

    pub(crate) fn take_checkpoint_eligibilities(
        &mut self,
    ) -> Vec<crate::checkpoint::CheckpointEligibility> {
        std::mem::take(&mut self.completed_checkpoint_eligibilities)
    }

    pub(crate) fn take_job_start_eligibility(
        &mut self,
    ) -> Option<crate::checkpoint::CheckpointEligibility> {
        self.job_start_eligibility.take()
    }

    /// Records one ordered named-boundary intent per newly committed shipout.
    /// Structural and command-owned continuations gate later publication.
    /// Returns the replay projection of TeX's current execution mode.
    #[must_use]
    pub fn current_mode(&self) -> Mode {
        self.modes.current_mode()
    }

    #[cfg(test)]
    pub(crate) fn mode_nest_for_test(&self) -> &ModeNest {
        &self.modes
    }

    #[cfg(test)]
    pub(crate) fn mode_nest_mut_for_test(&mut self) -> &mut ModeNest {
        &mut self.modes
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
    fn enter_main_control(&mut self, stores: &mut CommandContext<'_, G>) -> bool {
        // Seeds `line` before the first command is delivered; every step
        // republishes it after delivery (see `apply_operation`).
        if std::mem::replace(&mut self.main_control_entered, true) {
            return false;
        }
        schedule_everyjob(&mut self.command, stores);
        true
    }

    /// Appends already-committed records to the operation's commit buffer.
    /// They are published only when the whole operation commits.
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
            OperationDelivery::Alignment(alignment),
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
        scanned: &mut PreparedColdCommand<G>,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Option<Result<ReplayStep, ExecError>> {
        let applied = match scanned {
            ColdOperation::ReplayCompleted(episode) => {
                self.completed_replay_episode = Some(*episode);
                Ok(ReplayStep::Continue)
            }
            ColdOperation::Math(request) => {
                self.apply_math_request(*request, stores, diagnostic_effects)
            }
            ColdOperation::DisplayAlignmentRecovery => {
                self.recover_display_alignment_closer(stores, diagnostic_effects)
            }
            ColdOperation::MathDelimiter(boundary) => {
                self.apply_math_delimiter(*boundary, stores, diagnostic_effects)
            }
            // TeX82 §1137's `hmode+math_shift: init_math` and §1193's
            // `mmode+math_shift: if cur_group=math_shift_group then
            // after_math else off_save`. §1090 backs a `vmode+math_shift` up
            // and runs `new_graf(true)` first, so vertical mode never reaches
            // this step.
            ColdOperation::MathShift { pairing } => {
                self.apply_math_shift(*pairing, stores, diagnostic_effects)
            }
            ColdOperation::DiscretionaryOpening(opening) => {
                self.begin_discretionary(*opening, stores, diagnostic_effects)
            }
            ColdOperation::DiscretionaryPartEnd => {
                self.finish_discretionary_part(stores, diagnostic_effects)
            }
            ColdOperation::DiscretionaryHyphen { origin } => {
                self.apply_discretionary_hyphen(*origin, stores, diagnostic_effects)
            }
            // TeX82 §1123's `make_accent` runs §1270's `do_assignments`
            // between the accent code and §1124's base character, so it
            // executes whole commands of its own before it can finish.
            ColdOperation::Accent(accent) => self.apply_accent(*accent, stores, diagnostic_effects),
            ColdOperation::InputStream { request, resource } => match request {
                RootedInputStreamRequest::Open {
                    stream, file_name, ..
                } => {
                    let slot = replay_stream_slot(*stream);
                    let packed_name = file_name.packed();
                    stores.world_mut().close_in(slot);
                    if let Some(resource) = resource {
                        if let Err(error) = stores
                            .world_mut()
                            .set_memory_file(&packed_name, resource.bytes().to_vec())
                        {
                            return Some(Err(error.into()));
                        }
                        let content = match InputReadState::read_input_file(
                            &mut stores.input_open_context(),
                            std::path::Path::new(&packed_name),
                        ) {
                            Ok(content) => content,
                            Err(error) => return Some(Err(error.into())),
                        };
                        if let Err(error) = stores.world_mut().open_in_content(slot, &content) {
                            return Some(Err(error.into()));
                        }
                    }
                    Ok(ReplayStep::Continue)
                }
                RootedInputStreamRequest::Close { stream, .. } => {
                    stores.world_mut().close_in(replay_stream_slot(*stream));
                    Ok(ReplayStep::Continue)
                }
                RootedInputStreamRequest::Read { .. } => {
                    return None;
                }
            },
            ColdOperation::PdfSetRandomSeed { seed } => {
                stores.world_mut().set_pdf_random_seed(*seed);
                Ok(ReplayStep::Continue)
            }
            ColdOperation::PdfResetTimer => {
                stores.world_mut().reset_pdf_timer();
                Ok(ReplayStep::Continue)
            }
            _ => return None,
        };
        // TeX82 §§994/1005 run `fire_up` inside the host-owned operation's
        // `build_page` call.  In particular, §1200's display resumption has
        // already installed its horizontal level when page fire-up enters
        // §1025's output routine.  Do this before handing the completed step
        // back to any driver: the unobserved driver returns directly here,
        // while observed drivers have a later publication-only tail.
        let applied = applied.and_then(|step| {
            let pending = PendingPageOutputFacts::capture(
                &stores.command_context().expect("live generation"),
            );
            self.fire_pending_page_output(stores, diagnostic_effects, pending)?;
            Ok(step)
        });
        self.main_loop_active = false;
        Some(applied)
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
    /// Enters TeX82 §1117's live `disc_group` after the command processor has
    /// consumed only its opening brace.
    fn begin_discretionary(
        &mut self,
        _opening: ScannedDiscretionaryOpening,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<ReplayStep, ExecError> {
        if matches!(
            self.modes.current_mode(),
            Mode::Vertical | Mode::InternalVertical
        ) {
            let mut context = stores.command_context().expect("live generation");
            start_paragraph(
                &mut self.command,
                &mut self.modes,
                &mut context,
                diagnostic_effects,
                true,
            )?;
        }
        {
            let mut context = stores.command_context().expect("live generation");
            crate::box_runtime::flush_pending_hchars_with_fuel(
                &mut self.modes,
                &mut context,
                diagnostic_effects,
                self.fuel.fuel_mut(),
            )?;
        }
        self.open_discretionary_part(stores, diagnostic_effects)?;
        self.active_discretionaries.push(ActiveDiscretionary {
            parts: Vec::new(),
            rejected: false,
        });
        Ok(ReplayStep::Continue)
    }

    fn open_discretionary_part(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<(), ExecError> {
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
        enter_group(
            &mut context,
            &mut self.command,
            diagnostic_effects,
            GroupKind::Disc,
        );
        Ok(())
    }

    /// Implements §1120's `build_discretionary`: finish the current live
    /// restricted-horizontal list, `unsave`, and either scan the next opening
    /// brace or append the completed three-part node.
    fn finish_discretionary_part(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<ReplayStep, ExecError> {
        let mut level = {
            let mut context = stores.command_context().expect("live generation");
            crate::box_runtime::commit_current_list(
                &mut self.modes,
                &mut context,
                diagnostic_effects,
                self.fuel.fuel_mut(),
            )?
        };
        // TeX82 §1121 advances `q` across the admissible prefix and, on the
        // first forbidden node `p`, severs `link(q)`. Thus the prefix remains
        // this discretionary part while `show_box(p)` reports and flushes the
        // entire suffix beginning at the offending node.
        let (nodes, deleted, prefix_end) = {
            let mut stores = stores.command_context().expect("live generation");
            let part_nodes = level.list().nodes(&stores);
            let part_len = part_nodes.len();
            let first_forbidden = match part_nodes.try_for_each_range(0..part_len, |index, node| {
                if matches!(
                    node,
                    tex_state::NodeView::Char { .. }
                        | tex_state::NodeView::Lig { .. }
                        | tex_state::NodeView::Kern { .. }
                        | tex_state::NodeView::Rule { .. }
                        | tex_state::NodeView::HList(_)
                        | tex_state::NodeView::VList(_)
                ) {
                    core::ops::ControlFlow::Continue(())
                } else {
                    core::ops::ControlFlow::Break(index)
                }
            }) {
                core::ops::ControlFlow::Break(index) => Some(index),
                core::ops::ControlFlow::Continue(()) => None,
            };
            let prefix_end = first_forbidden.unwrap_or(part_len);
            let part = level.list_mutation().take_span();
            let nodes = stores.slice_page_node_span(part, 0..prefix_end);
            let deleted = first_forbidden
                .map(|index| stores.slice_page_node_span(part, index..part_len).list());
            let aftergroup = leave_group_payloads(
                &mut stores,
                &mut self.command,
                diagnostic_effects,
                GroupKind::Disc,
            )
            .map_err(|_| ExecError::MissingToken {
                context: "discretionary group",
            })?;
            schedule_aftergroup(
                &mut self.command_machine(diagnostic_effects),
                &mut stores,
                aftergroup,
            )?;
            (nodes, deleted, prefix_end)
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
            report_improper_discretionary(&mut stores, diagnostic_effects, deleted, context)?;
        }
        if replacement_too_long {
            let mut stores = stores.command_context().expect("live generation");
            let context = self.command.output_open_context(&stores);
            crate::error_report::report_ordered_error(
                &mut stores,
                diagnostic_effects,
                "Discretionary list is too long",
                &["Wow---I never thought anybody would tweak me here."],
                context,
            )?;
        }
        if part_count < 3 {
            let mut diagnostics = Vec::new();
            {
                let mut context = stores.command_context().expect("live generation");
                let mut host_facts = ExecutorHostFacts {
                    modes: &self.modes,
                    pdf_ignore_depth: self.pdf_ignore_depth,
                    telemetry: &mut self.episode_telemetry,
                };
                let mut processor = command_processor(
                    &mut self.command,
                    self.fuel.fuel_mut(),
                    &mut self.capabilities,
                    &mut host_facts,
                    &mut self.operation_observations,
                    diagnostic_effects,
                    &mut context,
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
            report_pending_diagnostics(stores, diagnostic_effects, diagnostics)?;
            self.open_discretionary_part(stores, diagnostic_effects)?;
            return Ok(ReplayStep::Continue);
        }
        let active = self
            .active_discretionaries
            .pop()
            .expect("three parts require an active discretionary");
        if active.rejected {
            return Ok(ReplayStep::Continue);
        }
        let [pre, post, mut replace]: [tex_state::page_node_arena::PageListSpan; 3] = active
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
            // Section 1120 calls `unsave` before diagnosing a forbidden
            // nonempty replacement in math mode. Publish that completed
            // restoration program before the synchronous error dialogue.
            command_context.publish_diagnostic_effects_before_synchronous_print(diagnostic_effects);
            report_escaped_error(
                &mut command_context,
                diagnostic_effects,
                "Illegal math ",
                "discretionary",
                "",
                &[
                    "Sorry: The third part of a discretionary break must be",
                    "empty, in math formulas. I had to delete your third part.",
                ],
                context,
            )?;
            replace = tex_state::page_node_arena::PageListSpan::empty();
        }
        let physical_replace_count = stores
            .command_context()
            .expect("live generation")
            .page_node_span(replace)
            .expect("discretionary replacement is a live page list")
            .len()
            .try_into()
            .expect("TeX discretionary replacement count fits a quarterword");
        self.modes.current_list_mutation().push(
            &mut stores.command_context().expect("live generation"),
            Node::Disc {
                kind: DiscKind::Discretionary,
                pre: pre.list(),
                post: post.list(),
                replace: replace.list(),
                physical_replace_count,
            },
        );
        Ok(ReplayStep::Continue)
    }

    /// Executes TeX82 §1113's `append_discretionary` shorthand for `\-`.
    fn apply_discretionary_hyphen(
        &mut self,
        origin: OriginId,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<ReplayStep, ExecError> {
        if matches!(
            self.modes.current_mode(),
            Mode::Vertical | Mode::InternalVertical
        ) {
            let mut context = stores.command_context().expect("live generation");
            start_paragraph(
                &mut self.command,
                &mut self.modes,
                &mut context,
                diagnostic_effects,
                true,
            )?;
        }
        let pre = {
            let mut stores = stores.command_context().expect("live generation");
            crate::box_runtime::flush_pending_hchars_with_fuel(
                &mut self.modes,
                &mut stores,
                diagnostic_effects,
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
                        diagnostic_effects,
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
        self.modes.current_list_mutation().push(
            &mut stores.command_context().expect("live generation"),
            Node::Disc {
                kind: DiscKind::ExplicitHyphen,
                pre,
                post: empty,
                replace: empty,
                physical_replace_count: 0,
            },
        );
        Ok(ReplayStep::Continue)
    }

    /// Captures TeX's checked save-stack projection into the resident
    /// operation facts while semantic admission is still live.
    fn capture_save_stack_usage(
        preparation: &mut OperationPreparation<'_, G>,
        stores: &CommandContext<'_, G>,
        boxes: &ReplayBoxes<G>,
        command: &tex_command::CommandState<G>,
        profile: CommandProfile,
    ) {
        // TeX82 §§645/1083 keeps ordinary box specs immediately below their
        // §273 boundaries. Vcenters and insertions deliberately have smaller
        // projections (§§1167/1099), so derive the words from each live kind.
        let box_spec_words = boxes
            .active_boxes
            .iter()
            .map(|active| active.kind.save_stack_spec_words())
            .fold(0_usize, usize::saturating_add);
        let (aftergroup_words, latest_aftergroup_position) =
            command.aftergroup_save_stack_projection();
        let checked = stores
            .checked_save_stack_words(
                aftergroup_words,
                latest_aftergroup_position,
                profile.capabilities().supports_etex(),
            )
            .saturating_add(box_spec_words);
        preparation.record_checked_save_stack_words(checked);
    }

    /// Drains the field-level save projection after application. Exceptional
    /// host-owned paths which had to release command admission reacquire only
    /// for this missing scalar rather than moving a second facts aggregate.
    fn settle_save_stack_usage(
        &mut self,
        stores: &mut Universe<G>,
        preparation: &mut OperationPreparation<'_, G>,
    ) {
        let checked = if let Some(checked) = preparation.take_checked_save_stack_words() {
            checked
        } else {
            let context = stores.command_context().expect("save-stack admission");
            Self::capture_save_stack_usage(
                preparation,
                &context,
                &self.boxes,
                self.command.state(),
                self.command_profile(),
            );
            preparation
                .take_checked_save_stack_words()
                .expect("exceptional settlement captures its save projection")
        };
        self.max_save_stack = self.max_save_stack.max(checked);
    }

    #[allow(clippy::too_many_arguments)]
    fn command_requires_transaction(
        &self,
        stores: &mut Universe<G>,
        frame: &CommandEpisode<G>,
    ) -> bool {
        let pdf_output = frame
            .current_option()
            .filter(|command| {
                crate::transaction_protocol::command_uses_pdf_output(command.meaning())
            })
            .map(|_| {
                stores
                    .command_context()
                    .expect("PDF transaction classification admission")
                    .int_param(IntParam::PDF_OUTPUT)
            });
        let end_group_can_package_box = frame.current_option().is_some_and(|command| {
            matches!(
                command.meaning(),
                ResolvedMeaning::Static(Meaning::CharToken {
                    cat: Catcode::EndGroup,
                    ..
                })
            )
        }) && !self.boxes.active_boxes.is_empty();
        let innermost_group = end_group_can_package_box
            .then(|| {
                stores
                    .command_context()
                    .expect("group classification admission")
                    .innermost_group_kind()
            })
            .flatten();
        command_requires_transaction_from_facts(
            self.modes.current_mode(),
            &self.boxes,
            frame,
            pdf_output,
            innermost_group,
        )
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
        mut initial_delivery: Option<OperationDelivery>,
        mut tracked_region: Option<&mut Option<Result<TrackedRegionRecord, DependencyRegionError>>>,
    ) -> Result<StepResult, ExecError> {
        if self.publish_pending_named_boundary(stores)?.is_some() {
            return Ok(StepResult::Progress(ReplayStep::Continue));
        }
        let initial_boundaries = self.completed_boundaries.len();
        let initial_effect_pos = stores.world().effect_pos();
        let initial_artifacts = stores.world().artifact_commits().len();
        let initial_format_dump = self.dumped_format.is_some();
        let initial_diagnostic = self.first_causal_context.is_some();
        let initial_error_count = stores.world().error_channel().error_count();
        let mut operations = 0_usize;
        let mut last_step = ReplayStep::Continue;
        let mut direct_attempt_recorded = false;
        let mut command_episode = CommandEpisode::default();
        let mut cold_operation = ColdOperationSlot::default();
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
            let mut preparation_scope = OperationPreparationScope;
            let mut host_preparation = OperationPreparation::new(&mut preparation_scope);
            if operations != 0
                && let Some(boundary) = self.publish_pending_named_boundary(stores)?
            {
                self.record_direct_episode_commit(
                    stores,
                    operations,
                    crate::EpisodeCommitBoundary::NamedCheckpoint(boundary),
                    initial_artifacts,
                    initial_boundaries,
                    initial_effect_pos,
                );
                return Ok(StepResult::Progress(last_step));
            }
            // Private revisions require every scanner-time immutable
            // allocation to belong to one fixed-size operation suffix. The
            // scope therefore opens before delivery preflight, while a
            // resource retry reuses the exact scope moved into its
            // continuation instead of nesting another owner around it.
            let resumed_resource = self.pending_resource_operation.take().map(|pending| {
                let (operation, resume, pending) = self
                    .command
                    .resume_attempt(stores, pending.attempt)
                    .unwrap_or_else(|_| {
                        panic!("resource continuation belongs to the admitted generation")
                    });
                assert_eq!(
                    resume, SUSPENDED_RESOURCE_RESUME,
                    "resource continuation resumes at its prepared-operation cursor"
                );
                (operation, pending)
            });
            // Drain an occupied retry in place. Keeping the outer Option and
            // destination aggregate out of locals prevents ordinary delivery
            // from copying the frame-sized vacant representation. A genuine
            // retry moves only its operation capability and exact destination
            // fields into the caller-owned slots used by this iteration.
            let retained_operation = if resumed_resource.is_some() {
                debug_assert!(self.pending_direct_operation.is_none());
                None
            } else if let Some(pending) = self.pending_direct_operation.as_mut() {
                let operation =
                    match std::mem::replace(&mut pending.state, PendingDirectState::Fresh) {
                        PendingDirectState::Fresh => None,
                        PendingDirectState::Retained(operation) => Some(operation),
                    };
                match &mut pending.destination {
                    PendingDirectDestination::Alignment(pending) => {
                        host_preparation.fill_delivery(
                            OperationDelivery::AlignmentRetry {
                                alignment: pending.alignment,
                                cursor: pending.cursor,
                            },
                            pending.scanner.take(),
                            pending.expansion.take(),
                        );
                    }
                    PendingDirectDestination::Frame(pending) => {
                        (command_episode, cold_operation) = pending.frame.take_parts();
                        match pending.resume {
                            PendingFrameResume::Delivery => host_preparation.fill_delivery(
                                OperationDelivery::Command,
                                None,
                                None,
                            ),
                            PendingFrameResume::ColdExecution(barrier) => {
                                host_preparation.fill_delivery(
                                    OperationDelivery::SuspendedCold { barrier },
                                    None,
                                    None,
                                );
                            }
                        }
                    }
                };
                self.pending_direct_operation = None;
                operation
            } else {
                None
            };
            let operation = match resumed_resource {
                Some((operation, pending)) => {
                    (command_episode, cold_operation) = pending.frame.into_parts();
                    let _ = command_episode.error.take();
                    host_preparation.fill_delivery(
                        OperationDelivery::SuspendedCold {
                            barrier: pending.barrier,
                        },
                        None,
                        None,
                    );
                    Some(operation)
                }
                None => retained_operation,
            };
            let operation_mark = self.begin_direct_operation(stores, operation);
            let mut diagnostic_effects = DiagnosticEffects::new();
            // A cascading §1026 page break can become ready while the prior
            // operation still owns a rollback-restorable mode root. Resume
            // that builder continuation in its own journaled operation before
            // delivering another TeX command.
            let pending_page_output = if !self.page_region_succession_pending
                && !self.boxes.output_routine_active
            {
                PendingPageOutputFacts::capture(&stores.command_context().expect("live generation"))
            } else {
                PendingPageOutputFacts::default()
            };
            if pending_page_output.is_pending() {
                let applied = self
                    .fire_pending_page_output(stores, &mut diagnostic_effects, pending_page_output)
                    .map(|()| ReplayStep::Continue);
                let boundary = self.episode_commit_boundary(
                    stores,
                    &applied,
                    1,
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
                            diagnostic_effects,
                        );
                    }
                };
                stores
                    .world_mut()
                    .publish_diagnostic_effects(diagnostic_effects);
                if let Some(error) =
                    self.admit_observed_receipt(stores, OperationTermination::Continue)
                {
                    self.commit_direct_operation(stores, operation_mark);
                    return Err(error);
                }
                self.commit_direct_operation(stores, operation_mark);
                self.record_direct_episode_commit(
                    stores,
                    1,
                    boundary.unwrap_or(crate::EpisodeCommitBoundary::SliceLimit),
                    initial_artifacts,
                    initial_boundaries,
                    initial_effect_pos,
                );
                return Ok(StepResult::Progress(step));
            }
            if host_preparation.has_delivery() {
                // A retry reuses only its typed delivery/scanner owners. Live
                // executor facts are sampled by the resumed processor if and
                // when that scanner requests one.
            } else if let Some(delivery) = initial_delivery.take() {
                host_preparation.fill_delivery(delivery, None, None);
            } else if self.preflight_replay_delivery(
                stores,
                &mut host_preparation,
                &mut diagnostic_effects,
                &mut command_episode,
                &mut cold_operation,
            ) == PreflightReadiness::Failed
            {
                let error = command_episode.take_error();
                if let Some(mark) = episode_tracked_mark.take() {
                    let _ = stores.abandon_dependency_region(mark);
                }
                if execution_error_is_fuel(&error) {
                    self.episode_telemetry
                        .record_semantic_barrier(crate::SemanticEpisodeBarrier::Fuel);
                }
                let result = self.finish_resource_preflight_failure(stores, error);
                if let Err(error) = &result {
                    Self::publish_pdf_fatal_error(stores, error)?;
                }
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
                    assert!(
                        command_episode.has_preflight(),
                        "resource delivery retains its exact retry frame"
                    );
                    let destination = PendingDirectDestination::Frame(PendingFrameDestination {
                        frame: OperationFrame::new(command_episode, cold_operation),
                        resume: PendingFrameResume::Delivery,
                    });
                    let operation = self.retain_direct_operation_for_retry(stores, operation_mark);
                    self.pending_direct_operation = Some(PendingDirectOperation {
                        state: PendingDirectState::Retained(operation),
                        destination,
                    });
                } else {
                    self.commit_direct_operation(stores, operation_mark);
                }
                return result;
            }

            let alignment_delivery = match host_preparation.delivery() {
                OperationDelivery::Alignment(alignment) => Some(Some(*alignment)),
                OperationDelivery::AlignmentRetry { alignment, .. } => Some(*alignment),
                OperationDelivery::Replay
                    if self.active_alignment.is_some()
                        || (self.modes.current_mode() == Mode::DisplayMath
                            && self.modes.current_list().has_display_alignment()) =>
                {
                    Some(None)
                }
                _ => None,
            };
            let barrier = operation_barrier(host_preparation.delivery(), &command_episode);
            if matches!(
                barrier,
                Some(crate::transaction_protocol::CommandBarrier::Resource)
            ) && !(matches!(host_preparation.delivery(), OperationDelivery::Command)
                && command_episode.current_option().is_some_and(|command| {
                    matches!(
                        command.meaning(),
                        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            UnexpandablePrimitive::PdfXImage
                        ))
                    )
                })
                && stores.int_param(IntParam::PDF_OUTPUT) <= 0)
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
                let applied = match self.execute_typed_operation(
                    stores,
                    &mut host_preparation,
                    &mut diagnostic_effects,
                    &mut command_episode,
                    &mut cold_operation,
                ) {
                    Err(TypedOperationError::Preparation(error)) => {
                        command_episode.error = Some(error);
                        if let Some(mark) = tracked_mark {
                            let _ = stores.abandon_dependency_region(mark);
                        }
                        if command_episode.has_unavailable(&cold_operation) {
                            let result = self.finish_unavailable_prepared_resource_operation(
                                stores,
                                operation_mark,
                                command_episode,
                                cold_operation,
                                barrier,
                            );
                            if matches!(result, Ok(StepResult::Suspended(_))) {
                                self.advance_telemetry.rollbacks += 1;
                                #[cfg(feature = "profiling")]
                                self.episode_telemetry
                                    .record_rollback(crate::SemanticEpisodeBarrier::Resource, 1);
                                #[cfg(not(feature = "profiling"))]
                                self.episode_telemetry
                                    .record_rollback(crate::SemanticEpisodeBarrier::Resource);
                            }
                            return result;
                        }
                        let result = self.finish_resource_preflight_failure(
                            stores,
                            command_episode.take_error(),
                        );
                        if matches!(result, Ok(StepResult::Suspended(_))) {
                            let alignment_scanner = command_episode.alignment_scanner.take();
                            let destination = own_alignment_retry_child(
                                alignment_delivery,
                                command_episode,
                                cold_operation,
                                alignment_scanner,
                            )
                            .expect("resource suspension retains one direct caller destination");
                            self.retain_direct_delivery_for_retry(
                                stores,
                                operation_mark,
                                destination,
                            );
                            self.advance_telemetry.rollbacks += 1;
                            #[cfg(feature = "profiling")]
                            self.episode_telemetry
                                .record_rollback(crate::SemanticEpisodeBarrier::Resource, 1);
                            #[cfg(not(feature = "profiling"))]
                            self.episode_telemetry
                                .record_rollback(crate::SemanticEpisodeBarrier::Resource);
                        } else {
                            self.commit_direct_operation(stores, operation_mark);
                        }
                        return result;
                    }
                    Err(TypedOperationError::Application(error)) => Err(error),
                    Ok(step) => Ok(step),
                };
                self.settle_save_stack_usage(stores, &mut host_preparation);
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
                            diagnostic_effects,
                        );
                    }
                };
                stores
                    .world_mut()
                    .publish_diagnostic_effects(diagnostic_effects);
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
                barrier,
                Some(crate::transaction_protocol::CommandBarrier::Transaction(_))
            ) || self.command_requires_transaction(stores, &command_episode)
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
                let tracked_mark = episode_tracked_mark.take();
                if let Some(crate::transaction_protocol::CommandBarrier::Transaction(transaction)) =
                    barrier
                {
                    transaction
                        .admit(transaction.projection())
                        .expect("direct dispatch owns the exact narrow projection");
                }
                let applied = match self.execute_typed_operation(
                    stores,
                    &mut host_preparation,
                    &mut diagnostic_effects,
                    &mut command_episode,
                    &mut cold_operation,
                ) {
                    Err(TypedOperationError::Preparation(error)) => {
                        command_episode.error = Some(error);
                        if let Some(mark) = tracked_mark {
                            let _ = stores.abandon_dependency_region(mark);
                        }
                        if command_episode.has_unavailable(&cold_operation) {
                            let result = self.finish_unavailable_prepared_resource_operation(
                                stores,
                                operation_mark,
                                command_episode,
                                cold_operation,
                                barrier,
                            );
                            return result;
                        }
                        let result = self.finish_resource_preflight_failure(
                            stores,
                            command_episode.take_error(),
                        );
                        match result {
                            Ok(step @ StepResult::Suspended(_)) => {
                                let alignment_scanner = command_episode.alignment_scanner.take();
                                let destination = own_alignment_retry_child(
                                    alignment_delivery,
                                    command_episode,
                                    cold_operation,
                                    alignment_scanner,
                                )
                                .expect(
                                    "resource suspension retains one direct caller destination",
                                );
                                self.retain_direct_delivery_for_retry(
                                    stores,
                                    operation_mark,
                                    destination,
                                );
                                return Ok(step);
                            }
                            Ok(step) => {
                                self.commit_direct_operation(stores, operation_mark);
                                return Ok(step);
                            }
                            Err(error) => {
                                let alignment_scanner = command_episode.alignment_scanner.take();
                                let destination = own_alignment_retry_child(
                                    alignment_delivery,
                                    command_episode,
                                    cold_operation,
                                    alignment_scanner,
                                )
                                .expect("retained failure owns one direct caller destination");
                                self.retain_direct_delivery_for_retry(
                                    stores,
                                    operation_mark,
                                    destination,
                                );
                                Self::publish_pdf_fatal_error(stores, &error)?;
                                return Err(error);
                            }
                        }
                    }
                    Err(TypedOperationError::Application(error)) => Err(error),
                    Ok(step) => Ok(step),
                };
                self.episode_telemetry.record_attempt();
                self.advance_telemetry.attempts += 1;
                self.settle_save_stack_usage(stores, &mut host_preparation);
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
                            if matches!(
                                command_episode.phase,
                                Some(PreflightCommandPhase::ImmediatePdfRetry(_))
                            ) {
                                command_episode.clear_cold(&mut cold_operation);
                            }
                            self.pending_direct_operation =
                                command_episode
                                    .has_preflight()
                                    .then(|| PendingDirectOperation {
                                        state: PendingDirectState::Fresh,
                                        destination: PendingDirectDestination::Frame(
                                            PendingFrameDestination {
                                                frame: OperationFrame::new(
                                                    command_episode,
                                                    ColdOperationSlot::default(),
                                                ),
                                                resume: PendingFrameResume::Delivery,
                                            },
                                        ),
                                    });
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
                            diagnostic_effects,
                        );
                    }
                };
                stores
                    .world_mut()
                    .publish_diagnostic_effects(diagnostic_effects);
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
            let preserves_undefined_for_executor_diagnostic =
                matches!(host_preparation.delivery(), OperationDelivery::Command)
                    && command_episode.current_option().is_some_and(|command| {
                        matches!(
                            command.meaning(),
                            ResolvedMeaning::Static(Meaning::Undefined | Meaning::Unknown(_))
                        )
                    });
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
            let applied = match self.execute_typed_operation(
                stores,
                &mut host_preparation,
                &mut diagnostic_effects,
                &mut command_episode,
                &mut cold_operation,
            ) {
                Err(TypedOperationError::Preparation(error)) => {
                    command_episode.error = Some(error);
                    if let Some(interaction) = saved_interaction {
                        stores.set_interaction_mode(interaction);
                    }
                    if let Some(mark) = tracked_mark {
                        let _ = stores.abandon_dependency_region(mark);
                    }
                    let result = if command_episode.has_unavailable(&cold_operation) {
                        self.finish_unavailable_prepared_resource_operation(
                            stores,
                            operation_mark,
                            command_episode,
                            cold_operation,
                            barrier,
                        )
                    } else {
                        let result = self.finish_resource_preflight_failure(
                            stores,
                            command_episode.take_error(),
                        );
                        if matches!(result, Ok(StepResult::Suspended(_))) {
                            let alignment_scanner = command_episode.alignment_scanner.take();
                            let destination = own_alignment_retry_child(
                                alignment_delivery,
                                command_episode,
                                cold_operation,
                                alignment_scanner,
                            )
                            .expect("resource suspension retains one direct caller destination");
                            self.retain_direct_delivery_for_retry(
                                stores,
                                operation_mark,
                                destination,
                            );
                        } else {
                            self.commit_direct_operation(stores, operation_mark);
                        }
                        result
                    };
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
                Err(TypedOperationError::Application(error)) => Err(error),
                Ok(step) => Ok(step),
            };
            if let Some(interaction) = saved_interaction {
                stores.set_interaction_mode(interaction);
            }
            operations += 1;
            self.settle_save_stack_usage(stores, &mut host_preparation);
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
                    if execution_error_needs_command_retry(&error) {
                        let result = self.finish_resource_preflight_failure(stores, error);
                        if matches!(result, Ok(StepResult::Suspended(_))) {
                            assert!(
                                command_episode.has_unavailable(&cold_operation),
                                "nested resource suspension retains its enclosing operation"
                            );
                            self.discard_direct_operation(stores, operation_mark);
                            self.pending_direct_operation = Some(PendingDirectOperation {
                                state: PendingDirectState::Fresh,
                                destination: PendingDirectDestination::Frame(
                                    PendingFrameDestination {
                                        frame: OperationFrame::new(command_episode, cold_operation),
                                        resume: PendingFrameResume::ColdExecution(barrier),
                                    },
                                ),
                            });
                            self.advance_telemetry.rollbacks += 1;
                            #[cfg(feature = "profiling")]
                            self.episode_telemetry
                                .record_rollback(crate::SemanticEpisodeBarrier::Resource, 1);
                            #[cfg(not(feature = "profiling"))]
                            self.episode_telemetry
                                .record_rollback(crate::SemanticEpisodeBarrier::Resource);
                        }
                        return result;
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
                        diagnostic_effects,
                    );
                }
            };
            stores
                .world_mut()
                .publish_diagnostic_effects(diagnostic_effects);
            if let Some(error) =
                self.admit_observed_receipt(stores, operation_termination(step, self.fatal))
            {
                self.commit_direct_operation(stores, operation_mark);
                return Err(error);
            }
            self.commit_direct_operation(stores, operation_mark);
            last_step = step;
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
        delivery: OperationDelivery,
        transaction: OperationTransaction,
        max_operations: usize,
        tracked_region: Option<&mut Option<Result<TrackedRegionRecord, DependencyRegionError>>>,
    ) -> Result<StepResult, ExecError> {
        // Once execution begins, this control can no longer truthfully
        // produce job-start restart eligibility even if the caller skipped
        // the ordinary initial publication hook.
        self.job_start_eligibility = None;
        if self.operation_observations.is_none() {
            // A caller may resume an observed resource suspension through
            // the unobserved API. The semantic continuation is independent
            // of instrumentation, so drop the moved evidence owner instead
            // of publishing it into some later unrelated observed step.
            self.suspended_operation_observation = None;
        }
        let initial_delivery =
            matches!(transaction, OperationTransaction::Alignment).then_some(delivery);
        self.execute_direct_episode(stores, max_operations, initial_delivery, tracked_region)
    }

    /// Attempts one atomic main-control operation.
    ///
    /// Missing retained input is returned as a typed suspension after the
    /// exact command/input continuation has been retained. The next call
    /// creates a fresh processor borrow and resumes that continuation without
    /// redelivering the command.
    pub fn advance(&mut self, stores: &mut Universe<G>) -> Result<StepResult, ExecError> {
        if self.terminal_revision_step.is_some() {
            return Err(ExecError::ExecutionAlreadyTerminated);
        }
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
            OperationDelivery::Replay,
            OperationTransaction::Advance,
            1,
            None,
        )
    }

    /// Advances one production driver chunk under a single bounded retry
    /// point. The public one-operation [`Self::advance`] contract remains
    /// available to diagnostic and focused-test callers.
    pub fn advance_episode(&mut self, stores: &mut Universe<G>) -> Result<StepResult, ExecError> {
        if self.terminal_revision_step.is_some() {
            return Err(ExecError::ExecutionAlreadyTerminated);
        }
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
            OperationDelivery::Replay,
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
        if self.terminal_revision_step.is_some() {
            return Err(ExecError::ExecutionAlreadyTerminated);
        }
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
            OperationDelivery::Replay,
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
        let continuation = self.pending_diagnostic_operation.take();
        let mut command_episode = CommandEpisode::default();
        let mut cold_operation = ColdOperationSlot::default();
        let mut preparation_scope = OperationPreparationScope;
        let mut host_preparation = OperationPreparation::new(&mut preparation_scope);
        let retained_attempt = match continuation {
            Some(PendingDiagnosticOperation {
                operation,
                destination: PendingDiagnosticDestination::<G> { frame, barrier },
            }) => {
                (command_episode, cold_operation) = frame.into_parts();
                let _ = command_episode.error.take();
                if command_episode.has_unavailable(&cold_operation) {
                    host_preparation.fill_delivery(
                        OperationDelivery::SuspendedCold { barrier },
                        None,
                        None,
                    );
                } else {
                    host_preparation.fill_delivery(OperationDelivery::Command, None, None);
                }
                Some(operation)
            }
            None => None,
        };
        let operation_mark = self.begin_direct_operation(stores, retained_attempt);
        let mut diagnostic_effects = DiagnosticEffects::new();
        if !host_preparation.has_delivery() {
            self.ensure_primitive_handles(stores);
            let (command, cursor, retry_expansion, source_provenance) = {
                let mut context = stores.command_context().expect("live generation");
                let mut host_facts = ExecutorHostFacts {
                    modes: &self.modes,
                    pdf_ignore_depth: self.pdf_ignore_depth,
                    telemetry: &mut self.episode_telemetry,
                };
                let mut processor = command_processor(
                    &mut self.command,
                    self.fuel.fuel_mut(),
                    &mut self.capabilities,
                    &mut host_facts,
                    &mut self.operation_observations,
                    &mut diagnostic_effects,
                    &mut context,
                );
                let command = processor
                    .get_x_token_preserving_undefined()
                    .map_err(command_error);
                let cursor = processor.delivery_cursor();
                let retry_expansion = command
                    .as_ref()
                    .err()
                    .and_then(|_| processor.take_pending_expansion_work());
                let source_provenance = command
                    .as_ref()
                    .ok()
                    .and_then(Option::as_ref)
                    .and_then(|command| processor.source_provenance(command));
                (command, cursor, retry_expansion, source_provenance)
            };
            let command = match command {
                Ok(command) => command,
                Err(error) => {
                    let result = self.finish_resource_preflight_failure(stores, error);
                    if matches!(result, Ok(StepResult::Suspended(_))) {
                        let expansion = retry_expansion
                            .expect("resource expansion retains its exact parked root");
                        let operation =
                            self.retain_direct_operation_for_retry(stores, operation_mark);
                        let mut frame = CommandEpisode::default();
                        frame.admit_expanding(expansion, false, cursor);
                        self.pending_diagnostic_operation = Some(PendingDiagnosticOperation {
                            operation,
                            destination: PendingDiagnosticDestination {
                                frame: OperationFrame::new(frame, ColdOperationSlot::default()),
                                barrier: None,
                            },
                        });
                    } else {
                        self.commit_direct_operation(stores, operation_mark);
                    }
                    return match result? {
                        StepResult::Suspended(need) => Ok(DiagnosticStepResult::Suspended(need)),
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
            if !tex_command::exceeds_max_non_prefixed_command(static_meaning(command.meaning())) {
                let step = DiagnosticStep::Token {
                    spelling: command.spelling(),
                    meaning: static_meaning(command.meaning()),
                    control_sequence: command.control_sequence(),
                    source_provenance,
                };
                self.commit_direct_operation(stores, operation_mark);
                return Ok(DiagnosticStepResult::Progress(step));
            }
            command_episode.admit_settled(command, Some(cursor));
            host_preparation.fill_delivery(OperationDelivery::Command, None, None);
        }
        let mode_mark = self.modes.begin_journal();
        let barrier = operation_barrier(host_preparation.delivery(), &command_episode);
        let applied = match self.execute_typed_operation(
            stores,
            &mut host_preparation,
            &mut diagnostic_effects,
            &mut command_episode,
            &mut cold_operation,
        ) {
            Err(TypedOperationError::Preparation(error)) => {
                command_episode.error = Some(error);
                assert!(
                    command_episode.alignment_scanner.is_none(),
                    "diagnostic retry cannot own an alignment scanner destination"
                );
                let unavailable = command_episode.has_unavailable(&cold_operation);
                let result =
                    self.finish_resource_preflight_failure(stores, command_episode.take_error());
                self.modes
                    .rollback_journal(mode_mark)
                    .expect("diagnostic assignment owns the mode mark");
                if matches!(result, Ok(StepResult::Suspended(_))) {
                    assert!(
                        unavailable || command_episode.has_preflight(),
                        "diagnostic resource suspension owns an exact retry"
                    );
                    let destination = PendingDiagnosticDestination {
                        frame: OperationFrame::new(command_episode, cold_operation),
                        barrier,
                    };
                    let operation = self.retain_direct_operation_for_retry(stores, operation_mark);
                    self.pending_diagnostic_operation = Some(PendingDiagnosticOperation {
                        operation,
                        destination,
                    });
                } else {
                    self.pending_diagnostic_operation = None;
                    self.commit_direct_operation(stores, operation_mark);
                }
                return match result? {
                    StepResult::Suspended(need) => Ok(DiagnosticStepResult::Suspended(need)),
                    StepResult::Progress(_) => {
                        unreachable!("diagnostic assignment failure made progress")
                    }
                };
            }
            Err(TypedOperationError::Application(error)) => Err(error),
            Ok(step) => Ok(step),
        };
        match applied {
            Ok(_) => {
                self.modes
                    .commit_journal(mode_mark)
                    .expect("diagnostic assignment owns the mode mark");
                self.commit_direct_operation(stores, operation_mark);
                stores
                    .world_mut()
                    .publish_diagnostic_effects(diagnostic_effects);
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
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<ReplayStep, ExecError> {
        let result = self.apply_operation(stores, settled, diagnostic_effects);
        if result.is_ok()
            && let Some(error) = self.operation_evidence_limit_error()
        {
            return Err(error);
        }
        result
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
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<ReplayStep, ExecError> {
        if matches!(
            self.modes.current_mode(),
            Mode::Vertical | Mode::InternalVertical
        ) {
            let mut context = stores.command_context().expect("live generation");
            start_paragraph(
                &mut self.command,
                &mut self.modes,
                &mut context,
                diagnostic_effects,
                true,
            )?;
        }
        let mut context = stores.command_context().expect("live generation");
        crate::box_runtime::flush_pending_hchars_with_fuel(
            &mut self.modes,
            &mut context,
            diagnostic_effects,
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
                diagnostic_effects,
                accent_font,
                char::from(accent),
                self.command_profile() == CommandProfile::ETEX26,
            );
            return Ok(ReplayStep::Continue);
        };
        drop(context);
        let base = self.do_assignments_then_accent_base(stores, diagnostic_effects)?;
        let accent_origin = scanned.accent_provenance.primary;
        let etex_extended = self.command_profile() == CommandProfile::ETEX26;
        let mut context = stores.command_context().expect("live generation");
        let mut geometry = pack_geometry_sink(&self.command, &mut self.operation_observations);
        apply_accent_nodes(
            &mut self.modes,
            &mut context,
            diagnostic_effects,
            &mut geometry,
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
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<Option<(u8, tex_state::token::OriginId)>, ExecError> {
        // None of §1270's assignments is a §1030 `main_loop` entry.
        self.main_loop_active = false;
        loop {
            let outcome = {
                let mut context = stores.command_context().expect("live generation");
                let mut host_facts = ExecutorHostFacts {
                    modes: &self.modes,
                    pdf_ignore_depth: self.pdf_ignore_depth,
                    telemetry: &mut self.episode_telemetry,
                };
                let mut processor = command_processor(
                    &mut self.command,
                    self.fuel.fuel_mut(),
                    &mut self.capabilities,
                    &mut host_facts,
                    &mut self.operation_observations,
                    diagnostic_effects,
                    &mut context,
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
                    let step =
                        self.execute_nested_operation(stores, Some(command), diagnostic_effects);
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
    fn fire_pending_page_output(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
        mut pending: PendingPageOutputFacts,
    ) -> Result<(), ExecError> {
        self.finish_pending_page_region_succession(stores);
        if self.page_region_succession_pending {
            // The active operation can still restore a mode root. Its commit
            // retries succession before this builder continuation resumes.
            return Ok(());
        }
        if self.boxes.output_routine_active || !pending.is_pending() {
            return Ok(());
        }
        while !self.boxes.output_routine_active {
            // TeX82 §§1012/1014--1015 returns from default `ship_out`
            // to the same §994 `build_page` invocation. Page-region
            // succession can defer that return until the contributing
            // operation's mode journal commits, so the page builder owns an
            // exact rollback-coupled continuation instead of borrowing the
            // backed-up command as a scheduler.
            let mut context = stores.command_context().expect("live generation");
            if pending.resume_after_output {
                assert!(
                    context.take_page_builder_resume_after_output(),
                    "captured page-builder continuation remains authoritative until selection"
                );
                crate::page_builder::build_page(
                    &mut context,
                    diagnostic_effects,
                    self.command.state(),
                )?;
                pending.fire_up = context.page_fire_up();
                pending.resume_after_output = false;
            }
            let selected = {
                let Some(fire_up) = pending.fire_up.take() else {
                    break;
                };
                let error_context = crate::diagnostics::ExecutionDiagnosticContext::source_free(
                    self.command.output_open_context(&context),
                );
                let mut geometry =
                    pack_geometry_sink(&self.command, &mut self.operation_observations);
                crate::page_output::select_pending_page_output(
                    &mut context,
                    diagnostic_effects,
                    &mut geometry,
                    fire_up,
                    error_context,
                )?
            };
            match selected {
                crate::page_output::SelectedPageOutput::Default(page) => {
                    // TeX82 §§1006/1012 finishes the contributing
                    // command and page-cost reports before the default
                    // `fire_up` path enters §638 `ship_out`. The reports are
                    // now fully detached and command admission is closed, so
                    // publish their ordered batch before the marker becomes
                    // host-visible. Deferred whatsit diagnostics are added by
                    // the nested shipout transaction at its own pre-commit
                    // boundary.
                    drop(context);
                    stores
                        .world_mut()
                        .publish_diagnostic_effects(std::mem::take(diagnostic_effects));
                    let mut command = CommandMachine {
                        state: &mut self.command,
                        fuel: self.fuel.fuel_mut(),
                        capabilities: &mut self.capabilities,
                        host_facts: CommandMachineHostFacts::Live(ExecutorHostFacts {
                            modes: &self.modes,
                            pdf_ignore_depth: self.pdf_ignore_depth,
                            telemetry: &mut self.episode_telemetry,
                        }),
                        observations: &mut self.operation_observations,
                        assignment_receipts: None,
                        diagnostic_effects,
                        shown_mode: &mut self.shown_mode,
                        initex: self.initex,
                        emit_dvi_override: self.emit_dvi_override,
                        immediate_prints: &mut self.immediate_prints,
                        prepared_shipout: &mut self.prepared_shipout,
                        pending_show_completion: None,
                        pending_outer_page_build_context: None,
                        output_routine_active: self.boxes.output_routine_active,
                    };
                    let publication = shipout_replay_box(page, stores, &mut command, &self.modes)?;
                    // Detached output no longer owns runtime nodes. Establish
                    // the complete next-page owner before allowing the old
                    // region to retire; the outer vertical mode is rootless
                    // throughout the default output path.
                    drop(command);
                    self.page_region_succession_pending = true;
                    self.finish_pending_page_region_succession(stores);
                    if let Some(receipt) = publication.and_then(|publication| publication.dvi) {
                        push_prepared_dvi_page(&mut self.prepared_dvi_pages, receipt);
                    }
                    if self.page_region_succession_pending {
                        break;
                    }
                    pending.resume_after_output = true;
                    continue;
                }
                crate::page_output::SelectedPageOutput::UserRoutine => {
                    let enclosing = self.operation_observations.take();
                    if enclosing.is_some() {
                        self.operation_observations = Some(ObservationBuffer::default());
                        publish_named_token_list_pushes(
                            &mut self.command,
                            &mut context,
                            diagnostic_effects,
                            &mut self.operation_observations,
                        );
                    }
                    let mut host_facts = ExecutorHostFacts {
                        modes: &self.modes,
                        pdf_ignore_depth: self.pdf_ignore_depth,
                        telemetry: &mut self.episode_telemetry,
                    };
                    let mut processor = command_processor(
                        &mut self.command,
                        self.fuel.fuel_mut(),
                        &mut self.capabilities,
                        &mut host_facts,
                        &mut self.operation_observations,
                        diagnostic_effects,
                        &mut context,
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
                    enter_group(
                        &mut context,
                        &mut self.command,
                        diagnostic_effects,
                        GroupKind::Output,
                    );
                    drop(context);
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

    /// Runs one live `math_choice_group` to its closing brace.
    ///
    /// TeX82 §1172/§1174 use
    /// ``push_math(math_choice_group); scan_left_brace``: the body is ordinary
    /// input that main control reads until §1174's `build_choices` sees its
    /// matching `}`. The mandatory brace has already been consumed by the
    /// scanner that requested this group; this branch path opens the
    /// save/mode levels and steps until that specific branch closes. Ordinary
    /// §1153 `math_group` fields instead return directly to the production
    /// main-control loop through [`Self::accept_math_field`].
    fn execute_live_math_choice_group(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
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
            diagnostic_effects,
            GroupKind::MathChoice,
        );
        self.modes.push_at_line(
            Mode::Math,
            self.command
                .current_file_line_number()
                .try_into()
                .unwrap_or(i32::MAX),
        )?;
        // This group opener and its body execute inside one outer command
        // operation. e-TeX [19.282] nevertheless prints the tracinggroups
        // entry synchronously before fetching the first body command. A body
        // `\message`, `\write`, or show completion writes through World, so
        // commit the already-complete detached entry before nested main
        // control can overtake it.
        stores
            .world_mut()
            .publish_diagnostic_effects(std::mem::take(diagnostic_effects));
        self.main_loop_active = false;
        while stores
            .command_context()
            .expect("live generation")
            .group_frames()
            .len()
            > enclosing_depth
        {
            let step = self.execute_nested_operation(stores, None, diagnostic_effects)?;
            // `execute_live_math_choice_group` fuses the body into its opener's
            // outer operation, but TeX finishes each nested command before
            // fetching the next one. Publish its completed diagnostic
            // program at that same boundary so a following immediate World
            // write cannot overtake tracing from `\vcenter`, math choices,
            // or any other nested group transition.
            stores
                .world_mut()
                .publish_diagnostic_effects(std::mem::take(diagnostic_effects));
            match step {
                ReplayStep::End | ReplayStep::EndOfInput => {
                    return Err(ExecError::MissingToken {
                        context: "math group closing brace",
                    });
                }
                ReplayStep::Continue => {}
            }
        }
        self.finish_math_level(stores, diagnostic_effects)
    }

    /// Closes any `\left` group TeX82 §1192 would have to recover, then pops
    /// the math mode level and finishes its mlist (§1184's `fin_mlist`).
    fn finish_math_level(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<tex_state::node_arena::PageListId, ExecError> {
        self.main_loop_active = false;
        while left_group_open(&self.modes, stores) {
            // The `\right.` applied below is exactly the closer §1065 selects
            // for `math_left_group`, so the report is §1064's `off_save`.
            let mut command_context = stores.command_context().expect("live generation");
            let context = self.command.output_open_context(&command_context);
            report_escaped_error(
                &mut command_context,
                diagnostic_effects,
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
                diagnostic_effects,
            )?;
        }
        let mut context = stores.command_context().expect("math-list admission");
        let mut level = crate::box_runtime::commit_current_list(
            &mut self.modes,
            &mut context,
            diagnostic_effects,
            self.fuel.fuel_mut(),
        )?;
        let (nodes, incomplete) = {
            let mut list = level.list_mutation();
            (list.take_nodes(), list.take_incomplete_fraction())
        };
        finish_math_list(nodes, incomplete, &mut context)
    }

    /// Opens and runs one `\mathchoice` branch: TeX82 §1172/§1174's
    /// ``push_math(math_choice_group); scan_left_brace`` followed by the live
    /// body main control reads until `build_choices` closes it.
    fn execute_math_choice_branch(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<tex_state::node_arena::PageListId, ExecError> {
        self.command_scan_math_choice_group(stores, diagnostic_effects)?;
        self.execute_live_math_choice_group(stores, diagnostic_effects)
    }

    /// Stores one completed TeX82 §1151 field or opens its live §1153 group.
    ///
    /// §1151 ends with `math_type(p):=math_char; character(p):=qi(c mod 256)`
    /// and §1151's own `fam` rule -- it never builds a noad, so `c`'s class
    /// bits are deliberately dropped here. The scalar case is a value, not
    /// deferred input: the command processor has already read, expanded, and
    /// classified everything the field consumed, so nothing is replayed and
    /// no input level is opened (`umber2-johp.265`).
    fn accept_math_field(
        &mut self,
        field: tex_command::MathFieldEpisode,
        target: ActiveMathFieldTarget,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<(), ExecError> {
        let field = match field.body {
            MathFieldBody::Missing => MathField::Empty,
            MathFieldBody::Character(code) => MathField::MathChar(
                math_char(
                    &stores.command_context().expect("live generation"),
                    u32::from(code),
                    field.provenance.primary,
                )?
                .1,
            ),
            MathFieldBody::OpenGroup => {
                // TeX82 §1153 returns to `main_control` immediately after
                // `push_math(math_group)`. The field body is not an inner
                // executor loop: each of its commands must therefore retain
                // the normal typed delivery/scanner/resource continuation.
                // `EndMathGroup` consumes this exact destination at §1186.
                enter_group(
                    &mut stores.command_context().expect("math-group admission"),
                    &mut self.command,
                    diagnostic_effects,
                    GroupKind::Math,
                );
                self.modes.push_at_line(
                    Mode::Math,
                    self.command
                        .current_file_line_number()
                        .try_into()
                        .unwrap_or(i32::MAX),
                )?;
                self.active_math_fields.push(target);
                self.main_loop_active = false;
                return Ok(());
            }
        };
        fill_math_field_target(
            &mut self.modes,
            &mut stores.command_context().expect("math-field admission"),
            target,
            field,
        );
        Ok(())
    }

    fn apply_math_request(
        &mut self,
        request: MathRequest,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<ReplayStep, ExecError> {
        match request {
            MathRequest::Character(value) => {
                append_math_char(
                    self.modes.current_list_mutation(),
                    &mut stores.command_context().expect("live generation"),
                    u32::from(value.code),
                    value.provenance.primary,
                )?;
            }
            MathRequest::Delimiter(value) => {
                append_math_char(
                    self.modes.current_list_mutation(),
                    &mut stores.command_context().expect("live generation"),
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
                let mut context = stores.command_context().expect("math text-field admission");
                let node_index = self.modes.current_list().nodes(&context).len();
                self.modes.current_list_mutation().push(
                    &mut context,
                    Node::MathNoad(MathNoad::new(noad_kind_for_text(kind), MathField::Empty)),
                );
                drop(context);
                let episode = self.command_scan_math_field(stores, diagnostic_effects)?;
                self.accept_math_field(
                    episode,
                    ActiveMathFieldTarget::Nucleus {
                        node_index,
                        simplify_accent: kind == MathTextFieldKind::Ord,
                    },
                    stores,
                    diagnostic_effects,
                )?;
            }
            MathRequest::Script(script) => {
                let target = reserve_script_target(
                    self.modes.current_list_mutation(),
                    stores,
                    diagnostic_effects,
                    script.kind,
                )?;
                let episode = self.command_scan_math_field(stores, diagnostic_effects)?;
                self.accept_math_field(
                    episode,
                    ActiveMathFieldTarget::Script(target),
                    stores,
                    diagnostic_effects,
                )?;
            }
            MathRequest::Limits(kind) => {
                if !apply_limits(
                    self.modes.current_list_mutation(),
                    &mut stores.command_context().expect("math limits admission"),
                    kind,
                ) {
                    // §1159 falls through to the error only when the tail is
                    // not an `op_noad`; the switch is dropped and the job
                    // continues.
                    let context = self
                        .command
                        .output_open_context(&stores.command_context().expect("live generation"));
                    let mut report = stores.print_err("Limit controls must follow a math operator");
                    report.help(&["I'm ignoring this misplaced \\limits or \\nolimits command."]);
                    report.context(context);
                    report.error().defer_recovery(diagnostic_effects)?;
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
                    report.error().defer_recovery(diagnostic_effects)?;
                }
            }
            MathRequest::Style(style) => self.modes.current_list_mutation().push(
                &mut stores.command_context().expect("math style admission"),
                Node::MathStyle(match style {
                    MathStyleKind::Display => MathStyle::Display,
                    MathStyleKind::Text => MathStyle::Text,
                    MathStyleKind::Script => MathStyle::Script,
                    MathStyleKind::ScriptScript => MathStyle::ScriptScript,
                }),
            ),
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
                    let display = self.execute_math_choice_branch(stores, diagnostic_effects)?;
                    *self
                        .active_math_choices
                        .last_mut()
                        .expect("live math choice") = 1;
                    let text = self.execute_math_choice_branch(stores, diagnostic_effects)?;
                    *self
                        .active_math_choices
                        .last_mut()
                        .expect("live math choice") = 2;
                    let script = self.execute_math_choice_branch(stores, diagnostic_effects)?;
                    *self
                        .active_math_choices
                        .last_mut()
                        .expect("live math choice") = 3;
                    let script_script =
                        self.execute_math_choice_branch(stores, diagnostic_effects)?;
                    Ok::<_, ExecError>((display, text, script, script_script))
                })();
                self.active_math_choices.pop();
                let (display, text, script, script_script) = branches?;
                self.modes.current_list_mutation().push(
                    &mut stores.command_context().expect("math choice admission"),
                    Node::MathChoice(MathChoice {
                        display,
                        text,
                        script,
                        script_script,
                    }),
                );
            }
            MathRequest::Radical(delimiter) => {
                let mut context = stores.command_context().expect("math radical admission");
                let node_index = self.modes.current_list().nodes(&context).len();
                self.modes.current_list_mutation().push(
                    &mut context,
                    Node::MathNoad(MathNoad::new(
                        NoadKind::Radical {
                            delimiter: delimiter.code,
                        },
                        MathField::Empty,
                    )),
                );
                drop(context);
                let episode = self.command_scan_math_field(stores, diagnostic_effects)?;
                self.accept_math_field(
                    episode,
                    ActiveMathFieldTarget::Nucleus {
                        node_index,
                        simplify_accent: false,
                    },
                    stores,
                    diagnostic_effects,
                )?;
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
                    report.error().defer_recovery(diagnostic_effects)?;
                    self.apply_error_stop_transition(stores, diagnostic_effects)?;
                    self.command_scan_math_character(stores, diagnostic_effects)?
                };
                let accent = math_char(
                    &stores.command_context().expect("live generation"),
                    u32::from(accent.code),
                    accent.provenance.primary,
                )?
                .1;
                let mut context = stores.command_context().expect("math accent admission");
                let node_index = self.modes.current_list().nodes(&context).len();
                self.modes.current_list_mutation().push(
                    &mut context,
                    Node::MathNoad(MathNoad::new(NoadKind::Accent { accent }, MathField::Empty)),
                );
                drop(context);
                let episode = self.command_scan_math_field(stores, diagnostic_effects)?;
                self.accept_math_field(
                    episode,
                    ActiveMathFieldTarget::Nucleus {
                        node_index,
                        simplify_accent: false,
                    },
                    stores,
                    diagnostic_effects,
                )?;
            }
            MathRequest::MuMaterial(ScannedMathMuMaterial::Glue(glue)) => {
                self.modes.current_list_mutation().push(
                    &mut stores.command_context().expect("math glue admission"),
                    Node::Glue {
                        spec: glue,
                        kind: GlueKind::MuSkip,
                        leader: None,
                    },
                )
            }
            MathRequest::MuMaterial(ScannedMathMuMaterial::Kern(amount)) => {
                self.modes.current_list_mutation().push(
                    &mut stores.command_context().expect("math kern admission"),
                    Node::Kern {
                        amount,
                        kind: KernKind::Mu,
                    },
                )
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
                        diagnostic_effects,
                        token,
                        mode,
                        Some(context),
                    )?;
                } else {
                    let display = take_finished_math_list(&mut self.modes, stores)?;
                    let mut context = stores.command_context().expect("live generation");
                    enter_group(
                        &mut context,
                        &mut self.command,
                        diagnostic_effects,
                        GroupKind::MathShift,
                    );
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
                    let display = context
                        .admit_page_node_span(display)
                        .expect("equation-number display belongs to the live page owner");
                    self.modes
                        .current_list_mutation()
                        .set_display_eq_no(&context, crate::mode::DisplayEqNo { side, display });
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
        diagnostic_effects: &mut DiagnosticEffects,
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
            diagnostic_effects,
            "Missing $$ inserted",
            &[
                "Displays can use special alignments (like \\eqalignno)",
                "only if nothing but the alignment itself is between $$'s.",
            ],
            context,
        )?;
        self.finish_display_alignment(
            stores,
            diagnostic_effects,
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
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<ReplayStep, ExecError> {
        match self.modes.current_mode() {
            Mode::Horizontal | Mode::RestrictedHorizontal => {
                debug_assert_ne!(pairing, MathShiftPairing::ProbeDisplayEnd);
                crate::box_runtime::flush_pending_hchars_with_fuel(
                    &mut self.modes,
                    &mut stores.command_context().expect("math-shift admission"),
                    diagnostic_effects,
                    self.fuel.fuel_mut(),
                )?;
                // §1138 already applied its own `mode>0` test while probing:
                // in restricted horizontal mode the second `$` was backed up
                // rather than consumed, so `paired` is false there and this
                // must not retest the mode and disagree with the backup.
                if pairing == MathShiftPairing::Paired {
                    self.enter_display(stores, diagnostic_effects)?;
                } else {
                    self.enter_math_level(false, stores, diagnostic_effects)?;
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
                    let content = self.prepare_math_list(stores, diagnostic_effects)?;
                    let eq = self.finish_equation_number_mlist(stores, diagnostic_effects)?;
                    let paired = self.scan_display_end(stores, diagnostic_effects)?;
                    if !paired {
                        report_unpaired_display_end(&self.command, diagnostic_effects, stores)?;
                    }
                    let (display, finished) =
                        self.finish_equation_number_group(stores, diagnostic_effects, eq, content)?;
                    self.finish_display_math_content(
                        stores,
                        diagnostic_effects,
                        display,
                        Some(finished),
                        false,
                        None,
                    )?;
                } else {
                    debug_assert_eq!(pairing, MathShiftPairing::Unpaired);
                    self.finish_inline_math(stores, diagnostic_effects)?;
                }
            }
            Mode::DisplayMath => {
                debug_assert_eq!(pairing, MathShiftPairing::ProbeDisplayEnd);
                let display_alignment = self.modes.current_list_mutation().take_display_alignment();
                if let Some((nodes, aux_prev_depth)) = display_alignment {
                    let paired = self.scan_display_end(stores, diagnostic_effects)?;
                    if !paired {
                        report_unpaired_display_end(&self.command, diagnostic_effects, stores)?;
                    }
                    self.finish_display_alignment(
                        stores,
                        diagnostic_effects,
                        crate::align::FinishedAlignment {
                            nodes,
                            aux_prev_depth,
                            aux_space_factor: None,
                        },
                    )?;
                    return Ok(ReplayStep::Continue);
                }
                let (content, display_level) =
                    self.prepare_display_math_list(stores, diagnostic_effects)?;
                let paired = self.scan_display_end(stores, diagnostic_effects)?;
                if !paired {
                    report_unpaired_display_end(&self.command, diagnostic_effects, stores)?;
                }
                self.finish_display_math_content(
                    stores,
                    diagnostic_effects,
                    content,
                    None,
                    true,
                    Some(display_level),
                )?;
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
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<tex_state::node_arena::PageListId, ExecError> {
        let math_font_context = self
            .command
            .output_open_context(&stores.command_context().expect("math-font admission"));
        let rejected = crate::math::reject_invalid_math_fonts_at_outer_barrier(
            stores,
            diagnostic_effects,
            math_font_context,
        )?;
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
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<
        (
            tex_state::node_arena::PageListId,
            crate::mode::ModeLevelSummary,
        ),
        ExecError,
    > {
        let content = self.prepare_math_list(stores, diagnostic_effects)?;
        let level = crate::box_runtime::commit_current_list(
            &mut self.modes,
            &mut stores.command_context().expect("math-list admission"),
            diagnostic_effects,
            self.fuel.fuel_mut(),
        )?;
        Ok((content, level))
    }

    fn scan_display_end(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<bool, ExecError> {
        // TeX82 §§1185/1194's `fin_mlist` has already popped the display
        // level. Publish that live nest before §1197's nested expansion
        // episode: capabilities were last sampled at the start of the outer
        // main-control step, when the current mode was still display math.
        self.ensure_primitive_handles(stores);
        let mode = self.modes.current_mode();
        let shown_mode = self.shown_mode;
        let mut context = stores
            .command_context()
            .expect("display-end scan requires a live generation");
        let mut machine = self.command_machine(diagnostic_effects);
        let mut processor = machine.processor(&mut context);
        prepare_command_trace(&mut processor, mode, shown_mode);
        let paired = processor
            .scan_display_end_math_shift()
            .map_err(command_error)?;
        let command_trace_printed = processor.command_trace_printed();
        drop(processor);
        if command_trace_printed {
            *machine.shown_mode = Some(mode);
        }
        drop(machine);
        drop(context);
        Ok(paired)
    }

    fn enter_math_level(
        &mut self,
        display: bool,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<(), ExecError> {
        let mut context = stores.command_context().expect("live generation");
        enter_group(
            &mut context,
            &mut self.command,
            diagnostic_effects,
            GroupKind::MathShift,
        );
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

    fn enter_display(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<(), ExecError> {
        let (paragraph, dimensions, pre_display_size, prototype, extended) = {
            let mut context = stores.command_context().expect("live generation");
            // TeX82 §1138 interrupts the paragraph while the opening display
            // shift is still the current command. Section 661 therefore uses
            // this live input `line` as the paragraph's ending line; a
            // source-free detached context would incorrectly print line zero.
            let error_context = crate::diagnostics::ExecutionDiagnosticContext::new(
                self.command
                    .current_file_line_number()
                    .try_into()
                    .unwrap_or(i32::MAX),
                0,
                false,
                self.command.output_open_context(&context),
            );
            let mut geometry = pack_geometry_sink(&self.command, &mut self.operation_observations);
            let paragraph = crate::paragraph_end::interrupt_paragraph_for_display(
                &mut self.modes,
                &mut context,
                diagnostic_effects,
                &mut geometry,
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
                paragraph
                    .last_line
                    .as_ref()
                    .map(|line| crate::math::display::display_line_prototype(&mut context, *line))
            } else {
                None
            };
            (paragraph, dimensions, pre_display_size, prototype, extended)
        };
        // TeX82 §1145 opens `math_shift_group` before these local parameter
        // definitions, so §283 restores all of them when the display ends.
        // `\everydisplay` is scheduled only after the definitions are live.
        self.enter_math_level(true, stores, diagnostic_effects)?;
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
        self.modes.current_list_mutation().set_display_interrupt(
            &context,
            crate::mode::DisplayInterrupt::new(&context, paragraph.active_directions, prototype),
        );
        Ok(())
    }

    fn finish_inline_math(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<(), ExecError> {
        let mut content = take_finished_math_list(&mut self.modes, stores)?;
        let diagnostic_text = self
            .command
            .output_open_context(&stores.command_context().expect("inline-math admission"));
        let conversion_error_context =
            crate::math::MathConversionErrorContext::new(diagnostic_text.clone());
        if crate::math::reject_invalid_math_fonts_at_outer_barrier(
            stores,
            diagnostic_effects,
            diagnostic_text,
        )? {
            content = tex_state::node_arena::PageListId::empty();
        }
        let mut context = stores.command_context().expect("inline-math admission");
        let _ = crate::box_runtime::commit_current_list(
            &mut self.modes,
            &mut context,
            diagnostic_effects,
            self.fuel.fuel_mut(),
        )?;
        let insert_penalties = self.modes.current_mode() == Mode::Horizontal;
        let mut geometry = pack_geometry_sink(&self.command, &mut self.operation_observations);
        let (nodes, _) = crate::math::finish_inline_math_list_node(
            &mut context,
            diagnostic_effects,
            &mut geometry,
            tex_state::math::MathListNode {
                display: false,
                content,
            },
            insert_penalties,
            conversion_error_context,
        );
        let nodes = context.reclaim_unique_page_list(nodes);
        self.modes
            .current_list_mutation()
            .append_unique_list(&mut context, nodes);
        self.modes.current_list_mutation().set_space_factor(1000);
        let aftergroup = leave_group_payloads(
            &mut context,
            &mut self.command,
            diagnostic_effects,
            GroupKind::MathShift,
        )
        .map_err(|_| ExecError::MissingToken {
            context: "math shift group",
        })?;
        self.active_math_shifts.pop();
        schedule_aftergroup(
            &mut self.command_machine(diagnostic_effects),
            &mut context,
            aftergroup,
        )?;
        Ok(())
    }

    fn finish_equation_number_mlist(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<crate::mode::DisplayEqNo, ExecError> {
        let mut level = crate::box_runtime::commit_current_list(
            &mut self.modes,
            &mut stores.command_context().expect("equation-number admission"),
            diagnostic_effects,
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
        diagnostic_effects: &mut DiagnosticEffects,
        eq: crate::mode::DisplayEqNo,
        content: tex_state::node_arena::PageListId,
    ) -> Result<
        (
            tex_state::node_arena::PageListId,
            crate::math::display::FinishedEqNo,
        ),
        ExecError,
    > {
        let mut context = stores.command_context().expect("equation-number admission");
        let diagnostic_text = self.command.output_open_context(&context);
        let conversion_error_context =
            crate::math::MathConversionErrorContext::new(diagnostic_text.clone());
        let diagnostic_context = crate::diagnostics::ExecutionDiagnosticContext::new(
            self.command
                .current_file_line_number()
                .try_into()
                .unwrap_or(i32::MAX),
            0,
            false,
            diagnostic_text,
        );
        let mut geometry = pack_geometry_sink(&self.command, &mut self.operation_observations);
        let finished = crate::math::display::finish_eq_no(
            &mut context,
            diagnostic_effects,
            &mut geometry,
            &diagnostic_context,
            eq.side,
            content,
            Some(&conversion_error_context),
        );
        let aftergroup = leave_group_payloads(
            &mut context,
            &mut self.command,
            diagnostic_effects,
            GroupKind::MathShift,
        )
        .map_err(|_| ExecError::MissingToken {
            context: "equation number group",
        })?;
        self.active_math_shifts.pop();
        schedule_aftergroup(
            &mut self.command_machine(diagnostic_effects),
            &mut context,
            aftergroup,
        )?;
        Ok((eq.display.list(), finished))
    }

    fn finish_display_alignment(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
        finished: crate::align::FinishedAlignment,
    ) -> Result<(), ExecError> {
        self.finish_display_alignment_inner(stores, diagnostic_effects, finished, true)
    }

    fn finish_display_alignment_inner(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
        finished: crate::align::FinishedAlignment,
        scan_optional_space: bool,
    ) -> Result<(), ExecError> {
        let mut context = stores
            .command_context()
            .expect("display-alignment admission");
        let mut level = crate::box_runtime::commit_current_list(
            &mut self.modes,
            &mut context,
            diagnostic_effects,
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
        let aftergroup = leave_group_payloads(
            &mut context,
            &mut self.command,
            diagnostic_effects,
            GroupKind::MathShift,
        )
        .map_err(|_| ExecError::MissingToken {
            context: "display alignment group",
        })?;
        self.active_math_shifts.pop();
        schedule_aftergroup(
            &mut self.command_machine(diagnostic_effects),
            &mut context,
            aftergroup,
        )?;
        drop(context);
        self.resume_display_inner(
            stores,
            diagnostic_effects,
            interrupt.active_directions,
            scan_optional_space,
        )
    }

    fn finish_display_math_content(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
        mut content: tex_state::node_arena::PageListId,
        eq_no: Option<crate::math::display::FinishedEqNo>,
        fonts_checked: bool,
        display_level: Option<crate::mode::ModeLevelSummary>,
    ) -> Result<(), ExecError> {
        let diagnostic_text = self
            .command
            .output_open_context(&stores.command_context().expect("display-math admission"));
        let conversion_error_context =
            crate::math::MathConversionErrorContext::new(diagnostic_text.clone());
        // TeX82 §§1195--1196 converts and packs the display while the
        // closing math shift is still current, so §661's `line` is the
        // shift's live input line rather than a source-free zero.
        let diagnostic_context = crate::diagnostics::ExecutionDiagnosticContext::new(
            self.command
                .current_file_line_number()
                .try_into()
                .unwrap_or(i32::MAX),
            0,
            false,
            diagnostic_text.clone(),
        );
        // TeX82 §1194 performs this check before every display `fin_mlist`,
        // including the saved outer mlist after an equation number.
        if !fonts_checked
            && crate::math::reject_invalid_math_fonts_at_outer_barrier(
                stores,
                diagnostic_effects,
                diagnostic_text,
            )?
        {
            content = tex_state::node_arena::PageListId::empty();
        }
        let mut context = stores.command_context().expect("display-math admission");
        let mut level = match display_level {
            Some(level) => level,
            None => crate::box_runtime::commit_current_list(
                &mut self.modes,
                &mut context,
                diagnostic_effects,
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
        let (active_directions, prototype) = interrupt.into_parts();
        let mut geometry = pack_geometry_sink(&self.command, &mut self.operation_observations);
        crate::math::display::finish_display_math(
            &mut self.modes,
            &mut context,
            diagnostic_effects,
            &mut geometry,
            &diagnostic_context,
            content,
            eq_no,
            prototype,
            Some(&conversion_error_context),
        )?;
        let aftergroup = leave_group_payloads(
            &mut context,
            &mut self.command,
            diagnostic_effects,
            GroupKind::MathShift,
        )
        .map_err(|_| ExecError::MissingToken {
            context: "display math group",
        })?;
        self.active_math_shifts.pop();
        schedule_aftergroup(
            &mut self.command_machine(diagnostic_effects),
            &mut context,
            aftergroup,
        )?;
        drop(context);
        self.resume_display(stores, diagnostic_effects, active_directions)
    }

    fn resume_display(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
        directions: Vec<tex_state::node::Direction>,
    ) -> Result<(), ExecError> {
        self.resume_display_inner(stores, diagnostic_effects, directions, true)
    }

    fn resume_display_inner(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
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
        self.modes.current_list_mutation().append(
            &mut stores
                .command_context()
                .expect("display direction admission"),
            directions.into_iter().map(Node::Direction),
        );
        if scan_optional_space {
            self.scan_optional_space(stores, diagnostic_effects)?;
        }
        let mut context = stores.command_context().expect("display-resume admission");
        let error_context = self.command.output_open_context(&context);
        crate::math::display::build_page_after_display_resume(
            &self.modes,
            &mut context,
            diagnostic_effects,
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
    fn scan_optional_space(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<(), ExecError> {
        let mode = self.modes.current_mode();
        let shown_mode = self.shown_mode;
        let mut context = stores
            .command_context()
            .expect("optional-space scan requires a live generation");
        let mut machine = self.command_machine(diagnostic_effects);
        let mut processor = machine.processor(&mut context);
        let mut diagnostics = Vec::new();
        // TeX82 §§299/1200: resume_after_display has already pushed the new
        // horizontal mode when its scanner expands this token. The expansion
        // therefore owns the same pending mode prefix as every other
        // get_x_token boundary, including §1197's staged display-end probe.
        prepare_command_trace(&mut processor, mode, shown_mode);
        let mut destination = None;
        let fetched = processor.get_x_token_into(&mut destination);
        let command_trace_printed = processor.command_trace_printed();
        match fetched {
            Ok(tex_command::DeliveryStatus::Command)
                if !matches!(
                    destination
                        .as_ref()
                        .expect("command status initializes destination")
                        .meaning(),
                    ResolvedMeaning::Static(Meaning::CharToken {
                        cat: Catcode::Space,
                        ..
                    })
                ) =>
            {
                let command = destination
                    .take()
                    .expect("command status initializes destination");
                processor.back_input(command).map_err(command_error)?;
            }
            Ok(tex_command::DeliveryStatus::End | tex_command::DeliveryStatus::Command) => {}
            Ok(_) => unreachable!("ordinary expanded delivery returns only commands"),
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
        drop(machine);
        drop(context);
        // §1200 performs this expanded fetch synchronously before §1125's
        // page builder. Diagnostics produced by expansion therefore belong
        // to this nested scanner boundary; leaving them on CommandState<G> lets
        // the following outer main-control step report them only after
        // build_page has emitted its tracingpages state.
        self.capture_first_causal_context(stores, &diagnostics);
        report_pending_diagnostics(stores, diagnostic_effects, diagnostics)
    }

    fn apply_math_delimiter(
        &mut self,
        boundary: MathDelimiterBoundary,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
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
                    diagnostic_effects,
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
                self.modes.current_list_mutation().push(
                    &mut stores.command_context().expect("math-left admission"),
                    Node::MathNoad(MathNoad::new(
                        NoadKind::LeftDelimiter {
                            delimiter: boundary.delimiter.code,
                        },
                        MathField::Empty,
                    )),
                );
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
                    let mut context = stores.command_context().expect("math-middle admission");
                    let _ = crate::box_runtime::commit_current_list(
                        &mut self.modes,
                        &mut context,
                        diagnostic_effects,
                        self.fuel.fuel_mut(),
                    )?;
                    let aftergroup = leave_group_payloads(
                        &mut context,
                        &mut self.command,
                        diagnostic_effects,
                        GroupKind::MathLeft,
                    )
                    .map_err(|_| ExecError::MissingToken {
                        context: "math left group",
                    })?;
                    self.active_math_left_boundaries.pop();
                    schedule_aftergroup(
                        &mut self.command_machine(diagnostic_effects),
                        &mut context,
                        aftergroup,
                    )?;

                    enter_group(
                        &mut context,
                        &mut self.command,
                        diagnostic_effects,
                        GroupKind::MathLeft,
                    );
                    self.modes.push_at_line(
                        Mode::Math,
                        self.command
                            .current_file_line_number()
                            .try_into()
                            .unwrap_or(i32::MAX),
                    )?;
                    self.active_math_left_boundaries.push(true);
                    let content = context.reclaim_unique_page_list(content);
                    let mut list = self.modes.current_list_mutation();
                    list.append_unique_list(&mut context, content);
                    list.push(
                        &mut context,
                        Node::MathNoad(MathNoad::new(
                            NoadKind::MiddleDelimiter {
                                delimiter: boundary.delimiter.code,
                            },
                            MathField::Empty,
                        )),
                    );
                } else {
                    // etex.ch [48.1192] splits §1192's report by noad type.
                    let mut command_context = stores.command_context().expect("live generation");
                    let context = self.command.output_open_context(&command_context);
                    report_escaped_error(
                        &mut command_context,
                        diagnostic_effects,
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
                        diagnostic_effects,
                        "Extra ",
                        "right",
                        "",
                        &["I'm ignoring a \\right that had no matching \\left."],
                        context,
                    )?;
                    return Ok(ReplayStep::Continue);
                }
                let content = take_finished_math_list(&mut self.modes, stores)?;
                let mut context = stores.command_context().expect("math-right admission");
                let _ = crate::box_runtime::commit_current_list(
                    &mut self.modes,
                    &mut context,
                    diagnostic_effects,
                    self.fuel.fuel_mut(),
                )?;
                let aftergroup = leave_group_payloads(
                    &mut context,
                    &mut self.command,
                    diagnostic_effects,
                    GroupKind::MathLeft,
                )
                .map_err(|_| ExecError::MissingToken {
                    context: "math left group",
                })?;
                self.active_math_left_boundaries.pop();
                schedule_aftergroup(
                    &mut self.command_machine(diagnostic_effects),
                    &mut context,
                    aftergroup,
                )?;
                let boundary = context.publish_page_nodes(vec![Node::MathNoad(MathNoad::new(
                    NoadKind::RightDelimiter {
                        delimiter: boundary.delimiter.code,
                    },
                    MathField::Empty,
                ))]);
                let content = context.compose_page_node_sequences(&[content, boundary]);
                self.modes.current_list_mutation().push(
                    &mut context,
                    Node::MathNoad(MathNoad::new(
                        NoadKind::Normal(NoadClass::Inner),
                        MathField::SubMlist(content),
                    )),
                );
            }
        }
        Ok(ReplayStep::Continue)
    }

    fn command_scan_math_field(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<tex_command::MathFieldEpisode, ExecError> {
        let mut context = stores.command_context().expect("live generation");
        let mut host_facts = ExecutorHostFacts {
            modes: &self.modes,
            pdf_ignore_depth: self.pdf_ignore_depth,
            telemetry: &mut self.episode_telemetry,
        };
        let mut processor = command_processor(
            &mut self.command,
            self.fuel.fuel_mut(),
            &mut self.capabilities,
            &mut host_facts,
            &mut self.operation_observations,
            diagnostic_effects,
            &mut context,
        );
        let scanned = processor.scan_math_field_episode();
        scanned.map_err(command_error)
    }

    /// Runs TeX82 §436's `scan_fifteen_bit_int` after an executor-owned
    /// diagnostic has completed, as required by §1110's text-accent path.
    fn command_scan_math_character(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<tex_command::ScannedMathCharacter, ExecError> {
        let mut context = stores.command_context().expect("live generation");
        let mut host_facts = ExecutorHostFacts {
            modes: &self.modes,
            pdf_ignore_depth: self.pdf_ignore_depth,
            telemetry: &mut self.episode_telemetry,
        };
        let mut processor = command_processor(
            &mut self.command,
            self.fuel.fuel_mut(),
            &mut self.capabilities,
            &mut host_facts,
            &mut self.operation_observations,
            diagnostic_effects,
            &mut context,
        );
        processor.scan_math_character().map_err(command_error)
    }

    /// TeX82 §1172/§1174's `scan_left_brace` for one `\mathchoice` branch.
    /// §403 recovery opens the group anyway, so the recovered flag is
    /// diagnostic only.
    fn command_scan_math_choice_group(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<bool, ExecError> {
        let mut context = stores.command_context().expect("live generation");
        let mut host_facts = ExecutorHostFacts {
            modes: &self.modes,
            pdf_ignore_depth: self.pdf_ignore_depth,
            telemetry: &mut self.episode_telemetry,
        };
        let mut processor = command_processor(
            &mut self.command,
            self.fuel.fuel_mut(),
            &mut self.capabilities,
            &mut host_facts,
            &mut self.operation_observations,
            diagnostic_effects,
            &mut context,
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
        if self.terminal_revision_step.is_some() {
            return Err(ExecError::ExecutionAlreadyTerminated);
        }
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
            OperationDelivery::Replay,
            OperationTransaction::Advance,
            1,
            None,
        );
        let mut pending = self.operation_observations.take().unwrap_or_default();
        let receipt_start = self.operation_receipt_start.take();
        if matches!(stepped, Ok(StepResult::Suspended(_)))
            && (self.pending_resource_operation.is_some()
                || self.pending_direct_operation.is_some())
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
    /// Returns the effect cursor immediately before final-cleanup framing.
    #[must_use]
    pub const fn job_body_effect_end(&self) -> Option<tex_state::EffectPos> {
        self.job_body_effect_end
    }

    fn apply_operation(
        &mut self,
        stores: &mut Universe<G>,
        command: Option<tex_command::CurrentCommand<G>>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<ReplayStep, ExecError> {
        let mut frame = CommandEpisode::default();
        let mut cold = ColdOperationSlot::default();
        let delivery = if let Some(command) = command {
            frame.admit_settled(command, None);
            OperationDelivery::Command
        } else {
            OperationDelivery::Replay
        };
        let mut preparation_scope = OperationPreparationScope;
        let mut host_preparation = OperationPreparation::new(&mut preparation_scope);
        host_preparation.fill_delivery(delivery, None, None);
        let result = self.execute_typed_operation(
            stores,
            &mut host_preparation,
            diagnostic_effects,
            &mut frame,
            &mut cold,
        );
        self.settle_save_stack_usage(stores, &mut host_preparation);
        result.map_err(TypedOperationError::into_exec_error)
    }

    fn apply_error_stop_transition(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<(), ExecError> {
        let Some(request) = diagnostic_effects.take_error_stop_recovery() else {
            return Ok(());
        };
        let mut context = stores
            .command_context()
            .expect("error-stop transition has a live generation");
        let mut host_facts = ExecutorHostFacts {
            modes: &self.modes,
            pdf_ignore_depth: self.pdf_ignore_depth,
            telemetry: &mut self.episode_telemetry,
        };
        let mut processor = command_processor(
            &mut self.command,
            self.fuel.fuel_mut(),
            &mut self.capabilities,
            &mut host_facts,
            &mut self.operation_observations,
            diagnostic_effects,
            &mut context,
        );
        processor.set_output_routine_active(self.boxes.output_routine_active);
        processor
            .apply_error_stop_recovery(request)
            .map_err(command_error)
    }

    /// Completes one canonically delivered operation. Common unexpandable
    /// families scan and apply here without a
    /// universal DTO; cold and barrier families enter a borrow-typed execution
    /// episode immediately after immutable resource resolution.
    fn execute_typed_operation(
        &mut self,
        stores: &mut Universe<G>,
        host_preparation: &mut OperationPreparation<'_, G>,
        diagnostic_effects: &mut DiagnosticEffects,
        frame: &mut CommandEpisode<G>,
        cold: &mut ColdOperationSlot<G>,
    ) -> Result<ReplayStep, TypedOperationError> {
        let result = self.dispatch_typed_operation(
            stores,
            host_preparation,
            diagnostic_effects,
            frame,
            cold,
        );
        if result.is_ok() {
            self.apply_error_stop_transition(stores, diagnostic_effects)
                .map_err(TypedOperationError::Application)?;
            frame.clear_preflight();
            frame.clear_operation_origin();
        }
        result
    }

    fn dispatch_typed_operation(
        &mut self,
        stores: &mut Universe<G>,
        host_preparation: &mut OperationPreparation<'_, G>,
        diagnostic_effects: &mut DiagnosticEffects,
        frame: &mut CommandEpisode<G>,
        cold: &mut ColdOperationSlot<G>,
    ) -> Result<ReplayStep, TypedOperationError> {
        let delivery = host_preparation.take_delivery();
        let scanner_resume = host_preparation.take_scanner();
        let expansion_resume = host_preparation.take_expansion();
        if matches!(
            &delivery,
            OperationDelivery::SuspendedCold { .. } | OperationDelivery::ResidentCold
        ) {
            assert!(
                frame.has_unavailable(cold)
                    && frame.error.is_none()
                    && frame.alignment_scanner.is_none(),
                "prepared delivery resumes the exact occupied operation frame; its scalar retry phase may retain the command cursor"
            );
        } else if matches!(&delivery, OperationDelivery::Command) {
            frame.assert_command_only();
        } else if matches!(&delivery, OperationDelivery::ResidentHot) {
            frame.assert_hot_only();
        } else {
            frame.assert_empty();
        }
        let mode = self.modes.current_mode();
        let outer_paragraph_was_active = mode == Mode::Horizontal && self.modes.depth() == 2;
        let source_role = frame.operation_source_role();
        if matches!(delivery, OperationDelivery::ResidentHot) {
            let applied = self.apply_hot_operation(
                stores,
                host_preparation,
                diagnostic_effects,
                frame.hot_mut(),
                OperationOutputStart {
                    outer_paragraph_was_active,
                    source_role,
                    artifact_count: stores.world().artifact_commits().len(),
                    effect_count: stores.world().effect_records().len(),
                    prepared_page_count: self.prepared_dvi_pages.len(),
                },
            );
            frame.hot = None;
            return applied.map_err(TypedOperationError::Application);
        }
        if matches!(
            delivery,
            OperationDelivery::ResidentCold | OperationDelivery::SuspendedCold { .. }
        ) {
            return self.execute_scanned_cold_episode(
                stores,
                host_preparation,
                diagnostic_effects,
                frame,
                cold,
                OperationOutputStart {
                    outer_paragraph_was_active,
                    source_role,
                    artifact_count: stores.world().artifact_commits().len(),
                    effect_count: stores.world().effect_records().len(),
                    prepared_page_count: self.prepared_dvi_pages.len(),
                },
            );
        }
        let tracked_region_is_active = stores
            .command_context()
            .is_ok_and(|context| context.tracked_region_is_active());
        if tracked_region_is_active {
            let mode_fingerprint = self.modes.semantic_fingerprint(stores);
            let mut context = stores
                .command_context()
                .expect("tracked region keeps its generation admitted");
            let mode_key = DependencyKey::Engine(DependencyEngineField::Mode);
            let inner_key = DependencyKey::Engine(DependencyEngineField::InnerMode);
            let last_node_key = DependencyKey::Engine(DependencyEngineField::LastNodeType);
            // Executor-owned mode facts have no state-layer mutation facade.
            // Advance their conservative generation once per observed outer
            // operation so validation always compares the canonical value
            // after another operation has had a chance to mutate the nest.
            let mut host_facts = ExecutorHostFacts {
                modes: &self.modes,
                pdf_ignore_depth: self.pdf_ignore_depth,
                telemetry: &mut self.episode_telemetry,
            };
            let last_node_type =
                tex_command::CommandHostFacts::last_node_type(&mut host_facts, &context);
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
            let (group_level, group_type) = context.current_group_values();
            context.observe_changed_command_projection(
                DependencyKey::Engine(DependencyEngineField::GroupLevel),
                DependencyValue::Integer(i64::from(group_level)),
            );
            context.observe_changed_command_projection(
                DependencyKey::Engine(DependencyEngineField::GroupType),
                DependencyValue::Integer(i64::from(group_type)),
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
        self.ensure_primitive_handles(stores);
        let mut context = stores.command_context().expect("live generation");
        if matches!(&delivery, OperationDelivery::Replay) && self.enter_main_control(&mut context) {
            // §1030's prologue precedes `big_switch`, so its push is published
            // ahead of the first command this step delivers rather than with
            // the step's own applied records.
            publish_named_token_list_pushes(
                &mut self.command,
                &mut context,
                diagnostic_effects,
                &mut self.operation_observations,
            );
        }
        let alignment_preamble = alignment_preamble(self.active_alignment.as_mut());
        let (innermost_group, job_is_all_over) = (
            context.innermost_group_kind(),
            crate::page_output::job_is_all_over(&context),
        );
        let mut diagnostics = Vec::new();
        let scanned = {
            #[cfg(feature = "profiling")]
            tex_state::measurement::record_hot_core_phase(
                tex_state::measurement::HotCorePhase::DeliveryAndScan,
            );
            #[cfg(feature = "profiling")]
            let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
                tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan,
            );
            let mut host_facts = ExecutorHostFacts {
                modes: &self.modes,
                pdf_ignore_depth: self.pdf_ignore_depth,
                telemetry: &mut self.episode_telemetry,
            };
            let mut processor = command_processor(
                &mut self.command,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
                &mut host_facts,
                &mut self.operation_observations,
                diagnostic_effects,
                &mut context,
            );
            let scanner_resume = if matches!(&delivery, OperationDelivery::Command) {
                frame.scanner.take()
            } else {
                scanner_resume
            };
            processor.install_scanner_resume(scanner_resume);
            if let Some(expansion) = expansion_resume {
                processor.install_expansion_resume(expansion);
            }
            processor.set_output_routine_active(self.boxes.output_routine_active);
            let display_alignment_tail = matches!(&delivery, OperationDelivery::Replay)
                && mode == Mode::DisplayMath
                && self.modes.current_list().has_display_alignment();
            let scanned = (|| -> Result<ScannedOperation, ExecError> {
                Ok(match delivery {
                    OperationDelivery::Command => scan_preflight_command(
                        &mut processor,
                        frame,
                        cold,
                        mode,
                        &self.boxes,
                        innermost_group,
                        job_is_all_over,
                        self.modes.current_list().display_eq_no().is_some(),
                        &mut self.shown_mode,
                        &mut diagnostics,
                    )?,
                    OperationDelivery::Replay if display_alignment_tail => {
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
                                    frame.admit_settled(command, None);
                                    dispatch_main_control_command(
                                        &mut processor,
                                        frame,
                                        cold,
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
                                    retain_cold_operation(
                                        frame,
                                        cold,
                                        ColdOperation::<G>::DisplayAlignmentRecovery,
                                    )
                                }
                            },
                            None => {
                                retain_cold_operation(frame, cold, ColdOperation::<G>::EndOfInput)
                            }
                        }
                    }
                    OperationDelivery::Replay => scan_replay_step(
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
                        frame,
                        cold,
                    )?,
                    OperationDelivery::Alignment(alignment) => scan_alignment_delivery_step(
                        &mut processor,
                        alignment,
                        &ReplayBoxes::default(),
                        innermost_group,
                        mode,
                        job_is_all_over,
                        self.main_loop_active,
                        &mut self.shown_mode,
                        &mut diagnostics,
                        frame,
                        cold,
                    )?,
                    OperationDelivery::AlignmentRetry { alignment, cursor } => {
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
                                frame,
                                cold,
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
                                frame,
                                cold,
                            )?,
                        }
                    }
                    OperationDelivery::ResidentHot => {
                        unreachable!("pre-scanned hot delivery bypasses operation preparation")
                    }
                    OperationDelivery::ResidentCold => {
                        unreachable!("pre-scanned cold delivery bypasses operation preparation")
                    }
                    OperationDelivery::SuspendedCold { .. } => {
                        unreachable!("prepared cold operations bypass operand scanning")
                    }
                })
            })();
            let cursor = processor.delivery_cursor();
            let retry_expansion = processor.take_pending_expansion_work();
            let scanner_resume = processor.take_scanner_resume();
            let retained_command_scan = frame.is_command_scan();
            let alignment_scanner = if retained_command_scan {
                assert!(
                    scanner_resume.is_none(),
                    "the direct-operation parent already owns its exact scanner child"
                );
                None
            } else if let Some(expansion) = retry_expansion {
                frame.clear_preflight();
                frame.admit_expanding(expansion, self.main_loop_active, cursor);
                assert!(
                    scanner_resume.is_none(),
                    "parked expansion owns its scanner child internally"
                );
                None
            } else if frame.has_preflight() {
                frame.retain_scanner(cursor, scanner_resume);
                None
            } else {
                scanner_resume
            };
            let scanned = match scanned {
                Ok(scanned) => scanned,
                Err(error) => {
                    frame.write_retry_failure(error, cursor, alignment_scanner);
                    return Err(TypedOperationError::Preparation(frame.take_error()));
                }
            };
            if frame.command.is_none()
                && frame.has_preflight()
                && !matches!(
                    frame.phase,
                    Some(PreflightCommandPhase::ImmediatePdfRetry(_))
                )
            {
                frame.clear_preflight();
            }
            #[cfg(feature = "profiling")]
            if matches!(scanned, ScannedOperation::Cold) {
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
        drop(context);
        // tex.web's `line` is maintained by `get_next` as it moves to a new
        // input line, so it is already the delivered command's own line by
        // the time that command is applied. Publish it here, after delivery,
        // rather than at the step's start: §660/§675's box diagnostics and
        // §1091's `mode_line` both name the line the command is *on*, and a
        // command that is the first thing on a line is scanned by a step
        // that began on the previous one.
        self.capture_first_reported_command_error_context(stores);
        self.capture_first_causal_context(stores, &diagnostics);
        if let Err(error) = report_pending_diagnostics(stores, diagnostic_effects, diagnostics) {
            return Err(TypedOperationError::Preparation(error));
        }
        match scanned {
            ScannedOperation::Cold => self.execute_scanned_cold_episode(
                stores,
                host_preparation,
                diagnostic_effects,
                frame,
                cold,
                OperationOutputStart {
                    outer_paragraph_was_active,
                    source_role,
                    artifact_count: stores.world().artifact_commits().len(),
                    effect_count: stores.world().effect_records().len(),
                    prepared_page_count: self.prepared_dvi_pages.len(),
                },
            ),
            ScannedOperation::Hot => {
                let applied = self.apply_hot_operation(
                    stores,
                    host_preparation,
                    diagnostic_effects,
                    frame.hot_mut(),
                    OperationOutputStart {
                        outer_paragraph_was_active,
                        source_role,
                        artifact_count: stores.world().artifact_commits().len(),
                        effect_count: stores.world().effect_records().len(),
                        prepared_page_count: self.prepared_dvi_pages.len(),
                    },
                );
                frame.hot = None;
                applied.map_err(TypedOperationError::Application)
            }
        }
    }

    /// Executes a completed cold branch through its typed borrow-owned episode.
    fn execute_scanned_cold_episode(
        &mut self,
        stores: &mut Universe<G>,
        host_preparation: &mut OperationPreparation<'_, G>,
        diagnostic_effects: &mut DiagnosticEffects,
        frame: &mut CommandEpisode<G>,
        cold: &mut ColdOperationSlot<G>,
        output_start: OperationOutputStart,
    ) -> Result<ReplayStep, TypedOperationError> {
        let immediate_pdf_retry = match frame.unavailable_mut(cold) {
            ColdOperation::ImmediateExtension(RootedImmediateExtension::PdfExtensionInDviMode(
                primitive,
            )) => Some(*primitive),
            _ => None,
        };
        if let Some(primitive) = immediate_pdf_retry {
            frame.clear_preflight();
            frame.admit_immediate_pdf(primitive);
        }
        let episode =
            self.prepare_cold_execution_episode(stores, frame.unavailable_mut(cold), output_start)?;
        let result = self
            .execute_cold_episode(stores, host_preparation, episode, diagnostic_effects)
            .map_err(TypedOperationError::Application);
        if result.is_ok() {
            frame.clear_cold(cold);
        }
        result
    }

    fn prepare_cold_execution_episode<'operation>(
        &mut self,
        stores: &mut Universe<G>,
        operation: &'operation mut PreparedColdCommand<G>,
        output_start: OperationOutputStart,
    ) -> Result<ColdExecutionEpisode<'operation, G>, TypedOperationError> {
        let resource_result = {
            self.resolve_font_resource(operation, stores)
                .and_then(|()| self.resolve_input_stream_resource(operation, stores))
                .and_then(|()| self.resolve_pdf_image_resource(operation, stores))
        };
        if let Err(error) = resource_result {
            return Err(TypedOperationError::Preparation(error));
        }
        let completed_preamble = match &*operation {
            ColdOperation::AlignmentPreambleStart { alignment } => {
                let alignment = *alignment;
                let preamble = match self
                    .command
                    .state_mut()
                    .take_completed_alignment_preamble(alignment)
                {
                    Ok(preamble) => preamble,
                    Err(_) => {
                        return Err(TypedOperationError::Preparation(ExecError::MissingToken {
                            context: "completed alignment preamble",
                        }));
                    }
                };
                Some((alignment, preamble))
            }
            _ => None,
        };
        let mut alignment_roots = completed_preamble
            .as_ref()
            .map(|(_, preamble)| {
                let mut roots = Vec::with_capacity(preamble.columns.len() * 2);
                for templates in &preamble.columns {
                    roots.extend(templates.u_template.map(OperationTokenRoot::attempt));
                    roots.push(OperationTokenRoot::attempt(templates.v_template));
                }
                roots
            })
            .unwrap_or_default();
        if prepare_cold_operation(
            operation,
            self.command.state_mut(),
            stores,
            &mut alignment_roots,
        )
        .is_err()
        {
            return Err(TypedOperationError::Preparation(ExecError::MissingToken {
                context: "cold operation root preparation",
            }));
        }
        let alignment_preamble = completed_preamble.map(|(alignment, preamble)| {
            let mut promoted = alignment_roots
                .into_iter()
                .map(|mut root| root.take_prepared());
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
        Ok(ColdExecutionEpisode {
            operation,
            alignment_preamble,
            output_start,
        })
    }

    /// Applies a measured common operation without constructing the universal
    /// scan/preparation DTOs. `CommandProcessor` has released its borrow, but
    /// the enclosing direct-operation transaction and persistent interpreter
    /// remain the same ones that performed delivery and scanning.
    fn apply_hot_operation(
        &mut self,
        stores: &mut Universe<G>,
        host_preparation: &mut OperationPreparation<'_, G>,
        diagnostic_effects: &mut DiagnosticEffects,
        operation: &mut hot_apply::HotOperation<G>,
        output_start: OperationOutputStart,
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
        // TeX82 §1211's measured definition, let, and catcode arms reach
        // §1269's `done` label without an intervening host transition. Admit
        // their authoritative state directly in this callback's stack slot,
        // and retain that one borrow through semantic apply, ordered evidence
        // publication, and `afterassignment` backup. Group transitions can
        // open page-output work, so they deliberately leave the callback at
        // the existing host boundary and use the generic tail below.
        let (result, settled_in_admission, pending_page_output) = stores
            .with_command_context(|context| {
                let mut result = hot_apply::apply(
                    operation,
                    context,
                    &mut self.modes,
                    &mut CommandMachine {
                        state: &mut self.command,
                        fuel: self.fuel.fuel_mut(),
                        capabilities: &mut self.capabilities,
                        host_facts: CommandMachineHostFacts::Forbidden,
                        observations: &mut self.operation_observations,
                        assignment_receipts: assignment_receipts.as_mut(),
                        diagnostic_effects,
                        shown_mode: &mut self.shown_mode,
                        initex: self.initex,
                        emit_dvi_override: self.emit_dvi_override,
                        immediate_prints: &mut self.immediate_prints,
                        prepared_shipout: &mut self.prepared_shipout,
                        pending_show_completion: None,
                        pending_outer_page_build_context: None,
                        output_routine_active: self.boxes.output_routine_active,
                    },
                );
                Self::capture_save_stack_usage(
                    host_preparation,
                    context,
                    &self.boxes,
                    self.command.state(),
                    self.command_profile(),
                );
                // These are exactly §1211's assignment leaves. Unlike group
                // transitions, none can contribute material, invoke the page
                // builder, or cross a World publication boundary. A pending
                // builder continuation is drained by the direct-episode loop
                // before another command is delivered. Capture the page facts
                // from this existing admission rather than opening another
                // command context after the callback closes.
                if result.is_err() || !fires_afterassignment {
                    let pending_page_output = PendingPageOutputFacts::capture(context);
                    return (result, false, pending_page_output);
                }

                #[cfg(feature = "profiling")]
                tex_state::measurement::record_hot_core_phase(
                    tex_state::measurement::HotCorePhase::EvidencePublication,
                );
                #[cfg(feature = "profiling")]
                let _evidence_allocation_scope = tex_state::measurement::hot_core_allocation_scope(
                    tex_state::measurement::HotCoreAllocationOwner::EvidencePublication,
                );
                publish_named_token_list_pushes(
                    &mut self.command,
                    context,
                    diagnostic_effects,
                    &mut self.operation_observations,
                );
                self.observe_committed(
                    assignment_receipts
                        .take()
                        .into_iter()
                        .flatten()
                        .map(CommandObservation::Mutation),
                );
                // §1269 publishes the completed assignment mutation before
                // §325 observes the replay-level push of the saved token.
                let mut host_facts = ExecutorHostFacts {
                    modes: &self.modes,
                    pdf_ignore_depth: self.pdf_ignore_depth,
                    telemetry: &mut self.episode_telemetry,
                };
                if let Err(error) = schedule_afterassignment(
                    &mut self.command,
                    self.fuel.fuel_mut(),
                    &mut self.capabilities,
                    &mut host_facts,
                    &mut self.operation_observations,
                    diagnostic_effects,
                    context,
                ) {
                    result = Err(error);
                }
                let pending_page_output = PendingPageOutputFacts::capture(context);
                (result, true, pending_page_output)
            })
            .map_err(|_| ExecError::MissingToken {
                context: "hot operation admission",
            })?;
        if result.is_ok() {
            self.fire_pending_page_output(stores, diagnostic_effects, pending_page_output)?;
        }
        #[cfg(feature = "profiling")]
        tex_state::measurement::record_hot_core_phase(
            tex_state::measurement::HotCorePhase::EvidencePublication,
        );
        #[cfg(feature = "profiling")]
        let _evidence_allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::EvidencePublication,
        );
        if result.is_ok() && !settled_in_admission {
            stores
                .with_command_context(|context| {
                    publish_named_token_list_pushes(
                        &mut self.command,
                        context,
                        diagnostic_effects,
                        &mut self.operation_observations,
                    );
                })
                .expect("live generation");
            let mut records = Vec::new();
            records.extend(
                assignment_receipts
                    .into_iter()
                    .flatten()
                    .map(CommandObservation::Mutation),
            );
            records.extend(
                committed_stream_effect_observations(
                    output_start.effect_count,
                    output_start.prepared_page_count,
                    stores,
                    &self.prepared_dvi_pages,
                )
                .into_iter()
                .map(CommandObservation::Effect),
            );
            for shipout in committed_shipout_observations(output_start.artifact_count, stores) {
                records.push(CommandObservation::Effect(shipout));
            }
            self.page_output_observations.append_to(&mut records);
            self.observe_committed(records);
        }
        if result.is_ok() && fires_afterassignment && !settled_in_admission {
            stores
                .with_command_context(|context| {
                    let mut host_facts = ExecutorHostFacts {
                        modes: &self.modes,
                        pdf_ignore_depth: self.pdf_ignore_depth,
                        telemetry: &mut self.episode_telemetry,
                    };
                    schedule_afterassignment(
                        &mut self.command,
                        self.fuel.fuel_mut(),
                        &mut self.capabilities,
                        &mut host_facts,
                        &mut self.operation_observations,
                        diagnostic_effects,
                        context,
                    )
                })
                .map_err(|_| ExecError::MissingToken {
                    context: "afterassignment admission",
                })??;
        }
        debug_assert!(
            !settled_in_admission
                || (stores.world().artifact_commits().len() == output_start.artifact_count
                    && stores.world().effect_records().len() == output_start.effect_count
                    && self.prepared_dvi_pages.len() == output_start.prepared_page_count
                    && self.page_output_observations.is_empty()),
            "hot assignment admission cannot cross a page or host-effect publication"
        );
        self.page_output_observations.clear();
        if result.is_ok() {
            self.finish_shipout_publication(
                output_start.artifact_count,
                output_start.effect_count,
                stores,
                output_start.source_role,
            );
            self.finish_paragraph_boundary(
                output_start.outer_paragraph_was_active,
                output_start.source_role,
                stores,
            );
        }
        result
    }

    fn execute_cold_episode(
        &mut self,
        stores: &mut Universe<G>,
        host_preparation: &mut OperationPreparation<'_, G>,
        episode: ColdExecutionEpisode<'_, G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<ReplayStep, ExecError> {
        let ColdExecutionEpisode {
            operation,
            alignment_preamble,
            output_start,
        } = episode;
        let nested_snapshot = operation
            .executes_nested_operations()
            .then(|| {
                self.command
                    .state_mut()
                    .transient_snapshot(stores)
                    .map_err(|_| ExecError::MissingToken {
                        context: "nested operation rollback snapshot",
                    })
            })
            .transpose()?;
        let parking = self.suspend_main_control_parking(operation);
        #[cfg(feature = "profiling")]
        tex_state::measurement::record_hot_core_phase(
            tex_state::measurement::HotCorePhase::SemanticApply,
        );
        #[cfg(feature = "profiling")]
        let _semantic_allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::SemanticApply,
        );
        if let Some(applied) = self.apply_host_owned_step(operation, stores, diagnostic_effects) {
            let result =
                self.finish_host_owned_step(applied, output_start, stores, diagnostic_effects);
            if let Some(snapshot) = nested_snapshot {
                let settled = if result
                    .as_ref()
                    .is_err_and(execution_error_needs_command_retry)
                {
                    self.command
                        .state_mut()
                        .rollback_transient(snapshot, stores)
                } else {
                    self.command.state_mut().commit_transient(snapshot, stores)
                };
                settled.map_err(|_| ExecError::MissingToken {
                    context: "nested operation command settlement",
                })?;
            }
            return result;
        }
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
        let resident = &*operation;
        let reports_synchronous_auxiliary_error = matches!(
            resident,
            ColdOperation::IllegalPrevDepth { .. } | ColdOperation::IllegalSpaceFactor { .. }
        ) || matches!(
            resident,
            ColdOperation::PrevGraf { value } if *value < 0
        ) || matches!(
            resident,
            ColdOperation::SpaceFactor { value } if !(1..=32767).contains(value)
        );
        if reports_synchronous_auxiliary_error
            || matches!(
                resident,
                ColdOperation::OffSave(_)
                    | ColdOperation::AlignmentRecovery { .. }
                    | ColdOperation::Message { .. }
                    | ColdOperation::SetInteractionMode(_)
                    | ColdOperation::SetInteractionModeValue { .. }
            )
        {
            // TeX82 §§1030/1064: command tracing happens when the offending
            // command is fetched, before `off_save`/`align_error` prints its
            // balancing-token error. The trace is detached while scanning,
            // whereas the legacy recovery report writes synchronously during
            // apply. Publish that already-committed trace before admitting
            // the one context which stays resident through ordinary semantic
            // application, so the two mechanisms cannot invert their
            // canonical order.
            //
            // §1279's `\message`/`\errmessage` and §1264's
            // `new_interaction` are synchronous World-facing boundaries.
            // Any macro trace produced while scanning the message must be
            // visible before its expanded text is printed.
            //
            // `new_interaction` similarly requires the already-rendered
            // §1030 command trace to reach the old selector before `print_ln`
            // and the selector transition. Otherwise the unconditional
            // `print_ln` overtakes the detached trace, moving TeX's blank line
            // from after `\batchmode` to before it and routing later output
            // from the wrong partial-line state.
            //
            // §§1243--1244's auxiliary-assignment rejection arms likewise
            // call the synchronous error reporter after their operand scan.
            // A conditional expanded as the integer terminator has already
            // completed both its command and boolean-result traces at this
            // point, so publish the whole scan program before `int_error` or
            // `report_illegal_case` can overtake it.
            stores
                .world_mut()
                .publish_diagnostic_effects(std::mem::take(diagnostic_effects));
        }
        let scanned = &*operation;
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
            ColdOperation::AlignmentFinish { alignment, .. } => {
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
        let output_routine_was_active = self.boxes.output_routine_active;
        let mut command = CommandMachine {
            state: &mut self.command,
            fuel: self.fuel.fuel_mut(),
            capabilities: &mut self.capabilities,
            host_facts: CommandMachineHostFacts::Detached {
                pdf_ignore_depth: self.pdf_ignore_depth,
                telemetry: &mut self.episode_telemetry,
            },
            observations: &mut self.operation_observations,
            assignment_receipts: assignment_receipts.as_mut(),
            diagnostic_effects,
            shown_mode: &mut self.shown_mode,
            initex: self.initex,
            emit_dvi_override: self.emit_dvi_override,
            immediate_prints: &mut self.immediate_prints,
            prepared_shipout: &mut self.prepared_shipout,
            pending_show_completion: None,
            pending_outer_page_build_context: None,
            output_routine_active: self.boxes.output_routine_active,
        };
        let (mut result, mut post_apply_facts, redundant_glue, effect) = if matches!(
            &*operation,
            ColdOperation::ImmediateExtension(RootedImmediateExtension::PdfForm(_))
        ) {
            // Immediate form creation crosses back to the aggregate Universe;
            // only this uncommon host boundary publishes outside the resident
            // semantic context admitted below.
            let provenance_demand = stores.provenance_demand();
            let provenance_budget_bytes =
                stores.provenance_budgets().detached_artifact_recipe_bytes;
            let (form, source_resolver, post_apply_facts, effect) = stores
                .with_command_context(|context| {
                    let effect = applied_effect_observation(&*operation, context);
                    let request = match &mut *operation {
                        ColdOperation::ImmediateExtension(RootedImmediateExtension::PdfForm(
                            request,
                        )) => request,
                        _ => unreachable!("immediate-form discriminant remains resident"),
                    };
                    let form = apply_pdf_form_request(
                        request,
                        context,
                        &mut self.modes,
                        &mut command,
                        true,
                    )?
                    .expect("immediate form creation returns a publication record");
                    let form_page = context
                        .copy_pdf_form_to_page(form.object())
                        .ok_or(ExecError::PdfXFormVoidBox)?;
                    let source_resolver =
                        DetachedArtifactSourceResolver::capture_page_list(form_page, context);
                    let post_apply_facts =
                        PostApplyFacts::capture(parking, self.modes.current_mode(), context);
                    Ok::<_, ExecError>((form, source_resolver, post_apply_facts, effect))
                })
                .map_err(|_| ExecError::MissingToken {
                    context: "immediate form admission",
                })??;
            let mut geometry = DetachedShipoutGeometry::default();
            publish_immediate_pdf_form(
                form,
                &mut command,
                &self.modes,
                stores,
                &source_resolver,
                provenance_demand,
                provenance_budget_bytes,
                &mut geometry,
            )?;
            if let Some(geometry) = geometry.0 {
                crate::shipout::ShipoutGeometrySink::committed_shipout_geometry(
                    &mut command.shipout_geometry_sink(),
                    geometry,
                );
            }
            (Ok(ReplayStep::Continue), post_apply_facts, false, effect)
        } else {
            stores
                .with_command_context(|context| {
                    if let ColdOperation::ShowGroups { diagnostic } = &mut *operation
                        && diagnostic.is_none()
                    {
                        *diagnostic = Some(detached_showgroups(
                            context,
                            &self.active_alignment,
                            &self.boxes,
                            &self.active_discretionaries,
                            &self.active_math_choices,
                            &self.active_math_left_boundaries,
                            &self.active_math_shifts,
                        ));
                    }
                    let reassigning_glue = Self::local_glue_pointer_reassigned(
                        context,
                        &*operation,
                        &self.skip_pointer_sources,
                        &self.muskip_pointer_sources,
                    );
                    let redundant_glue = Self::etex_redundant_local_glue_assignment(
                        context,
                        &*operation,
                        &self.skip_pointer_sources,
                        &self.muskip_pointer_sources,
                    );
                    match &mut *operation {
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
                    let effect = applied_effect_observation(&*operation, context);
                    let result = apply_cold_operation(
                        operation,
                        context,
                        &mut self.modes,
                        &mut self.next_alignment_identity,
                        &mut self.active_alignment,
                        &mut command,
                        &mut self.boxes,
                        &self.active_discretionaries,
                        &self.active_math_choices,
                        &mut self.active_math_fields,
                        &self.active_math_left_boundaries,
                        &self.active_math_shifts,
                        &mut self.prepared_dvi_pages,
                    );
                    if result.is_ok() {
                        command.publish_named_token_list_pushes(context);
                    }
                    Self::capture_save_stack_usage(
                        host_preparation,
                        context,
                        &self.boxes,
                        command.state,
                        command.state.profile(),
                    );
                    let post_apply_facts =
                        PostApplyFacts::capture(parking, self.modes.current_mode(), context);
                    (result, post_apply_facts, redundant_glue, effect)
                })
                .map_err(|_| ExecError::MissingToken {
                    context: "cold operation admission",
                })?
        };
        if result.is_ok()
            && let Some(completion) = command.pending_show_completion.take()
        {
            // TeX82 §§1293/1298 completes a long `\show` only after
            // `end_diagnostic` has restored the selector and made the whole
            // dump visible. The dump is an operation-local detached program,
            // whereas `error` still owns the live World dialogue, so release
            // command admission, atomically publish the dump, and only then
            // enter the synchronous `! OK.` completion. This also preserves
            // the dump when ErrorStop exits from the dialogue.
            stores
                .world_mut()
                .publish_diagnostic_effects(std::mem::take(command.diagnostic_effects));
            let completion_result = {
                let mut context = stores
                    .command_context()
                    .expect("show completion has a live generation");
                crate::diagnostics::complete_show(
                    &mut context,
                    command.diagnostic_effects,
                    completion.long,
                    Some(completion.context),
                )
            };
            if let Err(error) = completion_result {
                result = Err(error);
            }
        }
        if result.is_ok()
            && let Some(context) = command.pending_outer_page_build_context.take()
        {
            // TeX82 §1099 runs `build_page` after §1100 has closed the
            // insertion group. Group-command and restoration traces are
            // detached, while §993/§1009 recovery still uses the live error
            // dialogue. Publish the completed group-close program at this
            // outer admission boundary before page building can report.
            stores
                .world_mut()
                .publish_diagnostic_effects(std::mem::take(command.diagnostic_effects));
            let mut stores = stores.command_context().expect("live generation");
            crate::vertical::build_page_if_outer_vertical_with_error_context(
                &self.modes,
                &mut stores,
                command.diagnostic_effects,
                &context,
            )?;
            post_apply_facts.page_output = PendingPageOutputFacts::capture(&stores);
        }
        if result.is_ok() {
            if !command.immediate_prints.is_empty() {
                // TeX82 §1375 performs an immediate write only after its
                // token list has been expanded. Expansion/command traces are
                // detached during that scan, while the resulting write is an
                // outer World publication; commit the former first so the
                // expanded text cannot overtake its own trace.
                stores
                    .world_mut()
                    .publish_diagnostic_effects(std::mem::take(command.diagnostic_effects));
            }
            for print in command.immediate_prints.drain(..) {
                match print.encoded {
                    Some(bytes) if print.ensure_line_start => {
                        stores.world_mut().publish_print_nl_encoded_bytes(
                            print.sink,
                            &bytes,
                            print.max_print_line,
                        );
                    }
                    Some(bytes) => {
                        stores.world_mut().publish_print_encoded_bytes(
                            print.sink,
                            &bytes,
                            print.max_print_line,
                        );
                    }
                    None if print.ensure_line_start => {
                        stores.world_mut().publish_print_nl_text(
                            print.sink,
                            &print.text,
                            print.max_print_line,
                        );
                    }
                    None => {
                        stores.world_mut().publish_print_text(
                            print.sink,
                            &print.text,
                            print.max_print_line,
                        );
                    }
                }
            }
            if let Some(shipout) = command.prepared_shipout.take() {
                // TeX82 §§367/1006 print the delivered command and page
                // cost before §638 opens the shipout marker. Both reports
                // are detached while their respective admitted scanners run,
                // so move their complete ordered program into World before
                // the outer shipout transaction can materialize `[`. Any
                // diagnostics produced by deferred whatsit replay join that
                // transaction separately at its own pre-commit boundary.
                stores
                    .world_mut()
                    .publish_diagnostic_effects(std::mem::take(command.diagnostic_effects));
                if let Some(receipt) =
                    shipout_replay_box(shipout, stores, &mut command, &self.modes)?
                        .and_then(|publication| publication.dvi)
                {
                    push_prepared_dvi_page(&mut self.prepared_dvi_pages, receipt);
                }
            }
        } else {
            command.immediate_prints.clear();
            if let Some(shipout) = command.prepared_shipout.take()
                && let Some(region) = shipout.region
            {
                stores
                    .release_page_node_region(region)
                    .expect("aborted shipout command releases its nested page region");
            }
        }
        let completed_output_routine =
            output_routine_was_active && !self.boxes.output_routine_active;
        drop(command);
        if result.is_ok() && completed_output_routine {
            // §1026 has consumed the output mode list and established the
            // complete held-over contribution closure. Rotate only now, after
            // any user `\shipout` has detached its output and after box255 has
            // either been consumed or diagnosed and cleared.
            self.page_region_succession_pending = true;
            self.finish_pending_page_region_succession(stores);
        }
        if result.is_ok()
            && !redundant_glue
            && let Some((muskip, index, source_identity)) = glue_assignment
            && let Some((index, physical, source_identity, pointer_sources)) = {
                let context = stores.command_context().expect("live generation");
                if muskip {
                    context.muskip(index).map(|physical| {
                        (
                            index,
                            physical,
                            source_identity,
                            &mut self.muskip_pointer_sources,
                        )
                    })
                } else {
                    context.glue_register(index).ok().flatten().map(|physical| {
                        (
                            index,
                            physical,
                            source_identity,
                            &mut self.skip_pointer_sources,
                        )
                    })
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
                .make_string_pool_string(&receipt.pool_string());
            self.dumped_format = Some(receipt);
        }
        if let (Ok(ReplayStep::End), Some((dump, incomplete_conditions))) = (&result, end.as_ref())
        {
            // TeX82 §1030 traces the delivered `\end` before §1335 starts
            // final cleanup. The trace was rendered while command state was
            // admitted, but its detached program must be replayed here,
            // after that admission has ended and before cleanup writes its
            // transcript notice directly to World.
            stores
                .world_mut()
                .publish_diagnostic_effects(std::mem::take(diagnostic_effects));
            self.end_of_job_final_cleanup(stores, *dump, incomplete_conditions.clone());
        } else if matches!(result, Ok(ReplayStep::EndOfInput))
            && self.root_completion == RootCompletionPolicy::RequireTeXEnd
        {
            result = Ok(self.handle_root_end_of_input(stores));
        }
        if result.is_ok()
            && let Some(main_loop_active) = post_apply_facts.main_loop_active
        {
            self.main_loop_active = main_loop_active;
        }
        if result.is_ok() {
            self.fire_pending_page_output(
                stores,
                diagnostic_effects,
                post_apply_facts.page_output,
            )?;
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
            // command-processor episode's own borrow has ended. Named token
            // pushes were already sent directly to this operation's optional
            // commit sink while their live command context was admitted.
            let mut records: Vec<CommandObservation> = Vec::new();
            let effects = committed_stream_effect_observations(
                output_start.effect_count,
                output_start.prepared_page_count,
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
            for shipout in committed_shipout_observations(output_start.artifact_count, stores) {
                records.push(CommandObservation::Effect(shipout));
            }
            self.page_output_observations.append_to(&mut records);
            self.observe_committed(records);
        }
        // TeX82 §1211 commits the assignment inside its case arm, then
        // reaches §1269's `done:` and `back_input`. Publish the mutation
        // before the replay-level push for that saved token.
        if result.is_ok() && fires_afterassignment {
            stores
                .with_command_context(|context| {
                    let mut host_facts = ExecutorHostFacts {
                        modes: &self.modes,
                        pdf_ignore_depth: self.pdf_ignore_depth,
                        telemetry: &mut self.episode_telemetry,
                    };
                    schedule_afterassignment(
                        &mut self.command,
                        self.fuel.fuel_mut(),
                        &mut self.capabilities,
                        &mut host_facts,
                        &mut self.operation_observations,
                        diagnostic_effects,
                        context,
                    )
                })
                .map_err(|_| ExecError::MissingToken {
                    context: "afterassignment admission",
                })??;
        }
        self.page_output_observations.clear();
        if result.is_ok() {
            self.finish_shipout_publication(
                output_start.artifact_count,
                output_start.effect_count,
                stores,
                output_start.source_role,
            );
            self.finish_paragraph_boundary(
                output_start.outer_paragraph_was_active,
                output_start.source_role,
                stores,
            );
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
        let mut diagnostic_effects = DiagnosticEffects::new();
        let scanned = self.scan_startup_file_name_once(stores, &mut diagnostic_effects);
        self.operation_observations
            .take()
            .unwrap_or_default()
            .consume_into(Some(observer));
        stores
            .world_mut()
            .publish_diagnostic_effects(diagnostic_effects);
        scanned
    }

    fn scan_startup_file_name_once(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<String, ExecError> {
        let filename = {
            let mut context = stores.command_context().expect("live generation");
            let mut host_facts = ExecutorHostFacts {
                modes: &self.modes,
                pdf_ignore_depth: self.pdf_ignore_depth,
                telemetry: &mut self.episode_telemetry,
            };
            let mut processor = command_processor(
                &mut self.command,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
                &mut host_facts,
                &mut self.operation_observations,
                diagnostic_effects,
                &mut context,
            );
            let mut destination = None;
            if processor
                .get_x_token_into(&mut destination)
                .map_err(command_error)?
                != tex_command::DeliveryStatus::Command
            {
                return Err(ExecError::MissingToken {
                    context: "terminal filename",
                });
            }
            let first = destination
                .take()
                .expect("command status initializes destination");
            processor.back_input(first).map_err(command_error)?;
            let mut filename = String::new();
            loop {
                if processor
                    .get_x_token_into(&mut destination)
                    .map_err(command_error)?
                    != tex_command::DeliveryStatus::Command
                {
                    return Err(ExecError::MissingToken {
                        context: "terminal filename",
                    });
                }
                let command = destination
                    .take()
                    .expect("command status initializes destination");
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
        let mut context = stores.command_context().expect("live generation");
        let mut host_facts = ExecutorHostFacts {
            modes: &self.modes,
            pdf_ignore_depth: self.pdf_ignore_depth,
            telemetry: &mut self.episode_telemetry,
        };
        let mut processor = command_processor(
            &mut self.command,
            self.fuel.fuel_mut(),
            &mut self.capabilities,
            &mut host_facts,
            &mut self.operation_observations,
            diagnostic_effects,
            &mut context,
        );
        let mut destination = None;
        let exhausted = processor.get_x_token_into(&mut destination);
        let exhausted = exhausted.map_err(command_error);
        drop(processor);
        self.operation_observations = silenced;
        let terminal_exhausted = exhausted? == tex_command::DeliveryStatus::End;
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

/// Prepared source-side settlement kept outside the destination generation.
///
/// The receipt deliberately owns the live mode owner. Dropping it is only an
/// unwind guard; normal aggregate settlement calls [`Self::accept`] or
/// [`Self::reject`] after the destination page/layout owners have settled.
#[doc(hidden)]
pub struct PreparedCheckpointControl {
    modes: ModeNest,
}

impl PreparedCheckpointControl {
    pub fn accept(self) {
        #[cfg(feature = "profiling")]
        let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::GenerationBoundary,
        );
        if self.modes.is_checkpoint_candidate() {
            drop(self.modes.accept_checkpoint_candidate());
        }
    }

    pub fn reject(self) {
        #[cfg(feature = "profiling")]
        let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::GenerationBoundary,
        );
        if self.modes.is_checkpoint_candidate() {
            self.modes.reject_checkpoint_candidate();
        }
    }
}

/// TeX82 §1121's improper-discretionary report. The completed part is
/// frozen before validation so its detached node list remains available to
/// `show_box` even though recovery rejects the enclosing discretionary.
fn report_improper_discretionary<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    deleted: tex_state::node_arena::PageListId,
    context: String,
) -> Result<(), ExecError> {
    let text = crate::node_dump::dump_page_list(
        stores,
        deleted,
        crate::node_dump::DumpConfig::read(stores),
    );

    // TeX82 §§1120--1121 closes the discretionary part's group before
    // checking its nodes. Any `\tracingrestores` lines from that `unsave`
    // therefore precede the synchronous error dialogue.
    stores.publish_diagnostic_effects_before_synchronous_print(diagnostic_effects);
    let mut report = stores.print_err("Improper discretionary list");
    report
        .help(&["Discretionary lists must contain only boxes and kerns."])
        .context(context);
    report.error().defer_recovery(diagnostic_effects)?;

    let mut diagnostic = stores.begin_diagnostic(diagnostic_effects);
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
    stores: &mut CommandContext<'_, G>,
    code: u32,
    origin: tex_state::token::OriginId,
) -> Result<(), ExecError> {
    let (class, character) = math_char(stores, code, origin)?;
    list.push(
        stores,
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(class),
            MathField::MathChar(character),
        )),
    );
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
    stores: &mut CommandContext<'_, G>,
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
    append_math_char(modes.current_list_mutation(), stores, code, origin)
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
    diagnostic_effects: &mut DiagnosticEffects,
    kind: MathScriptKind,
) -> Result<ScriptTarget, ExecError> {
    let mut context = stores.command_context().expect("math script admission");
    // `t<>empty`: the tail was eligible but already carries this script.
    let tail_index = list.nodes(&context).len().checked_sub(1);
    let (eligible, occupied) = match tail_index.and_then(|index| list.nodes(&context).get(index)) {
        Some(tex_state::node_arena::NodeView::MathNoad(noad))
            if !matches!(
                noad.kind,
                NoadKind::LeftDelimiter { .. }
                    | NoadKind::RightDelimiter { .. }
                    | NoadKind::MiddleDelimiter { .. }
            ) =>
        {
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
        let index = list.nodes(&context).len();
        list.push(
            &mut context,
            Node::MathNoad(MathNoad::new(
                NoadKind::Normal(NoadClass::Ord),
                MathField::Empty,
            )),
        );
        index
    };
    drop(context);

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
        report.error().defer_recovery(diagnostic_effects)?;
    }

    Ok(ScriptTarget { node_index, kind })
}

pub(crate) fn fill_script_target<G>(
    mut list: crate::mode::ModeListMutation<'_>,
    stores: &mut CommandContext<'_, G>,
    target: ScriptTarget,
    field: MathField,
) {
    list.with_node_mut(stores, target.node_index, |node| {
        let Node::MathNoad(noad) = node else {
            unreachable!("reserved canonical script target must remain a noad")
        };
        let reserved = script_field_mut(noad, target.kind);
        debug_assert!(matches!(reserved, MathField::Empty));
        *reserved = field;
    })
    .expect("reserved canonical script target must remain present");
}

/// Applies §1151/§1186's finished field to the parent list position saved by
/// the opener. The mode level containing the field has already been popped.
fn fill_math_field_target<G>(
    modes: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    target: ActiveMathFieldTarget,
    field: MathField,
) {
    match target {
        ActiveMathFieldTarget::Script(target) => {
            fill_script_target(modes.current_list_mutation(), stores, target, field);
        }
        ActiveMathFieldTarget::Nucleus {
            node_index,
            simplify_accent,
        } => {
            // TeX82 §1186's second brace simplification: when a braced
            // field contains exactly one accent noad and is the nucleus of
            // an Ord atom, replace that Ord atom by the accent itself.
            let accent = if simplify_accent && let MathField::SubMlist(list) = field {
                let nodes = stores
                    .page_node_list(list)
                    .expect("math field belongs to the live page arena")
                    .nodes();
                match nodes.get(0) {
                    Some(tex_state::node_arena::NodeView::MathNoad(accent))
                        if nodes.len() == 1 && matches!(accent.kind, NoadKind::Accent { .. }) =>
                    {
                        Some(accent.clone())
                    }
                    _ => None,
                }
            } else {
                None
            };
            modes
                .current_list_mutation()
                .with_node_mut(stores, node_index, |node| {
                    if let Some(accent) = accent {
                        *node = Node::MathNoad(accent);
                    } else {
                        let Node::MathNoad(noad) = node else {
                            unreachable!("reserved math noad must remain a noad")
                        };
                        debug_assert!(matches!(noad.nucleus, MathField::Empty));
                        noad.nucleus = field;
                    }
                })
                .expect("reserved math noad must remain present");
        }
    }
}

fn apply_limits<G>(
    mut list: crate::mode::ModeListMutation<'_>,
    stores: &mut CommandContext<'_, G>,
    kind: MathLimitKind,
) -> bool {
    // TeX82 §1159's `math_limit_switch`: the subtype is set only when
    // `head<>tail` *and* the tail is an `op_noad`. `with_last_node_mut`
    // returns `None` for the empty list, which is `head=tail`.
    list.with_last_node_mut(stores, |node| {
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
    let numerator = list.take_span();
    let context = stores.command_context().expect("live generation");
    list.set_incomplete_fraction(
        &context,
        crate::mode::IncompleteFraction {
            numerator,
            thickness: match fraction.thickness {
                Some(value) => FractionThickness::Explicit(value),
                None => FractionThickness::Default,
            },
            left_delimiter: fraction.left_delimiter.map(|value| value.code),
            right_delimiter: fraction.right_delimiter.map(|value| value.code),
        },
    );
    true
}

fn finish_math_list<G>(
    output: tex_state::node_arena::PageListId,
    incomplete: Option<crate::mode::IncompleteFraction>,
    stores: &mut CommandContext<'_, G>,
) -> Result<tex_state::node_arena::PageListId, ExecError> {
    if let Some(fraction) = incomplete {
        let denominator = output;
        // TeX82 §1185 and e-TeX [48.1185]: `delim_ptr` identifies the most
        // recent `\left` or `\middle` in a math-left group.  Completion moves
        // only the nodes after that boundary into the numerator, then links
        // the fraction noad immediately after the boundary.
        let numerator_nodes = stores
            .page_node_span(fraction.numerator)
            .expect("fraction numerator belongs to the live page arena")
            .nodes();
        let boundary = numerator_nodes.iter().rposition(|node| {
            matches!(
                node,
                tex_state::NodeView::MathNoad(MathNoad {
                    kind: NoadKind::LeftDelimiter { .. } | NoadKind::MiddleDelimiter { .. },
                    ..
                })
            )
        });
        let numerator_len = numerator_nodes.len();
        let _ = numerator_nodes;
        let (prefix, numerator) = if let Some(boundary) = boundary {
            (
                Some(stores.slice_page_node_span(fraction.numerator, 0..boundary + 1)),
                stores.slice_page_node_span(fraction.numerator, boundary + 1..numerator_len),
            )
        } else {
            (None, fraction.numerator)
        };
        let fraction = Node::FractionNoad(MathFraction {
            numerator: numerator.list(),
            denominator,
            thickness: fraction.thickness,
            left_delimiter: fraction.left_delimiter,
            right_delimiter: fraction.right_delimiter,
        });
        if let Some(prefix) = prefix {
            let fraction = stores.publish_page_nodes(vec![fraction]);
            return Ok(stores.compose_page_node_sequences(&[prefix.list(), fraction]));
        }
        return Ok(stores.publish_page_nodes(vec![fraction]));
    }
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
    if nodes.len() == 1
        && let Some(tex_state::node_arena::NodeView::MathNoad(noad)) = nodes.get(0)
        && noad.kind == NoadKind::Normal(NoadClass::Ord)
        && matches!(noad.subscript, MathField::Empty)
        && matches!(noad.superscript, MathField::Empty)
    {
        return noad.nucleus;
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
        nodes,
        incomplete,
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
    diagnostic_effects: &mut DiagnosticEffects,
    prefix: &str,
    escaped: &str,
    suffix: &str,
    help: &[&str],
    context: String,
) -> Result<(), ExecError> {
    let mut report = stores.print_err(prefix);
    report.print_esc(escaped).print(suffix);
    report.help(help).context(context);
    report.error().defer_recovery(diagnostic_effects)?;
    Ok(())
}

/// TeX82 §1084's `scan_box` recovery for a command that is not a box.
///
/// §1084 reports through `back_error`, and every caller here has already had
/// the rejected command backed up during scanning, so only the report is
/// left.
fn report_missing_box<G>(
    command: &CommandState<G>,
    diagnostic_effects: &mut DiagnosticEffects,
    stores: &mut CommandContext<'_, G>,
) -> Result<(), ExecError> {
    let context = command.output_open_context(stores);
    crate::error_report::report_error(
        stores,
        diagnostic_effects,
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
    diagnostic_effects: &mut DiagnosticEffects,
) -> Result<(), ExecError> {
    // TeX82 §1084 reaches `box_error` only after `scan_box` has finished
    // expanding and tracing the rejected operand. Those completed scanner
    // diagnostics precede the synchronous `back_error` dialogue.
    stores.publish_diagnostic_effects_before_synchronous_print(diagnostic_effects);
    report_escaped_error(
        stores,
        diagnostic_effects,
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
    diagnostic_effects: &mut DiagnosticEffects,
    stores: &mut CommandContext<'_, G>,
) -> Result<(), ExecError> {
    crate::error_report::report_error(
        stores,
        diagnostic_effects,
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
    diagnostic_effects: &mut DiagnosticEffects,
    stores: &mut Universe<G>,
) -> Result<(), ExecError> {
    let mut stores = stores
        .command_context()
        .expect("display diagnostic admission");
    let context = command.output_open_context(&stores);
    crate::error_report::report_error(
        &mut stores,
        diagnostic_effects,
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
    let starts_left_node = |node: Option<tex_state::node_arena::NodeView<'_>>| {
        matches!(
            node,
            Some(tex_state::node_arena::NodeView::MathNoad(MathNoad {
                kind: NoadKind::LeftDelimiter { .. },
                ..
            }))
        )
    };
    let context = stores.command_context().expect("math-left query admission");
    starts_left_node(modes.current_list().nodes(&context).first())
        || modes
            .current_list()
            .incomplete_fraction()
            .is_some_and(|fraction| {
                context
                    .page_node_span(fraction.numerator)
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

impl MainControlParking {
    /// Reduces TeX82 §1030's post-apply parking decision to the one scalar the
    /// main-control loop consumes after command admission closes.
    ///
    /// §1034's `main_loop` is reached only from `hmode`, so the mode tested is
    /// the one the step left behind: §1090's `vmode+letter` opens a paragraph
    /// first and arrives in horizontal mode, while `mmode+letter` (§1154)
    /// appends a math char and never enters the loop at all.
    ///
    /// A character the current font does not contain never reaches lookahead:
    /// §1036's `main_loop_move+2` issues `char_warning`, frees the would-be
    /// node, and jumps to `big_switch`. With `\nullfont` selected -- §552
    /// gives it `font_bc=1`, `font_ec=0` -- that is every character.
    fn post_apply<G>(self, mode: Mode, context: &CommandContext<'_, G>) -> Option<bool> {
        if self.resumes_interrupted_fetch {
            return None;
        }
        Some(self.character.is_some_and(|character| {
            matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal)
                && u8::try_from(u32::from(character)).ok().is_some_and(|code| {
                    context
                        .font_char_metrics(context.current_font(), code)
                        .is_some()
                })
        }))
    }
}

/// Copy-small page-output state captured while the operation's authoritative
/// command context is already admitted.
#[derive(Clone, Copy, Default)]
struct PendingPageOutputFacts {
    fire_up: Option<PageFireUp>,
    resume_after_output: bool,
}

impl PendingPageOutputFacts {
    fn capture<G>(context: &CommandContext<'_, G>) -> Self {
        Self {
            fire_up: context.page_fire_up(),
            resume_after_output: context.page_builder_resume_after_output_pending(),
        }
    }

    fn is_pending(self) -> bool {
        self.fire_up.is_some() || self.resume_after_output
    }
}

/// The complete copy-small settlement of one callback-scoped semantic apply.
#[derive(Clone, Copy)]
struct PostApplyFacts {
    main_loop_active: Option<bool>,
    page_output: PendingPageOutputFacts,
}

impl PostApplyFacts {
    fn capture<G>(
        parking: MainControlParking,
        mode: Mode,
        context: &CommandContext<'_, G>,
    ) -> Self {
        Self {
            main_loop_active: parking.post_apply(mode, context),
            page_output: PendingPageOutputFacts::capture(context),
        }
    }
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
        crate::test_harness::with_nonstop_plain_universe(|stores| {
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
            crate::test_harness::with_admitted(stores, |context| {
                let current_list = control.modes.current_list();
                let Some(tex_state::NodeView::Disc {
                    kind: DiscKind::ExplicitHyphen,
                    pre,
                    ..
                }) = current_list.nodes(context).last()
                else {
                    panic!("canonical replay appended an explicit discretionary hyphen");
                };
                assert!(pre.is_empty());
            });
        });
    }

    #[test]
    fn missing_hyphen_glyph_leaves_pre_break_empty() {
        // TeX82 §§581/1113: an in-range hyphen character is constructed via
        // `new_character`, which warns and returns null for an absent glyph.
        crate::test_harness::with_nonstop_plain_universe(|stores| {
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
            crate::test_harness::with_admitted(stores, |context| {
                let current_list = control.modes.current_list();
                let Some(tex_state::NodeView::Disc { pre, .. }) =
                    current_list.nodes(context).last()
                else {
                    panic!("canonical replay appended an explicit discretionary hyphen");
                };
                assert!(pre.is_empty());
            });
        });
    }

    #[test]
    fn deferred_write_trace_precedes_unbalanced_report() {
        // TeX82 §§1369--1372: `write_out` traces the write-text token list
        // and expands its condition before testing the frozen `\endwrite`
        // stopper. Atomic shipout staging must retain that live-call order.
        crate::test_harness::with_nonstop_plain_universe(|stores| {
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
