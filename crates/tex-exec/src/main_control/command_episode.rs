//! Resident command episodes and typed cold-operation boundaries.

use super::*;

/// The small command-delivery choice at the front of one operation.
///
/// Delivery selects only how the next completed command enters main control;
/// typed dispatch then stays inside the selected hot or cold execution branch.
pub(super) enum OperationDelivery {
    Replay,
    /// The caller-owned command episode contains the sole live command and
    /// its compact delivery coordinates.
    Command,
    /// TeX82 §1038's main-loop lookahead delivered this command with bare
    /// `get_next`; it must not acquire an expanded-delivery observation when
    /// the scanner borrow resumes.
    /// Expansion settled in the processor borrow that produced this command,
    /// including its canonical expanded observation. This covers both raw
    /// preflight and an in-place TeX82 `goto reswitch`/§1270 handoff.
    Alignment(AlignmentIdentity),
    /// Ordinary application completed in the admitted command context.
    AppliedDirect,
    /// Ordinary preflight completed delivery and scanning in its admitted
    /// context; the adjacent typed slot contains the cold operation.
    ResidentCold,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum PreflightCommandPhase {
    Settled,
    Raw,
    ImmediatePdfRetry(UnexpandablePrimitive),
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
/// This value is resident in the executor loop only for delivery state.
/// Completed hot operands return directly to their admitted caller and never
/// enter it. A resource miss unwinds this owner; no caller or scanner frame
/// survives the host boundary.
pub(super) struct CommandEpisode<G> {
    pub(super) error: Option<ExecError>,
    pub(super) command: Option<tex_command::CurrentCommand<G>>,
    pub(super) phase: Option<PreflightCommandPhase>,
    pub(super) cursor: Option<tex_command::CommandDeliveryCursor>,
    /// Host/VFS source role active when this detached operation was formed.
    /// This is written only when the command slot is about to be retired, then
    /// travels with that operation through resource suspension.
    pub(super) source_role: Option<tex_command::SourceRole>,
}

impl<G> Default for CommandEpisode<G> {
    fn default() -> Self {
        Self {
            error: None,
            command: None,
            phase: None,
            cursor: None,
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
        assert!(self.command.replace(command).is_none());
        assert!(self.phase.replace(phase).is_none());
        self.cursor = cursor;
    }

    /// Marks a command which raw delivery already wrote into this frame.
    ///
    /// The initial delivery and synchronous expansion paths use the frame's
    /// `command` field as their destination. Advancing that resident value to
    /// a new phase updates only the delivery facts.
    pub(super) fn mark_resident_command(
        &mut self,
        phase: PreflightCommandPhase,
        cursor: Option<tex_command::CommandDeliveryCursor>,
    ) {
        assert!(self.command.is_some());
        assert!(self.phase.replace(phase).is_none());
        self.cursor = cursor;
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

    pub(super) fn admit_immediate_pdf(&mut self, primitive: UnexpandablePrimitive) {
        assert!(self.command.is_none());
        assert!(
            self.phase
                .replace(PreflightCommandPhase::ImmediatePdfRetry(primitive))
                .is_none()
        );
        self.cursor = None;
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

    pub(super) fn replace_current(&mut self, command: tex_command::CurrentCommand<G>) {
        self.command = Some(command);
    }

    pub(super) fn discard_resident_command(&mut self) {
        self.command = None;
    }

    pub(super) fn has_preflight(&self) -> bool {
        self.phase.is_some()
    }

    pub(super) fn clear_preflight(&mut self) {
        let _ = self.command.take();
        self.phase = None;
        self.cursor = None;
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
                && self.command.is_none()
                && self.phase.is_none()
                && self.cursor.is_none()
                && self.source_role.is_none(),
            "one command attempt owns one empty operation frame"
        );
    }

    pub(super) fn write_retry_failure(
        &mut self,
        error: ExecError,
        cursor: tex_command::CommandDeliveryCursor,
    ) {
        self.error = Some(error);
        self.cursor = Some(cursor);
    }

    pub(super) fn assert_command_only(&self) {
        assert!(
            self.error.is_none() && self.phase.is_some(),
            "command delivery owns only its operation-local command frame"
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
        cold.write(operation);
    }

    pub(super) fn mark_resident_cold(&mut self, cold: &ColdOperationSlot<G>) {
        assert!(
            cold.operation.is_some(),
            "cold scanning fills the resident leaf before publishing its tag"
        );
    }

    pub(super) fn unavailable<'a>(&self, cold: &'a ColdOperationSlot<G>) -> &'a ColdOperation<G> {
        cold.operation
            .as_ref()
            .expect("operation frame owns its unavailable cold leaf")
    }

    pub(super) fn unavailable_mut<'a>(
        &self,
        cold: &'a mut ColdOperationSlot<G>,
    ) -> &'a mut ColdOperation<G> {
        cold.operation
            .as_mut()
            .expect("operation frame owns its unavailable cold leaf")
    }

    pub(super) fn clear_cold(&mut self, cold: &mut ColdOperationSlot<G>) {
        cold.operation = None;
    }
}

impl<G> std::ops::Deref for CommandEpisode<G> {
    type Target = tex_command::CurrentCommand<G>;

    fn deref(&self) -> &Self::Target {
        self.current()
    }
}

/// One command after canonical delivery and operand scanning.
///
/// The hot variant is a family-sized borrow-release operand. Only the cold
/// variant materializes a typed cold operation.
pub(super) enum ScannedOperation<G> {
    Hot(hot_apply::HotOperation<G>),
    Cold,
}

pub(super) fn retain_cold_operation<G>(
    frame: &mut CommandEpisode<G>,
    cold: &mut ColdOperationSlot<G>,
    operation: ColdOperation<G>,
) -> ScannedOperation<G> {
    frame.write_unavailable(cold, operation);
    ScannedOperation::Cold
}

pub(super) fn retain_hot_operation<G>(
    operation: hot_apply::HotOperation<G>,
) -> ScannedOperation<G> {
    ScannedOperation::Hot(operation)
}
