//! Resident command episodes and typed suspension-only operation frames.

use super::*;

/// The small command-delivery choice at the front of one operation.
///
/// Delivery selects only how the next completed command enters main control;
/// typed dispatch then stays inside the selected hot or cold execution branch.
pub(super) enum OperationDelivery {
    Replay,
    /// The caller-owned command episode contains the sole live command and
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
    /// Ordinary delivery and operand scanning completed inside the
    /// preflight processor borrow. The typed family operand is the real Rust
    /// borrow barrier before semantic state application; no command or
    /// universal scanned-step DTO crosses it.
    ResidentHot,
    /// Ordinary preflight completed delivery and scanning in its admitted
    /// context; the adjacent typed slot contains the cold operation.
    ResidentCold,
    /// Delivery completed before a cold semantic step. The semantic step still
    /// runs through the sole executor below.
    /// The suspension frame has restored the resident typed cold branch.
    SuspendedCold,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum PreflightCommandPhase {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RegisterAssignmentScanPhase {
    RegisterIndex,
    OptionalEquals,
    Value,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum UnaryOperationScanPhase {
    OptionalEquals,
    Value,
}

#[derive(Debug)]
pub(super) enum ParagraphShapeScanPhase {
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
pub(super) enum PenaltyArrayScanPhase {
    OptionalEquals,
    Count,
    Value { remaining: usize, values: Vec<i32> },
}

#[derive(Debug)]
pub(super) enum FontDimenScanPhase {
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
pub(super) enum FontIntegerScanPhase {
    Font,
    OptionalEquals { font: FontId },
    Value { font: FontId },
}

#[derive(Clone, Copy, Debug)]
pub(super) enum CodeTableScanPhase {
    Character,
    OptionalEquals { character: char },
    Value { character: char },
}

#[derive(Clone, Copy, Debug)]
pub(super) enum PdfFontCodeScanPhase {
    Font,
    Character { font: FontId },
    OptionalEquals { font: FontId, character: u8 },
    Value { font: FontId, character: u8 },
}

#[derive(Clone, Copy, Debug)]
pub(super) enum PdfFontExpandScanPhase {
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
pub(super) enum OpenOutScanPhase {
    Stream,
    OptionalEquals { stream: u8 },
    FileName { stream: u8 },
}

#[derive(Clone, Copy, Debug)]
pub(super) enum MarksScanPhase {
    Class,
    Text { class: u16 },
}

#[derive(Clone, Copy, Debug)]
pub(super) enum CatCodeScanPhase {
    Character,
    OptionalEquals { character: char },
    Value { character: char },
}

#[derive(Clone, Copy, Debug)]
pub(super) enum MathFamilyScanPhase {
    Family,
    OptionalEquals {
        family: tex_command::ScannedMathFamily,
    },
    Font {
        family: tex_command::ScannedMathFamily,
    },
}

#[derive(Clone, Copy, Debug)]
pub(super) enum ArithmeticIndexedTarget {
    Integer,
    Dimension,
    Glue { mu: bool },
}

#[derive(Clone, Copy, Debug)]
pub(super) enum ArithmeticScanPhase {
    TargetCommand,
    TargetIndex { target: ArithmeticIndexedTarget },
    Keyword { target: ArithmeticTarget },
    Operand { target: ArithmeticTarget },
}

#[derive(Clone, Copy, Debug)]
pub(super) enum LeaderGlueResult {
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
pub(super) enum PendingOperationScanPhase {
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

pub(super) fn own_alignment_retry_child<G>(
    alignment: Option<Option<AlignmentIdentity>>,
    mut episode: CommandEpisode<G>,
    cold: ColdOperationSlot<G>,
    alignment_scanner: Option<tex_command::ScannerFrameKey<G>>,
) -> Option<PendingDirectDestination<G>> {
    let Some((alignment, cursor)) = alignment.zip(episode.cursor) else {
        assert!(
            alignment_scanner.is_none(),
            "a detached scanner continuation requires its typed alignment destination"
        );
        return episode
            .has_preflight()
            .then_some(PendingDirectDestination::Frame(PendingFrameDestination {
                frame: OperationFrame::new(episode, cold),
                resume: PendingFrameResume::Delivery,
            }));
    };
    match episode.phase {
        // Alignment remains the caller of its suspended expanded delivery,
        // but it retains only the exact parked root rather than a command
        // projection or scanner wrapper.
        Some(PreflightCommandPhase::Expanding { .. }) if episode.command.is_none() => {
            assert!(
                alignment_scanner.is_none(),
                "an expansion child and alignment retry cannot share scanner capabilities"
            );
            assert!(
                episode.scanner.is_none(),
                "parked expansion owns its scanner child internally"
            );
            let expansion = episode.take_expansion();
            episode.clear_preflight();
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
            let _ = retry;
            Some(PendingDirectDestination::Frame(PendingFrameDestination {
                frame: OperationFrame::new(episode, cold),
                resume: PendingFrameResume::Delivery,
            }))
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

/// Caller-owned storage for the uncommon operation leaf.
///
/// The resident frame carries only the mutually exclusive payload tag and the
/// measured hot operand. Cold scanning installs its completed value once in
/// this adjacent typed slot; preparation and application borrow that same
/// value in place. The slot moves with the frame only at a genuine typed
/// suspension boundary.
pub(super) struct ColdOperationSlot<G> {
    pub(super) operation: Option<PreparedColdCommand<G>>,
}

impl<G> Default for ColdOperationSlot<G> {
    fn default() -> Self {
        Self { operation: None }
    }
}

impl<G> ColdOperationSlot<G> {
    /// Moves a completed leaf into the slot at a genuine suspension handoff.
    ///
    /// Ordinary scanner helpers construct directly through `write_cold_scan!`;
    /// this by-value boundary remains only where the operation itself must
    /// move into the unavailable-resource owner.
    #[inline(always)]
    pub(super) fn write(&mut self, operation: ColdOperation<G>) {
        assert!(
            self.operation.is_none(),
            "one command episode owns one cold leaf"
        );
        self.operation = Some(operation);
    }
}

pub(super) struct ColdExecutionEpisode<'operation, G> {
    pub(super) operation: &'operation mut PreparedColdCommand<G>,
    pub(super) alignment_preamble: Option<PreparedAlignmentPreamble<G>>,
    pub(super) output_start: OperationOutputStart,
}

pub(super) enum TypedOperationError {
    Preparation(ExecError),
    Application(ExecError),
}

impl TypedOperationError {
    pub(super) fn into_exec_error(self) -> ExecError {
        match self {
            Self::Preparation(error) | Self::Application(error) => error,
        }
    }
}

/// Singular stationary owner for one command attempt.
///
/// This value is resident in the executor loop. It owns delivery/scanner state
/// and the hot branch result, but is not a suspension frame and is never
/// moved into generation-lived storage. A genuine suspension packages it in
/// [`OperationFrame`] exactly once.
pub(super) struct CommandEpisode<G> {
    pub(super) hot: Option<hot_apply::HotOperation<G>>,
    pub(super) error: Option<ExecError>,
    pub(super) command: Option<tex_command::CurrentCommand<G>>,
    pub(super) expansion: Option<tex_command::ExpansionWorkKey<G>>,
    pub(super) phase: Option<PreflightCommandPhase>,
    pub(super) cursor: Option<tex_command::CommandDeliveryCursor>,
    pub(super) scanner: Option<tex_command::ScannerFrameKey<G>>,
    pub(super) scalar: tex_command::ScalarScanFrame,
    pub(super) operation_scan: Option<PendingOperationScanPhase>,
    pub(super) alignment_scanner: Option<tex_command::ScannerFrameKey<G>>,
    /// Host/VFS source role active when this detached operation was formed.
    /// This is written only when the command slot is about to be retired, then
    /// travels with that operation through resource suspension.
    pub(super) source_role: Option<tex_command::SourceRole>,
}

impl<G> Default for CommandEpisode<G> {
    fn default() -> Self {
        Self {
            hot: None,
            error: None,
            command: None,
            expansion: None,
            phase: None,
            cursor: None,
            scanner: None,
            scalar: tex_command::ScalarScanFrame::default(),
            operation_scan: None,
            alignment_scanner: None,
            source_role: None,
        }
    }
}

impl<G> CommandEpisode<G> {
    pub(super) fn admit_settled(
        &mut self,
        command: tex_command::CurrentCommand<G>,
        cursor: Option<tex_command::CommandDeliveryCursor>,
    ) {
        self.admit_command(command, PreflightCommandPhase::Settled, cursor);
    }

    pub(super) fn admit_command(
        &mut self,
        command: tex_command::CurrentCommand<G>,
        phase: PreflightCommandPhase,
        cursor: Option<tex_command::CommandDeliveryCursor>,
    ) {
        assert!(self.scalar.is_empty());
        assert!(self.command.replace(command).is_none());
        assert!(self.expansion.is_none());
        assert!(self.phase.replace(phase).is_none());
        self.cursor = cursor;
        self.scanner = None;
        self.operation_scan = None;
    }

    /// Marks a command which raw delivery already wrote into this frame.
    ///
    /// The initial delivery and synchronous expansion paths use the frame's
    /// `command` field as their destination. Advancing that resident value to
    /// a new phase must therefore update only the scalar phase facts rather
    /// than taking and reinserting the whole command.
    pub(super) fn mark_resident_command(
        &mut self,
        phase: PreflightCommandPhase,
        cursor: Option<tex_command::CommandDeliveryCursor>,
    ) {
        assert!(self.scalar.is_empty());
        assert!(self.command.is_some());
        assert!(self.expansion.is_none());
        assert!(self.phase.replace(phase).is_none());
        self.cursor = cursor;
        self.scanner = None;
        self.operation_scan = None;
    }

    pub(super) fn mark_resident_settled(
        &mut self,
        cursor: Option<tex_command::CommandDeliveryCursor>,
    ) {
        self.mark_resident_command(PreflightCommandPhase::Settled, cursor);
    }

    pub(super) fn mark_resident_raw(&mut self, cursor: Option<tex_command::CommandDeliveryCursor>) {
        self.mark_resident_command(PreflightCommandPhase::Raw, cursor);
    }

    pub(super) fn admit_expanding(
        &mut self,
        expansion: tex_command::ExpansionWorkKey<G>,
        main_loop: bool,
        cursor: tex_command::CommandDeliveryCursor,
    ) {
        assert!(self.scalar.is_empty());
        assert!(self.command.is_none());
        assert!(self.expansion.replace(expansion).is_none());
        assert!(
            self.phase
                .replace(PreflightCommandPhase::Expanding { main_loop })
                .is_none()
        );
        self.cursor = Some(cursor);
        self.scanner = None;
        self.operation_scan = None;
    }

    pub(super) fn admit_immediate_pdf(&mut self, primitive: UnexpandablePrimitive) {
        assert!(self.scalar.is_empty());
        assert!(self.command.is_none());
        assert!(self.expansion.is_none());
        assert!(
            self.phase
                .replace(PreflightCommandPhase::ImmediatePdfRetry(primitive))
                .is_none()
        );
        self.cursor = None;
        self.scanner = None;
        self.operation_scan = None;
    }

    pub(super) fn current(&self) -> &tex_command::CurrentCommand<G> {
        self.command
            .as_ref()
            .expect("live operation frame owns its admitted command")
    }

    pub(super) fn current_option(&self) -> Option<&tex_command::CurrentCommand<G>> {
        self.command.as_ref()
    }

    pub(super) fn take_current(&mut self) -> tex_command::CurrentCommand<G> {
        self.command
            .take()
            .expect("live operation frame owns its admitted command")
    }

    pub(super) fn take_expansion(&mut self) -> tex_command::ExpansionWorkKey<G> {
        self.expansion
            .take()
            .expect("expanding operation frame owns its parked expansion")
    }

    pub(super) fn replace_current(&mut self, command: tex_command::CurrentCommand<G>) {
        self.command = Some(command);
    }

    pub(super) fn settle_resident(&mut self) {
        assert!(self.command.is_some());
        assert!(self.expansion.is_none());
        self.phase = Some(PreflightCommandPhase::Settled);
        self.operation_scan = None;
    }

    pub(super) fn discard_resident_command(&mut self) {
        self.command = None;
    }

    pub(super) fn retain_scanner(
        &mut self,
        cursor: tex_command::CommandDeliveryCursor,
        scanner: Option<tex_command::ScannerFrameKey<G>>,
    ) {
        self.cursor = Some(cursor);
        self.scanner = scanner;
    }

    pub(super) fn retain_operation_scan(
        &mut self,
        cursor: tex_command::CommandDeliveryCursor,
        phase: PendingOperationScanPhase,
        scanner: tex_command::ScannerFrameKey<G>,
    ) {
        self.phase = Some(PreflightCommandPhase::OperationScan);
        self.cursor = Some(cursor);
        self.scanner = Some(scanner);
        self.operation_scan = Some(phase);
    }

    pub(super) fn is_command_scan(&self) -> bool {
        matches!(
            self.phase,
            Some(
                PreflightCommandPhase::OperationScan
                    | PreflightCommandPhase::PrefixedCommandScan { .. }
                    | PreflightCommandPhase::PrefixScan { .. }
            )
        )
    }

    pub(super) fn has_preflight(&self) -> bool {
        self.phase.is_some()
    }

    pub(super) fn clear_preflight(&mut self) {
        assert!(self.scalar.is_empty());
        let _ = self.command.take();
        let _ = self.expansion.take();
        self.phase = None;
        self.cursor = None;
        self.scanner = None;
        self.operation_scan = None;
    }

    pub(super) fn retain_source_role(&mut self) {
        self.source_role = self
            .current_option()
            .and_then(tex_command::CurrentCommand::active_source_role);
    }

    pub(super) fn operation_source_role(&self) -> Option<tex_command::SourceRole> {
        self.current_option()
            .and_then(tex_command::CurrentCommand::active_source_role)
            .or(self.source_role)
    }

    pub(super) fn clear_operation_origin(&mut self) {
        self.source_role = None;
    }

    pub(super) fn assert_empty(&self) {
        assert!(
            self.error.is_none()
                && self.hot.is_none()
                && self.command.is_none()
                && self.expansion.is_none()
                && self.phase.is_none()
                && self.cursor.is_none()
                && self.scanner.is_none()
                && self.scalar.is_empty()
                && self.operation_scan.is_none()
                && self.alignment_scanner.is_none()
                && self.source_role.is_none(),
            "one command attempt owns one empty operation frame"
        );
    }

    pub(super) fn write_retry_failure(
        &mut self,
        error: ExecError,
        cursor: tex_command::CommandDeliveryCursor,
        alignment_scanner: Option<tex_command::ScannerFrameKey<G>>,
    ) {
        assert!(
            !self.has_preflight() || alignment_scanner.is_none(),
            "one failed operation retains exactly one scanner destination"
        );
        self.error = Some(error);
        self.cursor = Some(cursor);
        self.alignment_scanner = alignment_scanner;
    }

    pub(super) fn assert_command_only(&self) {
        assert!(
            self.error.is_none()
                && self.hot.is_none()
                && self.phase.is_some()
                && self.alignment_scanner.is_none(),
            "command delivery owns only its operation-local command frame"
        );
    }

    pub(super) fn assert_hot_only(&self) {
        assert!(
            self.error.is_none()
                && self.hot.is_some()
                && self.command.is_none()
                && self.expansion.is_none()
                && self.phase.is_none()
                && self.cursor.is_none()
                && self.scanner.is_none()
                && self.scalar.is_empty()
                && self.operation_scan.is_none()
                && self.alignment_scanner.is_none(),
            "pre-scanned hot delivery lives only in its operation frame"
        );
    }

    pub(super) fn take_error(&mut self) -> ExecError {
        self.error
            .take()
            .expect("failed preparation writes its diagnostic into the frame")
    }

    pub(super) fn has_unavailable(&self, cold: &ColdOperationSlot<G>) -> bool {
        cold.operation.is_some()
    }

    pub(super) fn write_unavailable(
        &mut self,
        cold: &mut ColdOperationSlot<G>,
        operation: ColdOperation<G>,
    ) {
        assert!(self.hot.is_none(), "cold and hot branches are exclusive");
        cold.write(operation);
    }

    pub(super) fn mark_resident_cold(&mut self, cold: &ColdOperationSlot<G>) {
        assert!(self.hot.is_none(), "cold and hot branches are exclusive");
        assert!(
            cold.operation.is_some(),
            "cold scanning fills the resident leaf before publishing its tag"
        );
    }

    #[cfg(feature = "profiling")]
    pub(super) fn unavailable<'a>(&self, cold: &'a ColdOperationSlot<G>) -> &'a ColdOperation<G> {
        assert!(self.hot.is_none(), "cold and hot branches are exclusive");
        cold.operation
            .as_ref()
            .expect("operation frame owns its unavailable cold leaf")
    }

    pub(super) fn unavailable_mut<'a>(
        &self,
        cold: &'a mut ColdOperationSlot<G>,
    ) -> &'a mut ColdOperation<G> {
        assert!(self.hot.is_none(), "cold and hot branches are exclusive");
        cold.operation
            .as_mut()
            .expect("operation frame owns its unavailable cold leaf")
    }

    pub(super) fn clear_cold(&mut self, cold: &mut ColdOperationSlot<G>) {
        assert!(self.hot.is_none(), "cold and hot branches are exclusive");
        cold.operation = None;
    }

    pub(super) fn write_hot(&mut self, operation: hot_apply::HotOperation<G>) {
        assert!(self.hot.is_none(), "one command owns one hot operation");
        self.hot = Some(operation);
    }

    pub(super) fn hot_mut(&mut self) -> &mut hot_apply::HotOperation<G> {
        self.hot
            .as_mut()
            .expect("command episode owns its hot operation")
    }
}

impl<G> std::ops::Deref for CommandEpisode<G> {
    type Target = tex_command::CurrentCommand<G>;

    fn deref(&self) -> &Self::Target {
        self.current()
    }
}

/// Move-only state retained only across a real resource or diagnostic retry.
///
/// Ordinary synchronous commands never construct this type. The resident
/// [`CommandEpisode`] and cold leaf are packaged only at the suspension seam,
/// where their exact scanner, retry, and rollback coordinates must outlive the
/// executor call.
pub(super) struct OperationFrame<G> {
    pub(super) episode: Option<CommandEpisode<G>>,
    pub(super) cold: Option<ColdOperationSlot<G>>,
}

#[cfg(feature = "profiling")]
std::thread_local! {
    static OPERATION_FRAME_CONSTRUCTIONS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

impl<G> OperationFrame<G> {
    #[inline]
    pub(super) fn new(episode: CommandEpisode<G>, cold: ColdOperationSlot<G>) -> Self {
        #[cfg(feature = "profiling")]
        OPERATION_FRAME_CONSTRUCTIONS.with(|count| count.set(count.get() + 1));
        Self {
            episode: Some(episode),
            cold: Some(cold),
        }
    }

    #[inline]
    pub(super) fn into_parts(mut self) -> (CommandEpisode<G>, ColdOperationSlot<G>) {
        self.take_parts()
    }

    #[inline]
    pub(super) fn take_parts(&mut self) -> (CommandEpisode<G>, ColdOperationSlot<G>) {
        (
            self.episode
                .take()
                .expect("suspended operation frame owns its command episode"),
            self.cold
                .take()
                .expect("suspended operation frame owns its cold slot"),
        )
    }
}

#[cfg(feature = "profiling")]
pub(super) fn operation_frame_constructions() -> u64 {
    OPERATION_FRAME_CONSTRUCTIONS.with(std::cell::Cell::get)
}

/// One command after canonical delivery and operand scanning.
///
/// The hot variant is a family-sized borrow-release operand. Only the cold
/// variant materializes a typed cold operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ScannedOperation {
    Hot,
    Cold,
}

pub(super) fn retain_cold_operation<G>(
    frame: &mut CommandEpisode<G>,
    cold: &mut ColdOperationSlot<G>,
    operation: ColdOperation<G>,
) -> ScannedOperation {
    frame.write_unavailable(cold, operation);
    ScannedOperation::Cold
}

pub(super) fn retain_hot_operation<G>(
    frame: &mut CommandEpisode<G>,
    operation: hot_apply::HotOperation<G>,
) -> ScannedOperation {
    frame.write_hot(operation);
    ScannedOperation::Hot
}

pub(super) struct PendingResourceOperation<G> {
    pub(super) attempt: tex_command::PendingCommandAttempt<G, SuspendedResourceResume<G>>,
}

pub(super) struct SuspendedResourceResume<G> {
    pub(super) frame: OperationFrame<G>,
}

pub(super) const SUSPENDED_RESOURCE_RESUME: tex_command::AttemptResumePoint =
    tex_command::AttemptResumePoint {
        command: 1,
        scanner: 0,
        expansion: 0,
        subordinate: 0,
    };

#[derive(Debug)]
pub(super) struct PendingAlignmentDelivery<G> {
    pub(super) alignment: Option<AlignmentIdentity>,
    pub(super) cursor: tex_command::CommandDeliveryCursor,
    pub(super) scanner: Option<tex_command::ScannerFrameKey<G>>,
    pub(super) expansion: Option<tex_command::ExpansionWorkKey<G>>,
}

// Both variants are stored in the singular operation owner. Boxing preflight
// state would allocate at the direct-operation continuation boundary.
#[allow(clippy::large_enum_variant)]
pub(super) enum PendingDirectDestination<G> {
    Alignment(PendingAlignmentDelivery<G>),
    Frame(PendingFrameDestination<G>),
}

pub(super) struct PendingFrameDestination<G> {
    pub(super) frame: OperationFrame<G>,
    pub(super) resume: PendingFrameResume,
}

#[derive(Clone, Copy)]
pub(super) enum PendingFrameResume {
    Delivery,
    ColdExecution,
}

pub(super) enum PendingDirectState {
    /// A retry whose prior attempt was rolled back and therefore owns no
    /// attempt-local coordinate. Its next operation starts fresh.
    Fresh,
    /// A live attempt moved together with the exact caller phase that owns its
    /// scanner child and delivery cursor.
    Retained(tex_command::CommandAttemptOperation),
}

pub(super) struct PendingDirectOperation<G> {
    pub(super) state: PendingDirectState,
    pub(super) destination: PendingDirectDestination<G>,
}

impl<G> std::fmt::Debug for PendingDirectOperation<G> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self {
                state: PendingDirectState::Fresh,
                ..
            } => "PendingDirectOperation::<G>::Fresh",
            Self {
                state: PendingDirectState::Retained(_),
                destination,
            } => match destination {
                PendingDirectDestination::Alignment(_) => {
                    "PendingDirectOperation::<G>::RetainedAlignment"
                }
                PendingDirectDestination::Frame(_) => "PendingDirectOperation::<G>::RetainedFrame",
            },
        })
    }
}

pub(super) struct PendingDiagnosticOperation<G> {
    pub(super) operation: tex_command::CommandAttemptOperation,
    pub(super) destination: PendingDiagnosticDestination<G>,
}

pub(super) struct PendingDiagnosticDestination<G> {
    pub(super) frame: OperationFrame<G>,
}

impl<G> std::fmt::Debug for PendingDiagnosticOperation<G> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PendingDiagnosticOperation::<G>::Frame")
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
