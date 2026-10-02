//! Production main-control driver.
//!
//! This intentionally owns only typed execution mutations. Raw delivery,
//! expansion, macro calls, input nesting, and operand collection remain in
//! `tex-command`; no independent source stack is accepted here.

use std::path::PathBuf;
use std::sync::Arc;
use tex_command::{
    AlignmentCellDelimiter, AlignmentCellOpening, AlignmentIdentity, AlignmentRequest,
    AlignmentRequestResult, CommandError, CommandHostCapabilities, CommandHostContext,
    CommandProcessor, CommandProfile, CommandState, FatalError, FontLoadRequest, FontResource,
    GeneratedFontKind, HyphenationDataKind, ImmediateExtension, MathDelimiterBoundary,
    MathDelimiterBoundaryKind, MathFieldBody, MathLimitKind, MathRequest, MathScriptKind,
    MathStyleKind, MathTextFieldKind, PdfImageRequest, PdfImageResource, PdfReferenceObjectRequest,
    PreparedAlignmentCellTemplates, RegisteredSourceKind, ResourceFulfillment,
    ResourceInstallOutcome, ResourceNeed, ResourceProvider, RestrictedIntegerClass, ScannedAccent,
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
use crate::interpreter::PersistentInterpreter;
use crate::mode::{AlignColumn, AlignState, AlignmentKind, AlignmentPackSpec};
use crate::vertical::is_outer_vertical;
use crate::{ExecError, Mode, ModeNest};

mod code_table;
mod cold;
mod definition_assignments;
mod delivery;
mod direct;
mod discretionary;
mod executor_facts;
mod host_boundaries;
mod leader_values;
mod resources;
mod root_source;
mod semantic;
mod semantic_owners;

use code_table::recover_code_table_value;

use cold::*;
use delivery::*;
use executor_facts::ExecutorHostFacts;
use host_boundaries::*;
use leader_values::*;

fn font_metrics(resource: &FontResource) -> Option<&tex_state::FileContent> {
    match resource {
        FontResource::Tfm { metrics, .. }
        | FontResource::MappedTfm { metrics, .. }
        | FontResource::ClassicTfmFallback { metrics } => Some(metrics),
        FontResource::Unavailable | FontResource::OpenType(_) => None,
    }
}

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
    page_output_observations: Option<ObservationBuffer>,
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
    /// True when the most recent operation returned an error after committing
    /// its direct-operation roots. Observed drivers use this narrow query to
    /// publish the committed receipt even though the typed result is `Err`.
    error_operation_committed: bool,
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
    /// Demand is installed by the runner before execution. A paragraph cut
    /// either consumes it immediately or is permanently skipped.
    paragraph_checkpoint_demand: Option<bool>,
    paragraph_checkpoint_cut: bool,
    /// The sole job-start restart capability. Restored controls do not regain
    /// it, and successful initial capture consumes it permanently.
    job_start_eligibility: Option<crate::checkpoint::CheckpointEligibility>,
    /// Ordered named-boundary intents waiting for command-owned scanner,
    /// macro, resource, and structural continuations to become quiescent.
    /// Source site of the most recent typed resource suspension. This is
    /// retained outside snapshots so a host protocol no-progress invariant
    /// can still identify the command whose retry failed to advance.
    pending_resource_site: Option<OriginId>,
    /// One call-scoped provider decline, kept beside its canonical need until
    /// the outer step driver consumes the suspension sideband. It is never
    /// checkpointed, compared for resource identity, or retained across the
    /// next drive.
    declined_resource_attempt: Option<ResourceNeed>,
    /// A command failed or suspended mid-flight. Owners keep whatever the
    /// command had already written, so only a full checkpoint restore clears
    /// this latch; nothing here unwinds a partial command.
    halted: bool,
    /// Set exactly when the runner receives End. No later delivery is legal.
    ended: bool,
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
    /// First committed recoverable report for explicit causal diagnostics.
    /// This runtime evidence is not part of command snapshots, convergence
    /// identity, semantic hashes, or output event streams.
    first_recoverable_diagnostic: Option<Box<crate::FirstRecoverableDiagnostic>>,
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
    origin: tex_state::page_node_arena::BuiltBoxOrigin,
}

#[derive(Debug)]
struct ActiveReplayBox {
    target: Option<PendingSetBox>,
    shipout_region: Option<tex_state::node_region::PageClosureBuildMark>,
    box_segment_start: Option<tex_state::node_region::PageClosureBuildMark>,
    /// An insertion or adjustment opened inside an isolated hbox; its
    /// completed interval belongs to the enclosing page list after hpack.
    migration_segment_start: Option<tex_state::node_region::PageClosureBuildMark>,
    /// Construction coordinates of migrating material in this hbox. They
    /// remain non-owning until the original wrapper is consumed.
    migration_segments: Vec<tex_state::node_region::PageBoxSegment>,
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
    tabskips: Vec<tex_state::node::GlueValue>,
    default_tabskip: tex_state::node::GlueValue,
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
    fn push_box(&mut self, stores: &mut CommandContext<'_, G>, active: ActiveReplayBox) {
        stores.add_box_spec_words(active.kind.save_stack_spec_words());
        self.active_boxes.push(active);
    }

    fn pop_box(&mut self, stores: &mut CommandContext<'_, G>) -> Option<ActiveReplayBox> {
        let active = self.active_boxes.pop()?;
        stores.remove_box_spec_words(active.kind.save_stack_spec_words());
        Some(active)
    }

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

/// Non-semantic accounting for actual canonical command admission and stops.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AdvanceTelemetry {
    pub attempts: u64,
    pub commits: u64,
    /// Resource failures that halted the direct call; host replay owns undo.
    pub resource_halts: u64,
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
/// These are the production processor-borrow helpers in `tex-exec`, and the
/// architecture test in `crates/tex-exec/tests/it.rs` pins their ownership
/// there. A single
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
struct CommandMachine<'a, 'provider, G> {
    state: &'a mut PersistentInterpreter<G>,
    fuel: &'a mut tex_command::CommandFuel,
    capabilities: &'a mut CommandHostCapabilities,
    resource_provider: Option<&'provider mut dyn ResourceProvider<G>>,
    declined_resource_attempt: &'a mut Option<ResourceNeed>,
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

impl<G> CommandMachine<'_, '_, G> {
    fn with_processor_for_modes<R>(
        &mut self,
        context: &mut tex_state::CommandContext<'_, G>,
        modes: &ModeNest,
        use_processor: impl FnOnce(&mut CommandProcessor<'_, '_, G>) -> R,
    ) -> R {
        let Self {
            state,
            fuel,
            capabilities,
            resource_provider,
            declined_resource_attempt,
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
                let host = match resource_provider.as_deref_mut() {
                    Some(provider) => CommandHostContext::with_facts_and_resource_provider(
                        capabilities,
                        facts,
                        provider,
                        declined_resource_attempt,
                    ),
                    None => CommandHostContext::with_facts(capabilities, facts),
                };
                let mut processor =
                    state.processor(context, host, fuel, observer, diagnostic_effects);
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
                let host = match resource_provider.as_deref_mut() {
                    Some(provider) => CommandHostContext::with_facts_and_resource_provider(
                        capabilities,
                        &mut facts,
                        provider,
                        declined_resource_attempt,
                    ),
                    None => CommandHostContext::with_facts(capabilities, &mut facts),
                };
                let mut processor =
                    state.processor(context, host, fuel, observer, diagnostic_effects);
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
        use_processor: impl FnOnce(&mut CommandProcessor<'_, '_, G>) -> R,
    ) -> R {
        let Self {
            state,
            fuel,
            capabilities,
            resource_provider,
            declined_resource_attempt,
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
                let host = match resource_provider.as_deref_mut() {
                    Some(provider) => CommandHostContext::with_facts_and_resource_provider(
                        capabilities,
                        facts,
                        provider,
                        declined_resource_attempt,
                    ),
                    None => CommandHostContext::with_facts(capabilities, facts),
                };
                let mut processor =
                    state.processor(context, host, fuel, observer, diagnostic_effects);
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
                let host = match resource_provider.as_deref_mut() {
                    Some(provider) => CommandHostContext::with_facts_and_resource_provider(
                        capabilities,
                        &mut facts,
                        provider,
                        declined_resource_attempt,
                    ),
                    None => CommandHostContext::with_facts(capabilities, &mut facts),
                };
                let mut processor =
                    state.processor(context, host, fuel, observer, diagnostic_effects);
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
    ) -> CommandProcessor<'episode, 'admission, G> {
        let observer = self
            .observations
            .as_mut()
            .map(|buffer| buffer as &mut dyn CommandObserver);
        let host = match self.resource_provider.as_deref_mut() {
            Some(provider) => CommandHostContext::with_facts_and_resource_provider(
                self.capabilities,
                &mut self.host_facts,
                provider,
                self.declined_resource_attempt,
            ),
            None => CommandHostContext::with_facts(self.capabilities, &mut self.host_facts),
        };
        let mut processor =
            self.state
                .processor(context, host, self.fuel, observer, self.diagnostic_effects);
        processor.set_output_routine_active(self.output_routine_active);
        processor
    }

    fn shipout_geometry_sink(&mut self) -> MainControlShipoutGeometrySink<'_, G> {
        MainControlShipoutGeometrySink {
            command: self.state,
            observations: self.observations,
        }
    }

    fn retain_assignment_mutation(&mut self, record: MutationRecord) {
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
) -> CommandProcessor<'episode, 'admission, G> {
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

#[allow(clippy::too_many_arguments)] // each argument is a distinct replay owner
fn command_processor_with_resource_provider<'episode, 'admission, G>(
    command: &'episode mut PersistentInterpreter<G>,
    fuel: &'episode mut tex_command::CommandFuel,
    capabilities: &'episode mut CommandHostCapabilities,
    host_facts: &'episode mut dyn tex_command::CommandHostFacts<G>,
    resource_provider: &'episode mut dyn ResourceProvider<G>,
    declined_resource_attempt: &'episode mut Option<ResourceNeed>,
    observations: &'episode mut ObservationSlot,
    diagnostic_effects: &'episode mut DiagnosticEffects,
    stores: &'episode mut CommandContext<'admission, G>,
) -> CommandProcessor<'episode, 'admission, G> {
    let observer = observations
        .as_mut()
        .map(|buffer| buffer as &mut dyn CommandObserver);
    command.processor(
        stores,
        CommandHostContext::with_facts_and_resource_provider(
            capabilities,
            host_facts,
            resource_provider,
            declined_resource_attempt,
        ),
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
            page_output_observations: None,
            operation_observations: None,
            operation_receipt_start: None,
            error_operation_committed: false,
            completed_replay_episode: None,
            prepared_dvi_pages: PreparedDviPages::default(),
            immediate_prints: Vec::new(),
            prepared_shipout: None,
            page_region_succession_pending: false,
            paragraph_checkpoint_demand: None,
            paragraph_checkpoint_cut: false,
            job_start_eligibility: Some(crate::checkpoint::CheckpointEligibility::job_start()),
            pending_resource_site: None,
            declined_resource_attempt: None,
            halted: false,
            ended: false,
            fatal: None,
            captured_fatal_origin: None,
            first_causal_context: None,
            first_recoverable_diagnostic: None,
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

    pub(crate) fn into_rejected_checkpoint_command(self) -> CommandState<G> {
        let (mut command, control) = self.into_checkpoint_candidate_parts();
        control.reject();
        command.reject_checkpoint_candidate();
        command
    }

    pub(crate) fn terminal_is_quiescent(&self, stores: &mut Universe<G>) -> bool {
        self.command.named_boundary_is_quiescent()
            && self.active_alignment.is_none()
            && self.operation_observations.is_none()
            && self.operation_receipt_start.is_none()
            && self.prepared_shipout.is_none()
            && !self.page_region_succession_pending
            && self.immediate_prints.is_empty()
            && self.pending_resource_site.is_none()
            && !self.halted
            && !self.boxes.output_routine_active
            && !PendingPageOutputFacts::capture(
                &stores.command_context().expect("terminal owner admission"),
            )
            .is_pending()
    }

    /// TeX82 §§81, 93's fatal `jump_out` discards active procedure frames.
    /// Only the canonical runner can mark the resulting `End` as terminal;
    /// ordinary completion still requires the full named-boundary check.
    pub(crate) fn terminal_revision_is_ready(
        &self,
        stores: &mut Universe<G>,
        step: MainControlStep,
    ) -> bool {
        if self.ended && self.fatal.is_some() && step == MainControlStep::End {
            true
        } else {
            self.terminal_is_quiescent(stores)
        }
    }

    pub(crate) fn mark_ended(&mut self) {
        self.ended = true;
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
        drop(processor);
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
        self.promote_first_recoverable(stores, &mut diagnostic_effects);
        stores
            .world_mut()
            .publish_diagnostic_effects_preserving(&mut diagnostic_effects);
        result
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
        if self.active_alignment.is_some()
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
            || self.prepared_shipout.is_some()
            || self.page_region_succession_pending
            || !self.prepared_dvi_pages.is_empty()
            || !self.immediate_prints.is_empty()
            || self.pending_resource_site.is_some()
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
        if !stores.checkpoint_eligible() {
            return Err(tex_command::CommandSummaryError::CheckpointIneligible);
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
        if !stores.checkpoint_eligible() {
            return Err(tex_command::CommandSummaryError::CheckpointIneligible);
        }
        // A halted command stopped mid-flight; only a restore resumes it.
        if self.halted {
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
        };
        self.capture_checkpoint_with_identity_demand(eligibility, stores, budget_counters, false)
    }

    /// Selects convergence identity for an incremental session. Mode identity
    /// is computed at each demanded quiescent checkpoint boundary.
    #[doc(hidden)]
    pub fn enable_reachable_state_identity(&mut self, stores: &mut Universe<G>) {
        let _ = self.command.enable_reachable_state_identity();
        stores.enable_reachable_state_identity();
    }

    /// Restores a complete named checkpoint and its visible output prefix.
    /// Current ordinary state must be quiescent; interrupted resource calls
    /// use `restore_resource_replay` after their Rust frames have unwound.
    pub fn restore_checkpoint(
        &mut self,
        checkpoint: &crate::EngineCheckpoint<G>,
        stores: &mut Universe<G>,
        output: &mut crate::OutputLedger,
    ) -> Result<(), crate::CheckpointRestoreError> {
        if !stores.checkpoint_eligible() {
            return Err(crate::CheckpointRestoreError::Runtime(
                tex_state::UniverseError::State(tex_state::StateError::CheckpointIneligible),
            ));
        }
        self.restore_checkpoint_after_replay(checkpoint, stores, output)
    }

    pub(crate) fn restore_checkpoint_after_replay(
        &mut self,
        checkpoint: &crate::EngineCheckpoint<G>,
        stores: &mut Universe<G>,
        output: &mut crate::OutputLedger,
    ) -> Result<(), crate::CheckpointRestoreError> {
        let prepared =
            checkpoint.prepare_state_after_replay(&self.command, &self.modes, stores, output)?;
        // No selected component can fail now. Drop interrupted executor
        // owners while their generation/page/PDF backing remains attached;
        // neither aftergroup tokens nor ordinary unsave diagnostics run.
        self.reset_checkpoint_replay_runtime(checkpoint.boundary());
        checkpoint.apply_prepared_state_after_replay(
            &mut self.command,
            &mut self.modes,
            stores,
            output,
            prepared,
        );
        Ok(())
    }

    /// Restores a full aggregate checkpoint after a resource operation has
    /// unwound. Preflight precedes interrupted-owner destruction and backing
    /// restore. The next call starts ordinary main control from
    /// the restored command, mode, World, and output roots.
    pub fn restore_resource_replay(
        &mut self,
        checkpoint: &crate::EngineCheckpoint<G>,
        stores: &mut Universe<G>,
        output: &mut crate::OutputLedger,
    ) -> Result<(), crate::CheckpointRestoreError> {
        self.restore_checkpoint_after_replay(checkpoint, stores, output)?;
        Ok(())
    }

    #[cfg(test)]
    fn restore_semantic_checkpoint(
        &mut self,
        checkpoint: &crate::EngineCheckpoint<G>,
        stores: &mut Universe<G>,
    ) -> Result<(), crate::CheckpointRestoreError> {
        checkpoint.restore_semantic_state(&mut self.command, &mut self.modes, stores)?;
        self.active_alignment = None;
        self.boxes = ReplayBoxes::default();
        self.paragraph_checkpoint_demand = None;
        self.paragraph_checkpoint_cut = false;
        self.error_operation_committed = false;
        self.fatal = None;
        self.captured_fatal_origin = None;
        Ok(())
    }

    /// Clears interrupted executor owners before destructive aggregate restore. These fields are deliberately not
    /// part of `EngineCheckpoint`: a fork constructs them from defaults, and
    /// an in-place resource replay must establish the same quiescent shape.
    pub(crate) fn reset_checkpoint_replay_runtime(&mut self, boundary: crate::EngineBoundary) {
        let root_main_source = (boundary != crate::EngineBoundary::JobStart)
            .then_some(self.root_main_source)
            .flatten();
        // The interrupted command's scratch cannot cross backing restoration.
        // Retire its linear owner while the current generation is attached.
        self.command.abandon_checkpoint_attempt();
        // Host capabilities are the monotonic admission registry for this
        // retained run.  They intentionally live outside the checkpoint:
        // restoring the parser/runtime/output roots must not discard inputs
        // (or other resources) admitted by an earlier replay.  The fresh
        // command episode below will consult the same registry through its
        // ordinary scanner path; only the executor-owned continuation state
        // is reset here.
        self.active_alignment = None;
        self.boxes = ReplayBoxes::default();
        self.active_discretionaries.clear();
        self.active_math_choices.clear();
        self.active_math_fields.clear();
        self.active_math_left_boundaries.clear();
        self.active_math_shifts.clear();
        self.skip_pointer_sources.clear();
        self.muskip_pointer_sources.clear();
        self.main_loop_active = false;
        self.set_box_forbidden_depth = 0;
        self.shown_mode = None;
        self.main_control_entered = boundary != crate::EngineBoundary::JobStart;
        self.startup_terminal_line.clear();
        self.terminal_line_was_empty = None;
        self.root_main_source = root_main_source;
        self.page_output_observations = None;
        self.operation_observations = None;
        self.operation_receipt_start = None;
        self.error_operation_committed = false;
        self.completed_replay_episode = None;
        self.prepared_dvi_pages = PreparedDviPages::default();
        self.immediate_prints.clear();
        self.prepared_shipout = None;
        self.page_region_succession_pending = false;
        self.paragraph_checkpoint_demand = None;
        self.paragraph_checkpoint_cut = false;
        self.pending_resource_site = None;
        self.declined_resource_attempt = None;
        self.halted = false;
        self.ended = false;
        self.fatal = None;
        self.captured_fatal_origin = None;
        self.first_causal_context = None;
        self.first_recoverable_diagnostic = None;
        self.job = crate::job::JobFraming::default();
        self.job_body_effect_end = None;
        self.pdf_navigation_finalized = false;
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
    /// those needs the live job-framing owner. The direct end handler calls
    /// this tail after semantic termination so notices follow the paren close.
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
    ) -> CommandMachine<'operation, 'operation, G> {
        self.command_machine_with_resource_provider(diagnostic_effects, None)
    }

    fn command_machine_with_resource_provider<'operation, 'provider>(
        &'operation mut self,
        diagnostic_effects: &'operation mut DiagnosticEffects,
        resource_provider: Option<&'provider mut dyn ResourceProvider<G>>,
    ) -> CommandMachine<'operation, 'provider, G> {
        CommandMachine {
            state: &mut self.command,
            fuel: self.fuel.fuel_mut(),
            capabilities: &mut self.capabilities,
            resource_provider,
            declined_resource_attempt: &mut self.declined_resource_attempt,
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

    /// Returns the command site retained for the most recent resource need.
    #[must_use]
    pub const fn pending_resource_site(&self) -> Option<OriginId> {
        self.pending_resource_site
    }

    /// Releases the diagnostic-only site retained for the resource need that
    /// has just been answered by the outer ledger. The answer does not revive
    /// this direct execution; aggregate checkpoint replay clears the
    /// invalidation latch below.
    pub(crate) fn acknowledge_resource_need(&mut self) {
        self.pending_resource_site = None;
    }

    /// Reports whether the most recent error returned after its direct
    /// operation had committed. This is the observation driver's publication
    /// decision; it does not classify or replace the typed error.
    #[must_use]
    pub const fn error_operation_committed(&self) -> bool {
        self.error_operation_committed
    }

    /// Consumes the one provider-decline sideband produced during the current
    /// step. The outer driver uses it to suspend without invoking its legacy
    /// host a second time.
    pub(crate) fn take_declined_resource_attempt(&mut self) -> Option<ResourceNeed> {
        self.declined_resource_attempt.take()
    }

    fn ensure_not_halted(&self) -> Result<(), ExecError> {
        if self.halted {
            Err(ExecError::Halted)
        } else {
            Ok(())
        }
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

    /// Installs the host's compact paragraph policy before execution.
    pub(crate) fn set_paragraph_checkpoint_demand(&mut self, identity: Option<bool>) {
        debug_assert!(!self.paragraph_checkpoint_cut);
        self.paragraph_checkpoint_demand = identity;
    }

    pub(crate) fn take_paragraph_checkpoint(
        &mut self,
        stores: &Universe<G>,
    ) -> Option<(crate::checkpoint::CheckpointEligibility, bool)> {
        if !std::mem::take(&mut self.paragraph_checkpoint_cut) {
            return None;
        }
        let wants_identity = self.paragraph_checkpoint_demand.take()?;
        let restartable = stores.checkpoint_eligible()
            && self.command.named_boundary_is_quiescent()
            && self.active_alignment.is_none()
            && self.operation_observations.is_none()
            && self.operation_receipt_start.is_none()
            && self.prepared_shipout.is_none()
            && !self.page_region_succession_pending
            && self.immediate_prints.is_empty()
            && self.pending_resource_site.is_none()
            && !self.halted
            && !self.boxes.output_routine_active
            && self.modes.restart_checkpoint_is_quiescent();
        restartable.then(|| {
            (
                crate::checkpoint::CheckpointEligibility::named(
                    crate::EngineBoundary::OuterParagraphEnd,
                ),
                wants_identity,
            )
        })
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
        self.ensure_not_halted()?;
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
        mut tracked_region: Option<&mut Option<Result<TrackedRegionRecord, DependencyRegionError>>>,
        mut resource_provider: Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<StepResult, ExecError> {
        self.error_operation_committed = false;
        let initial_effect_pos = stores.world().effect_pos();
        let initial_artifacts = stores.world().committed_artifacts().len();
        let initial_boundaries = 0;
        let initial_format_dump = self.dumped_format.is_some();
        let initial_diagnostic = self.first_causal_context.is_some();
        let initial_error_count = stores.world().error_channel().error_count();
        let mut operations = 0usize;
        let mut tracked_mark = if tracked_region.is_some() {
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
        self.episode_telemetry.record_attempt();
        self.advance_telemetry.attempts += 1;
        loop {
            let mut diagnostic_effects = DiagnosticEffects::new();
            let mut completed_native_commands = 0;
            let pending =
                if !self.page_region_succession_pending && !self.boxes.output_routine_active {
                    PendingPageOutputFacts::capture_runtime(stores)
                } else {
                    PendingPageOutputFacts::default()
                };
            let applied = if pending.is_pending() {
                self.fire_pending_page_output(stores, &mut diagnostic_effects, pending)
                    .map(|()| ReplayStep::Continue)
            } else {
                self.execute_control_chunk(
                    stores,
                    None,
                    None,
                    &mut diagnostic_effects,
                    tracked_mark.is_some(),
                    &mut resource_provider,
                    max_operations - operations,
                    &mut completed_native_commands,
                )
            };
            operations += completed_native_commands + 1;
            self.update_save_stack_highwater(stores);
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
                    if let Some(mark) = tracked_mark.take() {
                        if error.as_fatal().is_some() || error.requires_terminal_settlement() {
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
                    if execution_error_is_fuel(&error) {
                        self.episode_telemetry
                            .record_semantic_barrier(crate::SemanticEpisodeBarrier::Fuel);
                    }
                    if execution_error_is_resource(&error) {
                        let result = self.finish_resource_preflight_failure(
                            stores,
                            error,
                            &mut diagnostic_effects,
                        );
                        if matches!(result, Ok(StepResult::Suspended(_))) {
                            self.halt_direct_operation();
                            self.advance_telemetry.resource_halts += 1;
                            #[cfg(feature = "profiling")]
                            self.episode_telemetry
                                .record_rollback(crate::SemanticEpisodeBarrier::Resource, 1);
                            #[cfg(not(feature = "profiling"))]
                            self.episode_telemetry
                                .record_rollback(crate::SemanticEpisodeBarrier::Resource);
                        } else if result.is_err() {
                            let evidence = self.commit_direct_operation_after_error(
                                stores,
                                &mut diagnostic_effects,
                                OperationTermination::Failed,
                            );
                            return evidence.map_or(result, Err);
                        } else {
                            if diagnostic_effects.has_first_recoverable() {
                                self.promote_first_recoverable(stores, &mut diagnostic_effects);
                            }
                        }
                        return result;
                    }
                    return self.finish_direct_failure(
                        stores,
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
                .publish_diagnostic_effects_preserving(&mut diagnostic_effects);
            if let Some(error) =
                self.admit_observed_receipt(stores, operation_termination(step, self.fatal))
            {
                if diagnostic_effects.has_first_recoverable() {
                    self.promote_first_recoverable(stores, &mut diagnostic_effects);
                }
                return Err(error);
            }
            if diagnostic_effects.has_first_recoverable() {
                self.promote_first_recoverable(stores, &mut diagnostic_effects);
            }
            let tracked_result = tracked_mark.take().map(|mark| {
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

    /// Runs up to `max_operations` commands. Every error leaves owners
    /// wherever the failing command stopped, so it latches [`Self::halted`]
    /// until a full checkpoint restore replaces this control.
    fn execute_operation(
        &mut self,
        stores: &mut Universe<G>,
        max_operations: usize,
        tracked_region: Option<&mut Option<Result<TrackedRegionRecord, DependencyRegionError>>>,
        resource_provider: Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<StepResult, ExecError> {
        // Once execution begins, this control can no longer truthfully
        // produce job-start restart eligibility even if the caller skipped
        // the ordinary initial publication hook.
        self.job_start_eligibility = None;
        let result =
            self.execute_direct_episode(stores, max_operations, tracked_region, resource_provider);
        if result.is_err() {
            self.halted = true;
        }
        result
    }

    /// Shared entry preamble. Returns `false` when a recorded fatal stop
    /// leaves only the terminal `End` step to deliver.
    fn begin_advance(&mut self, stores: &mut Universe<G>) -> Result<bool, ExecError> {
        if self.ended {
            return Err(ExecError::ExecutionAlreadyTerminated);
        }
        self.declined_resource_attempt = None;
        self.ensure_not_halted()?;
        if !self.pure_memo_initialized {
            let runtime = stores.take_pure_memo_config().map_or_else(
                tex_state::PureMemoRuntime::default,
                tex_state::PureMemoRuntime::new,
            );
            self.install_pure_memo_runtime(runtime);
        }
        stores.attach_pure_memo_capability(&self.pure_memo);
        Ok(self.fatal.is_none())
    }

    fn advance_chunk(
        &mut self,
        stores: &mut Universe<G>,
        max_operations: usize,
        resource_provider: Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<StepResult, ExecError> {
        if !self.begin_advance(stores)? {
            return Ok(StepResult::Progress(MainControlStep::End));
        }
        self.execute_operation(stores, max_operations, None, resource_provider)
    }

    fn advance_tracked(
        &mut self,
        stores: &mut Universe<G>,
        resource_provider: Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<TrackedStepResult, ExecError> {
        let mut region = None;
        let step = if self.begin_advance(stores)? {
            self.execute_operation(stores, 1, Some(&mut region), resource_provider)?
        } else {
            StepResult::Progress(MainControlStep::End)
        };
        Ok(TrackedStepResult { step, region })
    }

    /// Attempts one main-control operation.
    ///
    /// Missing retained input is returned as a typed suspension that halts
    /// this control. The outer checkpoint owner must restore a full
    /// checkpoint before entering main control again; this object never
    /// resumes a parked scanner or caller frame.
    pub fn advance(&mut self, stores: &mut Universe<G>) -> Result<StepResult, ExecError> {
        self.advance_chunk(stores, 1, None)
    }

    /// Provider-aware ordinary advance. A ready or unavailable resource is
    /// installed during this call and the operation continues in place;
    /// only a declined provider result reaches the outer suspension seam.
    pub fn advance_with_resource_provider(
        &mut self,
        stores: &mut Universe<G>,
        resource_provider: &mut dyn ResourceProvider<G>,
    ) -> Result<StepResult, ExecError> {
        self.advance_chunk(stores, 1, Some(resource_provider))
    }

    /// Advances one production driver chunk. The public one-operation
    /// [`Self::advance`] contract remains available to diagnostic and
    /// focused-test callers.
    pub fn advance_episode(&mut self, stores: &mut Universe<G>) -> Result<StepResult, ExecError> {
        self.advance_chunk(stores, 256, None)
    }

    /// Provider-aware production-sized episode advance.
    pub fn advance_episode_with_resource_provider(
        &mut self,
        stores: &mut Universe<G>,
        resource_provider: &mut dyn ResourceProvider<G>,
    ) -> Result<StepResult, ExecError> {
        self.advance_chunk(stores, 256, Some(resource_provider))
    }

    /// Attempts one ordinary main-control operation while collecting detached
    /// semantic dependency evidence for that operation only.
    ///
    /// Recording failure never changes the TeX result. A committed supported
    /// operation returns a record, a fail-closed barrier returns its typed
    /// rejection, and a failure or resource suspension returns no region.
    pub fn advance_with_tracked_region(
        &mut self,
        stores: &mut Universe<G>,
    ) -> Result<TrackedStepResult, ExecError> {
        self.advance_tracked(stores, None)
    }

    /// Provider-aware tracked advance used by observed dependency routes.
    pub fn advance_with_tracked_region_and_resource_provider(
        &mut self,
        stores: &mut Universe<G>,
        resource_provider: &mut dyn ResourceProvider<G>,
    ) -> Result<TrackedStepResult, ExecError> {
        self.advance_tracked(stores, Some(resource_provider))
    }

    /// Expands one command for an analysis host without entering ordinary
    /// typesetting. TeX82 §1270's command-code partition still routes every
    /// assignment through §1211 `prefixed_command`; other spellings, including
    /// undefined control sequences, are returned to the host with provenance.
    pub fn diagnostic_expand_step(
        &mut self,
        stores: &mut Universe<G>,
    ) -> Result<DiagnosticStepResult, ExecError> {
        self.declined_resource_attempt = None;
        self.diagnostic_expand_step_inner(stores, None)
    }

    /// Provider-aware diagnostic expansion. Resource answers are resolved in
    /// the same admitted command operation; only a declined answer reaches
    /// the diagnostic suspension result.
    pub fn diagnostic_expand_step_with_resource_provider(
        &mut self,
        stores: &mut Universe<G>,
        resource_provider: &mut dyn ResourceProvider<G>,
    ) -> Result<DiagnosticStepResult, ExecError> {
        self.declined_resource_attempt = None;
        self.diagnostic_expand_step_inner(stores, Some(resource_provider))
    }

    fn diagnostic_expand_step_inner(
        &mut self,
        stores: &mut Universe<G>,
        mut resource_provider: Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<DiagnosticStepResult, ExecError> {
        self.ensure_not_halted()?;
        let mut diagnostic_effects = DiagnosticEffects::new();
        self.ensure_primitive_handles(stores);
        let (command, cursor) = {
            self.ensure_primitive_handles(stores);
            let (status, destination, cursor, source_provenance) = {
                let mut context = stores.command_context().expect("live generation");
                let mut host_facts = ExecutorHostFacts {
                    modes: &self.modes,
                    pdf_ignore_depth: self.pdf_ignore_depth,
                    telemetry: &mut self.episode_telemetry,
                };
                let mut processor = if let Some(provider) = resource_provider.as_deref_mut() {
                    command_processor_with_resource_provider(
                        &mut self.command,
                        self.fuel.fuel_mut(),
                        &mut self.capabilities,
                        &mut host_facts,
                        provider,
                        &mut self.declined_resource_attempt,
                        &mut self.operation_observations,
                        &mut diagnostic_effects,
                        &mut context,
                    )
                } else {
                    command_processor(
                        &mut self.command,
                        self.fuel.fuel_mut(),
                        &mut self.capabilities,
                        &mut host_facts,
                        &mut self.operation_observations,
                        &mut diagnostic_effects,
                        &mut context,
                    )
                };
                let mut destination = None;
                let status = processor
                    .get_x_token_preserving_undefined_into(&mut destination)
                    .map_err(command_error);
                let cursor = processor.delivery_cursor();
                let source_provenance =
                    if matches!(status, Ok(tex_command::DeliveryStatus::Command)) {
                        destination
                            .as_ref()
                            .and_then(|command| processor.source_provenance(command))
                    } else {
                        None
                    };
                (status, destination, cursor, source_provenance)
            };
            let status = match status {
                Ok(status) => status,
                Err(error) => {
                    let result = self.finish_resource_preflight_failure(
                        stores,
                        error,
                        &mut diagnostic_effects,
                    );
                    if matches!(result, Ok(StepResult::Suspended(_))) {
                        // The host retry starts from a retained full
                        // checkpoint; no expansion or caller frame survives.
                        self.halt_direct_operation();
                    } else {
                        if diagnostic_effects.has_first_recoverable() {
                            self.promote_first_recoverable(stores, &mut diagnostic_effects);
                        }
                    }
                    return match result? {
                        StepResult::Suspended(need) => Ok(DiagnosticStepResult::Suspended(need)),
                        StepResult::Progress(_) => {
                            unreachable!("diagnostic expansion failure made progress")
                        }
                    };
                }
            };
            let command = match status {
                tex_command::DeliveryStatus::Command => {
                    destination.expect("command status filled diagnostic caller slot")
                }
                tex_command::DeliveryStatus::End => {
                    debug_assert!(destination.is_none());
                    if diagnostic_effects.has_first_recoverable() {
                        self.promote_first_recoverable(stores, &mut diagnostic_effects);
                    }
                    return Ok(DiagnosticStepResult::Progress(DiagnosticStep::EndOfInput));
                }
                _ => unreachable!("diagnostic expanded delivery returns command or end"),
            };
            if !tex_command::exceeds_max_non_prefixed_command(static_meaning(command.meaning())) {
                let step = DiagnosticStep::Token {
                    spelling: command.spelling(),
                    meaning: static_meaning(command.meaning()),
                    control_sequence: command.control_sequence(),
                    source_provenance,
                };
                if diagnostic_effects.has_first_recoverable() {
                    self.promote_first_recoverable(stores, &mut diagnostic_effects);
                }
                return Ok(DiagnosticStepResult::Progress(step));
            }
            (command, cursor)
        };
        let applied = self.execute_control(
            stores,
            Some(command),
            Some(cursor),
            &mut diagnostic_effects,
            false,
            &mut resource_provider,
        );
        let applied = match applied {
            Err(error) if execution_error_is_resource(&error) => {
                let result =
                    self.finish_resource_preflight_failure(stores, error, &mut diagnostic_effects);
                if matches!(result, Ok(StepResult::Suspended(_))) {
                    self.halt_direct_operation();
                } else {
                    if diagnostic_effects.has_first_recoverable() {
                        self.promote_first_recoverable(stores, &mut diagnostic_effects);
                    }
                }
                return match result? {
                    StepResult::Suspended(need) => Ok(DiagnosticStepResult::Suspended(need)),
                    StepResult::Progress(_) => {
                        unreachable!("diagnostic assignment failure made progress")
                    }
                };
            }
            other => other,
        };
        match applied {
            Ok(_) => {
                if diagnostic_effects.has_first_recoverable() {
                    self.promote_first_recoverable(stores, &mut diagnostic_effects);
                }
                stores
                    .world_mut()
                    .publish_diagnostic_effects_preserving(&mut diagnostic_effects);
                Ok(DiagnosticStepResult::Progress(DiagnosticStep::Assignment))
            }
            Err(error) => {
                self.halt_direct_operation();
                let mut stores = stores.command_context().expect("live generation");
                Err(error.freeze_diagnostic_origin(
                    &mut stores,
                    self.command.diagnostic_input_context(8),
                ))
            }
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
        tracked_region_is_active: bool,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<ReplayStep, ExecError> {
        let result = self.apply_operation(
            stores,
            settled,
            diagnostic_effects,
            tracked_region_is_active,
            resource_provider,
        );
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
        tracked_region_is_active: bool,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
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
        let base = self.do_assignments_then_accent_base(
            stores,
            diagnostic_effects,
            tracked_region_is_active,
            resource_provider,
        )?;
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
        tracked_region_is_active: bool,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
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
                let mut processor = if let Some(provider) = resource_provider.as_deref_mut() {
                    command_processor_with_resource_provider(
                        &mut self.command,
                        self.fuel.fuel_mut(),
                        &mut self.capabilities,
                        &mut host_facts,
                        provider,
                        &mut self.declined_resource_attempt,
                        &mut self.operation_observations,
                        diagnostic_effects,
                        &mut context,
                    )
                } else {
                    command_processor(
                        &mut self.command,
                        self.fuel.fuel_mut(),
                        &mut self.capabilities,
                        &mut host_facts,
                        &mut self.operation_observations,
                        diagnostic_effects,
                        &mut context,
                    )
                };
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
                    let step = self.execute_nested_operation(
                        stores,
                        Some(command),
                        diagnostic_effects,
                        tracked_region_is_active,
                        resource_provider,
                    );
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
                        .publish_diagnostic_effects_preserving(diagnostic_effects);
                    let mut command = CommandMachine {
                        state: &mut self.command,
                        fuel: self.fuel.fuel_mut(),
                        capabilities: &mut self.capabilities,
                        resource_provider: None,
                        declined_resource_attempt: &mut self.declined_resource_attempt,
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
                        self.page_output_observations
                            .get_or_insert_with(ObservationBuffer::default)
                            .append(&mut deferred);
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
        tracked_region_is_active: bool,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<tex_state::page_node_arena::PageListId, ExecError> {
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
            .publish_diagnostic_effects_preserving(diagnostic_effects);
        self.main_loop_active = false;
        while stores
            .command_context()
            .expect("live generation")
            .group_frames()
            .len()
            > enclosing_depth
        {
            let step = self.execute_nested_operation(
                stores,
                None,
                diagnostic_effects,
                tracked_region_is_active,
                resource_provider,
            )?;
            // `execute_live_math_choice_group` fuses the body into its opener's
            // outer operation, but TeX finishes each nested command before
            // fetching the next one. Publish its completed diagnostic
            // program at that same boundary so a following immediate World
            // write cannot overtake tracing from `\vcenter`, math choices,
            // or any other nested group transition.
            stores
                .world_mut()
                .publish_diagnostic_effects_preserving(diagnostic_effects);
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
    ) -> Result<tex_state::page_node_arena::PageListId, ExecError> {
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
        tracked_region_is_active: bool,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<tex_state::page_node_arena::PageListId, ExecError> {
        self.command_scan_math_choice_group(stores, diagnostic_effects, resource_provider)?;
        self.execute_live_math_choice_group(
            stores,
            diagnostic_effects,
            tracked_region_is_active,
            resource_provider,
        )
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
        tracked_region_is_active: bool,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
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
                let episode =
                    self.command_scan_math_field(stores, diagnostic_effects, resource_provider)?;
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
                let episode =
                    self.command_scan_math_field(stores, diagnostic_effects, resource_provider)?;
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
                    let display = self.execute_math_choice_branch(
                        stores,
                        diagnostic_effects,
                        tracked_region_is_active,
                        resource_provider,
                    )?;
                    *self
                        .active_math_choices
                        .last_mut()
                        .expect("live math choice") = 1;
                    let text = self.execute_math_choice_branch(
                        stores,
                        diagnostic_effects,
                        tracked_region_is_active,
                        resource_provider,
                    )?;
                    *self
                        .active_math_choices
                        .last_mut()
                        .expect("live math choice") = 2;
                    let script = self.execute_math_choice_branch(
                        stores,
                        diagnostic_effects,
                        tracked_region_is_active,
                        resource_provider,
                    )?;
                    *self
                        .active_math_choices
                        .last_mut()
                        .expect("live math choice") = 3;
                    let script_script = self.execute_math_choice_branch(
                        stores,
                        diagnostic_effects,
                        tracked_region_is_active,
                        resource_provider,
                    )?;
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
                let episode =
                    self.command_scan_math_field(stores, diagnostic_effects, resource_provider)?;
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
                    self.command_scan_math_character(stores, diagnostic_effects, resource_provider)?
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
                let episode =
                    self.command_scan_math_field(stores, diagnostic_effects, resource_provider)?;
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
            MathRequest::MuMaterial(ScannedMathMuMaterial::Glue { spec, shared_zero }) => {
                self.modes.current_list_mutation().push(
                    &mut stores.command_context().expect("math glue admission"),
                    Node::Glue {
                        origin: if shared_zero {
                            tex_state::node::GlueSpecOrigin::SharedZero
                        } else {
                            tex_state::node::GlueSpecOrigin::Owned
                        },
                        spec,
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
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
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
            resource_provider,
        )?;
        Ok(ReplayStep::Continue)
    }

    fn apply_math_shift(
        &mut self,
        pairing: MathShiftPairing,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
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
                    let paired =
                        self.scan_display_end(stores, diagnostic_effects, resource_provider)?;
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
                        resource_provider,
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
                    let paired =
                        self.scan_display_end(stores, diagnostic_effects, resource_provider)?;
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
                        resource_provider,
                    )?;
                    return Ok(ReplayStep::Continue);
                }
                let (content, display_level) =
                    self.prepare_display_math_list(stores, diagnostic_effects)?;
                let paired =
                    self.scan_display_end(stores, diagnostic_effects, resource_provider)?;
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
                    resource_provider,
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
    ) -> Result<tex_state::page_node_arena::PageListId, ExecError> {
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
            tex_state::page_node_arena::PageListId::empty()
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
            tex_state::page_node_arena::PageListId,
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
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
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
        let provider = resource_provider.take();
        let mut machine = self.command_machine_with_resource_provider(diagnostic_effects, provider);
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
        let provider = machine.resource_provider.take();
        drop(machine);
        *resource_provider = provider;
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
            content = tex_state::page_node_arena::PageListId::empty();
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
        unsave_group(
            &mut context,
            &mut self.command_machine(diagnostic_effects),
            GroupKind::MathShift,
        )
        .map_err(|_| ExecError::MissingToken {
            context: "math shift group",
        })?;
        self.active_math_shifts.pop();
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
        content: tex_state::page_node_arena::PageListId,
    ) -> Result<
        (
            tex_state::page_node_arena::PageListId,
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
        unsave_group(
            &mut context,
            &mut self.command_machine(diagnostic_effects),
            GroupKind::MathShift,
        )
        .map_err(|_| ExecError::MissingToken {
            context: "equation number group",
        })?;
        self.active_math_shifts.pop();
        Ok((eq.display.list(), finished))
    }

    fn finish_display_alignment(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
        finished: crate::align::FinishedAlignment,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<(), ExecError> {
        self.finish_display_alignment_inner(
            stores,
            diagnostic_effects,
            finished,
            true,
            resource_provider,
        )
    }

    fn finish_display_alignment_inner(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
        finished: crate::align::FinishedAlignment,
        scan_optional_space: bool,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
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
        unsave_group(
            &mut context,
            &mut self.command_machine(diagnostic_effects),
            GroupKind::MathShift,
        )
        .map_err(|_| ExecError::MissingToken {
            context: "display alignment group",
        })?;
        self.active_math_shifts.pop();
        drop(context);
        self.resume_display_inner(
            stores,
            diagnostic_effects,
            interrupt.active_directions,
            scan_optional_space,
            resource_provider,
        )
    }

    #[allow(clippy::too_many_arguments)] // display completion owns independent mode/PDF/replay state
    fn finish_display_math_content(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
        mut content: tex_state::page_node_arena::PageListId,
        eq_no: Option<crate::math::display::FinishedEqNo>,
        fonts_checked: bool,
        display_level: Option<crate::mode::ModeLevelSummary>,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
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
            content = tex_state::page_node_arena::PageListId::empty();
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
        unsave_group(
            &mut context,
            &mut self.command_machine(diagnostic_effects),
            GroupKind::MathShift,
        )
        .map_err(|_| ExecError::MissingToken {
            context: "display math group",
        })?;
        self.active_math_shifts.pop();
        drop(context);
        self.resume_display(
            stores,
            diagnostic_effects,
            active_directions,
            resource_provider,
        )
    }

    fn resume_display(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
        directions: Vec<tex_state::node::Direction>,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<(), ExecError> {
        self.resume_display_inner(
            stores,
            diagnostic_effects,
            directions,
            true,
            resource_provider,
        )
    }

    fn resume_display_inner(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
        directions: Vec<tex_state::node::Direction>,
        scan_optional_space: bool,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
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
            .begin_paragraph_hyphen_context(language, left, right);
        self.modes.current_list_mutation().set_space_factor(1000);
        self.modes.current_list_mutation().append(
            &mut stores
                .command_context()
                .expect("display direction admission"),
            directions.into_iter().map(Node::Direction),
        );
        if scan_optional_space {
            self.scan_optional_space(stores, diagnostic_effects, resource_provider)?;
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
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<(), ExecError> {
        let mode = self.modes.current_mode();
        let shown_mode = self.shown_mode;
        let mut context = stores
            .command_context()
            .expect("optional-space scan requires a live generation");
        let provider = resource_provider.take();
        let mut machine = self.command_machine_with_resource_provider(diagnostic_effects, provider);
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
        let provider = machine.resource_provider.take();
        drop(machine);
        *resource_provider = provider;
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
                    unsave_group(
                        &mut context,
                        &mut self.command_machine(diagnostic_effects),
                        GroupKind::MathLeft,
                    )
                    .map_err(|_| ExecError::MissingToken {
                        context: "math left group",
                    })?;
                    self.active_math_left_boundaries.pop();

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
                unsave_group(
                    &mut context,
                    &mut self.command_machine(diagnostic_effects),
                    GroupKind::MathLeft,
                )
                .map_err(|_| ExecError::MissingToken {
                    context: "math left group",
                })?;
                self.active_math_left_boundaries.pop();
                let boundary = context.publish_page_nodes([Node::MathNoad(MathNoad::new(
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
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<tex_command::MathFieldEpisode, ExecError> {
        let mut context = stores.command_context().expect("live generation");
        let mut host_facts = ExecutorHostFacts {
            modes: &self.modes,
            pdf_ignore_depth: self.pdf_ignore_depth,
            telemetry: &mut self.episode_telemetry,
        };
        let mut processor = if let Some(provider) = resource_provider.as_deref_mut() {
            command_processor_with_resource_provider(
                &mut self.command,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
                &mut host_facts,
                provider,
                &mut self.declined_resource_attempt,
                &mut self.operation_observations,
                diagnostic_effects,
                &mut context,
            )
        } else {
            command_processor(
                &mut self.command,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
                &mut host_facts,
                &mut self.operation_observations,
                diagnostic_effects,
                &mut context,
            )
        };
        let scanned = processor.scan_math_field_episode();
        scanned.map_err(command_error)
    }

    /// Runs TeX82 §436's `scan_fifteen_bit_int` after an executor-owned
    /// diagnostic has completed, as required by §1110's text-accent path.
    fn command_scan_math_character(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<tex_command::ScannedMathCharacter, ExecError> {
        let mut context = stores.command_context().expect("live generation");
        let mut host_facts = ExecutorHostFacts {
            modes: &self.modes,
            pdf_ignore_depth: self.pdf_ignore_depth,
            telemetry: &mut self.episode_telemetry,
        };
        let mut processor = if let Some(provider) = resource_provider.as_deref_mut() {
            command_processor_with_resource_provider(
                &mut self.command,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
                &mut host_facts,
                provider,
                &mut self.declined_resource_attempt,
                &mut self.operation_observations,
                diagnostic_effects,
                &mut context,
            )
        } else {
            command_processor(
                &mut self.command,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
                &mut host_facts,
                &mut self.operation_observations,
                diagnostic_effects,
                &mut context,
            )
        };
        processor.scan_math_character().map_err(command_error)
    }

    /// TeX82 §1172/§1174's `scan_left_brace` for one `\mathchoice` branch.
    /// §403 recovery opens the group anyway, so the recovered flag is
    /// diagnostic only.
    fn command_scan_math_choice_group(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<bool, ExecError> {
        let mut context = stores.command_context().expect("live generation");
        let mut host_facts = ExecutorHostFacts {
            modes: &self.modes,
            pdf_ignore_depth: self.pdf_ignore_depth,
            telemetry: &mut self.episode_telemetry,
        };
        let mut processor = if let Some(provider) = resource_provider.as_deref_mut() {
            command_processor_with_resource_provider(
                &mut self.command,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
                &mut host_facts,
                provider,
                &mut self.declined_resource_attempt,
                &mut self.operation_observations,
                diagnostic_effects,
                &mut context,
            )
        } else {
            command_processor(
                &mut self.command,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
                &mut host_facts,
                &mut self.operation_observations,
                diagnostic_effects,
                &mut context,
            )
        };
        let scanned = processor.scan_math_choice_group();
        scanned.map_err(command_error)
    }

    /// Atomic observed variant of [`Self::advance`]. Observations are held
    /// until both command delivery and executor application have committed.
    pub fn advance_with_observer(
        &mut self,
        stores: &mut Universe<G>,
        observer: &mut dyn CommandObserver,
    ) -> Result<StepResult, ExecError> {
        self.advance_with_observer_inner(stores, observer, None)
    }

    /// Atomic observed advance with the cold resource provider enabled.
    /// Ready and unavailable answers continue inside this operation; a
    /// declined answer is returned as the ordinary resource suspension.
    pub fn advance_with_observer_and_resource_provider(
        &mut self,
        stores: &mut Universe<G>,
        observer: &mut dyn CommandObserver,
        resource_provider: &mut dyn ResourceProvider<G>,
    ) -> Result<StepResult, ExecError> {
        self.advance_with_observer_inner(stores, observer, Some(resource_provider))
    }

    fn advance_with_observer_inner(
        &mut self,
        stores: &mut Universe<G>,
        observer: &mut dyn CommandObserver,
        resource_provider: Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<StepResult, ExecError> {
        if !self.begin_advance(stores)? {
            return Ok(StepResult::Progress(MainControlStep::End));
        }
        let effect_start = stores.world().effect_pos();
        let artifact_start = stores.world().committed_artifacts().len();
        // Occupying the slot is what makes this operation observed. Every
        // command-processor episode the operation runs, including the nested
        // ones a host-applied step runs, publishes into this one buffer.
        self.operation_observations = Some(ObservationBuffer::default());
        self.operation_receipt_start = Some(OperationReceiptStart {
            effect: effect_start.raw(),
            artifact: artifact_start,
        });
        let stepped = self.execute_operation(stores, 1, None, resource_provider);
        let mut pending = self.operation_observations.take().unwrap_or_default();
        self.operation_receipt_start.take();
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
        let publish = matches!(stepped, Ok(StepResult::Progress(_)))
            || (stepped.is_err() && self.error_operation_committed());
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
        let (site, frozen, context, first_recoverable) =
            if let Some((site, frozen, context)) = self.captured_fatal_origin.as_ref() {
                (
                    *site,
                    frozen.clone(),
                    context.clone(),
                    self.first_recoverable_diagnostic.as_deref().cloned(),
                )
            } else if fatal == FatalError::TooManyErrors {
                let first = self.first_recoverable_diagnostic.as_deref()?;
                (
                    DiagnosticSite::new(None),
                    first.origin.clone(),
                    first.context.clone(),
                    Some(first.clone()),
                )
            } else {
                return None;
            };
        Some(ExecError::Captured {
            error: Box::new(ExecError::Fatal(fatal)),
            site,
            frozen: Some(Box::new(crate::FrozenDiagnosticEvidence {
                origin: frozen.clone(),
                context: context.clone(),
                first_recoverable: first_recoverable.map(Box::new),
            })),
        })
    }

    /// Returns the first committed recoverable diagnostic retained by this
    /// run, without exposing it to semantic state or output projections.
    #[must_use]
    pub fn first_recoverable_diagnostic(&self) -> Option<&crate::FirstRecoverableDiagnostic> {
        self.first_recoverable_diagnostic.as_deref()
    }

    /// Moves the first committed recoverable diagnostic into a completed-run
    /// result. The run owner is the only caller that may consume this slot.
    pub fn take_first_recoverable_diagnostic(
        &mut self,
    ) -> Option<Box<crate::FirstRecoverableDiagnostic>> {
        self.first_recoverable_diagnostic.take()
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
        tracked_region_is_active: bool,
        resource_provider: &mut Option<&mut dyn ResourceProvider<G>>,
    ) -> Result<ReplayStep, ExecError> {
        self.execute_control(
            stores,
            command,
            None,
            diagnostic_effects,
            tracked_region_is_active,
            resource_provider,
        )
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
    /// Scans the initial terminal filename through canonical command delivery,
    /// retaining committed observations for the caller.
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
            .publish_diagnostic_effects_preserving(&mut diagnostic_effects);
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
    deleted: tex_state::page_node_arena::PageListId,
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
    command: &mut CommandMachine<'_, '_, G>,
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
        Some(tex_state::node_view::NodeView::MathNoad(noad))
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
                    Some(tex_state::node_view::NodeView::MathNoad(accent))
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
    output: tex_state::page_node_arena::PageListId,
    incomplete: Option<crate::mode::IncompleteFraction>,
    stores: &mut CommandContext<'_, G>,
) -> Result<tex_state::page_node_arena::PageListId, ExecError> {
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
            let fraction = stores.publish_page_nodes([fraction]);
            return Ok(stores.compose_page_node_sequences(&[prefix.list(), fraction]));
        }
        return Ok(stores.publish_page_nodes([fraction]));
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
    list: tex_state::page_node_arena::PageListId,
) -> MathField {
    let nodes = stores
        .page_node_list(list)
        .expect("math group belongs to the live page arena")
        .nodes();
    if nodes.len() == 1
        && let Some(tex_state::node_view::NodeView::MathNoad(noad)) = nodes.get(0)
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
) -> Result<tex_state::page_node_arena::PageListId, ExecError> {
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
    let starts_left_node = |node: Option<tex_state::node_view::NodeView<'_>>| {
        matches!(
            node,
            Some(tex_state::node_view::NodeView::MathNoad(MathNoad {
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

    fn capture_runtime<G>(stores: &Universe<G>) -> Self {
        Self {
            fire_up: stores.page_fire_up(),
            resume_after_output: stores.page_builder_resume_after_output_pending(),
        }
    }

    fn is_pending(self) -> bool {
        self.fire_up.is_some() || self.resume_after_output
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
        | ExecError::Command(CommandError::FuelExhausted(_)) => true,
        _ => false,
    }
}

fn execution_error_is_resource(error: &ExecError) -> bool {
    match error {
        ExecError::Captured { error, .. } => execution_error_is_resource(error),
        ExecError::MissingInput { .. }
        | ExecError::MissingInputProbe { .. }
        | ExecError::MissingFont { .. }
        | ExecError::MissingPdfImage { .. } => true,
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
                control.advance(stores).expect("paragraph start"),
                StepResult::Progress(MainControlStep::Continue)
            );
            assert_eq!(
                control.advance(stores).expect("explicit hyphen"),
                StepResult::Progress(MainControlStep::Continue)
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
                control.advance(stores).expect("paragraph start"),
                StepResult::Progress(MainControlStep::Continue)
            );
            assert_eq!(
                control.advance(stores).expect("explicit hyphen"),
                StepResult::Progress(MainControlStep::Continue)
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
            while let StepResult::Progress(MainControlStep::Continue) =
                control.advance(stores).expect("write source executes")
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
