#![allow(
    clippy::result_large_err,
    reason = "failure must return the unique arena or fork authority without another allocation"
)]

use core::fmt;

use tex_dense_prefix::VacantSlot;

use crate::{
    AcceptedBlockTable, AcceptedCandidateTables, ArenaError, ArenaMetrics, BlockStore, ForkShape,
    LogicalCursor,
};

/// Convenience owner combining a store and accepted table. Production pools
/// may instead keep those two caller-owned values separate.
pub struct GenerationArena<T> {
    store: BlockStore<T>,
    table: AcceptedBlockTable<T>,
}

impl<T> Default for GenerationArena<T> {
    fn default() -> Self {
        Self {
            store: BlockStore::new(),
            table: AcceptedBlockTable::new(),
        }
    }
}

impl<T> GenerationArena<T> {
    pub fn push_with<F>(&mut self, build: F) -> Result<usize, ArenaError>
    where
        F: for<'slot> FnOnce(VacantSlot<'slot, T>) -> tex_dense_prefix::InitializedSlot<'slot, T>,
    {
        let index = self.table.len();
        self.table.push_with(&mut self.store, build)?;
        Ok(index)
    }

    #[must_use]
    pub fn get(&self, index: usize) -> Option<&T> {
        let position = self.table.position_at_dense_index(index)?;
        self.table.view(&self.store).get(position).ok()
    }

    pub fn cursor(&mut self) -> LogicalCursor {
        self.table.cursor()
    }

    #[must_use]
    pub fn metrics(&self) -> ArenaMetrics {
        self.table.metrics().merged(self.store.metrics())
    }
}

impl<T: Copy> GenerationArena<T> {
    pub fn fork(
        self,
        checkpoint: LogicalCursor,
    ) -> Result<GenerationFork<T>, GenerationForkFailure<T>> {
        let Self { mut store, table } = self;
        match table.fork(&mut store, checkpoint) {
            Ok(tables) => Ok(GenerationFork { store, tables }),
            Err((error, table)) => Err(GenerationForkFailure {
                error,
                arena: Self { store, table },
            }),
        }
    }
}

pub struct GenerationForkFailure<T> {
    error: ArenaError,
    arena: GenerationArena<T>,
}

impl<T> GenerationForkFailure<T> {
    #[must_use]
    pub const fn error(&self) -> &ArenaError {
        &self.error
    }

    #[must_use]
    pub fn into_parts(self) -> (ArenaError, GenerationArena<T>) {
        (self.error, self.arena)
    }
}

impl<T> fmt::Debug for GenerationForkFailure<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GenerationForkFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

pub struct GenerationFork<T: Copy> {
    store: BlockStore<T>,
    tables: AcceptedCandidateTables<T>,
}

impl<T: Copy> GenerationFork<T> {
    #[must_use]
    pub fn candidate_get(&self, index: usize) -> Option<&T> {
        let position = self.tables.candidate_position_at(index)?;
        let (_, candidate) = self.tables.views(&self.store);
        candidate.get(position).ok()
    }

    pub fn candidate_push(&mut self, value: T) -> Result<usize, ArenaError> {
        let index = self.tables.candidate_len();
        self.tables
            .candidate_push_with(&mut self.store, |slot| slot.insert(value))?;
        Ok(index)
    }

    #[must_use]
    pub fn shape(&self) -> ForkShape {
        self.tables.shape()
    }

    #[must_use]
    pub fn metrics(&self) -> ArenaMetrics {
        self.tables.metrics().merged(self.store.metrics())
    }

    pub fn accept(self) -> Result<GenerationArena<T>, GenerationSettlementFailure<T>> {
        let Self { mut store, tables } = self;
        match tables.accept(&mut store) {
            Ok(table) => Ok(GenerationArena { store, table }),
            Err((error, tables)) => Err(GenerationSettlementFailure {
                error,
                fork: Self { store, tables },
            }),
        }
    }

    pub fn reject(self) -> Result<GenerationArena<T>, GenerationSettlementFailure<T>> {
        let Self { mut store, tables } = self;
        match tables.reject(&mut store) {
            Ok(table) => Ok(GenerationArena { store, table }),
            Err((error, tables)) => Err(GenerationSettlementFailure {
                error,
                fork: Self { store, tables },
            }),
        }
    }
}

pub struct GenerationSettlementFailure<T: Copy> {
    error: ArenaError,
    fork: GenerationFork<T>,
}

impl<T: Copy> GenerationSettlementFailure<T> {
    #[must_use]
    pub const fn error(&self) -> &ArenaError {
        &self.error
    }

    #[must_use]
    pub fn into_parts(self) -> (ArenaError, GenerationFork<T>) {
        (self.error, self.fork)
    }
}

impl<T: Copy> fmt::Debug for GenerationSettlementFailure<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GenerationSettlementFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}
