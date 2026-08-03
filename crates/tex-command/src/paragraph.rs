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
        if self.input != transaction.starting_input {
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
