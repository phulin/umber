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
/// levels and without consulting a retired input stack.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ParagraphInputTransaction {
    pub(crate) starting_input: InputState,
    pub(crate) ending_input: InputState,
    pub(crate) starting_parameters: crate::macro_call::ParameterState,
    pub(crate) ending_parameters: crate::macro_call::ParameterState,
    pub(crate) starting_conditions: crate::conditionals::ConditionStack,
    pub(crate) ending_conditions: crate::conditionals::ConditionStack,
    starting_math_shifts: Vec<ParagraphMathShift>,
    ending_math_shifts: Vec<ParagraphMathShift>,
    coverage: ParagraphInputCoverage,
}

impl ParagraphInputTransaction {
    #[must_use]
    pub const fn coverage(&self) -> &ParagraphInputCoverage {
        &self.coverage
    }

    #[must_use]
    pub fn starting_math_shifts(&self) -> &[ParagraphMathShift] {
        &self.starting_math_shifts
    }

    #[must_use]
    pub fn ending_math_shifts(&self) -> &[ParagraphMathShift] {
        &self.ending_math_shifts
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

fn input_level_identity(level: &crate::input::InputLevel) -> crate::input::InputLevelId {
    match level {
        crate::input::InputLevel::Source(source) => source.identity,
        crate::input::InputLevel::Tokens(tokens) => tokens.identity,
    }
}

fn set_input_level_identity(
    level: &mut crate::input::InputLevel,
    identity: crate::input::InputLevelId,
) {
    match level {
        crate::input::InputLevel::Source(source) => source.identity = identity,
        crate::input::InputLevel::Tokens(tokens) => tokens.identity = identity,
    }
}

fn parameter_front_identities(
    recorded: &crate::macro_call::ParameterState,
    live: &crate::macro_call::ParameterState,
    mut definitions_equal: impl FnMut(
        tex_state::ids::MacroDefinitionId,
        tex_state::ids::MacroDefinitionId,
    ) -> bool,
) -> Option<
    Vec<(
        crate::macro_call::MacroActivationId,
        crate::macro_call::MacroActivationId,
    )>,
> {
    if recorded.activations.len() != live.activations.len() {
        return None;
    }
    recorded
        .activations
        .iter()
        .zip(&live.activations)
        .map(|(recorded, live)| {
            (recorded.name == live.name
                && definitions_equal(recorded.definition, live.definition)
                && crate::macro_call::macro_arguments_semantic_eq(
                    &recorded.arguments,
                    &live.arguments,
                ))
            .then_some((recorded.identity, live.identity))
        })
        .collect()
}

fn rebase_activation_identity(
    identity: crate::macro_call::MacroActivationId,
    recorded_next: u64,
    live_next: u64,
    front: &[(
        crate::macro_call::MacroActivationId,
        crate::macro_call::MacroActivationId,
    )],
) -> Option<crate::macro_call::MacroActivationId> {
    front
        .iter()
        .find_map(|(recorded, live)| (*recorded == identity).then_some(*live))
        .or_else(|| {
            (identity.0 >= recorded_next).then(|| {
                identity
                    .0
                    .checked_sub(recorded_next)?
                    .checked_add(live_next)
                    .map(crate::macro_call::MacroActivationId)
            })?
        })
}

fn rebase_token_behavior_activation(
    level: &mut crate::input::InputLevel,
    recorded_next: u64,
    live_next: u64,
    front: &[(
        crate::macro_call::MacroActivationId,
        crate::macro_call::MacroActivationId,
    )],
) -> Option<()> {
    let crate::input::InputLevel::Tokens(tokens) = level else {
        return Some(());
    };
    let crate::input::TokenBehavior::MacroBody(identity) = &mut tokens.behavior else {
        return Some(());
    };
    *identity = rebase_activation_identity(*identity, recorded_next, live_next, front)?;
    Some(())
}

fn adopt_live_front_origins(
    recorded: &mut InputState,
    live: &InputState,
    recorded_activation_next: u64,
    live_activation_next: u64,
    activation_front: &[(
        crate::macro_call::MacroActivationId,
        crate::macro_call::MacroActivationId,
    )],
) -> Option<Vec<(crate::input::InputLevelId, crate::input::InputLevelId)>> {
    if recorded.levels.len() != live.levels.len() {
        return None;
    }
    let mut identities = Vec::with_capacity(recorded.levels.len());
    for (recorded, live) in recorded.levels.iter_mut().zip(&live.levels) {
        rebase_token_behavior_activation(
            recorded,
            recorded_activation_next,
            live_activation_next,
            activation_front,
        )?;
        let recorded_identity = input_level_identity(recorded);
        let live_identity = input_level_identity(live);
        match (&mut *recorded, live) {
            (
                crate::input::InputLevel::Tokens(recorded),
                crate::input::InputLevel::Tokens(live),
            ) => match (&mut recorded.payload, &live.payload) {
                (
                    crate::input::TokenPayload::Stored {
                        tokens: recorded_tokens,
                        origins: recorded_origins,
                    },
                    crate::input::TokenPayload::Stored {
                        tokens: live_tokens,
                        origins: live_origins,
                    },
                ) if recorded_tokens == live_tokens => *recorded_origins = *live_origins,
                (
                    crate::input::TokenPayload::Transient(recorded),
                    crate::input::TokenPayload::Transient(live),
                ) => recorded.adopt_matching_origins(live)?,
                (
                    crate::input::TokenPayload::BackedUp(recorded),
                    crate::input::TokenPayload::BackedUp(live),
                ) => recorded.adopt_matching_origins(live)?,
                (
                    crate::input::TokenPayload::ArgumentRange {
                        buffer: recorded_buffer,
                        range: recorded_range,
                    },
                    crate::input::TokenPayload::ArgumentRange {
                        buffer: live_buffer,
                        range: live_range,
                    },
                ) if recorded_range == live_range => {
                    recorded_buffer.adopt_matching_origins(live_buffer)?;
                }
                (recorded, live) if recorded == live => {}
                _ => return None,
            },
            (recorded, live) if recorded == live => {}
            _ => return None,
        }
        identities.push((recorded_identity, live_identity));
        set_input_level_identity(recorded, live_identity);
    }
    recorded.next_level_identity = live.next_level_identity;
    Some(identities)
}

fn rebase_ending_input_identities(
    ending: &mut InputState,
    recorded_next: u64,
    live_next: u64,
    front: &[(crate::input::InputLevelId, crate::input::InputLevelId)],
    recorded_activation_next: u64,
    live_activation_next: u64,
    activation_front: &[(
        crate::macro_call::MacroActivationId,
        crate::macro_call::MacroActivationId,
    )],
) -> Option<()> {
    for level in &mut ending.levels {
        let old = input_level_identity(level);
        let rebound = front
            .iter()
            .find_map(|(recorded, live)| (*recorded == old).then_some(*live))
            .or_else(|| {
                (old.0 >= recorded_next).then(|| {
                    old.0
                        .checked_sub(recorded_next)?
                        .checked_add(live_next)
                        .map(crate::input::InputLevelId)
                })?
            })?;
        set_input_level_identity(level, rebound);
        rebase_token_behavior_activation(
            level,
            recorded_activation_next,
            live_activation_next,
            activation_front,
        )?;
    }
    ending.next_level_identity = ending
        .next_level_identity
        .checked_sub(recorded_next)?
        .checked_add(live_next)?;
    Some(())
}

fn rebase_ending_parameters(
    ending: &mut crate::macro_call::ParameterState,
    recorded_next: u64,
    live_next: u64,
    front: &[(
        crate::macro_call::MacroActivationId,
        crate::macro_call::MacroActivationId,
    )],
    live_starting: &crate::macro_call::ParameterState,
) -> Option<()> {
    for activation in &mut ending.activations {
        let old = activation.identity;
        activation.identity = rebase_activation_identity(old, recorded_next, live_next, front)?;
        if let Some((_, live_identity)) = front.iter().find(|(recorded, _)| *recorded == old) {
            let live = live_starting
                .activations
                .iter()
                .find(|candidate| candidate.identity == *live_identity)?;
            activation.name = live.name;
            activation.definition = live.definition;
            activation.arguments = live.arguments.clone();
            activation.invocation = live.invocation;
        }
    }
    ending.next_activation_identity = ending
        .next_activation_identity
        .checked_sub(recorded_next)?
        .checked_add(live_next)?;
    Some(())
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ActiveParagraphInputTransaction {
    starting_input: InputState,
    starting_parameters: crate::macro_call::ParameterState,
    starting_conditions: crate::conditionals::ConditionStack,
    starting_math_shifts: Vec<ParagraphMathShift>,
    current_math_shifts: Vec<ParagraphMathShift>,
    root_start: Option<usize>,
    delivered_commands: usize,
    transitions: Vec<InputRecord>,
}

/// Executor-owned `math_shift_group` continuation crossing a paragraph input
/// transaction. TeX82 §§1090 and 1145 allow display entry to finish an hlist
/// while the paragraph's remaining input continues inside the new group.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ParagraphMathShift {
    Inline,
    Display,
    EqNoLeft,
    EqNoRight,
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
            starting_parameters: self.parameters.clone(),
            starting_conditions: self.conditions.clone(),
            starting_math_shifts: Vec::new(),
            current_math_shifts: Vec::new(),
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
            starting_parameters: active.starting_parameters,
            ending_parameters: self.parameters.clone(),
            starting_conditions: active.starting_conditions,
            ending_conditions: self.conditions.clone(),
            starting_math_shifts: active.starting_math_shifts,
            ending_math_shifts: active.current_math_shifts,
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

    pub fn record_paragraph_math_shift_enter(&mut self, shift: ParagraphMathShift) {
        if let Some(active) = &mut self.paragraph_input_transaction {
            active.current_math_shifts.push(shift);
        }
    }

    pub fn record_paragraph_math_shift_exit(&mut self) {
        if let Some(active) = &mut self.paragraph_input_transaction {
            active.current_math_shifts.pop();
        }
    }

    /// Replays a paragraph transition only from its exact recorded input root.
    pub fn replay_paragraph_input_transaction(
        &mut self,
        transaction: &ParagraphInputTransaction,
    ) -> Result<(), ParagraphInputReplayError> {
        self.replay_paragraph_input_transaction_with(transaction, |left, right| left == right)
    }

    /// Replays after comparing allocation-backed macro definitions through
    /// their owning semantic store boundary.
    pub fn replay_paragraph_input_transaction_with(
        &mut self,
        transaction: &ParagraphInputTransaction,
        definitions_equal: impl FnMut(
            tex_state::ids::MacroDefinitionId,
            tex_state::ids::MacroDefinitionId,
        ) -> bool,
    ) -> Result<(), ParagraphInputReplayError> {
        let mut starting_input = transaction.starting_input.clone();
        let recorded_next = starting_input.next_level_identity;
        let live_next = self.input.next_level_identity;
        let recorded_activation_next = transaction.starting_parameters.next_activation_identity;
        let live_activation_next = self.parameters.next_activation_identity;
        let ending_conditions = crate::conditionals::ConditionStack::rebase_paragraph_transition(
            &transaction.starting_conditions,
            &transaction.ending_conditions,
            &self.conditions,
        )
        .ok_or(ParagraphInputReplayError::StartingInputMismatch)?;
        let Some(activation_front) = parameter_front_identities(
            &transaction.starting_parameters,
            &self.parameters,
            definitions_equal,
        ) else {
            return Err(ParagraphInputReplayError::StartingInputMismatch);
        };
        let Some(front_identities) = adopt_live_front_origins(
            &mut starting_input,
            &self.input,
            recorded_activation_next,
            live_activation_next,
            &activation_front,
        ) else {
            return Err(ParagraphInputReplayError::StartingInputMismatch);
        };
        if self.input != starting_input {
            return Err(ParagraphInputReplayError::StartingInputMismatch);
        }
        let mut ending_input = transaction.ending_input.clone();
        rebase_ending_input_identities(
            &mut ending_input,
            recorded_next,
            live_next,
            &front_identities,
            recorded_activation_next,
            live_activation_next,
            &activation_front,
        )
        .ok_or(ParagraphInputReplayError::StartingInputMismatch)?;
        let mut ending_parameters = transaction.ending_parameters.clone();
        rebase_ending_parameters(
            &mut ending_parameters,
            recorded_activation_next,
            live_activation_next,
            &activation_front,
            &self.parameters,
        )
        .ok_or(ParagraphInputReplayError::StartingInputMismatch)?;
        self.input = ending_input;
        self.parameters = ending_parameters;
        self.conditions = ending_conditions;
        Ok(())
    }

    pub(crate) fn paragraph_input_is_recording(&self) -> bool {
        self.paragraph_input_transaction.is_some()
    }

    /// Whether the active paragraph entered while a macro activation owned
    /// its input front. Group-stack effects begun from such a frame cannot be
    /// reconstructed by an input-only paragraph transition.
    #[must_use]
    pub fn paragraph_started_in_macro_frame(&self) -> bool {
        self.paragraph_input_transaction
            .as_ref()
            .is_some_and(|active| {
                !active.starting_parameters.activations.is_empty()
                    || active.starting_input.levels.iter().any(|level| {
                        matches!(
                            level,
                            crate::input::InputLevel::Tokens(tokens)
                                if matches!(tokens.behavior, crate::input::TokenBehavior::MacroBody(_))
                        )
                    })
            })
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
