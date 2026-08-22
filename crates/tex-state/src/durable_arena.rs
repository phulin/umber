//! Typed append-only arenas for generation-durable immutable values.

use core::marker::PhantomData;
use core::num::NonZeroU32;

use crate::generation::ArenaToken;
use crate::glue::GlueSpec;
use crate::provenance::OriginRecord;
use crate::token::TokenWord;

#[cfg(test)]
#[path = "durable_arena/tests.rs"]
mod tests;

pub(super) enum TokenListNamespace {}
pub(super) enum GlueNamespace {}
pub(super) enum ProvenanceNamespace {}

macro_rules! dense_id {
    ($name:ident) => {
        pub struct $name<G> {
            row: NonZeroU32,
            _brand: PhantomData<fn(&G) -> &G>,
        }

        impl<G> Clone for $name<G> {
            fn clone(&self) -> Self {
                *self
            }
        }

        impl<G> Copy for $name<G> {}

        impl<G> PartialEq for $name<G> {
            fn eq(&self, other: &Self) -> bool {
                self.row == other.row
            }
        }

        impl<G> Eq for $name<G> {}

        impl<G> core::hash::Hash for $name<G> {
            fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
                self.row.hash(state);
            }
        }

        impl<G> core::fmt::Debug for $name<G> {
            fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                formatter.write_str(concat!(stringify!($name), "(..)"))
            }
        }

        impl<G> $name<G> {
            fn from_row(row: NonZeroU32) -> Self {
                Self {
                    row,
                    _brand: PhantomData,
                }
            }

            fn index(self) -> usize {
                self.row.get() as usize - 1
            }

            pub(crate) fn format_index(self) -> u32 {
                self.row.get() - 1
            }
        }
    };
}

dense_id!(TokenListId);
dense_id!(GlueId);
dense_id!(ProvenanceId);

impl<G> TokenListId<G> {
    pub(crate) fn format_validation_coordinate(index: u32) -> Option<Self> {
        index
            .checked_add(1)
            .and_then(NonZeroU32::new)
            .map(Self::from_row)
    }
}

impl<G> ProvenanceId<G> {
    pub(crate) fn from_origin_index(index: u32) -> Option<Self> {
        index
            .checked_add(1)
            .and_then(NonZeroU32::new)
            .map(Self::from_row)
    }
}

/// Failure to stage a complete durable row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableAllocationError {
    CapacityOverflow,
    AllocationFailed,
}

fn next_row(len: usize) -> Result<NonZeroU32, DurableAllocationError> {
    len.checked_add(1)
        .and_then(|row| u32::try_from(row).ok())
        .and_then(NonZeroU32::new)
        .ok_or(DurableAllocationError::CapacityOverflow)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TokenSpan {
    start: u32,
    len: u32,
}

impl TokenSpan {
    fn checked(start: usize, len: usize) -> Option<Self> {
        let start = u32::try_from(start).ok()?;
        let len = u32::try_from(len).ok()?;
        start.checked_add(len)?;
        Some(Self { start, len })
    }

    fn resolve(self, words: &[TokenWord]) -> &[TokenWord] {
        let start = self.start as usize;
        &words[start..start + self.len as usize]
    }
}

/// Durable token-register lists, physically separate from definition text.
pub(crate) struct TokenListArena<G> {
    rows: Vec<TokenSpan>,
    words: Vec<TokenWord>,
    _brand: PhantomData<fn(&G) -> &G>,
}

impl<G> TokenListArena<G> {
    pub(crate) fn dense_copy(&self) -> Result<Self, DurableAllocationError> {
        let mut rows = Vec::new();
        rows.try_reserve_exact(self.rows.len())
            .map_err(|_| DurableAllocationError::AllocationFailed)?;
        rows.extend_from_slice(&self.rows);
        let mut words = Vec::new();
        words
            .try_reserve_exact(self.words.len())
            .map_err(|_| DurableAllocationError::AllocationFailed)?;
        words.extend_from_slice(&self.words);
        Ok(Self {
            rows,
            words,
            _brand: PhantomData,
        })
    }

    pub(super) fn new(_token: ArenaToken<G, TokenListNamespace>) -> Self {
        Self {
            rows: Vec::new(),
            words: Vec::new(),
            _brand: PhantomData,
        }
    }

    /// Copies and publishes one complete list after all reservations succeed.
    pub(crate) fn allocate(
        &mut self,
        words: &[TokenWord],
    ) -> Result<TokenListId<G>, DurableAllocationError> {
        let row = next_row(self.rows.len())?;
        let final_word_len = self
            .words
            .len()
            .checked_add(words.len())
            .ok_or(DurableAllocationError::CapacityOverflow)?;
        u32::try_from(final_word_len).map_err(|_| DurableAllocationError::CapacityOverflow)?;
        let span = TokenSpan::checked(self.words.len(), words.len())
            .ok_or(DurableAllocationError::CapacityOverflow)?;

        self.words
            .try_reserve(words.len())
            .map_err(|_| DurableAllocationError::AllocationFailed)?;
        self.rows
            .try_reserve(1)
            .map_err(|_| DurableAllocationError::AllocationFailed)?;
        self.words.extend_from_slice(words);
        self.rows.push(span);
        Ok(TokenListId::from_row(row))
    }

    /// Reserves a complete promotion batch without publishing a row.
    pub(crate) fn reserve_batch(
        &mut self,
        rows: usize,
        words: usize,
    ) -> Result<(), DurableAllocationError> {
        self.rows
            .len()
            .checked_add(rows)
            .and_then(|len| u32::try_from(len).ok())
            .ok_or(DurableAllocationError::CapacityOverflow)?;
        self.words
            .len()
            .checked_add(words)
            .and_then(|len| u32::try_from(len).ok())
            .ok_or(DurableAllocationError::CapacityOverflow)?;
        self.rows
            .try_reserve(rows)
            .map_err(|_| DurableAllocationError::AllocationFailed)?;
        self.words
            .try_reserve(words)
            .map_err(|_| DurableAllocationError::AllocationFailed)
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn get(&self, id: TokenListId<G>) -> &[TokenWord] {
        self.rows[id.index()].resolve(&self.words)
    }

    #[must_use]
    pub(crate) const fn len(&self) -> usize {
        self.rows.len()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub(crate) fn capture_format_rows(&self) -> Vec<Vec<u32>> {
        self.rows
            .iter()
            .map(|span| {
                span.resolve(&self.words)
                    .iter()
                    .map(|word| word.raw())
                    .collect()
            })
            .collect()
    }
}

/// Durable immutable glue specifications.
pub(crate) struct GlueArena<G> {
    rows: Vec<GlueSpec>,
    _brand: PhantomData<fn(&G) -> &G>,
}

impl<G> GlueArena<G> {
    pub(crate) fn dense_copy(&self) -> Result<Self, DurableAllocationError> {
        let mut rows = Vec::new();
        rows.try_reserve_exact(self.rows.len())
            .map_err(|_| DurableAllocationError::AllocationFailed)?;
        rows.extend_from_slice(&self.rows);
        Ok(Self {
            rows,
            _brand: PhantomData,
        })
    }

    pub(super) fn new(_token: ArenaToken<G, GlueNamespace>) -> Self {
        Self {
            rows: Vec::new(),
            _brand: PhantomData,
        }
    }

    pub(crate) fn allocate(
        &mut self,
        value: GlueSpec,
    ) -> Result<GlueId<G>, DurableAllocationError> {
        let row = next_row(self.rows.len())?;
        self.rows
            .try_reserve(1)
            .map_err(|_| DurableAllocationError::AllocationFailed)?;
        self.rows.push(value);
        Ok(GlueId::from_row(row))
    }

    /// Reserves a complete promotion batch without publishing a row.
    pub(crate) fn reserve_batch(&mut self, rows: usize) -> Result<(), DurableAllocationError> {
        self.rows
            .len()
            .checked_add(rows)
            .and_then(|len| u32::try_from(len).ok())
            .ok_or(DurableAllocationError::CapacityOverflow)?;
        self.rows
            .try_reserve(rows)
            .map_err(|_| DurableAllocationError::AllocationFailed)
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn get(&self, id: GlueId<G>) -> GlueSpec {
        self.rows[id.index()]
    }

    #[must_use]
    pub(crate) const fn len(&self) -> usize {
        self.rows.len()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub(crate) fn capture_format_rows(&self) -> Vec<crate::format::schema::FormatGlue> {
        self.rows
            .iter()
            .map(|value| crate::format::schema::FormatGlue {
                width: value.width.raw(),
                stretch: value.stretch.raw(),
                stretch_order: value.stretch_order as u8,
                shrink: value.shrink.raw(),
                shrink_order: value.shrink_order as u8,
            })
            .collect()
    }
}

/// Durable generation-local provenance records.
pub(crate) struct ProvenanceArena<G> {
    rows: Vec<OriginRecord>,
    _brand: PhantomData<fn(&G) -> &G>,
}

impl<G> ProvenanceArena<G> {
    pub(crate) fn dense_copy(&self) -> Result<Self, DurableAllocationError> {
        let mut rows = Vec::new();
        rows.try_reserve_exact(self.rows.len())
            .map_err(|_| DurableAllocationError::AllocationFailed)?;
        rows.extend_from_slice(&self.rows);
        Ok(Self {
            rows,
            _brand: PhantomData,
        })
    }

    pub(super) fn new(_token: ArenaToken<G, ProvenanceNamespace>) -> Self {
        Self {
            rows: Vec::new(),
            _brand: PhantomData,
        }
    }

    pub(crate) fn allocate(
        &mut self,
        value: OriginRecord,
    ) -> Result<ProvenanceId<G>, DurableAllocationError> {
        let row = next_row(self.rows.len())?;
        self.rows
            .try_reserve(1)
            .map_err(|_| DurableAllocationError::AllocationFailed)?;
        self.rows.push(value);
        Ok(ProvenanceId::from_row(row))
    }

    pub(crate) fn coordinate_at(&self, index: u32) -> Option<ProvenanceId<G>> {
        ((index as usize) < self.rows.len()).then(|| {
            ProvenanceId::from_origin_index(index).expect("live provenance index is nonzero")
        })
    }

    /// Reserves a complete promotion batch without publishing a row.
    pub(crate) fn reserve_batch(&mut self, rows: usize) -> Result<(), DurableAllocationError> {
        self.rows
            .len()
            .checked_add(rows)
            .and_then(|len| u32::try_from(len).ok())
            .ok_or(DurableAllocationError::CapacityOverflow)?;
        self.rows
            .try_reserve(rows)
            .map_err(|_| DurableAllocationError::AllocationFailed)
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn get(&self, id: ProvenanceId<G>) -> OriginRecord {
        self.rows[id.index()]
    }

    #[must_use]
    pub(crate) const fn len(&self) -> usize {
        self.rows.len()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}
