//! Production main-control driver.
//!
//! This intentionally owns only typed execution mutations. Raw delivery,
//! expansion, macro calls, input nesting, and operand collection remain in
//! `tex-command`; no independent source stack is accepted here.

use std::path::PathBuf;
use std::sync::Arc;
use tex_command::{
    AlignmentCellDelimiter, AlignmentCellOpening, AlignmentIdentity, AlignmentRequest,
    CommandError, CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandProfile,
    CommandState, FatalError, FontLoadRequest, FontResource, GeneratedFontKind, MathFieldBody,
    MathFractionKind, MathScriptKind, PdfImageRequest, PdfImageResource,
    PreparedAlignmentCellTemplates, RegisteredSourceKind, ResourceFulfillment,
    ResourceInstallOutcome, ResourceNeed, ResourceProvider, RestrictedIntegerClass,
    ScannedGeneratedFontDefinition, ScannedPackingSpec, SourceRegistration,
    SourceRegistrationError,
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
    GroupKind, ObservedDependency, ParagraphShapeLine, PenaltyArrayKind, PrintSink, StreamSlot,
    TrackedRegionBarrier, Universe,
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
use crate::mode::{AlignColumn, AlignState, AlignmentKind, AlignmentPackSpec};
use crate::vertical::is_outer_vertical;
use crate::{ExecError, Mode, ModeNest};

mod code_table;
mod cold;
mod definition_assignments;
mod delivery;
mod execution_state;
mod executor_facts;
use execution_state::ExecutionState;
mod host_boundaries;
mod interpreter;
mod observations;
mod root_source;
mod semantic;

use code_table::recover_code_table_value;

use cold::*;
use delivery::*;
use executor_facts::ExecutorHostFacts;
use host_boundaries::*;

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
    /// Each [`CommandProcessor`] exclusively borrows this actual state and
    /// the matching admitted context; native semantics use those same loans.
    command: CommandState<G>,
    fuel: tex_command::CommandFuelLedger,
    capabilities: CommandHostCapabilities,
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
    /// One call-scoped provider decline, kept beside its canonical need until
    /// the outer step driver consumes the suspension sideband. It is never
    /// checkpointed, compared for resource identity, or retained across the
    /// next drive.
    declined_resource_attempt: Option<ResourceNeed>,
    execution: ExecutionState<G>,
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
    fn construction_group(self, appended: bool, mode: Mode) -> GroupKind {
        if self == Self::HBox && appended && matches!(mode, Mode::Vertical | Mode::InternalVertical)
        {
            GroupKind::AdjustedHBox
        } else {
            self.group_kind()
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
    /// TeX82 §645's `scan_spec` result, kept from `init_align` until §805
    /// packages the preamble prototype box with it.
    packing: AlignmentPackSpec,
    columns: Vec<PreparedAlignmentCellTemplates<G>>,
    repeat_start: Option<usize>,
    column: usize,
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
        self.pending_leader.is_none()
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
    parts: [tex_state::page_node_arena::PageListSpan; 3],
    completed: usize,
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
    state: &'a mut CommandState<G>,
    fuel: &'a mut tex_command::CommandFuel,
    capabilities: &'a mut CommandHostCapabilities,
    resource_provider: Option<&'provider mut dyn ResourceProvider<G>>,
    declined_resource_attempt: Option<&'a mut Option<ResourceNeed>>,
    host_facts: ExecutorHostFacts<'a, G>,
    observations: Option<&'a mut dyn CommandObserver>,
    diagnostic_effects: &'a mut DiagnosticEffects,
    emit_dvi_override: Option<bool>,
}

fn reborrow_observer<'loan>(
    observer: &'loan mut Option<&mut dyn CommandObserver>,
) -> Option<&'loan mut dyn CommandObserver> {
    observer
        .as_mut()
        .map(|observer| &mut **observer as &mut dyn CommandObserver)
}

impl<G> CommandMachine<'_, '_, G> {
    fn with_processor<R>(
        &mut self,
        context: &mut tex_state::CommandContext<'_, G>,
        use_processor: impl FnOnce(
            &mut CommandProcessor<'_, '_, G, &mut dyn tex_command::CommandHostFacts<G>>,
        ) -> R,
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
            ..
        } = self;
        let observer = reborrow_observer(observations);
        let host = match resource_provider.as_deref_mut() {
            Some(provider) => CommandHostContext::with_facts_and_resource_provider(
                capabilities,
                host_facts as &mut dyn tex_command::CommandHostFacts<G>,
                provider,
                declined_resource_attempt
                    .as_deref_mut()
                    .expect("provider owns its decline loan"),
            ),
            None => CommandHostContext::with_facts(
                capabilities,
                host_facts as &mut dyn tex_command::CommandHostFacts<G>,
            ),
        };
        let mut processor =
            CommandProcessor::new(state, context, host, fuel, observer, diagnostic_effects);
        use_processor(&mut processor)
    }

    fn with_processor_and_diagnostics<R>(
        &mut self,
        context: &mut tex_state::CommandContext<'_, G>,
        diagnostic_effects: &mut DiagnosticEffects,
        use_processor: impl FnOnce(
            &mut CommandProcessor<'_, '_, G, &mut dyn tex_command::CommandHostFacts<G>>,
        ) -> R,
    ) -> R {
        let Self {
            state,
            fuel,
            capabilities,
            resource_provider,
            declined_resource_attempt,
            host_facts,
            observations,
            ..
        } = self;
        let observer = reborrow_observer(observations);
        let host = match resource_provider.as_deref_mut() {
            Some(provider) => CommandHostContext::with_facts_and_resource_provider(
                capabilities,
                host_facts as &mut dyn tex_command::CommandHostFacts<G>,
                provider,
                declined_resource_attempt
                    .as_deref_mut()
                    .expect("provider owns its decline loan"),
            ),
            None => CommandHostContext::with_facts(
                capabilities,
                host_facts as &mut dyn tex_command::CommandHostFacts<G>,
            ),
        };
        let mut processor =
            CommandProcessor::new(state, context, host, fuel, observer, diagnostic_effects);
        use_processor(&mut processor)
    }

    fn shipout_geometry_sink(&mut self) -> MainControlShipoutGeometrySink<'_, G> {
        MainControlShipoutGeometrySink {
            command: self.state,
            observations: reborrow_observer(&mut self.observations),
        }
    }
}

fn command_processor<'episode, 'admission, G, Facts: tex_command::CommandHostFacts<G> + ?Sized>(
    command: &'episode mut CommandState<G>,
    fuel: &'episode mut tex_command::CommandFuel,
    capabilities: &'episode mut CommandHostCapabilities,
    host_facts: &'episode mut Facts,
    observations: &'episode mut ObservationSlot,
    diagnostic_effects: &'episode mut DiagnosticEffects,
    stores: &'episode mut CommandContext<'admission, G>,
) -> CommandProcessor<'episode, 'admission, G, &'episode mut Facts> {
    let observer = observations
        .as_mut()
        .map(|buffer| buffer as &mut dyn CommandObserver);
    CommandProcessor::new(
        command,
        stores,
        CommandHostContext::with_facts(capabilities, host_facts),
        fuel,
        observer,
        diagnostic_effects,
    )
}

#[allow(clippy::too_many_arguments)] // each argument is a distinct replay owner
fn command_processor_with_resource_provider<
    'episode,
    'admission,
    G,
    Facts: tex_command::CommandHostFacts<G> + ?Sized,
>(
    command: &'episode mut CommandState<G>,
    fuel: &'episode mut tex_command::CommandFuel,
    capabilities: &'episode mut CommandHostCapabilities,
    host_facts: &'episode mut Facts,
    resource_provider: &'episode mut dyn ResourceProvider<G>,
    declined_resource_attempt: &'episode mut Option<ResourceNeed>,
    observations: &'episode mut ObservationSlot,
    diagnostic_effects: &'episode mut DiagnosticEffects,
    stores: &'episode mut CommandContext<'admission, G>,
) -> CommandProcessor<'episode, 'admission, G, &'episode mut Facts> {
    let observer = observations
        .as_mut()
        .map(|buffer| buffer as &mut dyn CommandObserver);
    CommandProcessor::new(
        command,
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
    command: &mut CommandState<G>,
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
        Self::with_profile(CommandProfile::default())
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
            self.execution
                .modes
                .preflight_page_region_succession(&context)
                .ok_or(tex_state::UniverseError::State(
                    tex_state::StateError::InvalidCursor,
                ))?
        };
        stores.prepare_page_region_after_output(modes)
    }

    fn finish_pending_page_region_succession(&mut self, stores: &mut Universe<G>) {
        if !self.execution.page_region_succession_ready() {
            return;
        }
        self.prepare_page_region_succession(stores)
            .expect("rootless mode lists admit the current page region");
        stores
            .commit_page_region_after_output()
            .expect("prepared page succession commits");
        self.execution.page_region_succession_pending = false;
    }

    fn from_owners(command: CommandState<G>, modes: ModeNest) -> Self {
        Self {
            command,
            fuel: tex_command::CommandFuelLedger::default(),
            capabilities: CommandHostCapabilities::default(),
            operation_observations: None,
            declined_resource_attempt: None,
            execution: ExecutionState::with_modes(modes),
        }
    }

    pub(crate) fn from_checkpoint_fork(command: CommandState<G>, modes: ModeNest) -> Self {
        let mut control = Self::from_owners(command, modes);
        control.execution.job_start_eligibility = None;
        control
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
        let Self {
            command,
            execution: ExecutionState { modes, .. },
            ..
        } = self;
        (command, PreparedCheckpointControl { modes })
    }

    /// Parks a quiescent command owner from an independently materialized
    /// JobStart lane. No mode or command fork exists in this case, so there is
    /// deliberately no settlement receipt: retained checkpoints already own
    /// their rootless mode summaries and the live terminal mode nest can drop.
    pub(crate) fn into_independent_parked_command(self) -> CommandState<G> {
        debug_assert!(self.command.named_boundary_is_quiescent());
        self.command
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
        self.execution.modes.accept_checkpoint_candidate_in_place();
        self.command.accept_checkpoint_candidate();
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
            && self.execution.active_alignment.is_none()
            && self.operation_observations.is_none()
            && self.execution.operation_receipt_start.is_none()
            && self.execution.prepared_shipout.is_none()
            && self.execution.prepared_pdf_form.is_none()
            && self.execution.pending_job_end.is_none()
            && !self.execution.page_region_succession_pending
            && self.execution.pending_resource_site.is_none()
            && !self.execution.halted
            && !self.execution.boxes.output_routine_active
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
        if self.execution.ended && self.execution.fatal.is_some() && step == MainControlStep::End {
            true
        } else {
            self.terminal_is_quiescent(stores)
        }
    }

    pub(crate) fn mark_ended(&mut self) {
        self.execution.ended = true;
    }

    /// Replaces the execution-owned memo service between candidate runs.
    pub fn install_pure_memo_runtime(&mut self, runtime: tex_state::PureMemoRuntime) {
        self.execution.pure_memo = Arc::new(std::sync::Mutex::new(runtime));
        self.execution.pure_memo_initialized = true;
    }

    /// Returns the execution-owned memo service to its session owner.
    pub fn take_pure_memo_runtime(&mut self) -> tex_state::PureMemoRuntime {
        let runtime = std::mem::take(&mut self.execution.pure_memo);
        self.execution.pure_memo_initialized = false;
        Arc::try_unwrap(runtime)
            .expect("Universe<G> retains only a weak memo capability")
            .into_inner()
            .expect("memo runtime mutex is not poisoned")
    }

    #[must_use]
    pub fn pure_memo_stats(&self) -> tex_state::PureMemoStats {
        self.execution
            .pure_memo
            .lock()
            .expect("memo runtime mutex is not poisoned")
            .stats()
    }

    /// Attaches this execution-owned service before candidate setup records
    /// carried history outside the command step loop.
    pub fn attach_pure_memo_capability(&self, stores: &mut Universe<G>) {
        stores.attach_pure_memo_capability(&self.execution.pure_memo);
    }

    /// Reports direct-operation accounting without exposing the mode journal
    /// or any rollback capability. The savepoint fields remain schema-stable
    /// profiling counters and stay zero on the canonical path.
    #[must_use]
    pub const fn advance_telemetry(&self) -> AdvanceTelemetry {
        self.execution.advance_telemetry
    }

    /// Reports why bounded semantic episodes returned to their live session.
    #[must_use]
    pub const fn episode_telemetry(&self) -> crate::EpisodeTelemetry {
        self.execution.episode_telemetry
    }

    pub(crate) fn record_external_episode_barrier(
        &mut self,
        barrier: crate::SemanticEpisodeBarrier,
    ) {
        self.execution
            .episode_telemetry
            .record_semantic_barrier(barrier);
    }

    /// Samples host cancellation before mutating command, mode, or Universe<G>
    /// state.
    pub fn advance_when(
        &mut self,
        stores: &mut Universe<G>,
        readiness: AdvanceReadiness,
    ) -> Result<AdvanceOutcome, ExecError> {
        if readiness == AdvanceReadiness::Cancelled {
            self.execution
                .episode_telemetry
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
            modes: &self.execution.modes,
            shown_mode: &mut self.execution.shown_mode,
            output_routine_active: &self.execution.boxes.output_routine_active,
            pdf_ignore_depth: self.execution.pdf_ignore_depth,
            telemetry: &mut self.execution.episode_telemetry,
        };
        let processor = command_processor(
            &mut self.command,
            self.fuel.fuel_mut(),
            &mut self.capabilities,
            &mut host_facts as &mut dyn tex_command::CommandHostFacts<G>,
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
        Self::from_owners(CommandState::new(profile), ModeNest::default())
    }

    /// Creates INITEX command state for a profile whose primitive meanings
    /// the composed driver has already installed in `stores`.
    #[must_use]
    pub fn prepared_initex(profile: CommandProfile) -> Self {
        let mut control = Self::with_profile(profile);
        control.execution.next_alignment_identity = 1;
        control.execution.initex = true;
        control
    }

    /// Creates a fresh TeX82 INITEX environment.
    ///
    /// The primitive definitions are installed from the engine's static TeX82
    /// registries, before any fixture or host source is registered.
    #[must_use]
    pub fn tex82_initex(stores: &mut Universe<G>) -> Self {
        tex_command::install_tex82_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        let mut control = Self::prepared_initex(CommandProfile::TEX82);
        control.bind_primitive_handles(stores);
        control
    }

    /// Binds process-local accelerators after INITEX installation or format
    /// registry reconstruction has installed the complete driver profile.
    pub fn bind_primitive_handles(&mut self, stores: &Universe<G>) {
        self.execution.primitive_registry_len = Some(stores.primitive_registry_len());
        self.execution.pdf_ignore_depth = stores.primitive_handle("pdfignoreddimen");
    }

    fn ensure_primitive_handles(&mut self, stores: &Universe<G>) {
        if self.execution.primitive_registry_len != Some(stores.primitive_registry_len()) {
            self.bind_primitive_handles(stores);
        }
    }

    /// Borrows command state for source registration and snapshots.
    #[must_use]
    pub fn command_mut(&mut self) -> &mut CommandState<G> {
        &mut self.command
    }

    /// Returns the number of live TeX input levels.
    #[must_use]
    pub fn input_level_count(&self) -> usize {
        self.command.input_level_count()
    }

    /// Returns the immutable profile of this command processor.
    #[must_use]
    pub fn command_profile(&self) -> CommandProfile {
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
        self.execution.emit_dvi_override = Some(enabled);
    }

    /// Reports whether this job terminated through an effective INITEX
    /// `\dump`, so the host can publish the same `RunResult` contract as the
    /// retired executor without inspecting command delivery.
    #[must_use]
    pub const fn dumped_format(&self) -> bool {
        self.execution.dumped_format.is_some()
    }

    /// Returns §1328's engine-owned identity after a successful INITEX dump.
    #[must_use]
    pub fn format_dump_receipt(&self) -> Option<&crate::job::FormatDumpReceipt> {
        self.execution.dumped_format.as_ref()
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
        if self.execution.dumped_format.is_none() {
            return Ok(None);
        }
        if !self.command.format_dump_is_quiescent() {
            return Err(crate::FormatDumpError::LiveCommandState);
        }
        if self.execution.active_alignment.is_some()
            || !self.execution.boxes.format_dump_is_quiescent()
            || !self.execution.active_discretionaries.is_empty()
            || !self.execution.active_math_choices.is_empty()
            || !self.execution.active_math_fields.is_empty()
            || !self.execution.active_math_left_boundaries.is_empty()
            || !self.execution.active_math_shifts.is_empty()
            || self.execution.main_loop_active
            || self.execution.set_box_forbidden_depth != 0
            || self.operation_observations.is_some()
            || self.execution.operation_receipt_start.is_some()
            || self.execution.prepared_shipout.is_some()
            || self.execution.prepared_pdf_form.is_some()
            || self.execution.pending_job_end.is_some()
            || self.execution.page_region_succession_pending
            || !self.execution.prepared_dvi_pages.is_empty()
            || self.execution.pending_resource_site.is_some()
        {
            return Err(crate::FormatDumpError::LiveExecutorState);
        }
        if self.execution.modes.depth() != 1
            || self.execution.modes.current_mode() != Mode::Vertical
            || !self.execution.modes.current_list().is_empty()
        {
            return Err(crate::FormatDumpError::LiveModeState);
        }
        let image = stores
            .capture_format_image()
            .map_err(crate::FormatDumpError::State)?;
        self.command.close_format_dump_boundary();
        let receipt = self
            .execution
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
            &mut self.execution.modes,
            stores,
            budget_counters,
        );
        if result.is_err() {
            self.execution.job_start_eligibility =
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
        if self.execution.halted {
            return Err(tex_command::CommandSummaryError::AttemptSuspended);
        }
        crate::EngineCheckpoint::capture_checkpoint_with_identity_demand(
            eligibility,
            &mut self.command,
            &mut self.execution.modes,
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
        let prepared = checkpoint.prepare_state_after_replay(
            &self.command,
            &self.execution.modes,
            stores,
            output,
        )?;
        // No selected component can fail now. Drop interrupted executor
        // owners while their generation/page/PDF backing remains attached;
        // neither aftergroup tokens nor ordinary unsave diagnostics run.
        self.reset_checkpoint_replay_runtime(checkpoint.boundary());
        checkpoint.apply_prepared_state_after_replay(
            &mut self.command,
            &mut self.execution.modes,
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
        checkpoint.restore_semantic_state(&mut self.command, &mut self.execution.modes, stores)?;
        self.execution.active_alignment = None;
        self.execution.boxes = ReplayBoxes::default();
        self.execution.paragraph_checkpoint_demand = None;
        self.execution.paragraph_checkpoint_cut = false;
        self.execution.error_operation_committed = false;
        self.execution.fatal = None;
        self.execution.captured_fatal_origin = None;
        Ok(())
    }

    /// Clears interrupted executor owners before destructive aggregate restore. These fields are deliberately not
    /// part of `EngineCheckpoint`: a fork constructs them from defaults, and
    /// an in-place resource replay must establish the same quiescent shape.
    pub(crate) fn reset_checkpoint_replay_runtime(&mut self, boundary: crate::EngineBoundary) {
        let root_main_source = (boundary != crate::EngineBoundary::JobStart)
            .then_some(self.execution.root_main_source)
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
        self.execution.active_alignment = None;
        self.execution.boxes = ReplayBoxes::default();
        self.execution.active_discretionaries.clear();
        self.execution.active_math_choices.clear();
        self.execution.active_math_fields.clear();
        self.execution.active_math_left_boundaries.clear();
        self.execution.active_math_shifts.clear();
        self.execution.skip_pointer_sources.clear();
        self.execution.muskip_pointer_sources.clear();
        self.execution.main_loop_active = false;
        self.execution.set_box_forbidden_depth = 0;
        self.execution.shown_mode = None;
        self.execution.main_control_entered = boundary != crate::EngineBoundary::JobStart;
        self.execution.startup_terminal_line.clear();
        self.execution.terminal_line_was_empty = None;
        self.execution.root_main_source = root_main_source;
        self.operation_observations = None;
        self.execution.operation_receipt_start = None;
        self.execution.error_operation_committed = false;
        self.execution.prepared_dvi_pages = PreparedDviPages::default();
        self.execution.prepared_shipout = None;
        self.execution.prepared_pdf_form = None;
        self.execution.pending_job_end = None;
        self.execution.page_region_succession_pending = false;
        self.execution.paragraph_checkpoint_demand = None;
        self.execution.paragraph_checkpoint_cut = false;
        self.execution.pending_resource_site = None;
        self.declined_resource_attempt = None;
        self.execution.halted = false;
        self.execution.ended = false;
        self.execution.fatal = None;
        self.execution.captured_fatal_origin = None;
        self.execution.first_causal_context = None;
        self.execution.first_recoverable_diagnostic = None;
        self.execution.job = crate::job::JobFraming::default();
        self.execution.job_body_effect_end = None;
        self.execution.pdf_navigation_finalized = false;
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
        let binary = self.execution.engine_binary.unwrap_or_else(|| {
            crate::job::EngineBinaryIdentity::for_profile(self.command_profile())
        });
        stores.set_engine_capacity_profile(binary.capacity_profile());
        let etex = self.command_profile() == CommandProfile::ETEX26;
        // §534's `**` line is exactly what §313 pseudoprints for the base
        // terminal level; a driver that frames the job here rather than
        // scanning the line through `scan_startup_file_name` supplies it here.
        first_line.clone_into(&mut self.execution.startup_terminal_line);
        self.command.set_terminal_context_line(first_line);
        let engine = crate::job::JobEngineFraming {
            binary,
            extended_mode: etex,
        };
        if print_terminal_banner {
            crate::job::begin_job_with_terminal_banner(
                &mut self.execution.job,
                stores,
                &mut self.capabilities,
                self.execution.initex,
                self.execution.preloaded_format.as_ref(),
                engine,
                crate::job::StartupLineFraming {
                    first_line,
                    input_name,
                    terminal_banner: true,
                },
            );
        } else {
            crate::job::begin_job_with_terminal_banner(
                &mut self.execution.job,
                stores,
                &mut self.capabilities,
                self.execution.initex,
                self.execution.preloaded_format.as_ref(),
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
        self.execution.preloaded_format = Some(format);
    }

    /// Restores the host-selected INITEX framing bit after an aggregate
    /// `JobStart` checkpoint is forked into a new revision generation.
    /// Command and runtime semantics come from the checkpoint; this bit is
    /// operational job framing and is deliberately absent from it.
    pub fn set_initex_mode(&mut self, initex: bool) {
        debug_assert!(self.execution.startup_terminal_line.is_empty());
        debug_assert!(self.execution.root_main_source.is_none());
        self.execution.initex = initex;
    }

    /// Selects the engine binary identity used by startup framing and shared
    /// compiled command semantics.
    pub fn set_engine_binary(&mut self, binary: crate::job::EngineBinaryIdentity) {
        self.command
            .set_engine_semantics(binary.command_semantics());
        self.execution.engine_binary = Some(binary);
    }

    /// Installs the executable-owned immutable store capacities before a
    /// pre-job aggregate checkpoint is captured. Normal startup repeats this
    /// selection idempotently when it frames the root input.
    pub fn prepare_job_start_stores(&self, stores: &mut Universe<G>) {
        let binary = self.execution.engine_binary.unwrap_or_else(|| {
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
        usage.nest_stack = self.execution.modes.maximum_saved_depth();
        usage.parameter_stack = command_usage.parameter_stack;
        usage.buffer_stack = command_usage.buffer_stack;
        // TeX82 §1334 prints `max_save_stack+6`; §273's conservative
        // check reserves room for six subsequent unchecked words.
        usage.save_stack = self.execution.max_save_stack.saturating_add(6);
        crate::job::finish_job(
            stores,
            self.command_profile(),
            self.execution.engine_binary.unwrap_or_else(|| {
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
        self.execution
            .job
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
    /// [`MainControlStep::End`]: closing every still-open paren, reporting
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
        self.execution.job_body_effect_end = Some(stores.world().effect_pos());
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
        if dump && !self.execution.initex {
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
        if std::mem::replace(&mut self.execution.pdf_navigation_finalized, true) {
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
            self.execution.job_body_effect_end = Some(stores.world().effect_pos());
        }
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
            declined_resource_attempt: Some(&mut self.declined_resource_attempt),
            host_facts: ExecutorHostFacts {
                modes: &self.execution.modes,
                shown_mode: &mut self.execution.shown_mode,
                output_routine_active: &self.execution.boxes.output_routine_active,
                pdf_ignore_depth: self.execution.pdf_ignore_depth,
                telemetry: &mut self.execution.episode_telemetry,
            },
            observations: self
                .operation_observations
                .as_mut()
                .map(|buffer| buffer as &mut dyn CommandObserver),

            diagnostic_effects,

            emit_dvi_override: self.execution.emit_dvi_override,
        }
    }

    /// Returns the command site retained for the most recent resource need.
    #[must_use]
    pub const fn pending_resource_site(&self) -> Option<OriginId> {
        self.execution.pending_resource_site
    }

    /// Releases the diagnostic-only site retained for the resource need that
    /// has just been answered by the outer ledger. The answer does not revive
    /// this direct execution; aggregate checkpoint replay clears the
    /// invalidation latch below.
    pub(crate) fn acknowledge_resource_need(&mut self) {
        self.execution.pending_resource_site = None;
    }

    /// Reports whether the most recent error returned after its direct
    /// operation had committed. This is the observation driver's publication
    /// decision; it does not classify or replace the typed error.
    #[must_use]
    pub const fn error_operation_committed(&self) -> bool {
        self.execution.error_operation_committed
    }

    /// Consumes the one provider-decline sideband produced during the current
    /// step. The outer driver uses it to suspend without invoking its legacy
    /// host a second time.
    pub(crate) fn take_declined_resource_attempt(&mut self) -> Option<ResourceNeed> {
        self.declined_resource_attempt.take()
    }

    fn ensure_not_halted(&self) -> Result<(), ExecError> {
        if self.execution.halted {
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
        take_prepared_dvi_pages(&mut self.execution.prepared_dvi_pages)
    }

    /// Installs the host's compact paragraph policy before execution.
    pub(crate) fn set_paragraph_checkpoint_demand(&mut self, identity: Option<bool>) {
        debug_assert!(!self.execution.paragraph_checkpoint_cut);
        self.execution.paragraph_checkpoint_demand = identity;
    }

    pub(crate) fn take_paragraph_checkpoint(
        &mut self,
        stores: &Universe<G>,
    ) -> Option<(crate::checkpoint::CheckpointEligibility, bool)> {
        if !std::mem::take(&mut self.execution.paragraph_checkpoint_cut) {
            return None;
        }
        let wants_identity = self.execution.paragraph_checkpoint_demand.take()?;
        let restartable = stores.checkpoint_eligible()
            && self.command.named_boundary_is_quiescent()
            && self.execution.active_alignment.is_none()
            && self.operation_observations.is_none()
            && self.execution.operation_receipt_start.is_none()
            && self.execution.prepared_shipout.is_none()
            && self.execution.prepared_pdf_form.is_none()
            && self.execution.pending_job_end.is_none()
            && !self.execution.page_region_succession_pending
            && self.execution.pending_resource_site.is_none()
            && !self.execution.halted
            && !self.execution.boxes.output_routine_active
            && self.execution.modes.restart_checkpoint_is_quiescent();
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
        self.execution.job_start_eligibility.take()
    }

    /// Records one ordered named-boundary intent per newly committed shipout.
    /// Structural and command-owned continuations gate later publication.
    /// Returns the replay projection of TeX's current execution mode.
    #[must_use]
    pub fn current_mode(&self) -> Mode {
        self.execution.modes.current_mode()
    }

    #[cfg(test)]
    pub(crate) fn mode_nest_for_test(&self) -> &ModeNest {
        &self.execution.modes
    }

    #[cfg(test)]
    pub(crate) fn mode_nest_mut_for_test(&mut self) -> &mut ModeNest {
        &mut self.execution.modes
    }

    /// Returns the structural alignment started by the most recent replayed
    /// `\halign` or `\valign`, if it has not yet been finished.
    #[must_use]
    pub fn active_alignment(&self) -> Option<AlignmentIdentity> {
        self.execution
            .active_alignment
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
            && self
                .execution
                .active_alignment
                .as_ref()
                .map(|active| active.identity)
                == Some(identity)
        {
            self.execution.active_alignment = None;
            if let Some(outer) = self.execution.boxes.suspended_alignments.pop() {
                self.command
                    .apply_alignment_request(
                        &stores.command_context().expect("live generation"),
                        AlignmentRequest::Resume(outer.identity),
                    )
                    .map_err(|_| ExecError::MissingToken {
                        context: "nested alignment resumption",
                    })?;
                self.execution.active_alignment = Some(outer);
            }
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
        if std::mem::replace(&mut self.execution.main_control_entered, true) {
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
        self.execution.error_operation_committed = false;
        let initial_effect_pos = stores.world().effect_pos();
        let initial_artifacts = stores.world().committed_artifacts().len();
        let initial_boundaries = 0;
        let initial_format_dump = self.execution.dumped_format.is_some();
        let initial_diagnostic = self.execution.first_causal_context.is_some();
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
        self.execution.episode_telemetry.record_attempt();
        self.execution.advance_telemetry.attempts += 1;
        loop {
            let mut diagnostic_effects = DiagnosticEffects::new();
            let mut completed_native_commands = 0;
            let pending = if !self.execution.page_region_succession_pending
                && !self.execution.boxes.output_routine_active
            {
                PendingPageOutputFacts::capture_runtime(stores)
            } else {
                PendingPageOutputFacts::default()
            };
            let applied = if pending.is_pending() {
                self.fire_pending_page_output(stores, &mut diagnostic_effects, pending)
                    .map(|()| MainControlStep::Continue)
            } else {
                self.interpret_chunk(
                    stores,
                    &mut diagnostic_effects,
                    tracked_mark.is_some(),
                    &mut resource_provider,
                    max_operations - operations,
                    &mut completed_native_commands,
                )
            };
            operations += completed_native_commands.max(1);
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
                        self.execution
                            .episode_telemetry
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
                            self.execution.advance_telemetry.resource_halts += 1;
                            #[cfg(feature = "profiling")]
                            self.execution
                                .episode_telemetry
                                .record_rollback(crate::SemanticEpisodeBarrier::Resource, 1);
                            #[cfg(not(feature = "profiling"))]
                            self.execution
                                .episode_telemetry
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
            if let Some(error) = self
                .admit_observed_receipt(stores, operation_termination(step, self.execution.fatal))
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
        self.execution.job_start_eligibility = None;
        let result =
            self.execute_direct_episode(stores, max_operations, tracked_region, resource_provider);
        if result.is_err() {
            self.execution.halted = true;
        }
        result
    }

    /// Shared entry preamble. Returns `false` when a recorded fatal stop
    /// leaves only the terminal `End` step to deliver.
    fn begin_advance(&mut self, stores: &mut Universe<G>) -> Result<bool, ExecError> {
        if self.execution.ended {
            return Err(ExecError::ExecutionAlreadyTerminated);
        }
        self.declined_resource_attempt = None;
        self.ensure_not_halted()?;
        if !self.execution.pure_memo_initialized {
            let runtime = stores.take_pure_memo_config().map_or_else(
                tex_state::PureMemoRuntime::default,
                tex_state::PureMemoRuntime::new,
            );
            self.install_pure_memo_runtime(runtime);
        }
        stores.attach_pure_memo_capability(&self.execution.pure_memo);
        Ok(self.execution.fatal.is_none())
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
                    modes: &self.execution.modes,
                    shown_mode: &mut self.execution.shown_mode,
                    output_routine_active: &self.execution.boxes.output_routine_active,
                    pdf_ignore_depth: self.execution.pdf_ignore_depth,
                    telemetry: &mut self.execution.episode_telemetry,
                };
                let mut processor = if let Some(provider) = resource_provider.as_deref_mut() {
                    command_processor_with_resource_provider(
                        &mut self.command,
                        self.fuel.fuel_mut(),
                        &mut self.capabilities,
                        &mut host_facts as &mut dyn tex_command::CommandHostFacts<G>,
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
                        &mut host_facts as &mut dyn tex_command::CommandHostFacts<G>,
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
        let applied = self.interpret_fetched(
            stores,
            command,
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
        if self.execution.page_region_succession_pending {
            // The active operation can still restore a mode root. Its commit
            // retries succession before this builder continuation resumes.
            return Ok(());
        }
        if self.execution.boxes.output_routine_active || !pending.is_pending() {
            return Ok(());
        }
        while !self.execution.boxes.output_routine_active {
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
                crate::page_builder::build_page(&mut context, diagnostic_effects, &self.command)?;
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
                let mut geometry = pack_geometry_sink(
                    &self.command,
                    self.operation_observations
                        .as_mut()
                        .map(|buffer| buffer as &mut dyn CommandObserver),
                );
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
                        declined_resource_attempt: Some(&mut self.declined_resource_attempt),
                        host_facts: ExecutorHostFacts {
                            modes: &self.execution.modes,
                            shown_mode: &mut self.execution.shown_mode,
                            output_routine_active: &self.execution.boxes.output_routine_active,
                            pdf_ignore_depth: self.execution.pdf_ignore_depth,
                            telemetry: &mut self.execution.episode_telemetry,
                        },
                        observations: self
                            .operation_observations
                            .as_mut()
                            .map(|buffer| buffer as &mut dyn CommandObserver),

                        diagnostic_effects,

                        emit_dvi_override: self.execution.emit_dvi_override,
                    };
                    let publication = shipout_replay_box(page, stores, &mut command)?;
                    // Detached output no longer owns runtime nodes. Establish
                    // the complete next-page owner before allowing the old
                    // region to retire; the outer vertical mode is rootless
                    // throughout the default output path.
                    self.execution.page_region_succession_pending = true;
                    self.finish_pending_page_region_succession(stores);
                    if let Some(receipt) = publication.and_then(|publication| publication.dvi) {
                        push_prepared_dvi_page(&mut self.execution.prepared_dvi_pages, receipt);
                    }
                    if self.execution.page_region_succession_pending {
                        break;
                    }
                    pending.resume_after_output = true;
                    continue;
                }
                crate::page_output::SelectedPageOutput::UserRoutine => {
                    let mut host_facts = ExecutorHostFacts {
                        modes: &self.execution.modes,
                        shown_mode: &mut self.execution.shown_mode,
                        output_routine_active: &self.execution.boxes.output_routine_active,
                        pdf_ignore_depth: self.execution.pdf_ignore_depth,
                        telemetry: &mut self.execution.episode_telemetry,
                    };
                    let mut processor = command_processor(
                        &mut self.command,
                        self.fuel.fuel_mut(),
                        &mut self.capabilities,
                        &mut host_facts as &mut dyn tex_command::CommandHostFacts<G>,
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
                    opened?;
                    self.command
                        .begin_group(&mut context, diagnostic_effects, GroupKind::Output);
                    drop(context);
                    self.execution.modes.push_at_line(
                        Mode::InternalVertical,
                        -i32::try_from(self.command.current_file_line_number()).unwrap_or(i32::MAX),
                    )?;
                    self.execution.boxes.output_routine_active = true;
                    self.execution.boxes.output_routine_opening_pending = true;
                }
            }
        }
        Ok(())
    }

    /// Executes one command while publishing its demanded semantic evidence.
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
        self.execution.operation_receipt_start = Some(OperationReceiptStart {
            effect: effect_start.raw(),
            artifact: artifact_start,
        });
        let stepped = self.execute_operation(stores, 1, None, resource_provider);
        let mut pending = self.operation_observations.take().unwrap_or_default();
        self.execution.operation_receipt_start.take();
        match &stepped {
            Ok(StepResult::Progress(_)) => {}
            Ok(StepResult::Suspended(_)) => {}
            Err(error) => pending.receipt.set_termination(
                error
                    .as_fatal()
                    .map_or(OperationTermination::Failed, OperationTermination::Fatal),
            ),
        }
        let expected_termination = if let Some(fatal) = self.execution.fatal {
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
        self.execution.fatal.get_or_insert(fatal);
        MainControlStep::End
    }

    /// The fatal error that ended this session, if §93's `succumb` ran.
    ///
    /// `Some` is exactly TeX82 §76's `history=fatal_error_stop`: the job did
    /// not run to `\end`, and no further operation will deliver a command.
    #[must_use]
    pub const fn fatal_error(&self) -> Option<FatalError> {
        self.execution.fatal
    }

    /// Reconstructs the source-bearing fatal for a diagnostic session owner.
    pub(crate) fn captured_fatal_error(&self) -> Option<ExecError> {
        let fatal = self.execution.fatal?;
        let (site, frozen, context, first_recoverable) =
            if let Some((site, frozen, context)) = self.execution.captured_fatal_origin.as_ref() {
                (
                    *site,
                    frozen.clone(),
                    context.clone(),
                    self.execution
                        .first_recoverable_diagnostic
                        .as_deref()
                        .cloned(),
                )
            } else if fatal == FatalError::TooManyErrors {
                let first = self.execution.first_recoverable_diagnostic.as_deref()?;
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
        self.execution.first_recoverable_diagnostic.as_deref()
    }

    /// Moves the first committed recoverable diagnostic into a completed-run
    /// result. The run owner is the only caller that may consume this slot.
    pub fn take_first_recoverable_diagnostic(
        &mut self,
    ) -> Option<Box<crate::FirstRecoverableDiagnostic>> {
        self.execution.first_recoverable_diagnostic.take()
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
        self.execution.job_body_effect_end
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
                modes: &self.execution.modes,
                shown_mode: &mut self.execution.shown_mode,
                output_routine_active: &self.execution.boxes.output_routine_active,
                pdf_ignore_depth: self.execution.pdf_ignore_depth,
                telemetry: &mut self.execution.episode_telemetry,
            };
            let mut processor = command_processor(
                &mut self.command,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
                &mut host_facts as &mut dyn tex_command::CommandHostFacts<G>,
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
            modes: &self.execution.modes,
            shown_mode: &mut self.execution.shown_mode,
            output_routine_active: &self.execution.boxes.output_routine_active,
            pdf_ignore_depth: self.execution.pdf_ignore_depth,
            telemetry: &mut self.execution.episode_telemetry,
        };
        let mut processor = command_processor(
            &mut self.command,
            self.fuel.fuel_mut(),
            &mut self.capabilities,
            &mut host_facts as &mut dyn tex_command::CommandHostFacts<G>,
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
        self.execution.startup_terminal_line.clone_from(&filename);
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

/// The actual mathcode and its demanded dependency projection (TeX82 §1155).
fn math_character_code<G>(stores: &mut CommandContext<'_, G>, ch: char) -> u32 {
    let code = stores.mathcode(ch);
    stores.observe_command_projection(
        tex_state::DependencyKey::Code {
            table: tex_state::DependencyCodeTable::Mathcode,
            scalar: ch.into(),
        },
        DependencyValue::Integer(i64::from(code)),
    );
    code
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
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    kind: MathScriptKind,
) -> Result<ScriptTarget, ExecError> {
    // `t<>empty`: the tail was eligible but already carries this script.
    let tail_index = list.nodes(stores).len().checked_sub(1);
    let (eligible, occupied) = match tail_index.and_then(|index| list.nodes(stores).get(index)) {
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
        let index = list.nodes(stores).len();
        list.push(
            stores,
            Node::MathNoad(MathNoad::new(
                NoadKind::Normal(NoadClass::Ord),
                MathField::Empty,
            )),
        );
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
    kind: LimitType,
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
        noad.kind = NoadKind::Operator(kind);
        true
    })
    .unwrap_or(false)
}

fn start_fraction<G>(
    mut list: crate::mode::ModeListMutation<'_>,
    stores: &mut CommandContext<'_, G>,
    fraction: tex_command::ScannedMathFraction,
) -> bool {
    if list.incomplete_fraction().is_some() {
        return false;
    }
    let numerator = list.take_span();
    list.set_incomplete_fraction(
        stores,
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
fn operation_termination(step: MainControlStep, fatal: Option<FatalError>) -> OperationTermination {
    fatal.map_or_else(
        || match step {
            MainControlStep::Continue => OperationTermination::Continue,
            MainControlStep::End => OperationTermination::End,
            MainControlStep::EndOfInput => OperationTermination::EndOfInput,
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
                let current_list = control.execution.modes.current_list();
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
                let current_list = control.execution.modes.current_list();
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
