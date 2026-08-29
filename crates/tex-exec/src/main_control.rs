//! Production main-control driver.
//!
//! This intentionally owns only typed execution mutations. Raw delivery,
//! expansion, macro calls, input nesting, and operand collection remain in
//! `tex-command`; no independent source stack is accepted here.

use std::collections::VecDeque;
use std::ops::ControlFlow;
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
    /// Scalar identity of the one host-selected root main input file.
    ///
    /// This is execution policy, not a source-role classification. Boundary
    /// formation compares it with command state's active external file frame;
    /// token provenance and file names never participate.
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
    /// Move-only restart eligibility produced alongside root-main outer
    /// paragraph evidence. Shipout evidence never enters this lane.
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
    Replay,
    /// The caller-owned operation frame contains the sole live command and
    /// its compact delivery/scanner coordinates.
    Command,
    /// TeX82 §1038's main-loop lookahead delivered this command with bare
    /// `get_next`; it must not acquire an expanded-delivery observation when
    /// the scanner borrow resumes.
    /// Expansion settled in the processor borrow that produced this command,
    /// including its canonical expanded observation. This covers both raw
    /// preflight and an in-place TeX82 `goto reswitch`/§1270 handoff.
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
    /// The caller-owned operation frame already contains the scanned payload.
    Prepared,
}

#[derive(Clone, Copy, Debug)]
enum PreflightCommandPhase {
    /// Ordinary expanded delivery has not yet published its observation.
    Replay,
    Settled,
    Raw,
    Expanding {
        main_loop: bool,
    },
    OperationScan,
    PrefixedCommandScan {
        global: bool,
        flags: MeaningFlags,
        set_box_allowed: bool,
    },
    PrefixScan {
        global: bool,
        flags: MeaningFlags,
        alignment: Option<AlignmentIdentity>,
        set_box_allowed: bool,
    },
    ImmediatePdfRetry(UnexpandablePrimitive),
}

/// The one command owner for delivery, scanning, and a possible retry.
///
/// Ordinary attempts keep this frame in [`OperationFrame`]. Only a genuine
/// suspension moves it into a typed continuation. The large scalar phase is
/// separate from the compact dispatch tag so changing a cursor or scanner
/// never reconstructs the whole carrier.
#[derive(Debug)]
struct PreflightCommand<G> {
    command: Option<tex_command::CurrentCommand<G>>,
    expansion: Option<tex_command::ExpansionWorkKey<G>>,
    phase: PreflightCommandPhase,
    cursor: Option<tex_command::CommandDeliveryCursor>,
    scanner: Option<tex_command::ScannerFrameKey<G>>,
    operation_scan: Option<PendingOperationScanPhase>,
}

impl<G> PreflightCommand<G> {
    fn replay(command: tex_command::CurrentCommand<G>) -> Self {
        Self {
            command: Some(command),
            expansion: None,
            phase: PreflightCommandPhase::Replay,
            cursor: None,
            scanner: None,
            operation_scan: None,
        }
    }

    fn settled(
        command: tex_command::CurrentCommand<G>,
        cursor: Option<tex_command::CommandDeliveryCursor>,
    ) -> Self {
        Self {
            command: Some(command),
            expansion: None,
            phase: PreflightCommandPhase::Settled,
            cursor,
            scanner: None,
            operation_scan: None,
        }
    }

    fn raw(
        command: tex_command::CurrentCommand<G>,
        cursor: Option<tex_command::CommandDeliveryCursor>,
    ) -> Self {
        Self {
            command: Some(command),
            expansion: None,
            phase: PreflightCommandPhase::Raw,
            cursor,
            scanner: None,
            operation_scan: None,
        }
    }

    fn expanding(
        expansion: tex_command::ExpansionWorkKey<G>,
        main_loop: bool,
        cursor: tex_command::CommandDeliveryCursor,
    ) -> Self {
        Self {
            command: None,
            expansion: Some(expansion),
            phase: PreflightCommandPhase::Expanding { main_loop },
            cursor: Some(cursor),
            scanner: None,
            operation_scan: None,
        }
    }

    fn operation_scan(
        command: tex_command::CurrentCommand<G>,
        cursor: tex_command::CommandDeliveryCursor,
        phase: PendingOperationScanPhase,
        scanner: tex_command::ScannerFrameKey<G>,
    ) -> Self {
        Self {
            command: Some(command),
            expansion: None,
            phase: PreflightCommandPhase::OperationScan,
            cursor: Some(cursor),
            scanner: Some(scanner),
            operation_scan: Some(phase),
        }
    }

    fn current(&self) -> &tex_command::CurrentCommand<G> {
        self.command
            .as_ref()
            .expect("live preflight command frame owns its command")
    }

    fn current_option(&self) -> Option<&tex_command::CurrentCommand<G>> {
        self.command.as_ref()
    }

    fn take_current(&mut self) -> tex_command::CurrentCommand<G> {
        self.command
            .take()
            .expect("live preflight command frame owns its command")
    }

    fn take_expansion(&mut self) -> tex_command::ExpansionWorkKey<G> {
        self.expansion
            .take()
            .expect("expanding preflight frame owns its parked expansion")
    }

    fn replace_current(&mut self, command: tex_command::CurrentCommand<G>) {
        self.command = Some(command);
    }

    fn settle(&mut self, command: tex_command::CurrentCommand<G>) {
        self.command = Some(command);
        self.phase = PreflightCommandPhase::Settled;
        self.operation_scan = None;
    }

    fn retain_scanner(
        &mut self,
        cursor: tex_command::CommandDeliveryCursor,
        scanner: Option<tex_command::ScannerFrameKey<G>>,
    ) {
        self.cursor = Some(cursor);
        self.scanner = scanner;
    }

    fn retain_operation_scan(
        &mut self,
        cursor: tex_command::CommandDeliveryCursor,
        phase: PendingOperationScanPhase,
        scanner: tex_command::ScannerFrameKey<G>,
    ) {
        self.phase = PreflightCommandPhase::OperationScan;
        self.cursor = Some(cursor);
        self.scanner = Some(scanner);
        self.operation_scan = Some(phase);
    }

    fn is_command_scan(&self) -> bool {
        matches!(
            self.phase,
            PreflightCommandPhase::OperationScan
                | PreflightCommandPhase::PrefixedCommandScan { .. }
                | PreflightCommandPhase::PrefixScan { .. }
        )
    }

    fn immediate_pdf(primitive: UnexpandablePrimitive) -> Self {
        Self {
            command: None,
            expansion: None,
            phase: PreflightCommandPhase::ImmediatePdfRetry(primitive),
            cursor: None,
            scanner: None,
            operation_scan: None,
        }
    }
}

impl<G> std::ops::Deref for PreflightCommand<G> {
    type Target = tex_command::CurrentCommand<G>;

    fn deref(&self) -> &Self::Target {
        self.current()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RegisterAssignmentScanPhase {
    RegisterIndex,
    OptionalEquals,
    Value,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UnaryOperationScanPhase {
    OptionalEquals,
    Value,
}

#[derive(Debug)]
enum ParagraphShapeScanPhase {
    OptionalEquals,
    Count,
    Indent {
        remaining: usize,
        lines: Vec<ParagraphShapeLine>,
    },
    Width {
        remaining: usize,
        lines: Vec<ParagraphShapeLine>,
        indent: Scaled,
    },
}

#[derive(Debug)]
enum PenaltyArrayScanPhase {
    OptionalEquals,
    Count,
    Value { remaining: usize, values: Vec<i32> },
}

#[derive(Debug)]
enum FontDimenScanPhase {
    Number,
    Font {
        number: i32,
    },
    OptionalEquals {
        number: i32,
        font: FontId,
        recovery_context: Option<String>,
    },
    Value {
        number: i32,
        font: FontId,
        recovery_context: Option<String>,
    },
}

#[derive(Clone, Copy, Debug)]
enum FontIntegerScanPhase {
    Font,
    OptionalEquals { font: FontId },
    Value { font: FontId },
}

#[derive(Clone, Copy, Debug)]
enum CodeTableScanPhase {
    Character,
    OptionalEquals { character: char },
    Value { character: char },
}

#[derive(Clone, Copy, Debug)]
enum PdfFontCodeScanPhase {
    Font,
    Character { font: FontId },
    OptionalEquals { font: FontId, character: u8 },
    Value { font: FontId, character: u8 },
}

#[derive(Clone, Copy, Debug)]
enum PdfFontExpandScanPhase {
    Font,
    OptionalEquals {
        font: FontId,
    },
    Stretch {
        font: FontId,
    },
    Shrink {
        font: FontId,
        stretch: i32,
    },
    Step {
        font: FontId,
        stretch: i32,
        shrink: i32,
    },
    AutoExpand {
        font: FontId,
        stretch: i32,
        shrink: i32,
        step: i32,
    },
}

#[derive(Clone, Copy, Debug)]
enum OpenOutScanPhase {
    Stream,
    OptionalEquals { stream: u8 },
    FileName { stream: u8 },
}

#[derive(Clone, Copy, Debug)]
enum MarksScanPhase {
    Class,
    Text { class: u16 },
}

#[derive(Clone, Copy, Debug)]
enum CatCodeScanPhase {
    Character,
    OptionalEquals { character: char },
    Value { character: char },
}

#[derive(Clone, Copy, Debug)]
enum MathFamilyScanPhase {
    Family,
    OptionalEquals {
        family: tex_command::ScannedMathFamily,
    },
    Font {
        family: tex_command::ScannedMathFamily,
    },
}

#[derive(Clone, Copy, Debug)]
enum ArithmeticIndexedTarget {
    Integer,
    Dimension,
    Glue { mu: bool },
}

#[derive(Clone, Copy, Debug)]
enum ArithmeticScanPhase {
    TargetCommand,
    TargetIndex { target: ArithmeticIndexedTarget },
    Keyword { target: ArithmeticTarget },
    Operand { target: ArithmeticTarget },
}

#[derive(Clone, Copy, Debug)]
enum LeaderGlueResult {
    Payload {
        kind: GlueKind,
        payload: LeaderPayload,
    },
    Register {
        kind: GlueKind,
        index: u16,
        copy: bool,
    },
}

#[derive(Debug)]
enum PendingOperationScanPhase {
    Count {
        index: Option<u16>,
        global: bool,
        phase: RegisterAssignmentScanPhase,
    },
    Dimension {
        index: Option<u16>,
        global: bool,
        phase: RegisterAssignmentScanPhase,
    },
    BoxDimension {
        index: Option<u16>,
        dimension: tex_state::BoxDimension,
        global: bool,
        phase: RegisterAssignmentScanPhase,
    },
    Glue {
        index: Option<u16>,
        global: bool,
        mu: bool,
        phase: RegisterAssignmentScanPhase,
    },
    Unary {
        meaning: Meaning,
        global: bool,
        origin: tex_state::token::OriginId,
        phase: UnaryOperationScanPhase,
    },
    ParagraphShape {
        global: bool,
        phase: ParagraphShapeScanPhase,
    },
    PenaltyArray {
        kind: tex_state::PenaltyArrayKind,
        global: bool,
        phase: PenaltyArrayScanPhase,
    },
    FontDimen(FontDimenScanPhase),
    FontInteger {
        primitive: UnexpandablePrimitive,
        phase: FontIntegerScanPhase,
    },
    CodeTable {
        primitive: UnexpandablePrimitive,
        global: bool,
        phase: CodeTableScanPhase,
    },
    PdfFontCode {
        primitive: UnexpandablePrimitive,
        phase: PdfFontCodeScanPhase,
    },
    PdfFontExpand(PdfFontExpandScanPhase),
    FontOnly {
        meaning: Meaning,
    },
    OpenOut(OpenOutScanPhase),
    Marks(MarksScanPhase),
    CatCode {
        global: bool,
        phase: CatCodeScanPhase,
    },
    MathFamily {
        size: tex_command::MathFamilySize,
        global: bool,
        phase: MathFamilyScanPhase,
    },
    Arithmetic {
        primitive: UnexpandablePrimitive,
        global: bool,
        phase: ArithmeticScanPhase,
    },
    LeaderGlue {
        mode: Mode,
        result: LeaderGlueResult,
    },
    LeaderPayload {
        primitive: UnexpandablePrimitive,
        mode: Mode,
    },
    LeaderCommand {
        mode: Mode,
        result: LeaderGlueResult,
    },
}

fn own_alignment_retry_child<G>(
    alignment: Option<Option<AlignmentIdentity>>,
    cursor: Option<tex_command::CommandDeliveryCursor>,
    retry: Option<PreflightCommand<G>>,
    alignment_scanner: Option<tex_command::ScannerFrameKey<G>>,
) -> Option<PendingDirectDestination<G>> {
    let Some((alignment, cursor)) = alignment.zip(cursor) else {
        assert!(
            alignment_scanner.is_none(),
            "a detached scanner continuation requires its typed alignment destination"
        );
        return retry.map(PendingDirectDestination::Preflight);
    };
    match retry {
        // Alignment remains the caller of its suspended expanded delivery,
        // but it retains only the exact parked root rather than a command
        // projection or scanner wrapper.
        Some(PreflightCommand {
            phase: PreflightCommandPhase::Expanding { .. },
            command: None,
            expansion: Some(expansion),
            scanner: None,
            ..
        }) => {
            assert!(
                alignment_scanner.is_none(),
                "an expansion child and alignment retry cannot share scanner capabilities"
            );
            Some(PendingDirectDestination::Alignment(
                PendingAlignmentDelivery {
                    alignment,
                    cursor,
                    scanner: None,
                    expansion: Some(expansion),
                },
            ))
        }
        // A settled/raw command has already crossed alignment delivery. Its
        // operand scanner, command, and cursor are the exact next caller; an
        // alignment retry would fetch past it and strand that scanner child.
        Some(retry) => {
            assert!(
                alignment_scanner.is_none(),
                "a command retry and alignment retry cannot share one scanner capability"
            );
            Some(PendingDirectDestination::Preflight(retry))
        }
        // Alignment itself suspended without a command-owned continuation.
        None => Some(PendingDirectDestination::Alignment(
            PendingAlignmentDelivery {
                alignment,
                cursor,
                scanner: alignment_scanner,
                expansion: None,
            },
        )),
    }
}

struct PreflightDelivery<G> {
    delivery: OperationDelivery<G>,
    capabilities: crate::transaction_protocol::CommandCapabilities,
    scanner: Option<tex_command::ScannerFrameKey<G>>,
    expansion: Option<tex_command::ExpansionWorkKey<G>>,
}

fn preflight_delivery_from_retry<G>(
    command: PreflightCommand<G>,
    frame: &mut OperationFrame<G>,
) -> PreflightDelivery<G> {
    let capabilities = match command.phase {
        PreflightCommandPhase::ImmediatePdfRetry(primitive) => {
            crate::transaction_protocol::canonical_static_command_capabilities(
                Meaning::UnexpandablePrimitive(primitive),
            )
        }
        PreflightCommandPhase::Expanding { .. } => {
            crate::transaction_protocol::canonical_static_command_capabilities(Meaning::Relax)
        }
        _ => {
            crate::transaction_protocol::canonical_command_capabilities(command.current().meaning())
        }
    };
    assert!(frame.command.replace(command).is_none());
    PreflightDelivery::<G> {
        capabilities,
        delivery: OperationDelivery::<G>::Command,
        scanner: None,
        expansion: None,
    }
}

#[derive(Clone, Copy)]
struct OperationOutputStart {
    outer_paragraph_was_active: bool,
    root_main_file_origin: bool,
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

/// Compact result coordinate for the unified dispatch seam.
///
/// Every payload lives in the caller-owned [`OperationFrame`]. Keeping this
/// status payload-free prevents construction and application from transferring
/// the complete cold operation merely to cross a borrow boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OperationReadiness {
    Applied,
    Prepared,
    Failed,
}

/// Singular reusable output slot for one command attempt.
///
/// Preparation writes exactly one of `applied`, `prepared`, or `error` and
/// returns only [`OperationReadiness`]. Ordinary completion consumes the
/// occupied fields individually and leaves the frame empty for the next loop
/// iteration. A resource suspension moves this exact frame into the attempt's
/// singular continuation; it is never appended to generation-lived storage.
struct OperationFrame<G> {
    applied: Option<Result<ReplayStep, ExecError>>,
    prepared: Option<PreparedColdCommand<G>>,
    alignment_preamble: Option<PreparedAlignmentPreamble<G>>,
    output_start: Option<OperationOutputStart>,
    error: Option<ExecError>,
    unavailable: Option<ColdOperation<G>>,
    cursor: Option<tex_command::CommandDeliveryCursor>,
    command: Option<PreflightCommand<G>>,
    alignment_scanner: Option<tex_command::ScannerFrameKey<G>>,
}

impl<G> Default for OperationFrame<G> {
    fn default() -> Self {
        Self {
            applied: None,
            prepared: None,
            alignment_preamble: None,
            output_start: None,
            error: None,
            unavailable: None,
            cursor: None,
            command: None,
            alignment_scanner: None,
        }
    }
}

impl<G> OperationFrame<G> {
    fn assert_empty(&self) {
        assert!(
            self.applied.is_none()
                && self.prepared.is_none()
                && self.alignment_preamble.is_none()
                && self.output_start.is_none()
                && self.error.is_none()
                && self.unavailable.is_none()
                && self.cursor.is_none()
                && self.command.is_none()
                && self.alignment_scanner.is_none(),
            "one command attempt owns one empty operation frame"
        );
    }

    fn write_retry_failure(
        &mut self,
        error: ExecError,
        cursor: tex_command::CommandDeliveryCursor,
        alignment_scanner: Option<tex_command::ScannerFrameKey<G>>,
    ) {
        assert!(
            self.command.is_none() || alignment_scanner.is_none(),
            "one failed operation retains exactly one scanner destination"
        );
        self.error = Some(error);
        self.cursor = Some(cursor);
        self.alignment_scanner = alignment_scanner;
    }

    fn assert_command_only(&self) {
        assert!(
            self.applied.is_none()
                && self.prepared.is_none()
                && self.alignment_preamble.is_none()
                && self.output_start.is_none()
                && self.error.is_none()
                && self.unavailable.is_none()
                && self.cursor.is_none()
                && self.command.is_some()
                && self.alignment_scanner.is_none(),
            "command delivery owns only its operation-local command frame"
        );
    }

    fn take_error(&mut self) -> ExecError {
        self.error
            .take()
            .expect("failed preparation writes its diagnostic into the frame")
    }
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
    attempt: tex_command::PendingCommandAttempt<G, PreparedResourceResume<G>>,
}

struct PreparedResourceResume<G> {
    frame: OperationFrame<G>,
    capabilities: crate::transaction_protocol::CommandCapabilities,
}

const PREPARED_RESOURCE_RESUME: tex_command::AttemptResumePoint = tex_command::AttemptResumePoint {
    command: 1,
    scanner: 0,
    expansion: 0,
    subordinate: 0,
};

#[derive(Debug)]
struct PendingAlignmentDelivery<G> {
    alignment: Option<AlignmentIdentity>,
    cursor: tex_command::CommandDeliveryCursor,
    scanner: Option<tex_command::ScannerFrameKey<G>>,
    expansion: Option<tex_command::ExpansionWorkKey<G>>,
}

// Both variants are stored in the singular operation owner. Boxing preflight
// state would allocate at the direct-operation continuation boundary.
#[allow(clippy::large_enum_variant)]
enum PendingDirectDestination<G> {
    Alignment(PendingAlignmentDelivery<G>),
    Preflight(PreflightCommand<G>),
}

enum PendingDirectOperation<G> {
    /// A retry whose prior attempt was rolled back and therefore owns no
    /// attempt-local coordinate. Its next operation starts fresh.
    Fresh(PreflightCommand<G>),
    /// A live attempt moved together with the exact caller phase that owns its
    /// scanner child and delivery cursor.
    Retained {
        operation: tex_command::CommandAttemptOperation,
        destination: PendingDirectDestination<G>,
    },
}

impl<G> std::fmt::Debug for PendingDirectOperation<G> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Fresh(_) => "PendingDirectOperation::<G>::Fresh",
            Self::Retained { destination, .. } => match destination {
                PendingDirectDestination::Alignment(_) => {
                    "PendingDirectOperation::<G>::RetainedAlignment"
                }
                PendingDirectDestination::Preflight(_) => {
                    "PendingDirectOperation::<G>::RetainedPreflight"
                }
            },
        })
    }
}

struct PendingDiagnosticOperation<G> {
    operation: tex_command::CommandAttemptOperation,
    destination: PendingDiagnosticDestination<G>,
}

#[allow(
    clippy::large_enum_variant,
    reason = "the singular suspended attempt retains its exact operation frame without boxing or an ordinary-path indirection"
)]
enum PendingDiagnosticDestination<G> {
    Prepared { frame: OperationFrame<G> },
    Preflight(PreflightCommand<G>),
}

impl<G> std::fmt::Debug for PendingDiagnosticOperation<G> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match &self.destination {
            PendingDiagnosticDestination::Prepared { .. } => {
                "PendingDiagnosticOperation::<G>::Prepared"
            }
            PendingDiagnosticDestination::Preflight(_) => {
                "PendingDiagnosticOperation::<G>::Preflight"
            }
        })
    }
}

impl<G> std::fmt::Debug for PendingResourceOperation<G> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingResourceOperation<G>")
            .field("state", &"owned command attempt")
            .finish_non_exhaustive()
    }
}

struct PreflightDeliveryError<G> {
    error: ExecError,
    retry: Option<PreflightCommand<G>>,
}

impl<G> From<ExecError> for PreflightDeliveryError<G> {
    fn from(error: ExecError) -> Self {
        Self { error, retry: None }
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
#[derive(Debug)]
struct DirectOperationMark<G> {
    state: tex_state::StateOperation<G>,
    mode: crate::mode::ModeJournalCursor,
    attempt: tex_command::CommandAttemptOperation,
    page: tex_state::fork_arena::OperationMark<tex_state::fork_arena::PageMaterialLane>,
}

#[derive(Clone, Copy)]
enum OperationTransaction {
    Advance,
    Alignment,
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

/// One checkpoint intent frozen at the operation that formed its boundary.
///
/// Input retirement may expose an enclosing source before command state is
/// quiescent enough to publish a checkpoint. Retaining the active external
/// file decision here prevents that later stack transition from changing the
/// boundary's origin eligibility.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingNamedBoundary {
    boundary: crate::EngineBoundary,
    root_main_file_origin: bool,
}

/// Explicit live-observer boundary for detached shipout geometry.
struct MainControlShipoutGeometrySink<'a, G> {
    command: &'a PersistentInterpreter<G>,
    observations: &'a mut ObservationSlot,
}

/// Explicit operation-local boundary for TeX82 §§649--676 packing geometry.
///
/// The command cursor supplies detached source coordinates while the
/// observation slot remains the sole rollback/publication owner. The sink
/// cannot outlive this command operation and owns no engine-state handle.
struct MainControlPackGeometrySink<'a> {
    line: u32,
    source: Option<tex_command::SourceId>,
    observations: &'a mut ObservationSlot,
}

fn pack_geometry_sink<'a, G>(
    command: &PersistentInterpreter<G>,
    observations: &'a mut ObservationSlot,
) -> MainControlPackGeometrySink<'a> {
    MainControlPackGeometrySink {
        line: command.current_file_line_number(),
        source: command.current_file_source_id(),
        observations,
    }
}

impl crate::geometry::PackGeometrySink for MainControlPackGeometrySink<'_> {
    fn committed_hpack(&mut self, width: Scaled, height: Scaled, depth: Scaled) {
        let Some(observations) = self.observations.as_mut() else {
            return;
        };
        observations.committed(CommandObservation::Geometry(GeometryRecord::Hpack {
            width_sp: i64::from(width.raw()),
            height_sp: i64::from(height.raw()),
            depth_sp: i64::from(depth.raw()),
            line: self.line,
            source: self.source,
        }));
    }

    fn committed_vpack(&mut self, width: Scaled, height: Scaled, depth: Scaled) {
        let Some(observations) = self.observations.as_mut() else {
            return;
        };
        observations.committed(CommandObservation::Geometry(GeometryRecord::Vpack {
            width_sp: i64::from(width.raw()),
            height_sp: i64::from(height.raw()),
            depth_sp: i64::from(depth.raw()),
            line: self.line,
            source: self.source,
        }));
    }
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
        let consumed = other.receipt.reset_for_next_operation();
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
        let consumed = self.receipt.reset_for_next_operation();
        debug_assert!(consumed.records <= MAX_EXECUTION_RECEIPT_RECORDS);
        self.attempted = 0;
        self.overflowed = false;
        self.receipt_attempted = 1;
        self.receipt_overflowed = false;
    }

    fn clear(&mut self) {
        self.records.clear();
        let consumed = self.receipt.reset_for_next_operation();
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

struct PendingShowCompletion {
    long: bool,
    context: String,
}

impl<G> CommandMachine<'_, G> {
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
            CommandHostContext::new(self.capabilities),
            self.fuel,
            observer,
            self.diagnostic_effects,
        );
        processor.set_output_routine_active(self.output_routine_active);
        processor
    }

    fn processor_with_diagnostic_effects<'episode, 'admission>(
        &'episode mut self,
        context: &'episode mut tex_state::CommandContext<'admission, G>,
        diagnostic_effects: &'episode mut DiagnosticEffects,
    ) -> InterpreterProcessor<'episode, 'admission, G> {
        let observer = self
            .observations
            .as_mut()
            .map(|buffer| buffer as &mut dyn CommandObserver);
        let mut processor = self.state.processor(
            context,
            CommandHostContext::new(self.capabilities),
            self.fuel,
            observer,
            diagnostic_effects,
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
    observations: &'episode mut ObservationSlot,
    diagnostic_effects: &'episode mut DiagnosticEffects,
    stores: &'episode mut CommandContext<'admission, G>,
) -> InterpreterProcessor<'episode, 'admission, G> {
    let observer = observations
        .as_mut()
        .map(|buffer| buffer as &mut dyn CommandObserver);
    command.processor(
        stores,
        CommandHostContext::new(capabilities),
        fuel,
        observer,
        diagnostic_effects,
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
            end_job_ejection_pending: false,
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
        mut self,
    ) -> (CommandState<G>, PreparedCheckpointControl) {
        let modes = std::mem::take(&mut self.modes);
        let command = self.command.into_state();
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
        let mut control = self;
        control.accept_checkpoint_candidate_in_place();
    }

    /// Returns command and mode roots through their rejection paths before
    /// aggregate state rejection. Consuming `self` prevents later use of a
    /// partially settled command machine.
    pub fn reject_checkpoint_candidate(self) {
        let (mut command, control) = self.into_checkpoint_candidate_parts();
        control.reject();
        command.reject_checkpoint_candidate();
    }

    /// Settles a quiescent command/mode candidate while retaining the live
    /// control owner and any host suspension continuation it carries.
    pub(crate) fn accept_checkpoint_candidate_in_place(&mut self) {
        self.modes.accept_checkpoint_candidate();
        self.command.state_mut().accept_checkpoint_candidate();
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
            debug_assert_eq!(resume, PREPARED_RESOURCE_RESUME);
            Some(operation)
        } else if let Some(pending) = self.pending_direct_operation.take() {
            match pending {
                PendingDirectOperation::Fresh(_) => None,
                PendingDirectOperation::Retained {
                    operation,
                    destination: _,
                } => Some(operation),
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
        self.refresh_host_capabilities(&command_context);
        let processor = command_processor(
            &mut self.command,
            self.fuel.fuel_mut(),
            &mut self.capabilities,
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
        &self,
        context: &CommandContext<'_, G>,
        scanned: &ColdOperation<G, T, D>,
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
                    &self.skip_pointer_sources,
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
        context: &CommandContext<'_, G>,
        scanned: &ColdOperation<G, T, D>,
    ) -> bool {
        context.int_param(IntParam::ETEX_EXTENDED_MODE) > 0
            && self.local_glue_pointer_reassigned(context, scanned)
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
            || self.end_job_ejection_pending
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
                crate::checkpoint::CheckpointEligibility::outer_paragraph_end()
            }
            crate::EngineBoundary::ShipoutComplete => {
                panic!("shipout completion does not publish checkpoint eligibility")
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
        let ColdOperation::<G>::FontDefinition {
            request, resource, ..
        } = scanned
        else {
            return Ok(());
        };
        stores.poison_dependency_region(TrackedRegionBarrier::UnsupportedHostCapability);
        let path = crate::canonical_font_resource_path(&request.name);
        let Some(resolved_resource) = self.capabilities.font(&path) else {
            return Err(ExecError::MissingFont {
                request: request.clone(),
            });
        };
        **resource = Some(resolved_resource);
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
            attr: request.attr,
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
        let id = self.command.register_source(source)?;
        // `id` was just allocated by this command state, so this can fail
        // only if the state implementation has violated its own invariant.
        self.command
            .open_registered_source(id)
            .expect("freshly registered source must be openable");
        self.root_main_source = Some(id);
        Ok(id)
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

    /// Refreshes executor-owned mode facts for the next processor borrow.
    ///
    /// This is intentionally call-local capability state rather than part of
    /// a command snapshot or durable session summary.
    fn refresh_host_capabilities(&mut self, stores: &CommandContext<'_, G>) {
        self.capabilities
            .set_conditional_state(self.modes.conditional_state());
        self.capabilities.set_space_factor(
            matches!(
                self.modes.current_mode(),
                Mode::Horizontal | Mode::RestrictedHorizontal
            )
            .then(|| self.modes.current_list().space_factor()),
        );
        let ignored_depth = crate::mode::ignored_depth_with_handle(stores, self.pdf_ignore_depth);
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
    fn last_node_value(
        &self,
        context: &CommandContext<'_, G>,
    ) -> Option<tex_command::LastNodeItem> {
        if is_outer_vertical(&self.modes) {
            return match crate::effective_tail::EffectiveTail::find(
                context.page_contributions().iter(),
            ) {
                Some(tail) => Self::classify_last_node(context, tail.node()),
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
        crate::effective_tail::EffectiveTail::find(self.modes.current_list().nodes(context).iter())
            .and_then(|tail| Self::classify_last_node(context, tail.node()))
    }

    /// e-TeX 2.6 `etex.ch` [26.424]'s `find_effective_tail` result for
    /// `\lastnodetype`.
    fn last_node_type_value(&self, context: &CommandContext<'_, G>) -> i32 {
        if is_outer_vertical(&self.modes) {
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
        crate::effective_tail::EffectiveTail::find(self.modes.current_list().nodes(context).iter())
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
                .last()
                .and_then(|node| Self::classify_last_node(stores, node)),
            _ => None,
        }
    }

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
    fn finish_shipout_publication(
        &mut self,
        artifact_count: usize,
        _effect_count: usize,
        stores: &mut Universe<G>,
        root_main_file_origin: bool,
    ) {
        let committed = stores
            .world()
            .artifact_commits()
            .len()
            .saturating_sub(artifact_count);
        let intent = PendingNamedBoundary {
            boundary: crate::EngineBoundary::ShipoutComplete,
            root_main_file_origin,
        };
        for _ in 0..committed {
            self.pending_named_boundaries.push_back(intent);
        }
    }

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
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> ControlFlow<Result<ReplayStep, ExecError>, PreparedColdCommand<G>> {
        let applied = match scanned {
            ColdOperation::ReplayCompleted(episode) => {
                self.completed_replay_episode = Some(episode);
                Ok(ReplayStep::Continue)
            }
            ColdOperation::Math(request) => {
                self.apply_math_request(request, stores, diagnostic_effects)
            }
            ColdOperation::DisplayAlignmentRecovery => {
                self.recover_display_alignment_closer(stores, diagnostic_effects)
            }
            ColdOperation::MathDelimiter(boundary) => {
                self.apply_math_delimiter(boundary, stores, diagnostic_effects)
            }
            // TeX82 §1137's `hmode+math_shift: init_math` and §1193's
            // `mmode+math_shift: if cur_group=math_shift_group then
            // after_math else off_save`. §1090 backs a `vmode+math_shift` up
            // and runs `new_graf(true)` first, so vertical mode never reaches
            // this step.
            ColdOperation::MathShift { pairing } => {
                self.apply_math_shift(pairing, stores, diagnostic_effects)
            }
            ColdOperation::DiscretionaryOpening(opening) => {
                self.begin_discretionary(opening, stores, diagnostic_effects)
            }
            ColdOperation::DiscretionaryPartEnd => {
                self.finish_discretionary_part(stores, diagnostic_effects)
            }
            ColdOperation::DiscretionaryHyphen { origin } => {
                self.apply_discretionary_hyphen(origin, stores, diagnostic_effects)
            }
            // TeX82 §1123's `make_accent` runs §1270's `do_assignments`
            // between the accent code and §1124's base character, so it
            // executes whole commands of its own before it can finish.
            ColdOperation::Accent(accent) => self.apply_accent(accent, stores, diagnostic_effects),
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
            self.fire_pending_page_output(stores, diagnostic_effects)?;
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
        output_start: OperationOutputStart,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
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
        self.fire_pending_page_output(stores, diagnostic_effects)?;
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
                    diagnostic_effects,
                )
                .into_iter()
                .map(CommandObservation::Input)
                .collect();
            if opens_output_batch {
                // Same order as the ordinary tail: the named token-list push
                // command state held across the transition, then the shipouts
                // it committed, then the episode's own records.
                records.extend(
                    committed_shipout_observations(output_start.artifact_count, stores)
                        .into_iter()
                        .map(CommandObservation::Effect),
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
                self.page_output_observations.append_to(&mut records);
            }
            self.observe_committed(records);
            self.page_output_observations.clear();
        }
        self.finish_shipout_publication(
            output_start.artifact_count,
            output_start.effect_count,
            stores,
            output_start.root_main_file_origin,
        );
        self.finish_paragraph_boundary(
            output_start.outer_paragraph_was_active,
            output_start.root_main_file_origin,
            stores,
        );
        Ok(applied)
    }

    /// Publishes the ordinary cold paragraph boundary after `end_graf`.
    fn finish_paragraph_boundary(
        &mut self,
        outer_paragraph_was_active: bool,
        root_main_file_origin: bool,
        stores: &mut Universe<G>,
    ) {
        if outer_paragraph_was_active
            && self.modes.current_mode() == Mode::Vertical
            && self.modes.depth() == 1
            && stores
                .command_context()
                .expect("paragraph-boundary admission")
                .execution_group_depth()
                == 0
        {
            self.pending_named_boundaries
                .push_back(PendingNamedBoundary {
                    boundary: crate::EngineBoundary::OuterParagraphEnd,
                    root_main_file_origin,
                });
        }
    }

    fn active_external_file_is_root_main(&self) -> bool {
        self.root_main_source
            .is_some_and(|root| self.command.current_file_source_id() == Some(root))
    }

    /// Publishes at most one queued named boundary after every command-owned
    /// continuation has retired. This runs before another delivery, so a
    /// captured row cannot include effects from the following command.
    fn publish_pending_named_boundary(
        &mut self,
        stores: &mut Universe<G>,
    ) -> Result<Option<crate::EngineBoundary>, ExecError> {
        loop {
            let Some(pending) = self.pending_named_boundaries.front().copied() else {
                return Ok(None);
            };
            if self.has_external_attempt_owner() {
                return Ok(None);
            }
            if pending.boundary == crate::EngineBoundary::ShipoutComplete
                && (self.boxes.output_routine_active
                    || self.modes.depth() != 1
                    || stores
                        .command_context()
                        .expect("shipout-boundary admission")
                        .execution_group_depth()
                        != 0)
            {
                return Ok(None);
            }
            let mut diagnostic_effects = DiagnosticEffects::new();
            let attempt = self.command.begin_attempt_operation();
            let retirement = {
                let mut context = stores.command_context().expect("named-boundary admission");
                let mut processor = command_processor(
                    &mut self.command,
                    self.fuel.fuel_mut(),
                    &mut self.capabilities,
                    &mut self.operation_observations,
                    &mut diagnostic_effects,
                    &mut context,
                );
                processor.retire_exhausted_token_levels_for_named_boundary()
            };
            if let Err(error) = retirement {
                self.command
                    .rollback_attempt_operation(attempt)
                    .expect("named-boundary retirement owns its attempt scope");
                return Err(command_error(error));
            }
            self.command
                .commit_attempt_operation(attempt)
                .map_err(|_| ExecError::MissingToken {
                    context: "named-boundary attempt scope",
                })?;
            stores
                .world_mut()
                .publish_diagnostic_effects(diagnostic_effects);
            if !self.command.named_boundary_is_quiescent() {
                return Ok(None);
            }
            let published = self
                .pending_named_boundaries
                .pop_front()
                .expect("inspected named-boundary intent remains queued");
            debug_assert_eq!(published, pending);
            if !published.root_main_file_origin {
                continue;
            }
            if published.boundary == crate::EngineBoundary::ShipoutComplete {
                stores
                    .release_page_suffix_if_rootless(self.modes.retains_page_node_handles())
                    .map_err(|_| ExecError::MissingToken {
                        context: "rootless shipout page release",
                    })?;
            }
            if published.boundary == crate::EngineBoundary::OuterParagraphEnd {
                self.completed_checkpoint_eligibilities
                    .push(crate::checkpoint::CheckpointEligibility::outer_paragraph_end());
            }
            self.completed_boundaries.push(published.boundary);
            return Ok(Some(published.boundary));
        }
    }

    /// Publishes every named boundary that became quiescent during terminal
    /// cleanup. Ordinary execution publishes one intent before the following
    /// delivery; terminal cleanup has no following delivery, so the canonical
    /// runner must drain the safe suffix before closing its output ledger.
    pub(crate) fn publish_terminal_named_boundaries(
        &mut self,
        stores: &mut Universe<G>,
    ) -> Result<(), ExecError> {
        while !self.pending_named_boundaries.is_empty()
            && self.publish_pending_named_boundary(stores)?.is_some()
        {}
        Ok(())
    }

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
            let context = stores.command_context().expect("live generation");
            let mut stores = LinearCommandContext::new(context);
            let first_forbidden = level.list().nodes(&stores).iter().position(|node| {
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
            let part_len = level.list().nodes(&stores).len();
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
                let mut processor = command_processor(
                    &mut self.command,
                    self.fuel.fuel_mut(),
                    &mut self.capabilities,
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

    /// Records TeX's checked save-stack high-water projection after one
    /// direct main-control operation.
    fn record_save_stack_usage(&mut self, stores: &CommandContext<'_, G>) {
        // TeX82 §§645/1083 keeps ordinary box specs immediately below their
        // §273 boundaries. Vcenters and insertions deliberately have smaller
        // projections (§§1167/1099), so derive the words from each live kind.
        let box_spec_words = self
            .boxes
            .active_boxes
            .iter()
            .map(|active| active.kind.save_stack_spec_words())
            .fold(0_usize, usize::saturating_add);
        let (aftergroup_words, latest_aftergroup_position) =
            self.command.aftergroup_save_stack_projection();
        let checked = stores
            .checked_save_stack_words(
                aftergroup_words,
                latest_aftergroup_position,
                self.command_profile().capabilities().supports_etex(),
            )
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
        command: Option<&PreflightCommand<G>>,
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
        ) && command
            .and_then(PreflightCommand::current_option)
            .is_some_and(|command| {
                matches!(
                    command.meaning(),
                    ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                        UnexpandablePrimitive::PdfStartLink
                    ))
                )
            })
        {
            return true;
        }
        if command
            .is_some_and(|command| matches!(command.phase, PreflightCommandPhase::Expanding { .. }))
        {
            return true;
        }
        // A right brace normally owns only the save stack and group state,
        // but the brace that packages an active box can also run the page or
        // explicit-shipout pipeline. Route that dynamic continuation through
        // typed preparation so its PDF, effect, and output capabilities are
        // known before direct semantic apply. Braces inside the box remain
        // ordinary because their innermost save-stack group does not name the
        // box body.
        if let Some(command) = command.and_then(PreflightCommand::current_option)
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
        command
            .and_then(PreflightCommand::current_option)
            .is_some_and(|command| {
                matches!(
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
            })
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

    fn begin_direct_operation(
        &mut self,
        stores: &mut Universe<G>,
        attempt: Option<tex_command::CommandAttemptOperation>,
    ) -> DirectOperationMark<G> {
        DirectOperationMark {
            state: stores
                .begin_state_operation()
                .expect("live generation has a state operation journal"),
            mode: self.modes.begin_journal(),
            attempt: attempt.unwrap_or_else(|| self.command.begin_attempt_operation()),
            page: stores.page_node_cursor(),
        }
    }

    fn commit_direct_operation(&mut self, stores: &mut Universe<G>, mark: DirectOperationMark<G>) {
        let DirectOperationMark {
            state,
            mode,
            attempt,
            ..
        } = mark;
        stores
            .commit_state_operation(state)
            .expect("direct operation owns the active state operation");
        self.modes
            .commit_journal(mode)
            .expect("direct operation owns the top mode journal frame");
        self.command
            .commit_attempt_operation(attempt)
            .expect("committed operation owns a valid command-attempt scope");
        self.finish_pending_page_region_succession(stores);
    }

    fn retain_direct_operation_for_retry(
        &mut self,
        stores: &mut Universe<G>,
        mark: DirectOperationMark<G>,
    ) -> tex_command::CommandAttemptOperation {
        let DirectOperationMark {
            state,
            mode,
            attempt,
            ..
        } = mark;
        stores
            .commit_state_operation(state)
            .expect("retained operation owns the active state operation");
        self.modes
            .commit_journal(mode)
            .expect("direct operation owns the top mode journal frame");
        attempt
    }

    fn retain_direct_delivery_for_retry(
        &mut self,
        stores: &mut Universe<G>,
        mark: DirectOperationMark<G>,
        destination: PendingDirectDestination<G>,
    ) {
        let operation = self.retain_direct_operation_for_retry(stores, mark);
        assert!(
            self.pending_direct_operation
                .replace(PendingDirectOperation::Retained {
                    operation,
                    destination,
                })
                .is_none(),
            "one direct retry owns the active operation"
        );
    }

    fn suspend_prepared_resource_operation(
        &mut self,
        stores: &Universe<G>,
        operation: tex_command::CommandAttemptOperation,
        frame: OperationFrame<G>,
        capabilities: crate::transaction_protocol::CommandCapabilities,
    ) {
        let pending = PreparedResourceResume::<G> {
            frame,
            capabilities,
        };
        let attempt = self
            .command
            .suspend_attempt(stores, operation, PREPARED_RESOURCE_RESUME, pending)
            .expect("live main control can retain its admitted generation");
        self.pending_resource_operation = Some(PendingResourceOperation::<G> { attempt });
    }

    /// Finishes a failed prepared-resource preflight while the operation
    /// capability still has exactly one structural location.
    ///
    /// Diagnostic classification runs before suspension moves the attempt out
    /// of command state. A genuine resource suspension then moves it into the
    /// pending continuation and retains the other journals; every terminal
    /// result commits while the owner is still installed. Callers therefore
    /// cannot commit an emptied command attempt after moving its owner.
    fn finish_unavailable_prepared_resource_operation(
        &mut self,
        stores: &mut Universe<G>,
        mark: DirectOperationMark<G>,
        mut frame: OperationFrame<G>,
        capabilities: crate::transaction_protocol::CommandCapabilities,
    ) -> Result<StepResult, ExecError> {
        assert!(
            frame.unavailable.is_some(),
            "unavailable resource remains in its attempt-owned frame"
        );
        let error = frame.take_error();
        let result = self.finish_resource_preflight_failure(stores, error);
        if matches!(result, Ok(StepResult::Suspended(_))) {
            let operation = self.retain_direct_operation_for_retry(stores, mark);
            self.suspend_prepared_resource_operation(stores, operation, frame, capabilities);
        } else {
            self.commit_direct_operation(stores, mark);
        }
        result
    }

    fn discard_direct_operation(&mut self, stores: &mut Universe<G>, mark: DirectOperationMark<G>) {
        stores
            .restore_state(mark.state)
            .expect("direct operation state cursor belongs to the live generation");
        self.modes
            .rollback_journal(mark.mode)
            .expect("direct operation owns the top mode journal frame");
        self.command
            .rollback_attempt_operation(mark.attempt)
            .expect("rollback owns valid command-attempt coordinates");
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
        diagnostic_effects: DiagnosticEffects,
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
        stores
            .world_mut()
            .publish_diagnostic_effects(diagnostic_effects);
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
        let mut operation_frame = OperationFrame::default();
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
                    resume, PREPARED_RESOURCE_RESUME,
                    "resource continuation resumes at its prepared-operation cursor"
                );
                (operation, pending)
            });
            let pending_direct = if resumed_resource.is_none() {
                self.pending_direct_operation.take()
            } else {
                debug_assert!(self.pending_direct_operation.is_none());
                None
            };
            let (retained_operation, pending_destination) = match pending_direct {
                Some(PendingDirectOperation::Fresh(command)) => {
                    (None, Some(PendingDirectDestination::Preflight(command)))
                }
                Some(PendingDirectOperation::Retained {
                    operation,
                    destination,
                }) => (Some(operation), Some(destination)),
                None => (None, None),
            };
            let (operation, resumed_resource) = match resumed_resource {
                Some((operation, mut pending)) => {
                    let _ = pending.frame.error.take();
                    operation_frame = pending.frame;
                    (Some(operation), Some(pending.capabilities))
                }
                None => (retained_operation, None),
            };
            let operation_mark = self.begin_direct_operation(stores, operation);
            let mut diagnostic_effects = DiagnosticEffects::new();
            // A cascading §1026 page break can become ready while the prior
            // operation still owns a rollback-restorable mode root. Resume
            // that builder continuation in its own journaled operation before
            // delivering another TeX command.
            let resumes_page_output = !self.page_region_succession_pending
                && !self.boxes.output_routine_active
                && stores
                    .command_context()
                    .expect("live generation")
                    .page_fire_up()
                    .is_some();
            if resumes_page_output {
                let applied = self
                    .fire_pending_page_output(stores, &mut diagnostic_effects)
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
            let preflight = if let Some(capabilities) = resumed_resource {
                PreflightDelivery::<G> {
                    delivery: OperationDelivery::<G>::Prepared,
                    capabilities,
                    scanner: None,
                    expansion: None,
                }
            } else if let Some(delivery) = initial_delivery.take() {
                PreflightDelivery::<G> {
                    delivery,
                    capabilities:
                        crate::transaction_protocol::canonical_static_command_capabilities(
                            Meaning::Relax,
                        ),
                    scanner: None,
                    expansion: None,
                }
            } else if let Some(destination) = pending_destination {
                match destination {
                    PendingDirectDestination::Alignment(pending) => PreflightDelivery::<G> {
                        delivery: OperationDelivery::<G>::AlignmentRetry {
                            alignment: pending.alignment,
                            cursor: pending.cursor,
                        },
                        capabilities:
                            crate::transaction_protocol::canonical_static_command_capabilities(
                                Meaning::Relax,
                            ),
                        scanner: pending.scanner,
                        expansion: pending.expansion,
                    },
                    PendingDirectDestination::Preflight(command) => {
                        preflight_delivery_from_retry(command, &mut operation_frame)
                    }
                }
            } else {
                let preflight = match self.preflight_replay_delivery(
                    stores,
                    &mut diagnostic_effects,
                    &mut operation_frame,
                ) {
                    Ok(preflight) => preflight,
                    Err(failure) => {
                        let PreflightDeliveryError { error, retry } = failure;
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
                            let destination = retry
                                .map(PendingDirectDestination::Preflight)
                                .expect("resource delivery retains its exact retry command");
                            let operation =
                                self.retain_direct_operation_for_retry(stores, operation_mark);
                            self.pending_direct_operation =
                                Some(PendingDirectOperation::Retained {
                                    operation,
                                    destination,
                                });
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
                OperationDelivery::<G>::Replay
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
                    && matches!(&preflight.delivery, OperationDelivery::<G>::Command)
                    && operation_frame.command.as_ref().is_some_and(|command| {
                        command.current_option().is_some_and(|command| {
                            matches!(
                                command.meaning(),
                                ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                                    UnexpandablePrimitive::PdfXImage
                                ))
                            )
                        })
                    }))
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
                let prepared = self.prepare_operation(
                    stores,
                    preflight.delivery,
                    preflight.scanner,
                    preflight.expansion,
                    &mut diagnostic_effects,
                    &mut operation_frame,
                );
                if prepared == OperationReadiness::Failed {
                    if let Some(mark) = tracked_mark {
                        let _ = stores.abandon_dependency_region(mark);
                    }
                    if operation_frame.unavailable.is_some() {
                        let result = self.finish_unavailable_prepared_resource_operation(
                            stores,
                            operation_mark,
                            operation_frame,
                            preflight.capabilities,
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
                    let result = self
                        .finish_resource_preflight_failure(stores, operation_frame.take_error());
                    if matches!(result, Ok(StepResult::Suspended(_))) {
                        let destination = own_alignment_retry_child(
                            alignment_delivery,
                            operation_frame.cursor.take(),
                            operation_frame.command.take(),
                            operation_frame.alignment_scanner.take(),
                        )
                        .expect("resource suspension retains one direct caller destination");
                        self.retain_direct_delivery_for_retry(stores, operation_mark, destination);
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
                let applied = self.apply_ready_operation(
                    stores,
                    prepared,
                    &mut diagnostic_effects,
                    &mut operation_frame,
                );
                self.record_save_stack_usage(
                    &stores.command_context().expect("save-stack admission"),
                );
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
                preflight.capabilities.preflight(),
                crate::transaction_protocol::CommandPreflight::Transaction(_)
            ) || self.command_requires_transaction(
                stores,
                preflight.capabilities,
                operation_frame.command.as_ref(),
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
                if let crate::transaction_protocol::CommandPreflight::Transaction(transaction) =
                    preflight.capabilities.preflight()
                {
                    let transaction = transaction.transaction();
                    transaction
                        .admit(transaction.projection())
                        .expect("preflight owns the exact narrow projection");
                }
                let prepared = self.prepare_operation(
                    stores,
                    preflight.delivery,
                    preflight.scanner,
                    preflight.expansion,
                    &mut diagnostic_effects,
                    &mut operation_frame,
                );
                if prepared == OperationReadiness::Failed {
                    if let Some(mark) = tracked_mark {
                        let _ = stores.abandon_dependency_region(mark);
                    }
                    if operation_frame.unavailable.is_some() {
                        let result = self.finish_unavailable_prepared_resource_operation(
                            stores,
                            operation_mark,
                            operation_frame,
                            preflight.capabilities,
                        );
                        return result;
                    }
                    let result = self
                        .finish_resource_preflight_failure(stores, operation_frame.take_error());
                    match result {
                        Ok(step @ StepResult::Suspended(_)) => {
                            let destination = own_alignment_retry_child(
                                alignment_delivery,
                                operation_frame.cursor.take(),
                                operation_frame.command.take(),
                                operation_frame.alignment_scanner.take(),
                            )
                            .expect("resource suspension retains one direct caller destination");
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
                            let destination = own_alignment_retry_child(
                                alignment_delivery,
                                operation_frame.cursor.take(),
                                operation_frame.command.take(),
                                operation_frame.alignment_scanner.take(),
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
                if prepared == OperationReadiness::Prepared
                    && let ColdOperation::ImmediateExtension(
                        RootedImmediateExtension::PdfExtensionInDviMode(primitive),
                    ) = operation_frame
                        .prepared
                        .as_ref()
                        .expect("prepared readiness owns its operation payload")
                {
                    operation_frame.command = Some(PreflightCommand::immediate_pdf(*primitive));
                }
                self.episode_telemetry.record_attempt();
                self.advance_telemetry.attempts += 1;
                let applied = self.apply_ready_operation(
                    stores,
                    prepared,
                    &mut diagnostic_effects,
                    &mut operation_frame,
                );
                self.record_save_stack_usage(
                    &stores.command_context().expect("save-stack admission"),
                );
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
                            self.pending_direct_operation = operation_frame
                                .command
                                .take()
                                .map(PendingDirectOperation::Fresh);
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
                matches!(&preflight.delivery, OperationDelivery::<G>::Command)
                    && operation_frame.command.as_ref().is_some_and(|command| {
                        command.current_option().is_some_and(|command| {
                            matches!(
                                command.meaning(),
                                ResolvedMeaning::Static(Meaning::Undefined | Meaning::Unknown(_))
                            )
                        })
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
            let prepared = self.prepare_operation(
                stores,
                preflight.delivery,
                preflight.scanner,
                preflight.expansion,
                &mut diagnostic_effects,
                &mut operation_frame,
            );
            if prepared == OperationReadiness::Failed {
                if let Some(interaction) = saved_interaction {
                    stores.set_interaction_mode(interaction);
                }
                if let Some(mark) = tracked_mark {
                    let _ = stores.abandon_dependency_region(mark);
                }
                let result = if operation_frame.unavailable.is_some() {
                    self.finish_unavailable_prepared_resource_operation(
                        stores,
                        operation_mark,
                        operation_frame,
                        preflight.capabilities,
                    )
                } else {
                    let result = self
                        .finish_resource_preflight_failure(stores, operation_frame.take_error());
                    if matches!(result, Ok(StepResult::Suspended(_))) {
                        let destination = own_alignment_retry_child(
                            alignment_delivery,
                            operation_frame.cursor.take(),
                            operation_frame.command.take(),
                            operation_frame.alignment_scanner.take(),
                        )
                        .expect("resource suspension retains one direct caller destination");
                        self.retain_direct_delivery_for_retry(stores, operation_mark, destination);
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
            let applied = self.apply_ready_operation(
                stores,
                prepared,
                &mut diagnostic_effects,
                &mut operation_frame,
            );
            if let Some(interaction) = saved_interaction {
                stores.set_interaction_mode(interaction);
            }
            operations += 1;
            self.record_save_stack_usage(&stores.command_context().expect("save-stack admission"));
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
        delivery: OperationDelivery<G>,
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
            OperationDelivery::<G>::Replay,
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
            OperationDelivery::<G>::Replay,
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
            OperationDelivery::<G>::Replay,
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
        let mut operation_frame = OperationFrame::default();
        let (retained_attempt, continuation) = match continuation {
            Some(PendingDiagnosticOperation {
                operation,
                destination: PendingDiagnosticDestination::<G>::Prepared { mut frame },
            }) => {
                let _ = frame.error.take();
                operation_frame = frame;
                (
                    Some(operation),
                    Some((OperationDelivery::<G>::Prepared, None)),
                )
            }
            Some(PendingDiagnosticOperation {
                operation,
                destination: PendingDiagnosticDestination::<G>::Preflight(command),
            }) => {
                let preflight = preflight_delivery_from_retry(command, &mut operation_frame);
                (
                    Some(operation),
                    Some((preflight.delivery, preflight.scanner)),
                )
            }
            None => (None, None),
        };
        let operation_mark = self.begin_direct_operation(stores, retained_attempt);
        let mut diagnostic_effects = DiagnosticEffects::new();
        let assignment = match continuation {
            Some(continuation) => Some(continuation),
            None => {
                self.ensure_primitive_handles(stores);
                let (command, cursor, retry_expansion) = {
                    let mut context = stores.command_context().expect("live generation");
                    self.refresh_host_capabilities(&context);
                    let mut processor = command_processor(
                        &mut self.command,
                        self.fuel.fuel_mut(),
                        &mut self.capabilities,
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
                    (command, cursor, retry_expansion)
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
                            self.pending_diagnostic_operation = Some(PendingDiagnosticOperation {
                                operation,
                                destination: PendingDiagnosticDestination::Preflight(
                                    PreflightCommand::expanding(expansion, false, cursor),
                                ),
                            });
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
                assert!(
                    operation_frame
                        .command
                        .replace(PreflightCommand::settled(command, Some(cursor)))
                        .is_none(),
                    "diagnostic assignment owns an empty command frame",
                );
                Some((OperationDelivery::<G>::Command, None))
            }
        };
        let (delivery, scanner) = assignment.expect("diagnostic assignment continuation");
        let mode_mark = self.modes.begin_journal();
        let prepared = self.prepare_operation(
            stores,
            delivery,
            scanner,
            None,
            &mut diagnostic_effects,
            &mut operation_frame,
        );
        if prepared == OperationReadiness::Failed {
            assert!(
                operation_frame.alignment_scanner.is_none(),
                "diagnostic retry cannot own an alignment scanner destination"
            );
            let unavailable = operation_frame.unavailable.is_some();
            let destination = if unavailable {
                None
            } else {
                operation_frame
                    .command
                    .take()
                    .map(PendingDiagnosticDestination::<G>::Preflight)
            };
            let result =
                self.finish_resource_preflight_failure(stores, operation_frame.take_error());
            self.modes
                .rollback_journal(mode_mark)
                .expect("diagnostic assignment owns the mode mark");
            if matches!(result, Ok(StepResult::Suspended(_))) {
                let destination = if unavailable {
                    PendingDiagnosticDestination::<G>::Prepared {
                        frame: operation_frame,
                    }
                } else {
                    destination.expect("diagnostic resource suspension owns an exact retry")
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
        match self.apply_ready_operation(
            stores,
            prepared,
            &mut diagnostic_effects,
            &mut operation_frame,
        ) {
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
        self.record_save_stack_usage(&stores.command_context().expect("save-stack admission"));
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
                let mut processor = command_processor(
                    &mut self.command,
                    self.fuel.fuel_mut(),
                    &mut self.capabilities,
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
    ) -> Result<(), ExecError> {
        self.finish_pending_page_region_succession(stores);
        if self.page_region_succession_pending {
            // The active operation can still restore a mode root. Its commit
            // retries succession before this builder continuation resumes.
            return Ok(());
        }
        while !self.boxes.output_routine_active {
            let mut context = stores.command_context().expect("live generation");
            let selected = {
                let Some(fire_up) = context.page_fire_up() else {
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
                    let publication = shipout_replay_box(page, stores, &mut command)?;
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
                                        &mut context,
                                        diagnostic_effects,
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
        self.refresh_host_capabilities(&context);
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
        let mut context =
            LinearCommandContext::new(stores.command_context().expect("inline-math admission"));
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
        self.modes
            .current_list_mutation()
            .append_list(&mut context, nodes);
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
        let mut context =
            LinearCommandContext::new(stores.command_context().expect("equation-number admission"));
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
        let mut context = LinearCommandContext::new(
            stores
                .command_context()
                .expect("display-alignment admission"),
        );
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
        let mut context =
            LinearCommandContext::new(stores.command_context().expect("display-math admission"));
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
                    let mut context = LinearCommandContext::new(
                        stores.command_context().expect("math-middle admission"),
                    );
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
                    let mut list = self.modes.current_list_mutation();
                    list.append_list(&mut context, content);
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
                let mut context = LinearCommandContext::new(
                    stores.command_context().expect("math-right admission"),
                );
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
        let mut processor = command_processor(
            &mut self.command,
            self.fuel.fuel_mut(),
            &mut self.capabilities,
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
        let mut processor = command_processor(
            &mut self.command,
            self.fuel.fuel_mut(),
            &mut self.capabilities,
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
        let mut processor = command_processor(
            &mut self.command,
            self.fuel.fuel_mut(),
            &mut self.capabilities,
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
            OperationDelivery::<G>::Replay,
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
    #[allow(
        clippy::result_large_err,
        reason = "resource failure moves the complete inline retry owner without a lifecycle allocation"
    )]
    fn preflight_replay_delivery(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
        frame: &mut OperationFrame<G>,
    ) -> Result<Option<PreflightDelivery<G>>, PreflightDeliveryError<G>> {
        frame.assert_empty();
        let mode = self.modes.current_mode();
        if self.active_alignment.is_some()
            || (mode == Mode::DisplayMath && self.modes.current_list().has_display_alignment())
        {
            return Ok(Some(PreflightDelivery::<G> {
                delivery: OperationDelivery::<G>::Replay,
                capabilities: crate::transaction_protocol::canonical_static_command_capabilities(
                    Meaning::Relax,
                ),
                scanner: None,
                expansion: None,
            }));
        }

        self.ensure_primitive_handles(stores);
        let mut context = stores.command_context().expect("live generation");
        if self.enter_main_control(&mut context) {
            let entry_records: Vec<CommandObservation> = self
                .command
                .publish_named_token_list_pushes(&mut context, diagnostic_effects)
                .into_iter()
                .map(CommandObservation::Input)
                .collect();
            self.observe_committed(entry_records);
        }
        self.refresh_host_capabilities(&context);
        let innermost_group = context.innermost_group_kind();
        let mut diagnostics = Vec::new();
        let raw_main_loop_delivery = self.main_loop_active;
        let (
            delivery_status,
            destination,
            settled_in_preflight,
            trace_reported,
            fused_hot,
            fused_retry,
            fused_error,
        ) = {
            let mut processor = command_processor(
                &mut self.command,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
                &mut self.operation_observations,
                diagnostic_effects,
                &mut context,
            );
            processor.set_output_routine_active(self.boxes.output_routine_active);
            let mut destination = None;
            let mut delivery_status = processor
                .get_next_with_replay_completion_into(&mut destination)
                .map_err(command_error)?;
            let mut settled_in_preflight = false;
            if delivery_status == tex_command::DeliveryStatus::Command
                && matches!(
                    destination
                        .as_ref()
                        .expect("command status initializes destination")
                        .meaning(),
                    ResolvedMeaning::Macro { .. }
                        | ResolvedMeaning::Static(Meaning::ExpandablePrimitive(_))
                        | ResolvedMeaning::Static(Meaning::Undefined | Meaning::Unknown(_))
                )
            {
                let command = destination
                    .take()
                    .expect("command status initializes destination");
                prepare_command_trace(&mut processor, mode, self.shown_mode);
                match processor.settle_preflight_command_into(
                    command,
                    self.main_loop_active,
                    &mut destination,
                ) {
                    Ok(status) => {
                        settled_in_preflight = true;
                        delivery_status = status;
                    }
                    Err(error) => {
                        // The expansion driver moves its live command into
                        // command state only after an actual immutable-host
                        // suspension. Fuel and semantic failures have no
                        // retry command and must not clone one speculatively.
                        let retry_expansion = processor.take_pending_expansion_work();
                        let retry = retry_expansion.map(|expansion| {
                            PreflightCommand::<G>::expanding(
                                expansion,
                                self.main_loop_active,
                                processor.delivery_cursor(),
                            )
                        });
                        drop(processor);
                        return Err(PreflightDeliveryError {
                            error: command_error(error),
                            retry,
                        });
                    }
                }
            }
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
            if diagnostics.is_empty() && delivery_status == tex_command::DeliveryStatus::Command {
                let command = destination
                    .take()
                    .expect("command status initializes destination");
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
                    let mut suspended_operation_scan = None;
                    match scan_direct_hot_command(
                        &mut processor,
                        &command,
                        innermost_group,
                        &mut suspended_operation_scan,
                    ) {
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
                            fused_retry = if let Some(phase) = suspended_operation_scan {
                                Some(PreflightCommand::operation_scan(
                                    command,
                                    cursor,
                                    phase,
                                    processor.take_scanner_resume().expect(
                                        "a suspended hot scalar scan retains its exact child",
                                    ),
                                ))
                            } else {
                                let retry_expansion = processor.take_pending_expansion_work();
                                let scanner = processor.take_scanner_resume();
                                Some(if let Some(expansion) = retry_expansion {
                                    PreflightCommand::<G>::expanding(
                                        expansion,
                                        self.main_loop_active,
                                        cursor,
                                    )
                                } else {
                                    let mut owner =
                                        PreflightCommand::<G>::settled(command, Some(cursor));
                                    owner.scanner = scanner;
                                    owner
                                })
                            };
                            fused_error = Some(error);
                        }
                    }
                } else {
                    destination = Some(command);
                }
            }
            (
                delivery_status,
                destination,
                settled_in_preflight,
                trace_reported,
                fused_hot,
                fused_retry,
                fused_error,
            )
        };
        drop(context);
        if let Some(error) = fused_error {
            let retry = execution_error_needs_command_retry(&error)
                .then_some(fused_retry)
                .flatten();
            return Err(PreflightDeliveryError { error, retry });
        }
        self.capture_first_causal_context(stores, &diagnostics);
        report_pending_diagnostics(stores, diagnostic_effects, diagnostics)?;
        if let Some((operation, meaning)) = fused_hot {
            return Ok(Some(PreflightDelivery::<G> {
                delivery: OperationDelivery::<G>::Hot(operation),
                capabilities: crate::transaction_protocol::canonical_command_capabilities(meaning),
                scanner: None,
                expansion: None,
            }));
        }

        let passive =
            || crate::transaction_protocol::canonical_static_command_capabilities(Meaning::Relax);
        let command = match delivery_status {
            tex_command::DeliveryStatus::End => {
                frame.unavailable = Some(ColdOperation::<G>::EndOfInput);
                return Ok(Some(PreflightDelivery::<G> {
                    delivery: OperationDelivery::<G>::Prepared,
                    capabilities: passive(),
                    scanner: None,
                    expansion: None,
                }));
            }
            tex_command::DeliveryStatus::ReplayCompleted(episode) => {
                frame.unavailable = Some(ColdOperation::<G>::ReplayCompleted(episode));
                return Ok(Some(PreflightDelivery::<G> {
                    delivery: OperationDelivery::<G>::Prepared,
                    capabilities: passive(),
                    scanner: None,
                    expansion: None,
                }));
            }
            tex_command::DeliveryStatus::Command => {
                destination.expect("command status initializes destination")
            }
            _ => unreachable!("raw preflight delivery has no alignment event"),
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
            let mut context = stores.command_context().expect("live generation");
            let mut processor = command_processor(
                &mut self.command,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
                &mut self.operation_observations,
                diagnostic_effects,
                &mut context,
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
            frame.unavailable = Some(ColdOperation::<G>::NoBoundary {
                suppress_right: true,
            });
            return Ok(Some(PreflightDelivery::<G> {
                delivery: OperationDelivery::<G>::Prepared,
                capabilities: crate::transaction_protocol::canonical_command_capabilities(
                    command.meaning(),
                ),
                scanner: None,
                expansion: None,
            }));
        }
        let capabilities =
            crate::transaction_protocol::canonical_command_capabilities(command.meaning());
        frame.command = Some(if settled_in_preflight {
            PreflightCommand::settled(command, None)
        } else if raw_main_loop_delivery && continues_main_loop {
            PreflightCommand::raw(command, None)
        } else {
            PreflightCommand::replay(command)
        });
        Ok(Some(PreflightDelivery::<G> {
            delivery: OperationDelivery::<G>::Command,
            capabilities,
            scanner: None,
            expansion: None,
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
        command: Option<tex_command::CurrentCommand<G>>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<ReplayStep, ExecError> {
        let mut frame = OperationFrame::default();
        let delivery = if let Some(command) = command {
            frame.command = Some(PreflightCommand::settled(command, None));
            OperationDelivery::<G>::Command
        } else {
            OperationDelivery::<G>::Replay
        };
        let readiness =
            self.prepare_operation(stores, delivery, None, None, diagnostic_effects, &mut frame);
        if readiness == OperationReadiness::Failed {
            return Err(frame.take_error());
        }
        self.apply_ready_operation(stores, readiness, diagnostic_effects, &mut frame)
    }

    fn apply_ready_operation(
        &mut self,
        stores: &mut Universe<G>,
        readiness: OperationReadiness,
        diagnostic_effects: &mut DiagnosticEffects,
        frame: &mut OperationFrame<G>,
    ) -> Result<ReplayStep, ExecError> {
        let result = match readiness {
            OperationReadiness::Applied => frame
                .applied
                .take()
                .expect("applied preparation writes its result into the frame"),
            OperationReadiness::Prepared => self.apply_prepared_operation(
                stores,
                frame
                    .prepared
                    .take()
                    .expect("cold preparation writes its operation into the frame"),
                frame.alignment_preamble.take(),
                frame
                    .output_start
                    .take()
                    .expect("cold preparation writes its output cursor into the frame"),
                diagnostic_effects,
            ),
            OperationReadiness::Failed => {
                unreachable!("failed preparation is handled before application")
            }
        };
        if result.is_ok() {
            self.apply_error_stop_transition(stores, diagnostic_effects)?;
            let _ = frame.command.take();
        }
        result
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
        let mut processor = command_processor(
            &mut self.command,
            self.fuel.fuel_mut(),
            &mut self.capabilities,
            &mut self.operation_observations,
            diagnostic_effects,
            &mut context,
        );
        processor.set_output_routine_active(self.boxes.output_routine_active);
        processor
            .apply_error_stop_recovery(request)
            .map_err(command_error)
    }

    /// Completes one canonical operation after mutation-free capability
    /// preflight. Common unexpandable families scan and apply here without a
    /// universal DTO; cold and barrier families return a prepared value after
    /// immutable resource resolution.
    fn prepare_operation(
        &mut self,
        stores: &mut Universe<G>,
        delivery: OperationDelivery<G>,
        scanner_resume: Option<tex_command::ScannerFrameKey<G>>,
        expansion_resume: Option<tex_command::ExpansionWorkKey<G>>,
        diagnostic_effects: &mut DiagnosticEffects,
        frame: &mut OperationFrame<G>,
    ) -> OperationReadiness {
        if matches!(&delivery, OperationDelivery::<G>::Prepared) {
            assert!(
                frame.unavailable.is_some()
                    && frame.applied.is_none()
                    && frame.prepared.is_none()
                    && frame.alignment_preamble.is_none()
                    && frame.output_start.is_none()
                    && frame.error.is_none()
                    && frame.cursor.is_none()
                    && frame.alignment_scanner.is_none(),
                "prepared delivery resumes the exact occupied operation frame"
            );
        } else if matches!(&delivery, OperationDelivery::<G>::Command) {
            frame.assert_command_only();
        } else {
            frame.assert_empty();
        }
        let mode = self.modes.current_mode();
        let tracked_region_is_active = stores
            .command_context()
            .is_ok_and(|context| context.tracked_region_is_active());
        if tracked_region_is_active {
            let mode_fingerprint = self.modes.semantic_fingerprint(stores);
            let mut context = stores
                .command_context()
                .expect("tracked region keeps its generation admitted");
            let last_node_type = self.last_node_type_value(&context);
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
        if matches!(&delivery, OperationDelivery::<G>::Replay)
            && self.enter_main_control(&mut context)
        {
            // §1030's prologue precedes `big_switch`, so its push is published
            // ahead of the first command this step delivers rather than with
            // the step's own applied records.
            let entry_records: Vec<CommandObservation> = self
                .command
                .publish_named_token_list_pushes(&mut context, diagnostic_effects)
                .into_iter()
                .map(CommandObservation::Input)
                .collect();
            self.observe_committed(entry_records);
        }
        self.refresh_host_capabilities(&context);
        let outer_paragraph_was_active = mode == Mode::Horizontal && self.modes.depth() == 2;
        let root_main_file_origin = self.active_external_file_is_root_main();
        let alignment_preamble = alignment_preamble(self.active_alignment.as_mut());
        let (innermost_group, job_is_all_over) = (
            context.innermost_group_kind(),
            crate::page_output::job_is_all_over(&context),
        );
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
                diagnostic_effects,
                &mut context,
            );
            let scanner_resume = if matches!(&delivery, OperationDelivery::<G>::Command) {
                frame
                    .command
                    .as_mut()
                    .expect("command delivery owns its operation command")
                    .scanner
                    .take()
            } else {
                scanner_resume
            };
            processor.install_scanner_resume(scanner_resume);
            if let Some(expansion) = expansion_resume {
                processor.install_expansion_resume(expansion);
            }
            processor.set_output_routine_active(self.boxes.output_routine_active);
            let display_alignment_tail = matches!(&delivery, OperationDelivery::<G>::Replay)
                && mode == Mode::DisplayMath
                && self.modes.current_list().has_display_alignment();
            let scanned = (|| -> Result<ScannedOperation<G>, ExecError> {
                Ok(match delivery {
                    OperationDelivery::<G>::Command => scan_preflight_command(
                        &mut processor,
                        frame
                            .command
                            .as_mut()
                            .expect("command delivery owns its operation command"),
                        mode,
                        &self.boxes,
                        innermost_group,
                        job_is_all_over,
                        self.modes.current_list().display_eq_no().is_some(),
                        &mut self.shown_mode,
                        &mut diagnostics,
                    )?,
                    OperationDelivery::<G>::Replay if display_alignment_tail => {
                        match processor
                            .next_do_assignments_command()
                            .map_err(command_error)?
                        {
                            Some(command) => match command.meaning() {
                                meaning
                                    if tex_command::exceeds_max_non_prefixed_command(
                                        static_meaning(meaning.clone()),
                                    ) || matches!(
                                        meaning,
                                        ResolvedMeaning::Static(Meaning::CharToken {
                                            cat: Catcode::MathShift,
                                            ..
                                        })
                                    ) =>
                                {
                                    assert!(
                                        frame
                                            .command
                                            .replace(PreflightCommand::settled(command, None))
                                            .is_none(),
                                        "display-alignment delivery owns an empty command frame",
                                    );
                                    dispatch_main_control_command(
                                        &mut processor,
                                        frame.command.as_mut().expect(
                                            "display-alignment delivery installed its command frame",
                                        ),
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
                    OperationDelivery::<G>::Replay => scan_replay_step(
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
                        &mut frame.command,
                    )?,
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
                        &mut frame.command,
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
                                &mut frame.command,
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
                                &mut frame.command,
                            )?,
                        }
                    }
                    OperationDelivery::<G>::Hot(_) => {
                        unreachable!("pre-scanned hot delivery bypasses processor construction")
                    }
                    OperationDelivery::<G>::Prepared => frame
                        .unavailable
                        .take()
                        .expect("prepared delivery owns its scanned frame payload")
                        .into(),
                })
            })();
            let cursor = processor.delivery_cursor();
            let retry_expansion = processor.take_pending_expansion_work();
            let scanner_resume = processor.take_scanner_resume();
            let retained_command_scan = frame
                .command
                .as_ref()
                .is_some_and(PreflightCommand::is_command_scan);
            let alignment_scanner = if retained_command_scan {
                assert!(
                    scanner_resume.is_none(),
                    "the direct-operation parent already owns its exact scanner child"
                );
                None
            } else if let Some(expansion) = retry_expansion {
                frame.command = Some(PreflightCommand::expanding(
                    expansion,
                    self.main_loop_active,
                    cursor,
                ));
                assert!(
                    scanner_resume.is_none(),
                    "parked expansion owns its scanner child internally"
                );
                None
            } else if let Some(command) = frame.command.as_mut() {
                command.retain_scanner(cursor, scanner_resume);
                None
            } else {
                scanner_resume
            };
            let scanned = match scanned {
                Ok(scanned) => scanned,
                Err(error) => {
                    frame.write_retry_failure(error, cursor, alignment_scanner);
                    return OperationReadiness::Failed;
                }
            };
            if frame.command.as_ref().is_some_and(|command| {
                command.command.is_none()
                    && !matches!(command.phase, PreflightCommandPhase::ImmediatePdfRetry(_))
            }) {
                let _ = frame.command.take();
            }
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
        drop(context);
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
        if let Err(error) = report_pending_diagnostics(stores, diagnostic_effects, diagnostics) {
            frame.error = Some(error);
            return OperationReadiness::Failed;
        }
        let scanned = match scanned {
            ScannedOperation::<G>::Cold(scanned) => scanned,
            ScannedOperation::<G>::Hot(operation) => {
                frame.applied = Some(self.apply_hot_operation(
                    stores,
                    diagnostic_effects,
                    operation,
                    OperationOutputStart {
                        outer_paragraph_was_active,
                        root_main_file_origin,
                        artifact_count: stores.world().artifact_commits().len(),
                        effect_count: stores.world().effect_records().len(),
                        prepared_page_count: self.prepared_dvi_pages.len(),
                    },
                ));
                return OperationReadiness::Applied;
            }
        };
        frame.unavailable = Some(scanned);
        let resource_result = {
            let scanned = frame
                .unavailable
                .as_mut()
                .expect("scanned operation occupies its caller-owned frame");
            self.resolve_font_resource(scanned, stores)
                .and_then(|()| self.resolve_input_stream_resource(scanned, stores))
                .and_then(|()| self.resolve_pdf_image_resource(scanned, stores))
        };
        if let Err(error) = resource_result {
            frame.error = Some(error);
            return OperationReadiness::Failed;
        }
        let completed_preamble = match frame
            .unavailable
            .as_ref()
            .expect("resolved operation remains in its caller-owned frame")
        {
            ColdOperation::AlignmentPreambleStart { alignment } => {
                let alignment = *alignment;
                let preamble = match self
                    .command
                    .state_mut()
                    .take_completed_alignment_preamble(alignment)
                {
                    Ok(preamble) => preamble,
                    Err(_) => {
                        frame.error = Some(ExecError::MissingToken {
                            context: "completed alignment preamble",
                        });
                        return OperationReadiness::Failed;
                    }
                };
                Some((alignment, preamble))
            }
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
        let scanned = frame
            .unavailable
            .take()
            .expect("resolved operation remains in its caller-owned frame");
        let (scanned, promoted_alignment_roots) = match prepare_cold_operation(
            scanned,
            self.command.state_mut(),
            stores,
            &alignment_roots,
        ) {
            Ok(prepared) => prepared,
            Err(_) => {
                frame.error = Some(ExecError::MissingToken {
                    context: "cold operation root preparation",
                });
                return OperationReadiness::Failed;
            }
        };
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
        frame.prepared = Some(scanned);
        frame.alignment_preamble = alignment_preamble;
        frame.output_start = Some(OperationOutputStart {
            outer_paragraph_was_active,
            root_main_file_origin,
            artifact_count: stores.world().artifact_commits().len(),
            effect_count: stores.world().effect_records().len(),
            prepared_page_count: self.prepared_dvi_pages.len(),
        });
        OperationReadiness::Prepared
    }

    /// Applies a measured common operation without constructing the universal
    /// scan/preparation DTOs. `CommandProcessor` has released its borrow, but
    /// the enclosing direct-operation transaction and persistent interpreter
    /// remain the same ones that performed delivery and scanning.
    fn apply_hot_operation(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
        operation: hot_apply::HotOperation<G>,
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
        if result.is_ok() {
            self.fire_pending_page_output(stores, diagnostic_effects)?;
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
                    diagnostic_effects,
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
                diagnostic_effects,
                context,
            )?;
        }
        self.page_output_observations.clear();
        if result.is_ok() {
            self.finish_shipout_publication(
                output_start.artifact_count,
                output_start.effect_count,
                stores,
                output_start.root_main_file_origin,
            );
            self.finish_paragraph_boundary(
                output_start.outer_paragraph_was_active,
                output_start.root_main_file_origin,
                stores,
            );
        }
        result
    }

    fn apply_prepared_operation(
        &mut self,
        stores: &mut Universe<G>,
        scanned: PreparedColdCommand<G>,
        alignment_preamble: Option<PreparedAlignmentPreamble<G>>,
        output_start: OperationOutputStart,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<ReplayStep, ExecError> {
        let parking = self.suspend_main_control_parking(&scanned);
        #[cfg(feature = "profiling")]
        tex_state::measurement::record_hot_core_phase(
            tex_state::measurement::HotCorePhase::SemanticApply,
        );
        #[cfg(feature = "profiling")]
        let _semantic_allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::SemanticApply,
        );
        let scanned = match self.apply_host_owned_step(scanned, stores, diagnostic_effects) {
            ControlFlow::Break(applied) => {
                return self.finish_host_owned_step(
                    applied,
                    output_start,
                    stores,
                    diagnostic_effects,
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
        let context = stores.command_context().expect("cold operation admission");
        let mut scanned = match scanned {
            ColdOperation::ShowGroups { diagnostic: None } => ColdOperation::ShowGroups {
                diagnostic: Some(detached_showgroups(
                    &context,
                    &self.active_alignment,
                    &self.boxes,
                    &self.active_discretionaries,
                    &self.active_math_choices,
                    &self.active_math_left_boundaries,
                    &self.active_math_shifts,
                )),
            },
            scanned => scanned,
        };
        let reassigning_glue = self.local_glue_pointer_reassigned(&context, &scanned);
        let redundant_glue = self.etex_redundant_local_glue_assignment(&context, &scanned);
        drop(context);
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
        let reports_synchronous_auxiliary_error = matches!(
            &scanned,
            ColdOperation::IllegalPrevDepth { .. } | ColdOperation::IllegalSpaceFactor { .. }
        ) || matches!(
            &scanned,
            ColdOperation::PrevGraf { value } if *value < 0
        ) || matches!(
            &scanned,
            ColdOperation::SpaceFactor { value } if !(1..=32767).contains(value)
        );
        if reports_synchronous_auxiliary_error
            || matches!(
                &scanned,
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
            // apply. Publish that
            // already-committed trace at this admission boundary so the two
            // mechanisms cannot invert their canonical order. This recovery
            // operation requests no host resource and its error remains an
            // observable result even when ErrorStop jumps out.
            //
            // §1279's `\message`/`\errmessage` and §1264's
            // `new_interaction` are synchronous World-facing boundaries.
            // Any macro trace produced while scanning the message must be
            // visible before its expanded text is printed.
            //
            // `new_interaction` similarly requires the already-rendered
            // §1030 command trace to reach the old selector before `print_ln`
            // and the selector transition.
            // Otherwise the unconditional `print_ln` overtakes the detached
            // trace, moving TeX's blank line from after `\batchmode` to before
            // it and routing later output from the wrong partial-line state.
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
        let effect = {
            let context = stores.command_context().expect("live generation");
            applied_effect_observation(&scanned, &context)
        };
        let output_routine_was_active = self.boxes.output_routine_active;
        let mut command = CommandMachine {
            state: &mut self.command,
            fuel: self.fuel.fuel_mut(),
            capabilities: &mut self.capabilities,
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
        let mut result = match scanned {
            ColdOperation::ImmediateExtension(RootedImmediateExtension::PdfForm(request)) => {
                let provenance_demand = stores.provenance_demand();
                let provenance_budget_bytes =
                    stores.provenance_budgets().detached_artifact_recipe_bytes;
                let (form, source_resolver) = {
                    let mut context =
                        stores
                            .command_context()
                            .map_err(|_| ExecError::MissingToken {
                                context: "immediate form admission",
                            })?;
                    let form = apply_pdf_form_request(
                        request,
                        &mut context,
                        &mut self.modes,
                        &mut command,
                        true,
                    )?
                    .expect("immediate form creation returns a publication record");
                    let form_page = context
                        .copy_pdf_form_to_page(form.object())
                        .ok_or(ExecError::PdfXFormVoidBox)?;
                    let source_resolver =
                        DetachedArtifactSourceResolver::capture_page_list(form_page, &context);
                    (form, source_resolver)
                };
                let mut geometry = DetachedShipoutGeometry::default();
                publish_immediate_pdf_form(
                    form,
                    &mut command,
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
                Ok(ReplayStep::Continue)
            }
            scanned => {
                let context = stores
                    .command_context()
                    .map_err(|_| ExecError::MissingToken {
                        context: "cold operation admission",
                    })?;
                apply_cold_operation(
                    scanned,
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
                    &mut self.end_job_ejection_pending,
                )
            }
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
                if print.ensure_line_start {
                    stores.world_mut().publish_print_nl_text(
                        print.sink,
                        &print.text,
                        print.max_print_line,
                    );
                } else {
                    stores.world_mut().publish_print_text(
                        print.sink,
                        &print.text,
                        print.max_print_line,
                    );
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
                if let Some(receipt) = shipout_replay_box(shipout, stores, &mut command)?
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
        if result.is_ok() {
            self.resume_main_control_parking(parking, stores);
        }
        if result.is_ok() {
            self.fire_pending_page_output(stores, diagnostic_effects)?;
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
                        diagnostic_effects,
                    )
                    .into_iter()
                    .map(CommandObservation::Input),
            );
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
                diagnostic_effects,
                context,
            )?;
        }
        self.page_output_observations.clear();
        if result.is_ok() {
            self.finish_shipout_publication(
                output_start.artifact_count,
                output_start.effect_count,
                stores,
                output_start.root_main_file_origin,
            );
            self.finish_paragraph_boundary(
                output_start.outer_paragraph_was_active,
                output_start.root_main_file_origin,
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
            let mut processor = command_processor(
                &mut self.command,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
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
        let mut processor = command_processor(
            &mut self.command,
            self.fuel.fuel_mut(),
            &mut self.capabilities,
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
    pub fn accept(mut self) {
        self.modes.accept_checkpoint_candidate();
    }

    pub fn reject(mut self) {
        self.modes.reject_checkpoint_candidate();
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
    let (eligible, occupied) =
        match tail_index.and_then(|index| list.nodes(&context).owned_node(index)) {
            Some(Node::MathNoad(noad))
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
                match nodes.owned_node(0) {
                    Some(Node::MathNoad(accent))
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
                Node::MathNoad(MathNoad {
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
        && let Some(Node::MathNoad(noad)) = nodes.owned_node(0)
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
    let starts_left_node = |node: Option<&Node>| {
        matches!(
            node,
            Some(Node::MathNoad(MathNoad {
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
    retry: &mut Option<PreflightCommand<G>>,
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
                retry,
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
                retry,
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
        retry,
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
            let command = processor.commit_alignment_lookahead_delivery(lookahead);
            let current_line = command
                .direct_source_line_number()
                .unwrap_or_else(|| processor.current_file_line_number());
            Ok(ColdOperation::<G>::AlignmentFinish {
                alignment,
                current_line,
            })
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
    retry: &mut Option<PreflightCommand<G>>,
) -> Result<ScannedOperation<G>, ExecError> {
    prepare_command_trace(processor, mode, *shown_mode);
    let mut destination = None;
    if processor
        .get_x_token_into(&mut destination)
        .map_err(command_error)?
        != tex_command::DeliveryStatus::Command
    {
        return Ok(ColdOperation::<G>::EndOfInput.into());
    }
    let command = destination
        .take()
        .expect("command status initializes destination");
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
        _ => {
            assert!(
                retry
                    .replace(PreflightCommand::settled(command, None))
                    .is_none(),
                "noalign delivery owns an empty command frame",
            );
            dispatch_main_control_command(
                processor,
                retry
                    .as_mut()
                    .expect("noalign delivery installed its command frame"),
                mode,
                boxes,
                innermost_group,
                job_is_all_over,
                false,
                shown_mode,
                diagnostics,
                None,
                true,
            )
        }
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
    retry: &mut Option<PreflightCommand<G>>,
) -> Result<ScannedOperation<G>, ExecError> {
    prepare_command_trace(processor, mode, *shown_mode);
    let mut destination = None;
    let delivery = processor
        .get_x_alignment_delivery_into(main_loop_active, &mut destination)
        .map_err(command_error)?;
    match delivery {
        tex_command::DeliveryStatus::End => Ok(ColdOperation::<G>::EndOfInput.into()),
        // An executor-owned replay episode (a math field/group/choice branch
        // or discretionary part) retired mid-cell. This must be reported
        // exactly like ordinary `scan_step`'s `ReplayCompleted` case, rather
        // than falling through to interpret whatever the cascade found next
        // as this cell's own content: that next token can belong to the
        // *enclosing* cell/field context, not the just-retired episode.
        tex_command::DeliveryStatus::ReplayCompleted(episode) => {
            Ok(ColdOperation::<G>::ReplayCompleted(episode).into())
        }
        tex_command::DeliveryStatus::Command => {
            let command = destination.expect("command status initializes destination");
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
            assert!(
                retry
                    .replace(PreflightCommand::settled(command, None))
                    .is_none(),
                "alignment delivery owns an empty command frame",
            );
            dispatch_main_control_command(
                processor,
                retry
                    .as_mut()
                    .expect("alignment delivery installed its command frame"),
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
        tex_command::DeliveryStatus::AlignmentEndTemplate => {
            let event = tex_command::AlignmentDeliveryEvent::EndTemplate(
                destination.expect("alignment status initializes destination"),
            );
            scan_alignment_delivery_event(processor, alignment, event).map(Into::into)
        }
        tex_command::DeliveryStatus::AlignmentClosingBrace => {
            let event = tex_command::AlignmentDeliveryEvent::ClosingBrace(
                destination.expect("alignment status initializes destination"),
            );
            scan_alignment_delivery_event(processor, alignment, event).map(Into::into)
        }
        tex_command::DeliveryStatus::PendingExpanded => {
            unreachable!("alignment delivery commits terminal observations")
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
        tex_command::AlignmentDeliveryEvent::EndTemplate(delimiter) => {
            processor
                .begin_alignment_v_template(
                    alignment,
                    tex_command::AlignmentDeliveryEvent::EndTemplate(delimiter),
                )
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
    command: &mut PreflightCommand<G>,
    main_loop: bool,
    mode: Mode,
    boxes: &ReplayBoxes<G>,
    innermost_group: Option<GroupKind>,
    job_is_all_over: bool,
    display_eq_no: bool,
    shown_mode: &mut Option<Mode>,
    diagnostics: &mut Vec<PendingDiagnostic<G>>,
) -> Result<ScannedOperation<G>, ExecError> {
    let mut destination = None;
    match processor
        .resume_expansion_into(command.take_expansion(), main_loop, &mut destination)
        .map_err(command_error)?
    {
        tex_command::DeliveryStatus::End => {
            return Ok(ColdOperation::<G>::EndOfInput.into());
        }
        tex_command::DeliveryStatus::ReplayCompleted(episode) => {
            return Ok(ColdOperation::<G>::ReplayCompleted(episode).into());
        }
        tex_command::DeliveryStatus::Command => {}
        _ => unreachable!("preflight settlement has no alignment event"),
    };
    command.settle(destination.expect("command status initializes destination"));
    // TeX82 §§380 and 473--479 keep operand scanning under the newly settled
    // unexpandable command. Expansion owns the retry only until settlement;
    // after this point a resource failure must re-enter this command before
    // any nested scanner continuation can resume.
    let continues_main_loop = main_loop
        && matches!(
            command.current().meaning(),
            ResolvedMeaning::Static(
                Meaning::CharToken {
                    cat: Catcode::Letter | Catcode::Other,
                    ..
                } | Meaning::CharGiven(_)
                    | Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char)
            )
        );
    if !continues_main_loop {
        report_main_control_command_trace(processor, mode, command.current(), boxes, shown_mode);
    }
    if main_loop
        && matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal)
        && matches!(
            command.current().meaning(),
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

#[allow(clippy::too_many_arguments)]
fn scan_preflight_command<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    command: &mut PreflightCommand<G>,
    mode: Mode,
    boxes: &ReplayBoxes<G>,
    innermost_group: Option<GroupKind>,
    job_is_all_over: bool,
    display_eq_no: bool,
    shown_mode: &mut Option<Mode>,
    diagnostics: &mut Vec<PendingDiagnostic<G>>,
) -> Result<ScannedOperation<G>, ExecError> {
    if let Some(cursor) = command.cursor {
        processor.resume_delivery_cursor(cursor);
    }
    match command.phase {
        PreflightCommandPhase::Replay => {
            processor.resume_current_command(command.current());
            processor.observe_expanded_delivery(command.current());
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
        PreflightCommandPhase::Settled | PreflightCommandPhase::Raw => {
            processor.resume_current_command(command.current());
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
        PreflightCommandPhase::Expanding { main_loop } => {
            prepare_command_trace(processor, mode, *shown_mode);
            settle_preflight_step(
                processor,
                command,
                main_loop,
                mode,
                boxes,
                innermost_group,
                job_is_all_over,
                display_eq_no,
                shown_mode,
                diagnostics,
            )
        }
        PreflightCommandPhase::OperationScan => {
            processor.resume_current_command(command.current());
            let phase = command
                .operation_scan
                .take()
                .expect("operation-scan phase owns its exact scalar state");
            let mut suspended = None;
            let result = resume_pending_operation_scan(processor, phase, &mut suspended);
            if let Err(error) = &result
                && execution_error_needs_command_retry(error)
                && let Some(phase) = suspended
            {
                let child = processor
                    .take_scanner_resume()
                    .expect("a resuspended scalar scan retains its exact child capability");
                command.retain_operation_scan(processor.delivery_cursor(), phase, child);
            }
            if result.is_ok() {
                command.phase = PreflightCommandPhase::Settled;
                command.operation_scan = None;
            }
            result
        }
        PreflightCommandPhase::PrefixScan {
            global,
            flags,
            alignment,
            set_box_allowed,
        } => {
            processor.resume_current_command(command.current());
            let origin = command.current().origin();
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
                Some((global, flags)),
            )
            .map_err(|error| error.capture_command_origin(origin))
        }
        PreflightCommandPhase::PrefixedCommandScan {
            global,
            flags,
            set_box_allowed,
        } => {
            processor.resume_current_command(command.current());
            let mut suspended_operation_scan = None;
            let result = scan_command(
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
                &mut suspended_operation_scan,
            );
            if let Err(error) = &result
                && execution_error_needs_command_retry(error)
            {
                let child = processor
                    .take_scanner_resume()
                    .expect("a resuspended prefixed command retains its exact scanner child");
                if let Some(phase) = suspended_operation_scan {
                    command.retain_operation_scan(processor.delivery_cursor(), phase, child);
                } else {
                    command.phase = PreflightCommandPhase::PrefixedCommandScan {
                        global,
                        flags,
                        set_box_allowed,
                    };
                    command.retain_scanner(processor.delivery_cursor(), Some(child));
                }
            }
            if result.is_ok() {
                command.phase = PreflightCommandPhase::Settled;
                command.operation_scan = None;
            }
            result
        }
        PreflightCommandPhase::ImmediatePdfRetry(primitive) => match primitive {
            UnexpandablePrimitive::PdfObject => Ok(ColdOperation::<G>::ImmediateExtension(
                ImmediateExtension::PdfObject(
                    processor.scan_pdf_object_request().map_err(command_error)?,
                )
                .into(),
            )
            .into()),
            UnexpandablePrimitive::PdfXForm => Ok(ColdOperation::<G>::ImmediateExtension(
                ImmediateExtension::PdfForm(
                    processor
                        .scan_pdf_form_request(UnexpandablePrimitive::PdfXForm)
                        .map_err(command_error)?,
                )
                .into(),
            )
            .into()),
            UnexpandablePrimitive::PdfXImage => Ok(ColdOperation::<G>::PdfXImage {
                request: processor
                    .scan_pdf_image_request()
                    .map_err(command_error)?
                    .into(),
                resource: PdfImageResource::Unavailable,
            }
            .into()),
            _ => unreachable!("only immediate PDF retries reach this delivery"),
        },
    }
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
    command_owner: &mut Option<PreflightCommand<G>>,
) -> Result<ScannedOperation<G>, ExecError> {
    // TeX82 §1030 has two fetch labels, not one. `big_switch` uses
    // `get_x_token`; §1034's inner character loop instead re-enters at
    // §1038's `main_loop_lookahead`, whose bare `get_next` is what keeps a
    // run of adjacent characters from being delivered through expansion.
    prepare_command_trace(processor, mode, *shown_mode);
    let mut destination = None;
    let delivery = if main_loop_active {
        processor.main_loop_lookahead_into(&mut destination)
    } else {
        processor.get_x_token_with_replay_completion_into(&mut destination)
    };
    match delivery.map_err(command_error)? {
        tex_command::DeliveryStatus::End => {
            return Ok(ColdOperation::<G>::EndOfInput.into());
        }
        tex_command::DeliveryStatus::ReplayCompleted(episode) => {
            return Ok(ColdOperation::<G>::ReplayCompleted(episode).into());
        }
        tex_command::DeliveryStatus::Command => {}
        _ => unreachable!("main-control delivery has no alignment event"),
    };
    let command = destination.expect("command status initializes destination");
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
    assert!(
        command_owner
            .replace(PreflightCommand::settled(command, None))
            .is_none(),
        "fresh replay delivery owns an empty command frame",
    );
    dispatch_main_control_command(
        processor,
        command_owner
            .as_mut()
            .expect("fresh replay delivery installed its command frame"),
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

fn scan_count_register_assignment<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    mut index: Option<u16>,
    global: bool,
    phase: RegisterAssignmentScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<ColdOperation<G>, ExecError> {
    if phase == RegisterAssignmentScanPhase::RegisterIndex {
        let scalar_phase = PendingOperationScanPhase::Count {
            index,
            global,
            phase: RegisterAssignmentScanPhase::RegisterIndex,
        };
        let scan = processor.scan_profile_register_index_retained();
        index = Some(retain_operation_scalar(
            processor,
            scan,
            scalar_phase,
            suspended,
        )?);
    }
    if phase != RegisterAssignmentScanPhase::Value {
        let scalar_phase = PendingOperationScanPhase::Count {
            index,
            global,
            phase: RegisterAssignmentScanPhase::OptionalEquals,
        };
        let scan = processor.scan_optional_equals_retained();
        let _ = retain_operation_scalar(processor, scan, scalar_phase, suspended)?;
    }
    let scalar_phase = PendingOperationScanPhase::Count {
        index,
        global,
        phase: RegisterAssignmentScanPhase::Value,
    };
    let scan = processor.scan_integer_retained();
    let value = retain_operation_scalar(processor, scan, scalar_phase, suspended)?.value;
    Ok(ColdOperation::Count {
        index: index.expect("count assignment retains its completed register index"),
        value,
        global,
    })
}

fn scan_dimension_register_assignment<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    mut index: Option<u16>,
    global: bool,
    phase: RegisterAssignmentScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<ColdOperation<G>, ExecError> {
    if phase == RegisterAssignmentScanPhase::RegisterIndex {
        let scalar_phase = PendingOperationScanPhase::Dimension {
            index,
            global,
            phase: RegisterAssignmentScanPhase::RegisterIndex,
        };
        let scan = processor.scan_profile_register_index_retained();
        index = Some(retain_operation_scalar(
            processor,
            scan,
            scalar_phase,
            suspended,
        )?);
    }
    if phase != RegisterAssignmentScanPhase::Value {
        let scalar_phase = PendingOperationScanPhase::Dimension {
            index,
            global,
            phase: RegisterAssignmentScanPhase::OptionalEquals,
        };
        let scan = processor.scan_optional_equals_retained();
        let _ = retain_operation_scalar(processor, scan, scalar_phase, suspended)?;
    }
    let scalar_phase = PendingOperationScanPhase::Dimension {
        index,
        global,
        phase: RegisterAssignmentScanPhase::Value,
    };
    let scan = processor.scan_dimension_retained();
    let value = retain_operation_scalar(processor, scan, scalar_phase, suspended)?.value;
    Ok(ColdOperation::Dimen {
        index: index.expect("dimension assignment retains its completed register index"),
        value,
        global,
    })
}

fn scan_glue_register_assignment<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    mut index: Option<u16>,
    global: bool,
    mu: bool,
    phase: RegisterAssignmentScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<ColdOperation<G>, ExecError> {
    if phase == RegisterAssignmentScanPhase::RegisterIndex {
        let scalar_phase = PendingOperationScanPhase::Glue {
            index,
            global,
            mu,
            phase: RegisterAssignmentScanPhase::RegisterIndex,
        };
        let scan = processor.scan_profile_register_index_retained();
        index = Some(retain_operation_scalar(
            processor,
            scan,
            scalar_phase,
            suspended,
        )?);
    }
    if phase != RegisterAssignmentScanPhase::Value {
        let scalar_phase = PendingOperationScanPhase::Glue {
            index,
            global,
            mu,
            phase: RegisterAssignmentScanPhase::OptionalEquals,
        };
        let scan = processor.scan_optional_equals_retained();
        let _ = retain_operation_scalar(processor, scan, scalar_phase, suspended)?;
    }
    let scalar_phase = PendingOperationScanPhase::Glue {
        index,
        global,
        mu,
        phase: RegisterAssignmentScanPhase::Value,
    };
    let scan = processor.scan_glue_retained(mu);
    let value = retain_operation_scalar(processor, scan, scalar_phase, suspended)?.value;
    let source_identity = processor.scanned_glue_identity();
    let source_register = processor.scanned_glue_register();
    let index = index.expect("glue assignment retains its completed register index");
    if mu {
        Ok(ColdOperation::Muskip {
            index,
            value,
            source_identity,
            source_register,
            redundant: false,
            reassigning: false,
            global,
        })
    } else {
        Ok(ColdOperation::Skip {
            index,
            value,
            source_identity,
            source_register,
            redundant: false,
            reassigning: false,
            global,
        })
    }
}

fn scan_box_dimension_assignment<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    mut index: Option<u16>,
    dimension: tex_state::BoxDimension,
    global: bool,
    phase: RegisterAssignmentScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<ColdOperation<G>, ExecError> {
    if phase == RegisterAssignmentScanPhase::RegisterIndex {
        let scalar_phase = PendingOperationScanPhase::BoxDimension {
            index,
            dimension,
            global,
            phase: RegisterAssignmentScanPhase::RegisterIndex,
        };
        let scan = processor.scan_profile_register_index_retained();
        index = Some(retain_operation_scalar(
            processor,
            scan,
            scalar_phase,
            suspended,
        )?);
    }
    if phase != RegisterAssignmentScanPhase::Value {
        let scalar_phase = PendingOperationScanPhase::BoxDimension {
            index,
            dimension,
            global,
            phase: RegisterAssignmentScanPhase::OptionalEquals,
        };
        let scan = processor.scan_optional_equals_retained();
        let _ = retain_operation_scalar(processor, scan, scalar_phase, suspended)?;
    }
    let scalar_phase = PendingOperationScanPhase::BoxDimension {
        index,
        dimension,
        global,
        phase: RegisterAssignmentScanPhase::Value,
    };
    let scan = processor.scan_dimension_retained();
    let value = retain_operation_scalar(processor, scan, scalar_phase, suspended)?.value;
    Ok(ColdOperation::BoxDimensionAssignment {
        index: index.expect("box-dimension assignment retains its completed register index"),
        dimension,
        value,
        global,
    })
}

fn retain_operation_scalar<G, T>(
    processor: &mut CommandProcessor<'_, '_, G>,
    scan: tex_command::RetainedScalarScan<G, T>,
    phase: PendingOperationScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<T, ExecError> {
    match scan {
        tex_command::RetainedScalarScan::Complete(value) => {
            *suspended = None;
            Ok(value)
        }
        tex_command::RetainedScalarScan::Suspended { error, child } => {
            processor.install_scanner_resume(Some(child));
            *suspended = Some(phase);
            Err(command_error(error))
        }
        tex_command::RetainedScalarScan::Failed(error) => {
            *suspended = None;
            Err(command_error(error))
        }
    }
}

fn scan_unary_scalar_operation<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    meaning: Meaning,
    global: bool,
    origin: tex_state::token::OriginId,
    phase: UnaryOperationScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<ColdOperation<G>, ExecError> {
    let has_optional_equals = matches!(
        meaning,
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::PrevDepth
                | UnexpandablePrimitive::InteractionMode
                | UnexpandablePrimitive::SpaceFactor
                | UnexpandablePrimitive::PrevGraf
        ) | Meaning::IntParam(_)
            | Meaning::DimenParam(_)
            | Meaning::PageDimension(_)
            | Meaning::PageInteger(_)
    );
    if phase == UnaryOperationScanPhase::OptionalEquals && has_optional_equals {
        let scalar_phase = PendingOperationScanPhase::Unary {
            meaning,
            global,
            origin,
            phase: UnaryOperationScanPhase::OptionalEquals,
        };
        let scan = processor.scan_optional_equals_retained();
        let _ = retain_operation_scalar(processor, scan, scalar_phase, suspended)?;
    }
    let scalar_phase = PendingOperationScanPhase::Unary {
        meaning,
        global,
        origin,
        phase: UnaryOperationScanPhase::Value,
    };
    match meaning {
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HSkip) => {
            let scan = processor.scan_glue_retained(false);
            let value = retain_operation_scalar(processor, scan, scalar_phase, suspended)?.value;
            Ok(ColdOperation::HorizontalSkip { value })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VSkip) => {
            let scan = processor.scan_glue_retained(false);
            let value = retain_operation_scalar(processor, scan, scalar_phase, suspended)?.value;
            Ok(ColdOperation::VerticalSkip { value })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Kern) => {
            let scan = processor.scan_dimension_retained();
            let amount = retain_operation_scalar(processor, scan, scalar_phase, suspended)?.value;
            Ok(ColdOperation::Kern { amount })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PrevDepth) => {
            let scan = processor.scan_dimension_retained();
            let value = retain_operation_scalar(processor, scan, scalar_phase, suspended)?.value;
            Ok(ColdOperation::PrevDepth { value })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Penalty) => {
            let scan = processor.scan_integer_retained();
            let amount = retain_operation_scalar(processor, scan, scalar_phase, suspended)?.value;
            Ok(ColdOperation::Penalty { amount })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfRefXImage) => {
            let scan = processor.scan_integer_retained();
            let object = retain_operation_scalar(processor, scan, scalar_phase, suspended)?.value;
            Ok(ColdOperation::PdfRefXImage { object })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfSetRandomSeed) => {
            let scan = processor.scan_integer_retained();
            let seed = retain_operation_scalar(processor, scan, scalar_phase, suspended)?.value;
            Ok(ColdOperation::PdfSetRandomSeed {
                seed: seed.saturating_abs(),
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SetLanguage) => {
            let scan = processor.scan_integer_retained();
            let language = retain_operation_scalar(processor, scan, scalar_phase, suspended)?.value;
            Ok(ColdOperation::SetLanguage { language })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::InteractionMode) => {
            let scan = processor.scan_integer_retained();
            let value = retain_operation_scalar(processor, scan, scalar_phase, suspended)?.value;
            Ok(ColdOperation::SetInteractionModeValue {
                value,
                context: processor.error_context(),
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SpaceFactor) => {
            let scan = processor.scan_integer_retained();
            let value = retain_operation_scalar(processor, scan, scalar_phase, suspended)?.value;
            Ok(ColdOperation::SpaceFactor { value })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PrevGraf) => {
            let scan = processor.scan_integer_retained();
            let value = retain_operation_scalar(processor, scan, scalar_phase, suspended)?.value;
            Ok(ColdOperation::PrevGraf { value })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char) => {
            let scan = processor.scan_integer_retained();
            let value = retain_operation_scalar(processor, scan, scalar_phase, suspended)?.value;
            Ok(ColdOperation::CharacterCode {
                value,
                origin,
                suppress_left_boundary: false,
            })
        }
        Meaning::IntParam(index) => {
            let scan = processor.scan_integer_retained();
            let value = retain_operation_scalar(processor, scan, scalar_phase, suspended)?.value;
            Ok(ColdOperation::IntParam {
                index,
                value,
                global,
            })
        }
        Meaning::DimenParam(index) => {
            let scan = processor.scan_dimension_retained();
            let value = retain_operation_scalar(processor, scan, scalar_phase, suspended)?.value;
            Ok(ColdOperation::DimenParam {
                index,
                value,
                global,
            })
        }
        Meaning::PageDimension(dimension) => {
            let scan = processor.scan_dimension_retained();
            let value = retain_operation_scalar(processor, scan, scalar_phase, suspended)?.value;
            Ok(ColdOperation::PageDimension { dimension, value })
        }
        Meaning::PageInteger(integer) => {
            let scan = processor.scan_integer_retained();
            let value = retain_operation_scalar(processor, scan, scalar_phase, suspended)?.value;
            Ok(ColdOperation::PageInteger { integer, value })
        }
        _ => unreachable!("unary scalar descriptor restricts command meanings"),
    }
}

fn scan_paragraph_shape_assignment<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    global: bool,
    phase: ParagraphShapeScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<ColdOperation<G>, ExecError> {
    let phase = if matches!(phase, ParagraphShapeScanPhase::OptionalEquals) {
        let scalar_phase = PendingOperationScanPhase::ParagraphShape {
            global,
            phase: ParagraphShapeScanPhase::OptionalEquals,
        };
        let scan = processor.scan_optional_equals_retained();
        let _ = retain_operation_scalar(processor, scan, scalar_phase, suspended)?;
        ParagraphShapeScanPhase::Count
    } else {
        phase
    };
    let phase = if matches!(phase, ParagraphShapeScanPhase::Count) {
        let scalar_phase = PendingOperationScanPhase::ParagraphShape {
            global,
            phase: ParagraphShapeScanPhase::Count,
        };
        let scan = processor.scan_integer_retained();
        let count = retain_operation_scalar(processor, scan, scalar_phase, suspended)?
            .value
            .max(0) as usize;
        let mut lines = Vec::new();
        lines
            .try_reserve_exact(count)
            .map_err(|_| ExecError::ArithmeticOverflow)?;
        ParagraphShapeScanPhase::Indent {
            remaining: count,
            lines,
        }
    } else {
        phase
    };
    let (mut remaining, mut lines, mut retained_indent) = match phase {
        ParagraphShapeScanPhase::Indent { remaining, lines } => (remaining, lines, None),
        ParagraphShapeScanPhase::Width {
            remaining,
            lines,
            indent,
        } => (remaining, lines, Some(indent)),
        ParagraphShapeScanPhase::OptionalEquals | ParagraphShapeScanPhase::Count => unreachable!(),
    };
    while remaining != 0 {
        let indent = match retained_indent.take() {
            Some(indent) => indent,
            None => {
                let scan = processor.scan_dimension_retained();
                match scan {
                    tex_command::RetainedScalarScan::Complete(indent) => indent.value,
                    tex_command::RetainedScalarScan::Suspended { error, child } => {
                        processor.install_scanner_resume(Some(child));
                        *suspended = Some(PendingOperationScanPhase::ParagraphShape {
                            global,
                            phase: ParagraphShapeScanPhase::Indent { remaining, lines },
                        });
                        return Err(command_error(error));
                    }
                    tex_command::RetainedScalarScan::Failed(error) => {
                        return Err(command_error(error));
                    }
                }
            }
        };
        let scan = processor.scan_dimension_retained();
        let width = match scan {
            tex_command::RetainedScalarScan::Complete(width) => width.value,
            tex_command::RetainedScalarScan::Suspended { error, child } => {
                processor.install_scanner_resume(Some(child));
                *suspended = Some(PendingOperationScanPhase::ParagraphShape {
                    global,
                    phase: ParagraphShapeScanPhase::Width {
                        remaining,
                        lines,
                        indent,
                    },
                });
                return Err(command_error(error));
            }
            tex_command::RetainedScalarScan::Failed(error) => {
                return Err(command_error(error));
            }
        };
        lines.push(ParagraphShapeLine { indent, width });
        remaining -= 1;
    }
    Ok(ColdOperation::ParagraphShape { lines, global })
}

fn scan_penalty_array_assignment<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    kind: tex_state::PenaltyArrayKind,
    global: bool,
    phase: PenaltyArrayScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<ColdOperation<G>, ExecError> {
    let phase = if matches!(phase, PenaltyArrayScanPhase::OptionalEquals) {
        let scalar_phase = PendingOperationScanPhase::PenaltyArray {
            kind,
            global,
            phase: PenaltyArrayScanPhase::OptionalEquals,
        };
        let scan = processor.scan_optional_equals_retained();
        let _ = retain_operation_scalar(processor, scan, scalar_phase, suspended)?;
        PenaltyArrayScanPhase::Count
    } else {
        phase
    };
    let phase = if matches!(phase, PenaltyArrayScanPhase::Count) {
        let scalar_phase = PendingOperationScanPhase::PenaltyArray {
            kind,
            global,
            phase: PenaltyArrayScanPhase::Count,
        };
        let scan = processor.scan_integer_retained();
        let count = retain_operation_scalar(processor, scan, scalar_phase, suspended)?
            .value
            .max(0) as usize;
        let mut values = Vec::new();
        values
            .try_reserve_exact(count)
            .map_err(|_| ExecError::ArithmeticOverflow)?;
        PenaltyArrayScanPhase::Value {
            remaining: count,
            values,
        }
    } else {
        phase
    };
    let PenaltyArrayScanPhase::Value {
        mut remaining,
        mut values,
    } = phase
    else {
        unreachable!()
    };
    while remaining != 0 {
        let scan = processor.scan_integer_retained();
        let value = match scan {
            tex_command::RetainedScalarScan::Complete(value) => value.value,
            tex_command::RetainedScalarScan::Suspended { error, child } => {
                processor.install_scanner_resume(Some(child));
                *suspended = Some(PendingOperationScanPhase::PenaltyArray {
                    kind,
                    global,
                    phase: PenaltyArrayScanPhase::Value { remaining, values },
                });
                return Err(command_error(error));
            }
            tex_command::RetainedScalarScan::Failed(error) => {
                return Err(command_error(error));
            }
        };
        values.push(value);
        remaining -= 1;
    }
    Ok(ColdOperation::PenaltyArray {
        kind,
        values,
        global,
    })
}

fn scan_font_dimen_assignment<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    phase: FontDimenScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<ColdOperation<G>, ExecError> {
    let phase = if matches!(phase, FontDimenScanPhase::Number) {
        let scan = processor.scan_integer_retained();
        let number = retain_operation_scalar(
            processor,
            scan,
            PendingOperationScanPhase::FontDimen(FontDimenScanPhase::Number),
            suspended,
        )?
        .value;
        FontDimenScanPhase::Font { number }
    } else {
        phase
    };
    let phase = match phase {
        FontDimenScanPhase::Font { number } => {
            let scan = processor.scan_font_selector_retained();
            let font = retain_operation_scalar(
                processor,
                scan,
                PendingOperationScanPhase::FontDimen(FontDimenScanPhase::Font { number }),
                suspended,
            )?;
            let recovery_context =
                (!processor.font_dimen_writable(font, number)).then(|| processor.error_context());
            FontDimenScanPhase::OptionalEquals {
                number,
                font,
                recovery_context,
            }
        }
        phase => phase,
    };
    let phase = match phase {
        FontDimenScanPhase::OptionalEquals {
            number,
            font,
            recovery_context,
        } => {
            let scan = processor.scan_optional_equals_retained();
            match scan {
                tex_command::RetainedScalarScan::Complete(_) => FontDimenScanPhase::Value {
                    number,
                    font,
                    recovery_context,
                },
                tex_command::RetainedScalarScan::Suspended { error, child } => {
                    processor.install_scanner_resume(Some(child));
                    *suspended = Some(PendingOperationScanPhase::FontDimen(
                        FontDimenScanPhase::OptionalEquals {
                            number,
                            font,
                            recovery_context,
                        },
                    ));
                    return Err(command_error(error));
                }
                tex_command::RetainedScalarScan::Failed(error) => {
                    return Err(command_error(error));
                }
            }
        }
        phase => phase,
    };
    let FontDimenScanPhase::Value {
        number,
        font,
        recovery_context,
    } = phase
    else {
        unreachable!()
    };
    let scan = processor.scan_dimension_retained();
    match scan {
        tex_command::RetainedScalarScan::Complete(value) => Ok(ColdOperation::FontDimen {
            font,
            number,
            value: value.value,
            recovery_context,
        }),
        tex_command::RetainedScalarScan::Suspended { error, child } => {
            processor.install_scanner_resume(Some(child));
            *suspended = Some(PendingOperationScanPhase::FontDimen(
                FontDimenScanPhase::Value {
                    number,
                    font,
                    recovery_context,
                },
            ));
            Err(command_error(error))
        }
        tex_command::RetainedScalarScan::Failed(error) => Err(command_error(error)),
    }
}

fn scan_font_integer_assignment<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    primitive: UnexpandablePrimitive,
    phase: FontIntegerScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<ColdOperation<G>, ExecError> {
    let phase = if matches!(phase, FontIntegerScanPhase::Font) {
        let scan = processor.scan_font_selector_retained();
        let font = retain_operation_scalar(
            processor,
            scan,
            PendingOperationScanPhase::FontInteger {
                primitive,
                phase: FontIntegerScanPhase::Font,
            },
            suspended,
        )?;
        FontIntegerScanPhase::OptionalEquals { font }
    } else {
        phase
    };
    let phase = match phase {
        FontIntegerScanPhase::OptionalEquals { font } => {
            let scan = processor.scan_optional_equals_retained();
            let _ = retain_operation_scalar(
                processor,
                scan,
                PendingOperationScanPhase::FontInteger {
                    primitive,
                    phase: FontIntegerScanPhase::OptionalEquals { font },
                },
                suspended,
            )?;
            FontIntegerScanPhase::Value { font }
        }
        phase => phase,
    };
    let FontIntegerScanPhase::Value { font } = phase else {
        unreachable!()
    };
    let scan = processor.scan_integer_retained();
    let value = retain_operation_scalar(
        processor,
        scan,
        PendingOperationScanPhase::FontInteger {
            primitive,
            phase: FontIntegerScanPhase::Value { font },
        },
        suspended,
    )?
    .value;
    Ok(ColdOperation::FontInteger {
        font,
        skew: primitive == UnexpandablePrimitive::SkewChar,
        value,
    })
}

fn scan_code_table_assignment<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    primitive: UnexpandablePrimitive,
    global: bool,
    phase: CodeTableScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<ColdOperation<G>, ExecError> {
    let phase = if matches!(phase, CodeTableScanPhase::Character) {
        let scan =
            processor.scan_restricted_integer_retained(RestrictedIntegerClass::CharacterCode);
        let character = retain_operation_scalar(
            processor,
            scan,
            PendingOperationScanPhase::CodeTable {
                primitive,
                global,
                phase: CodeTableScanPhase::Character,
            },
            suspended,
        )?
        .value;
        let character =
            char::from_u32(character as u32).expect("scan_char_num returns a valid character");
        CodeTableScanPhase::OptionalEquals { character }
    } else {
        phase
    };
    let phase = match phase {
        CodeTableScanPhase::OptionalEquals { character } => {
            let scan = processor.scan_optional_equals_retained();
            let _ = retain_operation_scalar(
                processor,
                scan,
                PendingOperationScanPhase::CodeTable {
                    primitive,
                    global,
                    phase: CodeTableScanPhase::OptionalEquals { character },
                },
                suspended,
            )?;
            CodeTableScanPhase::Value { character }
        }
        phase => phase,
    };
    let CodeTableScanPhase::Value { character } = phase else {
        unreachable!()
    };
    let scan = processor.scan_integer_retained();
    let value = retain_operation_scalar(
        processor,
        scan,
        PendingOperationScanPhase::CodeTable {
            primitive,
            global,
            phase: CodeTableScanPhase::Value { character },
        },
        suspended,
    )?
    .value;
    Ok(ColdOperation::CodeTable {
        primitive,
        character,
        value,
        global,
    })
}

fn scan_pdf_font_code_assignment<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    primitive: UnexpandablePrimitive,
    phase: PdfFontCodeScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<ColdOperation<G>, ExecError> {
    let phase = if matches!(phase, PdfFontCodeScanPhase::Font) {
        let scan = processor.scan_font_selector_retained();
        let font = retain_operation_scalar(
            processor,
            scan,
            PendingOperationScanPhase::PdfFontCode {
                primitive,
                phase: PdfFontCodeScanPhase::Font,
            },
            suspended,
        )?;
        PdfFontCodeScanPhase::Character { font }
    } else {
        phase
    };
    let phase = match phase {
        PdfFontCodeScanPhase::Character { font } => {
            let scan =
                processor.scan_restricted_integer_retained(RestrictedIntegerClass::CharacterCode);
            let character = retain_operation_scalar(
                processor,
                scan,
                PendingOperationScanPhase::PdfFontCode {
                    primitive,
                    phase: PdfFontCodeScanPhase::Character { font },
                },
                suspended,
            )?
            .value;
            PdfFontCodeScanPhase::OptionalEquals {
                font,
                character: u8::try_from(character)
                    .expect("pdfTeX character scanner is byte bounded"),
            }
        }
        phase => phase,
    };
    let phase = match phase {
        PdfFontCodeScanPhase::OptionalEquals { font, character } => {
            let scan = processor.scan_optional_equals_retained();
            let _ = retain_operation_scalar(
                processor,
                scan,
                PendingOperationScanPhase::PdfFontCode {
                    primitive,
                    phase: PdfFontCodeScanPhase::OptionalEquals { font, character },
                },
                suspended,
            )?;
            PdfFontCodeScanPhase::Value { font, character }
        }
        phase => phase,
    };
    let PdfFontCodeScanPhase::Value { font, character } = phase else {
        unreachable!()
    };
    let scan = processor.scan_integer_retained();
    let value = retain_operation_scalar(
        processor,
        scan,
        PendingOperationScanPhase::PdfFontCode {
            primitive,
            phase: PdfFontCodeScanPhase::Value { font, character },
        },
        suspended,
    )?
    .value;
    Ok(ColdOperation::PdfFontCode {
        table: pdf_font_code_table(primitive),
        font,
        character,
        value,
    })
}

fn scan_pdf_font_expand_assignment<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    phase: PdfFontExpandScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<ColdOperation<G>, ExecError> {
    let phase = if matches!(phase, PdfFontExpandScanPhase::Font) {
        let scan = processor.scan_font_selector_retained();
        let font = retain_operation_scalar(
            processor,
            scan,
            PendingOperationScanPhase::PdfFontExpand(PdfFontExpandScanPhase::Font),
            suspended,
        )?;
        PdfFontExpandScanPhase::OptionalEquals { font }
    } else {
        phase
    };
    let phase = match phase {
        PdfFontExpandScanPhase::OptionalEquals { font } => {
            let scan = processor.scan_optional_equals_retained();
            let _ = retain_operation_scalar(
                processor,
                scan,
                PendingOperationScanPhase::PdfFontExpand(PdfFontExpandScanPhase::OptionalEquals {
                    font,
                }),
                suspended,
            )?;
            PdfFontExpandScanPhase::Stretch { font }
        }
        phase => phase,
    };
    let phase = match phase {
        PdfFontExpandScanPhase::Stretch { font } => {
            let scan = processor.scan_integer_retained();
            let stretch = retain_operation_scalar(
                processor,
                scan,
                PendingOperationScanPhase::PdfFontExpand(PdfFontExpandScanPhase::Stretch { font }),
                suspended,
            )?
            .value;
            PdfFontExpandScanPhase::Shrink { font, stretch }
        }
        phase => phase,
    };
    let phase = match phase {
        PdfFontExpandScanPhase::Shrink { font, stretch } => {
            let scan = processor.scan_integer_retained();
            let shrink = retain_operation_scalar(
                processor,
                scan,
                PendingOperationScanPhase::PdfFontExpand(PdfFontExpandScanPhase::Shrink {
                    font,
                    stretch,
                }),
                suspended,
            )?
            .value;
            PdfFontExpandScanPhase::Step {
                font,
                stretch,
                shrink,
            }
        }
        phase => phase,
    };
    let phase = match phase {
        PdfFontExpandScanPhase::Step {
            font,
            stretch,
            shrink,
        } => {
            let scan = processor.scan_integer_retained();
            let step = retain_operation_scalar(
                processor,
                scan,
                PendingOperationScanPhase::PdfFontExpand(PdfFontExpandScanPhase::Step {
                    font,
                    stretch,
                    shrink,
                }),
                suspended,
            )?
            .value;
            PdfFontExpandScanPhase::AutoExpand {
                font,
                stretch,
                shrink,
                step,
            }
        }
        phase => phase,
    };
    let PdfFontExpandScanPhase::AutoExpand {
        font,
        stretch,
        shrink,
        step,
    } = phase
    else {
        unreachable!()
    };
    let scan = processor.scan_keyword_retained("autoexpand");
    let auto_expand = retain_operation_scalar(
        processor,
        scan,
        PendingOperationScanPhase::PdfFontExpand(PdfFontExpandScanPhase::AutoExpand {
            font,
            stretch,
            shrink,
            step,
        }),
        suspended,
    )?
    .value;
    let spec = tex_typeset::expansion::FontExpansionSpec::new(stretch, shrink, step, auto_expand)?;
    Ok(ColdOperation::PdfFontExpand { font, spec })
}

fn scan_font_only_operation<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    meaning: Meaning,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<ColdOperation<G>, ExecError> {
    let scan = processor.scan_font_selector_retained();
    let font = retain_operation_scalar(
        processor,
        scan,
        PendingOperationScanPhase::FontOnly { meaning },
        suspended,
    )?;
    match meaning {
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfNoLigatures) => {
            Ok(ColdOperation::PdfNoLigatures { font })
        }
        _ => unreachable!("font-only descriptor restricts command meanings"),
    }
}

fn scan_open_out_operation<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    phase: OpenOutScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<ColdOperation<G>, ExecError> {
    let phase = if matches!(phase, OpenOutScanPhase::Stream) {
        let scan = processor.scan_restricted_integer_retained(RestrictedIntegerClass::FourBit);
        let stream = retain_operation_scalar(
            processor,
            scan,
            PendingOperationScanPhase::OpenOut(OpenOutScanPhase::Stream),
            suspended,
        )?
        .value as u8;
        OpenOutScanPhase::OptionalEquals { stream }
    } else {
        phase
    };
    let phase = match phase {
        OpenOutScanPhase::OptionalEquals { stream } => {
            let scan = processor.scan_optional_equals_retained();
            let _ = retain_operation_scalar(
                processor,
                scan,
                PendingOperationScanPhase::OpenOut(OpenOutScanPhase::OptionalEquals { stream }),
                suspended,
            )?;
            OpenOutScanPhase::FileName { stream }
        }
        phase => phase,
    };
    let OpenOutScanPhase::FileName { stream } = phase else {
        unreachable!()
    };
    let scan = processor.scan_file_name_retained();
    let file_name = retain_operation_scalar(
        processor,
        scan,
        PendingOperationScanPhase::OpenOut(OpenOutScanPhase::FileName { stream }),
        suspended,
    )?;
    Ok(ColdOperation::DeferredOpenOut {
        stream,
        file_name: file_name.packed(),
    })
}

fn scan_marks_operation<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    phase: MarksScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<ColdOperation<G>, ExecError> {
    let phase = if matches!(phase, MarksScanPhase::Class) {
        let scan = processor.scan_extended_register_index_retained();
        let class = retain_operation_scalar(
            processor,
            scan,
            PendingOperationScanPhase::Marks(MarksScanPhase::Class),
            suspended,
        )?;
        MarksScanPhase::Text { class }
    } else {
        phase
    };
    let MarksScanPhase::Text { class } = phase else {
        unreachable!()
    };
    let scan = processor.scan_balanced_text_retained(true);
    let text = retain_operation_scalar(
        processor,
        scan,
        PendingOperationScanPhase::Marks(MarksScanPhase::Text { class }),
        suspended,
    )?;
    Ok(ColdOperation::Mark {
        class,
        tokens: text.tokens,
    })
}

fn scan_math_family_assignment<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    size: tex_command::MathFamilySize,
    global: bool,
    phase: MathFamilyScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<ColdOperation<G>, ExecError> {
    let phase = if matches!(phase, MathFamilyScanPhase::Family) {
        let scan = processor.scan_math_family_retained(size);
        let family = retain_operation_scalar(
            processor,
            scan,
            PendingOperationScanPhase::MathFamily {
                size,
                global,
                phase: MathFamilyScanPhase::Family,
            },
            suspended,
        )?;
        MathFamilyScanPhase::OptionalEquals { family }
    } else {
        phase
    };
    let phase = match phase {
        MathFamilyScanPhase::OptionalEquals { family } => {
            let scan = processor.scan_optional_equals_retained();
            let _ = retain_operation_scalar(
                processor,
                scan,
                PendingOperationScanPhase::MathFamily {
                    size,
                    global,
                    phase: MathFamilyScanPhase::OptionalEquals { family },
                },
                suspended,
            )?;
            MathFamilyScanPhase::Font { family }
        }
        phase => phase,
    };
    let MathFamilyScanPhase::Font { family } = phase else {
        unreachable!()
    };
    let scan = processor.scan_font_selector_retained();
    let font = retain_operation_scalar(
        processor,
        scan,
        PendingOperationScanPhase::MathFamily {
            size,
            global,
            phase: MathFamilyScanPhase::Font { family },
        },
        suspended,
    )?;
    Ok(ColdOperation::MathFamily {
        family,
        font,
        global,
    })
}

fn resume_pending_operation_scan<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    pending: PendingOperationScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<ScannedOperation<G>, ExecError> {
    let cold = match pending {
        PendingOperationScanPhase::Count {
            index,
            global,
            phase,
        } => scan_count_register_assignment(processor, index, global, phase, suspended),
        PendingOperationScanPhase::Dimension {
            index,
            global,
            phase,
        } => scan_dimension_register_assignment(processor, index, global, phase, suspended),
        PendingOperationScanPhase::BoxDimension {
            index,
            dimension,
            global,
            phase,
        } => scan_box_dimension_assignment(processor, index, dimension, global, phase, suspended),
        PendingOperationScanPhase::Glue {
            index,
            global,
            mu,
            phase,
        } => scan_glue_register_assignment(processor, index, global, mu, phase, suspended),
        PendingOperationScanPhase::Unary {
            meaning,
            global,
            origin,
            phase,
        } => scan_unary_scalar_operation(processor, meaning, global, origin, phase, suspended),
        PendingOperationScanPhase::ParagraphShape { global, phase } => {
            scan_paragraph_shape_assignment(processor, global, phase, suspended)
        }
        PendingOperationScanPhase::PenaltyArray {
            kind,
            global,
            phase,
        } => scan_penalty_array_assignment(processor, kind, global, phase, suspended),
        PendingOperationScanPhase::FontDimen(phase) => {
            scan_font_dimen_assignment(processor, phase, suspended)
        }
        PendingOperationScanPhase::FontInteger { primitive, phase } => {
            scan_font_integer_assignment(processor, primitive, phase, suspended)
        }
        PendingOperationScanPhase::CodeTable {
            primitive,
            global,
            phase,
        } => scan_code_table_assignment(processor, primitive, global, phase, suspended),
        PendingOperationScanPhase::PdfFontCode { primitive, phase } => {
            scan_pdf_font_code_assignment(processor, primitive, phase, suspended)
        }
        PendingOperationScanPhase::PdfFontExpand(phase) => {
            scan_pdf_font_expand_assignment(processor, phase, suspended)
        }
        PendingOperationScanPhase::FontOnly { meaning } => {
            scan_font_only_operation(processor, meaning, suspended)
        }
        PendingOperationScanPhase::OpenOut(phase) => {
            scan_open_out_operation(processor, phase, suspended)
        }
        PendingOperationScanPhase::Marks(phase) => {
            scan_marks_operation(processor, phase, suspended)
        }
        PendingOperationScanPhase::CatCode { global, phase } => {
            return hot_apply::scan_catcode_assignment(processor, global, phase, suspended)
                .map(ScannedOperation::Hot);
        }
        PendingOperationScanPhase::MathFamily {
            size,
            global,
            phase,
        } => scan_math_family_assignment(processor, size, global, phase, suspended),
        PendingOperationScanPhase::Arithmetic {
            primitive,
            global,
            phase,
        } => scan_arithmetic_assignment(processor, primitive, global, phase, suspended),
        PendingOperationScanPhase::LeaderGlue { mode, result } => {
            return scan_retained_leader_glue(processor, mode, result, suspended).map(Into::into);
        }
        PendingOperationScanPhase::LeaderPayload { primitive, mode } => {
            scan_leaders_step(processor, primitive, mode, suspended)
        }
        PendingOperationScanPhase::LeaderCommand { mode, result } => {
            return scan_retained_leader_command(processor, mode, result, suspended)
                .map(Into::into);
        }
    }?;
    Ok(cold.into())
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
    suspended_operation_scan: &mut Option<PendingOperationScanPhase>,
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
        suspended_operation_scan,
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
    command: &mut PreflightCommand<G>,
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
            command.current().meaning(),
            ResolvedMeaning::Static(
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                } | Meaning::Relax
            )
        )
    {
        let mut destination = None;
        if next_non_blank_non_relax_x_token_into(processor, &mut destination)
            .map_err(command_error)?
            != tex_command::DeliveryStatus::Command
        {
            return Err(ExecError::MissingToken {
                context: "leader glue",
            });
        }
        command.replace_current(
            destination
                .take()
                .expect("command status initializes destination"),
        );
    }
    let origin = command.current().origin();
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
        None,
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
    command: &mut PreflightCommand<G>,
    mode: Mode,
    boxes: &ReplayBoxes<G>,
    innermost_group: Option<GroupKind>,
    job_is_all_over: bool,
    display_eq_no: bool,
    shown_mode: &mut Option<Mode>,
    diagnostics: &mut Vec<PendingDiagnostic<G>>,
    alignment: Option<AlignmentIdentity>,
    set_box_allowed: bool,
    mut initial_prefix: Option<(bool, MeaningFlags)>,
) -> Result<ScannedOperation<G>, ExecError> {
    // TeX82 §1078 fetches the command following a completed leader payload
    // inside `box_end`, before control returns to §1030's `big_switch` or
    // §1211's prefix loop. Split replay finishes the box in one step and
    // delivers that command in the next, so classify it at this same outer
    // boundary. In particular, a non-glue `\global` is the command that
    // `back_error` must restore; allowing it into the prefix loop first would
    // consume and restore the following assignment instead.
    if let Some((kind, payload)) = boxes.pending_leader.as_ref() {
        let result = LeaderGlueResult::Payload {
            kind: *kind,
            payload: *payload,
        };
        let mut suspended = None;
        let scanned = scan_leader_glue_command(
            processor,
            &mut command.command,
            mode,
            result,
            &mut suspended,
        );
        if let Err(error) = &scanned
            && execution_error_needs_command_retry(error)
            && let Some(phase) = suspended
        {
            let child = processor
                .take_scanner_resume()
                .expect("a suspended leader glue scan retains its exact child capability");
            command.retain_operation_scan(processor.delivery_cursor(), phase, child);
        }
        let Some(operation) = scanned? else {
            return Ok(ColdOperation::<G>::LeadersNotFollowedByGlue.into());
        };
        return Ok(operation.into());
    }
    // §1030's `reswitch:` label sits *above* the big case, not at the fetch:
    // a case that has already fetched its own replacement command dispatches
    // that command in place. `goto reswitch` is therefore not `back_input`,
    // and a case using it pushes no input level and delivers nothing twice.
    // This loop is that label.
    let mut suppress_left_boundary = false;
    loop {
        let (mut global, mut flags) = initial_prefix
            .take()
            .unwrap_or((false, MeaningFlags::EMPTY));
        loop {
            let retained_global = global;
            let retained_flags = flags;
            #[cfg(feature = "profiling")]
            {
                tex_state::measurement::record_hot_core_command_family(hot_core_command_family(
                    command.current().meaning(),
                ));
                if let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(primitive)) =
                    command.current().meaning()
                {
                    tex_state::measurement::record_hot_core_unexpandable_opcode(
                        usize::try_from(primitive.operand())
                            .expect("unexpandable primitive operand fits usize"),
                    );
                }
            }
            match command.current().meaning() {
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
            let mut destination = None;
            let next = match next_non_blank_non_relax_x_token_into(processor, &mut destination) {
                Ok(tex_command::DeliveryStatus::Command) => destination
                    .take()
                    .expect("command status initializes destination"),
                Ok(tex_command::DeliveryStatus::End) => {
                    return Err(ExecError::MissingPrefixedCommand);
                }
                Ok(_) => unreachable!("ordinary expanded delivery returns only commands"),
                Err(error) => {
                    let error = command_error(error);
                    if execution_error_needs_command_retry(&error) {
                        let child = processor
                            .take_scanner_resume()
                            .expect("a suspended prefix fetch retains its exact expansion child");
                        command.phase = PreflightCommandPhase::PrefixScan {
                            global: retained_global,
                            flags: retained_flags,
                            alignment,
                            set_box_allowed,
                        };
                        command.retain_scanner(processor.delivery_cursor(), Some(child));
                    }
                    return Err(error);
                }
            };
            command.replace_current(next);
            // §1211's `if cur_cmd<=max_non_prefixed_command then <Discard
            // erroneous prefixes and return>`: §209's partition, not a
            // hand-listed set of assignment families.
            if !tex_command::exceeds_max_non_prefixed_command(static_meaning(
                command.current().meaning(),
            )) {
                let printed = tex_command::PrintCommand::from_current(command.current());
                // §1212's `back_error`: the substantive command is retained
                // and re-delivered without the discarded prefixes.
                processor
                    .back_input(command.take_current())
                    .map_err(command_error)?;
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
                command.current().meaning(),
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
                tex_command::PrintCommand::from_current(command.current()),
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
            command.current().meaning(),
            ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::IgnoreSpaces
            ))
        ) {
            let next = if let Some(alignment) = alignment {
                let mut destination = None;
                loop {
                    match processor
                        .get_x_alignment_delivery_into(false, &mut destination)
                        .map_err(command_error)?
                    {
                        tex_command::DeliveryStatus::End => {
                            return Ok(ColdOperation::<G>::EndOfInput.into());
                        }
                        tex_command::DeliveryStatus::ReplayCompleted(episode) => {
                            return Ok(ColdOperation::<G>::ReplayCompleted(episode).into());
                        }
                        tex_command::DeliveryStatus::AlignmentEndTemplate => {
                            let event = tex_command::AlignmentDeliveryEvent::EndTemplate(
                                destination
                                    .take()
                                    .expect("alignment status initializes destination"),
                            );
                            return scan_alignment_delivery_event(processor, alignment, event)
                                .map(Into::into);
                        }
                        tex_command::DeliveryStatus::AlignmentClosingBrace => {
                            let event = tex_command::AlignmentDeliveryEvent::ClosingBrace(
                                destination
                                    .take()
                                    .expect("alignment status initializes destination"),
                            );
                            return scan_alignment_delivery_event(processor, alignment, event)
                                .map(Into::into);
                        }
                        tex_command::DeliveryStatus::Command
                            if matches!(
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
                            destination = None
                        }
                        tex_command::DeliveryStatus::Command => {
                            break destination
                                .take()
                                .expect("command status initializes destination");
                        }
                        tex_command::DeliveryStatus::PendingExpanded => {
                            unreachable!("alignment delivery commits terminal observations");
                        }
                    }
                }
            } else {
                let mut destination = None;
                if next_non_blank_x_token_into(processor, &mut destination)
                    .map_err(command_error)?
                    != tex_command::DeliveryStatus::Command
                {
                    return Ok(ColdOperation::<G>::EndOfInput.into());
                }
                destination
                    .take()
                    .expect("command status initializes destination")
            };
            command.replace_current(next);
            report_command_trace(processor, mode, command.current(), shown_mode);
            continue;
        }
        if matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal)
            && matches!(
                command.current().meaning(),
                ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::NoBoundary
                ))
            )
        {
            let mut destination = None;
            if processor
                .get_x_token_into(&mut destination)
                .map_err(command_error)?
                != tex_command::DeliveryStatus::Command
            {
                return Ok(ColdOperation::<G>::Continue.into());
            }
            let next = destination
                .take()
                .expect("command status initializes destination");
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
            command.replace_current(next);
            report_command_trace(processor, mode, command.current(), shown_mode);
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
                    command.current().meaning(),
                    ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                        UnexpandablePrimitive::Gdef | UnexpandablePrimitive::Xdef
                    ))
                ),
        );
        command.phase = PreflightCommandPhase::Settled;
        command.operation_scan = None;
        let mut suspended_operation_scan = None;
        let scanned_result = scan_command(
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
            &mut suspended_operation_scan,
        );
        if let Err(error) = &scanned_result
            && execution_error_needs_command_retry(error)
        {
            let child = processor
                .take_scanner_resume()
                .expect("a suspended substantive command retains its exact scanner capability");
            if let Some(phase) = suspended_operation_scan {
                command.retain_operation_scan(processor.delivery_cursor(), phase, child);
            } else {
                command.phase = PreflightCommandPhase::PrefixedCommandScan {
                    global,
                    flags,
                    set_box_allowed,
                };
                command.retain_scanner(processor.delivery_cursor(), Some(child));
            }
        }
        let mut scanned = scanned_result?;
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
    let shipout_box_constructor = boxes.pending_shipout.is_some()
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
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<ColdOperation<G>, ExecError> {
    let kind = crate::box_runtime::leader_glue_kind(primitive);
    *suspended = Some(PendingOperationScanPhase::LeaderPayload { primitive, mode });
    let payload = processor.scan_leader_payload().map_err(command_error)?;
    *suspended = None;
    match payload {
        ScannedLeaderPayload::Missing => Ok(ColdOperation::<G>::MissingLeaderPayload),
        ScannedLeaderPayload::Construction(construction) => {
            Ok(ColdOperation::<G>::BeginLeaderBox { construction, kind })
        }
        ScannedLeaderPayload::Rule(rule) => {
            let payload = LeaderPayload::Rule {
                width: rule.width,
                height: rule.height,
                depth: rule.depth,
            };
            let result = LeaderGlueResult::Payload { kind, payload };
            *suspended = Some(PendingOperationScanPhase::LeaderCommand { mode, result });
            let mut destination = None;
            if next_non_blank_non_relax_x_token_into(processor, &mut destination)
                .map_err(command_error)?
                != tex_command::DeliveryStatus::Command
            {
                return Err(ExecError::MissingToken {
                    context: "leader glue",
                });
            }
            let mut glue_command = destination;
            *suspended = None;
            let Some(operation) =
                scan_leader_glue_command(processor, &mut glue_command, mode, result, suspended)?
            else {
                return Ok(ColdOperation::<G>::LeadersNotFollowedByGlue);
            };
            Ok(operation)
        }
        // Register payloads must retain their destructive/copy ownership at
        // replay time.  Keep the command scanner's completed glue read, then
        // use the regular typed box read path to obtain the node.
        ScannedLeaderPayload::BoxRegister { index, copy } => {
            let result = LeaderGlueResult::Register { kind, index, copy };
            *suspended = Some(PendingOperationScanPhase::LeaderCommand { mode, result });
            let mut destination = None;
            if next_non_blank_non_relax_x_token_into(processor, &mut destination)
                .map_err(command_error)?
                != tex_command::DeliveryStatus::Command
            {
                return Err(ExecError::MissingToken {
                    context: "leader glue",
                });
            }
            let mut glue_command = destination;
            *suspended = None;
            let Some(operation) =
                scan_leader_glue_command(processor, &mut glue_command, mode, result, suspended)?
            else {
                return Ok(ColdOperation::<G>::LeadersNotFollowedByGlue);
            };
            Ok(operation)
        }
    }
}

fn scan_retained_leader_command<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    mode: Mode,
    result: LeaderGlueResult,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<ColdOperation<G>, ExecError> {
    *suspended = Some(PendingOperationScanPhase::LeaderCommand { mode, result });
    let mut destination = None;
    if next_non_blank_non_relax_x_token_into(processor, &mut destination).map_err(command_error)?
        != tex_command::DeliveryStatus::Command
    {
        return Err(ExecError::MissingToken {
            context: "leader glue",
        });
    }
    let mut glue_command = destination;
    *suspended = None;
    scan_leader_glue_command(processor, &mut glue_command, mode, result, suspended)?.ok_or(
        ExecError::MissingToken {
            context: "leader glue command",
        },
    )
}

fn scan_leader_glue_command<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    command: &mut Option<tex_command::CurrentCommand<G>>,
    mode: Mode,
    result: LeaderGlueResult,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<Option<ColdOperation<G>>, ExecError> {
    let horizontal = matches!(
        mode,
        Mode::Horizontal | Mode::RestrictedHorizontal | Mode::Math | Mode::DisplayMath
    );
    let primitive = match command
        .as_ref()
        .expect("leader glue scanner owns its current command")
        .meaning()
    {
        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(primitive)) => primitive,
        _ => {
            processor
                .back_input(
                    command
                        .take()
                        .expect("leader glue scanner owns its current command"),
                )
                .map_err(command_error)?;
            return Ok(None);
        }
    };
    if (horizontal && primitive == UnexpandablePrimitive::HSkip)
        || (!horizontal && primitive == UnexpandablePrimitive::VSkip)
    {
        let scan = processor.scan_glue_retained(false);
        let glue = retain_operation_scalar(
            processor,
            scan,
            PendingOperationScanPhase::LeaderGlue { mode, result },
            suspended,
        )?
        .value;
        return Ok(Some(complete_leader_glue(result, glue)));
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
        processor
            .back_input(
                command
                    .take()
                    .expect("leader glue scanner owns its current command"),
            )
            .map_err(command_error)?;
        return Ok(None);
    };
    let unit = Scaled::from_raw(if negative {
        -Scaled::UNITY
    } else {
        Scaled::UNITY
    });
    let zero = Scaled::from_raw(0);
    let glue = if shrink {
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
    };
    Ok(Some(complete_leader_glue(result, glue)))
}

fn scan_retained_leader_glue<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    mode: Mode,
    result: LeaderGlueResult,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<ColdOperation<G>, ExecError> {
    let scan = processor.scan_glue_retained(false);
    let glue = retain_operation_scalar(
        processor,
        scan,
        PendingOperationScanPhase::LeaderGlue { mode, result },
        suspended,
    )?
    .value;
    Ok(complete_leader_glue(result, glue))
}

fn complete_leader_glue<G>(result: LeaderGlueResult, glue: GlueSpec) -> ColdOperation<G> {
    match result {
        LeaderGlueResult::Payload { kind, payload } => ColdOperation::Leaders {
            kind,
            payload,
            glue,
        },
        LeaderGlueResult::Register { kind, index, copy } => ColdOperation::LeaderRegister {
            kind,
            index,
            copy,
            glue,
        },
    }
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
    command: &mut PreflightCommand<G>,
    global: bool,
    flags: MeaningFlags,
    mode: Mode,
    boxes: &ReplayBoxes<G>,
    innermost_group: Option<GroupKind>,
    job_is_all_over: bool,
    display_eq_no: bool,
    set_box_allowed: bool,
    shown_mode: &mut Option<Mode>,
    suspended_operation_scan: &mut Option<PendingOperationScanPhase>,
) -> Result<ScannedOperation<G>, ExecError> {
    if let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
        primitive @ (UnexpandablePrimitive::TextFont
        | UnexpandablePrimitive::ScriptFont
        | UnexpandablePrimitive::ScriptScriptFont),
    )) = command.meaning()
    {
        let size = tex_command::MathFamilySize::of_primitive(primitive)
            .expect("the outer match restricts this to `def_family`");
        return scan_math_family_assignment(
            processor,
            size,
            global,
            MathFamilyScanPhase::Family,
            suspended_operation_scan,
        )
        .map(Into::into);
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
            .scan_math_request(command)
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
        processor
            .back_input(command.take_current())
            .map_err(command_error)?;
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
                .insert_partoken_before(command.take_current())
                .map_err(command_error)?;
            return Ok(ColdOperation::<G>::Continue.into());
        }
        return Ok(ColdOperation::<G>::BoxEndGroup {
            ships_out: box_state.shipout_region.is_some(),
            current_line: i32::try_from(processor.current_file_line_number()).unwrap_or(i32::MAX),
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
                .insert_partoken_before(command.take_current())
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
        processor
            .back_input(command.take_current())
            .map_err(command_error)?;
        return Ok(ColdOperation::<G>::ParagraphStart.into());
    }
    if let Some(operation) = hot_apply::scan(
        processor,
        command,
        global,
        flags,
        innermost_group,
        suspended_operation_scan,
    )? {
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
        suspended_operation_scan,
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
                let Some(Node::Disc {
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
                let Some(Node::Disc { pre, .. }) = current_list.nodes(context).last() else {
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
