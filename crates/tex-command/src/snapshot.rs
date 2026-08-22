//! Bounded in-session command snapshots and named command summaries.
//!
//! A retained value owns one complete generation at coarse granularity and a
//! fixed tuple of scalar cursors. It never owns, clones, or borrows an input,
//! token, definition, provenance, or attempt row. The subsystem which owns the
//! live command timeline is responsible for validating these cursors before it
//! restores anything.

use core::fmt;
use core::marker::PhantomData;
use std::sync::{Arc, Mutex};

use tex_state::{GenerationOwner, Universe};

use crate::attempt::AttemptMark;
use crate::processor::ScannerStatus;
use crate::profile::{CommandProfileBoundary, CommandProfileFingerprint, CommandProfileMismatch};
use crate::state::{CommandState, CommandStateRoots};

/// Immutable aggregate roots retained by one command generation.
///
/// Rows own copy-on-write command roots, not individual token, input, or
/// provenance values. A snapshot owns this timeline only through the single
/// coarse [`CommandGenerationOwner`].
pub(crate) struct CommandTimeline<G> {
    rows: Mutex<Vec<CommandTimelineRow<G>>>,
}

struct CommandTimelineRow<G> {
    roots: Arc<CommandStateRoots<G>>,
    attempt: AttemptMark,
}

impl<G> Default for CommandTimeline<G> {
    fn default() -> Self {
        Self {
            rows: Mutex::new(Vec::new()),
        }
    }
}

impl<G> fmt::Debug for CommandTimeline<G> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CommandTimeline(..)")
    }
}

impl<G> CommandTimeline<G> {
    pub(crate) fn retain(
        &self,
        roots: Arc<CommandStateRoots<G>>,
        attempt: AttemptMark,
        arenas: CommandArenaCursors,
        stacks: CommandStackCursors,
    ) -> Result<CommandSnapshotCursor, CommandSummaryError> {
        if !attempt.is_empty() {
            return Err(CommandSummaryError::AttemptSuspended);
        }
        let mut rows = self.rows.lock().expect("command timeline is not poisoned");
        let row = u32::try_from(rows.len()).map_err(|_| CommandSummaryError::TimelineCapacity)?;
        rows.push(CommandTimelineRow { roots, attempt });
        Ok(CommandSnapshotCursor::new(
            row.checked_add(1)
                .ok_or(CommandSummaryError::TimelineCapacity)?,
            arenas,
            stacks,
        ))
    }

    pub(crate) fn resolve(
        &self,
        cursor: CommandSnapshotCursor,
    ) -> Option<(Arc<CommandStateRoots<G>>, AttemptMark)> {
        let row = cursor.command_journal().checked_sub(1)? as usize;
        self.rows
            .lock()
            .expect("command timeline is not poisoned")
            .get(row)
            .map(|row| (Arc::clone(&row.roots), row.attempt))
    }
}

/// Sole coarse owner retained by one command snapshot or summary.
pub struct CommandGenerationOwner<G> {
    generation: GenerationOwner<G>,
    timeline: Arc<CommandTimeline<G>>,
}

impl<G> Clone for CommandGenerationOwner<G> {
    fn clone(&self) -> Self {
        Self {
            generation: self.generation.clone(),
            timeline: Arc::clone(&self.timeline),
        }
    }
}

impl<G> fmt::Debug for CommandGenerationOwner<G> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CommandGenerationOwner(..)")
    }
}

impl<G> CommandGenerationOwner<G> {
    pub(crate) fn new(generation: GenerationOwner<G>, timeline: Arc<CommandTimeline<G>>) -> Self {
        Self {
            generation,
            timeline,
        }
    }

    pub(crate) fn addresses(
        &self,
        generation: &GenerationOwner<G>,
        timeline: &Arc<CommandTimeline<G>>,
    ) -> bool {
        self.generation.same_generation(generation) && Arc::ptr_eq(&self.timeline, timeline)
    }

    pub(crate) fn resolve(
        &self,
        cursor: CommandSnapshotCursor,
    ) -> Option<(Arc<CommandStateRoots<G>>, AttemptMark)> {
        self.timeline.resolve(cursor)
    }
}

/// Watermarks for command-owned append-only storage.
///
/// Each coordinate is an exclusive row count. The corresponding arena
/// validates the coordinate against its own generation before truncation.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct CommandArenaCursors {
    input_rows: u32,
    input_words: u32,
    parameter_words: u32,
    builder_words: u32,
    attempt_rows: u32,
}

impl CommandArenaCursors {
    #[must_use]
    pub const fn new(
        input_rows: u32,
        input_words: u32,
        parameter_words: u32,
        builder_words: u32,
        attempt_rows: u32,
    ) -> Self {
        Self {
            input_rows,
            input_words,
            parameter_words,
            builder_words,
            attempt_rows,
        }
    }

    #[must_use]
    pub const fn input_rows(self) -> u32 {
        self.input_rows
    }

    #[must_use]
    pub const fn input_words(self) -> u32 {
        self.input_words
    }

    #[must_use]
    pub const fn parameter_words(self) -> u32 {
        self.parameter_words
    }

    #[must_use]
    pub const fn builder_words(self) -> u32 {
        self.builder_words
    }

    #[must_use]
    pub const fn attempt_rows(self) -> u32 {
        self.attempt_rows
    }
}

/// Length cursors for command-owned stacks and ordered ledgers.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct CommandStackCursors {
    input_depth: u32,
    parameter_depth: u32,
    condition_depth: u32,
    alignment_depth: u32,
    replay_depth: u32,
    diagnostic_count: u32,
    framing_event_count: u32,
    group_payload_depth: u32,
    aftergroup_payload_count: u32,
    afterassignment_present: bool,
}

impl CommandStackCursors {
    #[must_use]
    pub const fn input_depth(self) -> u32 {
        self.input_depth
    }

    #[must_use]
    pub const fn parameter_depth(self) -> u32 {
        self.parameter_depth
    }

    #[must_use]
    pub const fn condition_depth(self) -> u32 {
        self.condition_depth
    }

    #[must_use]
    pub const fn alignment_depth(self) -> u32 {
        self.alignment_depth
    }

    #[must_use]
    pub const fn replay_depth(self) -> u32 {
        self.replay_depth
    }

    #[must_use]
    pub const fn diagnostic_count(self) -> u32 {
        self.diagnostic_count
    }

    #[must_use]
    pub const fn framing_event_count(self) -> u32 {
        self.framing_event_count
    }

    #[must_use]
    pub const fn group_payload_depth(self) -> u32 {
        self.group_payload_depth
    }

    #[must_use]
    pub const fn aftergroup_payload_count(self) -> u32 {
        self.aftergroup_payload_count
    }

    #[must_use]
    pub const fn afterassignment_present(self) -> bool {
        self.afterassignment_present
    }
}

/// Complete fixed-size command coordinate captured at a restorable boundary.
///
/// `command_journal` addresses scalar and replacement mutations. Arena and
/// stack cursors address append-only suffixes. Restoration must acquire the
/// retained generation before replaying the journal or exposing any restored
/// coordinate, and may truncate suffixes only after roots have transferred.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct CommandSnapshotCursor {
    command_journal: u32,
    arenas: CommandArenaCursors,
    stacks: CommandStackCursors,
}

impl CommandSnapshotCursor {
    #[must_use]
    pub const fn new(
        command_journal: u32,
        arenas: CommandArenaCursors,
        stacks: CommandStackCursors,
    ) -> Self {
        Self {
            command_journal,
            arenas,
            stacks,
        }
    }

    #[must_use]
    pub const fn command_journal(self) -> u32 {
        self.command_journal
    }

    #[must_use]
    pub const fn arenas(self) -> CommandArenaCursors {
        self.arenas
    }

    #[must_use]
    pub const fn stacks(self) -> CommandStackCursors {
        self.stacks
    }
}

/// Exact in-session command snapshot for one admitted generation.
///
/// The default owner is [`CommandGenerationOwner<G>`]. The owner parameter
/// exists so the fixed-cursor contract can be tested without constructing a
/// live TeX session; production construction remains crate-private.
pub struct CommandStateSnapshot<G, Owner = CommandGenerationOwner<G>> {
    generation: Owner,
    cursor: CommandSnapshotCursor,
    brand: PhantomData<fn(&G) -> &G>,
}

impl<G, Owner: Clone> Clone for CommandStateSnapshot<G, Owner> {
    fn clone(&self) -> Self {
        Self {
            generation: self.generation.clone(),
            cursor: self.cursor,
            brand: PhantomData,
        }
    }
}

impl<G, Owner: fmt::Debug> fmt::Debug for CommandStateSnapshot<G, Owner> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommandStateSnapshot")
            .field("generation", &self.generation)
            .field("cursor", &self.cursor)
            .finish()
    }
}

impl<G, Owner> CommandStateSnapshot<G, Owner> {
    #[must_use]
    pub(crate) const fn new(generation: Owner, cursor: CommandSnapshotCursor) -> Self {
        Self {
            generation,
            cursor,
            brand: PhantomData,
        }
    }

    #[must_use]
    pub const fn cursor(&self) -> CommandSnapshotCursor {
        self.cursor
    }

    #[must_use]
    pub(crate) const fn generation(&self) -> &Owner {
        &self.generation
    }
}

impl<G> CommandStateSnapshot<G> {
    /// Whether this snapshot addresses the admitted generation retained by
    /// `generation`.
    #[must_use]
    pub(crate) fn addresses(
        &self,
        generation: &GenerationOwner<G>,
        timeline: &Arc<CommandTimeline<G>>,
    ) -> bool {
        self.generation.addresses(generation, timeline)
    }
}

/// Restartable command state retained at a named in-session boundary.
///
/// A summary differs from an operation snapshot only in its publication
/// proof: construction requires quiescent command state and records the
/// portable profile fingerprint. The live form still contains no copied
/// command graph; cold detachment turns its selected roots into recipes.
pub struct CommandSummary<G, Owner = CommandGenerationOwner<G>> {
    generation: Owner,
    cursor: CommandSnapshotCursor,
    profile_fingerprint: u64,
    root_source_anchor: Option<u64>,
    brand: PhantomData<fn(&G) -> &G>,
}

impl<G, Owner: Clone> Clone for CommandSummary<G, Owner> {
    fn clone(&self) -> Self {
        Self {
            generation: self.generation.clone(),
            cursor: self.cursor,
            profile_fingerprint: self.profile_fingerprint,
            root_source_anchor: self.root_source_anchor,
            brand: PhantomData,
        }
    }
}

impl<G, Owner: fmt::Debug> fmt::Debug for CommandSummary<G, Owner> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommandSummary")
            .field("generation", &self.generation)
            .field("cursor", &self.cursor)
            .field("profile_fingerprint", &self.profile_fingerprint)
            .field("root_source_anchor", &self.root_source_anchor)
            .finish()
    }
}

impl<G, Owner> CommandSummary<G, Owner> {
    #[must_use]
    pub(crate) const fn new(
        generation: Owner,
        cursor: CommandSnapshotCursor,
        profile_fingerprint: u64,
        root_source_anchor: Option<u64>,
    ) -> Self {
        Self {
            generation,
            cursor,
            profile_fingerprint,
            root_source_anchor,
            brand: PhantomData,
        }
    }

    #[must_use]
    pub const fn cursor(&self) -> CommandSnapshotCursor {
        self.cursor
    }

    #[must_use]
    pub const fn profile_fingerprint(&self) -> u64 {
        self.profile_fingerprint
    }

    #[must_use]
    pub const fn root_source_anchor(&self) -> Option<u64> {
        self.root_source_anchor
    }

    #[must_use]
    pub(crate) const fn generation(&self) -> &Owner {
        &self.generation
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn into_parts(self) -> (Owner, CommandSnapshotCursor, u64, Option<u64>) {
        (
            self.generation,
            self.cursor,
            self.profile_fingerprint,
            self.root_source_anchor,
        )
    }
}

impl<G> CommandSummary<G> {
    /// Copies the one selected quiescent command root into an independent
    /// destination timeline whose coarse immutable owner is the freshly
    /// relocated state generation.
    pub fn dense_copy_for_compaction(
        &self,
        destination_generation: GenerationOwner<G>,
    ) -> Result<Self, CommandRestoreError> {
        let (roots, attempt) = self
            .generation
            .resolve(self.cursor)
            .ok_or(CommandRestoreError::InvalidCursor)?;
        if !attempt.is_empty() {
            return Err(CommandRestoreError::InvalidCursor);
        }
        let timeline = Arc::new(CommandTimeline::default());
        let cursor = timeline
            .retain(
                Arc::new(roots.as_ref().clone()),
                attempt,
                self.cursor.arenas(),
                self.cursor.stacks(),
            )
            .map_err(|_| CommandRestoreError::InvalidCursor)?;
        Ok(Self::new(
            CommandGenerationOwner::new(destination_generation, timeline),
            cursor,
            self.profile_fingerprint,
            self.root_source_anchor,
        ))
    }
}

/// The first nonquiescent command-state class preventing summary publication.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CommandSummaryError {
    ConditionalSkip,
    MacroMatch,
    DefinitionScan,
    AlignmentScan,
    AbsorbingScan,
    ExpansionActive,
    AlignmentTemplateActive,
    SuspendedAlignment,
    LiveTokenBuilder,
    LiveRollbackRoot,
    ScannerWarningContext,
    PendingSemanticDiagnostic,
    ResourceSuspension,
    AttemptSuspended,
    TimelineCapacity,
    GenerationUnavailable,
}

impl fmt::Display for CommandSummaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ConditionalSkip => "conditional skipping is active",
            Self::MacroMatch => "macro argument matching is active",
            Self::DefinitionScan => "definition scanning is active",
            Self::AlignmentScan => "alignment scanning is active",
            Self::AbsorbingScan => "balanced token absorption is active",
            Self::ExpansionActive => "command expansion is active",
            Self::AlignmentTemplateActive => "alignment template delivery is active",
            Self::SuspendedAlignment => "an alignment delivery context is suspended",
            Self::LiveTokenBuilder => "a semantic token builder is live",
            Self::LiveRollbackRoot => "a temporary rollback root is live",
            Self::ScannerWarningContext => "scanner warning context remains installed",
            Self::PendingSemanticDiagnostic => {
                "a command semantic diagnostic is awaiting executor delivery"
            }
            Self::ResourceSuspension => "a command resource request is pending",
            Self::AttemptSuspended => "the command attempt is owned by a suspension",
            Self::TimelineCapacity => "the command checkpoint timeline is full",
            Self::GenerationUnavailable => "the command generation is unavailable",
        })
    }
}

impl std::error::Error for CommandSummaryError {}

/// Validation failure for one in-session command-root restore.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandRestoreError {
    Profile(CommandProfileMismatch),
    ForeignGeneration,
    InvalidCursor,
}

impl fmt::Display for CommandRestoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Profile(error) => error.fmt(formatter),
            Self::ForeignGeneration => {
                formatter.write_str("the command checkpoint belongs to another generation")
            }
            Self::InvalidCursor => formatter.write_str("the command checkpoint cursor is invalid"),
        }
    }
}

impl std::error::Error for CommandRestoreError {}

/// Fully validated command-root switch. Applying it cannot fail.
pub struct PreparedCommandRestore<G> {
    timeline: Arc<CommandTimeline<G>>,
    roots: Arc<CommandStateRoots<G>>,
    attempt: AttemptMark,
}

impl<G> CommandState<G> {
    /// Reports whether the live command machine can publish one named
    /// boundary without retaining a scanner, delivery, or attempt owner.
    ///
    /// The executor recomputes unreachable attempt suffixes before calling
    /// this projection. It does not allocate a timeline row or mutate state.
    #[must_use]
    pub fn named_boundary_is_quiescent(&self) -> bool {
        self.validate_summary_quiescence().is_ok()
    }

    /// Reports whether the terminal format boundary can discard the remaining
    /// command-only state without abandoning an active scanner or attempt.
    ///
    /// TeX's terminal `\dump` can leave unread input or macro levels behind;
    /// those levels are not part of a fresh-job format image. They are dropped
    /// only after aggregate image capture has succeeded.
    #[must_use]
    pub fn format_dump_is_quiescent(&self) -> bool {
        self.validate_summary_quiescence().is_ok()
    }

    /// Drops the command-only remainder of one successfully captured terminal
    /// format boundary. Engine semantics live in the format's Universe state;
    /// no input, continuation, timeline, or attempt owner crosses with it.
    pub fn close_format_dump_boundary(&mut self) {
        assert!(
            self.format_dump_is_quiescent(),
            "terminal format closure requires quiescent command state"
        );
        self.roots = Arc::new(crate::state::CommandStateRoots::default());
        self.timeline = Arc::new(CommandTimeline::default());
        self.attempt = crate::CommandAttempt::default();
    }

    fn checkpoint_arenas(
        &self,
        attempt: AttemptMark,
    ) -> Result<CommandArenaCursors, CommandSummaryError> {
        let attempt_rows = attempt
            .checked_row_count()
            .ok_or(CommandSummaryError::TimelineCapacity)?;
        Ok(CommandArenaCursors::new(0, 0, 0, 0, attempt_rows))
    }

    fn checkpoint_stacks(&self) -> Result<CommandStackCursors, CommandSummaryError> {
        let aftergroup_payload_count = self
            .group_payloads
            .iter()
            .try_fold(0_u32, |count, group| {
                let group_count = u32::try_from(group.tokens.len()).ok()?;
                count.checked_add(group_count)
            })
            .ok_or(CommandSummaryError::TimelineCapacity)?;
        Ok(CommandStackCursors {
            input_depth: u32::try_from(self.input.levels.len())
                .map_err(|_| CommandSummaryError::TimelineCapacity)?,
            parameter_depth: u32::try_from(self.parameters.activations.len())
                .map_err(|_| CommandSummaryError::TimelineCapacity)?,
            condition_depth: u32::try_from(self.conditions.frames.len())
                .map_err(|_| CommandSummaryError::TimelineCapacity)?,
            alignment_depth: u32::try_from(self.alignment.align_stack.len())
                .map_err(|_| CommandSummaryError::TimelineCapacity)?,
            replay_depth: u32::try_from(self.replay_completions.len())
                .map_err(|_| CommandSummaryError::TimelineCapacity)?,
            diagnostic_count: u32::try_from(self.semantic_diagnostics.len())
                .map_err(|_| CommandSummaryError::TimelineCapacity)?,
            framing_event_count: u32::try_from(self.file_framing_events.len())
                .map_err(|_| CommandSummaryError::TimelineCapacity)?,
            group_payload_depth: u32::try_from(self.group_payloads.len())
                .map_err(|_| CommandSummaryError::TimelineCapacity)?,
            aftergroup_payload_count,
            afterassignment_present: self.afterassignment.is_some(),
        })
    }

    fn validate_summary_quiescence(&self) -> Result<(), CommandSummaryError> {
        match self.scanner.status() {
            ScannerStatus::Normal => {}
            ScannerStatus::Skipping { .. } => return Err(CommandSummaryError::ConditionalSkip),
            ScannerStatus::Defining { .. } => return Err(CommandSummaryError::DefinitionScan),
            ScannerStatus::Matching { .. } => return Err(CommandSummaryError::MacroMatch),
            ScannerStatus::Aligning { .. } => return Err(CommandSummaryError::AlignmentScan),
            ScannerStatus::Absorbing { .. } => return Err(CommandSummaryError::AbsorbingScan),
        }
        if self.scanner.warning().is_some() {
            return Err(CommandSummaryError::ScannerWarningContext);
        }
        if !self.semantic_diagnostics.is_empty() {
            return Err(CommandSummaryError::PendingSemanticDiagnostic);
        }
        if self.pending_input_open.is_some()
            || self.pending_file_enquiry.is_some()
            || !self.pending_integer_scans.is_empty()
            || !self.pending_scan_toks.is_empty()
            || !self.pending_expansions.is_empty()
            || !self.pending_expandafters.is_empty()
            || !self.pending_csnames.is_empty()
        {
            return Err(CommandSummaryError::ResourceSuspension);
        }
        if self.transient.active_expansion_depth != 0
            || !self.replay_completions.is_empty()
            || !self.pending_replay_completions.is_empty()
        {
            return Err(CommandSummaryError::ExpansionActive);
        }
        if self.alignment.active_alignment.is_some() || self.alignment.active_cell.is_some() {
            return Err(CommandSummaryError::AlignmentTemplateActive);
        }
        if !self.alignment.suspended.is_empty() || !self.alignment.align_stack.is_empty() {
            return Err(CommandSummaryError::SuspendedAlignment);
        }
        if !self.transient.builders.is_empty() {
            return Err(CommandSummaryError::LiveTokenBuilder);
        }
        if !self.transient.rollback_roots.is_empty() {
            return Err(CommandSummaryError::LiveRollbackRoot);
        }
        if !self.attempt.is_empty() {
            return Err(CommandSummaryError::AttemptSuspended);
        }
        Ok(())
    }

    fn retain_cursor(
        &self,
        attempt: AttemptMark,
    ) -> Result<CommandSnapshotCursor, CommandSummaryError> {
        let arenas = self.checkpoint_arenas(attempt)?;
        let stacks = self.checkpoint_stacks()?;
        self.timeline
            .retain(Arc::clone(&self.roots), attempt, arenas, stacks)
    }

    fn resolve_restore(
        &self,
        owner: &CommandGenerationOwner<G>,
        cursor: CommandSnapshotCursor,
    ) -> Result<PreparedCommandRestore<G>, CommandRestoreError> {
        let (roots, attempt) = owner
            .resolve(cursor)
            .ok_or(CommandRestoreError::InvalidCursor)?;
        let attempt_rows = attempt
            .checked_row_count()
            .ok_or(CommandRestoreError::InvalidCursor)?;
        let arenas = cursor.arenas();
        let stacks = cursor.stacks();
        let matches = arenas.input_rows() == 0
            && arenas.input_words() == 0
            && arenas.parameter_words() == 0
            && arenas.builder_words() == 0
            && arenas.attempt_rows() == attempt_rows
            && stacks.input_depth() as usize == roots.input.levels.len()
            && stacks.parameter_depth() as usize == roots.parameters.activations.len()
            && stacks.condition_depth() as usize == roots.conditions.frames.len()
            && stacks.alignment_depth() as usize == roots.alignment.align_stack.len()
            && stacks.replay_depth() as usize == roots.replay_completions.len()
            && stacks.diagnostic_count() as usize == roots.semantic_diagnostics.len()
            && stacks.framing_event_count() as usize == roots.file_framing_events.len();
        let root_aftergroup_payload_count =
            roots.group_payloads.iter().try_fold(0_u32, |count, group| {
                let group_count = u32::try_from(group.tokens.len()).ok()?;
                count.checked_add(group_count)
            });
        let matches = matches
            && stacks.group_payload_depth() as usize == roots.group_payloads.len()
            && Some(stacks.aftergroup_payload_count()) == root_aftergroup_payload_count
            && stacks.afterassignment_present() == roots.afterassignment.is_some();
        if !matches {
            return Err(CommandRestoreError::InvalidCursor);
        }
        self.attempt
            .arena()
            .validate_mark(attempt)
            .map_err(|_| CommandRestoreError::InvalidCursor)?;
        Ok(PreparedCommandRestore {
            timeline: Arc::clone(&self.timeline),
            roots,
            attempt,
        })
    }

    /// Captures one exact in-session command root without cloning its live
    /// graph. The attempt arena stays on the command machine and is addressed
    /// only by the bounded mark retained in the coarse timeline row.
    pub fn snapshot(
        &self,
        universe: &Universe<G>,
    ) -> Result<CommandStateSnapshot<G>, CommandSummaryError> {
        let generation = universe
            .generation_owner()
            .map_err(|_| CommandSummaryError::GenerationUnavailable)?;
        let attempt = self.attempt.arena().mark();
        let cursor = self.retain_cursor(attempt)?;
        Ok(CommandStateSnapshot::new(
            CommandGenerationOwner::new(generation, Arc::clone(&self.timeline)),
            cursor,
        ))
    }

    /// Publishes a bounded named-boundary summary without cloning the live
    /// command graph.
    pub fn publish_summary(
        &self,
        universe: &Universe<G>,
    ) -> Result<CommandSummary<G>, CommandSummaryError> {
        self.validate_summary_quiescence()?;
        let generation = universe
            .generation_owner()
            .map_err(|_| CommandSummaryError::GenerationUnavailable)?;
        let attempt = self.attempt.arena().mark();
        let cursor = self.retain_cursor(attempt)?;
        let root_source_anchor = self.input.levels.iter().find_map(|level| {
            let crate::input::InputLevel::Source(source) = level else {
                return None;
            };
            Some(source.cursor.next_physical_offset)
        });
        Ok(CommandSummary::new(
            CommandGenerationOwner::new(generation, Arc::clone(&self.timeline)),
            cursor,
            self.checkpoint_profile_fingerprint().get(),
            root_source_anchor,
        ))
    }

    /// Validates every command owner, profile, and cursor without mutating
    /// the destination.
    pub fn prepare_summary_restore(
        &self,
        summary: &CommandSummary<G>,
        universe: &Universe<G>,
    ) -> Result<PreparedCommandRestore<G>, CommandRestoreError> {
        self.profile()
            .validate_fingerprint(
                CommandProfileBoundary::Summary,
                CommandProfileFingerprint::from_u64(summary.profile_fingerprint()),
            )
            .map_err(CommandRestoreError::Profile)?;
        let generation = universe
            .generation_owner()
            .map_err(|_| CommandRestoreError::ForeignGeneration)?;
        if !summary.generation().addresses(&generation, &self.timeline) {
            return Err(CommandRestoreError::ForeignGeneration);
        }
        self.resolve_restore(summary.generation(), summary.cursor())
    }

    /// Revalidates the destination and attempt suffix, then installs one
    /// prepared command root before discarding that suffix.
    pub fn apply_prepared_restore(
        &mut self,
        restore: PreparedCommandRestore<G>,
    ) -> Result<(), CommandRestoreError> {
        if !Arc::ptr_eq(&restore.timeline, &self.timeline) {
            return Err(CommandRestoreError::ForeignGeneration);
        }
        self.attempt
            .arena()
            .validate_mark(restore.attempt)
            .map_err(|_| CommandRestoreError::InvalidCursor)?;
        self.roots = restore.roots;
        self.attempt
            .arena_mut()
            .truncate(restore.attempt)
            .expect("prepared command restore validated its attempt mark");
        Ok(())
    }

    /// Validates and restores one named-boundary summary atomically.
    pub fn restore_summary(
        &mut self,
        summary: &CommandSummary<G>,
        universe: &Universe<G>,
    ) -> Result<(), CommandRestoreError> {
        let restore = self.prepare_summary_restore(summary, universe)?;
        self.apply_prepared_restore(restore)
    }

    /// Validates an exact operation snapshot without changing live command
    /// roots or attempt storage.
    pub fn prepare_snapshot_restore(
        &self,
        snapshot: &CommandStateSnapshot<G>,
        universe: &Universe<G>,
    ) -> Result<PreparedCommandRestore<G>, CommandRestoreError> {
        let generation = universe
            .generation_owner()
            .map_err(|_| CommandRestoreError::ForeignGeneration)?;
        if !snapshot.addresses(&generation, &self.timeline) {
            return Err(CommandRestoreError::ForeignGeneration);
        }
        let restore = self.resolve_restore(snapshot.generation(), snapshot.cursor())?;
        self.profile()
            .validate_fingerprint(
                CommandProfileBoundary::Snapshot,
                restore.roots.expansion.profile.fingerprint(),
            )
            .map_err(CommandRestoreError::Profile)?;
        Ok(restore)
    }

    /// Restores one exact operation snapshot after complete validation.
    pub fn rollback(
        &mut self,
        snapshot: &CommandStateSnapshot<G>,
        universe: &Universe<G>,
    ) -> Result<(), CommandRestoreError> {
        let restore = self.prepare_snapshot_restore(snapshot, universe)?;
        self.apply_prepared_restore(restore)
    }
}

#[cfg(test)]
#[path = "snapshot/tests.rs"]
mod tests;
