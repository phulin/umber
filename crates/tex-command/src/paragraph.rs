//! Command-owned paragraph input recording and replay.

use std::sync::Arc;

use crate::input::InputState;
use crate::{CommandObservation, InputRecord};

fn root_source_anchor(input: &InputState) -> Option<usize> {
    input.levels.iter().find_map(|level| {
        let crate::input::InputLevel::Source(source) = level else {
            return None;
        };
        usize::try_from(
            source
                .cursor
                .line
                .as_ref()
                .map_or(source.cursor.next_physical_offset, |line| line.byte_cursor),
        )
        .ok()
    })
}

/// Stable coverage of the canonical input work performed by one paragraph.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ParagraphInputCoverage {
    root_start: Option<usize>,
    root_end: Option<usize>,
    delivered_commands: usize,
    transitions: Arc<[InputRecord]>,
}

impl ParagraphInputCoverage {
    #[must_use]
    pub const fn root_start(&self) -> Option<usize> {
        self.root_start
    }

    #[must_use]
    pub const fn root_end(&self) -> Option<usize> {
        self.root_end
    }

    #[must_use]
    pub const fn delivered_commands(&self) -> usize {
        self.delivered_commands
    }

    #[must_use]
    pub fn transitions(&self) -> impl ExactSizeIterator<Item = &InputRecord> {
        self.transitions.iter()
    }
}

/// Exact canonical input transition for a completed paragraph.
///
/// Both endpoints are command-owned input state.  The executor can retain or
/// replay this value without learning the representation of source and token
/// levels and without consulting the retired `tex_lex::InputStack`.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ParagraphInputTransaction {
    starting_input: InputState,
    ending_input: InputState,
    coverage: ParagraphInputCoverage,
}

impl ParagraphInputTransaction {
    #[must_use]
    pub const fn coverage(&self) -> &ParagraphInputCoverage {
        &self.coverage
    }

    /// Rebinds a transaction wholly contained in an unchanged root prefix.
    ///
    /// A region crossing or following the edit is deliberately rejected: its
    /// line cursor and token provenance belong to the old physical mapping.
    #[must_use]
    pub fn rebind_unchanged_root_prefix(
        &self,
        old: &[u8],
        new: Arc<[u8]>,
        unchanged_end: usize,
    ) -> Option<Self> {
        if self.coverage.root_end? > unchanged_end
            || unchanged_end > old.len()
            || unchanged_end > new.len()
            || old[..unchanged_end] != new[..unchanged_end]
        {
            return None;
        }
        let mut rebound = self.clone();
        rebind_root_input(&mut rebound.starting_input, old, Arc::clone(&new))?;
        rebind_root_input(&mut rebound.ending_input, old, new)?;
        Some(rebound)
    }

    /// Rebinds a transaction wholly outside one edited root interval.
    /// Prefix offsets remain fixed; suffix offsets and line numbers translate
    /// by the exact replacement deltas. Transactions intersecting the edit are
    /// rejected.
    #[must_use]
    pub fn rebind_edited_root(
        &self,
        old: &[u8],
        new: Arc<[u8]>,
        edited: std::ops::Range<usize>,
    ) -> Option<Self> {
        let start = self.coverage.root_start?;
        let end = self.coverage.root_end?;
        if edited.start > edited.end || edited.end > old.len() {
            return None;
        }
        let replacement_len = new
            .len()
            .checked_sub(old.len() - (edited.end - edited.start))?;
        let new_suffix = edited.start.checked_add(replacement_len)?;
        let (byte_delta, line_delta) = if end <= edited.start {
            if old[..edited.start] != new[..edited.start] {
                return None;
            }
            (0, 0)
        } else if start >= edited.end {
            if old[edited.end..] != new[new_suffix..] {
                return None;
            }
            let byte_delta = i64::try_from(new_suffix).ok()? - i64::try_from(edited.end).ok()?;
            let old_lines = old[edited.start..edited.end]
                .iter()
                .filter(|byte| **byte == b'\n')
                .count();
            let new_lines = new[edited.start..new_suffix]
                .iter()
                .filter(|byte| **byte == b'\n')
                .count();
            (
                byte_delta,
                i64::try_from(new_lines).ok()? - i64::try_from(old_lines).ok()?,
            )
        } else {
            return None;
        };
        let mut rebound = self.clone();
        let coverage_delta = isize::try_from(byte_delta).ok()?;
        rebound.coverage.root_start = Some(start.checked_add_signed(coverage_delta)?);
        rebound.coverage.root_end = Some(end.checked_add_signed(coverage_delta)?);
        rebind_root_input_with_delta(
            &mut rebound.starting_input,
            old,
            Arc::clone(&new),
            byte_delta,
            line_delta,
        )?;
        rebind_root_input_with_delta(&mut rebound.ending_input, old, new, byte_delta, line_delta)?;
        Some(rebound)
    }
}

fn rebind_root_input(input: &mut InputState, old: &[u8], new: Arc<[u8]>) -> Option<()> {
    rebind_root_input_with_delta(input, old, new, 0, 0)
}

fn rebind_root_input_with_delta(
    input: &mut InputState,
    old: &[u8],
    new: Arc<[u8]>,
    byte_delta: i64,
    line_delta: i64,
) -> Option<()> {
    let source = input.levels.iter_mut().find_map(|level| match level {
        crate::input::InputLevel::Source(source) => Some(source),
        crate::input::InputLevel::Tokens(_) => None,
    })?;
    if source.cursor.backing.bytes.as_ref() != old {
        return None;
    }
    // A revised editor run registers its root into a fresh CommandState, so
    // the root keeps the same deterministic source identity. Allocating a new
    // identity here made the rebound starting input differ from that live
    // front even when every byte and cursor coordinate outside the edit was
    // proven unchanged. Rebind the immutable backing in place instead: the
    // identity remains the command-owned provenance key, while the descriptor
    // and bytes become those of the revised root.
    let id = source.cursor.backing.id;
    let backing = source
        .cursor
        .backing
        .rebind_generated(id, Arc::clone(&new))
        .ok()?;
    let registered = input
        .registered_sources
        .iter_mut()
        .find(|registered| registered.id == id)?;
    *registered = backing.clone();
    source.cursor.backing = backing;
    source.cursor.next_physical_offset = source
        .cursor
        .next_physical_offset
        .checked_add_signed(byte_delta)?;
    source.cursor.next_line_number = source
        .cursor
        .next_line_number
        .checked_add_signed(line_delta)?;
    if let Some(line) = &mut source.cursor.line {
        line.rehome_edited_backing(id, &new, source.cursor.backing.mode, byte_delta, line_delta)?;
    }
    for level in &mut input.levels {
        let crate::input::InputLevel::Tokens(tokens) = level else {
            continue;
        };
        if let crate::input::TokenPayload::BackedUp(buffer) = &mut tokens.payload {
            buffer.rehome_source(id, byte_delta)?;
        }
    }
    Some(())
}

fn adopt_live_front_origins(recorded: &mut InputState, live: &InputState) -> Option<()> {
    if recorded.levels.len() != live.levels.len() {
        return None;
    }
    for (recorded, live) in recorded.levels.iter_mut().zip(&live.levels) {
        match (recorded, live) {
            (
                crate::input::InputLevel::Tokens(recorded),
                crate::input::InputLevel::Tokens(live),
            ) => match (&mut recorded.payload, &live.payload) {
                (
                    crate::input::TokenPayload::BackedUp(recorded),
                    crate::input::TokenPayload::BackedUp(live),
                ) => recorded.adopt_matching_origins(live)?,
                (recorded, live) if recorded == live => {}
                _ => return None,
            },
            (recorded, live) if recorded == live => {}
            _ => return None,
        }
    }
    Some(())
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ActiveParagraphInputTransaction {
    starting_input: InputState,
    root_start: Option<usize>,
    delivered_commands: usize,
    transitions: Vec<InputRecord>,
}

/// Why a recorded paragraph transition could not be installed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParagraphInputReplayError {
    StartingInputMismatch,
}

impl crate::CommandState {
    /// Starts recording at TeX82 §1091's `new_graf` input boundary.
    pub fn begin_paragraph_input_transaction(&mut self) {
        debug_assert!(self.paragraph_input_transaction.is_none());
        self.paragraph_input_transaction = Some(ActiveParagraphInputTransaction {
            starting_input: self.input.clone(),
            root_start: root_source_anchor(&self.input),
            delivered_commands: 0,
            transitions: Vec::new(),
        });
    }

    /// Finishes the current paragraph's exact input transaction.
    pub fn finish_paragraph_input_transaction(&mut self) -> Option<ParagraphInputTransaction> {
        let active = self.paragraph_input_transaction.take()?;
        Some(ParagraphInputTransaction {
            starting_input: active.starting_input,
            ending_input: self.input.clone(),
            coverage: ParagraphInputCoverage {
                root_start: active.root_start,
                root_end: root_source_anchor(&self.input),
                delivered_commands: active.delivered_commands,
                transitions: active.transitions.into(),
            },
        })
    }

    /// Abandons an in-progress recording after an accepted region validates.
    /// The input itself is left untouched for the replay transaction to
    /// validate and advance atomically.
    pub fn abandon_paragraph_input_transaction(&mut self) {
        self.paragraph_input_transaction = None;
    }

    /// Replays a paragraph transition only from its exact recorded input root.
    pub fn replay_paragraph_input_transaction(
        &mut self,
        transaction: &ParagraphInputTransaction,
    ) -> Result<(), ParagraphInputReplayError> {
        let mut starting_input = transaction.starting_input.clone();
        if adopt_live_front_origins(&mut starting_input, &self.input).is_none()
            || self.input != starting_input
        {
            return Err(ParagraphInputReplayError::StartingInputMismatch);
        }
        self.input = transaction.ending_input.clone();
        Ok(())
    }

    pub(crate) fn paragraph_input_is_recording(&self) -> bool {
        self.paragraph_input_transaction.is_some()
    }

    pub(crate) fn record_paragraph_observation(&mut self, observation: &CommandObservation) {
        let Some(active) = &mut self.paragraph_input_transaction else {
            return;
        };
        match observation {
            CommandObservation::Command(_) => active.delivered_commands += 1,
            CommandObservation::Input(record) => active.transitions.push(record.clone()),
            _ => {}
        }
    }
}
