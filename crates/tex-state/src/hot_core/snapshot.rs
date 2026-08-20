//! Fixed-size aggregate snapshots over the compact hot-core substrates.
//!
//! This remains a storage-only boundary. The words, stacks, and cursors have
//! no TeX command meaning yet, and neither this runtime mark nor its compact
//! coordinates are format or incremental-checkpoint DTOs.

use core::fmt;
use core::mem::size_of;
use core::num::{NonZeroU32, NonZeroU64};
use std::sync::atomic::{AtomicU64, Ordering};

use super::arena::{
    AcceptedRuntimeValueRegions, RegionArenaError, RegionCoordinate, RuntimeValueRegionAccounting,
    RuntimeValueRegionArena, RuntimeValueRegionMark,
};
use super::journal::{FirstWriteJournal, FirstWriteJournalError, FirstWriteMark};
use super::stack::{PodStack, PodStackAccounting, PodStackError, PodStackMark};
use super::state::{DenseBank, DenseBankError, DenseBankOwner};

const FIRST_HOT_CORE_IDENTITY: u64 = 1 << 48;
static NEXT_HOT_CORE_IDENTITY: AtomicU64 = AtomicU64::new(FIRST_HOT_CORE_IDENTITY);

/// One typed arena family in the storage-only aggregate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HotArenaKind {
    TokenWord,
    TokenList,
    MacroRecord,
    MacroRoot,
    Glue,
    Provenance,
}

/// One typed stack family in the storage-only aggregate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HotStackKind {
    Input,
    Parameter,
    Condition,
    Group,
    Save,
    Mode,
}

/// A rejected aggregate lifecycle or snapshot operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HotCoreError {
    IdentityExhausted,
    ForeignCore,
    ForeignBase,
    InvalidJournalCursor,
    CursorCapacityExhausted,
    SnapshotActive,
    Arena(HotArenaKind, RegionArenaError),
    Stack(HotStackKind, PodStackError),
    State(DenseBankError),
    MutationJournal(FirstWriteJournalError<DenseBankError>),
}

impl fmt::Display for HotCoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IdentityExhausted => f.write_str("hot-core identity space is exhausted"),
            Self::ForeignCore => f.write_str("snapshot belongs to another hot-core candidate"),
            Self::ForeignBase => f.write_str("snapshot belongs to another accepted base"),
            Self::InvalidJournalCursor => {
                f.write_str("snapshot contains a non-ancestor external journal cursor")
            }
            Self::CursorCapacityExhausted => {
                f.write_str("external hot-core journal cursor space is exhausted")
            }
            Self::SnapshotActive => f.write_str("candidate has an active aggregate snapshot"),
            Self::Arena(kind, error) => write!(f, "{kind:?} arena rejected operation: {error}"),
            Self::Stack(kind, error) => write!(f, "{kind:?} stack rejected operation: {error}"),
            Self::State(error) => write!(f, "dense state rejected operation: {error}"),
            Self::MutationJournal(error) => {
                write!(f, "mutation journal rejected operation: {error}")
            }
        }
    }
}

impl std::error::Error for HotCoreError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HotCoreOwner {
    candidate: NonZeroU64,
    accepted_base: NonZeroU64,
}

/// Cursors owned by cold ledgers outside the compact arenas and dense bank.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ExternalJournalCursors {
    page: u32,
    pdf: u32,
    effect: u32,
    output: u32,
    source: u32,
    resource: u32,
}

impl ExternalJournalCursors {
    fn is_ancestor_of(self, current: Self) -> bool {
        self.page <= current.page
            && self.pdf <= current.pdf
            && self.effect <= current.effect
            && self.output <= current.output
            && self.source <= current.source
            && self.resource <= current.resource
    }

    fn advance_all(&mut self, amount: u32) -> Result<(), HotCoreError> {
        let advance = |cursor: u32| {
            cursor
                .checked_add(amount)
                .ok_or(HotCoreError::CursorCapacityExhausted)
        };
        let advanced = Self {
            page: advance(self.page)?,
            pdf: advance(self.pdf)?,
            effect: advance(self.effect)?,
            output: advance(self.output)?,
            source: advance(self.source)?,
            resource: advance(self.resource)?,
        };
        *self = advanced;
        Ok(())
    }
}

/// A fixed-size runtime rollback mark.
///
/// It owns no live data. Every field is an identity, watermark, length, or
/// journal cursor, so shallow size and retained bytes do not depend on the
/// candidate's live-state size.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HotSnapshot {
    owner: HotCoreOwner,
    value_regions: RuntimeValueRegionMark,
    input_stack: PodStackMark,
    parameter_stack: PodStackMark,
    condition_stack: PodStackMark,
    group_stack: PodStackMark,
    save_stack: PodStackMark,
    mode_stack: PodStackMark,
    mutation_journal: FirstWriteMark<DenseBankOwner>,
    external_journals: ExternalJournalCursors,
}

impl HotSnapshot {
    pub(crate) const fn retained_bytes(self) -> usize {
        0
    }
}

/// Immutable accepted arena layers shared once per candidate.
#[derive(Clone)]
pub(crate) struct AcceptedHotCore {
    identity: NonZeroU64,
    value_regions: AcceptedRuntimeValueRegions<u64, u64, u64, u64, u64, u64>,
}

impl AcceptedHotCore {
    pub(crate) fn new(initial_chunk_capacity: NonZeroU32) -> Result<Self, HotCoreError> {
        Ok(Self {
            identity: fresh_hot_core_identity()?,
            value_regions: AcceptedRuntimeValueRegions::new(initial_chunk_capacity),
        })
    }

    pub(crate) fn candidate(
        &self,
        dense_cells: u32,
        initial_value: u64,
    ) -> Result<HotCore, HotCoreError> {
        let state = DenseBank::filled(dense_cells, initial_value).map_err(HotCoreError::State)?;
        let mutation_journal = FirstWriteJournal::new(&state);
        Ok(HotCore {
            owner: HotCoreOwner {
                candidate: fresh_hot_core_identity()?,
                accepted_base: self.identity,
            },
            value_regions: self
                .value_regions
                .candidate()
                .map_err(|error| HotCoreError::Arena(HotArenaKind::TokenWord, error))?,
            input_stack: PodStack::new(),
            parameter_stack: PodStack::new(),
            condition_stack: PodStack::new(),
            group_stack: PodStack::new(),
            save_stack: PodStack::new(),
            mode_stack: PodStack::new(),
            state,
            mutation_journal,
            external_journals: ExternalJournalCursors::default(),
        })
    }

    pub(crate) fn resolve(
        &self,
        kind: HotArenaKind,
        coordinate: RegionCoordinate<u64>,
    ) -> Result<&u64, RegionArenaError> {
        match kind {
            HotArenaKind::TokenWord => self.value_regions.resolve_token_word(coordinate),
            HotArenaKind::TokenList => self.value_regions.resolve_token_list(coordinate),
            HotArenaKind::MacroRecord => self.value_regions.resolve_macro_record(coordinate),
            HotArenaKind::MacroRoot => self.value_regions.resolve_macro_root(coordinate),
            HotArenaKind::Glue => self.value_regions.resolve_glue(coordinate),
            HotArenaKind::Provenance => self.value_regions.resolve_provenance(coordinate),
        }
    }

    pub(crate) fn accounting(&self) -> RuntimeValueRegionAccounting {
        self.value_regions.accounting()
    }
}

/// Mutable candidate overlay composed from every compact storage primitive.
pub(crate) struct HotCore {
    owner: HotCoreOwner,
    value_regions: RuntimeValueRegionArena<u64, u64, u64, u64, u64, u64>,
    input_stack: PodStack<u64>,
    parameter_stack: PodStack<u64>,
    condition_stack: PodStack<u64>,
    group_stack: PodStack<u64>,
    save_stack: PodStack<u64>,
    mode_stack: PodStack<u64>,
    state: DenseBank<u64>,
    mutation_journal: FirstWriteJournal<DenseBank<u64>>,
    external_journals: ExternalJournalCursors,
}

impl HotCore {
    /// Opens one narrow aggregate transaction in constant shallow work.
    pub(crate) fn snapshot(&mut self) -> Result<HotSnapshot, HotCoreError> {
        let snapshot = HotSnapshot {
            owner: self.owner,
            value_regions: self
                .value_regions
                .mark()
                .map_err(|error| HotCoreError::Arena(HotArenaKind::TokenWord, error))?,
            input_stack: self
                .input_stack
                .mark()
                .map_err(|error| HotCoreError::Stack(HotStackKind::Input, error))?,
            parameter_stack: self
                .parameter_stack
                .mark()
                .map_err(|error| HotCoreError::Stack(HotStackKind::Parameter, error))?,
            condition_stack: self
                .condition_stack
                .mark()
                .map_err(|error| HotCoreError::Stack(HotStackKind::Condition, error))?,
            group_stack: self
                .group_stack
                .mark()
                .map_err(|error| HotCoreError::Stack(HotStackKind::Group, error))?,
            save_stack: self
                .save_stack
                .mark()
                .map_err(|error| HotCoreError::Stack(HotStackKind::Save, error))?,
            mode_stack: self
                .mode_stack
                .mark()
                .map_err(|error| HotCoreError::Stack(HotStackKind::Mode, error))?,
            mutation_journal: self
                .mutation_journal
                .mark(&self.state)
                .map_err(HotCoreError::MutationJournal)?,
            external_journals: self.external_journals,
        };
        Ok(snapshot)
    }

    /// Rejects the candidate suffix and restores every component atomically.
    pub(crate) fn rollback(&mut self, snapshot: HotSnapshot) -> Result<(), HotCoreError> {
        self.validate_snapshot(snapshot)?;
        self.mutation_journal
            .rollback(&mut self.state, snapshot.mutation_journal)
            .map_err(HotCoreError::MutationJournal)?;
        self.value_regions
            .truncate(snapshot.value_regions)
            .map_err(|error| HotCoreError::Arena(HotArenaKind::TokenWord, error))?;
        truncate_stack(
            &mut self.input_stack,
            snapshot.input_stack,
            HotStackKind::Input,
        )?;
        truncate_stack(
            &mut self.parameter_stack,
            snapshot.parameter_stack,
            HotStackKind::Parameter,
        )?;
        truncate_stack(
            &mut self.condition_stack,
            snapshot.condition_stack,
            HotStackKind::Condition,
        )?;
        truncate_stack(
            &mut self.group_stack,
            snapshot.group_stack,
            HotStackKind::Group,
        )?;
        truncate_stack(
            &mut self.save_stack,
            snapshot.save_stack,
            HotStackKind::Save,
        )?;
        truncate_stack(
            &mut self.mode_stack,
            snapshot.mode_stack,
            HotStackKind::Mode,
        )?;
        self.external_journals = snapshot.external_journals;
        Ok(())
    }

    /// Accepts values written since the snapshot and retires its inverses.
    pub(crate) fn commit(&mut self, snapshot: HotSnapshot) -> Result<(), HotCoreError> {
        self.validate_snapshot(snapshot)?;
        self.mutation_journal
            .commit(&mut self.state, snapshot.mutation_journal)
            .map_err(HotCoreError::MutationJournal)
    }

    /// Seals candidate arena overlays as a new immutable accepted base.
    pub(crate) fn accept(self) -> Result<AcceptedHotCore, HotCoreError> {
        if !self.mutation_journal.is_idle() {
            return Err(HotCoreError::SnapshotActive);
        }
        self.validate_arena_accepts()?;
        let identity = fresh_hot_core_identity()?;
        Ok(AcceptedHotCore {
            identity,
            value_regions: self
                .value_regions
                .accept()
                .map_err(|error| HotCoreError::Arena(HotArenaKind::TokenWord, error))?,
        })
    }

    pub(crate) fn append_arena_word(
        &mut self,
        kind: HotArenaKind,
        value: u64,
    ) -> Result<RegionCoordinate<u64>, HotCoreError> {
        let result = match kind {
            HotArenaKind::TokenWord => self.value_regions.append_token_word(value),
            HotArenaKind::TokenList => self.value_regions.append_token_list(value),
            HotArenaKind::MacroRecord => self.value_regions.append_macro_record(value),
            HotArenaKind::MacroRoot => self.value_regions.append_macro_root(value),
            HotArenaKind::Glue => self.value_regions.append_glue(value),
            HotArenaKind::Provenance => self.value_regions.append_provenance(value),
        };
        result.map_err(|error| HotCoreError::Arena(kind, error))
    }

    pub(crate) fn resolve(
        &self,
        kind: HotArenaKind,
        coordinate: RegionCoordinate<u64>,
    ) -> Result<&u64, RegionArenaError> {
        match kind {
            HotArenaKind::TokenWord => self.value_regions.resolve_token_word(coordinate),
            HotArenaKind::TokenList => self.value_regions.resolve_token_list(coordinate),
            HotArenaKind::MacroRecord => self.value_regions.resolve_macro_record(coordinate),
            HotArenaKind::MacroRoot => self.value_regions.resolve_macro_root(coordinate),
            HotArenaKind::Glue => self.value_regions.resolve_glue(coordinate),
            HotArenaKind::Provenance => self.value_regions.resolve_provenance(coordinate),
        }
    }

    pub(crate) fn push_stack(
        &mut self,
        kind: HotStackKind,
        value: u64,
    ) -> Result<(), HotCoreError> {
        self.stack_mut(kind)
            .push(value)
            .map_err(|error| HotCoreError::Stack(kind, error))
    }

    pub(crate) fn stack_len(&self, kind: HotStackKind) -> usize {
        self.stack(kind).len()
    }

    pub(crate) fn state_value(&self, index: usize) -> Result<u64, HotCoreError> {
        let coordinate = self.state.coordinate(index).map_err(HotCoreError::State)?;
        self.state.get(coordinate).map_err(HotCoreError::State)
    }

    pub(crate) fn write_state(&mut self, index: usize, value: u64) -> Result<(), HotCoreError> {
        let coordinate = self.state.coordinate(index).map_err(HotCoreError::State)?;
        self.mutation_journal
            .write(&mut self.state, coordinate, value)
            .map_err(HotCoreError::MutationJournal)
    }

    pub(crate) fn advance_external_journals(&mut self, amount: u32) -> Result<(), HotCoreError> {
        self.external_journals.advance_all(amount)
    }

    pub(crate) fn accounting(&self) -> HotCoreAccounting {
        let arenas = self.value_regions.accounting();
        let stacks = [
            self.input_stack.accounting(),
            self.parameter_stack.accounting(),
            self.condition_stack.accounting(),
            self.group_stack.accounting(),
            self.save_stack.accounting(),
            self.mode_stack.accounting(),
        ]
        .into_iter()
        .fold(PodStackAccounting::default(), add_stack_accounting);
        let state = self.state.accounting();
        let journal = self.mutation_journal.accounting();
        HotCoreAccounting {
            arena_logical_values: arenas.logical_values,
            arena_logical_bytes: arenas.logical_bytes,
            stack_logical_entries: stacks.logical_entries,
            stack_logical_bytes: stacks.logical_bytes,
            dense_logical_cells: state.logical_cells,
            dense_logical_value_bytes: state.logical_value_bytes,
            journal_logical_inverses: journal.logical_inverses,
            active_snapshots: journal.active_marks,
            retained_bytes: arenas
                .retained_payload_bytes
                .saturating_add(arenas.registry_capacity.saturating_mul(size_of::<usize>()))
                .saturating_add(stacks.retained_heap_bytes)
                .saturating_add(state.retained_heap_bytes)
                .saturating_add(journal.retained_heap_bytes),
        }
    }

    fn validate_snapshot(&self, snapshot: HotSnapshot) -> Result<(), HotCoreError> {
        if snapshot.owner.candidate != self.owner.candidate {
            return Err(HotCoreError::ForeignCore);
        }
        if snapshot.owner.accepted_base != self.owner.accepted_base {
            return Err(HotCoreError::ForeignBase);
        }
        self.mutation_journal
            .validate_rollback(&self.state, snapshot.mutation_journal)
            .map_err(HotCoreError::MutationJournal)?;
        self.value_regions
            .validate_mark(snapshot.value_regions)
            .map_err(|error| HotCoreError::Arena(HotArenaKind::TokenWord, error))?;
        validate_stack(&self.input_stack, snapshot.input_stack, HotStackKind::Input)?;
        validate_stack(
            &self.parameter_stack,
            snapshot.parameter_stack,
            HotStackKind::Parameter,
        )?;
        validate_stack(
            &self.condition_stack,
            snapshot.condition_stack,
            HotStackKind::Condition,
        )?;
        validate_stack(&self.group_stack, snapshot.group_stack, HotStackKind::Group)?;
        validate_stack(&self.save_stack, snapshot.save_stack, HotStackKind::Save)?;
        validate_stack(&self.mode_stack, snapshot.mode_stack, HotStackKind::Mode)?;
        if !snapshot
            .external_journals
            .is_ancestor_of(self.external_journals)
        {
            return Err(HotCoreError::InvalidJournalCursor);
        }
        Ok(())
    }

    fn validate_arena_accepts(&self) -> Result<(), HotCoreError> {
        self.value_regions
            .validate_accept()
            .map_err(|error| HotCoreError::Arena(HotArenaKind::TokenWord, error))
    }

    fn stack(&self, kind: HotStackKind) -> &PodStack<u64> {
        match kind {
            HotStackKind::Input => &self.input_stack,
            HotStackKind::Parameter => &self.parameter_stack,
            HotStackKind::Condition => &self.condition_stack,
            HotStackKind::Group => &self.group_stack,
            HotStackKind::Save => &self.save_stack,
            HotStackKind::Mode => &self.mode_stack,
        }
    }

    fn stack_mut(&mut self, kind: HotStackKind) -> &mut PodStack<u64> {
        match kind {
            HotStackKind::Input => &mut self.input_stack,
            HotStackKind::Parameter => &mut self.parameter_stack,
            HotStackKind::Condition => &mut self.condition_stack,
            HotStackKind::Group => &mut self.group_stack,
            HotStackKind::Save => &mut self.save_stack,
            HotStackKind::Mode => &mut self.mode_stack,
        }
    }
}

/// Aggregate logical and retained accounting used by plateau gates.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct HotCoreAccounting {
    pub(crate) arena_logical_values: usize,
    pub(crate) arena_logical_bytes: usize,
    pub(crate) stack_logical_entries: usize,
    pub(crate) stack_logical_bytes: usize,
    pub(crate) dense_logical_cells: usize,
    pub(crate) dense_logical_value_bytes: usize,
    pub(crate) journal_logical_inverses: usize,
    pub(crate) active_snapshots: usize,
    pub(crate) retained_bytes: usize,
}

fn validate_stack(
    stack: &PodStack<u64>,
    mark: PodStackMark,
    kind: HotStackKind,
) -> Result<(), HotCoreError> {
    stack
        .validate_mark(mark)
        .map_err(|error| HotCoreError::Stack(kind, error))
}

fn truncate_stack(
    stack: &mut PodStack<u64>,
    mark: PodStackMark,
    kind: HotStackKind,
) -> Result<(), HotCoreError> {
    stack
        .truncate(mark)
        .map_err(|error| HotCoreError::Stack(kind, error))
}

fn add_stack_accounting(left: PodStackAccounting, right: PodStackAccounting) -> PodStackAccounting {
    PodStackAccounting {
        logical_entries: left.logical_entries.saturating_add(right.logical_entries),
        logical_bytes: left.logical_bytes.saturating_add(right.logical_bytes),
        inline_capacity: left.inline_capacity.saturating_add(right.inline_capacity),
        retained_heap_entries: left
            .retained_heap_entries
            .saturating_add(right.retained_heap_entries),
        retained_heap_bytes: left
            .retained_heap_bytes
            .saturating_add(right.retained_heap_bytes),
    }
}

fn fresh_hot_core_identity() -> Result<NonZeroU64, HotCoreError> {
    let raw = NEXT_HOT_CORE_IDENTITY
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .map_err(|_| HotCoreError::IdentityExhausted)?;
    NonZeroU64::new(raw).ok_or(HotCoreError::IdentityExhausted)
}
