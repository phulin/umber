//! Generation-checked dense mutable banks for packed hot-core values.

use core::fmt;
use core::marker::PhantomData;
use core::mem::size_of;
use core::num::{NonZeroU32, NonZeroU64};
use smallvec::SmallVec;
use std::sync::atomic::{AtomicU64, Ordering};

use super::journal::JournalTarget;

const INLINE_CELLS: usize = 32;
const FIRST_BANK_NAMESPACE: u64 = 1 << 32;
static NEXT_BANK_NAMESPACE: AtomicU64 = AtomicU64::new(FIRST_BANK_NAMESPACE);

#[derive(Clone, Copy)]
struct DenseCell<T: Copy> {
    value: T,
    write_epoch: u32,
}

/// A rejected dense-bank operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DenseBankError {
    NamespaceExhausted,
    GenerationExhausted,
    IndexCapacityExhausted,
    AllocationFailed,
    ForeignNamespace,
    StaleGeneration,
    IndexOutOfBounds,
}

impl fmt::Display for DenseBankError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NamespaceExhausted => "dense-bank namespace space is exhausted",
            Self::GenerationExhausted => "dense-bank generation space is exhausted",
            Self::IndexCapacityExhausted => "dense-bank index space is exhausted",
            Self::AllocationFailed => "dense-bank storage allocation failed",
            Self::ForeignNamespace => "coordinate belongs to a foreign dense bank",
            Self::StaleGeneration => "coordinate belongs to a stale dense-bank generation",
            Self::IndexOutOfBounds => "coordinate is outside its dense bank",
        })
    }
}

impl std::error::Error for DenseBankError {}

/// Runtime owner identity for one dense-bank generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DenseBankOwner {
    namespace: NonZeroU64,
    generation: NonZeroU32,
}

/// A compact typed coordinate naming one mutable dense-bank cell.
pub(crate) struct DenseBankCoordinate<T> {
    namespace: NonZeroU64,
    generation: NonZeroU32,
    index: u32,
    marker: PhantomData<fn() -> T>,
}

impl<T> DenseBankCoordinate<T> {
    pub(crate) const fn index(self) -> u32 {
        self.index
    }
}

impl<T> Copy for DenseBankCoordinate<T> {}

impl<T> Clone for DenseBankCoordinate<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> fmt::Debug for DenseBankCoordinate<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DenseBankCoordinate")
            .field("namespace", &self.namespace)
            .field("generation", &self.generation)
            .field("index", &self.index)
            .finish()
    }
}

impl<T> PartialEq for DenseBankCoordinate<T> {
    fn eq(&self, other: &Self) -> bool {
        self.namespace == other.namespace
            && self.generation == other.generation
            && self.index == other.index
    }
}

impl<T> Eq for DenseBankCoordinate<T> {}

/// Logical and retained storage owned by one dense bank.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DenseBankAccounting {
    pub(crate) logical_cells: usize,
    pub(crate) logical_value_bytes: usize,
    pub(crate) inline_capacity: usize,
    pub(crate) retained_heap_cells: usize,
    pub(crate) retained_heap_bytes: usize,
}

/// Fixed-length direct-indexed mutable values with typed runtime coordinates.
pub(crate) struct DenseBank<T: Copy> {
    owner: DenseBankOwner,
    cells: SmallVec<[DenseCell<T>; INLINE_CELLS]>,
}

impl<T: Copy> DenseBank<T> {
    pub(crate) fn filled(len: u32, value: T) -> Result<Self, DenseBankError> {
        let namespace = fresh_bank_namespace()?;
        let len = len as usize;
        let mut cells = SmallVec::new();
        cells
            .try_reserve_exact(len)
            .map_err(|_| DenseBankError::AllocationFailed)?;
        cells.resize(
            len,
            DenseCell {
                value,
                write_epoch: 0,
            },
        );
        Ok(Self {
            owner: DenseBankOwner {
                namespace,
                generation: NonZeroU32::MIN,
            },
            cells,
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.cells.len()
    }

    pub(crate) fn coordinate(
        &self,
        index: usize,
    ) -> Result<DenseBankCoordinate<T>, DenseBankError> {
        if index >= self.cells.len() {
            return Err(DenseBankError::IndexOutOfBounds);
        }
        let index = u32::try_from(index).map_err(|_| DenseBankError::IndexCapacityExhausted)?;
        Ok(DenseBankCoordinate {
            namespace: self.owner.namespace,
            generation: self.owner.generation,
            index,
            marker: PhantomData,
        })
    }

    pub(crate) fn get(&self, coordinate: DenseBankCoordinate<T>) -> Result<T, DenseBankError> {
        Ok(self.cell(coordinate)?.value)
    }

    /// Starts a new runtime generation while retaining the exact allocation.
    ///
    /// Any journal for the previous generation must already be quiescent. Its
    /// owner identity then rejects this bank instead of interpreting stale
    /// inverse coordinates.
    pub(crate) fn reset_generation(&mut self, value: T) -> Result<(), DenseBankError> {
        let generation = self
            .owner
            .generation
            .get()
            .checked_add(1)
            .and_then(NonZeroU32::new)
            .ok_or(DenseBankError::GenerationExhausted)?;
        self.owner.generation = generation;
        for cell in &mut self.cells {
            cell.value = value;
            cell.write_epoch = 0;
        }
        Ok(())
    }

    pub(crate) fn accounting(&self) -> DenseBankAccounting {
        let retained_heap_cells = if self.cells.spilled() {
            self.cells.capacity()
        } else {
            0
        };
        DenseBankAccounting {
            logical_cells: self.cells.len(),
            logical_value_bytes: self.cells.len().saturating_mul(size_of::<T>()),
            inline_capacity: INLINE_CELLS,
            retained_heap_cells,
            retained_heap_bytes: retained_heap_cells.saturating_mul(size_of::<DenseCell<T>>()),
        }
    }

    fn cell(&self, coordinate: DenseBankCoordinate<T>) -> Result<&DenseCell<T>, DenseBankError> {
        self.validate(coordinate)?;
        Ok(&self.cells[coordinate.index as usize])
    }

    fn validate(&self, coordinate: DenseBankCoordinate<T>) -> Result<(), DenseBankError> {
        if coordinate.namespace != self.owner.namespace {
            return Err(DenseBankError::ForeignNamespace);
        }
        if coordinate.generation != self.owner.generation {
            return Err(DenseBankError::StaleGeneration);
        }
        if coordinate.index as usize >= self.cells.len() {
            return Err(DenseBankError::IndexOutOfBounds);
        }
        Ok(())
    }
}

impl<T: Copy> JournalTarget for DenseBank<T> {
    type Coordinate = DenseBankCoordinate<T>;
    type Value = T;
    type Owner = DenseBankOwner;
    type Error = DenseBankError;

    fn journal_owner(&self) -> Self::Owner {
        self.owner
    }

    fn read_for_journal(
        &self,
        coordinate: Self::Coordinate,
    ) -> Result<(Self::Value, u32), Self::Error> {
        let cell = self.cell(coordinate)?;
        Ok((cell.value, cell.write_epoch))
    }

    fn validate_for_journal(&self, coordinate: Self::Coordinate) -> Result<(), Self::Error> {
        self.validate(coordinate)
    }

    fn write_validated(
        &mut self,
        coordinate: Self::Coordinate,
        value: Self::Value,
        write_epoch: u32,
    ) {
        debug_assert!(self.validate(coordinate).is_ok());
        let cell = &mut self.cells[coordinate.index as usize];
        cell.value = value;
        cell.write_epoch = write_epoch;
    }
}

fn fresh_bank_namespace() -> Result<NonZeroU64, DenseBankError> {
    let namespace = NEXT_BANK_NAMESPACE
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
            next.checked_add(1)
        })
        .map_err(|_| DenseBankError::NamespaceExhausted)?;
    NonZeroU64::new(namespace).ok_or(DenseBankError::NamespaceExhausted)
}
