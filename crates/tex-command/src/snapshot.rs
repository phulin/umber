//! Bounded in-session command snapshots and named command summaries.
//!
//! A retained value owns one coarse generation/timeline capability and a fixed
//! tuple of scalar cursors. Checkpoint publication appends a reusable frame;
//! it never clones the aggregate command roots. Generation-owned logical
//! stacks and compact scalar undo restore the selected prefix before the sole
//! current command borrower resumes execution.

use core::fmt;
use core::marker::PhantomData;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

use tex_state::{GenerationOwner, Universe};

use crate::attempt::AttemptMark;
use crate::processor::ScannerStatus;
use crate::profile::{CommandProfileBoundary, CommandProfileFingerprint, CommandProfileMismatch};
use crate::scalar_journal::{PackedJournal, PackedJournalMark};
use crate::state::{CommandState, CommandStateRoots};

static NEXT_COMMAND_TIMELINE_OWNER: AtomicU64 = AtomicU64::new(1);

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

/// Sole physical command-history owner for one admitted generation.
///
/// Checkpoint frames retain stable range identities in `ForkArena`; scalar
/// undo lives separately in descriptor-free fixed chunks. Retained snapshots
/// carry only scalar marks and a one-cell frame coordinate; no snapshot aliases
/// either mutable owner.
pub(crate) struct CommandTimeline<G> {
    owner: u64,
    next_serial: u32,
    frame_pages: Vec<CommandFramePage>,
    free_frames: Vec<CommandFrameKey>,
    accepted_frames: VecDeque<CommandFrameKey>,
    scalars: PackedJournal<CommandRootUndo<G>, 128>,
    pending_input: PackedJournal<PendingInputUndo, 8>,
    touched_scalars: u16,
    pending_input_touched: bool,
    coalesced_writes: u64,
    ordered_events: u64,
    fork: Option<CommandTimelineFork>,
    frames_released: u64,
    frames_reused: u64,
}

const COMMAND_FRAMES_PER_PAGE: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CommandFrameKey {
    slot: u32,
    generation: u32,
}

struct CommandFrameSlot {
    generation: u32,
    frame: Option<CommandTimelineFrame>,
}

struct CommandFramePage {
    slots: Box<[CommandFrameSlot]>,
}

#[derive(Clone, Copy, Debug)]
struct CommandTimelineFrame {
    serial: u32,
    cursor: CommandSnapshotCursor,
    attempt: AttemptMark,
}

#[derive(Clone, Copy)]
struct CommandTimelineMark {
    frame: CommandFrameKey,
    scalars: PackedJournalMark,
    pending_input: PackedJournalMark,
}

struct CommandTimelineFork {
    prefix_len: usize,
    detached: VecDeque<CommandFrameKey>,
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
    ExpansionDiagnosticPush(Option<u64>),
}

const _: () = assert!(std::mem::size_of::<CommandRootUndo<()>>() <= 32);

struct PendingInputUndo(Option<crate::ScannedFileName>);

#[repr(u8)]
enum CommandScalarSlot {
    NameInProgress,
    Afterassignment,
    CumulativeExpansions,
    AlignState,
    NextInputLevelIdentity,
    NextSourceIdentity,
    RetainedFileLineNumber,
    ForceEof,
}

impl CommandScalarSlot {
    const fn bit(self) -> u16 {
        1 << self as u8
    }
}

impl<G> CommandRootUndo<G> {
    fn swap(&mut self, roots: &mut CommandStateRoots<G>) {
        match self {
            Self::NameInProgress(value) => std::mem::swap(value, &mut roots.name_in_progress),
            Self::Afterassignment(value) => std::mem::swap(value, &mut roots.afterassignment),
            Self::CumulativeExpansions(value) => {
                std::mem::swap(value, &mut roots.expansion.cumulative_expansions);
            }
            Self::AlignState(value) => std::mem::swap(value, &mut roots.alignment.align_state),
            Self::NextInputLevelIdentity(value) => {
                std::mem::swap(value, &mut roots.input.next_level_identity);
            }
            Self::NextSourceIdentity(value) => {
                std::mem::swap(value, &mut roots.input.next_source_identity);
            }
            Self::RetainedFileLineNumber(value) => {
                std::mem::swap(value, &mut roots.input.retained_file_line_number);
            }
            Self::ForceEof(value) => std::mem::swap(value, &mut roots.input.force_eof),
            Self::ExpansionDiagnosticPush(value) => match value.take() {
                Some(diagnostic) => roots.expansion.pending_diagnostics.push(diagnostic),
                None => {
                    *value = roots.expansion.pending_diagnostics.pop();
                }
            },
        }
    }
}

impl<G> Default for CommandTimeline<G> {
    fn default() -> Self {
        Self {
            next_serial: 0,
            owner: NEXT_COMMAND_TIMELINE_OWNER.fetch_add(1, Ordering::Relaxed),
            frame_pages: Vec::new(),
            free_frames: Vec::new(),
            accepted_frames: VecDeque::new(),
            scalars: PackedJournal::default(),
            pending_input: PackedJournal::default(),
            touched_scalars: 0,
            pending_input_touched: false,
            coalesced_writes: 0,
            ordered_events: 0,
            fork: None,
            frames_released: 0,
            frames_reused: 0,
        }
    }
}

impl<G> fmt::Debug for CommandTimeline<G> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CommandTimeline(..)")
    }
}

impl<G> CommandTimeline<G> {
    fn add_frame_page(&mut self) -> Result<(), CommandSummaryError> {
        let start = self
            .frame_pages
            .len()
            .checked_mul(COMMAND_FRAMES_PER_PAGE)
            .ok_or(CommandSummaryError::TimelineCapacity)?;
        let end = start
            .checked_add(COMMAND_FRAMES_PER_PAGE)
            .ok_or(CommandSummaryError::TimelineCapacity)?;
        u32::try_from(end).map_err(|_| CommandSummaryError::TimelineCapacity)?;
        let slots = std::iter::repeat_with(|| CommandFrameSlot {
            generation: 1,
            frame: None,
        })
        .take(COMMAND_FRAMES_PER_PAGE)
        .collect::<Box<[_]>>();
        self.frame_pages.push(CommandFramePage { slots });
        self.free_frames
            .extend((start..end).rev().map(|slot| CommandFrameKey {
                slot: slot as u32,
                generation: 1,
            }));
        Ok(())
    }

    fn frame_slot(&self, key: CommandFrameKey) -> Option<&CommandFrameSlot> {
        let slot = key.slot as usize;
        let page = self.frame_pages.get(slot / COMMAND_FRAMES_PER_PAGE)?;
        let slot = page.slots.get(slot % COMMAND_FRAMES_PER_PAGE)?;
        (slot.generation == key.generation && slot.frame.is_some()).then_some(slot)
    }

    fn frame(&self, key: CommandFrameKey) -> Option<&CommandTimelineFrame> {
        self.frame_slot(key)?.frame.as_ref()
    }

    fn allocate_frame(
        &mut self,
        frame: CommandTimelineFrame,
    ) -> Result<CommandFrameKey, CommandSummaryError> {
        if self.free_frames.is_empty() {
            self.add_frame_page()?;
        } else {
            self.frames_reused = self.frames_reused.saturating_add(1);
        }
        let key = self
            .free_frames
            .pop()
            .expect("frame page publication supplied free rows");
        let slot = key.slot as usize;
        let page = &mut self.frame_pages[slot / COMMAND_FRAMES_PER_PAGE];
        let slot = &mut page.slots[slot % COMMAND_FRAMES_PER_PAGE];
        debug_assert_eq!(slot.generation, key.generation);
        debug_assert!(slot.frame.is_none());
        slot.frame = Some(frame);
        Ok(key)
    }

    fn retire_frame(&mut self, key: CommandFrameKey) {
        let index = key.slot as usize;
        let page = &mut self.frame_pages[index / COMMAND_FRAMES_PER_PAGE];
        let slot = &mut page.slots[index % COMMAND_FRAMES_PER_PAGE];
        assert_eq!(
            slot.generation, key.generation,
            "retiring a live command frame"
        );
        assert!(slot.frame.take().is_some(), "retiring a live command frame");
        slot.generation = slot.generation.wrapping_add(1).max(1);
        self.free_frames.push(CommandFrameKey {
            slot: key.slot,
            generation: slot.generation,
        });
        self.frames_released = self.frames_released.saturating_add(1);
    }

    fn frame_position(&self, key: CommandFrameKey) -> Option<usize> {
        self.frame(key)?;
        self.accepted_frames
            .iter()
            .position(|candidate| *candidate == key)
    }

    fn release_frame(&mut self, mark: CommandTimelineMark) -> bool {
        if self.fork.is_some() {
            return false;
        }
        let Some(position) = self.frame_position(mark.frame) else {
            return false;
        };
        let key = self
            .accepted_frames
            .remove(position)
            .expect("validated frame position exists");
        self.retire_frame(key);
        true
    }

    fn release_prefix(&mut self, mark: CommandTimelineMark) -> Option<usize> {
        if self.fork.is_some()
            || !self.scalars.validates(mark.scalars)
            || !self.pending_input.validates(mark.pending_input)
        {
            return None;
        }
        let scalars = self.scalars.release_prefix(mark.scalars, drop)?;
        let pending = self
            .pending_input
            .release_prefix(mark.pending_input, drop)?;
        Some(scalars.saturating_add(pending))
    }

    fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(
                self.frame_pages
                    .iter()
                    .map(|page| {
                        page.slots
                            .len()
                            .saturating_mul(std::mem::size_of::<CommandFrameSlot>())
                    })
                    .sum::<usize>(),
            )
            .saturating_add(
                self.free_frames
                    .capacity()
                    .saturating_mul(std::mem::size_of::<CommandFrameKey>()),
            )
            .saturating_add(
                self.accepted_frames
                    .capacity()
                    .saturating_mul(std::mem::size_of::<CommandFrameKey>()),
            )
            .saturating_add(self.scalars.retained_bytes())
            .saturating_add(self.pending_input.retained_bytes())
    }

    fn live_frame_count(&self) -> usize {
        self.accepted_frames.len()
    }

    fn frame_capacity(&self) -> usize {
        self.frame_pages
            .len()
            .saturating_mul(COMMAND_FRAMES_PER_PAGE)
    }

    #[cfg(test)]
    fn source_cells_copied(&self) -> u64 {
        0
    }

    #[cfg(any(test, feature = "profiling"))]
    pub(crate) fn packed_journal_counters(&self) -> CommandTimelineCounters {
        let scalar = self.scalars.counters();
        let pending = self.pending_input.counters();
        CommandTimelineCounters {
            records: scalar.records.saturating_add(pending.records),
            record_bytes: scalar.record_bytes.saturating_add(pending.record_bytes),
            descriptor_publications: 0,
            coalesced_writes: self.coalesced_writes,
            ordered_events: self.ordered_events,
            chunks_acquired: scalar
                .chunks_acquired
                .saturating_add(pending.chunks_acquired),
            chunks_reused: scalar.chunks_reused.saturating_add(pending.chunks_reused),
            ..CommandTimelineCounters::default()
        }
    }

    fn retain(
        &mut self,
        attempt: AttemptMark,
        arenas: CommandArenaCursors,
        stacks: CommandStackCursors,
    ) -> Result<(CommandSnapshotCursor, CommandTimelineMark), CommandSummaryError> {
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
    ) -> Result<(CommandSnapshotCursor, CommandTimelineMark), CommandSummaryError> {
        let serial = self
            .next_serial
            .checked_add(1)
            .ok_or(CommandSummaryError::TimelineCapacity)?;
        self.next_serial = serial;
        let cursor = CommandSnapshotCursor::new(serial, arenas, stacks);
        self.scalars.warm_first_page();
        let scalars = self.scalars.mark();
        let pending_input = self.pending_input.mark();
        self.touched_scalars = 0;
        self.pending_input_touched = false;
        let frame = self.allocate_frame(CommandTimelineFrame {
            serial,
            cursor,
            attempt,
        })?;
        self.accepted_frames.push_back(frame);
        Ok((
            cursor,
            CommandTimelineMark {
                frame,
                scalars,
                pending_input,
            },
        ))
    }

    fn resolve(
        &self,
        cursor: CommandSnapshotCursor,
        mark: CommandTimelineMark,
    ) -> Option<AttemptMark> {
        let frame = self.frame(mark.frame)?;
        (self.scalars.validates(mark.scalars)
            && self.pending_input.validates(mark.pending_input)
            && frame.serial == cursor.command_journal()
            && frame.cursor == cursor)
            .then_some(frame.attempt)
    }

    fn has_live_frame(&self) -> bool {
        !self.accepted_frames.is_empty()
    }

    fn record_scalar(&mut self, slot: CommandScalarSlot, undo: CommandRootUndo<G>) {
        if !self.has_live_frame() {
            return;
        }
        let bit = slot.bit();
        if self.touched_scalars & bit != 0 {
            self.coalesced_writes = self.coalesced_writes.saturating_add(1);
            return;
        }
        self.touched_scalars |= bit;
        self.scalars.append(undo);
    }

    pub(crate) fn record_name_in_progress(&mut self, old: bool) {
        self.record_scalar(
            CommandScalarSlot::NameInProgress,
            CommandRootUndo::NameInProgress(old),
        );
    }

    pub(crate) fn record_afterassignment(&mut self, old: Option<crate::state::CommandPayload<G>>) {
        self.record_scalar(
            CommandScalarSlot::Afterassignment,
            CommandRootUndo::Afterassignment(old),
        );
    }

    pub(crate) fn record_cumulative_expansions(&mut self, old: u64) {
        self.record_scalar(
            CommandScalarSlot::CumulativeExpansions,
            CommandRootUndo::CumulativeExpansions(old),
        );
    }

    pub(crate) fn record_align_state(&mut self, old: i32) {
        self.record_scalar(
            CommandScalarSlot::AlignState,
            CommandRootUndo::AlignState(old),
        );
    }

    pub(crate) fn record_next_input_level_identity(&mut self, old: u64) {
        self.record_scalar(
            CommandScalarSlot::NextInputLevelIdentity,
            CommandRootUndo::NextInputLevelIdentity(old),
        );
    }

    pub(crate) fn record_next_source_identity(&mut self, old: u64) {
        self.record_scalar(
            CommandScalarSlot::NextSourceIdentity,
            CommandRootUndo::NextSourceIdentity(old),
        );
    }

    pub(crate) fn record_retained_file_line_number(&mut self, old: i32) {
        self.record_scalar(
            CommandScalarSlot::RetainedFileLineNumber,
            CommandRootUndo::RetainedFileLineNumber(old),
        );
    }

    pub(crate) fn record_force_eof(&mut self, old: bool) {
        self.record_scalar(CommandScalarSlot::ForceEof, CommandRootUndo::ForceEof(old));
    }

    pub(crate) fn record_pending_input_open(&mut self, old: Option<crate::ScannedFileName>) {
        if !self.has_live_frame() {
            return;
        }
        if self.pending_input_touched {
            self.coalesced_writes = self.coalesced_writes.saturating_add(1);
            return;
        }
        self.pending_input_touched = true;
        self.pending_input.append(PendingInputUndo(old));
    }

    pub(crate) fn record_expansion_diagnostic_push(&mut self) {
        if self.has_live_frame() {
            // Ordered diagnostic pushes are the deliberately non-coalescible
            // class: every payload must survive reverse rollback and forward
            // redo in its original vector order.
            self.scalars
                .append(CommandRootUndo::ExpansionDiagnosticPush(None));
            self.ordered_events = self.ordered_events.saturating_add(1);
        }
    }

    fn restore_roots(
        &mut self,
        mark: CommandTimelineMark,
        roots: &mut CommandStateRoots<G>,
    ) -> bool {
        let Some(position) = self.frame_position(mark.frame) else {
            return false;
        };
        if self.fork.is_some()
            || !self.scalars.validates(mark.scalars)
            || !self.pending_input.validates(mark.pending_input)
        {
            return false;
        }
        self.scalars
            .restore(mark.scalars, |inverse| inverse.swap(roots));
        self.pending_input.restore(mark.pending_input, |inverse| {
            std::mem::swap(&mut inverse.0, &mut roots.pending_input_open);
        });
        let mut suffix = self.accepted_frames.split_off(position + 1);
        while let Some(frame) = suffix.pop_front() {
            self.retire_frame(frame);
        }
        self.touched_scalars = 0;
        self.pending_input_touched = false;
        true
    }

    fn can_begin_checkpoint_candidate(&self, mark: CommandTimelineMark) -> bool {
        self.fork.is_none()
            && self.frame_position(mark.frame).is_some()
            && self.scalars.validates(mark.scalars)
            && self.pending_input.validates(mark.pending_input)
    }

    fn begin_checkpoint_candidate(
        &mut self,
        mark: CommandTimelineMark,
        roots: &mut CommandStateRoots<G>,
    ) {
        assert!(
            self.can_begin_checkpoint_candidate(mark),
            "command candidate cursor was prevalidated"
        );
        self.scalars
            .begin_checkpoint_candidate(mark.scalars, |inverse| inverse.swap(roots));
        self.pending_input
            .begin_checkpoint_candidate(mark.pending_input, |inverse| {
                std::mem::swap(&mut inverse.0, &mut roots.pending_input_open);
            });
        let position = self
            .frame_position(mark.frame)
            .expect("prevalidated command mark begins the sole fork");
        let detached = self.accepted_frames.split_off(position + 1);
        self.fork = Some(CommandTimelineFork {
            prefix_len: position + 1,
            detached,
        });
        self.touched_scalars = 0;
        self.pending_input_touched = false;
    }

    fn reject_checkpoint_candidate(&mut self, roots: &mut CommandStateRoots<G>) {
        let fork = self
            .fork
            .take()
            .expect("command rejection requires a candidate fork");
        self.scalars
            .reject_checkpoint_candidate(|inverse| inverse.swap(roots));
        self.pending_input.reject_checkpoint_candidate(|inverse| {
            std::mem::swap(&mut inverse.0, &mut roots.pending_input_open);
        });
        let mut candidate = self.accepted_frames.split_off(fork.prefix_len);
        while let Some(frame) = candidate.pop_front() {
            self.retire_frame(frame);
        }
        self.accepted_frames.extend(fork.detached);
        self.touched_scalars = 0;
        self.pending_input_touched = false;
    }

    fn accept_checkpoint_candidate(&mut self) {
        let mut fork = self
            .fork
            .take()
            .expect("command acceptance requires a candidate fork");
        self.scalars.accept_checkpoint_candidate();
        self.pending_input.accept_checkpoint_candidate();
        while let Some(frame) = fork.detached.pop_front() {
            self.retire_frame(frame);
        }
        self.touched_scalars = 0;
        self.pending_input_touched = false;
    }
}

/// Structural evidence for the packed root journal.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[doc(hidden)]
pub struct CommandTimelineCounters {
    pub records: u64,
    pub record_bytes: u64,
    pub descriptor_publications: u64,
    pub coalesced_writes: u64,
    pub ordered_events: u64,
    pub chunks_acquired: u64,
    pub chunks_reused: u64,
    pub logical_payload_admissions: u64,
    pub full_frame_history_clones: u64,
    pub logical_records: u64,
    pub logical_record_bytes: u64,
    pub logical_coalesced_mutations: u64,
    pub displaced_payloads: u64,
    pub displaced_reuses: u64,
}

/// Command-owner evidence produced when aggregate checkpoint history releases
/// one summary.
///
/// `JobStart` is frozen outside the live command owner. Releasing an interior
/// summary can therefore advance the journal and logical-stack floors to the
/// earliest surviving restart root. This receipt reports both returned prefix
/// chunks and authoritative physical frame occupancy.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[doc(hidden)]
pub struct CommandCheckpointReleaseReceipt {
    timeline_frames_live: usize,
    timeline_frame_capacity: usize,
    timeline_frames_released: u64,
    command_journal_chunks_released: usize,
    logical_stack_chunks_released: usize,
}

impl CommandCheckpointReleaseReceipt {
    #[must_use]
    pub const fn timeline_frames_live(self) -> usize {
        self.timeline_frames_live
    }

    #[must_use]
    pub const fn timeline_frame_capacity(self) -> usize {
        self.timeline_frame_capacity
    }

    #[must_use]
    pub const fn timeline_frames_released(self) -> u64 {
        self.timeline_frames_released
    }

    #[must_use]
    pub const fn command_journal_chunks_released(self) -> usize {
        self.command_journal_chunks_released
    }

    #[must_use]
    pub const fn logical_stack_chunks_released(self) -> usize {
        self.logical_stack_chunks_released
    }
}

/// Coarse generation plus stable physical-timeline identity retained by one
/// command snapshot or summary.
///
/// The timeline itself remains solely owned by the live command state. Marks
/// carry only its scalar identity; no checkpoint aliases mutable command
/// storage.
pub struct CommandGenerationOwner<G> {
    generation: GenerationOwner<G>,
    timeline_owner: u64,
    attempt: AttemptMark,
    timeline: CommandTimelineMark,
}

impl<G> Clone for CommandGenerationOwner<G> {
    fn clone(&self) -> Self {
        Self {
            generation: self.generation.clone(),
            timeline_owner: self.timeline_owner,
            attempt: self.attempt,
            timeline: self.timeline,
        }
    }
}

impl<G> fmt::Debug for CommandGenerationOwner<G> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CommandGenerationOwner(..)")
    }
}

impl<G> CommandGenerationOwner<G> {
    fn new(
        generation: GenerationOwner<G>,
        timeline_owner: u64,
        attempt: AttemptMark,
        timeline: CommandTimelineMark,
    ) -> Self {
        Self {
            generation,
            timeline_owner,
            attempt,
            timeline,
        }
    }

    pub(crate) fn addresses(&self, generation: &GenerationOwner<G>, timeline_owner: u64) -> bool {
        self.generation.same_generation(generation) && self.timeline_owner == timeline_owner
    }

    fn checkpoint_owner_id(&self) -> tex_state::CheckpointOwnerId {
        self.generation.checkpoint_owner_id()
    }

    fn addresses_cursor(&self, cursor: CommandSnapshotCursor) -> bool {
        cursor.command_journal() != 0
    }
}

/// Watermarks for command-owned append-only storage.
///
/// Each coordinate is an exclusive row count. The corresponding arena
/// validates the coordinate against its own generation before truncation.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct CommandArenaCursors {
    input_rows: PackedJournalMark,
    input_words: PackedJournalMark,
    parameter_words: PackedJournalMark,
    builder_words: PackedJournalMark,
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
            input_rows: PackedJournalMark::synthetic(input_rows),
            input_words: PackedJournalMark::synthetic(input_words),
            parameter_words: PackedJournalMark::synthetic(parameter_words),
            builder_words: PackedJournalMark::synthetic(builder_words),
            attempt_rows,
        }
    }

    const fn live(
        input_rows: PackedJournalMark,
        input_words: PackedJournalMark,
        parameter_words: PackedJournalMark,
        builder_words: PackedJournalMark,
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
        self.input_rows.record_count()
    }

    #[must_use]
    pub const fn input_words(self) -> u32 {
        self.input_words.record_count()
    }

    #[must_use]
    pub const fn parameter_words(self) -> u32 {
        self.parameter_words.record_count()
    }

    #[must_use]
    pub const fn builder_words(self) -> u32 {
        self.builder_words.record_count()
    }

    const fn input_rows_mark(self) -> PackedJournalMark {
        self.input_rows
    }

    const fn input_words_mark(self) -> PackedJournalMark {
        self.input_words
    }

    const fn parameter_words_mark(self) -> PackedJournalMark {
        self.parameter_words
    }

    const fn builder_words_mark(self) -> PackedJournalMark {
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
    alignment_undo: PackedJournalMark,
    suspended_alignment_depth: u32,
    suspended_alignment_undo: PackedJournalMark,
    replay_depth: u32,
    diagnostic_count: u32,
    group_payload_depth: u32,
    aftergroup_payload_count: u32,
    aftergroup_payload_undo: PackedJournalMark,
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
    const fn alignment_undo(self) -> PackedJournalMark {
        self.alignment_undo
    }

    #[must_use]
    const fn suspended_alignment_depth(self) -> u32 {
        self.suspended_alignment_depth
    }

    #[must_use]
    const fn suspended_alignment_undo(self) -> PackedJournalMark {
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
    const fn aftergroup_payload_undo(self) -> PackedJournalMark {
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
    pub(crate) fn addresses(&self, generation: &GenerationOwner<G>, timeline_owner: u64) -> bool {
        self.generation.addresses(generation, timeline_owner)
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

    fn addresses(&self, generation: &GenerationOwner<G>, timeline_owner: u64) -> bool {
        self.generation.addresses(generation, timeline_owner)
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
    timeline_owner: u64,
    timeline: CommandTimelineMark,
    cursor: CommandSnapshotCursor,
    attempt: AttemptMark,
    brand: PhantomData<fn(&G) -> &G>,
}

impl<G> CommandState<G> {
    /// Runs only the command-family fork for its standalone allocation gate.
    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    pub fn profile_fork_summary(
        self,
        summary: &CommandSummary<G>,
        universe: &Universe<G>,
    ) -> Result<Self, CommandRestoreError> {
        Self::fork_summary(self, summary, universe, universe)
    }

    /// Moves the returned command-root loan into one candidate and restores the
    /// accepted summary's exact logical prefix. Runtime scratch and attempt
    /// lanes begin quiescent.
    #[doc(hidden)]
    pub fn fork_summary(
        mut command: Self,
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
            .addresses(&source_generation, command.timeline.owner)
            || !source_generation.same_generation(&destination_generation)
        {
            return Err(CommandRestoreError::ForeignGeneration);
        }
        let restore = command.resolve_restore(summary.generation(), summary.cursor())?;
        if !restore.attempt.is_empty() {
            return Err(CommandRestoreError::InvalidCursor);
        }
        command
            .profile()
            .validate_fingerprint(
                CommandProfileBoundary::Summary,
                CommandProfileFingerprint::from_u64(summary.profile_fingerprint()),
            )
            .map_err(CommandRestoreError::Profile)?;
        command.begin_prepared_candidate(restore);
        Ok(command)
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
        self.timeline = CommandTimeline::default();
        self.attempt = crate::CommandAttempt::default();
        self.scratch = crate::execution_scratch::ExecutionScratch::default();
        self.active_attempt_operation = None;
    }

    fn checkpoint_arenas(
        &mut self,
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
        Ok(CommandArenaCursors::live(
            input.undo,
            parameters.undo,
            conditions.undo,
            groups.undo,
            attempt_rows,
        ))
    }

    fn checkpoint_stacks(&mut self) -> Result<CommandStackCursors, CommandSummaryError> {
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
        &mut self,
        attempt: AttemptMark,
    ) -> Result<(CommandSnapshotCursor, CommandTimelineMark), CommandSummaryError> {
        let arenas = self.checkpoint_arenas(attempt)?;
        let stacks = self.checkpoint_stacks()?;
        self.timeline.retain(attempt, arenas, stacks)
    }

    fn resolve_restore(
        &self,
        owner: &CommandGenerationOwner<G>,
        cursor: CommandSnapshotCursor,
    ) -> Result<PreparedCommandRestore<G>, CommandRestoreError> {
        if !owner.addresses_cursor(cursor) || owner.timeline_owner != self.timeline.owner {
            return Err(CommandRestoreError::InvalidCursor);
        }
        let attempt = self
            .timeline
            .resolve(cursor, owner.timeline)
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
                undo: arenas.input_rows_mark(),
            })
            && self
                .parameters
                .activations
                .validates(crate::timeline::LogicalStackMark {
                    top: cursor.stacks().parameter_depth(),
                    undo: arenas.input_words_mark(),
                })
            && self
                .conditions
                .frames
                .validates(crate::timeline::LogicalStackMark {
                    top: cursor.stacks().condition_depth(),
                    undo: arenas.parameter_words_mark(),
                })
            && self
                .group_payloads
                .validates(crate::timeline::LogicalStackMark {
                    top: cursor.stacks().group_payload_depth(),
                    undo: arenas.builder_words_mark(),
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
            timeline_owner: self.timeline.owner,
            timeline: owner.timeline,
            cursor,
            attempt,
            brand: PhantomData,
        })
    }

    fn restore_logical_stacks(&mut self, cursor: CommandSnapshotCursor) -> bool {
        let stacks = cursor.stacks();
        let arenas = cursor.arenas();
        self.input
            .levels
            .restore(crate::timeline::LogicalStackMark {
                top: stacks.input_depth(),
                undo: arenas.input_rows_mark(),
            })
            && self
                .parameters
                .activations
                .restore(crate::timeline::LogicalStackMark {
                    top: stacks.parameter_depth(),
                    undo: arenas.input_words_mark(),
                })
            && self
                .conditions
                .frames
                .restore(crate::timeline::LogicalStackMark {
                    top: stacks.condition_depth(),
                    undo: arenas.parameter_words_mark(),
                })
            && self
                .group_payloads
                .restore(crate::timeline::LogicalStackMark {
                    top: stacks.group_payload_depth(),
                    undo: arenas.builder_words_mark(),
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

    fn begin_prepared_candidate(&mut self, restore: PreparedCommandRestore<G>) {
        debug_assert_eq!(restore.timeline_owner, self.timeline.owner);
        let stacks = restore.cursor.stacks();
        let arenas = restore.cursor.arenas();
        self.timeline
            .begin_checkpoint_candidate(restore.timeline, &mut self.roots);
        self.input
            .levels
            .begin_checkpoint_candidate(crate::timeline::LogicalStackMark {
                top: stacks.input_depth(),
                undo: arenas.input_rows_mark(),
            });
        self.parameters
            .activations
            .begin_checkpoint_candidate(crate::timeline::LogicalStackMark {
                top: stacks.parameter_depth(),
                undo: arenas.input_words_mark(),
            });
        self.conditions
            .frames
            .begin_checkpoint_candidate(crate::timeline::LogicalStackMark {
                top: stacks.condition_depth(),
                undo: arenas.parameter_words_mark(),
            });
        self.group_payloads
            .begin_checkpoint_candidate(crate::timeline::LogicalStackMark {
                top: stacks.group_payload_depth(),
                undo: arenas.builder_words_mark(),
            });
        self.aftergroup_payloads
            .begin_checkpoint_candidate(crate::timeline::LogicalStackMark {
                top: stacks.aftergroup_payload_count(),
                undo: stacks.aftergroup_payload_undo(),
            });
        self.alignment
            .align_stack
            .begin_checkpoint_candidate(crate::timeline::LogicalStackMark {
                top: stacks.alignment_depth(),
                undo: stacks.alignment_undo(),
            });
        self.alignment
            .suspended
            .begin_checkpoint_candidate(crate::timeline::LogicalStackMark {
                top: stacks.suspended_alignment_depth(),
                undo: stacks.suspended_alignment_undo(),
            });
        self.attempt
            .arena_mut()
            .truncate(restore.attempt)
            .expect("prevalidated command candidate attempt mark");
    }

    /// Consumes the sole physical command owner and opens its prevalidated
    /// accepted/detached-prior/current transaction at `restore`.
    #[doc(hidden)]
    pub fn begin_checkpoint_candidate(mut self, restore: PreparedCommandRestore<G>) -> Self {
        self.begin_prepared_candidate(restore);
        self
    }

    #[doc(hidden)]
    pub fn reject_checkpoint_candidate(&mut self) {
        self.alignment.suspended.reject_checkpoint_candidate();
        self.alignment.align_stack.reject_checkpoint_candidate();
        self.aftergroup_payloads.reject_checkpoint_candidate();
        self.group_payloads.reject_checkpoint_candidate();
        self.conditions.frames.reject_checkpoint_candidate();
        self.parameters.activations.reject_checkpoint_candidate();
        self.input.levels.reject_checkpoint_candidate();
        self.timeline.reject_checkpoint_candidate(&mut self.roots);
    }

    #[doc(hidden)]
    pub fn accept_checkpoint_candidate(&mut self) {
        self.timeline.accept_checkpoint_candidate();
        self.input.levels.accept_checkpoint_candidate();
        self.parameters.activations.accept_checkpoint_candidate();
        self.conditions.frames.accept_checkpoint_candidate();
        self.group_payloads.accept_checkpoint_candidate();
        self.aftergroup_payloads.accept_checkpoint_candidate();
        self.alignment.align_stack.accept_checkpoint_candidate();
        self.alignment.suspended.accept_checkpoint_candidate();
    }

    /// Captures one exact in-session command position as bounded marks. The
    /// live root remains exclusively mutable and no accumulated payload moves.
    pub fn snapshot(
        &mut self,
        universe: &Universe<G>,
    ) -> Result<CommandStateSnapshot<G>, CommandSummaryError> {
        let generation = universe
            .generation_owner()
            .map_err(|_| CommandSummaryError::GenerationUnavailable)?;
        let attempt = self.attempt.arena().mark();
        let (cursor, timeline) = self.retain_cursor(attempt)?;
        Ok(CommandStateSnapshot::new(
            CommandGenerationOwner::new(generation, self.timeline.owner, attempt, timeline),
            cursor,
        ))
    }

    /// Captures a move-only rollback point inside one live operation.
    ///
    /// This is the synchronous nested-episode counterpart to [`Self::snapshot`]:
    /// it may retain an open attempt mark, but neither the mark nor its coarse
    /// generation owner can escape through a clone, summary, or detached DTO.
    pub fn transient_snapshot(
        &mut self,
        universe: &Universe<G>,
    ) -> Result<TransientCommandSnapshot<G>, CommandSummaryError> {
        let generation = universe
            .generation_owner()
            .map_err(|_| CommandSummaryError::GenerationUnavailable)?;
        let attempt = self.attempt.arena().mark();
        let arenas = self.checkpoint_arenas(attempt)?;
        let stacks = self.checkpoint_stacks()?;
        let (cursor, timeline) = self.timeline.retain_transient(attempt, arenas, stacks)?;
        Ok(TransientCommandSnapshot::new(
            CommandGenerationOwner::new(generation, self.timeline.owner, attempt, timeline),
            cursor,
        ))
    }

    /// Publishes a bounded named-boundary summary. Ordinary mutation remains
    /// direct before and after capture; accumulated command roots are not
    /// cloned.
    pub fn publish_summary(
        &mut self,
        universe: &Universe<G>,
    ) -> Result<CommandSummary<G>, CommandSummaryError> {
        self.validate_summary_quiescence()?;
        let generation = universe
            .generation_owner()
            .map_err(|_| CommandSummaryError::GenerationUnavailable)?;
        let attempt = self.attempt.arena().mark();
        let (cursor, timeline) = self.retain_cursor(attempt)?;
        let root_source_anchor = self.input.levels.iter().find_map(|level| {
            let crate::input::InputLevel::Source(source) = level else {
                return None;
            };
            Some(source.cursor.next_physical_offset)
        });
        let retained_owner_bytes = self
            .roots
            .retained_bytes()
            .saturating_add(self.timeline.retained_bytes());
        Ok(CommandSummary::new(
            CommandGenerationOwner::new(generation, self.timeline.owner, attempt, timeline),
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
        if !summary
            .generation()
            .addresses(&generation, self.timeline.owner)
        {
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
        if restore.timeline_owner != self.timeline.owner {
            return Err(CommandRestoreError::ForeignGeneration);
        }
        self.attempt
            .arena()
            .validate_mark(restore.attempt)
            .map_err(|_| CommandRestoreError::InvalidCursor)?;
        let restored = self
            .timeline
            .restore_roots(restore.timeline, &mut self.roots);
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

    /// Validates one aggregate checkpoint-history release against this sole
    /// physical command owner and reports its exact storage effect.
    ///
    /// `oldest_retained` is also validated when present so a stale or foreign
    /// aggregate low-water cannot be published. JobStart is frozen
    /// independently, so the chosen ordinary low-water becomes the journals'
    /// logical base and every whole prefix chunk is returned to its pool.
    #[doc(hidden)]
    pub fn release_checkpoint_summary(
        &mut self,
        released: &CommandSummary<G>,
        oldest_retained: Option<&CommandSummary<G>>,
    ) -> Result<CommandCheckpointReleaseReceipt, CommandRestoreError> {
        if released.generation().timeline_owner != self.timeline.owner
            || oldest_retained
                .is_some_and(|oldest| oldest.generation().timeline_owner != self.timeline.owner)
        {
            return Err(CommandRestoreError::ForeignGeneration);
        }
        self.resolve_restore(released.generation(), released.cursor())?;
        if let Some(oldest) = oldest_retained {
            self.resolve_restore(oldest.generation(), oldest.cursor())?;
        }
        let floor = oldest_retained.unwrap_or(released);
        if !self.timeline.release_frame(released.generation().timeline) {
            return Err(CommandRestoreError::InvalidCursor);
        }
        let command_journal_chunks_released = self
            .timeline
            .release_prefix(floor.generation().timeline)
            .ok_or(CommandRestoreError::InvalidCursor)?;
        let cursor = floor.cursor();
        let arenas = cursor.arenas();
        let stacks = cursor.stacks();
        let mut logical_stack_chunks_released = 0usize;
        macro_rules! release_stack_prefix {
            ($stack:expr, $top:expr, $undo:expr) => {
                logical_stack_chunks_released = logical_stack_chunks_released.saturating_add(
                    $stack
                        .release_prefix(crate::timeline::LogicalStackMark {
                            top: $top,
                            undo: $undo,
                        })
                        .ok_or(CommandRestoreError::InvalidCursor)?,
                );
            };
        }
        release_stack_prefix!(
            self.input.levels,
            stacks.input_depth(),
            arenas.input_rows_mark()
        );
        release_stack_prefix!(
            self.parameters.activations,
            stacks.parameter_depth(),
            arenas.input_words_mark()
        );
        release_stack_prefix!(
            self.conditions.frames,
            stacks.condition_depth(),
            arenas.parameter_words_mark()
        );
        release_stack_prefix!(
            self.group_payloads,
            stacks.group_payload_depth(),
            arenas.builder_words_mark()
        );
        release_stack_prefix!(
            self.aftergroup_payloads,
            stacks.aftergroup_payload_count(),
            stacks.aftergroup_payload_undo()
        );
        release_stack_prefix!(
            self.alignment.align_stack,
            stacks.alignment_depth(),
            stacks.alignment_undo()
        );
        release_stack_prefix!(
            self.alignment.suspended,
            stacks.suspended_alignment_depth(),
            stacks.suspended_alignment_undo()
        );
        Ok(CommandCheckpointReleaseReceipt {
            timeline_frames_live: self.timeline.live_frame_count(),
            timeline_frame_capacity: self.timeline.frame_capacity(),
            timeline_frames_released: self.timeline.frames_released,
            command_journal_chunks_released,
            logical_stack_chunks_released,
        })
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
        if !snapshot.addresses(&generation, self.timeline.owner) {
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
        if !snapshot.addresses(&generation, self.timeline.owner) {
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
