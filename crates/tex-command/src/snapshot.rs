//! Bounded in-session command snapshots and named command summaries.
//!
//! A retained value owns one coarse generation/timeline capability and a fixed
//! tuple of scalar cursors. Checkpoint publication appends a reusable frame;
//! it never clones the aggregate command roots. Generation-owned logical
//! stacks and compact scalar undo restore the selected prefix before the sole
//! current command borrower resumes execution.

use core::fmt;
use core::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};

use tex_state::{GenerationOwner, Universe};

use crate::attempt::AttemptMark;
use crate::processor::ScannerStatus;
use crate::profile::{CommandProfileBoundary, CommandProfileFingerprint, CommandProfileMismatch};
use crate::scalar_journal::{PackedJournal, PackedJournalMark};
use crate::state::{CommandState, CommandStateRoots};

mod boundary;

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
    fresh_frames: Vec<CommandFrameKey>,
    /// Newest whole retired chain. Its tail names the next older chain.
    reusable_frames: Option<CommandFrameChain>,
    frame_head: Option<CommandFrameKey>,
    frame_tail: Option<CommandFrameKey>,
    occupied_frames: usize,
    scalars: PackedJournal<CommandRootUndo<G>, 128>,
    pending_input: PackedJournal<PendingInputUndo, 8>,
    touched_scalars: u16,
    pending_input_touched: bool,
    coalesced_writes: u64,
    ordered_events: u64,
    #[cfg(feature = "profiling")]
    alignment_delivery_journal_attempts: u64,
    fork: Option<CommandTimelineFork>,
    frames_released: u64,
    frames_reused: u64,
    frame_chain_transfers: u64,
    frame_reuse_link_visits: u64,
    frame_reuse_visits: u64,
    frame_reuse_incarnations: u64,
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
    previous: Option<CommandFrameKey>,
    next: Option<CommandFrameKey>,
    /// Installed while this row is a prospective chain tail, before the
    /// chain can cross a settlement boundary.
    reusable_next: Option<CommandFrameChain>,
}

struct CommandFramePage {
    slots: Box<[CommandFrameSlot]>,
}

#[derive(Clone, Copy, Debug)]
struct CommandTimelineFrame {
    serial: u32,
    rollback: CommandRollbackCoordinates,
}

/// Exact marks into the command owner's reversible logical stacks.
///
/// These coordinates live once in the timeline row that owns them. The
/// snapshot cursor carries only the row serial needed to reject a mismatched
/// owner/cursor pair; it does not repeat private stack lengths or journal
/// positions.
#[derive(Clone, Copy, Debug)]
struct CommandRollbackCoordinates {
    input: crate::input::InputStackMark,
    parameters: crate::timeline::LogicalStackMark,
    conditions: crate::timeline::LogicalStackMark,
    groups: crate::timeline::LogicalStackMark,
    aftergroups: crate::timeline::LogicalStackMark,
    alignment: crate::timeline::LogicalStackMark,
    suspended_alignment: crate::timeline::LogicalStackMark,
}

#[derive(Clone, Copy)]
struct CommandTimelineMark {
    frame: CommandFrameKey,
    scalars: PackedJournalMark,
    pending_input: PackedJournalMark,
}

#[derive(Clone, Copy, Debug)]
struct CommandFrameChain {
    head: CommandFrameKey,
    tail: CommandFrameKey,
}

struct CommandTimelineFork {
    prefix_tail: CommandFrameKey,
    detached: Option<CommandFrameChain>,
    candidate: Option<CommandFrameChain>,
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
        }
    }
}

impl<G> Default for CommandTimeline<G> {
    fn default() -> Self {
        Self {
            next_serial: 0,
            owner: NEXT_COMMAND_TIMELINE_OWNER.fetch_add(1, Ordering::Relaxed),
            frame_pages: Vec::new(),
            fresh_frames: Vec::new(),
            reusable_frames: None,
            frame_head: None,
            frame_tail: None,
            occupied_frames: 0,
            scalars: PackedJournal::default(),
            pending_input: PackedJournal::default(),
            touched_scalars: 0,
            pending_input_touched: false,
            coalesced_writes: 0,
            ordered_events: 0,
            #[cfg(feature = "profiling")]
            alignment_delivery_journal_attempts: 0,
            fork: None,
            frames_released: 0,
            frames_reused: 0,
            frame_chain_transfers: 0,
            frame_reuse_link_visits: 0,
            frame_reuse_visits: 0,
            frame_reuse_incarnations: 0,
        }
    }
}

impl<G> CommandTimeline<G> {
    pub(crate) const fn owner_id(&self) -> u64 {
        self.owner
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
            previous: None,
            next: None,
            reusable_next: None,
        })
        .take(COMMAND_FRAMES_PER_PAGE)
        .collect::<Box<[_]>>();
        self.frame_pages.push(CommandFramePage { slots });
        self.fresh_frames
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

    fn frame_slot_mut(&mut self, key: CommandFrameKey) -> Option<&mut CommandFrameSlot> {
        let slot = key.slot as usize;
        let page = self.frame_pages.get_mut(slot / COMMAND_FRAMES_PER_PAGE)?;
        let slot = page.slots.get_mut(slot % COMMAND_FRAMES_PER_PAGE)?;
        (slot.generation == key.generation && slot.frame.is_some()).then_some(slot)
    }

    fn next_frame(&self, key: CommandFrameKey) -> Option<CommandFrameKey> {
        self.frame_slot(key).and_then(|slot| slot.next)
    }

    fn allocate_frame(
        &mut self,
        frame: CommandTimelineFrame,
    ) -> Result<CommandFrameKey, CommandSummaryError> {
        let (key, reused) = if let Some(key) = self.take_reusable_frame() {
            self.frames_reused = self.frames_reused.saturating_add(1);
            (key, true)
        } else {
            if self.fresh_frames.is_empty() {
                self.add_frame_page()?;
            }
            let key = self
                .fresh_frames
                .pop()
                .expect("frame page publication supplied fresh rows");
            self.frames_reused = self
                .frames_reused
                .saturating_add(u64::from(key.generation != 1));
            (key, false)
        };
        let slot = key.slot as usize;
        let page = &mut self.frame_pages[slot / COMMAND_FRAMES_PER_PAGE];
        let slot = &mut page.slots[slot % COMMAND_FRAMES_PER_PAGE];
        debug_assert_eq!(slot.generation, key.generation);
        debug_assert!(slot.frame.is_none());
        slot.frame = Some(frame);
        slot.previous = self.frame_tail;
        slot.next = None;
        slot.reusable_next = None;
        if let Some(tail) = self.frame_tail {
            let tail = self
                .frame_slot_mut(tail)
                .expect("command frame tail remains live");
            tail.next = Some(key);
            tail.reusable_next = None;
        } else {
            self.frame_head = Some(key);
        }
        self.frame_tail = Some(key);
        self.occupied_frames = self.occupied_frames.saturating_add(usize::from(!reused));
        let candidate = if let Some(fork) = &mut self.fork {
            match &mut fork.candidate {
                Some(candidate) => {
                    candidate.tail = key;
                }
                None => {
                    fork.candidate = Some(CommandFrameChain {
                        head: key,
                        tail: key,
                    });
                }
            }
            fork.candidate
        } else {
            None
        };
        if let Some(candidate) = candidate {
            self.prepare_reusable_chain(candidate);
        }
        Ok(key)
    }

    fn take_reusable_frame(&mut self) -> Option<CommandFrameKey> {
        let mut chain = self.reusable_frames?;
        let stale = chain.head;
        let index = stale.slot as usize;
        let slot = &mut self.frame_pages[index / COMMAND_FRAMES_PER_PAGE].slots
            [index % COMMAND_FRAMES_PER_PAGE];
        assert_eq!(slot.generation, stale.generation);
        assert!(slot.frame.take().is_some());
        let next = slot.next;
        let reusable_next = slot.reusable_next;
        slot.previous = None;
        slot.next = None;
        slot.reusable_next = None;
        slot.generation = slot.generation.wrapping_add(1).max(1);
        let fresh = CommandFrameKey {
            slot: stale.slot,
            generation: slot.generation,
        };
        if chain.head == chain.tail {
            self.reusable_frames = reusable_next;
        } else {
            chain.head = next.expect("reusable frame chain reaches its tail");
            self.reusable_frames = Some(chain);
        }
        self.refresh_detached_reusable_link();
        self.frame_reuse_visits = self.frame_reuse_visits.saturating_add(1);
        self.frame_reuse_incarnations = self.frame_reuse_incarnations.saturating_add(1);
        Some(fresh)
    }

    fn chain_from(&self, head: Option<CommandFrameKey>) -> Option<CommandFrameChain> {
        let head = head?;
        let tail = self.frame_tail.expect("nonempty suffix has a tail");
        Some(CommandFrameChain { head, tail })
    }

    fn retire_chain(&mut self, chain: Option<CommandFrameChain>) {
        let Some(chain) = chain else {
            return;
        };
        self.reusable_frames = Some(chain);
        self.frame_chain_transfers = self.frame_chain_transfers.saturating_add(1);
    }

    fn prepare_reusable_chain(&mut self, chain: CommandFrameChain) {
        // Settlement must only move `chain`. Install the link to the older
        // reusable owner while the tail is already being selected or
        // published, then refresh it if lazy allocation advances that owner.
        let reusable = self.reusable_frames;
        let index = chain.tail.slot as usize;
        let slot = &mut self.frame_pages[index / COMMAND_FRAMES_PER_PAGE].slots
            [index % COMMAND_FRAMES_PER_PAGE];
        assert_eq!(slot.generation, chain.tail.generation);
        assert!(slot.frame.is_some());
        slot.reusable_next = reusable;
        self.frame_reuse_link_visits = self.frame_reuse_link_visits.saturating_add(1);
    }

    fn refresh_detached_reusable_link(&mut self) {
        let Some(detached) = self.fork.as_ref().and_then(|fork| fork.detached) else {
            return;
        };
        self.prepare_reusable_chain(detached);
    }

    fn release_frame(&mut self, mark: CommandTimelineMark) -> bool {
        if self.fork.is_some() || self.frame(mark.frame).is_none() {
            return false;
        }
        let (previous, next) = {
            let slot = self
                .frame_slot(mark.frame)
                .expect("validated command frame remains live");
            (slot.previous, slot.next)
        };
        if let Some(previous) = previous {
            self.frame_slot_mut(previous)
                .expect("released frame predecessor remains live")
                .next = next;
        } else {
            self.frame_head = next;
        }
        if let Some(next) = next {
            self.frame_slot_mut(next)
                .expect("released frame successor remains live")
                .previous = previous;
        } else {
            self.frame_tail = previous;
        }
        self.frame_slot_mut(mark.frame)
            .expect("released command frame remains live")
            .previous = None;
        self.frame_slot_mut(mark.frame)
            .expect("released command frame remains live")
            .next = None;
        let index = mark.frame.slot as usize;
        let slot = &mut self.frame_pages[index / COMMAND_FRAMES_PER_PAGE].slots
            [index % COMMAND_FRAMES_PER_PAGE];
        assert_eq!(slot.generation, mark.frame.generation);
        assert!(slot.frame.take().is_some());
        slot.previous = None;
        slot.next = None;
        slot.reusable_next = None;
        slot.generation = slot.generation.wrapping_add(1).max(1);
        self.fresh_frames.push(CommandFrameKey {
            slot: mark.frame.slot,
            generation: slot.generation,
        });
        self.occupied_frames = self.occupied_frames.saturating_sub(1);
        self.frames_released = self.frames_released.saturating_add(1);
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
                self.fresh_frames
                    .capacity()
                    .saturating_mul(std::mem::size_of::<CommandFrameKey>()),
            )
            .saturating_add(self.scalars.retained_bytes())
            .saturating_add(self.pending_input.retained_bytes())
    }

    fn live_frame_count(&self) -> usize {
        self.occupied_frames
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
            selected_rewind_records: scalar
                .selected_rewind_records
                .saturating_add(pending.selected_rewind_records),
            candidate_reject_records: scalar
                .candidate_reject_records
                .saturating_add(pending.candidate_reject_records),
            accepted_redo_records: scalar
                .accepted_redo_records
                .saturating_add(pending.accepted_redo_records),
            candidate_chunks_released: scalar
                .candidate_chunks_released
                .saturating_add(pending.candidate_chunks_released),
            accepted_chunks_released: scalar
                .accepted_chunks_released
                .saturating_add(pending.accepted_chunks_released),
            frame_chain_transfers: self.frame_chain_transfers,
            frame_reuse_link_visits: self.frame_reuse_link_visits,
            frame_reuse_visits: self.frame_reuse_visits,
            frame_reuse_incarnations: self.frame_reuse_incarnations,
            #[cfg(feature = "profiling")]
            alignment_delivery_journal_attempts: self.alignment_delivery_journal_attempts,
            ..CommandTimelineCounters::default()
        }
    }

    fn retain(
        &mut self,
        attempt: AttemptMark,
        rollback: CommandRollbackCoordinates,
    ) -> Result<(CommandSnapshotCursor, CommandTimelineMark), CommandSummaryError> {
        if !attempt.is_empty() {
            return Err(CommandSummaryError::AttemptSuspended);
        }
        self.retain_transient(rollback)
    }

    fn retain_transient(
        &mut self,
        rollback: CommandRollbackCoordinates,
    ) -> Result<(CommandSnapshotCursor, CommandTimelineMark), CommandSummaryError> {
        let serial = self
            .next_serial
            .checked_add(1)
            .ok_or(CommandSummaryError::TimelineCapacity)?;
        self.next_serial = serial;
        let cursor = CommandSnapshotCursor::new(serial);
        self.scalars.warm_first_page();
        let scalars = self.scalars.mark();
        let pending_input = self.pending_input.mark();
        self.touched_scalars = 0;
        self.pending_input_touched = false;
        let frame = self.allocate_frame(CommandTimelineFrame { serial, rollback })?;
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
    ) -> Option<CommandRollbackCoordinates> {
        let frame = self.frame(mark.frame)?;
        (self.scalars.validates(mark.scalars)
            && self.pending_input.validates(mark.pending_input)
            && frame.serial == cursor.timeline_serial())
        .then_some(frame.rollback)
    }

    fn has_live_frame(&self) -> bool {
        self.frame_head.is_some()
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

    pub(crate) fn record_delivery_align_state(&mut self, old: i32) {
        #[cfg(feature = "profiling")]
        {
            self.alignment_delivery_journal_attempts =
                self.alignment_delivery_journal_attempts.saturating_add(1);
        }
        self.record_align_state(old);
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

    fn restore_roots(
        &mut self,
        mark: CommandTimelineMark,
        roots: &mut CommandStateRoots<G>,
    ) -> bool {
        if self.frame(mark.frame).is_none() {
            return false;
        }
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
        let suffix = self.chain_from(self.next_frame(mark.frame));
        self.frame_slot_mut(mark.frame)
            .expect("restored command frame remains live")
            .next = None;
        self.frame_tail = Some(mark.frame);
        if let Some(suffix) = suffix {
            self.frame_slot_mut(suffix.head)
                .expect("restored command suffix remains live")
                .previous = None;
            self.prepare_reusable_chain(suffix);
            self.retire_chain(Some(suffix));
        }
        self.touched_scalars = 0;
        self.pending_input_touched = false;
        true
    }

    fn can_begin_checkpoint_candidate(&self, mark: CommandTimelineMark) -> bool {
        self.fork.is_none()
            && self.frame(mark.frame).is_some()
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
        let detached = self.chain_from(self.next_frame(mark.frame));
        self.frame_slot_mut(mark.frame)
            .expect("prevalidated command mark begins the sole fork")
            .next = None;
        if let Some(detached) = detached {
            self.frame_slot_mut(detached.head)
                .expect("detached command suffix remains live")
                .previous = None;
            self.prepare_reusable_chain(detached);
        }
        self.frame_tail = Some(mark.frame);
        self.fork = Some(CommandTimelineFork {
            prefix_tail: mark.frame,
            detached,
            candidate: None,
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
        self.frame_slot_mut(fork.prefix_tail)
            .expect("candidate prefix tail remains live")
            .next = None;
        if let Some(candidate) = fork.candidate {
            self.frame_slot_mut(candidate.head)
                .expect("candidate command suffix remains live")
                .previous = None;
        }
        self.retire_chain(fork.candidate);
        if let Some(detached) = fork.detached {
            self.frame_slot_mut(fork.prefix_tail)
                .expect("candidate prefix tail remains live")
                .next = Some(detached.head);
            self.frame_slot_mut(detached.head)
                .expect("detached command suffix remains live")
                .previous = Some(fork.prefix_tail);
        }
        self.frame_tail = Some(fork.detached.map_or(fork.prefix_tail, |chain| chain.tail));
        self.touched_scalars = 0;
        self.pending_input_touched = false;
    }

    fn accept_checkpoint_candidate(&mut self) {
        let fork = self
            .fork
            .take()
            .expect("command acceptance requires a candidate fork");
        self.scalars.accept_checkpoint_candidate();
        self.pending_input.accept_checkpoint_candidate();
        self.retire_chain(fork.detached);
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
    #[cfg(feature = "profiling")]
    pub alignment_delivery_journal_attempts: u64,
    pub chunks_acquired: u64,
    pub chunks_reused: u64,
    pub selected_rewind_records: u64,
    pub candidate_reject_records: u64,
    pub accepted_redo_records: u64,
    pub candidate_chunks_released: u64,
    pub accepted_chunks_released: u64,
    pub frame_chain_transfers: u64,
    pub frame_reuse_link_visits: u64,
    pub frame_reuse_visits: u64,
    pub frame_reuse_incarnations: u64,
    pub logical_payload_admissions: u64,
    pub full_frame_history_clones: u64,
    pub logical_records: u64,
    pub logical_record_bytes: u64,
    pub logical_coalesced_mutations: u64,
    pub logical_stored_state_captures: u64,
    pub logical_owner_swaps: u64,
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

/// Timeline identity copied beside one exact retained command owner.
///
/// The private timeline row owns every rollback mark. This serial exists only
/// to reject a cursor paired with a different row owner; its value never
/// describes command roots, stack lengths, or arena positions.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct CommandSnapshotCursor {
    timeline_serial: u32,
}

impl CommandSnapshotCursor {
    #[must_use]
    pub const fn new(timeline_serial: u32) -> Self {
        Self { timeline_serial }
    }

    #[must_use]
    pub const fn command_journal(self) -> u32 {
        self.timeline_serial
    }

    const fn timeline_serial(self) -> u32 {
        self.timeline_serial
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
    replay: crate::input::ReplayTransientMark,
    scratch: crate::execution_scratch::ExecutionScratchTransientMark,
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
    fn new(
        generation: CommandGenerationOwner<G>,
        cursor: CommandSnapshotCursor,
        replay: crate::input::ReplayTransientMark,
        scratch: crate::execution_scratch::ExecutionScratchTransientMark,
    ) -> Self {
        Self {
            generation,
            cursor,
            replay,
            scratch,
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
    rollback: CommandRollbackCoordinates,
    attempt: AttemptMark,
    brand: PhantomData<fn(&G) -> &G>,
}

#[cfg(test)]
#[path = "snapshot/tests.rs"]
mod tests;
