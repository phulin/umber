//! Bounded in-session command snapshots and named command summaries.
//!
//! A retained value owns one coarse generation/timeline capability and a fixed
//! tuple of scalar cursors. Checkpoint publication appends a reusable frame;
//! it never clones the aggregate command roots. Generation-owned logical
//! stacks and compact scalar undo restore the selected prefix before the sole
//! current command borrower resumes execution.

use core::fmt;
use core::marker::PhantomData;
use std::cell::RefCell;
use std::rc::Rc;

use tex_state::{GenerationOwner, Universe};

use crate::attempt::AttemptMark;
use crate::processor::ScannerStatus;
use crate::profile::{CommandProfileBoundary, CommandProfileFingerprint, CommandProfileMismatch};
use crate::state::{CommandState, CommandStateRoots};

fn bounded_command_identity<G>(roots: &CommandStateRoots<G>) -> u64 {
    use std::hash::{Hash as _, Hasher as _};

    let mut hash = 0xcbf2_9ce4_8422_2325_u64 ^ 0x636f_6d6d_616e_6431;
    let mut feed = |value: u64| {
        for byte in value.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    let mut semantics = std::collections::hash_map::DefaultHasher::new();
    roots.engine_semantics.hash(&mut semantics);
    feed(semantics.finish());
    feed(roots.input.levels.len() as u64);
    feed(roots.input.retained_file_line_number as u64);
    feed(u64::from(roots.input.terminal_context_line.is_some()));
    feed(roots.input.pending_sources.len() as u64);
    feed(u64::from(roots.input.force_eof));
    feed(roots.parameters.activations.len() as u64);
    feed(roots.conditions.tracked_stack_projection());
    feed(roots.alignment.align_state as u64);
    feed(roots.alignment.align_stack.len() as u64);
    feed(roots.alignment.suspended.len() as u64);
    feed(u64::from(roots.alignment.active_alignment.is_some()));
    feed(u64::from(roots.alignment.active_cell.is_some()));
    feed(u64::from(roots.alignment.completed_preamble.is_some()));
    feed(roots.replay_completions.len() as u64);
    feed(roots.pending_replay_completions.len() as u64);
    feed(roots.semantic_diagnostics.len() as u64);
    feed(roots.group_payloads.len() as u64);
    feed(roots.aftergroup_payloads.len() as u64);
    feed(u64::from(roots.afterassignment.is_some()));
    feed(u64::from(roots.name_in_progress));
    feed(u64::from(roots.pending_input_open.is_some()));
    feed(roots.named_token_list_pushes.len() as u64);
    hash
}

/// Monotonic identity source for in-session command checkpoints.
///
/// The live command machine borrows the aggregate roots exclusively. Its drop
/// returns that loan here for a candidate fork; a checkpoint retains only one
/// reusable frame and its compact coordinates.
pub(crate) struct CommandTimeline<G> {
    next_serial: u32,
    frames: Vec<CommandTimelineFrame>,
    free_frames: Vec<usize>,
    returned_roots: Option<CommandStateRoots<G>>,
    root_undo: Vec<CommandRootUndo<G>>,
}

#[derive(Clone, Copy, Debug)]
struct CommandTimelineFrame {
    serial: u32,
    cursor: CommandSnapshotCursor,
    attempt: AttemptMark,
    root_undo: usize,
    owners: u32,
}

enum CommandRootUndo<G> {
    NameInProgress(bool),
    Afterassignment(Option<crate::state::CommandPayload<G>>),
    CumulativeExpansions(u64),
    AlignState(i32),
    NextInputLevelIdentity(u64),
    NextSourceIdentity(u64),
    RetainedFileLineNumber(i32),
    ForceEof(bool),
    PendingInputOpen(Option<crate::ScannedFileName>),
    ExpansionDiagnosticPush,
}

impl<G> Default for CommandTimeline<G> {
    fn default() -> Self {
        Self {
            next_serial: 0,
            frames: Vec::with_capacity(64),
            free_frames: Vec::with_capacity(64),
            returned_roots: None,
            root_undo: Vec::with_capacity(256),
        }
    }
}

impl<G> fmt::Debug for CommandTimeline<G> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CommandTimeline(..)")
    }
}

impl<G> CommandTimeline<G> {
    fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(
                self.frames
                    .capacity()
                    .saturating_mul(std::mem::size_of::<CommandTimelineFrame>()),
            )
            .saturating_add(
                self.free_frames
                    .capacity()
                    .saturating_mul(std::mem::size_of::<usize>()),
            )
            .saturating_add(
                self.root_undo
                    .capacity()
                    .saturating_mul(std::mem::size_of::<CommandRootUndo<G>>()),
            )
            .saturating_add(
                self.returned_roots
                    .as_ref()
                    .map_or(0, CommandStateRoots::retained_bytes),
            )
    }

    #[cfg(test)]
    fn live_frame_count(&self) -> usize {
        self.frames.iter().filter(|frame| frame.serial != 0).count()
    }

    #[cfg(test)]
    fn frame_capacity(&self) -> usize {
        self.frames.capacity()
    }

    pub(crate) fn retain(
        &mut self,
        attempt: AttemptMark,
        arenas: CommandArenaCursors,
        stacks: CommandStackCursors,
    ) -> Result<CommandSnapshotCursor, CommandSummaryError> {
        if !attempt.is_empty() {
            return Err(CommandSummaryError::AttemptSuspended);
        }
        self.retain_transient(attempt, arenas, stacks)
    }

    fn retain_transient(
        &mut self,
        attempt: AttemptMark,
        arenas: CommandArenaCursors,
        stacks: CommandStackCursors,
    ) -> Result<CommandSnapshotCursor, CommandSummaryError> {
        let serial = self
            .next_serial
            .checked_add(1)
            .ok_or(CommandSummaryError::TimelineCapacity)?;
        self.next_serial = serial;
        let cursor = CommandSnapshotCursor::new(serial, arenas, stacks);
        let frame = CommandTimelineFrame {
            serial,
            cursor,
            attempt,
            root_undo: self.root_undo.len(),
            owners: 1,
        };
        if let Some(slot) = self.free_frames.pop() {
            self.frames[slot] = frame;
        } else {
            self.frames.push(frame);
        }
        Ok(cursor)
    }

    fn retain_owner(&mut self, serial: u32) -> bool {
        let Some(frame) = self.frames.iter_mut().find(|frame| frame.serial == serial) else {
            return false;
        };
        let Some(owners) = frame.owners.checked_add(1) else {
            return false;
        };
        frame.owners = owners;
        true
    }

    fn release_owner(&mut self, serial: u32) {
        let Some((slot, frame)) = self
            .frames
            .iter_mut()
            .enumerate()
            .find(|(_, frame)| frame.serial == serial)
        else {
            return;
        };
        frame.owners -= 1;
        if frame.owners == 0 {
            frame.serial = 0;
            self.free_frames.push(slot);
        }
    }

    fn resolve(
        &self,
        cursor: CommandSnapshotCursor,
    ) -> Option<(AttemptMark, CommandSnapshotCursor)> {
        self.frames
            .iter()
            .find(|frame| frame.serial == cursor.command_journal() && frame.cursor == cursor)
            .map(|frame| (frame.attempt, frame.cursor))
    }

    pub(crate) fn return_roots(&mut self, roots: CommandStateRoots<G>) {
        debug_assert!(self.returned_roots.is_none());
        self.returned_roots = Some(roots);
    }

    fn take_roots(&mut self) -> Option<CommandStateRoots<G>> {
        self.returned_roots.take()
    }

    fn has_live_frame(&self) -> bool {
        self.frames.iter().any(|frame| frame.serial != 0)
    }

    pub(crate) fn record_name_in_progress(&mut self, old: bool) {
        if self.has_live_frame() {
            self.root_undo.push(CommandRootUndo::NameInProgress(old));
        }
    }

    pub(crate) fn record_afterassignment(&mut self, old: Option<crate::state::CommandPayload<G>>) {
        if self.has_live_frame() {
            self.root_undo.push(CommandRootUndo::Afterassignment(old));
        }
    }

    pub(crate) fn record_cumulative_expansions(&mut self, old: u64) {
        if self.has_live_frame() {
            self.root_undo
                .push(CommandRootUndo::CumulativeExpansions(old));
        }
    }

    pub(crate) fn record_align_state(&mut self, old: i32) {
        if self.has_live_frame() {
            self.root_undo.push(CommandRootUndo::AlignState(old));
        }
    }

    pub(crate) fn record_next_input_level_identity(&mut self, old: u64) {
        if self.has_live_frame() {
            self.root_undo
                .push(CommandRootUndo::NextInputLevelIdentity(old));
        }
    }

    pub(crate) fn record_next_source_identity(&mut self, old: u64) {
        if self.has_live_frame() {
            self.root_undo
                .push(CommandRootUndo::NextSourceIdentity(old));
        }
    }

    pub(crate) fn record_retained_file_line_number(&mut self, old: i32) {
        if self.has_live_frame() {
            self.root_undo
                .push(CommandRootUndo::RetainedFileLineNumber(old));
        }
    }

    pub(crate) fn record_force_eof(&mut self, old: bool) {
        if self.has_live_frame() {
            self.root_undo.push(CommandRootUndo::ForceEof(old));
        }
    }

    pub(crate) fn record_pending_input_open(&mut self, old: Option<crate::ScannedFileName>) {
        if self.has_live_frame() {
            self.root_undo.push(CommandRootUndo::PendingInputOpen(old));
        }
    }

    pub(crate) fn record_expansion_diagnostic_push(&mut self) {
        if self.has_live_frame() {
            self.root_undo
                .push(CommandRootUndo::ExpansionDiagnosticPush);
        }
    }

    fn restore_roots(
        &mut self,
        cursor: CommandSnapshotCursor,
        roots: &mut CommandStateRoots<G>,
    ) -> bool {
        let Some(target) = self
            .frames
            .iter()
            .find(|frame| frame.serial == cursor.command_journal() && frame.cursor == cursor)
            .map(|frame| frame.root_undo)
        else {
            return false;
        };
        while self.root_undo.len() > target {
            match self
                .root_undo
                .pop()
                .expect("validated command undo suffix exists")
            {
                CommandRootUndo::NameInProgress(old) => roots.name_in_progress = old,
                CommandRootUndo::Afterassignment(old) => roots.afterassignment = old,
                CommandRootUndo::CumulativeExpansions(old) => {
                    roots.expansion.cumulative_expansions = old;
                }
                CommandRootUndo::AlignState(old) => roots.alignment.align_state = old,
                CommandRootUndo::NextInputLevelIdentity(old) => {
                    roots.input.next_level_identity = old;
                }
                CommandRootUndo::NextSourceIdentity(old) => {
                    roots.input.next_source_identity = old;
                }
                CommandRootUndo::RetainedFileLineNumber(old) => {
                    roots.input.retained_file_line_number = old;
                }
                CommandRootUndo::ForceEof(old) => roots.input.force_eof = old,
                CommandRootUndo::PendingInputOpen(old) => roots.pending_input_open = old,
                CommandRootUndo::ExpansionDiagnosticPush => {
                    roots.expansion.pending_diagnostics.pop();
                }
            }
        }
        for (slot, frame) in self.frames.iter_mut().enumerate() {
            if frame.serial != 0 && frame.root_undo > target {
                frame.serial = 0;
                frame.owners = 0;
                self.free_frames.push(slot);
            }
        }
        true
    }
}

/// Sole coarse capability retained by one command snapshot or summary.
///
/// Command state and its in-session checkpoints never cross a thread boundary,
/// so the command timeline deliberately uses [`Rc`]. The independently shared
/// state generation retains its existing atomic owner.
pub struct CommandGenerationOwner<G> {
    generation: GenerationOwner<G>,
    timeline: Rc<RefCell<CommandTimeline<G>>>,
    attempt: AttemptMark,
    serial: u32,
}

impl<G> Clone for CommandGenerationOwner<G> {
    fn clone(&self) -> Self {
        assert!(
            self.timeline.borrow_mut().retain_owner(self.serial),
            "retained command owner must name a live timeline frame"
        );
        Self {
            generation: self.generation.clone(),
            timeline: Rc::clone(&self.timeline),
            attempt: self.attempt,
            serial: self.serial,
        }
    }
}

impl<G> Drop for CommandGenerationOwner<G> {
    fn drop(&mut self) {
        self.timeline.borrow_mut().release_owner(self.serial);
    }
}

impl<G> fmt::Debug for CommandGenerationOwner<G> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CommandGenerationOwner(..)")
    }
}

impl<G> CommandGenerationOwner<G> {
    pub(crate) fn new(
        generation: GenerationOwner<G>,
        timeline: Rc<RefCell<CommandTimeline<G>>>,
        attempt: AttemptMark,
        cursor: CommandSnapshotCursor,
    ) -> Self {
        Self {
            generation,
            timeline,
            attempt,
            serial: cursor.command_journal(),
        }
    }

    pub(crate) fn addresses(
        &self,
        generation: &GenerationOwner<G>,
        timeline: &Rc<RefCell<CommandTimeline<G>>>,
    ) -> bool {
        self.generation.same_generation(generation) && Rc::ptr_eq(&self.timeline, timeline)
    }

    pub(crate) fn addresses_generation(&self, generation: &GenerationOwner<G>) -> bool {
        self.generation.same_generation(generation)
    }

    fn checkpoint_owner_id(&self) -> tex_state::CheckpointOwnerId {
        self.generation.checkpoint_owner_id()
    }

    pub(crate) fn resolve(&self, cursor: CommandSnapshotCursor) -> Option<AttemptMark> {
        (cursor.command_journal() == self.serial)
            .then(|| self.timeline.borrow().resolve(cursor))
            .flatten()
            .map(|(attempt, _)| attempt)
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
    alignment_undo: u32,
    suspended_alignment_depth: u32,
    suspended_alignment_undo: u32,
    replay_depth: u32,
    diagnostic_count: u32,
    group_payload_depth: u32,
    aftergroup_payload_count: u32,
    aftergroup_payload_undo: u32,
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
    const fn alignment_undo(self) -> u32 {
        self.alignment_undo
    }

    #[must_use]
    const fn suspended_alignment_depth(self) -> u32 {
        self.suspended_alignment_depth
    }

    #[must_use]
    const fn suspended_alignment_undo(self) -> u32 {
        self.suspended_alignment_undo
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
    pub const fn group_payload_depth(self) -> u32 {
        self.group_payload_depth
    }

    #[must_use]
    pub const fn aftergroup_payload_count(self) -> u32 {
        self.aftergroup_payload_count
    }

    #[must_use]
    const fn aftergroup_payload_undo(self) -> u32 {
        self.aftergroup_payload_undo
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
        timeline: &Rc<RefCell<CommandTimeline<G>>>,
    ) -> bool {
        self.generation.addresses(generation, timeline)
    }
}

/// Move-only rollback point for a synchronous nested command episode.
///
/// Unlike [`CommandStateSnapshot`], this cursor may name the exact open
/// attempt suffix owned by its caller. It cannot be cloned, summarized,
/// serialized, or detached, and rollback consumes it once. The caller must
/// therefore finish the nested episode before the enclosing operation can
/// commit or close its scope.
pub struct TransientCommandSnapshot<G> {
    generation: CommandGenerationOwner<G>,
    cursor: CommandSnapshotCursor,
    brand: PhantomData<fn(&G) -> &G>,
}

impl<G> fmt::Debug for TransientCommandSnapshot<G> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TransientCommandSnapshot")
            .field("generation", &self.generation)
            .field("cursor", &self.cursor)
            .finish()
    }
}

impl<G> TransientCommandSnapshot<G> {
    fn new(generation: CommandGenerationOwner<G>, cursor: CommandSnapshotCursor) -> Self {
        Self {
            generation,
            cursor,
            brand: PhantomData,
        }
    }

    fn addresses(
        &self,
        generation: &GenerationOwner<G>,
        timeline: &Rc<RefCell<CommandTimeline<G>>>,
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
    reachable_state_identity_root: Option<u64>,
    retained_owner_bytes: usize,
    brand: PhantomData<fn(&G) -> &G>,
}

impl<G, Owner: Clone> Clone for CommandSummary<G, Owner> {
    fn clone(&self) -> Self {
        Self {
            generation: self.generation.clone(),
            cursor: self.cursor,
            profile_fingerprint: self.profile_fingerprint,
            root_source_anchor: self.root_source_anchor,
            reachable_state_identity_root: self.reachable_state_identity_root,
            retained_owner_bytes: self.retained_owner_bytes,
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
        reachable_state_identity_root: Option<u64>,
        retained_owner_bytes: usize,
    ) -> Self {
        Self {
            generation,
            cursor,
            profile_fingerprint,
            root_source_anchor,
            reachable_state_identity_root,
            retained_owner_bytes,
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

    /// Returns the command owner's maintained future-state root, if the
    /// command ownership lineage supports the complete identity contract.
    #[must_use]
    pub const fn reachable_state_identity_root(&self) -> Option<u64> {
        self.reachable_state_identity_root
    }

    /// Returns the authoritative command-generation charge captured without
    /// traversing semantic payloads.
    #[must_use]
    pub const fn retained_owner_bytes(&self) -> usize {
        self.retained_owner_bytes
    }

    #[must_use]
    pub(crate) const fn generation(&self) -> &Owner {
        &self.generation
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn into_parts(
        self,
    ) -> (
        Owner,
        CommandSnapshotCursor,
        u64,
        Option<u64>,
        Option<u64>,
        usize,
    ) {
        (
            self.generation,
            self.cursor,
            self.profile_fingerprint,
            self.root_source_anchor,
            self.reachable_state_identity_root,
            self.retained_owner_bytes,
        )
    }
}

impl<G> CommandSummary<G> {
    /// Returns the coarse generation id used only to deduplicate accounting.
    #[must_use]
    pub fn checkpoint_owner_id(&self) -> tex_state::CheckpointOwnerId {
        self.generation.checkpoint_owner_id()
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
    timeline: Rc<RefCell<CommandTimeline<G>>>,
    cursor: CommandSnapshotCursor,
    attempt: AttemptMark,
}

impl<G> CommandState<G> {
    fn fork_timeline_summary(summary: &CommandSummary<G>) -> Result<Self, CommandRestoreError> {
        let attempt = summary
            .generation()
            .resolve(summary.cursor())
            .ok_or(CommandRestoreError::InvalidCursor)?;
        if !attempt.is_empty() {
            return Err(CommandRestoreError::InvalidCursor);
        }
        let roots = summary
            .generation
            .timeline
            .borrow_mut()
            .take_roots()
            .ok_or(CommandRestoreError::InvalidCursor)?;
        let mut fork = Self::from_returned_roots(roots, Rc::clone(&summary.generation.timeline));
        let restored = {
            let timeline = Rc::clone(&fork.timeline);
            timeline
                .borrow_mut()
                .restore_roots(summary.cursor(), &mut fork.roots)
        };
        if !restored || !fork.restore_logical_stacks(summary.cursor()) {
            return Err(CommandRestoreError::InvalidCursor);
        }
        Ok(fork)
    }

    /// Runs only the command-family fork for its standalone allocation gate.
    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    pub fn profile_fork_summary(summary: &CommandSummary<G>) -> Result<Self, CommandRestoreError> {
        Self::fork_timeline_summary(summary)
    }

    /// Moves the returned command-root loan into one candidate and restores the
    /// accepted summary's exact logical prefix. Runtime scratch and attempt
    /// lanes begin quiescent.
    #[doc(hidden)]
    pub fn fork_summary(
        summary: &CommandSummary<G>,
        source: &Universe<G>,
        destination: &Universe<G>,
    ) -> Result<Self, CommandRestoreError> {
        let source_generation = source
            .generation_owner()
            .map_err(|_| CommandRestoreError::ForeignGeneration)?;
        let destination_generation = destination
            .generation_owner()
            .map_err(|_| CommandRestoreError::ForeignGeneration)?;
        if !summary
            .generation()
            .addresses_generation(&source_generation)
            || !source_generation.same_generation(&destination_generation)
        {
            return Err(CommandRestoreError::ForeignGeneration);
        }
        let fork = Self::fork_timeline_summary(summary)?;
        fork.profile()
            .validate_fingerprint(
                CommandProfileBoundary::Summary,
                CommandProfileFingerprint::from_u64(summary.profile_fingerprint()),
            )
            .map_err(CommandRestoreError::Profile)?;
        Ok(fork)
    }

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
        self.validate_format_dump_quiescence().is_ok()
    }

    /// Drops the command-only remainder of one successfully captured terminal
    /// format boundary. Engine semantics live in the format's Universe state;
    /// no input, continuation, timeline, or attempt owner crosses with it.
    pub fn close_format_dump_boundary(&mut self) {
        assert!(
            self.format_dump_is_quiescent(),
            "terminal format closure requires quiescent command state"
        );
        self.roots = crate::state::CommandStateRoots::default();
        self.timeline = Rc::new(RefCell::new(CommandTimeline::default()));
        self.attempt = crate::CommandAttempt::default();
        self.scratch = crate::execution_scratch::ExecutionScratch::default();
        self.active_attempt_operation = None;
    }

    fn checkpoint_arenas(
        &self,
        attempt: AttemptMark,
    ) -> Result<CommandArenaCursors, CommandSummaryError> {
        let attempt_rows = attempt
            .checked_row_count()
            .ok_or(CommandSummaryError::TimelineCapacity)?;
        let input = self
            .input
            .levels
            .mark()
            .ok_or(CommandSummaryError::TimelineCapacity)?;
        let parameters = self
            .parameters
            .activations
            .mark()
            .ok_or(CommandSummaryError::TimelineCapacity)?;
        let conditions = self
            .conditions
            .frames
            .mark()
            .ok_or(CommandSummaryError::TimelineCapacity)?;
        let groups = self
            .group_payloads
            .mark()
            .ok_or(CommandSummaryError::TimelineCapacity)?;
        Ok(CommandArenaCursors::new(
            input.undo,
            parameters.undo,
            conditions.undo,
            groups.undo,
            attempt_rows,
        ))
    }

    fn checkpoint_stacks(&self) -> Result<CommandStackCursors, CommandSummaryError> {
        let aftergroups = self
            .aftergroup_payloads
            .mark()
            .ok_or(CommandSummaryError::TimelineCapacity)?;
        let alignment = self
            .alignment
            .align_stack
            .mark()
            .ok_or(CommandSummaryError::TimelineCapacity)?;
        let suspended = self
            .alignment
            .suspended
            .mark()
            .ok_or(CommandSummaryError::TimelineCapacity)?;
        Ok(CommandStackCursors {
            input_depth: u32::try_from(self.input.levels.len())
                .map_err(|_| CommandSummaryError::TimelineCapacity)?,
            parameter_depth: u32::try_from(self.parameters.activations.len())
                .map_err(|_| CommandSummaryError::TimelineCapacity)?,
            condition_depth: u32::try_from(self.conditions.frames.len())
                .map_err(|_| CommandSummaryError::TimelineCapacity)?,
            alignment_depth: alignment.top,
            alignment_undo: alignment.undo,
            suspended_alignment_depth: suspended.top,
            suspended_alignment_undo: suspended.undo,
            replay_depth: u32::try_from(self.replay_completions.len())
                .map_err(|_| CommandSummaryError::TimelineCapacity)?,
            diagnostic_count: u32::try_from(self.semantic_diagnostics.len())
                .map_err(|_| CommandSummaryError::TimelineCapacity)?,
            group_payload_depth: u32::try_from(self.group_payloads.len())
                .map_err(|_| CommandSummaryError::TimelineCapacity)?,
            aftergroup_payload_count: aftergroups.top,
            aftergroup_payload_undo: aftergroups.undo,
            afterassignment_present: self.afterassignment.is_some(),
        })
    }

    fn validate_summary_quiescence(&self) -> Result<(), CommandSummaryError> {
        self.validate_format_dump_quiescence()?;
        if self.transient.active_expansion_depth != 0
            || !self.replay_completions.is_empty()
            || !self.pending_replay_completions.is_empty()
        {
            return Err(CommandSummaryError::ExpansionActive);
        }
        Ok(())
    }

    /// Validates state that cannot be discarded even at a terminal format
    /// boundary. Input and macro replay levels are intentionally omitted:
    /// TeX's `\dump` may be delivered through either, and a successful format
    /// capture closes those command-only levels rather than serializing them.
    fn validate_format_dump_quiescence(&self) -> Result<(), CommandSummaryError> {
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
        if self.pending_input_open.is_some() {
            return Err(CommandSummaryError::ResourceSuspension);
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
        if self.active_attempt_operation.is_some()
            || !self.attempt.is_empty()
            || !self.scratch.is_quiescent()
        {
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
        self.timeline.borrow_mut().retain(attempt, arenas, stacks)
    }

    fn resolve_restore(
        &self,
        owner: &CommandGenerationOwner<G>,
        cursor: CommandSnapshotCursor,
    ) -> Result<PreparedCommandRestore<G>, CommandRestoreError> {
        let attempt = owner
            .resolve(cursor)
            .ok_or(CommandRestoreError::InvalidCursor)?;
        let attempt_rows = attempt
            .checked_row_count()
            .ok_or(CommandRestoreError::InvalidCursor)?;
        let arenas = cursor.arenas();
        let matches = self
            .input
            .levels
            .validates(crate::timeline::LogicalStackMark {
                top: cursor.stacks().input_depth(),
                undo: arenas.input_rows(),
            })
            && self
                .parameters
                .activations
                .validates(crate::timeline::LogicalStackMark {
                    top: cursor.stacks().parameter_depth(),
                    undo: arenas.input_words(),
                })
            && self
                .conditions
                .frames
                .validates(crate::timeline::LogicalStackMark {
                    top: cursor.stacks().condition_depth(),
                    undo: arenas.parameter_words(),
                })
            && self
                .group_payloads
                .validates(crate::timeline::LogicalStackMark {
                    top: cursor.stacks().group_payload_depth(),
                    undo: arenas.builder_words(),
                })
            && self
                .aftergroup_payloads
                .validates(crate::timeline::LogicalStackMark {
                    top: cursor.stacks().aftergroup_payload_count(),
                    undo: cursor.stacks().aftergroup_payload_undo(),
                })
            && self
                .alignment
                .align_stack
                .validates(crate::timeline::LogicalStackMark {
                    top: cursor.stacks().alignment_depth(),
                    undo: cursor.stacks().alignment_undo(),
                })
            && self
                .alignment
                .suspended
                .validates(crate::timeline::LogicalStackMark {
                    top: cursor.stacks().suspended_alignment_depth(),
                    undo: cursor.stacks().suspended_alignment_undo(),
                })
            && arenas.attempt_rows() == attempt_rows;
        if !matches {
            return Err(CommandRestoreError::InvalidCursor);
        }
        self.attempt
            .arena()
            .validate_mark(attempt)
            .map_err(|_| CommandRestoreError::InvalidCursor)?;
        Ok(PreparedCommandRestore {
            timeline: Rc::clone(&self.timeline),
            cursor,
            attempt,
        })
    }

    fn restore_logical_stacks(&mut self, cursor: CommandSnapshotCursor) -> bool {
        let stacks = cursor.stacks();
        let arenas = cursor.arenas();
        self.input
            .levels
            .restore(crate::timeline::LogicalStackMark {
                top: stacks.input_depth(),
                undo: arenas.input_rows(),
            })
            && self
                .parameters
                .activations
                .restore(crate::timeline::LogicalStackMark {
                    top: stacks.parameter_depth(),
                    undo: arenas.input_words(),
                })
            && self
                .conditions
                .frames
                .restore(crate::timeline::LogicalStackMark {
                    top: stacks.condition_depth(),
                    undo: arenas.parameter_words(),
                })
            && self
                .group_payloads
                .restore(crate::timeline::LogicalStackMark {
                    top: stacks.group_payload_depth(),
                    undo: arenas.builder_words(),
                })
            && self
                .aftergroup_payloads
                .restore(crate::timeline::LogicalStackMark {
                    top: stacks.aftergroup_payload_count(),
                    undo: stacks.aftergroup_payload_undo(),
                })
            && self
                .alignment
                .align_stack
                .restore(crate::timeline::LogicalStackMark {
                    top: stacks.alignment_depth(),
                    undo: stacks.alignment_undo(),
                })
            && self
                .alignment
                .suspended
                .restore(crate::timeline::LogicalStackMark {
                    top: stacks.suspended_alignment_depth(),
                    undo: stacks.suspended_alignment_undo(),
                })
    }

    /// Captures one exact in-session command position as bounded marks. The
    /// live root remains exclusively mutable and no accumulated payload moves.
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
            CommandGenerationOwner::new(generation, Rc::clone(&self.timeline), attempt, cursor),
            cursor,
        ))
    }

    /// Captures a move-only rollback point inside one live operation.
    ///
    /// This is the synchronous nested-episode counterpart to [`Self::snapshot`]:
    /// it may retain an open attempt mark, but neither the mark nor its coarse
    /// generation owner can escape through a clone, summary, or detached DTO.
    pub fn transient_snapshot(
        &self,
        universe: &Universe<G>,
    ) -> Result<TransientCommandSnapshot<G>, CommandSummaryError> {
        let generation = universe
            .generation_owner()
            .map_err(|_| CommandSummaryError::GenerationUnavailable)?;
        let attempt = self.attempt.arena().mark();
        let arenas = self.checkpoint_arenas(attempt)?;
        let stacks = self.checkpoint_stacks()?;
        let cursor = self
            .timeline
            .borrow_mut()
            .retain_transient(attempt, arenas, stacks)?;
        Ok(TransientCommandSnapshot::new(
            CommandGenerationOwner::new(generation, Rc::clone(&self.timeline), attempt, cursor),
            cursor,
        ))
    }

    /// Publishes a bounded named-boundary summary. Ordinary mutation remains
    /// direct before and after capture; accumulated command roots are not
    /// cloned.
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
        let retained_owner_bytes = self
            .roots
            .retained_bytes()
            .saturating_add(self.timeline.borrow().retained_bytes());
        Ok(CommandSummary::new(
            CommandGenerationOwner::new(generation, Rc::clone(&self.timeline), attempt, cursor),
            cursor,
            self.checkpoint_profile_fingerprint().get(),
            root_source_anchor,
            self.reachable_state_identity_root(),
            retained_owner_bytes,
        ))
    }

    /// Enables bounded command-root publication before incremental execution.
    /// Late selection fails closed because existing command activity may have
    /// crossed mutation barriers without maintaining semantic value metadata.
    #[doc(hidden)]
    pub fn enable_reachable_state_identity(&mut self) -> bool {
        if self.roots.reachable_state_identity_enabled {
            return true;
        }
        if !self.attempt.is_empty()
            || self.active_attempt_operation.is_some()
            || !self.roots.replay_completions.is_empty()
            || !self.roots.pending_replay_completions.is_empty()
        {
            return false;
        }
        self.roots.reachable_state_identity_enabled = true;
        true
    }

    fn reachable_state_identity_root(&self) -> Option<u64> {
        self.roots
            .reachable_state_identity_enabled
            .then(|| bounded_command_identity(&self.roots))
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
        if !Rc::ptr_eq(&restore.timeline, &self.timeline) {
            return Err(CommandRestoreError::ForeignGeneration);
        }
        self.attempt
            .arena()
            .validate_mark(restore.attempt)
            .map_err(|_| CommandRestoreError::InvalidCursor)?;
        let restored = {
            let timeline = Rc::clone(&self.timeline);
            timeline
                .borrow_mut()
                .restore_roots(restore.cursor, &mut self.roots)
        };
        if !restored {
            return Err(CommandRestoreError::InvalidCursor);
        }
        assert!(self.restore_logical_stacks(restore.cursor));
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
                self.expansion.profile.fingerprint(),
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

    /// Consumes and restores one synchronous nested-episode rollback point.
    pub fn rollback_transient(
        &mut self,
        snapshot: TransientCommandSnapshot<G>,
        universe: &Universe<G>,
    ) -> Result<(), CommandRestoreError> {
        let generation = universe
            .generation_owner()
            .map_err(|_| CommandRestoreError::ForeignGeneration)?;
        if !snapshot.addresses(&generation, &self.timeline) {
            return Err(CommandRestoreError::ForeignGeneration);
        }
        let restore = self.resolve_restore(&snapshot.generation, snapshot.cursor)?;
        self.profile()
            .validate_fingerprint(
                CommandProfileBoundary::Snapshot,
                self.expansion.profile.fingerprint(),
            )
            .map_err(CommandRestoreError::Profile)?;
        self.apply_prepared_restore(restore)
    }
}

#[cfg(test)]
#[path = "snapshot/tests.rs"]
mod tests;
